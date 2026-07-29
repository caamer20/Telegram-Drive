use serde::{Deserialize, Serialize};

const MAX_TITLE_CHARS: usize = 256;
const MAX_FILE_NAME_CHARS: usize = 512;
const MAX_MIME_TYPE_CHARS: usize = 128;
const MAX_START_POSITION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativePlayerSource {
    pub folder_id: Option<i64>,
    pub message_id: i32,
    pub title: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub start_position_ms: Option<i64>,
    pub autoplay: Option<bool>,
}
impl NativePlayerSource {
    pub fn validate(&self) -> crate::Result<()> {
        if self.message_id <= 0 {
            return Err(crate::Error::InvalidInput(
                "messageId must be a positive integer".into(),
            ));
        }
        if self.folder_id.is_some_and(|id| id <= 0) {
            return Err(crate::Error::InvalidInput(
                "folderId must be null or a positive integer".into(),
            ));
        }
        validate_text("title", &self.title, MAX_TITLE_CHARS, false)?;
        if let Some(file_name) = &self.file_name {
            validate_text("fileName", file_name, MAX_FILE_NAME_CHARS, true)?;
        }
        if let Some(mime_type) = &self.mime_type {
            validate_text("mimeType", mime_type, MAX_MIME_TYPE_CHARS, true)?;
            if mime_type.contains(':') || mime_type.contains('/') && mime_type.contains("//") {
                return Err(crate::Error::InvalidInput("mimeType is not valid".into()));
            }
        }
        if self
            .start_position_ms
            .is_some_and(|position| !(0..=MAX_START_POSITION_MS).contains(&position))
        {
            return Err(crate::Error::InvalidInput(
                "startPositionMs is outside the supported range".into(),
            ));
        }
        Ok(())
    }
}

fn validate_text(
    field: &str,
    value: &str,
    max_chars: usize,
    allow_empty: bool,
) -> crate::Result<()> {
    let trimmed = value.trim();
    if !allow_empty && trimmed.is_empty() {
        return Err(crate::Error::InvalidInput(format!("{field} is required")));
    }
    if value.chars().count() > max_chars {
        return Err(crate::Error::InvalidInput(format!("{field} is too long")));
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("://")
        || lower.starts_with("file:")
        || lower.starts_with("content:")
        || value.chars().any(|character| character == '\0')
    {
        return Err(crate::Error::InvalidInput(format!(
            "{field} contains a forbidden URI or control value"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NativePlayerErrorCategory {
    Network,
    Authentication,
    Server,
    Container,
    VideoCodec,
    AudioCodec,
    DecoderInit,
    DecoderRuntime,
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativePlayerError {
    pub category: NativePlayerErrorCategory,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NativePlayerExitReason {
    Back,
    Ended,
    Error,
    External,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativePlayerResult {
    pub position_ms: i64,
    pub duration_ms: i64,
    pub completed: bool,
    pub exit_reason: NativePlayerExitReason,
    pub error: Option<NativePlayerError>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NativePlaybackStatus {
    Idle,
    Buffering,
    Ready,
    Ended,
    Error,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativePlaybackState {
    pub state: NativePlaybackStatus,
    pub is_playing: bool,
    pub position_ms: i64,
    pub duration_ms: i64,
}

impl Default for NativePlaybackState {
    fn default() -> Self {
        Self {
            state: NativePlaybackStatus::Idle,
            is_playing: false,
            position_ms: 0,
            duration_ms: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedStreamSource {
    pub base_url: String,
    pub token: String,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f64>,
    pub bitrate: Option<u64>,
    pub bit_depth: Option<u8>,
    pub hdr: Option<bool>,
}

impl ResolvedStreamSource {
    pub fn direct(base_url: String, token: String) -> Self {
        Self {
            base_url,
            token,
            codec: None,
            width: None,
            height: None,
            frame_rate: None,
            bitrate: None,
            bit_depth: None,
            hdr: None,
        }
    }

    pub(crate) fn validate_loopback(&self) -> crate::Result<()> {
        let port = self
            .base_url
            .strip_prefix("http://127.0.0.1:")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| {
                crate::Error::StreamServer("trusted resolver returned a non-loopback address".into())
            })?;
        let _ = port;
        if self.token.is_empty() || self.token.len() > 512 {
            return Err(crate::Error::StreamServer(
                "trusted resolver returned invalid credentials".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativePlayerRequest {
    pub folder_id: Option<i64>,
    pub message_id: i32,
    pub title: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub start_position_ms: i64,
    pub autoplay: bool,
    pub stream_url: String,
    pub authorization_token: String,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f64>,
    pub bitrate: Option<u64>,
    pub bit_depth: Option<u8>,
    pub hdr: Option<bool>,
}

impl NativePlayerRequest {
    pub(crate) fn new(source: NativePlayerSource, resolved: ResolvedStreamSource) -> Self {
        let folder = source
            .folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "home".to_string());
        Self {
            folder_id: source.folder_id,
            message_id: source.message_id,
            title: source.title,
            file_name: source.file_name,
            mime_type: source.mime_type,
            start_position_ms: source.start_position_ms.unwrap_or(0),
            autoplay: source.autoplay.unwrap_or(true),
            stream_url: format!(
                "{}/stream/{}/{}",
                resolved.base_url, folder, source.message_id
            ),
            authorization_token: resolved.token,
            codec: resolved.codec,
            width: resolved.width,
            height: resolved.height,
            frame_rate: resolved.frame_rate,
            bitrate: resolved.bitrate,
            bit_depth: resolved.bit_depth,
            hdr: resolved.hdr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_source() -> NativePlayerSource {
        NativePlayerSource {
            folder_id: None,
            message_id: 7,
            title: "Movie".into(),
            file_name: Some("movie.mkv".into()),
            mime_type: Some("video/x-matroska".into()),
            start_position_ms: Some(10),
            autoplay: Some(true),
        }
    }

    #[test]
    fn validates_identity_arguments_and_rejects_uri_injection() {
        valid_source().validate().unwrap();
        for value in ["file:///tmp/movie.mp4", "content://media/1", "https://example.test/x"] {
            let mut source = valid_source();
            source.file_name = Some(value.into());
            assert!(source.validate().is_err());
        }
        let mut source = valid_source();
        source.message_id = 0;
        assert!(source.validate().is_err());
    }

    #[test]
    fn internal_request_keeps_token_out_of_uri() {
        let request = NativePlayerRequest::new(
            valid_source(),
            ResolvedStreamSource::direct(
                "http://127.0.0.1:49152".into(),
                "top-secret".into(),
            ),
        );
        assert_eq!(request.stream_url, "http://127.0.0.1:49152/stream/home/7");
        assert!(!request.stream_url.contains("top-secret"));
    }

    #[test]
    fn serialized_public_results_never_have_token_fields() {
        let result = NativePlayerResult {
            position_ms: 1,
            duration_ms: 2,
            completed: false,
            exit_reason: NativePlayerExitReason::Back,
            error: None,
        };
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.to_ascii_lowercase().contains("token"));
        assert!(!serialized.to_ascii_lowercase().contains("url"));
    }
}
