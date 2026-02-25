use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub engine: EngineConfig,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            sources: Vec::new(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineConfig {
    #[serde(default = "default_event_buffer_size")]
    pub event_buffer_size: usize,
    #[serde(default)]
    pub log_level: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            event_buffer_size: default_event_buffer_size(),
            log_level: "info".to_string(),
        }
    }
}

fn default_event_buffer_size() -> usize {
    1000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceConfig {
    pub name: String,
    #[serde(flatten)]
    pub source_type: SourceType,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceType {
    FileWatcher {
        paths: Vec<PathBuf>,
        #[serde(default)]
        pattern: Option<String>,
        #[serde(default = "default_true")]
        recursive: bool,
    },
    WindowWatcher {
        #[serde(default)]
        title_pattern: Option<String>,
        #[serde(default)]
        process_pattern: Option<String>,
    },
    ProcessMonitor {
        #[serde(default)]
        process_name: Option<String>,
        #[serde(default)]
        monitor_threads: bool,
        #[serde(default)]
        monitor_files: bool,
        #[serde(default)]
        monitor_network: bool,
    },
    RegistryMonitor {
        root: String,
        key: String,
        #[serde(default)]
        recursive: bool,
    },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleConfig {
    pub name: String,
    pub description: Option<String>,
    pub trigger: TriggerConfig,
    pub action: ActionConfig,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerConfig {
    FileCreated {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },
    FileModified {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },
    FileDeleted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },
    WindowFocused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title_contains: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_name: Option<String>,
    },
    WindowUnfocused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title_contains: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_name: Option<String>,
    },
    WindowCreated,
    ProcessStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_name: Option<String>,
    },
    ProcessStopped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_name: Option<String>,
    },
    RegistryChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value_name: Option<String>,
    },
    Timer {
        #[serde(default = "default_timer_interval")]
        interval_seconds: u64,
    },
}

fn default_timer_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionConfig {
    Execute {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_dir: Option<PathBuf>,
    },
    PowerShell {
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_dir: Option<PathBuf>,
    },
    Log {
        message: String,
        #[serde(default = "default_log_level")]
        level: String,
    },
    Notify {
        title: String,
        message: String,
    },
    HttpRequest {
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        headers: HashMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
    },
    Media {
        command: String,
    },
    Script {
        path: PathBuf,
        #[serde(default = "default_script_function")]
        function: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default = "default_script_on_error")]
        on_error: String,
    },
}

fn default_script_function() -> String {
    "on_event".to_string()
}

fn default_script_on_error() -> String {
    "fail".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_http_method() -> String {
    "POST".to_string()
}

impl Config {
    pub fn load_from_file(path: &PathBuf) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::FileRead(path.clone(), e.to_string()))?;

        let config: Config =
            toml::from_str(&contents).map_err(|e| ConfigError::Parse(e.to_string()))?;

        Ok(config)
    }

    pub fn load_from_dir(dir: &PathBuf) -> Result<Self, ConfigError> {
        let mut config = Config::default();

        if !dir.exists() {
            return Ok(config);
        }

        for entry in
            std::fs::read_dir(dir).map_err(|e| ConfigError::FileRead(dir.clone(), e.to_string()))?
        {
            let entry = entry.map_err(|e| ConfigError::FileRead(dir.clone(), e.to_string()))?;
            let path = entry.path();

            if path.extension().map(|e| e == "toml").unwrap_or(false) {
                let file_config = Self::load_from_file(&path)?;
                config.sources.extend(file_config.sources);
                config.rules.extend(file_config.rules);
            }
        }

        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate that all rules reference valid sources
        for rule in &self.rules {
            if rule.name.is_empty() {
                return Err(ConfigError::Validation(format!("Rule must have a name")));
            }
        }

        // Validate sources have unique names
        let mut names = std::collections::HashSet::new();
        for source in &self.sources {
            if !names.insert(&source.name) {
                return Err(ConfigError::Validation(format!(
                    "Duplicate source name: {}",
                    source.name
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum ConfigError {
    FileRead(PathBuf, String),
    Parse(String),
    Validation(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::FileRead(path, msg) => {
                write!(f, "Failed to read config file {:?}: {}", path, msg)
            }
            ConfigError::Parse(msg) => write!(f, "Failed to parse config: {}", msg),
            ConfigError::Validation(msg) => write!(f, "Config validation error: {}", msg),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_simple_config() {
        let toml_str = r#"
[engine]
event_buffer_size = 500
log_level = "debug"

[[sources]]
name = "downloads_watcher"
type = "file_watcher"
paths = ["C:/Users/Downloads"]
pattern = "*.exe"
recursive = false
enabled = true

[[rules]]
name = "test_rule"
description = "Test file watcher"
trigger = { type = "file_created", pattern = "*.txt" }
action = { type = "log", message = "File created!" }
enabled = true
"#;

        let config: Config = toml::from_str(toml_str).expect("Failed to parse config");

        assert_eq!(config.engine.event_buffer_size, 500);
        assert_eq!(config.engine.log_level, "debug");
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.rules.len(), 1);

        let rule = &config.rules[0];
        assert_eq!(rule.name, "test_rule");
        assert!(rule.enabled);
    }

    #[test]
    fn test_load_from_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
[[sources]]
name = "test_source"
type = "file_watcher"
paths = ["C:/test"]
enabled = true

[[rules]]
name = "file_watcher"
trigger = {{ type = "file_created" }}
action = {{ type = "execute", command = "echo", args = ["Hello"] }}
enabled = true
"#
        )
        .unwrap();

        let config =
            Config::load_from_file(&temp_file.path().to_path_buf()).expect("Failed to load config");

        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.rules.len(), 1);
    }

    #[test]
    fn test_validate_duplicate_source_names() {
        let config = Config {
            sources: vec![
                SourceConfig {
                    name: "test".to_string(),
                    source_type: SourceType::FileWatcher {
                        paths: vec![PathBuf::from("/test")],
                        pattern: None,
                        recursive: false,
                    },
                    enabled: true,
                },
                SourceConfig {
                    name: "test".to_string(),
                    source_type: SourceType::FileWatcher {
                        paths: vec![PathBuf::from("/test2")],
                        pattern: None,
                        recursive: false,
                    },
                    enabled: true,
                },
            ],
            ..Default::default()
        };

        assert!(config.validate().is_err());
    }
}
