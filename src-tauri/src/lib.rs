mod db;
mod models;
mod runner;
mod runtime;
mod state;
mod windows;

use models::{RunStateEvent, RuntimeInfo, RuntimeMeta, Script};
use state::AppState;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

fn now_ms() -> i64 {
    db::now_ms()
}

fn emit_changed(app: &AppHandle) {
    let _ = app.emit("scripts-changed", ());
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
                if cached_at.elapsed() < std::time::Duration::from_secs(300) {
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
        stop_timeout_sec: 3,
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
        stop_timeout_sec: 3,
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
    name: String,
    args: Vec<String>,
    cwd: Option<String>,
    interpreter_path: Option<String>,
    stop_timeout_sec: i64,
    enabled: bool,
) -> Result<Script, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::update_script(
        &conn,
        &id,
        &name,
        &args,
        cwd.as_deref(),
        interpreter_path.as_deref(),
        stop_timeout_sec,
        enabled,
    )?;
    let script = db::get_script(&conn, &id)?.ok_or_else(|| "脚本不存在".to_string())?;
    drop(conn);
    emit_changed(&app);
    Ok(script)
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
fn get_run_log(
    state: State<AppState>,
    run_id: String,
    max_lines: usize,
) -> Result<Vec<String>, String> {
    let path = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_run_log_path(&conn, &run_id)?
    };
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let content = std::fs::read_to_string(path).map_err(|e| format!("读取日志失败: {e}"))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines.max(1));
    Ok(lines[start..].iter().map(|line| line.to_string()).collect())
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
fn show_main(app: AppHandle) -> Result<(), String> {
    windows::ensure_main(&app)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn show_panel(app: AppHandle) -> Result<(), String> {
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
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let conn = db::open(&data_dir)
                .map_err(|e| Box::<dyn std::error::Error>::from(std::io::Error::other(e)))?;
            app.manage(AppState::new(conn, data_dir));
            windows::setup_tray(&app)?;
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
            delete_script,
            reorder_scripts,
            run_script,
            stop_script,
            stop_all_scripts,
            get_run_log,
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
