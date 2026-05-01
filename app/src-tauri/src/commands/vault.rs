use std::path::Path;

use base64::{engine::general_purpose, Engine as _};
use grammers_client::Client;
use tauri::{Emitter, State};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::bandwidth::BandwidthManager;
use crate::commands::TelegramState;
use crate::models::{FileMetadata, FolderMetadata};
use crate::vault::cache::decrypt_file_to_cache;
use crate::vault::crypto::{random_key, unwrap_key, wrap_key};
use crate::vault::format::{
    decrypt_file_to_path, encrypt_file_to_path, file_key_aad, write_encrypted_manifest,
};
use crate::vault::manifest::{FileRecord, FolderRecord, VaultManifest};
use crate::vault::state::{
    config_exists, load_config, make_config, manifest_path, random_positive_id, save_config,
    save_local_manifest, unlock_from_disk, vault_cache_dir, UnlockedVault, VaultRuntime,
    VaultStatus,
};
use crate::vault::{format, storage};

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    id: String,
    percent: u8,
}

async fn connected_client(state: &State<'_, TelegramState>) -> Result<Client, String> {
    state
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Telegram client is not connected".to_string())
}

pub async fn persist_manifest(
    app_handle: &tauri::AppHandle,
    client: &Client,
    vault: &mut UnlockedVault,
) -> Result<(), String> {
    let encrypted = format::encrypt_manifest(&vault.master_key, &vault.manifest)?;
    let path = manifest_path(app_handle)?;
    write_encrypted_manifest(&path, &encrypted)?;
    let message_id = storage::upload_object(
        client,
        vault.config.bucket_id,
        &path,
        format!(
            "tdv1 manifest {} {}",
            vault.config.vault_id, vault.manifest.generation
        ),
    )
    .await?;
    vault.config.latest_manifest_message_id = Some(message_id);
    save_config(app_handle, &vault.config)
}

fn path_file_name(path: &str) -> Result<String, String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .ok_or_else(|| "Selected path has no valid filename".to_string())
}

fn file_ext(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_string())
}

fn folder_to_metadata(folder: &FolderRecord) -> FileMetadata {
    FileMetadata {
        id: folder.id,
        folder_id: folder.parent_id,
        name: folder.name.clone(),
        size: 0,
        mime_type: None,
        file_ext: None,
        created_at: folder.created_at.clone(),
        icon_type: "folder".to_string(),
    }
}

fn file_to_metadata(file: &FileRecord) -> FileMetadata {
    FileMetadata {
        id: file.id,
        folder_id: file.folder_id,
        name: file.name.clone(),
        size: file.size,
        mime_type: file.mime_type.clone(),
        file_ext: file.file_ext.clone(),
        created_at: file.created_at.clone(),
        icon_type: "file".to_string(),
    }
}

fn ensure_unique_id(manifest: &VaultManifest) -> i64 {
    loop {
        let id = random_positive_id();
        if !manifest.files.iter().any(|file| file.id == id)
            && !manifest.folders.iter().any(|folder| folder.id == id)
        {
            return id;
        }
    }
}

fn image_mime_from_name(name: &str) -> Option<&'static str> {
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())?
        .to_ascii_lowercase();

    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn image_mime_from_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }

    let sample_len = bytes.len().min(512);
    let sample = String::from_utf8_lossy(&bytes[..sample_len]).to_ascii_lowercase();
    if sample.contains("<svg") {
        return Some("image/svg+xml");
    }
    None
}

