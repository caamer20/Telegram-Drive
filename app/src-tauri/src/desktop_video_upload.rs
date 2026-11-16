//! Desktop media preparation and publication; original video bytes are never rewritten.
pub(crate) struct VideoUploadMetadata {
    pub(crate) duration: std::time::Duration,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) mime_type: &'static str,
    thumbnail: Vec<u8>,
    supports_streaming: bool,
    source: std::path::PathBuf,
    fingerprint: crate::resumable_upload::SourceFingerprint,
    reservation: std::sync::Mutex<Option<crate::bandwidth::BandwidthReservation>>,
}

fn is_mp4_family_video(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "mp4" | "m4v" | "mov"
    )
}

async fn prepare(
    source_path: &str,
    upload_name: &str,
    media_mode: bool,
    mime_type: &'static str,
    resource: Option<std::path::PathBuf>,
    deadline: Instant,
) -> Result<Option<VideoUploadMetadata>, String> {
    if !media_mode || !is_mp4_family_video(upload_name) {
        return Ok(None);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let _stop = StopOnDrop(stop.clone());
    let permit = tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        DECODERS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| preview_error("Timed out waiting for the video preview decoder"))?
    .map_err(|_| unavailable())?;
    let source = std::path::PathBuf::from(source_path);
    let fingerprint = crate::resumable_upload::SourceFingerprint::of(&source).await?;
    let source_path = source_path.to_string();
    let inspection_stop = stop.clone();
    let (parsed, permit) = tokio::task::spawn_blocking(move || {
        let mut source = std::fs::File::open(&source_path).map_err(|error| {
            format!("Could not open the video for metadata inspection: {error}")
        })?;
        let (metadata, supports_streaming) = metadata_atoms(&mut source, &inspection_stop, deadline)?;
        let context = mp4parse::read_mp4(&mut std::io::Cursor::new(metadata)).map_err(|error| format!("Could not read MP4 video metadata: {error}"))?;
        let track = context
            .tracks
            .iter()
            .find(|track| track.track_type == mp4parse::TrackType::Video)
            .ok_or_else(|| "The selected file does not contain a video track".to_string())?;

        let duration_secs = match (track.duration.as_ref(), track.timescale.as_ref()) {
            (Some(duration), Some(timescale)) if timescale.0 > 0 => {
                duration.0 as f64 / timescale.0 as f64
            }
            _ => 0.0,
        };
        let track_header = track
            .tkhd
            .as_ref()
            .ok_or_else(|| "The video does not contain display dimensions".to_string())?;
        let mut width = track_header.width >> 16;
        let mut height = track_header.height >> 16;
        if track_header.matrix.b != 0 || track_header.matrix.c != 0 {
            std::mem::swap(&mut width, &mut height);
        }
        if width == 0 || height == 0 {
            return Err("The video has invalid display dimensions".to_string());
        }
        Ok::<_, String>(((duration_secs, width, height, supports_streaming), permit))
    })
    .await
    .map_err(|error| format!("Video metadata inspection failed: {error}"))?
    .map_err(|error| {
        format!(
            "[VIDEO_METADATA_UNAVAILABLE] {error}. Choose File mode to upload this video without an inline preview"
        )
    })?;

    let thumbnail = thumbnail(
        source.to_str().ok_or_else(unavailable)?,
        i32::try_from(parsed.1).unwrap_or(i32::MAX),
        i32::try_from(parsed.2).unwrap_or(i32::MAX),
        resource,
        deadline,
        stop,
        permit,
    )
    .await?;
    if crate::resumable_upload::SourceFingerprint::of(&source).await? != fingerprint {
        return Err(source_changed());
    }
    Ok(Some(VideoUploadMetadata {
        duration: if parsed.0.is_finite() && parsed.0 >= 0.0 {
            std::time::Duration::from_secs_f64(parsed.0)
        } else {
            std::time::Duration::ZERO
        },
        width: i32::try_from(parsed.1).unwrap_or(i32::MAX),
        height: i32::try_from(parsed.2).unwrap_or(i32::MAX),
        mime_type,
        thumbnail,
        supports_streaming: parsed.3,
        source,
        fingerprint,
        reservation: std::sync::Mutex::new(None),
    }))
}

