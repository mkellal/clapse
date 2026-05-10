use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct TraceEvent {
    pub name: Option<String>,
    pub cat: Option<String>,
    pub ph: String, // Phase: B (Begin), E (End), X (Complete), etc.
    pub ts: f64,    // Timestamp
    pub pid: u32,
    pub tid: u32,
    pub dur: Option<f64>,   // Duration for complete events
    pub args: Option<Args>, // Extra data varies wildly
}

#[derive(Debug, Deserialize)]
pub struct Args {
    pub detail: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Args { detail: None }
    }
}

#[derive(Debug, Deserialize)]
pub struct TraceData {
    #[serde(rename = "traceEvents")]
    pub trace_events: Vec<TraceEvent>,
    #[serde(rename = "beginningOfTime")]
    pub beginning_of_time: f64,
}

pub fn parse_trace_file(path: &PathBuf) -> Option<TraceData> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to read {:?}: {}", path, e);
            return None;
        }
    };

    match serde_json::from_slice::<TraceData>(&bytes) {
        Ok(data) => Some(data),
        Err(e) => {
            eprintln!("Failed to parse {:?}: {}", path, e);
            return None;
        }
    }
}
