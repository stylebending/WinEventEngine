use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::Html,
    routing::{delete, get, post, put},
    Json, Router,
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::time::{interval, Duration};
use tracing::{error, info};

use crate::{MetricUpdate, MetricsCollector, MetricsSnapshot};

pub trait RuleManager: Send + Sync {
    fn get_rules(&self) -> Vec<serde_json::Value>;
    fn add_rule(&self, rule: serde_json::Value) -> Result<serde_json::Value, String>;
    fn update_rule(&self, name: &str, rule: serde_json::Value) -> Result<serde_json::Value, String>;
    fn delete_rule(&self, name: &str) -> Result<(), String>;
    fn enable_rule(&self, name: &str, enabled: bool) -> Result<(), String>;
    fn validate_rule(&self, rule: serde_json::Value) -> Result<(), String>;
    fn test_rule_match(&self, rule: serde_json::Value, event_json: &str) -> Result<bool, String>;
    fn export_rules(&self) -> Result<String, String>;
    fn import_rules(&self, content: &str) -> Result<usize, String>;
}

pub struct ApiState {
    pub metrics: Arc<MetricsCollector>,
    pub rule_manager: Option<Arc<dyn RuleManager>>,
}

impl ApiState {
    pub fn new(metrics: Arc<MetricsCollector>) -> Self {
        Self {
            metrics,
            rule_manager: None,
        }
    }

    pub fn with_rule_manager<M: RuleManager + 'static>(mut self, manager: M) -> Self {
        self.rule_manager = Some(Arc::new(manager));
        self
    }
}

/// HTTP server for serving metrics with WebSocket support
pub struct MetricsServer {
    state: Arc<ApiState>,
    port: u16,
}

impl MetricsServer {
    /// Create a new metrics server
    pub fn new(collector: Arc<MetricsCollector>, port: u16) -> Self {
        let state = Arc::new(ApiState::new(collector));
        Self { state, port }
    }

    /// Create a new metrics server with rule manager
    pub fn with_rule_manager<M: RuleManager + 'static>(collector: Arc<MetricsCollector>, port: u16, manager: M) -> Self {
        let state = Arc::new(ApiState::new(collector).with_rule_manager(manager));
        Self { state, port }
    }

    /// Start the HTTP server
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let app = Router::new()
            .route("/", get(root_handler))
            .route("/automations", get(automations_handler))
            .route("/automations/new", get(automation_new_handler))
            .route("/test", get(test_handler))
            .route("/import-export", get(import_export_handler))
            .route("/ws", get(websocket_handler))
            .route("/api/rules", get(rules_list_handler))
            .route("/api/rules", post(rules_create_handler))
            .route("/api/rules/:name", get(rules_get_handler))
            .route("/api/rules/:name", put(rules_update_handler))
            .route("/api/rules/:name", delete(rules_delete_handler))
            .route("/api/rules/:name/enable", post(rules_enable_handler))
            .route("/api/rules/test", post(rules_validate_handler))
            .route("/api/rules/export", get(rules_export_handler))
            .route("/api/rules/import", post(rules_import_handler))
            .with_state(self.state.clone());

        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        info!("Starting metrics server on http://{}", addr);
        info!("WebSocket endpoint available at ws://{}/ws", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

/// WebSocket handler - upgrades HTTP to WebSocket connection
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ApiState>>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.metrics.clone()))
}