/// The wire payload shared by both upload entry points.
pub(crate) fn media(
    video: &VideoUploadMetadata,
    file: grammers_client::types::media::Uploaded,
    thumbnail: grammers_client::types::media::Uploaded,
) -> grammers_tl_types::types::InputMediaUploadedDocument {
    use grammers_client::types::Attribute;
    let filename = match &file.raw {
        grammers_tl_types::enums::InputFile::File(file) => file.name.clone(),
        grammers_tl_types::enums::InputFile::Big(file) => file.name.clone(),
        grammers_tl_types::enums::InputFile::StoryDocument(_) => String::new(),
    };
    grammers_tl_types::types::InputMediaUploadedDocument {
        nosound_video: false,
        force_file: false,
        spoiler: false,
        attributes: vec![
            Attribute::FileName(filename).into(),
            Attribute::Video {
                round_message: false,
                supports_streaming: video.supports_streaming,
                duration: video.duration,
                w: video.width,
                h: video.height,
            }
            .into(),
        ],
        file: file.raw,
        thumb: Some(thumbnail.raw),
        mime_type: video.mime_type.to_string(),
        stickers: None,
        ttl_seconds: None,
        video_cover: None,
        video_timestamp: None,
    }
}
/// Only the Telegram boundary is replaceable. The application and native
/// journeys share byte pacing, quota, account/cancellation and wire construction.
pub(crate) type ThumbnailReader = crate::traffic::Reader<std::io::Cursor<Vec<u8>>>;
pub(crate) struct Publication {
    pub(crate) media: grammers_tl_types::types::InputMediaUploadedDocument,
    pub(crate) caption: String,
}
pub(crate) trait Transport: Sync {
    type Receipt;
    fn upload_thumbnail(
        &self,
        reader: ThumbnailReader,
        len: usize,
    ) -> impl std::future::Future<Output = Result<grammers_client::types::media::Uploaded, String>> + Send;
    fn publish(
        &self,
        payload: Publication,
    ) -> impl std::future::Future<Output = Result<Self::Receipt, String>> + Send;
}
pub(crate) struct TelegramTransport<'a> {
    pub(crate) client: &'a grammers_client::Client,
    pub(crate) peer: &'a grammers_client::types::Peer,
}
impl Transport for TelegramTransport<'_> {
    type Receipt = grammers_client::types::Message;
    async fn upload_thumbnail(
        &self,
        mut reader: ThumbnailReader,
        len: usize,
    ) -> Result<grammers_client::types::media::Uploaded, String> {
        self.client
            .upload_stream(&mut reader, len, "video-preview.jpg".into())
            .await
            .map_err(crate::commands::utils::map_error)
    }
    async fn publish(&self, payload: Publication) -> Result<Self::Receipt, String> {
        self.client
            .send_message(
                self.peer,
                grammers_client::InputMessage::new()
                    .text(payload.caption)
                    .media(payload.media),
            )
            .await
            .map_err(crate::commands::utils::map_error)
    }
}
/// A publication attempt uses freshly uploaded thumbnail parts: they are never
/// reused across retries, while the original movie bytes remain untouched.
#[allow(clippy::too_many_arguments)] // Carries the captured account, traffic policy and cancellation lease.
pub(crate) async fn publish<T: Transport>(
    video: &VideoUploadMetadata,
    file: grammers_client::types::media::Uploaded,
    caption: String,
    transport: &T,
    account: &crate::workspace::AccountGuard,
    bandwidth: std::sync::Arc<crate::bandwidth::BandwidthManager>,
    network: std::sync::Arc<crate::vpn_optimizer::NetworkConfig>,
    cancel: &mut tokio::sync::oneshot::Receiver<()>,
) -> Result<T::Receipt, String> {
    account.validate()?;
    let attempt = async {
        validate_source(video).await?;
        let bytes = &video.thumbnail;
        let admitted = {
            video
                .reservation
                .lock()
                .map_err(|_| "Video upload admission is unavailable".to_string())?
                .take()
        };
        let mut reservation = match admitted {
            Some(hold) => hold,
            None => crate::bandwidth::BandwidthReservation::upload(bandwidth, bytes.len() as u64)?,
        };
        let reader = crate::traffic::Reader::new(
            std::io::Cursor::new(bytes.clone()),
            crate::traffic::Traffic {
                network,
                account: account.clone(),
                direction: crate::traffic::Direction::Upload,
            },
        );
        let thumb = transport.upload_thumbnail(reader, bytes.len()).await?;
        account.validate()?;
        validate_source(video).await?;
        account.validate()?;
        let receipt = transport
            .publish(Publication {
                media: media(video, file, thumb),
                caption,
            })
            .await?;
        reservation.commit();
        Ok(receipt)
    };
    tokio::select! {
        biased;
        _ = cancel => Err("Transfer cancelled".into()),
        result = async { loop { account.validate()?; tokio::time::sleep(std::time::Duration::from_millis(50)).await; } #[allow(unreachable_code)] Ok::<(), String>(()) } => { result?; unreachable!() },
        result = attempt => result,
    }
}
pub(crate) fn caption(filename: &str, enabled: Option<bool>) -> String {
    if !enabled.unwrap_or(true) {
        return String::new();
    }
    let mut units = 0;
    filename
        .chars()
        .take_while(|character| {
            units += character.len_utf16();
            units <= 1024
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // Captures source, presentation and account-bound cancellation.
pub(crate) async fn prepare_for_account(
    source: &str,
    name: &str,
    media_mode: bool,
    mime: &'static str,
    resource: Option<std::path::PathBuf>,
    bandwidth: std::sync::Arc<crate::bandwidth::BandwidthManager>,
    account: &crate::workspace::AccountGuard,
    cancel: &mut tokio::sync::oneshot::Receiver<()>,
) -> Result<Option<VideoUploadMetadata>, String> {
    account.validate()?;
    let deadline = Instant::now() + Duration::from_secs(DECODE_SECONDS);
    let prepared = tokio::select! {
        biased;
        _ = cancel => Err("Transfer cancelled".into()),
        result = async { loop { account.validate()?; tokio::time::sleep(std::time::Duration::from_millis(50)).await; } #[allow(unreachable_code)] Ok::<(), String>(()) } => { result?; unreachable!() },
        result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), prepare(source, name, media_mode, mime, resource, deadline)) => result.map_err(|_| preview_error("Video preview preparation timed out"))?,
    }?;
    account.validate()?;
    if let Some(video) = prepared.as_ref() {
        let hold = crate::bandwidth::BandwidthReservation::upload(
            bandwidth,
            video.thumbnail.len() as u64,
        )?;
        *video
            .reservation
            .lock()
            .map_err(|_| "Video upload admission is unavailable".to_string())? = Some(hold);
    }
    Ok(prepared)
}
fn source_changed() -> String {
    "[VIDEO_SOURCE_CHANGED] The video changed while its preview or upload was being prepared. Retry the upload".into()
}
async fn validate_source(video: &VideoUploadMetadata) -> Result<(), String> {
    if crate::resumable_upload::SourceFingerprint::of(&video.source).await? == video.fingerprint {
        Ok(())
    } else {
        Err(source_changed())
    }
}
/// Parse only bounded metadata atoms. Movie payloads are skipped with seeks,
/// so large files do not need to be read through just to determine dimensions.
fn metadata_atoms(
    file: &mut std::fs::File,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<(Vec<u8>, bool), String> {
    use std::io::{Read, Seek, SeekFrom};
    const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
    let length = file
        .metadata()
        .map_err(|_| "Could not inspect the video layout".to_string())?
        .len();
    let mut offset = 0u64;
    let mut metadata = Vec::new();
    let mut previous_atom_start = 0u64;
    let mut streaming_decision = None;
    for _ in 0..65_536 {
        stopped(stop)?;
        if Instant::now() >= deadline {
            return Err("Video metadata inspection timed out".into());
        }
        if offset == length {
            return Ok((metadata, streaming_decision.unwrap_or(false)));
        }
        if length.saturating_sub(offset) < 8 {
            return Err("The video contains a truncated container box".into());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| "Could not inspect the video layout".to_string())?;
        let mut header = [0; 8];
        file.read_exact(&mut header)
            .map_err(|_| "Could not inspect the video layout".to_string())?;
        let short = u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|_| "Invalid video container box".to_string())?,
        );
        let (size, header_size) = match short {
            0 => (length - offset, 8),
            1 => {
                let mut extended = [0; 8];
                file.read_exact(&mut extended)
                    .map_err(|_| "The video contains a truncated container box".to_string())?;
                (u64::from_be_bytes(extended), 16)
            }
            size => (u64::from(size), 8),
        };
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= length)
            .ok_or_else(|| "The video contains an invalid container box".to_string())?;
        if size < header_size || end <= offset {
            return Err("The video contains an invalid container box".into());
        }
        // Telegram Desktop 7.3.1 checks the previous atom's start, not the
        // current moov offset: a large mdat may precede an eligible moov.
        // Latch that decision while still validating the complete container.
        if streaming_decision.is_none() {
            if previous_atom_start >= 128 * 1024 {
                streaming_decision = Some(false);
            } else if &header[4..] == b"moov" {
                streaming_decision = Some(true);
            }
        }
        if &header[4..] == b"moov" || &header[4..] == b"ftyp" {
            if metadata.len() as u64 + size > METADATA_LIMIT {
                return Err("The video metadata exceeds the safe inspection limit".into());
            }
            file.seek(SeekFrom::Start(offset))
                .map_err(|_| "Could not inspect the video layout".to_string())?;
            let start = metadata.len();
            metadata.resize(start + size as usize, 0);
            for chunk in metadata[start..].chunks_mut(64 * 1024) {
                stopped(stop)?;
                if Instant::now() >= deadline {
                    return Err("Video metadata inspection timed out".into());
                }
                file.read_exact(chunk)
                    .map_err(|_| "Could not read the video metadata".to_string())?;
            }
            // mp4parse expects explicit lengths; normalize only the private
            // in-memory view of a size-zero metadata atom, never the source.
            if short == 0 {
                metadata[start..start + 4].copy_from_slice(&(size as u32).to_be_bytes());
            }
        }
        previous_atom_start = offset;
        offset = end;
    }
    Err("The video contains too many container boxes".into())
}

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
    time::{Duration, Instant},
};
static DECODERS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(1)));
const DECODE_SECONDS: u64 = 30;
const FRAME_LIMIT: u64 = 2 * 1024 * 1024;
const THUMB_LIMIT: usize = 200 * 1024;
struct StopOnDrop(Arc<AtomicBool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn stopped(stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::Relaxed) {
        Err("Transfer cancelled".into())
    } else {
        Ok(())
    }
}
fn unavailable() -> String {
    preview_error("Could not create a video preview using this computer's media decoder")
}
fn preview_error(reason: &str) -> String {
    format!("[VIDEO_THUMBNAIL_UNAVAILABLE] {reason}. Choose File mode to upload without an inline preview")
}
async fn thumbnail(
    source: &str,
    width: i32,
    height: i32,
    resource: Option<PathBuf>,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> Result<Vec<u8>, String> {
    let source = PathBuf::from(source);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        stopped(&stop)?;
        let candidates = ffmpeg_candidates(resource.as_deref());
        #[cfg(target_os = "windows")]
        let (candidates, probe_failure) = ready_ffmpeg_candidates(candidates, deadline, &stop)?;
        #[cfg(target_os = "windows")]
        {
            // Native decoding retains the full remaining budget when no external
            // decoder exists. Otherwise reserve time for every known fallback.
            let native_deadline = attempt_deadline(deadline, candidates.len() + 1);
            let native = native_thumbnail(
                &source,
                width,
                height,
                resource.as_deref(),
                native_deadline,
                &stop,
            );
            if let Ok(bytes) = native.and_then(|bytes| normalize(&bytes)) {
                stopped(&stop)?;
                return Ok(bytes);
            }
        }
        stopped(&stop)?;
        let _ = (width, height);
        if Instant::now() >= deadline {
            return Err(preview_error("Video preview preparation timed out"));
        }
        #[cfg(target_os = "windows")]
        if candidates.is_empty() {
            if let Some(failure) = probe_failure {
                return Err(preview_error(failure));
            }
        }
        ffmpeg_thumbnail(&source, &candidates, deadline, &stop)
    })
    .await
    .map_err(|_| unavailable())?
}
/// Select the bundled decoder and first executable on the process's search path.
/// Absolute paths preserve spaces/Unicode and keep later working-directory
/// changes from altering selection. Bundled and PATH aliases share one attempt.
fn ffmpeg_candidates(resource: Option<&Path>) -> Vec<PathBuf> {
    let name = if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut candidates = Vec::new();
    if let Some(path) = resource.map(|directory| directory.join(name)) {
        if path.is_file() {
            if let Ok(path) = path.canonicalize() {
                candidates.push(path);
            }
        }
    }
    if let Some(search) = executable_search_path() {
        for directory in std::env::split_paths(&search) {
            let path = directory.join(name);
            if !path.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if !path
                    .metadata()
                    .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
                {
                    continue;
                }
            }
            if let Ok(path) = path.canonicalize() {
                if !candidates.contains(&path) {
                    candidates.push(path);
                }
                break;
            }
        }
    }
    candidates
}
fn executable_search_path() -> Option<std::ffi::OsString> {
    let search = std::env::var_os("PATH");
    if search.is_some() {
        return search;
    }
    // execvp uses the platform default when PATH is absent. An explicitly
    // supplied PATH is left unchanged, including an empty one.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::ffi::OsStringExt;
        let length = libc::confstr(libc::_CS_PATH, std::ptr::null_mut(), 0);
        if length == 0 || length > 64 * 1024 {
            return None;
        }
        let mut path = vec![0; length];
        let written = libc::confstr(libc::_CS_PATH, path.as_mut_ptr().cast(), length);
        if written == 0 || written > length {
            return None;
        }
        path.truncate(written - 1); // confstr includes the terminating NUL.
        Some(std::ffi::OsString::from_vec(path))
    }
    #[cfg(not(unix))]
    None
}
#[cfg(target_os = "windows")]
fn ready_ffmpeg_candidates(
    candidates: Vec<PathBuf>,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<(Vec<PathBuf>, Option<&'static str>), String> {
    let mut ready = Vec::new();
    let mut failure = None;
    for candidate in candidates {
        stopped(stop)?;
        if Instant::now() >= deadline {
            return Err(preview_error("Video preview preparation timed out"));
        }
        // File existence alone cannot detect an invalid PE, wrong architecture,
        // execution policy or failed job attachment. Probe under the same limits
        // before reserving native decoding time for this alternative.
        let mut command = std::process::Command::new(&candidate);
        command.arg("-version");
        let result = crate::process_budget::run(
            command,
            deadline.min(Instant::now() + Duration::from_secs(2)),
            false,
            || stopped(stop),
            None,
        );
        stopped(stop)?;
        match result {
            Ok(result) if result.success => ready.push(candidate),
            Ok(_) => failure = Some("FFmpeg was found but its startup check failed"),
            Err(error) => failure = Some(ffmpeg_failure(&error)),
        }
    }
    Ok((ready, failure))
}
fn attempt_deadline(deadline: Instant, remaining: usize) -> Instant {
    let now = Instant::now();
    now + deadline.saturating_duration_since(now) / remaining.max(1) as u32
}
fn normalize(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > FRAME_LIMIT {
        return Err(unavailable());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| unavailable())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| unavailable())?
        .thumbnail(320, 320)
        .to_rgb8();
    if image.width() == 0 || image.height() == 0 {
        return Err(unavailable());
    }
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
        .encode(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|_| unavailable())?;
    if jpeg.len() >= THUMB_LIMIT {
        return Err(unavailable());
    }
    Ok(jpeg)
}
fn ffmpeg_thumbnail(
    source: &Path,
    candidates: &[PathBuf],
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if candidates.is_empty() {
        return Err(preview_error("No FFmpeg executable was found by the app. Install FFmpeg and make it available on PATH, then fully restart the app"));
    }
    let directory = crate::temp_artifacts::staging_root()
        .map_err(|_| preview_error("Could not create temporary storage for the video preview"))?
        .join(format!("video-preview-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory)
        .map_err(|_| preview_error("Could not create temporary storage for the video preview"))?;
    let scratch = Scratch(directory);
    let output = scratch.0.join("frame.jpg");
    let mut failure = "FFmpeg could not decode a frame from this video";
    for (index, candidate) in candidates.iter().enumerate() {
        stopped(stop)?;
        if Instant::now() >= deadline {
            return Err(preview_error("Video preview preparation timed out"));
        }
        let _ = std::fs::remove_file(&output);
        let mut command = std::process::Command::new(candidate);
        command
            .args([
                "-nostdin",
                "-v",
                "error",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(source)
            .args([
                "-map",
                "0:v:0",
                "-frames:v",
                "1",
                "-vf",
                "scale=320:320:force_original_aspect_ratio=decrease",
                "-fs",
                "2097152",
                "-y",
            ])
            .arg(&output);
        let result = crate::process_budget::run(
            command,
            attempt_deadline(deadline, candidates.len() - index),
            false,
            || stopped(stop),
            Some((&output, FRAME_LIMIT)),
        );
        stopped(stop)?;
        // Report only fixed categories. Decoder stderr and OS errors can contain
        // private paths or media metadata and are never copied into UI/logs.
        failure = match result {
            Ok(result) if result.success => {
                if let Ok(bytes) = std::fs::read(&output) {
                    if let Ok(jpeg) = normalize(&bytes) {
                        return Ok(jpeg);
                    }
                }
                "FFmpeg produced an invalid preview"
            }
            Ok(_) => "FFmpeg could not decode a frame from this video",
            Err(error) => ffmpeg_failure(&error),
        };
    }
    Err(preview_error(failure))
}
fn ffmpeg_failure(error: &str) -> &'static str {
    match error {
        "HEIC_DECODER_UNAVAILABLE" => "FFmpeg was found but could not be started",
        "HEIC_DECODE_DEADLINE" => "FFmpeg video preview decoding timed out",
        "HEIC_PROCESS_LIMIT" => "FFmpeg could not be safely started for video preview decoding",
        "HEIC_MEMORY_LIMIT" | "HEIC_OUTPUT_LIMIT" => {
            "FFmpeg reached a video preview resource limit"
        }
        "HEIC_MEMORY_MONITOR_UNAVAILABLE" => {
            "FFmpeg could not be safely monitored during video preview decoding"
        }
        _ => "FFmpeg could not finish video preview decoding",
    }
}
#[cfg(target_os = "windows")]
fn native_thumbnail(
    source: &Path,
    width: i32,
    height: i32,
    resource: Option<&Path>,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<Vec<u8>, String> {
    #[cfg(feature = "native-e2e")]
    if let Some(mode) = resource.and_then(|directory| {
        std::fs::read_to_string(directory.join(".video-upload-native-mode")).ok()
    }) {
        if mode == "wait" {
            while Instant::now() < deadline {
                stopped(stop)?;
                std::thread::sleep(Duration::from_millis(25));
            }
            return Err(unavailable());
        }
        if mode == "fail" {
            return Err(unavailable());
        }
        if mode == "delay" {
            let ready = Instant::now() + Duration::from_secs(16);
            while Instant::now() < ready {
                stopped(stop)?;
                if Instant::now() >= deadline {
                    return Err(unavailable());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let _ = resource;
    windows_thumbnail(source, width, height, deadline, stop)
}
#[cfg(target_os = "windows")]
fn windows_thumbnail(
    source: &Path,
    width: i32,
    height: i32,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<Vec<u8>, String> {
    use windows::{
        Foundation::TimeSpan,
        Media::Editing::{MediaClip, MediaComposition, VideoFramePrecision},
        Storage::{StorageFile, Streams::DataReader},
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|_| unavailable())?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    let _apartment = Apartment;
    macro_rules! wait {
        ($operation:expr) => {{
            let operation = $operation.map_err(|_| unavailable())?;
            loop {
                if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                    let _ = operation.Cancel();
                    return Err(if stop.load(Ordering::Relaxed) {
                        "Transfer cancelled".into()
                    } else {
                        unavailable()
                    });
                }
                match operation.Status() {
                    Ok(status) if status.0 != 0 => break,
                    Ok(_) => {}
                    Err(_) => {
                        let _ = operation.Cancel();
                        return Err(unavailable());
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            operation.GetResults().map_err(|_| unavailable())?
        }};
    }
    let path = source.canonicalize().map_err(|_| unavailable())?;
    let path = path.to_string_lossy();
    let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
    };
    let file = wait!(StorageFile::GetFileFromPathAsync(
        &windows::core::HSTRING::from(path.as_str())
    ));
    let clip = wait!(MediaClip::CreateFromFileAsync(&file));
    let composition = MediaComposition::new().map_err(|_| unavailable())?;
    composition
        .Clips()
        .map_err(|_| unavailable())?
        .Append(&clip)
        .map_err(|_| unavailable())?;
    let longest = width.max(height).max(1) as f64;
    let w = ((width as f64 * 320.0 / longest).round() as i32).max(1);
    let h = ((height as f64 * 320.0 / longest).round() as i32).max(1);
    let stream = wait!(composition.GetThumbnailAsync(
        TimeSpan { Duration: 0 },
        w,
        h,
        VideoFramePrecision::NearestFrame
    ));
    let length = stream.Size().map_err(|_| unavailable())?;
    if length == 0 || length > FRAME_LIMIT {
        return Err(unavailable());
    }
    let input = stream.GetInputStreamAt(0).map_err(|_| unavailable())?;
    let reader = DataReader::CreateDataReader(&input).map_err(|_| unavailable())?;
    let loaded = wait!(reader.LoadAsync(length as u32));
    if u64::from(loaded) != length {
        return Err(unavailable());
    }
    let mut bytes = vec![0; length as usize];
    reader.ReadBytes(&mut bytes).map_err(|_| unavailable())?;
    Ok(bytes)
}
