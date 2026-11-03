mod autostart;
mod cron;
mod db;
mod logging;
mod metrics;
mod models;
mod runner;
mod runtime;
mod scheduler;
mod settings;
mod state;
mod triggers;
mod windows;

use models::{
    RunRecord, RunStateEvent, RuntimeInfo, RuntimeMeta, Schedule, ScheduleEdit, SchedulePreview,
    Script, ScriptEdit, Trigger, TriggerEdit, TriggerEvent,
};
use state::AppState;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use uuid::Uuid;

fn now_ms() -> i64 {
    db::now_ms()
}

fn default_stop_timeout(state: &AppState) -> i64 {
    let conn = match state.db.lock() {
        Ok(conn) => conn,
        Err(_) => return 3,
    };
    settings::get_i64(&conn, "default_stop_timeout", 3).clamp(1, 60)
}

fn emit_changed(app: &AppHandle) {
    let _ = app.emit("scripts-changed", ());
}

fn read_log_tail(path: &str, max_lines: usize) -> Result<Vec<String>, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("读取日志失败: {e}"))?;
    let length = file.metadata().map(|value| value.len()).unwrap_or(0);
    let start = length.saturating_sub(2 * 1024 * 1024);
    if start > 0 {
        file.seek(SeekFrom::Start(start))
            .map_err(|e| format!("读取日志失败: {e}"))?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("读取日志失败: {e}"))?;
    let text = logging::decode_output(&bytes);
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(max_lines.max(1));
    Ok(lines[start..]
        .iter()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect())
}

#[tauri::command]
fn list_scripts(state: State<AppState>) -> Result<Vec<Script>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::list_scripts(&conn)
}

#[tauri::command]
fn list_runtime_catalog() -> Vec<RuntimeMeta> {
    runtime::catalog()
}

#[tauri::command]
async fn detect_runtimes(
    state: State<'_, AppState>,
    force: bool,
) -> Result<Vec<RuntimeInfo>, String> {
    if !force {
        if let Ok(cache) = state.runtime_cache.lock() {
            if let Some((cached_at, cached)) = cache.as_ref() {
                if cached_at.elapsed() < std::time::Duration::from_secs(3600)
                    && cached.iter().all(runtime::cached_signature_valid)
                {
                    return Ok(cached.clone());
                }
            }
        }
    }
    let detected = tauri::async_runtime::spawn_blocking(runtime::detect_all)
        .await
        .map_err(|e| format!("运行时检测任务失败: {e}"))?;
    if let Ok(mut cache) = state.runtime_cache.lock() {
        *cache = Some((std::time::Instant::now(), detected.clone()));
    }
    Ok(detected)
}

#[tauri::command]
fn create_script_from_path(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<Script, String> {
    let source = PathBuf::from(&path);
    if !source.exists() || !source.is_file() {
        return Err(format!("脚本文件不存在: {path}"));
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let spec = runtime::spec_for_extension(extension)
        .ok_or_else(|| format!("无法根据扩展名识别脚本类型: .{extension}"))?;
    let canonical = std::fs::canonicalize(&source).unwrap_or(source.clone());
    let path_key = canonical.display().to_string();
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        if db::path_exists(&conn, &path_key)? {
            return Err("这个脚本已经在列表中".to_string());
        }
    }
    let name = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("未命名脚本")
        .to_string();
    let cwd = source.parent().map(|p| p.display().to_string());
    let created_at = now_ms();
    let sort_order = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::max_sort_order(&conn)? + 1
    };
    let script = Script {
        id: Uuid::new_v4().to_string(),
        name,
        path: path_key,
        runtime_id: spec.id.to_string(),
        args: Vec::new(),
        cwd,
        env: Default::default(),
        interpreter_path: None,
        process_match: None,
        stop_timeout_sec: default_stop_timeout(&state),
        timeout_sec: None,
        stop_mode: "graceful".into(),
        max_instances: 1,
        instance_policy: "parallel".into(),
        success_codes: "0".into(),
        content_hash: None,
        sort_order,
        enabled: true,
        created_at,
        updated_at: created_at,
        status: "stopped".into(),
        last_error: None,
        last_run_id: None,
    };
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::insert_script(&conn, &script)?;
    }
    emit_changed(&app);
    Ok(script)
}

