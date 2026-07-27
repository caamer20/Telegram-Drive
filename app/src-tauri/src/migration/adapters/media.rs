// FFmpeg production adapters for Pipeline V2
// Inspector: wraps ffprobe to populate VideoMetadata
// Processor: wraps ffmpeg for remux/transcode
//
// Test seam: the `ProcessRunner` trait allows injecting a fake process runner
// for automated tests without requiring FFmpeg on the test machine.

use crate::migration::pipeline::stages::{MediaInspector, VideoMetadata, VideoProcessor};
use serde::Deserialize;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Process runner seam — allows injection of fake runner in tests
// ---------------------------------------------------------------------------

pub trait ProcessRunner: Send + Sync {
    fn run_command(
        &self,
        program: &str,
        args: &[String],
        on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, String>> + Send + '_>>;
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Real process runner (used in production)
// ---------------------------------------------------------------------------

pub struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn run_command(
        &self,
        program: &str,
        args: &[String],
        on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, String>> + Send + '_>> {
        let program = program.to_string();
        let args = args.to_vec();

        Box::pin(async move {
            let mut child = tokio::process::Command::new(&program)
                .args(&args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("ProcessRunner: spawn failed: {}", e))?;

            let mut stdout = child.stdout.take().unwrap();
            let mut stderr = child.stderr.take().unwrap();

            let stdout_task = tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = [0; 1024];
                let mut current_line = String::new();
                let mut full_output = Vec::new();
                while let Ok(n) = stdout.read(&mut buf).await {
                    if n == 0 { break; }
                    if full_output.len() < 1024 * 1024 {
                        full_output.extend_from_slice(&buf[..n]);
                    }
                    if let Some(ref cb) = on_progress {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        for ch in text.chars() {
                            if ch == '\n' || ch == '\r' {
                                if !current_line.is_empty() {
                                    cb(&current_line);
                                    current_line.clear();
                                }
                            } else {
                                current_line.push(ch);
                            }
                        }
                    }
                }
                full_output
            });

            let stderr_task = tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = [0; 4096];
                let mut full_output = Vec::new();
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 { break; }
                    if full_output.len() < 1024 * 1024 { // max 1MB
                        full_output.extend_from_slice(&buf[..n]);
                    }
                }
                full_output
            });

            let status = child.wait().await.map_err(|e| format!("Wait error: {}", e))?;
            let stdout_out = stdout_task.await.unwrap_or_default();
            let stderr_out = stderr_task.await.unwrap_or_default();

            Ok(ProcessOutput {
                exit_code: status.code().unwrap_or(-1),
                stdout: stdout_out,
                stderr: stderr_out,
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Production FFmpeg adapter
// ---------------------------------------------------------------------------

pub struct FFmpegMediaAdapter {
    ffprobe_path: PathBuf,
    ffmpeg_path: PathBuf,
    process_runner: Arc<dyn ProcessRunner>,
    cancel_token: Arc<AtomicBool>,
    max_threads: usize,
    app_handle: Option<tauri::AppHandle>,
    hevc_encoder: String,
}

impl FFmpegMediaAdapter {
    pub fn new(
        ffprobe_path: PathBuf,
        ffmpeg_path: PathBuf,
        cancel_token: Arc<AtomicBool>,
        max_threads: usize,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        let mut hevc_encoder = "libx265".to_string();
        if cfg!(target_os = "macos") {
            if let Ok(output) = std::process::Command::new(&ffmpeg_path)
                .args(["-encoders"])
                .output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("hevc_videotoolbox") {
                    hevc_encoder = "hevc_videotoolbox".to_string();
                }
            }
        }

        Self {
            ffprobe_path,
            ffmpeg_path,
            process_runner: Arc::new(RealProcessRunner),
            cancel_token,
            max_threads,
            app_handle,
            hevc_encoder,
        }
    }

    /// Constructor for tests — accepts injectable process runner
    pub fn new_with_runner(
        ffprobe_path: PathBuf,
        ffmpeg_path: PathBuf,
        process_runner: Arc<dyn ProcessRunner>,
        cancel_token: Arc<AtomicBool>,
        max_threads: usize,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            ffprobe_path,
            ffmpeg_path,
            process_runner,
            cancel_token,
            max_threads,
            app_handle,
            hevc_encoder: "libx265".to_string(),
        }
    }

    fn build_ffprobe_args(source_path: &Path) -> Vec<String> {
        vec![
            "-v".to_string(),
            "error".to_string(),
            "-show_streams".to_string(),
            "-show_format".to_string(),
            "-of".to_string(),
            "json".to_string(),
            source_path.to_string_lossy().to_string(),
        ]
    }

    fn build_remux_args(input_path: &Path, output_path: &Path) -> Vec<String> {
        vec![
            "-y".to_string(),
            "-i".to_string(),
            input_path.to_string_lossy().to_string(),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "0:a:0?".to_string(),
            "-sn".to_string(),
            "-dn".to_string(),
            "-c".to_string(),
            "copy".to_string(),
            "-max_muxing_queue_size".to_string(),
            "1024".to_string(),
            "-movflags".to_string(),
            "+faststart".to_string(),
            "-progress".to_string(),
            "pipe:1".to_string(),
            "-nostats".to_string(),
            output_path.to_string_lossy().to_string(),
        ]
    }

    fn build_transcode_args(
        input_path: &Path,
        output_path: &Path,
        is_10bit: bool,
        encoder: &str,
    ) -> Vec<String> {
        let mut args = vec![
            "-y".to_string(),
            "-i".to_string(),
            input_path.to_string_lossy().to_string(),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "0:a:0?".to_string(),
            "-sn".to_string(),
            "-dn".to_string(),
            "-vf".to_string(),
            "scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2".to_string(),
        ];

        let pix_fmt = if is_10bit { "yuv420p10le" } else { "yuv420p" };
        let profile = if is_10bit { "main10" } else { "main" };

        args.push("-c:v".to_string());
        args.push(encoder.to_string());

        if encoder == "hevc_videotoolbox" {
            args.push("-profile:v".to_string());
            args.push(profile.to_string());
            args.push("-q:v".to_string());
            args.push("60".to_string());
        } else {
            args.push("-preset".to_string());
            args.push("faster".to_string());
            args.push("-crf".to_string());
            args.push("26".to_string());
            args.push("-profile:v".to_string());
            args.push(profile.to_string());
        }

        args.push("-pix_fmt".to_string());
        args.push(pix_fmt.to_string());
        args.push("-c:a".to_string());
        args.push("aac".to_string());
        args.push("-b:a".to_string());
        args.push("128k".to_string());
        args.push("-max_muxing_queue_size".to_string());
        args.push("1024".to_string());
        args.push("-movflags".to_string());
        args.push("+faststart".to_string());
        args.push("-progress".to_string());
        args.push("pipe:1".to_string());
        args.push("-nostats".to_string());
        args.push(output_path.to_string_lossy().to_string());

        args
    }
}

