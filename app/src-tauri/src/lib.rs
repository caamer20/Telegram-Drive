pub mod crypto;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod desktop_lifecycle;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod desktop_notifications;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod desktop_power;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod desktop_preferences;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod desktop_tray;
pub mod installation;
pub mod linux_startup;
pub mod models;
#[cfg(not(target_os = "android"))]
mod network_keepalive;

/// Initialize COM in Multi-Threaded Apartment mode on Windows worker threads.
/// Tauri's main thread uses STA (required for WebView2/DragDrop), so any spawned
/// background threads that touch COM APIs (e.g., Actix, Tokio, networking)
/// must explicitly init COM as MTA to avoid OLE_E_WRONGCOMPOBJ / RPC_E_CHANGED_MODE
/// errors during startup and teardown.
#[cfg(target_os = "windows")]
fn init_com_on_worker_thread() {
    extern "system" {
        fn CoInitializeEx(reserved: *const std::ffi::c_void, coinit: u32) -> i32;
    }
    const COINIT_MULTITHREADED: u32 = 0x0;
    // HRESULT codes
    const S_OK: i32 = 0;
    const S_FALSE: i32 = 1;
    const RPC_E_CHANGED_MODE: i32 = -2147417850; // 0x80010106

    let hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED) };
    match hr {
        S_OK | S_FALSE => {
            log::info!(
                "COM MTA initialized on worker thread (hr=0x{:x})",
                hr as u32
            );
        }
        RPC_E_CHANGED_MODE => {
            // Thread was already initialized with a different apartment model.
            // This is non-fatal; the existing mode will be used.
            log::warn!(
                "COM already initialized in a different mode on this worker thread (hr=0x{:x})",
                hr as u32
            );
        }
        _ => {
            log::error!(
                "Failed to initialize COM on worker thread (hr=0x{:x})",
                hr as u32
            );
        }
    }
}

pub mod bandwidth;
pub mod commands;
pub mod proxy_secret;
pub mod socks5_bridge;
pub mod temp_artifacts;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod transfer_engine;
pub mod vpn_optimizer;

use tauri::{Emitter, Manager};

use commands::streaming::StreamConfig;
use commands::TelegramState;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod android_security;
pub mod android_updates;
pub mod api_routes;
pub mod api_secret;
pub mod crypto_commands;
pub mod db;
mod db_migrations;
pub mod fmp4_remux;
pub mod jni_cache;
mod local_cors;
pub mod mp4_utils;
pub mod server;
mod server_lifecycle;
pub mod share_routes;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod sponsor_link_bridge;
pub mod sync_engine;
pub mod transcode;
pub mod upload_service;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod webdav;
pub mod workspace;

/// Single source of truth for the Actix streaming server port.
/// Referenced in lib.rs (server startup) and exposed to the frontend
/// via cmd_get_stream_info so no component ever hardcodes the port.
pub const STREAM_PORT: u16 = 14201;

/// Generate a random 32-character hex token for streaming server auth
fn generate_stream_token() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.random()).collect();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Holds the Actix-web server stop handle so we can shut it down
/// from the RunEvent::Exit handler for graceful Ctrl+C termination.
pub struct ActixServerHandle(pub Arc<std::sync::Mutex<Option<actix_web::dev::ServerHandle>>>);

/// Serialized lifecycle for the restartable desktop REST API server.
pub struct ApiServerLifecycle(pub Arc<server_lifecycle::LocalServerLifecycle>);

/// Serialized lifecycle for the restartable desktop WebDAV server.
pub struct WebDavServerLifecycle(pub Arc<server_lifecycle::LocalServerLifecycle>);

/// Restart (or stop) the API server based on current settings.
/// Called from Tauri commands when the user changes API settings.
#[cfg(not(target_os = "android"))]
pub async fn restart_api_server(app: &tauri::AppHandle) -> Result<(), String> {
    let lifecycle = app.state::<ApiServerLifecycle>().0.clone();
    let generation = lifecycle.request_restart();
    let _operation = lifecycle.lock_operation().await;
    if !lifecycle.is_current(generation) {
        return Ok(());
    }

    let old_handle = lifecycle.take_handle();
    if let Some(handle) = old_handle {
        log::info!("Stopping existing API server...");
        handle.stop(true).await;
    }
    if !lifecycle.is_current(generation) {
        return Ok(());
    }

    let settings = commands::api_settings::load_settings(app);
    if !settings.enabled {
        log::info!("API server disabled");
        return Ok(());
    }

    // Need TelegramState to share with the API server
    let tg_state = Arc::new(app.state::<TelegramState>().inner().clone());
    let bw_manager = app
        .state::<Arc<bandwidth::BandwidthManager>>()
        .inner()
        .clone();
    let net_config = app
        .state::<Arc<vpn_optimizer::NetworkConfig>>()
        .inner()
        .clone();
    let db_pool = app.state::<db::DbConnection>().inner().clone();
    let api_port = settings.port;
    let api_account_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let key_hash = settings.key_hash.clone();
    let lifecycle_for_thread = lifecycle.clone();

    // Resolve cache dirs before the thread spawn since app is a reference
    let preview_dir = app
        .path()
        .app_cache_dir()
        .unwrap_or_default()
        .join("previews");
    let thumbnail_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_default()
        .join("thumbnails");

    let (startup_sender, startup_receiver) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        init_com_on_worker_thread();
        let sys = actix_rt::System::new();
        sys.block_on(async move {
            let mut startup_sender = Some(startup_sender);
            let api_state_data = actix_web::web::Data::new(tg_state);
            let api_state = actix_web::web::Data::new(api_routes::ApiState { key_hash });
            let cache_dirs = actix_web::web::Data::new(api_routes::CacheDirs {
                account_root: api_account_root,
                thumbnail_dir,
                preview_dir,
            });
            let api_bw = actix_web::web::Data::new(bw_manager);
            let api_net = actix_web::web::Data::new(net_config);
            let api_db = actix_web::web::Data::new(db_pool);

            log::info!("Starting REST API server on port {}", api_port);
            let listener = match server_lifecycle::bind_loopback_with_retry(
                api_port,
                &lifecycle_for_thread,
                generation,
            )
            .await
            {
                Ok(Some(listener)) => listener,
                Ok(None) => {
                    let _ = startup_sender.take().unwrap().send(Ok(()));
                    return;
                }
                Err(error) => {
                    let message = format!("Could not start REST API on port {api_port}: {error}");
                    log::error!("{message}");
                    lifecycle_for_thread.set_error(generation, message.clone());
                    let _ = startup_sender.take().unwrap().send(Err(message));
                    return;
                }
            };

            let server = match actix_web::HttpServer::new(move || {
                let cors = actix_cors::Cors::default()
                    .allowed_origin_fn(|origin, _req_head| {
                        local_cors::is_allowed_origin_header(origin)
                    })
                    .allow_any_method()
                    .allow_any_header();

                actix_web::App::new()
                    .wrap(cors)
                    .app_data(api_state_data.clone())
                    .app_data(api_state.clone())
                    .app_data(cache_dirs.clone())
                    .app_data(api_bw.clone())
                    .app_data(api_net.clone())
                    .app_data(api_db.clone())
                    .configure(api_routes::configure_api)
            })
            .listen(listener)
            {
                Ok(bound) => bound.run(),
                Err(error) => {
                    let message = format!("Could not start REST API on port {api_port}: {error}");
                    log::error!("{message}");
                    lifecycle_for_thread.set_error(generation, message.clone());
                    let _ = startup_sender.take().unwrap().send(Err(message));
                    return;
                }
            };
            if !lifecycle_for_thread.install_handle(generation, server.handle()) {
                server.handle().stop(false).await;
                let _ = startup_sender.take().unwrap().send(Ok(()));
                let _ = server.await;
                return;
            }
            log::info!("REST API server started on http://127.0.0.1:{}", api_port);
            let _ = startup_sender.take().unwrap().send(Ok(()));
            let result = server.await;
            let error = result
                .err()
                .map(|value| format!("REST API server stopped: {value}"));
            lifecycle_for_thread.server_finished(generation, error);
        });
    });

    match startup_receiver.await {
        Ok(result) => result,
        Err(_) => {
            let message = "REST API startup task ended unexpectedly".to_string();
            lifecycle.set_error(generation, message.clone());
            Err(message)
        }
    }
}

