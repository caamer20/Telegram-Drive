use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ResolvedMediaLibrarySession {
    pub base_url: String,
    pub authorization_token: String,
}

impl ResolvedMediaLibrarySession {
    pub fn new(base_url: String, authorization_token: String) -> Self {
        Self {
            base_url,
            authorization_token,
        }
    }

    pub(crate) fn validate(&self) -> crate::Result<()> {
        let port = self
            .base_url
            .strip_prefix("http://127.0.0.1:")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| {
                crate::Error::InvalidSession(
                    "trusted resolver returned a non-loopback address".into(),
                )
            })?;
        let _ = port;
        if self.authorization_token.is_empty() || self.authorization_token.len() > 512 {
            return Err(crate::Error::InvalidSession(
                "trusted resolver returned invalid credentials".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenMediaLibraryRequest {
    pub base_url: String,
    pub authorization_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClearMediaLibraryRequest {
    pub account_id: Option<i64>,
}

impl From<ResolvedMediaLibrarySession> for OpenMediaLibraryRequest {
    fn from(value: ResolvedMediaLibrarySession) -> Self {
        Self {
            base_url: value.base_url,
            authorization_token: value.authorization_token,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MediaLibraryExitReason {
    Back,
    Closed,
    Replaced,
    Error,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaLibraryResult {
    pub exit_reason: MediaLibraryExitReason,
    pub account_id: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaLibraryStatus {
    Closed,
    Opening,
    Open,
    Offline,
    Closing,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaLibraryState {
    pub status: MediaLibraryStatus,
    pub is_open: bool,
    pub account_id: Option<i64>,
    pub online: bool,
    pub sync_running: bool,
}

impl Default for MediaLibraryState {
    fn default() -> Self {
        Self {
            status: MediaLibraryStatus::Closed,
            is_open: false,
            account_id: None,
            online: false,
            sync_running: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_dynamic_ipv4_loopback_sessions() {
        ResolvedMediaLibrarySession::new("http://127.0.0.1:49152".into(), "private-token".into())
            .validate()
            .unwrap();
        for url in [
            "http://localhost:49152",
            "http://[::1]:49152",
            "https://127.0.0.1:49152",
            "http://127.0.0.1:0",
            "http://127.0.0.1:49152/path",
        ] {
            assert!(ResolvedMediaLibrarySession::new(url.into(), "token".into())
                .validate()
                .is_err());
        }
    }

    #[test]
    fn public_models_never_serialize_credentials() {
        let state = MediaLibraryState::default();
        let result = MediaLibraryResult {
            exit_reason: MediaLibraryExitReason::Back,
            account_id: Some(7),
            error: None,
        };
        for value in [
            serde_json::to_string(&state).unwrap(),
            serde_json::to_string(&result).unwrap(),
        ] {
            let lower = value.to_ascii_lowercase();
            assert!(!lower.contains("token"));
            assert!(!lower.contains("authorization"));
            assert!(!lower.contains("baseurl"));
        }
    }
}
