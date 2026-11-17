use crate::{
    db::DbConnection,
    sync_engine::{
        config::{self, SyncPair, SyncSettings},
        policy::{StoredPairPolicy, SyncPreferences},
        preview::{self, SyncPreview, SyncPreviewRequest},
        restart_sync_engine, SyncEngine, SyncStatus,
    },
};
use serde::Serialize;
use sqlite::State as SqliteState;
use std::path::Path;
use tauri::{Manager, State};

fn request_account(
    app: &tauri::AppHandle,
    owner_id: &str,
) -> Result<crate::workspace::AccountGuard, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    crate::workspace::AccountGuard::open(&root, Some(owner_id))
}

fn change_account<T>(
    connection: &sqlite::Connection,
    account: &crate::workspace::AccountGuard,
    operation: impl FnOnce(&sqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    connection
        .execute("BEGIN IMMEDIATE")
        .map_err(|error| error.to_string())?;
    let result = (|| {
        account.validate()?;
        let value = operation(connection)?;
        account.validate()?;
        Ok(value)
    })();
    match result {
        Ok(value) => match connection.execute("COMMIT") {
            Ok(()) => Ok(value),
            Err(error) => {
                let _ = connection.execute("ROLLBACK");
                Err(error.to_string())
            }
        },
        Err(error) => {
            let _ = connection.execute("ROLLBACK");
            Err(error)
        }
    }
}

/// Validate the stored mapping inside the same transaction as its mutation.
/// Unowned legacy mappings may be disabled or removed, but require review to run.
fn change_owned_pair<T>(
    connection: &sqlite::Connection,
    account: &crate::workspace::AccountGuard,
    pair_id: i64,
    allow_unreviewed: bool,
    operation: impl FnOnce(&sqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    change_account(connection, account, |connection| {
        let mut statement = connection.prepare("SELECT s.value FROM sync_pairs p LEFT JOIN sync_settings s ON s.key = 'sync_pair_policy:' || p.id WHERE p.id = ?")
            .map_err(|error| error.to_string())?;
        statement
            .bind((1, pair_id))
            .map_err(|error| error.to_string())?;
        if statement.next().map_err(|error| error.to_string())? != SqliteState::Row {
            return Err("Sync mapping was not found".into());
        }
        let stored = statement
            .read::<Option<String>, _>(0)
            .map_err(|error| error.to_string())?;
        let owner = stored
            .map(|value| {
                serde_json::from_str::<StoredPairPolicy>(&value)
                    .map(|policy| policy.account_owner)
                    .map_err(|_| "Stored sync mapping policy is unreadable".to_string())
            })
            .transpose()?
            .flatten();
        if owner
            .as_deref()
            .is_some_and(|owner| owner != account.owner.to_string())
            || (owner.is_none() && !allow_unreviewed)
        {
            return Err(
                "ACCOUNT_CHANGED: Review this mapping under its original Telegram account".into(),
            );
        }
        drop(statement);
        operation(connection)
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncLogEntry {
    pub id: i64,
    pub pair_id: Option<i64>,
    pub action: String,
    pub relative_path: Option<String>,
    pub detail: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    pub pair_id: i64,
    pub relative_path: String,
    pub local_path: String,
    pub label: Option<String>,
}

fn sync_paths_overlap(left: &Path, right: &Path) -> bool {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let components = |path: &Path| {
            path.components()
                .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
                .collect::<Vec<_>>()
        };
        let left = components(left);
        let right = components(right);
        left.starts_with(&right) || right.starts_with(&left)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        left.starts_with(right) || right.starts_with(left)
    }
}

async fn with_sync_paused<T>(
    app: &tauri::AppHandle,
    operation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let engine = app.state::<SyncEngine>();
    let _reconfigure = engine.reconfigure_lock.lock().await;
    engine.shutdown_and_wait().await?;
    let result = {
        let _lock = engine.operation_lock.lock().await;
        operation.await
    };
    let restart = engine.start().await;
    match result {
        Ok(value) => {
            restart?;
            Ok(value)
        }
        Err(error) => {
            if let Err(restart_error) = restart {
                log::error!(
                    "Could not resume other sync mappings after a settings error: {restart_error}"
                );
            }
            Err(error)
        }
    }
}

async fn visible_pairs(
    db: &DbConnection,
    include_unreviewed: bool,
    account: &crate::workspace::AccountGuard,
) -> Result<Vec<SyncPair>, String> {
    account.validate()?;
    let owner = account.owner.to_string();
    let pairs = config::load_pairs(db.clone(), false).await?;
    account.validate()?;
    Ok(pairs
        .into_iter()
        .filter(|pair| {
            pair.account_owner.as_deref() == Some(owner.as_str())
                || (include_unreviewed && pair.account_owner.is_none())
        })
        .collect())
}

#[tauri::command]
pub async fn cmd_get_sync_settings(
    app: tauri::AppHandle,
    owner_id: String,
    db: State<'_, DbConnection>,
) -> Result<SyncSettings, String> {
    let account = request_account(&app, &owner_id)?;
    let result = config::load_settings(db.inner().clone()).await?;
    account.validate()?;
    Ok(result)
}

#[tauri::command]
pub async fn cmd_toggle_sync(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    enabled: bool,
    owner_id: String,
) -> Result<SyncSettings, String> {
    let account = request_account(&app, &owner_id)?;
    let account_for_db = account.clone();
    crate::db::with_connection(db.inner().clone(), move |connection| {
      change_account(connection, &account_for_db, |connection| {
        let mut statement = connection.prepare("INSERT INTO sync_settings (key, value) VALUES ('sync_enabled', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .map_err(|error| error.to_string())?;
        statement.bind((1, if enabled { "true" } else { "false" })).map_err(|error| error.to_string())?;
        statement.next().map_err(|error| error.to_string())?;
        Ok(())
      })
    }).await?;
    account.validate()?;
    restart_sync_engine(&app).await?;
    let result = config::load_settings(db.inner().clone()).await?;
    account.validate()?;
    Ok(result)
}

/// Chooses how mapped folders are scanned. The incremental scanner is opt-in:
/// it trusts recorded hashes for files whose size and modification time are
/// unchanged, and re-verifies every file periodically.
#[tauri::command]
pub async fn cmd_set_sync_scanner(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    scanner: String,
    owner_id: String,
) -> Result<SyncSettings, String> {
    if !matches!(
        scanner.as_str(),
        config::SCANNER_FULL | config::SCANNER_INCREMENTAL
    ) {
        return Err("Choose the full or the incremental sync scanner".into());
    }
    let account = request_account(&app, &owner_id)?;
    let account_for_db = account.clone();
    crate::db::with_connection(db.inner().clone(), move |connection| {
        change_account(connection, &account_for_db, |connection| {
            let mut statement = connection.prepare("INSERT INTO sync_settings (key, value) VALUES ('sync_scanner', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .map_err(|error| error.to_string())?;
            statement
                .bind((1, scanner.as_str()))
                .map_err(|error| error.to_string())?;
            statement.next().map_err(|error| error.to_string())?;
            Ok(())
        })
    })
    .await?;
    account.validate()?;
    restart_sync_engine(&app).await?;
    let result = config::load_settings(db.inner().clone()).await?;
    account.validate()?;
    Ok(result)
}

#[tauri::command]
// Named IPC fields preserve the existing sync command contract.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_add_sync_pair(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    local_path: String,
    channel_id: i64,
    label: Option<String>,
    sync_direction: Option<String>,
    preferences: Option<SyncPreferences>,
    preview_token: Option<String>,
    is_active: Option<bool>,
    owner_id: String,
) -> Result<SyncPair, String> {
    let request = SyncPreviewRequest {
        pair_id: None,
        local_path,
        channel_id,
        sync_direction: sync_direction.unwrap_or_else(|| "upload_only".into()),
        preferences: preferences.unwrap_or_default(),
    }
    .normalized()?;
    let account = preview::consume_review(&app, &request, preview_token.as_deref(), &owner_id)?;
    with_sync_paused(&app, async {
    account.validate()?;
    let canonical = Path::new(&request.local_path).to_path_buf();
    let local_path = request.local_path.clone();
    let direction = request.sync_direction.clone();
    let preferences = request.preferences.clone();
    let is_active = is_active.unwrap_or(false);
    let created_at = crate::app_time::unix_seconds();
    let folder_key = channel_id.to_string();
    let canonical_for_check = canonical.clone();
    let local_path_for_db = local_path.clone();
    let folder_key_for_db = folder_key.clone();
    let label_for_db = label.clone();
    let direction_for_db = direction.clone();
    let policy_for_db = StoredPairPolicy { account_owner: Some(account.owner.to_string()), preferences: preferences.clone() };
    let account_for_db = account.clone();
    let (id, fallback_label) = crate::db::with_connection(db.inner().clone(), move |connection| {
        connection.execute("BEGIN IMMEDIATE").map_err(|error| error.to_string())?;
        let result = (|| {
        account_for_db.validate()?;
        let mut existing_pairs = connection
            .prepare("SELECT local_path, channel_id FROM sync_pairs")
            .map_err(|error| error.to_string())?;
        while existing_pairs.next().map_err(|error| error.to_string())? == SqliteState::Row {
            let existing_path: String =
                existing_pairs.read(0).map_err(|error| error.to_string())?;
            let existing_channel: i64 =
                existing_pairs.read(1).map_err(|error| error.to_string())?;
            if existing_channel == channel_id {
                return Err("A Telegram channel can be mapped to only one local folder".to_string());
            }
            if sync_paths_overlap(&canonical_for_check, Path::new(&existing_path)) {
                return Err(
                    "Sync folders cannot be identical, nested, or contain another sync folder"
                        .to_string(),
                );
            }
        }
        drop(existing_pairs);
        let mut channel = connection
            .prepare("SELECT name FROM folder_metadata WHERE channel_id = ?")
            .map_err(|error| error.to_string())?;
        channel
            .bind((1, channel_id))
            .map_err(|error| error.to_string())?;
        if channel.next().map_err(|error| error.to_string())? != SqliteState::Row {
            return Err("Selected Telegram channel is not in the folder list".to_string());
        }
        let fallback_label = channel.read::<String, _>(0).ok();
        drop(channel);
        let mut statement = connection.prepare(
            "INSERT INTO sync_pairs (local_path, channel_id, folder_key, label, sync_direction, is_active, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        ).map_err(|error| error.to_string())?;
        statement
            .bind((1, local_path_for_db.as_str()))
            .map_err(|error| error.to_string())?;
        statement
            .bind((2, channel_id))
            .map_err(|error| error.to_string())?;
        statement
            .bind((3, folder_key_for_db.as_str()))
            .map_err(|error| error.to_string())?;
        statement
            .bind::<(usize, Option<&str>)>((4, label_for_db.as_deref().or(fallback_label.as_deref())))
            .map_err(|error| error.to_string())?;
        statement
            .bind((5, direction_for_db.as_str()))
            .map_err(|error| error.to_string())?;
        statement
            .bind((6, i64::from(is_active)))
            .map_err(|error| error.to_string())?;
        statement
            .bind((7, created_at))
            .map_err(|error| error.to_string())?;
        statement.next().map_err(|error| error.to_string())?;
        drop(statement);
        let mut id_statement = connection
            .prepare("SELECT last_insert_rowid()")
            .map_err(|error| error.to_string())?;
        id_statement.next().map_err(|error| error.to_string())?;
        let id = id_statement
            .read::<i64, _>(0)
            .map_err(|error| error.to_string())?;
        drop(id_statement);
        config::write_pair_policy(connection, id, &policy_for_db)?;
        account_for_db.validate()?;
        Ok((id, fallback_label))
        })();
        match result {
            Ok(value) => { connection.execute("COMMIT").map_err(|error| error.to_string())?; Ok(value) }
            Err(error) => { let _ = connection.execute("ROLLBACK"); Err(error) }
        }
    }).await?;
    account.validate()?;
    Ok(SyncPair {
        id,
        local_path,
        channel_id,
        folder_key,
        label: label.or(fallback_label),
        sync_direction: direction,
        is_active,
        created_at,
        account_owner: Some(account.owner.to_string()),
        preferences,
    })
    }).await
}

#[tauri::command]
pub async fn cmd_preview_sync_pair(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    request: SyncPreviewRequest,
    owner_id: String,
) -> Result<SyncPreview, String> {
    preview::preview_pair(&app, db.inner(), request, &owner_id).await
}

#[tauri::command]
pub async fn cmd_update_sync_pair(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    request: SyncPreviewRequest,
    preview_token: String,
    is_active: bool,
    owner_id: String,
) -> Result<SyncPair, String> {
    let request = request.normalized()?;
    let pair_id = request
        .pair_id
        .ok_or("Choose an existing mapping to update")?;
    let account = preview::consume_review(&app, &request, Some(&preview_token), &owner_id)?;
    let pair = config::load_pairs(db.inner().clone(), false)
        .await?
        .into_iter()
        .find(|pair| pair.id == pair_id)
        .ok_or("Sync mapping was not found")?;
    if pair.local_path != request.local_path
        || pair.channel_id != request.channel_id
        || pair
            .account_owner
            .as_deref()
            .is_some_and(|owner| owner != account.owner.to_string())
    {
        return Err(
            "This mapping changed or belongs to another Telegram account; preview it again".into(),
        );
    }
    with_sync_paused(&app, async {
        account.validate()?;
        let direction = request.sync_direction.clone();
        let policy = StoredPairPolicy {
            account_owner: Some(account.owner.to_string()),
            preferences: request.preferences.clone(),
        };
        let account_for_db = account.clone();
        crate::db::with_connection(db.inner().clone(), move |connection| {
            connection
                .execute("BEGIN IMMEDIATE")
                .map_err(|error| error.to_string())?;
            let result = (|| {
                account_for_db.validate()?;
                let mut statement = connection
                    .prepare("UPDATE sync_pairs SET sync_direction = ?, is_active = ? WHERE id = ?")
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((1, direction.as_str()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((2, i64::from(is_active)))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((3, pair_id))
                    .map_err(|error| error.to_string())?;
                statement.next().map_err(|error| error.to_string())?;
                drop(statement);
                config::write_pair_policy(connection, pair_id, &policy)?;
                account_for_db.validate()?;
                Ok(())
            })();
            match result {
                Ok(()) => connection
                    .execute("COMMIT")
                    .map_err(|error| error.to_string()),
                Err(error) => {
                    let _ = connection.execute("ROLLBACK");
                    Err(error)
                }
            }
        })
        .await?;
        account.validate()?;
        Ok(SyncPair {
            sync_direction: request.sync_direction,
            preferences: request.preferences,
            account_owner: Some(account.owner.to_string()),
            is_active,
            ..pair
        })
    })
    .await
}

#[tauri::command]
pub async fn cmd_set_sync_pair_active(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    pair_id: i64,
    is_active: bool,
    owner_id: String,
) -> Result<(), String> {
    let account = request_account(&app, &owner_id)?;
    with_sync_paused(&app, async {
        account.validate()?;
        crate::db::with_connection(db.inner().clone(), move |connection| {
            change_owned_pair(connection, &account, pair_id, !is_active, |connection| {
                let mut statement = connection
                    .prepare("UPDATE sync_pairs SET is_active = ? WHERE id = ?")
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((1, i64::from(is_active)))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((2, pair_id))
                    .map_err(|error| error.to_string())?;
                statement.next().map_err(|error| error.to_string())?;
                Ok(())
            })
        })
        .await
    })
    .await
}

#[tauri::command]
pub async fn cmd_get_sync_pairs(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    owner_id: String,
) -> Result<Vec<SyncPair>, String> {
    let account = request_account(&app, &owner_id)?;
    visible_pairs(db.inner(), true, &account).await
}

#[tauri::command]
pub async fn cmd_remove_sync_pair(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    pair_id: i64,
    owner_id: String,
) -> Result<(), String> {
    let account = request_account(&app, &owner_id)?;
    with_sync_paused(&app, async {
        account.validate()?;
        crate::db::with_connection(db.inner().clone(), move |connection| {
            change_owned_pair(connection, &account, pair_id, true, |connection| {
                let mut statement = connection
                    .prepare("DELETE FROM sync_state WHERE pair_id = ?")
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((1, pair_id))
                    .map_err(|error| error.to_string())?;
                statement.next().map_err(|error| error.to_string())?;
                drop(statement);
                let mut statement = connection
                    .prepare("DELETE FROM sync_pairs WHERE id = ?")
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((1, pair_id))
                    .map_err(|error| error.to_string())?;
                statement.next().map_err(|error| error.to_string())?;
                drop(statement);
                let mut settings = connection
                    .prepare("DELETE FROM sync_settings WHERE key IN (?, ?)")
                    .map_err(|error| error.to_string())?;
                settings
                    .bind((1, format!("sync_pair_policy:{pair_id}").as_str()))
                    .map_err(|error| error.to_string())?;
                settings
                    .bind((2, format!("sync_pair_cleanup:{pair_id}").as_str()))
                    .map_err(|error| error.to_string())?;
                settings.next().map_err(|error| error.to_string())?;
                Ok::<(), String>(())
            })
        })
        .await
    })
    .await
}

#[tauri::command]
pub async fn cmd_get_sync_status(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    engine: State<'_, SyncEngine>,
    owner_id: String,
) -> Result<SyncStatus, String> {
    let account = request_account(&app, &owner_id)?;
    let pairs = visible_pairs(db.inner(), true, &account).await?;
    let mut status = engine.status.read().await.clone();
    status
        .pairs
        .retain(|status| pairs.iter().any(|pair| pair.id == status.pair_id));
    status.active_pairs = pairs.iter().filter(|pair| pair.is_active).count();
    let conflict_counts = crate::db::with_connection(db.inner().clone(), |connection| {
        let mut statement = connection.prepare("SELECT pair_id, COUNT(*) FROM sync_state WHERE sync_status = 'conflict' GROUP BY pair_id").map_err(|error| error.to_string())?;
        let mut counts = std::collections::HashMap::new();
        while statement.next().map_err(|error| error.to_string())? == SqliteState::Row {
            counts.insert(statement.read::<i64, _>(0).map_err(|error| error.to_string())?, statement.read::<i64, _>(1).map_err(|error| error.to_string())?.max(0) as usize);
        }
        Ok(counts)
    }).await?;
    for pair in &mut status.pairs {
        pair.conflicts = conflict_counts.get(&pair.pair_id).copied().unwrap_or(0);
    }
    status.conflicts = status.pairs.iter().map(|pair| pair.conflicts).sum();
    status.pending_ops = status.pairs.iter().map(|pair| pair.pending_ops).sum();
    status.last_error = status
        .pairs
        .iter()
        .filter(|status| {
            pairs
                .iter()
                .any(|pair| pair.id == status.pair_id && pair.is_active)
        })
        .find_map(|pair| pair.last_error.clone());
    account.validate()?;
    Ok(status)
}

#[tauri::command]
pub async fn cmd_get_sync_conflicts(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    owner_id: String,
) -> Result<Vec<SyncConflict>, String> {
    let account = request_account(&app, &owner_id)?;
    let allowed: std::collections::HashSet<_> = visible_pairs(db.inner(), false, &account)
        .await?
        .into_iter()
        .map(|pair| pair.id)
        .collect();
    crate::db::with_connection(db.inner().clone(), move |connection| {
    let mut statement = connection.prepare(
        "SELECT s.pair_id, s.relative_path, p.local_path, p.label FROM sync_state s JOIN sync_pairs p ON p.id = s.pair_id WHERE s.sync_status = 'conflict' ORDER BY s.pair_id, s.relative_path",
    ).map_err(|error| error.to_string())?;
    let mut conflicts = Vec::new();
    while statement.next().map_err(|error| error.to_string())? == SqliteState::Row {
        if !allowed.contains(&statement.read::<i64, _>(0).map_err(|error| error.to_string())?) { continue; }
        conflicts.push(SyncConflict {
            pair_id: statement.read(0).map_err(|error| error.to_string())?,
            relative_path: statement.read(1).map_err(|error| error.to_string())?,
            local_path: statement.read(2).map_err(|error| error.to_string())?,
            label: statement.read::<Option<String>, _>(3).ok().flatten(),
        });
    }
    account.validate()?;
    Ok(conflicts)
    }).await
}

#[tauri::command]
pub async fn cmd_get_sync_log(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    limit: Option<i64>,
    owner_id: String,
) -> Result<Vec<SyncLogEntry>, String> {
    let account = request_account(&app, &owner_id)?;
    let allowed: std::collections::HashSet<_> = visible_pairs(db.inner(), false, &account)
        .await?
        .into_iter()
        .map(|pair| pair.id)
        .collect();
    crate::db::with_connection(db.inner().clone(), move |connection| {
    let mut statement = connection.prepare(
        "SELECT id, pair_id, action, relative_path, detail, created_at FROM sync_log ORDER BY id DESC LIMIT ?",
    ).map_err(|error| error.to_string())?;
    statement
        .bind((1, limit.unwrap_or(100).clamp(1, 500)))
        .map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    while statement.next().map_err(|error| error.to_string())? == SqliteState::Row {
        if statement.read::<Option<i64>, _>(1).ok().flatten().is_some_and(|pair_id| !allowed.contains(&pair_id)) { continue; }
        entries.push(SyncLogEntry {
            id: statement.read(0).map_err(|error| error.to_string())?,
            pair_id: statement.read::<Option<i64>, _>(1).ok().flatten(),
            action: statement.read(2).map_err(|error| error.to_string())?,
            relative_path: statement.read::<Option<String>, _>(3).ok().flatten(),
            detail: statement.read::<Option<String>, _>(4).ok().flatten(),
            created_at: statement.read(5).map_err(|error| error.to_string())?,
        });
    }
    account.validate()?;
    Ok(entries)
    }).await
}

#[tauri::command]
pub async fn cmd_resolve_conflict(
    app: tauri::AppHandle,
    db: State<'_, DbConnection>,
    pair_id: i64,
    path: String,
    resolution: String,
    owner_id: String,
) -> Result<(), String> {
    if !matches!(
        resolution.as_str(),
        "keep_local" | "keep_remote" | "keep_both"
    ) {
        return Err("Unknown conflict resolution".to_string());
    }
    let account = request_account(&app, &owner_id)?;
    with_sync_paused(&app, async {
    account.validate()?;
    let path_for_db = path.clone();
    let resolution_for_db = resolution.clone();
    let account_for_db = account.clone();
    crate::db::with_connection(db.inner().clone(), move |connection| {
      change_owned_pair(connection, &account_for_db, pair_id, false, |connection| {
        let mut statement = connection.prepare(
            "UPDATE sync_state SET sync_status = ? WHERE pair_id = ? AND relative_path = ? AND sync_status = 'conflict'",
        ).map_err(|error| error.to_string())?;
        statement
            .bind((1, resolution_for_db.as_str()))
            .map_err(|error| error.to_string())?;
        statement
            .bind((2, pair_id))
            .map_err(|error| error.to_string())?;
        statement
            .bind((3, path_for_db.as_str()))
            .map_err(|error| error.to_string())?;
        statement.next().map_err(|error| error.to_string())?;
        Ok(())
      })
    }).await?;
    config::log_sync(
        db.inner().clone(),
        Some(pair_id),
        "resolve_conflict".to_string(),
        Some(path),
        Some(resolution),
    )
    .await;
    account.validate()?;
    Ok(())
    }).await
}
