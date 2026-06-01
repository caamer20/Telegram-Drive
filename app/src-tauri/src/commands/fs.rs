use tauri::{State, Emitter};
use grammers_client::types::{Media, Peer};
use grammers_client::InputMessage;
use grammers_tl_types as tl;
use crate::TelegramState;
use crate::models::{FolderMetadata, FileMetadata};
use crate::bandwidth::BandwidthManager;
use crate::commands::utils::{resolve_peer, map_error};
use crate::vpn_optimizer::{NetworkConfig, backoff_ms};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::Mutex;
use tokio::sync::oneshot;

static UPLOAD_CANCELLATIONS: OnceLock<Mutex<HashMap<String, oneshot::Sender<()>>>> = OnceLock::new();

fn get_upload_cancellations() -> &'static Mutex<HashMap<String, oneshot::Sender<()>>> {
    UPLOAD_CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn url_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    result.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

pub fn clean_android_path(raw_path: &str) -> String {
    let decoded = url_decode(raw_path);
    log::info!("URL Decoded path: {}", decoded);
    let mut cleaned = decoded;
    if cleaned.starts_with("raw%3/") {
        cleaned = cleaned.replace("raw%3/", "/");
    }
    if cleaned.starts_with("raw://") {
        cleaned = cleaned.replace("raw://", "/");
    } else if cleaned.starts_with("file://") {
        cleaned = cleaned.replace("file://", "");
    } else if cleaned.starts_with("raw:") {
        cleaned = cleaned.replace("raw:", "");
    }
    if !cleaned.starts_with("content://") {
        cleaned = cleaned.replace("//", "/");
    }
    log::info!("Cleaned absolute path: {}", cleaned);
    cleaned
}

