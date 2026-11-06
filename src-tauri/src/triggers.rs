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
    script_id: String,
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
}

#[derive(Debug, Clone, Copy, Default)]
struct CpuHistory {
    last_cpu_ms: u64,
    last_at: i64,
    percent: f64,
}

/// Per-condition sampling state is keyed by `(trigger id, condition index)`;
/// `active` is the edge state of the trigger as a whole.
#[derive(Default)]
struct ResourceStates {
    conditions: HashMap<(String, usize), ConditionState>,
    cpu: HashMap<(String, usize), CpuHistory>,
    active: HashMap<String, bool>,
}

impl ResourceStates {
    /// Drops state of triggers that were deleted or whose definition changed.
    fn retain_unchanged(
        &mut self,
        old: &HashMap<String, String>,
        new: &HashMap<String, String>,
    ) {
        let keep = |id: &String| new.get(id).is_some() && new.get(id) == old.get(id);
        self.conditions.retain(|(id, _), _| keep(id));
        self.cpu.retain(|(id, _), _| keep(id));
        self.active.retain(|id, _| keep(id));
    }
}

fn trigger_signatures(triggers: &[Trigger]) -> HashMap<String, String> {
    triggers
        .iter()
        .map(|trigger| {
            (
                trigger.id.clone(),
                format!("{}\n{}", trigger.event_type, trigger.config_json),
            )
        })
        .collect()
}

/// Edge detector for `resource_enter`. Returns true when the trigger must fire.
/// It re-arms only once every condition has left its hysteresis band.
fn resource_enter_step(active: &mut bool, all_true: bool, armed: bool, all_exit: bool) -> bool {
    if all_true && armed && !*active {
        *active = true;
        true
    } else {
        if *active && all_exit {
            *active = false;
        }
        false
    }
}

/// Edge detector for `resource_exit`. Returns true when the trigger must fire.
fn resource_exit_step(active: &mut bool, all_true: bool, armed: bool, all_exit: bool) -> bool {
    if all_true && armed && !*active {
        *active = true;
    }
    if *active && all_exit {
        *active = false;
        return true;
    }
    false
}

/// PIDs in `subject` that are absent from `other`, or present with a different
/// creation time (the PID was reused). An unreadable creation time (0) matches.
fn changed_processes(
    subject: &HashMap<u32, ProcessMeta>,
    other: &HashMap<u32, ProcessMeta>,
) -> Vec<u32> {
    subject
        .iter()
        .filter(|(pid, meta)| {
            other.get(pid).is_none_or(|old| {
                old.created_ms != 0 && meta.created_ms != 0 && old.created_ms != meta.created_ms
            })
        })
        .map(|(pid, _)| *pid)
        .collect()
}

/// Whether a lifecycle event may fire a script start/exit trigger.
fn script_event_matches(config: &TriggerConfig, trigger_id: &str, event: &LifecycleEvent) -> bool {
    // A trigger's own action must not re-fire it, or `run_script` loops forever.
    if event.source == "trigger" && event.source_id.as_deref() == Some(trigger_id) {
        return false;
    }
    config.script_id.trim().is_empty() || config.script_id == event.script_id
}

