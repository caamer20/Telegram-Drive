use crate::migration::db::open_migration_db_at_path;
use crate::migration::disk_reserve::get_total_reserved_disk_space;
use crate::migration::pipeline_v2::config::PipelineConfig;
use crate::migration::pipeline_v2::runner::PipelineRunner;
use crate::migration::pipeline_v2::stages::{
    LocalFinalizer, MediaInspector, SourceDownloader, TelegramUploader, VideoMetadata,
    VideoProcessor,
};

use std::env;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

// Setup Temp Directory Helper
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let path = env::temp_dir().join(format!("temp_pipeline_tests_{}", rand::random::<u64>()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// Fake Adapter Implementations
struct FakeDownloader {
    call_count: Arc<AtomicUsize>,
    active_downloads: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

impl SourceDownloader for FakeDownloader {
    fn download_file(
        &self,
        _item_id: i64,
        source_item_id: &str,
        dest_path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let active = self.active_downloads.fetch_add(1, Ordering::Relaxed) + 1;

        let max_a = self.max_active.clone();
        let active_d = self.active_downloads.clone();
        let path = dest_path.to_path_buf();
        let source_id = source_item_id.to_string();

        Box::pin(async move {
            loop {
                let current_max = max_a.load(Ordering::Relaxed);
                if active > current_max {
                    max_a.store(active, Ordering::Relaxed);
                }
                break;
            }
            // Ghi file download giả lập
            let _ = fs::write(&path, b"fake_downloaded_bytes");
            active_d.fetch_sub(1, Ordering::Relaxed);
            
            if source_id.starts_with("hash:") {
                Ok(source_id.strip_prefix("hash:").unwrap().to_string())
            } else {
                Ok("fake_sha256".to_string())
            }
        })
    }
}

struct FakeInspector {
    video_codec: String,
}

impl MediaInspector for FakeInspector {
    fn inspect_file(
        &self,
        _path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<VideoMetadata, String>> + Send>> {
        let codec = self.video_codec.clone();
        Box::pin(async move {
            Ok(VideoMetadata {
                container: "mp4".to_string(),
                video_codec: codec,
                audio_codec: "aac".to_string(),
                duration: 120.0,
                width: 1920,
                height: 1080,
                bitrate: 5_000_000,
                is_valid: true,
                rotation: 0,
                file_size: 1000,
            })
        })
    }
}

struct FakeProcessor {
    call_count: Arc<AtomicUsize>,
    active_processors: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

impl VideoProcessor for FakeProcessor {
    fn process_video(
        &self,
        _input_path: &Path,
        output_path: &Path,
        _decision: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let active = self.active_processors.fetch_add(1, Ordering::Relaxed) + 1;

        let max_a = self.max_active.clone();
        let active_p = self.active_processors.clone();
        let path = output_path.to_path_buf();

        Box::pin(async move {
            loop {
                let current_max = max_a.load(Ordering::Relaxed);
                if active > current_max {
                    max_a.store(active, Ordering::Relaxed);
                }
                break;
            }
            let _ = fs::write(&path, b"fake_processed_bytes");
            active_p.fetch_sub(1, Ordering::Relaxed);
            Ok("fake_processed_sha256".to_string())
        })
    }
}

struct FakeUploader {
    call_count: Arc<AtomicUsize>,
    active_uploads: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    delay: Duration,
    received_bytes: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl TelegramUploader for FakeUploader {
    fn upload_file(
        &self,
        path: &Path,
        _random_id: i64,
        _filename: &str,
    ) -> Pin<Box<dyn Future<Output = Result<i64, String>> + Send>> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let active = self.active_uploads.fetch_add(1, Ordering::Relaxed) + 1;

        let max_a = self.max_active.clone();
        let active_u = self.active_uploads.clone();
        let delay = self.delay;
        let file_path = path.to_path_buf();
        let bytes_received = self.received_bytes.clone();

        Box::pin(async move {
            loop {
                let current_max = max_a.load(Ordering::Relaxed);
                if active > current_max {
                    max_a.store(active, Ordering::Relaxed);
                }
                break;
            }
            if let Ok(data) = fs::read(&file_path) {
                let mut guard = bytes_received.lock().unwrap();
                *guard = data;
            }
            if delay.as_millis() > 0 {
                tokio::time::sleep(delay).await;
            }
            active_u.fetch_sub(1, Ordering::Relaxed);
            Ok(9999_i64) // Return fake message ID
        })
    }
}

struct FakeLocalFinalizer {
    call_count: Arc<AtomicUsize>,
    active_finalizers: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

impl LocalFinalizer for FakeLocalFinalizer {
    fn finalize_local(
        &self,
        source_path: &Path,
        dest_path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let active = self.active_finalizers.fetch_add(1, Ordering::Relaxed) + 1;

        let max_a = self.max_active.clone();
        let active_f = self.active_finalizers.clone();
        let src = source_path.to_path_buf();
        let dst = dest_path.to_path_buf();

        Box::pin(async move {
            loop {
                let current_max = max_a.load(Ordering::Relaxed);
                if active > current_max {
                    max_a.store(active, Ordering::Relaxed);
                }
                break;
            }
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::copy(&src, &dst);
            active_f.fetch_sub(1, Ordering::Relaxed);
            Ok(())
        })
    }
}

// 1. Integration Test: Pipeline Concurrency & Overlap
#[tokio::test]
async fn test_pipeline_overlap_and_concurrency_limits() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_overlap.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    // Seed 3 videos có source_item_id đầy đủ
    for i in 1..=3 {
        conn.execute(format!(
            "INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
             VALUES ({}, 1, 'video_{}.mp4', 'video_{}.mp4', 'source_{}', 'pending', 'discovered', 0);",
            i, i, i, i
        )).unwrap();
    }
    drop(conn);

    let downloader_calls = Arc::new(AtomicUsize::new(0));
    let active_downloads = Arc::new(AtomicUsize::new(0));
    let max_downloads = Arc::new(AtomicUsize::new(0));

    let downloader = Arc::new(FakeDownloader {
        call_count: downloader_calls.clone(),
        active_downloads: active_downloads.clone(),
        max_active: max_downloads.clone(),
    });

    let inspector = Arc::new(FakeInspector {
        video_codec: "h264".to_string(), // passthrough
    });

    let processor_calls = Arc::new(AtomicUsize::new(0));
    let active_processors = Arc::new(AtomicUsize::new(0));
    let max_processors = Arc::new(AtomicUsize::new(0));

    let processor = Arc::new(FakeProcessor {
        call_count: processor_calls.clone(),
        active_processors: active_processors.clone(),
        max_active: max_processors.clone(),
    });

    let uploader_calls = Arc::new(AtomicUsize::new(0));
    let active_uploads = Arc::new(AtomicUsize::new(0));
    let max_uploads = Arc::new(AtomicUsize::new(0));

    let uploader = Arc::new(FakeUploader {
        call_count: uploader_calls.clone(),
        active_uploads: active_uploads.clone(),
        max_active: max_uploads.clone(),
        delay: Duration::from_millis(50),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });

    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, uploader, finalizer);

    // Chờ tệp tin được xử lý
    tokio::time::sleep(Duration::from_millis(600)).await;

    // Verify downloads
    assert_eq!(downloader_calls.load(Ordering::Relaxed), 3);
    assert_eq!(uploader_calls.load(Ordering::Relaxed), 3);

    // Concurrency limits checked
    assert!(max_downloads.load(Ordering::Relaxed) <= 2);
    assert!(max_processors.load(Ordering::Relaxed) <= 1);
    assert!(max_uploads.load(Ordering::Relaxed) <= 1);
}

// 2. Integration Test: Backpressure
#[tokio::test]
async fn test_pipeline_backpressure() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_backpressure.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();

    // Seed 15 tệp tin có source_item_id đầy đủ
    for i in 1..=15 {
        conn.execute(format!(
            "INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
             VALUES ({}, 1, 'video_{}.mp4', 'video_{}.mp4', 'source_{}', 'pending', 'discovered', 0);",
            i, i, i, i
        )).unwrap();
    }
    drop(conn);

    let downloader_calls = Arc::new(AtomicUsize::new(0));
    let downloader = Arc::new(FakeDownloader {
        call_count: downloader_calls.clone(),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let inspector = Arc::new(FakeInspector {
        video_codec: "h265".to_string(), // transcode
    });

    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    // Uploader phản hồi cực chậm (1 giây)
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_secs(1),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });

    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let mut config = PipelineConfig::default();
    config.download_concurrency = 1;
    config.download_queue_capacity = 1;
    config.processing_concurrency = 1;
    config.processing_queue_capacity = 1;
    config.upload_concurrency = 1;
    config.upload_queue_capacity = 1;

    let runner = Arc::new(PipelineRunner::new(
        config,
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, uploader, finalizer);

    // Chờ 200ms
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Planner và Downloader sẽ bị chặn do upload queue đầy (Backpressure hoạt động)
    // Với queue capacities và concurrency = 1, tối đa 8 items được download
    assert!(downloader_calls.load(Ordering::Relaxed) <= 8);
}

// 3. Integration Test: Image unchanged
#[tokio::test]
async fn test_pipeline_image_route() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_image.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at) VALUES (1, 1, 'photo.jpg', 'photo.jpg', 'source_1', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let inspector = Arc::new(FakeInspector {
        video_codec: "h264".to_string(),
    });

    let processor_calls = Arc::new(AtomicUsize::new(0));
    let processor = Arc::new(FakeProcessor {
        call_count: processor_calls.clone(),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let uploaded_bytes = Arc::new(std::sync::Mutex::new(vec![]));
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(10),
        received_bytes: uploaded_bytes.clone(),
    });

    let finalizer_calls = Arc::new(AtomicUsize::new(0));
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: finalizer_calls.clone(),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, uploader, finalizer);

    tokio::time::sleep(Duration::from_millis(400)).await;

    // Verify
    assert_eq!(processor_calls.load(Ordering::Relaxed), 0); // Không chạy video processing
    assert_eq!(finalizer_calls.load(Ordering::Relaxed), 0); // Không chạy local finalizer

    // Nhận đúng byte (SHA-256 không đổi)
    let bytes = uploaded_bytes.lock().unwrap();
    assert_eq!(*bytes, b"fake_downloaded_bytes");
}

// 4. Integration Test: Other file route
#[tokio::test]
async fn test_pipeline_other_file_route() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_other.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at) VALUES (1, 1, 'doc.zip', 'documents/doc.zip', 'source_1', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let inspector = Arc::new(FakeInspector {
        video_codec: "h264".to_string(),
    });

    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let uploader_calls = Arc::new(AtomicUsize::new(0));
    let uploader = Arc::new(FakeUploader {
        call_count: uploader_calls.clone(),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(10),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });

    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, uploader, finalizer);

    tokio::time::sleep(Duration::from_millis(400)).await;

    // Verify
    assert_eq!(uploader_calls.load(Ordering::Relaxed), 0); // Không upload telegram

    let backup_file = tmp
        .path()
        .join("backup")
        .join("OneDrive_Archive")
        .join("documents/doc.zip");
    assert!(backup_file.exists());
}

