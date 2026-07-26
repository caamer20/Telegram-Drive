use crate::migration::db::MigrationDb;

use crate::migration::pipeline::classifier::{classify_file, FileCategory};
use crate::migration::pipeline::config::PipelineConfig;
use crate::migration::pipeline::stages::{
    LocalFinalizer, MediaInspector, PipelineItem, PipelineStage, SourceDownloader,
    TelegramMediaKind, TelegramUploadRequest, TelegramUploadResult, TelegramUploader,
    VideoProcessor,
};
use crate::migration::pipeline::transitions::update_item_pipeline_stage;
use crate::migration::quota_reserve::{commit_quota, release_quota, reserve_quota};
use crate::migration::telegram_idempotency::get_deterministic_random_id;
use chrono::Utc;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub const STATE_RUNNING: u8 = 0;
pub const STATE_STOPPED: u8 = 1;
pub const STATE_CANCELLED: u8 = 2;

#[derive(Debug, Clone)]
pub struct CancellationToken {
    state: Arc<std::sync::atomic::AtomicU8>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            state: Arc::new(std::sync::atomic::AtomicU8::new(STATE_RUNNING)),
        }
    }

    pub fn cancel(&self) {
        self.state.store(STATE_CANCELLED, Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.state.store(STATE_STOPPED, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_CANCELLED
    }
    pub fn is_stopped(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_STOPPED
    }
    pub fn is_running(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_RUNNING
    }
}

fn sanitize_path(path: &str) -> PathBuf {
    let mut safe = PathBuf::new();
    for component in std::path::Path::new(path).components() {
        if let std::path::Component::Normal(c) = component {
            let s = c.to_string_lossy().to_ascii_uppercase();
            if matches!(
                s.as_ref(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            ) {
                safe.push(format!("{}_safe", c.to_string_lossy()));
            } else {
                safe.push(c);
            }
        }
    }
    safe
}

/// Parse flood wait seconds from Telegram error string
fn parse_flood_wait_seconds(err_str: &str) -> Option<i64> {
    if let Some(idx) = err_str.find("FLOOD_WAIT_") {
        let rest = &err_str[idx + "FLOOD_WAIT_".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<i64>().ok()
    } else if let Some(idx) = err_str.find("flood wait") {
        let digits: String = err_str[idx..]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        digits.parse::<i64>().ok()
    } else {
        None
    }
}

/// Trạng thái hoạt động nội bộ của Pipeline Runner
pub struct PipelineRunner {
    pub config: PipelineConfig,
    pub db: MigrationDb,
    pub job_id: i64,
    pub workspace_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub cancel_token: CancellationToken,
    pub active_tasks: std::sync::Mutex<Vec<JoinHandle<()>>>,
    pub ms_session: Arc<tokio::sync::Mutex<Option<crate::migration::microsoft::MicrosoftSession>>>,
}

impl PipelineRunner {
    pub fn new(
        config: PipelineConfig,
        db: MigrationDb,
        job_id: i64,
        workspace_dir: PathBuf,
        backup_dir: PathBuf,
        ms_session: Arc<tokio::sync::Mutex<Option<crate::migration::microsoft::MicrosoftSession>>>,
    ) -> Self {
        let _ = std::fs::create_dir_all(&workspace_dir);
        let _ = std::fs::create_dir_all(&backup_dir);
        Self {
            config,
            db,
            job_id,
            workspace_dir,
            backup_dir,
            cancel_token: CancellationToken::new(),
            active_tasks: std::sync::Mutex::new(vec![]),
            ms_session,
        }
    }

    /// Khởi chạy toàn bộ bounded pipeline
    pub fn start(
        self: Arc<Self>,
        downloader: Arc<dyn SourceDownloader>,
        inspector: Arc<dyn MediaInspector>,
        processor: Arc<dyn VideoProcessor>,
        uploader: Arc<dyn TelegramUploader>,
        finalizer: Arc<dyn LocalFinalizer>,
    ) -> CancellationToken {
        let (download_tx, download_rx) = mpsc::channel(self.config.download_queue_capacity);
        let (process_tx, process_rx) = mpsc::channel(self.config.processing_queue_capacity);
        let (upload_tx, upload_rx) = mpsc::channel(self.config.upload_queue_capacity);
        let (local_tx, local_rx) = mpsc::channel(self.config.local_finalizer_queue_capacity);

        let _runner_clone = self.clone();
        let cancel = self.cancel_token.clone();

        let crawler = crate::migration::pipeline::crawler::StreamingCrawler {
            db: self.db.clone(),
            job_id: self.job_id,
            ms_session: self.ms_session.clone(),
            cancel_token: self.cancel_token.clone(),
        };
        let crawler_arc = Arc::new(crawler);
        
        let runner_for_loader = self.clone();
        let download_tx_loader = download_tx.clone();
        let process_tx_loader = process_tx.clone();
        let upload_tx_loader = upload_tx.clone();
        let local_tx_loader = local_tx.clone();

        let planner_handle = tokio::spawn(async move {
            let _ = runner_for_loader.dispatch_recovering_items(
                download_tx_loader,
                process_tx_loader,
                upload_tx_loader,
                local_tx_loader,
            ).await;
            let _ = crawler_arc.run(download_tx).await;
        });

        // 2. Task Downloader (tải tệp từ nguồn)
        let runner_clone = self.clone();
        let upload_tx_clone = upload_tx.clone();
        let download_handle = tokio::spawn(async move {
            let _ = runner_clone
                .run_downloader(
                    download_rx,
                    process_tx,
                    upload_tx_clone,
                    local_tx,
                    downloader,
                )
                .await;
        });

        // 3. Task Processor (inspect và xử lý video bằng ffmpeg)
        let runner_clone = self.clone();
        let process_handle = tokio::spawn(async move {
            let _ = runner_clone
                .run_processor(process_rx, upload_tx, inspector, processor)
                .await;
        });

        // 4. Task Uploader (tải tệp lên Telegram)
        let runner_clone = self.clone();
        let upload_handle = tokio::spawn(async move {
            let _ = runner_clone.run_uploader(upload_rx, uploader).await;
        });

        // 5. Task Local Finalizer (di chuyển tệp backup local)
        let runner_clone = self.clone();
        let local_handle = tokio::spawn(async move {
            let _ = runner_clone.run_local_finalizer(local_rx, finalizer).await;
        });

        let mut guard = self.active_tasks.lock().unwrap();
        guard.push(planner_handle);
        guard.push(download_handle);
        guard.push(process_handle);
        guard.push(upload_handle);
        guard.push(local_handle);

        cancel
    }

    pub async fn run_to_completion(&self) -> Result<(), String> {
        // Drain active tasks (5 main loop tasks)
        let main_tasks: Vec<JoinHandle<()>> = {
            let mut guard = self.active_tasks.lock().unwrap();
            std::mem::take(&mut *guard)
        };

        let mut first_error: Option<String> = None;
        for handle in main_tasks {
            match handle.await {
                Ok(_) => {}
                Err(e) => {
                    let err_msg = if e.is_panic() {
                        format!("Worker panicked: {:?}", e)
                    } else {
                        format!("Worker cancelled: {}", e)
                    };
                    log::error!("Pipeline task error: {}", err_msg);
                    if first_error.is_none() {
                        first_error = Some(err_msg);
                    }
                }
            }
        }

        // Finalize job state
        self.finalize_job().await?;

        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }

    /// Finalize job state based on item outcomes
    async fn finalize_job(&self) -> Result<(), String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;
        
        // Count items by stage
        let mut stmt = conn.prepare(
            "SELECT pipeline_stage, COUNT(*) FROM migration_items WHERE job_id = ? GROUP BY pipeline_stage"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, self.job_id)).map_err(|e| e.to_string())?;
        
        let mut failed_count: i64 = 0;
        let mut completed_telegram: i64 = 0;
        let mut completed_local: i64 = 0;
        let mut waiting_quota: i64 = 0;
        let mut reconciliation: i64 = 0;
        let mut pending_count: i64 = 0;
        
        while let Ok(sqlite::State::Row) = stmt.next() {
            let stage: String = stmt.read(0).unwrap_or_default();
            let count: i64 = stmt.read(1).unwrap_or(0);
            match stage.as_str() {
                "completed_telegram" => completed_telegram = count,
                "completed_local" => completed_local = count,
                "failed" => failed_count = count,
                "waiting_for_quota" => waiting_quota = count,
                "reconciliation_required" => reconciliation = count,
                _ => pending_count += count,
            }
        }
        
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        
        let job_state = if self.cancel_token.is_stopped() {
            "stopped"
        } else if waiting_quota > 0 && pending_count == 0 {
            "waiting_for_quota"
        } else if failed_count > 0 || reconciliation > 0 {
            "completed_with_errors"
        } else {
            "completed"
        };
        
        let mut upd = conn.prepare(
            "UPDATE migration_jobs SET state = ?, completed_items = ?, failed_items = ?, waiting_items = ?, completed_at = ?, updated_at = ? WHERE id = ?"
        ).map_err(|e| e.to_string())?;
        upd.bind((1, job_state)).map_err(|e| e.to_string())?;
        upd.bind((2, completed_telegram + completed_local)).map_err(|e| e.to_string())?;
        upd.bind((3, failed_count)).map_err(|e| e.to_string())?;
        upd.bind((4, waiting_quota + reconciliation)).map_err(|e| e.to_string())?;
        upd.bind((5, now)).map_err(|e| e.to_string())?;
        upd.bind((6, now)).map_err(|e| e.to_string())?;
        upd.bind((7, self.job_id)).map_err(|e| e.to_string())?;
        upd.next().map_err(|e| e.to_string())?;
        
        Ok(())
    }

    async fn dispatch_recovering_items(
        &self,
        download_tx: mpsc::Sender<PipelineItem>,
        process_tx: mpsc::Sender<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        local_tx: mpsc::Sender<PipelineItem>,
    ) -> Result<(), String> {
        let items = {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT id, job_id, name, path, source_item_id, size, item_category, original_sha256, processed_sha256, video_decision, pipeline_stage 
                 FROM migration_items 
                 WHERE job_id = ? AND pipeline_stage IN ('discovered', 'downloaded', 'reconciliation_required');"
            ).map_err(|e| e.to_string())?;
            
            stmt.bind((1, self.job_id)).map_err(|e| e.to_string())?;
            
            let mut items = Vec::new();
            while let Ok(sqlite::State::Row) = stmt.next() {
                let id: i64 = stmt.read(0).unwrap();
                let job_id: i64 = stmt.read(1).unwrap();
                let name: String = stmt.read(2).unwrap();
                let path: String = stmt.read(3).unwrap();
                let source_item_id: String = stmt.read(4).unwrap();
                let size_bytes: i64 = stmt.read(5).unwrap();
                let _category: String = stmt.read(6).unwrap_or_default();
                let original_sha256: Option<String> = stmt.read(7).unwrap_or_default();
                let processed_sha256: Option<String> = stmt.read(8).unwrap_or_default();
                let video_decision: Option<String> = stmt.read(9).unwrap_or_default();
                let state: String = stmt.read(10).unwrap();
                
                items.push(PipelineItem {
                    id,
                    job_id,
                    name,
                    source_path: path,
                    source_item_id: Some(source_item_id),
                    size_bytes,
                    source_etag: None,
                    source_last_modified: None,
                    source_fingerprint_type: None,
                    source_fingerprint_value: None,
                    state: state.clone(),
                    original_sha256,
                    processed_sha256,
                    local_artifact_path: None,
                    telegram_random_id: None,
                    video_decision,
                });
            }
            items
        };

        for item in items {
            if self.cancel_token.is_cancelled() || self.cancel_token.is_stopped() {
                break;
            }
            let stage = item.state.as_str();
            match stage {
                "discovered" => {
                    let _ = download_tx.send(item).await;
                }
                "downloaded" => {
                    // Need to route based on category
                    let category = crate::migration::pipeline::classifier::classify_file(&item.name);
                    match category {
                        crate::migration::pipeline::classifier::FileCategory::Video => {
                            let _ = process_tx.send(item).await;
                        }
                        crate::migration::pipeline::classifier::FileCategory::Image => {
                            let _ = upload_tx.send(item).await;
                        }
                        crate::migration::pipeline::classifier::FileCategory::Other => {
                            let _ = local_tx.send(item).await;
                        }
                    }
                }
                "reconciliation_required" => {
                    let _ = upload_tx.send(item).await;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Check if job has any non-terminal items remaining
    pub fn has_pending_items(&self) -> bool {
        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(_) => return true, // assume pending on error
        };
        let mut stmt = match conn.prepare(
            "SELECT COUNT(*) FROM migration_items WHERE job_id = ? AND pipeline_stage NOT IN ('completed_telegram', 'completed_local', 'failed', 'reconciliation_required');"
        ) {
            Ok(s) => s,
            Err(_) => return true,
        };
        stmt.bind((1, self.job_id)).ok();
        if let Ok(sqlite::State::Row) = stmt.next() {
            let count: i64 = stmt.read(0).unwrap_or(1);
            count > 0
        } else {
            true
        }
    }



    /// Task Downloader tải tệp và định tuyến
    async fn run_downloader(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        process_tx: mpsc::Sender<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        local_tx: mpsc::Sender<PipelineItem>,
        downloader: Arc<dyn SourceDownloader>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();
        println!("Downloader loop started with {} workers", self.config.download_concurrency);

        for _ in 0..self.config.download_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let downloader_clone = downloader.clone();
            let process_tx_clone = process_tx.clone();
            let upload_tx_clone = upload_tx.clone();
            let local_tx_clone = local_tx.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();

            let handle = tokio::spawn(async move {
                loop {
                    if cancel.is_cancelled() || cancel.is_stopped() {
                        break;
                    }

                    let item_opt = {
                        let mut guard = rx_clone.lock().await;
                        guard.recv().await
                    };
                    let mut item = match item_opt {
                        Some(i) => i,
                        None => break, // Channel closed
                    };

                println!("Downloader task active for item {}", item.id);

                // 1. Chuyển sang Downloading
                if let Err(e) =
                    update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Downloading)
                {
                    println!(
                        "Downloader failed to update stage to Downloading for item {}: {}",
                        item.id, e
                    );
                }


                let category = classify_file(&item.name);

                // 3. Thực hiện tải (ghi tệp tạm .part rồi rename)
                let part_path = workspace.join(format!("{}.part", item.id));
                let final_path = workspace.join(format!("{}", item.id));

                if let Some(source_id) = &item.source_item_id {
                    println!("Downloader starting download_file for item {}", item.id);
                    match downloader_clone
                        .download_file(item.id, source_id, &part_path)
                        .await
                    {
                        Ok(sha256) => {
                            println!(
                                "Downloader download_file OK for item {}, sha256={}",
                                item.id, sha256
                            );
                            // Đảm bảo tạo directory đích
                            if let Some(parent) = final_path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            // Atomic Rename
                            if let Err(e) = std::fs::rename(&part_path, &final_path) {
                                println!("Downloader rename failed for item {}: {}", item.id, e);
                                let _ = update_item_pipeline_stage(
                                    &db_clone,
                                    item.id,
                                    PipelineStage::Failed,
                                );
                                // Release disk on rename failure
                                {
                            }
                            } else {
                                println!("Downloader rename OK for item {}", item.id);
                                item.original_sha256 = Some(sha256.clone());
                                // Update original hash in DB
                                {
                                    let conn = db_clone.lock().unwrap();
                                    let mut upd_hash = conn.prepare("UPDATE migration_items SET original_sha256 = ? WHERE id = ?;").unwrap();
                                    upd_hash.bind((1, sha256.as_str())).unwrap();
                                    upd_hash.bind((2, item.id)).unwrap();
                                    upd_hash.next().unwrap();
                                }

                                if let Err(e) = update_item_pipeline_stage(
                                    &db_clone,
                                    item.id,
                                    PipelineStage::Downloaded,
                                ) {
                                    println!("Downloader failed to update stage to Downloaded for item {}: {}", item.id, e);
                                }

                                // 4. Giải phóng disk reservation
                                {
                                    let _conn = db_clone.lock().unwrap();
                                }

                                println!("Downloader routing item {} as {:?}", item.id, category);

                                // 5. Định tuyến (routing)
                                match category {
                                    FileCategory::Video => {
                                        let _ = update_item_pipeline_stage(
                                            &db_clone,
                                            item.id,
                                            PipelineStage::QueuedProcessing,
                                        );
                                        let _ = process_tx_clone.send(item).await;
                                    }
                                    FileCategory::Image => {
                                        let _ = update_item_pipeline_stage(
                                            &db_clone,
                                            item.id,
                                            PipelineStage::QueuedUpload,
                                        );
                                        let _ = upload_tx_clone.send(item).await;
                                    }
                                    FileCategory::Other => {
                                        let _ = update_item_pipeline_stage(
                                            &db_clone,
                                            item.id,
                                            PipelineStage::SavingLocal,
                                        );
                                        let _ = local_tx_clone.send(item).await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("Downloader download_file Err for item {}: {}", item.id, e);
                            let _ = update_item_pipeline_stage(
                                &db_clone,
                                item.id,
                                PipelineStage::Failed,
                            );
                            let _ = std::fs::remove_file(&part_path);
                        }
                    }
                } else {
                    println!("Downloader source_item_id is None for item {}", item.id);
                    let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Failed);
                }
                }
            });
            workers.push(handle);
        }

        for w in workers {
            if let Err(e) = w.await {
                let err_msg = if e.is_panic() {
                    format!("Downloader worker panicked: {:?}", e)
                } else {
                    format!("Downloader worker cancelled: {}", e)
                };
                log::error!("{}", err_msg);
                return Err(err_msg);
            }
        }
        Ok(())
    }
    async fn run_processor(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        inspector: Arc<dyn MediaInspector>,
        processor: Arc<dyn VideoProcessor>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();

        for _ in 0..self.config.processing_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let inspector_clone = inspector.clone();
            let processor_clone = processor.clone();
            let upload_tx_clone = upload_tx.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();

            let handle = tokio::spawn(async move {
                loop {
                    if cancel.is_cancelled() || cancel.is_stopped() {
                        break;
                    }
                    
                    let item_opt = {
                        let mut guard = rx_clone.lock().await;
                        guard.recv().await
                    };
                    let mut item = match item_opt {
                        Some(i) => i,
                        None => break,
                    };

                let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Processing);

                let input_path = workspace.join(format!("{}", item.id));
                let output_path = workspace.join(format!("{}.processed.mp4", item.id));

                // Inspect video
                match inspector_clone.inspect_file(&input_path).await {
                    Ok(meta) => {
                        // Quyết định xử lý: passthrough | remux_copy | transcode
                        let decision = if meta.video_codec == "h264" {
                            // h264: pass-through if mp4/mov container, else remux
                            let container = meta.container.to_ascii_lowercase();
                            if container == "mp4" || container == "mov" {
                                "passthrough"
                            } else {
                                "remux_copy"
                            }
                        } else {
                            "transcode"
                        };

                        item.video_decision = Some(decision.to_string());

                        // Cập nhật decision
                        {
                            let conn = db_clone.lock().unwrap();
                            let mut upd = conn
                                .prepare(
                                    "UPDATE migration_items SET video_decision = ? WHERE id = ?;",
                                )
                                .unwrap();
                            upd.bind((1, decision)).unwrap();
                            upd.bind((2, item.id)).unwrap();
                            upd.next().unwrap();
                        }

                        if decision == "passthrough" {
                            let _ = update_item_pipeline_stage(
                                &db_clone,
                                item.id,
                                PipelineStage::QueuedUpload,
                            );
                            let _ = upload_tx_clone.send(item).await;
                        } else {
                            // Gọi FFmpeg processor
                            match processor_clone
                                .process_video(&input_path, &output_path, decision)
                                .await
                            {
                                Ok(proc_hash) => {
                                    item.processed_sha256 = Some(proc_hash.clone());
                                    // Update processed hash
                                    {
                                        let conn = db_clone.lock().unwrap();
                                        let mut upd = conn.prepare("UPDATE migration_items SET processed_sha256 = ? WHERE id = ?;").unwrap();
                                        upd.bind((1, proc_hash.as_str())).unwrap();
                                        upd.bind((2, item.id)).unwrap();
                                        upd.next().unwrap();
                                    }

                                    let item_id = item.id;
                                    let _ = update_item_pipeline_stage(
                                        &db_clone,
                                        item_id,
                                        PipelineStage::QueuedUpload,
                                    );
                                    println!("Processor sending item {} to upload_tx", item_id);
                                    let _ = upload_tx_clone.send(item).await;
                                    println!(
                                        "Processor successfully sent item {} to upload_tx",
                                        item_id
                                    );

                                    // Dọn dẹp tệp tin gốc nếu transcode thành công
                                    let _ = std::fs::remove_file(&input_path);
                                }
                                Err(_) => {
                                    let _ = update_item_pipeline_stage(
                                        &db_clone,
                                        item.id,
                                        PipelineStage::Failed,
                                    );
                                }
                            }
                        }
                    }
                    Err(_) => {
                        let _ =
                            update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Failed);
                    }
                }
                }
            });
            workers.push(handle);
        }
        
        for w in workers {
            if let Err(e) = w.await {
                let err_msg = if e.is_panic() {
                    format!("Processor worker panicked: {:?}", e)
                } else {
                    format!("Processor worker cancelled: {}", e)
                };
                log::error!("{}", err_msg);
                return Err(err_msg);
            }
        }
        Ok(())
    }

    /// Task Uploader upload tệp tin lên Telegram
    async fn run_uploader(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        uploader: Arc<dyn TelegramUploader>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();

        for _ in 0..self.config.upload_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let uploader_clone = uploader.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();

            let handle = tokio::spawn(async move {
                loop {
                    if cancel.is_cancelled() || cancel.is_stopped() {
                        break;
                    }
                    
                    let item_opt = {
                        let mut guard = rx_clone.lock().await;
                        guard.recv().await
                    };
                    let item = match item_opt {
                        Some(i) => i,
                        None => break,
                    };

                    println!("Uploader loop received item {}", item.id);
            // Enforce flood wait check before uploading
            loop {
                let wait_until = {
                    let conn = db_clone.lock().unwrap();
                    let mut stmt = conn.prepare("SELECT flood_wait_until FROM migration_jobs WHERE id = ? LIMIT 1").unwrap();
                    stmt.bind((1, item.job_id)).unwrap();
                    if let Ok(sqlite::State::Row) = stmt.next() {
                        stmt.read::<i64, _>(0).unwrap_or(0)
                    } else {
                        0
                    }
                };

                let now = Utc::now().timestamp();
                if wait_until > now {
                    let sleep_secs = (wait_until - now) as u64;
                    log::info!("Upload: Job {} is under FloodWait, sleeping for {} seconds", item.job_id, sleep_secs);
                    
                    // Sleep in small increments to remain responsive to cancellation
                    let mut slept = 0;
                    while slept < sleep_secs {
                        if cancel.is_cancelled() {
                            return;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        slept += 1;
                    }
                } else {
                    break;
                }
            }

            println!("Uploader spawned task active for item {}", item.id);

                // Chọn đúng tệp processed hoặc original để upload
                // A2 fix: remux_copy ALSO uses .processed.mp4, not just transcode
                let (file_path, artifact_size) = {
                    let path = if matches!(
                        item.video_decision.as_deref(),
                        Some("remux_copy" | "transcode")
                    ) {
                        workspace.join(format!("{}.processed.mp4", item.id))
                    } else {
                        workspace.join(format!("{}", item.id))
                    };
                    let size = std::fs::metadata(&path)
                        .map(|m| m.len() as i64)
                        .unwrap_or(item.size_bytes);
                    (path, size)
                };

                // Quota check — atomic reserve before upload
                loop {
                    if cancel.is_cancelled() || cancel.is_stopped() {
                        break;
                    }
                    let date_string = Utc::now().format("%Y-%m-%d").to_string();
                    let reserved = {
                        let conn = db_clone.lock().unwrap();
                        reserve_quota(
                            &conn,
                            item.id,
                            item.job_id,
                            &date_string,
                            artifact_size,
                            7200, // 2 hour expiry
                        )
                    };
                    
                    match reserved {
                        Ok(_) => break,
                        Err(e) => {
                            log::warn!("Upload: quota reserve failed for item {}: {} — sleeping 5 mins before retry", item.id, e);
                            let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::WaitingForQuota);
                            
                            // Sleep 5 minutes in small increments
                            let sleep_secs = 300;
                            let mut slept = 0;
                            while slept < sleep_secs {
                                if cancel.is_cancelled() || cancel.is_stopped() {
                                    break;
                                }
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                slept += 1;
                            }
                        }
                    }
                }
                
                if cancel.is_cancelled() || cancel.is_stopped() {
                    return; // Exit worker
                }

                let date_string = Utc::now().format("%Y-%m-%d").to_string();
                let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Uploading);

                // Sinh deterministic random_id
                let upload_attempt_id =
                    format!("job_{}_item_{}_attempt_1", item.job_id, item.id);
                let random_id = get_deterministic_random_id(&upload_attempt_id);

                // Determine media kind
                let media_kind = match classify_file(&item.name) {
                    FileCategory::Video => TelegramMediaKind::Video,
                    FileCategory::Image => TelegramMediaKind::Image,
                    FileCategory::Other => TelegramMediaKind::Other,
                };

                let request = TelegramUploadRequest {
                    job_id: item.job_id,
                    item_id: item.id,
                    path: file_path.clone(),
                    filename: item.name.clone(),
                    random_id,
                    destination_id: None,
                    media_kind,
                };

                match uploader_clone.upload_file(request).await {
                    Ok(result) => match result {
                        TelegramUploadResult::Confirmed {
                            message_id,
                            random_id: confirmed_random_id,
                        } => {
                            // Update artifact size in DB
                            {
                                let conn = db_clone.lock().unwrap();
                                let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_items SET artifact_size = ? WHERE id = ?;",
                                    )
                                    .unwrap();
                                upd.bind((1, artifact_size)).unwrap();
                                upd.bind((2, item.id)).unwrap();
                                upd.next().unwrap();
                            }

                            // Commit quota
                            {
                                let conn = db_clone.lock().unwrap();
                                let _ = commit_quota(&conn, item.id, &date_string);
                            }

                            // Ghi nhận thành công
                            {
                                let conn = db_clone.lock().unwrap();
                                let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_items SET telegram_message_id = ?, telegram_random_id = ? WHERE id = ?;",
                                    )
                                    .unwrap();
                                upd.bind((1, message_id)).unwrap();
                                upd.bind((2, confirmed_random_id)).unwrap();
                                upd.bind((3, item.id)).unwrap();
                                upd.next().unwrap();
                            }

                            let _ = update_item_pipeline_stage(
                                &db_clone,
                                item.id,
                                PipelineStage::CompletedTelegram,
                            );

                            // Dọn dẹp tệp tin trong workspace
                            let _ = std::fs::remove_file(&file_path);
                        }
                        TelegramUploadResult::ReconciliationRequired {
                            random_id: rec_random_id,
                            reason,
                        } => {
                            log::warn!(
                                "Upload: reconciliation_required for item {}, random_id={}, reason: {}",
                                item.id, rec_random_id, reason
                            );

                            // Commit quota conservatively (assume sent)
                            {
                                let conn = db_clone.lock().unwrap();
                                let _ = commit_quota(&conn, item.id, &date_string);
                            }

                            // Update item stage
                            {
                                let conn = db_clone.lock().unwrap();
                                let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_items SET telegram_random_id = ?, last_error = ? WHERE id = ?;",
                                    )
                                    .unwrap();
                                upd.bind((1, rec_random_id)).unwrap();
                                upd.bind((2, reason.as_str())).unwrap();
                                upd.bind((3, item.id)).unwrap();
                                upd.next().unwrap();
                            }

                            let _ = update_item_pipeline_stage(
                                &db_clone,
                                item.id,
                                PipelineStage::ReconciliationRequired,
                            );
                        }
                    },
                    Err(e) => {
                        // Release quota on confirmed failure
                        {
                            let conn = db_clone.lock().unwrap();
                            let _ = release_quota(&conn, item.id);
                        }

                        // Check if it's a FloodWait
                        if let Some(seconds) = parse_flood_wait_seconds(&e) {
                            log::warn!(
                                "Upload: FloodWait {}s for item {}",
                                seconds,
                                item.id
                            );
                            // Persist flood wait state
                            let now = Utc::now().timestamp();
                            let next_allowed = now + seconds;
                            {
                                let conn = db_clone.lock().unwrap();
                                let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_jobs SET flood_wait_until = ? WHERE id = ?;",
                                    )
                                    .unwrap();
                                upd.bind((1, next_allowed)).unwrap();
                                upd.bind((2, item.job_id)).unwrap();
                                upd.next().unwrap();
                            }
                            // Sleep outside of DB lock
                            tokio::time::sleep(tokio::time::Duration::from_secs(
                                seconds as u64,
                            ))
                            .await;
                        }

                        let _ = update_item_pipeline_stage(
                            &db_clone,
                            item.id,
                            PipelineStage::Failed,
                        );

                        if e.contains("permanent_error") {
                            // Dedupe promotion removed for canonical flow
                        }
                    }
                }
                }
            });
            workers.push(handle);
        }
        
        for w in workers {
            if let Err(e) = w.await {
                let err_msg = if e.is_panic() {
                    format!("Uploader worker panicked: {:?}", e)
                } else {
                    format!("Uploader worker cancelled: {}", e)
                };
                log::error!("{}", err_msg);
                return Err(err_msg);
            }
        }
        Ok(())
    }

    /// Task Local Finalizer lưu tệp Other vào backup_dir
    async fn run_local_finalizer(
        &self,
        rx: mpsc::Receiver<PipelineItem>,
        finalizer: Arc<dyn LocalFinalizer>,
    ) -> Result<(), String> {
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut workers = Vec::new();

        for _ in 0..self.config.local_finalizer_concurrency {
            let rx_clone = rx.clone();
            let db_clone = self.db.clone();
            let finalizer_clone = finalizer.clone();
            let workspace = self.workspace_dir.clone();
            let backup = self.backup_dir.clone();
            let cancel = self.cancel_token.clone();

            let handle = tokio::spawn(async move {
                loop {
                    if cancel.is_cancelled() || cancel.is_stopped() {
                        break;
                    }
                    
                    let item_opt = {
                        let mut guard = rx_clone.lock().await;
                        guard.recv().await
                    };
                    let item = match item_opt {
                        Some(i) => i,
                        None => break,
                    };
                    let input_path = workspace.join(format!("{}", item.id));
                    let safe_source = sanitize_path(&item.source_path);

                    let base_dest = backup.join("OneDrive_Archive").join(&safe_source);
                    let mut dest_path = base_dest.clone();
                    let mut counter = 1;

                    // Collision handle
                    while dest_path.exists() {
                        let file_stem = base_dest.file_stem().unwrap_or_default().to_string_lossy();
                        let extension = base_dest.extension().unwrap_or_default().to_string_lossy();
                        let new_name = if extension.is_empty() {
                            format!("{}_{}", file_stem, counter)
                        } else {
                            format!("{}_{}.{}", file_stem, counter, extension)
                        };
                        dest_path = base_dest.with_file_name(new_name);
                        counter += 1;
                    }

                    // Tạo parent directories của file đích
                    if let Some(parent) = dest_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }

                    match finalizer_clone
                        .finalize_local(&input_path, &dest_path)
                        .await
                    {
                        Ok(_) => {
                            // Cập nhật local artifact path
                            {
                                let conn = db_clone.lock().unwrap();
                                let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_items SET original_artifact_path = ? WHERE id = ?;",
                                    )
                                    .unwrap();
                                upd.bind((1, dest_path.to_str().unwrap_or_default()))
                                    .unwrap();
                                upd.bind((2, item.id)).unwrap();
                                upd.next().unwrap();
                            }

                            let _ = update_item_pipeline_stage(
                                &db_clone,
                                item.id,
                                PipelineStage::CompletedLocal,
                            );

                            // Dọn dẹp tệp tin workspace
                            let _ = std::fs::remove_file(&input_path);
                        }
                        Err(_) => {
                            let _ =
                                update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Failed);
                        }
                    }
                }
            });
            workers.push(handle);
        }
        
        for w in workers {
            if let Err(e) = w.await {
                let err_msg = if e.is_panic() {
                    format!("Local finalizer worker panicked: {:?}", e)
                } else {
                    format!("Local finalizer worker cancelled: {}", e)
                };
                log::error!("{}", err_msg);
                return Err(err_msg);
            }
        }
        Ok(())
    }
}
