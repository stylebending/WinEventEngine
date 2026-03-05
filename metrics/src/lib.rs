use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info};

/// Real-time metric update events for WebSocket broadcast
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum MetricUpdate {
    /// A new event was received
    #[serde(rename = "event_received")]
    EventReceived {
        timestamp: DateTime<Utc>,
        source: String,
        event_type: String,
    },

    /// A rule was evaluated
    #[serde(rename = "rule_evaluated")]
    RuleEvaluated {
        timestamp: DateTime<Utc>,
        rule_name: String,
    },

    /// A rule matched successfully
    #[serde(rename = "rule_matched")]
    RuleMatched {
        timestamp: DateTime<Utc>,
        rule_name: String,
    },

    /// An action was executed
    #[serde(rename = "action_executed")]
    ActionExecuted {
        timestamp: DateTime<Utc>,
        action_name: String,
        success: bool,
    },

    /// Periodic full metrics snapshot
    #[serde(rename = "snapshot")]
    Snapshot(MetricsSnapshot),

    /// System health update
    #[serde(rename = "health")]
    Health {
        timestamp: DateTime<Utc>,
        uptime_seconds: f64,
        active_plugins: usize,
        active_rules: usize,
    },
}

/// Default retention period for regular metrics (1 hour)
const DEFAULT_RETENTION_SECONDS: u64 = 3600;
/// Extended retention period for error-level metrics (24 hours)
const ERROR_RETENTION_SECONDS: u64 = 86400;
/// Cleanup interval (5 minutes)
const CLEANUP_INTERVAL_SECONDS: u64 = 300;

/// Metric value types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricValue {
    Counter(u64),
    Gauge(f64),
    Histogram(Vec<f64>),
}

/// A single metric sample with timestamp
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub timestamp: DateTime<Utc>,
    pub value: MetricValue,
    pub labels: HashMap<String, String>,
}

/// Metric type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
}

/// Metric metadata
#[derive(Debug, Clone)]
pub struct MetricMetadata {
    pub metric_type: MetricType,
    pub description: String,
    pub is_error_metric: bool,
}

/// Metrics collector with sliding window retention
pub struct MetricsCollector {
    /// Metric metadata registry
    metadata: DashMap<String, MetricMetadata>,
    /// Counter metrics (always increasing)
    counters: DashMap<String, AtomicU64>,
    /// Counter samples with timestamps for sliding window
    counter_samples: DashMap<String, Vec<(DateTime<Utc>, u64)>>,
    /// Gauge metrics (current value)
    gauges: DashMap<String, AtomicU64>,
    /// Gauge samples with timestamps
    gauge_samples: DashMap<String, Vec<(DateTime<Utc>, u64)>>,
    /// Histogram samples (stored as duration in nanoseconds)
    histograms: DashMap<String, Vec<(DateTime<Utc>, u64)>>,
    /// Engine start time for uptime calculation
    start_time: Instant,
    /// Retention configuration
    retention_seconds: u64,
    error_retention_seconds: u64,
    /// Cleanup task handle
    cleanup_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Broadcast channel for real-time metric updates
    update_tx: broadcast::Sender<MetricUpdate>,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    /// Create a new metrics collector with default retention
    pub fn new() -> Self {
        Self::with_retention(DEFAULT_RETENTION_SECONDS, ERROR_RETENTION_SECONDS)
    }

