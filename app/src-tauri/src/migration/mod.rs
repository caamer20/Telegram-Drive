pub mod adapters_v2;
pub mod auto_engine;
pub mod commands;
pub mod commands_v2;
pub mod db;
pub mod disk_reserve;
pub mod manifest;
pub mod media_processor;
pub mod microsoft;
pub mod models;
pub mod pipeline_v2;
pub mod quota_reserve;
pub mod repository_v2;
pub mod schema_v2;
pub mod session_store;
pub mod telegram_idempotency;
pub mod upload_adapter;
pub mod worker;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;

use crate::migration::pipeline_v2::runner::PipelineRunner;

/// Active V2 pipeline handle
pub struct ActivePipelineV2 {
    pub job_id: i64,
    pub runner: Arc<PipelineRunner>,
    pub cancel_token: crate::migration::pipeline_v2::runner::CancellationToken,
}

pub struct MigrationState {
    pub db: db::MigrationDb,
    pub ms_session: Arc<TokioMutex<Option<microsoft::MicrosoftSession>>>,
    pub worker_running: Arc<AtomicBool>,
    pub scan_running: Arc<AtomicBool>,
    pub scan_stop_requested: Arc<AtomicBool>,
    pub scan_progress: Arc<Mutex<Option<models::ScanProgressPayload>>>,
    pub cancel_token: Arc<AtomicBool>,
    pub pause_token: Arc<AtomicBool>,
    /// Active V2 pipeline run (only one at a time)
    pub active_pipeline_v2: Arc<TokioMutex<Option<ActivePipelineV2>>>,
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
            active_pipeline_v2: Arc::new(TokioMutex::new(None)),
        }
    }

    pub fn clone_state(&self) -> Arc<Self> {
        Arc::new(Self {
            db: self.db.clone(),
            ms_session: self.ms_session.clone(),
            worker_running: self.worker_running.clone(),
            scan_running: self.scan_running.clone(),
            scan_stop_requested: self.scan_stop_requested.clone(),
            scan_progress: self.scan_progress.clone(),
            cancel_token: Arc::new(AtomicBool::new(false)),
            pause_token: Arc::new(AtomicBool::new(false)),
            active_pipeline_v2: self.active_pipeline_v2.clone(),
        })
    }
}

#[cfg(test)]
mod db_tests;
#[cfg(test)]
mod pipeline_v2_tests;
