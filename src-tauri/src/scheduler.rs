use crate::cron;
use crate::db;
use crate::logging;
use crate::models::{LifecycleEvent, Schedule, StartOptions};
use crate::runner;
use crate::state::AppState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const CATCH_UP_LIMIT: usize = 10;

pub fn wake(app: &AppHandle) {
    app.state::<AppState>().scheduler_wake.notify_waiters();
}

pub(crate) fn next_after(
    kind: &str,
    expression: &str,
    timezone: &str,
    restart_delay_sec: i64,
    after_ms: i64,
) -> Result<Option<i64>, String> {
    match kind {
        "once" => Ok(None),
        "loop" => {
            let delay = restart_delay_sec.max(0) * 1000;
            let delay = if delay > 0 {
                delay
            } else {
                cron::parse_interval(expression)?
            };
            Ok(Some(after_ms + delay))
        }
        "cron" | "interval" => cron::next_run_ms(kind, expression, timezone, after_ms).map(Some),
        other => Err(format!("未知计划类型: {other}")),
    }
}

pub(crate) fn initial_next(
    kind: &str,
    expression: &str,
    timezone: &str,
    restart_delay_sec: i64,
    after_ms: i64,
) -> Result<Option<i64>, String> {
    match kind {
        "once" => cron::parse_once(expression).map(Some),
        "loop" => Ok(Some(after_ms)),
        _ => next_after(kind, expression, timezone, restart_delay_sec, after_ms),
    }
}

fn next_for_schedule(schedule: &Schedule, after_ms: i64) -> Result<Option<i64>, String> {
    next_after(
        &schedule.kind,
        &schedule.expression,
        &schedule.timezone,
        schedule.restart_delay_sec,
        after_ms,
    )
}

fn ensure_next(app: &AppHandle, schedule: &mut Schedule) -> Result<(), String> {
    if !schedule.enabled {
        return Ok(());
    }
    if schedule.next_run_at.is_some() {
        return Ok(());
    }
    let now = db::now_ms();
    let next = initial_next(
        &schedule.kind,
        &schedule.expression,
        &schedule.timezone,
        schedule.restart_delay_sec,
        now,
    )?;
    if next.is_some() {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::set_schedule_next(&conn, &schedule.id, next)?;
        schedule.next_run_at = next;
    }
    Ok(())
}

fn load_schedules(app: &AppHandle) -> Result<Vec<Schedule>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::list_schedules(&conn)
}

fn fire_once(app: &AppHandle, schedule: &mut Schedule, scheduled_at: i64) -> Result<(), String> {
    let now = db::now_ms();
    let state = app.state::<AppState>();
    let script = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_script(&conn, &schedule.script_id)?
    };
    let Some(script) = script else {
        mark_error(app, schedule, "计划关联的脚本不存在")?;
        return Ok(());
    };
    if !script.enabled {
        mark_error(app, schedule, "计划关联的脚本已禁用")?;
        let next = if matches!(schedule.kind.as_str(), "once" | "loop") {
            None
        } else {
            next_for_schedule(schedule, now)?
        };
        set_next(app, schedule, next)?;
        return Ok(());
    }
    if schedule.overlap_policy == "queue" && runner::active_count(app, &script.id) > 0 {
        if let Ok(conn) = state.db.lock() {
            let _ =
                db::mark_schedule_result(&conn, &schedule.id, "queued", Some("等待现有实例结束"));
        }
        let next = Some(now + 2000);
        set_next(app, schedule, next)?;
        schedule.last_status = Some("queued".into());
        let _ = app.emit("schedules-changed", ());
        return Ok(());
    }
    let options = StartOptions {
        source: if schedule.kind == "loop" {
            "loop".into()
        } else {
            "schedule".into()
        },
        source_id: Some(schedule.id.clone()),
        timeout_sec: schedule.timeout_sec.or(script.timeout_sec),
        success_codes: schedule.success_codes.clone(),
        overlap_policy: schedule.overlap_policy.clone(),
    };
    let next = if matches!(schedule.kind.as_str(), "once" | "loop") {
        None
    } else {
        next_for_schedule(schedule, now.max(scheduled_at))?
    };
    match runner::start_with(app, script, options) {
        Ok(event) => {
            let state = app.state::<AppState>();
            if let Ok(conn) = state.db.lock() {
                let _ = db::mark_schedule_started(&conn, &schedule.id, &event.run_id, now, next);
            }
            schedule.last_run_at = Some(now);
            schedule.last_run_id = Some(event.run_id);
            schedule.last_status = Some(if event.status == "queued" {
                "queued".into()
            } else {
                "running".into()
            });
            schedule.next_run_at = next;
            let _ = app.emit("schedules-changed", ());
        }
        Err(error) => {
            logging::app_log("WARN", &format!("计划 {} 启动失败: {error}", schedule.name));
            mark_error(app, schedule, &error)?;
            let next = if schedule.kind == "loop" {
                Some(now + schedule.restart_delay_sec.max(1) * 1000)
            } else if schedule.kind == "once" {
                None
            } else {
                next_for_schedule(schedule, now)?
            };
            set_next(app, schedule, next)?;
            if schedule.kind == "once"
                && let Ok(conn) = state.db.lock()
            {
                let _ = db::set_schedule_enabled(&conn, &schedule.id, false);
            }
        }
    }
    Ok(())
}

