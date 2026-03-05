//! WinEventEngine - Windows Event Automation Engine
//!
//! This crate provides the core engine for event-driven automation on Windows.
//! It can be used as a library for building custom applications or as a CLI tool.

pub mod config;
pub mod engine;
pub mod plugins;
pub mod service;

// Re-export commonly used types
pub use config::{Config, RuleConfig, ActionConfig, TriggerConfig, SourceConfig, SourceType, EngineConfig};
pub use engine::{Engine, EngineStatus, EngineError, EngineRuleManager};
pub use engine_core::event::{Event, EventKind};
pub use metrics::MetricsCollector;
pub use metrics::MetricsSnapshot;

// Version info
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the CLI mode (used by GUI when --cli flag is passed)
pub async fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    use tracing::info;
    
    println!("WinEventEngine CLI v{}", VERSION);
    println!("==========================");
    println!();
    
    // Load configuration
    let config_path = PathBuf::from("config.toml");
    let config = if config_path.exists() {
        match Config::load_from_file(&config_path) {
            Ok(config) => {
                println!("Loaded configuration from: {}", config_path.display());
                config
            }
            Err(e) => {
                println!("Warning: Failed to load config: {}", e);
                println!("Using default configuration");
                create_demo_config()
            }
        }
    } else {
        println!("No config.toml found, using default configuration");
        create_demo_config()
    };
    
    // Print status
    print_status(&config);
    
    // Initialize and run engine
    println!("Starting engine...");
    let mut engine = Engine::new(config, Some(config_path));
    
    match engine.initialize().await {
        Ok(_) => {
            let status = engine.get_status().await;
            println!();
            println!("Engine running with {} plugins and {} rules", 
                status.active_plugins, status.active_rules);
            println!("Press Ctrl+C to stop");
            
            // Wait for interrupt
            tokio::signal::ctrl_c().await?;
            println!();
            println!("Shutting down...");
            engine.shutdown().await;
            println!("Engine stopped");
            
            // Force exit to terminate any remaining spawned tasks
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Failed to initialize engine: {}", e);
            return Err(e.into());
        }
    }
}

/// Print configuration status
pub fn print_status(config: &Config) {
    println!("\nConfiguration Status");
    println!("====================\n");
    
    println!("Engine Settings:");
    println!("  Event Buffer Size: {}", config.engine.event_buffer_size);
    println!("  Log Level: {}\n", config.engine.log_level);
    
    println!("Event Sources ({}):", config.sources.len());
    for source in &config.sources {
        let status = if source.enabled { "enabled" } else { "disabled" };
        let type_name = match &source.source_type {
            SourceType::FileWatcher { .. } => "file_watcher",
            SourceType::WindowWatcher { .. } => "window_watcher",
            SourceType::ProcessMonitor { .. } => "process_monitor",
            SourceType::RegistryMonitor { .. } => "registry_monitor",
        };
        println!("  - {} ({}) [{}]", source.name, type_name, status);
    }
    println!();
    
    println!("Automation Rules ({}):", config.rules.len());
    for rule in &config.rules {
        let status = if rule.enabled { "enabled" } else { "disabled" };
        println!("  - {} [{}]", rule.name, status);
    }
    println!();
}

/// Create a default/demo configuration (for GUI use)
pub fn create_demo_config() -> Config {
    use config::*;
    use std::path::PathBuf;

    Config {
        engine: EngineConfig {
            event_buffer_size: 100,
            log_level: "info".to_string(),
            http_requests_enabled: false,
        },
        sources: vec![SourceConfig {
            name: "test_file_watcher".to_string(),
            source_type: SourceType::FileWatcher {
                paths: vec![PathBuf::from(".")],
                pattern: Some("*.txt".to_string()),
                recursive: false,
            },
            enabled: true,
        }],
        rules: vec![RuleConfig {
            name: "text_file_created".to_string(),
            description: Some("Detect when text files are created".to_string()),
            trigger: TriggerConfig::FileCreated {
                path: None,
                pattern: Some("*.txt".to_string()),
            },
            action: ActionConfig::Log {
                message: "Text file created!".to_string(),
                level: "info".to_string(),
            },
            enabled: true,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_demo_config() {
        let config = create_demo_config();
        assert!(!config.sources.is_empty());
        assert!(!config.rules.is_empty());
    }
}