/// Handle WebSocket connection
async fn handle_socket(socket: WebSocket, collector: Arc<MetricsCollector>) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to metric updates
    let mut updates = collector.subscribe();

    // Send initial snapshot
    let initial_snapshot = MetricUpdate::Snapshot(collector.get_snapshot());
    if let Ok(json) = serde_json::to_string(&initial_snapshot) {
        let _ = sender.send(Message::Text(json)).await;
    }

    // Create periodic snapshot interval (every 5 seconds)
    let mut snapshot_interval = interval(Duration::from_secs(5));

    info!("New WebSocket client connected");

    loop {
        tokio::select! {
            // Receive broadcast updates from metrics collector
            Ok(update) = updates.recv() => {
                match serde_json::to_string(&update) {
                    Ok(json) => {
                        if sender.send(Message::Text(json)).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                    Err(e) => {
                        error!("Failed to serialize metric update: {}", e);
                    }
                }
            }

            // Send periodic snapshots
            _ = snapshot_interval.tick() => {
                let snapshot = MetricUpdate::Snapshot(collector.get_snapshot());
                if let Ok(json) = serde_json::to_string(&snapshot) {
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }

            // Handle client messages (ping/pong, commands)
            Some(Ok(msg)) = receiver.next() => {
                match msg {
                    Message::Close(_) => {
                        info!("WebSocket client disconnected");
                        break;
                    }
                    Message::Ping(data) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Message::Text(text) => {
                        handle_client_command(&text, &collector).await;
                    }
                    _ => {}
                }
            }

            else => break,
        }
    }
}

/// Handle optional client commands via WebSocket
async fn handle_client_command(text: &str, _collector: &MetricsCollector) {
    if let Ok(cmd) = serde_json::from_str::<ClientCommand>(text) {
        match cmd {
            ClientCommand::Ping => {
                // Pong sent automatically by protocol
            }
        }
    }
}

#[derive(Debug, Deserialize)]
enum ClientCommand {
    Ping,
}

/// Root handler - full dashboard HTML
async fn root_handler(State(_state): State<Arc<ApiState>>) -> Html<String> {
    Html(DASHBOARD_HTML.to_string())
}

/// Automations list page
async fn automations_handler(State(_state): State<Arc<ApiState>>) -> Html<String> {
    Html(AUTOMATIONS_HTML.to_string())
}

/// Query params for automation editor
#[derive(Deserialize)]
pub struct AutomationQuery {
    name: Option<String>,
}

/// New/Edit automation page
async fn automation_new_handler(Query(params): Query<AutomationQuery>) -> Html<String> {
    let html = if let Some(name) = params.name {
        // For editing, we'll add a script to fetch and populate the form
        let mut editor_html = AUTOMATION_EDITOR_HTML.to_string();
        let fetch_script = format!(r#"
        <script>
        document.addEventListener('DOMContentLoaded', async () => {{
            document.getElementById('pageTitle').textContent = 'Edit Automation: {}';
            const response = await fetch(`/api/rules/{}`);
            if (response.ok) {{
                const data = await response.json();
                if (data.success && data.data) {{
                    const rule = data.data;
                    document.getElementById('name').value = rule.name || '';
                    document.getElementById('name').readOnly = true;
                    document.getElementById('description').value = rule.description || '';
                    
                    // Set trigger type first
                    const triggerType = rule.trigger?.type || 'file_created';
                    document.getElementById('triggerType').value = triggerType;
                    updateTriggerFields();
                    
                    // Fill trigger fields based on type
                    if (rule.trigger) {{
                        if (triggerType.startsWith('file_') && rule.trigger.pattern) {{
                            document.getElementById('pattern').value = rule.trigger.pattern;
                        }}
                        if ((triggerType === 'window_focused' || triggerType === 'window_unfocused') && rule.trigger.title_contains) {{
                            document.getElementById('titleContains').value = rule.trigger.title_contains;
                        }}
                        if ((triggerType === 'process_started' || triggerType === 'process_stopped') && rule.trigger.process_name) {{
                            document.getElementById('processName').value = rule.trigger.process_name;
                        }}
                        if (triggerType === 'timer' && rule.trigger.interval_seconds) {{
                            document.getElementById('intervalSeconds').value = rule.trigger.interval_seconds;
                        }}
                        if (triggerType === 'registry_changed' && rule.trigger.value_name) {{
                            document.getElementById('valueName').value = rule.trigger.value_name;
                        }}
                    }}
                    
                    // Set action type
                    const actionType = rule.action?.type || 'log';
                    document.getElementById('actionType').value = actionType;
                    updateActionFields();
                    
                    // Fill action fields based on type
                    if (rule.action) {{
                        if (actionType === 'log') {{
                            if (rule.action.message) document.getElementById('logMessage').value = rule.action.message;
                            if (rule.action.level) document.getElementById('logLevel').value = rule.action.level;
                        }} else if (actionType === 'execute') {{
                            if (rule.action.command) document.getElementById('executeCommand').value = rule.action.command;
                            if (rule.action.args) document.getElementById('executeArgs').value = rule.action.args.join(' ');
                        }} else if (actionType === 'powershell') {{
                            if (rule.action.script) document.getElementById('powershellScript').value = rule.action.script;
                        }} else if (actionType === 'notify') {{
                            if (rule.action.title) document.getElementById('notifyTitle').value = rule.action.title;
                            if (rule.action.message) document.getElementById('notifyMessage').value = rule.action.message;
                        }} else if (actionType === 'http_request') {{
                            if (rule.action.url) document.getElementById('httpUrl').value = rule.action.url;
                            if (rule.action.method) document.getElementById('httpMethod').value = rule.action.method;
                            if (rule.action.body) document.getElementById('httpBody').value = rule.action.body;
                        }} else if (actionType === 'media') {{
                            if (rule.action.command) document.getElementById('mediaCommand').value = rule.action.command;
                        }}
                    }}
                    
                    document.getElementById('enabled').checked = rule.enabled !== false;
                }}
            }}
        }});
        </script>"#, name, name);
        
        // Insert the script before the closing </body> tag
        if let Some(pos) = editor_html.rfind("</body>") {
            editor_html.insert_str(pos, &fetch_script);
        }
        editor_html
    } else {
        AUTOMATION_EDITOR_HTML.to_string()
    };
    
    Html(html)
}

/// Test rule page
async fn test_handler(State(_state): State<Arc<ApiState>>) -> Html<String> {
    Html(TEST_RULE_HTML.to_string())
}

/// Import/Export page
async fn import_export_handler(State(_state): State<Arc<ApiState>>) -> Html<String> {
    Html(IMPORT_EXPORT_HTML.to_string())
}

/// Full dashboard HTML with embedded CSS and JavaScript
const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>WinEventEngine Dashboard</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.1/dist/chart.umd.min.js"></script>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #0f172a;
            color: #e2e8f0;
            min-height: 100vh;
        }

        .header {
            background: linear-gradient(135deg, #1e293b 0%, #0f172a 100%);
            padding: 1rem 2rem;
            border-bottom: 1px solid #334155;
            display: flex;
            align-items: center;
            gap: 1rem;
        }

        .header h1 {
            font-size: 1.5rem;
            color: #f8fafc;
            display: flex;
            align-items: center;
            gap: 0.5rem;
            margin-right: auto;
        }

        .header .connection-status {
            display: flex;
            align-items: center;
            gap: 0.5rem;
            font-size: 0.875rem;
            color: #94a3b8;
        }

        .header .status-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #ef4444;
            transition: background 0.3s;
        }

        .header .status-dot.connected {
            background: #22c55e;
        }

        .nav {
            display: flex;
            gap: 0.5rem;
        }

        .nav a {
            color: #94a3b8;
            text-decoration: none;
            padding: 0.5rem 1rem;
            border-radius: 6px;
            font-size: 0.875rem;
            transition: all 0.2s;
        }

        .nav a:hover {
            background: #334155;
            color: #f8fafc;
        }

        .nav a.active {
            background: #3b82f6;
            color: #fff;
        }

        .container {
            max-width: 1400px;
            margin: 0 auto;
            padding: 2rem;
        }

        .grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 1.5rem;
            margin-bottom: 2rem;
        }

        .card {
            background: #1e293b;
            border-radius: 12px;
            padding: 1.5rem;
            border: 1px solid #334155;
        }

        .card h3 {
            font-size: 0.875rem;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            color: #94a3b8;
            margin-bottom: 1rem;
        }

        .metric-value {
            font-size: 2.5rem;
            font-weight: 700;
            color: #f8fafc;
        }

        .metric-value.success { color: #22c55e; }
        .metric-value.warning { color: #f59e0b; }
        .metric-value.error { color: #ef4444; }

        .chart-container {
            position: relative;
            height: 200px;
            margin-top: 1rem;
        }

        .event-log {
            max-height: 400px;
            overflow-y: auto;
        }

        .event-item {
            padding: 0.75rem;
            border-bottom: 1px solid #334155;
            font-size: 0.875rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .event-item:last-child {
            border-bottom: none;
        }

        .event-time {
            color: #64748b;
            font-size: 0.75rem;
        }

        .event-type {
            background: #334155;
            padding: 0.25rem 0.5rem;
            border-radius: 4px;
            font-size: 0.75rem;
        }

        .filters {
            display: flex;
            gap: 0.5rem;
            margin-bottom: 1rem;
        }

        .filter-btn {
            background: #334155;
            border: none;
            color: #e2e8f0;
            padding: 0.5rem 1rem;
            border-radius: 6px;
            cursor: pointer;
            font-size: 0.875rem;
            transition: background 0.2s;
        }

        .filter-btn:hover, .filter-btn.active {
            background: #475569;
        }

        @media (max-width: 768px) {
            .container {
                padding: 1rem;
            }
            .grid {
                grid-template-columns: 1fr;
            }
        }
    </style>
</head>
<body>
    <div class="header">
        <h1>WinEventEngine <span class="connection-status"><span class="status-dot" id="statusDot"></span><span id="statusText">Connecting...</span></span></h1>
        <nav class="nav">
            <a href="/" class="active">Dashboard</a>
            <a href="/automations">Automations</a>
            <a href="/test">Test Rules</a>
            <a href="/import-export">Import/Export</a>
        </nav>
    </div>

    <div class="container">
        <!-- Top Metrics Row -->
        <div class="grid">
            <div class="card">
                <h3>Events/sec</h3>
                <div class="metric-value" id="eventsPerSec">0</div>
                <div class="chart-container">
                    <canvas id="eventsChart"></canvas>
                </div>
            </div>
            <div class="card">
                <h3>Rule Matches/sec</h3>
                <div class="metric-value" id="matchesPerSec">0</div>
                <div class="chart-container">
                    <canvas id="matchesChart"></canvas>
                </div>
            </div>
            <div class="card">
                <h3>Actions Executed</h3>
                <div class="metric-value success" id="actionsCount">0</div>
                <div class="chart-container">
                    <canvas id="actionsChart"></canvas>
                </div>
            </div>
            <div class="card">
                <h3>System Health</h3>
                <div class="metric-value" id="uptime">0s</div>
                <div style="margin-top: 1rem; font-size: 0.875rem; color: #94a3b8;">
                    <div>Plugins: <span id="pluginCount">0</span></div>
                    <div>Rules: <span id="ruleCount">0</span></div>
                </div>
            </div>
        </div>

        <!-- Event Log -->
        <div class="card">
            <h3>Live Event Stream</h3>
            <div class="filters">
                <button class="filter-btn active" onclick="filterEvents('all')">All</button>
                <button class="filter-btn" onclick="filterEvents('event')">Events</button>
                <button class="filter-btn" onclick="filterEvents('rule')">Rules</button>
                <button class="filter-btn" onclick="filterEvents('action')">Actions</button>
            </div>
            <div class="event-log" id="eventLog">
                <div style="text-align: center; color: #64748b; padding: 2rem;">
                    Waiting for events...
                </div>
            </div>
        </div>
    </div>

    <script>
        // WebSocket connection management
        let ws = null;
        let reconnectInterval = 1000;
        let maxReconnectInterval = 30000;
        let eventBuffer = [];
        let currentFilter = 'all';

        // Chart instances
        let eventsChart, matchesChart, actionsChart;

        // Data buffers for charts (keep last 60 seconds)
        const eventsData = new Array(60).fill(0);
        const matchesData = new Array(60).fill(0);
        const actionsData = new Array(60).fill(0);

        // Initialize charts
        function initCharts() {
            const chartOptions = {
                responsive: true,
                maintainAspectRatio: false,
                plugins: { legend: { display: false } },
                scales: {
                    x: { display: false },
                    y: {
                        beginAtZero: true,
                        grid: { color: '#334155' },
                        ticks: { color: '#94a3b8', font: { size: 10 } }
                    }
                },
                elements: {
                    line: { tension: 0.4 },
                    point: { radius: 0 }
                }
            };

            eventsChart = new Chart(document.getElementById('eventsChart'), {
                type: 'line',
                data: {
                    labels: new Array(60).fill(''),
                    datasets: [{
                        data: eventsData,
                        borderColor: '#3b82f6',
                        backgroundColor: 'rgba(59, 130, 246, 0.1)',
                        fill: true,
                        borderWidth: 2
                    }]
                },
                options: chartOptions
            });

            matchesChart = new Chart(document.getElementById('matchesChart'), {
                type: 'line',
                data: {
                    labels: new Array(60).fill(''),
                    datasets: [{
                        data: matchesData,
                        borderColor: '#22c55e',
                        backgroundColor: 'rgba(34, 197, 94, 0.1)',
                        fill: true,
                        borderWidth: 2
                    }]
                },
                options: chartOptions
            });

            actionsChart = new Chart(document.getElementById('actionsChart'), {
                type: 'bar',
                data: {
                    labels: ['Success', 'Error'],
                    datasets: [{
                        data: [0, 0],
                        backgroundColor: ['#22c55e', '#ef4444'],
                        borderWidth: 0
                    }]
                },
                options: {
                    ...chartOptions,
                    scales: {
                        y: { beginAtZero: true, grid: { display: false }, ticks: { display: false } },
                        x: { grid: { display: false }, ticks: { color: '#94a3b8', font: { size: 10 } } }
                    }
                }
            });
        }
 
        // WebSocket connection - use sessionStorage to prevent multiple connections
        let ws = null;
        let reconnectInterval = 1000;
        const maxReconnectInterval = 30000;
        let isConnecting = false;

        function connect() {
            // Prevent multiple simultaneous connection attempts
            if (isConnecting) {
                console.log('Already connecting, skipping...');
                return;
            }
            
            // Check sessionStorage for existing connection
            try {
                const wasConnected = sessionStorage.getItem('ws_connected');
                // If we were connected recently, wait a bit before reconnecting
                if (wasConnected === 'true' && ws && ws.readyState === WebSocket.OPEN) {
                    console.log('WebSocket already connected');
                    return;
                }
            } catch(e) {}

            isConnecting = true;
            
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${protocol}//${window.location.host}/ws`;

            try {
                ws = new WebSocket(wsUrl);
            } catch(e) {
                isConnecting = false;
                console.error('Failed to create WebSocket:', e);
                return;
            }

            ws.onopen = () => {
                console.log('WebSocket connected');
                updateStatus(true);
                reconnectInterval = 1000;
                isConnecting = false;
                try {
                    sessionStorage.setItem('ws_connected', 'true');
                } catch(e) {}
            };

            ws.onmessage = (event) => {
                try {
                    const data = JSON.parse(event.data);
                    handleMessage(data);
                    
                    // Update uptime from health messages
                    if (data.type === 'health' && data.data && data.data.uptime_seconds !== undefined) {
                        const uptimeEl = document.getElementById('uptime');
                        if (uptimeEl) {
                            uptimeEl.textContent = formatUptime(data.data.uptime_seconds);
                        }
                    }
                    if (data.type === 'snapshot' && data.gauges && data.gauges['engine_uptime_seconds'] !== undefined) {
                        const uptimeEl = document.getElementById('uptime');
                        if (uptimeEl) {
                            uptimeEl.textContent = formatUptime(data.gauges['engine_uptime_seconds']);
                        }
                    }
                } catch (e) {
                    console.error('Failed to parse message:', e);
                }
            };

            ws.onclose = () => {
                console.log('WebSocket disconnected');
                updateStatus(false);
                isConnecting = false;
                try {
                    sessionStorage.setItem('ws_connected', 'false');
                } catch(e) {}
                // Add delay before reconnecting
                setTimeout(() => {
                    scheduleReconnect();
                }, 500);
            };

            ws.onerror = (error) => {
                console.error('WebSocket error:', error);
                ws.close();
            };
        }

        function formatUptime(seconds) {
            if (!seconds || seconds < 0) return '0s';
            const h = Math.floor(seconds / 3600);
            const m = Math.floor((seconds % 3600) / 60);
            const s = Math.floor(seconds % 60);
            if (h > 0) return `${h}h ${m}m`;
            if (m > 0) return `${m}m ${s}s`;
            return `${s}s`;
        }

        function scheduleReconnect() {
            setTimeout(() => {
                console.log(`Reconnecting in ${reconnectInterval}ms...`);
                reconnectInterval = Math.min(reconnectInterval * 2, maxReconnectInterval);
                connect();
            }, reconnectInterval);
        }

        // Update connection status UI
        function updateStatus(connected) {
            const dot = document.getElementById('statusDot');
            const text = document.getElementById('statusText');

            if (connected) {
                dot.classList.add('connected');
                text.textContent = 'Connected';
            } else {
                dot.classList.remove('connected');
                text.textContent = 'Disconnected';
            }
        }

        // Handle incoming WebSocket messages
        let eventCount = 0;
        let matchCount = 0;
        let actionSuccessCount = 0;
        let actionErrorCount = 0;
        let lastSecondEvents = 0;
        let lastSecondMatches = 0;

        function handleMessage(data) {
            switch(data.type) {
                case 'event_received':
                    lastSecondEvents++;
                    addEventToLog('event', `Event from ${data.data.source}`, data.data.event_type);
                    break;

                case 'rule_matched':
                    lastSecondMatches++;
                    addEventToLog('rule', `Rule matched: ${data.data.rule_name}`, 'match');
                    break;

                case 'action_executed':
                    if (data.data.success) {
                        actionSuccessCount++;
                    } else {
                        actionErrorCount++;
                    }
                    addEventToLog('action', `Action: ${data.data.action_name}`,
                        data.data.success ? 'success' : 'error');
                    break;

                case 'snapshot':
                    updateSnapshot(data.data);
                    break;
            }
        }

        // Add event to live log
        function addEventToLog(type, message, detail) {
            const log = document.getElementById('eventLog');

            if (log.children.length === 1 && log.children[0].style.textAlign === 'center') {
                log.innerHTML = '';
            }

            const item = document.createElement('div');
            item.className = 'event-item';
            item.dataset.type = type;

            const time = new Date().toLocaleTimeString();
            item.innerHTML = `
                <div>
                    <span class="event-type">${type}</span>
                    ${message}
                </div>
                <div style="text-align: right;">
                    <div style="color: #94a3b8; font-size: 0.75rem;">${detail}</div>
                    <div class="event-time">${time}</div>
                </div>
            `;

            log.insertBefore(item, log.firstChild);

            while (log.children.length > 100) {
                log.removeChild(log.lastChild);
            }

            applyFilter();
        }

        // Filter events in the log
        function filterEvents(type) {
            currentFilter = type;

            document.querySelectorAll('.filter-btn').forEach(btn => {
                btn.classList.remove('active');
                if (btn.textContent.toLowerCase().includes(type) ||
                    (type === 'all' && btn.textContent === 'All')) {
                    btn.classList.add('active');
                }
            });

            applyFilter();
        }

        function applyFilter() {
            document.querySelectorAll('.event-item').forEach(item => {
                if (currentFilter === 'all' || item.dataset.type === currentFilter) {
                    item.style.display = 'flex';
                } else {
                    item.style.display = 'none';
                }
            });
        }

        // Update UI from snapshot
        function updateSnapshot(snapshot) {
            document.getElementById('pluginCount').textContent =
                snapshot.gauges && snapshot.gauges.active_plugins ? snapshot.gauges.active_plugins : 0;
            document.getElementById('ruleCount').textContent =
                snapshot.gauges && snapshot.gauges.active_rules ? snapshot.gauges.active_rules : 0;
        }

        // Update charts and metrics every second
        setInterval(() => {
            document.getElementById('eventsPerSec').textContent = lastSecondEvents;
            document.getElementById('matchesPerSec').textContent = lastSecondMatches;

            eventsData.shift();
            eventsData.push(lastSecondEvents);
            eventsChart.update('none');

            matchesData.shift();
            matchesData.push(lastSecondMatches);
            matchesChart.update('none');

            document.getElementById('actionsCount').textContent =
                actionSuccessCount + actionErrorCount;
            actionsChart.data.datasets[0].data = [actionSuccessCount, actionErrorCount];
            actionsChart.update('none');

            lastSecondEvents = 0;
            lastSecondMatches = 0;

            const uptime = document.getElementById('uptime');
            const current = parseInt(uptime.textContent) || 0;
            uptime.textContent = (current + 1) + 's';
        }, 1000);

        // Initialize
        document.addEventListener('DOMContentLoaded', () => {
            initCharts();
            connect();
        });

        // Cleanup on page unload
        window.addEventListener('beforeunload', () => {
            if (ws) {
                ws.close();
            }
        });
    </script>
</body>
</html>"#;

/// Automations management page HTML
const AUTOMATIONS_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>WinEventEngine - Automations</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #e2e8f0; min-height: 100vh; }
        .header { background: linear-gradient(135deg, #1e293b 0%, #0f172a 100%); padding: 1rem 2rem; border-bottom: 1px solid #334155; display: flex; align-items: center; gap: 1rem; }
        .header h1 { font-size: 1.5rem; color: #f8fafc; display: flex; align-items: center; gap: 0.5rem; margin-right: auto; }
        .header .connection-status { display: flex; align-items: center; gap: 0.5rem; font-size: 0.875rem; color: #94a3b8; }
        .header .status-dot { width: 8px; height: 8px; border-radius: 50%; background: #ef4444; transition: background 0.3s; }
        .header .status-dot.connected { background: #22c55e; }
        .nav { display: flex; gap: 0.5rem; }
        .nav a { color: #94a3b8; text-decoration: none; padding: 0.5rem 1rem; border-radius: 6px; font-size: 0.875rem; transition: all 0.2s; }
        .nav a:hover, .nav a.active { background: #3b82f6; color: #fff; }
        .container { max-width: 1200px; margin: 0 auto; padding: 2rem; }
        .page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 2rem; }
        .page-header h2 { font-size: 1.5rem; }
        .btn { background: #3b82f6; color: white; border: none; padding: 0.5rem 1rem; border-radius: 6px; cursor: pointer; font-size: 0.875rem; }
        .btn:hover { background: #2563eb; }
        .btn-danger { background: #ef4444; }
        .btn-danger:hover { background: #dc2626; }
        .btn-success { background: #22c55e; }
        .btn-success:hover { background: #16a34a; }
        table { width: 100%; border-collapse: collapse; }
        th, td { padding: 1rem; text-align: left; border-bottom: 1px solid #334155; }
        th { background: #1e293b; font-weight: 600; color: #94a3b8; }
        tr:hover { background: #1e293b; }
        .status { display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 0.5rem; }
        .status.enabled { background: #22c55e; }
        .status.disabled { background: #ef4444; }
        .actions { display: flex; gap: 0.5rem; }
        .actions button { padding: 0.25rem 0.5rem; font-size: 0.75rem; }
        .empty-state { text-align: center; padding: 4rem; color: #64748b; }
    </style>
</head>
<body>
    <div class="header">
        <h1>WinEventEngine <span class="connection-status"><span class="status-dot" id="statusDot"></span><span id="statusText">Connecting...</span></span></h1>
        <nav class="nav">
            <a href="/">Dashboard</a>
            <a href="/automations" class="active">Automations</a>
            <a href="/test">Test Rules</a>
            <a href="/import-export">Import/Export</a>
        </nav>
    </div>
    <div class="container">
        <div class="page-header">
            <h2>Automations</h2>
            <a href="/automations/new" class="btn">+ New Automation</a>
        </div>
        <table id="rulesTable">
            <thead>
                <tr>
                    <th>Status</th>
                    <th>Name</th>
                    <th>Trigger</th>
                    <th>Action</th>
                    <th>Actions</th>
                </tr>
            </thead>
            <tbody id="rulesBody"></tbody>
        </table>
        <div class="empty-state" id="emptyState">No automations found. Create one to get started.</div>
    </div>
    <script>
        async function loadRules() {
            const response = await fetch('/api/rules');
            const data = await response.json();
            const tbody = document.getElementById('rulesBody');
            const emptyState = document.getElementById('emptyState');
            
            if (data.success && data.data && data.data.length > 0) {
                tbody.innerHTML = data.data.map(rule => `
                    <tr>
                        <td><span class="status ${rule.enabled ? 'enabled' : 'disabled'}"></span></td>
                        <td>${rule.name}</td>
                        <td>${rule.trigger.type}</td>
                        <td>${rule.action.type}</td>
                        <td class="actions">
                            <button class="btn" onclick="toggleRule('${rule.name}', ${!rule.enabled})">${rule.enabled ? 'Disable' : 'Enable'}</button>
                            <a href="/automations/new?name=${encodeURIComponent(rule.name)}" class="btn">Edit</a>
                            <button class="btn btn-danger" onclick="deleteRule('${rule.name}')">Delete</button>
                        </td>
                    </tr>
                `).join('');
                emptyState.style.display = 'none';
            } else {
                tbody.innerHTML = '';
                emptyState.style.display = 'block';
            }
        }
        
        async function toggleRule(name, enabled) {
            await fetch(`/api/rules/${name}/enable`, {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({enabled})
            });
            loadRules();
        }
        
        async function deleteRule(name) {
            if (confirm(`Delete automation "${name}"?`)) {
                await fetch(`/api/rules/${name}`, {method: 'DELETE'});
                loadRules();
            }
        }
        
        loadRules();
        
        // WebSocket connection for status
        let ws = null;
        let reconnectInterval = 1000;
        const maxReconnectInterval = 30000;
        let isConnecting = false;

        function connect() {
            if (isConnecting) { console.log('Already connecting, skipping...'); return; }
            try { if (sessionStorage.getItem('ws_connected') === 'true' && ws && ws.readyState === WebSocket.OPEN) { console.log('WebSocket already connected'); return; } } catch(e) {}
            isConnecting = true;
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${protocol}//${window.location.host}/ws`;
            try { ws = new WebSocket(wsUrl); } catch(e) { isConnecting = false; console.error('Failed to create WebSocket:', e); return; }
            ws.onopen = () => { console.log('WebSocket connected'); updateStatus(true); reconnectInterval = 1000; isConnecting = false; try { sessionStorage.setItem('ws_connected', 'true'); } catch(e) {} };
            ws.onmessage = (event) => { try { const data = JSON.parse(event.data); } catch (e) { console.error('Failed to parse message:', e); } };
            ws.onclose = () => { console.log('WebSocket disconnected'); updateStatus(false); isConnecting = false; try { sessionStorage.setItem('ws_connected', 'false'); } catch(e) {} setTimeout(() => { scheduleReconnect(); }, 500); };
            ws.onerror = (error) => { console.error('WebSocket error:', error); ws.close(); };
        }

        function scheduleReconnect() {
            if (reconnectInterval > maxReconnectInterval) reconnectInterval = maxReconnectInterval;
            console.log(`Reconnecting in ${reconnectInterval}ms...`);
            reconnectInterval = Math.min(reconnectInterval * 2, maxReconnectInterval);
            connect();
        }

        function updateStatus(connected) {
            const dot = document.getElementById('statusDot');
            const text = document.getElementById('statusText');
            if (connected) { dot.classList.add('connected'); text.textContent = 'Connected'; } 
            else { dot.classList.remove('connected'); text.textContent = 'Disconnected'; }
        }

        document.addEventListener('DOMContentLoaded', () => { connect(); });

        window.addEventListener('beforeunload', () => {
            if (ws) {
                ws.close();
            }
        });
    </script>
</body>
</html>"#;

/// Automation editor page HTML
const AUTOMATION_EDITOR_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>WinEventEngine - Create Automation</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #e2e8f0; min-height: 100vh; }
        .header { background: linear-gradient(135deg, #1e293b 0%, #0f172a 100%); padding: 1rem 2rem; border-bottom: 1px solid #334155; display: flex; align-items: center; gap: 1rem; }
        .header h1 { font-size: 1.5rem; color: #f8fafc; display: flex; align-items: center; gap: 0.5rem; margin-right: auto; }
        .header .connection-status { display: flex; align-items: center; gap: 0.5rem; font-size: 0.875rem; color: #94a3b8; }
        .header .status-dot { width: 8px; height: 8px; border-radius: 50%; background: #ef4444; transition: background 0.3s; }
        .header .status-dot.connected { background: #22c55e; }
        .nav { display: flex; gap: 0.5rem; }
        .nav a { color: #94a3b8; text-decoration: none; padding: 0.5rem 1rem; border-radius: 6px; font-size: 0.875rem; transition: all 0.2s; }
        .nav a:hover, .nav a.active { background: #3b82f6; color: #fff; }
        .container { max-width: 800px; margin: 0 auto; padding: 2rem; }
        .form-group { margin-bottom: 1.5rem; }
        label { display: block; margin-bottom: 0.5rem; color: #94a3b8; font-size: 0.875rem; }
        input, select, textarea { width: 100%; padding: 0.75rem; background: #1e293b; border: 1px solid #334155; border-radius: 6px; color: #f8fafc; font-size: 1rem; }
        input:focus, select:focus, textarea:focus { outline: none; border-color: #3b82f6; }
        .btn { background: #3b82f6; color: white; border: none; padding: 0.75rem 1.5rem; border-radius: 6px; cursor: pointer; font-size: 1rem; margin-right: 1rem; text-decoration: none; display: inline-block; }
        .btn:hover { background: #2563eb; }
        .btn-secondary { background: #334155; }
        .btn-secondary:hover { background: #475569; }
        .section { background: #1e293b; padding: 1.5rem; border-radius: 12px; margin-bottom: 1.5rem; }
        .section h3 { margin-bottom: 1rem; color: #f8fafc; }
        .hint { font-size: 0.75rem; color: #64748b; margin-top: 0.25rem; }
    </style>
</head>
<body>
    <div class="header">
        <h1>WinEventEngine <span class="connection-status"><span class="status-dot" id="statusDot"></span><span id="statusText">Connecting...</span></span></h1>
        <nav class="nav">
            <a href="/">Dashboard</a>
            <a href="/automations" class="active">Automations</a>
            <a href="/test">Test Rules</a>
            <a href="/import-export">Import/Export</a>
        </nav>
    </div>
    <div class="container">
        <form id="ruleForm">
            <div class="section">
                <h3 id="pageTitle">Create Automation</h3>
                <h3>Basic Info</h3>
                <div class="form-group">
                    <label>Name</label>
                    <input type="text" name="name" id="name" required placeholder="my_automation">
                </div>
                <div class="form-group">
                    <label>Description (optional)</label>
                    <input type="text" name="description" id="description" placeholder="What does this automation do?">
                </div>
            </div>
            
            <div class="section">
                <h3>Trigger (When)</h3>
                <div class="form-group">
                    <label>Event Type</label>
                    <select name="triggerType" id="triggerType" onchange="updateFields()">
                        <option value="file_created">File Created</option>
                        <option value="file_modified">File Modified</option>
                        <option value="file_deleted">File Deleted</option>
                        <option value="window_focused">Window Focused</option>
                        <option value="window_unfocused">Window Unfocused</option>
                        <option value="window_created">Window Created</option>
                        <option value="process_started">Process Started</option>
                        <option value="process_stopped">Process Stopped</option>
                        <option value="registry_changed">Registry Changed</option>
                        <option value="timer">Timer</option>
                    </select>
                </div>
                <div class="form-group" id="pathGroup" style="display:none">
                    <label>Watch Path</label>
                    <input type="text" name="path" id="path" placeholder="C:\Users\You\Folder">
                    <div class="hint">Directory path to watch for file events</div>
                </div>
                <div class="form-group" id="patternGroup">
                    <label>File Pattern (optional)</label>
                    <input type="text" name="pattern" id="pattern" placeholder="*.txt, *.log, etc.">
                    <div class="hint">Glob pattern to match file paths</div>
                </div>
                <div class="form-group" id="titleContainsGroup" style="display:none">
                    <label>Window Title Contains</label>
                    <input type="text" name="titleContains" id="titleContains" placeholder="Part of window title">
                </div>
                <div class="form-group" id="processNameGroup" style="display:none">
                    <label>Process Name</label>
                    <input type="text" name="processName" id="processName" placeholder="chrome.exe, notepad.exe, etc.">
                </div>
                <div class="form-group" id="timerGroup" style="display:none">
                    <label>Interval (seconds)</label>
                    <input type="number" name="intervalSeconds" id="intervalSeconds" value="60" min="1">
                </div>
                <div class="form-group" id="valueNameGroup" style="display:none">
                    <label>Registry Value Name (optional)</label>
                    <input type="text" name="valueName" id="valueName" placeholder="ValueName">
                </div>
            </div>
            
            <div class="section">
                <h3>Action (Then)</h3>
                <div class="form-group">
                    <label>Action Type</label>
                    <select name="actionType" id="actionType" onchange="updateFields()">
                        <option value="log">Log Message</option>
                        <option value="execute">Execute Command</option>
                        <option value="powershell">PowerShell Script</option>
                        <option value="notify">Show Notification</option>
                        <option value="http_request">HTTP Request</option>
                        <option value="media">Media Control</option>
                    </select>
                </div>
                <div id="logFields">
                    <div class="form-group">
                        <label>Message</label>
                        <input type="text" name="logMessage" id="logMessage" placeholder="Log message to write">
                    </div>
                    <div class="form-group">
                        <label>Level</label>
                        <select name="logLevel" id="logLevel">
                            <option value="debug">Debug</option>
                            <option value="info" selected>Info</option>
                            <option value="warn">Warning</option>
                            <option value="error">Error</option>
                        </select>
                    </div>
                </div>
                <div id="executeFields" style="display:none">
                    <div class="form-group">
                        <label>Command</label>
                        <input type="text" name="executeCommand" id="executeCommand" placeholder="echo, cmd, etc.">
                    </div>
                    <div class="form-group">
                        <label>Arguments</label>
                        <input type="text" name="executeArgs" id="executeArgs" placeholder="arg1 arg2 arg3">
                    </div>
                    <div class="form-group">
                        <label>Working Directory (optional)</label>
                        <input type="text" name="executeWorkingDir" id="executeWorkingDir" placeholder="C:\path\to\dir">
                    </div>
                </div>
                <div id="powershellFields" style="display:none">
                    <div class="form-group">
                        <label>Script</label>
                        <textarea name="powershellScript" id="powershellScript" rows="4" placeholder="Write-Host 'Hello World'"></textarea>
                    </div>
                    <div class="form-group">
                        <label>Working Directory (optional)</label>
                        <input type="text" name="powershellWorkingDir" id="powershellWorkingDir" placeholder="C:\path\to\dir">
                    </div>
                </div>
                <div id="notifyFields" style="display:none">
                    <div class="form-group">
                        <label>Title</label>
                        <input type="text" name="notifyTitle" id="notifyTitle" placeholder="Notification title">
                    </div>
                    <div class="form-group">
                        <label>Message</label>
                        <input type="text" name="notifyMessage" id="notifyMessage" placeholder="Notification message">
                    </div>
                </div>
                <div id="httpRequestFields" style="display:none">
                    <div class="form-group">
                        <label>URL</label>
                        <input type="text" name="httpUrl" id="httpUrl" placeholder="https://example.com/webhook">
                    </div>
                    <div class="form-group">
                        <label>Method</label>
                        <select name="httpMethod" id="httpMethod">
                            <option value="GET">GET</option>
                            <option value="POST" selected>POST</option>
                            <option value="PUT">PUT</option>
                            <option value="DELETE">DELETE</option>
                        </select>
                    </div>
                    <div class="form-group">
                        <label>Body (optional)</label>
                        <textarea name="httpBody" id="httpBody" rows="3" placeholder='{"key": "value"}'></textarea>
                    </div>
                </div>
                <div id="mediaFields" style="display:none">
                    <div class="form-group">
                        <label>Command</label>
                        <select name="mediaCommand" id="mediaCommand">
                            <option value="play">Play/Pause</option>
                            <option value="next">Next Track</option>
                            <option value="previous">Previous Track</option>
                        </select>
                    </div>
                </div>
            </div>
            
            <div class="form-group">
                <label>
                    <input type="checkbox" name="enabled" id="enabled" checked> Enable this automation
                </label>
            </div>
            
            <button type="submit" class="btn" id="submitBtn">Save Automation</button>
            <a href="/automations" class="btn btn-secondary">Cancel</a>
        </form>
    </div>
    <script>
        function updateFields() {
            const triggerType = document.getElementById("triggerType").value;
            const actionType = document.getElementById("actionType").value;
            
            // Show/hide trigger fields
            document.getElementById("pathGroup").style.display = triggerType.startsWith("file") ? "block" : "none";
            document.getElementById("patternGroup").style.display = triggerType.startsWith("file") ? "block" : "none";
            document.getElementById("titleContainsGroup").style.display = (triggerType === "window_focused" || triggerType === "window_unfocused") ? "block" : "none";
            document.getElementById("processNameGroup").style.display = 
                (triggerType === "process_started" || triggerType === "process_stopped" || triggerType === "window_focused" || triggerType === "window_unfocused") ? "block" : "none";
            document.getElementById("timerGroup").style.display = triggerType === "timer" ? "block" : "none";
            document.getElementById("valueNameGroup").style.display = triggerType === "registry_changed" ? "block" : "none";
            
            // Show/hide action fields
            document.getElementById("logFields").style.display = actionType === "log" ? "block" : "none";
            document.getElementById("executeFields").style.display = actionType === "execute" ? "block" : "none";
            document.getElementById("powershellFields").style.display = actionType === "powershell" ? "block" : "none";
            document.getElementById("notifyFields").style.display = actionType === "notify" ? "block" : "none";
            document.getElementById("httpRequestFields").style.display = actionType === "http_request" ? "block" : "none";
            document.getElementById("mediaFields").style.display = actionType === "media" ? "block" : "none";
        }
        
        document.getElementById("ruleForm").addEventListener("submit", async (e) => {
            e.preventDefault();
            const formData = new FormData(e.target);
            
            const triggerType = formData.get("triggerType");
            let trigger = { type: triggerType };
            if (formData.get("pattern")) trigger.pattern = formData.get("pattern");
            if (formData.get("path")) trigger.path = formData.get("path");
            if (formData.get("titleContains")) trigger.title_contains = formData.get("titleContains");
            if (formData.get("processName")) trigger.process_name = formData.get("processName");
            if (formData.get("intervalSeconds")) trigger.interval_seconds = parseInt(formData.get("intervalSeconds"));
            if (formData.get("valueName")) trigger.value_name = formData.get("valueName");
            
            const actionType = formData.get("actionType");
            let action = { type: actionType };
            if (actionType === "log") {
                action.message = formData.get("logMessage");
                action.level = formData.get("logLevel");
            } else if (actionType === "execute") {
                action.command = formData.get("executeCommand");
                action.args = formData.get("executeArgs").split(" ").filter(a => a);
            } else if (actionType === "powershell") {
                action.script = formData.get("powershellScript");
            } else if (actionType === "notify") {
                action.title = formData.get("notifyTitle");
                action.message = formData.get("notifyMessage");
            } else if (actionType === "http_request") {
                action.url = formData.get("httpUrl");
                action.method = formData.get("httpMethod");
                action.body = formData.get("httpBody");
                action.headers = {};
            } else if (actionType === "media") {
                action.command = formData.get("mediaCommand");
            }
            
            const rule = {
                name: formData.get("name"),
                description: formData.get("description") || null,
                trigger,
                action,
                enabled: formData.get("enabled") === "on"
            };
            
            const response = await fetch("/api/rules", {
                method: "POST",
                headers: {"Content-Type": "application/json"},
                body: JSON.stringify(rule)
            });
            
            const data = await response.json();
            if (data.success) {
                window.location.href = "/automations";
            } else {
                alert("Error: " + (data.error || "Unknown error"));
            }
        });
        
        updateFields();
        
        // WebSocket connection for status
        let ws = null;
        let reconnectInterval = 1000;
        const maxReconnectInterval = 30000;
        let isConnecting = false;

        function connect() {
            if (isConnecting) { console.log('Already connecting, skipping...'); return; }
            try { if (sessionStorage.getItem('ws_connected') === 'true' && ws && ws.readyState === WebSocket.OPEN) { console.log('WebSocket already connected'); return; } } catch(e) {}
            isConnecting = true;
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${protocol}//${window.location.host}/ws`;
            try { ws = new WebSocket(wsUrl); } catch(e) { isConnecting = false; console.error('Failed to create WebSocket:', e); return; }
            ws.onopen = () => { console.log('WebSocket connected'); updateStatus(true); reconnectInterval = 1000; isConnecting = false; try { sessionStorage.setItem('ws_connected', 'true'); } catch(e) {} };
            ws.onmessage = (event) => { try { const data = JSON.parse(event.data); } catch (e) { console.error('Failed to parse message:', e); } };
            ws.onclose = () => { console.log('WebSocket disconnected'); updateStatus(false); isConnecting = false; try { sessionStorage.setItem('ws_connected', 'false'); } catch(e) {} setTimeout(() => { scheduleReconnect(); }, 500); };
            ws.onerror = (error) => { console.error('WebSocket error:', error); ws.close(); };
        }

        function scheduleReconnect() {
            if (reconnectInterval > maxReconnectInterval) reconnectInterval = maxReconnectInterval;
            console.log(`Reconnecting in ${reconnectInterval}ms...`);
            reconnectInterval = Math.min(reconnectInterval * 2, maxReconnectInterval);
            connect();
        }

        function updateStatus(connected) {
            const dot = document.getElementById('statusDot');
            const text = document.getElementById('statusText');
            if (connected) { dot.classList.add('connected'); text.textContent = 'Connected'; } 
            else { dot.classList.remove('connected'); text.textContent = 'Disconnected'; }
        }

        document.addEventListener('DOMContentLoaded', () => {
            connect();
        });

        window.addEventListener('beforeunload', () => {
            if (ws) {
                ws.close();
            }
        });
    </script>
</body>
</html>"#;

/// Test rule page HTML
const TEST_RULE_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>WinEventEngine - Test Rules</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #e2e8f0; min-height: 100vh; }
        .header { background: linear-gradient(135deg, #1e293b 0%, #0f172a 100%); padding: 1rem 2rem; border-bottom: 1px solid #334155; display: flex; align-items: center; gap: 1rem; }
        .header h1 { font-size: 1.5rem; color: #f8fafc; display: flex; align-items: center; gap: 0.5rem; margin-right: auto; }
        .header .connection-status { display: flex; align-items: center; gap: 0.5rem; font-size: 0.875rem; color: #94a3b8; }
        .header .status-dot { width: 8px; height: 8px; border-radius: 50%; background: #ef4444; transition: background 0.3s; }
        .header .status-dot.connected { background: #22c55e; }
        .nav { display: flex; gap: 0.5rem; }
        .nav a { color: #94a3b8; text-decoration: none; padding: 0.5rem 1rem; border-radius: 6px; font-size: 0.875rem; transition: all 0.2s; }
        .nav a:hover, .nav a.active { background: #3b82f6; color: #fff; }
        .container { max-width: 1000px; margin: 0 auto; padding: 2rem; }
        .section { background: #1e293b; padding: 1.5rem; border-radius: 12px; margin-bottom: 1.5rem; }
        .section h3 { margin-bottom: 1rem; color: #f8fafc; }
        select, textarea { width: 100%; padding: 0.75rem; background: #0f172a; border: 1px solid #334155; border-radius: 6px; color: #f8fafc; font-family: monospace; font-size: 0.875rem; }
        textarea { min-height: 200px; resize: vertical; }
        .btn { background: #3b82f6; color: white; border: none; padding: 0.75rem 1.5rem; border-radius: 6px; cursor: pointer; font-size: 1rem; }
        .btn:hover { background: #2563eb; }
        .result { padding: 1rem; border-radius: 6px; margin-top: 1rem; }
        .result.match { background: #22c55e20; border: 1px solid #22c55e; color: #22c55e; }
        .result.no-match { background: #ef444420; border: 1px solid #ef4444; color: #ef4444; }
        .result.error { background: #f59e0b20; border: 1px solid #f59e0b; color: #f59e0b; }
    </style>
</head>
<body>
    <div class="header">
        <h1>WinEventEngine <span class="connection-status"><span class="status-dot" id="statusDot"></span><span id="statusText">Connecting...</span></span></h1>
        <nav class="nav">
            <a href="/">Dashboard</a>
            <a href="/automations">Automations</a>
            <a href="/test" class="active">Test Rules</a>
            <a href="/import-export">Import/Export</a>
        </nav>
    </div>
    <div class="container">
        <div class="section">
            <h3>Select Rule to Test</h3>
            <select id="ruleSelect" onchange="updateSampleEvent()">
                <option value="">-- Select a rule --</option>
            </select>
        </div>
        
        <div class="section">
            <h3>Event JSON</h3>
            <p style="color: #64748b; margin-bottom: 1rem; font-size: 0.875rem;">Paste an event JSON to test if it matches the rule</p>
            <textarea id="eventJson" placeholder='{"id": "...", "timestamp": "...", "kind": {"FileCreated": {"path": "C:/test.txt"}}, "source": "file_watcher", "metadata": {}}'></textarea>
        </div>
        
        <button class="btn" onclick="testRule()">Test Rule</button>
        
        <div id="result" style="display: none"></div>
    </div>
    
    <script>
        async function loadRules() {
            const response = await fetch('/api/rules');
            const data = await response.json();
            const select = document.getElementById('ruleSelect');
            
            if (data.success && data.data) {
                data.data.forEach(rule => {
                    const option = document.createElement('option');
                    option.value = JSON.stringify(rule);
                    option.textContent = rule.name + ' (' + rule.trigger.type + ')';
                    select.appendChild(option);
                });
            }
        }
        
        function updateSampleEvent() {
            const ruleSelect = document.getElementById('ruleSelect');
            const eventJson = document.getElementById('eventJson');
            
            if (!ruleSelect.value) return;
            
            const rule = JSON.parse(ruleSelect.value);
            const triggerType = rule.trigger.type;
            
            let sampleEvent = {
                id: "00000000-0000-0000-0000-000000000000",
                timestamp: "2024-01-01T00:00:00Z",
                source: "test",
                metadata: {}
            };
            
            switch(triggerType) {
                case 'file_created':
                case 'file_modified':
                case 'file_deleted':
                    sampleEvent.kind = { FileCreated: { path: "C:/test.txt" } };
                    break;
                case 'window_focused':
                case 'window_unfocused':
                case 'window_created':
                    sampleEvent.kind = { WindowFocused: { hwnd: 12345, title: "Notepad" } };
                    break;
                case 'process_started':
                case 'process_stopped':
                    sampleEvent.kind = { ProcessStarted: { pid: 1234, name: "notepad.exe", path: "C:/Windows/notepad.exe" } };
                    break;
                case 'registry_changed':
                    sampleEvent.kind = { RegistryChanged: { root: "HKCU", key: "Software\\Test", value_name: "TestValue" } };
                    break;
                case 'timer':
                    sampleEvent.kind = { TimerTick: {} };
                    break;
            }
            
            eventJson.value = JSON.stringify(sampleEvent, null, 2);
        }
        
        async function testRule() {
            const ruleSelect = document.getElementById('ruleSelect');
            const eventJson = document.getElementById('eventJson');
            const result = document.getElementById('result');
            
            if (!ruleSelect.value || !eventJson.value) {
                result.className = 'result error';
                result.textContent = 'Please select a rule and provide event JSON';
                result.style.display = 'block';
                return;
            }
            
            const rule = JSON.parse(ruleSelect.value);
            const event = eventJson.value;
            
            const response = await fetch('/api/rules/test', {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({ rule, event })
            });
            
            const data = await response.json();
            
            if (data.success) {
                if (data.data) {
                    result.className = 'result match';
                    result.textContent = '✓ Rule MATCHED the event!';
                } else {
                    result.className = 'result no-match';
                    result.textContent = '✗ Rule did NOT match the event';
                }
            } else {
                result.className = 'result error';
                result.textContent = 'Error: ' + (data.error || 'Unknown error');
            }
            result.style.display = 'block';
        }
        
        loadRules();
        
        // WebSocket connection for status
        let ws;
        let reconnectInterval = 1000;
        const maxReconnectInterval = 30000;

        function connect() {
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${protocol}//${window.location.host}/ws`;

            ws = new WebSocket(wsUrl);

            ws.onopen = () => {
                console.log('WebSocket connected');
                updateStatus(true);
                reconnectInterval = 1000;
            };

            ws.onmessage = (event) => {
                try {
                    const data = JSON.parse(event.data);
                } catch (e) {
                    console.error('Failed to parse message:', e);
                }
            };

            ws.onclose = () => {
                console.log('WebSocket disconnected');
                updateStatus(false);
                scheduleReconnect();
            };

            ws.onerror = (error) => {
                console.error('WebSocket error:', error);
                ws.close();
            };
        }

        function scheduleReconnect() {
            setTimeout(() => {
                console.log(`Reconnecting in ${reconnectInterval}ms...`);
                reconnectInterval = Math.min(reconnectInterval * 2, maxReconnectInterval);
                connect();
            }, reconnectInterval);
        }

        function updateStatus(connected) {
            const dot = document.getElementById('statusDot');
            const text = document.getElementById('statusText');

            if (connected) {
                dot.classList.add('connected');
                text.textContent = 'Connected';
            } else {
                dot.classList.remove('connected');
                text.textContent = 'Disconnected';
            }
        }

        document.addEventListener('DOMContentLoaded', () => {
            connect();
        });

        window.addEventListener('beforeunload', () => {
            if (ws) {
                ws.close();
            }
        });
    </script>
</body>
</html>"#;

/// Import/Export page HTML
const IMPORT_EXPORT_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>WinEventEngine - Import/Export</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #e2e8f0; min-height: 100vh; }
        .header { background: linear-gradient(135deg, #1e293b 0%, #0f172a 100%); padding: 1rem 2rem; border-bottom: 1px solid #334155; display: flex; align-items: center; gap: 1rem; }
        .header h1 { font-size: 1.5rem; color: #f8fafc; display: flex; align-items: center; gap: 0.5rem; margin-right: auto; }
        .header .connection-status { display: flex; align-items: center; gap: 0.5rem; font-size: 0.875rem; color: #94a3b8; }
        .header .status-dot { width: 8px; height: 8px; border-radius: 50%; background: #ef4444; transition: background 0.3s; }
        .header .status-dot.connected { background: #22c55e; }
        .nav { display: flex; gap: 0.5rem; }
        .nav a { color: #94a3b8; text-decoration: none; padding: 0.5rem 1rem; border-radius: 6px; font-size: 0.875rem; transition: all 0.2s; }
        .nav a:hover, .nav a.active { background: #3b82f6; color: #fff; }
        .container { max-width: 800px; margin: 0 auto; padding: 2rem; }
        .section { background: #1e293b; padding: 1.5rem; border-radius: 12px; margin-bottom: 1.5rem; }
        .section h3 { margin-bottom: 1rem; color: #f8fafc; }
        textarea { width: 100%; padding: 0.75rem; background: #0f172a; border: 1px solid #334155; border-radius: 6px; color: #f8fafc; font-family: monospace; font-size: 0.875rem; min-height: 300px; }
        .btn { background: #3b82f6; color: white; border: none; padding: 0.75rem 1.5rem; border-radius: 6px; cursor: pointer; font-size: 1rem; margin-right: 1rem; }
        .btn:hover { background: #2563eb; }
        .btn-success { background: #22c55e; }
        .btn-success:hover { background: #16a34a; }
        .message { padding: 1rem; border-radius: 6px; margin-top: 1rem; }
        .message.success { background: #22c55e20; border: 1px solid #22c55e; color: #22c55e; }
        .message.error { background: #ef444420; border: 1px solid #ef4444; color: #ef4444; }
    </style>
</head>
<body>
    <div class="header">
        <h1>WinEventEngine <span class="connection-status"><span class="status-dot" id="statusDot"></span><span id="statusText">Connecting...</span></span></h1>
        <nav class="nav">
            <a href="/">Dashboard</a>
            <a href="/automations">Automations</a>
            <a href="/test">Test Rules</a>
            <a href="/import-export" class="active">Import/Export</a>
        </nav>
    </div>
    <div class="container">
        <div class="section">
            <h3>Export Automations</h3>
            <p style="color: #64748b; margin-bottom: 1rem;">Download all your automations as JSON.</p>
            <button class="btn btn-success" onclick="exportRules()">Export to JSON</button>
            <textarea id="exportArea" readonly placeholder="Exported JSON will appear here..."></textarea>
        </div>
        
        <div class="section">
            <h3>Import Automations</h3>
            <p style="color: #64748b; margin-bottom: 1rem;">Paste JSON to import automations. Existing rules with the same name will be skipped.</p>
            <textarea id="importArea" placeholder="Paste JSON array of rules here..."></textarea>
            <button class="btn" onclick="importRules()" style="margin-top: 1rem;">Import</button>
            <div id="importMessage" style="display: none"></div>
        </div>
    </div>
    
    <script>
        async function exportRules() {
            const response = await fetch('/api/rules/export');
            const text = await response.text();
            document.getElementById('exportArea').value = text;
        }
        
        async function importRules() {
            const content = document.getElementById('importArea').value;
            const message = document.getElementById('importMessage');
            
            const response = await fetch('/api/rules/import', {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify({ content })
            });
            
            const data = await response.json();
            
            if (data.success) {
                message.className = 'message success';
                message.textContent = `Successfully imported ${data.data} rules!`;
            } else {
                message.className = 'message error';
                message.textContent = 'Error: ' + (data.error || 'Unknown error');
            }
            message.style.display = 'block';
        }
        
        // WebSocket connection for status
        let ws = null;
        let reconnectInterval = 1000;
        const maxReconnectInterval = 30000;
        let isConnecting = false;

        function connect() {
            if (isConnecting) { console.log('Already connecting, skipping...'); return; }
            try { if (sessionStorage.getItem('ws_connected') === 'true' && ws && ws.readyState === WebSocket.OPEN) { console.log('WebSocket already connected'); return; } } catch(e) {}
            isConnecting = true;
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${protocol}//${window.location.host}/ws`;
            try { ws = new WebSocket(wsUrl); } catch(e) { isConnecting = false; console.error('Failed to create WebSocket:', e); return; }
            ws.onopen = () => { console.log('WebSocket connected'); updateStatus(true); reconnectInterval = 1000; isConnecting = false; try { sessionStorage.setItem('ws_connected', 'true'); } catch(e) {} };
            ws.onmessage = (event) => { try { const data = JSON.parse(event.data); } catch (e) { console.error('Failed to parse message:', e); } };
            ws.onclose = () => { console.log('WebSocket disconnected'); updateStatus(false); isConnecting = false; try { sessionStorage.setItem('ws_connected', 'false'); } catch(e) {} setTimeout(() => { scheduleReconnect(); }, 500); };
            ws.onerror = (error) => { console.error('WebSocket error:', error); ws.close(); };
        }

        function scheduleReconnect() {
            if (reconnectInterval > maxReconnectInterval) reconnectInterval = maxReconnectInterval;
            console.log(`Reconnecting in ${reconnectInterval}ms...`);
            reconnectInterval = Math.min(reconnectInterval * 2, maxReconnectInterval);
            connect();
        }

        function updateStatus(connected) {
            const dot = document.getElementById('statusDot');
            const text = document.getElementById('statusText');
            if (connected) { dot.classList.add('connected'); text.textContent = 'Connected'; } 
            else { dot.classList.remove('connected'); text.textContent = 'Disconnected'; }
        }

        document.addEventListener('DOMContentLoaded', () => { connect(); });

        window.addEventListener('beforeunload', () => { if (ws) { ws.close(); } });
    </script>
</body>
</html>"#;

/// Prometheus format metrics handler
async fn metrics_handler(State(state): State<Arc<ApiState>>) -> String {
    state.metrics.get_prometheus_format()
}

/// JSON snapshot handler
async fn snapshot_handler(State(state): State<Arc<ApiState>>) -> Json<MetricsSnapshot> {
    Json(state.metrics.get_snapshot())
}

/// Health check handler
async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        timestamp: chrono::Utc::now(),
    })
}

async fn rules_list_handler(State(state): State<Arc<ApiState>>) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    if let Some(manager) = &state.rule_manager {
        let rules = manager.get_rules();
        Json(ApiResponse::success(rules))
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_get_handler(State(state): State<Arc<ApiState>>, Path(name): Path<String>) -> Json<ApiResponse<serde_json::Value>> {
    if let Some(manager) = &state.rule_manager {
        let rules = manager.get_rules();
        if let Some(rule) = rules.into_iter().find(|r| r.get("name").and_then(|v| v.as_str()) == Some(&name)) {
            Json(ApiResponse::success(rule))
        } else {
            Json(ApiResponse::error(&format!("Rule '{}' not found", name)))
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_create_handler(State(state): State<Arc<ApiState>>, Json(rule): Json<serde_json::Value>) -> Json<ApiResponse<serde_json::Value>> {
    if let Some(manager) = &state.rule_manager {
        match manager.add_rule(rule) {
            Ok(created) => Json(ApiResponse::success(created)),
            Err(e) => Json(ApiResponse::error(&e)),
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_update_handler(State(state): State<Arc<ApiState>>, Path(name): Path<String>, Json(rule): Json<serde_json::Value>) -> Json<ApiResponse<serde_json::Value>> {
    if let Some(manager) = &state.rule_manager {
        match manager.update_rule(&name, rule) {
            Ok(updated) => Json(ApiResponse::success(updated)),
            Err(e) => Json(ApiResponse::error(&e)),
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_delete_handler(State(state): State<Arc<ApiState>>, Path(name): Path<String>) -> Json<ApiResponse<()>> {
    if let Some(manager) = &state.rule_manager {
        match manager.delete_rule(&name) {
            Ok(()) => Json(ApiResponse::success(())),
            Err(e) => Json(ApiResponse::error(&e)),
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_enable_handler(State(state): State<Arc<ApiState>>, Path(name): Path<String>, Json(payload): Json<EnableRequest>) -> Json<ApiResponse<()>> {
    if let Some(manager) = &state.rule_manager {
        match manager.enable_rule(&name, payload.enabled) {
            Ok(()) => Json(ApiResponse::success(())),
            Err(e) => Json(ApiResponse::error(&e)),
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_validate_handler(State(state): State<Arc<ApiState>>, Json(rule): Json<serde_json::Value>) -> Json<ApiResponse<()>> {
    if let Some(manager) = &state.rule_manager {
        match manager.validate_rule(rule) {
            Ok(()) => Json(ApiResponse::success(())),
            Err(e) => Json(ApiResponse::error(&e)),
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_test_handler(State(state): State<Arc<ApiState>>, Json(payload): Json<TestRuleRequest>) -> Json<ApiResponse<bool>> {
    if let Some(manager) = &state.rule_manager {
        match manager.test_rule_match(payload.rule, &payload.event) {
            Ok(matched) => Json(ApiResponse::success(matched)),
            Err(e) => Json(ApiResponse::error(&e)),
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

async fn rules_export_handler(State(state): State<Arc<ApiState>>) -> Result<String, String> {
    if let Some(manager) = &state.rule_manager {
        manager.export_rules().map_err(|e| e)
    } else {
        Err("Rule manager not available".to_string())
    }
}

async fn rules_import_handler(State(state): State<Arc<ApiState>>, Json(payload): Json<ImportRequest>) -> Json<ApiResponse<usize>> {
    if let Some(manager) = &state.rule_manager {
        match manager.import_rules(&payload.content) {
            Ok(count) => Json(ApiResponse::success(count)),
            Err(e) => Json(ApiResponse::error(&e)),
        }
    } else {
        Json(ApiResponse::error("Rule manager not available"))
    }
}

#[derive(Debug, Deserialize)]
struct EnableRequest {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct TestRuleRequest {
    rule: serde_json::Value,
    event: String,
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    content: String,
}

#[derive(Debug, Serialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

impl<T> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn error(message: &str) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.to_string()),
        }
    }
}

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthResponse {
    status: String,
    timestamp: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_server_creation() {
        let collector = Arc::new(MetricsCollector::new());
        let server = MetricsServer::new(collector, 9090);

        assert_eq!(server.port, 9090);
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let response = health_handler().await;
        assert_eq!(response.status, "healthy");
    }
}
