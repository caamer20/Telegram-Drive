use crate::crypto::state::{CryptoState, UnlockSessionId};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use zeroize::{Zeroize, Zeroizing};

const CRYPTO_CONTRACT_VERSION: u16 = 2;
const CRYPTO_BACKEND_BUILD_ID: &str = concat!(env!("CARGO_PKG_VERSION"), "-tdenc2");

#[derive(Debug, Clone, Serialize)]
pub struct CryptoFeatureAvailability {
    pub upload: bool,
    pub read: bool,
    pub per_file_passphrase: bool,
    pub recovery: bool,
    pub share: bool,
    pub migration: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CryptoCapabilitiesResponse {
    pub contract_version: u16,
    pub app_version: String,
    pub backend_build_id: String,
    pub availability: String,
    pub blockers: Vec<String>,
    pub vault_backend: String,
    pub readable_formats: Vec<u16>,
    pub writable_formats: Vec<u16>,
    pub features: CryptoFeatureAvailability,

    // Retained frontend contract fields mirror the current TDENC2 features.
    // Readable/writable format lists independently keep TDENC1 quarantined.
    pub core_available: bool,
    pub mode_alpha: bool,
    pub upload_enabled: bool,
    pub read_enabled: bool,
    pub share_enabled: bool,
    pub migration_enabled: bool,
    pub supported_suites: Vec<u16>,
    pub envelope_version: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct CryptoInventoryEntry {
    pub envelope_version: i64,
    pub file_count: i64,
    pub ciphertext_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CryptoInventory {
    pub entries: Vec<CryptoInventoryEntry>,
    pub total_files: i64,
    pub total_ciphertext_bytes: i64,
    pub vault_exists: bool,
    pub experimental_format_quarantined: bool,
}

/// Encryption settings (non-secret preferences only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionSettings {
    pub default_mode: String, // "standard" | "vault"
    pub protect_metadata: bool,
    pub auto_lock_minutes: u32, // 0 = never (with warning)
    pub lock_on_sleep: bool,
    pub temp_policy: String, // "balanced" | "strict"
    pub remember_device: bool,
}

impl Default for EncryptionSettings {
    fn default() -> Self {
        Self {
            default_mode: "standard".to_string(),
            protect_metadata: true,
            auto_lock_minutes: 15,
            lock_on_sleep: true,
            temp_policy: "balanced".to_string(),
            remember_device: false,
        }
    }
}

/// Vault status response.
#[derive(Debug, Clone, Serialize)]
pub struct VaultStatus {
    pub exists: bool,
    pub is_unlocked: bool,
    pub session_id: Option<UnlockSessionId>,
    pub has_recovery: bool,
    pub created_at: Option<String>,
    /// Non-secret identifier of the unlocked vault; `None` while locked. Lets
    /// the interface bind the recovery drill to the vault it was run against.
    pub vault_id: Option<String>,
}

/// File encryption info for the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct FileEncryptionInfo {
    pub state: String,
    pub envelope_version: Option<u16>,
    pub profile_id: Option<String>,
    pub protection_mode: Option<String>,
    pub metadata_protected: Option<bool>,
    pub ciphertext_size: Option<u64>,
}

/// Protection intent for upload operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectionIntent {
    pub mode: String, // "standard" | "vault" | "passphrase" | "vault_and_passphrase"
    pub profile_id: Option<String>,
    pub protect_metadata: Option<bool>,
}

impl Default for ProtectionIntent {
    fn default() -> Self {
        Self {
            mode: "standard".to_string(),
            profile_id: None,
            protect_metadata: None,
        }
    }
}

// ── Tauri Commands ──────────────────────────────────────────────────

