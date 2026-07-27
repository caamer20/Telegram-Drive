// FFmpeg production adapters for Pipeline V2
// Inspector: wraps ffprobe to populate VideoMetadata
// Processor: wraps ffmpeg for remux/transcode
//
// Test seam: the `ProcessRunner` trait allows injecting a fake process runner
// for automated tests without requiring FFmpeg on the test machine.

use crate::migration::pipeline::stages::{
    validate_canonical_output, CanonicalVideoProfile, MediaInspector, VideoMetadata, VideoProcessor,
};
use serde::Deserialize;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
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
        cancel_token: tokio_util::sync::CancellationToken,
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
        cancel_token: tokio_util::sync::CancellationToken,
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
                    if n == 0 {
                        break;
                    }
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
                    if n == 0 {
                        break;
                    }
                    if full_output.len() < 1024 * 1024 {
                        full_output.extend_from_slice(&buf[..n]);
                    }
                }
                full_output
            });

            tokio::select! {
                status = child.wait() => {
                    let status = status.map_err(|e| format!("Wait error: {}", e))?;
                    let stdout_out = stdout_task.await.unwrap_or_default();
                    let stderr_out = stderr_task.await.unwrap_or_default();

                    Ok(ProcessOutput {
                        exit_code: status.code().unwrap_or(-1),
                        stdout: stdout_out,
                        stderr: stderr_out,
                    })
                }
                _ = cancel_token.cancelled() => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Err("ProcessRunner: Cancelled".to_string())
                }
            }
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
    cancel_token: tokio_util::sync::CancellationToken,
    app_handle: Option<tauri::AppHandle>,
    hevc_encoder: String,
}

