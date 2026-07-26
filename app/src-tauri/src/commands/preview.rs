use crate::bandwidth::BandwidthManager;
use crate::commands::utils::resolve_peer;
use crate::TelegramState;
use base64::{engine::general_purpose, Engine as _};
use grammers_client::types::Media;
use rand::Rng;
use std::sync::Arc;
use tauri::Manager;
use tauri::State;
use tokio::io::AsyncWriteExt;

/// Supported image file extensions for thumbnails.
/// Shared between Tauri commands and the REST API cache cleanup.
pub const THUMBNAIL_EXTS: &[&str] = &["jpg", "png", "gif", "webp"];

const PREVIEW_CACHE_MAX_FILES: usize = 30;
const PREVIEW_CACHE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

async fn prune_preview_cache(
    cache_dir: std::path::PathBuf,
    preserve_path: Option<std::path::PathBuf>,
) {
    let _ = tokio::task::spawn_blocking(move || {
        let mut read_dir = match std::fs::read_dir(&cache_dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        // First pass: delete any orphaned .part files left behind by
        // interrupted downloads. These are always stale and never preserved.
        for entry in read_dir.by_ref().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname.ends_with(".part") {
                let _ = std::fs::remove_file(&path);
            }
        }

        // Second pass: gather remaining files for size-based pruning.
        // Re-read the directory to get a fresh iterator after the first pass
        // may have modified it.
        let read_dir = match std::fs::read_dir(&cache_dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        let mut files: Vec<(std::path::PathBuf, std::time::SystemTime, u64)> = Vec::new();
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if preserve_path
                .as_ref()
                .is_some_and(|preserve| preserve == &path)
            {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                files.push((path, modified, meta.len()));
            }
        }
        files.sort_by_key(|(_, modified, _)| *modified);
        let mut total_bytes: u64 = files.iter().map(|(_, _, len)| *len).sum();
        while files.len() > PREVIEW_CACHE_MAX_FILES || total_bytes > PREVIEW_CACHE_MAX_TOTAL_BYTES {
            if let Some((path, _, len)) = files.first().cloned() {
                let _ = std::fs::remove_file(&path);
                total_bytes = total_bytes.saturating_sub(len);
                files.remove(0);
            } else {
                break;
            }
        }
    })
    .await;
}

