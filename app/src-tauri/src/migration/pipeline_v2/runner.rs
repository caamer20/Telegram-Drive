use crate::migration::db::MigrationDb;
use crate::migration::disk_reserve::{release_disk_space, reserve_disk_space};
use crate::migration::pipeline_v2::classifier::{classify_file, FileCategory};
use crate::migration::pipeline_v2::config::PipelineConfig;
use crate::migration::pipeline_v2::stages::{
    LocalFinalizer, MediaInspector, PipelineItem, PipelineStage, SourceDownloader,
    TelegramMediaKind, TelegramUploadRequest, TelegramUploadResult, TelegramUploader,
    VideoProcessor,
};
use crate::migration::pipeline_v2::transitions::update_item_pipeline_stage;
use crate::migration::quota_reserve::{commit_quota, release_quota, reserve_quota};
use crate::migration::telegram_idempotency::get_deterministic_random_id;
use chrono::Utc;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;

pub const STATE_RUNNING: u8 = 0;
pub const STATE_PAUSED: u8 = 1;
pub const STATE_STOPPED: u8 = 2;
pub const STATE_CANCELLED: u8 = 3;

#[derive(Debug, Clone)]
pub struct CancellationToken {
    state: Arc<std::sync::atomic::AtomicU8>,
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
    pub fn pause(&self) {
        self.state.store(STATE_PAUSED, Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.state.store(STATE_STOPPED, Ordering::Relaxed);
    }
    pub fn resume(&self) {
        self.state.store(STATE_RUNNING, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_CANCELLED
    }
    pub fn is_stopped(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_STOPPED
    }
    pub fn is_paused(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_PAUSED
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
    /// Per-item spawned tasks (download/process/upload/local-finalize)
    pub item_tasks: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
    /// Channel senders for graceful shutdown
    channel_tx: std::sync::Mutex<Option<(
        mpsc::Sender<PipelineItem>, // download_tx
    )>>,
}

impl PipelineRunner {
    pub fn new(
        config: PipelineConfig,
        db: MigrationDb,
        job_id: i64,
        workspace_dir: PathBuf,
        backup_dir: PathBuf,
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
            item_tasks: Arc::new(std::sync::Mutex::new(vec![])),
            channel_tx: std::sync::Mutex::new(None),
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

        // Save download_tx for graceful shutdown
        {
            let mut guard = self.channel_tx.lock().unwrap();
            *guard = Some((download_tx.clone(),));
        }

        let runner_clone = self.clone();
        let cancel = self.cancel_token.clone();

        // 1. Task Planner (quét DB và đưa item vào download queue)
        let planner_handle = tokio::spawn(async move {
            let _ = runner_clone.run_planner(download_tx).await;
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

    /// Theo dõi một task per-item được spawn bởi downloader/processor/uploader/local-finalizer
    pub fn track_item_task(&self, handle: JoinHandle<()>) {
        let mut guard = self.item_tasks.lock().unwrap();
        guard.push(handle);
    }

    /// Đợi tất cả các task chính kết thúc, sau đó đợi các item task, rồi kiểm tra completion
    pub async fn run_to_completion(&self) -> Result<(), String> {
        // 1. Drain active tasks (5 main loop tasks)
        let main_tasks: Vec<JoinHandle<()>> = {
            let mut guard = self.active_tasks.lock().unwrap();
            std::mem::take(&mut *guard)
        };

        for handle in main_tasks {
            let _ = handle.await;
        }

        // 2. Drain per-item spawned tasks
        let item_tasks: Vec<JoinHandle<()>> = {
            let mut guard = self.item_tasks.lock().unwrap();
            std::mem::take(&mut *guard)
        };

        for handle in item_tasks {
            let _ = handle.await;
        }

        // 3. Clean up channel senders to prevent leaks
        {
            let mut guard = self.channel_tx.lock().unwrap();
            *guard = None;
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
            "SELECT COUNT(*) FROM migration_items WHERE job_id = ? AND pipeline_stage NOT IN ('completed_telegram', 'completed_local', 'skipped_duplicate', 'failed', 'reconciliation_required') AND duplicate_of_item_id IS NULL;"
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

    /// Task Planner quét DB tìm tệp pending đưa vào download queue (Backpressure tự động qua bounded channel)
    async fn run_planner(&self, tx: mpsc::Sender<PipelineItem>) -> Result<(), String> {
        loop {
            if self.cancel_token.is_cancelled() || self.cancel_token.is_stopped() {
                // Break loop: planner stops, dropping tx channel which gracefully shuts down workers.
                break;
            }
            if self.cancel_token.is_paused() {
                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                continue;
            }

            // Quét DB lấy các tệp pending của job v2
            let items = {
                let conn = self.db.lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(
                    "SELECT id, job_id, name, source_path, source_item_id, size_bytes, source_etag, source_last_modified, source_fingerprint_type, source_fingerprint_value, state, original_sha256, processed_sha256, local_dest_path, telegram_random_id, video_decision, duplicate_of_item_id
                     FROM migration_items
                     WHERE job_id = ? AND pipeline_stage = 'discovered' AND duplicate_of_item_id IS NULL
                     LIMIT 10;"
                ).map_err(|e| e.to_string())?;
                stmt.bind((1, self.job_id)).map_err(|e| e.to_string())?;

                let mut res = vec![];
                while let Ok(sqlite::State::Row) = stmt.next() {
                    res.push(PipelineItem {
                        id: stmt.read(0).unwrap_or(0),
                        job_id: stmt.read(1).unwrap_or(0),
                        name: stmt.read(2).unwrap_or_default(),
                        source_path: stmt.read(3).unwrap_or_default(),
                        source_item_id: stmt.read(4).unwrap_or(None),
                        size_bytes: stmt.read(5).unwrap_or(0),
                        source_etag: stmt.read(6).unwrap_or(None),
                        source_last_modified: stmt.read(7).unwrap_or(None),
                        source_fingerprint_type: stmt.read(8).unwrap_or(None),
                        source_fingerprint_value: stmt.read(9).unwrap_or(None),
                        state: stmt.read(10).unwrap_or_else(|_| "pending".into()),
                        original_sha256: stmt.read(11).unwrap_or(None),
                        processed_sha256: stmt.read(12).unwrap_or(None),
                        local_dest_path: stmt.read(13).unwrap_or(None),
                        telegram_random_id: stmt.read(14).unwrap_or(None),
                        video_decision: stmt.read(15).unwrap_or(None),
                        duplicate_of_item_id: stmt.read(16).unwrap_or(None),
                    });
                }
                res
            };

            if items.is_empty() {
                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                continue;
            }

            println!(
                "Planner found {} items for job {}",
                items.len(),
                self.job_id
            );

            for item in items {
                // Đổi stage sang QueuedDownload trước khi gửi vào channel
                if let Err(e) =
                    update_item_pipeline_stage(&self.db, item.id, PipelineStage::QueuedDownload)
                {
                    println!("Planner failed to update stage for item {}: {}", item.id, e);
                    log::error!("Planner failed to update stage: {}", e);
                    continue;
                }

                println!("Planner successfully enqueued item {}", item.id);

                // Gửi vào channel (sẽ block ở đây nếu download queue bị đầy -> BACKPRESSURE)
                if tx.send(item).await.is_err() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Task Downloader tải tệp và định tuyến
    async fn run_downloader(
        &self,
        mut rx: mpsc::Receiver<PipelineItem>,
        process_tx: mpsc::Sender<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        local_tx: mpsc::Sender<PipelineItem>,
        downloader: Arc<dyn SourceDownloader>,
    ) -> Result<(), String> {
        let sem = Arc::new(Semaphore::new(self.config.download_concurrency));
        println!("Downloader loop started");

        while let Some(mut item) = rx.recv().await {
            println!(
                "Downloader received item: id={}, name={}",
                item.id, item.name
            );
            if self.cancel_token.is_cancelled() {
                break;
            }

            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;
            let db_clone = self.db.clone();
            let downloader_clone = downloader.clone();
            let process_tx_clone = process_tx.clone();
            let upload_tx_clone = upload_tx.clone();
            let local_tx_clone = local_tx.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();
            let item_tasks = self.item_tasks.clone(); // Shared vec for per-item tasks

            let handle = tokio::spawn(async move {
                let _permit = permit;
                if cancel.is_cancelled() {
                    return;
                }

                println!("Downloader spawn task active for item {}", item.id);

                // 1. Chuyển sang Downloading
                if let Err(e) =
                    update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Downloading)
                {
                    println!(
                        "Downloader failed to update stage to Downloading for item {}: {}",
                        item.id, e
                    );
                }

                // 2. Disk reservation & Quota check
                let res_id = format!("res_dl_{}", item.id);
                {
                    let conn = db_clone.lock().unwrap();
                    if let Err(e) = reserve_disk_space(
                        &conn,
                        &res_id,
                        item.job_id,
                        item.id,
                        "downloader",
                        item.size_bytes,
                        "download",
                        1800,
                    ) {
                        log::error!("Disk reserve failed: {}", e);
                        let _ =
                            update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Failed);
                        return;
                    }
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
                                    let conn = db_clone.lock().unwrap();
                                    let _ = release_disk_space(&conn, &res_id);
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

                                // 3.5. Dedupe check
                                if let Ok(true) =
                                    crate::migration::pipeline_v2::transitions::post_download_dedupe(
                                        &db_clone, item.id, &sha256,
                                    )
                                {
                                    println!("Downloader item {} deduped successfully!", item.id);
                                    let _ = std::fs::remove_file(&final_path);
                                    let conn = db_clone.lock().unwrap();
                                    let _ = release_disk_space(&conn, &res_id);
                                    return;
                                }

                                // 4. Giải phóng disk reservation
                                {
                                    let conn = db_clone.lock().unwrap();
                                    let _ = release_disk_space(&conn, &res_id);
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
                            // Release disk on download failure
                            {
                                let conn = db_clone.lock().unwrap();
                                let _ = release_disk_space(&conn, &res_id);
                            }
                        }
                    }
                } else {
                    println!("Downloader source_item_id is None for item {}", item.id);
                    let _ = update_item_pipeline_stage(&db_clone, item.id, PipelineStage::Failed);
                    // Release disk on missing source
                    {
                        let conn = db_clone.lock().unwrap();
                        let _ = release_disk_space(&conn, &res_id);
                    }
                }
            });
            // Track per-item task
            item_tasks.lock().unwrap().push(handle);
        }
        Ok(())
    }

    /// Task Processor phân tích và chuyển mã video (FFmpeg)
    async fn run_processor(
        &self,
        mut rx: mpsc::Receiver<PipelineItem>,
        upload_tx: mpsc::Sender<PipelineItem>,
        inspector: Arc<dyn MediaInspector>,
        processor: Arc<dyn VideoProcessor>,
    ) -> Result<(), String> {
        let sem = Arc::new(Semaphore::new(self.config.processing_concurrency));

        while let Some(mut item) = rx.recv().await {
            if self.cancel_token.is_cancelled() {
                break;
            }

            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;
            let db_clone = self.db.clone();
            let inspector_clone = inspector.clone();
            let processor_clone = processor.clone();
            let upload_tx_clone = upload_tx.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();
            let item_tasks = self.item_tasks.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                if cancel.is_cancelled() {
                    return;
                }

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
            });
            item_tasks.lock().unwrap().push(handle);
        }
        Ok(())
    }

    /// Task Uploader upload tệp tin lên Telegram
    async fn run_uploader(
        &self,
        mut rx: mpsc::Receiver<PipelineItem>,
        uploader: Arc<dyn TelegramUploader>,
    ) -> Result<(), String> {
        let sem = Arc::new(Semaphore::new(self.config.upload_concurrency));

        while let Some(item) = rx.recv().await {
            println!("Uploader loop received item {}", item.id);
            if self.cancel_token.is_cancelled() {
                break;
            }

            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;
            let db_clone = self.db.clone();
            let uploader_clone = uploader.clone();
            let workspace = self.workspace_dir.clone();
            let cancel = self.cancel_token.clone();
            let item_tasks = self.item_tasks.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                println!("Uploader spawned task active for item {}", item.id);
                if cancel.is_cancelled() {
                    return;
                }

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
                let date_string = Utc::now().format("%Y-%m-%d").to_string();
                {
                    let conn = db_clone.lock().unwrap();
                    if let Err(e) = reserve_quota(
                        &conn,
                        item.id,
                        item.job_id,
                        &date_string,
                        artifact_size,
                        7200, // 2 hour expiry
                    ) {
                        log::warn!(
                            "Upload: quota reserve failed for item {}: {} — moving to waiting_for_quota",
                            item.id, e
                        );
                        // conn goes out of scope here, dropping the lock
                        drop(conn);
                        let _ = update_item_pipeline_stage(
                            &db_clone,
                            item.id,
                            PipelineStage::WaitingForQuota,
                        );
                        return;
                    }
                }

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
                                        "UPDATE migration_items SET artifact_size_bytes = ? WHERE id = ?;",
                                    )
                                    .unwrap();
                                upd.bind((1, artifact_size)).unwrap();
                                upd.bind((2, item.id)).unwrap();
                                upd.next().unwrap();
                            }

                            // Commit quota
                            {
                                let conn = db_clone.lock().unwrap();
                                let _ = commit_quota(&conn, item.id);
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
                                let _ = commit_quota(&conn, item.id);
                            }

                            // Update item stage
                            {
                                let conn = db_clone.lock().unwrap();
                                let mut upd = conn
                                    .prepare(
                                        "UPDATE migration_items SET telegram_random_id = ?, last_error_message = ? WHERE id = ?;",
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
                                        "INSERT OR REPLACE INTO migration_pacing_state (key, next_allowed_at, updated_at) VALUES ('next_allowed_at', ?, ?);",
                                    )
                                    .unwrap();
                                upd.bind((1, next_allowed)).unwrap();
                                upd.bind((2, now)).unwrap();
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
                            let _ =
                                crate::migration::pipeline_v2::transitions::promote_canonical(
                                    &db_clone, item.id,
                                );
                        }
                    }
                }
            });
            item_tasks.lock().unwrap().push(handle);
        }
        Ok(())
    }

    /// Task Local Finalizer lưu tệp Other vào backup_dir
    async fn run_local_finalizer(
        &self,
        mut rx: mpsc::Receiver<PipelineItem>,
        finalizer: Arc<dyn LocalFinalizer>,
    ) -> Result<(), String> {
        let sem = Arc::new(Semaphore::new(self.config.local_finalizer_concurrency));

        while let Some(item) = rx.recv().await {
            if self.cancel_token.is_cancelled() {
                break;
            }

            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;
            let db_clone = self.db.clone();
            let finalizer_clone = finalizer.clone();
            let workspace = self.workspace_dir.clone();
            let backup = self.backup_dir.clone();
            let cancel = self.cancel_token.clone();
            let item_tasks = self.item_tasks.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                if cancel.is_cancelled() {
                    return;
                }

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
                        // Cập nhật local path
                        {
                            let conn = db_clone.lock().unwrap();
                            let mut upd = conn
                                .prepare(
                                    "UPDATE migration_items SET local_dest_path = ? WHERE id = ?;",
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
            });
            item_tasks.lock().unwrap().push(handle);
        }
        Ok(())
    }
}