/// Restart (or stop) the API server based on current settings.
/// Called from Tauri commands when the user changes API settings.
#[cfg(target_os = "android")]
pub async fn restart_api_server(_app: &tauri::AppHandle) -> Result<(), String> {
    log::info!("REST API disabled on mobile.");
    Ok(())
}

/// Restart (or stop) the loopback-only WebDAV server using persisted settings.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn restart_webdav_server(app: &tauri::AppHandle) -> Result<(), String> {
    let lifecycle = app.state::<WebDavServerLifecycle>().0.clone();
    let generation = lifecycle.request_restart();
    let _operation = lifecycle.lock_operation().await;
    if !lifecycle.is_current(generation) {
        return Ok(());
    }

    let old_handle = lifecycle.take_handle();
    if let Some(handle) = old_handle {
        log::info!("Stopping existing WebDAV server...");
        handle.stop(true).await;
    }
    if !lifecycle.is_current(generation) {
        return Ok(());
    }

    let settings = commands::webdav_settings::load_settings(app);
    if !settings.enabled {
        log::info!("WebDAV server disabled");
        return Ok(());
    }
    let Some(token_hash) = settings.token_hash else {
        let message = "Generate a WebDAV connection link before enabling the server".to_string();
        lifecycle.set_error(generation, message.clone());
        return Err(message);
    };

    let telegram_state = Arc::new(app.state::<TelegramState>().inner().clone());
    let bandwidth = app
        .state::<Arc<bandwidth::BandwidthManager>>()
        .inner()
        .clone();
    let network = app
        .state::<Arc<vpn_optimizer::NetworkConfig>>()
        .inner()
        .clone();
    let database = app.state::<db::DbConnection>().inner().clone();
    let webdav_account_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let staging_dir = match app.path().app_cache_dir() {
        Ok(path) => path.join("webdav-staging"),
        Err(error) => {
            let message = format!("Could not resolve the WebDAV cache directory: {error}");
            lifecycle.set_error(generation, message.clone());
            return Err(message);
        }
    };
    let port = settings.port;
    let write_enabled = settings.write_enabled;
    let lifecycle_for_thread = lifecycle.clone();

    let (startup_sender, startup_receiver) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        init_com_on_worker_thread();
        let system = actix_rt::System::new();
        system.block_on(async move {
            let mut startup_sender = Some(startup_sender);
            if let Err(error) = tokio::fs::create_dir_all(&staging_dir).await {
                let message = format!("Could not create the WebDAV staging directory: {error}");
                log::error!("{message}");
                lifecycle_for_thread.set_error(generation, message.clone());
                let _ = startup_sender.take().unwrap().send(Err(message));
                return;
            }
            if let Ok(mut entries) = tokio::fs::read_dir(&staging_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let is_staged_upload = entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("webdav-"));
                    if is_staged_upload {
                        let _ = tokio::fs::remove_file(entry.path()).await;
                    }
                }
            }

            let filesystem = webdav::TelegramDavFs::new(
                telegram_state,
                bandwidth,
                network,
                database,
                write_enabled,
                staging_dir,
                webdav_account_root,
            );
            let (handler, auth) = webdav::build_handler(filesystem, token_hash);
            let handler = actix_web::web::Data::new(handler);
            let auth = actix_web::web::Data::new(auth);

            log::info!("Starting WebDAV server on 127.0.0.1:{port}");
            let listener = match server_lifecycle::bind_loopback_with_retry(
                port,
                &lifecycle_for_thread,
                generation,
            )
            .await
            {
                Ok(Some(listener)) => listener,
                Ok(None) => {
                    let _ = startup_sender.take().unwrap().send(Ok(()));
                    return;
                }
                Err(error) => {
                    let message = format!("Could not start WebDAV on port {port}: {error}");
                    log::error!("{message}");
                    lifecycle_for_thread.set_error(generation, message.clone());
                    let _ = startup_sender.take().unwrap().send(Err(message));
                    return;
                }
            };

            let server = match actix_web::HttpServer::new(move || {
                actix_web::App::new()
                    .app_data(handler.clone())
                    .app_data(auth.clone())
                    .service(actix_web::web::resource("/{tail:.*}").to(webdav::webdav_handler))
            })
            .listen(listener)
            {
                Ok(bound) => bound.run(),
                Err(error) => {
                    let message = format!("Could not start WebDAV on port {port}: {error}");
                    log::error!("{message}");
                    lifecycle_for_thread.set_error(generation, message.clone());
                    let _ = startup_sender.take().unwrap().send(Err(message));
                    return;
                }
            };
            if !lifecycle_for_thread.install_handle(generation, server.handle()) {
                server.handle().stop(false).await;
                let _ = startup_sender.take().unwrap().send(Ok(()));
                let _ = server.await;
                return;
            }
            log::info!("WebDAV server started on http://127.0.0.1:{port}");
            let _ = startup_sender.take().unwrap().send(Ok(()));
            let result = server.await;
            let error = result
                .err()
                .map(|value| format!("WebDAV server stopped: {value}"));
            lifecycle_for_thread.server_finished(generation, error);
        });
    });

    match startup_receiver.await {
        Ok(result) => result,
        Err(_) => {
            let message = "WebDAV startup task ended unexpectedly".to_string();
            lifecycle.set_error(generation, message.clone());
            Err(message)
        }
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn restart_webdav_server(_app: &tauri::AppHandle) -> Result<(), String> {
    log::info!("WebDAV hosting is disabled on mobile platforms.");
    Ok(())
}

#[tauri::command]
fn cmd_open_file_externally(path: String, _app_handle: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("Failed to resolve JVM: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {}", e))?;

        if let Some(main_class) = crate::jni_cache::get_main_activity_jclass() {
            let path_jstr = env
                .new_string(&path)
                .map_err(|e| format!("Failed to create path JString: {}", e))?;

            let lower_ext = std::path::Path::new(&path)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();

            let mime_type = match lower_ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "pdf" => "application/pdf",
                "mp4" => "video/mp4",
                "m4v" => "video/x-m4v",
                "mkv" => "video/x-matroska",
                "mov" => "video/quicktime",
                "webm" => "video/webm",
                "avi" => "video/x-msvideo",
                "mp3" => "audio/mpeg",
                "m4a" => "audio/mp4",
                "aac" => "audio/aac",
                "flac" => "audio/flac",
                "ogg" | "oga" => "audio/ogg",
                "opus" => "audio/opus",
                "wav" => "audio/wav",
                "txt" => "text/plain",
                "zip" => "application/zip",
                _ => "application/octet-stream",
            };

            let mime_jstr = env
                .new_string(mime_type)
                .map_err(|e| format!("Failed to create mime JString: {}", e))?;

            let success = env
                .call_static_method(
                    &main_class,
                    "openFileExternally",
                    "(Ljava/lang/String;Ljava/lang/String;)Z",
                    &[
                        jni::objects::JValue::from(&path_jstr),
                        jni::objects::JValue::from(&mime_jstr),
                    ],
                )
                .map_err(|e| {
                    format!("Failed to call static JNI method openFileExternally: {}", e)
                })?;

            let success_bool = success
                .z()
                .map_err(|e| format!("Failed to parse boolean result: {}", e))?;
            if !success_bool {
                return Err("Failed to launch intent from Kotlin".to_string());
            }
            Ok(())
        } else {
            Err("MainActivity reference is not cached in JNI cache".to_string())
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_opener::OpenerExt;
        _app_handle
            .opener()
            .open_path(&path, None::<&str>)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn cmd_open_android_stream_player(
    stream_url: String,
    title: String,
    mime_type: String,
    media_id: String,
    preferences_json: String,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        if !stream_url.starts_with("http://localhost:") {
            return Err(
                "Android media streams must use the authenticated localhost server".to_string(),
            );
        }
        if media_id.is_empty() || media_id.len() > 128 {
            return Err("Android media ID is invalid".to_string());
        }
        let preferences: serde_json::Value = serde_json::from_str(&preferences_json)
            .map_err(|_| "Android media preferences are invalid".to_string())?;
        if !preferences.is_object() || preferences_json.len() > 2_048 {
            return Err("Android media preferences are invalid".to_string());
        }
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("Failed to resolve JVM: {e}"))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("Failed to attach thread: {e}"))?;
        let main_class = crate::jni_cache::get_main_activity_jclass()
            .ok_or_else(|| "MainActivity reference is not cached in JNI cache".to_string())?;
        let url = env.new_string(stream_url).map_err(|e| e.to_string())?;
        let title = env.new_string(title).map_err(|e| e.to_string())?;
        let mime = env.new_string(mime_type).map_err(|e| e.to_string())?;
        let media_id = env.new_string(media_id).map_err(|e| e.to_string())?;
        let preferences = env
            .new_string(preferences_json)
            .map_err(|e| e.to_string())?;
        let result = env.call_static_method(
            &main_class,
            "openMediaStream",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z",
            &[
                jni::objects::JValue::from(&url),
                jni::objects::JValue::from(&title),
                jni::objects::JValue::from(&mime),
                jni::objects::JValue::from(&media_id),
                jni::objects::JValue::from(&preferences),
            ],
        ).map_err(|e| format!("Failed to open the Android media player: {e}"))?;
        if result.z().map_err(|e| e.to_string())? {
            Ok(())
        } else {
            Err("Android rejected the media stream".to_string())
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (stream_url, title, mime_type, media_id, preferences_json);
        Err("The native stream player is only available on Android".to_string())
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidPlaybackHistoryEntry {
    media_id: String,
    title: String,
    position_ms: u64,
    duration_ms: u64,
    completed: bool,
    last_played_at: u64,
}

#[tauri::command]
fn cmd_get_android_playback_history() -> Result<Vec<AndroidPlaybackHistoryEntry>, String> {
    #[cfg(target_os = "android")]
    {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|error| format!("Failed to resolve JVM: {error}"))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|error| format!("Failed to attach playback history: {error}"))?;
        let main_class = crate::jni_cache::get_main_activity_jclass()
            .ok_or("MainActivity reference not cached")?;
        let value = env
            .call_static_method(
                &main_class,
                "getPlaybackHistory",
                "()Ljava/lang/String;",
                &[],
            )
            .map_err(|error| format!("Failed to read playback history: {error}"))?;
        let value = jni::objects::JString::from(value.l().map_err(|error| error.to_string())?);
        let json: String = env
            .get_string(&value)
            .map_err(|error| format!("Failed to decode playback history: {error}"))?
            .into();
        return serde_json::from_str(&json)
            .map_err(|error| format!("Failed to parse playback history: {error}"));
    }
    #[cfg(not(target_os = "android"))]
    Ok(Vec::new())
}

