# Troubleshooting

Common issues and their solutions.

## Installation Issues

### Missing Visual C++ Redistributable

**Error**: `The code execution cannot proceed because VCRUNTIME140.dll was not found`

**Solution**: Download and install [Visual C++ Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe) from Microsoft.

### Service Registration Fails

**Error**: `Access denied` or "Administrator privileges required" when installing service

**Solution**: Run the GUI as Administrator:
1. Right-click on `WinEventEngine.exe`
2. Select "Run as administrator"
3. Go to Settings tab
4. Click "Install Service"

Or via Command Prompt:
```cmd
Run as administrator: cmd.exe
WinEventEngine.exe --install
```

## Configuration Issues

### Config File Not Found

**Error**: `Failed to load configuration: No such file or directory`

**Solution**: 
1. Verify config file path
2. Use absolute path: `-c C:/full/path/to/config.toml`
3. Check file permissions

### Invalid TOML Syntax

**Error**: `TOML parse error at line X, column Y`

**Common mistakes**:
```toml
# Wrong: Using single quotes for strings
pattern = '*.txt'

# Right: Use double quotes
pattern = "*.txt"

# Wrong: Missing commas in arrays
paths = ["C:/Data" "D:/Backup"]

# Right: Add commas
paths = ["C:/Data", "D:/Backup"]
```

### Source Configuration Errors

**Error**: `missing field 'paths'`

**Solution**: File watcher uses `paths` (array), not `path`:
```toml
# Wrong:
path = "C:/Data"

# Right:
paths = ["C:/Data"]
```

## GUI Issues

### GUI Won't Start

**Problem**: Double-clicking executable does nothing

**Solutions**:
1. Check Windows Defender/antivirus isn't blocking it
2. Try running from terminal to see error messages
3. Verify Visual C++ Redistributable is installed
4. Check if another instance is already running

### Dashboard Shows "No events yet..."

**Problem**: No events appearing in GUI

**Checklist**:
1. ✅ Engine is running (check status at top of window)
2. ✅ Click "Start Engine" in Settings if stopped
3. ✅ Rules are enabled
4. ✅ Event sources are configured
5. ✅ Trigger test events manually to verify

**Debug**:
```bash
# Create a test file in monitored directory
echo "test" > C:/Monitored/test.txt
```

### GUI Freezes or Unresponsive

**Problem**: Interface becomes unresponsive

**Solutions**:
1. Wait for operations to complete (import/export can take time)
2. Restart the application
3. Check if engine process is still running
4. Reduce number of active rules

### Theme Not Applying

**Problem**: Theme change doesn't take effect

**Solution**: Theme changes apply immediately. If not:
1. Try switching to a different theme first
2. Restart the application
3. Check graphics drivers (Iced uses GPU acceleration)

## Runtime Issues

### Events Not Triggering

**Problem**: Rules aren't firing when expected

**Checklist**:
1. ✅ Source is enabled: `enabled = true`
2. ✅ Rule is enabled: `enabled = true`
3. ✅ Pattern matches exactly (case-sensitive on some systems)
4. ✅ File watcher has correct `paths` (plural, array)
5. ✅ Check engine logs for errors

**Debug**: Add a simple log action:
```toml
[[rules]]
name = "debug_all_events"
trigger = { type = "file_created", pattern = "*" }
action = { type = "log", message = "Event: {{EVENT_PATH}}", level = "debug" }
```

### Lua Script Errors

**Error**: `Lua execution error: ...`

**Common causes**:
1. **Syntax error**: Check Lua syntax with `lua -c script.lua`
2. **Function not found**: Ensure function name matches config
3. **Missing return**: Scripts must return `{success = true/false}`
4. **File not found**: Check script path is relative to `plugins/actions/`

**Debug**: Test script standalone:
```lua
-- Add at bottom for testing
local test_event = {
    kind = "FileCreated",
    source = "test",
    metadata = {path = "C:/test.txt"}
}
print(on_event(test_event))
```

### High CPU Usage

**Problem**: Engine consuming too much CPU

**Solutions**:
1. Reduce `event_buffer_size` in config
2. Add more specific patterns (avoid `*`)
3. Disable unused sources
4. Check for infinite loops in Lua scripts
5. Increase timer intervals

### Memory Leaks

**Problem**: Engine memory usage grows over time

**Solutions**:
1. Check Lua scripts for memory leaks
2. Verify action timeouts are set
3. Dashboard keeps last 50 events (normal)
4. Restart engine periodically as workaround

## Event Source Issues

### File Watcher Not Detecting Changes

**Problem**: Files created but no events fired

**Checklist**:
1. ✅ Directory exists and is accessible
2. ✅ Pattern matches (test with `*`)
3. ✅ Watch permissions (try running as admin)
4. ✅ Not a network drive (limited support)
5. ✅ Not a cloud-synced folder (OneDrive, Dropbox)

**Test**:
```bash
# Create a test file
echo "test" > C:/Monitored/test.txt

# Check engine logs in terminal
```

### Window Watcher Not Working

**Problem**: Window events not detected

