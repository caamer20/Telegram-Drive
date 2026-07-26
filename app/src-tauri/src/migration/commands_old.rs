use tauri::{Manager, State};

use crate::migration::db;
use crate::migration::microsoft;
use crate::migration::models::*;
use crate::migration::worker;
use crate::migration::MigrationState;

#[tauri::command]
pub async fn cmd_migration_ms_connect(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    client_id: Option<String>,
    tenant: Option<String>,
    redirect_uri: Option<String>,
) -> Result<MsAccountInfo, String> {
    let cid = client_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| microsoft::DEFAULT_MS_CLIENT_ID.to_string());
    let t = tenant
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "common".to_string());
    let r = redirect_uri
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| microsoft::DEFAULT_REDIRECT_URI.to_string());

    let session = microsoft::start_oauth_flow(&cid, &t, &r, &app_handle).await?;
    let info = session.account_info.clone();
    crate::migration::session_store::save(&app_handle, &session)?;
    *state.ms_session.lock().await = Some(session);
    if let Ok(mut progress) = state.scan_progress.lock() {
        *progress = None;
    }

    Ok(info)
}

#[tauri::command]
pub async fn cmd_migration_ms_disconnect(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    *state.ms_session.lock().await = None;
    if let Ok(mut progress) = state.scan_progress.lock() {
        *progress = None;
    }
    crate::migration::session_store::delete(&app_handle)?;
    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_ms_status(
    state: State<'_, MigrationState>,
) -> Result<Option<MsAccountInfo>, String> {
    let session_guard = state.ms_session.lock().await;
    if let Some(ref session) = *session_guard {
        Ok(Some(session.account_info.clone()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn cmd_migration_list_onedrive_folders(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    parent_id: Option<String>,
) -> Result<Vec<OneDriveItem>, String> {
    let http = reqwest::Client::new();
    let access_token = {
        let mut guard = state.ms_session.lock().await;
        if let Some(ref mut session) = *guard {
            if session.is_expired() {
                microsoft::refresh_access_token(session).await?;
                crate::migration::session_store::save(&app_handle, session)?;
            }
            session.access_token.clone()
        } else {
            return Err("Microsoft account not connected".into());
        }
    };

    microsoft::list_children(&http, &access_token, parent_id.as_deref()).await
}

#[tauri::command]
pub async fn cmd_migration_create_job(
    state: State<'_, MigrationState>,
) -> Result<MigrationJob, String> {
    db::create_job(&state.db)
}

#[tauri::command]
pub async fn cmd_migration_get_jobs(
    state: State<'_, MigrationState>,
) -> Result<Vec<MigrationJobSummary>, String> {
    db::get_jobs(&state.db)
}

#[tauri::command]
pub async fn cmd_migration_get_job(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<MigrationJobDetail, String> {
    let job = db::get_job(&state.db, job_id)?;
    let stats = db::get_job_stats(&state.db, job_id)?;
    let items = db::get_items_by_job(&state.db, job_id)?;

    // Calculate folder summaries
    let mut folder_map: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    for item in &items {
        if item.item_type == "file" {
            let parent_path = std::path::Path::new(&item.source_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let entry = folder_map.entry(parent_path).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += item.size_bytes;
        }
    }

    let folders = folder_map
        .into_iter()
        .map(|(path, (count, size))| {
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| {
                    if path.is_empty() {
                        "Root".into()
                    } else {
                        path.clone()
                    }
                });

            FolderSummary {
                source_path: path,
                name,
                file_count: count,
                total_size: size,
            }
        })
        .collect();

    Ok(MigrationJobDetail {
        job,
        stats,
        folders,
        files: items,
    })
}

#[tauri::command]
pub async fn cmd_migration_delete_job(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<(), String> {
    db::delete_job(&state.db, job_id)
}

#[tauri::command]
pub async fn cmd_migration_set_onedrive_folder(
    state: State<'_, MigrationState>,
    job_id: i64,
    folder_id: String,
    folder_path: String,
) -> Result<(), String> {
    db::set_onedrive_folder(&state.db, job_id, folder_id, folder_path)
}

#[tauri::command]
pub async fn cmd_migration_set_telegram_destination(
    state: State<'_, MigrationState>,
    job_id: i64,
    destination_id: Option<i64>,
    destination_name: String,
) -> Result<(), String> {
    db::set_telegram_destination(&state.db, job_id, destination_id, destination_name)
}

#[tauri::command]
pub async fn cmd_migration_set_local_dir(
    state: State<'_, MigrationState>,
    job_id: i64,
    local_dir: String,
) -> Result<(), String> {
    let path = std::path::Path::new(&local_dir);
    if !path.exists() {
        return Err(format!("Local directory does not exist: {}", local_dir));
    }
    db::set_local_dir(&state.db, job_id, local_dir)
}

#[tauri::command]
pub async fn cmd_migration_scan(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<MigrationStats, String> {
    let job = db::get_job(&state.db, job_id)?;
    let folder_id = job
        .onedrive_folder_id
        .ok_or_else(|| "OneDrive source folder not set".to_string())?;

    let http = reqwest::Client::new();
    let access_token = {
        let mut guard = state.ms_session.lock().await;
        if let Some(ref mut session) = *guard {
            if session.is_expired() {
                microsoft::refresh_access_token(session).await?;
                crate::migration::session_store::save(&app_handle, session)?;
            }
            session.access_token.clone()
        } else {
            return Err("Microsoft account not connected".into());
        }
    };

    let items = microsoft::scan_folder_recursive(&http, &access_token, &folder_id, "").await?;
    db::batch_insert_items(&state.db, job_id, &items)
}

#[tauri::command]
pub async fn cmd_migration_start(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<(), String> {
    let job = db::get_job(&state.db, job_id)?;
    if job.state != "ready" && job.state != "paused" && job.state != "draft" {
        return Err(format!("Cannot start job in '{}' state", job.state));
    }

    if job.onedrive_folder_id.is_none() || job.local_dir.is_none() {
        return Err(
            "Please complete job configuration (OneDrive folder and Local working dir)".into(),
        );
    }

    let mig_state = state.inner().clone_state();
    tauri::async_runtime::spawn(async move {
        worker::run_migration_worker(mig_state, job_id, app_handle).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_pause(
    state: State<'_, MigrationState>,
    _job_id: i64,
) -> Result<(), String> {
    state
        .pause_token
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_resume(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
) -> Result<(), String> {
    let job = db::get_job(&state.db, job_id)?;
    if job.state != "paused" && job.state != "failed" {
        return Err(format!("Cannot resume job in '{}' state", job.state));
    }

    let mig_state = state.inner().clone_state();
    tauri::async_runtime::spawn(async move {
        worker::run_migration_worker(mig_state, job_id, app_handle).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_cancel(
    state: State<'_, MigrationState>,
    _job_id: i64,
) -> Result<(), String> {
    state
        .cancel_token
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_retry_item(
    state: State<'_, MigrationState>,
    job_id: i64,
    item_id: i64,
) -> Result<(), String> {
    db::retry_item(&state.db, job_id, item_id)
}

#[tauri::command]
pub async fn cmd_migration_retry_all_failed(
    state: State<'_, MigrationState>,
    job_id: i64,
) -> Result<i64, String> {
    db::retry_all_failed(&state.db, job_id)
}

#[tauri::command]
pub async fn cmd_migration_get_auto_status(
    state: State<'_, MigrationState>,
) -> Result<AutoMigrationStatus, String> {
    let account = {
        let guard = state.ms_session.lock().await;
        guard.as_ref().map(|session| session.account_info.clone())
    };
    let profile = match account.as_ref() {
        Some(account) if !account.account_email.is_empty() => {
            db::get_auto_profile(&state.db, &account.account_email)?
        }
        _ => None,
    };
    let active_job = match profile.as_ref().and_then(|value| value.active_job_id) {
        Some(job_id) => Some(MigrationJobDetail {
            job: db::get_job(&state.db, job_id)?,
            stats: db::get_job_stats(&state.db, job_id)?,
            folders: Vec::new(),
            files: db::get_items_by_job(&state.db, job_id)?,
        }),
        None => None,
    };
    let in_memory_scan_progress = state
        .scan_progress
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let scan_progress = match (in_memory_scan_progress, account.as_ref()) {
        (Some(mut progress), _) => {
            // A crash/reload can leave a persisted `stopping` checkpoint even
            // though no scanner task exists anymore. Treat that stale state as
            // stopped so the UI can resume or reset it.
            if matches!(
                progress.phase.as_str(),
                "starting" | "enumerating" | "building_snapshot" | "stopping"
            ) && !state.scan_running.load(std::sync::atomic::Ordering::SeqCst)
            {
                progress.phase = "stopped".into();
                if let Some(account) = account.as_ref() {
                    let _ = db::set_auto_scan_status(&state.db, &account.account_email, "stopped");
                }
            }
            Some(progress)
        }
        (None, Some(account)) if !account.account_email.is_empty() => {
            db::get_auto_scan_checkpoint(&state.db, &account.account_email)?.map(|checkpoint| {
                ScanProgressPayload {
                    phase: if matches!(
                        checkpoint.status.as_str(),
                        "starting" | "enumerating" | "building_snapshot" | "stopping"
                    ) && !state.scan_running.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        "stopped".into()
                    } else {
                        checkpoint.status
                    },
                    pages_scanned: checkpoint.pages_scanned,
                    discovered_files: checkpoint.discovered_files,
                    discovered_folders: checkpoint.discovered_folders,
                    elapsed_ms: checkpoint.elapsed_ms,
                }
            })
        }
        _ => None,
    };
    Ok(AutoMigrationStatus {
        profile,
        account,
        active_job,
        scan_progress,
    })
}

#[tauri::command]
pub async fn cmd_migration_toggle_auto(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    enabled: bool,
) -> Result<AutoMigrationProfile, String> {
    let email = {
        let guard = state.ms_session.lock().await;
        guard
            .as_ref()
            .map(|s| s.account_info.account_email.clone())
            .unwrap_or_default()
    };
    if email.is_empty() {
        return Err("Microsoft account not connected".into());
    }

    let profile = db::upsert_auto_profile(&state.db, &email, enabled, None, None, None)?;

    if enabled {
        state
            .pause_token
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let handle_clone = app_handle.clone();
        tokio::spawn(async move {
            let _ = crate::migration::auto_engine::start_auto_engine(handle_clone).await;
        });
    } else {
        state
            .pause_token
            .store(true, std::sync::atomic::Ordering::Relaxed);
        db::pause_auto_job(&state.db, &email, "user")?;
    }

    Ok(profile)
}

#[tauri::command]
pub async fn cmd_migration_update_auto_settings(
    state: State<'_, MigrationState>,
    dest_id: Option<i64>,
    dest_name: Option<String>,
    temp_dir: Option<String>,
) -> Result<AutoMigrationProfile, String> {
    let email = {
        let guard = state.ms_session.lock().await;
        guard
            .as_ref()
            .map(|s| s.account_info.account_email.clone())
            .unwrap_or_default()
    };
    if email.is_empty() {
        return Err("Microsoft account not connected".into());
    }

    let current = db::get_auto_profile(&state.db, &email)?
        .map(|p| p.enabled)
        .unwrap_or(true);
    db::upsert_auto_profile(
        &state.db,
        &email,
        current,
        dest_id,
        dest_name.as_deref(),
        temp_dir.as_deref(),
    )
}

#[tauri::command]
pub async fn cmd_migration_get_daily_quota(
    state: State<'_, MigrationState>,
) -> Result<DailyMigrationQuota, String> {
    db::get_daily_quota(&state.db)
}

#[tauri::command]
pub async fn cmd_migration_rescan_auto(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    reset: Option<bool>,
) -> Result<(), String> {
    if reset.unwrap_or(false) {
        if state.scan_running.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("scan_already_running".into());
        }
        if state
            .worker_running
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err("migration_running".into());
        }
        let account_email = {
            let session = state.ms_session.lock().await;
            session
                .as_ref()
                .map(|value| value.account_info.account_email.clone())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "microsoft_not_connected".to_string())?
        };
        db::clear_auto_scan_checkpoint(&state.db, &account_email)?;
        if let Ok(mut progress) = state.scan_progress.lock() {
            *progress = None;
        }
    }
    crate::migration::auto_engine::start_rescan_auto_engine(app_handle)
}

#[tauri::command]
pub async fn cmd_migration_stop_auto_scan(
    app_handle: tauri::AppHandle,
) -> Result<ScanProgressPayload, String> {
    crate::migration::auto_engine::request_stop_auto_scan(app_handle).await
}

#[tauri::command]
pub async fn cmd_migration_get_scan_snapshot(
    state: State<'_, MigrationState>,
) -> Result<Vec<OneDriveItem>, String> {
    load_stopped_scan_snapshot(state.inner()).await
}

async fn load_stopped_scan_snapshot(state: &MigrationState) -> Result<Vec<OneDriveItem>, String> {
    let account_email = {
        let session = state.ms_session.lock().await;
        session
            .as_ref()
            .map(|value| value.account_info.account_email.clone())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "microsoft_not_connected".to_string())?
    };
    let values = db::load_auto_scan_items(&state.db, &account_email)?;
    if !values.is_empty() {
        return Ok(microsoft::build_delta_snapshot(&values));
    }
    let _checkpoint = db::get_auto_scan_checkpoint(&state.db, &account_email)?
        .ok_or_else(|| "scan_snapshot_not_available".to_string())?;
    let values = db::load_auto_scan_items(&state.db, &account_email)?;
    if values.is_empty() {
        return Err("scan_snapshot_not_available".to_string());
    }
    Ok(microsoft::build_delta_snapshot(&values))
}

#[tauri::command]
pub async fn cmd_migration_sync_scan_snapshot_item(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    source_item_id: String,
) -> Result<i64, String> {
    if state
        .worker_running
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("migration_running".into());
    }

    let account_email = {
        let session = state.ms_session.lock().await;
        session
            .as_ref()
            .map(|value| value.account_info.account_email.clone())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "microsoft_not_connected".to_string())?
    };
    let item = load_stopped_scan_snapshot(state.inner())
        .await?
        .into_iter()
        .find(|item| item.id == source_item_id)
        .ok_or_else(|| "scan_snapshot_item_not_found".to_string())?;
    if item.item_type != "file" {
        return Err("scan_snapshot_item_not_file".into());
    }

    let profile = db::get_auto_profile(&state.db, &account_email)?
        .ok_or_else(|| "auto_migration_profile_not_configured".to_string())?;
    let temp_dir = match profile.local_temp_dir {
        Some(dir) if !dir.trim().is_empty() => dir,
        _ => {
            let dir = app_handle
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?
                .join("temp_migration");
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            dir.to_string_lossy().to_string()
        }
    };
    let destination_name = profile
        .default_telegram_dest_name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Saved Messages".to_string());
    let (job_id, _) = db::create_migration_job(
        &state.db,
        Some("root"),
        Some("/ (Root)"),
        profile.default_telegram_dest_id,
        Some(&destination_name),
        &temp_dir,
        &[item],
    )?;

    let migration_state = state.inner().clone_state();
    tauri::async_runtime::spawn(async move {
        worker::run_migration_worker(migration_state, job_id, app_handle).await;
    });

    Ok(job_id)
}

#[tauri::command]
pub async fn cmd_migration_get_activity(
    state: State<'_, MigrationState>,
    job_id: i64,
    limit: Option<i64>,
) -> Result<Vec<MigrationActivity>, String> {
    db::get_activity(&state.db, job_id, limit.unwrap_or(100))
}

#[tauri::command]
pub async fn cmd_migration_delete_item(
    state: State<'_, MigrationState>,
    job_id: i64,
    item_id: i64,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    // 1. Fetch item detail from DB to get OneDrive source_item_id
    if let Ok(Some(item)) = db::get_item_by_id(&state.db, item_id) {
        if let Some(ref ms_item_id) = item.source_item_id {
            if !ms_item_id.is_empty() {
                // 2. Get valid access token from Microsoft session
                let access_token = {
                    let mut guard = state.ms_session.lock().await;
                    if let Some(ref mut session) = *guard {
                        if session.is_expired()
                            && microsoft::refresh_access_token(session).await.is_ok()
                        {
                            let _ = crate::migration::session_store::save(&app_handle, session);
                        }
                        Some(session.access_token.clone())
                    } else {
                        None
                    }
                };

                // 3. Delete file from OneDrive via Graph API
                if let Some(token) = access_token {
                    let http = reqwest::Client::new();
                    if let Err(e) = microsoft::delete_onedrive_item(&http, &token, ms_item_id).await
                    {
                        log::warn!(
                            "Failed to delete item from OneDrive (will still remove from DB): {}",
                            e
                        );
                    } else {
                        log::info!(
                            "Successfully deleted item {} ({}) from OneDrive",
                            item.name,
                            ms_item_id
                        );
                    }
                }
            }
        }
    }

    // 4. Remove item from migration DB
    db::delete_item(&state.db, job_id, item_id)
}

#[tauri::command]
pub async fn cmd_migration_rename_item(
    state: State<'_, MigrationState>,
    job_id: i64,
    item_id: i64,
    new_name: String,
) -> Result<(), String> {
    db::rename_item(&state.db, job_id, item_id, &new_name)
}

#[tauri::command]
pub async fn cmd_migration_sync_single_item(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    job_id: i64,
    item_id: i64,
) -> Result<(), String> {
    db::retry_item(&state.db, job_id, item_id)?;

    let mig_state = state.inner().clone_state();
    tauri::async_runtime::spawn(async move {
        worker::run_migration_worker(mig_state, job_id, app_handle).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn cmd_migration_queue_selected_items(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    source_item_ids: Vec<String>,
    action_type: String,
) -> Result<i64, String> {
    if state
        .worker_running
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("migration_running".into());
    }

    let account_email = {
        let session = state.ms_session.lock().await;
        session
            .as_ref()
            .map(|value| value.account_info.account_email.clone())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "microsoft_not_connected".to_string())?
    };

    let snapshot_items = load_stopped_scan_snapshot(state.inner()).await?;

    let selected_folder_paths: Vec<String> = snapshot_items
        .iter()
        .filter(|item| {
            let path = item.path.as_deref().unwrap_or(&item.name);
            item.item_type == "folder"
                && (source_item_ids.contains(&item.id)
                    || source_item_ids.iter().any(|id| id == path)
                    || source_item_ids.contains(&item.name))
        })
        .map(|item| {
            let path = item.path.as_deref().unwrap_or(&item.name);
            format!("{}/", path.trim_end_matches('/'))
        })
        .collect();

    let target_file_items: Vec<OneDriveItem> = snapshot_items
        .into_iter()
        .filter(|item| {
            if item.item_type != "file" {
                return false;
            }
            let item_path = item.path.as_deref().unwrap_or(&item.name);
            if source_item_ids.contains(&item.id)
                || source_item_ids.iter().any(|id| id == item_path)
                || source_item_ids.contains(&item.name)
            {
                return true;
            }
            selected_folder_paths
                .iter()
                .any(|prefix| item_path.starts_with(prefix))
        })
        .collect();

    if target_file_items.is_empty() {
        return Err("no_items_selected".into());
    }

    let profile = db::get_auto_profile(&state.db, &account_email)?
        .ok_or_else(|| "auto_migration_profile_not_configured".to_string())?;

    let temp_dir = match profile.local_temp_dir {
        Some(dir) if !dir.trim().is_empty() => dir,
        _ => {
            let dir = app_handle
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?
                .join("temp_migration");
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            dir.to_string_lossy().to_string()
        }
    };

    let destination_name = profile
        .default_telegram_dest_name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Saved Messages".to_string());

    let (job_id, _) = db::create_migration_job_with_action(
        &state.db,
        Some("root"),
        Some("/ (Root)"),
        profile.default_telegram_dest_id,
        Some(&destination_name),
        &temp_dir,
        &target_file_items,
        Some(&action_type),
    )?;

    let migration_state = state.inner().clone_state();
    tauri::async_runtime::spawn(async move {
        worker::run_migration_worker(migration_state, job_id, app_handle).await;
    });

    Ok(job_id)
}

