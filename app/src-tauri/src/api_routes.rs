use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use crate::commands::TelegramState;
use crate::commands::streaming::StreamConfig;
use crate::commands::utils::resolve_peer;
use crate::server::StreamTokenData;
use grammers_client::types::{Media, Peer};
use grammers_tl_types as tl;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::Instant;

/// Shared state for the API server — holds the key hash for auth checks
/// and a cache for the folder list (Grammers' iter_dialogs() exhausts on
/// first use and takes ~10s to recover).
pub struct ApiState {
    pub key_hash: Option<String>,
    pub folder_cache: RwLock<Option<FolderCache>>,
}

/// Cached result of a folder scan — avoids re-scanning dialogs on every
/// request.
pub struct FolderCache {
    pub folders: Vec<ApiFolder>,
    pub cached_at: Instant,
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

fn json_error(code: &str, message: &str, status: u16) -> HttpResponse {
    let body = ErrorBody {
        error: ErrorDetail {
            code: code.to_string(),
            message: message.to_string(),
        },
    };
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap())
        .json(body)
}

/// Validate API key against stored hash.
///
/// Checks the `X-API-Key` header first, then falls back to the `api_key`
/// query parameter. The query-parameter fallback is needed by the mobile
/// Flutter app which uses `VideoPlayerController.networkUrl()` — that
/// widget cannot set custom HTTP headers so the key must be in the URL.
fn check_auth(req: &HttpRequest, api_state: &web::Data<ApiState>) -> Result<(), HttpResponse> {
    let key_hash = match &api_state.key_hash {
        Some(h) => h,
        None => return Err(json_error("NO_KEY_CONFIGURED", "No API key has been configured. Generate one in Settings.", 401)),
    };

    // Prefer header (more secure — not logged in URLs).
    let from_header: Option<String> = req
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    // Fall back to query parameter (needed by video_player on mobile).
    let from_query: Option<String> = actix_web::web::Query::<FolderQuery>::from_query(req.query_string())
        .ok()
        .and_then(|q| q.api_key.clone());

    let provided = from_header.or(from_query);

    match provided.as_deref() {
        Some(key) if crate::commands::api_settings::verify_key(key, key_hash) => Ok(()),
        Some(_) => Err(json_error("UNAUTHORIZED", "Invalid API key", 401)),
        None => Err(json_error("UNAUTHORIZED", "Missing API key (provide X-API-Key header or ?api_key= parameter)", 401)),
    }
}

// ──────────────────────────────── Endpoints ────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    stream_token: String,
    stream_port: u16,
}

#[get("/api/v1/health")]
async fn api_health(
    token_data: web::Data<StreamTokenData>,
    stream_config: web::Data<StreamConfig>,
) -> impl Responder {
    HttpResponse::Ok().json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        stream_token: token_data.token.clone(),
        stream_port: stream_config.port,
    })
}

#[derive(serde::Deserialize)]
struct FilesQuery {
    folder_id: Option<i64>,
    page: Option<u32>,
    limit: Option<u32>,
    search: Option<String>,
}

#[derive(Serialize)]
struct FilesResponse {
    files: Vec<ApiFile>,
    page: u32,
    limit: u32,
    total: usize,
}

#[derive(Serialize)]
struct ApiFile {
    id: i64,
    folder_id: Option<i64>,
    name: String,
    size: u64,
    mime_type: Option<String>,
    created_at: String,
}

#[get("/api/v1/files")]
async fn api_list_files(
    req: HttpRequest,
    query: web::Query<FilesQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return e;
    }

    let client_opt = { tg_state.client.lock().await.clone() };
    let client = match client_opt {
        Some(c) => c,
        None => return json_error("NOT_CONNECTED", "Telegram client is not connected", 503),
    };

    let peer = match resolve_peer(&client, query.folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => return json_error("PEER_ERROR", &e, 400),
    };

    let mut all_files: Vec<ApiFile> = Vec::new();
    let mut msgs = client.iter_messages(&peer);

    while let Some(msg) = msgs.next().await.ok().flatten() {
        if let Some(doc) = msg.media() {
            let (name, size, mime) = match doc {
                Media::Document(d) => {
                    (d.name().to_string(), d.size(), d.mime_type().map(|s| s.to_string()))
                }
                Media::Photo(_) => ("Photo.jpg".to_string(), 0, Some("image/jpeg".into())),
                _ => ("Unknown".to_string(), 0, None),
            };

            // Apply search filter if provided
            if let Some(ref search) = query.search {
                if !name.to_lowercase().contains(&search.to_lowercase()) {
                    continue;
                }
            }

            all_files.push(ApiFile {
                id: msg.id() as i64,
                folder_id: query.folder_id,
                name,
                size: size as u64,
                mime_type: mime,
                created_at: msg.date().to_string(),
            });
        }
    }

    let total = all_files.len();
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(50).min(200).max(1);
    let start = ((page - 1) * limit) as usize;
    let paginated: Vec<ApiFile> = all_files.into_iter().skip(start).take(limit as usize).collect();

    HttpResponse::Ok().json(FilesResponse {
        files: paginated,
        page,
        limit,
        total,
    })
}

