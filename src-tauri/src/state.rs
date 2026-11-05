use crate::models::{LifecycleEvent, RuntimeInfo};
use crate::runner::{QueuedStart, RunHandle};
use rusqlite::Connection;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::{Notify, broadcast};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub runs: Mutex<HashMap<String, RunHandle>>,
    /// Per-script lock held from the instance-count check to `runs.insert`.
    pub start_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// FIFO of pending starts per script (`queue` overlap policy).
    pub queues: Mutex<HashMap<String, VecDeque<QueuedStart>>>,
    /// Placeholder `queued-*` run id -> real run id, for callers that persisted the placeholder.
    pub queue_alias: Mutex<HashMap<String, String>>,
    pub data_dir: PathBuf,
    pub runtime_cache: Mutex<Option<(std::time::Instant, Vec<RuntimeInfo>)>>,
    pub trigger_generation: AtomicU64,
    pub exiting: AtomicBool,
    pub adopting: AtomicBool,
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
            start_locks: Mutex::new(HashMap::new()),
            queues: Mutex::new(HashMap::new()),
            queue_alias: Mutex::new(HashMap::new()),
            data_dir,
            runtime_cache: Mutex::new(None),
            trigger_generation: AtomicU64::new(0),
            exiting: AtomicBool::new(false),
            adopting: AtomicBool::new(false),
            log_streaming: AtomicBool::new(false),
            lifecycle_tx,
            scheduler_wake: Arc::new(Notify::new()),
            trigger_wake: Arc::new(Notify::new()),
        }
    }
}
