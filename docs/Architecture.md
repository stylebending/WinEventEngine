# Architecture

Technical deep-dive into the WinEventEngine architecture.

## Table of Contents

- [Overview](#overview)
- [System Architecture](#system-architecture)
- [Component Details](#component-details)
- [Data Flow](#data-flow)
- [Event Sources](#event-sources)
- [Rule Engine](#rule-engine)
- [Action System](#action-system)
- [Lua Scripting Integration](#lua-scripting-integration)
- [GUI Architecture](#gui-architecture)
- [Metrics & Monitoring](#metrics--monitoring)
- [Security Model](#security-model)
- [Performance Considerations](#performance-considerations)

## Overview

WinEventEngine is built with a modular, event-driven architecture using Rust's async ecosystem (Tokio). The system decouples event generation from processing through an internal event bus, enabling high throughput and reliability.

## System Architecture

```
┌─────────────────────────────────────────────────────────┐
│              Windows Event Automation Engine            │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Event Sources (Plugins)                                │
│  ├─ File Watcher ───┐                                  │
│  ├─ Window Watcher ─┼──┐                                │
│  ├─ Process Monitor─┼──┼──┐                             │
│  └─ Registry Monitor┴──┼──┘                             │
│                        │                                │
│  Event Bus (Tokio mpsc channels)                        │
│                        │                                │
│  Rule Engine            │                                │
│  ├─ Pattern Matcher    │                                │
│  └─ Lua Evaluator ─────┘                                │
│                        │                                │
│  Action Executor        │                                │
│  ├─ ExecuteCommand      │                                │
│  ├─ PowerShell          │                                │
│  ├─ HTTP Request        │                                │
│  ├─ Log                 │                                │
│  └─ Lua Script ─────────┘                                │
│                                                         │
│  Supporting Systems                                     │
│  ├─ Configuration (TOML) + Hot Reload                   │
│  ├─ Metrics Collector (1h sliding window)               │
│  ├─ Windows Service Integration                         │
│  └─ Native GUI (Iced)                                   │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

## Component Details

### Event Sources

Event sources are implemented as plugins implementing the `EventSourcePlugin` trait:

```rust
#[async_trait]
pub trait EventSourcePlugin: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&mut self, emitter: EventEmitter) -> Result<(), PluginError>;
    async fn stop(&mut self) -> Result<(), PluginError>;
}
```

**Current Implementations:**

1. **File Watcher** (`notify` crate)
   - OS-specific file system notifications
   - Pattern matching with glob syntax
   - Recursive directory watching
   - Debouncing to handle rapid successive events

2. **Window Watcher** (Win32 API)
   - Windows event hooks (WH_SHELL, WH_CBT)
   - Tracks window creation, destruction, focus changes
   - Gets window title, class, executable path, PID

3. **Process Monitor** (ETW - Event Tracing for Windows)
   - Kernel-level process events
   - Real-time process start/stop notifications
   - Captures command line, parent PID, session ID
   - **Requires Administrator privileges**

4. **Registry Monitor** (ETW)
   - Kernel-level registry access monitoring
   - Tracks key/value creation, modification, deletion
   - Process context for each operation
   - **Requires Administrator privileges**

### Event Bus

The event bus uses Tokio's multi-producer, single-consumer (mpsc) channels:

```rust
pub type EventEmitter = mpsc::Sender<Event>;
pub type EventReceiver = mpsc::Receiver<Event>;
```

**Design Decisions:**
- **Backpressure handling**: Bounded channels with configurable buffer size
- **Event dropping**: Old events dropped if buffer full (logged)
- **Clone-on-send**: Events are cloned for each rule evaluation

**Buffer Configuration:**
```toml
[engine]
event_buffer_size = 1000  # Default: 1000 events
```

### Rule Engine

The rule engine evaluates events against configured rules:

**Matching Process:**
1. Event arrives from bus
2. Each rule's trigger is evaluated
3. If trigger matches, associated actions execute
4. Actions execute sequentially (order matters)

**Trigger Types:**
- Pattern matching (glob syntax)
- Process name matching
- Window title substring matching
- Composite AND/OR logic

**Example Flow:**
```
Event: FileCreated { path: "C:/Data/input.csv" }
  ↓
Rule: "process_csv"
  Trigger: file_created + pattern "*.csv"
  ✓ MATCH
  ↓
Action: Execute "python process.py {{EVENT_PATH}}"
```

### Action System

Actions implement the `Action` trait:

```rust
pub trait Action: Send + Sync {
    fn execute(&self, event: &Event) -> Result<ActionResult, ActionError>;
    fn description(&self) -> String;
    fn clone_box(&self) -> Box<dyn Action>;
}
```

**Action Types:**

1. **Execute** - Run external commands
   - Working directory support
   - Argument passing
   - Environment variable injection

2. **PowerShell** - Execute PowerShell scripts
   - Script content or file path
   - Variable injection via environment
   - Error stream capture

3. **HTTP Request** - Webhook integration
   - GET/POST/PUT/DELETE/PATCH
   - Custom headers
   - Body templating with event data
   - **Security**: Must be enabled in settings (disabled by default)
   - **Variables**: Response available via `HTTP_STATUS_CODE`, `HTTP_RESPONSE_BODY`, `HTTP_SUCCESS`

4. **Log** - Structured logging
   - Debug/Info/Warn/Error levels
   - Message templating
   - Contextual event data

5. **Script (Lua)** - Custom scripting
   - Sandboxed execution environment
   - Rich API (logging, HTTP, JSON, filesystem)
   - 30-second timeout (configurable)
   - Error handling strategies

## Data Flow

### Event Processing Pipeline

```
┌──────────────┐
│  OS Event    │  (File created, Process started, etc.)
└──────┬───────┘
       │
┌──────▼───────┐
│ Event Source │  (Translate OS event to Event struct)
│   Plugin     │
└──────┬───────┘
       │ mpsc::send()
┌──────▼───────┐
│  Event Bus   │  (Channel buffer)
└──────┬───────┘
       │ mpsc::recv()
┌──────▼───────┐
│ Rule Engine  │  (Evaluate triggers)
└──────┬───────┘
       │
┌──────▼───────┐
│   Action     │  (Execute matched actions)
│  Executor    │
└──────┬───────┘
       │
┌──────▼───────┐
│   Result     │  (Log success/failure)
└──────────────┘
```

**Latency Breakdown:**
- OS event to plugin: <1ms (kernel notification)
- Plugin to bus: <1ms (async send)
- Rule evaluation: 1-10ms (depends on complexity)
- Action execution: 10ms-30s (depends on action)
- Total typical: 10-100ms

## Lua Scripting Integration

### Architecture

```
┌─────────────────────────────────────────┐
│           ScriptAction                  │
│  (Rust - implements Action trait)       │
└──────────────┬──────────────────────────┘
               │ Creates fresh Lua state
┌──────────────▼──────────────────────────┐
│           Lua VM (mlua)                 │
│  - Sandboxed environment                │
│  - Script loaded & executed             │
│  - API bindings (log, http, etc.)       │
└──────────────┬──────────────────────────┘
               │ Calls user function
┌──────────────▼──────────────────────────┐
│         User Script                     │
│  function on_event(event)               │
│    -- Custom logic                      │
│    return {success = true}              │
│  end                                    │
└─────────────────────────────────────────┘
```

### Security Model

**Sandbox Restrictions:**
- ❌ No `dofile`, `loadfile`, `require` (can't load external code)
- ❌ No `io` library (direct file access)
- ❌ No `os` library (except safe functions)
- ❌ No `debug` library
- ❌ No loading of C modules
- ✅ File operations via restricted API
- ✅ HTTP only (no raw sockets)
- ✅ 30-second execution timeout
- ✅ Fresh Lua state per execution

**API Safety:**
- File operations restricted to safe directories
- HTTP requests limited to standard methods
- Command execution via controlled API
- All I/O goes through Rust-implemented APIs

## GUI Architecture

The native GUI is built with [Iced](https://iced.rs/), a cross-platform GUI library for Rust.

```
┌─────────────────────────────────────────┐
│           WinEventApp                   │
│  (Main Application State)               │
└──────────────┬──────────────────────────┘
               │
       ┌───────┴───────┐
       │               │
┌──────▼──────┐ ┌──────▼──────┐
│   Views     │ │   State     │
│             │ │             │
│ - Dashboard │ │ - Engine    │
│ - Rules     │ │ - Events    │
│ - Sources   │ │ - Metrics   │
│ - Tester    │ │ - Config    │
│ - Settings  │ │ - Theme     │
└─────────────┘ └─────────────┘
```

**GUI Features:**
- **Dashboard**: Real-time event stream, metrics cards, status indicators
- **Rules**: Create, edit, delete, enable/disable automation rules
- **Sources**: Configure and manage event source plugins
- **Event Tester**: Test rules against sample events with JSON input
- **Settings**: Theme selection, service management, HTTP security toggle

**Event Broadcasting:**
```rust
// Engine broadcasts events to GUI
tokio::sync::broadcast::channel(100)

// GUI subscribes and updates in real-time
```

**Update Flow:**
1. Engine processes event
2. Event broadcast to all subscribers
3. GUI receives event via broadcast channel
4. UI updates immediately (no polling)

## Metrics & Monitoring

### Metrics Collection

```rust
pub struct MetricsCollector {
    counters: DashMap<String, AtomicU64>,
    gauges: DashMap<String, AtomicU64>,
    histograms: DashMap<String, Vec<(DateTime<Utc>, u64)>>,
    // ... sliding window cleanup
}
```

**Collected Metrics:**
- `events_total` - Events by source and type
- `events_dropped_total` - Dropped due to full buffer
- `events_processing_duration_seconds` - Processing latency
- `rules_evaluated_total` - Rule evaluations
- `rules_matched_total` - Successful matches
- `actions_executed_total` - Actions by result
- `plugins_events_generated_total` - Events per plugin
- `engine_uptime_seconds` - Engine uptime

**Retention:**
- Regular metrics: 1 hour (sliding window)
- Error metrics: 24 hours
- Cleanup runs every 5 minutes

**GUI Dashboard:**
- Real-time metrics display
- Event counter cards
- Last update timestamp
- Auto-refresh every 2 seconds

## Security Model

### Process Isolation

- **Lua scripts**: Run in isolated Lua state, fresh per execution
- **Command execution**: Spawns separate process
- **File access**: Restricted API only, path validation
- **Network**: HTTP/HTTPS only via controlled API

### HTTP Request Security

**Disabled by Default:**
```toml
[engine]
http_requests_enabled = false  # Security: must explicitly enable
```

**Why Disabled by Default:**
- Prevents malicious rules from calling external APIs
- Protects against data exfiltration
- User must consciously enable

**Enabling HTTP:**
- Via GUI: Settings → Security → "Allow HTTP Request Actions"
- Via Config: Set `http_requests_enabled = true`
- Requires rule rebuild to take effect

### Privilege Requirements

| Component | Normal User | Administrator |
|-----------|-------------|---------------|
| File Watcher | ✓ | ✓ |
| Window Watcher | ✓ | ✓ |
| Process Monitor | ✗ | ✓ |
| Registry Monitor | ✗ | ✓ |
| Windows Service | ✗ | ✓ |
| HTTP Requests (toggle) | ✓ | ✓ |

### Data Protection

- Config file: User's responsibility to secure
- Logs: May contain sensitive paths (configure log rotation)
- HTTP responses: Stored in environment variables (session only)

## Performance Considerations

### Throughput

**Tested Capacity:**
- File events: 1000+ events/second
- Process events: 500+ events/second
- Rule evaluations: 10,000+ evaluations/second
- Action execution: Depends on action type

**Bottlenecks:**
1. **File I/O**: Script actions with file operations
2. **Network latency**: HTTP request actions
3. **Process spawn**: Execute actions
4. **Lua overhead**: Script compilation (mitigated by caching)

### Memory Usage

**Typical (idle):**
- Engine: 20-30MB
- GUI: 50-80MB
- Lua runtime (per script): ~1MB
- Metrics: ~5MB (1h retention)
- Total: ~100MB

**Under load:**
- Event buffer: Configurable (default 1000 events)
- Per-event overhead: ~500 bytes
- Lua scripts: Freed after execution
- GUI event history: Last 50 events

### Optimization Tips

1. **Use specific patterns** instead of `*`:
   - Good: `*.txt`
   - Bad: `*` (monitors everything)

2. **Batch actions** with composite rules:
   - Better than multiple single-action rules

3. **Set appropriate timeouts**:
   - Don't use default 30s for quick scripts
   - Increase for long-running operations

4. **Filter early**:
   - Use source filters before rule evaluation
   - Pattern match in trigger, not in Lua

5. **Monitor metrics**:
   - Watch `events_dropped_total`
   - Increase buffer size if dropping

## Technology Stack

- **Language**: Rust (2024 edition)
- **Async Runtime**: Tokio
- **GUI Framework**: Iced
- **Lua**: mlua (Lua 5.4)
- **Serialization**: serde + toml
- **Logging**: tracing
- **Windows API**: windows-rs
- **Build Tools**: cargo, winres (Windows resources)

## See Also

- [Configuration Reference](Configuration-Reference)
- [GUI Guide](GUI-Guide)
- [Event Types](Event-Types)
- [Lua Scripting API](Lua-Scripting-API)
