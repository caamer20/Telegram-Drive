use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Persisted API settings (written to api_settings.json in the app data dir)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiSettingsFile {
    pub enabled: bool,
    pub port: u16,
    pub key_hash: Option<String>,
}

impl Default for ApiSettingsFile {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8550,
            key_hash: None,
        }
    }
}

/// What the frontend sees (never exposes the hash)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiSettingsResponse {
    pub enabled: bool,
    pub port: u16,
    pub key_set: bool,
    pub running: bool,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("api_settings.json"))
}

pub fn load_settings(app: &AppHandle) -> ApiSettingsFile {
    let path = match settings_path(app) {
        Ok(p) => p,
        Err(_) => return ApiSettingsFile::default(),
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => ApiSettingsFile::default(),
    }
}

fn save_settings(app: &AppHandle, settings: &ApiSettingsFile) -> Result<(), String> {
    let path = settings_path(app)?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Verify a plaintext key against a stored hash using constant-time comparison
/// to prevent timing side-channel attacks.
pub fn verify_key(plaintext: &str, stored_hash: &str) -> bool {
    let computed = hash_key(plaintext);
    constant_time_eq::constant_time_eq(computed.as_bytes(), stored_hash.as_bytes())
}

#[tauri::command]
pub async fn cmd_get_api_settings(app: AppHandle) -> Result<ApiSettingsResponse, String> {
    let settings = load_settings(&app);
    let running = {
        let state = app.try_state::<crate::ApiServerRunning>();
        state
            .map(|s| s.0.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
    };
    Ok(ApiSettingsResponse {
        enabled: settings.enabled,
        port: settings.port,
        key_set: settings.key_hash.is_some(),
        running,
    })
}

#[tauri::command]
pub async fn cmd_update_api_settings(
    enabled: bool,
    port: u16,
    app: AppHandle,
) -> Result<ApiSettingsResponse, String> {
    // Validate port range
    if port < 1024 {
        return Err("Port must be 1024 or higher".to_string());
    }

    let mut settings = load_settings(&app);
    let port_changed = settings.port != port;
    let enabled_changed = settings.enabled != enabled;

    settings.enabled = enabled;
    settings.port = port;
    save_settings(&app, &settings)?;

    // Restart server if anything changed
    if port_changed || enabled_changed {
        crate::restart_api_server(&app);
    }

    let running = {
        let state = app.try_state::<crate::ApiServerRunning>();
        state
            .map(|s| s.0.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
    };

    Ok(ApiSettingsResponse {
        enabled: settings.enabled,
        port: settings.port,
        key_set: settings.key_hash.is_some(),
        running,
    })
}

#[tauri::command]
pub async fn cmd_regenerate_api_key(app: AppHandle) -> Result<String, String> {
    let mut settings = load_settings(&app);

    // Generate a secure 32-byte random key as hex
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rand::Rng::random(&mut rng)).collect();
    let plaintext_key: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();

    // Store only the hash
    settings.key_hash = Some(hash_key(&plaintext_key));
    save_settings(&app, &settings)?;

    // Restart server so middleware picks up the new hash
    crate::restart_api_server(&app);

    // Return the plaintext key ONCE — it is never stored or retrievable again
    Ok(plaintext_key)
}
