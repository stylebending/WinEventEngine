use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::prelude::*;
use windows_service::{
    define_windows_service,
    service::{ServiceAccess, ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceStatus},
    service::{ServiceInfo, ServiceStartType, ServiceType},
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

const SERVICE_NAME: &str = "WinEventEngine";
const SERVICE_DISPLAY_NAME: &str = "Windows Event Automation Engine";

fn log_to_file(msg: &str) {
    let log_path = get_service_log_path();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(file, "{}", msg);
    }
}

pub struct ServiceManagerHandle {
    manager: ServiceManager,
}

impl ServiceManagerHandle {
    pub fn new() -> Result<Self, ServiceError> {
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .map_err(|e| ServiceError::OpenScManager(e.to_string()))?;
        Ok(Self { manager })
    }

    pub fn install(&self, exe_path: &str) -> Result<(), ServiceError> {
        let service_info = ServiceInfo {
            name: SERVICE_NAME.into(),
            display_name: SERVICE_DISPLAY_NAME.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: windows_service::service::ServiceErrorControl::Normal,
            executable_path: exe_path.into(),
            launch_arguments: vec!["--run-service".into()],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };

        self.manager
            .create_service(&service_info, ServiceAccess::all())
            .map_err(|e| ServiceError::Install(e.to_string()))?;

        info!("Service installed successfully");
        Ok(())
    }

    pub fn uninstall(&self) -> Result<(), ServiceError> {
        let service = self
            .manager
            .open_service(SERVICE_NAME, ServiceAccess::DELETE)
            .map_err(|e| ServiceError::Uninstall(e.to_string()))?;

        service
            .delete()
            .map_err(|e| ServiceError::Uninstall(e.to_string()))?;

        info!("Service uninstalled successfully");
        Ok(())
    }
}

define_windows_service!(ffi_service_main, service_main);

static STOP_FLAG: AtomicBool = AtomicBool::new(false);

fn service_main(_arguments: Vec<std::ffi::OsString>) {
    log_to_file("Service starting...");
    
    let log_path = get_service_log_path();
    let log_file = match File::create(&log_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to create log file: {}", e);
            return;
        }
    };

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false);

    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(tracing_subscriber::filter::LevelFilter::INFO)
        .try_init();

    log_to_file("Tracing initialized");

    let config_path = get_default_config_path();
    log_to_file(&format!("Config path: {:?}", config_path));

    let config = if config_path.exists() {
        match crate::config::Config::load_from_file(&config_path) {
            Ok(cfg) => cfg,
            Err(e) => {
                log_to_file(&format!("Config load error: {}", e));
                crate::config::Config::default()
            }
        }
    } else {
        log_to_file("Using default config");
        crate::config::Config::default()
    };

    log_to_file("Creating engine...");
    let mut engine = crate::engine::Engine::new(config, Some(config_path));

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    
    let status_handle = match service_control_handler::register(SERVICE_NAME, move |control_event| {
        match control_event {
            ServiceControl::Stop => {
                log_to_file("Stop control received");
                STOP_FLAG.store(true, Ordering::Relaxed);
                let _ = shutdown_tx.try_send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => {
                ServiceControlHandlerResult::NoError
            }
            _ => {
                ServiceControlHandlerResult::NotImplemented
            }
        }
    }) {
        Ok(handle) => {
            log_to_file("Service status handle created");
            Some(handle)
        }
        Err(e) => {
            log_to_file(&format!("Failed to register service handler: {}, continuing without", e));
            None
        }
    };

    let engine_handle = thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            if let Err(e) = engine.initialize().await {
                log_to_file(&format!("Engine init error: {}", e));
                return;
            }
            
            log_to_file("Engine running successfully");
            
            tokio::select! {
                _ = async {
                    while !stop_flag_clone.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                } => {
                    log_to_file("Stop flag detected in engine");
                }
                _ = shutdown_rx.recv() => {
                    log_to_file("Shutdown signal received");
                }
            }
            
            log_to_file("Calling engine shutdown...");
            engine.shutdown().await;
            log_to_file("Engine shutdown complete");
        });
        
        log_to_file("Engine thread exiting");
    });

    log_to_file("Service running - engine initialized");
    
    if let Some(handle) = status_handle {
        let status = ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            controls_accepted: ServiceControlAccept::STOP,
            current_state: windows_service::service::ServiceState::Running,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(60),
            process_id: Some(std::process::id()),
        };
        
        let _ = handle.set_service_status(status);
        log_to_file("Service status set to RUNNING");
        
        while !STOP_FLAG.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }
        
        log_to_file("Stop signal received from SCM");
        
        let pending_status = ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            controls_accepted: ServiceControlAccept::STOP,
            current_state: windows_service::service::ServiceState::StopPending,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 1,
            wait_hint: Duration::from_secs(60),
            process_id: Some(std::process::id()),
        };
        let _ = handle.set_service_status(pending_status);
        
        let _ = engine_handle.join();
        
        let stopped_status = ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            controls_accepted: ServiceControlAccept::empty(),
            current_state: windows_service::service::ServiceState::Stopped,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 2,
            wait_hint: Duration::ZERO,
            process_id: Some(std::process::id()),
        };
        let _ = handle.set_service_status(stopped_status);
        
        log_to_file("Service status set to STOPPED");
    } else {
        while !STOP_FLAG.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }
        
        let _ = engine_handle.join();
    }
    
    log_to_file("Service stopped");
}

pub fn run_service() {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).unwrap();
}

pub fn get_service_log_path() -> PathBuf {
    let program_data = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".to_string());
    let log_dir = PathBuf::from(program_data).join("wineventengine").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    log_dir.join("service.log")
}

pub fn get_default_config_path() -> PathBuf {
    let program_data = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".to_string());
    let config_dir = PathBuf::from(program_data).join("wineventengine").join("config");
    let _ = std::fs::create_dir_all(&config_dir);
    config_dir.join("config.toml")
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum ServiceError {
    OpenScManager(String),
    Install(String),
    Uninstall(String),
    Start(String),
    Config(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::OpenScManager(msg) => write!(f, "Failed to open Service Control Manager: {}", msg),
            ServiceError::Install(msg) => write!(f, "Failed to install service: {}", msg),
            ServiceError::Uninstall(msg) => write!(f, "Failed to uninstall service: {}", msg),
            ServiceError::Start(msg) => write!(f, "Failed to start service: {}", msg),
            ServiceError::Config(msg) => write!(f, "Service configuration error: {}", msg),
        }
    }
}

impl std::error::Error for ServiceError {}