#[cfg(target_os = "android")]
pub fn copy_to_android_cache(raw_path: &str) -> Result<String, String> {
    log::info!("JNI copy_to_android_cache started for path: {}", raw_path);
    let ctx_obj = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx_obj.vm().cast()) }
        .map_err(|e| format!("Failed to get JavaVM: {}", e))?;
    let mut env = vm.attach_current_thread()
        .map_err(|e| format!("Failed to attach thread: {}", e))?;

    let ctx = unsafe { jni::objects::JObject::from_raw(ctx_obj.context().cast()) };

    // 1. URL Decode & Clean Path in Rust
    let cleaned = clean_android_path(raw_path);
    log::info!("JNI Cleaned path: {}", cleaned);

    // 2. Check if the main thread already pre-cached this URI.
    //    This is the primary path for content:// URIs — the background thread
    //    MUST NOT call ContentResolver.openInputStream() directly.
    if cleaned.starts_with("content://") || cleaned.starts_with("msf:") || cleaned.starts_with("/msf:") || cleaned.contains("msf%") {
        // Retrieve globally cached MainActivity class reference
        let cached_ref = crate::jni_cache::get_main_activity_class()
            .ok_or_else(|| "JNI: MainActivity class reference was NOT cached globally!".to_string())?;
        let main_class: jni::objects::JClass = unsafe { std::mem::transmute_copy(cached_ref.as_obj()) };

        // Step A: Check if onActivityResult pre-cached this URI.
        // Validate the cached file is non-empty before accepting it.
        {
            let j_uri_str = env.new_string(raw_path)
                .map_err(|e| format!("Failed to create URI string: {}", e))?;
            let cached_result = env.call_static_method(
                &main_class,
                "getCachedPath",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[jni::objects::JValue::from(&j_uri_str)],
            );
            if let Ok(cached_val) = cached_result {
                if let Ok(cached_jobj) = cached_val.l() {
                    if !cached_jobj.is_null() {
                        let cached_jstr: jni::objects::JString = cached_jobj.into();
                        if let Ok(cached_path) = env.get_string(&cached_jstr).map(String::from) {
                            if !cached_path.is_empty() {
                                // Validate the cached file actually exists and has content
                                match std::fs::metadata(&cached_path) {
                                    Ok(meta) if meta.len() > 0 => {
                                        log::info!("JNI: Found valid pre-cached path for URI: {} ({} bytes)", cached_path, meta.len());
                                        return Ok(cached_path);
                                    }
                                    Ok(meta) => {
                                        log::warn!("JNI: Pre-cache wrote invalid file: {} ({} bytes). Falling back to InputStream.", cached_path, meta.len());
                                    }
                                    Err(e) => {
                                        log::warn!("JNI: Pre-cached file missing: {} ({}). Falling back to InputStream.", cached_path, e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let _ = env.exception_clear();
        }

        // Step B: HARD BOUNDARY fallback — call getLocalFileFromUri() which posts the
        // ContentResolver.openInputStream work to the MAIN thread and blocks until done.
        // Background thread NEVER touches ContentResolver directly.
        // Validate the returned file is non-empty before accepting it.
        {
            log::info!("JNI: Pre-cache miss or invalid. Calling getLocalFileFromUri on main thread: {}", raw_path);
            let j_uri_fallback = env.new_string(raw_path)
                .map_err(|e| format!("Failed to create URI string for fallback: {}", e))?;
            let fallback_result = env.call_static_method(
                &main_class,
                "getLocalFileFromUri",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[jni::objects::JValue::from(&j_uri_fallback)],
            );
            if let Ok(fallback_val) = fallback_result {
                if let Ok(fallback_jobj) = fallback_val.l() {
                    if !fallback_jobj.is_null() {
                        let fallback_jstr: jni::objects::JString = fallback_jobj.into();
                        if let Ok(fallback_path) = env.get_string(&fallback_jstr).map(String::from) {
                            if !fallback_path.is_empty() {
                                // Validate the fallback file actually exists and has content
                                match std::fs::metadata(&fallback_path) {
                                    Ok(meta) if meta.len() > 0 => {
                                        log::info!("JNI: getLocalFileFromUri succeeded with valid file: {} ({} bytes)", fallback_path, meta.len());
                                        return Ok(fallback_path);
                                    }
                                    Ok(meta) => {
                                        log::warn!("JNI: getLocalFileFromUri wrote invalid file: {} ({} bytes). Falling back to InputStream.", fallback_path, meta.len());
                                    }
                                    Err(e) => {
                                        log::warn!("JNI: getLocalFileFromUri file missing: {} ({}). Falling back to InputStream.", fallback_path, e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let _ = env.exception_clear();
        }

        // Step C: All pre-cache paths failed or returned empty files.
        // Fall through to the raw InputStream approach below (step 3+).
        log::info!("JNI: Pre-cache and getLocalFileFromUri both failed or returned empty. Falling through to raw InputStream copy.");
    }

    // 3. Parse URI (fallback for non-content:// paths or pre-cache misses)
    let uri_class = env.find_class("android/net/Uri")
        .map_err(|e| format!("Failed to find android/net/Uri: {}", e))?;
    let j_cleaned = env.new_string(&cleaned)
        .map_err(|e| format!("Failed to create Java string: {}", e))?;
    let uri_val = env.call_static_method(
        &uri_class,
        "parse",
        "(Ljava/lang/String;)Landroid/net/Uri;",
        &[jni::objects::JValue::from(&j_cleaned)],
    ).map_err(|e| format!("Failed to parse URI: {}", e))?;
    
    let uri = uri_val.l()
        .map_err(|e| format!("URI result is not an object: {}", e))?;

    if uri.is_null() {
        return Err("Parsed URI is null".to_string());
    }

    // 4. Get ContentResolver
    let content_resolver = env.call_method(
        &ctx,
        "getContentResolver",
        "()Landroid/content/ContentResolver;",
        &[],
    ).map_err(|e| format!("Failed to get ContentResolver: {}", e))?
    .l()
    .map_err(|e| format!("ContentResolver is not an object: {}", e))?;

    // 5. Take Persistable URI Permission (best-effort, won't throw if it fails)
    if cleaned.starts_with("content://") {
        let intent_class = env.find_class("android/content/Intent")
            .map_err(|e| format!("Failed to find android/content/Intent: {}", e))?;
        if let Ok(flag_val) = env.get_static_field(
            &intent_class,
            "FLAG_GRANT_READ_URI_PERMISSION",
            "I",
        ) {
            if let Ok(flag_grant_read) = flag_val.i() {
                let res = env.call_method(
                    &content_resolver,
                    "takePersistableUriPermission",
                    "(Landroid/net/Uri;I)V",
                    &[jni::objects::JValue::from(&uri), jni::objects::JValue::from(flag_grant_read)],
                );
                if res.is_err() {
                    log::warn!("JNI: takePersistableUriPermission failed; clearing exception.");
                    let _ = env.exception_clear();
                }
            }
        }
    }

    // 6. Open Input Stream
    let input_stream = env.call_method(
        &content_resolver,
        "openInputStream",
        "(Landroid/net/Uri;)Ljava/io/InputStream;",
        &[jni::objects::JValue::from(&uri)],
    ).map_err(|e| format!("Failed to openInputStream: {}", e))?
    .l()
    .map_err(|e| format!("InputStream is not an object: {}", e))?;

    if input_stream.is_null() {
        return Err("InputStream is null".to_string());
    }

    // 7. Get Cache Dir
    let cache_dir_file = env.call_method(
        &ctx,
        "getCacheDir",
        "()Ljava/io/File;",
        &[],
    ).map_err(|e| format!("Failed to getCacheDir: {}", e))?
    .l()
    .map_err(|e| format!("Cache dir is not an object: {}", e))?;

    let cache_path_jstr = env.call_method(
        &cache_dir_file,
        "getAbsolutePath",
        "()Ljava/lang/String;",
        &[],
    ).map_err(|e| format!("Failed to get absolute path of cache: {}", e))?
    .l()
    .map_err(|e| format!("Cache path is not String: {}", e))?;

    let cache_path_jstring: jni::objects::JString = cache_path_jstr.into();
    let cache_path: String = env.get_string(&cache_path_jstring)
        .map_err(|e| format!("Failed to convert cache path to Rust: {}", e))?
        .into();

    // 8. Get display name or file name
    let mut file_name = "temp_upload".to_string();
    if cleaned.starts_with("content://") {
        let cursor_val = env.call_method(
            &content_resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[
                jni::objects::JValue::from(&uri),
                jni::objects::JValue::from(&jni::objects::JObject::null()),
                jni::objects::JValue::from(&jni::objects::JObject::null()),
                jni::objects::JValue::from(&jni::objects::JObject::null()),
                jni::objects::JValue::from(&jni::objects::JObject::null()),
            ],
        );
        
        if let Ok(c_res) = cursor_val {
            if let Ok(cursor_obj) = c_res.l() {
                if !cursor_obj.is_null() {
                    let j_display_name = env.new_string("_display_name")
                        .map_err(|e| format!("Failed to create display name string: {}", e))?;

                    let col_index = env.call_method(
                        &cursor_obj,
                        "getColumnIndex",
                        "(Ljava/lang/String;)I",
                        &[jni::objects::JValue::from(&j_display_name)],
                    ).ok()
                    .and_then(|r| r.i().ok())
                    .unwrap_or(-1);

                    let has_first = env.call_method(
                        &cursor_obj,
                        "moveToFirst",
                        "()Z",
                        &[],
                    ).ok()
                    .and_then(|r| r.z().ok())
                    .unwrap_or(false);

                    if col_index != -1 && has_first {
                        if let Ok(name_val) = env.call_method(
                            &cursor_obj,
                            "getString",
                            "(I)Ljava/lang/String;",
                            &[jni::objects::JValue::from(col_index)],
                        ) {
                            if let Ok(name_jstr_obj) = name_val.l() {
                                if !name_jstr_obj.is_null() {
                                     let name_jstring: jni::objects::JString = name_jstr_obj.into();
                                     if let Ok(name_rust) = env.get_string(&name_jstring).map(String::from) {
                                         file_name = name_rust;
                                     }
                                }
                            }
                        }
                    }
                    let _ = env.call_method(&cursor_obj, "close", "()V", &[]);
                }
            }
        }
    } else {
        if let Some(name) = std::path::Path::new(&cleaned).file_name() {
            file_name = name.to_string_lossy().to_string();
        }
    }

    // 9. Create cache file destination
    let cache_file_name = format!("upload_{}_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(), file_name);
    let dest_path = std::path::Path::new(&cache_path).join(cache_file_name);
    let dest_path_str = dest_path.to_string_lossy().to_string();

    // 10. Read InputStream bytes and write to local file in Rust (with retry).
    //    Uses a helper to avoid duplicating the read loop between first attempt and retry.

    // Helper: read all bytes from an InputStream JObject and write them to dest_path.
    // Returns Ok(total_bytes_read) on success, or Err(message) on failure.
    // Closes the stream when done (both on success and on read failure).
    let read_stream_to_file = |
        env: &mut jni::JNIEnv,
        stream: &jni::objects::JObject,
        dest_path: &str,
    | -> Result<u64, String> {
        let mut out_file = std::fs::File::create(dest_path)
            .map_err(|e| format!("Failed to create destination cache file: {}", e))?;

        const BUFFER_SIZE: i32 = 65536;
        let byte_array = env.new_byte_array(BUFFER_SIZE)
            .map_err(|e| format!("Failed to create Java byte array: {}", e))?;

        let mut total_read: u64 = 0;
        loop {
            let bytes_read = match env.call_method(
                stream,
                "read",
                "([B)I",
                &[jni::objects::JValue::from(&byte_array)],
            ) {
                Ok(val) => match val.i() {
                    Ok(n) => n,
                    Err(e) => return Err(format!("read result error: {}", e)),
                },
                Err(e) => {
                    let _ = env.exception_clear();
                    return Err(format!("Failed to read from InputStream: {}", e));
                }
            };

            if bytes_read <= 0 {
                break;
            }

            let java_bytes = env.convert_byte_array(&byte_array)
                .map_err(|e| format!("Failed to convert Java byte array: {}", e))?;

            use std::io::Write;
            out_file.write_all(&java_bytes[..bytes_read as usize])
                .map_err(|e| format!("Failed to write bytes to cache file: {}", e))?;
            total_read += bytes_read as u64;
        }

        let _ = env.call_method(stream, "close", "()V", &[]);

        // Validate the written file is non-empty
        match std::fs::metadata(dest_path) {
            Ok(meta) if meta.len() > 0 => Ok(total_read),
            Ok(meta) => Err(format!("File written is {} bytes (read {} bytes from stream)", meta.len(), total_read)),
            Err(e) => Err(format!("Result file missing: {}", e)),
        }
    };

    // First attempt: use the already-opened input_stream
    match read_stream_to_file(&mut env, &input_stream, &dest_path_str) {
        Ok(total_read) => {
            log::info!(
                "JNI InputStream first attempt succeeded: {} ({} bytes)",
                dest_path_str, total_read
            );
            return Ok(dest_path_str);
        }
        Err(err) => {
            log::warn!("JNI InputStream first attempt failed: {}. Retrying...", err);
        }
    }

    // Retry: re-open the InputStream and try again
    log::info!("JNI InputStream retry attempt for: {}", dest_path_str);
    let retry_result = env.call_method(
        &content_resolver,
        "openInputStream",
        "(Landroid/net/Uri;)Ljava/io/InputStream;",
        &[jni::objects::JValue::from(&uri)],
    );
    let retry_stream = match retry_result {
        Ok(val) => match val.l() {
            Ok(obj) if !obj.is_null() => obj,
            _ => return Err("Retry: Failed to open InputStream".to_string()),
        },
        Err(e) => {
            let _ = env.exception_clear();
            return Err(format!("Retry: Failed to open InputStream: {}", e));
        }
    };

    match read_stream_to_file(&mut env, &retry_stream, &dest_path_str) {
        Ok(total_read) => {
            log::info!(
                "JNI InputStream retry succeeded: {} ({} bytes)",
                dest_path_str, total_read
            );
            Ok(dest_path_str)
        }
        Err(err) => {
            Err(format!("InputStream copy failed after retry: {}", err))
        }
    }
}



#[cfg(not(target_os = "android"))]
pub fn copy_to_android_cache(_raw_path: &str) -> Result<String, String> {
    Err("Not supported on this platform".to_string())
}

#[tauri::command]
pub async fn cmd_create_folder(
    name: String,
    state: State<'_, TelegramState>,
) -> Result<FolderMetadata, String> {
    let client_opt = {
        state.client.lock().await.clone()
    };
    
    // --- MOCK ---
    if client_opt.is_none() {
        let mock_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        log::info!("[MOCK] Created folder '{}' with ID {}", name, mock_id);
        return Ok(FolderMetadata {
            id: mock_id,
            name,
            parent_id: None,
            username: None,
            is_public: false,
        });
    }
    // -----------
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;
    log::info!("Creating Telegram Channel: {}", name);
    
    let result = client.invoke(&tl::functions::channels::CreateChannel {
        broadcast: true,
        megagroup: false,
        title: format!("{} [TD]", name),
        about: "Telegram Drive Storage Folder\n[telegram-drive-folder]".to_string(),
        geo_point: None,
        address: None,
        for_import: false,
        forum: false,
        ttl_period: None, // Initial creation TTL
    }).await.map_err(map_error)?;
    
    let (chat_id, access_hash) = match &result {
        tl::enums::Updates::Updates(u) => {
             let chat = u.chats.first().ok_or("No chat in updates")?;
             match chat {
                 tl::enums::Chat::Channel(c) => {
                     // Cache the newly created peer immediately so that invite link generation
                     // and other commands don't experience a peer cache miss on warm start.
                     let channel_obj = grammers_client::types::Channel { raw: c.clone() };
                     state.peer_cache.write().await.insert(c.id, grammers_client::types::Peer::Channel(channel_obj));
                     (c.id, c.access_hash.unwrap_or(0))
                 }
                 _ => return Err("Created chat is not a channel".to_string()),
             }
        },
        _ => return Err("Unexpected response (not Updates::Updates)".to_string()), 
    };

    // Explicitly Disable TTL
    let _input_channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
         channel_id: chat_id,
         access_hash,
    });

    let _ = client.invoke(&tl::functions::messages::SetHistoryTtl {
        peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel { channel_id: chat_id, access_hash }),
        period: 0, 
    }).await;

    Ok(FolderMetadata {
        id: chat_id,
        name,
        parent_id: None,
        username: None,
        is_public: false,
    })
}

#[tauri::command]
pub async fn cmd_delete_folder(
    folder_id: i64,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let client_opt = {
        state.client.lock().await.clone()
    };
    
    if client_opt.is_none() {
        log::info!("[MOCK] Deleted folder ID {}", folder_id);
        return Ok(true);
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;
    log::info!("Deleting folder/channel: {}", folder_id);

    let peer = resolve_peer(&client, Some(folder_id), &state.peer_cache).await?;
    
    let input_channel = match peer {
        Peer::Channel(c) => {
             let chan = &c.raw;
             tl::enums::InputChannel::Channel(tl::types::InputChannel {
                 channel_id: chan.id,
                 access_hash: chan.access_hash.ok_or("No access hash for channel")?,
             })
        },
        _ => return Err("Only channels (folders) can be deleted.".to_string()),
    };
    
    client.invoke(&tl::functions::channels::DeleteChannel {
        channel: input_channel,
    }).await.map_err(|e| format!("Failed to delete channel: {}", e))?;
    
    Ok(true)
}

#[tauri::command]
pub async fn cmd_rename_folder(
    folder_id: i64,
    new_name: String,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let client_opt = {
        state.client.lock().await.clone()
    };
    
    if client_opt.is_none() {
        log::info!("[MOCK] Renamed folder ID {} to {}", folder_id, new_name);
        return Ok(true);
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;
    log::info!("Renaming folder/channel: {} to {}", folder_id, new_name);

    let peer = resolve_peer(&client, Some(folder_id), &state.peer_cache).await?;
    
    let input_channel = match peer {
        Peer::Channel(c) => {
             let chan = &c.raw;
             tl::enums::InputChannel::Channel(tl::types::InputChannel {
                 channel_id: chan.id,
                 access_hash: chan.access_hash.ok_or("No access hash for channel")?,
             })
        },
        _ => return Err("Only channels (folders) can be renamed.".to_string()),
    };
    
    client.invoke(&tl::functions::channels::EditTitle {
        channel: input_channel,
        title: format!("{} [TD]", new_name),
    }).await.map_err(|e| format!("Failed to rename channel: {}", e))?;
    
    Ok(true)
}


#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    id: String,
    percent: u8,
    uploaded_bytes: u64,
    total_bytes: u64,
    speed_bytes_per_sec: u64,
}

/// Async reader wrapper that tracks bytes read for progress reporting.
/// Wraps a tokio File and counts how many bytes have been consumed.
struct ProgressReader {
    inner: tokio::io::BufReader<tokio::fs::File>,
    bytes_read: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ProgressReader {
    async fn new(path: &str) -> Result<(Self, u64, std::sync::Arc<std::sync::atomic::AtomicU64>), String> {
        let file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
        let metadata = file.metadata().await.map_err(|e| e.to_string())?;
        let size = metadata.len();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let reader = Self {
            inner: tokio::io::BufReader::new(file),
            bytes_read: counter.clone(),
        };
        Ok((reader, size, counter))
    }
}

impl tokio::io::AsyncRead for ProgressReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &result {
            let after = buf.filled().len();
            let delta = (after - before) as u64;
            self.bytes_read.fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
        }
        result
    }
}

/// Delete a partial file with retries (best-effort cleanup)
fn cleanup_partial_file(path: &str) {
    let path = path.to_string();
    std::thread::spawn(move || {
        for attempt in 0..5 {
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    log::info!("Cleaned up partial file: {}", path);
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => {
                    log::warn!("Cleanup attempt {}/5 failed for {}: {}", attempt + 1, path, e);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
    });
}

#[tauri::command]
pub async fn cmd_cancel_transfer(
    transfer_id: String,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    log::info!("Cancelling transfer: {}", transfer_id);
    state.cancelled_transfers.write().await.insert(transfer_id.clone());
    if let Some(tx) = get_upload_cancellations().lock().unwrap().remove(&transfer_id) {
        let _ = tx.send(());
    }
    Ok(true)
}

#[cfg_attr(not(target_os = "android"), allow(unused_mut))]
#[tauri::command]
pub async fn cmd_upload_file(
    mut path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    as_video: Option<bool>,
    delete_after_upload: Option<bool>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
) -> Result<String, String> {
    let mut temp_cache_path: Option<String> = None;

    // Strict JNI Interception Guard for Android URI Schemes
    #[cfg(target_os = "android")]
    {
        if path.contains("content://") || path.contains("msf:") || path.contains("msf%") {
            match copy_to_android_cache(&path) {
                Ok(cached_path) => {
                    log::info!("JNI STRICT GUARD: Intercepted raw URI. Overwriting path: {} -> {}", path, cached_path);
                    temp_cache_path = Some(cached_path.clone());
                    path = cached_path;
                }
                Err(err) => {
                    return Err(format!("JNI STRICT GUARD FAILURE: Failed to copy raw URI {} to android cache: {}", path, err));
                }
            }
        }
    }

    let result = cmd_upload_file_inner(
        path.clone(),
        folder_id,
        transfer_id,
        as_video,
        delete_after_upload,
        app_handle,
        state,
        bw_state,
        net_config,
    ).await;

    if let Some(ref cache_path) = temp_cache_path {
        let _ = tokio::fs::remove_file(cache_path).await;
        log::info!("Removed temporary upload cache file: {}", cache_path);
    }

    result
}

async fn cmd_upload_file_inner(
    path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    as_video: Option<bool>,
    delete_after_upload: Option<bool>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
) -> Result<String, String> {

    let size = tokio::fs::metadata(&path).await.map_err(|e| e.to_string())?.len();
    bw_state.can_transfer(size)?;

    let tid = transfer_id.unwrap_or_default();

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!("[MOCK] Uploaded file {} to {:?}", path, folder_id);
        bw_state.add_up(size);
        if delete_after_upload.unwrap_or(false) {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                log::warn!("Auto-delete failed for mock upload {}: {}", path, e);
            } else {
                log::info!("Auto-deleted local file after successful mock upload: {}", path);
            }
        }
        return Ok("Mock upload successful".to_string());
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    // Emit start progress
    if !tid.is_empty() {
        let _ = app_handle.emit("upload-progress", ProgressPayload {
            id: tid.clone(), percent: 0, uploaded_bytes: 0, total_bytes: size, speed_bytes_per_sec: 0,
        });
    }

    // Create progress-tracking reader
    let (mut reader, file_size, bytes_counter) = ProgressReader::new(&path).await?;
    let file_name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    // Spawn a progress reporter task that emits events every 250ms
    let cancelled = state.cancelled_transfers.clone();
    let progress_tid = tid.clone();
    let progress_handle = app_handle.clone();
    let progress_counter = bytes_counter.clone();
    let progress_task = if !tid.is_empty() {
        Some(tokio::spawn(async move {
            let mut last_bytes: u64 = 0;
            let mut last_time = std::time::Instant::now();
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                let current = progress_counter.load(std::sync::atomic::Ordering::Relaxed);
                let now = std::time::Instant::now();
                let dt = now.duration_since(last_time).as_secs_f64();
                let speed = if dt > 0.0 { ((current - last_bytes) as f64 / dt) as u64 } else { 0 };
                let percent = if file_size > 0 { ((current as f64 / file_size as f64) * 100.0).min(99.0) as u8 } else { 0 };

                let _ = progress_handle.emit("upload-progress", ProgressPayload {
                    id: progress_tid.clone(), percent, uploaded_bytes: current, total_bytes: file_size, speed_bytes_per_sec: speed,
                });

                last_bytes = current;
                last_time = now;

                if current >= file_size { break; }
                // Check cancellation
                if cancelled.read().await.contains(&progress_tid) { break; }
            }
        }))
    } else {
        None
    };

    // Check cancellation before starting
    if state.cancelled_transfers.read().await.contains(&tid) {
        state.cancelled_transfers.write().await.remove(&tid);
        if let Some(t) = progress_task { t.abort(); }
        return Err("Transfer cancelled".to_string());
    }

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    if !tid.is_empty() {
        get_upload_cancellations().lock().unwrap().insert(tid.clone(), cancel_tx);
    }

    let client_clone = client.clone();
    let mut upload_task = tokio::spawn(async move {
        client_clone.upload_stream(&mut reader, file_size as usize, file_name).await
    });

    let upload_result = {
        tokio::select! {
            res = &mut upload_task => {
                if !tid.is_empty() {
                    get_upload_cancellations().lock().unwrap().remove(&tid);
                }
                res.map_err(|e| format!("Task join error: {}", e))?
            }
            _ = cancel_rx => {
                log::info!("Aborting upload task for transfer ID: {}", tid);
                upload_task.abort();
                state.cancelled_transfers.write().await.remove(&tid);
                if let Some(t) = progress_task { t.abort(); }
                return Err("Transfer cancelled".to_string());
            }
        }
    };

    // Stop progress reporter
    if let Some(t) = progress_task { t.abort(); }

    let uploaded_file = upload_result.map_err(map_error)?;

    let lower_path = path.to_lowercase();
    let is_video_ext = lower_path.ends_with(".mp4") || lower_path.ends_with(".mkv") || lower_path.ends_with(".avi") || lower_path.ends_with(".mov") || lower_path.ends_with(".webm") || lower_path.ends_with(".flv");
    let is_video = as_video.unwrap_or(false) && is_video_ext;

    let message = if is_video {
        use grammers_client::types::Attribute;
        InputMessage::new().text("").document(uploaded_file).attribute(Attribute::Video {
            round_message: false,
            supports_streaming: true,
            duration: std::time::Duration::from_secs(0),
            w: 0,
            h: 0,
        })
    } else {
        InputMessage::new().text("").file(uploaded_file)
    };

    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;

    // VPN-aware retry logic for send_message
    let max_retries = net_config.retry_attempts();
    let base_ms = net_config.retry_base_backoff_ms();
    let max_ms = net_config.retry_max_backoff_ms();
    let respect_flood = net_config.should_respect_flood_wait();
    let mut last_err = String::new();

    for attempt in 0..=max_retries {
        match client.send_message(&peer, message.clone()).await {
            Ok(_) => {
                bw_state.add_up(size);
                if !tid.is_empty() {
                    let _ = app_handle.emit("upload-progress", ProgressPayload {
                        id: tid, percent: 100, uploaded_bytes: size, total_bytes: size, speed_bytes_per_sec: 0,
                    });
                }
                if delete_after_upload.unwrap_or(false) {
                    if let Err(e) = tokio::fs::remove_file(&path).await {
                        log::error!("Failed to auto-delete local file: {}", e);
                    } else {
                        log::info!("Auto-deleted local file after successful upload: {}", path);
                    }
                }
                return Ok("File uploaded successfully".to_string());
            }
            Err(e) => {
                let err = map_error(e);
                log::warn!("send_message attempt {}/{}: {}", attempt + 1, max_retries + 1, err);

                // Handle FLOOD_WAIT: sleep the requested time if configured
                if respect_flood && err.starts_with("FLOOD_WAIT_") {
                    if let Ok(secs) = err.trim_start_matches("FLOOD_WAIT_").parse::<u64>() {
                        let wait = secs.min(300); // cap at 5 min
                        log::info!("Respecting FLOOD_WAIT: sleeping {}s", wait);
                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                        last_err = err;
                        continue;
                    }
                }

                last_err = err;
                if attempt < max_retries {
                    let delay = backoff_ms(attempt, base_ms, max_ms);
                    log::info!("Retrying in {}ms...", delay);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
        }
    }

    Err(format!("Upload failed after {} attempts: {}", max_retries + 1, last_err))
}

#[tauri::command]
pub async fn initiate_upload(
    path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    as_video: Option<bool>,
    delete_after_upload: Option<bool>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
) -> Result<String, String> {
    crate::upload_service::start_foreground_service();
    cmd_upload_file(
        path,
        folder_id,
        transfer_id,
        as_video,
        delete_after_upload,
        app_handle,
        state,
        bw_state,
        net_config,
    ).await
}

#[tauri::command]
pub async fn cmd_delete_file(
    message_id: i32,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
         log::info!("[MOCK] Deleted message {} from folder {:?}", message_id, folder_id);
        return Ok(true); 
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;
    client.delete_messages(&peer, &[message_id]).await.map_err(|e| e.to_string())?;
    Ok(true)
}

#[derive(Debug, serde::Deserialize)]
pub struct DownloadFileRequest {
    message_id: i32,
    save_path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
}

#[tauri::command]
pub async fn cmd_download_file(
    req: DownloadFileRequest,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
) -> Result<String, String> {
    let tid = req.transfer_id.unwrap_or_default();
    let save_path = req.save_path;
    let folder_id = req.folder_id;
    let message_id = req.message_id;

    #[cfg(target_os = "android")]
    let (actual_save_path, android_file_name) = {
        use tauri::Manager;
        let cache_dir = app_handle
            .path()
            .app_cache_dir()
            .map_err(|e| format!("Failed to get cache dir: {}", e))?;
        if !cache_dir.exists() {
            let _ = std::fs::create_dir_all(&cache_dir);
        }
        // Android: save_path may be a content:// URI. Try to extract a clean filename.
        let raw = std::path::Path::new(&save_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download.bin");
        // URL-decode in case the path came from a content:// URI (e.g. primary%2Fmyfile.pdf)
        let decoded = url_decode(raw).trim_end_matches('/').to_string();
        // If the decoded value still looks like a URI path, take only the last segment
        let clean_name = std::path::Path::new(&decoded)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&decoded)
            .to_string();
        let file_name = if clean_name.is_empty() { "download.bin".to_string() } else { clean_name };
        let cache_path = cache_dir.join(&file_name).to_string_lossy().to_string();
        log::info!("Android download: save_path='{}', extracted filename='{}', cache='{}'", save_path, file_name, cache_path);
        (cache_path, file_name)
    };

    #[cfg(not(target_os = "android"))]
    let actual_save_path = save_path.clone();

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
        log::info!("[MOCK] Downloaded message {} from {:?} to {}", message_id, folder_id, actual_save_path);
        if let Err(e) = tokio::fs::write(&actual_save_path, b"Mock Content").await { return Err(e.to_string()); }
        return Ok("Download successful".to_string());
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;
    
    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;

    // Use get_messages_by_id for efficient message lookup (same as server.rs)
    let messages = client.get_messages_by_id(&peer, &[message_id]).await.map_err(|e| e.to_string())?;
    
    let msg = messages.into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "Message not found".to_string())?;

    let media = msg.media()
        .ok_or_else(|| "No media in message".to_string())?;

    let expected_file_size = match &media {
        Media::Document(d) => Some(d.size() as u64),
        _ => None,
    };
    let total_size = expected_file_size.unwrap_or(match &media {
        Media::Photo(_) => 1024 * 1024,
        _ => 0,
    });
    
    bw_state.can_transfer(total_size)?;

    // Emit start
    if !tid.is_empty() {
        let _ = app_handle.emit("download-progress", ProgressPayload {
            id: tid.clone(), percent: 0, uploaded_bytes: 0, total_bytes: total_size, speed_bytes_per_sec: 0,
        });
    }

    // Stream download with per-chunk progress
    let mut download_iter = client.iter_download(&media);
    let mut file = tokio::fs::File::create(&actual_save_path).await.map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut last_emit_time = std::time::Instant::now();
    let mut last_emit_bytes: u64 = 0;
    let mut chunk_retry_budget = net_config.retry_attempts();

    while let Some(chunk) = download_iter.next().await.transpose() {
        // Check cancellation
        if state.cancelled_transfers.read().await.contains(&tid) {
            state.cancelled_transfers.write().await.remove(&tid);
            drop(file);
            cleanup_partial_file(&actual_save_path);
            return Err("Transfer cancelled".to_string());
        }

        let bytes = match chunk {
            Ok(b) => {
                chunk_retry_budget = net_config.retry_attempts(); // reset on success
                b
            },
            Err(e) => {
                let err = map_error(&e);
                if chunk_retry_budget > 0 {
                    chunk_retry_budget -= 1;
                    log::warn!("Download chunk error (retries left: {}): {}", chunk_retry_budget, err);
                    let delay = backoff_ms(0, net_config.retry_base_backoff_ms(), net_config.retry_max_backoff_ms());
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }
                return Err(format!("Download chunk error: {}", err));
            }
        };
        tokio::io::AsyncWriteExt::write_all(&mut file, &bytes).await.map_err(|e| e.to_string())?;
        downloaded += bytes.len() as u64;
        
        // Time-based progress emission (every 250ms)
        if !tid.is_empty() {
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_emit_time).as_secs_f64();
            if dt >= 0.25 || downloaded >= total_size {
                let speed = if dt > 0.0 { ((downloaded - last_emit_bytes) as f64 / dt) as u64 } else { 0 };
                let percent = if total_size > 0 { ((downloaded as f64 / total_size as f64) * 100.0).min(100.0) as u8 } else { 0 };
                let _ = app_handle.emit("download-progress", ProgressPayload {
                    id: tid.clone(), percent, uploaded_bytes: downloaded, total_bytes: total_size, speed_bytes_per_sec: speed,
                });
                last_emit_time = now;
                last_emit_bytes = downloaded;
            }
        }

        // Bandwidth throttle: if download limit is set, sleep to maintain rate
        let dl_limit = net_config.download_limit_bytes_per_sec();
        if dl_limit > 0 {
            let elapsed = last_emit_time.elapsed().as_secs_f64().max(0.001);
            let current_rate = (downloaded - last_emit_bytes) as f64 / elapsed;
            if current_rate > dl_limit as f64 {
                let sleep_ms = ((current_rate / dl_limit as f64 - 1.0) * elapsed * 1000.0) as u64;
                if sleep_ms > 0 && sleep_ms < 5000 {
                    tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                }
            }
        }
    }

    bw_state.add_down(total_size);

    // Explicitly flush, sync, and close the file before JNI/MediaStore copies it.
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| format!("Failed to flush downloaded file: {}", e))?;
    file.sync_all()
        .await
        .map_err(|e| format!("Failed to sync downloaded file: {}", e))?;
    drop(file);

    let actual_written = tokio::fs::metadata(&actual_save_path)
        .await
        .map_err(|e| format!("Downloaded file missing before save: {}", e))?
        .len();
    if actual_written == 0 {
        cleanup_partial_file(&actual_save_path);
        return Err("Downloaded file was empty before saving".to_string());
    }
    if actual_written != downloaded {
        cleanup_partial_file(&actual_save_path);
        return Err(format!(
            "Downloaded file size mismatch before saving: streamed {} bytes, file has {} bytes",
            downloaded, actual_written
        ));
    }
    if let Some(expected) = expected_file_size {
        if expected > 0 && downloaded != expected {
            cleanup_partial_file(&actual_save_path);
            return Err(format!(
                "Incomplete download before saving: expected {} bytes, received {} bytes",
                expected, downloaded
            ));
        }
    }
    log::info!(
        "Download completed to cache path {} ({} bytes)",
        actual_save_path,
        actual_written
    );

    // Emit completion
    if !tid.is_empty() {
        let _ = app_handle.emit("download-progress", ProgressPayload {
            id: tid, percent: 100, uploaded_bytes: downloaded, total_bytes: total_size, speed_bytes_per_sec: 0,
        });
    }

    #[cfg(target_os = "android")]
    {
        // Copy from actual_save_path to public downloads via MediaStore JNI!
        // Use the already-decoded filename from the cache path computation above
        let file_name = &android_file_name;
            
        let lower_ext = std::path::Path::new(file_name)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();
            
        let mime_type = match lower_ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "pdf" => "application/pdf",
            "mp4" => "video/mp4",
            "mp3" => "audio/mpeg",
            "txt" => "text/plain",
            "zip" => "application/zip",
            "bin" => "application/octet-stream",
            _ => "application/octet-stream",
        };

        log::info!("JNI: Copying {} from cache {} to public downloads", file_name, actual_save_path);
        
        let jni_success = {
            let mut success = false;
            let ctx = ndk_context::android_context();
            if let Ok(vm) = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) } {
                if let Ok(mut env) = vm.attach_current_thread() {
                    if let Some(cached_ref) = crate::jni_cache::get_main_activity_class() {
                        let main_class: jni::objects::JClass = unsafe { std::mem::transmute_copy(cached_ref.as_obj()) };
                        if let Ok(j_cache_path) = env.new_string(&actual_save_path) {
                            if let Ok(j_file_name) = env.new_string(file_name) {
                                if let Ok(j_mime_type) = env.new_string(mime_type) {
                                    let call_res = env.call_static_method(
                                        &main_class,
                                        "saveFileToPublicDownloads",
                                        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z",
                                        &[
                                            jni::objects::JValue::from(&j_cache_path),
                                            jni::objects::JValue::from(&j_file_name),
                                            jni::objects::JValue::from(&j_mime_type),
                                        ],
                                    );
                                    
                                    match call_res {
                                        Ok(val) => {
                                            if let Ok(b) = val.z() {
                                                success = b;
                                            }
                                        }
                                        Err(e) => {
                                            log::error!("JNI: saveFileToPublicDownloads call failed: {}", e);
                                            if env.exception_check().unwrap_or(false) {
                                                let _ = env.exception_describe();
                                                let _ = env.exception_clear();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        log::error!("JNI: MainActivity class reference was NOT cached globally!");
                    }
                }
            }
            success
        };
        
        if !jni_success {
            // Keep the cache file as a fallback so the user's data is not lost
            log::error!("JNI: Failed to copy to public downloads. Cache file preserved at: {}", actual_save_path);
            return Err("Failed to save downloaded file to public downloads folder".to_string());
        }
        
        // Only clean up the cache copy AFTER confirming JNI succeeded
        let _ = tokio::fs::remove_file(&actual_save_path).await;
        log::info!("JNI: Successfully copied to public downloads and cleaned up cache: {}", actual_save_path);
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
    if source_folder_id == target_folder_id { return Ok(true); }
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
        log::info!("[MOCK] Moved msgs {:?} from {:?} to {:?}", message_ids, source_folder_id, target_folder_id);
        return Ok(true); 
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let source_peer = resolve_peer(&client, source_folder_id, &state.peer_cache).await?;
    let target_peer = resolve_peer(&client, target_folder_id, &state.peer_cache).await?;

    match client.forward_messages(&target_peer, &message_ids, &source_peer).await {
        Ok(_) => {},
        Err(e) => return Err(format!("Forward failed: {}", e)),
    }
    
    match client.delete_messages(&source_peer, &message_ids).await {
        Ok(_) => {},
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
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;
    let mut files = Vec::new();
    
    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;

    let mut msgs = client.iter_messages(&peer);
    while let Some(msg) = msgs.next().await.map_err(|e| e.to_string())? {
        if let Some(doc) = msg.media() {
            let (name, size, mime, ext) = match doc {
                Media::Document(d) => {
                    let n = d.name().to_string();
                    let s = d.size();
                    let m = d.mime_type().map(|s| s.to_string());
                    let e = std::path::Path::new(&n).extension().map(|os| os.to_str().unwrap_or("").to_string());
                    (n, s, m, e)
                },
                Media::Photo(_) => ("Photo.jpg".to_string(), 0, Some("image/jpeg".into()), Some("jpg".into())),
                _ => ("Unknown".to_string(), 0, None, None),
            };
            files.push(FileMetadata {
                id: msg.id() as i64, folder_id, name, size: size as u64, mime_type: mime, file_ext: ext, created_at: msg.date().to_string(), icon_type: "file".into()
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
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;
    let mut files = Vec::new();
    
    log::info!("Searching global for: {}", query);

    let result = client.invoke(&tl::functions::messages::SearchGlobal {
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
    }).await.map_err(map_error)?;

    if let tl::enums::messages::Messages::Messages(msgs) = result {
        for msg in msgs.messages {
            if let tl::enums::Message::Message(m) = msg {
                if let Some(tl::enums::MessageMedia::Document(d)) = m.media {
                    if let Some(tl::enums::Document::Document(doc)) = d.document {
                        let name = doc.attributes.iter().find_map(|a| match a {
                            tl::enums::DocumentAttribute::Filename(f) => Some(f.file_name.clone()),
                            _ => None
                        }).unwrap_or("Unknown".to_string());
                        let size = doc.size as u64;
                        let mime = doc.mime_type.clone();
                        let ext = std::path::Path::new(&name).extension().map(|os| os.to_str().unwrap_or("").to_string());
                        let folder_id = match m.peer_id {
                            tl::enums::Peer::Channel(c) => Some(c.channel_id),
                            tl::enums::Peer::User(u) => Some(u.user_id),
                            tl::enums::Peer::Chat(c) => Some(c.chat_id),
                        };
                        files.push(FileMetadata {
                            id: m.id as i64, folder_id, name, size,
                            mime_type: Some(mime), file_ext: ext,
                            created_at: m.date.to_string(), icon_type: "file".into()
                        });
                    }
                }
            }
        }
    } else if let tl::enums::messages::Messages::Slice(msgs) = result {
        for msg in msgs.messages {
            if let tl::enums::Message::Message(m) = msg {
                if let Some(tl::enums::MessageMedia::Document(d)) = m.media {
                    if let Some(tl::enums::Document::Document(doc)) = d.document {
                        let name = doc.attributes.iter().find_map(|a| match a {
                            tl::enums::DocumentAttribute::Filename(f) => Some(f.file_name.clone()),
                            _ => None
                        }).unwrap_or("Unknown".to_string());
                        let size = doc.size as u64;
                        let mime = doc.mime_type.clone();
                        let ext = std::path::Path::new(&name).extension().map(|os| os.to_str().unwrap_or("").to_string());
                        let folder_id = match m.peer_id {
                            tl::enums::Peer::Channel(c) => Some(c.channel_id),
                            tl::enums::Peer::User(u) => Some(u.user_id),
                            tl::enums::Peer::Chat(c) => Some(c.chat_id),
                        };
                        files.push(FileMetadata {
                            id: m.id as i64, folder_id, name, size,
                            mime_type: Some(mime), file_ext: ext,
                            created_at: m.date.to_string(), icon_type: "file".into()
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
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;
    
    let mut folders = Vec::new();
    let mut dialogs = client.iter_dialogs();
    let mut discovered = HashMap::new();
    
    log::info!("Starting Folder Scan...");

    while let Some(dialog) = dialogs.next().await.map_err(|e| e.to_string())? {
        // Populate peer cache for every dialog we encounter (free priming)
        match &dialog.peer {
            Peer::Channel(c) => {
                let id = c.raw.id;
                discovered.insert(id, dialog.peer.clone());

                let name = c.raw.title.clone();
                let access_hash = c.raw.access_hash.unwrap_or(0);
                
                log::debug!("[SCAN] Processing Channel: '{}' (ID: {})", name, id);

                // Strategy 1: Title
                if name.to_lowercase().contains("[td]") {
                    log::info!(" -> MATCH via Title: {}", name);
                    let display_name = name.replace(" [TD]", "").replace(" [td]", "").replace("[TD]", "").replace("[td]", "").trim().to_string();
                    let username = c.raw.username.clone();
                    let is_public = username.is_some();
                    folders.push(FolderMetadata { id, name: display_name, parent_id: None, username, is_public });
                    continue; 
                }

                // Strategy 2: About (Only if we are the creator to avoid rate limits on third-party channels)
                if c.raw.creator {
                    let input_chan = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                        channel_id: c.raw.id,
                        access_hash,
                    });
                    
                    match client.invoke(&tl::functions::channels::GetFullChannel {
                        channel: input_chan,
                    }).await {
                        Ok(tl::enums::messages::ChatFull::Full(f)) => {
                            if let tl::enums::ChatFull::Full(cf) = f.full_chat {
                                 if cf.about.contains("[telegram-drive-folder]") {
                                     log::info!(" -> MATCH via About: {}", name);
                                     let username = c.raw.username.clone();
                                     let is_public = username.is_some();
                                     folders.push(FolderMetadata { id, name: name.clone(), parent_id: None, username, is_public });
                                 }
                            }
                        },
                        Err(e) => log::warn!(" -> Failed to get full info: {}", e),
                    }
                }
            },
            Peer::User(u) => {
                discovered.insert(u.raw.id(), dialog.peer.clone());
                log::debug!("[SCAN] Cached User Peer: {}", u.raw.id());
            },
            peer => {
                log::debug!("[SCAN] Skipped Peer: {:?}", peer);
            }
        }
    }
    
    {
        let mut cache = state.peer_cache.write().await;
        cache.extend(discovered);
    }
    
    let cache_len = state.peer_cache.read().await.len();
    log::info!("Scan complete. Found {} folders. Peer cache size: {}.", folders.len(), cache_len);
    Ok(folders)
}

/// Zip a folder's contents into a temp file and return the path.
/// The resulting zip preserves the relative directory structure.
#[tauri::command]
pub async fn cmd_zip_folder(
    folder_path: String,
) -> Result<String, String> {
    let folder_path = if cfg!(target_os = "android") {
        clean_android_path(&folder_path)
    } else {
        folder_path
    };

    let src = std::path::Path::new(&folder_path)
        .canonicalize()
        .map_err(|e| format!("Invalid folder path: {}", e))?;
    if !src.is_dir() {
        return Err(format!("'{}' is not a directory", folder_path));
    }

    let folder_name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "folder".to_string());

    let zip_path = std::env::temp_dir().join(format!("{}.zip", folder_name));
    let src_owned = src.clone();
    let out_path = zip_path.clone();

    // Run blocking I/O on a dedicated thread so we don't stall the async runtime
    let (zip_path_str, zip_size) = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::create(&out_path)
            .map_err(|e| format!("Failed to create zip file: {}", e))?;
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        for entry in walkdir::WalkDir::new(&src_owned).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let relative = path.strip_prefix(&src_owned).unwrap_or(path);

            if path.is_file() {
                let name = relative.to_string_lossy().to_string();
                zip_writer.start_file(&name, options)
                    .map_err(|e| format!("Failed to add '{}': {}", name, e))?;
                let mut f = std::fs::File::open(path)
                    .map_err(|e| format!("Failed to open '{}': {}", name, e))?;
                std::io::copy(&mut f, &mut zip_writer)
                    .map_err(|e| format!("Failed to write '{}': {}", name, e))?;
            } else if path.is_dir() && path != src_owned {
                let dir_name = format!("{}/", relative.to_string_lossy());
                zip_writer.add_directory(&dir_name, options)
                    .map_err(|e| format!("Failed to add dir '{}': {}", dir_name, e))?;
            }
        }

        zip_writer.finish().map_err(|e| format!("Failed to finalize zip: {}", e))?;
        let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
        Ok::<(String, u64), String>((out_path.to_string_lossy().to_string(), size))
    })
    .await
    .map_err(|e| format!("Zip task panicked: {}", e))?
    .map_err(|e: String| e)?;

    log::info!("Zipped '{}' -> '{}' ({} bytes)", folder_name, zip_path_str, zip_size);

    Ok(zip_path_str)
}

/// Delete a temporary zip file created by cmd_zip_folder.
#[tauri::command]
pub async fn cmd_delete_temp_zip(
    path: String,
) -> Result<(), String> {
    let path_clone = path.clone();
    tokio::task::spawn_blocking(move || {
        let p = std::path::Path::new(&path_clone);
        if !p.exists() {
            return Ok(());
        }
        let canonical_p = p.canonicalize().map_err(|e| format!("Invalid path: {}", e))?;
        let tmp = std::env::temp_dir().canonicalize().map_err(|e| format!("Could not resolve temp directory: {}", e))?;
        if !canonical_p.starts_with(&tmp) {
            return Err("Refusing to delete file outside temp directory".to_string());
        }
        std::fs::remove_file(&canonical_p).map_err(|e| e.to_string())?;
        log::info!("Cleaned up temp zip: {}", path_clone);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

/// Toggle a folder (channel) between private and public.
/// When making public, a username is generated from the channel title.
/// When making private, the username is removed.
#[tauri::command]
pub async fn cmd_toggle_folder_visibility(
    folder_id: i64,
    make_public: bool,
    desired_username: Option<String>,
    state: State<'_, TelegramState>,
) -> Result<FolderMetadata, String> {
    let client_opt = {
        state.client.lock().await.clone()
    };

    if client_opt.is_none() {
        log::info!("[MOCK] Toggle visibility for folder {}. Public: {}", folder_id, make_public);
        return Ok(FolderMetadata {
            id: folder_id,
            name: "Mock Folder".to_string(),
            parent_id: None,
            username: if make_public { desired_username } else { None },
            is_public: make_public,
        });
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let peer = resolve_peer(&client, Some(folder_id), &state.peer_cache).await?;
    let (channel_id, access_hash) = match &peer {
        Peer::Channel(c) => (c.raw.id, c.raw.access_hash.ok_or("No access hash for channel")?),
        _ => return Err("Only channels (folders) can be toggled.".to_string()),
    };

    let input_channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
        channel_id,
        access_hash,
    });

    // Extract channel name from the resolved peer for the return value
    let channel_name = match &peer {
        Peer::Channel(c) => {
            c.raw.title
                .replace(" [TD]", "")
                .replace(" [td]", "")
                .trim()
                .to_string()
        }
        _ => "Folder".to_string(),
    };

    if make_public {
        // Generate a username from the desired_username or channel title.
        // If desired_username is provided AND non-empty, use it directly;
        // otherwise auto-generate from the channel title.
        let username = if let Some(ref u) = desired_username {
            if !u.is_empty() {
                Some(u.clone())
            } else {
                None // empty string → fall through to auto-generation below
            }
        } else {
            None
        };

        let username = match username {
            Some(given) => {
                // User-provided username: check availability first
                let available = client
                    .invoke(&tl::functions::channels::CheckUsername {
                        channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                            channel_id,
                            access_hash,
                        }),
                        username: given.clone(),
                    })
                    .await
                    .map_err(|e| format!("Failed to check username availability: {}", map_error(e)))?;
                if !available {
                    return Err(format!("Username '{}' is not available. Try a different one.", given));
                }
                given
            }
            None => {
                // Auto-generate username from channel title
                // channel_name already has [TD] stripped above
                let mut base = channel_name.clone()
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_')
                    .take(30)
                    .collect::<String>();
                if base.len() < 5 {
                    let suffix: String = (0..6)
                        .map(|_| char::from(b'a' + (rand::random::<u8>() % 26)))
                        .collect();
                    base = format!("{}_{}", base, suffix);
                }
                // Try to find an available username
                let mut candidate = base.clone();
                for attempt in 1..=10 {
                    match client
                        .invoke(&tl::functions::channels::CheckUsername {
                            channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                                channel_id,
                                access_hash,
                            }),
                            username: candidate.clone(),
                        })
                        .await
                    {
                        Ok(true) => break,
                        _ => {
                            candidate = format!("{}{}", base, attempt);
                            if attempt == 10 {
                                return Err("Could not find an available username after 10 attempts".to_string());
                            }
                        }
                    }
                }
                candidate
            }
        };

        log::info!("Setting channel {} username to '{}'", channel_id, username);
        client
            .invoke(&tl::functions::channels::UpdateUsername {
                channel: input_channel,
                username: username.clone(),
            })
            .await
            .map_err(|e| format!("Failed to set username: {}", map_error(e)))?;

        Ok(FolderMetadata {
            id: channel_id,
            name: channel_name,
            parent_id: None,
            username: Some(username),
            is_public: true,
        })
    } else {
        // Make private: remove username
        log::info!("Removing username from channel {}", channel_id);
        client
            .invoke(&tl::functions::channels::UpdateUsername {
                channel: input_channel,
                username: String::new(),
            })
            .await
            .map_err(|e| format!("Failed to remove username: {}", map_error(e)))?;

        Ok(FolderMetadata {
            id: channel_id,
            name: channel_name,
            parent_id: None,
            username: None,
            is_public: false,
        })
    }
}

/// Export a Telegram invite link for a folder (channel).
/// For public channels, returns the t.me/username link directly.
/// For private channels, exports a hash-based invite link via the API.
#[derive(Debug, Serialize)]
pub struct FolderInviteInfo {
    pub link: String,
    pub is_public: bool,
    pub username: Option<String>,
}

#[tauri::command]
pub async fn cmd_export_folder_invite(
    folder_id: i64,
    state: State<'_, TelegramState>,
) -> Result<FolderInviteInfo, String> {
    let client_opt = {
        state.client.lock().await.clone()
    };

    if client_opt.is_none() {
        log::info!("[MOCK] Export invite for folder {}", folder_id);
        return Ok(FolderInviteInfo {
            link: "https://t.me/joinchat/mock-invite-hash".to_string(),
            is_public: false,
            username: None,
        });
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let peer = resolve_peer(&client, Some(folder_id), &state.peer_cache).await?;
    let (channel_id, access_hash) = match &peer {
        Peer::Channel(c) => (c.raw.id, c.raw.access_hash.ok_or("No access hash for channel")?),
        _ => return Err("Only channels (folders) can have invite links.".to_string()),
    };

    // Check if channel already has a public username (use the resolved peer directly)
    let username: Option<String> = match &peer {
        Peer::Channel(c) => c.raw.username.clone(),
        _ => None,
    };

    if let Some(ref uname) = username {
        // Public channel: return the t.me/username link
        Ok(FolderInviteInfo {
            link: format!("https://t.me/{}", uname),
            is_public: true,
            username: Some(uname.clone()),
        })
    } else {
        // Private channel: export an invite link
        let result = client
            .invoke(&tl::functions::messages::ExportChatInvite {
                peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                    channel_id,
                    access_hash,
                }),
                legacy_revoke_permanent: false,
                request_needed: false,
                expire_date: None,
                usage_limit: None,
                title: None,
                subscription_pricing: None,
            })
            .await
            .map_err(|e| format!("Failed to export invite: {}", map_error(e)))?;

        let link = match result {
            tl::enums::ExportedChatInvite::ChatInviteExported(c) => c.link,
            tl::enums::ExportedChatInvite::ChatInvitePublicJoinRequests => {
                return Err("Public join request channels do not have a custom private invite link. Share the public username directly instead.".to_string());
            }
        };

        Ok(FolderInviteInfo {
            link,
            is_public: false,
            username: None,
        })
    }
}