#[tauri::command]
fn create_new_script(
    app: AppHandle,
    state: State<AppState>,
    name: String,
    runtime_id: String,
    content: String,
) -> Result<Script, String> {
    if name.trim().is_empty() {
        return Err("脚本名称不能为空".to_string());
    }
    let spec =
        runtime::find_spec(&runtime_id).ok_or_else(|| format!("未知运行时: {runtime_id}"))?;
    let extension = spec.extensions.first().copied().unwrap_or("txt");
    let safe_name = name
        .trim()
        .chars()
        .map(|ch| if "<>:\"/\\|?*".contains(ch) { '_' } else { ch })
        .collect::<String>();
    let safe_name = if safe_name.is_empty() {
        "script".to_string()
    } else {
        safe_name
    };
    let script_dir = state.data_dir.join("scripts");
    std::fs::create_dir_all(&script_dir).map_err(|e| format!("创建脚本目录失败: {e}"))?;
    let mut path = script_dir.join(format!("{safe_name}.{extension}"));
    let mut counter = 1;
    while path.exists() {
        path = script_dir.join(format!("{safe_name}-{counter}.{extension}"));
        counter += 1;
    }
    std::fs::write(&path, content).map_err(|e| format!("写入脚本文件失败: {e}"))?;
    let created_at = now_ms();
    let sort_order = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::max_sort_order(&conn)? + 1
    };
    let script = Script {
        id: Uuid::new_v4().to_string(),
        name: safe_name,
        path: path.display().to_string(),
        runtime_id: spec.id.to_string(),
        args: Vec::new(),
        cwd: path.parent().map(|p| p.display().to_string()),
        env: Default::default(),
        interpreter_path: None,
        process_match: None,
        stop_timeout_sec: default_stop_timeout(&state),
        timeout_sec: None,
        stop_mode: "graceful".into(),
        max_instances: 1,
        instance_policy: "parallel".into(),
        success_codes: "0".into(),
        content_hash: None,
        sort_order,
        enabled: true,
        created_at,
        updated_at: created_at,
        status: "stopped".into(),
        last_error: None,
        last_run_id: None,
    };
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::insert_script(&conn, &script)?;
    }
    emit_changed(&app);
    Ok(script)
}

#[tauri::command]
fn update_script(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    edit: ScriptEdit,
) -> Result<Script, String> {
    if edit.name.trim().is_empty() {
        return Err("脚本名称不能为空".to_string());
    }
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::update_script(&conn, &id, &edit)?;
    let script = db::get_script(&conn, &id)?.ok_or_else(|| "脚本不存在".to_string())?;
    drop(conn);
    emit_changed(&app);
    Ok(script)
}

#[tauri::command]
fn pin_script_hash(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    pinned: bool,
) -> Result<Option<String>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    let script = db::get_script(&conn, &id)?.ok_or_else(|| "脚本不存在".to_string())?;
    let hash = if pinned {
        Some(logging::sha256_file(std::path::Path::new(&script.path))?)
    } else {
        None
    };
    db::set_script_hash(&conn, &id, hash.as_deref())?;
    drop(conn);
    emit_changed(&app);
    Ok(hash)
}

#[tauri::command]
fn delete_script(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    delete_file: bool,
) -> Result<(), String> {
    let path = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        let script = db::get_script(&conn, &id)?.ok_or_else(|| "脚本不存在".to_string())?;
        db::delete_script(&conn, &id)?;
        script.path
    };
    if delete_file {
        let path = PathBuf::from(path);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("删除脚本文件失败: {e}"))?;
        }
    }
    scheduler::wake(&app);
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
fn reorder_scripts(app: AppHandle, state: State<AppState>, ids: Vec<String>) -> Result<(), String> {
    let mut conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::reorder_scripts(&mut conn, &ids)?;
    drop(conn);
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
fn run_script(app: AppHandle, state: State<AppState>, id: String) -> Result<RunStateEvent, String> {
    let script = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_script(&conn, &id)?.ok_or_else(|| "脚本不存在".to_string())?
    };
    runner::start(&app, script)
}

#[tauri::command]
fn stop_script(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    let script_exists = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_script(&conn, &id)?.is_some()
    };
    if !script_exists {
        return Err("脚本不存在".to_string());
    }
    runner::stop(&app, &id)
}

#[tauri::command]
fn stop_all_scripts(app: AppHandle) {
    runner::stop_all(&app);
}

#[tauri::command]
async fn adopt_external_runs(app: AppHandle) -> Result<usize, String> {
    runner::adopt_external(&app)
}

