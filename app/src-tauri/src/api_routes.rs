use crate::api_catalog::{self, ApiFile, CatalogError};
use crate::bandwidth::{BandwidthManager, BandwidthReservation};
use crate::commands::fs::verify_forwarded_messages;
use crate::commands::preview::THUMBNAIL_EXTS;
use crate::commands::utils::{map_error, media_size, resolve_peer};
use crate::commands::TelegramState;
use crate::commands::{create_folder_inner, delete_folder_inner, rename_folder_inner};
use crate::crypto::policy::TELEGRAM_MAX_FILE_SIZE;
use crate::models::FolderMetadata;
use crate::vpn_optimizer::NetworkConfig;
use crate::workspace::AccountGuard;
use actix_multipart::Multipart;
use actix_web::web::Bytes;
use actix_web::{delete, get, patch, post, web, HttpRequest, HttpResponse, Responder};
use futures::{StreamExt, TryStreamExt};
use grammers_client::types::{Media, Peer};
use grammers_client::InputMessage;
use grammers_tl_types as tl;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWriteExt};

const MAX_MULTIPART_METADATA_BYTES: usize = 128;
const MAX_UPLOAD_FILENAME_CHARS: usize = 255;

fn sanitise_upload_filename(value: &str) -> String {
    let basename = value.rsplit(['/', '\\']).next().unwrap_or(value).trim();
    let cleaned: String = basename
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_UPLOAD_FILENAME_CHARS)
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "file".to_string()
    } else {
        cleaned
    }
}

/// Shared state for the API server — holds the key hash for auth checks
pub struct ApiState {
    pub key_hash: Option<String>,
}

/// Cache directory paths used by the API server for cleanup operations.
/// The thumbnail and preview caches live on disk and can become stale
/// when files are moved (forwarded → new message IDs).
pub struct CacheDirs {
    pub account_root: std::path::PathBuf,
    pub thumbnail_dir: std::path::PathBuf,
    pub preview_dir: std::path::PathBuf,
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
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap()).json(body)
}

async fn api_registered_encrypted(
    account: &crate::workspace::AccountGuard,
    client: &grammers_client::Client,
    state: &TelegramState,
    folder_id: Option<i64>,
    message_id: i32,
) -> Result<bool, String> {
    account.validate()?;
    let peer = resolve_peer(client, folder_id, &state.peer_cache).await?;
    let message = client
        .get_messages_by_id(peer, &[message_id])
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .flatten()
        .next()
        .ok_or("File not found")?;
    let Some(media) = message.media() else {
        account.validate()?;
        return Ok(false);
    };
    let encrypted = crate::commands::fs::resolve_remote_envelope(
        account,
        client,
        folder_id,
        message_id,
        &media,
        message.text(),
    )
    .await?
    .is_some();
    account.validate()?;
    Ok(encrypted)
}

async fn api_account_client(
    root: &std::path::Path,
    state: &TelegramState,
) -> Result<(crate::workspace::AccountGuard, grammers_client::Client), String> {
    let account = crate::workspace::AccountGuard::open(root, None)?;
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or("Telegram client is not connected")?;
    account.validate_client(&client).await?;
    Ok((account, client))
}

fn catalog_error(error: CatalogError) -> HttpResponse {
    match error {
        CatalogError::NotConnected => {
            json_error("NOT_CONNECTED", "Telegram client is not connected", 503)
        }
        CatalogError::Account(error) => json_error("ACCOUNT_UNAVAILABLE", &error, 503),
        CatalogError::Folder(error) => json_error("PEER_ERROR", &error, 400),
        // A partial walk would be reported as a complete, shorter library.
        CatalogError::Remote(error) => json_error(
            "FETCH_ERROR",
            &format!("Telegram did not return the complete listing: {error}"),
            502,
        ),
    }
}

/// The account this request acts for. Every endpoint checks it again before
/// answering, so a response never crosses an account switch.
fn api_account(root: &std::path::Path) -> Result<AccountGuard, Box<HttpResponse>> {
    AccountGuard::open(root, None)
        .map_err(|error| Box::new(json_error("ACCOUNT_UNAVAILABLE", &error, 503)))
}

async fn api_session(
    root: &std::path::Path,
    state: &TelegramState,
) -> Result<(AccountGuard, grammers_client::Client), HttpResponse> {
    let account = api_account(root).map_err(|response| *response)?;
    let client = api_catalog::client(&account, state)
        .await
        .map_err(catalog_error)?;
    Ok((account, client))
}

/// File identifiers are Telegram message identifiers: positive and 32-bit.
/// Anything else is rejected rather than truncated to a different file.
fn message_id(value: i64) -> Result<i32, Box<HttpResponse>> {
    i32::try_from(value)
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            Box::new(json_error(
                "INVALID_FILE_ID",
                "File identifiers are positive 32-bit integers",
                400,
            ))
        })
}

/// `folder_id` as a query gives it: absent means every folder; an empty
/// value, `null`, `none` or `home` means Saved Messages.
fn folder_scope(value: Option<&str>) -> Result<Option<Option<i64>>, Box<HttpResponse>> {
    match value.map(str::trim) {
        None => Ok(None),
        Some("" | "null" | "none" | "None" | "home") => Ok(Some(None)),
        Some(value) => value.parse::<i64>().map(|id| Some(Some(id))).map_err(|_| {
            Box::new(json_error(
                "INVALID_FOLDER_ID",
                "folder_id must be an integer, or null for Saved Messages",
                400,
            ))
        }),
    }
}

fn time_bound(value: Option<&str>, name: &str) -> Result<Option<i64>, Box<HttpResponse>> {
    match value {
        None => Ok(None),
        Some(value) => api_catalog::parse_time(value).map(Some).ok_or_else(|| {
            Box::new(json_error(
                "INVALID_TIMESTAMP",
                &format!("{name} must be an RFC 3339 timestamp such as 2026-06-05T10:00:00Z"),
                400,
            ))
        }),
    }
}

fn api_scope_error(account: &crate::workspace::AccountGuard) -> Option<HttpResponse> {
    account
        .validate()
        .err()
        .map(|error| json_error("ACCOUNT_CHANGED", &error, 409))
}

fn account_response(
    account: &crate::workspace::AccountGuard,
    response: HttpResponse,
) -> HttpResponse {
    api_scope_error(account).unwrap_or(response)
}

async fn api_protected_response(
    account: &crate::workspace::AccountGuard,
    client: &grammers_client::Client,
    folder: Option<i64>,
    message: i32,
    media: &Media,
    caption: &str,
    explanation: &str,
) -> Option<HttpResponse> {
    match crate::commands::fs::resolve_remote_envelope(
        account, client, folder, message, media, caption,
    )
    .await
    {
        Ok(None) => None,
        Ok(Some(_)) => Some(json_error("ENCRYPTED_ROUTE_UNAVAILABLE", explanation, 409)),
        Err(error) => Some(json_error("ENCRYPTION_STATE_UNKNOWN", &error, 503)),
    }
}

struct CleanupStream {
    account: crate::workspace::AccountGuard,
    file: tokio::fs::File,
    path: std::path::PathBuf,
}

impl futures::Stream for CleanupStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Err(error) = this.account.validate() {
            return Poll::Ready(Some(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                error,
            ))));
        }
        let mut buf = [0u8; 16384];
        let mut read_buf = tokio::io::ReadBuf::new(&mut buf);
        let file_pin = std::pin::Pin::new(&mut this.file);
        match file_pin.poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) => {
                if let Err(error) = this.account.validate() {
                    return Poll::Ready(Some(Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        error,
                    ))));
                }
                let filled = read_buf.filled();
                if filled.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(Bytes::copy_from_slice(filled))))
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for CleanupStream {
    fn drop(&mut self) {
        let path = self.path.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::remove_file(path).await;
        });
    }
}

fn peer_to_input_peer(peer: &Peer) -> Result<tl::enums::InputPeer, String> {
    match peer {
        Peer::User(u) => {
            let (id, access_hash) = match &u.raw {
                tl::enums::User::User(usr) => (usr.id, usr.access_hash.unwrap_or(0)),
                tl::enums::User::Empty(usr) => (usr.id, 0),
            };
            Ok(tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id: id,
                access_hash,
            }))
        }
        Peer::Channel(c) => Ok(tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
            channel_id: c.raw.id,
            access_hash: c.raw.access_hash.ok_or("No access hash for channel")?,
        })),
        _ => Err("Unsupported peer type".to_string()),
    }
}