    /// Create a metrics collector with custom retention periods
    pub fn with_retention(retention_seconds: u64, error_retention_seconds: u64) -> Self {
        // Create broadcast channel (capacity 1024 - drops old messages if clients slow)
        let (update_tx, _) = broadcast::channel(1024);

        let collector = Self {
            metadata: DashMap::new(),
            counters: DashMap::new(),
            counter_samples: DashMap::new(),
            gauges: DashMap::new(),
            gauge_samples: DashMap::new(),
            histograms: DashMap::new(),
            start_time: Instant::now(),
            retention_seconds,
            error_retention_seconds,
            cleanup_handle: RwLock::new(None),
            update_tx,
        };

        // Register built-in metadata
        collector.register_metadata(
            "events_total",
            MetricType::Counter,
            "Total events processed",
            false,
        );
        collector.register_metadata(
            "events_dropped_total",
            MetricType::Counter,
            "Total events dropped due to full buffer",
            true,
        );
        collector.register_metadata(
            "events_processing_duration_seconds",
            MetricType::Histogram,
            "Event processing duration in seconds",
            false,
        );
        collector.register_metadata(
            "rules_evaluated_total",
            MetricType::Counter,
            "Total rule evaluations",
            false,
        );
        collector.register_metadata(
            "rules_matched_total",
            MetricType::Counter,
            "Total successful rule matches",
            false,
        );
        collector.register_metadata(
            "rules_match_duration_seconds",
            MetricType::Histogram,
            "Rule matching duration in seconds",
            false,
        );
        collector.register_metadata(
            "actions_executed_total",
            MetricType::Counter,
            "Total actions executed",
            false,
        );
        collector.register_metadata(
            "actions_execution_duration_seconds",
            MetricType::Histogram,
            "Action execution duration in seconds",
            false,
        );
        collector.register_metadata(
            "plugins_events_generated_total",
            MetricType::Counter,
            "Total events generated by plugins",
            false,
        );
        collector.register_metadata(
            "plugins_errors_total",
            MetricType::Counter,
            "Total plugin errors",
            true,
        );
        collector.register_metadata(
            "engine_uptime_seconds",
            MetricType::Gauge,
            "Engine uptime in seconds",
            false,
        );
        collector.register_metadata(
            "config_reload_total",
            MetricType::Counter,
            "Total configuration reloads",
            false,
        );
        collector.register_metadata(
            "active_plugins",
            MetricType::Gauge,
            "Number of active plugins",
            false,
        );
        collector.register_metadata(
            "active_rules",
            MetricType::Gauge,
            "Number of active rules",
            false,
        );

        collector
    }

