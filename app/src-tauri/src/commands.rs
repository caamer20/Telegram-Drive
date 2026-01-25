use tauri::State;
use tauri::Manager;
use tokio::sync::Mutex;
use grammers_client::{Client, SignInError, InputMessage};
use grammers_client::types::{LoginToken, PasswordToken, Peer};
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use crate::models::{AuthResult, FileMetadata, FolderMetadata};
use std::sync::Arc;
use grammers_tl_types as tl;
use crate::bandwidth::BandwidthManager;

pub struct TelegramState {
    pub client: Mutex<Option<Client>>,
    pub login_token: Mutex<Option<LoginToken>>,
    pub password_token: Mutex<Option<PasswordToken>>,
}

// Helper to ensure client is initialized
async fn ensure_client_initialized(
    app_handle: &tauri::AppHandle,
    state: &State<'_, TelegramState>,
    api_id: i32,
) -> Result<Client, String> {
    let mut client_guard = state.client.lock().await;

    if let Some(client) = client_guard.as_ref() {
        return Ok(client.clone());
    }

    println!("Initializing new Telegram Client (v0.8) with ID: {}", api_id);
    
    // Resolve session path safely
    use tauri::Manager;
    let app_data_dir = app_handle.path().app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
        
    if !app_data_dir.exists() {
        std::fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("Failed to create app data dir: {}", e))?;
    }
    
    let session_path = app_data_dir.join("telegram.session");
    let session_path_str = session_path.to_string_lossy().to_string();
    println!("Opening session at: {}", session_path_str);
    
    // Grammers 0.8 initialization with corruption recovery
    let session = match SqliteSession::open(&session_path_str).map_err(|e| e.to_string()) {
        Ok(s) => s,
        Err(_) => {
            println!("Session file corrupted or invalid. Recreating...");
            let _ = std::fs::remove_file(&session_path);
            let _ = std::fs::remove_file(format!("{}-wal", session_path_str));
            let _ = std::fs::remove_file(format!("{}-shm", session_path_str));
            
            SqliteSession::open(&session_path_str)
                .map_err(|e| format!("Failed to open session after recreation: {}", e))?
        }
    };
        
    let session = Arc::new(session);
    let pool = SenderPool::new(session, api_id);
    let client = Client::new(&pool);
    
    // Spawn the network runner
    let SenderPool { runner, .. } = pool;
    tauri::async_runtime::spawn(async move {
        runner.run().await;
        println!("Telegram network runner stopped");
    });
    
    *client_guard = Some(client.clone());
    Ok(client)
}

// Helper to resolve peer from folder_id
async fn resolve_peer(client: &Client, folder_id: Option<i64>) -> Result<Peer, String> {
    if let Some(fid) = folder_id {
        let mut dialogs = client.iter_dialogs();
        while let Some(dialog) = dialogs.next().await.map_err(|e| e.to_string())? {
            // We use .raw.id() based on compiler suggestions that .id() might be missing on wrapper types in this version
            match &dialog.peer {
                Peer::Channel(c) => if c.raw.id == fid { return Ok(dialog.peer.clone()); },
                Peer::User(u) => if u.raw.id() == fid { return Ok(dialog.peer.clone()); },
                _ => {}
            }
        }
        Err(format!("Folder/Chat {} not found", fid))
    } else {
        match client.get_me().await {
            Ok(me) => Ok(Peer::User(me)),
            Err(e) => Err(e.to_string()),
        }
    }
}

// Helper for mock files
fn get_mock_files(folder_id: Option<i64>) -> Vec<FileMetadata> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    vec![
        FileMetadata {
            id: 1001,
            name: "Mock_Project_Specs.pdf".into(),
            size: 2048000, 
            mime_type: Some("application/pdf".into()),
            file_ext: Some("pdf".into()),
            icon_type: "file".into(),
            folder_id: None,
            created_at: format!("{}", now),
        },
        FileMetadata {
            id: 1002,
            name: "Vacation_Photos.zip".into(),
            size: 154000000, 
            mime_type: Some("application/zip".into()),
            file_ext: Some("zip".into()),
            icon_type: "file".into(),
            folder_id: Some(999), 
            created_at: format!("{}", now - 86400),
        },
        FileMetadata {
            id: 1003,
            name: "Notes.txt".into(),
            size: 1024, 
            mime_type: Some("text/plain".into()),
            file_ext: Some("txt".into()),
            icon_type: "file".into(),
            folder_id, // Dynamically result in current folder for test
            created_at: format!("{}", now),
        },
    ]
}