/// Spawn a blocking task to delete stale thumbnail and preview cache entries
/// for the given message IDs in the given source folder.
/// Best-effort: failures are silently ignored since cache cleanup is non-critical.
fn spawn_cache_cleanup(
    account: AccountGuard,
    thumb_dir: std::path::PathBuf,
    prev_dir: std::path::PathBuf,
    ids: Vec<i32>,
    folder_key: String,
) {
    tokio::spawn(async move {
        if let Some(cache) = prev_dir.parent() {
            let folder = folder_key.parse::<i64>().ok();
            for id in &ids {
                for thumbnail in [false, true] {
                    let _ = crate::workspace::assets::delete_cached_at(
                        cache.to_path_buf(),
                        account.clone(),
                        crate::workspace::store::file_key(folder, i64::from(*id)),
                        thumbnail,
                    )
                    .await;
                }
            }
        }
        tokio::task::spawn_blocking(move || {
            if account.validate().is_err() {
                return;
            }
            let legacy = crate::commands::preview::legacy_preview_mutation();
            let _shared = crate::workspace::cache_core::state();
            for mid in &ids {
                for ext in THUMBNAIL_EXTS {
                    let path =
                        thumb_dir.join(format!("{}_{}_{}.{}", account.owner, folder_key, mid, ext));
                    if path.exists()
                        && !legacy.is_active(&path)
                        && !crate::workspace::cache_core::kept(&path)
                    {
                        let _ = std::fs::remove_file(&path);
                    }
                }
                let prefix = format!("{}_{}_{}.", account.owner, folder_key, mid);
                if let Ok(entries) = std::fs::read_dir(&prev_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_file()
                            || legacy.is_active(&path)
                            || crate::workspace::cache_core::kept(&path)
                        {
                            continue;
                        }
                        if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                            if fname.starts_with(&prefix) {
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    }
                }
            }
        })
        .await
        .ok();
    });
}

/// Validate X-API-Key header against stored hash
fn check_auth(req: &HttpRequest, api_state: &web::Data<ApiState>) -> Result<(), Box<HttpResponse>> {
    let key_hash = match &api_state.key_hash {
        Some(h) => h,
        None => {
            return Err(Box::new(json_error(
                "NO_KEY_CONFIGURED",
                "No API key has been configured. Generate one in Settings.",
                401,
            )))
        }
    };

    let provided = req.headers().get("X-API-Key").and_then(|v| v.to_str().ok());

    match provided {
        Some(key) if crate::commands::api_settings::verify_key(key, key_hash) => Ok(()),
        Some(_) => Err(Box::new(json_error("UNAUTHORIZED", "Invalid API key", 401))),
        None => Err(Box::new(json_error(
            "UNAUTHORIZED",
            "Missing X-API-Key header",
            401,
        ))),
    }
}

// ──────────────────────────────── Endpoints ────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}

#[get("/api/v1/health")]
async fn api_health() -> impl Responder {
    HttpResponse::Ok().json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// The machine-readable contract for every route below.
pub const OPENAPI_DOCUMENT: &str = include_str!("../api/openapi-v1.json");

#[get("/api/v1/openapi.json")]
async fn api_openapi() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/json")
        .body(OPENAPI_DOCUMENT)
}

#[derive(serde::Deserialize, Clone)]
struct FilesQuery {
    folder_id: Option<String>,
    page: Option<u32>,
    limit: Option<u32>,
    search: Option<String>,
    offset_id: Option<i64>,
    sort: Option<String>,
    order: Option<String>,
    mime_type: Option<String>,
    created_after: Option<String>,
    created_before: Option<String>,
    size_min: Option<u64>,
    size_max: Option<u64>,
    fields: Option<String>,
    refresh: Option<bool>,
}

#[derive(serde::Deserialize)]
struct RefreshQuery {
    refresh: Option<bool>,
}

#[derive(Serialize)]
struct FilesResponse {
    data: Vec<serde_json::Value>,
    files: Vec<serde_json::Value>, // For backwards compatibility
    page: u32,
    limit: u32,
    total: usize,
    /// False when a folder holds more files than one listing covers.
    complete: bool,
    pagination: PaginationInfo,
}

#[derive(Serialize)]
struct PaginationInfo {
    page: u32,
    limit: u32,
    total: usize,
    total_pages: u32,
    has_next: bool,
    has_prev: bool,
}

/// One file as JSON, limited to the requested fields when any were named.
fn project_file(file: &ApiFile, fields: Option<&[String]>) -> serde_json::Value {
    let mut value = serde_json::to_value(file).unwrap_or_default();
    if let (Some(fields), Some(map)) = (fields, value.as_object_mut()) {
        map.retain(|key, _| fields.iter().any(|field| field == key));
    }
    value
}

#[get("/api/v1/files")]
async fn api_list_files(
    req: HttpRequest,
    query: web::Query<FilesQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }

    let scope = match folder_scope(query.folder_id.as_deref()) {
        Ok(scope) => scope,
        Err(error) => return *error,
    };
    let created_after = match time_bound(query.created_after.as_deref(), "created_after") {
        Ok(bound) => bound,
        Err(error) => return *error,
    };
    let created_before = match time_bound(query.created_before.as_deref(), "created_before") {
        Ok(bound) => bound,
        Err(error) => return *error,
    };

    let account = match api_account(&cache_dirs.account_root) {
        Ok(account) => account,
        Err(error) => return *error,
    };
    if query.refresh == Some(true) {
        api_catalog::invalidate(account.owner);
    }
    let (files, complete) = match api_catalog::files(&account, tg_state.get_ref(), scope).await {
        Ok(listing) => listing,
        Err(error) => return catalog_error(error),
    };

    let search = query.search.as_ref().map(|search| search.to_lowercase());
    let mime_type = query.mime_type.as_ref().map(|mime| mime.to_lowercase());
    let mut filtered: Vec<ApiFile> = files
        .into_iter()
        .filter(|file| {
            search
                .as_ref()
                .is_none_or(|search| file.name.to_lowercase().contains(search))
                && mime_type.as_ref().is_none_or(|wanted| {
                    file.mime_type
                        .as_ref()
                        .is_some_and(|mime| mime.to_lowercase().contains(wanted))
                })
                && query.size_min.is_none_or(|minimum| file.size >= minimum)
                && query.size_max.is_none_or(|maximum| file.size <= maximum)
                && created_after.is_none_or(|after| file.timestamp >= after)
                && created_before.is_none_or(|before| file.timestamp <= before)
                // Files older than a known one, for cursor-style clients.
                && query.offset_id.is_none_or(|offset| file.id < offset)
        })
        .collect();

    let sort_field = query.sort.as_deref().unwrap_or("created_at");
    let descending = query
        .order
        .as_deref()
        .is_some_and(|order| order.eq_ignore_ascii_case("desc"));
    filtered.sort_by(|a, b| {
        let ordering = match sort_field {
            "name" => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            "size" => a.size.cmp(&b.size),
            _ => a.timestamp.cmp(&b.timestamp),
        };
        let ordering = if descending {
            ordering.reverse()
        } else {
            ordering
        };
        // Equal keys keep one order, so pages never repeat or skip a file.
        ordering.then_with(|| (a.folder_id, a.id).cmp(&(b.folder_id, b.id)))
    });

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let total = filtered.len();
    let total_pages = u32::try_from(total.div_ceil(limit as usize)).unwrap_or(u32::MAX);
    let start = (page as usize - 1).saturating_mul(limit as usize);

    let fields: Option<Vec<String>> = query.fields.as_ref().map(|fields| {
        fields
            .split(',')
            .map(|field| field.trim().to_string())
            .collect()
    });
    let data: Vec<serde_json::Value> = filtered
        .iter()
        .skip(start)
        .take(limit as usize)
        .map(|file| project_file(file, fields.as_deref()))
        .collect();

    account_response(
        &account,
        HttpResponse::Ok().json(FilesResponse {
            files: data.clone(),
            data,
            page,
            limit,
            total,
            complete,
            pagination: PaginationInfo {
                page,
                limit,
                total,
                total_pages,
                has_next: page < total_pages,
                has_prev: page > 1,
            },
        }),
    )
}

#[derive(serde::Deserialize)]
struct FolderQuery {
    folder_id: Option<i64>,
}

#[get("/api/v1/files/{message_id}")]
async fn api_get_file(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<FolderQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let message_id = match message_id(path.into_inner()) {
        Ok(id) => id,
        Err(error) => return *error,
    };
    let (account, client) = match api_session(&cache_dirs.account_root, tg_state.get_ref()).await {
        Ok(session) => session,
        Err(error) => return error,
    };
    let response = async {
        let peer = match resolve_peer(&client, query.folder_id, &tg_state.peer_cache).await {
            Ok(p) => p,
            Err(e) => return json_error("PEER_ERROR", &e, 400),
        };
        match client.get_messages_by_id(peer, &[message_id]).await {
            Ok(messages) => match messages
                .into_iter()
                .flatten()
                .next()
                .and_then(|message| api_catalog::file_from_message(&message, query.folder_id))
            {
                Some(file) => HttpResponse::Ok().json(file),
                None => json_error("NOT_FOUND", "File not found", 404),
            },
            Err(e) => json_error("FETCH_ERROR", &format!("Failed to fetch file: {}", e), 500),
        }
    }
    .await;
    account_response(&account, response)
}

