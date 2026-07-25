use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::bandwidth::BandwidthManager;
use crate::commands::create_folder_inner;
use crate::commands::TelegramState;
use crate::migration::db::*;
use crate::migration::media_processor::{prepare_upload, MediaProcessError};
use crate::migration::microsoft::*;
use crate::migration::models::MigrationItem;
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

pub fn is_video_file(file_name: &str) -> bool {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "3gp"
    )
}

pub fn is_media_file(file_name: &str) -> bool {
    is_video_file(file_name)
}

pub fn is_immediate_stream_file(file_name: &str) -> bool {
    is_video_file(file_name)
}

pub fn should_batch_zip_file(_file_name: &str) -> bool {
    false
}

pub fn is_code_or_text_file(file_name: &str) -> bool {
    should_batch_zip_file(file_name)
}

pub fn get_parent_folder_info(source_path: &str, _file_name: &str) -> (String, String) {
    let path = std::path::Path::new(source_path);
    if let Some(parent) = path.parent() {
        let parent_str = parent.to_string_lossy().to_string();
        if !parent_str.is_empty() && parent_str != "." {
            let parent_name = parent
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Folder".to_string());
            return (parent_str, parent_name);
        }
    }
    ("".to_string(), "Root".to_string())
}

pub fn get_top_level_folder_info(source_path: &str, _file_name: &str) -> (String, String) {
    let path = std::path::Path::new(source_path);
    let mut components = path.components();
    if let Some(std::path::Component::Normal(first)) = components.next() {
        let first_str = first.to_string_lossy().to_string();
        if components.next().is_some() {
            return (first_str.clone(), first_str);
        }
    }
    ("".to_string(), "Root_Files".to_string())
}

pub fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if sanitized.trim_matches('_').is_empty() {
        "Folder".to_string()
    } else {
        sanitized
    }
}

pub fn create_zip_from_directory(src_dir: &std::path::Path, zip_path: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::create(zip_path).map_err(|e| format!("Failed to create zip file: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in std::fs::read_dir(src_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            zip.start_file(&filename, options)
                .map_err(|e| format!("Failed to add '{}' to zip: {}", filename, e))?;
            let mut f = std::fs::File::open(&path).map_err(|e| format!("Failed to open '{}': {}", filename, e))?;
            std::io::copy(&mut f, &mut zip).map_err(|e| format!("Failed to compress '{}': {}", filename, e))?;
        }
    }
    zip.finish().map_err(|e| format!("Failed to finalize zip file: {}", e))?;
    Ok(())
}

