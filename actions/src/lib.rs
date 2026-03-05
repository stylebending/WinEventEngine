pub mod script_action;

use engine_core::event::Event;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tracing::{error, info, warn};

pub use script_action::{ScriptAction, ScriptErrorBehavior};

pub trait Action: Send + Sync {
    fn execute(&self, event: &Event) -> Result<ActionResult, ActionError>;
    fn description(&self) -> String;
    fn clone_box(&self) -> Box<dyn Action>;
}

impl Clone for Box<dyn Action> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for Box<dyn Action> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Action({})", self.description())
    }
}

#[derive(Debug, Clone)]
pub enum ActionResult {
    Success { message: Option<String> },
    Skipped { reason: String },
}

#[derive(Debug, Clone)]
pub enum ActionError {
    Execution(String),
    Configuration(String),
    Timeout,
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionError::Execution(msg) => write!(f, "Execution error: {}", msg),
            ActionError::Configuration(msg) => write!(f, "Configuration error: {}", msg),
            ActionError::Timeout => write!(f, "Action timed out"),
        }
    }
}

impl std::error::Error for ActionError {}

impl From<mlua::Error> for ActionError {
    fn from(err: mlua::Error) -> Self {
        ActionError::Execution(format!("Lua error: {}", err))
    }
}

#[derive(Debug, Clone)]
pub struct ExecuteAction {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub timeout_seconds: Option<u64>,
}

impl ExecuteAction {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            working_dir: None,
            timeout_seconds: Some(30),
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }
}

