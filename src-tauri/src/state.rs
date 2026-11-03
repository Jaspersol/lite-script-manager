use crate::models::{LifecycleEvent, RuntimeInfo};
use crate::runner::RunHandle;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::{Notify, broadcast};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub runs: Mutex<HashMap<String, RunHandle>>,
    pub data_dir: PathBuf,
    pub runtime_cache: Mutex<Option<(std::time::Instant, Vec<RuntimeInfo>)>>,
    pub trigger_generation: AtomicU64,
    pub exiting: AtomicBool,
    pub adopting: AtomicBool,
    pub ui_visible: AtomicBool,
    pub log_streaming: AtomicBool,
    pub lifecycle_tx: broadcast::Sender<LifecycleEvent>,
    pub scheduler_wake: Arc<Notify>,
    pub trigger_wake: Arc<Notify>,
}

impl AppState {
    pub fn new(db: Connection, data_dir: PathBuf) -> Self {
        let (lifecycle_tx, _) = broadcast::channel(256);
        Self {
            db: Mutex::new(db),
            runs: Mutex::new(HashMap::new()),
            data_dir,
            runtime_cache: Mutex::new(None),
            trigger_generation: AtomicU64::new(0),
            exiting: AtomicBool::new(false),
            adopting: AtomicBool::new(false),
            ui_visible: AtomicBool::new(false),
            log_streaming: AtomicBool::new(false),
            lifecycle_tx,
            scheduler_wake: Arc::new(Notify::new()),
            trigger_wake: Arc::new(Notify::new()),
        }
    }
}
