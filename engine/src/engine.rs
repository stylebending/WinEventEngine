use crate::config::{ActionConfig, Config, RuleConfig, SourceConfig, SourceType, TriggerConfig};
use crate::plugins::file_watcher::FileWatcherPlugin;
use crate::plugins::process_monitor::ProcessMonitorPlugin;
use crate::plugins::registry_monitor::{RegistryMonitorPlugin, RegistryRoot};
use crate::plugins::window_watcher::WindowEventPlugin;
use actions::{Action, ActionExecutor, ExecuteAction, LogAction, LogLevel, PowerShellAction};
use bus::create_event_bus;
use engine_core::event::EventKind;
use engine_core::plugin::EventSourcePlugin;
use metrics::{
    record_event_processing_duration, record_rule_match_duration, MetricsCollector,
};
use metrics::server::RuleManager;
use rules::{EventKindMatcher, FilePatternMatcher, Rule, RuleMatcher, WindowMatcher, WindowEventType};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout, Instant};
use tracing::{error, info, warn};

pub struct Engine {
    config: Config,
    config_path: Option<PathBuf>,
    automations_path: Option<PathBuf>,
    plugins: Vec<Box<dyn EventSourcePlugin>>,
    rules: Vec<Rule>,
    rule_configs: Arc<RwLock<Vec<RuleConfig>>>,
    action_executor: ActionExecutor,
    event_sender: Option<mpsc::Sender<engine_core::event::Event>>,
    shutdown_flag: Arc<std::sync::atomic::AtomicBool>,
    config_reload_rx: Option<mpsc::Receiver<()>>,
    metrics: Arc<MetricsCollector>,
}

