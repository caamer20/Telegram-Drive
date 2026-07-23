use crate::commands::utils::resolve_peer;
use crate::mp4_utils;
use crate::TelegramState;
use grammers_client::types::Media;
use tauri::State;

#[derive(serde::Serialize)]
pub struct VideoMetadata {
    pub duration_secs: Option<f64>,
    pub video_codec: Option<String>,
    pub has_audio: bool,
    pub track_count: usize,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(serde::Deserialize)]
pub struct BatchMetadataRequest {
    pub message_id: i32,
    pub file_name: String,
}

#[derive(serde::Serialize)]
pub struct BatchMetadataEntry {
    pub message_id: i32,
    pub duration_secs: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[tauri::command]
pub async fn cmd_get_video_metadata(
    message_id: i32,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<VideoMetadata, String> {
    let client = { state.client.lock().await.clone() };
    let client = client.ok_or_else(|| "Not connected to Telegram".to_string())?;

    let buffer = download_moov_chunk(&client, message_id, folder_id, &state).await?;
    let meta = parse_mp4_metadata(&buffer)?;
    let (width, height) = mp4_utils::scan_video_tkhd_dimensions(&buffer);

    Ok(VideoMetadata {
        duration_secs: meta.duration_secs,
        video_codec: meta.video_codec,
        has_audio: meta.has_audio,
        track_count: meta.track_count,
        width,
        height,
    })
}

#[tauri::command]
pub async fn cmd_get_video_metadata_batch(
    requests: Vec<BatchMetadataRequest>,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<Vec<BatchMetadataEntry>, String> {
    let client = { state.client.lock().await.clone() };
    let client = client.ok_or_else(|| "Not connected to Telegram".to_string())?;
    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;

    let mut results: Vec<BatchMetadataEntry> = Vec::with_capacity(requests.len());

    for req in &requests {
        if !req.file_name.to_lowercase().ends_with(".mp4") {
            continue;
        }
        match download_and_process(&client, &peer, req).await {
            Ok(e) => results.push(e),
            Err(_) => results.push(BatchMetadataEntry {
                message_id: req.message_id,
                duration_secs: None,
                width: None,
                height: None,
            }),
        }
    }

    Ok(results)
}

// ── Internal helpers ─────────────────────────────────────────────────

struct ParsedMetadata {
    duration_secs: Option<f64>,
    video_codec: Option<String>,
    has_audio: bool,
    track_count: usize,
}

/// Download the first 2 MB of a file and parse metadata + scan tkhd.
async fn download_and_process(
    client: &grammers_client::Client,
    peer: &grammers_client::types::Peer,
    req: &BatchMetadataRequest,
) -> Result<BatchMetadataEntry, String> {
    let messages = client
        .get_messages_by_id(peer, &[req.message_id])
        .await
        .map_err(|e| e.to_string())?;
    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| format!("Message {} not found", req.message_id))?;
    let media = msg.media().ok_or_else(|| "No media".to_string())?;

    let size = match &media {
        Media::Document(d) => d.size() as u64,
        _ => return Err("Not a document".to_string()),
    };

    let buffer = download_bytes(client, &media, size).await?;
    let meta = parse_mp4_metadata(&buffer)?;
    let (width, height) = mp4_utils::scan_video_tkhd_dimensions(&buffer);

    Ok(BatchMetadataEntry {
        message_id: req.message_id,
        duration_secs: meta.duration_secs,
        width,
        height,
    })
}

async fn download_moov_chunk(
    client: &grammers_client::Client,
    message_id: i32,
    folder_id: Option<i64>,
    state: &TelegramState,
) -> Result<Vec<u8>, String> {
    let peer = resolve_peer(client, folder_id, &state.peer_cache).await?;
    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| format!("Message {message_id} not found"))?;
    let media = msg.media().ok_or_else(|| "No media".to_string())?;
    let size = match &media {
        Media::Document(d) => d.size() as u64,
        _ => return Err("Not a document".to_string()),
    };
    download_bytes(client, &media, size).await
}

/// Download at most the first 2 MB from a Telegram document.
async fn download_bytes(
    client: &grammers_client::Client,
    media: &Media,
    file_size: u64,
) -> Result<Vec<u8>, String> {
    let max_bytes = std::cmp::min(2 * 1024 * 1024, file_size) as usize;
    let mut buffer: Vec<u8> = Vec::with_capacity(max_bytes);
    let mut download_iter = client.iter_download(media);
    download_iter = download_iter.chunk_size(65536);

    while buffer.len() < max_bytes {
        match download_iter.next().await {
            Ok(Some(chunk)) => {
                let remaining = max_bytes.saturating_sub(buffer.len());
                let take = std::cmp::min(chunk.len(), remaining);
                buffer.extend_from_slice(&chunk[..take]);
            }
            Ok(None) => break,
            Err(e) => return Err(format!("Download error: {e}")),
        }
    }
    if buffer.is_empty() {
        return Err("Downloaded zero bytes".to_string());
    }
    Ok(buffer)
}

fn parse_mp4_metadata(buffer: &[u8]) -> Result<ParsedMetadata, String> {
    let mut cursor = std::io::Cursor::new(buffer);
    let context = mp4parse::read_mp4(&mut cursor).map_err(|e| format!("MP4 parse error: {e}"))?;

    let video_track = context
        .tracks
        .iter()
        .find(|t| t.track_type == mp4parse::TrackType::Video);

    let has_audio = context
        .tracks
        .iter()
        .any(|t| t.track_type == mp4parse::TrackType::Audio);

    let duration_secs = video_track.and_then(|t| {
        let d = t.duration.as_ref()?;
        let ts = t.timescale.as_ref()?;
        Some((d.0 as f64) / (ts.0 as f64))
    });

    Ok(ParsedMetadata {
        duration_secs,
        video_codec: None,
        has_audio,
        track_count: context.tracks.len(),
    })
}