#[derive(serde::Deserialize)]
struct FolderQuery {
    folder_id: Option<i64>,
    /// API key passed as query parameter (used by `VideoPlayerController.networkUrl()`
    /// on mobile which cannot set custom HTTP headers).
    api_key: Option<String>,
}

#[get("/api/v1/files/{message_id}")]
async fn api_get_file(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<FolderQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return e;
    }

    let message_id = path.into_inner() as i32;
    let client_opt = { tg_state.client.lock().await.clone() };
    let client = match client_opt {
        Some(c) => c,
        None => return json_error("NOT_CONNECTED", "Telegram client is not connected", 503),
    };

    let peer = match resolve_peer(&client, query.folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => return json_error("PEER_ERROR", &e, 400),
    };

    match client.get_messages_by_id(peer, &[message_id]).await {
        Ok(messages) => {
            if let Some(Some(msg)) = messages.first() {
                if let Some(doc) = msg.media() {
                    let (name, size, mime) = match doc {
                        Media::Document(d) => {
                            (d.name().to_string(), d.size(), d.mime_type().map(|s| s.to_string()))
                        }
                        Media::Photo(_) => ("Photo.jpg".to_string(), 0, Some("image/jpeg".into())),
                        _ => ("Unknown".to_string(), 0, None),
                    };
                    return HttpResponse::Ok().json(ApiFile {
                        id: msg.id() as i64,
                        folder_id: query.folder_id,
                        name,
                        size: size as u64,
                        mime_type: mime,
                        created_at: msg.date().to_string(),
                    });
                }
            }
            json_error("NOT_FOUND", "File not found", 404)
        }
        Err(e) => json_error("FETCH_ERROR", &format!("Failed to fetch file: {}", e), 500),
    }
}

#[get("/api/v1/files/{message_id}/download")]
async fn api_download_file(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<FolderQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return e;
    }

    let message_id = path.into_inner() as i32;
    let client_opt = { tg_state.client.lock().await.clone() };
    let client = match client_opt {
        Some(c) => c,
        None => return json_error("NOT_CONNECTED", "Telegram client is not connected", 503),
    };

    let peer = match resolve_peer(&client, query.folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => return json_error("PEER_ERROR", &e, 400),
    };

    match client.get_messages_by_id(peer, &[message_id]).await {
        Ok(messages) => {
            if let Some(Some(msg)) = messages.first() {
                if let Some(media) = msg.media() {
                    let size = match &media {
                        Media::Document(d) => d.size(),
                        _ => 0,
                    };
                    let mime = match &media {
                        Media::Document(d) => d.mime_type().unwrap_or("application/octet-stream").to_string(),
                        _ => "application/octet-stream".to_string(),
                    };
                    let filename = match &media {
                        Media::Document(d) => d.name().to_string(),
                        Media::Photo(_) => "Photo.jpg".to_string(),
                        _ => "download".to_string(),
                    };

                    let mut download_iter = client.iter_download(&media);
                    let stream = async_stream::stream! {
                        while let Some(chunk) = download_iter.next().await.transpose() {
                            match chunk {
                                Ok(bytes) => yield Ok::<_, actix_web::Error>(web::Bytes::from(bytes)),
                                Err(e) => {
                                    log::error!("API download stream error: {}", e);
                                    break;
                                }
                            }
                        }
                    };

                    return HttpResponse::Ok()
                        .insert_header(("Content-Type", mime))
                        .insert_header(("Content-Length", size.to_string()))
                        .insert_header(("Content-Disposition", format!("attachment; filename=\"{}\"", filename)))
                        .insert_header(("Accept-Ranges", "bytes"))
                        .streaming(stream);
                }
            }
            json_error("NOT_FOUND", "File not found", 404)
        }
        Err(e) => json_error("FETCH_ERROR", &format!("Failed to fetch file: {}", e), 500),
    }
}

#[derive(Clone, Serialize)]
pub struct ApiFolder {
    id: i64,
    name: String,
}

/// Maximum number of dialogs to scan when listing folders.
///
/// Scanning all dialogs can take minutes for users with hundreds of
/// Telegram chats, and each channel that doesn't have the [TD] marker
/// requires an additional `GetFullChannel` API call. This limit keeps
/// the response time predictable.
const MAX_FOLDER_DIALOGS: usize = 100;

/// Hard deadline for the folders endpoint (wall-clock, seconds).
///
/// If the scan takes longer, we return whatever folders were found
/// so far rather than making the client wait or time out.
const FOLDER_SCAN_TIMEOUT_SECS: u64 = 5;

/// How long (in seconds) to cache the folder list before re-scanning.
/// Grammers' `iter_dialogs()` returns stale/empty results if called a
/// second time within ~10s, so a 30s TTL keeps the cache warm without
/// excessive staleness.
const FOLDER_CACHE_TTL_SECS: u64 = 30;