/// Get encryption capabilities (feature flags, supported suites).
#[tauri::command]
pub async fn cmd_get_encryption_capabilities(
    crypto_state: State<'_, CryptoState>,
) -> Result<CryptoCapabilitiesResponse, String> {
    let features = crypto_state.get_features();
    Ok(CryptoCapabilitiesResponse {
        contract_version: CRYPTO_CONTRACT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        backend_build_id: CRYPTO_BACKEND_BUILD_ID.to_string(),
        availability: if features.core_available {
            "ready"
        } else {
            "blocked"
        }
        .to_string(),
        blockers: if features.core_available {
            Vec::new()
        } else {
            vec!["CRYPTO_BACKEND_DISABLED".to_string()]
        },
        vault_backend: "persistent_file".to_string(),
        readable_formats: if features.read_enabled {
            vec![2]
        } else {
            Vec::new()
        },
        writable_formats: if features.upload_enabled {
            vec![2]
        } else {
            Vec::new()
        },
        features: CryptoFeatureAvailability {
            upload: features.upload_enabled,
            read: features.read_enabled,
            per_file_passphrase: features.upload_enabled && features.read_enabled,
            recovery: features.core_available,
            share: features.share_enabled,
            migration: features.migration_enabled,
        },
        core_available: features.core_available,
        mode_alpha: features.mode_alpha,
        upload_enabled: features.upload_enabled,
        read_enabled: features.read_enabled,
        share_enabled: features.share_enabled,
        migration_enabled: features.migration_enabled,
        supported_suites: vec![1],
        envelope_version: 2,
    })
}

/// Read-only inventory of locally indexed encrypted objects. This command is
/// intentionally available while encryption is blocked so experimental
/// ciphertext can be identified and preserved.
#[tauri::command]
pub async fn cmd_get_crypto_inventory(
    app: tauri::AppHandle,
    owner_id: Option<String>,
    crypto_state: State<'_, CryptoState>,
) -> Result<CryptoInventory, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = crate::workspace::AccountGuard::open(&root, owner_id.as_deref())?;
    let groups = tokio::task::spawn_blocking(move || {
        account.validate()?;
        let groups = crate::workspace::envelope_cache::inventory(
            &crate::workspace::store::Store::open(&account.root, account.owner)?,
        )?;
        account.validate()?;
        Ok::<_, String>(groups)
    })
    .await
    .map_err(|e| e.to_string())??;
    let entries: Vec<_> = groups
        .into_iter()
        .map(|(version, count, size)| CryptoInventoryEntry {
            envelope_version: i64::from(version),
            file_count: i64::try_from(count).unwrap_or(i64::MAX),
            ciphertext_bytes: i64::try_from(size).unwrap_or(i64::MAX),
        })
        .collect();
    Ok(CryptoInventory {
        total_files: entries
            .iter()
            .fold(0i64, |total, entry| total.saturating_add(entry.file_count)),
        total_ciphertext_bytes: entries.iter().fold(0i64, |total, entry| {
            total.saturating_add(entry.ciphertext_bytes)
        }),
        experimental_format_quarantined: entries.iter().any(|entry| entry.envelope_version == 1),
        entries,
        vault_exists: crypto_state.vault_exists(),
    })
}

/// Get non-secret encryption settings.
#[tauri::command]
pub async fn cmd_get_encryption_settings() -> Result<EncryptionSettings, String> {
    // In production, load from Tauri store
    Ok(EncryptionSettings::default())
}

/// Update non-secret encryption settings.
#[tauri::command]
pub async fn cmd_update_encryption_settings(
    settings: EncryptionSettings,
    crypto_state: State<'_, CryptoState>,
) -> Result<EncryptionSettings, String> {
    if !matches!(
        settings.default_mode.as_str(),
        "standard" | "vault" | "passphrase" | "vault_and_passphrase"
    ) {
        return Err("[POLICY_REJECTED] Invalid default encryption mode".to_string());
    }
    if !matches!(settings.temp_policy.as_str(), "balanced" | "strict") {
        return Err("[POLICY_REJECTED] Invalid temporary plaintext policy".to_string());
    }
    if settings.auto_lock_minutes > 24 * 60 {
        return Err("[POLICY_REJECTED] Auto-lock timeout exceeds 24 hours".to_string());
    }
    let timeout = if settings.auto_lock_minutes == 0 {
        None
    } else {
        Some(std::time::Duration::from_secs(
            u64::from(settings.auto_lock_minutes) * 60,
        ))
    };
    crypto_state.set_auto_lock_timeout(timeout);
    Ok(settings)
}

/// Create a new vault.
#[tauri::command]
pub async fn cmd_create_vault(
    passphrase: String,
    crypto_state: State<'_, CryptoState>,
    app_handle: tauri::AppHandle,
) -> Result<UnlockSessionId, String> {
    let mut passphrase = Zeroizing::new(passphrase);
    if !crypto_state.get_features().core_available {
        passphrase.zeroize();
        return Err("[ENCRYPTION_BLOCKED] Vault creation is disabled until the production vault and TDENC2 format are ready.".to_string());
    }
    let result = crypto_state
        .create_vault(passphrase.as_bytes())
        .await
        .map_err(|e| e.to_string());
    passphrase.zeroize();
    if result.is_ok() {
        let _ = app_handle.emit("vault-unlocked", ());
    }
    result
}