impl MediaInspector for FFmpegMediaAdapter {
    fn inspect_file(
        &self,
        path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<VideoMetadata, String>> + Send>> {
        let ffprobe = self.ffprobe_path.to_string_lossy().to_string();
        let args = Self::build_ffprobe_args(path);
        let runner = self.process_runner.clone();
        let cancel = self.cancel_token.clone();

        Box::pin(async move {
            if cancel.load(Ordering::Relaxed) {
                return Err("Inspector: cancelled".to_string());
            }

            let output = runner
                .run_command(&ffprobe, &args, None)
                .await
                .map_err(|e| format!("Inspector: ffprobe failed: {}", e))?;

            if output.exit_code != 0 {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!(
                    "Inspector: ffprobe non-zero exit: {} — {}",
                    output.exit_code, stderr
                ));
            }

            parse_ffprobe_json(&output.stdout).map_err(|e| format!("Inspector: parse error: {}", e))
        })
    }
}

impl VideoProcessor for FFmpegMediaAdapter {
    fn process_video(
        &self,
        input_path: &Path,
        output_path: &Path,
        decision: &str,
        item_id: i64,
        job_id: i64,
        duration: f64,
        item_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        let ffmpeg = self.ffmpeg_path.to_string_lossy().to_string();
        let args = match decision {
            "canonical_passthrough_main8" | "canonical_passthrough_main10" => Self::build_remux_args(input_path, output_path),
            "canonical_transcode_main8" | "transcode" => Self::build_transcode_args(input_path, output_path, false, &self.hevc_encoder),
            "canonical_transcode_main10" => Self::build_transcode_args(input_path, output_path, true, &self.hevc_encoder),
            other => {
                let msg = format!("Processor: unsupported decision: {}", other);
                return Box::pin(async move { Err(msg) });
            }
        };
        let runner = self.process_runner.clone();
        let cancel = self.cancel_token.clone();
        let output_path_owned = output_path.to_path_buf();
        let app_handle = self.app_handle.clone();
        let duration_us = duration * 1_000_000.0;
        let item_name = item_name.to_string();

        Box::pin(async move {
            if cancel.load(Ordering::Relaxed) {
                return Err("Processor: cancelled".to_string());
            }

            let on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>> = if let Some(app) = app_handle {
                let last_emit_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                Some(Arc::new(move |line: &str| {
                    if let Some(time_us_str) = line.strip_prefix("out_time_us=") {
                        if let Ok(time_us) = time_us_str.trim().parse::<f64>() {
                            let percent = if duration_us > 0.0 {
                                (time_us / duration_us * 100.0).clamp(0.0, 100.0)
                            } else {
                                0.0
                            };
                            use tauri::Emitter;
                            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                            let last = last_emit_ms.load(std::sync::atomic::Ordering::Relaxed);
                            if now_ms - last >= 250 {
                                last_emit_ms.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                                let payload = serde_json::json!({
                                    "item_id": item_id,
                                    "job_id": job_id,
                                    "item_name": item_name,
                                    "percent": percent,
                                    "phase": "processing"
                                });
                                let _ = app.emit("migration:compression-progress", payload);
                            }
                        }
                    }
                }))
            } else {
                None
            };

            let output = runner
                .run_command(&ffmpeg, &args, on_progress)
                .await
                .map_err(|e| format!("Processor: ffmpeg spawn error: {}", e))?;

            if output.exit_code != 0 {
                // Cleanup partial output on failure
                let _ = tokio::fs::remove_file(&output_path_owned).await;
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!(
                    "Processor: ffmpeg non-zero exit: {} — {}",
                    output.exit_code, stderr
                ));
            }

            // Verify output exists and is non-empty
            let meta = tokio::fs::metadata(&output_path_owned)
                .await
                .map_err(|e| format!("Processor: output missing: {}", e))?;
            if meta.len() == 0 {
                let _ = tokio::fs::remove_file(&output_path_owned).await;
                return Err("Processor: output file is empty".to_string());
            }

            // Compute SHA-256 of processed file streaming chunked
            let mut file = tokio::fs::File::open(&output_path_owned).await
                .map_err(|e| format!("Processor: cannot open output: {}", e))?;
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                use tokio::io::AsyncReadExt;
                let count = file.read(&mut buffer).await
                    .map_err(|e| format!("Processor: error reading output: {}", e))?;
                if count == 0 { break; }
                hasher.update(&buffer[..count]);
            }
            let hash = hasher.finalize();
            Ok(format!("{:x}", hash))
        })
    }
}

