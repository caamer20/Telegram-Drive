use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use reqwest::Client;
use tauri::{AppHandle, Emitter, Manager};

use crate::migration::db::*;
use crate::migration::microsoft::*;
use crate::migration::models::MigrationJobDetail;
use crate::migration::worker::run_migration_worker;
use crate::migration::MigrationState;

struct ScanRunningGuard(Arc<AtomicBool>);

impl Drop for ScanRunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

static AUTO_PIPELINE_STARTING: AtomicBool = AtomicBool::new(false);

struct AutoPipelineStartingGuard;

impl Drop for AutoPipelineStartingGuard {
    fn drop(&mut self) {
        AUTO_PIPELINE_STARTING.store(false, Ordering::SeqCst);
    }
}

fn publish_scan_progress(
    app_handle: &AppHandle,
    progress: crate::migration::models::ScanProgressPayload,
) {
    let state = app_handle.state::<MigrationState>();
    if let Ok(mut current) = state.scan_progress.lock() {
        *current = Some(progress.clone());
    }
    let _ = app_handle.emit("migration:scan-progress", progress);
}

fn publish_scan_failure(app_handle: &AppHandle) {
    let state = app_handle.state::<MigrationState>();
    let mut progress = state
        .scan_progress
        .lock()
        .ok()
        .and_then(|value| value.clone())
        .unwrap_or(crate::migration::models::ScanProgressPayload {
            phase: "failed".into(),
            pages_scanned: 0,
            discovered_files: 0,
            discovered_folders: 0,
            elapsed_ms: 0,
        });
    progress.phase = "failed".into();
    publish_scan_progress(app_handle, progress);
}

pub async fn request_stop_auto_scan(
    app_handle: AppHandle,
) -> Result<crate::migration::models::ScanProgressPayload, String> {
    let state = app_handle.state::<MigrationState>();
    if !state.scan_running.load(Ordering::SeqCst) {
        let progress = state
            .scan_progress
            .lock()
            .ok()
            .and_then(|value| value.clone())
            .unwrap_or(crate::migration::models::ScanProgressPayload {
                phase: "stopped".into(),
                pages_scanned: 0,
                discovered_files: 0,
                discovered_folders: 0,
                elapsed_ms: 0,
            });
        return Ok(progress);
    }
    // Signal the scanner immediately. Do not wait for the Microsoft session
    // mutex first: a download in progress may hold it for a long time.
    state.scan_stop_requested.store(true, Ordering::SeqCst);
    let mut progress = state
        .scan_progress
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .unwrap_or(crate::migration::models::ScanProgressPayload {
            phase: "stopping".into(),
            pages_scanned: 0,
            discovered_files: 0,
            discovered_folders: 0,
            elapsed_ms: 0,
        });
    progress.phase = "stopping".into();
    publish_scan_progress(&app_handle, progress.clone());
    let account_email = match tokio::time::timeout(
        std::time::Duration::from_millis(500),
        state.ms_session.lock(),
    )
    .await
    {
        Ok(session_guard) => session_guard
            .as_ref()
            .map(|value| value.account_info.account_email.clone())
            .filter(|value| !value.is_empty()),
        Err(_) => None,
    };
    if let Some(email) = account_email {
        let _ = set_auto_scan_status(&state.db, &email, "stopped");
    }
    Ok(progress)
}

