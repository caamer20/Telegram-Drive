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
    pub retry_count: i64,
}

// Media metadata return from ffprobe
#[derive(Debug, Clone, Default)]
pub struct VideoMetadata {
    /// Full format_name string from ffprobe (e.g. "mov,mp4,m4a,3gp,3g2,mj2")
    pub container_format_names: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub audio_channels: u32,
    pub audio_sample_rate: u32,
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
    /// ISO BMFF major_brand from ffprobe format.tags.major_brand.
    pub major_brand: String,
}

impl VideoMetadata {
    /// Check if the container format is MP4-compatible (MP4 family)
    pub fn is_mp4_compatible(&self) -> bool {
        let formats: Vec<&str> = self
            .container_format_names
            .split(',')
            .map(|s| s.trim())
            .collect();
        formats.iter().any(|f| {
            matches!(
                *f,
                "mp4" | "mov" | "m4a" | "3gp" | "3g2" | "mj2" | "ismv" | "ipod"
            )
        })
    }

    /// Source passthrough is stricter than output validation: a QuickTime `.mov`
    /// must not become canonical merely because FFprobe reports the shared MOV/MP4 demuxer.
    pub fn is_mp4_source(&self, source_path: &Path) -> bool {
        let has_mp4_extension = source_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("mp4"))
            .unwrap_or(false);
        let brand = self.major_brand.trim().to_ascii_lowercase();
        let has_mp4_brand = matches!(
            brand.as_str(),
            "isom"
                | "iso2"
                | "iso3"
                | "iso4"
                | "iso5"
                | "iso6"
                | "mp41"
                | "mp42"
                | "avc1"
                | "dash"
                | "mmp4"
                | "msnv"
        );
        self.is_mp4_compatible() && (has_mp4_extension || has_mp4_brand)
    }

    /// Check if this is a valid HEVC Main 8-bit passthrough candidate
    pub fn is_canonical_main8(&self) -> bool {
        self.is_valid
            && self.duration > 0.0
            && self.video_codec == "hevc"
            && self.profile.to_ascii_lowercase() == "main"
            && self.pixel_format == "yuv420p"
            && self.is_mp4_compatible()
            && (self.audio_codec.is_empty() || self.audio_codec == "aac")
            && self.width <= 1920
            && self.height <= 1080
            && self.fps > 0.0
            && self.fps <= 60.0
            && self.color_transfer != "smpte2084"
            && self.color_transfer != "arib-std-b67"
    }

    /// Check if this is a valid HEVC Main10 passthrough candidate
    pub fn is_canonical_main10(&self) -> bool {
        let profile_lower = self.profile.to_ascii_lowercase().replace(' ', "");
        let valid_10bit_pix_fmt = matches!(
            self.pixel_format.as_str(),
            "yuv420p10le" | "yuv420p10be" | "p010le" | "p010be"
        );
        self.is_valid
            && self.duration > 0.0
            && self.video_codec == "hevc"
            && profile_lower == "main10"
            && valid_10bit_pix_fmt
            && self.is_mp4_compatible()
            && (self.audio_codec.is_empty() || self.audio_codec == "aac")
            && self.width <= 1920
            && self.height <= 1080
            && self.fps > 0.0
            && self.fps <= 60.0
    }

    /// Check if source is HDR (PQ or HLG transfer)
    pub fn is_hdr(&self) -> bool {
        self.color_transfer == "smpte2084" || self.color_transfer == "arib-std-b67"
    }

    /// Check if source has 10-bit pixel format
    pub fn is_10bit(&self) -> bool {
        self.pixel_format.contains("10")
            || self.pixel_format.contains("p010")
            || self
                .profile
                .to_ascii_lowercase()
                .replace(' ', "")
                .contains("10")
            || self.is_hdr()
    }
}

pub fn target_audio_bitrate(metadata: &VideoMetadata) -> u64 {
    match metadata.audio_channels {
        0 => 0,
        1 => 64_000,
        2 => 96_000,
        _ => 192_000,
    }
}

pub fn resolution_video_bitrate_cap(metadata: &VideoMetadata) -> u64 {
    let short_edge = metadata.width.min(metadata.height);
    let mut cap = if short_edge <= 480 {
        900_000u64
    } else if short_edge <= 720 {
        1_800_000u64
    } else {
        3_000_000u64
    };
    if metadata.fps > 30.0 {
        cap = cap.saturating_mul(6) / 5;
    }
    cap
}

pub fn target_video_bitrate(metadata: &VideoMetadata) -> u64 {
    let resolution_cap = resolution_video_bitrate_cap(metadata);
    if metadata.bitrate == 0 {
        return resolution_cap;
    }
    let source_cap = metadata
        .bitrate
        .saturating_mul(4)
        .checked_div(5)
        .unwrap_or(0)
        .saturating_sub(target_audio_bitrate(metadata))
        .max(96_000);
    resolution_cap.min(source_cap)
}

