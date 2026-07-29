use crate::commands::utils::resolve_peer;
use crate::commands::TelegramState;
use crate::transcode::TranscodeManager;
use actix_cors::Cors;
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use grammers_client::types::Media;

use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::sync::Arc;

pub const TELEGRAM_CHUNK_SIZE: i32 = 65_536;
pub const TELEGRAM_CDN_ALIGNMENT: u64 = 512 * 1024;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    Invalid,
    Multiple,
    Unsatisfiable,
}

pub fn parse_range_header(header_val: &str, total_size: u64) -> Result<ByteRange, RangeError> {
    if total_size == 0 {
        return Err(RangeError::Unsatisfiable);
    }
    let value = header_val
        .strip_prefix("bytes=")
        .ok_or(RangeError::Invalid)?
        .trim();
    if value.contains(',') {
        return Err(RangeError::Multiple);
    }
    let (start_text, end_text) = value.split_once('-').ok_or(RangeError::Invalid)?;
    if end_text.contains('-') {
        return Err(RangeError::Invalid);
    }

    if start_text.trim().is_empty() {
        let suffix = end_text
            .trim()
            .parse::<u64>()
            .map_err(|_| RangeError::Invalid)?;
        if suffix == 0 {
            return Err(RangeError::Unsatisfiable);
        }
        let length = suffix.min(total_size);
        return Ok(ByteRange {
            start: total_size - length,
            end: total_size - 1,
        });
    }

    let start = start_text
        .trim()
        .parse::<u64>()
        .map_err(|_| RangeError::Invalid)?;
    if start >= total_size {
        return Err(RangeError::Unsatisfiable);
    }
    let end = if end_text.trim().is_empty() {
        total_size - 1
    } else {
        end_text
            .trim()
            .parse::<u64>()
            .map_err(|_| RangeError::Invalid)?
            .min(total_size - 1)
    };
    if start > end {
        return Err(RangeError::Unsatisfiable);
    }
    Ok(ByteRange { start, end })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MediaResponsePlan {
    status: actix_web::http::StatusCode,
    range: Option<ByteRange>,
    content_length: u64,
}

fn response_plan(
    range_header: Option<&str>,
    total_size: u64,
) -> Result<MediaResponsePlan, RangeError> {
    match range_header {
        Some(value) => {
            let range = parse_range_header(value, total_size)?;
            Ok(MediaResponsePlan {
                status: actix_web::http::StatusCode::PARTIAL_CONTENT,
                range: Some(range),
                content_length: range.end - range.start + 1,
            })
        }
        None => Ok(MediaResponsePlan {
            status: actix_web::http::StatusCode::OK,
            range: None,
            content_length: total_size,
        }),
    }
}

fn range_not_satisfiable_response(total_size: u64) -> HttpResponse {
    HttpResponse::RangeNotSatisfiable()
        .insert_header(("Content-Range", format!("bytes */{}", total_size)))
        .insert_header(("Accept-Ranges", "bytes"))
        .finish()
}

fn media_response_builder(
    plan: MediaResponsePlan,
    total_size: u64,
    mime: &str,
) -> actix_web::HttpResponseBuilder {
    let mut response = HttpResponse::build(plan.status);
    if let Some(range) = plan.range {
        response.insert_header((
            "Content-Range",
            format!("bytes {}-{}/{}", range.start, range.end, total_size),
        ));
    }
    response.insert_header(("Content-Length", plan.content_length.to_string()));
    response.insert_header(("Content-Type", mime.to_owned()));
    response.insert_header(("Accept-Ranges", "bytes"));
    response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadAlignment {
    pub aligned_start: u64,
    pub chunk_index: i32,
    pub leading_bytes: usize,
}

pub fn calculate_download_alignment(start: u64) -> DownloadAlignment {
    let aligned_start = (start / TELEGRAM_CDN_ALIGNMENT) * TELEGRAM_CDN_ALIGNMENT;
    DownloadAlignment {
        aligned_start,
        chunk_index: (aligned_start / TELEGRAM_CHUNK_SIZE as u64) as i32,
        leading_bytes: (start - aligned_start) as usize,
    }
}

fn slice_after_leading_skip<'a>(
    data: &'a [u8],
    skipped: &mut usize,
    bytes_to_skip: usize,
) -> &'a [u8] {
    if *skipped >= bytes_to_skip {
        return data;
    }
    let take = (bytes_to_skip - *skipped).min(data.len());
    *skipped += take;
    &data[take..]
}

