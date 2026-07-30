use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use grammers_session::Session;
use grammers_tl_types as tl;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Manager;
use tauri::State;
use tokio::sync::oneshot;
use tokio::time::Duration;

use crate::commands::utils::map_error;
use crate::models::AuthResult;
use crate::TelegramState;
use grammers_client::SignInError;

/// Ensures the Telegram client is initialized.
///
/// IMPORTANT: This function properly manages runner lifecycle to prevent stack overflow.
/// Before spawning a new runner, it signals the old runner to shutdown.
pub async fn ensure_client_initialized(
    app_handle: &tauri::AppHandle,
    state: &State<'_, TelegramState>,
    api_id: i32,
) -> Result<Client, String> {
    #[cfg(target_os = "android")]
    {
        let mut count = 0;
        while ndk_context::android_context().vm().is_null()
            || ndk_context::android_context().context().is_null()
        {
            if count >= 200 {
                // 10 seconds timeout
                return Err("Timeout waiting for Android JNI context initialization.".to_string());
            }
            log::info!(
                "Waiting for Android JNI context to initialize ({}ms)...",
                count * 50
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            count += 1;
        }
        log::info!("Android JNI context is ready!");
    }

    let mut client_guard = state.client.lock().await;

    if let Some(client) = client_guard.as_ref() {
        return Ok(client.clone());
    }

    // CRITICAL: Shutdown existing runner before creating a new one
    // This prevents runner task accumulation which causes stack overflow
    let did_shutdown_old_runner = {
        let mut guard = state.runner_shutdown.lock().unwrap();
        if let Some(shutdown_tx) = guard.take() {
            log::info!("Signaling old runner to shutdown...");
            let _ = shutdown_tx.send(());
            true
        } else {
            false
        }
    }; // MutexGuard dropped here — before the await
    if did_shutdown_old_runner {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let runner_num = state.runner_count.fetch_add(1, Ordering::SeqCst) + 1;
    log::info!(
        "Initializing Telegram Client #{} with API ID: {}",
        runner_num,
        api_id
    );

    // Resolve session path safely
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;

    if !app_data_dir.exists() {
        std::fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("Failed to create app data dir: {}", e))?;
    }

    let session_path = app_data_dir.join("telegram.session");
    let session_path_str = session_path.to_string_lossy().to_string();
    log::info!("Opening session at: {}", session_path_str);

    let mut session_open_result = SqliteSession::open(&session_path_str);

    // Retry opening the session database up to 5 times (every 100ms)
    // in case the database is temporarily locked by the old shutting down runner.
    if session_open_result.is_err() {
        for attempt in 1..=5 {
            log::warn!("Failed to open session on attempt {} (database may be locked). Retrying in 100ms...", attempt);
            tokio::time::sleep(Duration::from_millis(100)).await;
            session_open_result = SqliteSession::open(&session_path_str);
            if session_open_result.is_ok() {
                break;
            }
        }
    }

    let session = match session_open_result.map_err(|e| e.to_string()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!(
                "Session file could not be opened after retries ({}). Recreating...",
                e
            );
            let _ = std::fs::remove_file(&session_path);
            let _ = std::fs::remove_file(format!("{}-wal", session_path_str));
            let _ = std::fs::remove_file(format!("{}-shm", session_path_str));

            SqliteSession::open(&session_path_str)
                .map_err(|err| format!("Failed to open session after recreation: {}", err))?
        }
    };

    let net_config = app_handle.state::<Arc<crate::vpn_optimizer::NetworkConfig>>();
    let preferred_dc = {
        let vpn = net_config.vpn.read().unwrap();
        if vpn.enabled {
            vpn.preferred_dc.clone()
        } else {
            "auto".to_string()
        }
    };
    if preferred_dc.starts_with("dc") && preferred_dc.len() > 2 {
        if let Ok(dc_id) = preferred_dc[2..].parse::<i32>() {
            log::info!("Setting preferred home DC ID: {}", dc_id);
            session.set_home_dc_id(dc_id);
        }
    }

    let mut connection_params = grammers_mtsender::ConnectionParams::default();
    if let Some(proxy_url) = net_config.effective_proxy_url() {
        log::info!("Using proxy: {}", proxy_url);
        connection_params.proxy_url = Some(proxy_url);
    }

    let session = Arc::new(session);
    let pool = SenderPool::with_configuration(session, api_id, connection_params);
    let client = Client::new(&pool);

    // Create shutdown channel for this runner
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    *state.runner_shutdown.lock().unwrap() = Some(shutdown_tx);

    // Spawn the network runner with shutdown support
    let SenderPool { runner, .. } = pool;
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            // Normal runner operation
            _ = runner.run() => {
                log::info!("Runner #{} exited normally", runner_num);
            }
            // Shutdown requested
            _ = shutdown_rx => {
                log::info!("Runner #{} shutdown requested, exiting", runner_num);
            }
        }
    });

    *client_guard = Some(client.clone());
    Ok(client)
}

