use crate::commands::utils::resolve_peer;
use crate::commands::TelegramState;
use crate::transcode::TranscodeManager;
use actix_cors::Cors;
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use grammers_client::types::Media;

use std::net::TcpListener;
use std::sync::Arc;

/// Ad banner HTML matching the working cameronamer.com/ad-banner.html structure.
/// Served from the streaming server so the iframe gets a real http://127.0.0.1 origin.
/// Key detail: referrerpolicy="no-referrer" on the invoke.js script tag prevents the
/// browser from sending a Referer header that Adsterra would reject.
/// No 'async' attribute — prevents race conditions where the script tries to inject
/// the ad iframe before the DOM is ready.
const AD_BANNER_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Ad Banner</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    html, body {
      width: 100%;
      height: 100%;
      overflow: hidden;
      background: #1a1a2e;
    }
  </style>
</head>
<body>
  <script type="text/javascript">
    window.atOptions = {
      'key' : '9cf449272b7e1c83054b82b7639c6029',
      'format' : 'iframe',
      'height' : 250,
      'width' : 300,
      'params' : {}
    };
  </script>
  <script 
    type="text/javascript" 
    src="https://www.highperformanceformat.com/9cf449272b7e1c83054b82b7639c6029/invoke.js"
    referrerpolicy="no-referrer">
  </script>
</body>
</html>"#;

/// Holds the per-session streaming token for Actix validation
pub struct StreamTokenData {
    pub token: String,
}

#[derive(serde::Deserialize)]
struct StreamQuery {
    token: Option<String>,
}

pub fn parse_range_header(header_val: &str, total_size: u64) -> Option<(u64, u64)> {
    if !header_val.starts_with("bytes=") {
        return None;
    }
    let s = &header_val["bytes=".len()..];
    let parts: Vec<&str> = s.split('-').collect();
    if parts.is_empty() {
        return None;
    }
    let start = parts[0].trim().parse::<u64>().ok()?;
    let end = if parts.len() > 1 && !parts[1].trim().is_empty() {
        let parsed_end = parts[1].trim().parse::<u64>().ok()?;
        std::cmp::min(parsed_end, total_size - 1)
    } else {
        total_size - 1
    };
    if start <= end {
        Some((start, end))
    } else {
        None
    }
}

