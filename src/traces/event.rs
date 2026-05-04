use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct TraceEvent {
    pub name: Option<String>,
    pub cat: Option<String>,
    pub ph: String, // Phase: B (Begin), E (End), X (Complete), etc.
    pub ts: f64,    // Timestamp
    pub pid: u32,
    pub tid: u32,
    pub args: Option<serde_json::Value>, // Extra data varies wildly
}

#[derive(Debug, Deserialize)]
pub struct Trace {
    pub traceEvents: Vec<TraceEvent>,
    pub beginningOfTime: f64,
}