use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::bandwidth::BandwidthManager;
use crate::commands::create_folder_inner;
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

fn emit_activity(
    db: &MigrationDb,
    app_handle: &AppHandle,
    job_id: i64,
    item_id: Option<i64>,
    item_name: Option<&str>,
    phase: &str,
    status: &str,
    message: Option<&str>,
) {
    if let Ok(activity) = record_activity(db, job_id, item_id, item_name, phase, status, message) {
        let _ = app_handle.emit("migration:activity", activity);
    }
}

fn auto_folder_name(file_name: &str) -> &'static str {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg" | "heic" => "Auto Images",
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "3gp" => "Auto Videos",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "csv" | "md" => {
            "Auto Documents"
        }
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "iso" => "Auto Archives",
        _ => "Auto Other",
    }
}

async fn ensure_auto_type_destination(
    app_handle: &AppHandle,
    folder_name: &str,
) -> Result<i64, String> {
    let db = app_handle
        .state::<crate::db::DbConnection>()
        .inner()
        .clone();
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT channel_id FROM folder_metadata WHERE name = ? LIMIT 1;")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, folder_name)).map_err(|e| e.to_string())?;
        if let Ok(sqlite::State::Row) = stmt.next() {
            return stmt.read(0).map_err(|e| e.to_string());
        }
    }

    let tg_state = app_handle.state::<TelegramState>();
    let client = tg_state
        .client
        .lock()
        .await
        .clone()
        .ok_or_else(|| "Telegram client not connected".to_string())?;
    let folder = create_folder_inner(folder_name, &client, &tg_state.peer_cache).await?;
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("INSERT OR IGNORE INTO folder_metadata (channel_id, name, username, is_public, display_order, group_id) VALUES (?, ?, ?, ?, 0, NULL);")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, folder.id)).map_err(|e| e.to_string())?;
    stmt.bind((2, folder.name.as_str()))
        .map_err(|e| e.to_string())?;
    stmt.bind((3, folder.username.as_deref()))
        .map_err(|e| e.to_string())?;
    stmt.bind((4, if folder.is_public { 1 } else { 0 }))
        .map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(folder.id)
}