pub fn is_below_optimization_target(metadata: &VideoMetadata, source_size: u64) -> bool {
    if !metadata.is_valid
        || metadata.duration <= 0.0
        || metadata.width == 0
        || metadata.height == 0
        || metadata.width > 1920
        || metadata.height > 1080
        || metadata.fps <= 0.0
        || metadata.fps > 60.0
    {
        return false;
    }
    let effective_bitrate = if metadata.bitrate > 0 {
        metadata.bitrate
    } else {
        ((source_size as f64 * 8.0) / metadata.duration) as u64
    };
    effective_bitrate
        <= resolution_video_bitrate_cap(metadata).saturating_add(target_audio_bitrate(metadata))
}

/// Canonical video profile for encoding decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalVideoProfile {
    Main8,
    Main10,
}

impl CanonicalVideoProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Main8 => "main",
            Self::Main10 => "main10",
        }
    }

    pub fn pixel_format(&self) -> &'static str {
        match self {
            Self::Main8 => "yuv420p",
            Self::Main10 => "yuv420p10le",
        }
    }
}

/// Validate encoded output against canonical policy
pub fn validate_canonical_output(
    source: &VideoMetadata,
    output: &VideoMetadata,
    expected_profile: CanonicalVideoProfile,
) -> Result<(), String> {
    if output.video_codec.is_empty() {
        return Err("Output has no video stream".into());
    }
    if !output.is_mp4_compatible() {
        return Err(format!(
            "Output container '{}' is not MP4-compatible",
            output.container_format_names
        ));
    }
    if output.video_codec != "hevc" {
        return Err(format!("Output codec '{}' is not HEVC", output.video_codec));
    }
    let expected_profile_str = expected_profile.as_str();
    if output.profile.to_ascii_lowercase().replace(' ', "") != expected_profile_str {
        return Err(format!(
            "Output profile '{}' does not match expected '{}'",
            output.profile, expected_profile_str
        ));
    }
    let expected_pix_fmt = expected_profile.pixel_format();
    if output.pixel_format != expected_pix_fmt {
        return Err(format!(
            "Output pixel format '{}' does not match expected '{}'",
            output.pixel_format, expected_pix_fmt
        ));
    }
    if !output.audio_codec.is_empty() && output.audio_codec != "aac" {
        return Err(format!(
            "Output audio codec '{}' is not AAC",
            output.audio_codec
        ));
    }
    if output.width == 0 || output.height == 0 {
        return Err("Output has zero dimensions".into());
    }
    if output.width > 1920 || output.height > 1080 {
        return Err(format!(
            "Output resolution {}x{} exceeds 1920x1080",
            output.width, output.height
        ));
    }
    if output.fps <= 0.0 || output.fps > 60.0 {
        return Err(format!("Output FPS {} is out of range (0, 60]", output.fps));
    }
    if output.duration <= 0.0 {
        return Err("Output has zero duration".into());
    }
    let tolerance = f64::max(2.0, source.duration * 0.02);
    if (output.duration - source.duration).abs() > tolerance {
        return Err(format!(
            "Output duration {} differs from source {} by more than tolerance {}",
            output.duration, source.duration, tolerance
        ));
    }
    Ok(())
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

#[derive(Debug, Clone)]
pub struct VideoProcessRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub decision: String,
    pub item_id: i64,
    pub job_id: i64,
    pub item_name: String,
    pub metadata: VideoMetadata,
}

pub trait VideoProcessor: Send + Sync {
    fn process_video(
        &self,
        request: VideoProcessRequest,
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

#[cfg(test)]
mod optimization_tests {
    use super::*;

    fn metadata(width: u32, height: u32, fps: f64, bitrate: u64) -> VideoMetadata {
        VideoMetadata {
            is_valid: true,
            duration: 60.0,
            width,
            height,
            fps,
            bitrate,
            audio_codec: "aac".to_string(),
            audio_channels: 2,
            ..Default::default()
        }
    }

    #[test]
    fn low_quality_video_skips_optimization() {
        assert!(is_below_optimization_target(
            &metadata(1280, 720, 30.0, 1_500_000),
            11_250_000,
        ));
    }

    #[test]
    fn high_resolution_fps_or_bitrate_requires_optimization() {
        assert!(!is_below_optimization_target(
            &metadata(3840, 2160, 30.0, 2_000_000),
            15_000_000,
        ));
        assert!(!is_below_optimization_target(
            &metadata(1920, 1080, 120.0, 2_000_000),
            15_000_000,
        ));
        assert!(!is_below_optimization_target(
            &metadata(1920, 1080, 30.0, 5_000_000),
            37_500_000,
        ));
    }

    #[test]
    fn missing_bitrate_uses_file_size_and_duration() {
        let meta = metadata(1280, 720, 30.0, 0);
        assert!(is_below_optimization_target(&meta, 10_000_000));
        assert!(!is_below_optimization_target(&meta, 20_000_000));
    }
}