fn mark_error(app: &AppHandle, schedule: &Schedule, error: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::mark_schedule_result(&conn, &schedule.id, "skipped", Some(error))?;
    drop(conn);
    let _ = app.emit("schedules-changed", ());
    Ok(())
}

fn set_next(app: &AppHandle, schedule: &mut Schedule, next: Option<i64>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
    db::set_schedule_next(&conn, &schedule.id, next)?;
    drop(conn);
    schedule.next_run_at = next;
    Ok(())
}

fn handle_due(app: &AppHandle, schedule: &mut Schedule, now: i64) -> Result<(), String> {
    let Some(mut next) = schedule.next_run_at else {
        ensure_next(app, schedule)?;
        return Ok(());
    };
    if next > now {
        return Ok(());
    }

    // If the process was suspended or the app was closed, compute how many
    // wall-clock occurrences were missed. A small grace period avoids treating
    // scheduler wake-up latency as a missed run.
    let mut missed = Vec::new();
    let mut cursor = next;
    let mut guard = 0;
    while cursor <= now.saturating_sub(2000) && guard < CATCH_UP_LIMIT {
        missed.push(cursor);
        guard += 1;
        let Some(value) = next_for_schedule(schedule, cursor)? else {
            break;
        };
        if value <= cursor {
            break;
        }
        cursor = value;
    }
    if !missed.is_empty() {
        match schedule.catch_up.as_str() {
            "all" => {
                for scheduled_at in &missed {
                    next = next_for_schedule(schedule, *scheduled_at)?.unwrap_or(next);
                    fire_once(app, schedule, *scheduled_at)?;
                }
                if schedule.next_run_at.is_none() {
                    set_next(app, schedule, next_for_schedule(schedule, now)?)?;
                }
                return Ok(());
            }
            "once" => {
                fire_once(app, schedule, missed[0])?;
                return Ok(());
            }
            _ => {
                if schedule.kind == "once" {
                    if let Ok(conn) = app.state::<AppState>().db.lock() {
                        let _ = db::mark_schedule_result(
                            &conn,
                            &schedule.id,
                            "skipped",
                            Some("已错过一次性执行时间"),
                        );
                        let _ = db::set_schedule_enabled(&conn, &schedule.id, false);
                    }
                    schedule.enabled = false;
                    schedule.last_status = Some("skipped".into());
                    let _ = app.emit("schedules-changed", ());
                    return Ok(());
                }
                next = next_for_schedule(schedule, now)?.unwrap_or(now);
                set_next(app, schedule, Some(next))?;
                let _ = app.emit("schedules-changed", ());
                return Ok(());
            }
        }
    }

    fire_once(app, schedule, next)?;
    let _ = next;
    Ok(())
}

fn scheduler_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let schedules = match load_schedules(&app) {
                Ok(values) => values,
                Err(error) => {
                    logging::app_log("ERROR", &format!("读取计划失败: {error}"));
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };
            let now = db::now_ms();
            let mut wake_at: Option<i64> = None;
            for mut schedule in schedules {
                if !schedule.enabled {
                    continue;
                }
                if let Err(error) = ensure_next(&app, &mut schedule) {
                    logging::app_log("ERROR", &format!("计算计划时间失败: {error}"));
                    continue;
                }
                match handle_due(&app, &mut schedule, now) {
                    Ok(()) => {}
                    Err(error) => {
                        logging::app_log(
                            "ERROR",
                            &format!("执行计划 {} 失败: {error}", schedule.name),
                        );
                    }
                }
                if let Some(next) = schedule.next_run_at {
                    wake_at = Some(wake_at.map_or(next, |current| current.min(next)));
                }
            }
            let wait = wake_at
                .map(|next| (next - db::now_ms()).clamp(250, 60_000) as u64)
                .unwrap_or(60_000);
            let state = app.state::<AppState>();
            tokio::select! {
                _ = state.scheduler_wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
            }
        }
    });
}

fn lifecycle_loop(app: AppHandle, failures: Arc<Mutex<HashMap<String, i64>>>) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let mut rx = state.lifecycle_tx.subscribe();
        while let Ok(event) = rx.recv().await {
            let schedules = match load_schedules(&app) {
                Ok(values) => values,
                Err(_) => continue,
            };
            for schedule in schedules {
                if schedule.last_run_id.as_deref() != Some(event.run_id.as_str()) {
                    continue;
                }
                update_schedule_result(&app, &schedule, &event, &failures);
            }
            wake(&app);
        }
    });
}