// 5. Integration Test: Pause/Resume
#[tokio::test]
async fn test_pipeline_pause_resume() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_pause.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at) VALUES (1, 1, 'file1.mp4', 'file1.mp4', 'source_1', 'pending', 'discovered', 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at) VALUES (2, 1, 'file2.mp4', 'file2.mp4', 'source_2', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let inspector = Arc::new(FakeInspector {
        video_codec: "h264".to_string(),
    });

    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(50),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });

    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    // Pause ngay lập tức
    let cancel = runner.clone().start(
        downloader.clone(),
        inspector.clone(),
        processor.clone(),
        uploader.clone(),
        finalizer.clone(),
    );
    cancel.cancel(); // Kích hoạt pause

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sau khi cancel, không tệp nào được claim mới thêm
    let conn = db.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT COUNT(*) FROM migration_items WHERE pipeline_stage = 'discovered';")
        .unwrap();
    assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
    let count: i64 = stmt.read(0).unwrap();
    assert!(count > 0);
}

// 6. Integration Test: Video Decision Routes (Passthrough vs Transcode)
#[tokio::test]
async fn test_video_decision_routes() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_decision.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at) VALUES (1, 1, 'video_h264.mp4', 'v1.mp4', 'src_1', 'pending', 'discovered', 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at) VALUES (2, 1, 'video_hevc.mp4', 'v2.mp4', 'src_2', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    // Inspector trả về hevc cho tệp 2, h264 cho tệp 1
    struct MixedInspector;
    impl MediaInspector for MixedInspector {
        fn inspect_file(
            &self,
            path: &Path,
        ) -> Pin<Box<dyn Future<Output = Result<VideoMetadata, String>> + Send>> {
            let is_hevc = path.file_name().and_then(|n| n.to_str()) == Some("2"); // item ID 2
            Box::pin(async move {
                Ok(VideoMetadata {
                    container: "mp4".to_string(),
                    video_codec: if is_hevc {
                        "hevc".to_string()
                    } else {
                        "h264".to_string()
                    },
                    audio_codec: "aac".to_string(),
                    duration: 60.0,
                    width: 1280,
                    height: 720,
                    bitrate: 1000,
                    is_valid: true,
                    rotation: 0,
                    file_size: 100,
                })
            })
        }
    }

    let processor_calls = Arc::new(AtomicUsize::new(0));
    let processor = Arc::new(FakeProcessor {
        call_count: processor_calls.clone(),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(1),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });

    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(
        downloader,
        Arc::new(MixedInspector),
        processor,
        uploader,
        finalizer,
    );

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Tệp 1 (h264) passthrough -> không chạy process.
    // Tệp 2 (hevc) transcode -> chạy process 1 lần.
    assert_eq!(processor_calls.load(Ordering::Relaxed), 1);
}