fn cached_preview_response(cached: crate::vault::cache::CachedVaultFile) -> Result<String, String> {
    if let Some(name_mime) = image_mime_from_name(&cached.name) {
        let bytes =
            std::fs::read(&cached.path).map_err(|e| format!("Failed to read preview: {}", e))?;
        let mime = image_mime_from_bytes(&bytes).unwrap_or(name_mime);
        let b64 = general_purpose::STANDARD.encode(&bytes);
        return Ok(format!("data:{};base64,{}", mime, b64));
    }

    Ok(cached.path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn cmd_vault_status(
    app_handle: tauri::AppHandle,
    runtime: State<'_, VaultRuntime>,
) -> Result<VaultStatus, String> {
    let guard = runtime.inner.lock().await;
    Ok(VaultStatus {
        configured: config_exists(&app_handle),
        unlocked: guard.is_some(),
        vault_id: guard.as_ref().map(|vault| vault.config.vault_id.clone()),
        generation: guard.as_ref().map(|vault| vault.manifest.generation),
    })
}

#[tauri::command]
pub async fn cmd_vault_create(
    app_handle: tauri::AppHandle,
    password: String,
    telegram_state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<VaultStatus, String> {
    if password.len() < 10 {
        return Err("Vault password must be at least 10 characters.".to_string());
    }
    if config_exists(&app_handle) {
        return Err("A vault is already configured on this device.".to_string());
    }

    let client = connected_client(&telegram_state).await?;
    let vault_id = Uuid::new_v4().to_string();
    let bucket_id = storage::create_ciphertext_bucket(&client, &vault_id).await?;
    let (config, master_key) = make_config(&password, vault_id.clone(), bucket_id)?;
    let manifest = VaultManifest::new(vault_id, bucket_id);
    let mut vault = UnlockedVault {
        config,
        master_key,
        manifest,
    };

    save_local_manifest(&app_handle, &vault)?;
    persist_manifest(&app_handle, &client, &mut vault).await?;

    let status = VaultStatus {
        configured: true,
        unlocked: true,
        vault_id: Some(vault.config.vault_id.clone()),
        generation: Some(vault.manifest.generation),
    };
    *runtime.inner.lock().await = Some(vault);
    Ok(status)
}

#[tauri::command]
pub async fn cmd_vault_unlock(
    app_handle: tauri::AppHandle,
    password: String,
    telegram_state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<VaultStatus, String> {
    if !config_exists(&app_handle) {
        return Err("No vault is configured on this device.".to_string());
    }

    let local_manifest = manifest_path(&app_handle)?;
    if !local_manifest.exists() {
        let config = load_config(&app_handle)?;
        let manifest_message_id = config.latest_manifest_message_id.ok_or_else(|| {
            "Local manifest missing and no remote manifest is recorded".to_string()
        })?;
        let client = connected_client(&telegram_state).await?;
        storage::download_object(
            &client,
            config.bucket_id,
            manifest_message_id,
            &local_manifest,
        )
        .await?;
    }

    let vault = unlock_from_disk(&app_handle, &password)?;
    let status = VaultStatus {
        configured: true,
        unlocked: true,
        vault_id: Some(vault.config.vault_id.clone()),
        generation: Some(vault.manifest.generation),
    };
    *runtime.inner.lock().await = Some(vault);
    Ok(status)
}

#[tauri::command]
pub async fn cmd_vault_lock(
    app_handle: tauri::AppHandle,
    runtime: State<'_, VaultRuntime>,
) -> Result<bool, String> {
    *runtime.inner.lock().await = None;
    if let Ok(cache_dir) = vault_cache_dir(&app_handle) {
        let _ = std::fs::remove_dir_all(cache_dir);
    }
    Ok(true)
}

#[tauri::command]
pub async fn cmd_vault_create_folder(
    name: String,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<FolderMetadata, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Folder name cannot be empty".to_string());
    }

    let client = connected_client(&state).await?;
    let mut guard = runtime.inner.lock().await;
    let vault = guard
        .as_mut()
        .ok_or_else(|| "Vault is locked. Unlock it before creating folders.".to_string())?;

    let folder = FolderRecord {
        id: ensure_unique_id(&vault.manifest),
        parent_id: None,
        name: trimmed.to_string(),
        created_at: VaultManifest::now(),
    };
    vault.manifest.folders.push(folder.clone());
    vault.manifest.touch_generation();
    persist_manifest(&app_handle, &client, vault).await?;

    Ok(FolderMetadata {
        id: folder.id,
        parent_id: folder.parent_id,
        name: folder.name,
    })
}

#[tauri::command]
pub async fn cmd_vault_delete_folder(
    folder_id: i64,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<bool, String> {
    let client = connected_client(&state).await?;
    let mut guard = runtime.inner.lock().await;
    let vault = guard
        .as_mut()
        .ok_or_else(|| "Vault is locked. Unlock it before deleting folders.".to_string())?;

    if vault
        .manifest
        .files
        .iter()
        .any(|file| file.folder_id == Some(folder_id))
        || vault
            .manifest
            .folders
            .iter()
            .any(|folder| folder.parent_id == Some(folder_id))
    {
        return Err("Folder must be empty before it can be deleted.".to_string());
    }

    let before = vault.manifest.folders.len();
    vault
        .manifest
        .folders
        .retain(|folder| folder.id != folder_id);
    if vault.manifest.folders.len() == before {
        return Err("Folder not found".to_string());
    }

    vault.manifest.touch_generation();
    persist_manifest(&app_handle, &client, vault).await?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_vault_upload_file(
    path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    let source_path = Path::new(&path);
    let metadata = std::fs::metadata(source_path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Only regular files can be uploaded".to_string());
    }
    bw_state.can_transfer(metadata.len())?;

    let tid = transfer_id.unwrap_or_default();
    if !tid.is_empty() {
        let _ = app_handle.emit(
            "upload-progress",
            ProgressPayload {
                id: tid.clone(),
                percent: 0,
            },
        );
    }

    let client = connected_client(&state).await?;
    let cache_dir = vault_cache_dir(&app_handle)?;
    let encrypted_temp = tempfile::Builder::new()
        .prefix("tdv1-upload-")
        .suffix(".blob")
        .tempfile_in(cache_dir)
        .map_err(|e| format!("Failed to create encrypted temp file: {}", e))?;
    let encrypted_path = encrypted_temp.path().to_path_buf();

    let mut guard = runtime.inner.lock().await;
    let vault = guard
        .as_mut()
        .ok_or_else(|| "Vault is locked. Unlock it before uploading files.".to_string())?;
    if !vault.manifest.folder_exists(folder_id) {
        return Err("Target folder not found".to_string());
    }

    let file_id = ensure_unique_id(&vault.manifest);
    let mut file_key = random_key();
    let stats = encrypt_file_to_path(
        source_path,
        &encrypted_path,
        &vault.config.vault_id,
        file_id,
        &file_key,
    )?;

    let blob_message_id = storage::upload_object(
        &client,
        vault.config.bucket_id,
        &encrypted_path,
        format!("tdv1 blob {} {}", vault.config.vault_id, file_id),
    )
    .await?;

    let wrapped_file_key = wrap_key(
        &vault.master_key,
        &file_key,
        &file_key_aad(&vault.config.vault_id, file_id),
    )?;
    file_key.zeroize();

    let now = VaultManifest::now();
    let name = path_file_name(&path)?;
    let record = FileRecord {
        id: file_id,
        folder_id,
        name: name.clone(),
        size: stats.plaintext_size,
        mime_type: None,
        file_ext: file_ext(&name),
        created_at: now.clone(),
        modified_at: now,
        blob_message_id,
        ciphertext_size: stats.ciphertext_size,
        chunk_size: stats.chunk_size,
        chunk_count: stats.chunk_count,
        wrapped_file_key,
    };

    vault.manifest.files.push(record);
    vault.manifest.touch_generation();
    persist_manifest(&app_handle, &client, vault).await?;
    bw_state.add_up(stats.ciphertext_size);

    if !tid.is_empty() {
        let _ = app_handle.emit(
            "upload-progress",
            ProgressPayload {
                id: tid,
                percent: 100,
            },
        );
    }

    Ok("Encrypted file uploaded successfully".to_string())
}

#[tauri::command]
pub async fn cmd_vault_delete_file(
    message_id: i64,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<bool, String> {
    let client = connected_client(&state).await?;
    let mut guard = runtime.inner.lock().await;
    let vault = guard
        .as_mut()
        .ok_or_else(|| "Vault is locked. Unlock it before deleting files.".to_string())?;

    let Some(index) = vault
        .manifest
        .files
        .iter()
        .position(|file| file.id == message_id && file.folder_id == folder_id)
    else {
        return Err("File not found".to_string());
    };

    let record = vault.manifest.files.remove(index);
    vault.manifest.touch_generation();
    persist_manifest(&app_handle, &client, vault).await?;
    let _ = storage::delete_object(&client, vault.config.bucket_id, record.blob_message_id).await;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_vault_download_file(
    message_id: i64,
    save_path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    let tid = transfer_id.unwrap_or_default();
    if !tid.is_empty() {
        let _ = app_handle.emit(
            "download-progress",
            ProgressPayload {
                id: tid.clone(),
                percent: 0,
            },
        );
    }

    let client = connected_client(&state).await?;
    let cache_dir = vault_cache_dir(&app_handle)?;
    let encrypted_temp = tempfile::Builder::new()
        .prefix("tdv1-download-")
        .suffix(".blob")
        .tempfile_in(cache_dir)
        .map_err(|e| format!("Failed to create ciphertext temp file: {}", e))?;
    let encrypted_path = encrypted_temp.path().to_path_buf();

    let guard = runtime.inner.lock().await;
    let vault = guard
        .as_ref()
        .ok_or_else(|| "Vault is locked. Unlock it before downloading files.".to_string())?;
    let record = vault
        .manifest
        .files
        .iter()
        .find(|file| file.id == message_id && file.folder_id == folder_id)
        .ok_or_else(|| "File not found".to_string())?
        .clone();

    bw_state.can_transfer(record.ciphertext_size)?;
    storage::download_object(
        &client,
        vault.config.bucket_id,
        record.blob_message_id,
        &encrypted_path,
    )
    .await?;

    let mut file_key = unwrap_key(
        &vault.master_key,
        &record.wrapped_file_key,
        &file_key_aad(&vault.config.vault_id, record.id),
    )?;
    decrypt_file_to_path(
        &encrypted_path,
        Path::new(&save_path),
        &vault.config.vault_id,
        record.id,
        &file_key,
    )?;
    file_key.zeroize();
    bw_state.add_down(record.ciphertext_size);

    if !tid.is_empty() {
        let _ = app_handle.emit(
            "download-progress",
            ProgressPayload {
                id: tid,
                percent: 100,
            },
        );
    }

    Ok("Encrypted file downloaded successfully".to_string())
}

#[tauri::command]
pub async fn cmd_vault_move_files(
    message_ids: Vec<i64>,
    source_folder_id: Option<i64>,
    target_folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<bool, String> {
    if source_folder_id == target_folder_id {
        return Ok(true);
    }

    let client = connected_client(&state).await?;
    let mut guard = runtime.inner.lock().await;
    let vault = guard
        .as_mut()
        .ok_or_else(|| "Vault is locked. Unlock it before moving files.".to_string())?;
    if !vault.manifest.folder_exists(target_folder_id) {
        return Err("Target folder not found".to_string());
    }

    let mut moved = 0usize;
    for id in message_ids {
        if let Some(file) = vault
            .manifest
            .files
            .iter_mut()
            .find(|file| file.id == id && file.folder_id == source_folder_id)
        {
            file.folder_id = target_folder_id;
            file.modified_at = VaultManifest::now();
            moved += 1;
        }
    }

    if moved > 0 {
        vault.manifest.touch_generation();
        persist_manifest(&app_handle, &client, vault).await?;
    }
    Ok(true)
}

#[tauri::command]
pub async fn cmd_vault_get_files(
    folder_id: Option<i64>,
    runtime: State<'_, VaultRuntime>,
) -> Result<Vec<FileMetadata>, String> {
    let guard = runtime.inner.lock().await;
    let vault = guard
        .as_ref()
        .ok_or_else(|| "Vault is locked. Unlock it before listing files.".to_string())?;

    let mut entries = Vec::new();
    entries.extend(
        vault
            .manifest
            .folders
            .iter()
            .filter(|folder| folder.parent_id == folder_id)
            .map(folder_to_metadata),
    );
    entries.extend(
        vault
            .manifest
            .files
            .iter()
            .filter(|file| file.folder_id == folder_id)
            .map(file_to_metadata),
    );
    Ok(entries)
}

#[tauri::command]
pub async fn cmd_vault_search_global(
    query: String,
    runtime: State<'_, VaultRuntime>,
) -> Result<Vec<FileMetadata>, String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let guard = runtime.inner.lock().await;
    let vault = guard
        .as_ref()
        .ok_or_else(|| "Vault is locked. Unlock it before searching files.".to_string())?;

    let mut results: Vec<FileMetadata> = vault
        .manifest
        .folders
        .iter()
        .filter(|folder| folder.name.to_lowercase().contains(&query))
        .map(folder_to_metadata)
        .collect();
    results.extend(
        vault
            .manifest
            .files
            .iter()
            .filter(|file| file.name.to_lowercase().contains(&query))
            .map(file_to_metadata),
    );
    Ok(results)
}

#[tauri::command]
pub async fn cmd_vault_scan_folders(
    runtime: State<'_, VaultRuntime>,
) -> Result<Vec<FolderMetadata>, String> {
    let guard = runtime.inner.lock().await;
    let vault = guard
        .as_ref()
        .ok_or_else(|| "Vault is locked. Unlock it before syncing folders.".to_string())?;

    Ok(vault
        .manifest
        .folders
        .iter()
        .map(|folder| FolderMetadata {
            id: folder.id,
            parent_id: folder.parent_id,
            name: folder.name.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn cmd_vault_get_preview(
    message_id: i64,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<String, String> {
    let client = connected_client(&state).await?;
    let cache_dir = vault_cache_dir(&app_handle)?;
    let cached = decrypt_file_to_cache(
        &client,
        runtime.inner(),
        &cache_dir,
        message_id,
        folder_id,
        "previews",
    )
    .await?;
    cached_preview_response(cached)
}

#[tauri::command]
pub async fn cmd_vault_get_thumbnail(
    message_id: i64,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    runtime: State<'_, VaultRuntime>,
) -> Result<String, String> {
    let client = connected_client(&state).await?;
    let cache_dir = vault_cache_dir(&app_handle)?;
    let cached = decrypt_file_to_cache(
        &client,
        runtime.inner(),
        &cache_dir,
        message_id,
        folder_id,
        "thumbnails",
    )
    .await?;
    cached_preview_response(cached)
}

#[cfg(test)]
mod tests {
    use super::{image_mime_from_bytes, image_mime_from_name};

    #[test]
    fn detects_common_image_mime_types() {
        assert_eq!(image_mime_from_name("photo.JPG"), Some("image/jpeg"));
        assert_eq!(image_mime_from_name("graphic.png"), Some("image/png"));
        assert_eq!(image_mime_from_name("archive.bin"), None);
        assert_eq!(
            image_mime_from_bytes(&[0xff, 0xd8, 0xff, 0x00]),
            Some("image/jpeg")
        );
        assert_eq!(
            image_mime_from_bytes(b"\x89PNG\r\n\x1a\nrest"),
            Some("image/png")
        );
        assert_eq!(
            image_mime_from_bytes(b"<svg viewBox=\"0 0 1 1\"></svg>"),
            Some("image/svg+xml")
        );
    }
}
