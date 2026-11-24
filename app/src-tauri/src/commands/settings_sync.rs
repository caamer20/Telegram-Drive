use crate::commands::utils::resolve_peer;
use crate::crypto::kdf::derive_passphrase_key;
use crate::TelegramState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use grammers_client::types::InputMessage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;
use tauri::{Manager, State};
use zeroize::Zeroizing;

const MESSAGE_PREFIX: &str = "TDSETTINGS1:";
const DEVICE_ID_FILE: &str = "settings-sync-device-id";
const MAX_TELEGRAM_MESSAGE_BYTES: usize = 4_000;
const MESSAGE_SCAN_LIMIT: usize = 1_000;
const MIN_PASSPHRASE_CHARACTERS: usize = 12;
const ARGON2_MEMORY_KIB: u32 = 65_536;
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;

const BOOLEAN_KEYS: &[&str] = &[
    "autoUpdate",
    "zipFolders",
    "strictUploadOrder",
    "videoFilenameCaption",
    "sidebarCollapsed",
    "hideGroups",
    "vpnMode",
    "adaptivePolling",
    "floodWaitRespect",
    "autoDetectVpn",
    "performanceMode",
    "linuxRenderingFix",
    "encryptionProtectMetadata",
    "encryptionLockOnSleep",
];

const NUMBER_RANGES: &[(&str, i64, i64)] = &[
    ("maxConcurrentUploads", 1, 32),
    ("maxConcurrentDownloads", 1, 32),
    ("timeoutMultiplier", 1, 20),
    ("retryAttempts", 0, 20),
    ("retryBaseBackoffSec", 0, 300),
    ("retryMaxBackoffSec", 1, 3_600),
    ("pollingMinSec", 1, 3_600),
    ("pollingMaxSec", 1, 86_400),
    ("dcFallbackAttempts", 0, 10),
    ("peerCacheSize", 1, 100_000),
    ("bandwidthLimitUpKBs", 0, 10_000_000),
    ("bandwidthLimitDownKBs", 0, 10_000_000),
    ("chunkSizeKb", 4, 512),
    ("keepAliveIntervalSec", 0, 3_600),
    ("archiveMaxBytes", 0, 4_294_967_296),
    ("transcodeCacheMaxGb", 1, 50),
    ("encryptionAutoLockMinutes", 0, 1_440),
];