#[tauri::command]
pub async fn cmd_connect(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    api_id: i32,
) -> Result<bool, String> {
    // Store API ID for auto-reconnect
    *state.api_id.lock().await = Some(api_id);
    ensure_client_initialized(&app_handle, &state, api_id).await?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_check_connection(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    // 1. Check if client exists and is responsive
    let client_msg_opt = {
        let guard = state.client.lock().await;
        guard.as_ref().cloned()
    };

    if let Some(client) = client_msg_opt {
        // Ping (e.g., get_me)
        if client.get_me().await.is_ok() {
            return Ok(true);
        }
        log::warn!("Connection check failed (get_me). Attempting reconnect...");
    } else {
        log::warn!("Connection check: No client found. Checking for saved API ID...");
    }

    // 2. Reconnect Logic
    let api_id_opt = *state.api_id.lock().await;
    if let Some(api_id) = api_id_opt {
        // Force re-init: Clear old client first to ensure fresh pool
        *state.client.lock().await = None;

        match ensure_client_initialized(&app_handle, &state, api_id).await {
            Ok(c) => {
                // Double check
                if c.get_me().await.is_ok() {
                    log::info!("Auto-reconnect successful.");
                    return Ok(true);
                } else {
                    return Err("Reconnect succeeded but ping failed.".to_string());
                }
            }
            Err(e) => return Err(format!("Auto-reconnect failed: {}", e)),
        }
    }

    Ok(false) // Not connected and no credentials to reconnect
}

#[tauri::command]
pub async fn cmd_reconnect_with_network_settings(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let api_id = *state.api_id.lock().await;
    let api_id = match api_id {
        Some(id) => id,
        None => return Err("Not authenticated — no API ID saved.".into()),
    };

    log::info!("Reconnecting with updated network settings...");

    // 1. Shutdown existing runner
    {
        let mut shutdown_guard = state.runner_shutdown.lock().unwrap();
        if let Some(shutdown_tx) = shutdown_guard.take() {
            log::info!("Signaling runner shutdown for reconnect...");
            let _ = shutdown_tx.send(());
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Clear old client
    *state.client.lock().await = None;

    // 3. Reinitialize with current network config (reads from NetworkConfig state)
    let client = ensure_client_initialized(&app_handle, &state, api_id).await?;

    // 4. Verify the new connection works
    match client.get_me().await {
        Ok(_me) => {
            log::info!("Reconnect successful — verified via get_me().");
            Ok(true)
        }
        Err(e) => {
            log::error!("Reconnect init succeeded but get_me failed: {}", e);
            Err(format!("Reconnected but ping failed: {}", e))
        }
    }
}

#[tauri::command]
pub async fn cmd_logout(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    log::info!("Logging out...");

    #[cfg(target_os = "android")]
    let media_account_id = {
        let client = state.client.lock().await.clone();
        match client {
            Some(client) => tokio::time::timeout(Duration::from_secs(3), client.get_me())
                .await
                .ok()
                .and_then(Result::ok)
                .map(|account| account.bare_id()),
            None => None,
        }
    };

    // Native screens own process-local credentials and must be closed and
    // account-scoped data cleared before the Telegram identity is discarded.
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_library::MediaLibraryExt;
        use tauri_plugin_native_player::NativePlayerExt;
        let _ = app_handle.native_player().close().await;
        let _ = app_handle.native_player().clear_pending_restore().await;
        let _ = app_handle.media_library().close().await;
        let _ = app_handle
            .media_library()
            .clear_account(media_account_id)
            .await;
    }

    // 1. Shutdown the network runner FIRST to prevent any operations
    {
        let mut shutdown_guard = state.runner_shutdown.lock().unwrap();
        if let Some(shutdown_tx) = shutdown_guard.take() {
            log::info!("Signaling runner shutdown for logout...");
            let _ = shutdown_tx.send(());
        }
    }

    // 2. Try to sign out from Telegram (if connected)
    let client_opt = { state.client.lock().await.clone() };
    if let Some(client) = client_opt {
        // We don't strictly care if this fails (e.g. network down), we just want to clear local state.
        let _ = client.sign_out().await;
    }

    // 3. Clear State
    *state.client.lock().await = None;
    *state.login_token.lock().await = None;
    *state.password_token.lock().await = None;
    *state.api_id.lock().await = None;
    crate::commands::utils::clear_peer_cache(&state.peer_cache).await;
    state.cancelled_transfers.write().await.clear();

    // 4. Remove Session File
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let session_path = app_data_dir.join("telegram.session");
    let _ = std::fs::remove_file(session_path);
    let _ = std::fs::remove_file(app_data_dir.join("telegram.session-wal"));
    let _ = std::fs::remove_file(app_data_dir.join("telegram.session-shm"));

    log::info!(
        "Logout complete. Runner count: {}",
        state.runner_count.load(Ordering::SeqCst)
    );
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

    // Store API ID
    *state.api_id.lock().await = Some(api_id);

    let client_handle = ensure_client_initialized(&app_handle, &state, api_id).await?;

    log::info!("Requesting code for {}", phone);

    let mut last_error = String::new();

    // Retry up to 2 times for AUTH_RESTART or 500
    for i in 1..=2 {
        match client_handle.request_login_code(&phone, &api_hash).await {
            Ok(token) => {
                let mut token_guard = state.login_token.lock().await;
                *token_guard = Some(token);
                return Ok("code_sent".to_string());
            }
            Err(e) => {
                let err_msg = e.to_string();
                log::warn!("Error requesting code (Attempt {}): {}", i, err_msg);

                if err_msg.contains("AUTH_RESTART") || err_msg.contains("500") {
                    log::info!("AUTH_RESTART error detected. Retrying...");
                    last_error = err_msg;
                    // Prepare for retry
                    continue;
                }

                // Other errors, fail immediately
                return Err(map_error(e));
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
    log::info!("Signing in with code...");

    let client = {
        let guard = state.client.lock().await;
        guard.as_ref().ok_or("Client not initialized")?.clone()
    };

    let token_guard = state.login_token.lock().await;
    let login_token = token_guard
        .as_ref()
        .ok_or("No login session found (restart flow)")?;

    match client.sign_in(login_token, &code).await {
        Ok(_user) => {
            log::info!("Successfully logged in.");
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
            log::error!("Sign in error: {}", e);
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
            log::info!("2FA Success.");
            Ok(AuthResult {
                success: true,
                next_step: Some("dashboard".to_string()),
                error: None,
            })
        }
        Err(e) => Err(format!("2FA Failed: {}", e)),
    }
}

/// QR Login -- Step 1: Export a login token and return the `tg://login?token=...` URL.
/// The frontend renders this as a QR code for the user to scan with their phone.
#[tauri::command]
pub async fn cmd_auth_qr_login(
    app_handle: tauri::AppHandle,
    api_id: i32,
    api_hash: String,
    state: State<'_, TelegramState>,
) -> Result<String, String> {
    if api_hash.trim().is_empty() {
        return Err("API Hash cannot be empty.".to_string());
    }

    // Store API ID
    *state.api_id.lock().await = Some(api_id);

    let client = ensure_client_initialized(&app_handle, &state, api_id).await?;

    log::info!("Requesting QR login token...");

    let result = client
        .invoke(&tl::functions::auth::ExportLoginToken {
            api_id,
            api_hash: api_hash.clone(),
            except_ids: vec![],
        })
        .await
        .map_err(|e| format!("ExportLoginToken failed: {}", e))?;

    match result {
        tl::enums::auth::LoginToken::Token(t) => {
            let encoded = URL_SAFE_NO_PAD.encode(&t.token);
            let url = format!("tg://login?token={}", encoded);
            log::info!("QR login URL generated, expires at {}", t.expires);
            Ok(url)
        }
        tl::enums::auth::LoginToken::Success(_s) => {
            // Already authorized (e.g. from a previous session)
            log::info!("QR login: already authorized");
            Ok("__authorized__".to_string())
        }
        tl::enums::auth::LoginToken::MigrateTo(m) => {
            log::info!("QR login: need to migrate to DC {}", m.dc_id);
            let encoded = URL_SAFE_NO_PAD.encode(&m.token);
            let url = format!("tg://login?token={}", encoded);
            Ok(url)
        }
    }
}

/// QR Login -- Step 2: Poll for scan completion.
/// Checks if the session became authorized after the user scanned the QR code.
///
/// IMPORTANT: We must NOT call auth.exportLoginToken here for polling.
/// Each call to exportLoginToken generates a NEW token and invalidates the
/// previous one, causing the scanned QR code to fail with "Invalid code".
/// Instead, we check is_authorized() which succeeds once the phone app
/// accepts the token via auth.acceptLoginToken.
#[tauri::command]
pub async fn cmd_auth_qr_poll(state: State<'_, TelegramState>) -> Result<AuthResult, String> {
    let client = {
        let guard = state.client.lock().await;
        guard.as_ref().ok_or("Client not initialized")?.clone()
    };

    // Check if the session is now authorized (user scanned QR on phone)
    match client.is_authorized().await {
        Ok(true) => {
            log::info!("QR login: session authorized!");
            Ok(AuthResult {
                success: true,
                next_step: Some("dashboard".to_string()),
                error: None,
            })
        }
        Ok(false) => {
            // Not yet scanned or accepted
            Ok(AuthResult {
                success: false,
                next_step: Some("waiting".to_string()),
                error: None,
            })
        }
        Err(e) => {
            log::warn!("QR poll auth check failed: {}", e);
            Ok(AuthResult {
                success: false,
                next_step: Some("waiting".to_string()),
                error: None,
            })
        }
    }
}