/// Extra headers to inject into streaming responses (e.g. Cache-Control, Content-Disposition).
pub struct StreamingExtras {
    pub extra_headers: Vec<(&'static str, String)>,
    pub log_label: &'static str,
}

/// Build a streaming HTTP response for a Telegram media file with optional byte-range support.
/// This is the single shared implementation used by the streaming server, REST API, and share routes.
pub fn build_media_response(
    client: &grammers_client::Client,
    media: &Media,
    req: &actix_web::HttpRequest,
    mime: &str,
    filename: Option<&str>,
    extras: StreamingExtras,
) -> HttpResponse {
    let size = match media {
        Media::Document(d) => d.size() as u64,
        Media::Photo(_) => 0,
        _ => 0,
    };

    // Parse Range header
    let mut start_byte = 0u64;
    let mut end_byte = if size > 0 { size - 1 } else { 0 };
    let mut is_range = false;

    if size > 0 {
        if let Some(range_header) = req.headers().get(actix_web::http::header::RANGE) {
            if let Ok(range_str) = range_header.to_str() {
                if let Some((start, end)) = parse_range_header(range_str, size) {
                    start_byte = start;
                    end_byte = end;
                    is_range = true;
                }
            }
        }
    }

    let content_length = if is_range {
        end_byte - start_byte + 1
    } else {
        size
    };

    // Chunk alignment for Telegram's upload.getFile offset requirement.
    //
    // CRITICAL: Without the `precise` flag (which grammers-client does not
    // expose), Telegram may route the request through a CDN that rounds the
    // offset down to a CDN chunk boundary (commonly 512 KB = 524288 bytes).
    // If our requested offset is not aligned to this boundary, the CDN
    // silently returns data starting from the rounded-down position.
    //
    // Example: requesting offset 111935488 (213.48 × 512 KB) gets rounded
    // to 111673344 (213 × 512 KB), introducing a 262 KB shift. This
    // misalignment accumulates across successive Range requests and
    // eventually corrupts the MP4 box parsing (triggering the "ORrI" error).
    //
    // Fix: always align to 512 KB boundaries, then slice off the leading
    // bytes to serve the exact byte range the client requested.
    let mut download_iter = client.iter_download(media);
    let mut bytes_to_skip: usize = 0;

    if start_byte > 0 {
        /// MTProto chunk size (must be divisible by grammers' MIN_CHUNK_SIZE).
        /// 65536 is safe — it is the default and widely tested.
        const CHUNK_SIZE: i32 = 65536;
        /// Telegram CDN alignment boundary. 512 KB is the largest observed
        /// CDN chunk size; aligning to this boundary prevents ANY rounding.
        const CDN_ALIGNMENT: u64 = 524288; // 512 KB

        // 1) Round the requested start down to a CDN-safe boundary.
        let cdn_aligned_start = (start_byte / CDN_ALIGNMENT) * CDN_ALIGNMENT;

        // 2) Compute how many 64 KB chunks to skip to reach that boundary.
        let chunk_index = (cdn_aligned_start / CHUNK_SIZE as u64) as i32;

        // Always set chunk size for predictable download behaviour.
        download_iter = download_iter.chunk_size(CHUNK_SIZE);
        if chunk_index > 0 {
            download_iter = download_iter.skip_chunks(chunk_index);
        }

        // 3) Leading bytes between the CDN-aligned offset and the client's
        //    actual requested start must be discarded.
        bytes_to_skip = (start_byte - cdn_aligned_start) as usize;

        // Safety: cdn_aligned_start ≤ start_byte by construction.
        debug_assert!(
            cdn_aligned_start <= start_byte,
            "CDN alignment invariant violated: aligned {} > requested {}",
            cdn_aligned_start,
            start_byte
        );

        log::debug!(
            "Range alignment: requested={}, cdn_aligned={}, chunk_index={}, bytes_to_skip={}",
            start_byte,
            cdn_aligned_start,
            chunk_index,
            bytes_to_skip,
        );
    }

    let label = extras.log_label;
    let stream = async_stream::stream! {
        let mut skipped: usize = 0;
        let mut total_yielded: u64 = 0;

        while let Some(chunk) = download_iter.next().await.transpose() {
            match chunk {
                Ok(data) => {
                    let mut data_slice = data;

                    if skipped < bytes_to_skip {
                        let to_skip = bytes_to_skip - skipped;
                        if data_slice.len() <= to_skip {
                            skipped += data_slice.len();
                            continue;
                        } else {
                            data_slice = data_slice[to_skip..].to_vec();
                            skipped = bytes_to_skip;
                        }
                    }

                    if total_yielded + data_slice.len() as u64 > content_length {
                        let allowed = (content_length - total_yielded) as usize;
                        if allowed > 0 {
                            yield Ok::<_, actix_web::Error>(web::Bytes::from(data_slice[..allowed].to_vec()));
                            total_yielded += allowed as u64;
                        }
                        break;
                    } else {
                        let len = data_slice.len() as u64;
                        yield Ok::<_, actix_web::Error>(web::Bytes::from(data_slice));
                        total_yielded += len;
                        if total_yielded >= content_length {
                            break;
                        }
                    }
                }
                Err(e) => {
                    log::error!("{} stream error: {}", label, e);
                    break;
                }
            }
        }
        log::debug!("{} stream completed (yielded: {})", label, total_yielded);
    };

    let mut resp = if is_range {
        let mut r = HttpResponse::PartialContent();
        r.insert_header((
            "Content-Range",
            format!("bytes {}-{}/{}", start_byte, end_byte, size),
        ));
        r.insert_header(("Content-Length", content_length.to_string()));
        r
    } else {
        let mut r = HttpResponse::Ok();
        r.insert_header(("Content-Length", size.to_string()));
        r
    };

    resp.insert_header(("Content-Type", mime.to_owned()));
    resp.insert_header(("Accept-Ranges", "bytes"));

    if let Some(fname) = filename {
        resp.insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", fname),
        ));
    }

    for (key, val) in &extras.extra_headers {
        resp.insert_header((*key, val.clone()));
    }

    resp.streaming(stream)
}

/// Serves the inline ad HTML so the iframe runs on a real http://127.0.0.1 origin.
/// Ad networks reject custom origins like tauri://localhost and https://tauri.localhost.
#[get("/ad-banner")]
async fn ad_banner() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-cache"))
        .body(AD_BANNER_HTML)
}

