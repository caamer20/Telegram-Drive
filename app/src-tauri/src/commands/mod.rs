use tokio::sync::Mutex;
use grammers_client::{Client};
use grammers_client::types::{LoginToken, PasswordToken};

pub struct TelegramState {
    pub client: Mutex<Option<Client>>,
    pub login_token: Mutex<Option<LoginToken>>,
    pub password_token: Mutex<Option<PasswordToken>>,
}

pub mod auth;
pub mod fs;
pub mod preview;
pub mod utils;

pub use auth::*;
pub use fs::*;
pub use preview::*;
pub use utils::*;
