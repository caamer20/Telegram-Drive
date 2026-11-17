use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::Manager;

use crate::models::FileMetadata;
use crate::workspace::{
    store::{file_key, Store},
    AccountGuard,
};

#[derive(Debug, Clone, Serialize)]
pub struct LocalFileActivity {
    #[serde(flatten)]
    pub file: FileMetadata,
    pub last_opened_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OpenedFile {
    folder_id: Option<i64>,
    message_id: i64,
    last_opened_at: i64,
    open_count: u64,
}

#[derive(Clone, Copy)]
enum ActivityChange<'a> {
    Open,
    Flag(&'a str, bool),
}

fn change_activity(
    store: &Store,
    file: &FileMetadata,
    change: ActivityChange<'_>,
) -> Result<(), String> {
    if file.id <= 0 {
        return Err("Invalid message identifier".into());
    }
    let key = file_key(file.folder_id, file.id);
    store.transaction(|| {
        // Incoming metadata belongs to the explicitly verified account/source.
        // Never copy a name or flag from the unowned legacy file_activity table.
        store.remember_local_file(file)?;
        match change {
            ActivityChange::Open => {
                let mut opened = store
                    .record::<OpenedFile>("opened", &key)?
                    .unwrap_or_default();
                opened.folder_id = file.folder_id;
                opened.message_id = file.id;
                opened.last_opened_at = crate::app_time::unix_seconds();
                opened.open_count = opened.open_count.saturating_add(1);
                store.put_record("opened", &key, &opened)
            }
            ActivityChange::Flag("favorite", value) => store.put_record("favorite", &key, &value),
            ActivityChange::Flag("pinned", value) => store.put_record("pin", &key, &value),
            ActivityChange::Flag(_, _) => Err("Unknown file activity flag".into()),
        }
    })
}

pub(crate) fn read_activity(
    store: &Store,
    view: &str,
    limit: Option<i64>,
) -> Result<Vec<LocalFileActivity>, String> {
    if !matches!(view, "favorites" | "pinned" | "recents") {
        return Err("Unknown smart view".into());
    }
    let opened: HashMap<_, _> = store
        .records::<OpenedFile>("opened")?
        .into_iter()
        .map(|value| (file_key(value.folder_id, value.message_id), value))
        .collect();
    let mut files = Vec::new();
    for entry in store.files()? {
        let activity = opened.get(&entry.key);
        let include = match view {
            "favorites" => entry.file.is_favorite,
            "pinned" => entry.file.is_pinned,
            _ => activity.is_some_and(|value| value.open_count > 0),
        };
        if include {
            files.push(LocalFileActivity {
                file: entry.file,
                last_opened_at: activity.map_or(0, |value| value.last_opened_at),
            });
        }
    }
    files.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.last_opened_at),
            entry.file.folder_id,
            entry.file.id,
        )
    });
    files.truncate(limit.unwrap_or(200).clamp(1, 1_000) as usize);
    Ok(files)
}