impl Action for ExecuteAction {
    fn execute(&self, _event: &Event) -> Result<ActionResult, ActionError> {
        let mut cmd = std::process::Command::new(&self.command);
        cmd.args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        info!("Executing: {} {}", self.command, self.args.join(" "));

        let output = cmd
            .spawn()
            .map_err(|e| ActionError::Execution(format!("Failed to spawn process: {}", e)))?
            .wait_with_output()
            .map_err(|e| ActionError::Execution(format!("Failed to wait for process: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                info!("Command output: {}", stdout.trim());
            }
            Ok(ActionResult::Success {
                message: Some(stdout.to_string()),
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ActionError::Execution(format!(
                "Command failed with exit code {:?}: {}",
                output.status.code(),
                stderr
            )))
        }
    }

    fn description(&self) -> String {
        format!("Execute: {} {}", self.command, self.args.join(" "))
    }

    fn clone_box(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct PowerShellAction {
    pub script: String,
    pub working_dir: Option<PathBuf>,
}

impl PowerShellAction {
    pub fn new(script: impl Into<String>) -> Self {
        Self {
            script: script.into(),
            working_dir: None,
        }
    }

    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }
}

impl Action for PowerShellAction {
    fn execute(&self, _event: &Event) -> Result<ActionResult, ActionError> {
        let mut cmd = std::process::Command::new("powershell.exe");
        cmd.arg("-Command")
            .arg(&self.script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }

        info!("Executing PowerShell script");

        let output = cmd
            .spawn()
            .map_err(|e| ActionError::Execution(format!("Failed to spawn PowerShell: {}", e)))?
            .wait_with_output()
            .map_err(|e| ActionError::Execution(format!("Failed to wait for PowerShell: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                info!("PowerShell output: {}", stdout.trim());
            }
            Ok(ActionResult::Success {
                message: Some(stdout.to_string()),
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ActionError::Execution(format!(
                "PowerShell failed: {}",
                stderr
            )))
        }
    }

    fn description(&self) -> String {
        format!("PowerShell: {}", &self.script[..self.script.len().min(50)])
    }

    fn clone_box(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

/// Media key action that uses direct Windows API calls (keybd_event) for instant media control
/// Supports smart play/pause that checks current media state before acting
#[derive(Debug, Clone)]
pub struct MediaKeyAction {
    pub command: String,
}

impl MediaKeyAction {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    /// Get the virtual key code for a media command
    fn get_vk_code(&self) -> u8 {
        match self.command.as_str() {
            "play_pause" | "play" | "pause" | "toggle" => 0xB3, // VK_MEDIA_PLAY_PAUSE
            "next" => 0xB0,                                     // VK_MEDIA_NEXT_TRACK
            "previous" | "prev" => 0xB1,                        // VK_MEDIA_PREV_TRACK
            "stop" => 0xB2,                                     // VK_MEDIA_STOP
            "volume_up" => 0xAF,                                // VK_VOLUME_UP
            "volume_down" => 0xAE,                              // VK_VOLUME_DOWN
            "mute" => 0xAD,                                     // VK_VOLUME_MUTE
            _ => 0xB3,                                          // Default to play/pause
        }
    }

    /// Check if media is currently playing using Windows GSMTC API
    /// Returns true if media is playing, false if paused/stopped or if state cannot be determined
    #[cfg(target_os = "windows")]
    fn is_media_playing(&self) -> bool {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

        // Try to get the session manager
        let manager_result = GlobalSystemMediaTransportControlsSessionManager::RequestAsync();

        // Block on the async operation with a timeout
        let manager = match manager_result {
            Ok(operation) => {
                // Use a simple blocking approach with timeout
                match operation.get() {
                    Ok(m) => m,
                    Err(e) => {
                        info!("Failed to get media session manager: {}", e);
                        return false; // Assume not playing if we can't determine state
                    }
                }
            }
            Err(e) => {
                info!("Failed to request media session manager: {}", e);
                return false; // Assume not playing if we can't determine state
            }
        };

        // Get the current session
        let session = match manager.GetCurrentSession() {
            Ok(s) => s,
            Err(_) => {
                // No active media session - assume not playing
                return false;
            }
        };

        // Get playback info
        let playback_info = match session.GetPlaybackInfo() {
            Ok(info) => info,
            Err(e) => {
                info!("Failed to get playback info: {}", e);
                return false; // Assume not playing if we can't determine state
            }
        };

        // Check the playback status
        match playback_info.PlaybackStatus() {
            Ok(status) => {
                use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus;
                matches!(
                    status,
                    GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
                )
            }
            Err(e) => {
                info!("Failed to get playback status: {}", e);
                false // Assume not playing if we can't determine state
            }
        }
    }

    /// Check if media is currently playing (non-Windows fallback)
    #[cfg(not(target_os = "windows"))]
    fn is_media_playing(&self) -> bool {
        false // Assume not playing on non-Windows platforms
    }
}

#[cfg(target_os = "windows")]
impl Action for MediaKeyAction {
    fn execute(&self, _event: &Event) -> Result<ActionResult, ActionError> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            keybd_event, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        };

        // Handle smart play/pause commands
        let should_send_key = match self.command.as_str() {
            "play" => {
                // Only send toggle if not already playing
                let is_playing = self.is_media_playing();
                if is_playing {
                    info!("Media is already playing, not sending play command");
                    return Ok(ActionResult::Success {
                        message: Some("Media already playing".to_string()),
                    });
                }
                true
            }
            "pause" => {
                // Only send toggle if currently playing
                let is_playing = self.is_media_playing();
                if !is_playing {
                    info!("Media is not playing, not sending pause command");
                    return Ok(ActionResult::Success {
                        message: Some("Media not playing".to_string()),
                    });
                }
                true
            }
            _ => true, // All other commands (play_pause, next, previous, etc.) always execute
        };

        if should_send_key {
            let vk_code = self.get_vk_code();

            info!(
                "Sending media key: {} (VK: 0x{:02X})",
                self.command, vk_code
            );

            unsafe {
                // Press the key
                keybd_event(vk_code, 0, KEYEVENTF_EXTENDEDKEY, 0);
                // Release the key
                keybd_event(vk_code, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP, 0);
            }

            Ok(ActionResult::Success {
                message: Some(format!("Media key '{}' sent", self.command)),
            })
        } else {
            unreachable!() // Should have returned earlier
        }
    }

    fn description(&self) -> String {
        format!("Media Key: {}", self.command)
    }

    fn clone_box(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

#[cfg(not(target_os = "windows"))]
impl Action for MediaKeyAction {
    fn execute(&self, _event: &Event) -> Result<ActionResult, ActionError> {
        Err(ActionError::Execution(
            "Media keys are only supported on Windows".to_string(),
        ))
    }

    fn description(&self) -> String {
        format!("Media Key: {} (Windows only)", self.command)
    }

    fn clone_box(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct LogAction {
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogAction {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            level: LogLevel::Info,
        }
    }

    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }
}

impl Action for LogAction {
    fn execute(&self, event: &Event) -> Result<ActionResult, ActionError> {
        let message = format!("{} [Event: {:?}]", self.message, event.kind);

        match self.level {
            LogLevel::Debug => tracing::debug!("{}", message),
            LogLevel::Info => tracing::info!("{}", message),
            LogLevel::Warn => tracing::warn!("{}", message),
            LogLevel::Error => tracing::error!("{}", message),
        }

        Ok(ActionResult::Success { message: None })
    }

    fn description(&self) -> String {
        format!("Log [{}]: {}", format!("{:?}", self.level), self.message)
    }

    fn clone_box(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

/// HTTP Request Action - Sends HTTP requests with variable templating support
#[derive(Debug, Clone)]
pub struct HttpRequestAction {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub timeout_ms: u64,
}

impl HttpRequestAction {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".to_string(),
            headers: HashMap::new(),
            body: None,
            timeout_ms: 30000, // 30 second default
        }
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into().to_uppercase();
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Replace template variables in a string
    fn substitute_variables(&self, template: &str, event: &Event) -> String {
        let mut result = template.to_string();

        // Event metadata
        result = result.replace("{{EVENT_TYPE}}", &format!("{:?}", event.kind));
        result = result.replace("{{EVENT_SOURCE}}", &event.source);
        result = result.replace("{{TIMESTAMP}}", &event.timestamp.to_rfc3339());

        // Event-specific fields
        match &event.kind {
            engine_core::event::EventKind::FileCreated { path }
            | engine_core::event::EventKind::FileModified { path }
            | engine_core::event::EventKind::FileDeleted { path } => {
                result = result.replace("{{EVENT_PATH}}", &path.to_string_lossy());
                if let Some(filename) = path.file_name() {
                    result = result.replace("{{FILENAME}}", &filename.to_string_lossy());
                }
                if let Some(parent) = path.parent() {
                    result = result.replace("{{DIRECTORY}}", &parent.to_string_lossy());
                }
            }
            engine_core::event::EventKind::FileRenamed { old_path, new_path } => {
                result = result.replace("{{OLD_PATH}}", &old_path.to_string_lossy());
                result = result.replace("{{NEW_PATH}}", &new_path.to_string_lossy());
            }
            engine_core::event::EventKind::WindowFocused { title, .. }
            | engine_core::event::EventKind::WindowUnfocused { title, .. }
            | engine_core::event::EventKind::WindowCreated { title, .. } => {
                result = result.replace("{{WINDOW_TITLE}}", title);
            }
            engine_core::event::EventKind::ProcessStarted {
                name, path, pid, ..
            } => {
                result = result.replace("{{PROCESS_NAME}}", name);
                result = result.replace("{{PROCESS_PATH}}", path);
                result = result.replace("{{PID}}", &pid.to_string());
            }
            engine_core::event::EventKind::ProcessStopped {
                pid,
                name,
                exit_code,
            } => {
                result = result.replace("{{PID}}", &pid.to_string());
                result = result.replace("{{PROCESS_NAME}}", name);
                if let Some(code) = exit_code {
                    result = result.replace("{{EXIT_CODE}}", &code.to_string());
                }
            }
            _ => {}
        }

        // Event metadata
        for (key, value) in &event.metadata {
            result = result.replace(&format!("{{{{METADATA_{}}}}}", key.to_uppercase()), value);
        }

        result
    }
}

impl Action for HttpRequestAction {
    fn execute(&self, event: &Event) -> Result<ActionResult, ActionError> {
        use reqwest::blocking::Client;
        use std::time::Duration;

        // Substitute variables in URL and body
        let url = self.substitute_variables(&self.url, event);
        let body = self
            .body
            .as_ref()
            .map(|b| self.substitute_variables(b, event));

        // Build request
        let client = Client::builder()
            .timeout(Duration::from_millis(self.timeout_ms))
            .build()
            .map_err(|e| {
                ActionError::Configuration(format!("Failed to create HTTP client: {}", e))
            })?;

        let mut request_builder = match self.method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "DELETE" => client.delete(&url),
            "PATCH" => client.patch(&url),
            "HEAD" => client.head(&url),
            _ => {
                return Err(ActionError::Configuration(format!(
                    "Invalid HTTP method: {}",
                    self.method
                )))
            }
        };

        // Add headers with variable substitution
        for (key, value) in &self.headers {
            let substituted_value = self.substitute_variables(value, event);
            request_builder = request_builder.header(key, substituted_value);
        }

        // Add body if present
        if let Some(ref body_content) = body {
            request_builder = request_builder.body(body_content.clone());
        }

        // Send request
        info!("Sending {} request to {}", self.method, url);
        let response = request_builder
            .send()
            .map_err(|e| ActionError::Execution(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        let response_body = response.text().unwrap_or_default();

        // Set environment variables for subsequent actions
        // Note: set_var is unsafe in Rust 2024 edition due to thread safety concerns.
        // In this context, it's acceptable as actions are executed sequentially.
        unsafe {
            std::env::set_var("HTTP_STATUS_CODE", status.as_u16().to_string());
            std::env::set_var("HTTP_RESPONSE_BODY", &response_body);
            std::env::set_var(
                "HTTP_SUCCESS",
                if status.is_success() { "true" } else { "false" },
            );
        }

        if status.is_success() {
            info!("HTTP request successful: {}", status);
            Ok(ActionResult::Success {
                message: Some(format!(
                    "HTTP {} - Response: {}",
                    status,
                    response_body.chars().take(200).collect::<String>()
                )),
            })
        } else {
            warn!("HTTP request failed: {}", status);
            Ok(ActionResult::Success {
                message: Some(format!(
                    "HTTP {} - Error: {}",
                    status,
                    response_body.chars().take(200).collect::<String>()
                )),
            })
        }
    }

    fn description(&self) -> String {
        format!("HTTP {} {}", self.method, self.url)
    }

    fn clone_box(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

pub struct CompositeAction {
    pub actions: Vec<Box<dyn Action>>,
    pub on_error: ErrorBehavior,
}

impl std::fmt::Debug for CompositeAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CompositeAction({} actions)", self.actions.len())
    }
}

impl Clone for CompositeAction {
    fn clone(&self) -> Self {
        Self {
            actions: self.actions.clone(),
            on_error: self.on_error,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorBehavior {
    Continue,
    Stop,
    SkipRemaining,
}

impl CompositeAction {
    pub fn new(actions: Vec<Box<dyn Action>>) -> Self {
        Self {
            actions,
            on_error: ErrorBehavior::Continue,
        }
    }

    pub fn with_error_behavior(mut self, behavior: ErrorBehavior) -> Self {
        self.on_error = behavior;
        self
    }
}

impl Action for CompositeAction {
    fn execute(&self, event: &Event) -> Result<ActionResult, ActionError> {
        let mut results = Vec::new();

        for action in &self.actions {
            match action.execute(event) {
                Ok(result) => results.push(result),
                Err(e) => {
                    error!("Action failed: {} - {}", action.description(), e);
                    match self.on_error {
                        ErrorBehavior::Continue => continue,
                        ErrorBehavior::Stop => return Err(e),
                        ErrorBehavior::SkipRemaining => break,
                    }
                }
            }
        }

        Ok(ActionResult::Success {
            message: Some(format!("Executed {} actions", results.len())),
        })
    }

    fn description(&self) -> String {
        format!("Composite ({} actions)", self.actions.len())
    }

    fn clone_box(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

pub struct ActionExecutor {
    actions: HashMap<String, Box<dyn Action>>,
}

impl ActionExecutor {
    pub fn new() -> Self {
        Self {
            actions: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: impl Into<String>, action: Box<dyn Action>) {
        self.actions.insert(name.into(), action);
    }

    pub fn execute(&self, name: &str, event: &Event) -> Result<ActionResult, ActionError> {
        match self.actions.get(name) {
            Some(action) => action.execute(event),
            None => Err(ActionError::Configuration(format!(
                "Action '{}' not found",
                name
            ))),
        }
    }
}

impl Clone for ActionExecutor {
    fn clone(&self) -> Self {
        Self {
            actions: self.actions.clone(),
        }
    }
}

impl Default for ActionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::event::EventKind;

    #[test]
    fn test_log_action() {
        let action = LogAction::new("Test message").with_level(LogLevel::Info);
        let event = Event::new(
            EventKind::FileCreated {
                path: PathBuf::from("test.txt"),
            },
            "test",
        );

        let result = action.execute(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn test_execute_action_echo() {
        let action = ExecuteAction::new("echo").with_args(vec!["Hello".to_string()]);
        let event = Event::new(EventKind::TimerTick, "test");

        let result = action.execute(&event);
        assert!(result.is_ok());

        if let Ok(ActionResult::Success { message: Some(msg) }) = result {
            assert!(msg.contains("Hello"));
        }
    }

    #[test]
    fn test_action_executor() {
        let mut executor = ActionExecutor::new();
        executor.register("log", Box::new(LogAction::new("Test")));

        let event = Event::new(EventKind::TimerTick, "test");
        let result = executor.execute("log", &event);
        assert!(result.is_ok());

        let result = executor.execute("nonexistent", &event);
        assert!(result.is_err());
    }
}
