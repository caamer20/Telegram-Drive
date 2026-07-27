pub mod adapters;
pub mod commands;
pub mod db;
pub mod events;

pub mod microsoft;
pub mod models;
pub mod pipeline;
pub mod quota_reserve;
pub mod session_store;
pub mod telegram_idempotency;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use crate::migration::pipeline::runner::PipelineRunner;

/// Active pipeline handle
pub struct ActivePipeline {
    pub job_id: i64,
    pub runner: Arc<PipelineRunner>,
    pub cancel_token: tokio_util::sync::CancellationToken,
}

pub struct MigrationState {
    pub db: db::MigrationDb,
    pub ms_session: Arc<TokioMutex<Option<microsoft::MicrosoftSession>>>,
    pub worker_running: Arc<AtomicBool>,
    pub scan_running: Arc<AtomicBool>,
    pub scan_stop_requested: Arc<AtomicBool>,

    pub cancel_token: tokio_util::sync::CancellationToken,
    pub pause_token: Arc<AtomicBool>,
    /// Active pipeline run (only one at a time)
    pub active_pipeline: Arc<TokioMutex<Option<ActivePipeline>>>,
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
            cancel_token: tokio_util::sync::CancellationToken::new(),
            pause_token: Arc::new(AtomicBool::new(false)),
            active_pipeline: Arc::new(TokioMutex::new(None)),
        }
    }

    pub fn clone_state(&self) -> Arc<Self> {
        Arc::new(Self {
            db: self.db.clone(),
            ms_session: self.ms_session.clone(),
            worker_running: self.worker_running.clone(),
            scan_running: self.scan_running.clone(),
            scan_stop_requested: self.scan_stop_requested.clone(),
            cancel_token: self.cancel_token.clone(),
            pause_token: self.pause_token.clone(),
            active_pipeline: self.active_pipeline.clone(),
        })
    }
}
