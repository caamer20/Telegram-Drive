use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineStage {
    Discovered,
    QueuedDownload,
    Downloading,
    Downloaded,
    QueuedProcessing,
    Processing,
    Processed,
    QueuedUpload,
    Uploading,
    WaitingForQuota,
    SavingLocal,
    CompletedTelegram,
    CompletedLocal,
    ReconciliationRequired,
    Failed,
}

impl PipelineStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::QueuedDownload => "queued_download",
            Self::Downloading => "downloading",
            Self::Downloaded => "downloaded",
            Self::QueuedProcessing => "queued_processing",
            Self::Processing => "processing",
            Self::Processed => "processed",
            Self::QueuedUpload => "queued_upload",
            Self::Uploading => "uploading",
            Self::WaitingForQuota => "waiting_for_quota",
            Self::SavingLocal => "saving_local",
            Self::CompletedTelegram => "completed_telegram",
            Self::CompletedLocal => "completed_local",
            Self::ReconciliationRequired => "reconciliation_required",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "discovered" => Self::Discovered,
            "queued_download" => Self::QueuedDownload,
            "downloading" => Self::Downloading,
            "downloaded" => Self::Downloaded,
            "queued_processing" => Self::QueuedProcessing,
            "processing" => Self::Processing,
            "processed" => Self::Processed,
            "queued_upload" => Self::QueuedUpload,
            "uploading" => Self::Uploading,
            "waiting_for_quota" => Self::WaitingForQuota,
            "saving_local" => Self::SavingLocal,
            "completed_telegram" => Self::CompletedTelegram,
            "completed_local" => Self::CompletedLocal,
            "reconciliation_required" => Self::ReconciliationRequired,
            "failed" => Self::Failed,
            _ => Self::Failed,
        }
    }

    /// Trả về true nếu stage là terminal (không thể chuyển tiếp)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::CompletedTelegram
                | Self::CompletedLocal
                | Self::ReconciliationRequired
                | Self::Failed
        )
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
    pub local_artifact_path: Option<String>,
    pub processed_artifact_path: Option<String>,
    pub telegram_random_id: Option<i64>,
    pub video_decision: Option<String>,
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
    pub color_transfer: String,
    pub color_primaries: String,
    pub profile: String,
    pub pixel_format: String,
    pub fps: f64,
}

// Decoupling dependency traits

/// Loại media để adapter biết cách gửi lên Telegram
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramMediaKind {
    Video,
    Image,
    Other,
}

/// Request typed cho Telegram upload — không hardcode item ID, không bỏ qua random_id
#[derive(Debug, Clone)]
pub struct TelegramUploadRequest {
    pub job_id: i64,
    pub item_id: i64,
    pub path: PathBuf,
    pub filename: String,
    pub random_id: i64,
    pub destination_id: Option<i64>,
    pub media_kind: TelegramMediaKind,
}

/// Kết quả typed từ Telegram upload
#[derive(Debug, Clone)]
pub enum TelegramUploadResult {
    Confirmed { message_id: i64, random_id: i64 },
    ReconciliationRequired { random_id: i64, reason: String },
}

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
        item_id: i64,
        job_id: i64,
        duration: f64,
        source_fps: f64,
        item_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
}

pub trait TelegramUploader: Send + Sync {
    fn upload_file(
        &self,
        request: TelegramUploadRequest,
    ) -> Pin<Box<dyn Future<Output = Result<TelegramUploadResult, String>> + Send>>;
}

pub trait LocalFinalizer: Send + Sync {
    fn finalize_local(
        &self,
        source_path: &Path,
        dest_path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
}