/// Unlock the vault and get a session handle.
#[tauri::command]
pub async fn cmd_unlock_vault(
    passphrase: String,
    crypto_state: State<'_, CryptoState>,
    app_handle: tauri::AppHandle,
) -> Result<UnlockSessionId, String> {
    let mut passphrase = Zeroizing::new(passphrase);
    let result = crypto_state
        .unlock(passphrase.as_bytes())
        .await
        .map_err(|e| e.to_string());
    passphrase.zeroize();
    if result.is_ok() {
        let _ = app_handle.emit("vault-unlocked", ());
    }
    result
}

#[tauri::command]
pub async fn cmd_change_vault_passphrase(
    current_passphrase: String,
    new_passphrase: String,
    crypto_state: State<'_, CryptoState>,
) -> Result<(), String> {
    let mut current_passphrase = Zeroizing::new(current_passphrase);
    let mut new_passphrase = Zeroizing::new(new_passphrase);
    let result = crypto_state
        .change_vault_passphrase(current_passphrase.as_bytes(), new_passphrase.as_bytes())
        .await
        .map_err(|error| error.to_string());
    current_passphrase.zeroize();
    new_passphrase.zeroize();
    result
}

/// Lock the vault and invalidate all handles.
/// Also revokes all active plaintext leases and clears the encrypted cache.
#[tauri::command]
pub async fn cmd_lock_vault(
    crypto_state: State<'_, CryptoState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    crypto_state.lock();
    // Emit event so frontend refreshes encrypted file states
    let _ = app_handle.emit("vault-locked", ());
    log::info!("Vault locked — all handles revoked, encrypted routes disabled");
    Ok(())
}

/// Record throttled foreground input for the inactivity deadline. Activity
/// arriving after an elapsed deadline locks atomically rather than reviving an
/// expired vault after process or device suspension.
#[tauri::command]
pub async fn cmd_record_vault_activity(
    crypto_state: State<'_, CryptoState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    if crypto_state.record_activity() {
        let _ = app_handle.emit("vault-locked", "auto_lock");
    }
    Ok(())
}

/// Stage a file passphrase behind a short-lived, opaque, single-use token.
/// The returned token is safe to keep in an in-memory queue; the passphrase is
/// consumed by the next upload/download command and is never persisted.
#[tauri::command]
pub async fn cmd_stage_file_passphrase(
    mut passphrase: String,
    crypto_state: State<'_, CryptoState>,
) -> Result<u64, String> {
    if !crypto_state.get_features().core_available {
        passphrase.zeroize();
        return Err("[ENCRYPTION_BLOCKED] Per-file passphrases are not available".to_string());
    }
    let result = crypto_state
        .stage_prompt_secret(passphrase.as_bytes())
        .map_err(|error| error.to_string());
    passphrase.zeroize();
    result
}

/// One vault-protected file, as stored in the local inventory.
#[derive(Debug, Clone, Serialize)]
pub struct VaultFileEntry {
    pub folder_id: Option<i64>,
    pub folder_name: Option<String>,
    pub message_id: i64,
    pub file_name: String,
    pub file_size: i64,
    pub created_at: String,
    pub encryption_state: String,
}

