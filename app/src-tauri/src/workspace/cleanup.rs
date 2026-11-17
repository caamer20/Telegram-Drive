//! Recoverable cleanup uses an explicit deletion grace period. The original
//! stays in Telegram until the deadline and is never represented as a backup.
use super::{
    assets::stored_file,
    store::{Store, WorkspaceFile},
    AccountGuard,
};
use crate::commands::{
    utils::{media_size, resolve_peer},
    TelegramState,
};
use grammers_client::{
    types::{Media, Message, Peer},
    Client,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

static OPERATIONS: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removal {
    pub id: String,
    pub key: String,
    pub file: WorkspaceFile,
    pub requested_at: i64,
    pub delete_after: i64,
    pub status: String,
    pub fingerprint: String,
    pub error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleResult {
    pub key: String,
    pub scheduled: bool,
    pub error: Option<String>,
}

fn fingerprint(message: &Message) -> Result<String, String> {
    let media = message.media().ok_or("FILE_NOT_FOUND")?;
    let identity = match &media {
        Media::Document(document) => format!("document:{}", document.id()),
        Media::Photo(photo) => format!("photo:{}", photo.id()),
        _ => return Err("UNSUPPORTED_FILE".into()),
    };
    Ok(format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{identity}:{}:{}:{}",
                media_size(&media),
                message
                    .edit_date()
                    .map(|date| date.timestamp())
                    .unwrap_or(0),
                message.text()
            )
            .as_bytes()
        )
    ))
}

async fn message(
    app: &tauri::AppHandle,
    account: &AccountGuard,
    file: &WorkspaceFile,
) -> Result<(Client, Peer, Option<Message>), String> {
    account.validate()?;
    let state = app.state::<TelegramState>();
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or("NETWORK_UNAVAILABLE")?;
    let peer = resolve_peer(&client, file.file.folder_id, &state.peer_cache)
        .await
        .map_err(|_| "NETWORK_UNAVAILABLE")?;
    let id = i32::try_from(file.file.id).map_err(|_| "INVALID_FILE")?;
    let value = client
        .get_messages_by_id(&peer, &[id])
        .await
        .map_err(|_| "NETWORK_UNAVAILABLE")?
        .into_iter()
        .flatten()
        .next();
    account.validate()?;
    Ok((client, peer, value))
}

pub fn may_restore(removal: &Removal) -> bool {
    matches!(removal.status.as_str(), "pending" | "failed")
}
pub fn is_due(removal: &Removal, now: i64) -> bool {
    matches!(removal.status.as_str(), "pending" | "deleting") && removal.delete_after <= now
}
pub fn list(store: &Store) -> Result<Vec<Removal>, String> {
    store.records("removal")
}

pub fn schedule(
    store: &Store,
    file: WorkspaceFile,
    fingerprint: String,
    days: u32,
    now: i64,
) -> Result<Removal, String> {
    if !(1..=30).contains(&days) {
        return Err("INVALID_RETENTION: Choose 1–30 days".into());
    }
    if let Some(existing) = store.record::<Removal>("removal", &file.key)? {
        if matches!(existing.status.as_str(), "pending" | "deleting") {
            return Ok(existing);
        }
    }
    let value = Removal {
        id: uuid::Uuid::new_v4().to_string(),
        key: file.key.clone(),
        file,
        requested_at: now,
        delete_after: now + days as i64 * 86_400_000,
        status: "pending".into(),
        fingerprint,
        error: None,
    };
    store.put_record("removal", &value.key, &value)?;
    Ok(value)
}

pub fn restore(store: &Store, key: &str) -> Result<Removal, String> {
    let mut value = store
        .record::<Removal>("removal", key)?
        .ok_or("REMOVAL_NOT_FOUND")?;
    if !may_restore(&value) {
        return Err("RESTORATION_UNAVAILABLE: This removal has already been processed".into());
    }
    value.status = "restored".into();
    value.error = None;
    store.put_record("removal", key, &value)?;
    Ok(value)
}