impl Engine {
    pub fn new(config: Config, config_path: Option<PathBuf>) -> Self {
        let metrics = Arc::new(MetricsCollector::new());
        
        // Determine automations file path (same directory as config, or current dir)
        let automations_path = config_path.as_ref().map(|p| {
            p.parent()
                .map(|p| p.join("automations.json"))
                .unwrap_or_else(|| PathBuf::from("automations.json"))
        }).unwrap_or_else(|| PathBuf::from("automations.json"));
        
        // Load existing automations from file if it exists
        let mut rule_configs: Vec<RuleConfig> = config.rules.clone();
        if automations_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&automations_path) {
                if let Ok(loaded) = serde_json::from_str::<Vec<RuleConfig>>(&content) {
                    // Merge: add automations that don't exist in config
                    let loaded_count = loaded.len();
                    for rule in loaded {
                        if !rule_configs.iter().any(|r| r.name == rule.name) {
                            rule_configs.push(rule);
                        }
                    }
                    info!("Loaded {} rules from automations.json", loaded_count);
                }
            }
        }
        
        Self {
            config,
            config_path,
            automations_path: Some(automations_path),
            plugins: Vec::new(),
            rules: Vec::new(),
            rule_configs: Arc::new(RwLock::new(rule_configs)),
            action_executor: ActionExecutor::new(),
            event_sender: None,
            shutdown_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            config_reload_rx: None,
            metrics,
        }
    }
    
    /// Get a reference to the metrics collector
    pub fn metrics(&self) -> Arc<MetricsCollector> {
        self.metrics.clone()
    }

    pub fn take_config_reload_rx(&mut self) -> Option<mpsc::Receiver<()>> {
        self.config_reload_rx.take()
    }

    pub async fn initialize(&mut self) -> Result<(), EngineError> {
        info!("Initializing Windows Event Automation Engine");

        // Create event bus
        let (sender, mut receiver) = create_event_bus(self.config.engine.event_buffer_size);
        self.event_sender = Some(sender.clone());

        // Initialize plugins from configuration
        self.initialize_plugins(sender.clone()).await?;

        // Initialize rules from configuration
        self.initialize_rules();

        // Initialize actions from configuration
        self.initialize_actions();

        // Start event processing loop
        let rules = self.rules.clone();
        let action_executor = self.action_executor.clone();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            info!("Event processing loop started");

            while let Some(event) = receiver.recv().await {
                let start_time = Instant::now();
                let event_source = event.source.clone();
                let event_type = format!("{:?}", event.kind);

                // Record event received with broadcast
                metrics.record_event_with_broadcast(&event_source, &event_type);

                tracing::debug!("Processing event: {:?} from {}", event.kind, event.source);

                for (idx, rule) in rules.iter().enumerate() {
                    if !rule.enabled {
                        continue;
                    }

                    // Record rule evaluation with broadcast
                    metrics.record_rule_evaluation_with_broadcast(&rule.name);

                    let match_start = Instant::now();
                    let matched = rule.matches(&event);
                    record_rule_match_duration(&metrics, &rule.name, match_start.elapsed());

                    if matched {
                        // Record successful rule match with broadcast
                        metrics.record_rule_match_with_broadcast(&rule.name);
                        info!("Rule '{}' matched event from {}", rule.name, event.source);

                        let action_name = format!("rule_{}_action", idx);
                        let action_start = Instant::now();

                        match action_executor.execute(&action_name, &event) {
                            Ok(result) => {
                                metrics.record_action_execution_with_broadcast(
                                    &action_name,
                                    true,
                                    action_start.elapsed()
                                );
                                info!("Action executed successfully: {:?}", result);
                            }
                            Err(e) => {
                                metrics.record_action_execution_with_broadcast(
                                    &action_name,
                                    false,
                                    action_start.elapsed()
                                );
                                error!("Action execution failed: {}", e);
                            }
                        }
                    }
                }

                // Record total event processing duration
                record_event_processing_duration(&metrics, start_time.elapsed());
            }

            info!("Event processing loop stopped");
        });

        info!("Engine initialized successfully");
        Ok(())
    }

    async fn initialize_plugins(
        &mut self,
        sender: mpsc::Sender<engine_core::event::Event>,
    ) -> Result<(), EngineError> {
        for source_config in &self.config.sources {
            if !source_config.enabled {
                info!("Skipping disabled source: {}", source_config.name);
                continue;
            }

            match self.create_plugin(source_config, sender.clone()).await {
                Ok(plugin) => {
                    info!("Initialized plugin: {}", source_config.name);
                    self.plugins.push(plugin);
                }
                Err(e) => {
                    error!("Failed to initialize plugin {}: {}", source_config.name, e);
                }
            }
        }

        Ok(())
    }

    async fn create_plugin(
        &self,
        config: &SourceConfig,
        sender: mpsc::Sender<engine_core::event::Event>,
    ) -> Result<Box<dyn EventSourcePlugin>, EngineError> {
        match &config.source_type {
            SourceType::FileWatcher {
                paths,
                pattern,
                recursive,
            } => {
                let mut plugin =
                    FileWatcherPlugin::new(&config.name, paths.clone()).with_recursive(*recursive);

                if let Some(pattern) = pattern {
                    plugin = plugin.with_pattern(pattern);
                }

                plugin
                    .start(sender)
                    .await
                    .map_err(|e| EngineError::PluginInit(config.name.clone(), e.to_string()))?;

                Ok(Box::new(plugin))
            }
            SourceType::WindowWatcher {
                title_pattern,
                process_pattern,
            } => {
                let mut plugin = WindowEventPlugin::new(&config.name);

                if let Some(title) = title_pattern {
                    plugin = plugin.with_title_filter(title);
                }

                if let Some(process) = process_pattern {
                    plugin = plugin.with_process_filter(process);
                }

                plugin
                    .start(sender)
                    .await
                    .map_err(|e| EngineError::PluginInit(config.name.clone(), e.to_string()))?;

                Ok(Box::new(plugin))
            }
            SourceType::ProcessMonitor {
                process_name,
                monitor_threads,
                monitor_files,
                monitor_network,
            } => {
                let mut plugin = ProcessMonitorPlugin::new(&config.name)
                    .with_thread_monitoring(*monitor_threads)
                    .with_file_monitoring(*monitor_files)
                    .with_network_monitoring(*monitor_network);

                if let Some(name) = process_name {
                    plugin = plugin.with_name_filter(name);
                }

                plugin
                    .start(sender)
                    .await
                    .map_err(|e| EngineError::PluginInit(config.name.clone(), e.to_string()))?;

                Ok(Box::new(plugin))
            }
            SourceType::RegistryMonitor {
                root,
                key,
                recursive,
            } => {
                let root_enum = match root.as_str() {
                    "HKLM" => RegistryRoot::HKEY_LOCAL_MACHINE,
                    "HKCU" => RegistryRoot::HKEY_CURRENT_USER,
                    "HKU" => RegistryRoot::HKEY_USERS,
                    "HKCC" => RegistryRoot::HKEY_CURRENT_CONFIG,
                    _ => {
                        return Err(EngineError::Config(format!(
                            "Invalid registry root: {}",
                            root
                        )));
                    }
                };

                let mut plugin = if *recursive {
                    RegistryMonitorPlugin::new(&config.name).watch_key_recursive(root_enum, key)
                } else {
                    RegistryMonitorPlugin::new(&config.name).watch_key(root_enum, key)
                };

                plugin
                    .start(sender)
                    .await
                    .map_err(|e| EngineError::PluginInit(config.name.clone(), e.to_string()))?;

                Ok(Box::new(plugin))
            }
        }
    }

    fn initialize_rules(&mut self) {
        let configs = self.rule_configs.read().unwrap();
        for rule_config in configs.iter() {
            if !rule_config.enabled {
                continue;
            }

            match self.create_rule(rule_config) {
                Ok(rule) => {
                    info!("Loaded rule: {}", rule.name);
                    self.rules.push(rule);
                }
                Err(e) => {
                    error!("Failed to create rule {}: {}", rule_config.name, e);
                }
            }
        }
    }

    fn create_rule(&self, config: &RuleConfig) -> Result<Rule, EngineError> {
        let matcher: Box<dyn RuleMatcher> = match &config.trigger {
            TriggerConfig::FileCreated { pattern, .. } => {
                let mut matcher = FilePatternMatcher::created();
                if let Some(pat) = pattern {
                    matcher = matcher
                        .with_file_pattern(pat)
                        .map_err(|e| EngineError::Config(format!("Invalid pattern: {}", e)))?;
                }
                Box::new(matcher)
            }
            TriggerConfig::FileModified { pattern, .. } => {
                let mut matcher = FilePatternMatcher::modified();
                if let Some(pat) = pattern {
                    matcher = matcher
                        .with_file_pattern(pat)
                        .map_err(|e| EngineError::Config(format!("Invalid pattern: {}", e)))?;
                }
                Box::new(matcher)
            }
            TriggerConfig::FileDeleted { pattern, .. } => {
                let mut matcher = FilePatternMatcher::deleted();
                if let Some(pat) = pattern {
                    matcher = matcher
                        .with_file_pattern(pat)
                        .map_err(|e| EngineError::Config(format!("Invalid pattern: {}", e)))?;
                }
                        Box::new(matcher)
            }
            TriggerConfig::WindowFocused {
                title_contains,
                process_name,
            } => Box::new(WindowMatcher {
                event_type: WindowEventType::Focused,
                title_contains: title_contains.clone(),
                process_name: process_name.clone(),
            }),
            TriggerConfig::WindowUnfocused {
                title_contains,
                process_name,
            } => Box::new(WindowMatcher {
                event_type: WindowEventType::Unfocused,
                title_contains: title_contains.clone(),
                process_name: process_name.clone(),
            }),
            TriggerConfig::WindowCreated => Box::new(EventKindMatcher {
                kind: EventKind::WindowCreated {
                    hwnd: 0,
                    title: String::new(),
                    process_id: 0,
                },
            }),
            TriggerConfig::ProcessStarted { process_name: _ } => Box::new(EventKindMatcher {
                kind: EventKind::ProcessStarted {
                    pid: 0,
                    parent_pid: 0,
                    name: String::new(),
                    path: String::new(),
                    command_line: String::new(),
                    session_id: 0,
                    user: String::new(),
                },
            }),
            TriggerConfig::ProcessStopped { process_name: _ } => Box::new(EventKindMatcher {
                kind: EventKind::ProcessStopped {
                    pid: 0,
                    name: String::new(),
                    exit_code: None,
                },
            }),
            TriggerConfig::RegistryChanged { value_name: _ } => Box::new(EventKindMatcher {
                kind: EventKind::RegistryChanged {
                    root: String::new(),
                    key: String::new(),
                    value_name: None,
                    change_type: engine_core::event::RegistryChangeType::Modified,
                },
            }),
            TriggerConfig::Timer {
                interval_seconds: _,
            } => Box::new(EventKindMatcher {
                kind: EventKind::TimerTick,
            }),
        };

        let mut rule = Rule::new(&config.name, matcher);

        if let Some(desc) = &config.description {
            rule = rule.with_description(desc);
        }

        Ok(rule.with_enabled(config.enabled))
    }

    fn initialize_actions(&mut self) {
        // Register actions from rule configurations
        for (idx, rule_config) in self.config.rules.iter().enumerate() {
            let action_name = format!("rule_{}_action", idx);
            let action: Box<dyn Action> = match &rule_config.action {
                ActionConfig::Execute {
                    command,
                    args,
                    working_dir,
                } => {
                    let mut exec = ExecuteAction::new(command).with_args(args.clone());
                    if let Some(dir) = working_dir {
                        exec = exec.with_working_dir(dir.clone());
                    }
                    Box::new(exec)
                }
                ActionConfig::PowerShell {
                    script,
                    working_dir,
                } => {
                    let mut ps = PowerShellAction::new(script);
                    if let Some(dir) = working_dir {
                        ps = ps.with_working_dir(dir.clone());
                    }
                    Box::new(ps)
                }
                ActionConfig::Log { message, level } => {
                    let log_level = match level.as_str() {
                        "debug" => LogLevel::Debug,
                        "info" => LogLevel::Info,
                        "warn" => LogLevel::Warn,
                        "error" => LogLevel::Error,
                        _ => LogLevel::Info,
                    };
                    Box::new(LogAction::new(message).with_level(log_level))
                }
                ActionConfig::Notify { title, message } => {
                    // For now, use log action as a placeholder for notifications
                    Box::new(LogAction::new(format!("{}: {}", title, message)))
                }
                ActionConfig::HttpRequest { url, .. } => {
                    // HTTP requests would need additional implementation
                    Box::new(LogAction::new(format!("HTTP request to: {}", url)))
                }
                ActionConfig::Media { command } => {
                    let script = match command.as_str() {
                        "play" => {
                            r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class MediaKeys {
    [DllImport("user32.dll", CharSet = CharSet.Auto, CallingConvention = CallingConvention.StdCall)]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    public const byte VK_MEDIA_PLAY_PAUSE = 0xB3;
    public static void PlayPause() {
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 0, UIntPtr.Zero);
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 2, UIntPtr.Zero);
    }
}
"@
[MediaKeys]::PlayPause()
"#
                        }
                        "pause" => {
                            r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class MediaKeys {
    [DllImport("user32.dll", CharSet = CharSet.Auto, CallingConvention = CallingConvention.StdCall)]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    public const byte VK_MEDIA_PLAY_PAUSE = 0xB3;
    public static void PlayPause() {
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 0, UIntPtr.Zero);
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 2, UIntPtr.Zero);
    }
}
"@
[MediaKeys]::PlayPause()
"#
                        }
                        "toggle" => {
                            r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class MediaKeys {
    [DllImport("user32.dll", CharSet = CharSet.Auto, CallingConvention = CallingConvention.StdCall)]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    public const byte VK_MEDIA_PLAY_PAUSE = 0xB3;
    public static void PlayPause() {
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 0, UIntPtr.Zero);
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 2, UIntPtr.Zero);
    }
}
"@
[MediaKeys]::PlayPause()
"#
                        }
                        _ => {
                            r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class MediaKeys {
    [DllImport("user32.dll", CharSet = CharSet.Auto, CallingConvention = CallingConvention.StdCall)]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    public const byte VK_MEDIA_PLAY_PAUSE = 0xB3;
    public static void PlayPause() {
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 0, UIntPtr.Zero);
        keybd_event(VK_MEDIA_PLAY_PAUSE, 0, 2, UIntPtr.Zero);
    }
}
"@
[MediaKeys]::PlayPause()
"#
                        }
                    };
                    Box::new(PowerShellAction::new(script))
                }
                ActionConfig::Script {
                    path,
                    function,
                    timeout_ms,
                    on_error,
                } => {
                    use actions::{ScriptAction, ScriptErrorBehavior};
                    
                    // Resolve path relative to plugins/actions/ if not absolute
                    let script_path = if path.is_absolute() {
                        path.clone()
                    } else {
                        PathBuf::from("plugins/actions").join(path)
                    };
                    
                    match ScriptAction::new(script_path, function.clone()) {
                        Ok(mut script_action) => {
                            // Set timeout if specified
                            if let Some(timeout) = timeout_ms {
                                script_action = script_action.with_timeout(*timeout);
                            }
                            
                            // Set error behavior
                            if let Ok(behavior) = on_error.parse::<ScriptErrorBehavior>() {
                                script_action = script_action.with_error_behavior(behavior);
                            }
                            
                            Box::new(script_action)
                        }
                        Err(e) => {
                            error!("Failed to create script action: {}", e);
                            // Fallback to log action showing the error
                            Box::new(LogAction::new(format!(
                                "Script action failed to load: {}",
                                e
                            )))
                        }
                    }
                }
            };

            self.action_executor.register(action_name, action);
        }
    }

    pub async fn shutdown(&mut self) {
        info!("Shutting down engine");

        for plugin in &mut self.plugins {
            if let Err(e) = plugin.stop().await {
                error!("Error stopping plugin: {}", e);
            }
        }

        info!("Engine shutdown complete");
    }

    pub fn get_status(&self) -> EngineStatus {
        EngineStatus {
            active_plugins: self.plugins.len(),
            active_rules: self.rules.len(),
        }
    }

    pub async fn reload(&mut self, new_config: Config) -> Result<(), EngineError> {
        info!("Starting full config reload");

        if let Err(e) = new_config.validate() {
            warn!(
                "New configuration validation failed: {}, keeping current config",
                e
            );
            self.metrics.record_config_reload_with_broadcast(false);
            return Err(EngineError::Config(e.to_string()));
        }

        info!("Stopping all plugins for reload");
        for plugin in &mut self.plugins {
            if let Err(e) = plugin.stop().await {
                error!("Error stopping plugin during reload: {}", e);
            }
        }
        self.plugins.clear();
        self.rules.clear();

        self.config = new_config;

        if let Some(sender) = &self.event_sender {
            self.initialize_plugins(sender.clone()).await?;
        }

        self.initialize_rules();
        self.initialize_actions();

        if let Some(_sender) = &self.event_sender {
            let rules = self.rules.clone();
            let action_executor = self.action_executor.clone();
            let metrics = self.metrics.clone();
            let mut receiver = bus::create_event_bus(self.config.engine.event_buffer_size).1;

            tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    let start_time = Instant::now();
                    let event_source = event.source.clone();
                    let event_type = format!("{:?}", event.kind);
                    
                    metrics.record_event_with_broadcast(&event_source, &event_type);

                    tracing::debug!("Processing event: {:?} from {}", event.kind, event.source);

                    for (idx, rule) in rules.iter().enumerate() {
                        if !rule.enabled {
                            continue;
                        }

                        metrics.record_rule_evaluation_with_broadcast(&rule.name);

                        let match_start = Instant::now();
                        let matched = rule.matches(&event);
                        record_rule_match_duration(&metrics, &rule.name, match_start.elapsed());

                        if matched {
                            metrics.record_rule_match_with_broadcast(&rule.name);
                            info!("Rule '{}' matched event from {}", rule.name, event.source);

                            let action_name = format!("rule_{}_action", idx);
                            let action_start = Instant::now();

                            match action_executor.execute(&action_name, &event) {
                                Ok(result) => {
                                    metrics.record_action_execution_with_broadcast(
                                        &action_name,
                                        true,
                                        action_start.elapsed()
                                    );
                                    info!("Action executed successfully: {:?}", result);
                                }
                                Err(e) => {
                                    metrics.record_action_execution_with_broadcast(
                                        &action_name,
                                        false,
                                        action_start.elapsed()
                                    );
                                    error!("Action execution failed: {}", e);
                                }
                            }
                        }
                    }

                    record_event_processing_duration(&metrics, start_time.elapsed());
                }
            });
        }
        
        self.metrics.record_config_reload_with_broadcast(true);

        let status = self.get_status();
        info!(
            "Config reload complete: {} plugins, {} rules",
            status.active_plugins, status.active_rules
        );

        Ok(())
    }

    pub async fn watch_config(&mut self) {
        let config_path = match &self.config_path {
            Some(p) => p.clone(),
            None => {
                info!("No config path configured, skipping config watcher");
                return;
            }
        };

        let (tx, rx) = mpsc::channel(10);
        self.config_reload_rx = Some(rx);

        let shutdown_flag = self.shutdown_flag.clone();

        tokio::spawn(async move {
            use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};

            let (notify_tx, mut notify_rx) = mpsc::channel(100);

            let mut watcher: RecommendedWatcher = match Watcher::new(
                move |res: Result<notify::Event, notify::Error>| {
                    if let Ok(event) = res {
                        let _ = notify_tx.blocking_send(event);
                    }
                },
                NotifyConfig::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    error!("Failed to create config watcher: {}", e);
                    return;
                }
            };

            let watch_path = if config_path.is_dir() {
                config_path.clone()
            } else {
                config_path.parent().unwrap_or(&config_path).to_path_buf()
            };

            if let Err(e) = watcher.watch(&watch_path, RecursiveMode::Recursive) {
                error!("Failed to watch config path: {}", e);
                return;
            }

            info!("Config watcher started for: {:?}", watch_path);
            let mut last_reload = std::time::Instant::now();
            let debounce_duration = Duration::from_millis(500);

            while !shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
                match timeout(Duration::from_millis(250), notify_rx.recv()).await {
                    Ok(Some(event)) => {
                        if let notify::EventKind::Modify(_) | notify::EventKind::Create(_) =
                            event.kind
                        {
                            if last_reload.elapsed() < debounce_duration {
                                continue;
                            }

                            let paths: Vec<_> = event
                                .paths
                                .iter()
                                .filter(|p| p.extension().map(|e| e == "toml").unwrap_or(false))
                                .collect();

                            if paths.is_empty() {
                                continue;
                            }

                            info!("Config change detected, signaling reload...");
                            let _ = tx.send(()).await;
                            last_reload = std::time::Instant::now();
                        }
                    }
                    Ok(None) => break,
                    Err(_) => continue,
                }
            }

            info!("Config watcher stopped");
        });
    }

    pub fn shutdown_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.shutdown_flag.clone()
    }

    pub fn rule_configs(&self) -> Arc<RwLock<Vec<RuleConfig>>> {
        self.rule_configs.clone()
    }

    pub fn create_rule_manager(&self) -> EngineRuleManager {
        EngineRuleManager {
            rule_configs: self.rule_configs.clone(),
            automations_path: self.automations_path.clone(),
        }
    }
}

