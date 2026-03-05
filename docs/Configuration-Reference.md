# Configuration Reference

Complete guide to configuring the Windows Event Automation Engine.

## Table of Contents

- [Basic Structure](#basic-structure)
- [Engine Settings](#engine-settings)
- [Security Settings](#security-settings)
- [Event Sources](#event-sources)
- [Rules](#rules)
- [Actions](#actions)
- [Variable Templating](#variable-templating)
- [Examples](#examples)

## Basic Structure

Configuration files use TOML format:

```toml
[engine]
event_buffer_size = 1000
log_level = "info"

[[sources]]
name = "my_watcher"
type = "file_watcher"
paths = ["C:/Data"]
pattern = "*.txt"

[[rules]]
name = "my_rule"
trigger = { type = "file_created", pattern = "*.txt" }
action = { type = "log", message = "File created!" }
```

## Engine Settings

```toml
[engine]
event_buffer_size = 1000      # Max events in buffer (default: 1000)
log_level = "info"            # debug, info, warn, error (default: info)
http_requests_enabled = false # Enable HTTP request actions (default: false)
```

## Security Settings

### HTTP Request Control

HTTP request actions are **disabled by default** for security. When disabled, rules with HTTP actions will log a warning instead of executing.

**Enable via config file:**
```toml
[engine]
http_requests_enabled = true
```

**Enable via GUI:**
1. Open WinEventEngine GUI
2. Go to **Settings** tab
3. Check **"Allow HTTP Request Actions"**

**Why disabled by default?**
- Prevents malicious rules from calling external APIs
- Protects against unauthorized data transmission
- Requires explicit user consent
- Applies to all HTTP request actions globally

## Event Sources

### File Watcher

```toml
[[sources]]
name = "file_monitor"
type = "file_watcher"
paths = ["C:/Data", "D:/Backup"]    # Directories to watch (required)
pattern = "*.txt"                    # File pattern (optional)
recursive = true                     # Watch subdirectories (default: false)
enabled = true                       # Enable/disable (default: true)
```

### Window Watcher

```toml
[[sources]]
name = "window_monitor"
type = "window_watcher"
enabled = true
```

### Process Monitor

```toml
[[sources]]
name = "process_monitor"
type = "process_monitor"
process_names = ["chrome.exe", "notepad.exe"]  # Filter processes (optional)
enabled = true
```

### Registry Monitor

```toml
[[sources]]
name = "registry_monitor"
type = "registry_monitor"
keys = [
    { root = "HKEY_LOCAL_MACHINE", path = "SOFTWARE", watch_tree = true }
]
enabled = true
```

### Timer

```toml
[[sources]]
name = "hourly_timer"
type = "timer"
interval_seconds = 3600  # Trigger every hour
enabled = true
```

## Rules

### Basic Rule Structure

```toml
[[rules]]
name = "rule_name"              # Unique name
description = "What this does"  # Optional description
enabled = true                  # Enable/disable

[rules.trigger]                 # When to trigger
type = "file_created"
pattern = "*.txt"

[rules.action]                  # What to do
type = "log"
message = "File created!"
```

### Multiple Actions

```toml
[[rules]]
name = "multi_action"
trigger = { type = "file_created" }

[[rules.action]]
type = "log"
message = "Step 1"

[[rules.action]]
type = "execute"
command = "backup.exe"
```

## Actions

### Log

```toml
action = { type = "log", message = "Event occurred", level = "info" }
```

Levels: `debug`, `info`, `warn`, `error`

### Execute Command

```toml
action = { 
    type = "execute", 
    command = "notepad.exe",
    args = ["file.txt"],
    working_dir = "C:/Temp"
}
```

### PowerShell

```toml
action = { 
    type = "powershell", 
    script = """
        Write-Host "Event: $env:EVENT_PATH"
    """,
    working_dir = "C:/Scripts"
}
```

### HTTP Request

**⚠️ Requires `http_requests_enabled = true` in engine settings**

```toml
action = { 
    type = "http_request", 
    url = "https://api.example.com/webhook",
    method = "POST",
    headers = { "Authorization" = "Bearer token", "Content-Type" = "application/json" },
    body = '{"event": "{{EVENT_PATH}}", "type": "{{EVENT_TYPE}}"}'
}
```

**HTTP Methods:** `GET`, `POST`, `PUT`, `DELETE`, `PATCH`, `HEAD`

**Response Variables** (set after execution for subsequent actions):
- `HTTP_STATUS_CODE` - HTTP status code (e.g., "200")
- `HTTP_RESPONSE_BODY` - Response body text
- `HTTP_SUCCESS` - "true" if status 200-299, "false" otherwise

### Lua Script

```toml
action = { 
    type = "script", 
    path = "my_script.lua",
    function = "on_event",
    timeout_ms = 30000,
    on_error = "fail"  # fail, continue, or log
}
```

### Media Control

```toml
action = { type = "media", command = "play_pause" }
```

**Commands:**
- `play_pause` or `toggle` - Toggle play/pause
- `play` - Start playback
- `pause` - Pause playback
- `next` - Next track
- `previous` or `prev` - Previous track
- `stop` - Stop playback
- `volume_up` - Increase volume
- `volume_down` - Decrease volume
- `mute` - Toggle mute

## Variable Templating

Template variables can be used in action parameters. They are replaced with actual event data at runtime.

### Common Variables

Available in all actions:

| Variable | Description | Example |
|----------|-------------|---------|
| `{{EVENT_TYPE}}` | Event type name | `FileCreated`, `WindowFocused` |
| `{{EVENT_SOURCE}}` | Source plugin name | `file_monitor`, `window_watcher` |
| `{{TIMESTAMP}}` | Event timestamp (RFC 3339) | `2024-01-15T10:30:00Z` |

### File Event Variables

Available for `FileCreated`, `FileModified`, `FileDeleted`:

| Variable | Description | Example |
|----------|-------------|---------|
| `{{EVENT_PATH}}` | Full file path | `C:/Data/document.txt` |
| `{{FILENAME}}` | File name only | `document.txt` |
| `{{DIRECTORY}}` | Parent directory | `C:/Data` |

**File Renamed Events** (`FileRenamed`):
- `{{OLD_PATH}}` - Original file path
- `{{NEW_PATH}}` - New file path

### Window Event Variables

Available for `WindowFocused`, `WindowUnfocused`, `WindowCreated`:

| Variable | Description | Example |
|----------|-------------|---------|
| `{{WINDOW_TITLE}}` | Window title | `Document - Notepad` |

### Process Event Variables

Available for `ProcessStarted`:

| Variable | Description | Example |
|----------|-------------|---------|
| `{{PROCESS_NAME}}` | Process name | `chrome.exe` |
| `{{PROCESS_PATH}}` | Full executable path | `C:/Program Files/Chrome/chrome.exe` |
| `{{PID}}` | Process ID | `12345` |

**Process Stopped Events** (`ProcessStopped`):
- `{{PID}}` - Process ID
- `{{PROCESS_NAME}}` - Process name
- `{{EXIT_CODE}}` - Exit code (if available)

### Metadata Variables

Custom metadata from events:

| Variable | Description |
|----------|-------------|
| `{{METADATA_KEY}}` | Replace KEY with actual metadata key name (uppercase) |

Example: If event has metadata `user_id`, use `{{METADATA_USER_ID}}`

### Usage Examples

**Discord webhook with file path:**
```toml
action = { 
    type = "http_request", 
    url = "https://discord.com/api/webhooks/...",
    method = "POST",
    headers = { "Content-Type" = "application/json" },
    body = '{"content": "File created: {{FILENAME}} in {{DIRECTORY}}"}'
}
```

**Log with window title:**
```toml
action = { 
    type = "log", 
    message = "Window focused: {{WINDOW_TITLE}} at {{TIMESTAMP}}",
    level = "info"
}
```

**PowerShell with process info:**
```toml
action = { 
    type = "powershell", 
    script = 'Write-Host "Process {{PROCESS_NAME}} (PID: {{PID}}) started at {{TIMESTAMP}}"'
}
```

**Execute command with file:**
```toml
action = { 
    type = "execute", 
    command = "process.exe",
    args = ["{{EVENT_PATH}}"]
}
```

## Environment Variables

Actions have access to these environment variables:

- `EVENT_PATH` - Path to the file (file events)
- `EVENT_TYPE` - Type of event
- `EVENT_SOURCE` - Source plugin name

After HTTP request actions:
- `HTTP_STATUS_CODE` - Response status code
- `HTTP_RESPONSE_BODY` - Response body
- `HTTP_SUCCESS` - "true" or "false"

## Examples

### Monitor Downloads for Executables

```toml
[[sources]]
name = "downloads_watcher"
type = "file_watcher"
paths = ["C:/Users/%USERNAME%/Downloads"]
pattern = "*.exe"

[[rules]]
name = "executable_alert"
trigger = { type = "file_created", pattern = "*.exe" }
action = { type = "log", message = "Executable downloaded!", level = "warn" }
```

### Backup Important Files

```toml
[[sources]]
name = "important_files"
type = "file_watcher"
paths = ["C:/Important"]
pattern = "*.docx"

[[rules]]
name = "backup_docs"
trigger = { type = "file_modified" }
action = { 
    type = "script", 
    path = "backup.lua",
    function = "on_event"
}
```

### Auto-commit on Config Changes

```toml
[[sources]]
name = "git_repo_watcher"
type = "file_watcher"
paths = ["C:/Projects/MyRepo"]
pattern = "*.toml"

[[rules]]
name = "auto_commit"
trigger = { type = "file_modified" }
action = { 
    type = "script", 
    path = "git_autocommit.lua",
    on_error = "log"
}
```

### Discord Notification with File Info

```toml
[engine]
http_requests_enabled = true  # Required for HTTP actions

[[rules]]
name = "discord_file_notification"
trigger = { type = "file_created", pattern = "*.txt" }
action = { 
    type = "http_request", 
    url = "https://discord.com/api/webhooks/YOUR_WEBHOOK_URL",
    method = "POST",
    headers = { "Content-Type" = "application/json" },
    body = '{"content": "📄 New file: {{FILENAME}}\n📁 Location: {{DIRECTORY}}\n⏰ Time: {{TIMESTAMP}}"}'
}
```

### Media Control on Window Focus

```toml
[[sources]]
name = "media_app_watcher"
type = "window_watcher"
enabled = true

[[rules]]
name = "pause_media_on_switch"
trigger = { type = "window_unfocused", title_contains = "Spotify" }
action = { type = "media", command = "play_pause" }
```

## See Also

- [Event Types](Event-Types) - All available event types
- [Lua Scripting API](Lua-Scripting-API) - Custom script documentation
- [Troubleshooting](Troubleshooting) - Common configuration issues
- [GUI Guide](GUI-Guide) - Using the native GUI
