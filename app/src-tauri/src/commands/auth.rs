use tauri::State;
use tauri::Manager;
use grammers_client::Client;
use std::sync::Arc;
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;

use crate::TelegramState;
use crate::models::{AuthResult};
use crate::commands::utils::map_error;
use grammers_client::SignInError;

// Helper to ensure client is initialized
pub async fn ensure_client_initialized(
    app_handle: &tauri::AppHandle,
    state: &State<'_, TelegramState>,
    api_id: i32,
) -> Result<Client, String> {
    let mut client_guard = state.client.lock().await;

    if let Some(client) = client_guard.as_ref() {
        return Ok(client.clone());
    }

    log::info!("Initializing new Telegram Client with ID: {}", api_id);
    
    // Resolve session path safely
    let app_data_dir = app_handle.path().app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
        
    if !app_data_dir.exists() {
        std::fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("Failed to create app data dir: {}", e))?;
    }
    
    let session_path = app_data_dir.join("telegram.session");
    let session_path_str = session_path.to_string_lossy().to_string();
    log::info!("Opening session at: {}", session_path_str);
    
    // Grammers initialization with corruption recovery
    let session = match SqliteSession::open(&session_path_str).map_err(|e| e.to_string()) {
        Ok(s) => s,
        Err(_) => {
            log::warn!("Session file corrupted or invalid. Recreating...");
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
        log::info!("Telegram network runner stopped");
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
    ensure_client_initialized(&app_handle, &state, api_id).await?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_logout(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    log::info!("Logging out...");
    
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
    
    log::info!("Requesting code for {}", phone);
    
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
    let login_token = token_guard.as_ref().ok_or("No login session found (restart flow)")?;

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
        Err(e) => Err(format!("2FA Failed: {}", e))
    }
}