fn sort_snapshot_items(items: &mut [crate::migration::models::OneDriveItem]) {
    items.sort_by(|left, right| {
        left.path
            .as_deref()
            .unwrap_or(&left.name)
            .cmp(right.path.as_deref().unwrap_or(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn spawn_worker(app_handle: &AppHandle, job_id: i64) {
    let app_clone = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state_ref = app_clone.state::<MigrationState>();
        let mig_clone = state_ref.inner().clone_state();
        run_migration_worker(mig_clone, job_id, app_clone).await;
    });
}

async fn account_and_token(app_handle: &AppHandle) -> Result<(String, String), String> {
    let mig_state = app_handle.state::<MigrationState>();
    let mut session_guard = mig_state.ms_session.lock().await;
    let session = session_guard
        .as_mut()
        .ok_or_else(|| "microsoft_not_connected".to_string())?;
    if session.is_expired() {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            refresh_access_token(session),
        )
        .await
        .map_err(|_| "Microsoft token refresh timed out".to_string())??;
        crate::migration::session_store::save(app_handle, session)?;
    }
    Ok((
        session.account_info.account_email.clone(),
        session.access_token.clone(),
    ))
}

async fn run_auto_engine(app_handle: AppHandle, force_rescan: bool) -> Result<Option<i64>, String> {
    let mig_state = app_handle.state::<MigrationState>();
    if mig_state
        .scan_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("scan_already_running".into());
    }
    let _scan_guard = ScanRunningGuard(mig_state.scan_running.clone());
    mig_state.scan_stop_requested.store(false, Ordering::SeqCst);

    let (account_email, access_token) = account_and_token(&app_handle).await?;
    if account_email.is_empty() {
        return Err("microsoft_not_connected".into());
    }

    let profile = match get_auto_profile(&mig_state.db, &account_email)? {
        Some(profile) => profile,
        None if force_rescan => {
            upsert_auto_profile(&mig_state.db, &account_email, true, None, None, None)?
        }
        None => return Ok(None),
    };

    if !force_rescan {
        if let Some(job_id) = profile.active_job_id {
            if let Ok(job) = get_job(&mig_state.db, job_id) {
                match job.state.as_str() {
                    "ready" | "paused" => {
                        spawn_worker(&app_handle, job_id);
                        return Ok(Some(job_id));
                    }
                    "running" => return Ok(Some(job_id)),
                    _ => return Ok(Some(job_id)),
                }
            }
        }
    } else {
        if mig_state.worker_running.load(Ordering::SeqCst) {
            return Err("migration_running".into());
        }
        if let Some(job_id) = profile.active_job_id {
            if get_job(&mig_state.db, job_id)
                .map(|job| job.state == "running")
                .unwrap_or(false)
            {
                return Err("migration_running".into());
            }
        }
    }
    let existing_checkpoint = get_auto_scan_checkpoint(&mig_state.db, &account_email)?;
    if !force_rescan
        && existing_checkpoint
            .as_ref()
            .map(|value| value.status == "stopped")
            .unwrap_or(false)
    {
        return Ok(None);
    }

    let temp_dir = match profile.local_temp_dir {
        Some(dir) if !dir.trim().is_empty() => dir,
        _ => {
            let dir = app_handle
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("temp_migration");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            dir.to_string_lossy().to_string()
        }
    };

    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let dest_name = profile
        .default_telegram_dest_name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Saved Messages".to_string());
    // Create the queue before enumeration so the worker can consume pages as
    // soon as they are persisted instead of waiting for the full snapshot.
    let (job_id, _) = create_migration_job(
        &mig_state.db,
        Some("root"),
        Some("/ (Root)"),
        profile.default_telegram_dest_id,
        Some(&dest_name),
        &temp_dir,
        &[],
    )?;
    // The UI restores the running queue through AutoMigrationStatus. Persist
    // ownership immediately instead of waiting until the full delta scan ends.
    set_auto_active_job(&mig_state.db, &account_email, job_id)?;
    if let Ok(activity) = record_activity(
        &mig_state.db,
        job_id,
        None,
        None,
        "scan",
        "started",
        Some("Đã bắt đầu quét OneDrive và xử lý tuần tự"),
    ) {
        let _ = app_handle.emit("migration:activity", activity);
    }
    spawn_worker(&app_handle, job_id);
    if existing_checkpoint.is_some() {
        set_auto_scan_status(&mig_state.db, &account_email, "enumerating")?;
    } else {
        save_auto_scan_page(
            &mig_state.db,
            &account_email,
            &[],
            Some(crate::migration::microsoft::DRIVE_DELTA_URL),
            &crate::migration::models::ScanProgressPayload {
                phase: "enumerating".into(),
                pages_scanned: 0,
                discovered_files: 0,
                discovered_folders: 0,
                elapsed_ms: 0,
            },
        )?;
    }
    let resume = match existing_checkpoint {
        Some(checkpoint) => crate::migration::microsoft::DeltaScanResume {
            next_url: checkpoint.next_link,
            pages_scanned: checkpoint.pages_scanned,
            elapsed_ms: checkpoint.elapsed_ms,
            entries: load_auto_scan_items(&mig_state.db, &account_email)?,
        },
        None => crate::migration::microsoft::DeltaScanResume {
            next_url: Some(crate::migration::microsoft::DRIVE_DELTA_URL.to_string()),
            pages_scanned: 0,
            elapsed_ms: 0,
            entries: Vec::new(),
        },
    };
    let progress_handle = app_handle.clone();
    let scan_result = scan_drive_delta(
        &http,
        &access_token,
        resume,
        &mig_state.scan_stop_requested,
        |values, next_link, progress| {
            save_auto_scan_page(&mig_state.db, &account_email, values, next_link, progress)?;
            let page_items = build_delta_snapshot(values);
            append_scan_items(&mig_state.db, job_id, &page_items)
        },
        move |progress| {
            publish_scan_progress(&progress_handle, progress);
        },
    )
    .await;
    let mut root_items = match scan_result {
        Ok(crate::migration::microsoft::DeltaScanOutcome::Completed(items)) => items,
        Ok(crate::migration::microsoft::DeltaScanOutcome::Stopped(progress)) => {
            set_auto_scan_status(&mig_state.db, &account_email, "stopped")?;
            publish_scan_progress(&app_handle, progress);
            return Err("scan_stopped".into());
        }
        Err(error) => {
            let _ = set_auto_scan_status(&mig_state.db, &account_email, "failed");
            publish_scan_failure(&app_handle);
            return Err(error);
        }
    };
    sort_snapshot_items(&mut root_items);

    append_scan_items(&mig_state.db, job_id, &root_items)?;
    let stats = get_job_stats(&mig_state.db, job_id)?;
    if let Err(error) = finalize_auto_scan(&mig_state.db, &account_email, job_id) {
        publish_scan_failure(&app_handle);
        return Err(error);
    }

    let current_progress = mig_state
        .scan_progress
        .lock()
        .ok()
        .and_then(|value| value.clone());
    publish_scan_progress(
        &app_handle,
        crate::migration::models::ScanProgressPayload {
            phase: "completed".into(),
            pages_scanned: current_progress
                .as_ref()
                .map(|value| value.pages_scanned)
                .unwrap_or(0),
            discovered_files: stats.total_files as usize,
            discovered_folders: stats.total_folders as usize,
            elapsed_ms: current_progress
                .as_ref()
                .map(|value| value.elapsed_ms)
                .unwrap_or(0),
        },
    );
    let _ = app_handle.emit(
        "migration:snapshot-ready",
        serde_json::json!({ "job_id": job_id }),
    );

    if let Ok(activity) = record_activity(
        &mig_state.db,
        job_id,
        None,
        None,
        "scan",
        "completed",
        Some(&format!("Đã tạo snapshot gồm {} file", stats.total_files)),
    ) {
        let _ = app_handle.emit("migration:activity", activity);
    }

    if stats.total_files == 0 {
        let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(format!(
            "UPDATE migration_jobs SET state = 'completed', completed_at = strftime('%s','now'),
             updated_at = strftime('%s','now') WHERE id = {job_id};"
        ))
        .map_err(|e| e.to_string())?;
        return Ok(Some(job_id));
    }

    // Auto migration is now explicitly started by the user from the pipeline
    // button; the legacy profile toggle must not prevent that run.
    spawn_worker(&app_handle, job_id);
    Ok(Some(job_id))
}

pub async fn start_auto_engine(app_handle: AppHandle) -> Result<(), String> {
    match run_auto_engine(app_handle, false).await {
        Ok(_) => Ok(()),
        Err(error) if error == "microsoft_not_connected" => Ok(()),
        Err(error) => Err(error),
    }
}

pub async fn rescan_auto_engine(app_handle: AppHandle) -> Result<MigrationJobDetail, String> {
    let job_id = run_auto_engine(app_handle.clone(), true)
        .await?
        .ok_or_else(|| "source_unavailable".to_string())?;
    let state = app_handle.state::<MigrationState>();
    let job = get_job(&state.db, job_id)?;
    let stats = get_job_stats(&state.db, job_id)?;
    let files = get_items_by_job(&state.db, job_id)?;
    Ok(MigrationJobDetail {
        job,
        stats,
        folders: Vec::new(),
        files,
    })
}

pub fn start_rescan_auto_engine(app_handle: AppHandle) -> Result<(), String> {
    let state = app_handle.state::<MigrationState>();
    if state.scan_running.load(Ordering::SeqCst) {
        return Err("scan_already_running".into());
    }
    if state.worker_running.load(Ordering::SeqCst) {
        return Err("migration_running".into());
    }
    if AUTO_PIPELINE_STARTING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("scan_already_running".into());
    }

    let current = state
        .scan_progress
        .lock()
        .ok()
        .and_then(|value| value.clone());
    publish_scan_progress(
        &app_handle,
        crate::migration::models::ScanProgressPayload {
            phase: "starting".into(),
            pages_scanned: current
                .as_ref()
                .map(|value| value.pages_scanned)
                .unwrap_or(0),
            discovered_files: current
                .as_ref()
                .map(|value| value.discovered_files)
                .unwrap_or(0),
            discovered_folders: current
                .as_ref()
                .map(|value| value.discovered_folders)
                .unwrap_or(0),
            elapsed_ms: current.as_ref().map(|value| value.elapsed_ms).unwrap_or(0),
        },
    );

    tauri::async_runtime::spawn(async move {
        let _starting_guard = AutoPipelineStartingGuard;
        if let Err(error) = rescan_auto_engine(app_handle.clone()).await {
            if error != "scan_stopped" {
                log::error!("Auto migration pipeline failed to start or run: {error}");
                publish_scan_failure(&app_handle);
                let _ = app_handle.emit(
                    "migration:pipeline-error",
                    serde_json::json!({ "error": error }),
                );
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::models::OneDriveItem;

    fn item(id: &str, path: &str) -> OneDriveItem {
        OneDriveItem {
            id: id.into(),
            name: path.rsplit('/').next().unwrap_or(path).into(),
            item_type: "file".into(),
            size: 1,
            path: Some(path.into()),
            child_count: None,
            etag: None,
            quickxor_hash: None,
            sha1_hash: None,
            last_modified: None,
        }
    }

    #[test]
    fn snapshot_order_is_stable_by_path_then_source_id() {
        let mut items = vec![
            item("b", "folder/z.txt"),
            item("z", "folder/a.txt"),
            item("a", "folder/a.txt"),
        ];
        sort_snapshot_items(&mut items);
        assert_eq!(
            items
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "z", "b"]
        );
    }
}