// 7. Integration Test: Same-snapshot duplicate deduplication
#[tokio::test]
async fn test_pipeline_dedupe_duplicate() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_dedupe.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();

    // Seed 1 canonical
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'canonical.jpg', 'canonical.jpg', 'src_1', 'pending', 'discovered', 0);").unwrap();
    // Seed 1 duplicate (trỏ duplicate_of_item_id = 1)
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, duplicate_of_item_id, created_at)
                  VALUES (2, 1, 'dup.jpg', 'dup.jpg', 'src_2', 'pending', 'discovered', 1, 0);").unwrap();
    drop(conn);

    let downloader_calls = Arc::new(AtomicUsize::new(0));
    let downloader = Arc::new(FakeDownloader {
        call_count: downloader_calls.clone(),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let inspector = Arc::new(FakeInspector {
        video_codec: "h264".to_string(),
    });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(1),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, uploader, finalizer);

    tokio::time::sleep(Duration::from_millis(400)).await;

    // Chỉ có canonical được download/upload
    assert_eq!(downloader_calls.load(Ordering::Relaxed), 1);
}

// 8. Integration Test: Disk Reservation Boundary
#[tokio::test]
async fn test_disk_reservation_pipeline() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_disk_res.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, size_bytes, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'file.mp4', 'file.mp4', 'src_1', 5_000_000, 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let inspector = Arc::new(FakeInspector {
        video_codec: "h264".to_string(),
    });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(1),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, uploader, finalizer);

    tokio::time::sleep(Duration::from_millis(400)).await;

    // Tổng dung lượng disk reservation sau khi kết thúc thành công là 0
    let conn = db.lock().unwrap();
    let total = get_total_reserved_disk_space(&conn, 1).unwrap();
    assert_eq!(total, 0);
}

