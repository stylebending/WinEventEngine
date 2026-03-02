use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    #[serde(with = "chrono::serde::ts_microseconds")]
    pub timestamp: DateTime<Utc>,
    pub kind: EventKind,
    pub source: String,
    pub metadata: HashMap<String, String>,
}

impl Event {
    pub fn new(kind: EventKind, source: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            kind,
            source: source.into(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    // File System Events
    FileCreated {
        path: PathBuf,
    },
    FileModified {
        path: PathBuf,
    },
    FileDeleted {
        path: PathBuf,
    },
    FileRenamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },

    // Window Events
    WindowCreated {
        hwnd: isize,
        title: String,
        process_id: u32,
    },
    WindowDestroyed {
        hwnd: isize,
    },
    WindowFocused {
        hwnd: isize,
        title: String,
    },
    WindowUnfocused {
        hwnd: isize,
        title: String,
    },

    // Process Events
    ProcessStarted {
        pid: u32,
        parent_pid: u32,
        name: String,
        path: String,
        command_line: String,
        session_id: u32,
        user: String,
    },
    ProcessStopped {
        pid: u32,
        name: String,
        exit_code: Option<u32>,
    },

    // Thread Events
    ThreadCreated {
        pid: u32,
        tid: u32,
        start_address: u64,
        user_stack: Option<String>,
    },
    ThreadDestroyed {
        pid: u32,
        tid: u32,
    },

    // File Events (from ETW - includes process context)
    FileAccessed {
        pid: u32,
        path: PathBuf,
        access_mask: u32,
    },
    FileIoRead {
        pid: u32,
        path: PathBuf,
        bytes_read: u64,
    },
    FileIoWrite {
        pid: u32,
        path: PathBuf,
        bytes_written: u64,
    },
    FileIoDelete {
        pid: u32,
        path: PathBuf,
    },

    // Network Events (from ETW)
    NetworkConnectionCreated {
        pid: u32,
        local_addr: String,
        local_port: u16,
        remote_addr: String,
        remote_port: u16,
        protocol: NetworkProtocol,
    },
    NetworkConnectionClosed {
        pid: u32,
        local_addr: String,
        local_port: u16,
        remote_addr: String,
        remote_port: u16,
    },

    // Registry Events
    RegistryChanged {
        root: String,
        key: String,
        value_name: Option<String>,
        change_type: RegistryChangeType,
    },

    // Timer (for testing/scheduled tasks)
    TimerTick,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistryChangeType {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkProtocol {
    Tcp,
    Udp,
    Other(String),
}
