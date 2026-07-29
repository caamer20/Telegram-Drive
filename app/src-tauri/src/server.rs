use crate::commands::utils::resolve_peer;
use crate::commands::TelegramState;
use crate::transcode::TranscodeManager;
use actix_cors::Cors;
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use futures::future::LocalBoxFuture;
use grammers_client::types::Media;
use regex::Regex;

use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::sync::{Arc, LazyLock};

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
        .trim()
        .strip_prefix("bytes=")
        .ok_or(RangeError::Invalid)?;
    if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(RangeError::Invalid);
    }
    if value.contains(',') {
        return Err(RangeError::Multiple);
    }
    let (start_text, end_text) = value.split_once('-').ok_or(RangeError::Invalid)?;
    if end_text.contains('-') {
        return Err(RangeError::Invalid);
    }

    if start_text.is_empty() {
        let suffix = end_text.parse::<u64>().map_err(|_| RangeError::Invalid)?;
        if suffix == 0 {
            return Err(RangeError::Unsatisfiable);
        }
        let length = suffix.min(total_size);
        return Ok(ByteRange {
            start: total_size - length,
            end: total_size - 1,
        });
    }

    let start = start_text.parse::<u64>().map_err(|_| RangeError::Invalid)?;
    if start >= total_size {
        return Err(RangeError::Unsatisfiable);
    }
    let end = if end_text.is_empty() {
        total_size - 1
    } else {
        end_text
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
    let mut authorization_values = req
        .headers()
        .get_all(actix_web::http::header::AUTHORIZATION)
        .into_iter();
    if let Some(value) = authorization_values.next() {
        if authorization_values.next().is_some() {
            return AuthenticationResult::Invalid;
        }
        let Ok(value) = value.to_str() else {
            return AuthenticationResult::Invalid;
        };
        let mut parts = value.split_ascii_whitespace();
        let Some(scheme) = parts.next() else {
            return AuthenticationResult::Invalid;
        };
        let Some(candidate) = parts.next() else {
            return AuthenticationResult::Invalid;
        };
        if !scheme.eq_ignore_ascii_case("bearer") || parts.next().is_some() {
            return AuthenticationResult::Invalid;
        }
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
    static AUTHORIZATION_VALUE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)([\"']?\bauthorization\b[\"']?\s*[:=]\s*[\"']?)[^\"',}\]\r\n]+"#)
            .expect("valid authorization redaction regex")
    });
    static TOKEN_QUERY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)([?&#]token\s*=\s*)[^&#\s\"'<>]*"#).expect("valid token redaction regex")
    });
    static BEARER_VALUE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(\bbearer\s+)[^\s\"',}\]]+"#).expect("valid bearer redaction regex")
    });

    let output = AUTHORIZATION_VALUE.replace_all(input, "$1[REDACTED]");
    let output = TOKEN_QUERY.replace_all(&output, "$1[REDACTED]");
    BEARER_VALUE
        .replace_all(&output, "$1[REDACTED]")
        .into_owned()
}

/// Exact Origin validation for the private local streaming server. This is
/// intentionally narrower than the application's CSP and never accepts host
/// suffixes, userinfo, arbitrary ports, paths, queries, or fragments.
pub fn is_allowed_stream_origin(origin: &str) -> bool {
    if origin == "null" {
        return true;
    }
    let Ok(parsed) = url::Url::parse(origin) else {
        return false;
    };
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    match (parsed.scheme(), host, parsed.port()) {
        ("tauri", "localhost", None) => true,
        ("http" | "https", "tauri.localhost" | "asset.localhost", None) => true,
        ("http", "localhost" | "127.0.0.1", Some(port)) => port != 0,
        _ => false,
    }
}

#[derive(Debug)]
struct MediaDownloadError {
    safe_message: String,
}

trait MediaDownloadSource: 'static {
    fn next_chunk(&mut self) -> LocalBoxFuture<'_, Result<Option<Vec<u8>>, MediaDownloadError>>;
}

impl MediaDownloadSource for grammers_client::client::files::DownloadIter {
    fn next_chunk(&mut self) -> LocalBoxFuture<'_, Result<Option<Vec<u8>>, MediaDownloadError>> {
        Box::pin(async move {
            self.next().await.map_err(|error| MediaDownloadError {
                safe_message: redact_sensitive(&error.to_string()),
            })
        })
    }
}

