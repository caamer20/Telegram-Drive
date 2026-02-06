use tauri::State;
use tauri::Manager;
use grammers_client::types::Media;
use base64::{Engine as _, engine::general_purpose};
use crate::TelegramState;
use crate::bandwidth::BandwidthManager;
use crate::commands::utils::resolve_peer;

#[tauri::command]
pub async fn cmd_get_preview(
    message_id: i32,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    
    let cache_dir = app_handle.path().app_data_dir().map_err(|e: tauri::Error| e.to_string())?.join("previews");
    if !cache_dir.exists() { let _ = std::fs::create_dir_all(&cache_dir); }
    log::info!("Using preview cache dir: {:?}", cache_dir);
    log::info!("Preview Request: msg_id={}", message_id);

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { return Ok("".to_string()); }
    let client = client_opt.unwrap();
    
    let peer = resolve_peer(&client, folder_id).await?;
    let mut msgs = client.iter_messages(&peer);
    let mut target_message = None;
    while let Some(m) = msgs.next().await.map_err(|e| e.to_string())? { 
        if m.id() == message_id { target_message = Some(m); break; } 
    }
    
    if let Some(msg) = target_message {
        if let Some(media) = msg.media() {
             let ext = match &media {
                 Media::Document(d) => {
                     let mut e = std::path::Path::new(d.name()).extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                     if e.is_empty() {
                         if let Some(mime) = d.mime_type() {
                              e = match mime {
                                  "image/jpeg" => "jpg".to_string(),
                                  "image/png" => "png".to_string(),
                                  "video/mp4" => "mp4".to_string(),
                                  _ => "bin".to_string(),
                              };
                         } else {
                             e = "bin".to_string();
                         }
                     }
                     e
                 },
                 Media::Photo(_) => "jpg".to_string(),
                 _ => "bin".to_string(),
             };
             
             let save_path = cache_dir.join(format!("{}.{}", message_id, ext));
             let save_path_str = save_path.to_string_lossy().to_string();
             
             let file_ready = if save_path.exists() {
                 log::info!("File ({}) exists in cache.", message_id);
                 true
             } else {
                 let size = match &media {
                    Media::Document(d) => d.size() as u64,
                    Media::Photo(_) => 1024 * 1024,
                    _ => 0,
                };
                
                log::info!("Downloading preview... Size: {}", size);
                if let Err(e) = bw_state.can_transfer(size) {
                    log::warn!("Bandwidth limit hit for preview: {}", e);
                    false
                } else {
                    match client.download_media(&media, &save_path_str).await {
                        Ok(_) => {
                            log::info!("Preview download complete.");
                            bw_state.add_down(size);
                            true
                        },
                        Err(e) => {
                            log::error!("Preview Download Error: {}", e);
                            false
                        }
                    }
                }
             };

             if file_ready {
                 let lower_ext = ext.to_lowercase();
                 if ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"].contains(&lower_ext.as_str()) {
                     log::info!("Converting image to Base64...");
                     match std::fs::read(&save_path) {
                         Ok(bytes) => {
                             let b64 = general_purpose::STANDARD.encode(&bytes);
                             let mime = match lower_ext.as_str() {
                                 "png" => "image/png",
                                 "gif" => "image/gif",
                                 "webp" => "image/webp",
                                 "bmp" => "image/bmp",
                                 "svg" => "image/svg+xml",
                                 _ => "image/jpeg",
                             };
                             return Ok(format!("data:{};base64,{}", mime, b64));
                         },
                         Err(e) => {
                             log::error!("Failed to read file for base64: {}", e);
                             return Ok(save_path_str);
                         }
                     }
                 }
                 log::info!("Returning path preview: {}", save_path_str);
                 return Ok(save_path_str);
             }
        }
    }

    Err("File not found or failed to download".to_string())
}

#[tauri::command]
pub async fn cmd_clean_cache(
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let cache_dir = app_handle.path().app_cache_dir().map_err(|e| e.to_string())?.join("previews");
    if cache_dir.exists() {
         let _ = std::fs::remove_dir_all(cache_dir);
    }
    Ok(())
}

/// Get a small thumbnail for inline display in file cards.
/// Returns base64 data URL for images, empty string for non-image files.
/// Uses same cache as cmd_get_preview for consistency.
#[tauri::command]
pub async fn cmd_get_thumbnail(
    message_id: i32,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
) -> Result<String, String> {
    // Check if thumbnail already in cache
    let cache_dir = app_handle.path().app_data_dir().map_err(|e: tauri::Error| e.to_string())?.join("thumbnails");
    if !cache_dir.exists() { let _ = std::fs::create_dir_all(&cache_dir); }
    
    // Check for any cached thumbnail for this message
    
    // Look for existing cached file
    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{}.", message_id)) {
                // Found cached thumbnail, return as base64
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    let ext = name.rsplit('.').next().unwrap_or("jpg");
                    let mime = match ext {
                        "png" => "image/png",
                        "gif" => "image/gif", 
                        "webp" => "image/webp",
                        _ => "image/jpeg",
                    };
                    let b64 = general_purpose::STANDARD.encode(&bytes);
                    return Ok(format!("data:{};base64,{}", mime, b64));
                }
            }
        }
    }
    
    // No cache, need to fetch from Telegram
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { return Ok("".to_string()); }
    let client = client_opt.unwrap();
    
    let peer = resolve_peer(&client, folder_id).await?;
    let mut msgs = client.iter_messages(&peer);
    
    while let Some(m) = msgs.next().await.map_err(|e| e.to_string())? {
        if m.id() == message_id {
            if let Some(media) = m.media() {
                // Only get thumbnails for photos and documents with photo thumbnails
                let (is_image, ext) = match &media {
                    Media::Photo(_) => (true, "jpg".to_string()),
                    Media::Document(d) => {
                        let mime = d.mime_type().unwrap_or("");
                        if mime.starts_with("image/") {
                            let e = match mime {
                                "image/png" => "png",
                                "image/gif" => "gif",
                                "image/webp" => "webp",
                                _ => "jpg",
                            };
                            (true, e.to_string())
                        } else {
                            // Not an image, return empty - FileCard will show icon
                            return Ok("".to_string());
                        }
                    },
                    _ => return Ok("".to_string()),
                };
                
                if is_image {
                    // Get photo thumbnail (smallest size for speed)
                    let save_path = cache_dir.join(format!("{}.{}", message_id, ext));
                    let save_path_str = save_path.to_string_lossy().to_string();
                    
                    // Download the thumbnail/photo
                    if client.download_media(&media, &save_path_str).await.is_ok() {
                        if let Ok(bytes) = std::fs::read(&save_path) {
                            let mime = match ext.as_str() {
                                "png" => "image/png",
                                "gif" => "image/gif",
                                "webp" => "image/webp",
                                _ => "image/jpeg",
                            };
                            let b64 = general_purpose::STANDARD.encode(&bytes);
                            return Ok(format!("data:{};base64,{}", mime, b64));
                        }
                    }
                }
            }
            break;
        }
    }
    
    Ok("".to_string())
}
