use std::sync::Arc;
use tokio::sync::Mutex;
use grammers_client::{Client};
use grammers_client::types::{LoginToken, PasswordToken};

#[derive(Clone)]
pub struct TelegramState {
    pub client: Arc<Mutex<Option<Client>>>,
    pub login_token: Arc<Mutex<Option<LoginToken>>>,
    pub password_token: Arc<Mutex<Option<PasswordToken>>>,
    pub api_id: Arc<Mutex<Option<i32>>>,
}

pub mod auth;
pub mod fs;
pub mod preview;
pub mod utils;

pub use auth::*;
pub use fs::*;
pub use preview::*;
pub use utils::*;
