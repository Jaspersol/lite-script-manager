use crate::db;
use crate::logging;
use crate::metrics::{self, CpuSampler, SystemMetrics};
use crate::models::{LifecycleEvent, StartOptions, Trigger};
use crate::runner;
use crate::settings;
use crate::state::AppState;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Default)]
struct Condition {
    #[serde(default)]
    metric: String,
    #[serde(default)]
    op: String,
    #[serde(default)]
    value: f64,
    #[serde(default)]
    for_sec: i64,
    #[serde(default)]
    hysteresis: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TriggerConfig {
    #[serde(default)]
    name: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    parent: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    metric: String,
    #[serde(default)]
    op: String,
    #[serde(default)]
    value: f64,
    #[serde(default)]
    for_sec: i64,
    #[serde(default)]
    hysteresis: f64,
    #[serde(default)]
    conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Deserialize)]
struct ActionSpec {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    script_id: String,
    #[serde(default)]
    trigger_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Clone, Default)]
struct ProcessMeta {
    pid: u32,
    name: String,
    parent: u32,
    created_ms: i64,
    path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ConditionState {
    since: i64,
    active: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct CpuHistory {
    last_cpu_ms: u64,
    last_at: i64,
    percent: f64,
}

pub fn wake(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.trigger_generation.fetch_add(1, Ordering::Relaxed);
    state.trigger_wake.notify_waiters();
}

fn parse_config(trigger: &Trigger) -> TriggerConfig {
    serde_json::from_str(&trigger.config_json).unwrap_or_default()
}

fn parse_actions(trigger: &Trigger) -> Vec<ActionSpec> {
    serde_json::from_str(&trigger.actions_json).unwrap_or_default()
}

fn load_triggers(app: &AppHandle) -> Vec<Trigger> {
    let state = app.state::<AppState>();
    let Ok(conn) = state.db.lock() else {
        return Vec::new();
    };
    db::list_triggers(&conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|trigger| trigger.enabled)
        .collect()
}

fn lower(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn enrich(meta: &mut ProcessMeta) {
    if meta.path.is_some() {
        return;
    }
    if let Some(command) = runner::process_command_line(meta.pid) {
        meta.path = Some(command);
    }
}

fn identity_matches(config: &TriggerConfig, meta: &ProcessMeta, parent_name: &str) -> bool {
    if !config.name.trim().is_empty() && lower(&config.name) != lower(&meta.name) {
        return false;
    }
    if !config.parent.trim().is_empty() && lower(&config.parent) != lower(parent_name) {
        return false;
    }
    true
}

fn process_matches(config: &TriggerConfig, meta: &ProcessMeta, parent_name: &str) -> bool {
    if !identity_matches(config, meta, parent_name) {
        return false;
    }
    if config.path.trim().is_empty() && config.command.trim().is_empty() {
        return true;
    }
    let Some(text) = meta.path.as_deref() else {
        return false;
    };
    let text = lower(text);
    if !config.path.trim().is_empty() && !text.contains(&lower(&config.path)) {
        return false;
    }
    if !config.command.trim().is_empty() && !text.contains(&lower(&config.command)) {
        return false;
    }
    true
}

/// Reads command lines for processes that a path/command trigger could match.
/// Doing this while every candidate is still alive lets `process_exited`
/// triggers match too, and the result is cached in the snapshot for later ticks.
fn enrich_process_triggers(triggers: &[Trigger], snapshot: &mut HashMap<u32, ProcessMeta>) {
    for trigger in triggers {
        if !matches!(
            trigger.event_type.as_str(),
            "process_started" | "process_exited"
        ) {
            continue;
        }
        let config = parse_config(trigger);
        if config.path.trim().is_empty() && config.command.trim().is_empty() {
            continue;
        }
        let pids = snapshot
            .iter()
            .filter(|(_, meta)| {
                let parent = snapshot
                    .get(&meta.parent)
                    .map(|value| value.name.as_str())
                    .unwrap_or("");
                identity_matches(&config, meta, parent)
            })
            .map(|(pid, _)| *pid)
            .collect::<Vec<_>>();
        for pid in pids {
            if let Some(meta) = snapshot.get_mut(&pid) {
                enrich(meta);
            }
        }
    }
}

fn compare(metric: f64, op: &str, value: f64) -> bool {
    match op {
        ">" => metric > value,
        ">=" => metric >= value,
        "<" => metric < value,
        "<=" => metric <= value,
        "==" | "=" => (metric - value).abs() < f64::EPSILON,
        "!=" => (metric - value).abs() >= f64::EPSILON,
        _ => false,
    }
}

fn condition_exit(metric: f64, condition: &Condition) -> bool {
    let hysteresis = condition.hysteresis.max(0.0);
    match condition.op.as_str() {
        ">" | ">=" => metric < condition.value - hysteresis,
        "<" | "<=" => metric > condition.value + hysteresis,
        _ => !compare(metric, &condition.op, condition.value),
    }
}

fn process_pids(config: &TriggerConfig, snapshot: &mut HashMap<u32, ProcessMeta>) -> Vec<u32> {
    let needs_path = !config.path.trim().is_empty() || !config.command.trim().is_empty();
    let mut candidates = Vec::new();
    for (pid, meta) in snapshot.iter() {
        let parent = snapshot
            .get(&meta.parent)
            .map(|value| value.name.as_str())
            .unwrap_or("");
        if identity_matches(config, meta, parent) {
            candidates.push(*pid);
        }
    }
    if needs_path {
        for pid in &candidates {
            if let Some(meta) = snapshot.get_mut(pid)
                && meta.path.is_none()
            {
                enrich(meta);
            }
        }
    }
    let mut values = Vec::new();
    for pid in candidates {
        let Some(meta) = snapshot.get(&pid) else {
            continue;
        };
        let parent = snapshot
            .get(&meta.parent)
            .map(|value| value.name.as_str())
            .unwrap_or("");
        if process_matches(config, meta, parent) {
            values.push(pid);
        }
    }
    values
}

fn resource_value(
    condition: &Condition,
    config: &TriggerConfig,
    system: &SystemMetrics,
    snapshot: &mut HashMap<u32, ProcessMeta>,
    cpu_history: &mut CpuHistory,
) -> Option<f64> {
    match condition.metric.as_str() {
        "system.cpu" => Some(system.cpu_percent),
        "system.memory" => Some(system.memory_percent),
        "system.process_count" | "process.count" => Some(system.process_count as f64),
        "disk.free_gb" => Some(system.disk_free_bytes as f64 / 1024.0 / 1024.0 / 1024.0),
        "process.memory" | "process.memory_mb" => {
            let pids = process_pids(config, snapshot);
            let value = metrics::process_metrics(&pids).memory_bytes;
            Some(if condition.metric.ends_with("_mb") {
                value as f64 / 1024.0 / 1024.0
            } else {
                value as f64
            })
        }
        "process.cpu" => {
            let pids = process_pids(config, snapshot);
            let sample = metrics::process_metrics(&pids);
            let now = db::now_ms();
            let elapsed = now.saturating_sub(cpu_history.last_at).max(1) as f64;
            let cores = std::thread::available_parallelism()
                .map(|value| value.get() as f64)
                .unwrap_or(1.0);
            let delta = sample.cpu_time_ms.saturating_sub(cpu_history.last_cpu_ms) as f64;
            cpu_history.percent = if cpu_history.last_at == 0 {
                0.0
            } else {
                (delta / (elapsed * cores)) * 100.0
            };
            cpu_history.last_cpu_ms = sample.cpu_time_ms;
            cpu_history.last_at = now;
            Some(cpu_history.percent)
        }
        _ => None,
    }
}

fn effective_conditions(config: &TriggerConfig) -> Vec<Condition> {
    if !config.conditions.is_empty() {
        config.conditions.clone()
    } else if !config.metric.is_empty() {
        vec![Condition {
            metric: config.metric.clone(),
            op: config.op.clone(),
            value: config.value,
            for_sec: config.for_sec,
            hysteresis: config.hysteresis,
        }]
    } else {
        Vec::new()
    }
}

fn notify(app: &AppHandle, title: &str, message: &str) {
    let title = if title.trim().is_empty() {
        "LiteScriptManager"
    } else {
        title
    };
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(message)
        .show();
    let _ = app.emit(
        "app-notification",
        serde_json::json!({ "title": title, "message": message }),
    );
}

pub fn fire_manual(app: &AppHandle, trigger_id: &str) -> Result<String, String> {
    let state = app.state::<AppState>();
    let trigger = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_trigger(&conn, trigger_id)?.ok_or_else(|| "触发器不存在".to_string())?
    };
    fire_trigger(app, &trigger, "manual")
}

fn fire_trigger(app: &AppHandle, trigger: &Trigger, event: &str) -> Result<String, String> {
    let state = app.state::<AppState>();
    if let Ok(conn) = state.db.lock()
        && let Ok(Some(current)) = db::get_trigger(&conn, &trigger.id)
    {
        if !current.enabled && event != "manual" {
            return Ok("disabled".into());
        }
        if event != "manual"
            && current.cooldown_sec > 0
            && current
                .last_triggered_at
                .is_some_and(|last| db::now_ms().saturating_sub(last) < current.cooldown_sec * 1000)
        {
            return Ok("cooldown".into());
        }
    }

    let actions = parse_actions(trigger);
    let mut results = Vec::new();
    for action in actions {
        let result = match action.kind.as_str() {
            "run_script" => run_action_script(app, &action, trigger, false),
            "stop_script" => runner::stop(app, &action.script_id).map(|_| "stopped".to_string()),
            "restart_script" => run_action_script(app, &action, trigger, true),
            "enable_trigger" | "disable_trigger" => {
                let enabled = action.kind == "enable_trigger";
                let state = app.state::<AppState>();
                match state.db.lock() {
                    Ok(conn) => {
                        let result = db::set_trigger_enabled(&conn, &action.trigger_id, enabled);
                        drop(conn);
                        if result.is_ok() {
                            wake(app);
                        }
                        result.map(|_| {
                            format!("trigger {}", if enabled { "enabled" } else { "disabled" })
                        })
                    }
                    Err(_) => Err("数据库锁失败".into()),
                }
            }
            "notify" => {
                notify(
                    app,
                    if action.title.is_empty() {
                        &trigger.name
                    } else {
                        &action.title
                    },
                    if action.message.is_empty() {
                        "触发器已触发"
                    } else {
                        &action.message
                    },
                );
                Ok("notified".into())
            }
            other => Err(format!("未知动作: {other}")),
        };
        match result {
            Ok(value) => results.push(format!("{}={value}", action.kind)),
            Err(error) => {
                logging::app_log(
                    "ERROR",
                    &format!("触发器 {} 动作 {} 失败: {error}", trigger.name, action.kind),
                );
                results.push(format!("{}={error}", action.kind));
            }
        }
    }
    if results.is_empty() {
        results.push("no action".into());
    }
    let result = results.join("; ");
    if let Ok(conn) = state.db.lock() {
        let _ = db::record_trigger_event(
            &conn,
            &Uuid::new_v4().to_string(),
            &trigger.id,
            event,
            &result,
        );
    }
    let _ = app.emit("triggers-changed", ());
    let _ = app.emit(
        "trigger-fired",
        serde_json::json!({
            "trigger_id": trigger.id,
            "trigger_name": trigger.name,
            "event": event,
            "result": result
        }),
    );
    Ok(result)
}

fn run_action_script(
    app: &AppHandle,
    action: &ActionSpec,
    trigger: &Trigger,
    restart: bool,
) -> Result<String, String> {
    let state = app.state::<AppState>();
    let script = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_script(&conn, &action.script_id)?.ok_or_else(|| "动作脚本不存在".to_string())?
    };
    let options = StartOptions {
        source: "trigger".into(),
        source_id: Some(trigger.id.clone()),
        timeout_sec: script.timeout_sec,
        success_codes: script.success_codes.clone(),
        overlap_policy: if restart {
            "replace".into()
        } else {
            "skip".into()
        },
    };
    let event = runner::start_with(app, script, options)?;
    Ok(if restart {
        format!("restarted {}", event.run_id)
    } else {
        format!("started {}", event.run_id)
    })
}

fn fire_process_matches(
    app: &AppHandle,
    triggers: &[Trigger],
    event_type: &str,
    current: &mut HashMap<u32, ProcessMeta>,
    previous: &HashMap<u32, ProcessMeta>,
) {
    let ids = if event_type == "process_started" {
        current
            .keys()
            .filter(|pid| !previous.contains_key(pid))
            .copied()
            .collect::<Vec<_>>()
    } else {
        previous
            .keys()
            .filter(|pid| !current.contains_key(pid))
            .copied()
            .collect::<Vec<_>>()
    };
    for pid in ids {
        let Some(meta) = (if event_type == "process_started" {
            current.get(&pid)
        } else {
            previous.get(&pid)
        }) else {
            continue;
        };
        let meta = meta.clone();
        let parent = current
            .get(&meta.parent)
            .or_else(|| previous.get(&meta.parent))
            .map(|value| value.name.as_str())
            .unwrap_or("")
            .to_string();
        for trigger in triggers {
            if trigger.event_type != event_type {
                continue;
            }
            let config = parse_config(trigger);
            if process_matches(&config, &meta, &parent)
                && let Err(error) =
                    fire_trigger(app, trigger, &format!("{event_type}:{}:{}", meta.name, pid))
            {
                logging::app_log("ERROR", &format!("触发器执行失败: {error}"));
            }
        }
    }
}

fn fire_script_event(app: &AppHandle, triggers: &[Trigger], event: &LifecycleEvent) {
    let event_type = if matches!(event.status.as_str(), "running" | "starting") {
        "script_started"
    } else {
        "script_exited"
    };
    for trigger in triggers {
        if trigger.event_type == event_type {
            let _ = fire_trigger(
                app,
                trigger,
                &format!("{event_type}:{}:{}", event.script_id, event.status),
            );
        }
    }
}

fn needs_process_snapshot(triggers: &[Trigger]) -> bool {
    triggers
        .iter()
        .any(|trigger| match trigger.event_type.as_str() {
            "process_started" | "process_exited" => true,
            "resource_enter" | "resource_exit" => {
                let config = parse_config(trigger);
                effective_conditions(&config).iter().any(|condition| {
                    matches!(
                        condition.metric.as_str(),
                        "process.memory" | "process.memory_mb" | "process.cpu"
                    )
                })
            }
            _ => false,
        })
}

fn monitor_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let mut previous: Option<HashMap<u32, ProcessMeta>> = None;
        let mut conditions: HashMap<String, ConditionState> = HashMap::new();
        let mut cpu: HashMap<String, CpuHistory> = HashMap::new();
        let mut sampler = CpuSampler::default();
        let mut lifecycle = state.lifecycle_tx.subscribe();
        let mut triggers: Vec<Trigger> = Vec::new();
        let mut loaded_generation = u64::MAX;
        let mut interval_ms: u64 = 1000;

