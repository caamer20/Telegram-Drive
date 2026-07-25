pub mod models;

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
pub mod socks5_bridge;
pub mod vpn_optimizer;

use tauri::Manager;

use commands::streaming::StreamConfig;
use commands::TelegramState;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod api_routes;
pub mod db;
pub mod fmp4_remux;
pub mod jni_cache;
pub mod migration;
pub mod mp4_utils;
pub mod server;
pub mod share_routes;
pub mod transcode;
pub mod upload_service;

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

/// Tracks whether the API server is currently running (for the frontend status dot)
pub struct ApiServerRunning(pub Arc<std::sync::atomic::AtomicBool>);

/// Holds the API server stop handle separately so we can restart it independently
pub struct ApiServerHandle(pub Arc<std::sync::Mutex<Option<actix_web::dev::ServerHandle>>>);

/// Restart (or stop) the API server based on current settings.
/// Called from Tauri commands when the user changes API settings.
#[cfg(not(target_os = "android"))]
pub fn restart_api_server(app: &tauri::AppHandle) {
    // Stop existing API server if running
    let api_handle_arc = app.state::<ApiServerHandle>().0.clone();
    let old_handle = api_handle_arc.lock().ok().and_then(|mut g| g.take());
    if let Some(handle) = old_handle {
        log::info!("Stopping existing API server...");
        drop(handle.stop(true));
        // Give it a moment to release the port
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    let settings = commands::api_settings::load_settings(app);
    let running_flag = app.state::<ApiServerRunning>().0.clone();

    if !settings.enabled {
        running_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        log::info!("API server disabled");
        return;
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
    let key_hash = settings.key_hash.clone();
    let handle_for_thread = api_handle_arc.clone();

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

    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        init_com_on_worker_thread();
        let sys = actix_rt::System::new();
        sys.block_on(async move {
            let api_state_data = actix_web::web::Data::new(tg_state);
            let api_state = actix_web::web::Data::new(api_routes::ApiState { key_hash });
            let cache_dirs = actix_web::web::Data::new(api_routes::CacheDirs {
                thumbnail_dir,
                preview_dir,
            });
            let api_bw = actix_web::web::Data::new(bw_manager);
            let api_net = actix_web::web::Data::new(net_config);
            let api_db = actix_web::web::Data::new(db_pool);

            log::info!("Starting REST API server on port {}", api_port);

            match actix_web::HttpServer::new(move || {
                let cors = actix_cors::Cors::default()
                    .allowed_origin_fn(|origin, _req_head| {
                        let origin_bytes = origin.as_bytes();
                        origin_bytes.starts_with(b"tauri://")
                            || origin_bytes.starts_with(b"http://tauri.localhost")
                            || origin_bytes.starts_with(b"https://tauri.localhost")
                            || origin_bytes.starts_with(b"http://localhost")
                            || origin_bytes.starts_with(b"http://127.0.0.1")
                            || origin_bytes.starts_with(b"https://asset.localhost")
                            || origin_bytes.starts_with(b"http://asset.localhost")
                            || origin_bytes == b"null"
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
            .bind(("127.0.0.1", api_port))
            {
                Ok(bound) => {
                    let server = bound.run();
                    *handle_for_thread.lock().unwrap() = Some(server.handle());
                    running_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    log::info!("REST API server started on http://127.0.0.1:{}", api_port);
                    server.await.ok();
                }
                Err(e) => {
                    running_flag.store(false, std::sync::atomic::Ordering::Relaxed);
                    log::error!("Failed to start API server on port {}: {}", api_port, e);
                }
            }
        });
    });
}

/// Restart (or stop) the API server based on current settings.
/// Called from Tauri commands when the user changes API settings.
#[cfg(target_os = "android")]
pub fn restart_api_server(_app: &tauri::AppHandle) {
    log::info!("REST API disabled on mobile.");
}

#[tauri::command]
fn cmd_open_file_externally(path: String, app_handle: tauri::AppHandle) -> Result<(), String> {
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
                "mp3" => "audio/mpeg",
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
        app_handle
            .opener()
            .open_path(&path, None::<&str>)
            .map_err(|e| e.to_string())
    }
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
        lines.push(format!(
            "XDG_SESSION_TYPE: {}",
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into())
        ));
        lines.push(format!(
            "XDG_CURRENT_DESKTOP: {}",
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into())
        ));
        lines.push(format!(
            "WEBKIT_DISABLE_DMABUF_RENDERER: {}",
            std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").unwrap_or_else(|_| "unset".into())
        ));
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let stream_token = generate_stream_token();

    // Shared handle for stopping the Actix streaming server during shutdown
    let server_handle: Arc<std::sync::Mutex<Option<actix_web::dev::ServerHandle>>> =
        Arc::new(std::sync::Mutex::new(None));
    let server_handle_for_setup = server_handle.clone();

    let builder = tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("telegram-drive".into()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init());

    // The updater plugin is not supported on Android and can cause crashes
    // (APKs are managed by the Play Store; the plugin attempts restricted FS ops).
    #[cfg(not(target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_window_state::Builder::default().build());

    let app = builder
        .setup(move |app| {
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
                login_token: Arc::new(Mutex::new(None)),
                password_token: Arc::new(Mutex::new(None)),
                api_id: Arc::new(Mutex::new(None)),
                runner_shutdown: Arc::new(std::sync::Mutex::new(None)),
                runner_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                peer_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
                cancelled_transfers: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
            });
            app.manage(Arc::new(bandwidth::BandwidthManager::new(app.handle())));
            app.manage(StreamConfig { token: stream_token.clone(), port: STREAM_PORT });
            app.manage(ActixServerHandle(server_handle_for_setup.clone()));
            app.manage(ApiServerHandle(Arc::new(std::sync::Mutex::new(None))));
            app.manage(ApiServerRunning(Arc::new(std::sync::atomic::AtomicBool::new(false))));

            // Initialize TranscodeManager for HLS streaming
            let app_data_dir = app.path().app_data_dir().map_err(|e| {
                log::error!("Failed to get app data dir: {}", e);
                e
            })?;
            let cache_root = app_data_dir.join("streaming");
            let transcode_manager = transcode::TranscodeManager::new(cache_root);
            // Detect FFmpeg (non-blocking spawn)
            let app_handle = app.handle().clone();
            let ffmpeg_path_arc = transcode_manager.ffmpeg_path.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(ffmpeg) = transcode::detect_ffmpeg(&app_handle).await {
                    *ffmpeg_path_arc.lock().await = Some(ffmpeg);
                }
            });
            let transcode_arc = Arc::new(transcode_manager);
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

            // Initialize Migration Database & State
            let migration_db = migration::db::init_migration_db(app.handle()).map_err(|e| {
                log::error!("Failed to initialize Migration SQLite database: {}", e);
                e
            })?;
            let restored_ms_session = match migration::session_store::load(app.handle()) {
                Ok(session) => session,
                Err(error) => {
                    log::warn!("Không thể khôi phục Microsoft session: {}", error);
                    None
                }
            };
            app.manage(migration::MigrationState::new_with_session(
                migration_db,
                restored_ms_session,
            ));

            // Start Streaming Server on dedicated thread (Actix needs its own runtime)
            // Disabled on Android: actix_rt::System creates a second Tokio runtime that
            // conflicts with Tauri's runtime and crashes the process on launch.
            #[cfg(not(target_os = "android"))]
            {
                let state = Arc::new(app.state::<TelegramState>().inner().clone());
                let token_for_server = stream_token.clone();
                let handle_for_thread = server_handle_for_setup.clone();
                let db_pool_for_server = db_pool.clone();
                let transcode_for_server = transcode_arc.clone();
                std::thread::spawn(move || {
                    #[cfg(target_os = "windows")]
                    init_com_on_worker_thread();
                    let sys = actix_rt::System::new();
                    sys.block_on(async move {
                        match server::start_server(state, STREAM_PORT, token_for_server, db_pool_for_server, transcode_for_server).await {
                            Ok(server) => {
                                if let Ok(mut handle) = handle_for_thread.lock() {
                                    *handle = Some(server.handle());
                                }
                                // Now await the server — blocks until stopped
                                server.await.ok();
                            }
                            Err(e) => log::error!("Streaming server failed: {}", e),
                        }
                    });
                });
            }
            #[cfg(target_os = "android")]
            {
                log::info!("Streaming server disabled on Android (Actix runtime conflict avoidance).");
            }

            // Start API server if enabled in settings
            restart_api_server(app.handle());

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
                        // TCP ping to Telegram DC2 (best-effort)
                        let _ = tauri::async_runtime::spawn_blocking(|| {
                            use std::net::TcpStream;
                            let addr: std::net::SocketAddr = match "149.154.167.50:443".parse() {
                            Ok(a) => a,
                            Err(e) => {
                                log::error!("VPN keep-alive: failed to parse DC2 address: {}", e);
                                return;
                            }
                        };
                        let _ = TcpStream::connect_timeout(
                                &addr,
                                std::time::Duration::from_secs(5),
                            );
                        }).await;
                    }
                });
            }

            // Auto Migration Engine trigger
            if let Ok(log_dir) = app.path().app_log_dir() {
                log::info!("==================================================");
                log::info!(
                    "TELEGRAM DRIVE LOG FILE: {:?}",
                    log_dir.join("telegram-drive.log")
                );
                log::info!("==================================================");
            }

            // Auto Migration is explicitly started from the Smart Auto-Migration
            // Center. Do not launch a scan implicitly during app startup; this
            // keeps the pipeline button and its logs authoritative.

            Ok(())
        })


        .invoke_handler(tauri::generate_handler![
            commands::cmd_auth_request_code,
            commands::cmd_auth_sign_in,
            commands::cmd_auth_check_password,
            commands::cmd_get_files,
            commands::cmd_upload_file,
            commands::initiate_upload,
            commands::cmd_upload_from_url,
            cmd_open_file_externally,
            upload_service::cmd_start_foreground_service,
            upload_service::cmd_stop_foreground_service,
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
            commands::cmd_logout,
            commands::cmd_scan_folders,
            commands::cmd_search_global,
            commands::cmd_check_connection,
            commands::cmd_is_network_available,
            commands::cmd_test_proxy_traffic,
            commands::cmd_reconnect_with_network_settings,
            commands::cmd_clean_cache,
            commands::cmd_get_thumbnail,
            commands::cmd_get_stream_info,
            commands::cmd_cancel_transfer,
            commands::cmd_auth_qr_login,
            commands::cmd_auth_qr_poll,
            commands::cmd_get_api_settings,
            commands::cmd_update_api_settings,
            commands::cmd_regenerate_api_key,
            commands::cmd_delete_image_thumbnail,
            commands::cmd_zip_folder,
            commands::cmd_delete_temp_zip,
            commands::cmd_apply_proxy_settings,
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
            cmd_list_cached_files,
            cmd_remove_cached_path,
            cmd_get_system_diagnostics,
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
            migration::commands::cmd_migration_ms_connect,
            migration::commands::cmd_migration_ms_disconnect,
            migration::commands::cmd_migration_ms_status,
            migration::commands::cmd_migration_list_onedrive_folders,
            migration::commands::cmd_migration_create_job,
            migration::commands::cmd_migration_get_jobs,
            migration::commands::cmd_migration_get_job,
            migration::commands::cmd_migration_delete_job,
            migration::commands::cmd_migration_set_onedrive_folder,
            migration::commands::cmd_migration_set_telegram_destination,
            migration::commands::cmd_migration_set_local_dir,
            migration::commands::cmd_migration_scan,
            migration::commands::cmd_migration_start,
            migration::commands::cmd_migration_pause,
            migration::commands::cmd_migration_resume,
            migration::commands::cmd_migration_cancel,
            migration::commands::cmd_migration_retry_item,
            migration::commands::cmd_migration_retry_all_failed,
            migration::commands::cmd_migration_get_auto_status,
            migration::commands::cmd_migration_toggle_auto,
            migration::commands::cmd_migration_update_auto_settings,
            migration::commands::cmd_migration_get_daily_quota,
            migration::commands::cmd_migration_rescan_auto,
            migration::commands::cmd_migration_stop_auto_scan,
            migration::commands::cmd_migration_get_scan_snapshot,
            migration::commands::cmd_migration_sync_scan_snapshot_item,
            migration::commands::cmd_migration_get_activity,
            // V1 destructive commands — disabled for V2 safety
            // migration::commands::cmd_migration_delete_item,
            // migration::commands::cmd_migration_rename_item,
            migration::commands::cmd_migration_sync_single_item,
            migration::commands::cmd_migration_queue_selected_items,
            // V2 backup commands
            migration::commands_v2::cmd_backup_v2_preflight,
            migration::commands_v2::cmd_backup_v2_start,
            migration::commands_v2::cmd_backup_v2_pause,
            migration::commands_v2::cmd_backup_v2_resume,
            migration::commands_v2::cmd_backup_v2_stop,
            migration::commands_v2::cmd_backup_v2_get_status,
            migration::commands_v2::cmd_backup_v2_retry_manifest,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            log::info!("Application exiting — shutting down background services...");

            // 1. Shutdown the grammers network runner
            let shutdown_arc = app_handle.state::<TelegramState>().runner_shutdown.clone();
            let runner_tx = shutdown_arc.lock().ok().and_then(|mut g| g.take());
            if let Some(tx) = runner_tx {
                log::info!("Signaling network runner shutdown...");
                let _ = tx.send(());
            }

            // 2. Stop the Actix streaming server (graceful)
            let server_arc = app_handle.state::<ActixServerHandle>().0.clone();
            let server_handle = server_arc.lock().ok().and_then(|mut g| g.take());
            if let Some(handle) = server_handle {
                log::info!("Stopping Actix streaming server...");
                drop(handle.stop(true));
            }

            // 3. Stop the API server (graceful)
            let api_arc = app_handle.state::<ApiServerHandle>().0.clone();
            let api_handle = api_arc.lock().ok().and_then(|mut g| g.take());
            if let Some(handle) = api_handle {
                log::info!("Stopping API server...");
                drop(handle.stop(true));
            }

            // 4. Stop local SOCKS5 proxy bridge (if running)
            if let Some(net_config) = app_handle.try_state::<Arc<vpn_optimizer::NetworkConfig>>() {
                log::info!("Stopping SOCKS5 bridge...");
                net_config.stop_http_bridge();
            }
        }
    });
}
