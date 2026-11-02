use crate::models::RuntimeInfo;
use crate::runner::RunHandle;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

pub struct AppState {
    pub db: Mutex<Connection>,
    pub runs: Mutex<HashMap<String, RunHandle>>,
    pub data_dir: PathBuf,
    pub runtime_cache: Mutex<Option<Vec<RuntimeInfo>>>,
    pub exiting: AtomicBool,
}

impl AppState {
    pub fn new(db: Connection, data_dir: PathBuf) -> Self {
        Self {
            db: Mutex::new(db),
            runs: Mutex::new(HashMap::new()),
            data_dir,
            runtime_cache: Mutex::new(None),
            exiting: AtomicBool::new(false),
        }
    }
}