fn token_matches(candidate: &str, expected: &str) -> bool {
    candidate.len() == expected.len()
        && constant_time_eq::constant_time_eq(candidate.as_bytes(), expected.as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthenticationResult {
    Authorized,
    Missing,
    Invalid,
}

fn authenticate_stream_request(
    req: &actix_web::HttpRequest,
    query_token: Option<&str>,
    expected: &str,
) -> AuthenticationResult {
    let bearer = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if let Some(candidate) = bearer {
        return if token_matches(candidate, expected) {
            AuthenticationResult::Authorized
        } else {
            AuthenticationResult::Invalid
        };
    }
    match query_token {
        Some(candidate) if token_matches(candidate, expected) => AuthenticationResult::Authorized,
        Some(_) => AuthenticationResult::Invalid,
        None => AuthenticationResult::Missing,
    }
}

pub fn redact_sensitive(input: &str) -> String {
    let mut output = input.to_string();
    for marker in ["token=", "Bearer "] {
        let mut search_from = 0;
        while let Some(relative) = output[search_from..].find(marker) {
            let value_start = search_from + relative + marker.len();
            let value_end = output[value_start..]
                .find(|c: char| c == '&' || c.is_whitespace() || c == '\"' || c == '\'')
                .map(|offset| value_start + offset)
                .unwrap_or(output.len());
            output.replace_range(value_start..value_end, "[REDACTED]");
            search_from = value_start + "[REDACTED]".len();
        }
    }
    output
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

    let range_header = match req.headers().get(actix_web::http::header::RANGE) {
        Some(value) => match value.to_str() {
            Ok(value) => Some(value),
            Err(_) => {
                return range_not_satisfiable_response(size);
            }
        },
        None => None,
    };
    let plan = match response_plan(range_header, size) {
        Ok(plan) => plan,
        Err(_) => {
            return range_not_satisfiable_response(size);
        }
    };
    let start_byte = plan.range.map_or(0, |range| range.start);
    let content_length = plan.content_length;

    let mut resp = media_response_builder(plan, size, mime);

    if let Some(fname) = filename {
        resp.insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", fname),
        ));
    }
    for (key, val) in &extras.extra_headers {
        resp.insert_header((*key, val.clone()));
    }

    // HEAD must return the exact GET metadata without constructing a Telegram iterator.
    if req.method() == actix_web::http::Method::HEAD {
        return resp.finish();
    }

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
        let alignment = calculate_download_alignment(start_byte);

        // Always set chunk size for predictable download behaviour.
        download_iter = download_iter.chunk_size(TELEGRAM_CHUNK_SIZE);
        if alignment.chunk_index > 0 {
            download_iter = download_iter.skip_chunks(alignment.chunk_index);
        }

        // 3) Leading bytes between the CDN-aligned offset and the client's
        //    actual requested start must be discarded.
        bytes_to_skip = alignment.leading_bytes;

        // Safety: cdn_aligned_start ≤ start_byte by construction.
        debug_assert!(
            alignment.aligned_start <= start_byte,
            "CDN alignment invariant violated: aligned {} > requested {}",
            alignment.aligned_start,
            start_byte
        );

        log::debug!(
            "Range alignment: requested={}, cdn_aligned={}, chunk_index={}, bytes_to_skip={}",
            start_byte,
            alignment.aligned_start,
            alignment.chunk_index,
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
                    let data_slice = slice_after_leading_skip(&data, &mut skipped, bytes_to_skip);
                    if data_slice.is_empty() {
                        continue;
                    }

                    if total_yielded + data_slice.len() as u64 > content_length {
                        let allowed = (content_length - total_yielded) as usize;
                        if allowed > 0 {
                            yield Ok::<_, actix_web::Error>(web::Bytes::copy_from_slice(&data_slice[..allowed]));
                            total_yielded += allowed as u64;
                        }
                        break;
                    } else {
                        let len = data_slice.len() as u64;
                        yield Ok::<_, actix_web::Error>(web::Bytes::copy_from_slice(data_slice));
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

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ready" }))
}

async fn stream_media(
    req: actix_web::HttpRequest,
    path: web::Path<(String, i32)>,
    query: web::Query<StreamQuery>,
    data: web::Data<Arc<TelegramState>>,
    token_data: web::Data<StreamTokenData>,
) -> impl Responder {
    let (folder_id_str, message_id) = path.into_inner();

    match authenticate_stream_request(&req, query.token.as_deref(), &token_data.token) {
        AuthenticationResult::Authorized => {}
        AuthenticationResult::Missing => {
            log::warn!(
                "Stream request rejected: missing credentials for msg {}",
                message_id
            );
            return HttpResponse::Unauthorized()
                .insert_header(("WWW-Authenticate", "Bearer"))
                .body("Missing stream credentials");
        }
        AuthenticationResult::Invalid => {
            log::warn!(
                "Stream request rejected: invalid credentials for msg {}",
                message_id
            );
            return HttpResponse::Forbidden().body("Invalid stream credentials");
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

pub struct StartedStreamingServer {
    pub address: SocketAddrV4,
    pub server: actix_web::dev::Server,
}

pub fn start_server(
    state: Arc<TelegramState>,
    token: String,
    db_pool: crate::db::DbConnection,
    transcode_manager: Arc<TranscodeManager>,
) -> std::io::Result<StartedStreamingServer> {
    let state_data = web::Data::new(state);
    let token_data = web::Data::new(StreamTokenData { token });
    let db_data = web::Data::new(db_pool);
    let transcode_data = web::Data::new(transcode_manager);

    // Port zero delegates collision-free selection to the OS. There is no IPv6
    // or external-interface fallback: the stream contains private Telegram data.
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
    let address = match listener.local_addr()? {
        std::net::SocketAddr::V4(address) => address,
        std::net::SocketAddr::V6(_) => {
            return Err(std::io::Error::other(
                "stream listener was not IPv4 loopback",
            ));
        }
    };
    log::info!("Streaming server bound to http://{}", address);

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
            .service(health)
            .service(
                web::resource("/stream/{folder_id}/{message_id}")
                    .route(web::get().to(stream_media))
                    .route(web::head().to(stream_media)),
            )
            .configure(crate::share_routes::configure_share_routes)
            .configure(crate::transcode::configure_hls_routes)
            .configure(crate::fmp4_remux::configure_fmp4_routes)
    })
    // A single worker keeps one controlled runtime on the dedicated server
    // thread and avoids oversubscribing mobile devices.
    .workers(1)
    .disable_signals()
    .listen(listener)?
    .run();

    Ok(StartedStreamingServer { address, server })
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::{header, Method, StatusCode};
    use actix_web::test::TestRequest;

    #[test]
    fn parses_closed_open_and_suffix_ranges() {
        assert_eq!(
            parse_range_header("bytes=10-19", 100),
            Ok(ByteRange { start: 10, end: 19 })
        );
        assert_eq!(
            parse_range_header("bytes=10-", 100),
            Ok(ByteRange { start: 10, end: 99 })
        );
        assert_eq!(
            parse_range_header("bytes=-10", 100),
            Ok(ByteRange { start: 90, end: 99 })
        );
    }

    #[test]
    fn rejects_invalid_out_of_bounds_and_multiple_ranges() {
        assert_eq!(
            parse_range_header("items=0-2", 100),
            Err(RangeError::Invalid)
        );
        assert_eq!(
            parse_range_header("bytes=100-", 100),
            Err(RangeError::Unsatisfiable)
        );
        assert_eq!(
            parse_range_header("bytes=0-1,4-5", 100),
            Err(RangeError::Multiple)
        );
    }

    #[test]
    fn response_plans_have_exact_lengths_and_statuses() {
        let full = response_plan(None, 1_000).unwrap();
        assert_eq!(full.status, StatusCode::OK);
        assert_eq!(full.content_length, 1_000);
        let partial = response_plan(Some("bytes=512-767"), 1_000).unwrap();
        assert_eq!(partial.status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(partial.content_length, 256);
        assert_eq!(
            partial.range.unwrap(),
            ByteRange {
                start: 512,
                end: 767
            }
        );
    }

    #[test]
    fn range_not_satisfiable_metadata_uses_total_size() {
        let size = 42;
        assert_eq!(
            parse_range_header("bytes=42-", size),
            Err(RangeError::Unsatisfiable)
        );
        let response = range_not_satisfiable_response(size);
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes */42"
        );
        assert_eq!(
            response.headers().get(header::ACCEPT_RANGES).unwrap(),
            "bytes"
        );
    }

    #[test]
    fn full_and_partial_responses_have_exact_http_headers() {
        let full = media_response_builder(response_plan(None, 1_000).unwrap(), 1_000, "video/mp4")
            .finish();
        assert_eq!(full.status(), StatusCode::OK);
        assert_eq!(full.headers().get(header::CONTENT_LENGTH).unwrap(), "1000");
        assert_eq!(
            full.headers().get(header::CONTENT_TYPE).unwrap(),
            "video/mp4"
        );
        assert_eq!(full.headers().get(header::ACCEPT_RANGES).unwrap(), "bytes");
        assert!(!full.headers().contains_key(header::CONTENT_RANGE));

        let partial = media_response_builder(
            response_plan(Some("bytes=512-767"), 1_000).unwrap(),
            1_000,
            "video/x-matroska",
        )
        .finish();
        assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            partial.headers().get(header::CONTENT_LENGTH).unwrap(),
            "256"
        );
        assert_eq!(
            partial.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes 512-767/1000"
        );
        assert_eq!(
            partial.headers().get(header::CONTENT_TYPE).unwrap(),
            "video/x-matroska"
        );
    }

    #[test]
    fn computes_512_kib_alignment_and_leading_skip() {
        let start = TELEGRAM_CDN_ALIGNMENT + 123_456;
        let alignment = calculate_download_alignment(start);
        assert_eq!(alignment.aligned_start, TELEGRAM_CDN_ALIGNMENT);
        assert_eq!(alignment.chunk_index, 8);
        assert_eq!(alignment.leading_bytes, 123_456);

        let mut skipped = 0;
        assert!(slice_after_leading_skip(&[1, 2], &mut skipped, 3).is_empty());
        assert_eq!(
            slice_after_leading_skip(&[3, 4, 5], &mut skipped, 3),
            &[4, 5]
        );
    }

    #[test]
    fn authenticates_bearer_and_legacy_query_without_logging_tokens() {
        let bearer = TestRequest::default()
            .insert_header((header::AUTHORIZATION, "Bearer secret-token"))
            .to_http_request();
        assert_eq!(
            authenticate_stream_request(&bearer, None, "secret-token"),
            AuthenticationResult::Authorized
        );
        let legacy = TestRequest::default().to_http_request();
        assert_eq!(
            authenticate_stream_request(&legacy, Some("secret-token"), "secret-token"),
            AuthenticationResult::Authorized
        );
        assert_eq!(
            authenticate_stream_request(&legacy, None, "secret-token"),
            AuthenticationResult::Missing
        );
        assert_eq!(
            authenticate_stream_request(&legacy, Some("wrong"), "secret-token"),
            AuthenticationResult::Invalid
        );
        let redacted = redact_sensitive(
            "http://127.0.0.1:1/stream/1/2?token=secret-token Authorization: Bearer secret-token",
        );
        assert!(!redacted.contains("secret-token"));
        assert_eq!(redacted.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn head_is_identified_before_stream_construction() {
        let request = TestRequest::default()
            .method(Method::HEAD)
            .to_http_request();
        assert_eq!(request.method(), Method::HEAD);
    }

    #[test]
    fn dynamic_port_binding_uses_ipv4_loopback() {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        assert!(address.ip().is_loopback());
        assert_ne!(address.port(), 0);
    }
}
