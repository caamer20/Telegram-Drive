use std::path::{Path, PathBuf};

use grammers_client::Client;
use zeroize::Zeroize;

use super::crypto::{unwrap_key, KEY_LEN};
use super::format::{decrypt_file_to_path, file_key_aad};
use super::manifest::FileRecord;
use super::state::{random_positive_id, VaultRuntime};
use super::storage;

#[derive(Debug, Clone)]
pub struct CachedVaultFile {
    pub path: PathBuf,
    pub name: String,
}

fn safe_extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            ext.chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .take(16)
                .collect::<String>()
        })
        .filter(|ext| !ext.is_empty())
}

fn cached_plaintext_path(cache_dir: &Path, namespace: &str, record: &FileRecord) -> PathBuf {
    let mut filename = format!(
        "{}-{}-{}",
        record.id, record.blob_message_id, record.ciphertext_size
    );
    if let Some(ext) = safe_extension(&record.name) {
        filename.push('.');
        filename.push_str(&ext);
    }
    cache_dir.join(namespace).join(filename)
}

pub async fn decrypt_file_to_cache(
    client: &Client,
    runtime: &VaultRuntime,
    cache_dir: &Path,
    message_id: i64,
    folder_id: Option<i64>,
    namespace: &str,
) -> Result<CachedVaultFile, String> {
    let (record, bucket_id, vault_id, mut master_key): (FileRecord, i64, String, [u8; KEY_LEN]) = {
        let guard = runtime.inner.lock().await;
        let vault = guard
            .as_ref()
            .ok_or_else(|| "Vault is locked. Unlock it before opening files.".to_string())?;
        let record = vault
            .manifest
            .files
            .iter()
            .find(|file| file.id == message_id && file.folder_id == folder_id)
            .ok_or_else(|| "File not found".to_string())?
            .clone();

        (
            record,
            vault.config.bucket_id,
            vault.config.vault_id.clone(),
            vault.master_key,
        )
    };

    let output_path = cached_plaintext_path(cache_dir, namespace, &record);
    if output_path.exists() {
        if std::fs::metadata(&output_path)
            .map(|metadata| metadata.len() == record.size)
            .unwrap_or(false)
        {
            master_key.zeroize();
            return Ok(CachedVaultFile {
                path: output_path,
                name: record.name,
            });
        }
        let _ = std::fs::remove_file(&output_path);
    }

    let output_parent = output_path
        .parent()
        .ok_or_else(|| "Invalid cache output path".to_string())?;
    std::fs::create_dir_all(output_parent)
        .map_err(|e| format!("Failed to create vault cache: {}", e))?;

    let temp_dir = cache_dir.join("tmp");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create vault temp cache: {}", e))?;
    let encrypted_temp = tempfile::Builder::new()
        .prefix("tdv1-cache-")
        .suffix(".blob")
        .tempfile_in(&temp_dir)
        .map_err(|e| format!("Failed to create ciphertext cache temp file: {}", e))?;
    let encrypted_path = encrypted_temp.path().to_path_buf();

    storage::download_object(client, bucket_id, record.blob_message_id, &encrypted_path).await?;

    let tmp_plaintext_path =
        output_parent.join(format!(".{}.tmp-{}", record.id, random_positive_id()));
    let mut file_key = unwrap_key(
        &master_key,
        &record.wrapped_file_key,
        &file_key_aad(&vault_id, record.id),
    )?;
    master_key.zeroize();

    let decrypt_result = decrypt_file_to_path(
        &encrypted_path,
        &tmp_plaintext_path,
        &vault_id,
        record.id,
        &file_key,
    );
    file_key.zeroize();
    if let Err(error) = decrypt_result {
        let _ = std::fs::remove_file(&tmp_plaintext_path);
        return Err(error);
    }

    std::fs::rename(&tmp_plaintext_path, &output_path)
        .map_err(|e| format!("Failed to store decrypted cache file: {}", e))?;

    Ok(CachedVaultFile {
        path: output_path,
        name: record.name,
    })
}