/// Download media to a file using `iter_download` with manual chunk writing.
/// Returns the number of bytes written.
///
/// Unlike `grammers_client::Client::download_media`, this returns an explicit
/// error when the download produces zero bytes (e.g. stale file references or
/// Telegram CDN stream drops).
async fn download_to_file<D: grammers_client::types::Downloadable>(
    client: &grammers_client::Client,
    media: &D,
    part_path: &std::path::Path,
) -> Result<u64, String> {
    let mut file = tokio::fs::File::create(part_path)
        .await
        .map_err(|e| format!("Failed to create .part file: {}", e))?;

    let mut download_iter = client.iter_download(media);
    download_iter = download_iter.chunk_size(65536);
    let mut written: u64 = 0;

    loop {
        match download_iter.next().await {
            Ok(Some(chunk)) => {
                file.write_all(&chunk)
                    .await
                    .map_err(|e| format!("Write error: {}", e))?;
                written += chunk.len() as u64;
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tokio::fs::remove_file(part_path).await;
                return Err(format!("Download error: {}", e));
            }
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Flush error: {}", e))?;
    drop(file);

    if written == 0 {
        let _ = tokio::fs::remove_file(part_path).await;
        return Err(
            "Download produced zero bytes (stale file reference or stream drop)".to_string(),
        );
    }

    Ok(written)
}

#[tauri::command]
pub async fn cmd_get_preview(
    message_id: i32,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, Arc<BandwidthManager>>,
) -> Result<String, String> {
    let cache_dir = app_handle
        .path()
        .app_cache_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("previews");
    if tokio::fs::metadata(&cache_dir).await.is_err() {
        let _ = tokio::fs::create_dir_all(&cache_dir).await;
    }
    log::info!("Using preview cache dir: {:?}", cache_dir);
    log::info!("Preview Request: msg_id={}", message_id);
    let client_opt = { state.client.lock().await.clone() };
    #[cfg(debug_assertions)]
    if client_opt.is_none() {
        return Ok("".to_string());
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;
    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    let target_message = messages.into_iter().flatten().next();

    if let Some(msg) = target_message {
        if let Some(media) = msg.media() {
            let ext = match &media {
                Media::Document(d) => {
                    let mut e = std::path::Path::new(d.name())
                        .extension()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if e.is_empty() {
                        if let Some(mime) = d.mime_type() {
                            e = match mime {
                                "image/jpeg" => "jpg".to_string(),
                                "image/png" => "png".to_string(),
                                "application/pdf" => "pdf".to_string(),
                                "video/mp4" => "mp4".to_string(),
                                _ => "bin".to_string(),
                            };
                        } else {
                            e = "bin".to_string();
                        }
                    }
                    e
                }
                Media::Photo(_) => "jpg".to_string(),
                _ => "bin".to_string(),
            };
            let folder_key = folder_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "home".to_string());
            let save_path = cache_dir.join(format!("{}_{}.{}", folder_key, message_id, ext));
            let save_path_str = save_path.to_string_lossy().to_string();

            // Prune the cache here, explicitly preserving the active file being previewed
            prune_preview_cache(cache_dir.clone(), Some(save_path.clone())).await;

            let cached_meta = tokio::fs::metadata(&save_path).await.ok();
            let file_ready = if cached_meta.as_ref().is_some_and(|meta| meta.len() > 0) {
                log::info!("File ({}) exists in cache.", message_id);
                true
            } else {
                if cached_meta.is_some() {
                    log::warn!(
                        "Preview cache file was empty; redownloading: {}",
                        save_path_str
                    );
                    let _ = tokio::fs::remove_file(&save_path).await;
                }
                let size = match &media {
                    Media::Document(d) => d.size() as u64,
                    Media::Photo(_) => 1024 * 1024,
                    _ => 0,
                };
                log::info!("Downloading preview... Size: {}", size);
                if let Err(e) = bw_state.try_reserve_down(size) {
                    log::warn!("Bandwidth limit hit for preview: {}", e);
                    false
                } else {
                    // Download to a temporary .part file to avoid race conditions
                    // when concurrent requests try to download the same file.
                    // After successful download, atomically rename to the final path.
                    //
                    // Use a random u64 suffix so concurrent requests for the
                    // same file write to separate .part files — preventing the inter-request
                    // delete/write race that previously produced empty files.
                    let unique_id = rand::rng().random::<u64>();
                    let part_path = save_path.with_extension(format!("{}_{}.part", ext, unique_id));
                    // part_path_str is no longer needed — download_to_file takes &Path directly

                    let mut download_ok = false;

                    // Early-exit: another concurrent request may have already completed
                    // the download and renamed its .part file to the final path.
                    if tokio::fs::metadata(&save_path)
                        .await
                        .is_ok_and(|m| m.len() > 0)
                    {
                        log::info!(
                            "Preview already downloaded by concurrent request (final file exists)"
                        );
                        bw_state.release_down(size);
                        download_ok = true;
                    }

                    // Attempt 1: download with original media (may have stale file reference)
                    if !download_ok {
                        let _ = tokio::fs::remove_file(&part_path).await;
                        match download_to_file(&client, &media, &part_path).await {
                            Ok(written) => {
                                log::info!("Preview download complete: {} bytes.", written);
                                match tokio::fs::rename(&part_path, &save_path).await {
                                    Ok(_) => {
                                        download_ok = true;
                                        prune_preview_cache(
                                            cache_dir.clone(),
                                            Some(save_path.clone()),
                                        )
                                        .await;
                                    }
                                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                        if tokio::fs::metadata(&save_path)
                                            .await
                                            .is_ok_and(|m| m.len() > 0)
                                        {
                                            log::info!(
                                                "Preview already downloaded by concurrent request"
                                            );
                                            download_ok = true;
                                        }
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "Failed to rename part file to final path: {}",
                                            e
                                        );
                                        let _ = tokio::fs::remove_file(&part_path).await;
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Preview Download Error (attempt 1/2): {}", e);
                            }
                        }
                    } // end attempt 1

                    // Attempt 2: re-fetch the message to get fresh file references, then retry
                    if !download_ok {
                        // Brief backoff before re-fetching
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                        // Re-fetch the message to obtain a Media object with a fresh file reference.
                        // Telegram file references expire; iter_download returns 0 bytes (caught
                        // by download_to_file) when the reference is stale.
                        if let Ok(fresh_messages) =
                            client.get_messages_by_id(&peer, &[message_id]).await
                        {
                            if let Some(fresh_msg) = fresh_messages.into_iter().flatten().next() {
                                if let Some(fresh_media) = fresh_msg.media() {
                                    let _ = tokio::fs::remove_file(&part_path).await;
                                    match download_to_file(&client, &fresh_media, &part_path).await
                                    {
                                        Ok(written) => {
                                            log::info!("Preview download complete after re-fetch: {} bytes.", written);
                                            match tokio::fs::rename(&part_path, &save_path).await {
                                                Ok(_) => {
                                                    download_ok = true;
                                                    prune_preview_cache(
                                                        cache_dir.clone(),
                                                        Some(save_path.clone()),
                                                    )
                                                    .await;
                                                }
                                                Err(e)
                                                    if e.kind() == std::io::ErrorKind::NotFound =>
                                                {
                                                    if tokio::fs::metadata(&save_path)
                                                        .await
                                                        .is_ok_and(|m| m.len() > 0)
                                                    {
                                                        log::info!("Preview already downloaded by concurrent request");
                                                        download_ok = true;
                                                    }
                                                }
                                                Err(e) => {
                                                    log::error!("Failed to rename part file to final path: {}", e);
                                                    let _ =
                                                        tokio::fs::remove_file(&part_path).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            log::error!(
                                                "Preview Download Error (attempt 2/2): {}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !download_ok {
                        bw_state.release_down(size);
                    }
                    download_ok
                }
            };
            if file_ready {
                let lower_ext = ext.to_lowercase();
                if ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"].contains(&lower_ext.as_str())
                {
                    log::info!("Converting file to Base64...");
                    match tokio::fs::read(&save_path).await {
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
                        }
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
pub async fn cmd_clean_preview_cache(app_handle: tauri::AppHandle) -> Result<(), String> {
    let cache_dir = app_handle
        .path()
        .app_cache_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("previews");

    let _ = tokio::task::spawn_blocking(move || {
        if cache_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(cache_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
    })
    .await;
    Ok(())
}

#[tauri::command]
pub async fn cmd_clean_cache(app_handle: tauri::AppHandle) -> Result<(), String> {
    let cache_dir = app_handle
        .path()
        .app_cache_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("previews");
    let thumb_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("thumbnails");

    let _ = tokio::task::spawn_blocking(move || {
        if cache_dir.exists() {
            let _ = std::fs::remove_dir_all(cache_dir);
        }
        if thumb_dir.exists() {
            let _ = std::fs::remove_dir_all(thumb_dir);
        }
    })
    .await;
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
    let cache_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("thumbnails");
    if tokio::fs::metadata(&cache_dir).await.is_err() {
        let _ = tokio::fs::create_dir_all(&cache_dir).await;
    }

    let folder_key = folder_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "home".to_string());

    // Check for any cached thumbnail for this message by checking predicted paths
    let supported_exts = THUMBNAIL_EXTS;
    for ext in supported_exts {
        let path = cache_dir.join(format!("{}_{}.{}", folder_key, message_id, ext));
        if tokio::fs::metadata(&path).await.is_ok() {
            if let Ok(bytes) = tokio::fs::read(&path).await {
                let mime = match *ext {
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

    // No cache, need to fetch from Telegram
    let client_opt = { state.client.lock().await.clone() };
    #[cfg(debug_assertions)]
    if client_opt.is_none() {
        return Ok("".to_string());
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;
    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    if let Some(m) = messages.into_iter().flatten().next() {
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
                }
                _ => return Ok("".to_string()),
            };

            if is_image {
                // Get photo thumbnail (largest available for best quality)
                let save_path = cache_dir.join(format!("{}_{}.{}", folder_key, message_id, ext));

                let thumbs = match &media {
                    Media::Photo(p) => p.thumbs(),
                    Media::Document(d) => d.thumbs(),
                    _ => vec![],
                };

                // Download to a temporary .part file to avoid race conditions
                // with concurrent thumbnail requests for the same file.
                //
                // Use a random u64 suffix so concurrent requests for the
                // same file write to separate .part files — preventing the inter-request
                // delete/write race that previously produced empty files.
                let unique_id = rand::rng().random::<u64>();
                let part_path = save_path.with_extension(format!("{}_{}.part", ext, unique_id));

                let mut download_ok = false;

                // Early-exit: another concurrent request may have already completed
                // the download and renamed its .part file to the final path.
                if tokio::fs::metadata(&save_path)
                    .await
                    .is_ok_and(|m| m.len() > 0)
                {
                    download_ok = true;
                }

                // Attempt 1: download with original media/thumbs (may have stale file reference)
                if !download_ok {
                    let _ = tokio::fs::remove_file(&part_path).await;
                    let ok = if let Some(thumb) = thumbs
                        .iter()
                        .filter(|t| t.size() > 0)
                        .max_by_key(|t| t.size())
                    {
                        download_to_file(&client, thumb, &part_path).await.is_ok()
                    } else {
                        download_to_file(&client, &media, &part_path).await.is_ok()
                    };
                    if ok {
                        download_ok = true;
                    }
                }

                // Attempt 2: re-fetch the message to get fresh file references, then retry
                if !download_ok {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if let Ok(fresh_messages) =
                        client.get_messages_by_id(&peer, &[message_id]).await
                    {
                        if let Some(fresh_msg) = fresh_messages.into_iter().flatten().next() {
                            if let Some(fresh_media) = fresh_msg.media() {
                                let fresh_thumbs = match &fresh_media {
                                    Media::Photo(p) => p.thumbs(),
                                    Media::Document(d) => d.thumbs(),
                                    _ => vec![],
                                };
                                let _ = tokio::fs::remove_file(&part_path).await;
                                let ok = if let Some(fresh_thumb) = fresh_thumbs
                                    .iter()
                                    .filter(|t| t.size() > 0)
                                    .max_by_key(|t| t.size())
                                {
                                    download_to_file(&client, fresh_thumb, &part_path)
                                        .await
                                        .is_ok()
                                } else {
                                    download_to_file(&client, &fresh_media, &part_path)
                                        .await
                                        .is_ok()
                                };
                                if ok {
                                    download_ok = true;
                                }
                            }
                        }
                    }
                }

                if download_ok {
                    // Atomically rename part file to final path
                    match tokio::fs::rename(&part_path, &save_path).await {
                        Ok(_) => {
                            if let Ok(bytes) = tokio::fs::read(&save_path).await {
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
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            // Another concurrent request already renamed our part file.
                            if let Ok(bytes) = tokio::fs::read(&save_path).await {
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
                        Err(_) => {
                            let _ = tokio::fs::remove_file(&part_path).await;
                        }
                    }
                }
            }
        }
    }

    Ok("".to_string())
}

/// Delete stale preview cache entries for a specific message in a specific folder.
/// Preview cache files are named `{folder_key}_{message_id}.{ext}`.
/// This removes all extensions for the given folder+message_id pair.
#[tauri::command]
pub async fn cmd_delete_preview_for_message(
    message_id: i32,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let cache_dir = app_handle
        .path()
        .app_cache_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("previews");

    let folder_key = folder_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "home".to_string());

    let prefix = format!("{}_{}.", folder_key, message_id);

    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if fname.starts_with(&prefix) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    })
    .await;
    Ok(())
}

#[tauri::command]
pub async fn cmd_delete_image_thumbnail(
    message_id: i32,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let cache_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?
        .join("thumbnails");

    let folder_key = folder_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "home".to_string());

    let _ = tokio::task::spawn_blocking(move || {
        let supported_exts = THUMBNAIL_EXTS;
        for ext in supported_exts {
            let path = cache_dir.join(format!("{}_{}.{}", folder_key, message_id, ext));
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
    })
    .await;
    Ok(())
}