#[tauri::command]
pub fn cmd_log(message: String) {
    println!("[FRONTEND] {}", message);
}

#[tauri::command]
pub fn cmd_get_bandwidth(bw_state: State<'_, BandwidthManager>) -> crate::bandwidth::BandwidthStats {
    bw_state.get_stats()
}

#[tauri::command]
pub async fn cmd_connect(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    api_id: i32,
) -> Result<bool, String> {
    ensure_client_initialized(&app_handle, &state, api_id).await?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_logout(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    println!("Logging out...");
    
    // 1. Try to sign out from Telegram (if connected)
    let client_opt = { state.client.lock().await.clone() };
    if let Some(client) = client_opt {
        // We don't strictly care if this fails (e.g. network down), we just want to clear local state.
        let _ = client.sign_out().await; 
    }

    // 2. Clear State
    *state.client.lock().await = None;
    *state.login_token.lock().await = None;
    *state.password_token.lock().await = None;

    // 3. Remove Session File
    let app_data_dir = app_handle.path().app_data_dir().unwrap();
    let session_path = app_data_dir.join("telegram.session");
    let _ = std::fs::remove_file(session_path);
    let _ = std::fs::remove_file(app_data_dir.join("telegram.session-wal"));
    let _ = std::fs::remove_file(app_data_dir.join("telegram.session-shm"));

    Ok(true)
}

#[tauri::command]
pub async fn cmd_auth_request_code(
    app_handle: tauri::AppHandle,
    phone: String,
    api_id: i32,
    api_hash: String,
    state: State<'_, TelegramState>,
) -> Result<String, String> {
    
    if api_hash.trim().is_empty() {
        return Err("API Hash cannot be empty.".to_string());
    }

    let client_handle = ensure_client_initialized(&app_handle, &state, api_id).await?;
    
    println!("Requesting code for {}", phone);
    
    let mut last_error = String::new();
    
    // Retry up to 2 times for AUTH_RESTART or 500
    for i in 1..=2 {
        match client_handle.request_login_code(&phone, &api_hash).await {
            Ok(token) => {
                let mut token_guard = state.login_token.lock().await;
                *token_guard = Some(token);
                return Ok("code_sent".to_string());
            },
            Err(e) => {
                let err_msg = e.to_string();
                println!("Error requesting code (Attempt {}): {}", i, err_msg);
                
                if err_msg.contains("AUTH_RESTART") || err_msg.contains("500") {
                    println!("AUTH_RESTART error detected. Retrying...");
                    last_error = err_msg;
                    // Prepare for retry
                    continue;
                }
                
                // Other errors, fail immediately
                return Err(format!("Telegram Error: {}", err_msg));
            }
        }
    }

    Err(format!("Telegram Error after retry: {}", last_error))
}

#[tauri::command]
pub async fn cmd_auth_sign_in(
    code: String,
    state: State<'_, TelegramState>,
) -> Result<AuthResult, String> {
    println!("Signing in with code...");
    
    let client = {
        let guard = state.client.lock().await;
        guard.as_ref().ok_or("Client not initialized")?.clone()
    };

    let token_guard = state.login_token.lock().await;
    let login_token = token_guard.as_ref().ok_or("No login session found (restart flow)")?;

    match client.sign_in(login_token, &code).await {
        Ok(_user) => {
             println!("Successfully logged in.");
             Ok(AuthResult {
                success: true,
                next_step: Some("dashboard".to_string()),
                error: None,
            })
        }
        Err(SignInError::PasswordRequired(token)) => {
            let mut pw_guard = state.password_token.lock().await;
            *pw_guard = Some(token);

            Ok(AuthResult {
                success: false,
                next_step: Some("password".to_string()),
                error: None,
            })
        }
        Err(e) => {
           println!("Sign in error: {}", e);
           Err(format!("Sign in failed: {}", e))
        }
    }
}

#[tauri::command]
pub async fn cmd_auth_check_password(
    password: String,
    state: State<'_, TelegramState>,
) -> Result<AuthResult, String> {
    let client = {
        let guard = state.client.lock().await;
        guard.as_ref().ok_or("Client not initialized")?.clone()
    };
    
    let mut pw_guard = state.password_token.lock().await;
    let pw_token = pw_guard.take().ok_or("No password session found")?;

    match client.check_password(pw_token, password.as_str()).await {
        Ok(_user) => {
             println!("2FA Success.");
             Ok(AuthResult {
                success: true,
                next_step: Some("dashboard".to_string()),
                error: None,
            })
        }
        Err(e) => Err(format!("2FA Failed: {}", e))
    }
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
        let mock_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        println!("[MOCK] Created folder '{}' with ID {}", name, mock_id);
        return Ok(FolderMetadata {
            id: mock_id,
            name,
            parent_id: None,
        });
    }
    // -----------
    let client = client_opt.unwrap();
    println!("Creating Telegram Channel: {}", name);
    
    let result = client.invoke(&tl::functions::channels::CreateChannel {
        broadcast: true,
        megagroup: false,
        title: name.clone(),
        about: "Telegram Drive Storage Folder\n[telegram-drive-folder]".to_string(),
        geo_point: None,
        address: None,
        for_import: false,
        forum: false,
        ttl_period: None, // Initial creation TTL
    }).await.map_err(|e| format!("Failed to create channel: {}", e))?;
    
    let (chat_id, access_hash) = match result {
        tl::enums::Updates::Updates(u) => {
             let chat = u.chats.first().ok_or("No chat in updates")?;
             match chat {
                 tl::enums::Chat::Channel(c) => (c.id, c.access_hash.unwrap_or(0)),
                 _ => return Err("Created chat is not a channel".to_string()),
             }
        },
        _ => return Err("Unexpected response (not Updates::Updates)".to_string()), 
    };

    // Explicitly Disable TTL
    // We must use InputChannel for this operation
    let _input_channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
         channel_id: chat_id,
         access_hash,
    });

    let _ = client.invoke(&tl::functions::messages::SetHistoryTtl {
        peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel { channel_id: chat_id, access_hash }),
        period: 0, // 0 = Infinite / Off
    }).await;

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
    let client_opt = {
        state.client.lock().await.clone()
    };
    
    if client_opt.is_none() {
        println!("[MOCK] Deleted folder ID {}", folder_id);
        return Ok(true);
    }
    let client = client_opt.unwrap();
    println!("Deleting folder/channel: {}", folder_id);

    // Resolve peer
    let peer = resolve_peer(&client, Some(folder_id)).await?;
    
    // Grammers doesn't have a high level 'delete_channel'. We use invoke.
    // We need InputChannel.
    let input_channel = match peer {
        Peer::Channel(c) => {
             let chan = &c.raw;
             // Compiler says c.raw is Channel (struct), so we access fields directly.
             // We need to check if 'Channel' type has 'access_hash'. It usually does.
             // If c.raw is ChannelForbidden, this might fail if it's strictly Channel.
             // But if compiler said type is `Channel`, it's likely the struct.
             // We'll try to use it.
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
pub async fn cmd_upload_file(
    path: String,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    // Check size
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    bw_state.can_transfer(size).map_err(|e| e)?;

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        println!("[MOCK] Uploaded file {} to {:?}", path, folder_id);
        bw_state.add_up(size);
        return Ok("Mock upload successful".to_string());
    }
    let client = client_opt.unwrap();
    
    let path_clone = path.clone();
    let client_clone = client.clone();
    
    let uploaded_file = tauri::async_runtime::spawn(async move {
        client_clone.upload_file(&path_clone).await
    }).await.map_err(|e| format!("Task join error: {}", e))?
      .map_err(|e| format!("Failed to upload file bytes: {}", e))?;
        
    let message = InputMessage::new().text("").file(uploaded_file);

    let peer = resolve_peer(&client, folder_id).await?;
    
    client.send_message(&peer, message).await.map_err(|e| e.to_string())?;
    
    bw_state.add_up(size);
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
         println!("[MOCK] Deleted message {} from folder {:?}", message_id, folder_id);
        return Ok(true); 
    }
    let client = client_opt.unwrap();

    let peer = resolve_peer(&client, folder_id).await?;
    client.delete_messages(&peer, &[message_id]).await.map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_download_file(
    message_id: i32,
    save_path: String,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
        println!("[MOCK] Downloaded message {} from {:?} to {}", message_id, folder_id, save_path);
        if let Err(e) = std::fs::write(&save_path, b"Mock Content") { return Err(e.to_string()); }
        return Ok("Download successful".to_string());
    }
    let client = client_opt.unwrap();
    
    let peer = resolve_peer(&client, folder_id).await?;
    let mut msgs = client.iter_messages(&peer);
    
    // Find message
    let mut target_message = None;
    while let Some(m) = msgs.next().await.map_err(|e| e.to_string())? { 
        if m.id() == message_id { target_message = Some(m); break; } 
    }

    if let Some(msg) = target_message {
        if let Some(media) = msg.media() {
            // Check bandwidth
            let size = match &media {
                grammers_client::types::Media::Document(d) => d.size() as u64,
                grammers_client::types::Media::Photo(_) => 1024 * 1024, // Estimate 1MB for photo
                _ => 0,
            };
            
            bw_state.can_transfer(size).map_err(|e| e)?;

            client.download_media(&media, &save_path).await.map_err(|e| e.to_string())?;
            bw_state.add_down(size);
            return Ok("Download successful".to_string());
        }
    }
    Err("Not found".to_string())
}

