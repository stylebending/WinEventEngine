# Windows Event Automation Engine

Welcome to the WinEventEngine documentation! This wiki contains everything you need to understand, configure, and extend the engine.

## What is WinEventEngine?

A universal event automation system for Windows that monitors system events (file changes, window activity, process creation, registry modifications) and executes automated actions based on configurable rules.

Automate everything: 
- Play/pause media when focusing specific windows
- Auto commit changes to your config/dot files/folders
- Auto build/test an application under configurable conditions
- Get Webhook (Discord/Slack/Telegram/etc) notifications for configurable events
- Send API requests for configurable conditions/events
- Write easy-to-learn Lua scripts to customize everything
- Manage everything through an intuitive native GUI
- Much more! Simple configuration, powerful results

## Key Features

- **Event Monitoring**: File system, windows, processes, and registry
- **Rule Engine**: Pattern-based matching with Lua scripting
- **Native GUI**: Modern interface with real-time dashboard
- **Windows Service**: Run as background service
- **Plugin System**: Write custom actions in Lua

## First Time Setup (5 minutes)

New to WinEventEngine? This minimal example verifies everything works. You'll create a file watcher that logs when text files are created.

**Requirements:**
- Windows 10/11
- [Visual C++ Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe) (usually pre-installed on most systems)

### 1. Download

Download `WinEventEngine.exe` from [GitHub Releases](https://github.com/stylebending/win_event_engine/releases) and save it to a folder (e.g., `C:\Tools\WinEventEngine\`).

### 2. Create Your First Config

Create a file named `config.toml` in the same folder as `WinEventEngine.exe`. This config watches the `test_folder` subdirectory for new `.txt` files and logs when they're created:

```toml
[engine]
event_buffer_size = 100
log_level = "info"

# Watch for new files in a test directory
[[sources]]
name = "my_watcher"
type = "file_watcher"
paths = ["./test_folder"]
pattern = "*.txt"
enabled = true

# Log when text files are created
[[rules]]
name = "text_file_alert"
description = "Notify when text files are created"
trigger = { type = "file_created", pattern = "*.txt" }
action = { type = "log", message = "New text file created!", level = "info" }
enabled = true
```

**Need help with configuration?** See the **[Configuration Reference](Configuration-Reference)** for all available options.

### 3. Run the Engine

Double-click `WinEventEngine.exe` to launch the GUI, or open a terminal in the folder:

```bash
# Create a test folder (in the same folder as WinEventEngine.exe)
mkdir test_folder

# Start the engine
WinEventEngine.exe -c config.toml

# The engine is now running!
```

### 4. View the Dashboard

The native GUI will open automatically, showing:
- Real-time event monitoring
- Metrics dashboard
- Event stream with timestamps

### 5. Test It

In another terminal, create a test file:

```bash
echo "Hello World" > test_folder\test.txt
```

Watch as:
1. The engine logs: `[LUA] New text file created!`
2. The GUI dashboard shows the event in real-time!
3. The event counter increases

You'll see real-time events as they happen!

## Next Steps

### Try a Real Use Case

Once you've verified the engine works, try **media automation**:

1. Copy `config.media_automation.toml.example` to `config.toml`
2. Edit the `title_contains` value to match your preferred window (e.g., "nvim", "VS Code", "Firefox")
3. Run the engine and switch between windows
4. Your media will automatically pause when you leave that window, and resume when you return

### Getting Started
- **[GUI Guide](GUI-Guide)** - Complete guide to using the native GUI
- **[Configuration Reference](Configuration-Reference)** - Complete configuration options and examples
- **[Event Types](Event-Types)** - All available events and their data

### Custom Actions
- **[Lua Scripting API](Lua-Scripting-API)** - Write custom actions in Lua

### Monitoring & Debugging
- **[Troubleshooting](Troubleshooting)** - Common issues and solutions

### Development
- **[Architecture](Architecture)** - Technical deep-dive into the system
- **[Contributing](https://github.com/stylebending/win_event_engine/blob/main/CONTRIBUTING.md)** - How to contribute to the project

## Running as a Windows Service

Run the engine in the background without keeping a terminal open. The service starts automatically on Windows startup.

### Option 1: Via GUI (Recommended)

1. Open WinEventEngine GUI
2. Go to **Settings** tab
3. Click **Install Service** (requires Administrator privileges)
4. Check **Start with Windows** to enable auto-start

### Option 2: Via CLI

Open an **Administrator terminal in the folder containing `WinEventEngine.exe`**:

```bash
# Install the service
WinEventEngine.exe --install

# Start the service
sc start WinEventEngine
```

### Manage the Service

**Via GUI:**
- Use the Settings tab to install/uninstall and toggle auto-start

**Via CLI:**
```bash
# Check status
sc query WinEventEngine

# Stop the service
sc stop WinEventEngine

# Uninstall (stops and removes)
WinEventEngine.exe --uninstall
```

**Notes:**
- Service starts automatically on Windows boot (if auto-start enabled)
- Config file path must be absolute or relative to WinEventEngine.exe location
- Requires Administrator privileges for install/uninstall

## Quick Reference

### Common Commands

```bash
# View help
WinEventEngine.exe --help

# Run with a config file
WinEventEngine.exe -c config.toml

# Check if the engine is running
WinEventEngine.exe --status

# Enable debug logging
WinEventEngine.exe -c config.toml --log-level debug

# Dry run (see what would happen without executing)
WinEventEngine.exe -c config.toml --dry-run

# Install as Windows Service (requires admin terminal)
WinEventEngine.exe --install

# Start the Service (requires admin terminal)
sc start WinEventEngine

# Stop the Service (requires admin terminal)
sc stop WinEventEngine

# Uninstall as Windows Service (requires admin terminal)
WinEventEngine.exe --uninstall
```

### Example Configurations

**Download Alert:**
```toml
[[rules]]
name = "download_alert"
trigger = { type = "file_created", pattern = "*.exe" }
action = { type = "log", message = "Executable downloaded!", level = "warn" }
```

**Auto-backup:**
```toml
[[rules]]
name = "auto_backup"
trigger = { type = "file_modified", pattern = "*.docx" }
action = { type = "script", path = "backup.lua", function = "on_event" }
```

**HTTP Webhook (Discord):**
```toml
[[rules]]
name = "discord_notification"
trigger = { type = "file_created", pattern = "*.exe" }
action = { 
    type = "http_request", 
    url = "https://discord.com/api/webhooks/YOUR_WEBHOOK",
    method = "POST",
    headers = { "Content-Type" = "application/json" },
    body = '{"content": "Executable downloaded: {{EVENT_PATH}}"}'
}
```

See **[Configuration Reference](Configuration-Reference)** for more examples.

## Quick Links

- [GitHub Repository](https://github.com/stylebending/win_event_engine)
- [Releases](https://github.com/stylebending/win_event_engine/releases)
- [Issues](https://github.com/stylebending/win_event_engine/issues)
- [License: MIT](https://github.com/stylebending/win_event_engine/blob/main/LICENSE)
