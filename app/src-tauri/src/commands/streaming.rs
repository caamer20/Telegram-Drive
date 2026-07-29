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
    state: Arc<Mutex<VersionedStreamStatus>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionedStreamStatus {
    generation: u64,
    status: StreamServerStatus,
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
            state: Arc::new(Mutex::new(VersionedStreamStatus {
                generation: 1,
                status: StreamServerStatus::Starting,
            })),
        }
    }

    pub fn generation(&self) -> u64 {
        self.state
            .lock()
            .expect("stream status mutex poisoned")
            .generation
    }

    pub fn mark_ready(&self, generation: u64, address: SocketAddrV4) -> bool {
        if address.port() == 0 || !address.ip().is_loopback() {
            return false;
        }
        self.transition(generation, |status| match status {
            StreamServerStatus::Starting => Some(StreamServerStatus::Ready(address)),
            _ => None,
        })
    }

    pub fn mark_failed(&self, generation: u64, error: impl Into<String>) -> bool {
        let error = crate::server::redact_sensitive(&error.into());
        self.transition(generation, |status| match status {
            StreamServerStatus::Starting | StreamServerStatus::Ready(_) => {
                Some(StreamServerStatus::Failed(error))
            }
            _ => None,
        })
    }

    pub fn mark_stopped(&self, generation: u64) -> bool {
        self.transition(generation, |status| match status {
            StreamServerStatus::Starting | StreamServerStatus::Ready(_) => {
                Some(StreamServerStatus::Stopped)
            }
            _ => None,
        })
    }

    fn transition(
        &self,
        generation: u64,
        update: impl FnOnce(&StreamServerStatus) -> Option<StreamServerStatus>,
    ) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.generation != generation {
            return false;
        }
        let Some(next) = update(&state.status) else {
            return false;
        };
        state.status = next;
        true
    }

    pub fn trusted_session(&self) -> Result<TrustedStreamSession, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "Streaming server readiness state is unavailable".to_string())?;
        match &state.status {
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
        config.mark_ready(
            config.generation(),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 49152),
        );
        let session = config.trusted_session().unwrap();
        assert_eq!(session.base_url, "http://127.0.0.1:49152");
        assert_eq!(session.token, "secret");
    }

    #[test]
    fn redacts_startup_failures() {
        let config = StreamConfig::new("secret".into());
        config.mark_failed(config.generation(), "failed URL ?token=secret");
        let error = config.trusted_session().unwrap_err();
        assert!(!error.contains("secret"));
        assert!(error.contains("[REDACTED]"));
    }

    #[test]
    fn enforces_valid_transitions_and_rejects_stale_generations() {
        let config = StreamConfig::new("secret".into());
        let generation = config.generation();
        assert!(!config.mark_ready(
            generation + 1,
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 49152),
        ));
        assert!(!config.mark_ready(generation, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),));
        assert!(config.mark_ready(generation, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 49152),));
        assert!(!config.mark_ready(generation, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 49153),));
        assert!(config.mark_stopped(generation));
        assert!(!config.mark_failed(generation, "late failure"));
        assert!(config.trusted_session().is_err());
    }

    #[test]
    fn shutdown_status_is_idempotent_and_terminal() {
        let config = StreamConfig::new("secret".into());
        let generation = config.generation();
        assert!(config.mark_stopped(generation));
        assert!(!config.mark_stopped(generation));
        assert!(!config.mark_failed(generation, "late failure"));
        assert!(!config.mark_ready(generation, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 49152),));
        assert_eq!(
            config.state.lock().unwrap().status,
            StreamServerStatus::Stopped,
        );
    }

    #[test]
    fn stale_generation_cannot_update_starting_or_stopped_status() {
        let config = StreamConfig::new("secret".into());
        let generation = config.generation();
        assert!(!config.mark_failed(generation + 1, "stale failure"));
        assert!(!config.mark_stopped(generation + 1));
        assert_eq!(
            config.state.lock().unwrap().status,
            StreamServerStatus::Starting,
        );
        assert!(config.mark_stopped(generation));
        assert!(!config.mark_failed(generation + 1, "late stale failure"));
        assert_eq!(
            config.state.lock().unwrap().status,
            StreamServerStatus::Stopped,
        );
    }
}
