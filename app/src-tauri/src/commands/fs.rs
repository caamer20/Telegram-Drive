use crate::bandwidth::BandwidthManager;
use crate::commands::utils::{map_error, resolve_peer};
use crate::models::{FileMetadata, FolderMetadata};
use crate::TelegramState;
use grammers_client::types::media::Uploaded;
use grammers_client::types::{Media, Peer};
use grammers_client::InputMessage;
use grammers_tl_types as tl;
use tauri::{Emitter, State};
use tokio::io::AsyncRead;

#[tauri::command]
pub async fn cmd_create_folder(
    name: String,
    state: State<'_, TelegramState>,
) -> Result<FolderMetadata, String> {
    let client_opt = { state.client.lock().await.clone() };

    // --- MOCK ---
    if client_opt.is_none() {
        let mock_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        log::info!("[MOCK] Created folder '{}' with ID {}", name, mock_id);
        return Ok(FolderMetadata {
            id: mock_id,
            name,
            parent_id: None,
        });
    }
    // -----------
    let client = client_opt.unwrap();
    log::info!("Creating Telegram Channel: {}", name);

    let result = client
        .invoke(&tl::functions::channels::CreateChannel {
            broadcast: true,
            megagroup: false,
            title: format!("{} [TD]", name),
            about: "Telegram Drive Storage Folder\n[telegram-drive-folder]".to_string(),
            geo_point: None,
            address: None,
            for_import: false,
            forum: false,
            ttl_period: None, // Initial creation TTL
        })
        .await
        .map_err(map_error)?;

    let (chat_id, access_hash) = match result {
        tl::enums::Updates::Updates(u) => {
            let chat = u.chats.first().ok_or("No chat in updates")?;
            match chat {
                tl::enums::Chat::Channel(c) => (c.id, c.access_hash.unwrap_or(0)),
                _ => return Err("Created chat is not a channel".to_string()),
            }
        }
        _ => return Err("Unexpected response (not Updates::Updates)".to_string()),
    };

    // Explicitly Disable TTL
    let _input_channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
        channel_id: chat_id,
        access_hash,
    });

    let _ = client
        .invoke(&tl::functions::messages::SetHistoryTtl {
            peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: chat_id,
                access_hash,
            }),
            period: 0,
        })
        .await;

    Ok(FolderMetadata {
        id: chat_id,
        name,
        parent_id: None,
    })
}

#[tauri::command]
pub async fn cmd_delete_folder(
    folder_id: i64,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let client_opt = { state.client.lock().await.clone() };

    if client_opt.is_none() {
        log::info!("[MOCK] Deleted folder ID {}", folder_id);
        return Ok(true);
    }
    let client = client_opt.unwrap();
    log::info!("Deleting folder/channel: {}", folder_id);

    let peer = resolve_peer(&client, Some(folder_id)).await?;

    let input_channel = match peer {
        Peer::Channel(c) => {
            let chan = &c.raw;
            tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: chan.id,
                access_hash: chan.access_hash.ok_or("No access hash for channel")?,
            })
        }
        _ => return Err("Only channels (folders) can be deleted.".to_string()),
    };

    client
        .invoke(&tl::functions::channels::DeleteChannel {
            channel: input_channel,
        })
        .await
        .map_err(|e| format!("Failed to delete channel: {}", e))?;

    Ok(true)
}

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    id: String,
    percent: u8,
}

pub async fn send_uploaded_to_folder(
    client: &grammers_client::Client,
    uploaded: Uploaded,
    folder_id: Option<i64>,
) -> Result<(), String> {
    let peer = resolve_peer(client, folder_id).await?;
    let message = InputMessage::new().text("").file(uploaded);
    client
        .send_message(&peer, message)
        .await
        .map_err(map_error)?;
    Ok(())
}

pub async fn upload_stream_to_folder<S: AsyncRead + Unpin>(
    client: &grammers_client::Client,
    stream: &mut S,
    size: usize,
    filename: String,
    folder_id: Option<i64>,
) -> Result<(), String> {
    let uploaded = client
        .upload_stream(stream, size, filename)
        .await
        .map_err(map_error)?;

    send_uploaded_to_folder(client, uploaded, folder_id).await
}

