mod config;
mod engine;
mod plugins;
mod service;

#[cfg(test)]
mod integration_tests;

// Re-export commonly used items from lib for binary use
use crate::config::*;

use clap::Parser;
use std::path::PathBuf;
use tokio::time::{timeout, Duration};
use tracing::{Level, debug, error, info, warn};
use tracing_subscriber;

#[derive(Parser, Debug)]
#[command(name = "Windows Event Automation Engine")]
#[command(about = "A universal event automation system for Windows")]
#[command(version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Directory containing configuration files
    #[arg(short = 'd', long, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// Run in dry-run mode (don't execute actions)
    #[arg(long)]
    dry_run: bool,

    /// Log level (debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Show engine status and exit
    #[arg(long)]
    status: bool,

    /// Disable hot-reloading of configuration
    #[arg(long)]
    no_watch: bool,

    /// Install as Windows Service (requires admin)
    #[arg(long)]
    install: bool,

    /// Uninstall Windows Service (requires admin)
    #[arg(long)]
    uninstall: bool,

    /// Run as Windows Service (internal - called by SCM)
    #[arg(long)]
    run_service: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = match cli.log_level.as_str() {
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .init();

    info!(
        "Windows Event Automation Engine v{}",
        env!("CARGO_PKG_VERSION")
    );

    // Handle service install/uninstall commands
    if cli.install || cli.uninstall {
        match service::ServiceManagerHandle::new() {
            Ok(manager) => {
                if cli.install {
                    let exe_path = std::env::current_exe()
                        .map_err(|e| format!("Failed to get executable path: {}", e))
                        .unwrap_or_else(|e| {
                            error!("{}", e);
                            std::process::exit(1);
                        });
                    match manager.install(exe_path.to_str().unwrap_or("")) {
                        Ok(_) => {
                            println!("Service installed successfully.");
                            println!(
                                "Use 'sc start WinEventEngine' or the Services control panel to start."
                            );
                        }
                        Err(e) => {
                            error!("Failed to install service: {}", e);
                            std::process::exit(1);
                        }
                    }
                } else if cli.uninstall {
                    match manager.uninstall() {
                        Ok(_) => {
                            println!("Service uninstalled successfully.");
                        }
                        Err(e) => {
                            error!("Failed to uninstall service: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to connect to Service Control Manager: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Run as Windows Service if --run-service flag is provided
    if cli.run_service {
        service::run_service();
        return;
    }

    // Determine config path for hot-reloading
    let config_path = if let Some(ref path) = cli.config {
        Some(path.clone())
    } else if let Some(ref dir) = cli.config_dir {
        Some(dir.clone())
    } else {
        let default_config = PathBuf::from("config.toml");
        let default_config_dir = PathBuf::from("config");
        if default_config.exists() {
            Some(default_config)
        } else if default_config_dir.exists() {
            Some(default_config_dir)
        } else {
            None
        }
    };

    // Load configuration
    let config = if let Some(config_path) = config_path.clone() {
        if config_path.is_dir() {
            info!("Loading configuration from directory: {:?}", config_path);
            match config::Config::load_from_dir(&config_path) {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!("Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            info!("Loading configuration from: {:?}", config_path);
            match config::Config::load_from_file(&config_path) {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!("Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }
    } else {
        // Try default locations
        let default_config = PathBuf::from("config.toml");
        let default_config_dir = PathBuf::from("config");

        if default_config.exists() {
            info!("Loading default configuration: config.toml");
            match config::Config::load_from_file(&default_config) {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!("Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            }
        } else if default_config_dir.exists() {
            info!("Loading configuration from config/ directory");
            match config::Config::load_from_dir(&default_config_dir) {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!("Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            // Use default/demo configuration
            info!("No configuration found, using default demo setup");
            create_demo_config()
        }
    };

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Configuration validation failed: {}", e);
        std::process::exit(1);
    }

    if cli.status {
        print_status(&config);
        return;
    }

fn create_demo_config() -> Config {
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

fn print_status(config: &Config) {
    println!("\nWinEventEngine Configuration Status");
    println!("====================================\n");
    
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

    if cli.dry_run {
        info!("Running in dry-run mode (actions will not be executed)");
    }

    // Ensure plugins directory exists
    if let Err(e) = std::fs::create_dir_all("plugins/actions") {
        warn!("Failed to create plugins/actions directory: {}", e);
    } else {
        debug!("Plugins directory ready");
    }

    // Create and initialize engine
    let mut engine_instance = engine::Engine::new(config, config_path.clone());

    if let Err(e) = engine_instance.initialize().await {
        error!("Failed to initialize engine: {}", e);
        std::process::exit(1);
    }

    // Start cleanup task for metrics
    let metrics = engine_instance.metrics();
    metrics.start_cleanup_task().await;

    let status = engine_instance.get_status().await;
    info!(
        "Engine running with {} plugins and {} rules",
        status.active_plugins, status.active_rules
    );

    // Start config hot-reloading if enabled
    let mut config_reload_rx = if !cli.no_watch && config_path.is_some() {
        engine_instance.watch_config().await;
        engine_instance.take_config_reload_rx()
    } else {
        if config_path.is_some() {
            info!("Hot-reloading disabled via --no-watch");
        }
        None
    };

    let shutdown_flag = engine_instance.shutdown_flag();

    // Setup graceful shutdown
    let (_shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let mut engine_for_shutdown = engine_instance;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal (Ctrl+C)");
                // Force immediate shutdown - don't wait for graceful completion
                break;
            }
            _ = shutdown_rx.recv() => {
                info!("Received shutdown command");
                break;
            }
            _ = async {
                match &mut config_reload_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                info!("Config change detected, reloading...");
                shutdown_flag.store(true, std::sync::atomic::Ordering::Relaxed);

                if let Some(ref path) = config_path {
                    match if path.is_dir() {
                        config::Config::load_from_dir(path)
                    } else {
                        config::Config::load_from_file(path)
                    } {
                        Ok(new_config) => {
                            if let Err(e) = engine_for_shutdown.reload(new_config).await {
                                error!("Failed to reload config: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to load new config: {}", e);
                        }
                    }
                }

                shutdown_flag.store(false, std::sync::atomic::Ordering::Relaxed);

                config_reload_rx = if !cli.no_watch && config_path.is_some() {
                    engine_for_shutdown.watch_config().await;
                    engine_for_shutdown.take_config_reload_rx()
                } else {
                    None
                };

                info!("Config reload complete, continuing to run...");
            }
        }
    }

    // Shutdown
    info!("Starting engine shutdown...");
    
    // Run shutdown with timeout to ensure we don't hang
    let shutdown_result = timeout(Duration::from_secs(3), async {
        engine_for_shutdown.shutdown().await;
    }).await;
    
    match shutdown_result {
        Ok(()) => info!("Engine shutdown completed gracefully"),
        Err(_) => {
            warn!("Engine shutdown timed out, forcing exit");
        }
    }
    
    info!("Engine stopped");
    
    // Force exit to ensure process terminates (kills any lingering threads)
    std::process::exit(0);
}


