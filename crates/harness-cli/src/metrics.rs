use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct CallMetric {
    pub role: String,
    pub provider: String,
    pub model: Option<String>,
    pub duration_ms: u128,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Default)]
struct Metrics {
    calls: Vec<CallMetric>,
}

static METRICS: OnceLock<Mutex<Metrics>> = OnceLock::new();

fn metrics() -> &'static Mutex<Metrics> {
    METRICS.get_or_init(|| Mutex::new(Metrics::default()))
}

pub fn reset() {
    *metrics().lock().expect("metrics mutex poisoned") = Metrics::default();
}

pub fn record(call: CallMetric) {
    metrics()
        .lock()
        .expect("metrics mutex poisoned")
        .calls
        .push(call);
}

#[derive(Serialize)]
struct MetricsReport<'a> {
    schema_version: u32,
    calls: &'a [CallMetric],
    call_count: usize,
    total_duration_ms: u128,
    known_cost_usd: f64,
    cost_complete: bool,
}

pub fn write(path: &Path) -> Result<(), String> {
    let guard = metrics().lock().expect("metrics mutex poisoned");
    let calls = &guard.calls;
    let report = MetricsReport {
        schema_version: 1,
        calls,
        call_count: calls.len(),
        total_duration_ms: calls.iter().map(|call| call.duration_ms).sum(),
        known_cost_usd: calls.iter().filter_map(|call| call.cost_usd).sum(),
        cost_complete: !calls.is_empty() && calls.iter().all(|call| call.cost_usd.is_some()),
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| format!("cannot write metrics: {error}"))
}

pub fn role_for_schema(schema_name: &str) -> String {
    schema_name.trim_end_matches(".schema.json").to_owned()
}
