//! VPN Optimizer & Proxy Configuration
//!
//! Stores runtime network configuration that all network operations read from.
//! When vpnMode is off, helpers return hardcoded defaults (zero behaviour change).
//! When vpnMode is on, helpers return user-configured values.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
use std::sync::RwLock;
use tauri::Manager;

/// Proxy configuration received from the frontend
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub enabled: bool,
    pub proxy_type: String, // "socks5" | "mtproto"
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default, skip_serializing)]
    pub password: String, // Loaded from the OS credential store; never persisted as JSON.
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProxyConfig")
            .field("enabled", &self.enabled)
            .field("proxy_type", &self.proxy_type)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_type: "socks5".into(),
            host: String::new(),
            port: 1080,
            username: String::new(),
            password: String::new(),
        }
    }
}

/// VPN optimizer configuration received from the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnConfig {
    pub enabled: bool,
    pub timeout_multiplier: u32,    // 1–5
    pub retry_attempts: u32,        // 0–5
    pub retry_base_backoff_ms: u64, // 500–5000
    pub retry_max_backoff_ms: u64,  // 8000–60000
    // Retained in saved settings for backward compatibility; polling is not configurable here.
    pub adaptive_polling: bool,
    pub polling_min_sec: u32,      // 10–30
    pub polling_max_sec: u32,      // 45–120
    pub preferred_dc: String,      // "auto" | "dc1"–"dc5"
    pub dc_fallback_attempts: u32, // 1–4
    pub flood_wait_respect: bool,
    pub peer_cache_size: usize, // Compatibility field; no runtime size control.
    pub bandwidth_limit_up_kbs: u32, // 0 = unlimited; effective behind bandwidth_schedule.
    #[serde(default)]
    pub bandwidth_schedule: bool,
    #[serde(default)]
    pub bandwidth_windows: Vec<crate::traffic::Window>,
    pub bandwidth_limit_down_kbs: u32, // 0 = unlimited
    pub chunk_size_kb: u32,            // 128, 256, 512
    pub keep_alive_interval_sec: u32,  // 0 = disabled, 30–120
    pub auto_detect_vpn: bool,         // Compatibility field; detection is explicitly requested.
    pub archive_max_bytes: u64,        // 0 = unlimited, max bytes for bulk archive (API)
}

impl Default for VpnConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_multiplier: 3,
            retry_attempts: 3,
            retry_base_backoff_ms: 1000,
            retry_max_backoff_ms: 30000,
            adaptive_polling: true,
            polling_min_sec: 15,
            polling_max_sec: 60,
            preferred_dc: "auto".into(),
            dc_fallback_attempts: 2,
            flood_wait_respect: true,
            peer_cache_size: 500,
            bandwidth_limit_up_kbs: 0,
            bandwidth_schedule: false,
            bandwidth_windows: Vec::new(),
            bandwidth_limit_down_kbs: 0,
            chunk_size_kb: 512,
            keep_alive_interval_sec: 0,
            auto_detect_vpn: false,
            archive_max_bytes: 256 * 1024 * 1024, // 256 MiB
        }
    }
}

/// Combined network config snapshot (what the frontend receives)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfigSnapshot {
    pub proxy: ProxyConfig,
    pub vpn: VpnConfig,
}

const LEGACY_PASSWORD_REQUIRES_CLEAR: &str = "proxy_legacy_password_requires_clear";

pub enum ProxyCredentialUpdate {
    Keep,
    Set(String),
    Clear,
    Import(String),
}

pub trait ProxySecretStore: Send + Sync {
    fn read(&self) -> Result<Option<String>, String>;
    fn write(&self, value: Option<&str>) -> Result<(), String>;
}
pub struct OsProxySecretStore;
impl ProxySecretStore for OsProxySecretStore {
    fn read(&self) -> Result<Option<String>, String> {
        crate::proxy_secret::load_password()
    }
    fn write(&self, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(password) => crate::proxy_secret::store_password(password),
            None => crate::proxy_secret::delete_password(),
        }
    }
}