**Solutions**:
1. Run as administrator (required for some windows)
2. Some system windows cannot be monitored
3. UWP apps have limited visibility

### Process Monitor Not Detecting

**Problem**: Process events not firing

**Solutions**:
1. Run as administrator
2. Check process name is exact (including `.exe`)
3. Some system processes are protected
4. Antivirus may block ETW

### Registry Monitor Access Denied

**Problem**: Cannot monitor registry keys

**Solutions**:
1. Run as administrator
2. Some keys are protected (HKLM\SAM, HKLM\SECURITY)
3. Check path format: `SOFTWARE\\MyApp` (double backslash in config)

## Action Issues

### Execute Action Fails

**Error**: `Execution error: program not found`

**Solutions**:
1. Use full path: `C:/Windows/System32/notepad.exe`
2. Add to PATH environment variable
3. Check file permissions
4. For PowerShell: Use `powershell.exe -Command "..."`

### HTTP Request Blocked

**Error**: "HTTP requests disabled" or action logs warning instead of executing

**Solutions**:
1. Enable HTTP requests in GUI: Settings → Security → "Allow HTTP Request Actions"
2. Or enable in config: `http_requests_enabled = true`
3. Restart engine after enabling
4. Verify the setting persisted (check Settings tab)

### HTTP Request Fails

**Error**: `HTTP 0` or timeout

**Solutions**:
1. Check URL is correct
2. Verify network connectivity
3. Check firewall rules
4. Verify HTTP requests are enabled (see above)
5. Some URLs require TLS 1.2+ (automatic in most cases)

### Script Action Timeout

**Error**: `Action timed out`

**Solutions**:
1. Increase timeout: `timeout_ms = 60000`
2. Optimize script logic
3. Avoid long-running operations
4. Use async patterns where possible

## Metrics & Dashboard Issues

### Metrics Showing 0

**Problem**: Dashboard metrics stay at 0

**Solutions**:
1. Wait a few seconds for first metrics collection (2-second interval)
2. Trigger an event to generate metrics
3. Check engine is processing events
4. Verify rules are matching

### Event Stream Empty

**Problem**: "No events yet..." message persists

**Checklist**:
1. ✅ Engine is running
2. ✅ Event sources are configured
3. ✅ Rules are enabled
4. ✅ Try creating a test file manually
5. ✅ Check if filtering is applied

## Service Issues

### Service Won't Start

**Error**: `The service did not respond to the start or control request`

**Solutions**:
1. Check Windows Event Viewer
2. Verify config file exists and is valid
3. Check file permissions on config
4. Try running manually first: `WinEventEngine.exe -c config.toml`

### Service Stops Unexpectedly

**Problem**: Service stops after starting

**Solutions**:
1. Check Windows Event Viewer
2. Look for panic messages in logs
3. Verify all paths in config exist
4. Test config manually before installing service

### Cannot Install Service

**Error**: "Administrator privileges required"

**Solution**: Must run as Administrator to install Windows services
1. Right-click WinEventEngine.exe
2. Select "Run as administrator"
3. Go to Settings tab
4. Click Install Service

## Import/Export Issues

### Import Fails

**Error**: "Failed to import rules" or validation errors

**Solutions**:
1. Check JSON file is valid (use online JSON validator)
2. Verify rules don't have duplicate names
3. Ensure all referenced scripts exist
4. Check file encoding (should be UTF-8)

### Export Creates Empty File

**Problem**: Exported file has no content

**Solutions**:
1. Verify you have rules to export
2. Check file permissions in destination folder
3. Try a different location (e.g., Desktop)
4. Ensure disk has free space

## Getting Help

### Enable Debug Logging

```toml
[engine]
log_level = "debug"
```

Or in GUI: Settings → (when log level config added)

### Check Logs

**Console Mode:**
Run from terminal to see logs:
```cmd
WinEventEngine.exe -c config.toml
```

**Service Mode:**
Service logs go to Windows Event Viewer:
1. Open Event Viewer
2. Windows Logs → Application
3. Look for "WinEventEngine" source

### Generate Debug Info

```cmd
WinEventEngine.exe --version
WinEventEngine.exe -c config.toml --dry-run
```

### Report Issues

When reporting issues, include:
1. Engine version (`WinEventEngine.exe --version`)
2. Windows version
3. Config file (remove sensitive data)
4. Relevant log excerpts
5. Steps to reproduce
6. GUI or CLI mode?

## Quick Fixes

| Problem | Quick Fix |
|---------|-----------|
| Won't start | Run as Administrator |
| No events | Check `enabled = true` in rules and sources |
| Script fails | Test with simple `log` action first |
| High CPU | Disable unused sources |
| Service fails | Check Event Viewer logs |
| HTTP blocked | Enable in Settings → Security |
| GUI frozen | Restart application |
| Import fails | Validate JSON format |

## See Also

- [Configuration Reference](Configuration-Reference) - Valid config options
- [GUI Guide](GUI-Guide) - Using the native GUI
- [Lua Scripting API](Lua-Scripting-API) - Debugging scripts
- [Architecture](Architecture) - Technical details
