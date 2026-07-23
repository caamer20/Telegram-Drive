use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::TelegramState;
use crate::migration::db::*;
use crate::migration::microsoft::*;
use crate::migration::upload_adapter::*;
use crate::migration::MigrationState;

pub async fn run_migration_worker(
    mig_state: Arc<MigrationState>,
    job_id: i64,
    app_handle: AppHandle,
) {
    log::info!("Starting migration worker for job {}", job_id);

    // Ensure worker_running guard
    if mig_state.worker_running.swap(true, Ordering::SeqCst) {
        log::warn!("Worker already running for another job. Aborting.");
        return;
    }

    mig_state.cancel_token.store(false, Ordering::Relaxed);
    mig_state.pause_token.store(false, Ordering::Relaxed);

    let res = worker_loop_inner(mig_state.clone(), job_id, app_handle.clone()).await;

    mig_state.worker_running.store(false, Ordering::SeqCst);

    if let Err(e) = res {
        log::error!("Worker loop error for job {}: {}", job_id, e);
        let _ = record_job_failed(&mig_state.db, job_id, &e);
        let _ = app_handle.emit(
            "migration:job-state",
            serde_json::json!({
                "job_id": job_id,
                "state": "failed",
                "previous_state": "running"
            }),
        );
    }
}

fn record_job_failed(db: &MigrationDb, job_id: i64, _err_msg: &str) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn
        .prepare("UPDATE migration_jobs SET state = 'failed', completed_at = ?, updated_at = ? WHERE id = ?;")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, now)).map_err(|e| e.to_string())?;
    stmt.bind((2, now)).map_err(|e| e.to_string())?;
    stmt.bind((3, job_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

async fn worker_loop_inner(
    mig_state: Arc<MigrationState>,
    job_id: i64,
    app_handle: AppHandle,
) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    // Mark job running
    {
        let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(format!(
            "UPDATE migration_jobs SET state = 'running', started_at = COALESCE(started_at, {}), updated_at = {} WHERE id = {};",
            now, now, job_id
        ))
        .map_err(|e| e.to_string())?;
    }

    let _ = app_handle.emit(
        "migration:job-state",
        serde_json::json!({
            "job_id": job_id,
            "state": "running",
            "previous_state": "ready"
        }),
    );

    loop {
        // Check cancel / pause tokens
        if mig_state.cancel_token.load(Ordering::Relaxed) {
            let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
            let now = chrono::Utc::now().timestamp();
            conn.execute(format!(
                "UPDATE migration_jobs SET state = 'cancelled', completed_at = {}, updated_at = {} WHERE id = {};",
                now, now, job_id
            ))
            .map_err(|e| e.to_string())?;

            let _ = app_handle.emit(
                "migration:job-state",
                serde_json::json!({
                    "job_id": job_id,
                    "state": "cancelled",
                    "previous_state": "running"
                }),
            );
            break;
        }

        if mig_state.pause_token.load(Ordering::Relaxed) {
            let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
            let now = chrono::Utc::now().timestamp();
            conn.execute(format!(
                "UPDATE migration_jobs SET state = 'paused', updated_at = {} WHERE id = {};",
                now, job_id
            ))
            .map_err(|e| e.to_string())?;

            let _ = app_handle.emit(
                "migration:job-state",
                serde_json::json!({
                    "job_id": job_id,
                    "state": "paused",
                    "previous_state": "running"
                }),
            );
            break;
        }

        let job = get_job(&mig_state.db, job_id)?;
        let local_dir_str = job.local_dir.ok_or_else(|| "Local working directory not set".to_string())?;
        let local_dir = std::path::Path::new(&local_dir_str);

        if !local_dir.exists() {
            return Err(format!("Local directory unavailable: {}", local_dir_str));
        }

        // Check cooldown
        if let Some(cooldown_until) = job.cooldown_until {
            let now = chrono::Utc::now().timestamp();
            if now < cooldown_until {
                let remaining = cooldown_until - now;
                let _ = app_handle.emit(
                    "migration:cooldown",
                    serde_json::json!({
                        "job_id": job_id,
                        "cooldown_until": cooldown_until,
                        "seconds_remaining": remaining
                    }),
                );
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            } else {
                // Clear cooldown
                let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                conn.execute(format!(
                    "UPDATE migration_jobs SET cooldown_until = NULL WHERE id = {};",
                    job_id
                ))
                .map_err(|e| e.to_string())?;

                let _ = app_handle.emit(
                    "migration:cooldown",
                    serde_json::json!({
                        "job_id": job_id,
                        "cooldown_until": null,
                        "seconds_remaining": 0
                    }),
                );
            }
        }

        // Fetch next pending item
        let items = get_items_by_job(&mig_state.db, job_id)?;
        let next_item = items.into_iter().find(|i| i.state == "pending" && i.item_type == "file");

        // Check daily upload quota limit (250GB)
        if let Ok(quota) = get_daily_quota(&mig_state.db) {
            if quota.uploaded_bytes >= quota.limit_bytes {
                log::warn!("Daily upload quota limit of 250GB reached. Pausing migration job.");
                let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                let now = chrono::Utc::now().timestamp();
                let _ = conn.execute(format!(
                    "UPDATE migration_jobs SET state = 'paused', updated_at = {} WHERE id = {};",
                    now, job_id
                ));
                let _ = app_handle.emit(
                    "migration:job-state",
                    serde_json::json!({
                        "job_id": job_id,
                        "state": "paused",
                        "reason": "daily_quota_reached"
                    }),
                );
                break;
            }
        }

        let item = match next_item {
            Some(i) => i,
            None => {

                // All items completed or skipped or failed
                let stats = get_job_stats(&mig_state.db, job_id)?;
                let final_state = if stats.pending_files == 0 {
                    if stats.failed_files > 0 {
                        "failed"
                    } else {
                        "completed"
                    }
                } else {
                    "completed"
                };

                let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                let now = chrono::Utc::now().timestamp();
                conn.execute(format!(
                    "UPDATE migration_jobs SET state = '{}', completed_at = {}, updated_at = {} WHERE id = {};",
                    final_state, now, now, job_id
                ))
                .map_err(|e| e.to_string())?;

                let _ = app_handle.emit(
                    "migration:job-state",
                    serde_json::json!({
                        "job_id": job_id,
                        "state": final_state,
                        "previous_state": "running"
                    }),
                );
                break;
            }
        };

        // Pre-download duplicate check
        if let (Some(fp_type), Some(fp_val)) = (
            &item.source_fingerprint_type,
            &item.source_fingerprint_value,
        ) {
            if check_fingerprint(&mig_state.db, fp_type, fp_val, item.size_bytes)? {
                log::info!("Pre-download duplicate skip for item: {}", item.name);
                record_item_skipped_duplicate(&mig_state.db, job_id, item.id, None)?;
                let _ = app_handle.emit(
                    "migration:item-complete",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item.name,
                        "status": "skipped_duplicate"
                    }),
                );
                continue;
            }
        }

        // Check MS token validity
        let access_token = {
            let mut session_guard = mig_state.ms_session.lock().await;
            if let Some(ref mut session) = *session_guard {
                if session.is_expired() {
                    refresh_access_token(DEFAULT_MS_CLIENT_ID, session).await?;
                }
                session.access_token.clone()
            } else {
                return Err("Microsoft authentication session expired. Please reconnect.".into());
            }
        };

        let source_item_id = item
            .source_item_id
            .as_ref()
            .ok_or_else(|| format!("Missing source_item_id for item {}", item.name))?;

        let part_filename = format!("mig_{}_{}.part", job_id, item.id);
        let part_path = local_dir.join(&part_filename);
        let part_path_str = part_path.to_string_lossy().to_string();

        // Mark downloading
        {
            let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
            conn.execute(format!(
                "UPDATE migration_items SET state = 'downloading' WHERE id = {};",
                item.id
            ))
            .map_err(|e| e.to_string())?;
        }

        let app_handle_dl = app_handle.clone();
        let item_name_dl = item.name.clone();
        let dl_res = download_item(
            &http,
            &access_token,
            source_item_id,
            &part_path_str,
            Some(move |bytes_done, bytes_total| {
                let percent = if bytes_total > 0 {
                    ((bytes_done as f64 / bytes_total as f64) * 100.0) as u8
                } else {
                    0
                };
                let _ = app_handle_dl.emit(
                    "migration:item-progress",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item_name_dl,
                        "phase": "downloading",
                        "percent": percent,
                        "bytes_done": bytes_done,
                        "bytes_total": bytes_total,
                        "speed_bytes_per_sec": 0
                    }),
                );
            }),
        )
        .await;

        let sha256_hash = match dl_res {
            Ok(hash) => hash,
            Err(e) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                record_item_failed(&mig_state.db, job_id, item.id, "download_failed", &e, true)?;
                let _ = app_handle.emit(
                    "migration:item-complete",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item.name,
                        "status": "failed",
                        "error_type": "download_failed",
                        "error_message": e
                    }),
                );
                continue;
            }
        };

        // Post-download SHA-256 duplicate check
        if check_fingerprint(&mig_state.db, "sha256", &sha256_hash, item.size_bytes)? {
            log::info!("Post-download duplicate skip for item: {}", item.name);
            let _ = tokio::fs::remove_file(&part_path).await;
            record_item_skipped_duplicate(&mig_state.db, job_id, item.id, Some(&sha256_hash))?;
            let _ = app_handle.emit(
                "migration:item-complete",
                serde_json::json!({
                    "job_id": job_id,
                    "item_id": item.id,
                    "item_name": item.name,
                    "status": "skipped_duplicate"
                }),
            );
            continue;
        }

        // Mark uploading
        {
            let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
            conn.execute(format!(
                "UPDATE migration_items SET state = 'uploading' WHERE id = {};",
                item.id
            ))
            .map_err(|e| e.to_string())?;
        }

        // Upload through Telegram
        let tg_state = app_handle.state::<TelegramState>();
        let client_opt = { tg_state.client.lock().await.clone() };
        let client = match client_opt {
            Some(c) => c,
            None => {
                let _ = tokio::fs::remove_file(&part_path).await;
                record_item_failed(
                    &mig_state.db,
                    job_id,
                    item.id,
                    "telegram_not_connected",
                    "Telegram client not connected",
                    false,
                )?;
                return Err("Telegram client not connected".into());
            }
        };

        let app_handle_ul = app_handle.clone();
        let item_name_ul = item.name.clone();
        let ul_res = upload_core(
            &client,
            &tg_state.peer_cache,
            &part_path_str,
            job.telegram_destination_id,
            &mig_state.cancel_token,
            Some(move |bytes_done, bytes_total| {
                let percent = if bytes_total > 0 {
                    ((bytes_done as f64 / bytes_total as f64) * 100.0) as u8
                } else {
                    0
                };
                let _ = app_handle_ul.emit(
                    "migration:item-progress",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item_name_ul,
                        "phase": "uploading",
                        "percent": percent,
                        "bytes_done": bytes_done,
                        "bytes_total": bytes_total,
                        "speed_bytes_per_sec": 0
                    }),
                );
            }),
        )
        .await;

        match ul_res {
            Ok(res) => {
                // Success transaction
                let provider_fp = match (
                    item.source_fingerprint_type.as_deref(),
                    item.source_fingerprint_value.as_deref(),
                ) {
                    (Some(t), Some(v)) => Some((t, v)),
                    _ => None,
                };

                record_item_success(
                    &mig_state.db,
                    job_id,
                    item.id,
                    &sha256_hash,
                    provider_fp,
                    item.size_bytes,
                    job.telegram_destination_id,
                    res.message_id,
                )?;

                let _ = add_daily_uploaded_bytes(&mig_state.db, item.size_bytes as u64);

                let _ = tokio::fs::remove_file(&part_path).await;

                let _ = app_handle.emit(
                    "migration:item-complete",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item.name,
                        "status": "completed"
                    }),
                );
            }
            Err(UploadError::FloodWait { seconds }) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                let cooldown_until = chrono::Utc::now().timestamp() + seconds + 60; // 60s safety buffer
                let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                conn.execute(format!(
                    "UPDATE migration_jobs SET cooldown_until = {} WHERE id = {};",
                    cooldown_until, job_id
                ))
                .map_err(|e| e.to_string())?;

                // Reset item state to pending for retry after cooldown
                conn.execute(format!(
                    "UPDATE migration_items SET state = 'pending' WHERE id = {};",
                    item.id
                ))
                .map_err(|e| e.to_string())?;

                let _ = app_handle.emit(
                    "migration:cooldown",
                    serde_json::json!({
                        "job_id": job_id,
                        "cooldown_until": cooldown_until,
                        "seconds_remaining": seconds + 60
                    }),
                );
            }
            Err(UploadError::TelegramFileTooLarge(msg)) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                record_item_failed(
                    &mig_state.db,
                    job_id,
                    item.id,
                    "telegram_file_too_large",
                    &msg,
                    false, // Do not auto-retry file too large
                )?;
                let _ = app_handle.emit(
                    "migration:item-complete",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item.name,
                        "status": "failed",
                        "error_type": "telegram_file_too_large",
                        "error_message": msg
                    }),
                );
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                let err_str = e.to_string();
                let err_code = match e {
                    UploadError::Auth(_) => "auth",
                    UploadError::Network(_) => "network",
                    _ => "upload_failed",
                };

                let should_increment = item.attempt_count < 3;
                record_item_failed(
                    &mig_state.db,
                    job_id,
                    item.id,
                    err_code,
                    &err_str,
                    should_increment,
                )?;

                let _ = app_handle.emit(
                    "migration:item-complete",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item.name,
                        "status": "failed",
                        "error_type": err_code,
                        "error_message": err_str
                    }),
                );
            }
        }
    }

    Ok(())
}