#[get("/stream/{folder_id}/{message_id}")]
async fn stream_media(
    req: actix_web::HttpRequest,
    path: web::Path<(String, i32)>,
    query: web::Query<StreamQuery>,
    data: web::Data<Arc<TelegramState>>,
    token_data: web::Data<StreamTokenData>,
) -> impl Responder {
    let (folder_id_str, message_id) = path.into_inner();

    // Validate session token
    match &query.token {
        Some(t) if t == &token_data.token => {
            log::debug!(
                "Stream request: Token validated successfully for msg {}",
                message_id
            );
        }
        _ => {
            log::error!(
                "Stream request failed: Invalid or missing stream token for msg {}",
                message_id
            );
            return HttpResponse::Forbidden().body("Invalid or missing stream token");
        }
    }

    // Parse folder ID
    let folder_id = if folder_id_str == "me" || folder_id_str == "home" || folder_id_str == "null" {
        log::debug!("Stream request: Using root folder for msg {}", message_id);
        None
    } else {
        match folder_id_str.parse::<i64>() {
            Ok(id) => {
                log::debug!(
                    "Stream request: Parsed folder ID {} for msg {}",
                    id,
                    message_id
                );
                Some(id)
            }
            Err(_) => {
                log::error!(
                    "Stream request failed: Invalid folder ID format '{}' for msg {}",
                    folder_id_str,
                    message_id
                );
                return HttpResponse::BadRequest().body("Invalid folder ID");
            }
        }
    };

    let client_opt = { data.client.lock().await.clone() };

    if let Some(client) = client_opt {
        log::debug!(
            "Stream request: Client acquired, resolving peer for msg {}...",
            message_id
        );
        match resolve_peer(&client, folder_id, &data.peer_cache).await {
            Ok(peer) => {
                log::debug!(
                    "Stream request: Peer resolved, fetching message {}...",
                    message_id
                );
                // Try to fetch message efficiently
                match client.get_messages_by_id(peer, &[message_id]).await {
                    Ok(messages) => {
                        if let Some(Some(msg)) = messages.first() {
                            if let Some(media) = msg.media() {
                                log::debug!(
                                    "Stream request: Message and media found for msg {}",
                                    message_id
                                );
                                let mime = mime_type_from_media(&media);
                                return build_media_response(
                                    &client,
                                    &media,
                                    &req,
                                    &mime,
                                    None,
                                    StreamingExtras {
                                        extra_headers: vec![(
                                            "Cache-Control",
                                            "private, max-age=120".to_string(),
                                        )],
                                        log_label: "Stream",
                                    },
                                );
                            } else {
                                log::error!(
                                    "Stream request failed: Media not found in message {}",
                                    message_id
                                );
                            }
                        } else {
                            log::error!("Stream request failed: Message {} not found", message_id);
                        }
                        HttpResponse::NotFound().body("Message or media not found")
                    }
                    Err(e) => {
                        log::error!(
                            "Stream request failed: Error fetching message {}: {}",
                            message_id,
                            e
                        );
                        HttpResponse::InternalServerError()
                            .body(format!("Failed to fetch message: {}", e))
                    }
                }
            }
            Err(e) => {
                log::error!(
                    "Stream request failed: Peer resolution error for msg {}: {}",
                    message_id,
                    e
                );
                HttpResponse::BadRequest().body(format!("Peer resolution failed: {}", e))
            }
        }
    } else {
        log::error!(
            "Stream request failed: Telegram client not connected for msg {}",
            message_id
        );
        HttpResponse::ServiceUnavailable().body("Telegram client not connected")
    }
}

fn mime_type_from_media(media: &Media) -> String {
    match media {
        Media::Document(d) => d
            .mime_type()
            .unwrap_or("application/octet-stream")
            .to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

pub async fn start_server(
    state: Arc<TelegramState>,
    port: u16,
    token: String,
    db_pool: crate::db::DbConnection,
    transcode_manager: Arc<TranscodeManager>,
) -> std::io::Result<actix_web::dev::Server> {
    let state_data = web::Data::new(state);
    let token_data = web::Data::new(StreamTokenData { token });
    let db_data = web::Data::new(db_pool);
    let transcode_data = web::Data::new(transcode_manager);

    log::info!("Starting Streaming Server on port {}", port);

    // Bind the listener to 127.0.0.1 explicitly.
    // The streaming server is only accessed from the local frontend — binding
    // to 0.0.0.0 is unnecessary and can trigger firewall prompts on Windows.
    // 127.0.0.1 is the most universally reliable loopback address across all
    // platforms (Windows, macOS, Linux) and pairs correctly with the "localhost"
    // hostname used by the client (localhost → 127.0.0.1 is the standard mapping).
    let ipv4_addr = format!("127.0.0.1:{}", port);
    let listener = match TcpListener::bind(&ipv4_addr) {
        Ok(l) => {
            log::info!("Streaming Server listening on {} (IPv4)", ipv4_addr);
            l
        }
        Err(e) => {
            log::warn!(
                "IPv4 loopback bind failed ({}), falling back to IPv6 loopback",
                e
            );
            let ipv6_addr = format!("[::1]:{}", port);
            let l = TcpListener::bind(&ipv6_addr)?;
            log::info!(
                "Streaming Server listening on {} (IPv6 loopback)",
                ipv6_addr
            );
            l
        }
    };

    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin_fn(|origin, _req_head| {
                let origin_bytes = origin.as_bytes();
                origin_bytes.starts_with(b"tauri://")
                    || origin_bytes.starts_with(b"http://tauri.localhost")
                    || origin_bytes.starts_with(b"https://tauri.localhost")
                    || origin_bytes.starts_with(b"http://localhost")
                    || origin_bytes.starts_with(b"http://127.0.0.1")
                    || origin_bytes.starts_with(b"https://asset.localhost")
                    || origin_bytes.starts_with(b"http://asset.localhost")
                    || origin_bytes == b"null"
            })
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(state_data.clone())
            .app_data(token_data.clone())
            .app_data(db_data.clone())
            .app_data(transcode_data.clone())
            .service(ad_banner)
            .service(stream_media)
            .configure(crate::share_routes::configure_share_routes)
            .configure(crate::transcode::configure_hls_routes)
            .configure(crate::fmp4_remux::configure_fmp4_routes)
    })
    .listen(listener)?
    .run();

    log::info!("Streaming Server started successfully on port {}", port);

    Ok(server)
}