struct HttpProxyBridge {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HttpProxyBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl HttpProxyBridge {
    fn prepare(proxy: &ProxyConfig) -> Result<Option<Self>, String> {
        if !proxy.enabled
            || proxy.host.is_empty()
            || !matches!(proxy.proxy_type.as_str(), "http" | "https")
        {
            return Ok(None);
        }
        let (port, task) = crate::socks5_bridge::start_bridge(
            proxy.host.clone(),
            proxy.port,
            proxy.proxy_type.clone(),
            proxy.username.clone(),
            proxy.password.clone(),
        )?;
        Ok(Some(Self { port, task }))
    }
}

/// Thread-safe global state managed via Tauri's state system
pub struct NetworkConfig {
    pub proxy: RwLock<ProxyConfig>,
    pub vpn: RwLock<VpnConfig>,
    pub pacer: crate::traffic::Pacer,
    updates: std::sync::Mutex<()>,
    bridge_handle: std::sync::Mutex<Option<HttpProxyBridge>>,
}

impl NetworkConfig {
    pub fn new() -> Self {
        Self {
            proxy: RwLock::new(ProxyConfig::default()),
            vpn: RwLock::new(VpnConfig::default()),
            pacer: crate::traffic::Pacer::default(),
            updates: std::sync::Mutex::new(()),
            bridge_handle: std::sync::Mutex::new(None),
        }
    }

    pub fn new_with_config(config: NetworkConfigSnapshot) -> Self {
        Self {
            proxy: RwLock::new(config.proxy),
            vpn: RwLock::new(config.vpn),
            pacer: crate::traffic::Pacer::default(),
            updates: std::sync::Mutex::new(()),
            bridge_handle: std::sync::Mutex::new(None),
        }
    }

