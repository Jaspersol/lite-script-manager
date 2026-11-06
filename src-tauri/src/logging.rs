use crate::models::LogLine;
use crate::settings;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

static APP_LOG: OnceLock<PathBuf> = OnceLock::new();
static APP_LOG_FILE: Mutex<Option<AppLogFile>> = Mutex::new(None);
const APP_LOG_LIMIT: u64 = 5 * 1024 * 1024;

struct AppLogFile {
    file: std::fs::File,
    written: u64,
}

fn open_app_log(path: &Path) -> Option<AppLogFile> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let written = file.metadata().map(|value| value.len()).unwrap_or(0);
    Some(AppLogFile { file, written })
}

pub fn init_app_log(data_dir: &Path) {
    let path = data_dir.join("logs").join("app.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = APP_LOG.set(path);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        app_log("PANIC", &info.to_string());
        previous(info);
    }));
}

pub fn app_log(level: &str, message: &str) {
    let Some(path) = APP_LOG.get() else {
        eprintln!("[{level}] {message}");
        return;
    };
    let Ok(mut guard) = APP_LOG_FILE.lock() else {
        return;
    };
    if guard.is_none() {
        *guard = open_app_log(path);
    }
    if guard
        .as_ref()
        .is_some_and(|log| log.written > APP_LOG_LIMIT)
    {
        *guard = None;
        let rotated = path.with_extension("log.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(path, rotated);
        *guard = open_app_log(path);
    }
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("[{timestamp}] [{level}] {message}\n");
    if let Some(log) = guard.as_mut()
        && log.file.write_all(line.as_bytes()).is_ok()
    {
        log.written = log.written.saturating_add(line.len() as u64);
    }
}

pub fn decode_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (text, _, _) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return text.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (text, _, _) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return text.into_owned();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    let (text, _, _) = encoding_rs::GBK.decode(bytes);
    text.into_owned()
}

pub struct LogRing {
    lines: Mutex<VecDeque<LogLine>>,
    bytes: AtomicUsize,
    limit: usize,
}

impl LogRing {
    pub fn new(limit: usize) -> Self {
        Self {
            lines: Mutex::new(VecDeque::new()),
            bytes: AtomicUsize::new(0),
            limit,
        }
    }

