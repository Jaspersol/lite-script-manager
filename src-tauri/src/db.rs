use crate::models::Script;
use rusqlite::{Connection, OptionalExtension, params};
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
             error TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_runs_script_started ON runs(script_id, started_at DESC);
         CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
         CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )
    .map_err(|e| format!("初始化数据库失败: {e}"))?;
    // Add columns introduced after the first schema version.
    add_column_if_missing(&conn, "scripts", "interpreter_path", "TEXT")?;
    add_column_if_missing(&conn, "scripts", "process_match", "TEXT")?;
    add_column_if_missing(
        &conn,
        "scripts",
        "stop_timeout_sec",
        "INTEGER NOT NULL DEFAULT 3",
    )?;
    // Any run left as running from a previous process is no longer controllable.
    let _ = conn.execute(
        "UPDATE runs SET status='failed', error='应用上次退出时运行状态未正常结束', ended_at=?1
         WHERE status IN ('running','stopping')",
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
            "SELECT id, name, path, runtime_id, args_json, cwd, env_json, interpreter_path, process_match, stop_timeout_sec, sort_order, enabled, created_at, updated_at
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
                sort_order: row.get(10)?,
                enabled: row.get::<_, i64>(11)? != 0,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
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
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<i32>>(3)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((run_id, status, error, exit_code)) = last {
            script.last_run_id = Some(run_id);
            script.status = match status.as_str() {
                "running" => "running".into(),
                "stopping" => "stopping".into(),
                "failed" => "failed".into(),
                "stopped" => "stopped".into(),
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
        "INSERT INTO scripts (id, name, path, runtime_id, args_json, cwd, env_json, interpreter_path, process_match, stop_timeout_sec, sort_order, enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
            script.sort_order,
            if script.enabled { 1 } else { 0 },
            script.created_at,
            script.updated_at,
        ],
    )
    .map_err(|e| format!("保存脚本失败: {e}"))?;
    Ok(())
}

pub fn update_script(
    conn: &Connection,
    id: &str,
    name: &str,
    args: &[String],
    cwd: Option<&str>,
    interpreter_path: Option<&str>,
    process_match: Option<&str>,
    stop_timeout_sec: i64,
    enabled: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE scripts SET name=?2, args_json=?3, cwd=?4, interpreter_path=?5, process_match=?6, stop_timeout_sec=?7, enabled=?8, updated_at=?9 WHERE id=?1",
        params![
            id,
            name,
            serde_json::to_string(args).unwrap_or_else(|_| "[]".into()),
            cwd,
            interpreter_path,
            process_match,
            stop_timeout_sec.max(1),
            if enabled { 1 } else { 0 },
            now_ms(),
        ],
    )
    .map_err(|e| format!("更新脚本失败: {e}"))?;
    Ok(())
}

pub fn delete_script(conn: &Connection, id: &str) -> Result<(), String> {
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

pub fn insert_run(
    conn: &Connection,
    run_id: &str,
    script_id: &str,
    pid: Option<u32>,
    started_at: i64,
    log_path: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO runs (id, script_id, status, pid, started_at, log_path) VALUES (?1, ?2, 'running', ?3, ?4, ?5)",
        params![run_id, script_id, pid.map(|p| p as i64), started_at, log_path],
    )
    .map_err(|e| format!("写入运行记录失败: {e}"))?;
    Ok(())
}

pub fn finish_run(
    conn: &Connection,
    run_id: &str,
    status: &str,
    exit_code: Option<i32>,
    error: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE runs SET status=?2, ended_at=?3, exit_code=?4, error=?5 WHERE id=?1",
        params![run_id, status, now_ms(), exit_code, error],
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

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