    /// Serialize updates to the shared file and publish only a durable snapshot.
    pub fn update_at(
        &self,
        path: &std::path::Path,
        patch: impl FnOnce(&mut NetworkConfigSnapshot) -> Result<(), String>,
    ) -> Result<(), String> {
        self.update_credentials_at(
            path,
            patch,
            ProxyCredentialUpdate::Keep,
            &OsProxySecretStore,
        )
    }
    pub fn update_credentials_at(
        &self,
        path: &std::path::Path,
        patch: impl FnOnce(&mut NetworkConfigSnapshot) -> Result<(), String>,
        update: ProxyCredentialUpdate,
        store: &dyn ProxySecretStore,
    ) -> Result<(), String> {
        let _update = self
            .updates
            .lock()
            .map_err(|_| "Network settings unavailable")?;
        let mut snapshot = self.snapshot();
        let previous_proxy = snapshot.proxy.clone();
        patch(&mut snapshot)?;
        if snapshot.vpn.bandwidth_windows.len() > 32
            || snapshot.vpn.bandwidth_limit_up_kbs > 65536
            || snapshot.vpn.bandwidth_limit_down_kbs > 65536
        {
            return Err("Invalid bandwidth limits or too many schedule windows".into());
        }
        for window in &snapshot.vpn.bandwidth_windows {
            window.validate()?;
        }
        // Preserve the last recoverable credential if startup could not migrate
        // it. A user must explicitly clear it before switching proxy identities.
        let legacy = legacy_password_at(path, &snapshot.proxy)?;
        let previous = match &update {
            ProxyCredentialUpdate::Keep => None,
            _ => Some(store.read()?),
        };
        let secure = previous.as_ref().and_then(Option::as_deref);
        let proposed = match &update {
            ProxyCredentialUpdate::Keep => None,
            ProxyCredentialUpdate::Clear => Some(None),
            ProxyCredentialUpdate::Import(password)
                if [legacy.as_deref(), secure]
                    .into_iter()
                    .flatten()
                    .any(|saved| saved != password) =>
            {
                return Err(LEGACY_PASSWORD_REQUIRES_CLEAR.into());
            }
            ProxyCredentialUpdate::Import(password) | ProxyCredentialUpdate::Set(password) => {
                Some(Some(password.as_str()))
            }
        };
        if let Some(value) = proposed {
            snapshot.proxy.password = value.unwrap_or_default().to_owned();
        }
        let replace_bridge = snapshot.proxy != previous_proxy;
        let prepared_bridge = if replace_bridge {
            HttpProxyBridge::prepare(&snapshot.proxy)?
        } else {
            None
        };

        // Validate first, then remember both the old value and its absence.
        // OS credentials and JSON have no shared transaction; on an operation
        // error restore the old credential, and report restoration failure.
        let write_credential = proposed.filter(|value| *value != secure);
        let restore = |error: String| -> String {
            if let Some(previous) = previous.as_ref().filter(|_| write_credential.is_some()) {
                if let Err(restore) = store.write(previous.as_deref()) {
                    return format!(
                        "{error}; prior proxy credential restoration failed: {restore}"
                    );
                }
            }
            error
        };
        if let Some(value) = write_credential {
            if let Err(error) = store.write(value) {
                return Err(restore(error));
            }
        }
        let preserved = if proposed.is_none() { legacy } else { None };
        if let Err(error) = write_network_config_at(path, &snapshot, preserved) {
            return Err(restore(error));
        }
        // Only the update serializer spans external IO. Configuration readers
        // and cancellation can proceed while credentials/storage are waiting.
        let mut proxy = self
            .proxy
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let mut vpn = self.vpn.write().unwrap_or_else(|error| error.into_inner());
        if replace_bridge {
            *self
                .bridge_handle
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = prepared_bridge;
        }
        *proxy = snapshot.proxy;
        *vpn = snapshot.vpn;
        drop(vpn);
        drop(proxy);
        self.pacer.changed();
        Ok(())
    }
    pub fn update_credentials(
        &self,
        app: &tauri::AppHandle,
        patch: impl FnOnce(&mut NetworkConfigSnapshot) -> Result<(), String>,
        update: ProxyCredentialUpdate,
    ) -> Result<(), String> {
        self.update_credentials_at(&settings_path(app)?, patch, update, &OsProxySecretStore)
    }

    pub fn update(
        &self,
        app: &tauri::AppHandle,
        patch: impl FnOnce(&mut NetworkConfigSnapshot) -> Result<(), String>,
    ) -> Result<(), String> {
        self.update_at(&settings_path(app)?, patch)
    }

    pub fn snapshot(&self) -> NetworkConfigSnapshot {
        NetworkConfigSnapshot {
            proxy: self.proxy.read().unwrap().clone(),
            vpn: self.vpn.read().unwrap().clone(),
        }
    }

    pub fn effective_proxy_url(&self) -> Option<String> {
        let proxy = self.proxy.read().unwrap();
        if !proxy.enabled || proxy.host.is_empty() {
            return None;
        }
        if proxy.proxy_type == "socks5" {
            if !proxy.username.is_empty() {
                let encoded_user = urlencoding::encode(&proxy.username);
                let encoded_pass = urlencoding::encode(&proxy.password);
                Some(format!(
                    "socks5://{}:{}@{}:{}",
                    encoded_user, encoded_pass, proxy.host, proxy.port
                ))
            } else {
                Some(format!("socks5://{}:{}", proxy.host, proxy.port))
            }
        } else if proxy.proxy_type == "http" || proxy.proxy_type == "https" {
            let guard = self.bridge_handle.lock().unwrap();
            guard
                .as_ref()
                .map(|bridge| format!("socks5://127.0.0.1:{}", bridge.port))
        } else {
            None
        }
    }

    pub fn start_http_bridge(&self) -> Result<(), String> {
        let _update = self
            .updates
            .lock()
            .map_err(|_| "Network settings unavailable")?;
        let route = HttpProxyBridge::prepare(
            &self
                .proxy
                .read()
                .map_err(|_| "Proxy settings unavailable")?
                .clone(),
        )?;
        *self
            .bridge_handle
            .lock()
            .map_err(|_| "Proxy bridge unavailable")? = route;
        Ok(())
    }

