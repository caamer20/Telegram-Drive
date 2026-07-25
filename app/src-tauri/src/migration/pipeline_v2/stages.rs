use std::future::Future;
use std::path::Path;
use std::pin::Pin;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineStage {
    Discovered,
    DedupeCheck,
    WaitingForCanonical,
    QueuedDownload,
    Downloading,
    Downloaded,
    QueuedProcessing,
    Processing,
    QueuedUpload,
    Uploading,
    SavingLocal,
    CompletedTelegram,
    CompletedLocal,
    SkippedDuplicate,
    RetryWait,
    ReconciliationRequired,
    Failed,
}

impl PipelineStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::DedupeCheck => "dedupe_check",
            Self::WaitingForCanonical => "waiting_for_canonical",
            Self::QueuedDownload => "queued_download",
            Self::Downloading => "downloading",
            Self::Downloaded => "downloaded",
            Self::QueuedProcessing => "queued_processing",
            Self::Processing => "processing",
            Self::QueuedUpload => "queued_upload",
            Self::Uploading => "uploading",
            Self::SavingLocal => "saving_local",
            Self::CompletedTelegram => "completed_telegram",
            Self::CompletedLocal => "completed_local",
            Self::SkippedDuplicate => "skipped_duplicate",
            Self::RetryWait => "retry_wait",
            Self::ReconciliationRequired => "reconciliation_required",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "discovered" => Self::Discovered,
            "dedupe_check" => Self::DedupeCheck,
            "waiting_for_canonical" => Self::WaitingForCanonical,
            "queued_download" => Self::QueuedDownload,
            "downloading" => Self::Downloading,
            "downloaded" => Self::Downloaded,
            "queued_processing" => Self::QueuedProcessing,
            "processing" => Self::Processing,
            "queued_upload" => Self::QueuedUpload,
            "uploading" => Self::Uploading,
            "saving_local" => Self::SavingLocal,
            "completed_telegram" => Self::CompletedTelegram,
            "completed_local" => Self::CompletedLocal,
            "skipped_duplicate" => Self::SkippedDuplicate,
            "retry_wait" => Self::RetryWait,
            "reconciliation_required" => Self::ReconciliationRequired,
            _ => Self::Failed,
        }
    }
}

// Struct represent item info flowing through pipeline channel
#[derive(Debug, Clone)]
pub struct PipelineItem {
    pub id: i64,
    pub job_id: i64,
    pub name: String,
    pub source_path: String,
    pub source_item_id: Option<String>,
    pub size_bytes: i64,
    pub source_etag: Option<String>,
    pub source_last_modified: Option<String>,
    pub source_fingerprint_type: Option<String>,
    pub source_fingerprint_value: Option<String>,
    pub state: String,
    pub original_sha256: Option<String>,
    pub processed_sha256: Option<String>,
    pub local_dest_path: Option<String>,
    pub telegram_random_id: Option<String>,
    pub video_decision: Option<String>,
    pub duplicate_of_item_id: Option<i64>,
}

// Media metadata return from ffprobe
#[derive(Debug, Clone, Default)]
pub struct VideoMetadata {
    pub container: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub bitrate: u64,
    pub is_valid: bool,
    pub rotation: i32,
    pub file_size: u64,
}

// Decoupling dependency traits
pub trait SourceDownloader: Send + Sync {
    fn download_file(
        &self,
        item_id: i64,
        source_item_id: &str,
        dest_path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
}

pub trait MediaInspector: Send + Sync {
    fn inspect_file(
        &self,
        path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<VideoMetadata, String>> + Send>>;
}

pub trait VideoProcessor: Send + Sync {
    fn process_video(
        &self,
        input_path: &Path,
        output_path: &Path,
        decision: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
}

pub trait TelegramUploader: Send + Sync {
    fn upload_file(
        &self,
        path: &Path,
        random_id: i64,
        filename: &str,
    ) -> Pin<Box<dyn Future<Output = Result<i64, String>> + Send>>;
}

pub trait LocalFinalizer: Send + Sync {
    fn finalize_local(
        &self,
        source_path: &Path,
        dest_path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
}