/// List every file that is protected by the vault/passphrase layer. File names
/// are already redacted in the inventory when metadata protection is on.
#[tauri::command]
pub async fn cmd_list_vault_files(
    db: State<'_, crate::db::DbConnection>,
) -> Result<Vec<VaultFileEntry>, String> {
    let database = db.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut statement = conn
            .prepare(
                "SELECT fi.folder_id, fm.name, fi.message_id, fi.file_name, fi.file_size, \
                    fi.created_at, fi.encryption_state \
                 FROM file_inventory fi \
                 LEFT JOIN folder_metadata fm ON fi.folder_id = fm.channel_id \
                 WHERE fi.encryption_state IS NOT NULL AND fi.encryption_state != 'plain' \
                 ORDER BY fi.created_at DESC LIMIT 500",
            )
            .map_err(|error| error.to_string())?;
        let mut files = Vec::new();
        while let sqlite::State::Row = statement.next().map_err(|error| error.to_string())? {
            files.push(VaultFileEntry {
                folder_id: statement.read::<Option<i64>, _>(0).ok().flatten(),
                folder_name: statement.read::<Option<String>, _>(1).ok().flatten(),
                message_id: statement.read::<i64, _>(2).map_err(|error| error.to_string())?,
                file_name: statement.read::<String, _>(3).map_err(|error| error.to_string())?,
                file_size: statement.read::<i64, _>(4).map_err(|error| error.to_string())?,
                created_at: statement
                    .read::<Option<String>, _>(5)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                encryption_state: statement.read::<String, _>(6).map_err(|error| error.to_string())?,
            });
        }
        Ok(files)
    })
    .await
}

/// Get the current vault status.
#[tauri::command]
pub async fn cmd_get_vault_status(
    crypto_state: State<'_, CryptoState>,
) -> Result<VaultStatus, String> {
    let exists = crypto_state.vault_exists();
    let is_locked = crypto_state.is_locked();
    Ok(VaultStatus {
        exists,
        is_unlocked: exists && !is_locked,
        session_id: None,
        has_recovery: false,
        created_at: None,
        vault_id: crypto_state.vault_identity(),
    })
}

/// Export a recovery bundle.
#[tauri::command]
pub async fn cmd_export_vault_recovery(
    recovery_passphrase: String,
    crypto_state: State<'_, CryptoState>,
) -> Result<String, String> {
    let mut recovery_passphrase = Zeroizing::new(recovery_passphrase);
    if !crypto_state.get_features().core_available {
        recovery_passphrase.zeroize();
        return Err("[RECOVERY_UNAVAILABLE] Recovery export is disabled because the current implementation cannot produce a valid recovery bundle.".to_string());
    }
    let bundle_result = crypto_state
        .export_recovery(recovery_passphrase.as_bytes())
        .await
        .map_err(|e| e.to_string());
    recovery_passphrase.zeroize();
    let bundle = bundle_result?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &bundle,
    ))
}

/// Result of checking a recovery bundle against the unlocked vault.
#[derive(Debug, Clone, Serialize)]
pub struct RecoveryVerificationResult {
    pub matches_vault: bool,
    pub missing_profiles: usize,
    pub complete: bool,
}

/// Prove that a saved recovery bundle can restore the unlocked vault. Nothing
/// is written: the vault file, its passphrase and its keys stay untouched.
#[tauri::command]
pub async fn cmd_verify_vault_recovery(
    mut bundle_base64: String,
    recovery_passphrase: String,
    crypto_state: State<'_, CryptoState>,
) -> Result<RecoveryVerificationResult, String> {
    let mut recovery_passphrase = Zeroizing::new(recovery_passphrase);
    if !crypto_state.get_features().core_available {
        bundle_base64.zeroize();
        recovery_passphrase.zeroize();
        return Err("[RECOVERY_UNAVAILABLE] Recovery verification is unavailable.".to_string());
    }
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        bundle_base64.trim(),
    )
    .map_err(|e| format!("Invalid base64: {}", e));
    bundle_base64.zeroize();
    let mut bundle = match decoded {
        Ok(bundle) => bundle,
        Err(error) => {
            recovery_passphrase.zeroize();
            return Err(error);
        }
    };
    let outcome = crypto_state
        .verify_recovery(&bundle, recovery_passphrase.as_bytes())
        .await
        .map_err(|e| e.to_string());
    bundle.zeroize();
    recovery_passphrase.zeroize();
    let result = outcome?;
    Ok(RecoveryVerificationResult {
        matches_vault: result.matches_vault_key,
        missing_profiles: result.missing_profiles,
        complete: result.is_complete(),
    })
}

