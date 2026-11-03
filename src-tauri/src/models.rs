use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub id: String,
    pub name: String,
    pub path: String,
    pub runtime_id: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub interpreter_path: Option<String>,
    #[serde(default)]
    pub process_match: Option<String>,
    #[serde(default = "default_stop_timeout")]
    pub stop_timeout_sec: i64,
    pub sort_order: i64,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default = "default_status")]
    pub status: String,
    pub last_error: Option<String>,
    pub last_run_id: Option<String>,
}

fn default_status() -> String {
    "stopped".to_string()
}

fn default_stop_timeout() -> i64 {
    3
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeCandidate {
    pub path: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub exe_path: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub candidates: Vec<RuntimeCandidate>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMeta {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunStateEvent {
    pub run_id: String,
    pub script_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub stream: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogBatchEvent {
    pub run_id: String,
    pub script_id: String,
    pub lines: Vec<LogLine>,
}