#[get("/api/v1/folders")]
async fn api_list_folders(
    req: HttpRequest,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return e;
    }

    // ── Cache check (fast path) ──────────────────────────────────────
    {
        let cache = api_state.folder_cache.read().await;
        if let Some(ref cached) = *cache {
            if cached.cached_at.elapsed().as_secs() < FOLDER_CACHE_TTL_SECS {
                return HttpResponse::Ok().json(&cached.folders);
            }
        }
    }

    let client_opt = { tg_state.client.lock().await.clone() };
    let client = match client_opt {
        Some(c) => c,
        None => return json_error("NOT_CONNECTED", "Telegram client is not connected", 503),
    };

    let mut folders: Vec<ApiFolder> = Vec::new();
    let mut dialogs = client.iter_dialogs();
    let mut scanned: usize = 0;

    // Bound the whole scan with a deadline so we never hang indefinitely.
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(FOLDER_SCAN_TIMEOUT_SECS);

    while scanned < MAX_FOLDER_DIALOGS {
        // Check deadline before each dialog — if expired, return partial results.
        if tokio::time::Instant::now() >= deadline {
            log::info!(
                "Folder scan timeout after {scanned} dialogs, returning {} folders",
                folders.len()
            );
            break;
        }

        let dialog = match tokio::time::timeout_at(deadline, dialogs.next()).await {
            Ok(Ok(Some(d))) => d,
            _ => break,
        };

        scanned += 1;

        match &dialog.peer {
            Peer::Channel(c) => {
                let id = c.raw.id;
                let name = c.raw.title.clone();
                let access_hash = c.raw.access_hash.unwrap_or(0);

                // Fast path: check if channel name contains [TD] marker
                if name.to_lowercase().contains("[td]") {
                    let display_name = name
                        .replace(" [TD]", "")
                        .replace(" [td]", "")
                        .replace("[TD]", "")
                        .replace("[td]", "")
                        .trim()
                        .to_string();
                    folders.push(ApiFolder { id, name: display_name });
                    continue;
                }

                // Slow path: check if channel about contains [telegram-drive-folder]
                let input_chan = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: c.raw.id,
                    access_hash,
                });

                let invoke_result = tokio::time::timeout_at(
                    deadline,
                    client.invoke(&tl::functions::channels::GetFullChannel {
                        channel: input_chan,
                    }),
                )
                .await;

                if let Ok(Ok(tl::enums::messages::ChatFull::Full(f))) = invoke_result {
                    if let tl::enums::ChatFull::Full(cf) = f.full_chat {
                        if cf.about.contains("[telegram-drive-folder]") {
                            folders.push(ApiFolder { id, name: name.clone() });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    log::info!(
        "Folder scan finished: scanned {scanned} dialogs, found {} folders",
        folders.len()
    );

    // ── Update cache ────────────────────────────────────────────────
    {
        let mut cache = api_state.folder_cache.write().await;
        *cache = Some(FolderCache {
            folders: folders.clone(),
            cached_at: Instant::now(),
        });
    }

    HttpResponse::Ok().json(folders)
}

/// Register all API routes on the Actix App
pub fn configure_api(cfg: &mut web::ServiceConfig) {
    cfg.service(api_health)
       .service(api_list_folders)
       .service(api_list_files)
       .service(api_get_file)
       .service(api_download_file);
}

// ──────────────────────────────── Tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── hash_key / verify_key ─────────────────────────────────────────────

    #[test]
    fn test_verify_key_matches() {
        let hash = crate::commands::api_settings::hash_key("hello");
        assert!(crate::commands::api_settings::verify_key("hello", &hash));
    }

    #[test]
    fn test_verify_key_mismatch() {
        let hash = crate::commands::api_settings::hash_key("hello");
        assert!(!crate::commands::api_settings::verify_key("world", &hash));
    }

    #[test]
    fn test_verify_key_empty_string() {
        let hash = crate::commands::api_settings::hash_key("");
        assert!(crate::commands::api_settings::verify_key("", &hash));
        assert!(!crate::commands::api_settings::verify_key(" ", &hash));
    }

    #[test]
    fn test_hash_key_deterministic() {
        let a = crate::commands::api_settings::hash_key("test-key-123");
        let b = crate::commands::api_settings::hash_key("test-key-123");
        assert_eq!(a, b);
    }

    // ── json_error ────────────────────────────────────────────────────────

    #[test]
    fn test_json_error_structure() {
        let resp = json_error("TEST_CODE", "test message", 400);
        assert_eq!(resp.status(), 400);
    }

    #[test]
    fn test_json_error_various_status_codes() {
        for status in [200, 400, 401, 403, 404, 500, 503] {
            let resp = json_error("CODE", "msg", status);
            assert_eq!(
                resp.status(),
                status,
                "json_error should preserve status code {status}"
            );
        }
    }

    // ── check_auth (via verify_key) ───────────────────────────────────────

    #[test]
    fn test_sha256_verification() {
        let plaintext = "5b85c90a809e870a3ccc6c1d347b7e1d80e0693360720b8f3a434b3da55895a5";
        let hash = crate::commands::api_settings::hash_key(plaintext);
        assert!(
            crate::commands::api_settings::verify_key(plaintext, &hash),
            "SHA-256 verification should match"
        );
    }

}

#[test]
fn test_simple_works() {
    assert_eq!(2 + 2, 4);
}