pub async fn upload_path_to_folder(
    client: &grammers_client::Client,
    path: &str,
    folder_id: Option<i64>,
) -> Result<(), String> {
    let path_clone = path.to_string();
    let client_clone = client.clone();
    let uploaded_file =
        tauri::async_runtime::spawn(async move { client_clone.upload_file(&path_clone).await })
            .await
            .map_err(|e| format!("Task join error: {}", e))?
            .map_err(map_error)?;

    send_uploaded_to_folder(client, uploaded_file, folder_id).await
}

#[tauri::command]
pub async fn cmd_upload_file(
    path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    bw_state.can_transfer(size)?;

    let tid = transfer_id.unwrap_or_default();

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!("[MOCK] Uploaded file {} to {:?}", path, folder_id);
        bw_state.add_up(size);
        return Ok("Mock upload successful".to_string());
    }
    let client = client_opt.unwrap();

    if !tid.is_empty() {
        let _ = app_handle.emit(
            "upload-progress",
            ProgressPayload {
                id: tid.clone(),
                percent: 0,
            },
        );
    }

    upload_path_to_folder(&client, &path, folder_id).await?;

    bw_state.add_up(size);

    if !tid.is_empty() {
        let _ = app_handle.emit(
            "upload-progress",
            ProgressPayload {
                id: tid,
                percent: 100,
            },
        );
    }

    Ok("File uploaded successfully".to_string())
}

#[tauri::command]
pub async fn cmd_delete_file(
    message_id: i32,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!(
            "[MOCK] Deleted message {} from folder {:?}",
            message_id,
            folder_id
        );
        return Ok(true);
    }
    let client = client_opt.unwrap();

    let peer = resolve_peer(&client, folder_id).await?;
    client
        .delete_messages(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_download_file(
    message_id: i32,
    save_path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    let tid = transfer_id.unwrap_or_default();

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!(
            "[MOCK] Downloaded message {} from {:?} to {}",
            message_id,
            folder_id,
            save_path
        );
        if let Err(e) = std::fs::write(&save_path, b"Mock Content") {
            return Err(e.to_string());
        }
        return Ok("Download successful".to_string());
    }
    let client = client_opt.unwrap();

    let peer = resolve_peer(&client, folder_id).await?;

    // Use get_messages_by_id for efficient message lookup (same as server.rs)
    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;

    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "Message not found".to_string())?;

    let media = msg
        .media()
        .ok_or_else(|| "No media in message".to_string())?;

    let total_size = match &media {
        Media::Document(d) => d.size() as u64,
        Media::Photo(_) => 1024 * 1024,
        _ => 0,
    };

    bw_state.can_transfer(total_size)?;

    // Emit start
    if !tid.is_empty() {
        let _ = app_handle.emit(
            "download-progress",
            ProgressPayload {
                id: tid.clone(),
                percent: 0,
            },
        );
    }

    // Stream download with per-chunk progress
    let mut download_iter = client.iter_download(&media);
    let mut file = std::fs::File::create(&save_path).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;

    while let Some(chunk) = download_iter.next().await.transpose() {
        let bytes = chunk.map_err(|e| format!("Download chunk error: {}", e))?;
        std::io::Write::write_all(&mut file, &bytes).map_err(|e| e.to_string())?;
        downloaded += bytes.len() as u64;

        if !tid.is_empty() && total_size > 0 {
            let percent = ((downloaded as f64 / total_size as f64) * 100.0).min(100.0) as u8;
            // Only emit when percent actually changes to avoid event spam
            if percent != last_percent {
                last_percent = percent;
                let _ = app_handle.emit(
                    "download-progress",
                    ProgressPayload {
                        id: tid.clone(),
                        percent,
                    },
                );
            }
        }
    }

    bw_state.add_down(total_size);

    // Emit completion
    if !tid.is_empty() {
        let _ = app_handle.emit(
            "download-progress",
            ProgressPayload {
                id: tid,
                percent: 100,
            },
        );
    }

    Ok("Download successful".to_string())
}

