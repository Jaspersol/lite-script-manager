use crate::models::{
    RunRecord, Schedule, ScheduleEdit, Script, ScriptEdit, Trigger, TriggerEdit, TriggerEvent,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn open(data_dir: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let db_path = data_dir.join("litescriptmanager.sqlite3");
    let conn = Connection::open(db_path).map_err(|e| format!("打开数据库失败: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS scripts (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             path TEXT NOT NULL,
             runtime_id TEXT NOT NULL,
             args_json TEXT NOT NULL DEFAULT '[]',
             cwd TEXT,
             env_json TEXT NOT NULL DEFAULT '{}',
             interpreter_path TEXT,
             process_match TEXT,
             stop_timeout_sec INTEGER NOT NULL DEFAULT 3,
             timeout_sec INTEGER,
             stop_mode TEXT NOT NULL DEFAULT 'graceful',
             max_instances INTEGER NOT NULL DEFAULT 1,
             instance_policy TEXT NOT NULL DEFAULT 'parallel',
             success_codes TEXT NOT NULL DEFAULT '0',
             content_hash TEXT,
             sort_order INTEGER NOT NULL DEFAULT 0,
             enabled INTEGER NOT NULL DEFAULT 1,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS runs (
             id TEXT PRIMARY KEY,
             script_id TEXT NOT NULL,
             status TEXT NOT NULL,
             pid INTEGER,
             started_at INTEGER NOT NULL,
             ended_at INTEGER,
             exit_code INTEGER,
             log_path TEXT,
             error TEXT,
             trigger_source TEXT NOT NULL DEFAULT 'manual',
             trigger_id TEXT,
             stop_reason TEXT,
             peak_memory_bytes INTEGER,
             cpu_time_ms INTEGER,
             timeout_sec INTEGER,
             log_bytes INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_runs_script_started ON runs(script_id, started_at DESC);
         CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
         CREATE INDEX IF NOT EXISTS idx_runs_started ON runs(started_at DESC);
         CREATE TABLE IF NOT EXISTS schedules (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             script_id TEXT NOT NULL,
             kind TEXT NOT NULL,
             expression TEXT NOT NULL,
             timezone TEXT NOT NULL DEFAULT 'local',
             enabled INTEGER NOT NULL DEFAULT 1,
             overlap_policy TEXT NOT NULL DEFAULT 'skip',
             catch_up TEXT NOT NULL DEFAULT 'none',
             timeout_sec INTEGER,
             success_codes TEXT NOT NULL DEFAULT '0',
             restart_delay_sec INTEGER NOT NULL DEFAULT 0,
             max_restarts INTEGER NOT NULL DEFAULT 0,
             backoff INTEGER NOT NULL DEFAULT 0,
             next_run_at INTEGER,
             last_run_at INTEGER,
             last_status TEXT,
             last_error TEXT,
             last_run_id TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_schedules_next ON schedules(enabled, next_run_at);
         CREATE TABLE IF NOT EXISTS triggers (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             enabled INTEGER NOT NULL DEFAULT 1,
             event_type TEXT NOT NULL,
             config_json TEXT NOT NULL DEFAULT '{}',
             actions_json TEXT NOT NULL DEFAULT '[]',
             cooldown_sec INTEGER NOT NULL DEFAULT 0,
             last_triggered_at INTEGER,
             last_result TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS trigger_events (
             id TEXT PRIMARY KEY,
             trigger_id TEXT NOT NULL,
             fired_at INTEGER NOT NULL,
             event TEXT NOT NULL,
             result TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_trigger_events_time ON trigger_events(fired_at DESC);
         CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )
    .map_err(|e| format!("初始化数据库失败: {e}"))?;

    // Add columns introduced after the first schema version.
    for (column, definition) in [
        ("interpreter_path", "TEXT"),
        ("process_match", "TEXT"),
        ("stop_timeout_sec", "INTEGER NOT NULL DEFAULT 3"),
        ("timeout_sec", "INTEGER"),
        ("stop_mode", "TEXT NOT NULL DEFAULT 'graceful'"),
        ("max_instances", "INTEGER NOT NULL DEFAULT 1"),
        ("instance_policy", "TEXT NOT NULL DEFAULT 'parallel'"),
        ("success_codes", "TEXT NOT NULL DEFAULT '0'"),
        ("content_hash", "TEXT"),
    ] {
        add_column_if_missing(&conn, "scripts", column, definition)?;
    }
    for (column, definition) in [
        ("trigger_source", "TEXT NOT NULL DEFAULT 'manual'"),
        ("trigger_id", "TEXT"),
        ("stop_reason", "TEXT"),
        ("peak_memory_bytes", "INTEGER"),
        ("cpu_time_ms", "INTEGER"),
        ("timeout_sec", "INTEGER"),
        ("log_bytes", "INTEGER"),
    ] {
        add_column_if_missing(&conn, "runs", column, definition)?;
    }

    // Any run left as running from a previous process is no longer controllable.
    let _ = conn.execute(
        "UPDATE runs SET status='failed', error='应用上次退出时运行状态未正常结束', ended_at=?1
         WHERE status IN ('running','stopping','starting','queued')",
        params![now_ms()],
    );
    set_setting(&conn, "app_version", env!("CARGO_PKG_VERSION"))?;
    Ok(conn)
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name=?2",
            params![table, column],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )
        .map_err(|e| format!("升级数据库字段失败 ({column}): {e}"))?;
    }
    Ok(())
}

pub fn list_scripts(conn: &Connection) -> Result<Vec<Script>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, name, path, runtime_id, args_json, cwd, env_json, interpreter_path,
                    process_match, stop_timeout_sec, timeout_sec, stop_mode, max_instances,
                    instance_policy, success_codes, content_hash, sort_order, enabled,
                    created_at, updated_at
             FROM scripts ORDER BY sort_order ASC, created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let args_json: String = row.get(4)?;
            let env_json: String = row.get(6)?;
            Ok(Script {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                runtime_id: row.get(3)?,
                args: serde_json::from_str(&args_json).unwrap_or_default(),
                cwd: row.get(5)?,
                env: serde_json::from_str(&env_json).unwrap_or_default(),
                interpreter_path: row.get(7)?,
                process_match: row.get(8)?,
                stop_timeout_sec: row.get(9)?,
                timeout_sec: row.get(10)?,
                stop_mode: row.get(11)?,
                max_instances: row.get(12)?,
                instance_policy: row.get(13)?,
                success_codes: row.get(14)?,
                content_hash: row.get(15)?,
                sort_order: row.get(16)?,
                enabled: row.get::<_, i64>(17)? != 0,
                created_at: row.get(18)?,
                updated_at: row.get(19)?,
                status: "stopped".into(),
                last_error: None,
                last_run_id: None,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut scripts = rows.filter_map(|r| r.ok()).collect::<Vec<_>>();
    for script in &mut scripts {
        let last = conn
            .query_row(
                "SELECT id, status, error, exit_code FROM runs WHERE script_id=?1 ORDER BY started_at DESC LIMIT 1",
                params![script.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i32>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((run_id, status, error, exit_code)) = last {
            script.last_run_id = Some(run_id);
            script.status = match status.as_str() {
                "running" | "starting" | "stopping" | "queued" => status.clone(),
                "failed" => "failed".into(),
                _ => "stopped".into(),
            };
            script.last_error = error.or_else(|| {
                if status == "failed" {
                    Some(format!("上次运行退出码 {}", exit_code.unwrap_or(-1)))
                } else {
                    None
                }
            });
        }
    }
    Ok(scripts)
}

pub fn get_script(conn: &Connection, id: &str) -> Result<Option<Script>, String> {
    let mut scripts = list_scripts(conn)?;
    Ok(scripts.drain(..).find(|s| s.id == id))
}

pub fn insert_script(conn: &Connection, script: &Script) -> Result<(), String> {
    conn.execute(
        "INSERT INTO scripts (id, name, path, runtime_id, args_json, cwd, env_json, interpreter_path,
                              process_match, stop_timeout_sec, timeout_sec, stop_mode, max_instances,
                              instance_policy, success_codes, content_hash, sort_order, enabled,
                              created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            script.id,
            script.name,
            script.path,
            script.runtime_id,
            serde_json::to_string(&script.args).unwrap_or_else(|_| "[]".into()),
            script.cwd,
            serde_json::to_string(&script.env).unwrap_or_else(|_| "{}".into()),
            script.interpreter_path,
            script.process_match,
            script.stop_timeout_sec,
            script.timeout_sec,
            script.stop_mode,
            script.max_instances,
            script.instance_policy,
            script.success_codes,
            script.content_hash,
            script.sort_order,
            if script.enabled { 1 } else { 0 },
            script.created_at,
            script.updated_at,
        ],
    )
    .map_err(|e| format!("保存脚本失败: {e}"))?;
    Ok(())
}

pub fn update_script(conn: &Connection, id: &str, edit: &ScriptEdit) -> Result<(), String> {
    conn.execute(
        "UPDATE scripts SET name=?2, args_json=?3, cwd=?4, interpreter_path=?5,
                            process_match=?6, stop_timeout_sec=?7, timeout_sec=?8,
                            stop_mode=?9, max_instances=?10, instance_policy=?11,
                            success_codes=?12, enabled=?13, updated_at=?14
         WHERE id=?1",
        params![
            id,
            edit.name,
            serde_json::to_string(&edit.args).unwrap_or_else(|_| "[]".into()),
            edit.cwd,
            edit.interpreter_path,
            edit.process_match,
            edit.stop_timeout_sec.max(1),
            edit.timeout_sec,
            edit.stop_mode,
            edit.max_instances.max(1),
            edit.instance_policy,
            edit.success_codes,
            if edit.enabled { 1 } else { 0 },
            now_ms(),
        ],
    )
    .map_err(|e| format!("更新脚本失败: {e}"))?;
    Ok(())
}

pub fn set_script_hash(conn: &Connection, id: &str, hash: Option<&str>) -> Result<(), String> {
    conn.execute(
        "UPDATE scripts SET content_hash=?2, updated_at=?3 WHERE id=?1",
        params![id, hash, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_script(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM schedules WHERE script_id=?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scripts WHERE id=?1", params![id])
        .map_err(|e| format!("删除脚本失败: {e}"))?;
    Ok(())
}

pub fn reorder_scripts(conn: &mut Connection, ids: &[String]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (index, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE scripts SET sort_order=?2, updated_at=?3 WHERE id=?1",
            params![id, index as i64, now_ms()],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn max_sort_order(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) FROM scripts",
        [],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

pub fn path_exists(conn: &Connection, path: &str) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scripts WHERE path=?1",
            params![path],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(count > 0)
}

#[allow(clippy::too_many_arguments)]
pub fn insert_run(
    conn: &Connection,
    run_id: &str,
    script_id: &str,
    pid: Option<u32>,
    started_at: i64,
    log_path: &str,
    source: &str,
    source_id: Option<&str>,
    timeout_sec: Option<i64>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO runs (id, script_id, status, pid, started_at, log_path, trigger_source, trigger_id, timeout_sec)
         VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            run_id,
            script_id,
            pid.map(|p| p as i64),
            started_at,
            log_path,
            source,
            source_id,
            timeout_sec
        ],
    )
    .map_err(|e| format!("写入运行记录失败: {e}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn finish_run(
    conn: &Connection,
    run_id: &str,
    status: &str,
    exit_code: Option<i32>,
    error: Option<&str>,
    stop_reason: Option<&str>,
    peak_memory_bytes: Option<i64>,
    cpu_time_ms: Option<i64>,
    log_bytes: Option<i64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE runs SET status=?2, ended_at=?3, exit_code=?4, error=?5, stop_reason=?6,
                         peak_memory_bytes=?7, cpu_time_ms=?8, log_bytes=?9 WHERE id=?1",
        params![
            run_id,
            status,
            now_ms(),
            exit_code,
            error,
            stop_reason,
            peak_memory_bytes,
            cpu_time_ms,
            log_bytes
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_run_log_path(conn: &Connection, run_id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT log_path FROM runs WHERE id=?1",
        params![run_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn run_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        id: row.get(0)?,
        script_id: row.get(1)?,
        script_name: row.get(2)?,
        status: row.get(3)?,
        pid: row.get::<_, Option<i64>>(4)?.map(|p| p as u32),
        started_at: row.get(5)?,
        ended_at: row.get(6)?,
        exit_code: row.get(7)?,
        log_path: row.get(8)?,
        error: row.get(9)?,
        source: row.get(10)?,
        source_id: row.get(11)?,
        stop_reason: row.get(12)?,
        peak_memory_bytes: row.get(13)?,
        cpu_time_ms: row.get(14)?,
        timeout_sec: row.get(15)?,
        log_bytes: row.get(16)?,
    })
}

const RUN_SELECT: &str = "SELECT r.id, r.script_id, COALESCE(s.name, r.script_id), r.status, r.pid,
        r.started_at, r.ended_at, r.exit_code, r.log_path, r.error, r.trigger_source,
        r.trigger_id, r.stop_reason, r.peak_memory_bytes, r.cpu_time_ms, r.timeout_sec, r.log_bytes
     FROM runs r LEFT JOIN scripts s ON s.id=r.script_id";

pub fn list_runs(
    conn: &Connection,
    script_id: Option<&str>,
    status: Option<&str>,
    source: Option<&str>,
    limit: i64,
) -> Result<Vec<RunRecord>, String> {
    let mut statement = conn
        .prepare(&format!(
            "{RUN_SELECT}
             WHERE (?1 IS NULL OR r.script_id=?1)
               AND (?2 IS NULL OR r.status=?2)
               AND (?3 IS NULL OR r.trigger_source=?3)
             ORDER BY r.started_at DESC LIMIT ?4"
        ))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(
            params![script_id, status, source, limit.clamp(1, 5000)],
            run_record,
        )
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_run(conn: &Connection, run_id: &str) -> Result<Option<RunRecord>, String> {
    conn.query_row(
        &format!("{RUN_SELECT} WHERE r.id=?1"),
        params![run_id],
        run_record,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn clear_runs(conn: &Connection, before_ms: Option<i64>) -> Result<usize, String> {
    let changed = if let Some(before) = before_ms {
        conn.execute("DELETE FROM runs WHERE started_at < ?1", params![before])
    } else {
        conn.execute("DELETE FROM runs", [])
    }
    .map_err(|e| e.to_string())?;
    Ok(changed)
}

pub fn list_schedules(conn: &Connection) -> Result<Vec<Schedule>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, name, script_id, kind, expression, timezone, enabled, overlap_policy,
                    catch_up, timeout_sec, success_codes, restart_delay_sec, max_restarts, backoff,
                    next_run_at, last_run_at, last_status, last_error, last_run_id, created_at, updated_at
             FROM schedules ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(Schedule {
                id: row.get(0)?,
                name: row.get(1)?,
                script_id: row.get(2)?,
                kind: row.get(3)?,
                expression: row.get(4)?,
                timezone: row.get(5)?,
                enabled: row.get::<_, i64>(6)? != 0,
                overlap_policy: row.get(7)?,
                catch_up: row.get(8)?,
                timeout_sec: row.get(9)?,
                success_codes: row.get(10)?,
                restart_delay_sec: row.get(11)?,
                max_restarts: row.get(12)?,
                backoff: row.get::<_, i64>(13)? != 0,
                next_run_at: row.get(14)?,
                last_run_at: row.get(15)?,
                last_status: row.get(16)?,
                last_error: row.get(17)?,
                last_run_id: row.get(18)?,
                created_at: row.get(19)?,
                updated_at: row.get(20)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_schedule(conn: &Connection, id: &str) -> Result<Option<Schedule>, String> {
    Ok(list_schedules(conn)?.drain(..).find(|s| s.id == id))
}

pub fn insert_schedule(conn: &Connection, schedule: &Schedule) -> Result<(), String> {
    conn.execute(
        "INSERT INTO schedules (id, name, script_id, kind, expression, timezone, enabled,
            overlap_policy, catch_up, timeout_sec, success_codes, restart_delay_sec, max_restarts,
            backoff, next_run_at, last_run_at, last_status, last_error, last_run_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            schedule.id, schedule.name, schedule.script_id, schedule.kind, schedule.expression,
            schedule.timezone, if schedule.enabled { 1 } else { 0 }, schedule.overlap_policy,
            schedule.catch_up, schedule.timeout_sec, schedule.success_codes,
            schedule.restart_delay_sec, schedule.max_restarts, if schedule.backoff { 1 } else { 0 },
            schedule.next_run_at, schedule.last_run_at, schedule.last_status, schedule.last_error,
            schedule.last_run_id, schedule.created_at, schedule.updated_at
        ],
    )
    .map_err(|e| format!("保存计划失败: {e}"))?;
    Ok(())
}

pub fn update_schedule(conn: &Connection, id: &str, edit: &ScheduleEdit) -> Result<(), String> {
    conn.execute(
        "UPDATE schedules SET name=?2, script_id=?3, kind=?4, expression=?5, timezone=?6,
            enabled=?7, overlap_policy=?8, catch_up=?9, timeout_sec=?10, success_codes=?11,
            restart_delay_sec=?12, max_restarts=?13, backoff=?14, updated_at=?15
         WHERE id=?1",
        params![
            id,
            edit.name,
            edit.script_id,
            edit.kind,
            edit.expression,
            edit.timezone,
            if edit.enabled { 1 } else { 0 },
            edit.overlap_policy,
            edit.catch_up,
            edit.timeout_sec,
            edit.success_codes,
            edit.restart_delay_sec.max(0),
            edit.max_restarts.max(0),
            if edit.backoff { 1 } else { 0 },
            now_ms()
        ],
    )
    .map_err(|e| format!("更新计划失败: {e}"))?;
    Ok(())
}

pub fn set_schedule_enabled(conn: &Connection, id: &str, enabled: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE schedules SET enabled=?2, updated_at=?3 WHERE id=?1",
        params![id, if enabled { 1 } else { 0 }, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_schedule_next(conn: &Connection, id: &str, next: Option<i64>) -> Result<(), String> {
    conn.execute(
        "UPDATE schedules SET next_run_at=?2, updated_at=?3 WHERE id=?1",
        params![id, next, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn mark_schedule_started(
    conn: &Connection,
    id: &str,
    run_id: &str,
    started_at: i64,
    next: Option<i64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE schedules SET last_run_at=?2, last_status='running', last_error=NULL,
            last_run_id=?3, next_run_at=?4, updated_at=?5 WHERE id=?1",
        params![id, started_at, run_id, next, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn mark_schedule_result(
    conn: &Connection,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE schedules SET last_status=?2, last_error=?3, updated_at=?4 WHERE id=?1",
        params![id, status, error, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_schedule(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM schedules WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_triggers(conn: &Connection) -> Result<Vec<Trigger>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, name, enabled, event_type, config_json, actions_json, cooldown_sec,
                    last_triggered_at, last_result, created_at, updated_at
             FROM triggers ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(Trigger {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                event_type: row.get(3)?,
                config_json: row.get(4)?,
                actions_json: row.get(5)?,
                cooldown_sec: row.get(6)?,
                last_triggered_at: row.get(7)?,
                last_result: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn get_trigger(conn: &Connection, id: &str) -> Result<Option<Trigger>, String> {
    Ok(list_triggers(conn)?.drain(..).find(|t| t.id == id))
}

pub fn insert_trigger(conn: &Connection, trigger: &Trigger) -> Result<(), String> {
    conn.execute(
        "INSERT INTO triggers (id, name, enabled, event_type, config_json, actions_json,
            cooldown_sec, last_triggered_at, last_result, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            trigger.id,
            trigger.name,
            if trigger.enabled { 1 } else { 0 },
            trigger.event_type,
            trigger.config_json,
            trigger.actions_json,
            trigger.cooldown_sec,
            trigger.last_triggered_at,
            trigger.last_result,
            trigger.created_at,
            trigger.updated_at
        ],
    )
    .map_err(|e| format!("保存触发器失败: {e}"))?;
    Ok(())
}

pub fn update_trigger(conn: &Connection, id: &str, edit: &TriggerEdit) -> Result<(), String> {
    conn.execute(
        "UPDATE triggers SET name=?2, enabled=?3, event_type=?4, config_json=?5,
            actions_json=?6, cooldown_sec=?7, updated_at=?8 WHERE id=?1",
        params![
            id,
            edit.name,
            if edit.enabled { 1 } else { 0 },
            edit.event_type,
            edit.config_json,
            edit.actions_json,
            edit.cooldown_sec.max(0),
            now_ms()
        ],
    )
    .map_err(|e| format!("更新触发器失败: {e}"))?;
    Ok(())
}

pub fn set_trigger_enabled(conn: &Connection, id: &str, enabled: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE triggers SET enabled=?2, updated_at=?3 WHERE id=?1",
        params![id, if enabled { 1 } else { 0 }, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_trigger(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM trigger_events WHERE trigger_id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM triggers WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn record_trigger_event(
    conn: &Connection,
    id: &str,
    trigger_id: &str,
    event: &str,
    result: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO trigger_events (id, trigger_id, fired_at, event, result) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, trigger_id, now_ms(), event, result],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE triggers SET last_triggered_at=?2, last_result=?3, updated_at=?2 WHERE id=?1",
        params![trigger_id, now_ms(), result],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_trigger_events(conn: &Connection, limit: i64) -> Result<Vec<TriggerEvent>, String> {
    let mut statement = conn
        .prepare(
            "SELECT e.id, e.trigger_id, COALESCE(t.name, e.trigger_id), e.fired_at, e.event, e.result
             FROM trigger_events e LEFT JOIN triggers t ON t.id=e.trigger_id
             ORDER BY e.fired_at DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![limit.clamp(1, 1000)], |row| {
            Ok(TriggerEvent {
                id: row.get(0)?,
                trigger_id: row.get(1)?,
                trigger_name: row.get(2)?,
                fired_at: row.get(3)?,
                event: row.get(4)?,
                result: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key=?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn get_settings(conn: &Connection) -> Result<HashMap<String, String>, String> {
    let mut statement = conn
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