pub struct EngineRuleManager {
    rule_configs: Arc<RwLock<Vec<RuleConfig>>>,
    automations_path: Option<PathBuf>,
}

impl EngineRuleManager {
    fn save_to_file(&self) {
        if let Some(ref path) = self.automations_path {
            let configs = self.rule_configs.read().unwrap();
            match serde_json::to_string_pretty(&*configs) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(path, json) {
                        error!("Failed to save automations: {}", e);
                    } else {
                        info!("Saved {} rules to automations.json", configs.len());
                    }
                }
                Err(e) => {
                    error!("Failed to serialize automations: {}", e);
                }
            }
        }
    }
}

impl RuleManager for EngineRuleManager {
    fn get_rules(&self) -> Vec<serde_json::Value> {
        let configs = self.rule_configs.read().unwrap();
        configs
            .iter()
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect()
    }

    fn add_rule(&self, rule: serde_json::Value) -> Result<serde_json::Value, String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        if rule_config.name.is_empty() {
            return Err("Rule name cannot be empty".to_string());
        }

        let mut configs = self.rule_configs.write().unwrap();
        
        if configs.iter().any(|r| r.name == rule_config.name) {
            return Err(format!("Rule '{}' already exists", rule_config.name));
        }

        configs.push(rule_config.clone());
        drop(configs);
        self.save_to_file();