/// Called by the frontend on mount (Android only) to check whether files were
/// shared into the app via Android's share sheet before the webview was ready
/// (cold start). Returns the count of pending shared files and resets the counter.
#[cfg(target_os = "android")]
#[tauri::command]
fn cmd_get_pending_share_count() -> Result<i32, String> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|e| format!("Failed to resolve JVM: {}", e))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("Failed to attach thread: {}", e))?;

    if let Some(main_class) = crate::jni_cache::get_main_activity_jclass() {
        let count = env
            .call_static_method(&main_class, "getAndClearShareCount", "()I", &[])
            .map_err(|e| format!("Failed to call getAndClearShareCount: {}", e))?;
        let count_int = count
            .i()
            .map_err(|e| format!("Failed to parse share count: {}", e))?;
        Ok(count_int)
    } else {
        Err("MainActivity reference not cached".to_string())
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn cmd_get_pending_share_count() -> Result<i32, String> {
    Ok(0) // Share intents are Android-only
}

#[cfg(target_os = "android")]
#[tauri::command]
fn cmd_get_pending_android_transfer_action() -> Result<String, String> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|error| format!("Failed to resolve JVM: {error}"))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|error| format!("Failed to attach transfer-action check: {error}"))?;
    let main_class =
        crate::jni_cache::get_main_activity_jclass().ok_or("MainActivity reference not cached")?;
    let value = env
        .call_static_method(
            &main_class,
            "getAndClearPendingTransferAction",
            "()Ljava/lang/String;",
            &[],
        )
        .map_err(|error| format!("Failed to read the pending transfer action: {error}"))?;
    let value = jni::objects::JString::from(value.l().map_err(|error| error.to_string())?);
    env.get_string(&value)
        .map(|result| result.into())
        .map_err(|error| format!("Failed to decode the pending transfer action: {error}"))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn cmd_get_pending_android_transfer_action() -> Result<String, String> {
    Ok(String::new())
}