#[tauri::command]
fn set_ui_visible(state: State<AppState>, visible: bool) {
    state
        .ui_visible
        .store(visible, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
fn set_log_streaming(state: State<AppState>, visible: bool) {
    state
        .log_streaming
        .store(visible, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
fn get_active_log(state: State<AppState>, run_id: String) -> Vec<models::LogLine> {
    state
        .runs
        .lock()
        .ok()
        .and_then(|runs| runs.get(&run_id).map(|run| run.ring.snapshot()))
        .unwrap_or_default()
}

#[tauri::command]
fn get_run_log(
    state: State<AppState>,
    run_id: String,
    max_lines: usize,
) -> Result<Vec<String>, String> {
    let active = state
        .runs
        .lock()
        .ok()
        .and_then(|runs| runs.get(&run_id).map(|run| run.ring.snapshot()))
        .unwrap_or_default();
    if !active.is_empty() {
        let start = active.len().saturating_sub(max_lines.max(1));
        return Ok(active[start..]
            .iter()
            .map(|line| format!("[{}] {}", line.stream, line.line))
            .collect());
    }
    let path = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_run_log_path(&conn, &run_id)?
    };
    match path {
        Some(path) if !path.is_empty() => read_log_tail(&path, max_lines),
        _ => Ok(Vec::new()),
    }
}

#[tauri::command]
fn open_log_file(state: State<AppState>, run_id: String) -> Result<(), String> {
    let path = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_run_log_path(&conn, &run_id)?
    }
    .filter(|value| !value.is_empty())
    .ok_or_else(|| "该运行没有日志文件".to_string())?;
    if !std::path::Path::new(&path).exists() {
        return Err("日志文件不存在".to_string());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开日志失败: {e}"))?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开日志失败: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
fn open_script_in_editor(state: State<AppState>, id: String) -> Result<(), String> {
    let path = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_script(&conn, &id)?.map(|script| script.path)
    }
    .ok_or_else(|| "脚本不存在".to_string())?;
    if !std::path::Path::new(&path).exists() {
        return Err("脚本文件不存在".to_string());
    }
    open_in_editor(&path)
}

#[cfg(windows)]
fn open_in_editor(path: &str) -> Result<(), String> {
    std::process::Command::new("notepad.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开记事本失败: {e}"))
}

#[cfg(not(windows))]
fn open_in_editor(path: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开编辑器失败: {e}"))
}

#[tauri::command]
fn list_runs(
    state: State<AppState>,
    script_id: Option<String>,
    status: Option<String>,
    source: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<RunRecord>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::list_runs(
        &conn,
        script_id.as_deref(),
        status.as_deref(),
        source.as_deref(),
        limit.unwrap_or(200),
    )
}

#[tauri::command]
fn get_run(state: State<AppState>, run_id: String) -> Result<Option<RunRecord>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::get_run(&conn, &run_id)
}

#[tauri::command]
fn clear_runs(
    app: AppHandle,
    state: State<AppState>,
    before_ms: Option<i64>,
) -> Result<usize, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    let count = db::clear_runs(&conn, before_ms)?;
    drop(conn);
    let _ = app.emit("runs-changed", ());
    Ok(count)
}

#[tauri::command]
fn list_schedules(state: State<AppState>) -> Result<Vec<Schedule>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::list_schedules(&conn)
}

fn schedule_next_for_edit(edit: &ScheduleEdit) -> Result<Option<i64>, String> {
    let now = now_ms();
    match edit.kind.as_str() {
        "loop" => Ok(Some(now)),
        "once" => Ok(Some(cron::parse_once(&edit.expression)?)),
        "cron" | "interval" => Ok(Some(cron::next_run_ms(
            &edit.kind,
            &edit.expression,
            &edit.timezone,
            now,
        )?)),
        other => Err(format!("未知计划类型: {other}")),
    }
}

#[tauri::command]
fn create_schedule(
    app: AppHandle,
    state: State<AppState>,
    edit: ScheduleEdit,
) -> Result<Schedule, String> {
    if edit.name.trim().is_empty() {
        return Err("计划名称不能为空".to_string());
    }
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        if db::get_script(&conn, &edit.script_id)?.is_none() {
            return Err("计划关联的脚本不存在".to_string());
        }
    }
    let next = schedule_next_for_edit(&edit)?;
    let now = now_ms();
    let schedule = Schedule {
        id: Uuid::new_v4().to_string(),
        name: edit.name,
        script_id: edit.script_id,
        kind: edit.kind,
        expression: edit.expression,
        timezone: edit.timezone,
        enabled: edit.enabled,
        overlap_policy: edit.overlap_policy,
        catch_up: edit.catch_up,
        timeout_sec: edit.timeout_sec,
        success_codes: edit.success_codes,
        restart_delay_sec: edit.restart_delay_sec,
        max_restarts: edit.max_restarts,
        backoff: edit.backoff,
        next_run_at: next,
        last_run_at: None,
        last_status: None,
        last_error: None,
        last_run_id: None,
        created_at: now,
        updated_at: now,
    };
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::insert_schedule(&conn, &schedule)?;
    }
    scheduler::wake(&app);
    let _ = app.emit("schedules-changed", ());
    Ok(schedule)
}