const STRING_VALUES: &[(&str, &[&str])] = &[
    ("viewMode", &["grid", "list"]),
    ("fileSortField", &["name", "size", "date"]),
    ("fileSortDirection", &["asc", "desc"]),
    ("videoUploadMode", &["file", "media"]),
    (
        "language",
        &[
            "system", "en", "es", "ru", "uk-UA", "pl-PL", "fa-IR", "ur-PK", "ms-MY", "zh-CN",
            "zh-TW", "fr", "it", "ar", "pt-BR", "de", "hi", "bn-BD", "id", "fil-PH", "tr", "th-TH",
            "ja", "ko", "vi",
        ],
    ),
    ("preferredDC", &["auto", "dc1", "dc2", "dc3", "dc4", "dc5"]),
    (
        "encryptionDefaultMode",
        &["standard", "vault", "passphrase", "vault_and_passphrase"],
    ),
    ("encryptionTempPolicy", &["balanced", "strict"]),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedSettingsEnvelope {
    version: u8,
    updated_at: i64,
    device_id: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SettingsPayload {
    schema_version: u8,
    settings: Value,
    // v3.9 readers ignore optional payload fields, but reject unknown settings keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    video_filename_caption: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SettingsSyncStatus {
    pub available: bool,
    pub updated_at: Option<i64>,
    pub device_id: Option<String>,
    pub current_device: bool,
}

#[derive(Debug, Serialize)]
pub struct SettingsSyncDownload {
    pub settings: Value,
    pub updated_at: i64,
    pub device_id: String,
}

fn validate_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARACTERS {
        return Err(format!(
            "The sync passphrase must contain at least {MIN_PASSPHRASE_CHARACTERS} characters"
        ));
    }
    Ok(())
}

fn sanitize_settings(settings: Value) -> Result<Value, String> {
    let object = settings
        .as_object()
        .ok_or_else(|| "Settings sync payload must be an object".to_string())?;
    let mut safe = Map::new();

    for (key, value) in object {
        if BOOLEAN_KEYS.contains(&key.as_str()) {
            if !value.is_boolean() {
                return Err(format!("Invalid value for synced setting {key}"));
            }
        } else if let Some((_, minimum, maximum)) =
            NUMBER_RANGES.iter().find(|(name, _, _)| name == key)
        {
            let number = value
                .as_i64()
                .ok_or_else(|| format!("Invalid value for synced setting {key}"))?;
            if number < *minimum || number > *maximum {
                return Err(format!(
                    "Synced setting {key} is outside its supported range"
                ));
            }
        } else if let Some((_, accepted)) = STRING_VALUES.iter().find(|(name, _)| name == key) {
            let text = value
                .as_str()
                .ok_or_else(|| format!("Invalid value for synced setting {key}"))?;
            if !accepted.contains(&text) {
                return Err(format!("Unsupported value for synced setting {key}"));
            }
        } else {
            return Err(format!("Setting {key} is not eligible for sync"));
        }
        safe.insert(key.clone(), value.clone());
    }

    if safe.is_empty() {
        return Err("No syncable settings were provided".to_string());
    }
    Ok(Value::Object(safe))
}

fn aad(envelope: &EncryptedSettingsEnvelope) -> String {
    format!(
        "telegram-drive:settings-sync:v1:{}:{}",
        envelope.updated_at, envelope.device_id
    )
}

fn encrypt_settings(
    settings: Value,
    passphrase: &str,
    device_id: String,
) -> Result<String, String> {
    validate_passphrase(passphrase)?;
    let mut settings = sanitize_settings(settings)?;
    let caption = settings
        .as_object_mut()
        .and_then(|values| values.remove("videoFilenameCaption"))
        .and_then(|value| value.as_bool());
    let payload = serde_json::to_vec(&SettingsPayload {
        schema_version: 1,
        settings,
        video_filename_caption: caption,
    })
    .map_err(|error| format!("Unable to encode settings: {error}"))?;
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 24];
    getrandom::getrandom(&mut salt)
        .map_err(|error| format!("Unable to create settings salt: {error}"))?;
    getrandom::getrandom(&mut nonce)
        .map_err(|error| format!("Unable to create settings nonce: {error}"))?;
    let key = derive_passphrase_key(
        passphrase.as_bytes(),
        &salt,
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
    )
    .map_err(|error| error.to_string())?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.expose())
        .map_err(|_| "Unable to initialize settings encryption".to_string())?;
    let mut envelope = EncryptedSettingsEnvelope {
        version: 1,
        updated_at: crate::app_time::unix_seconds(),
        device_id,
        salt: URL_SAFE_NO_PAD.encode(salt),
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        ciphertext: String::new(),
    };
    let associated_data = aad(&envelope);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &payload,
                aad: associated_data.as_bytes(),
            },
        )
        .map_err(|_| "Unable to encrypt settings".to_string())?;
    envelope.ciphertext = URL_SAFE_NO_PAD.encode(ciphertext);
    let encoded = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&envelope)
            .map_err(|error| format!("Unable to encode encrypted settings: {error}"))?,
    );
    let message = format!("{MESSAGE_PREFIX}{encoded}");
    if message.len() > MAX_TELEGRAM_MESSAGE_BYTES {
        return Err("Encrypted settings exceed Telegram's safe message size".to_string());
    }
    Ok(message)
}

fn parse_envelope(message: &str) -> Result<EncryptedSettingsEnvelope, String> {
    let encoded = message
        .strip_prefix(MESSAGE_PREFIX)
        .ok_or_else(|| "Not a Telegram Drive settings message".to_string())?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "Encrypted settings message is malformed".to_string())?;
    let envelope: EncryptedSettingsEnvelope = serde_json::from_slice(&bytes)
        .map_err(|_| "Encrypted settings envelope is malformed".to_string())?;
    if envelope.version != 1 || envelope.device_id.len() != 32 {
        return Err("Encrypted settings envelope version is unsupported".to_string());
    }
    Ok(envelope)
}

fn decrypt_settings(message: &str, passphrase: &str) -> Result<SettingsSyncDownload, String> {
    validate_passphrase(passphrase)?;
    let envelope = parse_envelope(message)?;
    let salt: [u8; 16] = URL_SAFE_NO_PAD
        .decode(&envelope.salt)
        .map_err(|_| "Encrypted settings salt is malformed".to_string())?
        .try_into()
        .map_err(|_| "Encrypted settings salt has the wrong length".to_string())?;
    let nonce: [u8; 24] = URL_SAFE_NO_PAD
        .decode(&envelope.nonce)
        .map_err(|_| "Encrypted settings nonce is malformed".to_string())?
        .try_into()
        .map_err(|_| "Encrypted settings nonce has the wrong length".to_string())?;
    let ciphertext = URL_SAFE_NO_PAD
        .decode(&envelope.ciphertext)
        .map_err(|_| "Encrypted settings payload is malformed".to_string())?;
    let key = derive_passphrase_key(
        passphrase.as_bytes(),
        &salt,
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
    )
    .map_err(|error| error.to_string())?;
    let cipher = XChaCha20Poly1305::new_from_slice(key.expose())
        .map_err(|_| "Unable to initialize settings decryption".to_string())?;
    let associated_data = aad(&envelope);
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: associated_data.as_bytes(),
            },
        )
        .map_err(|_| "The sync passphrase is incorrect or the backup is damaged".to_string())?;
    let mut payload: SettingsPayload = serde_json::from_slice(&plaintext)
        .map_err(|_| "Decrypted settings payload is malformed".to_string())?;
    if payload.schema_version != 1 {
        return Err("Decrypted settings schema is unsupported".to_string());
    }
    if let Some(caption) = payload.video_filename_caption {
        let values = payload
            .settings
            .as_object_mut()
            .ok_or_else(|| "Settings sync payload must be an object".to_string())?;
        values.insert("videoFilenameCaption".into(), Value::Bool(caption));
    }
    Ok(SettingsSyncDownload {
        settings: sanitize_settings(payload.settings)?,
        updated_at: envelope.updated_at,
        device_id: envelope.device_id,
    })
}