// ---------------------------------------------------------------------------
// ffprobe JSON parsing (reuses existing media_processor.rs patterns)
// ---------------------------------------------------------------------------

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
    profile: Option<String>,
    pix_fmt: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    r_frame_rate: Option<String>,
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
    #[serde(rename = "format_name")]
    format_name: Option<String>,
    #[serde(rename = "bit_rate")]
    bit_rate: Option<String>,
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

/// Parse ffprobe JSON output into VideoMetadata.
/// Public so integration tests can use it directly.
pub fn parse_ffprobe_json(bytes: &[u8]) -> Result<VideoMetadata, String> {
    let document: ProbeDocument =
        serde_json::from_slice(bytes).map_err(|e| format!("ffprobe JSON parse error: {}", e))?;

    let video = document
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));

    let audio = document
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    if let Some(video) = video {
        let width = video.width.filter(|w| *w > 0).unwrap_or(0);
        let height = video.height.filter(|h| *h > 0).unwrap_or(0);

        let rotation = video
            .side_data_list
            .iter()
            .find_map(|d| d.rotation)
            .or_else(|| {
                video
                    .tags
                    .as_ref()
                    .and_then(|t| t.rotate.as_deref())
                    .and_then(|v| v.parse().ok())
            })
            .map(normalize_rotation)
            .unwrap_or(0);

        let duration = video
            .duration
            .as_deref()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                document
                    .format
                    .as_ref()
                    .and_then(|f| f.duration.as_deref())
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(0.0);

        let bitrate = document
            .format
            .as_ref()
            .and_then(|f| f.bit_rate.as_deref())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        let fps = video
            .r_frame_rate
            .as_deref()
            .and_then(|fps_str| {
                let parts: Vec<&str> = fps_str.split('/').collect();
                if parts.len() == 2 {
                    let num: f64 = parts[0].parse().ok()?;
                    let den: f64 = parts[1].parse().ok()?;
                    if den > 0.0 {
                        Some(num / den)
                    } else {
                        None
                    }
                } else if parts.len() == 1 {
                    parts[0].parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0.0);

        let container = document
            .format
            .as_ref()
            .and_then(|f| f.format_name.as_deref())
            .map(|n| n.split(',').next().unwrap_or(n).to_string())
            .unwrap_or_default();

        Ok(VideoMetadata {
            container,
            video_codec: video
                .codec_name
                .as_ref()
                .map(|n| n.to_ascii_lowercase())
                .unwrap_or_default(),
            audio_codec: audio
                .and_then(|a| a.codec_name.as_deref())
                .map(|n| n.to_ascii_lowercase())
                .unwrap_or_default(),
            duration,
            width,
            height,
            bitrate,
            is_valid: width > 0 && height > 0,
            rotation,
            file_size: 0,
            color_transfer: video.color_transfer.clone().unwrap_or_default().to_ascii_lowercase(),
            color_primaries: video.color_primaries.clone().unwrap_or_default().to_ascii_lowercase(),
            profile: video.profile.clone().unwrap_or_default(),
            pixel_format: video.pix_fmt.clone().unwrap_or_default().to_ascii_lowercase(),
            fps,
        })
    } else {
        // No video stream
        Ok(VideoMetadata {
            container: document
                .format
                .as_ref()
                .and_then(|f| f.format_name.as_deref())
                .map(|n| n.split(',').next().unwrap_or(n).to_string())
                .unwrap_or_default(),
            video_codec: String::new(),
            audio_codec: audio
                .and_then(|a| a.codec_name.as_deref())
                .map(|n| n.to_ascii_lowercase())
                .unwrap_or_default(),
            duration: 0.0,
            width: 0,
            height: 0,
            bitrate: 0,
            is_valid: false,
            rotation: 0,
            file_size: 0,
            color_transfer: String::new(),
            color_primaries: String::new(),
            profile: String::new(),
            pixel_format: String::new(),
            fps: 0.0,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Fake process runner for testing
    struct FakeProcessRunner {
        responses: Mutex<Vec<Result<ProcessOutput, String>>>,
        #[allow(dead_code)]
        call_args: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl FakeProcessRunner {
        fn new(responses: Vec<Result<ProcessOutput, String>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                call_args: Mutex::new(vec![]),
            }
        }
    }

    impl ProcessRunner for FakeProcessRunner {
        fn run_command(
            &self,
            program: &str,
            args: &[String],
        ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, String>> + Send + '_>> {
            self.call_args
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));

            let response = self.responses.lock().unwrap().remove(0);

            Box::pin(async move { response })
        }
    }

    fn sample_ffprobe_output() -> Vec<u8> {
        r#"{
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1920,
                    "height": 1080,
                    "duration": "120.500000",
                    "side_data_list": [{"rotation": 90}]
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac"
                }
            ],
            "format": {
                "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
                "duration": "120.500000",
                "bit_rate": "5000000"
            }
        }"#
        .as_bytes()
        .to_vec()
    }

    fn adapter() -> FFmpegMediaAdapter {
        let runner = Arc::new(FakeProcessRunner::new(vec![Ok(ProcessOutput {
            exit_code: 0,
            stdout: sample_ffprobe_output(),
            stderr: vec![],
        })]));

        FFmpegMediaAdapter::new_with_runner(
            PathBuf::from("ffprobe"),
            PathBuf::from("ffmpeg"),
            runner,
            Arc::new(AtomicBool::new(false)),
            2,
        )
    }

    #[tokio::test]
    async fn test_parse_ffprobe_json() {
        let meta = parse_ffprobe_json(&sample_ffprobe_output()).unwrap();

        assert_eq!(meta.container, "mov");
        assert_eq!(meta.video_codec, "h264");
        assert_eq!(meta.audio_codec, "aac");
        assert_eq!(meta.width, 1920);
        assert_eq!(meta.height, 1080);
        assert_eq!(meta.duration, 120.5);
        assert_eq!(meta.bitrate, 5_000_000);
        assert_eq!(meta.rotation, 90);
        assert!(meta.is_valid);
    }

    #[tokio::test]
    async fn test_ffprobe_json_malformed() {
        let result = parse_ffprobe_json(b"not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parse error"));
    }

    #[tokio::test]
    async fn test_ffprobe_no_video_stream() {
        let json = r#"{"streams":[{"codec_type":"audio","codec_name":"aac"}],"format":{"format_name":"m4a"}}"#;
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert!(!meta.is_valid);
        assert!(meta.video_codec.is_empty());
        assert_eq!(meta.audio_codec, "aac");
    }

    #[tokio::test]
    async fn test_inspect_file_success() {
        let a = adapter();
        let meta = a.inspect_file(Path::new("test.mp4")).await.unwrap();
        assert_eq!(meta.video_codec, "h264");
        assert_eq!(meta.width, 1920);
        assert!(meta.is_valid);
    }

    #[tokio::test]
    async fn test_inspect_file_cancelled() {
        let runner = Arc::new(FakeProcessRunner::new(vec![]));
        let a = FFmpegMediaAdapter::new_with_runner(
            PathBuf::from("ffprobe"),
            PathBuf::from("ffmpeg"),
            runner,
            Arc::new(AtomicBool::new(true)), // Already cancelled
            2,
        );

        let result = a.inspect_file(Path::new("test.mp4")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cancelled"));
    }

    #[tokio::test]
    async fn test_inspect_file_nonzero_exit() {
        let runner = Arc::new(FakeProcessRunner::new(vec![Ok(ProcessOutput {
            exit_code: 1,
            stdout: vec![],
            stderr: b"Invalid data found".to_vec(),
        })]));
        let a = FFmpegMediaAdapter::new_with_runner(
            PathBuf::from("ffprobe"),
            PathBuf::from("ffmpeg"),
            runner,
            Arc::new(AtomicBool::new(false)),
            2,
        );

        let result = a.inspect_file(Path::new("corrupt.mp4")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-zero exit"));
    }

    #[tokio::test]
    async fn test_process_video_transcode_args() {
        let args = FFmpegMediaAdapter::build_transcode_args(
            Path::new("/tmp/in.mp4"),
            Path::new("/tmp/out.mp4"),
            2,
        );
        assert!(args.contains(&"-c:v".to_string()));

        assert!(args.contains(&"-c:a".to_string()));
        assert!(args.contains(&"aac".to_string()));
        assert!(args.contains(&"-threads".to_string()));
        assert!(args.contains(&"2".to_string()));
        // No shell injection
        assert!(!args
            .iter()
            .any(|a| a.contains("&&") || a.contains("|") || a.contains(";")));
    }

    #[tokio::test]
    async fn test_process_video_remux_args() {
        let args = FFmpegMediaAdapter::build_remux_args(
            Path::new("/tmp/in.mkv"),
            Path::new("/tmp/out.mp4"),
        );
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"copy".to_string()));
        assert!(args.contains(&"-movflags".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
        // No shell injection
        assert!(!args
            .iter()
            .any(|a| a.contains("&&") || a.contains("|") || a.contains(";")));
    }

    #[tokio::test]
    async fn test_process_video_cancelled() {
        let runner = Arc::new(FakeProcessRunner::new(vec![]));
        let cancel = Arc::new(AtomicBool::new(true));
        let a = FFmpegMediaAdapter::new_with_runner(
            PathBuf::from("ffprobe"),
            PathBuf::from("ffmpeg"),
            runner,
            cancel,
            2,
        );

        let result = a
            .process_video(Path::new("in.mp4"), Path::new("out.mp4"), "transcode")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cancelled"));
    }

    #[tokio::test]
    async fn test_process_video_nonzero_exit() {
        let runner = Arc::new(FakeProcessRunner::new(vec![Ok(ProcessOutput {
            exit_code: 1,
            stdout: vec![],
            stderr: b"Error while filtering".to_vec(),
        })]));
        let cancel = Arc::new(AtomicBool::new(false));
        let a = FFmpegMediaAdapter::new_with_runner(
            PathBuf::from("ffprobe"),
            PathBuf::from("ffmpeg"),
            runner,
            cancel,
            2,
        );

        let result = a
            .process_video(Path::new("in.mp4"), Path::new("out.mp4"), "transcode")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-zero exit"));
    }

    #[tokio::test]
    async fn test_process_video_unsupported_decision() {
        let runner = Arc::new(FakeProcessRunner::new(vec![]));
        let cancel = Arc::new(AtomicBool::new(false));
        let a = FFmpegMediaAdapter::new_with_runner(
            PathBuf::from("ffprobe"),
            PathBuf::from("ffmpeg"),
            runner,
            cancel,
            2,
        );

        let result = a
            .process_video(Path::new("in.mp4"), Path::new("out.mp4"), "unknown")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported decision"));
    }

    #[tokio::test]
    async fn test_passthrough_rejected_by_processor() {
        let runner = Arc::new(FakeProcessRunner::new(vec![]));
        let cancel = Arc::new(AtomicBool::new(false));
        let a = FFmpegMediaAdapter::new_with_runner(
            PathBuf::from("ffprobe"),
            PathBuf::from("ffmpeg"),
            runner,
            cancel,
            2,
        );

        let result = a
            .process_video(Path::new("in.mp4"), Path::new("out.mp4"), "passthrough")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported decision"));
    }

    #[tokio::test]
    async fn test_thread_limit_in_args() {
        let args = FFmpegMediaAdapter::build_transcode_args(
            Path::new("/tmp/in.mp4"),
            Path::new("/tmp/out.mp4"),
            1,
        );
        let thread_pos = args.iter().position(|a| a == "-threads").unwrap();
        assert_eq!(args[thread_pos + 1], "1");
    }

    #[tokio::test]
    async fn test_no_shell_injection_in_args() {
        let dangerous = "/tmp/file; rm -rf /";
        let args = FFmpegMediaAdapter::build_transcode_args(
            Path::new(dangerous),
            Path::new("/tmp/out.mp4"),
            2,
        );
        // The args are in a Vec, not a shell string
        assert!(args.contains(&dangerous.to_string()));
    }
}