#[tauri::command]
pub async fn cmd_get_preview(
    message_id: i32,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    
    use base64::{Engine as _, engine::general_purpose};
    
    // Check cache
    // Note: We use the creation logic as before...
    let cache_dir = app_handle.path().app_cache_dir().map_err(|e: tauri::Error| e.to_string())?.join("previews");
    if !cache_dir.exists() { let _ = std::fs::create_dir_all(&cache_dir); }
    
    println!("Preview Request: msg_id={}", message_id);

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
                 grammers_client::types::Media::Document(d) => {
                     let mut e = std::path::Path::new(d.name()).extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                     if e.is_empty() {
                         // Fallback map based on mime
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
                 grammers_client::types::Media::Photo(_) => "jpg".to_string(),
                 _ => "bin".to_string(),
             };
             
             let save_path = cache_dir.join(format!("{}.{}", message_id, ext));
             let save_path_str = save_path.to_string_lossy().to_string();
             
             let mut file_ready = false;
             if save_path.exists() {
                 println!("File exists in cache.");
                 file_ready = true;
             } else {
                 // Download
                 let size = match &media {
                    grammers_client::types::Media::Document(d) => d.size() as u64,
                    grammers_client::types::Media::Photo(_) => 1024 * 1024,
                    _ => 0,
                };
                
                println!("Downloading... Size: {}", size);
                bw_state.can_transfer(size).map_err(|e| e)?;
                 
                match client.download_media(&media, &save_path_str).await {
                    Ok(_) => {
                        println!("Download complete.");
                        bw_state.add_down(size);
                        file_ready = true;
                    },
                    Err(e) => {
                        println!("Download Error: {}", e);
                        return Err(format!("Download failed: {}", e));
                    }
                }
             }

             if file_ready {
                 // Hybrid Return: Base64 for images, Path for others
                 let lower_ext = ext.to_lowercase();
                 if ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"].contains(&lower_ext.as_str()) {
                     println!("Converting image to Base64...");
                     match std::fs::read(&save_path) {
                         Ok(bytes) => {
                             let b64 = general_purpose::STANDARD.encode(&bytes);
                             let mime = match lower_ext.as_str() {
                                 "svg" => "image/svg+xml",
                                 _ => "image/jpeg", // approximate default, browser handles most
                             }; 
                             // Better mime mapping
                             let real_mime = match lower_ext.as_str() {
                                 "png" => "image/png",
                                 "gif" => "image/gif",
                                 "webp" => "image/webp",
                                 "bmp" => "image/bmp",
                                 "svg" => "image/svg+xml",
                                 _ => "image/jpeg",
                             };
                             return Ok(format!("data:{};base64,{}", real_mime, b64));
                         },
                         Err(e) => return Err(format!("Failed to read file for base64: {}", e)),
                     }
                 } else {
                     return Ok(save_path_str);
                 }
             }
        }
    }

    Err("File not found anywhere".to_string())
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

