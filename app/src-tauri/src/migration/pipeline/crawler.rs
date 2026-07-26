use crate::migration::db::MigrationDb;
use crate::migration::microsoft::{parse_graph_item, send_graph_request};
use crate::migration::models::FolderQueueItem;
use crate::migration::pipeline::runner::CancellationToken;
use crate::migration::pipeline::stages::PipelineItem;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;
use crate::migration::microsoft::MicrosoftSession;

pub struct StreamingCrawler {
    pub db: MigrationDb,
    pub job_id: i64,
    pub ms_session: Arc<TokioMutex<Option<MicrosoftSession>>>,
    pub cancel_token: CancellationToken,
}

impl StreamingCrawler {
    pub async fn run(self: Arc<Self>, tx: mpsc::Sender<PipelineItem>) -> Result<(), String> {
        loop {
            if self.cancel_token.is_cancelled() || self.cancel_token.is_stopped() {
                break;
            }
            if self.cancel_token.is_paused() {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                continue;
            }

            // 1. Lấy một folder pending hoặc fetching từ database
            let folder_queue_item = self.get_next_folder()?;

            match folder_queue_item {
                Some(folder) => {
                    if let Err(e) = self.process_folder(folder, &tx).await {
                        log::error!("Crawler failed to process folder: {}", e);
                        // Sleep briefly on error to avoid tight spin loops on persistent network errors
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    }
                }
                None => {
                    // Nếu không còn folder nào pending, kiểm tra xem còn active file processing không.
                    // Nếu crawler đã cạn folder, nó có thể ngủ dài hơn chờ pipeline finish hoặc kết thúc.
                    log::info!("Crawler: No more folders to process.");
                    // We just break out of crawler loop. The channel `tx` is dropped, which cascades EOF to pipeline.
                    break;
                }
            }
        }
        
        println!("Crawler loop exited for job {}", self.job_id);
        Ok(())
    }

