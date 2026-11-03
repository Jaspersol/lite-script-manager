use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub interpreter_path: Option<String>,
    #[serde(default)]
    pub process_match: Option<String>,
    #[serde(default = "default_stop_timeout")]
    pub stop_timeout_sec: i64,
    #[serde(default)]
    pub timeout_sec: Option<i64>,
    #[serde(default = "default_stop_mode")]
    pub stop_mode: String,
    #[serde(default = "default_max_instances")]
    pub max_instances: i64,
    #[serde(default = "default_instance_policy")]
    pub instance_policy: String,
    #[serde(default = "default_success_codes")]
    pub success_codes: String,
    #[serde(default)]
    pub content_hash: Option<String>,
    pub sort_order: i64,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default = "default_status")]
    pub status: String,
    pub last_error: Option<String>,
    pub last_run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptEdit {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub interpreter_path: Option<String>,
    #[serde(default)]
    pub process_match: Option<String>,
    #[serde(default = "default_stop_timeout")]
    pub stop_timeout_sec: i64,
    #[serde(default)]
    pub timeout_sec: Option<i64>,
    #[serde(default = "default_stop_mode")]
    pub stop_mode: String,
    #[serde(default = "default_max_instances")]
    pub max_instances: i64,
    #[serde(default = "default_instance_policy")]
    pub instance_policy: String,
    #[serde(default = "default_success_codes")]
    pub success_codes: String,
    #[serde(default)]
    pub enabled: bool,
}

fn default_status() -> String {
    "stopped".to_string()
}

fn default_stop_timeout() -> i64 {
    3
}

fn default_stop_mode() -> String {
    "graceful".to_string()
}

fn default_max_instances() -> i64 {
    1
}

fn default_instance_policy() -> String {
    "parallel".to_string()
}

fn default_success_codes() -> String {
    "0".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeCandidate {
    pub path: String,
    pub version: Option<String>,
    pub source: String,
    pub kind: String,
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
    pub details: Vec<String>,
    #[serde(skip)]
    pub signature: String,
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
    pub source: Option<String>,
    pub source_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    pub script_id: String,
    pub kind: String,
    pub expression: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    pub enabled: bool,
    #[serde(default = "default_overlap")]
    pub overlap_policy: String,
    #[serde(default = "default_catch_up")]
    pub catch_up: String,
    #[serde(default)]
    pub timeout_sec: Option<i64>,
    #[serde(default = "default_success_codes")]
    pub success_codes: String,
    #[serde(default)]
    pub restart_delay_sec: i64,
    #[serde(default)]
    pub max_restarts: i64,
    #[serde(default)]
    pub backoff: bool,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_run_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_timezone() -> String {
    "local".to_string()
}

fn default_overlap() -> String {
    "skip".to_string()
}

fn default_catch_up() -> String {
    "none".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleEdit {
    pub name: String,
    pub script_id: String,
    pub kind: String,
    pub expression: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_overlap")]
    pub overlap_policy: String,
    #[serde(default = "default_catch_up")]
    pub catch_up: String,
    #[serde(default)]
    pub timeout_sec: Option<i64>,
    #[serde(default = "default_success_codes")]
    pub success_codes: String,
    #[serde(default)]
    pub restart_delay_sec: i64,
    #[serde(default)]
    pub max_restarts: i64,
    #[serde(default)]
    pub backoff: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub event_type: String,
    pub config_json: String,
    pub actions_json: String,
    pub cooldown_sec: i64,
    pub last_triggered_at: Option<i64>,
    pub last_result: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TriggerEdit {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub event_type: String,
    pub config_json: String,
    pub actions_json: String,
    #[serde(default)]
    pub cooldown_sec: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TriggerEvent {
    pub id: String,
    pub trigger_id: String,
    pub trigger_name: String,
    pub fired_at: i64,
    pub event: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    pub id: String,
    pub script_id: String,
    pub script_name: String,
    pub status: String,
    pub pid: Option<u32>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub log_path: Option<String>,
    pub error: Option<String>,
    pub source: String,
    pub source_id: Option<String>,
    pub stop_reason: Option<String>,
    pub peak_memory_bytes: Option<i64>,
    pub cpu_time_ms: Option<i64>,
    pub timeout_sec: Option<i64>,
    pub log_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleEvent {
    pub run_id: String,
    pub script_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub source: String,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StartOptions {
    pub source: String,
    pub source_id: Option<String>,
    pub timeout_sec: Option<i64>,
    pub success_codes: String,
    pub overlap_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchedulePreview {
    pub next: Vec<i64>,
    pub error: Option<String>,
}