#[get("/api/v1/files/{message_id}/download")]
async fn api_download_file(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<FolderQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }

    let message_id = match message_id(path.into_inner()) {
        Ok(id) => id,
        Err(error) => return *error,
    };
    let (account, client) =
        match api_account_client(&cache_dirs.account_root, tg_state.get_ref()).await {
            Ok(value) => value,
            Err(error) => return json_error("ACCOUNT_UNAVAILABLE", &error, 503),
        };
    let response = async {

    let peer = match resolve_peer(&client, query.folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => return json_error("PEER_ERROR", &e, 400),
    };

    match client.get_messages_by_id(peer, &[message_id]).await {
        Ok(messages) => {
            if let Some(Some(msg)) = messages.first() {
            if let Some(media) = msg.media() {
                if let Some(error) = api_protected_response(&account, &client, query.folder_id, message_id, &media, msg.text(), "Encrypted API downloads require a scoped decryption credential and are disabled").await { return error; }
                    let mime = match &media {
                        Media::Document(d) => d
                            .mime_type()
                            .unwrap_or("application/octet-stream")
                            .to_string(),
                        _ => "application/octet-stream".to_string(),
                    };
                    let filename = match &media {
                        Media::Document(d) => d.name().to_string(),
                        Media::Photo(_) => "Photo.jpg".to_string(),
                        _ => "download".to_string(),
                    };

                    return crate::server::build_media_response_guarded(
                        &client,
                        &media,
                        &req,
                        &mime,
                        Some(&filename),
                        crate::server::StreamingExtras {
                            bandwidth:req.app_data::<web::Data<Arc<crate::bandwidth::BandwidthManager>>>().expect("Shared bandwidth accounting").get_ref().clone(),
                            network:req.app_data::<web::Data<Arc<crate::vpn_optimizer::NetworkConfig>>>().expect("Shared network state").get_ref().clone(),
                            extra_headers: vec![],
                            log_label: "API download",
                        },
                        Some(account.clone()),
                    );
                }
            }
            json_error("NOT_FOUND", "File not found", 404)
        }
        Err(e) => json_error("FETCH_ERROR", &format!("Failed to fetch file: {}", e), 500),
    }
    }.await;
    account_response(&account, response)
}

#[derive(serde::Deserialize)]
struct BulkRequest {
    action: String,
    file_ids: Vec<serde_json::Value>,
    folder_id: Option<serde_json::Value>,
    payload: Option<BulkPayload>,
}

#[derive(serde::Deserialize)]
struct BulkPayload {
    folder_id: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct BulkResponse {
    success: bool,
    count: usize,
}

fn parse_bulk_file_ids(values: &[serde_json::Value]) -> Result<Vec<i32>, String> {
    if values.is_empty() {
        return Err("Select at least one file".to_string());
    }
    values
        .iter()
        .map(|value| {
            let id = value
                .as_i64()
                .and_then(|id| i32::try_from(id).ok())
                .or_else(|| value.as_str().and_then(|id| id.parse::<i32>().ok()))
                .filter(|id| *id > 0);
            id.ok_or_else(|| {
                "Every selected file must have a valid Telegram message ID".to_string()
            })
        })
        .collect()
}

enum ArchivePart {
    File { name: String, large: bool },
    Data(Vec<u8>),
}

#[derive(Debug, PartialEq, Eq)]
enum ArchiveDownloadError {
    Remote(String),
    Incomplete {
        expected: u64,
        actual: u64,
    },
    /// The archive writer stopped; its own error says why.
    Writer,
}

/// Hand one selected file to the archive writer as it downloads, so memory use
/// does not grow with the selection. The file counts only if every remote read
/// succeeds and its byte length matches Telegram's declaration: ZIP integrity
/// alone cannot detect a truncated input, because the writer would checksum
/// those partial bytes.
async fn stream_archive_file<S>(
    chunks: S,
    expected_size: u64,
    writer: &tokio::sync::mpsc::Sender<ArchivePart>,
) -> Result<(), ArchiveDownloadError>
where
    S: futures::Stream<Item = Result<Vec<u8>, String>>,
{
    futures::pin_mut!(chunks);
    let mut actual = 0_u64;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(ArchiveDownloadError::Remote)?;
        actual = actual.saturating_add(chunk.len() as u64);
        if actual > expected_size {
            return Err(ArchiveDownloadError::Incomplete {
                expected: expected_size,
                actual,
            });
        }
        writer
            .send(ArchivePart::Data(chunk))
            .await
            .map_err(|_| ArchiveDownloadError::Writer)?;
    }
    if actual != expected_size {
        return Err(ArchiveDownloadError::Incomplete {
            expected: expected_size,
            actual,
        });
    }
    Ok(())
}

/// ZIP entries need distinct names; two selected files may share one.
fn archive_entry_name(name: String, used: &mut HashSet<String>) -> String {
    if used.insert(name.clone()) {
        return name;
    }
    let path = std::path::Path::new(&name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("file")
        .to_string();
    let extension = path.extension().and_then(|extension| extension.to_str());
    let mut copy = 2_u32;
    loop {
        let candidate = match extension {
            Some(extension) => format!("{stem} ({copy}).{extension}"),
            None => format!("{stem} ({copy})"),
        };
        if used.insert(candidate.clone()) {
            return candidate;
        }
        copy += 1;
    }
}

/// All ZIP I/O runs on a blocking thread and never touches Actix workers.
fn spawn_archive_writer(
    path: std::path::PathBuf,
    mut parts: tokio::sync::mpsc::Receiver<ArchivePart>,
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio::task::spawn_blocking(move || {
        // `create_new` refuses to reuse an existing name instead of truncating it.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("ZIP_CREATE_FAILED: {}", e))?;
        let mut zip = zip::ZipWriter::new(file);
        while let Some(part) = parts.blocking_recv() {
            match part {
                ArchivePart::File { name, large } => zip
                    .start_file(
                        name,
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated)
                            .large_file(large),
                    )
                    .map_err(|e| format!("ZIP_ADD_FAILED: {}", e))?,
                ArchivePart::Data(bytes) => zip
                    .write_all(&bytes)
                    .map_err(|e| format!("ZIP_WRITE_FAILED: {}", e))?,
            }
        }
        zip.finish()
            .map_err(|e| format!("ZIP_FINISH_FAILED: {}", e))?;
        Ok(())
    })
}