    /// Start the background cleanup task
    pub async fn start_cleanup_task(&self) {
        let mut handle = self.cleanup_handle.write().await;
        if handle.is_none() {
            let retention = self.retention_seconds;
            let error_retention = self.error_retention_seconds;
            let counters = self.counter_samples.clone();
            let gauges = self.gauge_samples.clone();
            let histograms = self.histograms.clone();

            *handle = Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(CLEANUP_INTERVAL_SECONDS));
                
                loop {
                    interval.tick().await;
                    
                    let cutoff = Utc::now() - chrono::Duration::seconds(retention as i64);
                    let error_cutoff = Utc::now() - chrono::Duration::seconds(error_retention as i64);

                    // Clean up counter samples
                    for mut entry in counters.iter_mut() {
                        let samples = entry.value_mut();
                        samples.retain(|(ts, _)| *ts > cutoff);
                    }

                    // Clean up gauge samples
                    for mut entry in gauges.iter_mut() {
                        let samples = entry.value_mut();
                        samples.retain(|(ts, _)| *ts > cutoff);
                    }

                    // Clean up histogram samples (errors get longer retention)
                    for mut entry in histograms.iter_mut() {
                        let samples = entry.value_mut();
                        samples.retain(|(ts, _)| *ts > error_cutoff);
                    }

                    debug!("Metrics cleanup completed");
                }
            }));

            info!("Metrics cleanup task started");
        }
    }

    /// Stop the cleanup task
    pub async fn stop_cleanup_task(&self) {
        let mut handle = self.cleanup_handle.write().await;
        if let Some(h) = handle.take() {
            h.abort();
            info!("Metrics cleanup task stopped");
        }
    }

    fn register_metadata(&self, name: &str, metric_type: MetricType, description: &str, is_error_metric: bool) {
        self.metadata.insert(
            name.to_string(),
            MetricMetadata {
                metric_type,
                description: description.to_string(),
                is_error_metric,
            },
        );
    }

    fn build_key(name: &str, labels: &HashMap<String, String>) -> String {
        if labels.is_empty() {
            name.to_string()
        } else {
            let mut label_parts: Vec<String> = labels
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            label_parts.sort();
            format!("{}:{{{}}}", name, label_parts.join(","))
        }
    }

    /// Increment a counter metric
    pub fn increment_counter(&self, name: &str, labels: HashMap<String, String>, value: u64) {
        let key = Self::build_key(name, &labels);
        
        // Update the atomic counter
        if let Some(counter) = self.counters.get(&key) {
            counter.fetch_add(value, Ordering::Relaxed);
        } else {
            let counter = AtomicU64::new(value);
            self.counters.insert(key.clone(), counter);
        }

        // Record the sample with timestamp
        let now = Utc::now();
        self.counter_samples
            .entry(key)
            .or_insert_with(Vec::new)
            .push((now, value));

        debug!("Counter {} incremented by {}", name, value);
    }

    /// Set a gauge metric
    pub fn set_gauge(&self, name: &str, labels: HashMap<String, String>, value: f64) {
        let key = Self::build_key(name, &labels);
        let bits = value.to_bits();
        
        // Update the atomic gauge
        if let Some(gauge) = self.gauges.get(&key) {
            gauge.store(bits, Ordering::Relaxed);
        } else {
            self.gauges.insert(key.clone(), AtomicU64::new(bits));
        }

        // Record the sample with timestamp
        let now = Utc::now();
        self.gauge_samples
            .entry(key)
            .or_insert_with(Vec::new)
            .push((now, bits));

        debug!("Gauge {} set to {}", name, value);
    }

    /// Record a histogram observation
    pub fn record_histogram(&self, name: &str, labels: HashMap<String, String>, value: f64) {
        let key = Self::build_key(name, &labels);
        let nanos = (value * 1_000_000_000.0) as u64;
        
        let now = Utc::now();
        self.histograms
            .entry(key)
            .or_insert_with(Vec::new)
            .push((now, nanos));

        debug!("Histogram {} recorded value {}", name, value);
    }

    /// Get the current value of a counter
    pub fn get_counter(&self, name: &str, labels: &HashMap<String, String>) -> Option<u64> {
        let key = Self::build_key(name, labels);
        self.counters
            .get(&key)
            .map(|c| c.load(Ordering::Relaxed))
    }

    /// Get the current value of a gauge
    pub fn get_gauge(&self, name: &str, labels: &HashMap<String, String>) -> Option<f64> {
        let key = Self::build_key(name, labels);
        self.gauges
            .get(&key)
            .map(|g| f64::from_bits(g.load(Ordering::Relaxed)))
    }

    /// Get histogram statistics (count, sum, avg, min, max) for the retention window
    pub fn get_histogram_stats(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
    ) -> Option<HistogramStats> {
        let key = Self::build_key(name, labels);
        let cutoff = Utc::now() - chrono::Duration::seconds(self.retention_seconds as i64);

        self.histograms.get(&key).map(|samples| {
            let values: Vec<f64> = samples
                .iter()
                .filter(|(ts, _)| *ts > cutoff)
                .map(|(_, nanos)| *nanos as f64 / 1_000_000_000.0)
                .collect();

            if values.is_empty() {
                return HistogramStats::default();
            }

            let count = values.len() as u64;
            let sum: f64 = values.iter().sum();
            let avg = sum / count as f64;
            let min = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            let max = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));

            HistogramStats {
                count,
                sum,
                avg,
                min,
                max,
            }
        })
    }

    /// Get engine uptime in seconds
    pub fn get_uptime_seconds(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Get all metrics snapshot for export
    pub fn get_snapshot(&self) -> MetricsSnapshot {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(self.retention_seconds as i64);

        let mut counters = HashMap::new();
        for entry in self.counters.iter() {
            let key = entry.key();
            let value = entry.value().load(Ordering::Relaxed);
            counters.insert(key.clone(), value);
        }

        let mut gauges = HashMap::new();
        for entry in self.gauges.iter() {
            let key = entry.key();
            let value = f64::from_bits(entry.value().load(Ordering::Relaxed));
            gauges.insert(key.clone(), value);
        }
        
        // Always include uptime gauge
        gauges.insert(
            "engine_uptime_seconds".to_string(),
            self.get_uptime_seconds(),
        );

        let mut histograms = HashMap::new();
        for entry in self.histograms.iter() {
            let key = entry.key();
            let samples = entry.value();
            let values: Vec<f64> = samples
                .iter()
                .filter(|(ts, _)| *ts > cutoff)
                .map(|(_, nanos)| *nanos as f64 / 1_000_000_000.0)
                .collect();
            
            if !values.is_empty() {
                histograms.insert(key.clone(), values);
            }
        }

        MetricsSnapshot {
            timestamp: now,
            counters,
            gauges,
            histograms,
        }
    }

    /// Get Prometheus-formatted metrics
    pub fn get_prometheus_format(&self) -> String {
        let snapshot = self.get_snapshot();
        let mut output = String::new();

        // Counters
        for (key, value) in &snapshot.counters {
            let (name, labels) = self.parse_key(key);
            if let Some(meta) = self.metadata.get(&name) {
                if meta.metric_type == MetricType::Counter {
                    output.push_str(&format!("# HELP {} {}\n", name, meta.description));
                    output.push_str(&format!("# TYPE {} counter\n", name));
                    output.push_str(&format!("{}{} {}\n", name, self.format_labels(&labels), value));
                }
            }
        }

        // Gauges
        for (key, value) in &snapshot.gauges {
            let (name, labels) = self.parse_key(key);
            output.push_str(&format!("# HELP {} {}\n", name, name));
            output.push_str(&format!("# TYPE {} gauge\n", name));
            output.push_str(&format!("{}{} {}\n", name, self.format_labels(&labels), value));
        }

        // Histograms - output summary stats
        for (key, values) in &snapshot.histograms {
            let (name, labels) = self.parse_key(key);
            if let Some(meta) = self.metadata.get(&name) {
                if meta.metric_type == MetricType::Histogram && !values.is_empty() {
                    let count = values.len() as u64;
                    let sum: f64 = values.iter().sum();
                    
                    output.push_str(&format!("# HELP {} {}\n", name, meta.description));
                    output.push_str(&format!("# TYPE {} summary\n", name));
                    output.push_str(&format!(
                        "{}_sum{} {}\n",
                        name,
                        self.format_labels(&labels),
                        sum
                    ));
                    output.push_str(&format!(
                        "{}_count{} {}\n",
                        name,
                        self.format_labels(&labels),
                        count
                    ));
                }
            }
        }

        output
    }

    fn parse_key(&self, key: &str) -> (String, HashMap<String, String>) {
        if let Some(pos) = key.find(':') {
            let name = &key[..pos];
            let labels_str = &key[pos + 2..key.len() - 1]; // Remove :{ and }
            let mut labels = HashMap::new();
            
            for part in labels_str.split(',') {
                if let Some(eq_pos) = part.find('=') {
                    let k = part[..eq_pos].to_string();
                    let v = part[eq_pos + 1..].to_string();
                    labels.insert(k, v);
                }
            }
            
            (name.to_string(), labels)
        } else {
            (key.to_string(), HashMap::new())
        }
    }

    fn format_labels(&self, labels: &HashMap<String, String>) -> String {
        if labels.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = labels
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, v))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }

    /// Get the total value for a counter metric by name, summing across all label variations
    /// Returns the sum of all counters with the given base name
    pub fn get_counter_total(&self, name: &str) -> u64 {
        let mut total = 0u64;
        for entry in self.counters.iter() {
            let key = entry.key();
            // Check if this counter matches the base name (either exact match or starts with name:{
            if key == name || key.starts_with(&format!("{}:{{", name)) {
                total += entry.value().load(Ordering::Relaxed);
            }
        }
        total
    }

    /// Get the current value for a gauge metric by name
    /// If multiple gauges exist with the same base name, returns the first one found
    pub fn get_gauge_value(&self, name: &str) -> Option<f64> {
        for entry in self.gauges.iter() {
            let key = entry.key();
            // Check if this gauge matches the base name
            if key == name || key.starts_with(&format!("{}:{{", name)) {
                return Some(f64::from_bits(entry.value().load(Ordering::Relaxed)));
            }
        }
        None
    }

    /// Subscribe to real-time metric updates
    pub fn subscribe(&self) -> broadcast::Receiver<MetricUpdate> {
        self.update_tx.subscribe()
    }

    /// Broadcast an update to all subscribers
    pub fn broadcast(&self, update: MetricUpdate) {
        // Use let _ to ignore send errors (no subscribers is OK)
        let _ = self.update_tx.send(update);
    }

    /// Record an event and broadcast the update
    pub fn record_event_with_broadcast(&self, plugin: &str, event_type: &str) {
        record_event(self, plugin, event_type);

        self.broadcast(MetricUpdate::EventReceived {
            timestamp: Utc::now(),
            source: plugin.to_string(),
            event_type: event_type.to_string(),
        });
    }

    /// Record a rule match and broadcast the update
    pub fn record_rule_match_with_broadcast(&self, rule_name: &str) {
        record_rule_match(self, rule_name);

        self.broadcast(MetricUpdate::RuleMatched {
            timestamp: Utc::now(),
            rule_name: rule_name.to_string(),
        });
    }

    /// Record a rule evaluation and broadcast the update
    pub fn record_rule_evaluation_with_broadcast(&self, rule_name: &str) {
        record_rule_evaluation(self, rule_name);

        self.broadcast(MetricUpdate::RuleEvaluated {
            timestamp: Utc::now(),
            rule_name: rule_name.to_string(),
        });
    }

    /// Record an action execution and broadcast the update
    pub fn record_action_execution_with_broadcast(
        &self,
        action_name: &str,
        success: bool,
        duration: Duration,
    ) {
        record_action_execution(self, action_name, success, duration);

        self.broadcast(MetricUpdate::ActionExecuted {
            timestamp: Utc::now(),
            action_name: action_name.to_string(),
            success,
        });
    }

    /// Record a config reload and broadcast the update
    pub fn record_config_reload_with_broadcast(&self, success: bool, active_plugins: usize, active_rules: usize) {
        record_config_reload(self, success);

        // Also broadcast a health update with system status
        self.broadcast(MetricUpdate::Health {
            timestamp: Utc::now(),
            uptime_seconds: self.get_uptime_seconds(),
            active_plugins,
            active_rules,
        });
    }

    /// Set the engine status gauges (active plugins and rules count)
    pub fn set_engine_status(&self, active_plugins: usize, active_rules: usize) {
        self.set_gauge("active_plugins", HashMap::new(), active_plugins as f64);
        self.set_gauge("active_rules", HashMap::new(), active_rules as f64);
    }

    /// Update only the active rules gauge (without touching active_plugins)
    pub fn set_active_rules(&self, active_rules: usize) {
        self.set_gauge("active_rules", HashMap::new(), active_rules as f64);
    }

    /// Update only the active plugins gauge (without touching active_rules)
    pub fn set_active_plugins(&self, active_plugins: usize) {
        self.set_gauge("active_plugins", HashMap::new(), active_plugins as f64);
    }
}

