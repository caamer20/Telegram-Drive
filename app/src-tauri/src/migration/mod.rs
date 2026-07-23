pub mod auto_engine;
pub mod commands;
pub mod db;
pub mod microsoft;
pub mod models;
pub mod session_store;
pub mod upload_adapter;
pub mod worker;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;

pub struct MigrationState {
    pub db: db::MigrationDb,
    pub ms_session: Arc<TokioMutex<Option<microsoft::MicrosoftSession>>>,
    pub worker_running: Arc<AtomicBool>,
    pub scan_running: Arc<AtomicBool>,
    pub scan_stop_requested: Arc<AtomicBool>,
    pub scan_progress: Arc<Mutex<Option<models::ScanProgressPayload>>>,
    pub cancel_token: Arc<AtomicBool>,
    pub pause_token: Arc<AtomicBool>,
}

impl MigrationState {
    pub fn new(db: db::MigrationDb) -> Self {
        Self::new_with_session(db, None)
    }

    pub fn new_with_session(
        db: db::MigrationDb,
        session: Option<microsoft::MicrosoftSession>,
    ) -> Self {
        Self {
            db,
            ms_session: Arc::new(TokioMutex::new(session)),
            worker_running: Arc::new(AtomicBool::new(false)),
            scan_running: Arc::new(AtomicBool::new(false)),
            scan_stop_requested: Arc::new(AtomicBool::new(false)),
            scan_progress: Arc::new(Mutex::new(None)),
            cancel_token: Arc::new(AtomicBool::new(false)),
            pause_token: Arc::new(AtomicBool::new(false)),
        }
    }
}