#[post("/api/v1/files/bulk")]
async fn api_bulk_files(
    req: HttpRequest,
    body: web::Json<BulkRequest>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    net_config: web::Data<Arc<NetworkConfig>>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }

    let (account, client) =
        match api_account_client(&cache_dirs.account_root, tg_state.get_ref()).await {
            Ok(value) => value,
            Err(error) => return json_error("ACCOUNT_UNAVAILABLE", &error, 503),
        };
    let response = async {



    let ids = match parse_bulk_file_ids(&body.file_ids) {
        Ok(ids) => ids,
        Err(error) => return json_error("INVALID_FILE_IDS", &error, 400),
    };

    let source_folder: Option<i64> = body.folder_id.as_ref().and_then(|val| {
        if let Some(i) = val.as_i64() {
            Some(i)
        } else if let Some(s) = val.as_str() {
            s.parse::<i64>().ok()
        } else {
            None
        }
    });

    let target_folder: Option<i64> = body
        .payload
        .as_ref()
        .and_then(|p| p.folder_id.as_ref())
        .and_then(|val| {
            if let Some(i) = val.as_i64() {
                Some(i)
            } else if let Some(s) = val.as_str() {
                s.parse::<i64>().ok()
            } else {
                None
            }
        });

    if body.action != "delete" {
        for message_id in &ids {
            match api_registered_encrypted(&account, &client, tg_state.get_ref(), source_folder, *message_id).await {
                Ok(false) => {},
                Ok(true) => return json_error("ENCRYPTED_BULK_ACTION_UNAVAILABLE", "This bulk action is disabled for encrypted files until registry-safe handling is available", 409),
                Err(error) => return json_error("ENCRYPTION_STATE_UNKNOWN", &error, 503),
            }
        }
    }

    match body.action.as_str() {
        "delete" => {
            let peer = match resolve_peer(&client, source_folder, &tg_state.peer_cache).await {
                Ok(p) => p,
                Err(e) => return json_error("PEER_ERROR", &e, 400),
            };
            if let Some(error) = api_scope_error(&account) { return error; }
            if let Err(e) = client.delete_messages(&peer, &ids).await {
                return json_error("DELETE_FAILED", &e.to_string(), 500);
            }
            let changes = ids.iter().map(|message| crate::workspace::remote_changes::Change::Delete { folder: source_folder, message: *message }).collect();
            if let Err(error) = crate::workspace::remote_changes::record(&account, changes).await {
                return json_error("LOCAL_UPDATE_FAILED", &error, 500);
            }

            // Clean up stale thumbnail and preview caches for deleted messages.
            let source_folder_key = source_folder
                .map(|id| id.to_string())
                .unwrap_or_else(|| "home".to_string());
            spawn_cache_cleanup(
                account.clone(),
                cache_dirs.thumbnail_dir.clone(),
                cache_dirs.preview_dir.clone(),
                ids.clone(),
                source_folder_key,
            );
        }
        "move" => {
            let source_peer = match resolve_peer(&client, source_folder, &tg_state.peer_cache).await
            {
                Ok(p) => p,
                Err(e) => return json_error("PEER_ERROR", &e, 400),
            };
            let target_peer = match resolve_peer(&client, target_folder, &tg_state.peer_cache).await
            {
                Ok(p) => p,
                Err(e) => return json_error("PEER_ERROR", &e, 400),
            };
            if source_folder != target_folder {
                if let Some(error) = api_scope_error(&account) { return error; }
            let forwarded = match client
                    .forward_messages(&target_peer, &ids, &source_peer)
                    .await
                {
                    Ok(messages) => messages,
                    Err(e) => {
                        return json_error(
                            "MOVE_FORWARD_FAILED",
                            &format!("Forward failed: {}", e),
                            500,
                        )
                    }
                };
                if let Err(error) = verify_forwarded_messages(&ids, &forwarded) {
                    return json_error("MOVE_COPY_INCOMPLETE", &error, 502);
                }
                if let Some(error) = api_scope_error(&account) { return error; }
            if let Err(e) = client.delete_messages(&source_peer, &ids).await {
                    return json_error(
                        "MOVE_DELETE_FAILED",
                        &format!("Delete original failed: {}", e),
                        500,
                    );
                }

                // Clean up stale thumbnail and preview caches for the old message IDs.
                let changes = ids.iter().zip(forwarded.iter().flatten()).map(|(message, forwarded)| crate::workspace::remote_changes::Change::Move {
                    source: source_folder, message: *message, target: target_folder, new_message: forwarded.id(),
                }).collect();
                if let Err(error) = crate::workspace::remote_changes::record(&account, changes).await {
                    return json_error("LOCAL_UPDATE_FAILED", &error, 500);
                }
                // After a move (forward+delete), messages get new IDs in the target folder,
                // so any cached thumbnails/previews under the old IDs are orphaned.
                let source_folder_key = source_folder
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "home".to_string());
                spawn_cache_cleanup(
                    account.clone(),
                    cache_dirs.thumbnail_dir.clone(),
                    cache_dirs.preview_dir.clone(),
                    ids.clone(),
                    source_folder_key,
                );
            }
        }
        "archive" => {
            let peer = match resolve_peer(&client, source_folder, &tg_state.peer_cache).await {
                Ok(p) => p,
                Err(e) => return json_error("PEER_ERROR", &e, 400),
            };

            // Check the whole selection before downloading any of it.
            let max_bytes = net_config.archive_max_bytes();
            let mut total_bytes: u64 = 0;
            let mut names = HashSet::new();
            let mut selected = Vec::new();
            for mid in &ids {
                let messages = match client.get_messages_by_id(&peer, &[*mid]).await {
                    Ok(m) => m,
                    Err(error) => {
                        return json_error(
                            "ARCHIVE_FETCH_FAILED",
                            &format!("Could not fetch selected file {mid}: {error}"),
                            502,
                        )
                    }
                };
                let Some(message) = messages.into_iter().flatten().next() else {
                    return json_error(
                        "ARCHIVE_FILE_MISSING",
                        &format!("Selected file {mid} no longer exists"),
                        404,
                    );
                };
                let Some(media) = message.media() else {
                    return json_error(
                        "ARCHIVE_MEDIA_MISSING",
                        &format!("Selected file {mid} has no downloadable media"),
                        409,
                    );
                };
                let filename = match &media {
                    Media::Document(document) => sanitise_upload_filename(document.name()),
                    Media::Photo(_) => format!("photo_{mid}.jpg"),
                    _ => {
                        return json_error(
                            "ARCHIVE_MEDIA_UNSUPPORTED",
                            &format!("Selected file {mid} cannot be archived"),
                            409,
                        )
                    }
                };
                let expected_size = media_size(&media);
                total_bytes = match total_bytes.checked_add(expected_size) {
                    Some(total) if max_bytes == 0 || total <= max_bytes => total,
                    _ => {
                        return json_error(
                            "ARCHIVE_TOO_LARGE",
                            &format!("Archive exceeds the {} MiB limit", max_bytes / (1024 * 1024)),
                            413,
                        )
                    }
                };
                selected.push((*mid, media, archive_entry_name(filename, &mut names), expected_size));
            }

            let staging = match crate::temp_artifacts::staging_root() {
                Ok(staging) => staging,
                Err(error) => {
                    return json_error("TEMP_FILE_CREATE_FAILED", &error.to_string(), 500)
                }
            };
            let temp_zip_path = staging.join(format!(
                "archive_{}_{}.zip",
                rand::random::<u64>(),
                rand::random::<u64>()
            ));

            let mut reservation=match BandwidthReservation::download(req.app_data::<web::Data<Arc<BandwidthManager>>>().expect("Shared bandwidth accounting").get_ref().clone(),total_bytes) {
                Ok(hold)=>hold,Err(error)=>return json_error("BANDWIDTH_LIMIT",&error,429)
            };

            // Each file goes from Telegram to the ZIP on disk in chunks; the
            // selection is never held in memory.
            let (parts, receiver) = tokio::sync::mpsc::channel(8);
            let writer = spawn_archive_writer(temp_zip_path.clone(), receiver);
            let mut failure = None;
            for (mid, media, filename, expected_size) in &selected {
                let entry = ArchivePart::File {
                    name: filename.clone(),
                    large: *expected_size >= u64::from(u32::MAX),
                };
                if parts.send(entry).await.is_err() {
                    break;
                }
                let mut download_iter = client.iter_download(media);
                let chunk_account = account.clone();
                let chunk_network=net_config.clone();
                let chunks = async_stream::try_stream! {
                    while let Some(chunk) = download_iter.next().await.map_err(|error| error.to_string())? {
                        chunk_account.validate()?;
                        chunk_network.pacer.wait(&chunk_network,crate::traffic::Direction::Download,chunk.len(),||chunk_account.validate()).await?;
                        yield chunk;
                    }
                };
                match stream_archive_file(chunks, *expected_size, &parts).await {
                    Ok(()) => {}
                    Err(ArchiveDownloadError::Writer) => break,
                    Err(ArchiveDownloadError::Remote(error)) => {
                        failure = Some(json_error(
                            "ARCHIVE_DOWNLOAD_FAILED",
                            &format!("Could not finish selected file {mid}: {error}"),
                            502,
                        ));
                        break;
                    }
                    Err(ArchiveDownloadError::Incomplete { expected, actual }) => {
                        failure = Some(json_error(
                            "ARCHIVE_DOWNLOAD_INCOMPLETE",
                            &format!("Selected file {mid} has {actual} downloaded bytes; expected {expected}"),
                            502,
                        ));
                        break;
                    }
                }
            }
            drop(parts);
            let written = writer.await;
            let failure = failure.or_else(|| match written {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(json_error("ARCHIVE_FAILED", &e, 500)),
                Err(e) => Some(json_error("ARCHIVE_PANIC", &e.to_string(), 500)),
            });
            if let Some(failure) = failure {
                let _ = tokio::fs::remove_file(&temp_zip_path).await;
                return failure;
            }

            let file = match tokio::fs::File::open(&temp_zip_path).await {
                Ok(f) => f,
                Err(e) => return json_error("OPEN_ZIP_FAILED", &e.to_string(), 500),
            };

            if let Some(error)=api_scope_error(&account){return error;}
            reservation.commit();
            let stream = CleanupStream {
                account: account.clone(),
                file,
                path: temp_zip_path,
            };

            return HttpResponse::Ok()
                .content_type("application/zip")
                .insert_header((
                    actix_web::http::header::CONTENT_DISPOSITION,
                    "attachment; filename=\"archive.zip\"",
                ))
                .streaming(stream);
        }
        _ => return json_error("INVALID_ACTION", "Unsupported bulk action", 400),
    }

    api_catalog::invalidate_cached(account.owner);
    HttpResponse::Ok().json(BulkResponse {
        success: true,
        count: ids.len(),
    })
    }.await;
    account_response(&account, response)
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: Option<String>,
    folder_id: Option<String>,
    refresh: Option<bool>,
}