/// Returns a list of files that were shared into the app via Android's share sheet
/// and are currently cached in uriCacheMap, ready for upload.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedFileEntry {
    uri: String,
    cached_path: String,
    file_name: String,
    file_size: u64,
}

#[cfg(target_os = "android")]
#[tauri::command]
fn cmd_list_cached_files() -> Result<Vec<CachedFileEntry>, String> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|e| format!("Failed to resolve JVM: {}", e))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("Failed to attach thread: {}", e))?;

    if let Some(main_class) = crate::jni_cache::get_main_activity_jclass() {
        let json_val = env
            .call_static_method(&main_class, "listCachedFiles", "()Ljava/lang/String;", &[])
            .map_err(|e| format!("Failed to call listCachedFiles: {}", e))?;

        let json_jstr: jni::objects::JString = json_val
            .l()
            .map_err(|e| format!("listCachedFiles result is not a string: {}", e))?
            .into();
        let json_str: String = env
            .get_string(&json_jstr)
            .map_err(|e| format!("Failed to read listCachedFiles result: {}", e))?
            .into();

        let entries: Vec<CachedFileEntry> = serde_json::from_str(&json_str)
            .map_err(|e| format!("Failed to parse cached files JSON: {}", e))?;
        Ok(entries)
    } else {
        Err("MainActivity reference not cached".to_string())
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn cmd_list_cached_files() -> Result<Vec<CachedFileEntry>, String> {
    Ok(Vec::new()) // Share cache is Android-only
}

/// Removes a single cached file entry from the Kotlin uriCacheMap.
/// Called by the frontend when the user clears shared files.
#[cfg(target_os = "android")]
#[tauri::command]
fn cmd_remove_cached_path(uri: String) -> Result<(), String> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|e| format!("Failed to resolve JVM: {}", e))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("Failed to attach thread: {}", e))?;

    if let Some(main_class) = crate::jni_cache::get_main_activity_jclass() {
        let j_uri = env
            .new_string(&uri)
            .map_err(|e| format!("Failed to create URI string: {}", e))?;
        env.call_static_method(
            &main_class,
            "removeCachedPath",
            "(Ljava/lang/String;)V",
            &[jni::objects::JValue::from(&j_uri)],
        )
        .map_err(|e| format!("Failed to call removeCachedPath: {}", e))?;
        let _ = env.exception_clear();
        Ok(())
    } else {
        Err("MainActivity reference not cached".to_string())
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
fn cmd_remove_cached_path(_uri: String) -> Result<(), String> {
    Ok(()) // No-op on desktop
}

/// Gather system diagnostics and environment info for debugging.
/// Returns a formatted string suitable for copying to clipboard.
#[tauri::command]
fn cmd_get_system_diagnostics(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        let context = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
            .map_err(|error| format!("Unable to access Android diagnostics: {error}"))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|error| format!("Unable to attach Android diagnostics: {error}"))?;
        let main_class = jni_cache::get_main_activity_jclass()
            .ok_or("Android diagnostics are still initializing")?;
        let value = env
            .call_static_method(
                &main_class,
                "getSystemDiagnosticsJson",
                "()Ljava/lang/String;",
                &[],
            )
            .map_err(|error| format!("Unable to collect Android diagnostics: {error}"))?
            .l()
            .map_err(|error| format!("Android diagnostics returned an invalid value: {error}"))?;
        let json: String = env
            .get_string(&jni::objects::JString::from(value))
            .map_err(|error| format!("Unable to read Android diagnostics: {error}"))?
            .into();
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .map_err(|_| "Android diagnostics returned malformed data".to_string())?;
        let pretty = serde_json::to_string_pretty(&parsed)
            .map_err(|error| format!("Unable to format Android diagnostics: {error}"))?;
        return Ok(format!(
            "=== Telegram Drive Diagnostics ===\n{pretty}\n=================================="
        ));
    }

    #[cfg(not(target_os = "android"))]
    {
        let mut lines: Vec<String> = Vec::new();

        lines.push("=== Telegram Drive Diagnostics ===".into());
        lines.push(format!("Package: {}", env!("CARGO_PKG_NAME")));
        lines.push(format!("Version: {}", env!("CARGO_PKG_VERSION")));

        // OS info
        lines.push(format!(
            "OS: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));

        #[cfg(target_os = "linux")]
        {
            let installation = installation::current_installation_info();
            let package_type = if installation.managed_by_package_manager {
                "pacman package"
            } else if std::env::var_os("APPIMAGE").is_some() || std::env::var_os("APPDIR").is_some()
            {
                "AppImage"
            } else {
                "native/unknown"
            };
            lines.push(format!("Package Type: {package_type}"));
            lines.push(format!(
                "XDG_SESSION_TYPE: {}",
                std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into())
            ));
            lines.push(format!(
                "XDG_CURRENT_DESKTOP: {}",
                std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into())
            ));
            for name in [
                "GDK_BACKEND",
                "EGL_PLATFORM",
                "WEBKIT_DMABUF_RENDERER_FORCE_SHM",
                "WEBKIT_DISABLE_DMABUF_RENDERER",
                "WEBKIT_DISABLE_COMPOSITING_MODE",
                "TELEGRAM_DRIVE_SAFE_RENDERING",
            ] {
                lines.push(format!(
                    "{name}: {}",
                    std::env::var(name).unwrap_or_else(|_| "unset".into())
                ));
            }
        }

        #[cfg(target_os = "macos")]
        {
            lines.push("Package Type: macOS bundle".to_string());
        }

        #[cfg(target_os = "windows")]
        {
            lines.push("Package Type: Windows installer".to_string());
        }

        // App data dir
        if let Ok(dir) = app.path().app_data_dir() {
            lines.push(format!("App Data: {}", dir.display()));
        }

        // Check for FFmpeg
        #[cfg(unix)]
        {
            let which = std::process::Command::new("which")
                .arg("ffmpeg")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            lines.push(format!(
                "FFmpeg: {}",
                which.unwrap_or_else(|| "not found".into())
            ));
        }

        lines.push("==================================".into());

        Ok(lines.join("\n"))
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let stream_token = generate_stream_token();

    // Shared handle for stopping the Actix streaming server during shutdown
    let server_handle: Arc<std::sync::Mutex<Option<actix_web::dev::ServerHandle>>> =
        Arc::new(std::sync::Mutex::new(None));
    let server_handle_for_setup = server_handle.clone();

    let builder = tauri::Builder::default();

    // This must be the first desktop plugin so a secondary process cannot
    // initialize its own transfer engine before it is redirected here.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Err(error) = desktop_lifecycle::show_main_window(
            app,
            desktop_lifecycle::DesktopNavigationRequest::home(),
        ) {
            log::warn!("Could not restore the existing application instance: {error}");
        }
    }));

    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(sponsor_link_bridge::init());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_notification::init());

    // The updater plugin is not supported on Android and can cause crashes
    // (APKs are managed by the Play Store; the plugin attempts restricted FS ops).
    #[cfg(not(target_os = "android"))]
    let builder = if installation::is_packaged_windows_app() {
        // MSIX updates belong to the Store. Do not expose the standalone
        // installer path even if a frontend caller tries to invoke it.
        builder
    } else {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    };

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());

    let app = builder
        .setup(move |app| {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let preferences = desktop_preferences::DesktopPreferencesState::load(app.handle())
                    .map_err(|error| {
                        log::error!("Failed to initialize desktop preferences: {error}");
                        error
                    })?;
                app.manage(preferences);
                app.manage(desktop_lifecycle::DesktopLifecycleState::default());

                match desktop_tray::initialize(app.handle()) {
                    Ok(tray) => {
                        app.manage(tray);
                    }
                    Err(error) => {
                        // Close-to-background remains disabled because tray_ready
                        // was never set. The visible app is still fully usable.
                        log::error!("System tray is unavailable; background close is disabled: {error}");
                    }
                }

                let notifications =
                    desktop_notifications::DesktopNotificationCoordinator::initialize(app.handle())
                        .map_err(|error| {
                            log::error!("Failed to initialize desktop notifications: {error}");
                            error
                        })?;
                app.manage(notifications);
            }

            #[cfg(target_os = "android")]
            {
                // SAFETY NET: Wrap all Android JNI initialization in catch_unwind to prevent
                // any Rust panic from crossing the JNI/FFI boundary and SIGABRTing the process.
                let jni_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // In Tauri v2, Tauri does not use or initialize the legacy `ndk-context` crate.
                    // However, external crates like `reqwest` still require `ndk-context` to access
                    // JNI handles (e.g. system proxy settings) on Android background threads.
                    //
                    // CRITICAL: `with_webview` dispatches its callback asynchronously onto the
                    // WebView thread. We perform ALL JNI work (ndk-context init, ClassLoader
                    // caching, MainActivity caching) inside this single callback so there is no
                    // race between the init and subsequent usage.
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.with_webview(|webview| {
                            webview.jni_handle().exec(|env, context, _webview| {
                                // 1. Initialize ndk-context with the JVM and Activity pointers
                                if let Ok(vm) = env.get_java_vm() {
                                    unsafe {
                                        let _ = ndk_context::initialize_android_context(
                                            vm.get_java_vm_pointer().cast(),
                                            context.as_raw().cast(),
                                        );
                                    }
                                    log::info!("JNI: Successfully initialized ndk-context globally.");
                                } else {
                                    log::error!("JNI: Failed to get JavaVM from JNIEnv");
                                    return;
                                }

                                // 2. Cache ClassLoader and MainActivity class references
                                //    Using the same JNI env avoids the race condition where
                                //    ndk_context::android_context() was called before init completed.
                                if let Ok(class_loader_val) = env.call_method(
                                    &context,
                                    "getClassLoader",
                                    "()Ljava/lang/ClassLoader;",
                                    &[],
                                ) {
                                    if let Ok(class_loader_obj) = class_loader_val.l() {
                                        if let Ok(class_loader_global) = env.new_global_ref(&class_loader_obj) {
                                            let _ = crate::jni_cache::set_class_loader(class_loader_global);
                                        }

                                        let class_name_jstr = match env.new_string("com.cameronamer.telegramdrive.MainActivity") {
                                            Ok(s) => Some(s),
                                            Err(e) => {
                                                log::error!("JNI: Failed to create MainActivity class name string: {}", e);
                                                None
                                            }
                                        };
                                        if let Some(class_name_jstr) = class_name_jstr {
                                            if let Ok(main_class_obj_val) = env.call_method(
                                                &class_loader_obj,
                                                "loadClass",
                                                "(Ljava/lang/String;)Ljava/lang/Class;",
                                                &[jni::objects::JValue::from(&class_name_jstr)],
                                            ) {
                                                if let Ok(main_class_obj) = main_class_obj_val.l() {
                                                    if let Ok(main_class_global) = env.new_global_ref(main_class_obj) {
                                                        let _ = crate::jni_cache::set_main_activity_class(main_class_global);
                                                        log::info!("JNI: Successfully cached MainActivity class reference globally.");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                        });
                    }
                }));
                if let Err(e) = jni_result {
                    log::error!("JNI: Android initialization panicked (caught): {:?}", e);
                }
            }

            app.manage(TelegramState {
                client: Arc::new(Mutex::new(None)),
                session: Arc::new(Mutex::new(None)),
                phone_login: Arc::new(Mutex::new(None)),
                password_token: Arc::new(Mutex::new(None)),
                api_id: Arc::new(Mutex::new(None)),
                auth_attempt_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                runner_shutdown: Arc::new(std::sync::Mutex::new(None)),
                runner_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                peer_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
                active_file_loads: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
                cancelled_transfers: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
            });
            app.manage(Arc::new(bandwidth::BandwidthManager::new(app.handle())));
            app.manage(StreamConfig { token: stream_token.clone(), port: STREAM_PORT });

            // Initialize the passphrase-protected persistent production vault.
            // Test-only MemoryVault must never be constructed by the app.
            let crypto_data_dir = app.path().app_data_dir().map_err(|e| {
                log::error!("Failed to get app data dir for encryption vault: {}", e);
                e
            })?;
            let crypto_vault_path = crypto_data_dir.join("encryption").join("vault.v2");
            let crypto_vault = Box::new(crypto::vault::file::FileVault::new(crypto_vault_path));
            let crypto_state = crypto::state::CryptoState::new(crypto_vault);
            let crypto_state_for_auto_lock = crypto_state.clone();
            app.manage(crypto_state);
            let crypto_app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    crypto_state_for_auto_lock.wait_until_auto_locked().await;
                    let _ = crypto_app_handle.emit("vault-locked", "auto_lock");
                }
            });

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let power_monitor = desktop_power::DesktopPowerMonitor::new();
                power_monitor.start(app.handle().clone());
                app.manage(power_monitor);
            }

            app.manage(ActixServerHandle(server_handle_for_setup.clone()));
            app.manage(ApiServerLifecycle(server_lifecycle::LocalServerLifecycle::new()));
            app.manage(WebDavServerLifecycle(server_lifecycle::LocalServerLifecycle::new()));

            // Initialize TranscodeManager for HLS streaming
            let app_data_dir = app.path().app_data_dir().map_err(|e| {
                log::error!("Failed to get app data dir: {}", e);
                e
            })?;
            let cache_root = app_data_dir.join("streaming");
            let cache_limit = transcode::persisted_cache_limit_bytes(&app_data_dir);
            let transcode_manager =
                transcode::TranscodeManager::new_with_max_cache_bytes(cache_root, cache_limit);
            // Detect FFmpeg (non-blocking spawn)
            let app_handle = app.handle().clone();
            let ffmpeg_path_arc = transcode_manager.ffmpeg_path.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(ffmpeg) = transcode::detect_ffmpeg(&app_handle).await {
                    *ffmpeg_path_arc.lock().await = Some(ffmpeg);
                }
            });
            let transcode_arc = Arc::new(transcode_manager);
            transcode_arc.start_cache_reconciliation(true);
            app.manage(transcode_arc.clone());
            app.manage(fmp4_remux::Fmp4RemuxState::new());
            let loaded_config = vpn_optimizer::load_network_config(app.handle());
            let net_config = Arc::new(vpn_optimizer::NetworkConfig::new_with_config(loaded_config));
            app.manage(net_config.clone());

            // Auto-start SOCKS5 bridge on startup if HTTP/HTTPS proxy is configured
            {
                let start_config = net_config.clone();
                tauri::async_runtime::spawn(async move {
                    let (enabled, is_http_or_https) = {
                        let proxy = start_config.proxy.read().unwrap();
                        (proxy.enabled, proxy.proxy_type == "http" || proxy.proxy_type == "https")
                    };
                    if enabled && is_http_or_https {
                        if let Err(e) = start_config.start_http_bridge().await {
                            log::error!("Failed to auto-start SOCKS5 bridge on startup: {}", e);
                        }
                    }
                });
            }

            // Initialize SQLite Database
            let db_pool = db::init_db(app.handle()).map_err(|e| {
                log::error!("Failed to initialize SQLite database: {}", e);
                e
            })?;
            app.manage(db_pool.clone());
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let transfer_engine = transfer_engine::TransferEngine::initialize(app.handle().clone())
                    .map_err(|error| {
                        log::error!("Failed to initialize durable transfer engine: {error}");
                        error
                    })?;
                let initial_jobs = transfer_engine.startup_snapshot();
                app.manage(transfer_engine.clone());
                if let Some(notifications) = app.try_state::<
                    Arc<desktop_notifications::DesktopNotificationCoordinator>,
                >() {
                    notifications.seed(initial_jobs);
                    notifications.start();
                }
                transfer_engine.start();
            }
            let sync_engine = sync_engine::SyncEngine::new(db_pool.clone(), app.handle().clone());
            app.manage(sync_engine);
            let app_for_sync = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = app_for_sync.state::<sync_engine::SyncEngine>().start().await {
                    log::error!("Failed to initialize folder sync engine: {error}");
                }
            });

            // Actix Web's Server future runs on Tokio without requiring a separate
            // actix_rt::System. Sharing Tauri's runtime avoids nested-runtime
            // panics on Android while preserving the same loopback server and
            // route graph on desktop.
            let state = Arc::new(app.state::<TelegramState>().inner().clone());
            let token_for_server = stream_token.clone();
            let handle_for_runtime = server_handle_for_setup.clone();
            let db_pool_for_server = db_pool.clone();
            let transcode_for_server = transcode_arc.clone();
            let crypto_for_server = app.state::<crypto::state::CryptoState>().inner().clone();
            let share_account_root = app.path().app_data_dir()?;
            tauri::async_runtime::spawn(async move {
                match server::start_server(
                    state,
                    STREAM_PORT,
                    token_for_server,
                    db_pool_for_server,
                    transcode_for_server,
                    crypto_for_server,
                    share_account_root,
                )
                .await
                {
                    Ok(server) => {
                        let server_handle = server.handle();
                        let handle_was_stored = match handle_for_runtime.lock() {
                            Ok(mut handle) => {
                                *handle = Some(server_handle);
                                true
                            }
                            Err(error) => {
                                log::error!("Could not retain streaming server handle: {error}");
                                false
                            }
                        };
                        if !handle_was_stored {
                            server.handle().stop(false).await;
                            return;
                        }

                        if let Err(error) = server.await {
                            log::error!("Streaming server stopped with an error: {error}");
                        } else {
                            log::info!("Streaming server stopped.");
                        }
                        if let Ok(mut handle) = handle_for_runtime.lock() {
                            *handle = None;
                        }
                    }
                    Err(error) => log::error!("Streaming server failed to start: {error}"),
                }
            });

            // Start independently configured local servers without blocking setup.
            let api_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = restart_api_server(&api_app).await {
                    log::error!("REST API startup failed: {error}");
                }
            });
            let webdav_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = restart_webdav_server(&webdav_app).await {
                    log::error!("WebDAV startup failed: {error}");
                }
            });

            // Start VPN keep-alive background task
            // Disabled on Android: unnecessary on mobile and spawn_blocking may
            // conflict with the platform's background execution limits.
            #[cfg(not(target_os = "android"))]
            {
                let ka_config = net_config.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        let interval = ka_config.keep_alive_interval_sec();
                        if interval == 0 {
                            // Disabled — check again in 10s
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                            continue;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(interval as u64)).await;
                        // Resolve Telegram dynamically so keep-alive follows DNS
                        // changes and is not coupled to one datacenter address.
                        let _ = network_keepalive::probe_telegram().await;
                    }
                });
            }

            workspace::start_background(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace::storage::cmd_storage_read,
            workspace::storage::cmd_storage_limits,
            workspace::storage::cmd_storage_clear,
            workspace::cmd_workspace_account,
            workspace::cmd_workspace_read,
            workspace::cmd_workspace_mutate,
            workspace::cmd_workspace_index,
            workspace::playback::cmd_playback_read,
            workspace::playback::cmd_playback_mutate,
            workspace::assets::cmd_workspace_asset,
            workspace::assets::cmd_workspace_cancel_asset,
            workspace::cleanup::cmd_cleanup_list,
            workspace::cleanup::cmd_cleanup_schedule,
            workspace::cleanup::cmd_cleanup_restore,
            workspace::cleanup::cmd_cleanup_process,
            workspace::packs::cmd_offline_packs_list,
            workspace::packs::cmd_offline_pack_create,
            workspace::packs::cmd_offline_pack_action,
            workspace::packs::cmd_offline_pack_path,
            commands::cmd_preview_sync_pair,
            commands::cmd_update_sync_pair,
            commands::cmd_set_sync_pair_active,
            commands::cmd_auth_request_code,
            commands::cmd_auth_resend_code,
            commands::cmd_auth_cancel_code,
            commands::cmd_auth_sign_in,
            commands::cmd_auth_check_password,
            commands::cmd_get_files,
            commands::cmd_get_cached_files,
            commands::cmd_upload_file,
            commands::cmd_stage_android_upload,
            commands::cmd_delete_android_staged_upload,
            commands::cmd_validate_dropped_paths,
            commands::initiate_upload,
            commands::cmd_upload_from_url,
            cmd_open_file_externally,
            cmd_open_android_stream_player,
            upload_service::cmd_start_foreground_service,
            upload_service::cmd_stop_foreground_service,
            upload_service::cmd_update_foreground_service,
            commands::cmd_connect,
            commands::cmd_log,
            commands::cmd_delete_file,
            commands::cmd_download_file,
            commands::cmd_move_files,
            commands::cmd_create_folder,
            commands::cmd_delete_folder,
            commands::cmd_rename_folder,
            commands::cmd_rename_file,
            commands::cmd_get_bandwidth,
            commands::cmd_delete_preview_for_message,
            commands::cmd_get_preview,
            commands::cmd_clean_preview_cache,
            commands::cmd_get_offline_cache_status,
            commands::cmd_get_offline_files,
            commands::cmd_set_preview_cache_limit,
            commands::cmd_set_preview_pinned,
            commands::cmd_logout,
            commands::cmd_scan_folders,
            commands::cmd_search_global,
            commands::cmd_record_file_opened,
            commands::cmd_set_file_activity_flag,
            commands::cmd_get_file_activity,
            commands::cmd_get_startup_health,
            commands::cmd_get_storage_insight,
            commands::cmd_submit_crash_report,
            commands::cmd_get_settings_sync_status,
            commands::cmd_upload_settings_sync,
            commands::cmd_download_settings_sync,
            commands::cmd_get_sync_settings,
            commands::cmd_toggle_sync,
            commands::cmd_add_sync_pair,
            commands::cmd_get_sync_pairs,
            commands::cmd_remove_sync_pair,
            commands::cmd_get_sync_status,
            commands::cmd_get_sync_conflicts,
            commands::cmd_get_sync_log,
            commands::cmd_resolve_conflict,
            #[cfg(not(target_os = "ios"))]
            commands::cmd_get_supporter_status,
            #[cfg(not(target_os = "ios"))]
            commands::cmd_begin_supporter_checkout,
            #[cfg(not(target_os = "ios"))]
            commands::cmd_poll_supporter_checkout,
            #[cfg(not(target_os = "ios"))]
            commands::cmd_activate_supporter,
            #[cfg(not(target_os = "ios"))]
            commands::cmd_refresh_supporter,
            commands::cmd_check_connection,
            commands::cmd_is_network_available,
            commands::cmd_get_android_network_status,
            commands::cmd_get_android_transfer_environment,
            commands::cmd_configure_android_transfer_recovery,
            commands::cmd_test_proxy_traffic,
            commands::cmd_reconnect_with_network_settings,
            commands::cmd_clean_cache,
            commands::cmd_get_thumbnail,
            commands::cmd_get_stream_info,
            commands::cmd_cancel_transfer,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_activity,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_adopt_legacy,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_discard_legacy,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_enqueue,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_enqueue_many,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_list,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_set_limits,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_pause,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_resume,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_cancel,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_retry,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_supply_prompt_token,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_pause_all,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_resume_all,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_cancel_all,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            transfer_engine::cmd_transfer_clear_terminal,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_preferences::cmd_get_desktop_preferences,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_preferences::cmd_update_desktop_preferences,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_preferences::cmd_set_desktop_lock_on_sleep,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_notifications::cmd_get_notification_permission,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_notifications::cmd_request_notification_permission,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_lifecycle::cmd_desktop_frontend_ready,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_lifecycle::cmd_desktop_frontend_unready,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_lifecycle::cmd_show_main_window,
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            desktop_lifecycle::cmd_quit_application,
            commands::cmd_auth_qr_login,
            commands::cmd_auth_qr_poll,
            commands::cmd_get_api_settings,
            commands::cmd_update_api_settings,
            commands::cmd_regenerate_api_key,
            commands::webdav_settings::cmd_get_webdav_settings,
            commands::webdav_settings::cmd_update_webdav_settings,
            commands::webdav_settings::cmd_regenerate_webdav_token,
            commands::cmd_delete_image_thumbnail,
            commands::cmd_zip_folder,
            commands::cmd_delete_temp_zip,
            commands::cmd_apply_proxy_settings,
            commands::cmd_migrate_proxy_secret,
            api_secret::cmd_load_api_hash,
            api_secret::cmd_store_api_hash,
            api_secret::cmd_clear_api_hash,
            commands::cmd_clear_proxy_secret,
            commands::cmd_get_proxy_status,
            commands::cmd_apply_vpn_settings,
            commands::cmd_get_network_config,
            commands::cmd_check_latency,
            commands::cmd_detect_vpn,
            commands::cmd_create_share,
            commands::cmd_list_shares,
            commands::cmd_revoke_share,
            commands::cmd_toggle_folder_visibility,
            commands::cmd_export_folder_invite,
            cmd_get_pending_share_count,
            cmd_get_pending_android_transfer_action,
            cmd_get_android_playback_history,
            cmd_list_cached_files,
            cmd_remove_cached_path,
            cmd_get_system_diagnostics,
            installation::cmd_get_installation_info,
            installation::cmd_open_microsoft_store_updates,
            android_updates::cmd_check_android_update,
            android_updates::cmd_download_and_install_android_update,
            android_security::cmd_get_android_authentication_available,
            android_security::cmd_android_authenticate,
            android_security::cmd_configure_android_privacy,
            commands::cmd_get_video_metadata,
            commands::cmd_get_video_metadata_batch,
            transcode::cmd_get_transcode_capabilities,
            transcode::cmd_prepare_transcoded_stream,
            transcode::cmd_get_transcode_status,
            transcode::cmd_cancel_transcode,
            transcode::cmd_get_master_playlist_info,
            transcode::cmd_get_transcode_cache_info,
            transcode::cmd_set_transcode_cache_limit,
            transcode::cmd_get_cached_variants,
            transcode::cmd_get_detailed_transcode_cache,
            transcode::cmd_clear_transcode_cache,
            fmp4_remux::cmd_prepare_fmp4_stream,
            fmp4_remux::cmd_get_fmp4_status,
            commands::cmd_list_archive_contents,
            commands::cmd_extract_archive_entry,
            commands::cmd_get_enriched_folders,
            commands::cmd_update_folder_order,
            commands::cmd_create_group,
            commands::cmd_update_group,
            commands::cmd_delete_group,
            commands::cmd_assign_folder_to_group,
            commands::cmd_update_group_order,
            commands::cmd_get_groups,
            crypto_commands::cmd_get_encryption_capabilities,
            crypto_commands::cmd_get_crypto_inventory,
            crypto_commands::cmd_get_encryption_settings,
            crypto_commands::cmd_update_encryption_settings,
            crypto_commands::cmd_create_vault,
            crypto_commands::cmd_unlock_vault,
            crypto_commands::cmd_change_vault_passphrase,
            crypto_commands::cmd_lock_vault,
            crypto_commands::cmd_record_vault_activity,
            crypto_commands::cmd_stage_file_passphrase,
            crypto_commands::cmd_get_vault_status,
            crypto_commands::cmd_export_vault_recovery,
            crypto_commands::cmd_import_vault_recovery,
            crypto_commands::cmd_generate_recovery_key,
            crypto_commands::cmd_get_file_encryption_info,
            crypto_commands::cmd_verify_encrypted_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    let graceful_sync_exit_started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    app.run(move |app_handle, event| {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if let tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } = &event
        {
            if label == "main" {
                let lifecycle = app_handle.state::<desktop_lifecycle::DesktopLifecycleState>();
                let preferences =
                    app_handle.state::<desktop_preferences::DesktopPreferencesState>();
                if lifecycle.should_hide_on_close(&preferences) {
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window("main") {
                        match window.hide() {
                            Ok(()) => lifecycle.mark_hidden(app_handle),
                            Err(error) => log::error!(
                                "Could not hide the main window; it will remain visible: {error}"
                            ),
                        }
                    }
                    return;
                }
                lifecycle.mark_explicit_exit();
            }
        }

        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if matches!(&event, tauri::RunEvent::Resumed) {
            if let Some(power_monitor) =
                app_handle.try_state::<Arc<desktop_power::DesktopPowerMonitor>>()
            {
                power_monitor.check_for_suspend(app_handle);
            }
        }

        if let tauri::RunEvent::ExitRequested { code, api, .. } = &event {
            if !graceful_sync_exit_started.swap(true, std::sync::atomic::Ordering::SeqCst) {
                api.prevent_exit();
                let app_handle = app_handle.clone();
                let exit_code = code.unwrap_or(0);
                tauri::async_runtime::spawn(async move {
                    if let Some(sync_engine) = app_handle.try_state::<sync_engine::SyncEngine>() {
                        if let Err(error) = sync_engine.shutdown_and_wait().await {
                            log::error!("Folder sync did not shut down cleanly: {error}");
                        }
                    }
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    if let Some(transfer_engine) =
                        app_handle.try_state::<Arc<transfer_engine::TransferEngine>>()
                    {
                        transfer_engine.begin_shutdown();
                    }
                    if let Some(streaming_server) = app_handle.try_state::<ActixServerHandle>() {
                        let handle = streaming_server
                            .0
                            .lock()
                            .ok()
                            .and_then(|mut value| value.take());
                        if let Some(handle) = handle {
                            log::info!("Stopping Actix streaming server...");
                            handle.stop(true).await;
                        }
                    }
                    if let Some(api_server) = app_handle.try_state::<ApiServerLifecycle>() {
                        if let Some(handle) = api_server.0.begin_shutdown() {
                            log::info!("Stopping REST API server...");
                            handle.stop(true).await;
                        }
                    }
                    if let Some(webdav_server) = app_handle.try_state::<WebDavServerLifecycle>() {
                        if let Some(handle) = webdav_server.0.begin_shutdown() {
                            log::info!("Stopping WebDAV server...");
                            handle.stop(true).await;
                        }
                    }
                    app_handle.exit(exit_code);
                });
                return;
            }
        }
        if let tauri::RunEvent::Exit = event {
            log::info!("Application exiting — shutting down background services...");

            if let Some(crypto_state) = app_handle.try_state::<crypto::state::CryptoState>() {
                crypto_state.lock();
            }
            if let Some(sync_engine) = app_handle.try_state::<sync_engine::SyncEngine>() {
                sync_engine.shutdown();
            }

            // 1. Shutdown the grammers network runner
            let shutdown_arc = app_handle.state::<TelegramState>().runner_shutdown.clone();
            let runner_tx = shutdown_arc.lock().ok().and_then(|mut g| g.take());
            if let Some(tx) = runner_tx {
                log::info!("Signaling network runner shutdown...");
                let _ = tx.send(());
            }

            // 2. The streaming server is normally awaited during ExitRequested.
            // Keep an immediate fallback for forced/platform exits that bypass it.
            let server_arc = app_handle.state::<ActixServerHandle>().0.clone();
            let server_handle = server_arc.lock().ok().and_then(|mut g| g.take());
            if let Some(handle) = server_handle {
                log::info!("Stopping Actix streaming server immediately...");
                tauri::async_runtime::spawn(async move {
                    handle.stop(false).await;
                });
            }

            // 3. Immediate fallbacks for local servers when ExitRequested was bypassed.
            if let Some(handle) = app_handle.state::<ApiServerLifecycle>().0.begin_shutdown() {
                log::info!("Stopping REST API server immediately...");
                tauri::async_runtime::spawn(async move {
                    handle.stop(false).await;
                });
            }
            if let Some(handle) = app_handle
                .state::<WebDavServerLifecycle>()
                .0
                .begin_shutdown()
            {
                log::info!("Stopping WebDAV server immediately...");
                tauri::async_runtime::spawn(async move {
                    handle.stop(false).await;
                });
            }

            // 4. Stop local SOCKS5 proxy bridge (if running)
            if let Some(net_config) = app_handle.try_state::<Arc<vpn_optimizer::NetworkConfig>>() {
                log::info!("Stopping SOCKS5 bridge...");
                net_config.stop_http_bridge();
            }
        }
    });
}
