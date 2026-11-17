use super::{store::Store, AccountGuard};
use serde::{Deserialize, Serialize};
use tauri::Manager;

const MAX_HISTORY: usize = 500;
const MAX_QUEUE: usize = 100;
const MAX_BOOKMARKS: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackFile {
    pub folder_id: Option<i64>,
    pub message_id: i64,
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default = "plain")]
    pub encryption_state: String,
}

fn plain() -> String {
    "plain".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub position_ms: u64,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackRecord {
    pub media_id: String,
    pub owner_id: String,
    #[serde(flatten)]
    pub file: PlaybackFile,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub completed: bool,
    pub volume: f64,
    pub speed: f64,
    pub updated_at: i64,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPreferences {
    pub volume: f64,
    pub speed: f64,
}
impl Default for PlaybackPreferences {
    fn default() -> Self {
        Self {
            volume: 1.0,
            speed: 1.0,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSnapshot {
    pub owner_id: String,
    pub items: Vec<PlaybackRecord>,
    pub queue: Vec<PlaybackFile>,
    pub preferences: PlaybackPreferences,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PlaybackMutation {
    Progress {
        file: PlaybackFile,
        position_ms: u64,
        duration_ms: u64,
        volume: f64,
        speed: f64,
    },
    Finish {
        file: PlaybackFile,
        duration_ms: u64,
    },
    Restart {
        file: PlaybackFile,
    },
    Bookmark {
        file: PlaybackFile,
        position_ms: u64,
        label: String,
    },
    RemoveBookmark {
        file: PlaybackFile,
        position_ms: u64,
    },
    Forget {
        file: PlaybackFile,
    },
    Enqueue {
        files: Vec<PlaybackFile>,
    },
    RemoveQueue {
        file: PlaybackFile,
    },
    ClearQueue,
}

pub fn media_id(owner: i64, file: &PlaybackFile) -> String {
    format!(
        "{owner}:{}:{}",
        file.folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "saved".into()),
        file.message_id
    )
}

fn validate_file(file: &PlaybackFile) -> Result<(), String> {
    if file.message_id <= 0
        || file.message_id > i32::MAX as i64
        || file.folder_id.is_some_and(|id| id <= 0)
    {
        return Err("Invalid playback file identity".into());
    }
    if file.name.trim().is_empty()
        || file.name.len() > 4096
        || file.mime_type.as_ref().is_some_and(|mime| mime.len() > 200)
    {
        return Err("Invalid playback file metadata".into());
    }
    // Do not create another plaintext index of metadata-protected content.
    // Protected playback requires a separately designed vault-aware history.
    if file.encryption_state != "plain" {
        return Err(
            "PLAYBACK_HISTORY_UNAVAILABLE: History is unavailable for protected media".into(),
        );
    }
    Ok(())
}

pub fn snapshot(store: &Store) -> Result<PlaybackSnapshot, String> {
    Ok(PlaybackSnapshot {
        owner_id: store.owner.to_string(),
        items: store.records("playback")?,
        queue: store.record("playback_queue", "main")?.unwrap_or_default(),
        preferences: store
            .record("playback_preferences", "main")?
            .unwrap_or_default(),
    })
}

fn record(store: &Store, file: &PlaybackFile) -> Result<PlaybackRecord, String> {
    validate_file(file)?;
    let preferences: PlaybackPreferences = store
        .record("playback_preferences", "main")?
        .unwrap_or_default();
    let mut value = store
        .record::<PlaybackRecord>("playback", &media_id(store.owner, file))?
        .unwrap_or_else(|| PlaybackRecord {
            media_id: media_id(store.owner, file),
            owner_id: store.owner.to_string(),
            file: file.clone(),
            position_ms: 0,
            duration_ms: 0,
            completed: false,
            volume: preferences.volume,
            speed: preferences.speed,
            updated_at: 0,
            bookmarks: Vec::new(),
        });
    value.file = file.clone();
    value.updated_at = crate::app_time::unix_millis();
    Ok(value)
}

fn save(store: &Store, value: &PlaybackRecord) -> Result<(), String> {
    store.put_record("playback", &value.media_id, value)?;
    let records: Vec<PlaybackRecord> = store.records("playback")?;
    for old in records.into_iter().skip(MAX_HISTORY) {
        store.remove_record("playback", &old.media_id)?;
    }
    Ok(())
}

pub fn mutate(store: &Store, mutation: PlaybackMutation) -> Result<(), String> {
    store.transaction(|| {
        match mutation {
            PlaybackMutation::Progress {
                file,
                position_ms,
                duration_ms,
                volume,
                speed,
            } => {
                if !volume.is_finite()
                    || !speed.is_finite()
                    || !(0.0..=1.0).contains(&volume)
                    || !(0.5..=2.0).contains(&speed)
                {
                    return Err("Invalid playback volume or speed".into());
                }
                let mut value = record(store, &file)?;
                if duration_ms > 0 {
                    value.duration_ms = duration_ms;
                }
                // Periodic progress and shutdown saves cannot undo an explicit
                // completion. Only Restart starts a new viewing session.
                if !value.completed {
                    value.position_ms = if value.duration_ms > 0 {
                        position_ms.min(value.duration_ms)
                    } else {
                        position_ms
                    };
                }
                value.volume = volume;
                value.speed = speed;
                save(store, &value)?;
                store.put_record(
                    "playback_preferences",
                    "main",
                    &PlaybackPreferences { volume, speed },
                )?;
            }
            PlaybackMutation::Finish { file, duration_ms } => {
                let mut value = record(store, &file)?;
                value.completed = true;
                if duration_ms > 0 {
                    value.duration_ms = duration_ms;
                }
                if value.duration_ms > 0 {
                    value.position_ms = value.duration_ms;
                }
                save(store, &value)?;
            }
            PlaybackMutation::Restart { file } => {
                let mut value = record(store, &file)?;
                value.completed = false;
                value.position_ms = 0;
                save(store, &value)?;
            }
            PlaybackMutation::Bookmark {
                file,
                position_ms,
                label,
            } => {
                let label = label.trim();
                if label.is_empty() || label.chars().count() > 100 {
                    return Err("Bookmarks need a label of 1–100 characters".into());
                }
                let mut value = record(store, &file)?;
                let position_ms = if value.duration_ms > 0 {
                    position_ms.min(value.duration_ms)
                } else {
                    position_ms
                };
                value
                    .bookmarks
                    .retain(|bookmark| bookmark.position_ms != position_ms);
                if value.bookmarks.len() >= MAX_BOOKMARKS {
                    return Err("This file already has 100 bookmarks".into());
                }
                value.bookmarks.push(Bookmark {
                    position_ms,
                    label: label.into(),
                });
                value.bookmarks.sort_by_key(|bookmark| bookmark.position_ms);
                save(store, &value)?;
            }
            PlaybackMutation::RemoveBookmark { file, position_ms } => {
                let mut value = record(store, &file)?;
                value
                    .bookmarks
                    .retain(|bookmark| bookmark.position_ms != position_ms);
                save(store, &value)?;
            }
            PlaybackMutation::Forget { file } => {
                validate_file(&file)?;
                store.remove_record("playback", &media_id(store.owner, &file))?;
            }
            PlaybackMutation::Enqueue { files } => {
                if files.len() > MAX_QUEUE {
                    return Err("Choose up to 100 queue items".into());
                }
                let mut queue: Vec<PlaybackFile> =
                    store.record("playback_queue", "main")?.unwrap_or_default();
                for file in files {
                    validate_file(&file)?;
                    if !queue
                        .iter()
                        .any(|item| media_id(store.owner, item) == media_id(store.owner, &file))
                    {
                        queue.push(file);
                    }
                }
                if queue.len() > MAX_QUEUE {
                    return Err("Playback queue is limited to 100 files".into());
                }
                store.put_record("playback_queue", "main", &queue)?;
            }
            PlaybackMutation::RemoveQueue { file } => {
                validate_file(&file)?;
                let mut queue: Vec<PlaybackFile> =
                    store.record("playback_queue", "main")?.unwrap_or_default();
                queue.retain(|item| media_id(store.owner, item) != media_id(store.owner, &file));
                store.put_record("playback_queue", "main", &queue)?;
            }
            PlaybackMutation::ClearQueue => store.remove_record("playback_queue", "main")?,
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn cmd_playback_read(
    app: tauri::AppHandle,
    owner_id: String,
) -> Result<PlaybackSnapshot, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        let account = AccountGuard::open(&root, Some(&owner_id))?;
        let value = snapshot(&Store::open(&root, account.owner)?)?;
        account.validate()?;
        Ok(value)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn cmd_playback_mutate(
    app: tauri::AppHandle,
    owner_id: String,
    mutation: PlaybackMutation,
) -> Result<PlaybackSnapshot, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        let account = AccountGuard::open(&root, Some(&owner_id))?;
        let store = Store::open(&root, account.owner)?;
        account.validate()?;
        mutate(&store, mutation)?;
        account.validate()?;
        snapshot(&store)
    })
    .await
    .map_err(|error| error.to_string())?
}