/// Histogram statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistogramStats {
    pub count: u64,
    pub sum: f64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
}

/// Metrics snapshot for export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub timestamp: DateTime<Utc>,
    pub counters: HashMap<String, u64>,
    pub gauges: HashMap<String, f64>,
    pub histograms: HashMap<String, Vec<f64>>,
}

/// Convenience functions for common metrics

/// Record an event being processed
pub fn record_event(metrics: &MetricsCollector, plugin: &str, event_type: &str) {
    let mut labels = HashMap::new();
    labels.insert("plugin".to_string(), plugin.to_string());
    labels.insert("type".to_string(), event_type.to_string());
    metrics.increment_counter("events_total", labels, 1);
}

/// Record a dropped event
pub fn record_event_dropped(metrics: &MetricsCollector) {
    metrics.increment_counter("events_dropped_total", HashMap::new(), 1);
}

/// Record event processing duration
pub fn record_event_processing_duration(metrics: &MetricsCollector, duration: Duration) {
    metrics.record_histogram(
        "events_processing_duration_seconds",
        HashMap::new(),
        duration.as_secs_f64(),
    );
}

/// Record a rule evaluation
pub fn record_rule_evaluation(metrics: &MetricsCollector, rule_name: &str) {
    let mut labels = HashMap::new();
    labels.insert("rule".to_string(), rule_name.to_string());
    metrics.increment_counter("rules_evaluated_total", labels, 1);
}