// 9. Integration Test: Stop is not resume (Stop retains state, but halts pipeline)
#[tokio::test]
async fn test_pipeline_stop_is_not_resume() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_stop.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at) VALUES (1, 1, 'file1.mp4', 'file1.mp4', 'source_1', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let inspector = Arc::new(FakeInspector { video_codec: "h264".to_string() });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(50),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let cancel = runner.start(downloader.clone(), inspector, processor, uploader, finalizer);
    
    // Stop pipeline right away
    cancel.stop();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify
    assert_eq!(downloader.call_count.load(Ordering::Relaxed), 0);
}

// 10. Integration Test: Dedupe after download
#[tokio::test]
async fn test_pipeline_dedupes_equal_content_after_download() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_dedupe_dl.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    // 1 completed canonical item with original_sha256 = 'fake_sha256'
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, original_sha256, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'canonical.jpg', 'canonical.jpg', 'src_1', 'fake_sha256', 'completed', 'completed_telegram', 0);").unwrap();
    // 1 discovered item
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (2, 1, 'new.jpg', 'new.jpg', 'src_2', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let inspector = Arc::new(FakeInspector { video_codec: "h264".to_string() });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let uploader_calls = Arc::new(AtomicUsize::new(0));
    let uploader = Arc::new(FakeUploader {
        call_count: uploader_calls.clone(),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(10),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader.clone(), inspector, processor, uploader, finalizer);
    
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert_eq!(downloader.call_count.load(Ordering::Relaxed), 1);
    assert_eq!(uploader_calls.load(Ordering::Relaxed), 0); // Upload was skipped!

    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT pipeline_stage, duplicate_of_item_id FROM migration_items WHERE id = 2;").unwrap();
    assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
    assert_eq!(stmt.read::<String, _>(0).unwrap(), "skipped_duplicate");
    assert_eq!(stmt.read::<i64, _>(1).unwrap(), 1);
}

// 11. Test: Canonical Permanent Failure Promotion
#[tokio::test]
async fn test_canonical_permanent_failure_promotion() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_promo.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    // Canonical
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'canonical.mp4', 'canonical.mp4', 'src_1', 'pending', 'discovered', 0);").unwrap();
    // Duplicate waiting for canonical
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, duplicate_of_item_id, created_at)
                  VALUES (2, 1, 'dup.mp4', 'dup.mp4', 'src_2', 'pending', 'waiting_for_canonical', 1, 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let inspector = Arc::new(FakeInspector { video_codec: "h264".to_string() });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    // Uploader returns permanent_error
    struct FailingUploader;
    impl TelegramUploader for FailingUploader {
        fn upload_file(
            &self,
            _path: &Path,
            _random_id: i64,
            _filename: &str,
        ) -> Pin<Box<dyn Future<Output = Result<i64, String>> + Send>> {
            Box::pin(async move {
                Err("permanent_error".to_string())
            })
        }
    }

    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, Arc::new(FailingUploader), finalizer);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let conn = db.lock().unwrap();
    // Item 1 must be failed
    let mut stmt1 = conn.prepare("SELECT pipeline_stage FROM migration_items WHERE id = 1;").unwrap();
    assert_eq!(stmt1.next().unwrap(), sqlite::State::Row);
    assert_eq!(stmt1.read::<String, _>(0).unwrap(), "failed");

    // Item 2 must be promoted (its duplicate_of_item_id becomes NULL, stage queued_download or pending)
    let mut stmt2 = conn.prepare("SELECT pipeline_stage, duplicate_of_item_id FROM migration_items WHERE id = 2;").unwrap();
    assert_eq!(stmt2.next().unwrap(), sqlite::State::Row);
    assert_eq!(stmt2.read::<String, _>(0).unwrap(), "queued_download");
    assert!(stmt2.read::<Option<i64>, _>(1).unwrap().is_none());
}