fn device_id(path: &Path) -> Result<String, String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim();
        if existing.len() == 32
            && existing
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Ok(existing.to_ascii_lowercase());
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create settings sync directory: {error}"))?;
    }
    let mut random = [0_u8; 16];
    getrandom::getrandom(&mut random)
        .map_err(|error| format!("Unable to create settings sync device ID: {error}"))?;
    let id = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::fs::write(path, &id)
        .map_err(|error| format!("Unable to save settings sync device ID: {error}"))?;
    Ok(id)
}

async fn client_and_peer(
    state: &TelegramState,
) -> Result<(grammers_client::Client, grammers_client::types::Peer), String> {
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Connect to Telegram before syncing settings".to_string())?;
    let peer = resolve_peer(&client, None, &state.peer_cache).await?;
    Ok((client, peer))
}

async fn settings_messages(
    client: &grammers_client::Client,
    peer: &grammers_client::types::Peer,
) -> Result<Vec<(i32, String)>, String> {
    let mut messages = client.iter_messages(peer).limit(MESSAGE_SCAN_LIMIT);
    let mut found = Vec::new();
    while let Some(message) = messages.next().await.map_err(|error| error.to_string())? {
        if message.text().starts_with(MESSAGE_PREFIX) {
            found.push((message.id(), message.text().to_string()));
        }
    }
    Ok(found)
}

#[tauri::command]
pub async fn cmd_get_settings_sync_status(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
) -> Result<SettingsSyncStatus, String> {
    let local_id = device_id(
        &app_handle
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join(DEVICE_ID_FILE),
    )?;
    let (client, peer) = client_and_peer(state.inner()).await?;
    let Some((_, message)) = settings_messages(&client, &peer).await?.into_iter().next() else {
        return Ok(SettingsSyncStatus {
            available: false,
            updated_at: None,
            device_id: None,
            current_device: false,
        });
    };
    let envelope = parse_envelope(&message)?;
    Ok(SettingsSyncStatus {
        available: true,
        updated_at: Some(envelope.updated_at),
        current_device: envelope.device_id == local_id,
        device_id: Some(envelope.device_id),
    })
}

#[tauri::command]
pub async fn cmd_upload_settings_sync(
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    settings: Value,
    passphrase: String,
) -> Result<SettingsSyncStatus, String> {
    let local_id = device_id(
        &app_handle
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join(DEVICE_ID_FILE),
    )?;
    let encryption_device_id = local_id.clone();
    let message = tokio::task::spawn_blocking(move || {
        let passphrase = Zeroizing::new(passphrase);
        encrypt_settings(settings, &passphrase, encryption_device_id)
    })
    .await
    .map_err(|error| format!("Settings encryption task failed: {error}"))??;
    let (client, peer) = client_and_peer(state.inner()).await?;
    let old_ids = settings_messages(&client, &peer)
        .await?
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    client
        .send_message(&peer, InputMessage::new().text(message.clone()))
        .await
        .map_err(|error| format!("Unable to upload encrypted settings: {error}"))?;
    for old_id_batch in old_ids.chunks(100) {
        if let Err(error) = client.delete_messages(&peer, old_id_batch).await {
            log::warn!("Unable to remove superseded settings sync messages: {error}");
        }
    }
    let envelope = parse_envelope(&message)?;
    Ok(SettingsSyncStatus {
        available: true,
        updated_at: Some(envelope.updated_at),
        device_id: Some(local_id),
        current_device: true,
    })
}

#[tauri::command]
pub async fn cmd_download_settings_sync(
    state: State<'_, TelegramState>,
    passphrase: String,
) -> Result<SettingsSyncDownload, String> {
    let (client, peer) = client_and_peer(state.inner()).await?;
    let (_, message) = settings_messages(&client, &peer)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| "No encrypted settings backup was found in Saved Messages".to_string())?;
    tokio::task::spawn_blocking(move || {
        let passphrase = Zeroizing::new(passphrase);
        decrypt_settings(&message, &passphrase)
    })
    .await
    .map_err(|error| format!("Settings decryption task failed: {error}"))?
}