#[get("/api/v1/files/search")]
async fn api_search_files(
    req: HttpRequest,
    query: web::Query<SearchQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }

    let search = match query.q.as_deref().map(str::trim) {
        Some(q) if !q.is_empty() => q.to_lowercase(),
        _ => {
            return json_error(
                "INVALID_QUERY",
                "Search query parameter 'q' is required and cannot be empty",
                400,
            )
        }
    };
    let scope = match folder_scope(query.folder_id.as_deref()) {
        Ok(scope) => scope,
        Err(error) => return *error,
    };

    let account = match api_account(&cache_dirs.account_root) {
        Ok(account) => account,
        Err(error) => return *error,
    };
    if query.refresh == Some(true) {
        api_catalog::invalidate(account.owner);
    }
    let (files, _) = match api_catalog::files(&account, tg_state.get_ref(), scope).await {
        Ok(listing) => listing,
        Err(error) => return catalog_error(error),
    };
    let matching: Vec<ApiFile> = files
        .into_iter()
        .filter(|file| file.name.to_lowercase().contains(&search))
        .collect();

    account_response(&account, HttpResponse::Ok().json(matching))
}

#[delete("/api/v1/files/{message_id}")]
async fn api_delete_file(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<FolderQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let message_id = match message_id(path.into_inner()) {
        Ok(id) => id,
        Err(error) => return *error,
    };
    let folder_id = query.folder_id;

    let (account, client) =
        match api_account_client(&cache_dirs.account_root, tg_state.get_ref()).await {
            Ok(value) => value,
            Err(error) => return json_error("ACCOUNT_UNAVAILABLE", &error, 503),
        };

    let peer = match resolve_peer(&client, folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => return json_error("PEER_ERROR", &e, 400),
    };

    if let Some(error) = api_scope_error(&account) {
        return error;
    }
    match client.delete_messages(&peer, &[message_id]).await {
        Ok(_) => {
            if let Err(error) = crate::workspace::remote_changes::record(
                &account,
                vec![crate::workspace::remote_changes::Change::Delete {
                    folder: folder_id,
                    message: message_id,
                }],
            )
            .await
            {
                return json_error("LOCAL_UPDATE_FAILED", &error, 500);
            }
            api_catalog::invalidate_cached(account.owner);
            account_response(
                &account,
                HttpResponse::Ok().json(serde_json::json!({ "success": true })),
            )
        }
        Err(e) => json_error("DELETE_FAILED", &e.to_string(), 500),
    }
}

#[derive(serde::Deserialize)]
struct CopyRequest {
    folder_id: Option<i64>,
    source_folder_id: Option<i64>,
}

#[post("/api/v1/files/{message_id}/copy")]
async fn api_copy_file(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<CopyRequest>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let message_id = match message_id(path.into_inner()) {
        Ok(id) => id,
        Err(error) => return *error,
    };

    let (account, client) =
        match api_account_client(&cache_dirs.account_root, tg_state.get_ref()).await {
            Ok(value) => value,
            Err(error) => return json_error("ACCOUNT_UNAVAILABLE", &error, 503),
        };
    let response = async {
        let source_folder_id = body.source_folder_id;
        let target_folder_id = body.folder_id;

        let source_peer = match resolve_peer(&client, source_folder_id, &tg_state.peer_cache).await
        {
            Ok(p) => p,
            Err(e) => return json_error("SOURCE_PEER_ERROR", &e, 400),
        };
        let target_peer = match resolve_peer(&client, target_folder_id, &tg_state.peer_cache).await
        {
            Ok(p) => p,
            Err(e) => return json_error("TARGET_PEER_ERROR", &e, 400),
        };

        // Resolve the registry state before changing Telegram. Treat an unavailable
        // registry as a hard failure so an encrypted copy can never silently lose
        // the metadata required to decrypt it.
        let source_is_encrypted = match api_registered_encrypted(
            &account,
            &client,
            tg_state.get_ref(),
            source_folder_id,
            message_id,
        )
        .await
        {
            Ok(value) => value,
            Err(e) => return json_error("ENCRYPTION_REGISTRY_UNAVAILABLE", &e, 503),
        };

        if let Some(error) = api_scope_error(&account) {
            return error;
        }
        match client
            .forward_messages(&target_peer, &[message_id], &source_peer)
            .await
        {
            Ok(forwarded) => {
                if let Err(error) = crate::file_inventory::changed(&account, target_folder_id, &[])
                {
                    return json_error("ACCOUNT_CHANGED", &error, 409);
                }
                api_catalog::invalidate_cached(account.owner);
                if source_is_encrypted {
                    let new_id = forwarded
                        .first()
                        .and_then(|message| message.as_ref())
                        .map(|message| message.id());
                    let Some(new_id) = new_id else {
                        return json_error(
                            "ENCRYPTED_COPY_RECONCILIATION_REQUIRED",
                            "Telegram copied the file but did not return its new identifier",
                            500,
                        );
                    };

                    if !matches!(
                        api_registered_encrypted(
                            &account,
                            &client,
                            tg_state.get_ref(),
                            target_folder_id,
                            new_id
                        )
                        .await,
                        Ok(true)
                    ) {
                        return json_error(
                            "ENCRYPTED_COPY_RECONCILIATION_REQUIRED",
                            "Remote copy succeeded but local encryption indexing failed",
                            500,
                        );
                    }
                }
                HttpResponse::Ok().json(serde_json::json!({ "success": true }))
            }
            Err(e) => json_error("COPY_FAILED", &e.to_string(), 500),
        }
    }
    .await;
    account_response(&account, response)
}

#[derive(serde::Deserialize)]
struct UpdateFileRequest {
    name: Option<String>,
    folder_id: Option<i64>,
    source_folder_id: Option<i64>,
}

#[patch("/api/v1/files/{message_id}")]
async fn api_update_file(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateFileRequest>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let message_id = match message_id(path.into_inner()) {
        Ok(id) => id,
        Err(error) => return *error,
    };

    let (account, client) =
        match api_account_client(&cache_dirs.account_root, tg_state.get_ref()).await {
            Ok(value) => value,
            Err(error) => return json_error("ACCOUNT_UNAVAILABLE", &error, 503),
        };
    let response = async {
    // Reconcile the two mutation scopes even if a remote reply is uncertain.
    if let Err(error) = crate::file_inventory::changed(&account, body.source_folder_id, &[message_id]) {
        return json_error("ACCOUNT_CHANGED", &error, 409);
    }
    if let Some(target) = body.folder_id {
        if let Err(error) = crate::file_inventory::changed(&account, Some(target), &[]) {
            return json_error("ACCOUNT_CHANGED", &error, 409);
        }
    }
    api_catalog::invalidate_cached(account.owner);
    match api_registered_encrypted(&account, &client, tg_state.get_ref(), body.source_folder_id, message_id).await {
        Ok(true) => return json_error(
            "ENCRYPTED_UPDATE_UNAVAILABLE",
            "Encrypted rename/move through the local API is disabled until authenticated metadata and registry updates are supported",
            409,
        ),
        Ok(false) => {}
        Err(error) => return json_error("ENCRYPTION_STATE_UNKNOWN", &error, 503),
    }



    // Rename first — edits the original message's caption so the
    // updated name is carried over if a move (forward) follows.
    if let Some(ref new_name) = body.name {
        let rename_peer =
            match resolve_peer(&client, body.source_folder_id, &tg_state.peer_cache).await {
                Ok(p) => p,
                Err(e) => return json_error("PEER_ERROR", &e, 400),
            };

        // Verify the message exists before attempting to edit it.
        // This avoids a cryptic MESSAGE_ID_INVALID RPC error when the message
        // was moved or deleted since the file list was loaded.
        let messages = match client.get_messages_by_id(&rename_peer, &[message_id]).await {
            Ok(msgs) => msgs,
            Err(e) => {
                return json_error(
                    "FETCH_ERROR",
                    &format!("Failed to fetch message for rename: {}", e),
                    500,
                )
            }
        };
        if messages.iter().flatten().next().is_none() {
            return json_error(
                "MESSAGE_NOT_FOUND",
                &format!(
                    "Message {} not found in folder {:?}. The file may have been moved or deleted. Please refresh.",
                    message_id, body.source_folder_id
                ),
                404,
            );
        }

        let input_peer = match peer_to_input_peer(&rename_peer) {
            Ok(ip) => ip,
            Err(e) => return json_error("PEER_CONVERT_ERROR", &e, 400),
        };

        if let Some(error) = api_scope_error(&account) { return error; }
        if let Err(e) = client
            .invoke(&tl::functions::messages::EditMessage {
                peer: input_peer,
                id: message_id,
                no_webpage: false,
                invert_media: false,
                message: Some(new_name.clone()),
                media: None,
                reply_markup: None,
                entities: None,
                schedule_date: None,
                quick_reply_shortcut_id: None,
                schedule_repeat_period: None,
            })
            .await
        {
            return json_error("RENAME_FAILED", &e.to_string(), 500);
        }
        if let Err(error) = crate::workspace::remote_changes::record(&account, vec![crate::workspace::remote_changes::Change::Rename {
            folder: body.source_folder_id, message: message_id, name: new_name.clone(),
        }]).await {
            return json_error("LOCAL_UPDATE_FAILED", &error, 500);
        }
    }

    if let Some(target_folder_id) = body.folder_id {
        let source_folder_id = body.source_folder_id;
        if source_folder_id != body.folder_id {
            let source_peer =
                match resolve_peer(&client, source_folder_id, &tg_state.peer_cache).await {
                    Ok(p) => p,
                    Err(e) => return json_error("SOURCE_PEER_ERROR", &e, 400),
                };
            let target_peer =
                match resolve_peer(&client, Some(target_folder_id), &tg_state.peer_cache).await {
                    Ok(p) => p,
                    Err(e) => return json_error("TARGET_PEER_ERROR", &e, 400),
                };

            if let Some(error) = api_scope_error(&account) { return error; }
            let forwarded = match client
                .forward_messages(&target_peer, &[message_id], &source_peer)
                .await
            {
                Ok(messages) => messages,
                Err(e) => return json_error("MOVE_FORWARD_FAILED", &e.to_string(), 500),
            };
            if let Err(error) = verify_forwarded_messages(&[message_id], &forwarded) {
                return json_error("MOVE_COPY_INCOMPLETE", &error, 502);
            }
            if let Some(error) = api_scope_error(&account) { return error; }
            if let Err(e) = client.delete_messages(&source_peer, &[message_id]).await {
                return json_error("MOVE_DELETE_FAILED", &e.to_string(), 500);
            }

            // Clean up stale thumbnail and preview caches for the old message ID
            let changes = forwarded.iter().flatten().map(|forwarded| crate::workspace::remote_changes::Change::Move {
                source: source_folder_id, message: message_id, target: Some(target_folder_id), new_message: forwarded.id(),
            }).collect();
            if let Err(error) = crate::workspace::remote_changes::record(&account, changes).await {
                return json_error("LOCAL_UPDATE_FAILED", &error, 500);
            }
            let source_folder_key = source_folder_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "home".to_string());
            spawn_cache_cleanup(
                account.clone(),
                cache_dirs.thumbnail_dir.clone(),
                cache_dirs.preview_dir.clone(),
                vec![message_id],
                source_folder_key,
            );
        }
    }

    HttpResponse::Ok().json(serde_json::json!({ "success": true }))
    }.await;
    account_response(&account, response)
}