    fn get_next_folder(&self) -> Result<Option<FolderQueueItem>, String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT id, job_id, folder_id, parent_id, folder_path, state, next_page_link, has_more, discovered_files_count, discovered_folders_count, completed_files_count, last_error, created_at, updated_at
             FROM folder_queue
             WHERE job_id = ? AND state IN ('pending', 'fetching') AND has_more = 1
             ORDER BY id ASC LIMIT 1;"
        ).map_err(|e| e.to_string())?;
        
        stmt.bind((1, self.job_id)).map_err(|e| e.to_string())?;

        if let Ok(sqlite::State::Row) = stmt.next() {
            Ok(Some(FolderQueueItem {
                id: stmt.read(0).unwrap_or(0),
                job_id: stmt.read(1).unwrap_or(0),
                folder_id: stmt.read(2).unwrap_or_default(),
                parent_id: stmt.read(3).unwrap_or(None),
                folder_path: stmt.read(4).unwrap_or_default(),
                state: stmt.read(5).unwrap_or_default(),
                next_page_link: stmt.read(6).unwrap_or(None),
                has_more: stmt.read(7).unwrap_or(0) == 1,
                discovered_files_count: stmt.read(8).unwrap_or(0),
                discovered_folders_count: stmt.read(9).unwrap_or(0),
                completed_files_count: stmt.read(10).unwrap_or(0),
                last_error: stmt.read(11).unwrap_or(None),
                created_at: stmt.read(12).unwrap_or(0),
                updated_at: stmt.read(13).unwrap_or(0),
            }))
        } else {
            Ok(None)
        }
    }

    async fn process_folder(&self, folder: FolderQueueItem, tx: &mpsc::Sender<PipelineItem>) -> Result<(), String> {
        // Cập nhật state sang 'fetching'
        {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            let mut upd = conn.prepare("UPDATE folder_queue SET state = 'fetching' WHERE id = ?;").unwrap();
            upd.bind((1, folder.id)).unwrap();
            upd.next().unwrap();
        }

        let access_token = {
            let mut guard = self.ms_session.lock().await;
            if let Some(ref mut session) = *guard {
                if session.is_expired() {
                    crate::migration::microsoft::refresh_access_token(session).await?;
                }
                session.access_token.clone()
            } else {
                return Err("Microsoft account not connected".into());
            }
        };

        let request_url = if let Some(link) = &folder.next_page_link {
            link.clone()
        } else if folder.folder_id == "root" {
            "https://graph.microsoft.com/v1.0/me/drive/root/children?$top=200".to_string()
        } else {
            format!(
                "https://graph.microsoft.com/v1.0/me/drive/items/{}/children?$top=200",
                folder.folder_id
            )
        };
        let http = reqwest::Client::new();
        
        let cancel_future = async {
            loop {
                if self.cancel_token.is_cancelled() || self.cancel_token.is_stopped() {
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        };

        let json = tokio::select! {
            res = send_graph_request(&http, &request_url, &access_token) => {
                match res {
                    Ok(v) => v,
                    Err(e) => {
                        let conn = self.db.lock().map_err(|e2| e2.to_string())?;
                        let mut upd = conn.prepare("UPDATE folder_queue SET last_error = ? WHERE id = ?;").unwrap();
                        upd.bind((1, e.as_str())).unwrap();
                        upd.bind((2, folder.id)).unwrap();
                        upd.next().unwrap();
                        return Err(e);
                    }
                }
            }
            _ = cancel_future => {
                return Err("Crawler cancelled during Graph API request".to_string());
            }
        };

        let mut files_to_insert = Vec::new();
        let mut folders_to_insert = Vec::new();

        if let Some(arr) = json["value"].as_array() {
            for val in arr {
                let parsed = parse_graph_item(val, &folder.folder_path);
                if parsed.item_type == "folder" {
                    folders_to_insert.push(parsed);
                } else {
                    files_to_insert.push(parsed);
                }
            }
        }

        let next_link = json["@odata.nextLink"].as_str().map(ToString::to_string);
        let has_more = next_link.is_some();
        let state = if has_more { "fetching" } else { "completed" };

        let now = Utc::now().timestamp();
        
        let mut db_pipeline_items = Vec::new();

        // Transaction insert 
        {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            conn.execute("BEGIN TRANSACTION;").map_err(|e| e.to_string())?;
            
            struct TransactionGuard<'a>(&'a sqlite::Connection, bool);
            impl<'a> Drop for TransactionGuard<'a> {
                fn drop(&mut self) {
                    if !self.1 {
                        let _ = self.0.execute("ROLLBACK;");
                    }
                }
            }
            let mut guard = TransactionGuard(&conn, false);

            // 1. Insert child folders
            let mut stmt_folder = conn.prepare(
                "INSERT OR IGNORE INTO folder_queue (
                    job_id, folder_id, parent_id, folder_path, state, created_at, updated_at
                ) VALUES (?, ?, ?, ?, 'pending', ?, ?);"
            ).map_err(|e| e.to_string())?;
            
            for child_folder in &folders_to_insert {
                let child_path = if folder.folder_path.is_empty() || folder.folder_path == "/" {
                    child_folder.name.clone()
                } else {
                    format!("{}/{}", folder.folder_path, child_folder.name)
                };
                
                stmt_folder.bind((1, self.job_id)).unwrap();
                stmt_folder.bind((2, child_folder.id.as_str())).unwrap();
                stmt_folder.bind((3, folder.folder_id.as_str())).unwrap();
                stmt_folder.bind((4, child_path.as_str())).unwrap();
                stmt_folder.bind((5, now)).unwrap();
                stmt_folder.bind((6, now)).unwrap();
                stmt_folder.next().ok();
                stmt_folder.reset().ok();
            }

            // 2. Insert files
            let mut stmt_file = conn.prepare(
                "INSERT OR IGNORE INTO migration_items (
                    job_id, folder_id, source_item_id, name, path, size, item_category, pipeline_stage, created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, 'queued_download', ?, ?);"
            ).map_err(|e| e.to_string())?;

            for child_file in &files_to_insert {
                let child_path = if folder.folder_path.is_empty() || folder.folder_path == "/" {
                    child_file.name.clone()
                } else {
                    format!("{}/{}", folder.folder_path, child_file.name)
                };

                stmt_file.bind((1, self.job_id)).unwrap();
                stmt_file.bind((2, folder.folder_id.as_str())).unwrap();
                stmt_file.bind((3, child_file.id.as_str())).unwrap();
                stmt_file.bind((4, child_file.name.as_str())).unwrap();
                stmt_file.bind((5, child_path.as_str())).unwrap();
                stmt_file.bind((6, child_file.size)).unwrap();
                stmt_file.bind((7, child_file.item_type.as_str())).unwrap();
                stmt_file.bind((8, now)).unwrap();
                stmt_file.bind((9, now)).unwrap();
                
                stmt_file.next().ok();
                
                // Get the inserted row id and current stage to decide if we should push to the pipeline
                let mut last_id_stmt = conn.prepare("SELECT id, pipeline_stage FROM migration_items WHERE job_id = ? AND source_item_id = ? LIMIT 1;").unwrap();
                last_id_stmt.bind((1, self.job_id)).unwrap();
                last_id_stmt.bind((2, child_file.id.as_str())).unwrap();
                
                if let Ok(sqlite::State::Row) = last_id_stmt.next() {
                    let item_id = last_id_stmt.read::<i64, _>(0).unwrap();
                    let stage = last_id_stmt.read::<String, _>(1).unwrap_or_default();
                    
                    // Do not enqueue if completed, failed, or skipped
                    let terminal_states = ["completed_telegram", "completed_local", "failed", "skipped_duplicate", "reconciliation_required"];
                    if !terminal_states.contains(&stage.as_str()) {
                        db_pipeline_items.push(PipelineItem {
                            id: item_id,
                            job_id: self.job_id,
                            name: child_file.name.clone(),
                            source_path: child_path,
                            source_item_id: Some(child_file.id.clone()),
                            size_bytes: child_file.size,
                            source_etag: child_file.etag.clone(),
                            source_last_modified: child_file.last_modified.clone(),
                            source_fingerprint_type: None, // We don't save fingerprints initially now
                            source_fingerprint_value: None,
                            state: stage,
                            original_sha256: None,
                            processed_sha256: None,
                            local_dest_path: None,
                            telegram_random_id: None,
                            video_decision: None,

                        });
                    }
                }
                last_id_stmt.reset().ok();
                stmt_file.reset().ok();
            }
            
            // 3. Cập nhật thống kê job
            let mut upd_job = conn.prepare(
                "UPDATE migration_jobs 
                 SET discovered_folders = discovered_folders + ?, 
                     discovered_items = discovered_items + ?, 
                     waiting_items = waiting_items + ?
                 WHERE id = ?;"
            ).unwrap();
            upd_job.bind((1, folders_to_insert.len() as i64)).unwrap();
            upd_job.bind((2, files_to_insert.len() as i64)).unwrap();
            upd_job.bind((3, files_to_insert.len() as i64)).unwrap();
            upd_job.bind((4, self.job_id)).unwrap();
            upd_job.next().unwrap();

            // 4. Update folder state
            let mut upd_folder = conn.prepare(
                "UPDATE folder_queue 
                 SET state = ?, 
                     next_page_link = ?, 
                     has_more = ?, 
                     discovered_files_count = discovered_files_count + ?, 
                     discovered_folders_count = discovered_folders_count + ?,
                     last_error = NULL,
                     updated_at = ?
                 WHERE id = ?;"
            ).unwrap();
            upd_folder.bind((1, state)).unwrap();
            match next_link {
                Some(link) => upd_folder.bind((2, link.as_str())).unwrap(),
                None => upd_folder.bind((2, sqlite::Value::Null)).unwrap(),
            }
            upd_folder.bind((3, if has_more { 1i64 } else { 0i64 })).unwrap();
            upd_folder.bind((4, files_to_insert.len() as i64)).unwrap();
            upd_folder.bind((5, folders_to_insert.len() as i64)).unwrap();
            upd_folder.bind((6, now)).unwrap();
            upd_folder.bind((7, folder.id)).unwrap();
            upd_folder.next().unwrap();

            conn.execute("COMMIT;").map_err(|e| e.to_string())?;
            guard.1 = true;
        }

        // 5. Nạp thẳng các file mới quét được vào bounded channel
        // Bounded channel tx sẽ backpressure nếu hệ thống tải quá nhiều tệp chưa xử lý kịp.
        for item in db_pipeline_items {
            if self.cancel_token.is_cancelled() || self.cancel_token.is_stopped() {
                break;
            }
            // Gửi vào channel, block nếu channel đầy.
            // Điều này tạo ra backpressure tự nhiên lên Crawler.
            if tx.send(item).await.is_err() {
                log::error!("Crawler failed to send item to pipeline channel (channel closed)");
                break;
            }
        }

        Ok(())
    }
}