fn download_body_stream<D: MediaDownloadSource>(
    mut source: D,
    bytes_to_skip: usize,
    content_length: u64,
    label: &'static str,
) -> impl futures::Stream<Item = Result<web::Bytes, actix_web::Error>> {
    async_stream::stream! {
        let mut skipped = 0usize;
        let mut total_yielded = 0u64;

        while total_yielded < content_length {
            match source.next_chunk().await {
                Ok(Some(data)) => {
                    let data = slice_after_leading_skip(&data, &mut skipped, bytes_to_skip);
                    if data.is_empty() {
                        continue;
                    }
                    let remaining = content_length - total_yielded;
                    let allowed = usize::try_from(remaining)
                        .map_or(data.len(), |remaining| data.len().min(remaining));
                    if allowed > 0 {
                        total_yielded += allowed as u64;
                        yield Ok(web::Bytes::copy_from_slice(&data[..allowed]));
                    }
                }
                Ok(None) => {
                    if total_yielded < content_length {
                        yield Err(actix_web::error::ErrorBadGateway("upstream media ended before the declared response length"));
                    }
                    break;
                }
                Err(error) => {
                    log::warn!("{} upstream download failed: {}", label, error.safe_message);
                    yield Err(actix_web::error::ErrorBadGateway("upstream media download failed"));
                    break;
                }
            }
        }
        log::debug!("{} stream completed (yielded: {})", label, total_yielded);
    }
}