#[tauri::command]
pub async fn cmd_move_file(
    message_id: i32,
    source_folder_id: Option<i64>,
    target_folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    if source_folder_id == target_folder_id { return Ok(true); }
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
        println!("[MOCK] Moved msg {} from {:?} to {:?}", message_id, source_folder_id, target_folder_id);
        return Ok(true); 
    }
    let client = client_opt.unwrap();

    let source_peer = resolve_peer(&client, source_folder_id).await?;
    let target_peer = resolve_peer(&client, target_folder_id).await?;

    // Now call forward logic
    match client.forward_messages(&target_peer, &[message_id], &source_peer).await {
        Ok(_) => {},
        Err(e) => return Err(format!("Forward failed: {}", e)),
    }
    
    match client.delete_messages(&source_peer, &[message_id]).await {
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
        println!("[MOCK] Returning mock files for folder {:?}", folder_id);
        return Ok(get_mock_files(folder_id)); 
    }
    let client = client_opt.unwrap();
    let mut files = Vec::new();
    
    let peer = resolve_peer(&client, folder_id).await?;

    let mut msgs = client.iter_messages(&peer);
    let mut count = 0;
    while let Some(msg) = msgs.next().await.map_err(|e| e.to_string())? {
        if let Some(doc) = msg.media() {
                let (name, size, mime, ext) = match doc {
                    grammers_client::types::Media::Document(d) => {
                            let n = d.name().to_string();
                            let s = d.size() as i64;
                            let m = d.mime_type().map(|s| s.to_string());
                            let e = std::path::Path::new(&n).extension().map(|os| os.to_str().unwrap_or("").to_string());
                            (n, s, m, e)
                    },
                    grammers_client::types::Media::Photo(_) => ("Photo.jpg".to_string(), 0, Some("image/jpeg".into()), Some("jpg".into())),
                    _ => ("Unknown".to_string(), 0, None, None),
                };
                files.push(FileMetadata {
                    id: msg.id() as i64, folder_id, name, size: size as u64, mime_type: mime, file_ext: ext, created_at: msg.date().to_string(), icon_type: "file".into()
                });
                count += 1;
        }
        if count > 100 { break; }
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
    
    // Iterate dialogs
    while let Some(dialog) = dialogs.next().await.map_err(|e| e.to_string())? {
        // Check if it's a channel by matching the peer type directly
        match &dialog.peer {
            Peer::Channel(c) => {
                let id = c.raw.id;
                let name = c.raw.title.clone();

                let input_chan = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: c.raw.id,
                    access_hash: c.raw.access_hash.unwrap_or(0),
                });
                
                // Get full channel info to check "about"
                // This might be slow if many channels, but standard practice
                let full = client.invoke(&tl::functions::channels::GetFullChannel {
                    channel: input_chan,
                }).await;

                if let Ok(tl::enums::messages::ChatFull::Full(f)) = full {
                    match f.full_chat {
                        tl::enums::ChatFull::Full(cf) => {
                             if cf.about.contains("[telegram-drive-folder]") {
                                 folders.push(FolderMetadata {
                                     id,
                                     name,
                                     parent_id: None,
                                 });
                             }
                        }
                        _ => {}
                    }
                }
            },
            _ => {}
        }
    }
    
    Ok(folders)
}