#[tauri::command]
fn update_schedule(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    edit: ScheduleEdit,
) -> Result<Schedule, String> {
    if edit.name.trim().is_empty() {
        return Err("计划名称不能为空".to_string());
    }
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        if db::get_script(&conn, &edit.script_id)?.is_none() {
            return Err("计划关联的脚本不存在".to_string());
        }
        db::update_schedule(&conn, &id, &edit)?;
    }
    let next = schedule_next_for_edit(&edit)?;
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::set_schedule_next(&conn, &id, next)?;
    }
    scheduler::wake(&app);
    let _ = app.emit("schedules-changed", ());
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::get_schedule(&conn, &id)?.ok_or_else(|| "计划不存在".to_string())
}

#[tauri::command]
fn delete_schedule(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::delete_schedule(&conn, &id)?;
    drop(conn);
    scheduler::wake(&app);
    let _ = app.emit("schedules-changed", ());
    Ok(())
}

#[tauri::command]
fn run_schedule_now(app: AppHandle, state: State<AppState>, id: String) -> Result<String, String> {
    let schedule = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_schedule(&conn, &id)?.ok_or_else(|| "计划不存在".to_string())?
    };
    scheduler::run_now(&app, &schedule)
}

#[tauri::command]
fn preview_schedule(
    kind: String,
    expression: String,
    timezone: Option<String>,
    count: Option<usize>,
) -> SchedulePreview {
    match cron::preview(
        &kind,
        &expression,
        timezone.as_deref().unwrap_or("local"),
        now_ms(),
        count.unwrap_or(5),
    ) {
        Ok(next) => SchedulePreview { next, error: None },
        Err(error) => SchedulePreview {
            next: Vec::new(),
            error: Some(error),
        },
    }
}

#[tauri::command]
fn list_triggers(state: State<AppState>) -> Result<Vec<Trigger>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::list_triggers(&conn)
}

fn validate_trigger_edit(edit: &TriggerEdit) -> Result<(), String> {
    if edit.name.trim().is_empty() {
        return Err("触发器名称不能为空".to_string());
    }
    serde_json::from_str::<serde_json::Value>(&edit.config_json)
        .map_err(|e| format!("触发器条件 JSON 无效: {e}"))?;
    let actions: Vec<serde_json::Value> = serde_json::from_str(&edit.actions_json)
        .map_err(|e| format!("触发器动作 JSON 无效: {e}"))?;
    const ALLOWED: &[&str] = &[
        "run_script",
        "stop_script",
        "restart_script",
        "enable_trigger",
        "disable_trigger",
        "notify",
    ];
    for action in actions {
        let kind = action
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !ALLOWED.contains(&kind) {
            return Err(format!("不允许的触发器动作: {kind}"));
        }
    }
    Ok(())
}

#[tauri::command]
fn create_trigger(
    app: AppHandle,
    state: State<AppState>,
    edit: TriggerEdit,
) -> Result<Trigger, String> {
    validate_trigger_edit(&edit)?;
    let now = now_ms();
    let trigger = Trigger {
        id: Uuid::new_v4().to_string(),
        name: edit.name,
        enabled: edit.enabled,
        event_type: edit.event_type,
        config_json: edit.config_json,
        actions_json: edit.actions_json,
        cooldown_sec: edit.cooldown_sec.max(0),
        last_triggered_at: None,
        last_result: None,
        created_at: now,
        updated_at: now,
    };
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::insert_trigger(&conn, &trigger)?;
    }
    triggers::wake(&app);
    let _ = app.emit("triggers-changed", ());
    Ok(trigger)
}

#[tauri::command]
fn update_trigger(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    edit: TriggerEdit,
) -> Result<Trigger, String> {
    validate_trigger_edit(&edit)?;
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::update_trigger(&conn, &id, &edit)?;
    }
    triggers::wake(&app);
    let _ = app.emit("triggers-changed", ());
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::get_trigger(&conn, &id)?.ok_or_else(|| "触发器不存在".to_string())
}