/// Record a successful rule match
pub fn record_rule_match(metrics: &MetricsCollector, rule_name: &str) {
    let mut labels = HashMap::new();
    labels.insert("rule".to_string(), rule_name.to_string());
    metrics.increment_counter("rules_matched_total", labels, 1);
}

/// Record rule matching duration
pub fn record_rule_match_duration(metrics: &MetricsCollector, rule_name: &str, duration: Duration) {
    let mut labels = HashMap::new();
    labels.insert("rule".to_string(), rule_name.to_string());
    metrics.record_histogram(
        "rules_match_duration_seconds",
        labels,
        duration.as_secs_f64(),
    );
}

/// Record an action execution
pub fn record_action_execution(
    metrics: &MetricsCollector,
    action_name: &str,
    success: bool,
    duration: Duration,
) {
    let mut labels = HashMap::new();
    labels.insert("action".to_string(), action_name.to_string());
    labels.insert("status".to_string(), if success { "success".to_string() } else { "error".to_string() });
    metrics.increment_counter("actions_executed_total", labels.clone(), 1);
    
    metrics.record_histogram(
        "actions_execution_duration_seconds",
        labels,
        duration.as_secs_f64(),
    );
}

/// Record events generated by a plugin
pub fn record_plugin_event(metrics: &MetricsCollector, plugin: &str, event_type: &str) {
    let mut labels = HashMap::new();
    labels.insert("plugin".to_string(), plugin.to_string());
    labels.insert("type".to_string(), event_type.to_string());
    metrics.increment_counter("plugins_events_generated_total", labels, 1);
}