#[tauri::command]
pub async fn cmd_move_files(
    message_ids: Vec<i32>,
    source_folder_id: Option<i64>,
    target_folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    if source_folder_id == target_folder_id {
        return Ok(true);
    }
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!(
            "[MOCK] Moved msgs {:?} from {:?} to {:?}",
            message_ids,
            source_folder_id,
            target_folder_id
        );
        return Ok(true);
    }
    let client = client_opt.unwrap();

    let source_peer = resolve_peer(&client, source_folder_id).await?;
    let target_peer = resolve_peer(&client, target_folder_id).await?;

    match client
        .forward_messages(&target_peer, &message_ids, &source_peer)
        .await
    {
        Ok(_) => {}
        Err(e) => return Err(format!("Forward failed: {}", e)),
    }

    match client.delete_messages(&source_peer, &message_ids).await {
        Ok(_) => {}
        Err(e) => return Err(format!("Delete original failed: {}", e)),
    }

    Ok(true)
}

#[tauri::command]
pub async fn cmd_get_files(
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<Vec<FileMetadata>, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!("[MOCK] Returning mock files for folder {:?}", folder_id);
        return Ok(Vec::new()); // No mock files for now
    }
    let client = client_opt.unwrap();
    let mut files = Vec::new();

    let peer = resolve_peer(&client, folder_id).await?;

    let mut msgs = client.iter_messages(&peer);
    while let Some(msg) = msgs.next().await.map_err(|e| e.to_string())? {
        if let Some(doc) = msg.media() {
            let (name, size, mime, ext) = match doc {
                Media::Document(d) => {
                    let n = d.name().to_string();
                    let s = d.size();
                    let m = d.mime_type().map(|s| s.to_string());
                    let e = std::path::Path::new(&n)
                        .extension()
                        .map(|os| os.to_str().unwrap_or("").to_string());
                    (n, s, m, e)
                }
                Media::Photo(_) => (
                    "Photo.jpg".to_string(),
                    0,
                    Some("image/jpeg".into()),
                    Some("jpg".into()),
                ),
                _ => ("Unknown".to_string(), 0, None, None),
            };
            files.push(FileMetadata {
                id: msg.id() as i64,
                folder_id,
                name,
                size: size as u64,
                mime_type: mime,
                file_ext: ext,
                created_at: msg.date().to_string(),
                icon_type: "file".into(),
            });
        }
    }

    Ok(files)
}