#[tauri::command]
fn delete_trigger(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::delete_trigger(&conn, &id)?;
    drop(conn);
    triggers::wake(&app);
    let _ = app.emit("triggers-changed", ());
    Ok(())
}

#[tauri::command]
fn trigger_now(app: AppHandle, state: State<AppState>, id: String) -> Result<String, String> {
    let _ = state;
    triggers::fire_manual(&app, &id)
}

#[tauri::command]
fn list_trigger_events(
    state: State<AppState>,
    limit: Option<i64>,
) -> Result<Vec<TriggerEvent>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::list_trigger_events(&conn, limit.unwrap_or(100))
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<HashMap<String, String>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    settings::all(&conn)
}

#[tauri::command]
fn apply_hotkey(app: &AppHandle) -> Result<(), String> {
    let hotkey = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        settings::get(&conn, "hotkey_show_panel").unwrap_or_default()
    };
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    if hotkey.trim().is_empty() {
        return Ok(());
    }
    shortcuts
        .register(hotkey.as_str())
        .map_err(|e| format!("注册快捷键失败: {e}"))
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: State<AppState>,
    values: HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        settings::update(&conn, &values)?;
    }
    logging::auto_prune_logs(&state.db, &state.data_dir);
    let autostart_enabled = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        settings::get_bool(&conn, "start_with_windows", false)
    };
    autostart::apply(autostart_enabled)?;
    apply_hotkey(&app)?;
    scheduler::wake(&app);
    triggers::wake(&app);
    let _ = app.emit("settings-changed", ());
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    settings::all(&conn)
}

#[tauri::command]
fn close_panel(app: AppHandle) {
    windows::close_panel(&app);
}

#[tauri::command]
fn close_main(app: AppHandle) {
    windows::close_main(&app);
}

#[tauri::command]
async fn show_main(app: AppHandle) -> Result<(), String> {
    windows::ensure_main(&app)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn show_panel(app: AppHandle) -> Result<(), String> {
    windows::ensure_panel(&app)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    windows::exit_app(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|arg| arg == "--show-panel") {
                let _ = windows::ensure_panel(app);
            } else {
                let _ = windows::ensure_main(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        let _ = windows::ensure_panel(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            logging::init_app_log(&data_dir);
            let conn = db::open(&data_dir)
                .map_err(|e| Box::<dyn std::error::Error>::from(std::io::Error::other(e)))?;
            let _ = logging::prune_logs(&conn, &data_dir);
            let autostart_enabled = settings::get_bool(&conn, "start_with_windows", false);
            let open_panel = settings::get_bool(&conn, "open_panel_on_start", false);
            app.manage(AppState::new(conn, data_dir));
            let _ = autostart::apply(autostart_enabled);
            windows::setup_tray(&app)?;
            scheduler::start(app.handle());
            triggers::start(app.handle());
            if let Err(error) = apply_hotkey(app.handle()) {
                logging::app_log("WARN", &error);
            }
            if open_panel {
                let _ = windows::ensure_panel(app.handle());
            }
            if std::env::args().any(|arg| arg == "--show-main") {
                let _ = windows::ensure_main(app.handle());
            }
            if std::env::args().any(|arg| arg == "--show-panel") {
                let _ = windows::ensure_panel(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_scripts,
            list_runtime_catalog,
            detect_runtimes,
            create_script_from_path,
            create_new_script,
            update_script,
            pin_script_hash,
            delete_script,
            reorder_scripts,
            run_script,
            stop_script,
            stop_all_scripts,
            adopt_external_runs,
            set_ui_visible,
            set_log_streaming,
            get_active_log,
            get_run_log,
            open_log_file,
            open_script_in_editor,
            list_runs,
            get_run,
            clear_runs,
            list_schedules,
            create_schedule,
            update_schedule,
            delete_schedule,
            run_schedule_now,
            preview_schedule,
            list_triggers,
            create_trigger,
            update_trigger,
            delete_trigger,
            trigger_now,
            list_trigger_events,
            get_settings,
            update_settings,
            close_panel,
            close_main,
            show_main,
            show_panel,
            exit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if let Some(state) = app.try_state::<AppState>() {
                    if !state.exiting.load(std::sync::atomic::Ordering::SeqCst) {
                        api.prevent_exit();
                    }
                }
            }
        });
}
