use iced::widget::{button, column, container, row, text, Space};
use iced::{Element, Length, Theme, Alignment};
use std::sync::Arc;
use tokio::sync::Mutex;
use win_event_engine::{Engine, EngineStatus, RuleConfig};
use win_event_engine::config::Config;

use crate::theme::AppTheme;
use crate::views::{dashboard, rules, settings, sources, tester};

pub struct WinEventApp {
    // Engine - public so views can access
    pub engine: Option<Arc<Mutex<Engine>>>,
    pub engine_status: EngineStatus,
    pub engine_running: bool,
    
    // Navigation - public
    pub current_view: View,
    
    // Data - public
    pub events: Vec<EventDisplay>,
    pub rules: Vec<RuleConfig>,
    pub sources: Vec<serde_json::Value>,

    // UI State - public
    pub theme: AppTheme,
    pub notification: Option<(String, NotificationType)>,
    pub is_loading: bool,
    
    // Rule editor state
    pub editing_rule: Option<String>,
    pub rule_editor_open: bool,
    
    // Rule editor form fields
    pub rule_name: String,
    pub rule_description: String,
    pub rule_enabled: bool,
    pub trigger_type: String,
    pub trigger_path: String,
    pub trigger_pattern: String,
    pub trigger_title_contains: String,
    pub trigger_process_name: String,
    pub trigger_interval: String,
    pub action_type: String,
    pub action_command: String,
    pub action_args: String,
    pub action_script: String,
    pub action_message: String,
    pub action_log_level: String,
    pub action_media_command: String,
    
    // Event tester state
    pub test_rule_name: String,
    pub test_event_content: iced::widget::text_editor::Content,
    pub test_result: Option<(bool, String)>, // (matched, details)
    
    // Service status
    pub service_installed: bool,
    pub service_auto_start: bool,
    
    // Dashboard metrics
    pub metrics_events_total: u64,
    pub metrics_rules_matched: u64,
    pub metrics_actions_executed: u64,
    pub metrics_last_update: std::time::Instant,
    
    // Event subscription for real-time updates
    pub event_receiver: Option<tokio::sync::broadcast::Receiver<engine_core::event::Event>>,
    
    // Security settings
    pub http_requests_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Dashboard,
    Rules,
    Sources,
    EventTester,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationType {
    Success,
    Error,
    Info,
}

#[derive(Debug, Clone)]
pub struct EventDisplay {
    pub timestamp: String,
    pub source: String,
    pub event_type: String,
    pub details: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Navigation
    NavigateTo(View),
    
    // Theme
    ThemeChanged(AppTheme),
    
    // Engine operations
    StartEngine,
    StopEngine,
    EngineStarted(Result<(), String>),
    EngineStopped,
    
    // Data updates
    RefreshData,
    RulesUpdated(Vec<serde_json::Value>),
    NotificationShow(String, NotificationType),

    // Rule management
    CreateRule,
    EditRule(String),
    DeleteRule(String),
    SaveRule(RuleConfig),
    CancelEdit,
    ToggleRule(String, bool),

    // Source management
    RefreshSources,
    SourcesUpdated(Vec<serde_json::Value>),
    DeleteSource(String),
    ToggleSource(String, bool),

    // Rule editor form updates
    RuleNameChanged(String),
    RuleDescriptionChanged(String),
    RuleEnabledChanged(bool),
    TriggerTypeChanged(String),
    TriggerPathChanged(String),
    TriggerPatternChanged(String),
    TriggerTitleContainsChanged(String),
    TriggerProcessNameChanged(String),
    TriggerIntervalChanged(String),
    ActionTypeChanged(String),
    ActionCommandChanged(String),
    ActionArgsChanged(String),
    ActionScriptChanged(String),
    ActionMessageChanged(String),
    ActionLogLevelChanged(String),
    ActionMediaCommandChanged(String),
    
    // Import/Export
    ImportRules,
    ImportRulesFileSelected(std::path::PathBuf),
    ExportRules,
    ExportRulesFileSelected(std::path::PathBuf),
    
    // Event Tester
    TestRuleChanged(String),
    TestEventJsonAction(iced::widget::text_editor::Action),
    RunEventTest,
    EventTestResult(bool, String),
    
    // Settings
    ReloadConfig,
    ConfigReloaded(Result<(), String>),
    InstallService,
    UninstallService,
    CheckServiceStatus,
    ServiceStatusChecked(bool),
    CheckAutoStartStatus,
    AutoStartStatusChecked(bool),
    ToggleAutoStart(bool),
    ToggleHttpRequests(bool),
    
    // Notifications
    DismissNotification,
    NotificationShowTimed(String, NotificationType, u64), // message, type, seconds
    