    pub fn stop_http_bridge(&self) {
        self.bridge_handle.lock().unwrap().take();
    }

    // ── Helpers that return effective values ────────────────

    /// Network connect timeout in seconds. Default 5s, multiplied when VPN mode on.
    pub fn connect_timeout_secs(&self) -> u64 {
        let vpn = self.vpn.read().unwrap();
        if vpn.enabled {
            5 * vpn.timeout_multiplier as u64
        } else {
            5
        }
    }

    /// How many retry attempts for API calls. Default 0 (no retry) when VPN off.
    pub fn retry_attempts(&self) -> u32 {
        let vpn = self.vpn.read().unwrap();
        // One reliability retry is always available so mandatory Telegram
        // FLOOD_WAIT cooling periods can resume even when VPN tuning is off.
        if vpn.enabled {
            vpn.retry_attempts
        } else {
            1
        }
    }

    /// Base backoff duration in milliseconds for retries.
    pub fn retry_base_backoff_ms(&self) -> u64 {
        let vpn = self.vpn.read().unwrap();
        if vpn.enabled {
            vpn.retry_base_backoff_ms
        } else {
            1000
        }
    }

    /// Max backoff duration in milliseconds for retries.
    pub fn retry_max_backoff_ms(&self) -> u64 {
        let vpn = self.vpn.read().unwrap();
        if vpn.enabled {
            vpn.retry_max_backoff_ms
        } else {
            30000
        }
    }

    /// Whether to automatically sleep on FLOOD_WAIT errors.
    pub fn should_respect_flood_wait(&self) -> bool {
        let vpn = self.vpn.read().unwrap();
        // Telegram rate limits must be honored on every network. The setting
        // remains an advanced override while VPN mode is active.
        if vpn.enabled {
            vpn.flood_wait_respect
        } else {
            true
        }
    }

    /// Whether proxy is active and has a valid host.
    pub fn is_proxy_active(&self) -> bool {
        let proxy = self.proxy.read().unwrap();
        proxy.enabled && !proxy.host.is_empty()
    }

    /// Get proxy address as "host:port" if active.
    pub fn proxy_addr(&self) -> Option<String> {
        let proxy = self.proxy.read().unwrap();
        if proxy.enabled && !proxy.host.is_empty() {
            Some(format!("{}:{}", proxy.host, proxy.port))
        } else {
            None
        }
    }

    /// Download bandwidth limit in bytes/sec. 0 = unlimited.
    pub fn download_limit_bytes_per_sec(&self) -> u64 {
        let vpn = self.vpn.read().unwrap();
        if vpn.bandwidth_limit_down_kbs > 0 {
            vpn.bandwidth_limit_down_kbs as u64 * 1024
        } else {
            0 // unlimited
        }
    }

    /// Chunk size in bytes for transfers.
    pub fn chunk_size_bytes(&self) -> usize {
        let vpn = self.vpn.read().unwrap();
        if vpn.enabled {
            (vpn.chunk_size_kb as usize) * 1024
        } else {
            512 * 1024 // default 512KB
        }
    }

    /// Keep-alive ping interval in seconds. 0 = disabled.
    pub fn keep_alive_interval_sec(&self) -> u32 {
        let vpn = self.vpn.read().unwrap();
        if vpn.enabled {
            vpn.keep_alive_interval_sec
        } else {
            0
        }
    }