pub fn create_zip_from_directory_recursive(src_dir: &std::path::Path, zip_path: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::create(zip_path).map_err(|e| format!("Failed to create zip file: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in walkdir::WalkDir::new(src_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            let relative = path.strip_prefix(src_dir).unwrap_or(path);
            let name = relative.to_string_lossy().to_string();
            zip.start_file(&name, options)
                .map_err(|e| format!("Failed to add '{}' to zip: {}", name, e))?;
            let mut f = std::fs::File::open(path).map_err(|e| format!("Failed to open '{}': {}", name, e))?;
            std::io::copy(&mut f, &mut zip).map_err(|e| format!("Failed to compress '{}': {}", name, e))?;
        }
    }
    zip.finish().map_err(|e| format!("Failed to finalize zip file: {}", e))?;
    Ok(())
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

        // Fetch next pending media item for priority streaming
        let next_item = get_next_pending_media_item(&mig_state.db, job_id)?;

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

                // Scanning finished and all priority media files processed.
                // Now download remaining non-media files to local working directory (no Telegram upload, no OneDrive deletion)!
                let non_media_items = get_pending_items_by_job(&mig_state.db, job_id)?;
                if !non_media_items.is_empty() {
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

                    let base_local_non_media_dir = local_dir.join("NonVideo_Files");
                    let _ = std::fs::create_dir_all(&base_local_non_media_dir);

                    for nm_item in non_media_items {
                        let source_item_id = match nm_item.source_item_id.as_ref() {
                            Some(id) => id,
                            None => continue,
                        };

                        let file_dest_path = base_local_non_media_dir.join(&nm_item.source_path);
                        if let Some(parent) = file_dest_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let part_dest_path = base_local_non_media_dir.join(format!("{}.part", nm_item.id));
                        let part_dest_str = part_dest_path.to_string_lossy().to_string();

                        {
                            let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                            conn.execute(format!(
                                "UPDATE migration_items SET state = 'downloading' WHERE id = {};",
                                nm_item.id
                            ))
                            .map_err(|e| e.to_string())?;
                        }
                        emit_activity(
                            &mig_state.db,
                            &app_handle,
                            job_id,
                            Some(nm_item.id),
                            Some(&nm_item.name),
                            "downloading",
                            "started",
                            Some("Tải bản sao lưu local"),
                        );

                        let dl_res = download_item(
                            &http,
                            &access_token,
                            source_item_id,
                            &part_dest_str,
                            None::<fn(u64, u64)>,
                        )
                        .await;

                        match dl_res {
                            Ok(_) => {
                                let _ = std::fs::rename(&part_dest_path, &file_dest_path);
                                {
                                    let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                                    let now = chrono::Utc::now().timestamp();
                                    conn.execute(format!(
                                        "UPDATE migration_items SET state = 'completed', completed_at = {} WHERE id = {};",
                                        now, nm_item.id
                                    ))
                                    .map_err(|e| e.to_string())?;
                                }

                                emit_activity(
                                    &mig_state.db,
                                    &app_handle,
                                    job_id,
                                    Some(nm_item.id),
                                    Some(&nm_item.name),
                                    "completed",
                                    "success",
                                    Some("Đã lưu bản local (không upload Telegram, giữ nguyên OneDrive)"),
                                );
                                let _ = app_handle.emit(
                                    "migration:item-complete",
                                    serde_json::json!({
                                        "job_id": job_id,
                                        "item_id": nm_item.id,
                                        "item_name": nm_item.name,
                                        "status": "completed"
                                    }),
                                );
                            }
                            Err(error) => {
                                log::error!("Failed to download local non-media file {}: {}", nm_item.name, error);
                                record_item_failed(
                                    &mig_state.db,
                                    job_id,
                                    nm_item.id,
                                    "download_failed",
                                    &error,
                                    true,
                                )?;
                            }
                        }
                    }
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

        // Code and text files folder compression handler
        if is_code_or_text_file(&item.name) {
            let (parent_path, folder_name) = get_parent_folder_info(&item.source_path, &item.name);
            let pending_items = get_pending_items_by_job(&mig_state.db, job_id)?;
            let folder_code_items: Vec<MigrationItem> = pending_items
                .into_iter()
                .filter(|pi| {
                    is_code_or_text_file(&pi.name)
                        && get_parent_folder_info(&pi.source_path, &pi.name).0 == parent_path
                })
                .collect();

            if !folder_code_items.is_empty() {
                let folder_sanitized = sanitize_filename(&folder_name);
                let temp_folder_dir = local_dir.join(format!(
                    "zip_temp_{}_{}_{}",
                    job_id, item.id, folder_sanitized
                ));
                let _ = std::fs::create_dir_all(&temp_folder_dir);

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

                let mut downloaded_items = Vec::new();
                for code_item in &folder_code_items {
                    if let (Some(fp_type), Some(fp_val)) = (
                        &code_item.source_fingerprint_type,
                        &code_item.source_fingerprint_value,
                    ) {
                        if check_fingerprint(&mig_state.db, fp_type, fp_val, code_item.size_bytes)? {
                            log::info!("Pre-download duplicate skip for item: {}", code_item.name);
                            record_item_skipped_duplicate(&mig_state.db, job_id, code_item.id, None)?;
                            emit_activity(
                                &mig_state.db,
                                &app_handle,
                                job_id,
                                Some(code_item.id),
                                Some(&code_item.name),
                                "completed",
                                "skipped_duplicate",
                                None,
                            );
                            let _ = app_handle.emit(
                                "migration:item-complete",
                                serde_json::json!({
                                    "job_id": job_id,
                                    "item_id": code_item.id,
                                    "item_name": code_item.name,
                                    "status": "skipped_duplicate"
                                }),
                            );
                            continue;
                        }
                    }

                    let source_item_id = match code_item.source_item_id.as_ref() {
                        Some(id) => id,
                        None => continue,
                    };

                    let file_dest_path = temp_folder_dir.join(&code_item.name);
                    let part_dest_path = temp_folder_dir.join(format!("{}.part", code_item.name));
                    let part_dest_str = part_dest_path.to_string_lossy().to_string();

                    {
                        let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                        conn.execute(format!(
                            "UPDATE migration_items SET state = 'downloading' WHERE id = {};",
                            code_item.id
                        ))
                        .map_err(|e| e.to_string())?;
                    }
                    emit_activity(
                        &mig_state.db,
                        &app_handle,
                        job_id,
                        Some(code_item.id),
                        Some(&code_item.name),
                        "downloading",
                        "started",
                        None,
                    );

                    let dl_res = download_item(
                        &http,
                        &access_token,
                        source_item_id,
                        &part_dest_str,
                        None::<fn(u64, u64)>,
                    )
                    .await;

                    match dl_res {
                        Ok(_) => {
                            let _ = std::fs::rename(&part_dest_path, &file_dest_path);
                            downloaded_items.push(code_item.clone());
                        }
                        Err(error) => {
                            log::error!("Failed to download code file {}: {}", code_item.name, error);
                            record_item_failed(
                                &mig_state.db,
                                job_id,
                                code_item.id,
                                "download_failed",
                                &error,
                                true,
                            )?;
                        }
                    }
                }

                if !downloaded_items.is_empty() {
                    let zip_filename = format!("{}.zip", folder_sanitized);
                    let zip_path = local_dir.join(&zip_filename);

                    if let Err(e) = create_zip_from_directory(&temp_folder_dir, &zip_path) {
                        log::error!("Failed to zip folder {}: {}", folder_name, e);
                    } else {
                        let destination_id = if auto_job {
                            match ensure_auto_type_destination(&app_handle, "Auto Documents").await {
                                Ok(id) => Some(id),
                                Err(_) => job.telegram_destination_id,
                            }
                        } else {
                            job.telegram_destination_id
                        };

                        let tg_state = app_handle.state::<TelegramState>();
                        let client_guard = tg_state.client.lock().await;
                        if let Some(ref client) = *client_guard {
                            let zip_path_str = zip_path.to_string_lossy().to_string();
                            let upload_res = upload_core(
                                client,
                                &tg_state.peer_cache,
                                &zip_path_str,
                                &zip_filename,
                                destination_id,
                                &mig_state.cancel_token,
                                None::<fn(u64, u64)>,
                            )
                            .await;

                                match upload_res {
                                    Ok(res) => {
                                        for d_item in &downloaded_items {
                                            record_item_success(
                                                &mig_state.db,
                                                job_id,
                                                d_item.id,
                                                "",
                                                None,
                                                d_item.size_bytes,
                                                destination_id,
                                                res.message_id,
                                                auto_job,
                                            )?;
                                            emit_activity(
                                                &mig_state.db,
                                                &app_handle,
                                                job_id,
                                                Some(d_item.id),
                                                Some(&d_item.name),
                                                "completed",
                                                "success",
                                                Some(&format!("Đã nén trong {}", zip_filename)),
                                            );

                                            if auto_job {
                                                if let Some(source_id) = d_item.source_item_id.as_deref() {
                                                    if let Err(error) =
                                                        delete_onedrive_item(&http, &access_token, source_id).await
                                                    {
                                                        log::warn!(
                                                            "Uploaded zipped '{}' but failed to delete OneDrive source: {}",
                                                            d_item.name,
                                                            error
                                                        );
                                                        emit_activity(
                                                            &mig_state.db,
                                                            &app_handle,
                                                            job_id,
                                                            Some(d_item.id),
                                                            Some(&d_item.name),
                                                            "cleanup",
                                                            "failed",
                                                            Some(&error),
                                                        );
                                                    } else {
                                                        log::info!(
                                                            "Deleted OneDrive source after successful zip upload: {}",
                                                            d_item.name
                                                        );
                                                    }
                                                }
                                            }

                                            let _ = app_handle.emit(
                                                "migration:item-complete",
                                                serde_json::json!({
                                                    "job_id": job_id,
                                                    "item_id": d_item.id,
                                                    "item_name": d_item.name,
                                                    "status": "completed"
                                                }),
                                            );
                                        }
                                    }
                                Err(error) => {
                                    log::error!("Failed to upload zipped folder {}: {}", zip_filename, error);
                                    for d_item in &downloaded_items {
                                        record_item_failed(
                                            &mig_state.db,
                                            job_id,
                                            d_item.id,
                                            "upload_failed",
                                            &error.to_string(),
                                            true,
                                        )?;
                                    }
                                }
                            }
                        } else {
                            log::error!("Telegram client not connected for folder zip upload");
                        }
                        let _ = std::fs::remove_file(&zip_path);
                    }
                }
                let _ = std::fs::remove_dir_all(&temp_folder_dir);
                continue;
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

        // Check if item action is Download Only
        if item.action_type.as_deref() == Some("download") {
            let final_dest_path = local_dir.join(&item.name);
            let _ = std::fs::rename(&part_path, &final_dest_path);
            {
                let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                let now = chrono::Utc::now().timestamp();
                conn.execute(format!(
                    "UPDATE migration_items SET state = 'completed', completed_at = {} WHERE id = {};",
                    now, item.id
                ))
                .map_err(|e| e.to_string())?;
            }
            emit_activity(
                &mig_state.db,
                &app_handle,
                job_id,
                Some(item.id),
                Some(&item.name),
                "completed",
                "completed",
                Some("Tải xuống thành công (giữ lại local, không upload TD, không xóa OD)"),
            );
            let _ = app_handle.emit(
                "migration:item-complete",
                serde_json::json!({
                    "job_id": job_id,
                    "item_id": item.id,
                    "item_name": item.name,
                    "status": "completed"
                }),
            );
            continue;
        }

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

        let transcode_manager = app_handle.state::<Arc<crate::transcode::TranscodeManager>>();
        let cached_ffmpeg_path = transcode_manager.ffmpeg_path.lock().await.clone();
        let ffmpeg_path = match cached_ffmpeg_path {
            Some(path) => path,
            None => match crate::transcode::detect_ffmpeg(&app_handle).await {
                Some(path) => {
                    *transcode_manager.ffmpeg_path.lock().await = Some(path.clone());
                    path
                }
                None => std::path::PathBuf::from("ffmpeg"),
            },
        };
        let media_output_path =
            local_dir.join(format!("mig_{}_{}.transcoded.mp4", job_id, item.id));
        let app_handle_media = app_handle.clone();
        let item_name_media = item.name.clone();
        let media_revision = Arc::new(AtomicU64::new(0));
        let media_revision_cb = media_revision.clone();
        let attempt = item.attempt_count;
        let prepared_res = prepare_upload(
            &ffmpeg_path,
            &part_path,
            &item.name,
            &media_output_path,
            &mig_state.cancel_token,
            move |progress| {
                let revision = media_revision_cb.fetch_add(1, Ordering::Relaxed) + 1;
                let phase = progress.phase.as_str();
                let _ = app_handle_media.emit(
                    "migration:item-progress",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item_name_media,
                        "phase": phase,
                        "event_id": format!("{}:{}:{}:{}:{}", job_id, item.id, attempt, phase, revision),
                        "attempt": attempt,
                        "revision": revision,
                        "percent": progress.percent,
                        "bytes_done": 0,
                        "bytes_total": item.size_bytes.max(0),
                        "speed_bytes_per_sec": 0,
                        "timestamp": chrono::Utc::now().timestamp_millis()
                    }),
                );
            },
        )
        .await;
        let mut prepared_upload = match prepared_res {
            Ok(prepared) => {
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "processing",
                    "completed",
                    Some(match prepared.decision {
                        crate::migration::media_processor::TranscodeDecision::Transcode => {
                            "migration.video_transcoded"
                        }
                        crate::migration::media_processor::TranscodeDecision::PassthroughCompatible => {
                            "migration.video_passthrough"
                        }
                        crate::migration::media_processor::TranscodeDecision::PassthroughNonVideo => {
                            "migration.non_video_passthrough"
                        }
                    }),
                );
                prepared
            }
            Err(MediaProcessError::Cancelled) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                let conn = mig_state.db.lock().map_err(|e| e.to_string())?;
                conn.execute(format!(
                    "UPDATE migration_items SET state = 'pending' WHERE id = {};",
                    item.id
                ))
                .map_err(|e| e.to_string())?;
                continue;
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                let message = error.to_string();
                record_item_failed(
                    &mig_state.db,
                    job_id,
                    item.id,
                    error.code(),
                    &message,
                    item.attempt_count < 3,
                )?;
                emit_activity(
                    &mig_state.db,
                    &app_handle,
                    job_id,
                    Some(item.id),
                    Some(&item.name),
                    "processing",
                    "failed",
                    Some(&message),
                );
                let _ = app_handle.emit(
                    "migration:item-complete",
                    serde_json::json!({
                        "job_id": job_id,
                        "item_id": item.id,
                        "item_name": item.name,
                        "status": "failed",
                        "error_type": error.code(),
                        "error_message": message
                    }),
                );
                continue;
            }
        };
        let upload_path_str = prepared_upload.path.to_string_lossy().to_string();
        let upload_size = prepared_upload.size_bytes as i64;

        // Auto jobs are routed into one Telegram Drive channel per file type.
        let destination_id = if auto_job {
            let folder_name = auto_folder_name(&prepared_upload.upload_name).to_string();
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
                "bytes_total": upload_size,
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
            &upload_path_str,
            &prepared_upload.upload_name,
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
        prepared_upload.cleanup().await;
    }

    Ok(())
}

