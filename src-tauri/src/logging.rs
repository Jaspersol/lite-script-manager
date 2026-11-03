use crate::models::LogLine;
use crate::settings;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

static APP_LOG: OnceLock<PathBuf> = OnceLock::new();

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
    if std::fs::metadata(path)
        .map(|value| value.len())
        .unwrap_or(0)
        > 5 * 1024 * 1024
    {
        let rotated = path.with_extension("log.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(path, rotated);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let _ = writeln!(file, "[{timestamp}] [{level}] {message}");
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
    let bytes = std::fs::read(path).map_err(|e| format!("读取脚本文件失败: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Empty `allowed_roots` means all local paths are allowed.
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

pub fn prune_logs(conn: &Connection, data_dir: &Path) -> Result<(), String> {
    let retention_days = settings::get_i64(conn, "log_retention_days", 14).max(0);
    let max_bytes = settings::get_i64(conn, "log_max_total_mb", 200).max(0) as u64 * 1024 * 1024;
    let root = data_dir.join("logs");
    if !root.exists() {
        return Ok(());
    }
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
            {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((path, modified, metadata.len()));
            }
        }
    }
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(retention_days as u64 * 86_400))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for (path, modified, _) in &files {
        if retention_days == 0 || *modified < cutoff {
            let _ = std::fs::remove_file(path);
        }
    }
    files.retain(|(_, modified, _)| retention_days > 0 && *modified >= cutoff);
    let mut total = files.iter().map(|(_, _, size)| *size).sum::<u64>();
    files.sort_by_key(|(_, modified, _)| *modified);
    for (path, _, size) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}