        loop {
            let generation = state.trigger_generation.load(Ordering::Relaxed);
            if generation != loaded_generation {
                loaded_generation = generation;
                triggers = load_triggers(&app);
                interval_ms = {
                    let conn = state.db.lock().ok();
                    conn.map(|conn| settings::get_i64(&conn, "monitor_interval_ms", 1000))
                        .unwrap_or(1000)
                        .clamp(250, 60_000) as u64
                };
                if triggers.is_empty() {
                    previous = None;
                }
            }
            if triggers.is_empty() {
                tokio::select! {
                    _ = state.trigger_wake.notified() => {}
                    _ = lifecycle.recv() => {}
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                continue;
            }

            tokio::select! {
                _ = state.trigger_wake.notified() => {}
                event = lifecycle.recv() => {
                    if let Ok(event) = event {
                        fire_script_event(&app, &triggers, &event);
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(interval_ms)) => {}
            }

            let mut snapshot: Option<HashMap<u32, ProcessMeta>> = None;
            if needs_process_snapshot(&triggers) {
                let raw = runner::snapshot_processes();
                let mut current = HashMap::with_capacity(raw.len());
                for (pid, parent, name) in raw {
                    let created_ms = previous
                        .as_ref()
                        .and_then(|values| values.get(&pid))
                        .map(|value| value.created_ms)
                        .unwrap_or(0);
                    current.insert(
                        pid,
                        ProcessMeta {
                            pid,
                            name,
                            parent,
                            created_ms,
                            path: None,
                        },
                    );
                }
                for (pid, meta) in &mut current {
                    if meta.created_ms == 0 {
                        meta.created_ms = runner::process_create_time_ms(*pid).unwrap_or(0);
                    }
                }
                enrich_process_triggers(&triggers, &mut current);
                if let Some(previous) = previous.as_ref() {
                    fire_process_matches(
                        &app,
                        &triggers,
                        "process_started",
                        &mut current,
                        previous,
                    );
                    fire_process_matches(&app, &triggers, "process_exited", &mut current, previous);
                    // Reuse previously read command lines without dropping a
                    // path that was enriched during this tick.
                    for (pid, meta) in &mut current {
                        if meta.path.is_none()
                            && let Some(old) = previous.get(pid)
                        {
                            meta.path = old.path.clone();
                        }
                    }
                }
                snapshot = Some(current);
            }

            let resource_triggers = triggers
                .iter()
                .filter(|trigger| {
                    matches!(
                        trigger.event_type.as_str(),
                        "resource_enter" | "resource_exit"
                    )
                })
                .collect::<Vec<_>>();
            if resource_triggers.is_empty() {
                previous = snapshot;
                continue;
            }
            let mut empty = HashMap::new();
            let processes = snapshot.as_mut().unwrap_or(&mut empty);
            let system = metrics::system_metrics(&mut sampler);
            let now = db::now_ms();
            for trigger in resource_triggers {
                let config = parse_config(trigger);
                let values = effective_conditions(&config);
                if values.is_empty() {
                    continue;
                }
                let mut all_true = true;
                let mut all_exit = true;
                let mut armed = true;
                for condition in &values {
                    let Some(value) = resource_value(
                        condition,
                        &config,
                        &system,
                        processes,
                        cpu.entry(trigger.id.clone()).or_default(),
                    ) else {
                        all_true = false;
                        all_exit = true;
                        break;
                    };
                    let true_now = compare(value, &condition.op, condition.value);
                    let exited = condition_exit(value, condition);
                    all_true &= true_now;
                    all_exit &= exited;
                    let entry = conditions.entry(trigger.id.clone()).or_default();
                    if entry.since == 0 && true_now {
                        entry.since = now;
                    }
                    if !true_now {
                        entry.since = 0;
                    }
                    if entry.since > 0 && now - entry.since < condition.for_sec * 1000 {
                        armed = false;
                    }
                }
                let entry = conditions.entry(trigger.id.clone()).or_default();
                if trigger.event_type == "resource_enter" {
                    if all_true && armed && !entry.active {
                        entry.active = true;
                        let result = fire_trigger(
                            &app,
                            trigger,
                            &format!("resource_enter:{}", config.metric),
                        );
                        if let Err(error) = result {
                            logging::app_log("ERROR", &format!("资源触发器失败: {error}"));
                        }
                    } else if !all_true && !all_exit {
                        entry.active = false;
                        entry.since = 0;
                    }
                } else {
                    if all_true && armed && !entry.active {
                        entry.active = true;
                    }
                    if entry.active && all_exit {
                        entry.active = false;
                        entry.since = 0;
                        let _ = fire_trigger(
                            &app,
                            trigger,
                            &format!("resource_exit:{}", config.metric),
                        );
                    }
                }
            }
            previous = snapshot;
        }
    });
}