fn update_schedule_result(
    app: &AppHandle,
    schedule: &Schedule,
    event: &LifecycleEvent,
    failures: &Arc<Mutex<HashMap<String, i64>>>,
) {
    if !matches!(event.status.as_str(), "exited" | "failed" | "stopped") {
        return;
    }
    let state = app.state::<AppState>();
    if schedule.kind == "loop" && event.source == "loop" {
        let reason = {
            let conn = state.db.lock().ok();
            conn.and_then(|conn| db::get_run(&conn, &event.run_id).ok().flatten())
                .and_then(|run| run.stop_reason)
        };
        let mut count = failures.lock().ok();
        let failure_count = count
            .as_mut()
            .map(|values| values.entry(schedule.id.clone()).or_insert(0))
            .copied()
            .unwrap_or(0);
        let mut delay = schedule.restart_delay_sec.max(0);
        if delay == 0 {
            delay = cron::parse_interval(&schedule.expression)
                .map(|value| value / 1000)
                .unwrap_or(1);
        }
        let mut error = None;
        if event.status == "failed" {
            let next_failure = failure_count + 1;
            if let Some(values) = count.as_mut() {
                values.insert(schedule.id.clone(), next_failure);
            }
            if schedule.max_restarts > 0 && next_failure > schedule.max_restarts {
                error = Some(format!(
                    "达到最大重启次数 {}，循环已停止",
                    schedule.max_restarts
                ));
            } else if schedule.backoff {
                delay = [1, 2, 5, 10, 30]
                    .get((next_failure - 1) as usize)
                    .copied()
                    .unwrap_or(30);
            }
            logging::app_log(
                "WARN",
                &format!(
                    "循环 {} 第 {} 次失败，{} 秒后重试",
                    schedule.name, next_failure, delay
                ),
            );
        } else if let Some(values) = count.as_mut() {
            values.insert(schedule.id.clone(), 0);
        }

        let stopped_by_user = event.status == "stopped"
            && reason
                .as_deref()
                .is_some_and(|value| matches!(value, "user"));
        let exhausted = error
            .as_deref()
            .is_some_and(|value| value.starts_with("达到最大重启次数"));
        let can_restart =
            matches!(event.status.as_str(), "exited" | "failed") && !stopped_by_user && !exhausted;
        let next = if can_restart {
            Some(db::now_ms() + delay.max(1) * 1000)
        } else {
            None
        };
        if let Ok(conn) = state.db.lock() {
            let _ = db::mark_schedule_result(
                &conn,
                &schedule.id,
                if event.status == "failed" {
                    "failed"
                } else {
                    event.status.as_str()
                },
                error.as_deref(),
            );
            let _ = db::set_schedule_next(&conn, &schedule.id, next);
            if exhausted || stopped_by_user {
                let _ = db::set_schedule_enabled(&conn, &schedule.id, false);
            }
        }
    } else if schedule.last_run_id.as_deref() == Some(event.run_id.as_str())
        && let Ok(conn) = state.db.lock()
    {
        let _ = db::mark_schedule_result(&conn, &schedule.id, &event.status, None);
        if schedule.kind == "once" {
            let _ = db::set_schedule_enabled(&conn, &schedule.id, false);
        }
    }
    let _ = app.emit("schedules-changed", ());
}

pub fn start(app: &AppHandle) {
    let failures = Arc::new(Mutex::new(HashMap::new()));
    scheduler_loop(app.clone());
    lifecycle_loop(app.clone(), failures);
    wake(app);
}

pub fn run_now(app: &AppHandle, schedule: &Schedule) -> Result<String, String> {
    let state = app.state::<AppState>();
    let script = {
        let conn = state.db.lock().map_err(|_| "数据库锁失败".to_string())?;
        db::get_script(&conn, &schedule.script_id)?
            .ok_or_else(|| "计划关联的脚本不存在".to_string())?
    };
    let options = StartOptions {
        source: "schedule".into(),
        source_id: Some(schedule.id.clone()),
        timeout_sec: schedule.timeout_sec.or(script.timeout_sec),
        success_codes: schedule.success_codes.clone(),
        overlap_policy: schedule.overlap_policy.clone(),
    };
    let event = runner::start_with(app, script, options)?;
    if let Ok(conn) = state.db.lock() {
        let _ = db::mark_schedule_started(
            &conn,
            &schedule.id,
            &event.run_id,
            db::now_ms(),
            schedule.next_run_at,
        );
    }
    let _ = app.emit("schedules-changed", ());
    Ok(event.run_id)
}

#[cfg(test)]
mod tests {
    use super::{initial_next, next_after};

    #[test]
    fn loop_uses_restart_delay_or_expression() {
        assert_eq!(
            next_after("loop", "5m", "local", 0, 1_000).unwrap(),
            Some(301_000)
        );
        assert_eq!(
            next_after("loop", "5m", "local", 10, 1_000).unwrap(),
            Some(11_000)
        );
    }

    #[test]
    fn once_has_no_follow_up_run() {
        assert_eq!(
            next_after("once", "2026-01-01 00:00:00", "local", 0, 0).unwrap(),
            None
        );
    }

    #[test]
    fn initial_loop_starts_immediately() {
        assert_eq!(
            initial_next("loop", "1s", "local", 0, 5_000).unwrap(),
            Some(5_000)
        );
    }

    #[test]
    fn rejects_unknown_kind() {
        assert!(next_after("unknown", "", "local", 0, 0).is_err());
    }
}