        Ok(serde_json::to_value(rule_config).unwrap())
    }

    fn update_rule(&self, name: &str, rule: serde_json::Value) -> Result<serde_json::Value, String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        let mut configs = self.rule_configs.write().unwrap();
        
        if let Some(existing) = configs.iter_mut().find(|r| r.name == name) {
            *existing = rule_config.clone();
            drop(configs);
            self.save_to_file();
            
            Ok(serde_json::to_value(rule_config).unwrap())
        } else {
            Err(format!("Rule '{}' not found", name))
        }
    }

    fn delete_rule(&self, name: &str) -> Result<(), String> {
        let mut configs = self.rule_configs.write().unwrap();
        let initial_len = configs.len();
        configs.retain(|r| r.name != name);
        
        if configs.len() < initial_len {
            drop(configs);
            self.save_to_file();
            Ok(())
        } else {
            Err(format!("Rule '{}' not found", name))
        }
    }

    fn enable_rule(&self, name: &str, enabled: bool) -> Result<(), String> {
        let mut configs = self.rule_configs.write().unwrap();
        
        if let Some(rule) = configs.iter_mut().find(|r| r.name == name) {
            rule.enabled = enabled;
            drop(configs);
            self.save_to_file();
            Ok(())
        } else {
            Err(format!("Rule '{}' not found", name))
        }
    }

    fn validate_rule(&self, rule: serde_json::Value) -> Result<(), String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        if rule_config.name.is_empty() {
            return Err("Rule name cannot be empty".to_string());
        }

        match &rule_config.trigger {
            TriggerConfig::FileCreated { pattern, .. } |
            TriggerConfig::FileModified { pattern, .. } |
            TriggerConfig::FileDeleted { pattern, .. } => {
                if let Some(p) = pattern {
                    glob::Pattern::new(p)
                        .map_err(|e| format!("Invalid pattern '{}': {}", p, e))?;
                }
            }
            _ => {}
        }

        match &rule_config.action {
            ActionConfig::Execute { command, .. } => {
                if command.is_empty() {
                    return Err("Execute command cannot be empty".to_string());
                }
            }
            ActionConfig::PowerShell { script, .. } => {
                if script.is_empty() {
                    return Err("PowerShell script cannot be empty".to_string());
                }
            }
            ActionConfig::Log { message, .. } => {
                if message.is_empty() {
                    return Err("Log message cannot be empty".to_string());
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn test_rule_match(&self, rule: serde_json::Value, event_json: &str) -> Result<bool, String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        let event: engine_core::event::Event = serde_json::from_str(event_json)
            .map_err(|e| format!("Invalid event JSON: {}", e))?;

        let matcher: Box<dyn RuleMatcher> = match &rule_config.trigger {
            TriggerConfig::FileCreated { pattern, .. } => {
                let mut matcher = FilePatternMatcher::created();
                if let Some(pat) = pattern {
                    matcher = matcher
                        .with_file_pattern(pat)
                        .map_err(|e| format!("Invalid pattern: {}", e))?;
                }
                Box::new(matcher)
            }
            TriggerConfig::FileModified { pattern, .. } => {
                let mut matcher = FilePatternMatcher::modified();
                if let Some(pat) = pattern {
                    matcher = matcher
                        .with_file_pattern(pat)
                        .map_err(|e| format!("Invalid pattern: {}", e))?;
                }
                Box::new(matcher)
            }
            TriggerConfig::FileDeleted { pattern, .. } => {
                let mut matcher = FilePatternMatcher::deleted();
                if let Some(pat) = pattern {
                    matcher = matcher
                        .with_file_pattern(pat)
                        .map_err(|e| format!("Invalid pattern: {}", e))?;
                }
                Box::new(matcher)
            }
            TriggerConfig::WindowFocused {
                title_contains,
                process_name,
            } => Box::new(WindowMatcher {
                event_type: WindowEventType::Focused,
                title_contains: title_contains.clone(),
                process_name: process_name.clone(),
            }),
            TriggerConfig::WindowUnfocused {
                title_contains,
                process_name,
            } => Box::new(WindowMatcher {
                event_type: WindowEventType::Unfocused,
                title_contains: title_contains.clone(),
                process_name: process_name.clone(),
            }),
            TriggerConfig::WindowCreated => Box::new(EventKindMatcher {
                kind: EventKind::WindowCreated {
                    hwnd: 0,
                    title: String::new(),
                    process_id: 0,
                },
            }),
            TriggerConfig::ProcessStarted { process_name: _ } => Box::new(EventKindMatcher {
                kind: EventKind::ProcessStarted {
                    pid: 0,
                    parent_pid: 0,
                    name: String::new(),
                    path: String::new(),
                    command_line: String::new(),
                    session_id: 0,
                    user: String::new(),
                },
            }),
            TriggerConfig::ProcessStopped { process_name: _ } => Box::new(EventKindMatcher {
                kind: EventKind::ProcessStopped {
                    pid: 0,
                    name: String::new(),
                    exit_code: None,
                },
            }),
            TriggerConfig::RegistryChanged { value_name: _ } => Box::new(EventKindMatcher {
                kind: EventKind::RegistryChanged {
                    root: String::new(),
                    key: String::new(),
                    value_name: None,
                    change_type: engine_core::event::RegistryChangeType::Modified,
                },
            }),
            TriggerConfig::Timer { interval_seconds: _ } => Box::new(EventKindMatcher {
                kind: EventKind::TimerTick,
            }),
        };

        Ok(matcher.matches(&event))
    }

    fn export_rules(&self) -> Result<String, String> {
        let rules = self.get_rules();
        serde_json::to_string_pretty(&rules)
            .map_err(|e| format!("Failed to serialize rules: {}", e))
    }

    fn import_rules(&self, content: &str) -> Result<usize, String> {
        let rules: Vec<RuleConfig> = serde_json::from_str(content)
            .map_err(|e| format!("Invalid rules format: {}", e))?;

        let mut count = 0;
        let mut configs = self.rule_configs.write().unwrap();
        for rule in rules {
            if !configs.iter().any(|r| r.name == rule.name) {
                configs.push(rule);
                count += 1;
            }
        }

        Ok(count)
    }
}

#[derive(Debug, Clone)]
pub struct EngineStatus {
    pub active_plugins: usize,
    pub active_rules: usize,
}

#[derive(Debug, Clone)]
pub enum EngineError {
    Config(String),
    PluginInit(String, String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Config(msg) => write!(f, "Configuration error: {}", msg),
            EngineError::PluginInit(name, msg) => {
                write!(f, "Plugin '{}' initialization error: {}", name, msg)
            }
        }
    }
}

impl std::error::Error for EngineError {}
