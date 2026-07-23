//! Tauri commands for applying proxy and VPN optimizer settings.
//! These are called from the frontend when the user changes network configuration.

use crate::vpn_optimizer::{NetworkConfig, NetworkConfigSnapshot, ProxyConfig, VpnConfig};
use tauri::State;

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
    let config = ProxyConfig {
        enabled: req.enabled,
        proxy_type: req.proxy_type,
        host: req.host,
        port: req.port,
        username: req.username,
        password: req.password,
    };

    log::info!(
        "Applying proxy settings: enabled={}, type={}, host={}:{}",
        config.enabled,
        config.proxy_type,
        config.host,
        config.port
    );

    *net_config.proxy.write().map_err(|e| e.to_string())? = config.clone();

    // Start or stop local SOCKS5 bridge as needed
    if config.enabled && (config.proxy_type == "http" || config.proxy_type == "https") {
        if let Err(e) = net_config.start_http_bridge().await {
            log::error!("Failed to start SOCKS5 bridge for HTTP/HTTPS proxy: {}", e);
            return Err(format!("Failed to start SOCKS5 bridge: {}", e));
        }
    } else {
        net_config.stop_http_bridge();
    }

    let snapshot = net_config.snapshot();
    if let Err(e) = crate::vpn_optimizer::save_network_config(&app, &snapshot) {
        log::error!("Failed to save proxy settings to disk: {}", e);
    }

    Ok("Proxy settings applied".into())
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

    *net_config.vpn.write().map_err(|e| e.to_string())? = config;

    let snapshot = net_config.snapshot();
    if let Err(e) = crate::vpn_optimizer::save_network_config(&app, &snapshot) {
        log::error!("Failed to save VPN settings to disk: {}", e);
    }

    Ok("VPN settings applied".into())
}

/// Get current network configuration snapshot (called on startup / settings load).
#[tauri::command]
pub async fn cmd_get_network_config(
    net_config: State<'_, std::sync::Arc<NetworkConfig>>,
) -> Result<NetworkConfigSnapshot, String> {
    Ok(net_config.snapshot())
}