/// Record a plugin error
pub fn record_plugin_error(metrics: &MetricsCollector, plugin: &str, error_type: &str) {
    let mut labels = HashMap::new();
    labels.insert("plugin".to_string(), plugin.to_string());
    labels.insert("error_type".to_string(), error_type.to_string());
    metrics.increment_counter("plugins_errors_total", labels, 1);
}

/// Record a configuration reload
pub fn record_config_reload(metrics: &MetricsCollector, success: bool) {
    let mut labels = HashMap::new();
    labels.insert("status".to_string(), if success { "success".to_string() } else { "error".to_string() });
    metrics.increment_counter("config_reload_total", labels, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_counter_operations() {
        let metrics = MetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("plugin".to_string(), "test".to_string());

        metrics.increment_counter("events_total", labels.clone(), 1);
        metrics.increment_counter("events_total", labels.clone(), 2);

        let value = metrics.get_counter("events_total", &labels);
        assert_eq!(value, Some(3));
    }

    #[tokio::test]
    async fn test_gauge_operations() {
        let metrics = MetricsCollector::new();
        let labels = HashMap::new();

        metrics.set_gauge("engine_uptime_seconds", labels.clone(), 42.5);

        let value = metrics.get_gauge("engine_uptime_seconds", &labels);
        assert_eq!(value, Some(42.5));
    }

    #[tokio::test]
    async fn test_histogram_operations() {
        let metrics = MetricsCollector::new();
        let labels = HashMap::new();

        metrics.record_histogram("test_histogram", labels.clone(), 0.1);
        metrics.record_histogram("test_histogram", labels.clone(), 0.2);
        metrics.record_histogram("test_histogram", labels.clone(), 0.3);

        let stats = metrics.get_histogram_stats("test_histogram", &labels);
        assert!(stats.is_some());
        
        let stats = stats.unwrap();
        assert_eq!(stats.count, 3);
        assert!(stats.avg > 0.19 && stats.avg < 0.21);
    }

    #[tokio::test]
    async fn test_prometheus_format() {
        let metrics = MetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("plugin".to_string(), "file_watcher".to_string());

        metrics.increment_counter("events_total", labels, 5);
        metrics.set_gauge("engine_uptime_seconds", HashMap::new(), 123.0);

        let output = metrics.get_prometheus_format();
        assert!(output.contains("events_total"));
        assert!(output.contains("engine_uptime_seconds"));
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        use std::sync::Arc;
        use tokio::task;

        let metrics = Arc::new(MetricsCollector::new());
        let mut handles = vec![];

        for i in 0..10 {
            let m = metrics.clone();
            let handle = task::spawn(async move {
                let mut labels = HashMap::new();
                labels.insert("thread".to_string(), format!("{}", i));
                m.increment_counter("concurrent_test", labels, 1);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all increments happened
        let total: u64 = (0..10)
            .map(|i| {
                let mut labels = HashMap::new();
                labels.insert("thread".to_string(), format!("{}", i));
                metrics.get_counter("concurrent_test", &labels).unwrap_or(0)
            })
            .sum();

        assert_eq!(total, 10);
    }
}