    // Batch multiple messages
    Batch(Vec<Message>),
    
    // Window
    CloseRequested,
    WindowClosed,
    
    // Dashboard metrics
    RefreshMetrics,
    MetricsUpdated(u64, u64, u64), // (events_total, rules_matched, actions_executed)
    Tick,
    
    // Engine status
    RefreshEngineStatus,
    EngineStatusUpdated(EngineStatus),
    
    // Events
    EventReceived(engine_core::event::Event),
}

impl WinEventApp {
    pub fn new() -> (Self, iced::Task<Message>) {
        // Determine config directory
        let config_dir = dirs::config_dir()
            .map(|d| d.join("WinEventEngine"))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("config"));

        let config_path = config_dir.join("config.toml");

        // Load existing config or use empty default
        let config = if config_path.exists() {
            match Config::load_from_file(&config_path) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("Failed to load config from {:?}: {}", config_path, e);
                    Config::default()
                }
            }
        } else {
            Config::default()
        };

        let engine = Arc::new(Mutex::new(Engine::new(config, Some(config_path))));
        let engine_clone = engine.clone();
        
        let app = Self {
            engine: Some(engine),
            engine_status: EngineStatus {
                active_plugins: 0,
                active_rules: 0,
            },
            engine_running: false,
            current_view: View::Dashboard,
            events: Vec::new(),
            rules: Vec::new(),
            sources: Vec::new(),
            theme: AppTheme::Dark,
            notification: None,
            is_loading: true,
            editing_rule: None,
            rule_editor_open: false,
            // Rule editor form defaults
            rule_name: String::new(),
            rule_description: String::new(),
            rule_enabled: true,
            trigger_type: "window_focused".to_string(),
            trigger_path: String::new(),
            trigger_pattern: String::new(),
            trigger_title_contains: String::new(),
            trigger_process_name: String::new(),
            trigger_interval: "60".to_string(),
            action_type: "media".to_string(),
            action_command: String::new(),
            action_args: String::new(),
            action_script: String::new(),
            action_message: "Event triggered".to_string(),
            action_log_level: "info".to_string(),
            action_media_command: "play_pause".to_string(),
            // Event tester defaults
            test_rule_name: String::new(),
            test_event_content: iced::widget::text_editor::Content::with_text(r#"{
  "id": "00000000-0000-0000-0000-000000000000",
  "timestamp": 1704067200000000,
  "kind": {
    "WindowFocused": {
      "hwnd": 12345,
      "title": "Example Window"
    }
  },
  "source": "window_watcher",
  "metadata": {}
}"#),
            test_result: None,
            service_installed: false,
            service_auto_start: false,
            metrics_events_total: 0,
            metrics_rules_matched: 0,
            metrics_actions_executed: 0,
            metrics_last_update: std::time::Instant::now(),
            event_receiver: None,
            http_requests_enabled: false, // Security: disabled by default
        };
        
        // Initialize engine asynchronously
        let init_task = iced::Task::perform(
            async move {
                let mut eng = engine_clone.lock().await;
                match eng.initialize().await {
                    Ok(_) => Ok(()),
                    Err(e) => Err(e.to_string()),
                }
            },
            Message::EngineStarted,
        );
        
        (app, init_task)
    }
    
    pub fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::NavigateTo(view) => {
                self.current_view = view;
                // Refresh data when navigating to Rules view
                if view == View::Rules {
                    return self.update(Message::RefreshData);
                }
                // Refresh sources when navigating to Sources view
                if view == View::Sources {
                    return self.update(Message::RefreshSources);
                }
                // Check service status and auto-start when navigating to Settings view
                if view == View::Settings {
                    return iced::Task::batch(vec![
                        self.update(Message::CheckServiceStatus),
                        self.update(Message::CheckAutoStartStatus),
                    ]);
                }
                iced::Task::none()
            }
            
            Message::ThemeChanged(theme) => {
                self.theme = theme;
                iced::Task::none()
            }
            
            Message::StartEngine => {
                if self.engine.is_none() {
                    let config = win_event_engine::create_demo_config();
                    let engine = Arc::new(Mutex::new(Engine::new(config, None)));
                    let engine_clone = engine.clone();
                    self.engine = Some(engine);
                    
                    iced::Task::perform(
                        async move {
                            let mut eng = engine_clone.lock().await;
                            match eng.initialize().await {
                                Ok(_) => Ok(()),
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        Message::EngineStarted,
                    )
                } else {
                    iced::Task::none()
                }
            }
            
            Message::EngineStarted(result) => {
                self.is_loading = false;
                match result {
                    Ok(_) => {
                        self.engine_running = true;
                        // Subscribe to real-time events
                        if let Some(engine) = &self.engine {
                            let engine_clone = engine.clone();
                            let event_rx = tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(async {
                                    let eng = engine_clone.lock().await;
                                    eng.subscribe_to_events()
                                })
                            });
                            self.event_receiver = Some(event_rx);
                        }
                        
                        // Show notification and load data
                        return self.update(Message::Batch(vec![
                            Message::NotificationShowTimed(
                                "Engine started successfully".to_string(),
                                NotificationType::Success,
                                3,
                            ),
                            Message::RefreshData,
                            Message::RefreshEngineStatus,
                            Message::RefreshMetrics,
                        ]));
                    }
                    Err(e) => {
                        self.engine = None;
                        return self.update(Message::NotificationShowTimed(
                            format!("Failed to start engine: {}", e),
                            NotificationType::Error,
                            5,
                        ));
                    }
                }
            }
            
            Message::StopEngine => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    iced::Task::perform(
                        async move {
                            // Take ownership of engine from the Arc<Mutex> and shutdown
                            // Use block_in_place to avoid Send requirement on the guard
                            tokio::task::block_in_place(|| {
                                let rt = tokio::runtime::Handle::current();
                                rt.block_on(async move {
                                    let mut eng = engine_clone.lock().await;
                                    eng.shutdown().await;
                                });
                            });
                        },
                        |_| Message::EngineStopped,
                    )
                } else {
                    iced::Task::none()
                }
            }
            
            Message::EngineStopped => {
                self.engine = None;
                self.engine_running = false;
                self.engine_status = EngineStatus {
                    active_plugins: 0,
                    active_rules: 0,
                };
                self.event_receiver = None; // Clear event receiver
                return self.update(Message::NotificationShowTimed(
                    "Engine stopped".to_string(),
                    NotificationType::Info,
                    3,
                ));
            }

            Message::CreateRule => {
                self.editing_rule = None;
                self.rule_editor_open = true;
                // Reset form to defaults
                self.rule_name = String::new();
                self.rule_description = String::new();
                self.rule_enabled = true;
                self.trigger_type = "window_focused".to_string();
                self.trigger_path = String::new();
                self.trigger_pattern = String::new();
                self.trigger_title_contains = String::new();
                self.trigger_process_name = String::new();
                self.trigger_interval = "60".to_string();
                self.action_type = "media".to_string();
                self.action_command = String::new();
                self.action_args = String::new();
                self.action_script = String::new();
                self.action_message = "Event triggered".to_string();
                self.action_log_level = "info".to_string();
                self.action_media_command = "play_pause".to_string();
                iced::Task::none()
            }

            Message::EditRule(name) => {
                self.editing_rule = Some(name.clone());
                self.rule_editor_open = true;
                // Load existing rule data into form
                if let Some(rule) = self.rules.iter().find(|r| r.name == name) {
                    self.rule_name = rule.name.clone();
                    self.rule_description = rule.description.clone().unwrap_or_default();
                    self.rule_enabled = rule.enabled;
                    // Set trigger fields based on trigger type
                    match &rule.trigger {
                        win_event_engine::TriggerConfig::FileCreated { path, pattern } |
                        win_event_engine::TriggerConfig::FileModified { path, pattern } |
                        win_event_engine::TriggerConfig::FileDeleted { path, pattern } => {
                            self.trigger_type = match rule.trigger {
                                win_event_engine::TriggerConfig::FileCreated { .. } => "file_created",
                                win_event_engine::TriggerConfig::FileModified { .. } => "file_modified",
                                win_event_engine::TriggerConfig::FileDeleted { .. } => "file_deleted",
                                _ => "file_created",
                            }.to_string();
                            self.trigger_path = path.clone().unwrap_or_default();
                            self.trigger_pattern = pattern.clone().unwrap_or_default();
                        }
                        win_event_engine::TriggerConfig::WindowFocused { title_contains, process_name } |
                        win_event_engine::TriggerConfig::WindowUnfocused { title_contains, process_name } => {
                            self.trigger_type = match rule.trigger {
                                win_event_engine::TriggerConfig::WindowFocused { .. } => "window_focused",
                                win_event_engine::TriggerConfig::WindowUnfocused { .. } => "window_unfocused",
                                _ => "window_focused",
                            }.to_string();
                            self.trigger_title_contains = title_contains.clone().unwrap_or_default();
                            self.trigger_process_name = process_name.clone().unwrap_or_default();
                        }
                        win_event_engine::TriggerConfig::WindowCreated => {
                            self.trigger_type = "window_created".to_string();
                        }
                        win_event_engine::TriggerConfig::ProcessStarted { process_name } |
                        win_event_engine::TriggerConfig::ProcessStopped { process_name } => {
                            self.trigger_type = match rule.trigger {
                                win_event_engine::TriggerConfig::ProcessStarted { .. } => "process_started",
                                win_event_engine::TriggerConfig::ProcessStopped { .. } => "process_stopped",
                                _ => "process_started",
                            }.to_string();
                            self.trigger_process_name = process_name.clone().unwrap_or_default();
                        }
                        win_event_engine::TriggerConfig::Timer { interval_seconds } => {
                            self.trigger_type = "timer".to_string();
                            self.trigger_interval = interval_seconds.to_string();
                        }
                        _ => {}
                    }
                    // Set action fields based on action type
                    match &rule.action {
                        win_event_engine::ActionConfig::Log { message, level } => {
                            self.action_type = "log".to_string();
                            self.action_message = message.clone();
                            self.action_log_level = level.clone();
                        }
                        win_event_engine::ActionConfig::Execute { command, args, .. } => {
                            self.action_type = "execute".to_string();
                            self.action_command = command.clone();
                            self.action_args = args.join(" ");
                        }
                        win_event_engine::ActionConfig::PowerShell { script, .. } => {
                            self.action_type = "powershell".to_string();
                            self.action_script = script.clone();
                        }
                        win_event_engine::ActionConfig::Media { command } => {
                            self.action_type = "media".to_string();
                            self.action_media_command = command.clone();
                        }
                        _ => {}
                    }
                }
                iced::Task::none()
            }

            Message::DeleteRule(name) => {
                // Implement rule deletion
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    let name_for_async = name.clone();
                    let name_for_result = name.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            manager.delete_rule(&name_for_async).await
                        },
                        move |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::RefreshData,
                                Message::NotificationShowTimed(
                                    format!("Rule '{}' deleted", name_for_result),
                                    NotificationType::Success,
                                    2,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to delete rule: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::CancelEdit => {
                self.rule_editor_open = false;
                self.editing_rule = None;
                iced::Task::none()
            }
            
            Message::SaveRule(_rule) => {
                // Build rule from form data
                let trigger = match self.trigger_type.as_str() {
                    "file_created" => win_event_engine::TriggerConfig::FileCreated {
                        path: if self.trigger_path.is_empty() { None } else { Some(self.trigger_path.clone()) },
                        pattern: if self.trigger_pattern.is_empty() { None } else { Some(self.trigger_pattern.clone()) },
                    },
                    "file_modified" => win_event_engine::TriggerConfig::FileModified {
                        path: if self.trigger_path.is_empty() { None } else { Some(self.trigger_path.clone()) },
                        pattern: if self.trigger_pattern.is_empty() { None } else { Some(self.trigger_pattern.clone()) },
                    },
                    "file_deleted" => win_event_engine::TriggerConfig::FileDeleted {
                        path: if self.trigger_path.is_empty() { None } else { Some(self.trigger_path.clone()) },
                        pattern: if self.trigger_pattern.is_empty() { None } else { Some(self.trigger_pattern.clone()) },
                    },
                    "window_focused" => win_event_engine::TriggerConfig::WindowFocused {
                        title_contains: if self.trigger_title_contains.is_empty() { None } else { Some(self.trigger_title_contains.clone()) },
                        process_name: if self.trigger_process_name.is_empty() { None } else { Some(self.trigger_process_name.clone()) },
                    },
                    "window_unfocused" => win_event_engine::TriggerConfig::WindowUnfocused {
                        title_contains: if self.trigger_title_contains.is_empty() { None } else { Some(self.trigger_title_contains.clone()) },
                        process_name: if self.trigger_process_name.is_empty() { None } else { Some(self.trigger_process_name.clone()) },
                    },
                    "window_created" => win_event_engine::TriggerConfig::WindowCreated,
                    "process_started" => win_event_engine::TriggerConfig::ProcessStarted {
                        process_name: if self.trigger_process_name.is_empty() { None } else { Some(self.trigger_process_name.clone()) },
                    },
                    "process_stopped" => win_event_engine::TriggerConfig::ProcessStopped {
                        process_name: if self.trigger_process_name.is_empty() { None } else { Some(self.trigger_process_name.clone()) },
                    },
                    "timer" => win_event_engine::TriggerConfig::Timer {
                        interval_seconds: self.trigger_interval.parse().unwrap_or(60),
                    },
                    _ => win_event_engine::TriggerConfig::FileCreated { path: None, pattern: None },
                };

                let action = match self.action_type.as_str() {
                    "log" => win_event_engine::ActionConfig::Log {
                        message: self.action_message.clone(),
                        level: self.action_log_level.clone(),
                    },
                    "execute" => win_event_engine::ActionConfig::Execute {
                        command: self.action_command.clone(),
                        args: if self.action_args.is_empty() { vec![] } else { self.action_args.split_whitespace().map(String::from).collect() },
                        working_dir: None,
                    },
                    "powershell" => win_event_engine::ActionConfig::PowerShell {
                        script: self.action_script.clone(),
                        working_dir: None,
                    },
                    "media" => win_event_engine::ActionConfig::Media {
                        command: self.action_media_command.clone(),
                    },
                    _ => win_event_engine::ActionConfig::Log { message: "Event triggered".to_string(), level: "info".to_string() },
                };

                let rule = win_event_engine::RuleConfig {
                    name: self.rule_name.clone(),
                    description: if self.rule_description.is_empty() { None } else { Some(self.rule_description.clone()) },
                    trigger,
                    action,
                    enabled: self.rule_enabled,
                };

                // Save rule using RuleManager
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    let rule_json = serde_json::to_value(&rule).unwrap();
                    let is_edit = self.editing_rule.is_some();
                    // Use original name for editing, current name for new rules
                    let lookup_name = self.editing_rule.clone().unwrap_or_else(|| rule.name.clone());
                    let rule_name_for_notification = rule.name.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            if is_edit {
                                // Update existing rule using ORIGINAL name to find it
                                manager.update_rule(&lookup_name, rule_json).await
                            } else {
                                // Add new rule
                                manager.add_rule(rule_json).await
                            }
                        },
                        move |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::RefreshData,
                                Message::CancelEdit, // Close the editor
                                Message::NavigateTo(View::Rules), // Go back to rules view
                                Message::NotificationShowTimed(
                                    format!("Rule '{}' saved successfully", rule_name_for_notification),
                                    NotificationType::Success,
                                    3, // Show for 3 seconds
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to save rule: {}", e),
                                NotificationType::Error,
                                5, // Show for 5 seconds
                            ),
                        },
                    );
                }

                iced::Task::none()
            }

            Message::ToggleRule(name, enabled) => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    let name_for_async = name.clone();
                    let name_for_result = name.clone();
                    let status = if enabled { "enabled" } else { "disabled" };
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            manager.enable_rule(&name_for_async, enabled).await
                        },
                        move |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::RefreshData,
                                Message::NotificationShowTimed(
                                    format!("Rule '{}' {}", name_for_result, status),
                                    NotificationType::Success,
                                    2,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to toggle rule: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::DismissNotification => {
                self.notification = None;
                iced::Task::none()
            }

            Message::RefreshData => {
                // Refresh rules from engine
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            manager.get_rules().await
                        },
                        |rules| Message::RulesUpdated(rules),
                    );
                }
                iced::Task::none()
            }

            Message::RulesUpdated(rules_json) => {
                self.rules = rules_json.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect();
                iced::Task::none()
            }

            Message::RefreshSources => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            manager.get_sources().await
                        },
                        |sources| Message::SourcesUpdated(sources),
                    );
                }
                iced::Task::none()
            }

            Message::SourcesUpdated(sources) => {
                self.sources = sources;
                iced::Task::none()
            }

            Message::DeleteSource(name) => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    let name_clone = name.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            manager.delete_source(&name_clone).await
                        },
                        move |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::RefreshSources,
                                Message::NotificationShowTimed(
                                    format!("Source '{}' deleted", name),
                                    NotificationType::Success,
                                    3,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to delete source: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::ToggleSource(name, enabled) => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    let name_clone = name.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            manager.enable_source(&name_clone, enabled).await
                        },
                        move |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::RefreshSources,
                                Message::NotificationShowTimed(
                                    format!(
                                        "Source '{}' {}",
                                        name,
                                        if enabled { "enabled" } else { "disabled" }
                                    ),
                                    NotificationType::Success,
                                    3,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to toggle source: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::NotificationShow(msg, notification_type) => {
                self.notification = Some((msg, notification_type));
                iced::Task::none()
            }

            // Form update handlers
            Message::RuleNameChanged(value) => { self.rule_name = value; iced::Task::none() }
            Message::RuleDescriptionChanged(value) => { self.rule_description = value; iced::Task::none() }
            Message::RuleEnabledChanged(value) => { self.rule_enabled = value; iced::Task::none() }
            Message::TriggerTypeChanged(value) => { self.trigger_type = value; iced::Task::none() }
            Message::TriggerPathChanged(value) => { self.trigger_path = value; iced::Task::none() }
            Message::TriggerPatternChanged(value) => { self.trigger_pattern = value; iced::Task::none() }
            Message::TriggerTitleContainsChanged(value) => { self.trigger_title_contains = value; iced::Task::none() }
            Message::TriggerProcessNameChanged(value) => { self.trigger_process_name = value; iced::Task::none() }
            Message::TriggerIntervalChanged(value) => { self.trigger_interval = value; iced::Task::none() }
            Message::ActionTypeChanged(value) => { self.action_type = value; iced::Task::none() }
            Message::ActionCommandChanged(value) => { self.action_command = value; iced::Task::none() }
            Message::ActionArgsChanged(value) => { self.action_args = value; iced::Task::none() }
            Message::ActionScriptChanged(value) => { self.action_script = value; iced::Task::none() }
            Message::ActionMessageChanged(value) => { self.action_message = value; iced::Task::none() }
            Message::ActionLogLevelChanged(value) => { self.action_log_level = value; iced::Task::none() }
            Message::ActionMediaCommandChanged(value) => { self.action_media_command = value; iced::Task::none() }

            Message::NotificationShowTimed(msg, notification_type, seconds) => {
                self.notification = Some((msg, notification_type));
                // Return a task that waits N seconds then dismisses the notification
                iced::Task::perform(
                    async move {
                        tokio::time::sleep(tokio::time::Duration::from_secs(seconds)).await;
                    },
                    |_| Message::DismissNotification,
                )
            }

            Message::Batch(messages) => {
                // Process all messages in the batch
                let mut tasks = Vec::new();
                for msg in messages {
                    tasks.push(self.update(msg));
                }
                // Combine all tasks
                iced::Task::batch(tasks)
            }

            Message::ImportRules => {
                // Open file dialog for import
                return iced::Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("JSON", &["json"])
                            .set_title("Import Rules")
                            .pick_file()
                            .await
                    },
                    |file| {
                        if let Some(file) = file {
                            Message::ImportRulesFileSelected(file.path().to_path_buf())
                        } else {
                            Message::DismissNotification
                        }
                    },
                );
            }

            Message::ImportRulesFileSelected(path) => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            match tokio::fs::read_to_string(&path).await {
                                Ok(content) => {
                                    let eng = engine_clone.lock().await;
                                    let manager = eng.create_rule_manager();
                                    manager.import_rules(&content).await
                                }
                                Err(e) => Err(format!("Failed to read file: {}", e)),
                            }
                        },
                        |result| match result {
                            Ok(count) => Message::Batch(vec![
                                Message::RefreshData,
                                Message::NotificationShowTimed(
                                    format!("Imported {} rules", count),
                                    NotificationType::Success,
                                    3,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Import failed: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::ExportRules => {
                // Open file dialog for export
                return iced::Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("JSON", &["json"])
                            .set_title("Export Rules")
                            .save_file()
                            .await
                    },
                    |file| {
                        if let Some(file) = file {
                            Message::ExportRulesFileSelected(file.path().to_path_buf())
                        } else {
                            Message::DismissNotification
                        }
                    },
                );
            }

            Message::ExportRulesFileSelected(path) => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let manager = eng.create_rule_manager();
                            match manager.export_rules().await {
                                Ok(content) => {
                                    tokio::fs::write(&path, content).await
                                        .map_err(|e| format!("Failed to write file: {}", e))
                                }
                                Err(e) => Err(e),
                            }
                        },
                        |result| match result {
                            Ok(_) => Message::NotificationShowTimed(
                                "Rules exported successfully".to_string(),
                                NotificationType::Success,
                                3,
                            ),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Export failed: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            // Event Tester handlers
            Message::TestRuleChanged(rule_name) => {
                self.test_rule_name = rule_name;
                iced::Task::none()
            }

            Message::TestEventJsonAction(action) => {
                self.test_event_content.perform(action);
                iced::Task::none()
            }

            Message::RunEventTest => {
                if self.test_rule_name.is_empty() {
                    self.test_result = Some((false, "Please select a rule to test".to_string()));
                    return iced::Task::none();
                }

                let event_json = self.test_event_content.text();
                if event_json.is_empty() {
                    self.test_result = Some((false, "Please provide event JSON".to_string()));
                    return iced::Task::none();
                }

                // Find the selected rule
                if let Some(rule) = self.rules.iter().find(|r| r.name == self.test_rule_name) {
                    let rule_json = serde_json::to_value(rule).unwrap();
                    
                    if let Some(engine) = &self.engine {
                        let engine_clone = engine.clone();
                        return iced::Task::perform(
                            async move {
                                let eng = engine_clone.lock().await;
                                let manager = eng.create_rule_manager();
                                match manager.test_rule_match(rule_json, &event_json).await {
                                    Ok(matched) => (matched, if matched { "Rule matched the event".to_string() } else { "Rule did not match".to_string() }),
                                    Err(e) => (false, format!("Error: {}", e)),
                                }
                            },
                            |(matched, details)| Message::EventTestResult(matched, details),
                        );
                    }
                }
                iced::Task::none()
            }

            Message::EventTestResult(matched, details) => {
                self.test_result = Some((matched, details));
                iced::Task::none()
            }

            // Settings handlers (stub implementations)
            Message::ReloadConfig => {
                // TODO: Implement config reload
                iced::Task::none()
            }

            Message::ConfigReloaded(_) => {
                iced::Task::none()
            }

            Message::InstallService => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            eng.install_service()
                        },
                        |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::CheckServiceStatus,
                                Message::NotificationShowTimed(
                                    "Service installed successfully. Administrator privileges may be required to start the service.".to_string(),
                                    NotificationType::Success,
                                    5,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to install service: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::UninstallService => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            eng.uninstall_service()
                        },
                        |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::CheckServiceStatus,
                                Message::NotificationShowTimed(
                                    "Service uninstalled successfully".to_string(),
                                    NotificationType::Success,
                                    3,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to uninstall service: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::CheckServiceStatus => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            eng.is_service_installed()
                        },
                        Message::ServiceStatusChecked,
                    );
                }
                iced::Task::none()
            }

            Message::ServiceStatusChecked(installed) => {
                self.service_installed = installed;
                iced::Task::none()
            }

            Message::CheckAutoStartStatus => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            eng.is_service_auto_start()
                        },
                        Message::AutoStartStatusChecked,
                    );
                }
                iced::Task::none()
            }

            Message::AutoStartStatusChecked(auto_start) => {
                self.service_auto_start = auto_start;
                iced::Task::none()
            }

            Message::ToggleAutoStart(enabled) => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            eng.set_service_auto_start(enabled)
                        },
                        move |result| match result {
                            Ok(_) => Message::Batch(vec![
                                Message::CheckAutoStartStatus,
                                Message::NotificationShowTimed(
                                    if enabled {
                                        "Service will start automatically with Windows".to_string()
                                    } else {
                                        "Service will not start automatically with Windows".to_string()
                                    },
                                    NotificationType::Success,
                                    3,
                                ),
                            ]),
                            Err(e) => Message::NotificationShowTimed(
                                format!("Failed to change auto-start setting: {}", e),
                                NotificationType::Error,
                                5,
                            ),
                        },
                    );
                }
                iced::Task::none()
            }

            Message::ToggleHttpRequests(enabled) => {
                self.http_requests_enabled = enabled;
                // Update the engine config if engine is running
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let mut eng = engine_clone.lock().await;
                            eng.set_http_requests_enabled(enabled).await;
                        },
                        move |_| Message::NotificationShowTimed(
                            if enabled {
                                "HTTP requests enabled. Rules with HTTP actions will now execute.".to_string()
                            } else {
                                "HTTP requests disabled. Rules with HTTP actions will be blocked.".to_string()
                            },
                            NotificationType::Success,
                            3,
                        ),
                    );
                }
                iced::Task::none()
            }

            Message::CloseRequested => {
                // Shutdown engine when GUI closes
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let mut eng = engine_clone.lock().await;
                            eng.shutdown().await;
                        },
                        |_| Message::WindowClosed,
                    );
                }
                iced::Task::none()
            }

            Message::WindowClosed => {
                // Window will close after this
                iced::Task::none()
            }

            Message::Tick => {
                // Check for events first
                let mut event_tasks = vec![];
                if let Some(ref mut rx) = self.event_receiver {
                    // Try to receive any pending events (non-blocking)
                    loop {
                        match rx.try_recv() {
                            Ok(event) => {
                                event_tasks.push(Message::EventReceived(event));
                            }
                            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                        }
                    }
                }
                
                // Add metrics refresh
                event_tasks.push(Message::RefreshMetrics);
                
                if event_tasks.len() == 1 {
                    return self.update(event_tasks.into_iter().next().unwrap());
                } else {
                    return self.update(Message::Batch(event_tasks));
                }
            }

            Message::RefreshMetrics => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            let metrics = eng.metrics();
                            // Get totals using the aggregation methods
                            let events_total = metrics.get_counter_total("events_total");
                            let rules_matched = metrics.get_counter_total("rules_matched_total");
                            let actions_executed = metrics.get_counter_total("actions_executed_total");
                            (events_total, rules_matched, actions_executed)
                        },
                        |(events, rules, actions)| Message::MetricsUpdated(events, rules, actions),
                    );
                }
                iced::Task::none()
            }

            Message::MetricsUpdated(events, rules, actions) => {
                // Update dashboard metrics from aggregated totals
                self.metrics_events_total = events;
                self.metrics_rules_matched = rules;
                self.metrics_actions_executed = actions;
                self.metrics_last_update = std::time::Instant::now();
                iced::Task::none()
            }

            Message::RefreshEngineStatus => {
                if let Some(engine) = &self.engine {
                    let engine_clone = engine.clone();
                    return iced::Task::perform(
                        async move {
                            let eng = engine_clone.lock().await;
                            eng.get_status().await
                        },
                        Message::EngineStatusUpdated,
                    );
                }
                iced::Task::none()
            }

            Message::EngineStatusUpdated(status) => {
                self.engine_status = status;
                iced::Task::none()
            }

            Message::EventReceived(event) => {
                // Convert event to display format and add to list
                let timestamp = event.timestamp.format("%H:%M:%S").to_string();
                let event_type = format!("{:?}", event.kind);
                let details = match &event.kind {
                    engine_core::event::EventKind::WindowFocused { title, .. } => title.clone(),
                    engine_core::event::EventKind::WindowUnfocused { title, .. } => title.clone(),
                    engine_core::event::EventKind::WindowCreated { title, .. } => title.clone(),
                    engine_core::event::EventKind::WindowDestroyed { .. } => "Window closed".to_string(),
                    engine_core::event::EventKind::FileCreated { path } => path.to_string_lossy().to_string(),
                    engine_core::event::EventKind::FileModified { path } => path.to_string_lossy().to_string(),
                    engine_core::event::EventKind::FileDeleted { path } => path.to_string_lossy().to_string(),
                    engine_core::event::EventKind::ProcessStarted { name, .. } => name.clone(),
                    engine_core::event::EventKind::ProcessStopped { .. } => "Process stopped".to_string(),
                    _ => event_type.clone(),
                };
                
                let display_event = EventDisplay {
                    timestamp,
                    source: event.source.clone(),
                    event_type,
                    details,
                };
                
                // Add to front and keep last 50 events
                self.events.insert(0, display_event);
                if self.events.len() > 50 {
                    self.events.truncate(50);
                }
                
                iced::Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<Message> {
        let menu_bar = self.view_menu_bar();
        
        let content: Element<Message> = if self.rule_editor_open {
            // Show rule editor modal
            rules::view_editor(self)
        } else {
            match self.current_view {
                View::Dashboard => dashboard::view(self),
                View::Rules => rules::view(self),
                View::Sources => sources::view(self),
                View::EventTester => tester::view(self),
                View::Settings => settings::view(self),
            }
        };
        
        let mut main_content = column![
            menu_bar,
            content,
        ];
        
        // Add notification if present
        if let Some((msg, notification_type)) = &self.notification {
            let notification = view_notification(msg, *notification_type);
            main_content = column![notification, main_content];
        }
        
        container(main_content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn theme(&self) -> Theme {
        self.theme.to_iced_theme()
    }

    fn view_menu_bar(&self) -> Element<Message> {
        let dashboard_btn = button("Dashboard")
            .on_press(Message::NavigateTo(View::Dashboard));
        
        let rules_btn = button("Automations")
            .on_press(Message::NavigateTo(View::Rules));
        
        let sources_btn = button("Sources")
            .on_press(Message::NavigateTo(View::Sources));
        
        let tester_btn = button("Event Tester")
            .on_press(Message::NavigateTo(View::EventTester));
        
        let settings_btn = button("Settings")
            .on_press(Message::NavigateTo(View::Settings));
        
        row![
            dashboard_btn,
            rules_btn,
            sources_btn,
            tester_btn,
            settings_btn,
            Space::with_width(Length::Fill),
            text(format!("Status: {}", if self.engine_running { "Running" } else { "Stopped" })),
        ]
        .spacing(10)
        .padding(10)
        .into()
    }
}

fn view_notification(message: &str, notification_type: NotificationType) -> Element<Message> {
    let bg_color = match notification_type {
        NotificationType::Success => iced::Color::from_rgb(0.2, 0.8, 0.2),
        NotificationType::Error => iced::Color::from_rgb(0.9, 0.2, 0.2),
        NotificationType::Info => iced::Color::from_rgb(0.2, 0.6, 0.9),
    };
    
    container(
        row![
            text(message).style(move |_theme: &Theme| text::Style {
                color: Some(iced::Color::WHITE),
                ..Default::default()
            }),
            Space::with_width(Length::Fill),
            button("×").on_press(Message::DismissNotification),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding(10)
    .style(move |_theme: &Theme| container::Style {
        background: Some(bg_color.into()),
        ..Default::default()
    })
    .into()
}