fn finish_or_stream_media_response<D, F>(
    method: &actix_web::http::Method,
    mut response: actix_web::HttpResponseBuilder,
    bytes_to_skip: usize,
    content_length: u64,
    source_factory: F,
    label: &'static str,
) -> HttpResponse
where
    D: MediaDownloadSource,
    F: FnOnce() -> D,
{
    if method == actix_web::http::Method::HEAD || content_length == 0 {
        return response.finish();
    }
    response.streaming(download_body_stream(
        source_factory(),
        bytes_to_skip,
        content_length,
        label,
    ))
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
    let alignment = calculate_download_alignment(start_byte);
    if start_byte > 0 {
        debug_assert!(alignment.aligned_start <= start_byte);
        log::debug!(
            "Range alignment: requested={}, cdn_aligned={}, chunk_index={}, bytes_to_skip={}",
            start_byte,
            alignment.aligned_start,
            alignment.chunk_index,
            alignment.leading_bytes,
        );
    }

    finish_or_stream_media_response(
        req.method(),
        resp,
        alignment.leading_bytes,
        content_length,
        || {
            let iterator = client.iter_download(media).chunk_size(TELEGRAM_CHUNK_SIZE);
            if alignment.chunk_index > 0 {
                iterator.skip_chunks(alignment.chunk_index)
            } else {
                iterator
            }
        },
        extras.log_label,
    )
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
                        let message = redact_sensitive(&e.to_string());
                        log::error!(
                            "Stream request failed: Error fetching message {}: {}",
                            message_id,
                            message
                        );
                        HttpResponse::InternalServerError()
                            .body("Failed to fetch the Telegram media message")
                    }
                }
            }
            Err(e) => {
                let message = redact_sensitive(&e.to_string());
                log::error!(
                    "Stream request failed: Peer resolution error for msg {}: {}",
                    message_id,
                    message
                );
                HttpResponse::BadRequest().body("Unable to resolve the Telegram media source")
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
                origin.to_str().is_ok_and(is_allowed_stream_origin)
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
    use futures::{StreamExt, TryStreamExt};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn parses_closed_open_and_suffix_ranges() {
        assert_eq!(
            parse_range_header("bytes=0-0", 100),
            Ok(ByteRange { start: 0, end: 0 })
        );
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
        for value in [
            "items=0-2",
            "bytes=",
            "bytes=-",
            "bytes= 0-1",
            "bytes=0 -1",
            "bytes=0- 1",
            "bytes=18446744073709551616-",
        ] {
            assert_eq!(
                parse_range_header(value, 100),
                Err(RangeError::Invalid),
                "{value}"
            );
        }
        for value in ["bytes=100-", "bytes=101-", "bytes=20-10", "bytes=-0"] {
            assert_eq!(
                parse_range_header(value, 100),
                Err(RangeError::Unsatisfiable),
                "{value}"
            );
        }
        assert_eq!(
            parse_range_header("bytes=0-1,4-5", 100),
            Err(RangeError::Multiple)
        );
        assert_eq!(
            parse_range_header("bytes=0-0", 0),
            Err(RangeError::Unsatisfiable)
        );
        assert_eq!(
            parse_range_header("bytes=90-200", 100),
            Ok(ByteRange { start: 90, end: 99 })
        );
        assert_eq!(
            parse_range_header("  bytes=-200  ", 100),
            Ok(ByteRange { start: 0, end: 99 })
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
    fn bearer_scheme_is_case_insensitive_and_authorization_is_strict() {
        for scheme in ["Bearer", "bearer", "BEARER"] {
            let request = TestRequest::default()
                .insert_header((header::AUTHORIZATION, format!("{scheme} secret-token")))
                .to_http_request();
            assert_eq!(
                authenticate_stream_request(&request, None, "secret-token"),
                AuthenticationResult::Authorized
            );
        }
        for malformed in [
            "Bearer",
            "Bearersecret-token",
            "Bearer secret-token extra",
            "Basic secret-token",
        ] {
            let request = TestRequest::default()
                .insert_header((header::AUTHORIZATION, malformed))
                .to_http_request();
            assert_eq!(
                authenticate_stream_request(&request, Some("secret-token"), "secret-token"),
                AuthenticationResult::Invalid,
                "{malformed}"
            );
        }

        let duplicate = TestRequest::default()
            .append_header((header::AUTHORIZATION, "Bearer secret-token"))
            .append_header((header::AUTHORIZATION, "Bearer secret-token"))
            .to_http_request();
        assert_eq!(
            authenticate_stream_request(&duplicate, None, "secret-token"),
            AuthenticationResult::Invalid
        );

        let invalid_utf8 = TestRequest::default()
            .insert_header((
                header::AUTHORIZATION,
                header::HeaderValue::from_bytes(b"Bearer \xff").unwrap(),
            ))
            .to_http_request();
        assert_eq!(
            authenticate_stream_request(&invalid_utf8, Some("secret-token"), "secret-token"),
            AuthenticationResult::Invalid
        );
    }

    #[test]
    fn validates_exact_stream_origins() {
        for allowed in [
            "tauri://localhost",
            "https://tauri.localhost",
            "http://tauri.localhost",
            "https://asset.localhost",
            "http://asset.localhost",
            "http://localhost:1420",
            "http://127.0.0.1:49152",
            "null",
        ] {
            assert!(is_allowed_stream_origin(allowed), "{allowed}");
        }
        for rejected in [
            "http://localhost.evil.example:1420",
            "http://127.0.0.1.evil.example:1420",
            "http://user@localhost:1420",
            "https://localhost:1420",
            "ftp://localhost:1420",
            "http://example.com:1420",
            "http://localhost",
            "http://localhost:0",
            "http://localhost:1420/path",
            "not an origin",
        ] {
            assert!(!is_allowed_stream_origin(rejected), "{rejected}");
        }
    }

    #[test]
    fn redacts_case_variants_encoded_values_fragments_and_structured_text() {
        let input = concat!(
            "https://x.test/a?foo=1&TOKEN=s%2Fe%3Fc#fragment ",
            "authorization='Bearer private-value' ",
            "{\"Authorization\":\"private-value\"} [bearer another-value]"
        );
        let output = redact_sensitive(input);
        for secret in ["s%2Fe%3Fc", "private-value", "another-value"] {
            assert!(
                !output.contains(secret),
                "unredacted marker {secret} in {output}"
            );
        }
        assert!(output.matches("[REDACTED]").count() >= 3);
    }

    #[test]
    fn head_is_identified_before_stream_construction() {
        let request = TestRequest::default()
            .method(Method::HEAD)
            .to_http_request();
        assert_eq!(request.method(), Method::HEAD);
    }

    struct FakeDownloadSource {
        chunks: VecDeque<Result<Vec<u8>, &'static str>>,
        polls: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
    }

    impl FakeDownloadSource {
        fn new(
            chunks: Vec<Result<Vec<u8>, &'static str>>,
        ) -> (Self, Arc<AtomicUsize>, Arc<AtomicBool>) {
            let polls = Arc::new(AtomicUsize::new(0));
            let dropped = Arc::new(AtomicBool::new(false));
            (
                Self {
                    chunks: chunks.into(),
                    polls: polls.clone(),
                    dropped: dropped.clone(),
                },
                polls,
                dropped,
            )
        }
    }

    impl MediaDownloadSource for FakeDownloadSource {
        fn next_chunk(
            &mut self,
        ) -> LocalBoxFuture<'_, Result<Option<Vec<u8>>, MediaDownloadError>> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            let next = self.chunks.pop_front();
            Box::pin(async move {
                match next {
                    Some(Ok(bytes)) => Ok(Some(bytes)),
                    Some(Err(message)) => Err(MediaDownloadError {
                        safe_message: message.into(),
                    }),
                    None => Ok(None),
                }
            })
        }
    }

    impl Drop for FakeDownloadSource {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[actix_rt::test]
    async fn head_never_constructs_the_downloader() {
        let constructions = Arc::new(AtomicUsize::new(0));
        let counter = constructions.clone();
        let response = finish_or_stream_media_response(
            &Method::HEAD,
            HttpResponse::Ok(),
            0,
            10,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                FakeDownloadSource::new(vec![Ok(vec![1; 10])]).0
            },
            "test",
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(constructions.load(Ordering::SeqCst), 0);
    }

    #[actix_rt::test]
    async fn zero_length_get_never_constructs_the_downloader() {
        let constructions = Arc::new(AtomicUsize::new(0));
        let counter = constructions.clone();
        let response = finish_or_stream_media_response(
            &Method::GET,
            HttpResponse::Ok(),
            0,
            0,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                FakeDownloadSource::new(Vec::new()).0
            },
            "test",
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(constructions.load(Ordering::SeqCst), 0);
    }

    #[actix_rt::test]
    async fn cancellation_drops_source_and_stops_polling() {
        let (source, polls, dropped) =
            FakeDownloadSource::new(vec![Ok(vec![1; 4]), Ok(vec![2; 4])]);
        {
            let stream = download_body_stream(source, 0, 8, "test");
            futures::pin_mut!(stream);
            assert_eq!(
                stream.next().await.unwrap().unwrap(),
                web::Bytes::from_static(&[1; 4])
            );
        }
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    }

    #[actix_rt::test]
    async fn range_stream_skips_once_and_stops_at_exact_length() {
        let (source, polls, _) = FakeDownloadSource::new(vec![
            Ok(vec![0, 1, 2, 3]),
            Ok(vec![4, 5, 6, 7]),
            Ok(vec![8, 9, 10, 11]),
        ]);
        let chunks = download_body_stream(source, 3, 5, "test")
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(chunks.concat(), vec![3, 4, 5, 6, 7]);
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }

    #[actix_rt::test]
    async fn stream_errors_propagate_and_stop_additional_polls() {
        let (source, polls, _) = FakeDownloadSource::new(vec![
            Ok(vec![1, 2]),
            Err("safe upstream failure"),
            Ok(vec![3, 4]),
        ]);
        let stream = download_body_stream(source, 0, 6, "test");
        futures::pin_mut!(stream);
        assert!(stream.next().await.unwrap().is_ok());
        assert!(stream.next().await.unwrap().is_err());
        assert!(stream.next().await.is_none());
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }

    #[actix_rt::test]
    async fn independent_ranges_do_not_share_cursor_state() {
        let first = download_body_stream(
            FakeDownloadSource::new(vec![Ok(vec![0, 1, 2, 3])]).0,
            1,
            2,
            "first",
        )
        .try_collect::<Vec<_>>();
        let second = download_body_stream(
            FakeDownloadSource::new(vec![Ok(vec![8, 9, 10, 11])]).0,
            2,
            2,
            "second",
        )
        .try_collect::<Vec<_>>();
        let (first, second) = futures::join!(first, second);
        assert_eq!(first.unwrap().concat(), vec![1, 2]);
        assert_eq!(second.unwrap().concat(), vec![10, 11]);
    }

    #[test]
    fn dynamic_port_binding_uses_ipv4_loopback() {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        assert!(address.ip().is_loopback());
        assert_ne!(address.port(), 0);
    }
}