impl FFmpegMediaAdapter {
    pub fn new(
        ffprobe_path: PathBuf,
        ffmpeg_path: PathBuf,
        cancel_token: tokio_util::sync::CancellationToken,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        let mut hevc_encoder = "libx265".to_string();
        if cfg!(target_os = "macos") {
            if let Ok(output) = std::process::Command::new(&ffmpeg_path)
                .args(["-encoders"])
                .output()
            {
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
            app_handle,
            hevc_encoder,
        }
    }

    /// Constructor for tests — accepts injectable process runner
    pub fn new_with_runner(
        ffprobe_path: PathBuf,
        ffmpeg_path: PathBuf,
        process_runner: Arc<dyn ProcessRunner>,
        cancel_token: tokio_util::sync::CancellationToken,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            ffprobe_path,
            ffmpeg_path,
            process_runner,
            cancel_token,
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

    fn build_transcode_args(
        input_path: &Path,
        output_path: &Path,
        is_10bit: bool,
        encoder: &str,
        source_fps: f64,
    ) -> Vec<String> {
        let mut filter_chain = "scale=w='min(1920,iw)':h='min(1080,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2".to_string();
        if source_fps > 60.0 {
            filter_chain = format!("{},fps=60", filter_chain);
        }

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
            filter_chain,
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
        args.push("-tag:v".to_string());
        args.push("hvc1".to_string());

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
            if cancel.is_cancelled() {
                return Err("Inspector: cancelled".to_string());
            }

            let output = runner
                .run_command(&ffprobe, &args, None, cancel.clone())
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
        input_path: &std::path::Path,
        output_path: &std::path::Path,
        decision: &str,
        item_id: i64,
        job_id: i64,
        duration: f64,
        source_fps: f64,
        item_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        let ffmpeg = self.ffmpeg_path.to_string_lossy().to_string();
        let is_10bit = match decision {
            "canonical_transcode_main10" => true,
            _ => false,
        };
        let args = match decision {
            "canonical_transcode_main8" | "transcode" => Self::build_transcode_args(
                input_path,
                output_path,
                false,
                &self.hevc_encoder,
                source_fps,
            ),
            "canonical_transcode_main10" => Self::build_transcode_args(
                input_path,
                output_path,
                true,
                &self.hevc_encoder,
                source_fps,
            ),
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
        let ffprobe_path = self.ffprobe_path.to_string_lossy().to_string();
        let process_runner = self.process_runner.clone();
        let source_fps_val = source_fps;

        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err("Processor: cancelled".to_string());
            }

            let on_progress: Option<Arc<dyn Fn(&str) + Send + Sync>> = if let Some(app) = app_handle
            {
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
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64;
                            let last = last_emit_ms.load(std::sync::atomic::Ordering::Relaxed);
                            if now_ms - last >= 250 {
                                last_emit_ms.store(now_ms, std::sync::atomic::Ordering::Relaxed);
                                let payload = serde_json::json!({
                                    "item_id": item_id,
                                    "job_id": job_id,
                                    "item_name": item_name,
                                    "phase": "processing",
                                    "percent": percent,
                                    "bytes_done": (time_us / 1_000_000.0 * source_fps_val) as u64,
                                    "bytes_total": (duration_us / 1_000_000.0 * source_fps_val) as u64,
                                    "speed_bytes_per_sec": 0.0,
                                    "timestamp": now_ms as i64,
                                });
                                let _ = app.emit("migration:item-progress", payload);
                            }
                        }
                    }
                }))
            } else {
                None
            };

            let output = runner
                .run_command(&ffmpeg, &args, on_progress, cancel.clone())
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
            let mut file = tokio::fs::File::open(&output_path_owned)
                .await
                .map_err(|e| format!("Processor: cannot open output: {}", e))?;
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                use tokio::io::AsyncReadExt;
                let count = file
                    .read(&mut buffer)
                    .await
                    .map_err(|e| format!("Processor: error reading output: {}", e))?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            let hash = hasher.finalize();
            let hash_hex = format!("{:x}", hash);

            // Validate output with FFprobe
            let ffprobe_args = Self::build_ffprobe_args(&output_path_owned);
            let probe_result = process_runner
                .run_command(&ffprobe_path, &ffprobe_args, None, cancel.clone())
                .await;
            match probe_result {
                Ok(probe_output) if probe_output.exit_code == 0 => {
                    match parse_ffprobe_json(&probe_output.stdout) {
                        Ok(output_meta) => {
                            let expected_profile = if is_10bit {
                                CanonicalVideoProfile::Main10
                            } else {
                                CanonicalVideoProfile::Main8
                            };
                            // Create source metadata with just duration for tolerance check
                            let source_meta = VideoMetadata {
                                duration,
                                fps: source_fps_val,
                                ..Default::default()
                            };
                            if let Err(validation_err) = validate_canonical_output(
                                &source_meta,
                                &output_meta,
                                expected_profile,
                            ) {
                                let _ = tokio::fs::remove_file(&output_path_owned).await;
                                return Err(format!(
                                    "Processor: output validation failed: {}",
                                    validation_err
                                ));
                            }
                        }
                        Err(e) => {
                            let _ = tokio::fs::remove_file(&output_path_owned).await;
                            return Err(format!("Processor: ffprobe on output failed: {}", e));
                        }
                    }
                }
                Ok(probe_output) => {
                    let _ = tokio::fs::remove_file(&output_path_owned).await;
                    let stderr = String::from_utf8_lossy(&probe_output.stderr).to_string();
                    return Err(format!(
                        "Processor: ffprobe on output non-zero exit: {} — {}",
                        probe_output.exit_code, stderr
                    ));
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&output_path_owned).await;
                    return Err(format!("Processor: ffprobe on output failed: {}", e));
                }
            }

            Ok(hash_hex)
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
            .unwrap_or_default()
            .to_string();

        Ok(VideoMetadata {
            container_format_names: container,
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
            color_transfer: video
                .color_transfer
                .clone()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            color_primaries: video
                .color_primaries
                .clone()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            profile: video.profile.clone().unwrap_or_default(),
            pixel_format: video
                .pix_fmt
                .clone()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            fps,
        })
    } else {
        // No video stream
        Ok(VideoMetadata {
            container_format_names: document
                .format
                .as_ref()
                .and_then(|f| f.format_name.as_deref())
                .unwrap_or_default()
                .to_string(),
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
    use crate::migration::pipeline::stages::{validate_canonical_output, CanonicalVideoProfile};

    fn make_ffprobe_json(
        video_codec: &str,
        profile: &str,
        pix_fmt: &str,
        format_name: &str,
        audio_codec: &str,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
        duration: f64,
        color_transfer: &str,
    ) -> String {
        serde_json::json!({
            "streams": [{
                "codec_type": "video",
                "codec_name": video_codec,
                "profile": profile,
                "pix_fmt": pix_fmt,
                "width": width,
                "height": height,
                "duration": duration.to_string(),
                "r_frame_rate": format!("{}/{}", fps_num, fps_den),
                "color_transfer": color_transfer,
            }, {
                "codec_type": "audio",
                "codec_name": audio_codec,
            }],
            "format": {
                "format_name": format_name,
                "duration": duration.to_string(),
                "bit_rate": "1000000"
            }
        })
        .to_string()
    }

    #[test]
    fn test_parse_hevc_main8_mp4() {
        let json = make_ffprobe_json(
            "hevc",
            "Main",
            "yuv420p",
            "mov,mp4,m4a,3gp,3g2,mj2",
            "aac",
            1920,
            1080,
            30000,
            1001,
            60.0,
            "",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert!(meta.is_mp4_compatible());
        assert!(meta.is_canonical_main8());
        assert_eq!(meta.fps, 30000.0 / 1001.0);
        assert_eq!(meta.container_format_names, "mov,mp4,m4a,3gp,3g2,mj2");
    }

    #[test]
    fn test_parse_hevc_main10_mp4() {
        let json = make_ffprobe_json(
            "hevc",
            "Main 10",
            "yuv420p10le",
            "mov,mp4,m4a,3gp,3g2,mj2",
            "aac",
            1920,
            1080,
            24000,
            1001,
            120.0,
            "smpte2084",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert!(meta.is_mp4_compatible());
        assert!(meta.is_canonical_main10());
        assert!(meta.is_hdr());
        assert!(meta.is_10bit());
    }

    #[test]
    fn test_parse_h264_not_canonical() {
        let json = make_ffprobe_json(
            "h264", "High", "yuv420p", "mp4", "aac", 1920, 1080, 30000, 1001, 60.0, "",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert!(!meta.is_canonical_main8());
    }

    #[test]
    fn test_container_not_mp4_family() {
        let json = make_ffprobe_json(
            "hevc",
            "Main",
            "yuv420p",
            "matroska,webm",
            "aac",
            1920,
            1080,
            30000,
            1001,
            60.0,
            "",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert!(!meta.is_mp4_compatible());
        assert!(!meta.is_canonical_main8());
    }

    #[test]
    fn test_opus_audio_not_canonical() {
        let json = make_ffprobe_json(
            "hevc", "Main", "yuv420p", "mp4", "opus", 1920, 1080, 30000, 1001, 60.0, "",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert!(!meta.is_canonical_main8());
    }

    #[test]
    fn test_4k_not_canonical() {
        let json = make_ffprobe_json(
            "hevc", "Main", "yuv420p", "mp4", "aac", 3840, 2160, 30000, 1001, 60.0, "",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert!(!meta.is_canonical_main8());
    }

    #[test]
    fn test_120fps_not_canonical() {
        let json = make_ffprobe_json(
            "hevc", "Main", "yuv420p", "mp4", "aac", 1920, 1080, 120, 1, 60.0, "",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        assert_eq!(meta.fps, 120.0);
        assert!(!meta.is_canonical_main8());
    }

    #[test]
    fn test_parse_fps_fraction() {
        let json = make_ffprobe_json(
            "hevc", "Main", "yuv420p", "mp4", "aac", 1920, 1080, 24000, 1001, 60.0, "",
        );
        let meta = parse_ffprobe_json(json.as_bytes()).unwrap();
        let expected = 24000.0 / 1001.0;
        assert!((meta.fps - expected).abs() < 0.01);
    }

    #[test]
    fn test_main8_args_24fps() {
        let args = FFmpegMediaAdapter::build_transcode_args(
            std::path::Path::new("in.mp4"),
            std::path::Path::new("out.mp4"),
            false,
            "libx265",
            24.0,
        );
        let args_str = args.join(" ");
        assert!(
            !args_str.contains("fps=60"),
            "24fps should not get fps filter: {}",
            args_str
        );
        assert!(args_str.contains("hvc1"));
        assert!(args_str.contains("yuv420p"));
        assert!(args_str.contains("main"));
        assert!(args_str.contains("+faststart"));
    }

    #[test]
    fn test_main8_args_120fps() {
        let args = FFmpegMediaAdapter::build_transcode_args(
            std::path::Path::new("in.mp4"),
            std::path::Path::new("out.mp4"),
            false,
            "libx265",
            120.0,
        );
        let args_str = args.join(" ");
        assert!(
            args_str.contains("fps=60"),
            "120fps should get fps=60 filter: {}",
            args_str
        );
    }

    #[test]
    fn test_main8_args_60fps() {
        let args = FFmpegMediaAdapter::build_transcode_args(
            std::path::Path::new("in.mp4"),
            std::path::Path::new("out.mp4"),
            false,
            "libx265",
            60.0,
        );
        let args_str = args.join(" ");
        assert!(
            !args_str.contains("fps=60"),
            "60fps should not get fps filter: {}",
            args_str
        );
    }

    #[test]
    fn test_main10_args() {
        let args = FFmpegMediaAdapter::build_transcode_args(
            std::path::Path::new("in.mp4"),
            std::path::Path::new("out.mp4"),
            true,
            "libx265",
            30.0,
        );
        let args_str = args.join(" ");
        assert!(args_str.contains("yuv420p10le"));
        assert!(args_str.contains("main10"));
        assert!(args_str.contains("hvc1"));
    }

    #[test]
    fn test_videotoolbox_args() {
        let args = FFmpegMediaAdapter::build_transcode_args(
            std::path::Path::new("in.mp4"),
            std::path::Path::new("out.mp4"),
            false,
            "hevc_videotoolbox",
            30.0,
        );
        let args_str = args.join(" ");
        assert!(args_str.contains("hevc_videotoolbox"));
        assert!(args_str.contains("hvc1"));
        assert!(
            !args_str.contains("preset"),
            "VideoToolbox should not use preset"
        );
        assert!(!args_str.contains("crf"), "VideoToolbox should not use crf");
    }

    #[test]
    fn test_validate_canonical_main8_pass() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            video_codec: "hevc".into(),
            profile: "Main".into(),
            pixel_format: "yuv420p".into(),
            audio_codec: "aac".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_ok());
    }

    #[test]
    fn test_validate_canonical_main10_pass() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "hevc".into(),
            profile: "Main10".into(),
            pixel_format: "yuv420p10le".into(),
            audio_codec: "aac".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main10).is_ok());
    }

    #[test]
    fn test_validate_h264_output_fails() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "h264".into(),
            profile: "High".into(),
            pixel_format: "yuv420p".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_err());
    }

    #[test]
    fn test_validate_opus_audio_fails() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "hevc".into(),
            profile: "Main".into(),
            pixel_format: "yuv420p".into(),
            audio_codec: "opus".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_err());
    }

    #[test]
    fn test_validate_4k_output_fails() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "hevc".into(),
            profile: "Main".into(),
            pixel_format: "yuv420p".into(),
            audio_codec: "aac".into(),
            width: 3840,
            height: 2160,
            fps: 30.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_err());
    }

    #[test]
    fn test_validate_120fps_output_fails() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "hevc".into(),
            profile: "Main".into(),
            pixel_format: "yuv420p".into(),
            audio_codec: "aac".into(),
            width: 1920,
            height: 1080,
            fps: 120.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_err());
    }

    #[test]
    fn test_validate_duration_mismatch_fails() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "hevc".into(),
            profile: "Main".into(),
            pixel_format: "yuv420p".into(),
            audio_codec: "aac".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 30.0, // Half the source duration
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_err());
    }

    #[test]
    fn test_validate_wrong_profile_fails() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "hevc".into(),
            profile: "Main10".into(),
            pixel_format: "yuv420p".into(),
            audio_codec: "aac".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_err());
    }

    #[test]
    fn test_no_audio_is_valid() {
        let source = VideoMetadata {
            duration: 60.0,
            ..Default::default()
        };
        let output = VideoMetadata {
            container_format_names: "mp4".into(),
            video_codec: "hevc".into(),
            profile: "Main".into(),
            pixel_format: "yuv420p".into(),
            audio_codec: "".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 60.0,
            is_valid: true,
            ..Default::default()
        };
        assert!(validate_canonical_output(&source, &output, CanonicalVideoProfile::Main8).is_ok());
    }
}