    /// Maximum total uncompressed bytes for a single bulk archive (API).
    /// 0 = unlimited.
    pub fn archive_max_bytes(&self) -> u64 {
        let vpn = self.vpn.read().unwrap();
        if vpn.enabled {
            vpn.archive_max_bytes // 0 = unlimited when VPN is on
        } else {
            256 * 1024 * 1024 // default 256 MiB when VPN off
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute exponential backoff with jitter for a given attempt.
/// Returns duration in milliseconds.
pub fn backoff_ms(attempt: u32, base_ms: u64, max_ms: u64) -> u64 {
    let exp = base_ms.saturating_mul(1u64 << attempt.min(10));
    let capped = exp.min(max_ms);
    // Add ~25% jitter
    let jitter = (capped as f64 * 0.25 * rand::random::<f64>()) as u64;
    capped + jitter
}

fn settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("network_settings.json"))
}

pub fn load_network_config(app: &tauri::AppHandle) -> NetworkConfigSnapshot {
    let path = match settings_path(app) {
        Ok(p) => p,
        Err(_) => {
            return NetworkConfigSnapshot {
                proxy: ProxyConfig::default(),
                vpn: VpnConfig::default(),
            }
        }
    };
    load_network_config_at(&path, &OsProxySecretStore)
}

pub fn load_network_config_at(
    path: &std::path::Path,
    store: &impl ProxySecretStore,
) -> NetworkConfigSnapshot {
    let mut snapshot = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|_| NetworkConfigSnapshot {
            proxy: ProxyConfig::default(),
            vpn: VpnConfig::default(),
        }),
        Err(_) => NetworkConfigSnapshot {
            proxy: ProxyConfig::default(),
            vpn: VpnConfig::default(),
        },
    };

    // Keep the original route usable until a legacy conflict can be resolved
    // explicitly. Never overwrite a different secure credential during startup.
    if snapshot.proxy.password.is_empty() {
        match store.read() {
            Ok(Some(password)) => snapshot.proxy.password = password,
            Ok(None) => {}
            Err(error) => log::error!("Unable to load the saved proxy credential: {error}"),
        }
    } else {
        let migration = match store.read() {
            Ok(Some(saved)) if saved != snapshot.proxy.password => {
                log::warn!("Conflicting saved proxy credentials require an explicit choice.");
                return snapshot;
            }
            Ok(Some(_)) => Ok(()),
            Ok(None) => store.write(Some(&snapshot.proxy.password)),
            Err(error) => Err(error),
        };
        match migration {
            Ok(()) => {
                if let Err(error) = write_network_config_at(path, &snapshot, None) {
                    log::error!(
                        "Unable to scrub migrated proxy credentials from legacy settings: {error}"
                    );
                }
            }
            Err(error) => {
                log::error!("Proxy credential migration could not use secure storage: {error}")
            }
        }
    }

    snapshot
}

fn legacy_password_at(
    path: &std::path::Path,
    proxy: &ProxyConfig,
) -> Result<Option<String>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut saved: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Saved network settings could not be read.")?;
    if saved["proxy"]["password"]
        .as_str()
        .unwrap_or_default()
        .is_empty()
    {
        return Ok(None);
    }
    let original: ProxyConfig = serde_json::from_value(saved["proxy"].take())
        .map_err(|_| "Saved proxy settings could not be read.")?;
    if original.host != proxy.host
        || original.port != proxy.port
        || original.proxy_type != proxy.proxy_type
        || original.username != proxy.username
    {
        return Err(LEGACY_PASSWORD_REQUIRES_CLEAR.into());
    }
    Ok(Some(original.password))
}

fn write_network_config_at(
    path: &std::path::Path,
    config: &NetworkConfigSnapshot,
    legacy: Option<String>,
) -> Result<(), String> {
    let parent = path.parent().ok_or("Network settings path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let json = if let Some(password) = legacy {
        let mut document = serde_json::to_value(config).map_err(|error| error.to_string())?;
        document["proxy"]["password"] = serde_json::Value::String(password);
        serde_json::to_string_pretty(&document)
    } else {
        serde_json::to_string_pretty(config)
    }
    .map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(".network-settings.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(json.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        replace_network_settings_file(&temporary, path).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn replace_network_settings_file(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(target_os = "windows")]
fn replace_network_settings_file(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    fn wide(path: &std::path::Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let source = wide(source);
    let destination = wide(destination);
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