#[tauri::command]
pub async fn cmd_cleanup_list(
    app: tauri::AppHandle,
    owner_id: String,
) -> Result<Vec<Removal>, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let account = AccountGuard::open(&root, Some(&owner_id))?;
        let result = list(&Store::open(&root, account.owner)?)?;
        account.validate()?;
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn cmd_cleanup_schedule(
    app: tauri::AppHandle,
    owner_id: String,
    keys: Vec<String>,
    retention_days: u32,
) -> Result<Vec<ScheduleResult>, String> {
    if keys.is_empty() || keys.len() > 500 {
        return Err("Select between 1 and 500 files".into());
    }
    if !(1..=30).contains(&retention_days) {
        return Err("Choose a removal grace period from 1 to 30 days".into());
    }
    let _lock = OPERATIONS.get_or_init(|| Mutex::new(())).lock().await;
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, Some(&owner_id))?;
    let mut outcomes = Vec::new();
    for key in keys {
        let result = async {
            let file = stored_file(&account, &key)?;
            let (_, _, remote) = message(&app, &account, &file).await?;
            let remote = remote.ok_or("FILE_NOT_FOUND")?;
            let fingerprint = fingerprint(&remote)?;
            let store = Store::open(&root, account.owner)?;
            account.validate()?;
            schedule(
                &store,
                file,
                fingerprint,
                retention_days,
                crate::app_time::unix_millis(),
            )?;
            Ok::<(), String>(())
        }
        .await;
        outcomes.push(ScheduleResult {
            key,
            scheduled: result.is_ok(),
            error: result.err(),
        });
    }
    let _ = app.emit("workspace-changed", &owner_id);
    Ok(outcomes)
}

#[tauri::command]
pub async fn cmd_cleanup_restore(
    app: tauri::AppHandle,
    owner_id: String,
    key: String,
) -> Result<Removal, String> {
    // Serialize with the last-minute delete claim, so Restore never claims to
    // succeed while an irreversible Telegram operation is already in flight.
    let _lock = OPERATIONS.get_or_init(|| Mutex::new(())).lock().await;
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, Some(&owner_id))?;
    let store = Store::open(&root, account.owner)?;
    account.validate()?;
    let restored = restore(&store, &key)?;
    let _ = app.emit("workspace-changed", &owner_id);
    Ok(restored)
}

/// The delete callback runs only after durable intent has been committed.
/// An ambiguous RPC error stays `deleting`: claiming restoration at that point
/// could promise a file which the server has already removed.
async fn process_one<F, Fut>(
    root: &std::path::Path,
    owner: i64,
    mut removal: Removal,
    now: i64,
    remote: Option<String>,
    validate: impl Fn() -> Result<(), String>,
    delete: F,
) -> Result<Removal, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    if !is_due(&removal, now) {
        return Ok(removal);
    }
    validate()?;
    if let Some(fingerprint) = remote {
        if fingerprint != removal.fingerprint {
            removal.status = "failed".into();
            removal.error = Some("FILE_CHANGED".into());
            Store::open(root, owner)?.put_record("removal", &removal.key, &removal)?;
            return Ok(removal);
        }
        removal.status = "deleting".into();
        removal.error = None;
        Store::open(root, owner)?.put_record("removal", &removal.key, &removal)?;
        validate()?;
        if delete().await.is_err() {
            removal.error = Some("NETWORK_UNAVAILABLE".into());
            Store::open(root, owner)?.put_record("removal", &removal.key, &removal)?;
            return Ok(removal);
        }
    }
    removal.status = "deleted".into();
    removal.error = None;
    // Atomic terminal state and inventory removal are safe to repeat after a
    // crash: an absent remote message never triggers another delete RPC.
    let store = Store::open(root, owner)?;
    store.transaction(|| {
        store.put_record("removal", &removal.key, &removal)?;
        store.execute(
            "DELETE FROM workspace_files WHERE key=?",
            &[removal.key.clone().into()],
        )
    })?;
    Ok(removal)
}

#[tauri::command]
pub async fn cmd_cleanup_process(
    app: tauri::AppHandle,
    owner_id: String,
) -> Result<Vec<Removal>, String> {
    let _lock = OPERATIONS.get_or_init(|| Mutex::new(())).lock().await;
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, Some(&owner_id))?;
    let due = list(&Store::open(&root, account.owner)?)?;
    for removal in due
        .into_iter()
        .filter(|value| is_due(value, crate::app_time::unix_millis()))
    {
        account.validate()?;
        let (client, peer, remote) = match message(&app, &account, &removal.file).await {
            Ok(value) => value,
            Err(error) if error == "NETWORK_UNAVAILABLE" => continue,
            Err(error) => return Err(error),
        };
        let fingerprint = remote.as_ref().map(fingerprint).transpose()?;
        let id = i32::try_from(removal.file.file.id).map_err(|_| "INVALID_FILE")?;
        process_one(
            &root,
            account.owner,
            removal,
            crate::app_time::unix_millis(),
            fingerprint,
            || account.validate(),
            || async {
                client
                    .delete_messages(&peer, &[id])
                    .await
                    .map(|_| ())
                    .map_err(|_| "NETWORK_UNAVAILABLE".into())
            },
        )
        .await?;
        let _ = app.emit("workspace-changed", &owner_id);
    }
    cmd_cleanup_list(app, owner_id).await
}