#[post("/api/v1/files")]
async fn api_upload_file(
    req: HttpRequest,
    mut payload: Multipart,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    bw_manager: web::Data<Arc<BandwidthManager>>,
    net_config: web::Data<Arc<NetworkConfig>>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }

    let (account, client) = match api_session(&cache_dirs.account_root, tg_state.get_ref()).await {
        Ok(session) => session,
        Err(error) => return error,
    };

    let staging = match crate::temp_artifacts::staging_root() {
        Ok(staging) => staging,
        Err(error) => return json_error("TEMP_FILE_CREATE_FAILED", &error.to_string(), 500),
    };
    let temp_path = staging.join(format!(
        "upload_{}_{}",
        rand::random::<u64>(),
        rand::random::<u64>()
    ));
    // `create_new` refuses to reuse an existing name instead of truncating it.
    let mut file = match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .await
    {
        Ok(f) => f,
        Err(e) => return json_error("TEMP_FILE_CREATE_FAILED", &e.to_string(), 500),
    };

    let mut folder_id: Option<i64> = None;
    let mut filename = "file".to_string();
    let mut field_mime: Option<String> = None;
    let mut file_seen = false;
    let mut folder_seen = false;
    let mut file_size = 0_u64;

    loop {
        let mut field = match payload.try_next().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return json_error("INVALID_MULTIPART", &error.to_string(), 400);
            }
        };
        let content_disposition = field.content_disposition();
        let name = content_disposition
            .and_then(|cd| cd.get_name())
            .unwrap_or("");

        if name == "file" {
            if file_seen {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return json_error("MULTIPLE_FILES", "Upload exactly one file per request", 400);
            }
            file_seen = true;
            if let Some(fname) = content_disposition.and_then(|cd| cd.get_filename()) {
                filename = sanitise_upload_filename(fname);
            }
            field_mime = field.content_type().map(|m| m.to_string());
            while let Some(chunk) = field.next().await {
                let data = match chunk {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        return json_error("READ_ERROR", &e.to_string(), 400);
                    }
                };
                file_size = match file_size.checked_add(data.len() as u64) {
                    Some(total) if total <= TELEGRAM_MAX_FILE_SIZE => total,
                    _ => {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        return json_error(
                            "FILE_TOO_LARGE",
                            "File exceeds Telegram's upload size limit",
                            413,
                        );
                    }
                };
                if let Err(e) = file.write_all(&data).await {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    return json_error("WRITE_ERROR", &e.to_string(), 500);
                }
            }
        } else if name == "folder_id" {
            if folder_seen {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return json_error("DUPLICATE_FOLDER_ID", "folder_id may be provided once", 400);
            }
            folder_seen = true;
            let mut bytes = Vec::new();
            while let Some(chunk) = field.next().await {
                let data = match chunk {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        return json_error("READ_ERROR", &e.to_string(), 400);
                    }
                };
                if bytes.len().saturating_add(data.len()) > MAX_MULTIPART_METADATA_BYTES {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    return json_error("INVALID_FOLDER_ID", "folder_id is too long", 400);
                }
                bytes.extend_from_slice(&data);
            }
            let val_str = String::from_utf8_lossy(&bytes).trim().to_string();
            if !val_str.is_empty() && val_str != "null" && val_str != "none" {
                folder_id = match val_str.parse::<i64>() {
                    Ok(id) => Some(id),
                    Err(_) => {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        return json_error(
                            "INVALID_FOLDER_ID",
                            "folder_id must be an integer",
                            400,
                        );
                    }
                };
            }
        } else {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return json_error(
                "UNKNOWN_MULTIPART_FIELD",
                "Only file and folder_id fields are accepted",
                400,
            );
        }
    }

    if !file_seen {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return json_error("FILE_REQUIRED", "A multipart file field is required", 400);
    }

    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return json_error("WRITE_ERROR", &e.to_string(), 500);
    }
    drop(file);

    let persisted_size = match tokio::fs::metadata(&temp_path).await {
        Ok(m) => m.len(),
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return json_error("METADATA_ERROR", &e.to_string(), 500);
        }
    };
    if persisted_size != file_size {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return json_error("SIZE_MISMATCH", "Upload staging size did not match", 500);
    }

    let mut reservation =
        match BandwidthReservation::upload(bw_manager.get_ref().clone(), file_size) {
            Ok(reservation) => reservation,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return json_error("BANDWIDTH_LIMIT", &error, 400);
            }
        };

    let peer = match resolve_peer(&client, folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return json_error("PEER_ERROR", &e, 400);
        }
    };

    let open_file = match tokio::fs::File::open(&temp_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return json_error("OPEN_ERROR", &e.to_string(), 500);
        }
    };

    // The request body took time to arrive; the account may have changed.
    if let Some(error) = api_scope_error(&account) {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return error;
    }
    let mut open_file = crate::traffic::Reader::new(
        open_file,
        crate::traffic::Traffic {
            network: net_config.get_ref().clone(),
            account: account.clone(),
            direction: crate::traffic::Direction::Upload,
        },
    );
    let upload_res = client
        .upload_stream(&mut open_file, file_size as usize, filename.clone())
        .await;
    let uploaded_file = match upload_res {
        Ok(uf) => uf,
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return json_error("UPLOAD_FAILED", &map_error(e), 500);
        }
    };

    let message = InputMessage::new().text("").file(uploaded_file);

    let max_retries = net_config.retry_attempts();
    let base_ms = net_config.retry_base_backoff_ms();
    let max_ms = net_config.retry_max_backoff_ms();
    let respect_flood = net_config.should_respect_flood_wait();
    let mut last_err = String::new();
    let mut sent_msg = None;

    for attempt in 0..=max_retries {
        // Never publish into a folder after the account that asked is gone.
        if let Some(error) = api_scope_error(&account) {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return error;
        }
        match client.send_message(&peer, message.clone()).await {
            Ok(msg) => {
                if let Err(error) = crate::file_inventory::changed(&account, folder_id, &[]) {
                    return json_error("ACCOUNT_CHANGED", &error, 409);
                }
                sent_msg = Some(msg);
                break;
            }
            Err(e) => {
                let err = map_error(e);
                log::warn!(
                    "send_message attempt {}/{}: {}",
                    attempt + 1,
                    max_retries + 1,
                    err
                );

                if respect_flood && err.starts_with("FLOOD_WAIT_") {
                    if let Ok(secs) = err.trim_start_matches("FLOOD_WAIT_").parse::<u64>() {
                        let wait = secs.min(300);
                        log::info!("Respecting FLOOD_WAIT: sleeping {}s", wait);
                        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                        last_err = err;
                        continue;
                    }
                }

                if attempt < max_retries {
                    let wait = crate::vpn_optimizer::backoff_ms(attempt, base_ms, max_ms);
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }
                last_err = err;
            }
        }
    }

    let _ = tokio::fs::remove_file(&temp_path).await;

    let msg = match sent_msg {
        Some(m) => m,
        None => {
            return json_error("SEND_MESSAGE_FAILED", &last_err, 500);
        }
    };

    let response_file = ApiFile {
        id: i64::from(msg.id()),
        folder_id,
        document_name: filename.clone(),
        name: filename,
        size: file_size,
        mime_type: field_mime,
        created_at: api_catalog::rfc3339(msg.date()),
        encrypted: false,
        timestamp: msg.date().timestamp(),
        is_document: true,
    };
    reservation.commit();
    api_catalog::invalidate_cached(account.owner);

    HttpResponse::Ok().json(response_file)
}

