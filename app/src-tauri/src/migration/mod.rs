pub mod auto_engine;
pub mod commands;
pub mod db;
pub mod microsoft;
pub mod models;
pub mod upload_adapter;
pub mod worker;


use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

pub struct MigrationState {
    pub db: db::MigrationDb,
    pub ms_session: Arc<TokioMutex<Option<microsoft::MicrosoftSession>>>,
    pub worker_running: Arc<AtomicBool>,
    pub cancel_token: Arc<AtomicBool>,
    pub pause_token: Arc<AtomicBool>,
}

impl MigrationState {
    pub fn new(db: db::MigrationDb) -> Self {
        Self {
            db,
            ms_session: Arc::new(TokioMutex::new(None)),
            worker_running: Arc::new(AtomicBool::new(false)),
            cancel_token: Arc::new(AtomicBool::new(false)),
            pause_token: Arc::new(AtomicBool::new(false)),
        }
    }
}