// 12. Test: Retryable Canonical Failure No Promotion
#[tokio::test]
async fn test_retryable_canonical_failure_no_promotion() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_retry_promo.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    // Canonical
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'canonical.mp4', 'canonical.mp4', 'src_1', 'pending', 'discovered', 0);").unwrap();
    // Duplicate waiting for canonical
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, duplicate_of_item_id, created_at)
                  VALUES (2, 1, 'dup.mp4', 'dup.mp4', 'src_2', 'pending', 'waiting_for_canonical', 1, 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let inspector = Arc::new(FakeInspector { video_codec: "h264".to_string() });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    // Uploader returns retryable_error (not permanent_error)
    struct FailingUploader;
    impl TelegramUploader for FailingUploader {
        fn upload_file(
            &self,
            _path: &Path,
            _random_id: i64,
            _filename: &str,
        ) -> Pin<Box<dyn Future<Output = Result<i64, String>> + Send>> {
            Box::pin(async move {
                Err("retryable_error".to_string())
            })
        }
    }

    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, Arc::new(FailingUploader), finalizer);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let conn = db.lock().unwrap();
    // Item 1 must be failed
    let mut stmt1 = conn.prepare("SELECT pipeline_stage FROM migration_items WHERE id = 1;").unwrap();
    assert_eq!(stmt1.next().unwrap(), sqlite::State::Row);
    assert_eq!(stmt1.read::<String, _>(0).unwrap(), "failed");

    // Item 2 must NOT be promoted (still duplicate_of_item_id = 1, pipeline_stage = waiting_for_canonical)
    let mut stmt2 = conn.prepare("SELECT pipeline_stage, duplicate_of_item_id FROM migration_items WHERE id = 2;").unwrap();
    assert_eq!(stmt2.next().unwrap(), sqlite::State::Row);
    assert_eq!(stmt2.read::<String, _>(0).unwrap(), "waiting_for_canonical");
    assert_eq!(stmt2.read::<i64, _>(1).unwrap(), 1);
}