#[cfg(test)]
mod worker_code_text_tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_is_video_file() {
        assert!(is_video_file("video.mp4"));
        assert!(is_video_file("movie.mkv"));
        assert!(is_video_file("clip.mov"));

        assert!(!is_video_file("photo.jpg"));
        assert!(!is_video_file("song.mp3"));
        assert!(!is_video_file("main.py"));
        assert!(!is_video_file("notes.txt"));
        assert!(!is_video_file("data.db"));
        assert!(!is_video_file("archive.zip"));
    }

    #[test]
    fn test_get_parent_folder_info() {
        let (path, name) = get_parent_folder_info("Projects/my_app/src/main.rs", "main.rs");
        assert_eq!(path, "Projects/my_app/src");
        assert_eq!(name, "src");

        let (path_root, name_root) = get_parent_folder_info("index.js", "index.js");
        assert_eq!(path_root, "");
        assert_eq!(name_root, "Root");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("my-folder_1"), "my-folder_1");
        assert_eq!(sanitize_filename("src/code"), "src_code");
        assert_eq!(sanitize_filename("???"), "Folder");
    }

    #[test]
    fn test_get_top_level_folder_info() {
        let (path1, name1) = get_top_level_folder_info("Projects/Backend/src/main.py", "main.py");
        assert_eq!(path1, "Projects");
        assert_eq!(name1, "Projects");

        let (path2, name2) = get_top_level_folder_info("Docs/API/spec.md", "spec.md");
        assert_eq!(path2, "Docs");
        assert_eq!(name2, "Docs");

        let (path_root, name_root) = get_top_level_folder_info("README.md", "README.md");
        assert_eq!(path_root, "");
        assert_eq!(name_root, "Root_Files");
    }

    #[test]
    fn test_create_zip_from_directory_recursive() {
        let temp_dir = std::env::temp_dir().join(format!("test_zip_rec_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let zip_file = std::env::temp_dir().join(format!("test_rec_out_{}.zip", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let sub_dir = temp_dir.join("src/components");
        std::fs::create_dir_all(&sub_dir).unwrap();

        let file1 = sub_dir.join("Header.tsx");
        File::create(&file1).unwrap().write_all(b"export const Header = () => null;").unwrap();

        let result = create_zip_from_directory_recursive(&temp_dir, &zip_file);
        assert!(result.is_ok());
        assert!(zip_file.exists());
        assert!(std::fs::metadata(&zip_file).unwrap().len() > 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::remove_file(&zip_file);
    }
}