pub(crate) async fn folder_flags(
    account: &AccountGuard,
    folder_id: Option<i64>,
) -> Result<HashMap<i32, (bool, bool)>, String> {
    let account = account.clone();
    tokio::task::spawn_blocking(move || {
        account.validate()?;
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let flags = read_folder_flags(&Store::open(&account.root, account.owner)?, folder_id)?;
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let files = Store::open(&account.root, account.owner)?.folder_files(folder_id)?;
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let flags = files
            .into_iter()
            .filter_map(|file| {
                i32::try_from(file.id)
                    .ok()
                    .map(|id| (id, (file.is_favorite, file.is_pinned)))
            })
            .collect();
        account.validate()?;
        Ok(flags)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Flags are optional local decoration for a remote listing. A damaged cached
/// filename or flag must not prevent that listing from repairing its metadata.
/// Preserve the original rows and recover every valid flag independently.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn read_folder_flags(
    store: &Store,
    folder_id: Option<i64>,
) -> Result<HashMap<i32, (bool, bool)>, String> {
    let folder = folder_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "saved".into());
    let prefix = format!("{folder}:");
    let mut statement = store
        .db
        .prepare(
            "SELECT f.key,f.metadata,r.value,p.value FROM workspace_files f
         LEFT JOIN workspace_records r ON r.kind='favorite' AND r.id=f.key
         LEFT JOIN workspace_records p ON p.kind='pin' AND p.id=f.key
         WHERE f.folder=?",
        )
        .map_err(|e| e.to_string())?;
    statement
        .bind((1, folder.as_str()))
        .map_err(|e| e.to_string())?;
    let mut flags = HashMap::new();
    while statement.next().map_err(|e| e.to_string())? == sqlite::State::Row {
        let key = statement.read::<String, _>(0).map_err(|e| e.to_string())?;
        let Some(message) = key
            .strip_prefix(&prefix)
            .and_then(|id| id.parse::<i32>().ok())
            .filter(|id| *id > 0)
        else {
            continue;
        };
        let metadata = statement.read::<String, _>(1).map_err(|e| e.to_string())?;
        let file = serde_json::from_str::<FileMetadata>(&metadata)
            .ok()
            .filter(|file| file.id == i64::from(message) && file.folder_id == folder_id);
        let favorite = file.as_ref().is_some_and(|file| file.is_favorite);
        let pinned = file.as_ref().is_some_and(|file| file.is_pinned);
        let read_flag = |column, fallback| -> Result<bool, String> {
            Ok(statement
                .read::<Option<String>, _>(column)
                .map_err(|e| e.to_string())?
                .and_then(|value| serde_json::from_str::<bool>(&value).ok())
                .unwrap_or(fallback))
        };
        flags.insert(message, (read_flag(2, favorite)?, read_flag(3, pinned)?));
    }
    Ok(flags)
}

#[allow(clippy::too_many_arguments)]
fn metadata(
    folder_id: Option<i64>,
    message_id: i64,
    file_name: String,
    file_size: u64,
    mime_type: Option<String>,
    file_ext: Option<String>,
    created_at: Option<String>,
    encryption_state: Option<String>,
) -> FileMetadata {
    FileMetadata {
        id: message_id,
        folder_id,
        name: file_name,
        size: file_size,
        mime_type,
        file_ext,
        created_at: created_at.unwrap_or_default(),
        encryption_state: encryption_state.unwrap_or_else(|| "plain".into()),
        icon_type: "file".into(),
        is_favorite: false,
        is_pinned: false,
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn cmd_record_file_opened(
    app: tauri::AppHandle,
    owner_id: Option<String>,
    folder_id: Option<i64>,
    message_id: i64,
    file_name: String,
    file_size: u64,
    mime_type: Option<String>,
    file_ext: Option<String>,
    created_at: Option<String>,
    encryption_state: Option<String>,
) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, owner_id.as_deref())?;
    let file = metadata(
        folder_id,
        message_id,
        file_name,
        file_size,
        mime_type,
        file_ext,
        created_at,
        encryption_state,
    );
    tokio::task::spawn_blocking(move || {
        account.validate()?;
        let store = Store::open(&account.root, account.owner)?;
        change_activity(&store, &file, ActivityChange::Open)?;
        account.validate()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn cmd_set_file_activity_flag(
    app: tauri::AppHandle,
    owner_id: Option<String>,
    folder_id: Option<i64>,
    message_id: i64,
    file_name: String,
    file_size: u64,
    mime_type: Option<String>,
    file_ext: Option<String>,
    created_at: Option<String>,
    encryption_state: Option<String>,
    flag: String,
    value: bool,
) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, owner_id.as_deref())?;
    let file = metadata(
        folder_id,
        message_id,
        file_name,
        file_size,
        mime_type,
        file_ext,
        created_at,
        encryption_state,
    );
    tokio::task::spawn_blocking(move || {
        account.validate()?;
        let store = Store::open(&account.root, account.owner)?;
        change_activity(&store, &file, ActivityChange::Flag(&flag, value))?;
        account.validate()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn cmd_get_file_activity(
    app: tauri::AppHandle,
    owner_id: Option<String>,
    view: String,
    limit: Option<i64>,
) -> Result<Vec<LocalFileActivity>, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, owner_id.as_deref())?;
    tokio::task::spawn_blocking(move || {
        account.validate()?;
        let files = read_activity(&Store::open(&account.root, account.owner)?, &view, limit)?;
        account.validate()?;
        Ok(files)
    })
    .await
    .map_err(|e| e.to_string())?
}
