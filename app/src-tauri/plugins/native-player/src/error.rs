use serde::{ser::Serializer, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid native player source: {0}")]
    InvalidInput(String),
    #[error("native player is already open")]
    AlreadyOpen,
    #[error("native playback is only available on Android")]
    UnsupportedPlatform,
    #[error("stream server unavailable: {0}")]
    StreamServer(String),
    #[cfg(mobile)]
    #[error("native player bridge failed: {0}")]
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
