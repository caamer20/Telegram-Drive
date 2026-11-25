//! Tauri commands for applying proxy and VPN optimizer settings.
//! These are called from the frontend when the user changes network configuration.

use crate::vpn_optimizer::{NetworkConfig, ProxyConfig, ProxyCredentialUpdate, VpnConfig};
use tauri::State;

// Serialize fallback reads and disposal of failed resources. Healthy settings
// continue using the SDK's shared store and its regular write path.
static SETTINGS_RELOAD: std::sync::Mutex<()> = std::sync::Mutex::new(());
const SETTINGS_READ_ERROR: &str = "Saved settings could not be read.";

pub(crate) fn reload_missing_settings_at(
    app: &tauri::AppHandle,
    path: &std::path::Path,
) -> Result<Option<serde_json::Value>, String> {
    use tauri_plugin_store::{Error, StoreExt};

    let _reload = SETTINGS_RELOAD.lock().map_err(|_| SETTINGS_READ_ERROR)?;
    let store = app.store(path).map_err(|_| SETTINGS_READ_ERROR)?;
    let cached = || store.get("settings").filter(|value| !value.is_null());
    if let Some(settings) = cached() {
        return Ok(Some(settings));
    }
    match store.reload() {
        Ok(()) => Ok(cached()),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(cached()),
        Err(_) => {
            if let Some(settings) = cached() {
                return Ok(Some(settings));
            }
            // The plugin saves every registered store on exit. Leaving this
            // failed, empty settings resource registered could overwrite the file.
            store.close_resource();
            Err(SETTINGS_READ_ERROR.into())
        }
    }
}

/// Distinguish first launch from an SDK load that suppressed a disk error.
/// The webview receives only readiness for the fixed AppData store.
#[tauri::command]
pub async fn cmd_reload_missing_settings(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        reload_missing_settings_at(&app, std::path::Path::new("settings.json")).map(|_| ())
    })
    .await
    .map_err(|_| SETTINGS_READ_ERROR)?
}

#[derive(Debug, serde::Deserialize)]
pub struct ProxySettingsRequest {
    enabled: bool,
    proxy_type: String,
    host: String,
    port: u16,
    username: String,
    password: String,
}

/// Apply proxy settings from the frontend.
/// Stores the config in global state so network operations can read it.
#[tauri::command]
pub async fn cmd_apply_proxy_settings(
    req: ProxySettingsRequest,
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let change = if req.password.is_empty() {
        ProxyCredentialUpdate::Keep
    } else {
        ProxyCredentialUpdate::Set(req.password)
    };
    net_config.update_credentials(
        &app,
        move |snapshot| {
            snapshot.proxy = ProxyConfig {
                enabled: req.enabled,
                proxy_type: req.proxy_type,
                host: req.host,
                port: req.port,
                username: req.username,
                password: snapshot.proxy.password.clone(),
            };
            Ok(())
        },
        change,
    )?;
    Ok("Proxy settings applied".into())
}

/// Import a password found in a legacy frontend settings file. The caller may
/// scrub the legacy value only after this command succeeds.
#[tauri::command]
pub async fn cmd_migrate_proxy_secret(
    password: String,
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if password.is_empty() {
        return Ok(());
    }
    net_config.update_credentials(&app, |_| Ok(()), ProxyCredentialUpdate::Import(password))
}

#[tauri::command]
pub async fn cmd_clear_proxy_secret(
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    net_config.update_credentials(&app, |_| Ok(()), ProxyCredentialUpdate::Clear)
}

#[derive(Debug, serde::Deserialize)]
pub struct VpnSettingsRequest {
    enabled: bool,
    timeout_multiplier: u32,
    retry_attempts: u32,
    retry_base_backoff_ms: u64,
    retry_max_backoff_ms: u64,
    adaptive_polling: bool,
    polling_min_sec: u32,
    polling_max_sec: u32,
    preferred_dc: String,
    dc_fallback_attempts: u32,
    flood_wait_respect: bool,
    peer_cache_size: usize,
    bandwidth_limit_up_kbs: u32,
    #[serde(default)]
    bandwidth_schedule: bool,
    #[serde(default)]
    bandwidth_windows: Vec<crate::traffic::Window>,
    bandwidth_limit_down_kbs: u32,
    chunk_size_kb: u32,
    keep_alive_interval_sec: u32,
    auto_detect_vpn: bool,
    archive_max_bytes: u64,
}

/// Apply VPN optimizer settings from the frontend.
/// Stores the config in global state so network operations can read it.
#[tauri::command]
pub async fn cmd_apply_vpn_settings(
    req: VpnSettingsRequest,
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let config = VpnConfig {
        enabled: req.enabled,
        timeout_multiplier: req.timeout_multiplier.clamp(1, 5),
        retry_attempts: req.retry_attempts.clamp(0, 5),
        retry_base_backoff_ms: req.retry_base_backoff_ms.clamp(500, 5000),
        retry_max_backoff_ms: req.retry_max_backoff_ms.clamp(8000, 60000),
        adaptive_polling: req.adaptive_polling,
        polling_min_sec: req.polling_min_sec.clamp(10, 30),
        polling_max_sec: req.polling_max_sec.clamp(45, 120),
        preferred_dc: req.preferred_dc,
        dc_fallback_attempts: req.dc_fallback_attempts.clamp(1, 4),
        flood_wait_respect: req.flood_wait_respect,
        peer_cache_size: req.peer_cache_size.clamp(100, 2000),
        bandwidth_limit_up_kbs: req.bandwidth_limit_up_kbs,
        bandwidth_schedule: req.bandwidth_schedule,
        bandwidth_windows: req.bandwidth_windows,
        bandwidth_limit_down_kbs: req.bandwidth_limit_down_kbs,
        chunk_size_kb: req.chunk_size_kb.clamp(64, 512),
        keep_alive_interval_sec: if req.keep_alive_interval_sec == 0 {
            0
        } else {
            req.keep_alive_interval_sec.clamp(30, 120)
        },
        auto_detect_vpn: req.auto_detect_vpn,
        archive_max_bytes: req.archive_max_bytes,
    };

    log::info!(
        "Applying VPN settings: enabled={}, timeout={}x, retries={}, flood_wait={}",
        config.enabled,
        config.timeout_multiplier,
        config.retry_attempts,
        config.flood_wait_respect
    );

    net_config.update(&app, move |snapshot| {
        snapshot.vpn = config;
        Ok(())
    })?;

    Ok("VPN settings applied".into())
}