pub fn wake(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.trigger_generation.fetch_add(1, Ordering::Relaxed);
    // `notify_one` keeps a permit if the loop is busy, so the wake-up is not lost.
    state.trigger_wake.notify_one();
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

/// Lowercased process name with a trailing `.exe` removed, so `explorer` matches `explorer.exe`.
fn process_name_key(value: &str) -> String {
    let value = lower(value);
    value
        .strip_suffix(".exe")
        .map(str::to_string)
        .unwrap_or(value)
}

fn identity_matches(config: &TriggerConfig, meta: &ProcessMeta, parent_name: &str) -> bool {
    if !config.name.trim().is_empty()
        && process_name_key(&config.name) != process_name_key(&meta.name)
    {
        return false;
    }
    if !config.parent.trim().is_empty()
        && process_name_key(&config.parent) != process_name_key(parent_name)
    {
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

/// Reads command lines for processes that a path/command trigger can match.
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

pub(crate) fn notify(app: &AppHandle, title: &str, message: &str) {
    let title = if title.trim().is_empty() {
        "LSM"
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
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        // A trigger deleted while its event was in flight must not act.
        let Some(current) = db::get_trigger(&conn, &trigger.id)? else {
            return Ok("deleted".into());
        };
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
            "stop_script" => runner::stop_with_reason(app, &action.script_id, "trigger")
                .map(|_| "stopped".to_string()),
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
                        if crate::settings::is_english(app) {
                            "Trigger fired"
                        } else {
                            "触发器已触发"
                        }
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
        overlap_policy: "skip".into(),
    };
    let event = if restart {
        runner::restart_with(app, script, options)?
    } else {
        runner::start_with(app, script, options)?
    };
    Ok(if restart {
        format!("restarted {}", event.run_id)
    } else {
        format!("started {}", event.run_id)
    })
}

/// Runs the trigger actions on the blocking pool: they sleep, snapshot
/// processes and hit SQLite, which must not occupy an async worker thread.
async fn fire_blocking(app: &AppHandle, trigger: &Trigger, event: String) -> Result<String, String> {
    let app = app.clone();
    let trigger = trigger.clone();
    match tauri::async_runtime::spawn_blocking(move || fire_trigger(&app, &trigger, &event)).await {
        Ok(result) => result,
        Err(error) => Err(format!("触发器任务失败: {error}")),
    }
}

async fn fire_process_matches(
    app: &AppHandle,
    triggers: &[Trigger],
    event_type: &str,
    current: &HashMap<u32, ProcessMeta>,
    previous: &HashMap<u32, ProcessMeta>,
) {
    let started = event_type == "process_started";
    let (subject, other) = if started {
        (current, previous)
    } else {
        (previous, current)
    };
    for pid in changed_processes(subject, other) {
        let Some(meta) = subject.get(&pid) else {
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
                    fire_blocking(app, trigger, format!("{event_type}:{}:{}", meta.name, pid))
                        .await
            {
                logging::app_log("ERROR", &format!("触发器执行失败: {error}"));
            }
        }
    }
}

async fn fire_script_event(app: &AppHandle, triggers: &[Trigger], event: &LifecycleEvent) {
    let event_type = if matches!(event.status.as_str(), "running" | "starting") {
        "script_started"
    } else {
        "script_exited"
    };
    for trigger in triggers {
        if trigger.event_type == event_type
            && script_event_matches(&parse_config(trigger), &trigger.id, event)
        {
            let _ = fire_blocking(
                app,
                trigger,
                format!("{event_type}:{}:{}", event.script_id, event.status),
            )
            .await;
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
        let mut states = ResourceStates::default();
        let mut signatures: HashMap<String, String> = HashMap::new();
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
                let next = trigger_signatures(&triggers);
                states.retain_unchanged(&signatures, &next);
                signatures = next;
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
                        fire_script_event(&app, &triggers, &event).await;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(interval_ms)) => {}
            }

            let mut snapshot: Option<HashMap<u32, ProcessMeta>> = None;
            if needs_process_snapshot(&triggers) {
                let raw = runner::snapshot_processes();
                let mut current = HashMap::with_capacity(raw.len());
                for (pid, parent, name) in raw {
                    // Read every tick: a reused PID is told apart by its creation time.
                    let created_ms = runner::process_create_time_ms(pid).unwrap_or(0);
                    // Backfill cached command lines first so only missing ones are read.
                    let path = previous
                        .as_ref()
                        .and_then(|values| values.get(&pid))
                        .filter(|old| old.created_ms == created_ms)
                        .and_then(|old| old.path.clone());
                    current.insert(
                        pid,
                        ProcessMeta {
                            pid,
                            name,
                            parent,
                            created_ms,
                            path,
                        },
                    );
                }
                enrich_process_triggers(&triggers, &mut current);
                if let Some(previous) = previous.as_ref() {
                    fire_process_matches(&app, &triggers, "process_started", &current, previous)
                        .await;
                    fire_process_matches(&app, &triggers, "process_exited", &current, previous)
                        .await;
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
                for (index, condition) in values.iter().enumerate() {
                    let key = (trigger.id.clone(), index);
                    let Some(value) = resource_value(
                        condition,
                        &config,
                        &system,
                        processes,
                        states.cpu.entry(key.clone()).or_default(),
                    ) else {
                        all_true = false;
                        all_exit = true;
                        break;
                    };
                    let true_now = compare(value, &condition.op, condition.value);
                    let exited = condition_exit(value, condition);
                    all_true &= true_now;
                    all_exit &= exited;
                    let entry = states.conditions.entry(key).or_default();
                    if !true_now {
                        entry.since = 0;
                    } else if entry.since == 0 {
                        entry.since = now;
                    }
                    if entry.since > 0 && now - entry.since < condition.for_sec * 1000 {
                        armed = false;
                    }
                }
                let label = values
                    .iter()
                    .map(|condition| condition.metric.as_str())
                    .collect::<Vec<_>>()
                    .join("+");
                let active = states.active.entry(trigger.id.clone()).or_default();
                if trigger.event_type == "resource_enter" {
                    if resource_enter_step(active, all_true, armed, all_exit)
                        && let Err(error) =
                            fire_blocking(&app, trigger, format!("resource_enter:{label}")).await
                    {
                        logging::app_log("ERROR", &format!("资源触发器失败: {error}"));
                    }
                } else if resource_exit_step(active, all_true, armed, all_exit) {
                    let _ = fire_blocking(&app, trigger, format!("resource_exit:{label}")).await;
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

    fn process(pid: u32, created_ms: i64) -> (u32, ProcessMeta) {
        (
            pid,
            ProcessMeta {
                pid,
                name: "x.exe".into(),
                parent: 0,
                created_ms,
                path: None,
            },
        )
    }

    #[test]
    fn resource_enter_fires_again_after_metric_falls_away() {
        // threshold 80, hysteresis 5: 90, 30, 90, 30, 90
        let mut active = false;
        let mut fired = 0;
        for (all_true, all_exit) in [(true, false), (false, true), (true, false), (false, true), (true, false)] {
            if resource_enter_step(&mut active, all_true, true, all_exit) {
                fired += 1;
            }
        }
        assert_eq!(fired, 3);
    }

    #[test]
    fn resource_enter_stays_latched_inside_hysteresis_band() {
        // 90, then 77 (below threshold, above exit bound), then 90 again.
        let mut active = false;
        assert!(resource_enter_step(&mut active, true, true, false));
        assert!(!resource_enter_step(&mut active, false, true, false));
        assert!(!resource_enter_step(&mut active, true, true, false));
    }

    #[test]
    fn resource_exit_fires_once_per_excursion() {
        let mut active = false;
        assert!(!resource_exit_step(&mut active, true, true, false));
        assert!(resource_exit_step(&mut active, false, true, true));
        assert!(!resource_exit_step(&mut active, false, true, true));
    }

    #[test]
    fn reused_pid_counts_as_exit_and_start() {
        let previous: HashMap<_, _> = [process(10, 1000), process(11, 1000)].into();
        let current: HashMap<_, _> = [process(10, 2000), process(11, 1000)].into();
        assert_eq!(changed_processes(&current, &previous), vec![10]);
        assert_eq!(changed_processes(&previous, &current), vec![10]);
    }

    #[test]
    fn unreadable_creation_time_does_not_look_like_a_new_process() {
        let previous: HashMap<_, _> = [process(10, 1000)].into();
        let current: HashMap<_, _> = [process(10, 0)].into();
        assert!(changed_processes(&current, &previous).is_empty());
    }

    fn lifecycle(script_id: &str, source: &str, source_id: Option<&str>) -> LifecycleEvent {
        LifecycleEvent {
            run_id: "r".into(),
            script_id: script_id.into(),
            status: "running".into(),
            exit_code: None,
            source: source.into(),
            source_id: source_id.map(str::to_string),
        }
    }

    #[test]
    fn script_event_filters_by_script_id() {
        let config = config(r#"{"script_id":"a"}"#);
        assert!(script_event_matches(&config, "t", &lifecycle("a", "manual", None)));
        assert!(!script_event_matches(&config, "t", &lifecycle("b", "manual", None)));
    }

    #[test]
    fn script_event_ignores_runs_started_by_the_trigger_itself() {
        let config = config("{}");
        assert!(!script_event_matches(&config, "t", &lifecycle("a", "trigger", Some("t"))));
        assert!(script_event_matches(&config, "t", &lifecycle("a", "trigger", Some("other"))));
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn process_name_matches_without_exe_suffix() {
        let process = ProcessMeta {
            pid: 1,
            name: "explorer.exe".into(),
            parent: 0,
            created_ms: 0,
            path: None,
        };
        let config: TriggerConfig =
            serde_json::from_str(r#"{"name":"Explorer","parent":"WinLogon"}"#).unwrap();
        assert!(process_matches(&config, &process, "winlogon.exe"));
        let other: TriggerConfig = serde_json::from_str(r#"{"name":"explorer2"}"#).unwrap();
        assert!(!process_matches(&other, &process, ""));
    }
}