    pub fn push(&self, line: LogLine) {
        let size = line.line.len() + line.stream.len() + 2;
        if size > self.limit {
            return;
        }
        if let Ok(mut lines) = self.lines.lock() {
            lines.push_back(line);
            let mut total = self.bytes.load(Ordering::Relaxed) + size;
            while total > self.limit {
                if let Some(removed) = lines.pop_front() {
                    total = total.saturating_sub(removed.line.len() + removed.stream.len() + 2);
                } else {
                    total = 0;
                    break;
                }
            }
            self.bytes.store(total, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> Vec<LogLine> {
        self.lines
            .lock()
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default()
    }
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("读取脚本文件失败: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("读取脚本文件失败: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// An empty root list allows every local path.
pub fn path_allowed(path: &Path, allowed_roots: &str) -> bool {
    let roots = allowed_roots
        .split([';', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return true;
    }
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path_key = canonical.to_string_lossy().to_lowercase();
    roots.iter().any(|root| {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
        let root_key = root.to_string_lossy().to_lowercase();
        path_key == root_key || path_key.starts_with(&format!("{root_key}\\"))
    })
}

static LAST_AUTO_PRUNE: Mutex<Option<Instant>> = Mutex::new(None);
const AUTO_PRUNE_INTERVAL: Duration = Duration::from_secs(600);

pub fn prune_logs(conn: &Connection, data_dir: &Path) -> Result<(), String> {
    let retention_days = settings::get_i64(conn, "log_retention_days", 14).max(0);
    let max_bytes = settings::get_i64(conn, "log_max_total_mb", 200).max(0) as u64 * 1024 * 1024;
    prune_log_files(conn, data_dir, retention_days, max_bytes)
}

/// Runs at most once per [`AUTO_PRUNE_INTERVAL`].
pub fn auto_prune_logs(db: &Mutex<Connection>, data_dir: &Path) {
    let due = LAST_AUTO_PRUNE
        .lock()
        .map(|mut last| {
            if last.is_some_and(|at| at.elapsed() < AUTO_PRUNE_INTERVAL) {
                false
            } else {
                *last = Some(Instant::now());
                true
            }
        })
        .unwrap_or(false);
    if !due {
        return;
    }
    if let Ok(conn) = db.lock() {
        let retention_days = settings::get_i64(&conn, "log_retention_days", 14).max(0);
        let max_bytes =
            settings::get_i64(&conn, "log_max_total_mb", 200).max(0) as u64 * 1024 * 1024;
        let _ = prune_log_files(&conn, data_dir, retention_days, max_bytes);
    }
}

/// Returns the lowercased canonical form used to compare log paths.
fn normalise_log_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

/// Log files that a live run still appends to; pruning must never remove them.
fn active_log_paths(conn: &Connection) -> Vec<String> {
    let Ok(mut statement) = conn.prepare(
        "SELECT log_path FROM runs
         WHERE log_path IS NOT NULL AND status IN ('running', 'stopping', 'starting', 'queued')",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.flatten()
        .map(|path| normalise_log_path(Path::new(&path)))
        .collect()
}

/// Drops the log reference from the run so the UI does not point at a missing file.
fn clear_run_log_path(conn: &Connection, path: &Path) {
    let Some(run_id) = path.file_stem().and_then(|value| value.to_str()) else {
        return;
    };
    let _ = conn.execute(
        "UPDATE runs SET log_path=NULL WHERE id=?1 AND log_path IS NOT NULL",
        params![run_id],
    );
}

/// `retention_days == 0` means "no age-based cleanup" and `max_bytes == 0` means
/// "no size cap"; both keep files instead of deleting them. The log file of a
/// live run is never removed.
fn prune_log_files(
    conn: &Connection,
    data_dir: &Path,
    retention_days: i64,
    max_bytes: u64,
) -> Result<(), String> {
    let root = data_dir.join("logs");
    if !root.exists() {
        return Ok(());
    }
    let active = active_log_paths(conn);
    let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|v| v.to_str()) == Some("log")
                && path.file_name().and_then(|v| v.to_str()) != Some("app.log")
                && !active.contains(&normalise_log_path(&path))
            {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((path, modified, metadata.len()));
            }
        }
    }

    let mut removed: Vec<PathBuf> = Vec::new();
    if retention_days > 0 {
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(retention_days as u64 * 86_400))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        for (path, modified, _) in &files {
            if *modified < cutoff && std::fs::remove_file(path).is_ok() {
                removed.push(path.clone());
            }
        }
        files.retain(|(path, _, _)| !removed.contains(path));
    }

    if max_bytes > 0 {
        let mut total = files.iter().map(|(_, _, size)| *size).sum::<u64>();
        files.sort_by_key(|(_, modified, _)| *modified);
        for (path, _, size) in files {
            if total <= max_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
                removed.push(path);
            }
        }
    }

    for path in removed {
        clear_run_log_path(conn, &path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Connection, PathBuf) {
        let conn = Connection::open_in_memory().expect("open memory database");
        conn.execute_batch(
            "CREATE TABLE runs (id TEXT PRIMARY KEY, log_path TEXT, status TEXT NOT NULL);",
        )
        .expect("create runs table");
        let dir = std::env::temp_dir().join(format!("lsm-prune-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("logs").join("script-1")).expect("create log dir");
        (conn, dir)
    }

    fn write_log(
        conn: &Connection,
        dir: &Path,
        run_id: &str,
        status: &str,
        bytes: usize,
    ) -> PathBuf {
        let path = dir
            .join("logs")
            .join("script-1")
            .join(format!("{run_id}.log"));
        std::fs::write(&path, vec![b'x'; bytes]).expect("write log file");
        conn.execute(
            "INSERT INTO runs (id, log_path, status) VALUES (?1, ?2, ?3)",
            params![run_id, path.display().to_string(), status],
        )
        .expect("insert run");
        path
    }

    fn stored_path(conn: &Connection, run_id: &str) -> Option<String> {
        conn.query_row(
            "SELECT log_path FROM runs WHERE id=?1",
            params![run_id],
            |row| row.get(0),
        )
        .expect("read log path")
    }

    #[test]
    fn zero_limits_keep_every_log() {
        let (conn, dir) = setup();
        let active = write_log(&conn, &dir, "active-run", "running", 4);
        let older = write_log(&conn, &dir, "run-a", "exited", 4);
        let newest = write_log(&conn, &dir, "run-b", "exited", 4);

        prune_log_files(&conn, &dir, 0, 0).expect("prune");

        assert!(active.exists(), "active log must survive");
        assert!(older.exists(), "retention 0 keeps logs");
        assert!(newest.exists(), "size cap 0 keeps logs");
        assert!(stored_path(&conn, "run-a").is_some());

        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn size_cap_removes_oldest_but_never_the_active_log() {
        let (conn, dir) = setup();
        let active = write_log(&conn, &dir, "active-run", "running", 4);
        let first = write_log(&conn, &dir, "run-a", "exited", 4);
        let second = write_log(&conn, &dir, "run-b", "exited", 4);

        // Two inactive files of 4 bytes each: the oldest is dropped to fit 4.
        prune_log_files(&conn, &dir, 0, 4).expect("prune");
        assert!(active.exists(), "active log must survive");
        let removed_first = !first.exists();
        let removed_second = !second.exists();
        assert!(
            removed_first ^ removed_second,
            "exactly one inactive log is removed"
        );
        if removed_first {
            assert_eq!(stored_path(&conn, "run-a"), None);
            assert!(second.exists());
            assert!(stored_path(&conn, "run-b").is_some());
        } else {
            assert_eq!(stored_path(&conn, "run-b"), None);
            assert!(first.exists());
            assert!(stored_path(&conn, "run-a").is_some());
        }

        // Shrinking the cap below the last inactive file removes it too.
        prune_log_files(&conn, &dir, 0, 1).expect("prune");
        assert!(active.exists(), "active log must always survive");
        assert!(!first.exists() && !second.exists());
        assert_eq!(stored_path(&conn, "run-a"), None);
        assert_eq!(stored_path(&conn, "run-b"), None);
        assert!(stored_path(&conn, "active-run").is_some());

        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn age_limit_keeps_fresh_logs() {
        let (conn, dir) = setup();
        let log = write_log(&conn, &dir, "run-a", "exited", 4);

        prune_log_files(&conn, &dir, 1, 0).expect("prune");
        assert!(log.exists(), "a one-day-old log is still fresh");

        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}