#[get("/api/v1/folders")]
async fn api_list_folders(
    req: HttpRequest,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }

    let (account, client) = match api_session(&cache_dirs.account_root, tg_state.get_ref()).await {
        Ok(session) => session,
        Err(error) => return error,
    };
    match api_catalog::discover_folders(&account, &client, tg_state.get_ref()).await {
        Ok(folders) => account_response(&account, HttpResponse::Ok().json(folders)),
        Err(error) => catalog_error(error),
    }
}

#[derive(serde::Deserialize)]
struct CreateFolderRequest {
    name: String,
}

#[post("/api/v1/folders")]
async fn api_create_folder(
    req: HttpRequest,
    body: web::Json<CreateFolderRequest>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }

    let (account, client) = match api_session(&cache_dirs.account_root, tg_state.get_ref()).await {
        Ok(session) => session,
        Err(error) => return error,
    };
    if let Err(error) = crate::file_inventory::changed(&account, Some(i64::MIN), &[]) {
        return json_error("ACCOUNT_CHANGED", &error, 409);
    }
    api_catalog::invalidate_cached(account.owner);
    let response = match create_folder_inner(&body.name, &client, &tg_state.peer_cache).await {
        Ok(folder) => HttpResponse::Ok().json(folder),
        Err(e) => json_error("CREATE_FOLDER_FAILED", &e, 500),
    };
    account_response(&account, response)
}

#[derive(serde::Deserialize)]
struct RenameFolderRequest {
    name: String,
}

#[patch("/api/v1/folders/{folder_id}")]
async fn api_rename_folder(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<RenameFolderRequest>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let folder_id = path.into_inner();

    let (account, client) = match api_session(&cache_dirs.account_root, tg_state.get_ref()).await {
        Ok(session) => session,
        Err(error) => return error,
    };
    if let Err(error) = crate::file_inventory::changed(&account, Some(i64::MIN), &[]) {
        return json_error("ACCOUNT_CHANGED", &error, 409);
    }
    api_catalog::invalidate_cached(account.owner);
    let response =
        match rename_folder_inner(folder_id, &body.name, &client, &tg_state.peer_cache).await {
            Ok(_) => HttpResponse::Ok().json(serde_json::json!({ "success": true })),
            Err(e) => json_error("RENAME_FOLDER_FAILED", &e, 500),
        };
    account_response(&account, response)
}

#[delete("/api/v1/folders/{folder_id}")]
async fn api_delete_folder(
    req: HttpRequest,
    path: web::Path<i64>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let folder_id = path.into_inner();

    let (account, client) = match api_session(&cache_dirs.account_root, tg_state.get_ref()).await {
        Ok(session) => session,
        Err(error) => return error,
    };
    if let Err(error) = crate::file_inventory::changed(&account, Some(i64::MIN), &[]) {
        return json_error("ACCOUNT_CHANGED", &error, 409);
    }
    api_catalog::invalidate_cached(account.owner);
    // Checked again immediately before the folder is removed.
    if let Some(error) = api_scope_error(&account) {
        return error;
    }
    let response = match delete_folder_inner(folder_id, &client, &tg_state.peer_cache).await {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({ "success": true })),
        Err(e) => json_error("DELETE_FOLDER_FAILED", &e, 500),
    };
    account_response(&account, response)
}

#[derive(Serialize)]
struct FolderStat {
    id: Option<i64>,
    name: String,
    file_count: usize,
    size_bytes: u64,
}

#[derive(Serialize)]
struct MimeStat {
    mime_type: String,
    file_count: usize,
    size_bytes: u64,
}

#[derive(Serialize)]
struct StorageStatsResponse {
    total_storage_used_bytes: u64,
    total_file_count: usize,
    /// False when a folder holds more files than one listing covers.
    complete: bool,
    folders: Vec<FolderStat>,
    mime_types: Vec<MimeStat>,
}

/// Every folder with its complete listing, for the account-wide reports.
async fn report_listings(
    cache_dirs: &CacheDirs,
    state: &TelegramState,
    refresh: Option<bool>,
) -> Result<
    (
        AccountGuard,
        Vec<(api_catalog::Folder, api_catalog::Listing)>,
    ),
    HttpResponse,
> {
    let account = api_account(&cache_dirs.account_root).map_err(|response| *response)?;
    if refresh == Some(true) {
        api_catalog::invalidate(account.owner);
    }
    let folders = api_catalog::folders(&account, state)
        .await
        .map_err(catalog_error)?;
    let mut listings = Vec::with_capacity(folders.len());
    for folder in folders.iter() {
        let listing = api_catalog::listing(&account, state, folder.id)
            .await
            .map_err(catalog_error)?;
        listings.push((folder.clone(), listing));
    }
    Ok((account, listings))
}

#[get("/api/v1/storage/stats")]
async fn api_storage_stats(
    req: HttpRequest,
    query: web::Query<RefreshQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let (account, listings) =
        match report_listings(&cache_dirs, tg_state.get_ref(), query.refresh).await {
            Ok(listings) => listings,
            Err(error) => return error,
        };

    let mut total_storage_used_bytes: u64 = 0;
    let mut total_file_count: usize = 0;
    let mut complete = true;
    let mut folder_stats = Vec::new();
    let mut mime_map: HashMap<String, (usize, u64)> = HashMap::new();

    for (folder, listing) in listings {
        let mut file_count = 0;
        let mut size_bytes: u64 = 0;
        for file in listing.files.iter().filter(|file| file.is_document) {
            file_count += 1;
            size_bytes = size_bytes.saturating_add(file.size);
            let mime = file
                .mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let entry = mime_map.entry(mime).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.saturating_add(file.size);
        }
        complete &= listing.complete;
        total_storage_used_bytes = total_storage_used_bytes.saturating_add(size_bytes);
        total_file_count += file_count;
        folder_stats.push(FolderStat {
            id: folder.id,
            name: folder.name,
            file_count,
            size_bytes,
        });
    }

    let mut mime_types: Vec<MimeStat> = mime_map
        .into_iter()
        .map(|(mime_type, (file_count, size_bytes))| MimeStat {
            mime_type,
            file_count,
            size_bytes,
        })
        .collect();
    mime_types.sort_by(|a, b| a.mime_type.cmp(&b.mime_type));

    account_response(
        &account,
        HttpResponse::Ok().json(StorageStatsResponse {
            total_storage_used_bytes,
            total_file_count,
            complete,
            folders: folder_stats,
            mime_types,
        }),
    )
}

#[derive(Serialize)]
struct DuplicateGroup {
    name: String,
    size: u64,
    files: Vec<ApiFile>,
}

#[get("/api/v1/storage/duplicates")]
async fn api_storage_duplicates(
    req: HttpRequest,
    query: web::Query<RefreshQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let (account, listings) =
        match report_listings(&cache_dirs, tg_state.get_ref(), query.refresh).await {
            Ok(listings) => listings,
            Err(error) => return error,
        };

    // Same uploaded name and same size. A rename does not hide a duplicate.
    let mut file_groups: HashMap<(String, u64), Vec<ApiFile>> = HashMap::new();
    for (_, listing) in &listings {
        for file in listing.files.iter().filter(|file| file.is_document) {
            file_groups
                .entry((file.document_name.clone(), file.size))
                .or_default()
                .push(file.clone());
        }
    }

    let mut duplicates: Vec<DuplicateGroup> = file_groups
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|((name, size), files)| DuplicateGroup { name, size, files })
        .collect();
    duplicates.sort_by(|a, b| (&a.name, a.size).cmp(&(&b.name, b.size)));

    account_response(&account, HttpResponse::Ok().json(duplicates))
}

#[get("/api/v1/folders/empty")]
async fn api_empty_folders(
    req: HttpRequest,
    query: web::Query<RefreshQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let (account, listings) =
        match report_listings(&cache_dirs, tg_state.get_ref(), query.refresh).await {
            Ok(listings) => listings,
            Err(error) => return error,
        };

    // Empty means no file anywhere in the folder, not only at its newest
    // message. Saved Messages is not a folder that can be removed.
    let empty_folders: Vec<FolderMetadata> = listings
        .into_iter()
        .filter(|(_, listing)| listing.files.is_empty())
        .filter_map(|(folder, _)| {
            folder.id.map(|id| FolderMetadata {
                id,
                name: folder.name,
                parent_id: None,
                username: None,
                is_public: false,
                group_id: None,
                display_order: 0,
            })
        })
        .collect();

    account_response(&account, HttpResponse::Ok().json(empty_folders))
}

