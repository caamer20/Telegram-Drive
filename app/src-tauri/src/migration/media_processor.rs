use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};

const MAX_DISPLAY_WIDTH: u32 = 1920;
const MAX_DISPLAY_HEIGHT: u32 = 1080;
const MAX_ERROR_BYTES: usize = 8 * 1024;
const VIDEO_EXTENSIONS: &[&str] = &[
    "3gp", "avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "m2ts", "webm", "wmv",
];

#[derive(Debug, Clone, PartialEq)]
pub struct MediaProbe {
    pub has_video: bool,
    pub video_codec: Option<String>,
    pub encoded_width: Option<u32>,
    pub encoded_height: Option<u32>,
    pub rotation_degrees: i32,
    pub display_width: Option<u32>,
    pub display_height: Option<u32>,
    pub duration_seconds: Option<f64>,
    pub has_audio: bool,
}

impl MediaProbe {
    fn non_video() -> Self {
        Self {
            has_video: false,
            video_codec: None,
            encoded_width: None,
            encoded_height: None,
            rotation_degrees: 0,
            display_width: None,
            display_height: None,
            duration_seconds: None,
            has_audio: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscodeDecision {
    PassthroughNonVideo,
    PassthroughCompatible,
    Transcode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaProgressPhase {
    Analyzing,
    Processing,
}

impl MediaProgressPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Analyzing => "analyzing",
            Self::Processing => "processing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaProgress {
    pub phase: MediaProgressPhase,
    pub percent: u8,
}

#[derive(Debug)]
pub enum MediaProcessError {
    ToolUnavailable(String),
    ProbeFailed(String),
    TranscodeFailed(String),
    Cancelled,
    InsufficientDisk(String),
    InvalidOutput(String),
}

impl MediaProcessError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InsufficientDisk(_) => "insufficient_disk",
            Self::Cancelled => "unknown",
            Self::ToolUnavailable(_) | Self::ProbeFailed(_) | Self::TranscodeFailed(_) => "unknown",
            Self::InvalidOutput(_) => "unknown",
        }
    }
}

impl std::fmt::Display for MediaProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolUnavailable(message) => write!(f, "[media_tool] {message}"),
            Self::ProbeFailed(message) => write!(f, "[media_probe] {message}"),
            Self::TranscodeFailed(message) => write!(f, "[media_transcode] {message}"),
            Self::Cancelled => write!(f, "[media_cancelled] Video processing cancelled"),
            Self::InsufficientDisk(message) => write!(f, "[disk] {message}"),
            Self::InvalidOutput(message) => write!(f, "[media_output] {message}"),
        }
    }
}

impl std::error::Error for MediaProcessError {}

#[derive(Debug)]
pub struct PreparedUpload {
    pub path: PathBuf,
    pub upload_name: String,
    pub size_bytes: u64,
    pub decision: TranscodeDecision,
    owned_temp_path: Option<PathBuf>,
}

impl PreparedUpload {
    fn passthrough(
        source_path: &Path,
        source_name: &str,
        decision: TranscodeDecision,
    ) -> Result<Self, MediaProcessError> {
        let size_bytes = std::fs::metadata(source_path)
            .map_err(|error| MediaProcessError::InvalidOutput(error.to_string()))?
            .len();
        Ok(Self {
            path: source_path.to_path_buf(),
            upload_name: source_name.to_string(),
            size_bytes,
            decision,
            owned_temp_path: None,
        })
    }