// 13. Test: Path Traversal, Reserved Names and Collision
#[tokio::test]
async fn test_path_traversal_reserved_names_and_collision() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_security.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    // Path traversal attempt in source_path
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'traversal.txt', '../../escaped.txt', 'hash:traversal', 'pending', 'discovered', 0);").unwrap();
    // Windows reserved name CON in source_path
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (2, 1, 'con_file.txt', 'CON/file.txt', 'hash:con', 'pending', 'discovered', 0);").unwrap();
    // Collision item (has source_path dir/collision.txt, but we write dir/collision.txt in backup first)
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (3, 1, 'collision.txt', 'dir/collision.txt', 'hash:collision', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    // Create the pre-existing file in the backup directory
    let collision_dir = tmp.path().join("backup").join("OneDrive_Archive").join("dir");
    std::fs::create_dir_all(&collision_dir).unwrap();
    std::fs::write(collision_dir.join("collision.txt"), b"existing file content").unwrap();

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let inspector = Arc::new(FakeInspector { video_codec: "h264".to_string() });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(1),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader, inspector, processor, uploader, finalizer);
    tokio::time::sleep(Duration::from_millis(600)).await;

    let conn = db.lock().unwrap();

    // Debug print stage of all items
    for id in 1..=3 {
        let mut debug_stmt = conn.prepare(format!("SELECT pipeline_stage, local_dest_path FROM migration_items WHERE id = {};", id)).unwrap();
        if debug_stmt.next().unwrap() == sqlite::State::Row {
            let stage: String = debug_stmt.read(0).unwrap();
            let path: Option<String> = debug_stmt.read(1).unwrap();
            println!("DEBUG: Item {} is in stage '{}', path={:?}", id, stage, path);
        }
    }

    // 1. Path traversal verification: parent component ".." ignored, so path is resolved under backup dir safely
    let mut stmt1 = conn.prepare("SELECT local_dest_path, pipeline_stage FROM migration_items WHERE id = 1;").unwrap();
    assert_eq!(stmt1.next().unwrap(), sqlite::State::Row);
    let path1: Option<String> = stmt1.read(0).unwrap();
    let stage1: String = stmt1.read(1).unwrap();
    assert_eq!(stage1, "completed_local");
    let path1_str = path1.expect("item 1 path should be set");
    assert!(path1_str.contains("OneDrive_Archive"));
    assert!(path1_str.contains("escaped.txt"));
    assert!(!path1_str.contains(".."));

    // 2. Windows reserved name verification: CON sanitized to CON_safe
    let mut stmt2 = conn.prepare("SELECT local_dest_path, pipeline_stage FROM migration_items WHERE id = 2;").unwrap();
    assert_eq!(stmt2.next().unwrap(), sqlite::State::Row);
    let path2: Option<String> = stmt2.read(0).unwrap();
    let stage2: String = stmt2.read(1).unwrap();
    assert_eq!(stage2, "completed_local");
    let path2_str = path2.expect("item 2 path should be set");
    assert!(path2_str.contains("CON_safe"));

    // 3. Collision resolution verification: suffix added
    let mut stmt3 = conn.prepare("SELECT local_dest_path, pipeline_stage FROM migration_items WHERE id = 3;").unwrap();
    assert_eq!(stmt3.next().unwrap(), sqlite::State::Row);
    let path3: Option<String> = stmt3.read(0).unwrap();
    let stage3: String = stmt3.read(1).unwrap();
    assert_eq!(stage3, "completed_local");
    let path3_str = path3.expect("item 3 path should be set");
    assert!(path3_str.contains("collision_1.txt"));
}

