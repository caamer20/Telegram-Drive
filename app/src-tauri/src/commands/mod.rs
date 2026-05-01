use grammers_client::types::{LoginToken, PasswordToken};
use grammers_client::Client;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Tracks the lifecycle of the Telegram connection
///
/// IMPORTANT: The `runner_shutdown` field is critical for preventing stack overflow.
/// When reconnecting, we MUST shutdown the old runner before spawning a new one.
/// Without this, runner tasks accumulate and exhaust the thread stack.
#[derive(Clone)]
pub struct TelegramState {
    pub client: Arc<Mutex<Option<Client>>>,
    pub login_token: Arc<Mutex<Option<LoginToken>>>,
    pub password_token: Arc<Mutex<Option<PasswordToken>>>,
    pub api_id: Arc<Mutex<Option<i32>>>,
    /// Send to this channel to request runner shutdown.
    /// Uses std::sync::Mutex (not tokio) so it can be locked from synchronous
    /// contexts like the RunEvent::Exit handler.
    pub runner_shutdown: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// Counter for debugging runner lifecycle
    pub runner_count: Arc<std::sync::atomic::AtomicU32>,
}

pub mod auth;
pub mod fs;
pub mod network;
pub mod preview;
pub mod streaming;
pub mod utils;
pub mod vault;

pub use auth::*;
pub use fs::*;
pub use network::*;
pub use preview::*;
pub use streaming::*;
pub use utils::*;
pub use vault::*;