pub fn start(app: &AppHandle) {
    monitor_loop(app.clone());
    wake(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> TriggerConfig {
        serde_json::from_str(json).expect("valid trigger config")
    }

    fn meta(path: Option<&str>) -> ProcessMeta {
        ProcessMeta {
            pid: 100,
            name: "Notepad.exe".into(),
            parent: 1,
            created_ms: 0,
            path: path.map(str::to_string),
        }
    }

    #[test]
    fn matches_identity_path_and_command() {
        let config = config(
            r#"{"name":"notepad.exe","parent":"explorer.exe","path":"windows\\system32","command":"notepad"}"#,
        );
        let process = meta(Some(r"C:\Windows\System32\notepad.exe"));
        assert!(process_matches(&config, &process, "explorer.exe"));
    }

    #[test]
    fn rejects_mismatched_identity_or_path() {
        let process = meta(Some(r"C:\Windows\System32\notepad.exe"));
        assert!(!process_matches(
            &config(r#"{"name":"other.exe"}"#),
            &process,
            "explorer.exe"
        ));
        assert!(!process_matches(
            &config(r#"{"path":"other"}"#),
            &process,
            "explorer.exe"
        ));
        assert!(!process_matches(
            &config(r#"{"path":"notepad"}"#),
            &meta(None),
            "explorer.exe"
        ));
    }
}
