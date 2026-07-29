use std::net::SocketAddrV4;
use std::sync::{Arc, Mutex};
use tauri::State;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamServerStatus {
    Starting,
    Ready(SocketAddrV4),
    Failed(String),
    Stopped,
}

/// Per-process stream credentials and readiness. The selected port is written
/// only after the loopback listener has bound successfully.
#[derive(Clone)]
pub struct StreamConfig {
    token: Arc<str>,
    status: Arc<Mutex<StreamServerStatus>>,
}

#[derive(Debug, Clone)]
pub struct TrustedStreamSession {
    pub token: String,
    pub base_url: String,
}

impl StreamConfig {
    pub fn new(token: String) -> Self {
        Self {
            token: Arc::from(token),
            status: Arc::new(Mutex::new(StreamServerStatus::Starting)),
        }
    }

    pub fn mark_ready(&self, address: SocketAddrV4) {
        *self.status.lock().expect("stream status mutex poisoned") =
            StreamServerStatus::Ready(address);
    }

    pub fn mark_failed(&self, error: impl Into<String>) {
        let error = crate::server::redact_sensitive(&error.into());
        *self.status.lock().expect("stream status mutex poisoned") =
            StreamServerStatus::Failed(error);
    }

    pub fn mark_stopped(&self) {
        *self.status.lock().expect("stream status mutex poisoned") = StreamServerStatus::Stopped;
    }

    pub fn trusted_session(&self) -> Result<TrustedStreamSession, String> {
        match &*self
            .status
            .lock()
            .map_err(|_| "Streaming server readiness state is unavailable".to_string())?
        {
            StreamServerStatus::Ready(address) => Ok(TrustedStreamSession {
                token: self.token.to_string(),
                base_url: format!("http://{}", address),
            }),
            StreamServerStatus::Starting => Err("Streaming server is still starting".to_string()),
            StreamServerStatus::Failed(error) => {
                Err(format!("Streaming server failed to start: {error}"))
            }
            StreamServerStatus::Stopped => Err("Streaming server is not running".to_string()),
        }
    }
}

/// Desktop-only compatibility response. Android React code uses the native
/// player plugin and never invokes this token-bearing command.
#[derive(serde::Serialize)]
pub struct StreamInfo {
    pub token: String,
    pub base_url: String,
}

#[tauri::command]
pub fn cmd_get_stream_info(config: State<'_, StreamConfig>) -> Result<StreamInfo, String> {
    let session = config.trusted_session()?;
    Ok(StreamInfo {
        token: session.token,
        base_url: session.base_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn does_not_report_stream_info_before_readiness() {
        let config = StreamConfig::new("secret".into());
        assert_eq!(
            config.trusted_session().unwrap_err(),
            "Streaming server is still starting"
        );
    }

    #[test]
    fn returns_dynamic_ipv4_address_after_readiness() {
        let config = StreamConfig::new("secret".into());
        config.mark_ready(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 49152));
        let session = config.trusted_session().unwrap();
        assert_eq!(session.base_url, "http://127.0.0.1:49152");
        assert_eq!(session.token, "secret");
    }

    #[test]
    fn redacts_startup_failures() {
        let config = StreamConfig::new("secret".into());
        config.mark_failed("failed URL ?token=secret");
        let error = config.trusted_session().unwrap_err();
        assert!(!error.contains("secret"));
        assert!(error.contains("[REDACTED]"));
    }
}
