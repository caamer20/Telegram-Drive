use std::path::PathBuf;
use std::sync::Arc;

use rand::Rng;
use serde::Serialize;
use tauri::Manager;
use tokio::sync::Mutex;
use zeroize::Zeroize;

use super::crypto::{
    b64_decode, derive_unlock_key, random_key, unwrap_key, wrap_key, KdfParams, KEY_LEN,
};
use super::format::{
    decrypt_manifest, encrypt_manifest, header_aad, read_encrypted_manifest,
    write_encrypted_manifest, VaultConfig,
};
use super::manifest::VaultManifest;

#[derive(Clone)]
pub struct VaultRuntime {
    pub inner: Arc<Mutex<Option<UnlockedVault>>>,
}

impl Default for VaultRuntime {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Debug)]
pub struct UnlockedVault {
    pub config: VaultConfig,
    pub master_key: [u8; KEY_LEN],
    pub manifest: VaultManifest,
}

impl Drop for UnlockedVault {
    fn drop(&mut self) {
        self.master_key.zeroize();
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub configured: bool,
    pub unlocked: bool,
    pub vault_id: Option<String>,
    pub generation: Option<u64>,
}

pub fn vault_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))?
        .join("vault"))
}

pub fn vault_cache_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let path = app_handle
        .path()
        .app_cache_dir()
        .map_err(|e| format!("Failed to resolve app cache directory: {}", e))?
        .join("vault");
    std::fs::create_dir_all(&path).map_err(|e| format!("Failed to create vault cache: {}", e))?;
    Ok(path)
}

pub fn config_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(vault_dir(app_handle)?.join("vault.json"))
}

pub fn manifest_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(vault_dir(app_handle)?.join("manifest.tdv"))
}

pub fn load_config(app_handle: &tauri::AppHandle) -> Result<VaultConfig, String> {
    let path = config_path(app_handle)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("Failed to read vault config: {}", e))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Failed to parse vault config: {}", e))
}

pub fn save_config(app_handle: &tauri::AppHandle, config: &VaultConfig) -> Result<(), String> {
    let path = config_path(app_handle)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create vault dir: {}", e))?;
    }
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|e| format!("Failed to serialize vault config: {}", e))?;
    std::fs::write(path, bytes).map_err(|e| format!("Failed to write vault config: {}", e))
}

pub fn config_exists(app_handle: &tauri::AppHandle) -> bool {
    config_path(app_handle)
        .map(|path| path.exists())
        .unwrap_or(false)
}

pub fn make_config(
    password: &str,
    vault_id: String,
    bucket_id: i64,
) -> Result<(VaultConfig, [u8; KEY_LEN]), String> {
    let kdf = KdfParams::default();
    let salt = super::crypto::random_bytes::<16>();
    let mut unlock_key = derive_unlock_key(password, &salt, &kdf)?;
    let master_key = random_key();
    let wrapped_master_key = wrap_key(&unlock_key, &master_key, &header_aad(&vault_id))?;
    unlock_key.zeroize();
    Ok((
        VaultConfig {
            version: 1,
            vault_id,
            bucket_id,
            kdf,
            salt: super::crypto::b64_encode(&salt),
            wrapped_master_key,
            latest_manifest_message_id: None,
        },
        master_key,
    ))
}

pub fn unlock_from_disk(
    app_handle: &tauri::AppHandle,
    password: &str,
) -> Result<UnlockedVault, String> {
    let config = load_config(app_handle)?;
    if config.version != 1 {
        return Err(format!(
            "Unsupported vault config version: {}",
            config.version
        ));
    }
    let salt = b64_decode(&config.salt)?;
    let mut unlock_key = derive_unlock_key(password, &salt, &config.kdf)?;
    let master_key = unwrap_key(
        &unlock_key,
        &config.wrapped_master_key,
        &header_aad(&config.vault_id),
    )?;
    unlock_key.zeroize();

    let encrypted_manifest = read_encrypted_manifest(&manifest_path(app_handle)?)?;
    let manifest = decrypt_manifest(&master_key, &encrypted_manifest)?;
    if manifest.vault_id != config.vault_id || manifest.bucket_id != config.bucket_id {
        return Err("Vault manifest does not match local vault config".to_string());
    }

    Ok(UnlockedVault {
        config,
        master_key,
        manifest,
    })
}

pub fn save_local_manifest(
    app_handle: &tauri::AppHandle,
    vault: &UnlockedVault,
) -> Result<(), String> {
    let encrypted = encrypt_manifest(&vault.master_key, &vault.manifest)?;
    write_encrypted_manifest(&manifest_path(app_handle)?, &encrypted)
}

pub fn random_positive_id() -> i64 {
    const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
    loop {
        let raw = rand::thread_rng().gen_range(1..=JS_MAX_SAFE_INTEGER);
        if raw > 0 {
            return raw;
        }
    }
}
