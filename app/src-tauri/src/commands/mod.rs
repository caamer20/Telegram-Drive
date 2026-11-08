use grammers_client::types::{PasswordToken, Peer};
use grammers_client::Client;
use grammers_session::storages::SqliteSession;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::models::AuthCodeRequestResult;

#[derive(Clone)]
pub struct PhoneLoginState {
    pub attempt_id: u64,
    pub phone: String,
    pub phone_code_hash: String,
    pub delivery: AuthCodeRequestResult,
    pub resend_available_at: Instant,
    pub request_in_flight: bool,
}

/// Tracks the lifecycle of the Telegram connection
///
/// IMPORTANT: The `runner_shutdown` field is critical for preventing stack overflow.
/// When reconnecting, we MUST shutdown the old runner before spawning a new one.
/// Without this, runner tasks accumulate and exhaust the thread stack.
#[derive(Clone)]
pub struct TelegramState {
    pub client: Arc<Mutex<Option<Client>>>,
    /// Retained authenticated readiness for folder-sync startup and reconnects.
    pub sync_readiness: crate::sync_engine::readiness::SyncReadiness,
    pub session: Arc<Mutex<Option<Arc<SqliteSession>>>>,
    pub phone_login: Arc<Mutex<Option<PhoneLoginState>>>,
    pub password_token: Arc<Mutex<Option<PasswordToken>>>,
    pub api_id: Arc<Mutex<Option<i32>>>,
    pub auth_attempt_counter: Arc<std::sync::atomic::AtomicU64>,
    /// Send to this channel to request runner shutdown.
    /// Uses std::sync::Mutex (not tokio) so it can be locked from synchronous
    /// contexts like the RunEvent::Exit handler.
    pub runner_shutdown: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// Counter for debugging runner lifecycle
    pub runner_count: Arc<std::sync::atomic::AtomicU32>,
    /// Cache of folder_id → Peer to avoid O(N) dialog scanning on every operation.
    /// Populated lazily on first resolve_peer call, eagerly during cmd_scan_folders.
    /// Cleared on logout.
    pub peer_cache: Arc<tokio::sync::RwLock<HashMap<i64, Peer>>>,
    /// Current request generation for each folder file load. Replacing a
    /// generation cooperatively stops stale remote scans and their events.
    pub active_file_loads: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
    /// Set of transfer IDs that have been cancelled. Checked cooperatively
    /// in upload/download chunk loops. Cleared on logout.
    pub cancelled_transfers: Arc<tokio::sync::RwLock<HashSet<String>>>,
}

pub mod api_settings;
pub mod archive;
pub mod auth;
pub mod crash_reporting;
pub mod download_destination;
pub mod file_activity;
pub mod file_inventory;
pub mod folder_groups;
pub mod fs;
pub mod network;
pub mod preview;
pub mod settings;
pub mod settings_sync;
pub mod sharing;
pub mod startup;
pub mod storage_insights;
pub mod streaming;
#[cfg(not(target_os = "ios"))]
pub mod supporter;
pub mod sync;
pub mod utils;
pub mod video_metadata;
pub mod webdav_settings;

pub use api_settings::*;
pub use archive::*;
pub use auth::*;
pub use crash_reporting::*;
pub use file_activity::*;
pub use file_inventory::*;
pub use folder_groups::*;
pub use fs::*;
pub use network::*;
pub use preview::*;
pub use settings::*;
pub use settings_sync::*;
pub use sharing::*;
pub use startup::*;
pub use storage_insights::*;
pub use streaming::*;
#[cfg(not(target_os = "ios"))]
pub use supporter::*;
pub use sync::*;
pub use utils::*;
pub use video_metadata::*;

pub(crate) mod search;