async fn worker_loop_inner(
    mig_state: Arc<MigrationState>,
    job_id: i64,
    app_handle: AppHandle,
) -> Result<(), String> {
    let auto_job = is_auto_job(&mig_state.db, job_id)?;
    let bandwidth = app_handle.state::<Arc<BandwidthManager>>().inner().clone();
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut auto_destinations: HashMap<String, i64> = HashMap::new();

    // Mark job running
    {
        let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(format!(
            "UPDATE migration_jobs SET state = 'running', pause_reason = NULL, started_at = COALESCE(started_at, {}), updated_at = {} WHERE id = {};",
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
        let local_dir_str = job
            .local_dir
            .ok_or_else(|| "Local working directory not set".to_string())?;
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

        // Fetch next pending item (LIMIT 1 for maximum speed and zero memory overhead)
        let next_item = get_next_pending_item(&mig_state.db, job_id)?;

        let item = match next_item {
            Some(i) => i,
            None => {
                // Streaming auto scans append pages into this same queue. Keep
                // the worker alive while enumeration is still in progress.
                if auto_job && mig_state.scan_running.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
                if auto_job && mig_state.scan_stop_requested.load(Ordering::Relaxed) {
                    let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                    conn.execute(format!(
                        "UPDATE migration_jobs SET state = 'ready', updated_at = strftime('%s','now') WHERE id = {};",
                        job_id
                    ))
                    .map_err(|e| e.to_string())?;
                    break;
                }
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

        if auto_job {
            let quota = get_daily_quota(&mig_state.db)?;
            if would_exceed_daily_quota(quota.uploaded_bytes, item.size_bytes) {
                let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                let now = chrono::Utc::now().timestamp();
                conn.execute(format!(
                    "UPDATE migration_jobs SET state = 'paused', pause_reason = 'daily_quota',
                     updated_at = {now} WHERE id = {job_id};"
                ))
                .map_err(|e| e.to_string())?;
                drop(conn);
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "quota",
                    "paused",
                    Some("File tiếp theo vượt quota Auto Migration hôm nay"),
                );
                let _ = app_handle.emit(
                    "migration:job-state",
                    serde_json::json!({
                        "job_id": job_id,
                        "state": "paused",
                        "reason": "daily_quota"
                    }),
                );
                let resume_handle = app_handle.clone();
                let wait_seconds = quota
                    .resets_at
                    .saturating_sub(chrono::Utc::now().timestamp())
                    .max(1) as u64;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(wait_seconds)).await;
                    let _ = crate::migration::auto_engine::start_auto_engine(resume_handle).await;
                });
                break;
            }
        }

        // Pre-download duplicate check
        if let (Some(fp_type), Some(fp_val)) = (
            &item.source_fingerprint_type,
            &item.source_fingerprint_value,
        ) {
            if check_fingerprint(&mig_state.db, fp_type, fp_val, item.size_bytes)? {
                log::info!("Pre-download duplicate skip for item: {}", item.name);
                record_item_skipped_duplicate(&mig_state.db, job_id, item.id, None)?;
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "completed",
                    "skipped_duplicate",
                    None,
                );
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
                    refresh_access_token(session).await?;
                    crate::migration::session_store::save(&app_handle, session)?;
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
        emit_activity(
            &mig_state.db,
            &app_handle,
            job_id,
            Some(item.id),
            Some(&item.name),
            "downloading",
            "started",
            None,
        );
        let _ = app_handle.emit(
            "migration:item-progress",
            serde_json::json!({
                "job_id": job_id,
                "item_id": item.id,
                "item_name": item.name.clone(),
                "phase": "downloading",
                "event_id": format!("{}:{}:{}:downloading:0", job_id, item.id, item.attempt_count),
                "attempt": item.attempt_count,
                "revision": 0,
                "percent": 0,
                "bytes_done": 0,
                "bytes_total": item.size_bytes.max(0),
                "speed_bytes_per_sec": 0,
                "timestamp": chrono::Utc::now().timestamp_millis()
            }),
        );

        let app_handle_dl = app_handle.clone();
        let item_name_dl = item.name.clone();
        let dl_revision = Arc::new(AtomicU64::new(0));
        let dl_revision_cb = dl_revision.clone();
        let attempt = item.attempt_count;
        let dl_res = download_item(
            &http,
            &access_token,
            source_item_id,
            &part_path_str,
            Some(move |bytes_done, bytes_total| {
                let revision = dl_revision_cb.fetch_add(1, Ordering::Relaxed) + 1;
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
                        "event_id": format!("{}:{}:{}:downloading:{}", job_id, item.id, attempt, revision),
                        "attempt": attempt,
                        "revision": revision,
                        "percent": percent,
                        "bytes_done": bytes_done,
                        "bytes_total": bytes_total,
                        "speed_bytes_per_sec": 0
                        ,"timestamp": chrono::Utc::now().timestamp_millis()
                    }),
                );
            }),
        )
        .await;

        let sha256_hash = match dl_res {
            Ok(hash) => hash,
            Err(e) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                let is_not_found = e.contains("itemNotFound")
                    || e.contains("could not be found")
                    || e.contains("404");
                let err_code = "download_failed";
                record_item_failed(&mig_state.db, job_id, item.id, err_code, &e, !is_not_found)?;
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "downloading",
                    "failed",
                    Some(&e),
                );
                let _ = app_handle.emit(
                    "migration:item-complete",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item.name,
                        "status": "failed",
                        "error_type": err_code,
                        "error_message": if is_not_found { "File no longer exists on OneDrive (404 itemNotFound)".into() } else { e }
                    }),
                );
                continue;
            }
        };
        let downloaded_bytes = tokio::fs::metadata(&part_path)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(item.size_bytes.max(0) as u64);
        bandwidth.add_down(downloaded_bytes);

        // Post-download SHA-256 duplicate check
        if check_fingerprint(&mig_state.db, "sha256", &sha256_hash, item.size_bytes)? {
            log::info!("Post-download duplicate skip for item: {}", item.name);
            let _ = tokio::fs::remove_file(&part_path).await;
            record_item_skipped_duplicate(&mig_state.db, job_id, item.id, Some(&sha256_hash))?;
            emit_activity(
                &mig_state.db,
                &app_handle,
                job_id,
                Some(item.id),
                Some(&item.name),
                "completed",
                "skipped_duplicate",
                None,
            );
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

        // Auto jobs are routed into one Telegram Drive channel per file type.
        let destination_id = if auto_job {
            let folder_name = auto_folder_name(&item.name).to_string();
            if let Some(id) = auto_destinations.get(&folder_name) {
                Some(*id)
            } else {
                match ensure_auto_type_destination(&app_handle, &folder_name).await {
                    Ok(id) => {
                        auto_destinations.insert(folder_name, id);
                        Some(id)
                    }
                    Err(error) => {
                        log::error!(
                            "Could not prepare auto destination '{}': {}",
                            folder_name,
                            error
                        );
                        job.telegram_destination_id
                    }
                }
            }
        } else {
            job.telegram_destination_id
        };

        // Mark uploading
        {
            let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
            conn.execute(format!(
                "UPDATE migration_items SET state = 'uploading' WHERE id = {};",
                item.id
            ))
            .map_err(|e| e.to_string())?;
        }
        emit_activity(
            &mig_state.db,
            &app_handle,
            job_id,
            Some(item.id),
            Some(&item.name),
            "uploading",
            "started",
            None,
        );
        let _ = app_handle.emit(
            "migration:item-progress",
            serde_json::json!({
                "job_id": job_id,
                "item_id": item.id,
                "item_name": item.name.clone(),
                "phase": "uploading",
                "event_id": format!("{}:{}:{}:uploading:0", job_id, item.id, item.attempt_count),
                "attempt": item.attempt_count,
                "revision": 0,
                "percent": 0,
                "bytes_done": 0,
                "bytes_total": item.size_bytes.max(0),
                "speed_bytes_per_sec": 0,
                "timestamp": chrono::Utc::now().timestamp_millis()
            }),
        );

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
        let ul_revision = Arc::new(AtomicU64::new(0));
        let ul_revision_cb = ul_revision.clone();
        let attempt = item.attempt_count;
        let ul_res = upload_core(
            &client,
            &tg_state.peer_cache,
            &part_path_str,
            &item.name,
            destination_id,
            &mig_state.cancel_token,
            Some(move |bytes_done, bytes_total| {
                let revision = ul_revision_cb.fetch_add(1, Ordering::Relaxed) + 1;
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
                        "event_id": format!("{}:{}:{}:uploading:{}", job_id, item.id, attempt, revision),
                        "attempt": attempt,
                        "revision": revision,
                        "percent": percent,
                        "bytes_done": bytes_done,
                        "bytes_total": bytes_total,
                        "speed_bytes_per_sec": 0
                        ,"timestamp": chrono::Utc::now().timestamp_millis()
                    }),
                );
            }),
        )
        .await;

        match ul_res {
            Ok(res) => {
                bandwidth.add_up(res.file_size.max(0) as u64);
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
                    destination_id,
                    res.message_id,
                    auto_job,
                )?;
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "completed",
                    "completed",
                    None,
                );

                // Remove the OneDrive source only after Telegram confirms the upload.
                if auto_job {
                    if let Some(source_id) = item.source_item_id.as_deref() {
                        if let Err(error) =
                            delete_onedrive_item(&http, &access_token, source_id).await
                        {
                            log::warn!(
                                "Uploaded '{}' but failed to delete OneDrive source: {}",
                                item.name,
                                error
                            );
                            emit_activity(
                                &mig_state.db,
                                &app_handle,
                                job_id,
                                Some(item.id),
                                Some(&item.name),
                                "cleanup",
                                "failed",
                                Some(&error),
                            );
                        } else {
                            log::info!(
                                "Deleted OneDrive source after successful upload: {}",
                                item.name
                            );
                        }
                    }
                }

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
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "uploading",
                    "failed",
                    Some(&msg),
                );
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
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "uploading",
                    "failed",
                    Some(&err_str),
                );

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
