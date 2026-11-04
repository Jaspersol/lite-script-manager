use crate::db;
use rusqlite::Connection;
use std::collections::HashMap;

/// Keys the UI may read and write. Internal bookkeeping keys such as
/// `last_update_check_ms` are stored in the database but stay out of this list
/// so that saving settings from the frontend cannot overwrite them.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("start_with_windows", "false"),
    ("open_panel_on_start", "false"),
    ("close_main_exits", "false"),
    ("default_stop_timeout", "3"),
    ("log_retention_days", "14"),
    ("log_max_total_mb", "200"),
    ("monitor_interval_ms", "1000"),
    ("language", "zh-CN"),
    ("hotkey_show_panel", ""),
    ("allowed_roots", ""),
    ("verify_script_hash", "false"),
    ("auto_check_update", "true"),
    ("first_run_done", "false"),
];

pub fn defaults() -> HashMap<String, String> {
    DEFAULTS
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

pub fn all(conn: &Connection) -> Result<HashMap<String, String>, String> {
    let mut values = defaults();
    values.extend(db::get_settings(conn)?);
    Ok(values)
}

pub fn get(conn: &Connection, key: &str) -> Result<String, String> {
    Ok(db::get_setting(conn, key)?
        .unwrap_or_else(|| defaults().get(key).cloned().unwrap_or_default()))
}

pub fn get_i64(conn: &Connection, key: &str, fallback: i64) -> i64 {
    get(conn, key)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(fallback)
}

pub fn get_bool(conn: &Connection, key: &str, fallback: bool) -> bool {
    get(conn, key)
        .ok()
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(fallback)
}

pub fn update(conn: &Connection, values: &HashMap<String, String>) -> Result<(), String> {
    for (key, value) in values {
        if !DEFAULTS.iter().any(|(name, _)| name == key) {
            continue;
        }
        db::set_setting(conn, key, value)?;
    }
    Ok(())
}