#[get("/api/v1/files/{message_id}/thumbnail")]
async fn api_get_file_thumbnail(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<FolderQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
    transport: (
        web::Data<Arc<BandwidthManager>>,
        web::Data<Arc<NetworkConfig>>,
    ),
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let message = match message_id(path.into_inner()) {
        Ok(id) => id,
        Err(e) => return *e,
    };
    let account = match api_account(&cache_dirs.account_root) {
        Ok(a) => a,
        Err(e) => return *e,
    };
    let cache = match cache_dirs.preview_dir.parent() {
        Some(p) => p.to_path_buf(),
        None => return json_error("STORAGE_UNAVAILABLE", "Preview storage unavailable", 503),
    };
    let response = match crate::workspace::assets::remote_thumbnail_at(
        cache,
        account.clone(),
        tg_state.get_ref().clone(),
        (transport.0.get_ref().clone(), transport.1.get_ref().clone()),
        query.folder_id,
        message,
    )
    .await
    {
        Ok(path) => match tokio::fs::read(path).await {
            Ok(bytes) => HttpResponse::Ok().content_type("image/jpeg").body(bytes),
            Err(e) => json_error("STORAGE_UNAVAILABLE", &e.to_string(), 503),
        },
        Err(e) if e.starts_with("ENCRYPTED") => {
            json_error("ENCRYPTED_PREVIEW_UNAVAILABLE", &e, 403)
        }
        Err(e) if e.starts_with("FILE_NOT_FOUND") || e.starts_with("THUMBNAIL_UNAVAILABLE") => {
            json_error("NOT_FOUND", &e, 404)
        }
        Err(e) => json_error("THUMBNAIL_UNAVAILABLE", &e, 503),
    };
    account_response(&account, response)
}

#[derive(Serialize)]
struct MediaInfoResponse {
    duration_secs: Option<f64>,
    width: Option<i32>,
    height: Option<i32>,
    audio_title: Option<String>,
    audio_performer: Option<String>,
}

#[get("/api/v1/files/{message_id}/media-info")]
async fn api_media_info(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<FolderQuery>,
    tg_state: web::Data<Arc<TelegramState>>,
    api_state: web::Data<ApiState>,
    cache_dirs: web::Data<CacheDirs>,
) -> impl Responder {
    if let Err(e) = check_auth(&req, &api_state) {
        return *e;
    }
    let message_id = match message_id(path.into_inner()) {
        Ok(id) => id,
        Err(error) => return *error,
    };

    let (account, client) =
        match api_account_client(&cache_dirs.account_root, tg_state.get_ref()).await {
            Ok(value) => value,
            Err(error) => return json_error("ACCOUNT_UNAVAILABLE", &error, 503),
        };
    let response = async {
        let folder_id = query.folder_id;

        let peer = match resolve_peer(&client, folder_id, &tg_state.peer_cache).await {
            Ok(p) => p,
            Err(e) => return json_error("PEER_ERROR", &e, 400),
        };

        let messages = match client.get_messages_by_id(&peer, &[message_id]).await {
            Ok(msgs) => msgs,
            Err(e) => return json_error("GET_MESSAGE_ERROR", &e.to_string(), 500),
        };

        let msg = match messages.into_iter().flatten().next() {
            Some(m) => m,
            None => return json_error("NOT_FOUND", "File message not found", 404),
        };

        let media = match msg.media() {
            Some(m) => m,
            None => return json_error("NO_MEDIA", "Message has no media", 400),
        };
        if let Some(error) = api_protected_response(
            &account,
            &client,
            folder_id,
            message_id,
            &media,
            msg.text(),
            "Encrypted media metadata is not exposed by the local API",
        )
        .await
        {
            return error;
        }

        let mut info = MediaInfoResponse {
            duration_secs: None,
            width: None,
            height: None,
            audio_title: None,
            audio_performer: None,
        };

        if let Media::Document(d) = media {
            if let Some(tl::enums::Document::Document(doc)) = &d.raw.document {
                for attr in &doc.attributes {
                    match attr {
                        tl::enums::DocumentAttribute::Video(v) => {
                            info.duration_secs = Some(v.duration);
                            info.width = Some(v.w);
                            info.height = Some(v.h);
                        }
                        tl::enums::DocumentAttribute::Audio(a) => {
                            info.duration_secs = Some(a.duration as f64);
                            info.audio_title = a.title.clone();
                            info.audio_performer = a.performer.clone();
                        }
                        _ => {}
                    }
                }
            }
        }

        HttpResponse::Ok().json(info)
    }
    .await;
    account_response(&account, response)
}

/// Everything the REST API server shares with the application.
pub struct ApiServerParts {
    pub telegram: Arc<TelegramState>,
    pub key_hash: Option<String>,
    pub cache_dirs: CacheDirs,
    pub bandwidth: Arc<BandwidthManager>,
    pub network: Arc<NetworkConfig>,
    pub database: crate::db::DbConnection,
}

/// Serve the REST API on an already bound loopback listener.
pub fn serve(
    listener: std::net::TcpListener,
    parts: ApiServerParts,
) -> std::io::Result<actix_web::dev::Server> {
    let telegram = web::Data::new(parts.telegram);
    let api_state = web::Data::new(ApiState {
        key_hash: parts.key_hash,
    });
    let cache_dirs = web::Data::new(parts.cache_dirs);
    let bandwidth = web::Data::new(parts.bandwidth);
    let network = web::Data::new(parts.network);
    let database = web::Data::new(parts.database);
    Ok(actix_web::HttpServer::new(move || {
        let cors = actix_cors::Cors::default()
            .allowed_origin_fn(|origin, _req_head| {
                crate::local_cors::is_allowed_origin_header(origin)
            })
            .allow_any_method()
            .allow_any_header();
        actix_web::App::new()
            // The key is checked before a request body, query or path is
            // parsed, so an unauthenticated caller learns nothing from how
            // its input was rejected. Handlers check it again.
            .wrap_fn(|request, service| {
                use actix_web::dev::Service;
                let public = matches!(request.path(), "/api/v1/health" | "/api/v1/openapi.json");
                let refusal = if public {
                    None
                } else {
                    match request.app_data::<web::Data<ApiState>>() {
                        Some(state) => check_auth(request.request(), state)
                            .err()
                            .map(|response| *response),
                        None => Some(json_error("UNAUTHORIZED", "Invalid API key", 401)),
                    }
                };
                match refusal {
                    Some(response) => futures::future::Either::Left(std::future::ready(Ok(
                        request.into_response(response),
                    ))),
                    None => {
                        let response = service.call(request);
                        futures::future::Either::Right(async move {
                            response
                                .await
                                .map(|response| response.map_into_boxed_body())
                        })
                    }
                }
            })
            // Preflight requests carry no key; CORS answers them first.
            .wrap(cors)
            // Malformed input is reported in the same shape as every other error.
            .app_data(web::JsonConfig::default().error_handler(|error, _| {
                let response = json_error("INVALID_BODY", &error.to_string(), 400);
                actix_web::error::InternalError::from_response(error, response).into()
            }))
            .app_data(web::QueryConfig::default().error_handler(|error, _| {
                let response = json_error("INVALID_QUERY", &error.to_string(), 400);
                actix_web::error::InternalError::from_response(error, response).into()
            }))
            .app_data(web::PathConfig::default().error_handler(|error, _| {
                let response = json_error("NOT_FOUND", "No such resource", 404);
                actix_web::error::InternalError::from_response(error, response).into()
            }))
            .app_data(telegram.clone())
            .app_data(api_state.clone())
            .app_data(cache_dirs.clone())
            .app_data(bandwidth.clone())
            .app_data(network.clone())
            .app_data(database.clone())
            .configure(configure_api)
    })
    .listen(listener)?
    .run())
}

/// Register all API routes on the Actix App
pub fn configure_api(cfg: &mut web::ServiceConfig) {
    // Fixed paths are registered before the patterns that would capture them:
    // `/files/search` is not a file identifier.
    cfg.service(api_health)
        .service(api_openapi)
        .service(api_list_files)
        .service(api_search_files)
        .service(api_get_file)
        .service(api_download_file)
        .service(api_bulk_files)
        .service(api_delete_file)
        .service(api_copy_file)
        .service(api_update_file)
        .service(api_upload_file)
        .service(api_list_folders)
        .service(api_create_folder)
        .service(api_rename_folder)
        .service(api_delete_folder)
        .service(api_storage_stats)
        .service(api_storage_duplicates)
        .service(api_empty_folders)
        .service(api_get_file_thumbnail)
        .service(api_media_info);
}
