use grammers_client::types::Peer;
use grammers_client::InputMessage;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
            UploadError::FloodWait { seconds } => {
                write!(f, "Telegram flood wait for {} seconds", seconds)
            }
            UploadError::TelegramFileTooLarge(msg) => {
                write!(f, "Telegram file size limit exceeded: {}", msg)
            }
            UploadError::Network(msg) => write!(f, "Network error: {}", msg),
            UploadError::Auth(msg) => write!(f, "Authentication error: {}", msg),
            UploadError::Cancelled => write!(f, "Transfer cancelled"),
            UploadError::Unknown(msg) => write!(f, "Unknown upload error: {}", msg),
        }
    }
}

impl std::error::Error for UploadError {}

fn normalize_destination_id(folder_id: Option<i64>) -> Option<i64> {
    folder_id.filter(|id| *id != 0)
}

fn destination_description(folder_id: Option<i64>) -> String {
    match normalize_destination_id(folder_id) {
        Some(id) => format!("chat_id={id}"),
        None => "Saved Messages (self)".to_string(),
    }
}

fn upload_file_name(original_file_name: &str, local_path: &str) -> String {
    let original_file_name = original_file_name.trim();
    if !original_file_name.is_empty() {
        return original_file_name.to_string();
    }

    std::path::Path::new(local_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string())
}

pub fn parse_flood_wait_seconds(err_str: &str) -> Option<i64> {
    if let Some(idx) = err_str.find("FLOOD_WAIT_") {
        let rest = &err_str[idx + "FLOOD_WAIT_".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<i64>().ok()
    } else if let Some(idx) = err_str.find("flood wait") {
        // Try parsing digits in message
        let digits: String = err_str[idx..]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        digits.parse::<i64>().ok()
    } else {
        None
    }
}

pub fn parse_upload_error(err_str: String) -> UploadError {
    if let Some(seconds) = parse_flood_wait_seconds(&err_str) {
        UploadError::FloodWait { seconds }
    } else if err_str.contains("FILE_TOO_LARGE")
        || err_str.contains("too large")
        || err_str.contains("telegram_file_too_large")
    {
        UploadError::TelegramFileTooLarge(err_str)
    } else if err_str.contains("AUTH_KEY")
        || err_str.contains("SESSION_EXPIRED")
        || err_str.contains("Unauthorized")
    {
        UploadError::Auth(err_str)
    } else if err_str.contains("cancelled") || err_str.contains("Cancelled") {
        UploadError::Cancelled
    } else if err_str.contains("connection")
        || err_str.contains("timeout")
        || err_str.contains("reset")
    {
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
    original_file_name: &str,
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

    let file_name = upload_file_name(original_file_name, path);
    let normalized_folder_id = normalize_destination_id(folder_id);
    let destination = destination_description(folder_id);
    log::info!(
        "Preparing Telegram upload: file='{}', bytes={}, destination={}, requested_destination_id={:?}, local_path='{}'",
        file_name,
        file_size,
        destination,
        folder_id,
        path
    );

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
        client_clone
            .upload_stream(&mut reader, size as usize, fname_clone)
            .await
    });

    let uploaded_file = match upload_task.await {
        Ok(res) => match res {
            Ok(f) => f,
            Err(e) => {
                if let Some(t) = progress_task {
                    t.abort();
                }
                let err_msg = map_error(e);
                return Err(parse_upload_error(format!(
                    "Telegram binary upload failed [file='{}', bytes={}, local_path='{}']: {}",
                    file_name, file_size, path, err_msg
                )));
            }
        },
        Err(e) => {
            if let Some(t) = progress_task {
                t.abort();
            }
            return Err(UploadError::Unknown(format!(
                "Upload task join error: {}",
                e
            )));
        }
    };

    if let Some(t) = progress_task {
        t.abort();
    }

    if cancel_token.load(Ordering::Relaxed) {
        return Err(UploadError::Cancelled);
    }

    let peer = resolve_peer(client, normalized_folder_id, peer_cache)
        .await
        .map_err(|e| {
            UploadError::Unknown(format!(
                "Telegram peer resolution failed [destination={}, requested_destination_id={:?}, file='{}', bytes={}]: {}",
                destination, folder_id, file_name, file_size, e
            ))
        })?;

    let message = InputMessage::new().text("").file(uploaded_file);

    let send_res = client.send_message(&peer, message).await;

    match send_res {
        Ok(msg) => Ok(UploadResult {
            message_id: Some(msg.id()),
            file_name,
            file_size: file_size as i64,
        }),
        Err(e) => {
            let err_msg = map_error(e);
            Err(parse_upload_error(format!(
                "Telegram send_message failed [destination={}, requested_destination_id={:?}, file='{}', bytes={}]: {}",
                destination, folder_id, file_name, file_size, err_msg
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{destination_description, normalize_destination_id, upload_file_name};

    #[test]
    fn zero_destination_means_saved_messages() {
        assert_eq!(normalize_destination_id(Some(0)), None);
        assert_eq!(normalize_destination_id(None), None);
        assert_eq!(destination_description(Some(0)), "Saved Messages (self)");
    }

    #[test]
    fn valid_destination_id_is_preserved() {
        assert_eq!(normalize_destination_id(Some(42)), Some(42));
        assert_eq!(destination_description(Some(42)), "chat_id=42");
    }

    #[test]
    fn original_file_name_is_used_instead_of_checkpoint_name() {
        assert_eq!(
            upload_file_name("date_icon.png", "/tmp/mig_8_9067.part"),
            "date_icon.png"
        );
    }

    #[test]
    fn checkpoint_name_is_only_a_fallback_for_missing_original_name() {
        assert_eq!(
            upload_file_name("   ", "/tmp/mig_8_9067.part"),
            "mig_8_9067.part"
        );
    }
}
