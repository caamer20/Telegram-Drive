use serde::{ser::Serializer, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid media library session: {0}")]
    InvalidSession(String),
    #[error("media library is already open")]
    AlreadyOpen,
    #[error("media library is only available on Android")]
    UnsupportedPlatform,
    #[error("local media service unavailable: {0}")]
    LocalServer(String),
    #[cfg(mobile)]
    #[error("media library bridge failed: {0}")]
    PluginInvoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