#[tauri::command]
pub async fn cmd_search_global(
    query: String,
    state: State<'_, TelegramState>,
) -> Result<Vec<FileMetadata>, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        return Ok(Vec::new());
    }
    let client = client_opt.unwrap();
    let mut files = Vec::new();

    log::info!("Searching global for: {}", query);

    let result = client
        .invoke(&tl::functions::messages::SearchGlobal {
            q: query,
            filter: tl::enums::MessagesFilter::InputMessagesFilterDocument,
            min_date: 0,
            max_date: 0,
            offset_rate: 0,
            offset_peer: tl::enums::InputPeer::Empty,
            offset_id: 0,
            limit: 50,
            folder_id: None,
            broadcasts_only: false,
            groups_only: false,
            users_only: false,
        })
        .await
        .map_err(map_error)?;

    if let tl::enums::messages::Messages::Messages(msgs) = result {
        for msg in msgs.messages {
            if let tl::enums::Message::Message(m) = msg {
                if let Some(tl::enums::MessageMedia::Document(d)) = m.media {
                    if let tl::enums::Document::Document(doc) = d.document.unwrap() {
                        let name = doc
                            .attributes
                            .iter()
                            .find_map(|a| match a {
                                tl::enums::DocumentAttribute::Filename(f) => {
                                    Some(f.file_name.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or("Unknown".to_string());
                        let size = doc.size as u64;
                        let mime = doc.mime_type.clone();
                        let ext = std::path::Path::new(&name)
                            .extension()
                            .map(|os| os.to_str().unwrap_or("").to_string());
                        let folder_id = match m.peer_id {
                            tl::enums::Peer::Channel(c) => Some(c.channel_id),
                            tl::enums::Peer::User(u) => Some(u.user_id),
                            tl::enums::Peer::Chat(c) => Some(c.chat_id),
                        };
                        files.push(FileMetadata {
                            id: m.id as i64,
                            folder_id,
                            name,
                            size,
                            mime_type: Some(mime),
                            file_ext: ext,
                            created_at: m.date.to_string(),
                            icon_type: "file".into(),
                        });
                    }
                }
            }
        }
    } else if let tl::enums::messages::Messages::Slice(msgs) = result {
        for msg in msgs.messages {
            if let tl::enums::Message::Message(m) = msg {
                if let Some(tl::enums::MessageMedia::Document(d)) = m.media {
                    if let tl::enums::Document::Document(doc) = d.document.unwrap() {
                        let name = doc
                            .attributes
                            .iter()
                            .find_map(|a| match a {
                                tl::enums::DocumentAttribute::Filename(f) => {
                                    Some(f.file_name.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or("Unknown".to_string());
                        let size = doc.size as u64;
                        let mime = doc.mime_type.clone();
                        let ext = std::path::Path::new(&name)
                            .extension()
                            .map(|os| os.to_str().unwrap_or("").to_string());
                        let folder_id = match m.peer_id {
                            tl::enums::Peer::Channel(c) => Some(c.channel_id),
                            tl::enums::Peer::User(u) => Some(u.user_id),
                            tl::enums::Peer::Chat(c) => Some(c.chat_id),
                        };
                        files.push(FileMetadata {
                            id: m.id as i64,
                            folder_id,
                            name,
                            size,
                            mime_type: Some(mime),
                            file_ext: ext,
                            created_at: m.date.to_string(),
                            icon_type: "file".into(),
                        });
                    }
                }
            }
        }
    }

    Ok(files)
}

#[tauri::command]
pub async fn cmd_scan_folders(
    state: State<'_, TelegramState>,
) -> Result<Vec<FolderMetadata>, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        return Ok(Vec::new());
    }
    let client = client_opt.unwrap();

    let mut folders = Vec::new();
    let mut dialogs = client.iter_dialogs();

    log::info!("Starting Folder Scan...");

    while let Some(dialog) = dialogs.next().await.map_err(|e| e.to_string())? {
        match &dialog.peer {
            Peer::Channel(c) => {
                let id = c.raw.id;
                let name = c.raw.title.clone();
                let access_hash = c.raw.access_hash.unwrap_or(0);

                log::debug!("[SCAN] Processing Channel: '{}' (ID: {})", name, id);

                // Strategy 1: Title
                if name.to_lowercase().contains("[td]") {
                    log::info!(" -> MATCH via Title: {}", name);
                    let display_name = name
                        .replace(" [TD]", "")
                        .replace(" [td]", "")
                        .replace("[TD]", "")
                        .replace("[td]", "")
                        .trim()
                        .to_string();
                    folders.push(FolderMetadata {
                        id,
                        name: display_name,
                        parent_id: None,
                    });
                    continue;
                }

                // Strategy 2: About
                let input_chan = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: c.raw.id,
                    access_hash,
                });

                match client
                    .invoke(&tl::functions::channels::GetFullChannel {
                        channel: input_chan,
                    })
                    .await
                {
                    Ok(tl::enums::messages::ChatFull::Full(f)) => {
                        if let tl::enums::ChatFull::Full(cf) = f.full_chat {
                            if cf.about.contains("[telegram-drive-folder]") {
                                log::info!(" -> MATCH via About: {}", name);
                                folders.push(FolderMetadata {
                                    id,
                                    name: name.clone(),
                                    parent_id: None,
                                });
                            }
                        }
                    }
                    Err(e) => log::warn!(" -> Failed to get full info: {}", e),
                }
            }
            peer => {
                log::debug!("[SCAN] Skipped Peer: {:?}", peer);
            }
        }
    }

    log::info!("Scan complete. Found {} folders.", folders.len());
    Ok(folders)
}