// 14. Test: Terminal Item Not Replayed
#[tokio::test]
async fn test_terminal_item_no_replay() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_terminal_replay.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    // 1 completed item, 1 failed item
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'done.txt', 'done.txt', 'src_1', 'completed', 'completed_telegram', 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (2, 1, 'fail.txt', 'fail.txt', 'src_2', 'failed', 'failed', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let inspector = Arc::new(FakeInspector { video_codec: "h264".to_string() });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(1),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    let _cancel = runner.start(downloader.clone(), inspector, processor, uploader, finalizer);
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Neither of them should be processed again
    assert_eq!(downloader.call_count.load(Ordering::Relaxed), 0);
}

// 15. Test: Pipeline Pause and Resume to Completion
#[tokio::test]
async fn test_pipeline_pause_resume_to_completion() {
    let tmp = TempDir::new();
    let db_path = tmp.path().join("test_pause_resume.db");
    let db = open_migration_db_at_path(db_path).unwrap();

    let conn = db.lock().unwrap();
    conn.execute("INSERT INTO migration_jobs (id, state, pipeline_version, created_at, updated_at) VALUES (1, 'running', 2, 0, 0);").unwrap();
    conn.execute("INSERT INTO migration_items (id, job_id, name, source_path, source_item_id, state, pipeline_stage, created_at)
                  VALUES (1, 1, 'item.txt', 'item.txt', 'src_1', 'pending', 'discovered', 0);").unwrap();
    drop(conn);

    let downloader = Arc::new(FakeDownloader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_downloads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let inspector = Arc::new(FakeInspector { video_codec: "h264".to_string() });
    let processor = Arc::new(FakeProcessor {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_processors: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });
    let uploader = Arc::new(FakeUploader {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_uploads: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
        delay: Duration::from_millis(1),
        received_bytes: Arc::new(std::sync::Mutex::new(vec![])),
    });
    let finalizer = Arc::new(FakeLocalFinalizer {
        call_count: Arc::new(AtomicUsize::new(0)),
        active_finalizers: Arc::new(AtomicUsize::new(0)),
        max_active: Arc::new(AtomicUsize::new(0)),
    });

    let runner = Arc::new(PipelineRunner::new(
        PipelineConfig::default(),
        db.clone(),
        1,
        tmp.path().join("workspace"),
        tmp.path().join("backup"),
    ));

    // Pause immediately
    runner.cancel_token.pause();

    let cancel = runner.clone().start(
        downloader.clone(),
        inspector,
        processor,
        uploader,
        finalizer,
    );

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify it was paused and downloader call count is 0
    assert_eq!(downloader.call_count.load(Ordering::Relaxed), 0);

    // Resume the pipeline
    cancel.resume();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Downloader must have run and item completed local (since category is Other)
    assert_eq!(downloader.call_count.load(Ordering::Relaxed), 1);

    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT pipeline_stage FROM migration_items WHERE id = 1;").unwrap();
    assert_eq!(stmt.next().unwrap(), sqlite::State::Row);
    assert_eq!(stmt.read::<String, _>(0).unwrap(), "completed_local");
}