    fn transcoded(output_path: &Path, source_name: &str) -> Result<Self, MediaProcessError> {
        let size_bytes = std::fs::metadata(output_path)
            .map_err(|error| MediaProcessError::InvalidOutput(error.to_string()))?
            .len();
        if size_bytes == 0 {
            return Err(MediaProcessError::InvalidOutput(
                "FFmpeg produced an empty output file".to_string(),
            ));
        }
        let stem = Path::new(source_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("video");
        Ok(Self {
            path: output_path.to_path_buf(),
            upload_name: format!("{stem}.mp4"),
            size_bytes,
            decision: TranscodeDecision::Transcode,
            owned_temp_path: Some(output_path.to_path_buf()),
        })
    }

    pub async fn cleanup(&mut self) {
        if let Some(path) = self.owned_temp_path.take() {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

impl Drop for PreparedUpload {
    fn drop(&mut self) {
        if let Some(path) = self.owned_temp_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn is_owned_transcode_output(file_name: &str) -> bool {
    let Some(ids) = file_name
        .strip_prefix("mig_")
        .and_then(|value| value.strip_suffix(".transcoded.mp4"))
    else {
        return false;
    };
    let Some((job_id, item_id)) = ids.split_once('_') else {
        return false;
    };
    job_id.parse::<i64>().is_ok() && item_id.parse::<i64>().is_ok()
}

pub fn cleanup_orphaned_outputs(local_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(local_dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_owned = path.is_file()
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .map(is_owned_transcode_output)
                .unwrap_or(false);
        if is_owned && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[derive(Debug, Deserialize)]
struct ProbeDocument {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
    tags: Option<ProbeTags>,
    #[serde(default)]
    side_data_list: Vec<ProbeSideData>,
}

#[derive(Debug, Deserialize)]
struct ProbeTags {
    rotate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeSideData {
    rotation: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

fn normalize_rotation(value: i32) -> i32 {
    let normalized = value.rem_euclid(360);
    match normalized {
        45..=134 => 90,
        135..=224 => 180,
        225..=314 => 270,
        _ => 0,
    }
}

fn parse_probe_json(bytes: &[u8]) -> Result<MediaProbe, MediaProcessError> {
    let document: ProbeDocument = serde_json::from_slice(bytes)
        .map_err(|error| MediaProcessError::ProbeFailed(error.to_string()))?;
    let video = document
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"));
    let Some(video) = video else {
        return Ok(MediaProbe {
            has_audio: document
                .streams
                .iter()
                .any(|stream| stream.codec_type.as_deref() == Some("audio")),
            ..MediaProbe::non_video()
        });
    };
    let width = video.width.filter(|value| *value > 0);
    let height = video.height.filter(|value| *value > 0);
    let rotation = video
        .side_data_list
        .iter()
        .find_map(|data| data.rotation)
        .or_else(|| {
            video
                .tags
                .as_ref()
                .and_then(|tags| tags.rotate.as_deref())
                .and_then(|value| value.parse().ok())
        })
        .map(normalize_rotation)
        .unwrap_or(0);
    let (display_width, display_height) = match (width, height) {
        (Some(width), Some(height)) if rotation == 90 || rotation == 270 => {
            (Some(height), Some(width))
        }
        dimensions => dimensions,
    };
    let duration_seconds = video
        .duration
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            document
                .format
                .as_ref()
                .and_then(|format| format.duration.as_deref())
                .and_then(|value| value.parse::<f64>().ok())
        })
        .filter(|value| value.is_finite() && *value > 0.0);
    Ok(MediaProbe {
        has_video: true,
        video_codec: video
            .codec_name
            .as_ref()
            .map(|value| value.to_ascii_lowercase()),
        encoded_width: width,
        encoded_height: height,
        rotation_degrees: rotation,
        display_width,
        display_height,
        duration_seconds,
        has_audio: document
            .streams
            .iter()
            .any(|stream| stream.codec_type.as_deref() == Some("audio")),
    })
}

pub fn decide_transcode(probe: &MediaProbe) -> Result<TranscodeDecision, MediaProcessError> {
    if !probe.has_video {
        return Ok(TranscodeDecision::PassthroughNonVideo);
    }
    let width = probe.display_width.ok_or_else(|| {
        MediaProcessError::ProbeFailed("Video stream is missing a valid width".to_string())
    })?;
    let height = probe.display_height.ok_or_else(|| {
        MediaProcessError::ProbeFailed("Video stream is missing a valid height".to_string())
    })?;
    if probe.video_codec.as_deref() == Some("h264")
        && width <= MAX_DISPLAY_WIDTH
        && height <= MAX_DISPLAY_HEIGHT
    {
        Ok(TranscodeDecision::PassthroughCompatible)
    } else {
        Ok(TranscodeDecision::Transcode)
    }
}

fn is_video_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| VIDEO_EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn ffprobe_candidates(ffmpeg_path: &Path) -> Vec<PathBuf> {
    let executable = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let mut candidates = Vec::new();
    if let Some(parent) = ffmpeg_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        candidates.push(parent.join(executable));
    }
    candidates.push(PathBuf::from(executable));
    candidates.dedup();
    candidates
}

fn bounded_error(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(MAX_ERROR_BYTES);
    String::from_utf8_lossy(&bytes[start..]).trim().to_string()
}

fn ensure_output_space(source_path: &Path, output_path: &Path) -> Result<(), MediaProcessError> {
    let source_size = std::fs::metadata(source_path)
        .map_err(|error| MediaProcessError::InvalidOutput(error.to_string()))?
        .len();
    let required = source_size.saturating_add(64 * 1024 * 1024);
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    let canonical_parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let available = disks
        .iter()
        .filter(|disk| canonical_parent.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space());
    if let Some(available) = available {
        if available < required {
            return Err(MediaProcessError::InsufficientDisk(format!(
                "Need at least {required} bytes for video output, only {available} bytes available"
            )));
        }
    }
    Ok(())
}

async fn probe_media_with_name(
    ffmpeg_path: &Path,
    source_path: &Path,
    source_name: Option<&str>,
) -> Result<MediaProbe, MediaProcessError> {
    let video_candidate = is_video_candidate(source_path)
        || source_name
            .map(Path::new)
            .map(is_video_candidate)
            .unwrap_or(false);
    let mut last_unavailable = None;
    for candidate in ffprobe_candidates(ffmpeg_path) {
        match tokio::process::Command::new(&candidate)
            .arg("-v")
            .arg("error")
            .arg("-show_streams")
            .arg("-show_format")
            .arg("-of")
            .arg("json")
            .arg(source_path)
            .stdin(Stdio::null())
            .output()
            .await
        {
            Ok(output) if output.status.success() => return parse_probe_json(&output.stdout),
            Ok(output) => {
                if video_candidate {
                    return Err(MediaProcessError::ProbeFailed(bounded_error(
                        &output.stderr,
                    )));
                }
                return Ok(MediaProbe::non_video());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                last_unavailable = Some(format!("FFprobe not found at {}", candidate.display()));
            }
            Err(error) => {
                return Err(MediaProcessError::ToolUnavailable(error.to_string()));
            }
        }
    }
    if video_candidate {
        Err(MediaProcessError::ToolUnavailable(
            last_unavailable.unwrap_or_else(|| "FFprobe is unavailable".to_string()),
        ))
    } else {
        Ok(MediaProbe::non_video())
    }
}

pub async fn probe_media(
    ffmpeg_path: &Path,
    source_path: &Path,
) -> Result<MediaProbe, MediaProcessError> {
    probe_media_with_name(ffmpeg_path, source_path, None).await
}

async fn run_ffmpeg<F>(
    ffmpeg_path: &Path,
    source_path: &Path,
    output_path: &Path,
    duration_seconds: Option<f64>,
    cancel_token: &AtomicBool,
    on_progress: &F,
) -> Result<(), MediaProcessError>
where
    F: Fn(MediaProgress),
{
    let scale = "scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2";
    let mut child = tokio::process::Command::new(ffmpeg_path)
        .arg("-y")
        .arg("-i")
        .arg(source_path)
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("0:a:0?")
        .arg("-sn")
        .arg("-dn")
        .arg("-vf")
        .arg(scale)
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("medium")
        .arg("-crf")
        .arg("23")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("128k")
        .arg("-movflags")
        .arg("+faststart")
        .arg("-progress")
        .arg("pipe:2")
        .arg("-nostats")
        .arg(output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| MediaProcessError::ToolUnavailable(error.to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| MediaProcessError::TranscodeFailed("Missing FFmpeg stderr".to_string()))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut poll = tokio::time::interval(std::time::Duration::from_millis(200));
    let mut error_tail = String::new();
    let mut last_percent = 0u8;

    loop {
        tokio::select! {
            _ = poll.tick() => {
                if cancel_token.load(Ordering::Relaxed) {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    let _ = tokio::fs::remove_file(output_path).await;
                    return Err(MediaProcessError::Cancelled);
                }
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(value) = line.strip_prefix("out_time_us=")
                            .and_then(|value| value.parse::<f64>().ok())
                        {
                            if let Some(duration) = duration_seconds.filter(|value| *value > 0.0) {
                                let percent = ((value / 1_000_000.0 / duration) * 100.0)
                                    .clamp(0.0, 99.0) as u8;
                                if percent > last_percent {
                                    last_percent = percent;
                                    on_progress(MediaProgress {
                                        phase: MediaProgressPhase::Processing,
                                        percent,
                                    });
                                }
                            }
                        } else if !line.starts_with("frame=")
                            && !line.starts_with("fps=")
                            && !line.starts_with("bitrate=")
                            && !line.starts_with("total_size=")
                            && !line.starts_with("out_time")
                            && !line.starts_with("dup_frames=")
                            && !line.starts_with("drop_frames=")
                            && !line.starts_with("speed=")
                            && !line.starts_with("progress=")
                        {
                            error_tail.push_str(&line);
                            error_tail.push('\n');
                            if error_tail.len() > MAX_ERROR_BYTES {
                                error_tail.drain(..error_tail.len() - MAX_ERROR_BYTES);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        error_tail.push_str(&error.to_string());
                        break;
                    }
                }
            }
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|error| MediaProcessError::TranscodeFailed(error.to_string()))?;
    if !status.success() {
        let _ = tokio::fs::remove_file(output_path).await;
        return Err(MediaProcessError::TranscodeFailed(format!(
            "FFmpeg exited with {:?}: {}",
            status.code(),
            error_tail.trim()
        )));
    }
    on_progress(MediaProgress {
        phase: MediaProgressPhase::Processing,
        percent: 100,
    });
    Ok(())
}

pub async fn prepare_upload<F>(
    ffmpeg_path: &Path,
    source_path: &Path,
    source_name: &str,
    output_path: &Path,
    cancel_token: &AtomicBool,
    on_progress: F,
) -> Result<PreparedUpload, MediaProcessError>
where
    F: Fn(MediaProgress),
{
    if cancel_token.load(Ordering::Relaxed) {
        return Err(MediaProcessError::Cancelled);
    }
    on_progress(MediaProgress {
        phase: MediaProgressPhase::Analyzing,
        percent: 0,
    });
    let probe = probe_media_with_name(ffmpeg_path, source_path, Some(source_name)).await?;
    on_progress(MediaProgress {
        phase: MediaProgressPhase::Analyzing,
        percent: 100,
    });
    match decide_transcode(&probe)? {
        TranscodeDecision::PassthroughNonVideo => PreparedUpload::passthrough(
            source_path,
            source_name,
            TranscodeDecision::PassthroughNonVideo,
        ),
        TranscodeDecision::PassthroughCompatible => PreparedUpload::passthrough(
            source_path,
            source_name,
            TranscodeDecision::PassthroughCompatible,
        ),
        TranscodeDecision::Transcode => {
            ensure_output_space(source_path, output_path)?;
            let _ = tokio::fs::remove_file(output_path).await;
            on_progress(MediaProgress {
                phase: MediaProgressPhase::Processing,
                percent: 0,
            });
            run_ffmpeg(
                ffmpeg_path,
                source_path,
                output_path,
                probe.duration_seconds,
                cancel_token,
                &on_progress,
            )
            .await?;
            let output_probe = probe_media(ffmpeg_path, output_path).await?;
            let output_width = output_probe.display_width.unwrap_or(u32::MAX);
            let output_height = output_probe.display_height.unwrap_or(u32::MAX);
            if output_probe.video_codec.as_deref() != Some("h264")
                || output_width > MAX_DISPLAY_WIDTH
                || output_height > MAX_DISPLAY_HEIGHT
            {
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(MediaProcessError::InvalidOutput(format!(
                    "Expected H.264 within 1920x1080, got {:?} {}x{}",
                    output_probe.video_codec, output_width, output_height
                )));
            }
            PreparedUpload::transcoded(output_path, source_name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn probe(codec: &str, width: u32, height: u32) -> MediaProbe {
        MediaProbe {
            has_video: true,
            video_codec: Some(codec.to_string()),
            encoded_width: Some(width),
            encoded_height: Some(height),
            rotation_degrees: 0,
            display_width: Some(width),
            display_height: Some(height),
            duration_seconds: Some(10.0),
            has_audio: true,
        }
    }

    #[test]
    fn parses_rotation_and_display_dimensions() {
        let json = br#"{
          "streams": [
            {"codec_type":"video","codec_name":"hevc","width":3840,"height":2160,
             "side_data_list":[{"rotation":-90}]},
            {"codec_type":"audio","codec_name":"aac"}
          ],
          "format":{"duration":"12.5"}
        }"#;
        let parsed = parse_probe_json(json).expect("probe");
        assert_eq!(parsed.rotation_degrees, 270);
        assert_eq!(parsed.display_width, Some(2160));
        assert_eq!(parsed.display_height, Some(3840));
        assert_eq!(parsed.duration_seconds, Some(12.5));
        assert!(parsed.has_audio);
    }

    #[test]
    fn passthrough_only_for_compatible_h264() {
        assert_eq!(
            decide_transcode(&probe("h264", 1280, 720)).unwrap(),
            TranscodeDecision::PassthroughCompatible
        );
        assert_eq!(
            decide_transcode(&probe("vp9", 1280, 720)).unwrap(),
            TranscodeDecision::Transcode
        );
        assert_eq!(
            decide_transcode(&probe("h264", 3840, 2160)).unwrap(),
            TranscodeDecision::Transcode
        );
    }

    #[test]
    fn non_video_is_passthrough() {
        assert_eq!(
            decide_transcode(&MediaProbe::non_video()).unwrap(),
            TranscodeDecision::PassthroughNonVideo
        );
    }

    #[test]
    fn missing_video_dimensions_are_rejected() {
        let mut invalid = probe("h264", 1280, 720);
        invalid.display_width = None;
        assert!(matches!(
            decide_transcode(&invalid),
            Err(MediaProcessError::ProbeFailed(_))
        ));
    }

    #[tokio::test]
    async fn owned_output_is_cleaned_explicitly() {
        let path = std::env::temp_dir().join(format!(
            "telegram-drive-media-cleanup-{}-{}.mp4",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        tokio::fs::write(&path, b"output").await.unwrap();
        let mut prepared = PreparedUpload::transcoded(&path, "movie.mov").unwrap();
        assert_eq!(prepared.upload_name, "movie.mp4");
        prepared.cleanup().await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn zero_byte_output_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "telegram-drive-media-empty-{}-{}.mp4",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        tokio::fs::write(&path, []).await.unwrap();
        assert!(matches!(
            PreparedUpload::transcoded(&path, "movie.mov"),
            Err(MediaProcessError::InvalidOutput(_))
        ));
        let _ = tokio::fs::remove_file(path).await;
    }

    #[test]
    fn startup_cleanup_removes_only_owned_transcode_outputs() {
        let dir = std::env::temp_dir().join(format!(
            "telegram-drive-media-recovery-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let owned = dir.join("mig_12_34.transcoded.mp4");
        let unrelated = dir.join("family.transcoded.mp4");
        let malformed = dir.join("mig_job_item.transcoded.mp4");
        std::fs::write(&owned, b"partial").unwrap();
        std::fs::write(&unrelated, b"keep").unwrap();
        std::fs::write(&malformed, b"keep").unwrap();

        assert_eq!(cleanup_orphaned_outputs(&dir), 1);
        assert!(!owned.exists());
        assert!(unrelated.exists());
        assert!(malformed.exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn cancelled_before_probe_returns_cancelled() {
        let cancelled = AtomicBool::new(true);
        let result = prepare_upload(
            Path::new("ffmpeg"),
            Path::new("missing.mp4"),
            "missing.mp4",
            Path::new("missing.transcoded.mp4"),
            &cancelled,
            |_| {},
        )
        .await;
        assert!(matches!(result, Err(MediaProcessError::Cancelled)));
    }

    #[tokio::test]
    async fn corrupt_video_keeps_original_name_as_probe_error_hint() {
        let prefix = format!(
            "telegram-drive-corrupt-video-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let source = std::env::temp_dir().join(format!("{prefix}.part"));
        let output = std::env::temp_dir().join(format!("{prefix}.mp4"));
        tokio::fs::write(&source, b"not a video").await.unwrap();
        let result = prepare_upload(
            Path::new("ffmpeg"),
            &source,
            "broken.mp4",
            &output,
            &AtomicBool::new(false),
            |_| {},
        )
        .await;
        assert!(matches!(
            result,
            Err(MediaProcessError::ProbeFailed(_)) | Err(MediaProcessError::ToolUnavailable(_))
        ));
        let _ = tokio::fs::remove_file(source).await;
    }

    #[tokio::test]
    async fn ffmpeg_integration_transcodes_large_video_when_available() {
        let ffmpeg = PathBuf::from("ffmpeg");
        if tokio::process::Command::new(&ffmpeg)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|status| !status.success())
            .unwrap_or(true)
        {
            return;
        }
        let prefix = format!(
            "telegram-drive-media-integration-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let source = std::env::temp_dir().join(format!("{prefix}.mkv"));
        let output = std::env::temp_dir().join(format!("{prefix}.transcoded.mp4"));
        let generated = tokio::process::Command::new(&ffmpeg)
            .args(["-y", "-f", "lavfi", "-i", "color=size=2560x1440:rate=1"])
            .args(["-t", "1", "-c:v", "mpeg4"])
            .arg(&source)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .expect("generate fixture");
        assert!(generated.success());

        let cancelled = AtomicBool::new(false);
        let progress_values = std::sync::Mutex::new(Vec::new());
        let mut prepared = prepare_upload(
            &ffmpeg,
            &source,
            "large-video.mkv",
            &output,
            &cancelled,
            |progress| {
                progress_values
                    .lock()
                    .unwrap()
                    .push((progress.phase, progress.percent));
            },
        )
        .await
        .expect("transcode");
        assert_eq!(prepared.decision, TranscodeDecision::Transcode);
        assert_eq!(prepared.upload_name, "large-video.mp4");
        let probe = probe_media(&ffmpeg, &prepared.path)
            .await
            .expect("output probe");
        assert_eq!(probe.video_codec.as_deref(), Some("h264"));
        assert!(probe.display_width.unwrap() <= MAX_DISPLAY_WIDTH);
        assert!(probe.display_height.unwrap() <= MAX_DISPLAY_HEIGHT);
        let progress_values = progress_values.into_inner().unwrap();
        assert!(progress_values
            .windows(2)
            .all(|pair| { pair[0].0 != pair[1].0 || pair[0].1 <= pair[1].1 }));

        prepared.cleanup().await;
        let _ = tokio::fs::remove_file(source).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_kills_child_and_removes_unicode_output() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;

        let prefix = format!(
            "telegram-drive-media-cancel-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let fake_ffmpeg = std::env::temp_dir().join(format!("{prefix}.sh"));
        let source = std::env::temp_dir().join(format!("{prefix}.mp4"));
        let output = std::env::temp_dir().join(format!("{prefix}-đầu-ra.mp4"));
        tokio::fs::write(&fake_ffmpeg, b"#!/bin/sh\nsleep 10\n")
            .await
            .unwrap();
        let mut permissions = std::fs::metadata(&fake_ffmpeg).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fake_ffmpeg, permissions).unwrap();
        tokio::fs::write(&source, b"source").await.unwrap();
        tokio::fs::write(&output, b"partial").await.unwrap();

        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_signal = cancelled.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel_signal.store(true, Ordering::Relaxed);
        });
        let result = run_ffmpeg(
            &fake_ffmpeg,
            &source,
            &output,
            Some(10.0),
            cancelled.as_ref(),
            &|_| {},
        )
        .await;
        assert!(matches!(result, Err(MediaProcessError::Cancelled)));
        assert!(!output.exists());

        let _ = tokio::fs::remove_file(fake_ffmpeg).await;
        let _ = tokio::fs::remove_file(source).await;
    }
}