/// Import a recovery bundle.
///
/// `vault_passphrase` becomes the passphrase of the restored vault. It may be
/// omitted only while the vault is unlocked, in which case the current
/// passphrase is kept. `allow_key_replacement` must be set to replace an
/// unlocked vault with a bundle that lacks keys the vault currently holds.
#[tauri::command]
pub async fn cmd_import_vault_recovery(
    mut bundle_base64: String,
    recovery_passphrase: String,
    vault_passphrase: Option<String>,
    replace_existing: Option<bool>,
    allow_key_replacement: Option<bool>,
    crypto_state: State<'_, CryptoState>,
) -> Result<(), String> {
    let mut recovery_passphrase = Zeroizing::new(recovery_passphrase);
    let mut vault_passphrase = vault_passphrase
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new);
    if !crypto_state.get_features().core_available {
        bundle_base64.zeroize();
        recovery_passphrase.zeroize();
        vault_passphrase.zeroize();
        return Err("[RECOVERY_UNAVAILABLE] Recovery import is disabled until authenticated recovery bundles are implemented.".to_string());
    }
    if crypto_state.vault_exists() && replace_existing != Some(true) {
        bundle_base64.zeroize();
        recovery_passphrase.zeroize();
        vault_passphrase.zeroize();
        return Err(
            "[RECOVERY_CONFIRMATION_REQUIRED] Import would replace the existing vault".to_string(),
        );
    }
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        bundle_base64.trim(),
    )
    .map_err(|e| format!("Invalid base64: {}", e));
    bundle_base64.zeroize();
    let mut bundle = match decoded {
        Ok(bundle) => bundle,
        Err(error) => {
            recovery_passphrase.zeroize();
            vault_passphrase.zeroize();
            return Err(error);
        }
    };

    let result = crypto_state
        .import_recovery(
            &bundle,
            recovery_passphrase.as_bytes(),
            vault_passphrase
                .as_ref()
                .map(|passphrase| passphrase.as_bytes()),
            allow_key_replacement == Some(true),
            replace_existing == Some(true),
        )
        .await
        .map_err(|e| e.to_string());
    bundle.zeroize();
    recovery_passphrase.zeroize();
    vault_passphrase.zeroize();
    result
}

/// Generate a recovery key (256-bit random, displayed to user).
#[tauri::command]
pub async fn cmd_generate_recovery_key() -> Result<String, String> {
    Err("[RECOVERY_UNAVAILABLE] A recovery key cannot be generated until it is cryptographically connected to the vault.".to_string())
}

/// Get file encryption info for a given message.
#[tauri::command]
pub async fn cmd_get_file_encryption_info(
    app: tauri::AppHandle,
    message_id: i32,
    folder_id: Option<i64>,
    owner_id: Option<String>,
    state: State<'_, crate::TelegramState>,
    crypto_state: State<'_, CryptoState>,
) -> Result<FileEncryptionInfo, String> {
    let account = crate::workspace::AccountGuard::open(
        &app.path().app_data_dir().map_err(|e| e.to_string())?,
        owner_id.as_deref(),
    )?;
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Client not connected".to_string())?;
    account.validate_client(&client).await?;
    let peer = crate::commands::utils::resolve_peer(&client, folder_id, &state.peer_cache).await?;
    account.validate()?;
    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    account.validate()?;
    let message = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "Message not found".to_string())?;
    let media = message
        .media()
        .ok_or_else(|| "No media in message".to_string())?;
    let record = crate::commands::fs::resolve_remote_envelope(
        &account,
        &client,
        folder_id,
        message_id,
        &media,
        message.text(),
    )
    .await?;
    account.validate()?;
    Ok(encryption_info_from_current_record(
        record,
        !crypto_state.is_locked(),
    ))
}

fn encryption_info_from_current_record(
    record: Option<crate::crypto::registry::EncryptedFileRecord>,
    vault_unlocked: bool,
) -> FileEncryptionInfo {
    match record {
        Some(record) => FileEncryptionInfo {
            state: if record.envelope_version != crate::crypto::policy::FORMAT_VERSION {
                "encrypted_unsupported_version"
            } else if vault_unlocked
                && matches!(
                    record.protection_mode.as_str(),
                    "vault" | "vault_and_passphrase"
                )
            {
                "encrypted_unlocked"
            } else {
                "encrypted_locked"
            }
            .into(),
            envelope_version: Some(record.envelope_version),
            profile_id: record.key_profile_id,
            protection_mode: Some(record.protection_mode),
            metadata_protected: Some(record.metadata_protected),
            ciphertext_size: Some(record.ciphertext_size),
        },
        None => FileEncryptionInfo {
            state: "plain".into(),
            envelope_version: None,
            profile_id: None,
            protection_mode: None,
            metadata_protected: None,
            ciphertext_size: None,
        },
    }
}
