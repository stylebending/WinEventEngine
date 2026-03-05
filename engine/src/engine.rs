use crate::config::{ActionConfig, Config, RuleConfig, SourceConfig, SourceType, TriggerConfig};
use crate::plugins::file_watcher::FileWatcherPlugin;
use crate::plugins::process_monitor::ProcessMonitorPlugin;
use crate::plugins::registry_monitor::{RegistryMonitorPlugin, RegistryRoot};
use crate::plugins::window_watcher::WindowEventPlugin;
use actions::{Action, ActionExecutor, ExecuteAction, HttpRequestAction, LogAction, LogLevel, MediaKeyAction, PowerShellAction};
use bus::create_event_bus;
use engine_core::event::EventKind;
use engine_core::plugin::EventSourcePlugin;
use metrics::{
    record_event_processing_duration, record_rule_match_duration, MetricsCollector,
};
use rules::{EventKindMatcher, FilePatternMatcher, Rule, RuleMatcher, WindowMatcher, WindowEventType};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{Duration, timeout, Instant};
use tracing::{error, info, warn};

pub struct Engine {
    config: Config,
    config_path: Option<PathBuf>,
    automations_path: Option<PathBuf>,
    plugins: Vec<Box<dyn EventSourcePlugin>>,
    rules: Arc<RwLock<Vec<Rule>>>,
    rule_configs: Arc<RwLock<Vec<RuleConfig>>>,
    action_executor: Arc<RwLock<ActionExecutor>>,
    event_sender: Option<mpsc::Sender<engine_core::event::Event>>,
    shutdown_flag: Arc<std::sync::atomic::AtomicBool>,
    config_reload_rx: Option<mpsc::Receiver<()>>,
    metrics: Arc<MetricsCollector>,
    /// Tracks which source plugin types are currently running
    running_source_types: Arc<RwLock<HashSet<String>>>,
    /// Broadcast channel for real-time event updates
    event_broadcast_tx: tokio::sync::broadcast::Sender<engine_core::event::Event>,
    /// Channel for requesting plugin auto-provisioning
    plugin_request_tx: Option<mpsc::Sender<SourceConfig>>,
    /// Auto-provisioned plugins that need to be stopped on shutdown
    auto_provisioned_plugins: Arc<RwLock<Vec<Box<dyn EventSourcePlugin>>>>,
    /// Task handles for graceful shutdown
    event_processing_handle: Option<tokio::task::JoinHandle<()>>,
    plugin_provisioning_handle: Option<tokio::task::JoinHandle<()>>,
    /// Worker task handles
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
    /// Broadcast channel sender for dispatching events to workers
    worker_tx: Option<tokio::sync::broadcast::Sender<engine_core::event::Event>>,
    /// Number of worker tasks
    num_workers: usize,
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
        // User automations take precedence over config rules
        let mut rule_configs: Vec<RuleConfig> = Vec::new();
        if automations_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&automations_path) {
                if let Ok(loaded) = serde_json::from_str::<Vec<RuleConfig>>(&content) {
                    rule_configs.extend(loaded);
                    info!("Loaded {} rules from automations.json", rule_configs.len());
                }
            }
        }

        // Add config rules that don't conflict with user automations
        for rule in &config.rules {
            if !rule_configs.iter().any(|r| r.name == rule.name) {
                rule_configs.push(rule.clone());
            }
        }
        
        // Create broadcast channel for events (capacity 20 - drops oldest if overwhelmed)
        // Reduced from 100 to 20 to save memory (~80% reduction in buffer memory)
        let (event_broadcast_tx, _) = tokio::sync::broadcast::channel(20);
        
        // Determine worker count based on CPU cores (at least 2, at most 8)
        let num_workers = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 8))
            .unwrap_or(4);
        
        Self {
            config,
            config_path,
            automations_path: Some(automations_path),
            plugins: Vec::new(),
            rules: Arc::new(RwLock::new(Vec::new())),
            rule_configs: Arc::new(RwLock::new(rule_configs)),
            action_executor: Arc::new(RwLock::new(ActionExecutor::new())),
            event_sender: None,
            shutdown_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            config_reload_rx: None,
            metrics,
            running_source_types: Arc::new(RwLock::new(HashSet::new())),
            event_broadcast_tx,
            plugin_request_tx: None,
            auto_provisioned_plugins: Arc::new(RwLock::new(Vec::new())),
            event_processing_handle: None,
            plugin_provisioning_handle: None,
            worker_handles: Vec::new(),
            worker_tx: None,
            num_workers,
        }
    }
    
    /// Get a reference to the metrics collector
    pub fn metrics(&self) -> Arc<MetricsCollector> {
        self.metrics.clone()
    }

    /// Subscribe to real-time event updates
    pub fn subscribe_to_events(&self) -> tokio::sync::broadcast::Receiver<engine_core::event::Event> {
        self.event_broadcast_tx.subscribe()
    }

    /// Set whether HTTP requests are enabled and rebuild rules
    pub async fn set_http_requests_enabled(&mut self, enabled: bool) {
        self.config.engine.http_requests_enabled = enabled;
        self.rebuild_rules_and_actions().await;
        info!("HTTP requests {}", if enabled { "enabled" } else { "disabled" });
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

        // Create channel for plugin auto-provisioning and spawn listener
        let (plugin_request_tx, mut plugin_request_rx) = mpsc::channel::<SourceConfig>(16);
        self.plugin_request_tx = Some(plugin_request_tx);
        
        // Spawn background task that starts plugins on demand
        let event_sender = self.event_sender.clone();
        let running_source_types = self.running_source_types.clone();
        let metrics = self.metrics.clone();
        let auto_provisioned_plugins = self.auto_provisioned_plugins.clone();
        
        let plugin_provisioning_handle = tokio::spawn(async move {
            while let Some(source_config) = plugin_request_rx.recv().await {
                let sender = match &event_sender {
                    Some(s) => s.clone(),
                    None => {
                        error!("Cannot start plugin: no event sender available");
                        continue;
                    }
                };

                let type_name = source_config.source_type.type_name().to_string();
                info!("Auto-provisioning source plugin: {} ({})", source_config.name, type_name);

                match start_plugin(&source_config, sender).await {
                    Ok(plugin) => {
                        info!("Auto-provisioned plugin: {}", source_config.name);
                        running_source_types.write().await.insert(type_name.clone());
                        // Update plugin count gauge
                        let count = running_source_types.read().await.len();
                        metrics.set_active_plugins(count);
                        // Store plugin for proper shutdown (instead of leaking it)
                        auto_provisioned_plugins.write().await.push(plugin);
                    }
                    Err(e) => {
                        error!("Failed to auto-provision plugin {}: {}", source_config.name, e);
                    }
                }
            }
        });

        // Build rules and actions from all rule configs (TOML + automations.json)
        // This will also auto-provision any missing source plugins
        self.rebuild_rules_and_actions().await;

        // Create worker pool for parallel event processing
        // Use broadcast channel so all workers can receive events
        let (worker_tx, _) = tokio::sync::broadcast::channel::<engine_core::event::Event>(self.config.engine.event_buffer_size);
        self.worker_tx = Some(worker_tx.clone());
        
        let rules = self.rules.clone();
        let action_executor = self.action_executor.clone();
        let metrics = self.metrics.clone();
        let event_broadcast_tx = self.event_broadcast_tx.clone();
        let num_workers = self.num_workers;

        // Spawn worker tasks
        let mut worker_handles = Vec::with_capacity(num_workers);
        for worker_id in 0..num_workers {
            let rules = rules.clone();
            let action_executor = action_executor.clone();
            let metrics = metrics.clone();
            let mut worker_rx = worker_tx.subscribe();
            
            let handle = tokio::spawn(async move {
                info!("Event worker {} started", worker_id);
                
                loop {
                    match worker_rx.recv().await {
                        Ok(event) => {
                            let start_time = Instant::now();
                            
                            // Clone data briefly under lock, then release immediately
                            let (rules_snapshot, executor_snapshot) = {
                                let rules_guard = rules.read().await;
                                let executor_guard = action_executor.read().await;
                                (rules_guard.clone(), executor_guard.clone())
                            };
                            
                            // Process event without holding any locks
                            for (idx, rule) in rules_snapshot.iter().enumerate() {
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
                                    
                                    match executor_snapshot.execute(&action_name, &event) {
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
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                    }
                }
                
                info!("Event worker {} stopped", worker_id);
            });
            worker_handles.push(handle);
        }
        self.worker_handles = worker_handles;

        // Start event dispatcher - just forwards events to workers and broadcasts
        let event_broadcast_tx = self.event_broadcast_tx.clone();
        let metrics = self.metrics.clone();
        let worker_tx = worker_tx.clone();
        
        let event_processing_handle = tokio::spawn(async move {
            info!("Event dispatcher started with {} workers", num_workers);

            while let Some(event) = receiver.recv().await {
                let event_source = event.source.clone();
                let event_type = format!("{:?}", event.kind);

                // Broadcast event to subscribers (GUI, etc.) - ignore errors if no subscribers
                let _ = event_broadcast_tx.send(event.clone());

                // Record event received with broadcast
                metrics.record_event_with_broadcast(&event_source, &event_type);

                tracing::debug!("Dispatching event: {:?} from {}", event.kind, event.source);
                
                // Dispatch to workers via broadcast channel (non-blocking)
                let _ = worker_tx.send(event);
            }

            info!("Event dispatcher stopped");
        });

        // Set engine status gauges for dashboard
        let status = self.get_status().await;
        self.metrics.set_engine_status(status.active_plugins, status.active_rules);

        // Store task handles for graceful shutdown
        self.event_processing_handle = Some(event_processing_handle);
        self.plugin_provisioning_handle = Some(plugin_provisioning_handle);

        info!("Engine initialized successfully");
        Ok(())
    }

    async fn initialize_plugins(
        &mut self,
        sender: mpsc::Sender<engine_core::event::Event>,
    ) -> Result<(), EngineError> {
        // Collect enabled sources first to avoid holding lock across await
        let enabled_sources: Vec<_> = self.config.sources
            .iter()
            .filter(|s| s.enabled)
            .cloned()
            .collect();
        
        for source_config in enabled_sources {
            info!("Initializing plugin: {}", source_config.name);

            match self.create_plugin(&source_config, sender.clone()).await {
                Ok(plugin) => {
                    info!("Initialized plugin: {}", source_config.name);
                    // Update tracking - hold lock only for the brief update
                    let mut running = self.running_source_types.write().await;
                    running.insert(source_config.source_type.type_name().to_string());
                    drop(running);
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

    /// Rebuild rules and actions from current rule_configs.
    /// This is called on initialization and when rules are modified via the dashboard.
    async fn rebuild_rules_and_actions(&self) {
        let configs = self.rule_configs.read().await;
        let mut new_rules = Vec::new();
        let mut new_executor = ActionExecutor::new();
        let mut needed_source_types: HashSet<String> = HashSet::new();

        for (idx, rule_config) in configs.iter().enumerate() {
            // Track which source plugins are needed for enabled rules
            if rule_config.enabled {
                if let Some(source_type) = rule_config.trigger.required_source_type() {
                    needed_source_types.insert(source_type.to_string());
                }
            }

            match create_rule(rule_config) {
                Ok(rule) => {
                    info!("Loaded rule: {}", rule.name);
                    new_rules.push(rule);
                }
                Err(e) => {
                    error!("Failed to create rule {}: {}", rule_config.name, e);
                }
            }

            let action_name = format!("rule_{}_action", idx);
            let action = create_action(&rule_config.action, self.config.engine.http_requests_enabled);
            new_executor.register(action_name, action);
        }

        let rule_count = new_rules.len();

        // Write to shared state so the event processing loop picks up changes
        *self.rules.write().await = new_rules;
        *self.action_executor.write().await = new_executor;

        // Auto-provision missing source plugins
        if let Some(ref plugin_tx) = self.plugin_request_tx {
            let running = self.running_source_types.read().await;
            for source_type in &needed_source_types {
                if !running.contains(source_type) {
                    // Find a rule config that needs this source type to build a default config
                    let trigger = configs.iter()
                        .find(|r| r.enabled && r.trigger.required_source_type() == Some(source_type.as_str()))
                        .map(|r| &r.trigger);

                    if let Some(trigger) = trigger {
                        if let Some(source_config) = default_source_config(source_type, trigger) {
                            info!("Requesting auto-provision of source plugin: {}", source_type);
                            if let Err(e) = plugin_tx.try_send(source_config) {
                                error!("Failed to request plugin auto-provision: {}", e);
                            }
                        } else {
                            warn!(
                                "Cannot auto-provision '{}' plugin - requires manual configuration in config.toml",
                                source_type
                            );
                        }
                    }
                }
            }
            drop(running);
        }

        info!("Rules rebuilt: {} active rules", rule_count);
    }

    pub async fn shutdown(&mut self) {
        info!("Shutting down engine");
        
        // 1. Signal shutdown to all tasks
        self.shutdown_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        
        // 2. Drop channels to stop workers and dispatcher first (immediate)
        self.worker_tx = None;
        self.plugin_request_tx = None;
        self.event_sender = None;
        
        // 3. Abort all task handles immediately (don't wait for graceful completion)
        if let Some(handle) = self.event_processing_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.plugin_provisioning_handle.take() {
            handle.abort();
        }
        for handle in self.worker_handles.drain(..) {
            handle.abort();
        }
        
        // 4. Stop explicitly managed plugins with timeout
        for plugin in &mut self.plugins {
            let stop_result = timeout(Duration::from_millis(500), plugin.stop()).await;
            match stop_result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!("Error stopping plugin: {}", e),
                Err(_) => warn!("Plugin stop timed out"),
            }
        }
        
        // 5. Stop auto-provisioned plugins with timeout
        let mut auto_plugins = self.auto_provisioned_plugins.write().await;
        for plugin in auto_plugins.iter_mut() {
            let stop_result = timeout(Duration::from_millis(500), plugin.stop()).await;
            match stop_result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!("Error stopping auto-provisioned plugin: {}", e),
                Err(_) => warn!("Auto-provisioned plugin stop timed out"),
            }
        }
        auto_plugins.clear();
        
        // 6. Stop metrics cleanup
        let _ = timeout(Duration::from_millis(500), self.metrics.stop_cleanup_task()).await;
        
        info!("Engine shutdown complete");
    }

    pub async fn get_status(&self) -> EngineStatus {
        EngineStatus {
            active_plugins: self.plugins.len(),
            active_rules: self.rules.read().await.len(),
        }
    }

    pub async fn reload(&mut self, new_config: Config) -> Result<(), EngineError> {
        info!("Starting full config reload");

        if let Err(e) = new_config.validate() {
            warn!(
                "New configuration validation failed: {}, keeping current config",
                e
            );
            let status = self.get_status().await;
            self.metrics.record_config_reload_with_broadcast(false, status.active_plugins, status.active_rules);
            return Err(EngineError::Config(e.to_string()));
        }

        info!("Stopping all plugins for reload");
        for plugin in &mut self.plugins {
            if let Err(e) = plugin.stop().await {
                error!("Error stopping plugin during reload: {}", e);
            }
        }
        self.plugins.clear();
        self.running_source_types.write().await.clear();

        self.config = new_config;

        // Update rule_configs from new config (preserving automations.json rules)
        {
            let mut configs = self.rule_configs.write().await;
            *configs = self.config.rules.clone();
            // Re-merge automations from file if it exists
            if let Some(ref automations_path) = self.automations_path {
                if automations_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(automations_path) {
                        if let Ok(loaded) = serde_json::from_str::<Vec<RuleConfig>>(&content) {
                            for rule in loaded {
                                if !configs.iter().any(|r| r.name == rule.name) {
                                    configs.push(rule);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Restart plugins with existing event sender
        if let Some(sender) = &self.event_sender {
            self.initialize_plugins(sender.clone()).await?;
        }

        // Rebuild rules and actions — the existing event processing loop
        // reads from the shared Arc<RwLock<>> and will pick these up automatically
        self.rebuild_rules_and_actions().await;

        let status = self.get_status().await;
        self.metrics.record_config_reload_with_broadcast(true, status.active_plugins, status.active_rules);
        self.metrics.set_engine_status(status.active_plugins, status.active_rules);
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

    /// Creates a rule manager for the GUI that shares the engine's plugin request channel.
    /// The background task for auto-provisioning is already spawned in Engine::initialize().
    pub fn create_rule_manager(&self) -> EngineRuleManager {
        // Use the existing plugin request channel from Engine::initialize()
        // If not initialized yet (plugin_request_tx is None), create a temporary channel
        // This shouldn't happen in normal operation since initialize() is called before create_rule_manager()
        let plugin_request_tx = self.plugin_request_tx.clone()
            .expect("Engine must be initialized before creating rule manager");

        EngineRuleManager {
            config: Arc::new(RwLock::new(self.config.clone())),
            config_path: self.config_path.clone(),
            rule_configs: self.rule_configs.clone(),
            rules: self.rules.clone(),
            action_executor: self.action_executor.clone(),
            metrics: self.metrics.clone(),
            automations_path: self.automations_path.clone(),
            running_source_types: self.running_source_types.clone(),
            plugin_request_tx,
            http_requests_enabled: self.config.engine.http_requests_enabled,
        }
    }
}

/// Start a plugin from a SourceConfig. Standalone async function for use
/// by both Engine::create_plugin and the auto-provisioning background task.
async fn start_plugin(
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

/// Create a default SourceConfig for a given source type name.
/// Used when auto-provisioning plugins for rules created via the dashboard.
fn default_source_config(source_type_name: &str, trigger: &TriggerConfig) -> Option<SourceConfig> {
    match source_type_name {
        "window_watcher" => Some(SourceConfig {
            name: "auto_window_watcher".to_string(),
            source_type: SourceType::WindowWatcher {
                title_pattern: None,
                process_pattern: None,
            },
            enabled: true,
        }),
        "process_monitor" => Some(SourceConfig {
            name: "auto_process_monitor".to_string(),
            source_type: SourceType::ProcessMonitor {
                process_name: None,
                monitor_threads: false,
                monitor_files: false,
                monitor_network: false,
            },
            enabled: true,
        }),
        "file_watcher" => {
            let path = match trigger {
                TriggerConfig::FileCreated { path, .. }
                | TriggerConfig::FileModified { path, .. }
                | TriggerConfig::FileDeleted { path, .. } => {
                    path.clone().unwrap_or_else(|| ".".to_string())
                }
                _ => ".".to_string(),
            };
            Some(SourceConfig {
                name: format!("auto_file_watcher_{}", path.replace(['\\', '/', ':'], "_")),
                source_type: SourceType::FileWatcher {
                    paths: vec![PathBuf::from(path)],
                    pattern: None,
                    recursive: false,
                },
                enabled: true,
            })
        }
        // registry_monitor cannot be auto-provisioned without knowing root+key
        _ => None,
    }
}

/// Create a Rule (with matcher) from a RuleConfig.
/// Standalone function so both Engine and EngineRuleManager can use it.
fn create_rule(config: &RuleConfig) -> Result<Rule, EngineError> {
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

/// Create an Action from an ActionConfig.
/// Standalone function so both Engine and EngineRuleManager can use it.
fn create_action(action_config: &ActionConfig, http_requests_enabled: bool) -> Box<dyn Action> {
    match action_config {
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
        ActionConfig::HttpRequest { url, method, headers, body } => {
            // Security check: HTTP requests must be explicitly enabled
            if !http_requests_enabled {
                warn!("HTTP request blocked: http_requests_enabled is false in engine config");
                return Box::new(LogAction::new(
                    format!("HTTP request to {} blocked - enable http_requests_enabled in settings", url)
                ));
            }
            
            // Create HTTP request action with all parameters
            let mut action = HttpRequestAction::new(url)
                .with_method(method.clone());
            
            if !headers.is_empty() {
                action = action.with_headers(headers.clone());
            }
            
            if let Some(body_content) = body {
                action = action.with_body(body_content.clone());
            }
            
            Box::new(action)
        }
        ActionConfig::Media { command } => {
            // Use direct Windows API for instant media key execution
            Box::new(MediaKeyAction::new(command))
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
    }
}

impl Engine {
    /// Install the engine as a Windows service
    pub fn install_service(&self) -> Result<(), crate::service::ServiceError> {
        use crate::service::ServiceManagerHandle;
        
        let manager = ServiceManagerHandle::new()?;
        let exe_path = std::env::current_exe()
            .map_err(|e| crate::service::ServiceError::Config(format!("Failed to get executable path: {}", e)))?;
        let exe_path_str = exe_path.to_string_lossy().to_string();
        
        manager.install(&exe_path_str)
    }

    /// Uninstall the engine Windows service
    pub fn uninstall_service(&self) -> Result<(), crate::service::ServiceError> {
        use crate::service::ServiceManagerHandle;
        
        let manager = ServiceManagerHandle::new()?;
        manager.uninstall()
    }

    /// Check if the service is installed
    pub fn is_service_installed(&self) -> bool {
        use crate::service::ServiceManagerHandle;
        
        match ServiceManagerHandle::new() {
            Ok(manager) => manager.is_installed(),
            Err(_) => false,
        }
    }

    /// Check if the service is set to auto-start
    pub fn is_service_auto_start(&self) -> bool {
        use crate::service::ServiceManagerHandle;
        
        match ServiceManagerHandle::new() {
            Ok(manager) => manager.is_auto_start().unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Set the service auto-start status
    pub fn set_service_auto_start(&self, auto_start: bool) -> Result<(), crate::service::ServiceError> {
        use crate::service::ServiceManagerHandle;
        
        let manager = ServiceManagerHandle::new()?;
        manager.set_auto_start(auto_start)
    }
}

pub struct EngineRuleManager {
    config: Arc<RwLock<Config>>,
    config_path: Option<PathBuf>,
    rule_configs: Arc<RwLock<Vec<RuleConfig>>>,
    rules: Arc<RwLock<Vec<Rule>>>,
    action_executor: Arc<RwLock<ActionExecutor>>,
    metrics: Arc<MetricsCollector>,
    automations_path: Option<PathBuf>,
    running_source_types: Arc<RwLock<HashSet<String>>>,
    plugin_request_tx: mpsc::Sender<SourceConfig>,
    http_requests_enabled: bool,
}

impl EngineRuleManager {
    pub async fn save_to_file(&self) -> Result<(), String> {
        if let Some(ref path) = self.automations_path {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(format!("Failed to create directory {:?}: {}", parent, e));
                }
            }

            let configs = self.rule_configs.read().await;
            match serde_json::to_string_pretty(&*configs) {
                Ok(json) => {
                    match std::fs::write(path, json) {
                        Ok(_) => {
                            info!("Saved {} rules to automations.json", configs.len());
                            Ok(())
                        }
                        Err(e) => Err(format!("Failed to write automations file: {}", e)),
                    }
                }
                Err(e) => Err(format!("Failed to serialize automations: {}", e)),
            }
        } else {
            Err("No automations path configured".to_string())
        }
    }

    /// Rebuild the active rules and actions from the current rule_configs.
    /// Called after any CRUD operation to make changes take effect immediately.
    /// Also auto-provisions any source plugins required by the rules.
    async fn rebuild_active_rules(&self) {
        let configs = self.rule_configs.read().await;
        let mut new_rules = Vec::new();
        let mut new_executor = ActionExecutor::new();
        let mut needed_source_types: HashSet<String> = HashSet::new();

        for (idx, rule_config) in configs.iter().enumerate() {
            match create_rule(rule_config) {
                Ok(rule) => {
                    info!("Loaded rule: {}", rule.name);
                    new_rules.push(rule);
                }
                Err(e) => {
                    error!("Failed to create rule {}: {}", rule_config.name, e);
                }
            }

            let action_name = format!("rule_{}_action", idx);
            let action = create_action(&rule_config.action, self.http_requests_enabled);
            new_executor.register(action_name, action);

            // Track which source plugin types are needed
            if rule_config.enabled {
                if let Some(source_type) = rule_config.trigger.required_source_type() {
                    needed_source_types.insert(source_type.to_string());
                }
            }
        }

        let rule_count = new_rules.len();

        *self.rules.write().await = new_rules;
        *self.action_executor.write().await = new_executor;

        // Only update the rules gauge — don't touch active_plugins
        self.metrics.set_active_rules(rule_count);

        // Auto-provision missing source plugins
        let running = self.running_source_types.read().await;
        for source_type in &needed_source_types {
            if !running.contains(source_type) {
                // Find a rule config that needs this source type to build a default config
                let trigger = configs.iter()
                    .find(|r| r.enabled && r.trigger.required_source_type() == Some(source_type.as_str()))
                    .map(|r| &r.trigger);

                if let Some(trigger) = trigger {
                    if let Some(source_config) = default_source_config(source_type, trigger) {
                        info!("Requesting auto-provision of source plugin: {}", source_type);
                        if let Err(e) = self.plugin_request_tx.try_send(source_config) {
                            error!("Failed to request plugin auto-provision: {}", e);
                        }
                    } else {
                        warn!(
                            "Cannot auto-provision '{}' plugin — requires manual configuration in config.toml",
                            source_type
                        );
                    }
                }
            }
        }
        drop(running);

        info!("Rules hot-reloaded: {} active rules", rule_count);
    }
}

impl EngineRuleManager {
    pub async fn get_rules(&self) -> Vec<serde_json::Value> {
        let configs = self.rule_configs.read().await;
        configs
            .iter()
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect()
    }

    pub async fn add_rule(&self, rule: serde_json::Value) -> Result<serde_json::Value, String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        if rule_config.name.is_empty() {
            return Err("Rule name cannot be empty".to_string());
        }

        let mut configs = self.rule_configs.write().await;
        
        if configs.iter().any(|r| r.name == rule_config.name) {
            return Err(format!("Rule '{}' already exists", rule_config.name));
        }

        configs.push(rule_config.clone());
        drop(configs);
        self.save_to_file().await?;
        self.rebuild_active_rules().await;

        Ok(serde_json::to_value(rule_config).unwrap())
    }

    pub async fn update_rule(&self, name: &str, rule: serde_json::Value) -> Result<serde_json::Value, String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        let mut configs = self.rule_configs.write().await;
        
        if let Some(existing) = configs.iter_mut().find(|r| r.name == name) {
            *existing = rule_config.clone();
            drop(configs);
            self.save_to_file().await?;
            self.rebuild_active_rules().await;

            Ok(serde_json::to_value(rule_config).unwrap())
        } else {
            Err(format!("Rule '{}' not found", name))
        }
    }

    pub async fn delete_rule(&self, name: &str) -> Result<(), String> {
        let mut configs = self.rule_configs.write().await;
        let initial_len = configs.len();
        configs.retain(|r| r.name != name);

        if configs.len() < initial_len {
            drop(configs);
            self.save_to_file().await?;
            self.rebuild_active_rules().await;
            Ok(())
        } else {
            Err(format!("Rule '{}' not found", name))
        }
    }

    pub async fn enable_rule(&self, name: &str, enabled: bool) -> Result<(), String> {
        let mut configs = self.rule_configs.write().await;

        if let Some(rule) = configs.iter_mut().find(|r| r.name == name) {
            rule.enabled = enabled;
            drop(configs);
            self.save_to_file().await?;
            self.rebuild_active_rules().await;
            Ok(())
        } else {
            Err(format!("Rule '{}' not found", name))
        }
    }

    pub async fn validate_rule(&self, rule: serde_json::Value) -> Result<(), String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        if rule_config.name.is_empty() {
            return Err("Rule name cannot be empty".to_string());
        }

        match &rule_config.trigger {
            TriggerConfig::FileCreated { pattern, .. }
            | TriggerConfig::FileModified { pattern, .. }
            | TriggerConfig::FileDeleted { pattern, .. } => {
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

    pub async fn test_rule_match(&self, rule: serde_json::Value, event_json: &str) -> Result<bool, String> {
        let rule_config: RuleConfig = serde_json::from_value(rule)
            .map_err(|e| format!("Invalid rule format: {}", e))?;

        let event: engine_core::event::Event = serde_json::from_str(event_json)
            .map_err(|e| format!("Invalid event JSON: {}", e))?;

        let rule = create_rule(&rule_config)
            .map_err(|e| format!("Failed to create rule: {}", e))?;

        Ok(rule.matches(&event))
    }

    pub async fn export_rules(&self) -> Result<String, String> {
        let rules = self.get_rules().await;
        serde_json::to_string_pretty(&rules)
            .map_err(|e| format!("Failed to serialize rules: {}", e))
    }

    pub async fn import_rules(&self, content: &str) -> Result<usize, String> {
        let rules: Vec<RuleConfig> = serde_json::from_str(content)
            .map_err(|e| format!("Invalid rules format: {}", e))?;

        let mut count = 0;
        let mut configs = self.rule_configs.write().await;
        for rule in rules {
            if !configs.iter().any(|r| r.name == rule.name) {
                configs.push(rule);
                count += 1;
            }
        }
        drop(configs);

        if count > 0 {
            self.save_to_file().await?;
            self.rebuild_active_rules().await;
        }

        Ok(count)
    }

    // Source management methods
    pub async fn get_sources(&self) -> Vec<serde_json::Value> {
        let config = self.config.read().await;
        config
            .sources
            .iter()
            .filter_map(|s| serde_json::to_value(s).ok())
            .collect()
    }

    pub async fn add_source(&self, source: serde_json::Value) -> Result<serde_json::Value, String> {
        let source_config: SourceConfig = serde_json::from_value(source)
            .map_err(|e| format!("Invalid source format: {}", e))?;

        if source_config.name.is_empty() {
            return Err("Source name cannot be empty".to_string());
        }

        let mut config = self.config.write().await;

        if config.sources.iter().any(|s| s.name == source_config.name) {
            return Err(format!("Source '{}' already exists", source_config.name));
        }

        // Start the plugin if enabled
        if source_config.enabled {
            let type_name = source_config.source_type.type_name().to_string();
            if let Err(e) = self.plugin_request_tx.try_send(source_config.clone()) {
                error!("Failed to request plugin start: {}", e);
            } else {
                self.running_source_types.write().await.insert(type_name);
            }
        }

        config.sources.push(source_config.clone());
        drop(config);

        self.save_config().await;

        Ok(serde_json::to_value(source_config).unwrap())
    }

    pub async fn delete_source(&self, name: &str) -> Result<(), String> {
        let mut config = self.config.write().await;
        let initial_len = config.sources.len();
        config.sources.retain(|s| s.name != name);

        if config.sources.len() < initial_len {
            drop(config);
            self.save_config().await;
            Ok(())
        } else {
            Err(format!("Source '{}' not found", name))
        }
    }

    pub async fn enable_source(&self, name: &str, enabled: bool) -> Result<(), String> {
        let mut config = self.config.write().await;

        if let Some(source) = config.sources.iter_mut().find(|s| s.name == name) {
            source.enabled = enabled;

            // Start/stop plugin based on enabled state
            if enabled {
                let type_name = source.source_type.type_name().to_string();
                if let Err(e) = self.plugin_request_tx.try_send(source.clone()) {
                    error!("Failed to request plugin start: {}", e);
                } else {
                    self.running_source_types.write().await.insert(type_name);
                }
            }

            drop(config);
            self.save_config().await;
            Ok(())
        } else {
            Err(format!("Source '{}' not found", name))
        }
    }

    async fn save_config(&self) {
        if let Some(ref path) = self.config_path {
            let config = self.config.read().await;
            if let Err(e) = config.save_to_file(path) {
                error!("Failed to save config: {}", e);
            } else {
                info!("Saved config to {:?}", path);
            }
        }
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
