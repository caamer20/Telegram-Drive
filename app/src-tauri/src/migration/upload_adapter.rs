use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use grammers_client::types::Peer;
use grammers_client::InputMessage;
use tokio::sync::RwLock;

use crate::commands::utils::{map_error, resolve_peer};

#[derive(Debug, Clone)]
pub struct UploadResult {
    pub message_id: Option<i32>,
    pub file_name: String,
    pub file_size: i64,
}

#[derive(Debug)]
pub enum UploadError {
    FloodWait { seconds: i64 },
    TelegramFileTooLarge(String),
    Network(String),
    Auth(String),
    Cancelled,
    Unknown(String),
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadError::FloodWait { seconds } => write!(f, "Telegram flood wait for {} seconds", seconds),
            UploadError::TelegramFileTooLarge(msg) => write!(f, "Telegram file size limit exceeded: {}", msg),
            UploadError::Network(msg) => write!(f, "Network error: {}", msg),
            UploadError::Auth(msg) => write!(f, "Authentication error: {}", msg),
            UploadError::Cancelled => write!(f, "Transfer cancelled"),
            UploadError::Unknown(msg) => write!(f, "Unknown upload error: {}", msg),
        }
    }
}

impl std::error::Error for UploadError {}


pub fn parse_flood_wait_seconds(err_str: &str) -> Option<i64> {
    if let Some(idx) = err_str.find("FLOOD_WAIT_") {
        let rest = &err_str[idx + "FLOOD_WAIT_".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<i64>().ok()
    } else if let Some(idx) = err_str.find("flood wait") {
        // Try parsing digits in message
        let digits: String = err_str[idx..].chars().filter(|c| c.is_ascii_digit()).collect();
        digits.parse::<i64>().ok()
    } else {
        None
    }
}

pub fn parse_upload_error(err_str: String) -> UploadError {
    if let Some(seconds) = parse_flood_wait_seconds(&err_str) {
        UploadError::FloodWait { seconds }
    } else if err_str.contains("FILE_TOO_LARGE") || err_str.contains("too large") || err_str.contains("telegram_file_too_large") {
        UploadError::TelegramFileTooLarge(err_str)
    } else if err_str.contains("AUTH_KEY") || err_str.contains("SESSION_EXPIRED") || err_str.contains("Unauthorized") {
        UploadError::Auth(err_str)
    } else if err_str.contains("cancelled") || err_str.contains("Cancelled") {
        UploadError::Cancelled
    } else if err_str.contains("connection") || err_str.contains("timeout") || err_str.contains("reset") {
        UploadError::Network(err_str)
    } else {
        UploadError::Unknown(err_str)
    }
}

/// Shared core upload function
pub async fn upload_core<F>(
    client: &grammers_client::Client,
    peer_cache: &Arc<RwLock<HashMap<i64, Peer>>>,
    path: &str,
    folder_id: Option<i64>,
    cancel_token: &Arc<AtomicBool>,
    progress_cb: Option<F>,
) -> Result<UploadResult, UploadError>
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    if cancel_token.load(Ordering::Relaxed) {
        return Err(UploadError::Cancelled);
    }

    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| UploadError::Unknown(format!("Failed to read file metadata: {}", e)))?;
    let file_size = meta.len();

    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let (mut reader, size, bytes_counter) = crate::commands::fs::ProgressReader::new(path)
        .await
        .map_err(|e| UploadError::Unknown(format!("Failed to create reader: {}", e)))?;

    // Spawn progress reporter if callback provided
    let progress_task = if let Some(cb) = progress_cb {
        let cancel = cancel_token.clone();
        let counter = bytes_counter.clone();
        Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                let current = counter.load(Ordering::Relaxed);
                cb(current, size);
                if current >= size || cancel.load(Ordering::Relaxed) {
                    break;
                }
            }
        }))
    } else {
        None
    };

    let client_clone = client.clone();
    let fname_clone = file_name.clone();

    let upload_task = tokio::spawn(async move {
        client_clone.upload_stream(&mut reader, size as usize, fname_clone).await
    });

    let uploaded_file = match upload_task.await {
        Ok(res) => match res {
            Ok(f) => f,
            Err(e) => {
                if let Some(t) = progress_task { t.abort(); }
                let err_msg = map_error(e);
                return Err(parse_upload_error(err_msg));
            }
        },
        Err(e) => {
            if let Some(t) = progress_task { t.abort(); }
            return Err(UploadError::Unknown(format!("Upload task join error: {}", e)));
        }
    };

    if let Some(t) = progress_task { t.abort(); }

    if cancel_token.load(Ordering::Relaxed) {
        return Err(UploadError::Cancelled);
    }

    let peer = resolve_peer(client, folder_id, peer_cache)
        .await
        .map_err(|e| UploadError::Unknown(format!("Failed to resolve peer: {}", e)))?;

    let message = InputMessage::new().text("").file(uploaded_file);

    let send_res = client.send_message(&peer, message).await;

    match send_res {
        Ok(msg) => {
            Ok(UploadResult {
                message_id: Some(msg.id()),
                file_name,
                file_size: file_size as i64,
            })
        }
        Err(e) => {
            let err_msg = map_error(e);
            Err(parse_upload_error(err_msg))
        }
    }
}
