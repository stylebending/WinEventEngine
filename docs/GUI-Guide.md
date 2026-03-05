# GUI Guide

Complete guide to using the WinEventEngine native GUI.

## Table of Contents

- [Overview](#overview)
- [Getting Started](#getting-started)
- [Dashboard](#dashboard)
- [Automations (Rules)](#automations-rules)
- [Event Sources](#event-sources)
- [Event Tester](#event-tester)
- [Settings](#settings)
- [Keyboard Shortcuts](#keyboard-shortcuts)
- [Tips & Tricks](#tips--tricks)

## Overview

The WinEventEngine GUI provides an intuitive, modern interface for managing your automation rules. Built with the [Iced](https://iced.rs/) framework, it offers:

- **Real-time monitoring** of events and metrics
- **Visual rule management** with forms and validation
- **Interactive testing** of rules against sample events
- **Theme customization** (Dark, Light, System)
- **Service management** without command line
- **Import/Export** of automation rules

## Getting Started

### Launching the GUI

Simply double-click `WinEventEngine.exe` or run it from the terminal:

```bash
WinEventEngine.exe
```

The GUI will open automatically with the Dashboard view.

### First Time Setup

1. **Engine Status**: Check the top of the window for engine status
2. **Create Rules**: Navigate to the "Automations" tab to create your first rule
3. **Start Engine**: If the engine isn't running, click "Start Engine" in Settings
4. **Monitor**: Return to Dashboard to see events in real-time

### Navigation

Use the menu bar at the top to switch between views:

- **Dashboard** - Real-time event monitoring
- **Automations** - Create and manage rules
- **Sources** - Configure event sources
- **Event Tester** - Test rules against sample events
- **Settings** - Configure themes, service, and security

## Dashboard

The Dashboard provides a real-time view of system activity.

### Layout

```
┌──────────────────────────────────────────────────────────┐
│ Dashboard                                    [Theme]     │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ┌──────────────┬──────────────┬──────────────┐         │
│  │ Events Total │ Rules Matched│ Actions      │         │
│  │     42       │     12       │     8        │         │
│  └──────────────┴──────────────┴──────────────┘         │
│                                                          │
│  Event Stream                                            │
│  ┌─────────────────────────────────────────────────────┐│
│  │ 10:30:15  file_watcher  FileCreated  document.txt   ││
│  │ 10:30:12  window_watcher WindowFocused  Notepad    ││
│  │ 10:29:45  file_watcher  FileModified  config.toml  ││
│  │ ...                                                 ││
│  └─────────────────────────────────────────────────────┘│
│                                                          │
└──────────────────────────────────────────────────────────┘
```

### Metrics Cards

Three cards display key metrics:

- **Events Total**: Total number of events processed
- **Rules Matched**: Number of rule matches
- **Actions**: Number of actions executed

These update automatically every 2 seconds.

### Event Stream

The event stream shows the last 50 events with:

- **Timestamp** - When the event occurred (HH:MM:SS format)
- **Source** - Which plugin generated the event
- **Type** - Event type (FileCreated, WindowFocused, etc.)
- **Details** - Event-specific information (filename, window title, etc.)

**Newest events appear at the top.**

### Auto-Refresh

The dashboard automatically refreshes every 2 seconds. No manual refresh needed!

## Automations (Rules)

The Automations tab is where you create and manage automation rules.

### Rule List

The main view shows all your rules:

```
┌──────────────────────────────────────────────────────────┐
│ Automations                                    [+] [🔄]  │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ☑ text_file_alert            [Edit] [Delete] [Test]    │
│     File watcher → Log notification                      │
│                                                          │
│  ☑ media_control              [Edit] [Delete] [Test]    │
│     Window watcher → Media pause/play                    │
│                                                          │
│  ☐ disabled_rule              [Edit] [Delete] [Test]    │
│     Process monitor → Log (Disabled)                     │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

**Status Icons:**
- ☑ Rule is enabled
- ☐ Rule is disabled

**Actions:**
- **Edit** - Modify the rule
- **Delete** - Remove the rule
- **Test** - Test the rule in the Event Tester

### Creating a New Rule

Click the **+** button to create a new rule:

1. **Rule Name**: Unique name (required)
2. **Description**: Optional description of what the rule does
3. **Enabled**: Check to enable the rule immediately
4. **Trigger**: Configure when the rule fires
5. **Action**: Configure what happens when triggered

### Trigger Types

Select the trigger type and configure parameters:

**File Created/Modified/Deleted:**
- Pattern: File pattern (e.g., `*.txt`, `*.log`)
- Path: Optional specific directory

**Window Focused/Unfocused/Created:**
- Title Contains: Window title substring
- Process Name: Optional process filter

**Process Started/Stopped:**
- Process Name: Filter by executable name

**Timer:**
- Interval: Seconds between triggers

### Action Types

Configure what happens when the rule matches:

**Log:**
- Message: Text to log
- Level: debug, info, warn, error

**Execute:**
- Command: Program to run
- Arguments: Command line arguments
- Working Directory: Optional starting directory

**PowerShell:**
- Script: PowerShell script content
- Working Directory: Optional starting directory

**HTTP Request:**
- URL: Webhook or API endpoint
- Method: GET, POST, PUT, DELETE, PATCH
- Headers: Key-value pairs
- Body: Request body (supports variables)
- ⚠️ Requires HTTP requests enabled in Settings

**Media:**
- Command: play_pause, next, previous, stop, volume_up, volume_down, mute

**Script (Lua):**
- Path: Path to .lua file
- Function: Lua function to call
- Timeout: Maximum execution time
- On Error: fail, continue, or log

### Import/Export

**Export Rules:**
1. Click the export button (📤)
2. Choose save location
3. Rules saved as JSON file

**Import Rules:**
1. Click the import button (📥)
2. Select a previously exported JSON file
3. Rules are imported and validated

**Use Cases:**
- Backup your rules
- Share rules between computers
- Version control your automations

## Event Sources

The Sources tab shows configured event sources (plugins).

### Source List

Displays all event sources with their status:

```
┌──────────────────────────────────────────────────────────┐
│ Event Sources                                            │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ☑ file_monitor                                         │
│     Type: File Watcher                                   │
│     Paths: C:/Data, D:/Backup                            │
│     Pattern: *.txt                                       │
│                                                          │
│  ☑ window_watcher                                       │
│     Type: Window Watcher                                 │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

### Adding Sources

Sources are typically auto-provisioned when you create rules, but you can view their configuration here.

**Note:** Source configuration is primarily managed through the TOML config file. The GUI displays current sources but editing is limited.

## Event Tester

Test your rules against sample events without waiting for real events.

### Using the Tester

1. **Select Rule**: Choose which rule to test from the dropdown
2. **Event JSON**: Enter or paste a sample event
3. **Test**: Click the "Test Rule" button
4. **Results**: See if the rule matched and any output

### Sample Event Format

Default example provided:
```json
{
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
}
```

### Understanding Results

- **✓ RULE MATCHED**: The rule's trigger conditions were met
- **✗ Rule did not match**: The conditions weren't satisfied
- **Error messages**: JSON parsing errors or rule configuration issues

**Tip:** Use the tester to debug rule logic before enabling rules!

## Settings

Configure the GUI, engine, and system integration.

### Theme

Select your preferred appearance:

- **Dark** - Dark color scheme (default)
- **Light** - Light color scheme
- **System** - Follow Windows theme setting

Changes apply immediately.

### Engine Controls

**Start Engine**: Launch the automation engine
**Stop Engine**: Stop the engine gracefully

The engine status is shown at the top of the window.

### Service Management

**Service Status**: Shows if the Windows service is installed

**Install Service**: Install engine as Windows service
- Requires Administrator privileges
- Service runs in background without GUI
- Can be configured to start automatically

**Uninstall Service**: Remove the Windows service
- Requires Administrator privileges

**Start with Windows**: Toggle automatic service startup
- When enabled, service starts on Windows boot
- When disabled, manual start required

**Note:** Service management requires running the GUI as Administrator.

### Security

**Allow HTTP Request Actions**: Enable/disable HTTP requests

⚠️ **Security Feature:**
- Disabled by default
- Prevents rules from making unauthorized network requests
- Must be enabled for HTTP webhook actions to work
- Applies to all HTTP actions globally

**When to enable:**
- Using Discord/Slack webhooks
- API integrations
- Cloud notifications

**When to keep disabled:**
- Security-sensitive environments
- No external API needs
- Preventing data exfiltration

### Config Reload

**Reload Config**: Reload configuration from disk without restarting

Useful when:
- Editing config.toml manually
- Restoring from backup
- Testing configuration changes

## Keyboard Shortcuts

### Global

- **Ctrl+1** - Dashboard
- **Ctrl+2** - Automations
- **Ctrl+3** - Event Sources
- **Ctrl+4** - Event Tester
- **Ctrl+5** - Settings

### Rules List

- **Ctrl+N** - New Rule
- **Ctrl+E** - Edit selected rule
- **Delete** - Delete selected rule
- **Ctrl+T** - Test selected rule

### General

- **F5** - Refresh data (Automations tab)
- **Ctrl+R** - Reload config
- **Esc** - Close modal dialogs

## Tips & Tricks

### Rule Organization

1. **Use descriptive names**: `backup_important_docs` instead of `rule_1`
2. **Add descriptions**: Document what the rule does and why
3. **Group related rules**: Use consistent naming prefixes
4. **Disable unused rules**: Don't delete, just disable for later

### Testing Workflow

1. Create rule in disabled state
2. Use Event Tester to validate
3. Enable rule
4. Monitor Dashboard for activity
5. Adjust if needed

### Performance

- **Limit event history**: Dashboard keeps last 50 events
- **Use specific patterns**: `*.txt` is faster than `*`
- **Disable unused sources**: Reduces overhead
- **Monitor metrics**: Watch for dropped events

### Troubleshooting

**Rules not firing?**
- Check if rule is enabled
- Verify trigger conditions match
- Test with Event Tester
- Check engine is running

**Actions not executing?**
- Check action configuration
- Verify paths and permissions
- For HTTP: ensure security setting is enabled
- Check logs for errors

**GUI not showing events?**
- Ensure engine is running
- Check event sources are configured
- Verify rules are matching
- Try refreshing the view

### Best Practices

1. **Start simple**: Test with log actions first
2. **Use variables**: Leverage `{{EVENT_PATH}}`, `{{WINDOW_TITLE}}`, etc.
3. **Handle errors**: Set appropriate `on_error` for scripts
4. **Backup rules**: Export before major changes
5. **Monitor**: Keep Dashboard open while testing

## See Also

- [Configuration Reference](Configuration-Reference) - TOML config options
- [Event Types](Event-Types) - Available events and data
- [Troubleshooting](Troubleshooting) - Common issues
- [Architecture](Architecture) - Technical details
