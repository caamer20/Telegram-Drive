use tauri::State;
use std::sync::Arc;

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
    *state.ms_session.lock().await = Some(session);
    Ok(info)
}




#[tauri::command]
pub async fn cmd_migration_ms_disconnect(
    state: State<'_, MigrationState>,
) -> Result<(), String> {
    *state.ms_session.lock().await = None;
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
    parent_id: Option<String>,
) -> Result<Vec<OneDriveItem>, String> {
    let http = reqwest::Client::new();
    let access_token = {
        let mut guard = state.ms_session.lock().await;
        if let Some(ref mut session) = *guard {
            if session.is_expired() {
                microsoft::refresh_access_token(microsoft::DEFAULT_MS_CLIENT_ID, session).await?;
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
    let mut folder_map: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
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
                .unwrap_or_else(|| if path.is_empty() { "Root".into() } else { path.clone() });

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
                microsoft::refresh_access_token(microsoft::DEFAULT_MS_CLIENT_ID, session).await?;
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
        return Err("Please complete job configuration (OneDrive folder and Local working dir)".into());
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
    state.pause_token.store(true, std::sync::atomic::Ordering::Relaxed);
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
    state.cancel_token.store(true, std::sync::atomic::Ordering::Relaxed);
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
) -> Result<Option<AutoMigrationProfile>, String> {
    let email = {
        let guard = state.ms_session.lock().await;
        guard.as_ref().map(|s| s.account_info.account_email.clone()).unwrap_or_default()
    };
    if email.is_empty() {
        return Ok(None);
    }
    db::get_auto_profile(&state.db, &email)
}

#[tauri::command]
pub async fn cmd_migration_toggle_auto(
    state: State<'_, MigrationState>,
    app_handle: tauri::AppHandle,
    enabled: bool,
) -> Result<AutoMigrationProfile, String> {
    let email = {
        let guard = state.ms_session.lock().await;
        guard.as_ref().map(|s| s.account_info.account_email.clone()).unwrap_or_default()
    };
    if email.is_empty() {
        return Err("Microsoft account not connected".into());
    }

    let profile = db::upsert_auto_profile(&state.db, &email, enabled, None, None, None)?;

    if enabled {
        let handle_clone = app_handle.clone();
        tokio::spawn(async move {
            let _ = crate::migration::auto_engine::start_auto_engine(handle_clone).await;
        });
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
        guard.as_ref().map(|s| s.account_info.account_email.clone()).unwrap_or_default()
    };
    if email.is_empty() {
        return Err("Microsoft account not connected".into());
    }

    let current = db::get_auto_profile(&state.db, &email)?.map(|p| p.enabled).unwrap_or(true);
    db::upsert_auto_profile(&state.db, &email, current, dest_id, dest_name.as_deref(), temp_dir.as_deref())
}

#[tauri::command]
pub async fn cmd_migration_get_daily_quota(
    state: State<'_, MigrationState>,
) -> Result<DailyMigrationQuota, String> {
    db::get_daily_quota(&state.db)
}


impl MigrationState {
    pub fn clone_state(&self) -> Arc<Self> {
        Arc::new(Self {
            db: self.db.clone(),
            ms_session: self.ms_session.clone(),
            worker_running: self.worker_running.clone(),
            cancel_token: self.cancel_token.clone(),
            pause_token: self.pause_token.clone(),
        })
    }
}
