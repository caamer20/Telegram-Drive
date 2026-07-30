use crate::commands::utils::resolve_peer;
use crate::commands::TelegramState;
use crate::server::{
    authenticate_bearer_request, redact_sensitive, AuthenticationResult, StreamTokenData,
};
use actix_web::{web, HttpRequest, HttpResponse};
use grammers_client::types::{media::Document, photo_sizes::PhotoSize, Media, Peer};
use grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

const MAX_MEDIA_PAGE_LIMIT: usize = 200;
const MAX_THUMBNAIL_TARGET_PX: u32 = 1_024;
const MIN_THUMBNAIL_TARGET_PX: u32 = 64;
const MAX_THUMBNAIL_BYTES: usize = 12 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeAccountResponse {
    pub account_id: i64,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeDrivePeer {
    pub peer_id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeDrivePeersResponse {
    pub items: Vec<NativeDrivePeer>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NativeMediaType {
    Image,
    AnimatedImage,
    Video,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeMediaRecord {
    pub account_id: i64,
    pub peer_id: i64,
    pub folder_id: Option<i64>,
    pub message_id: i32,
    pub peer_name: Option<String>,
    pub sender_id: Option<i64>,
    pub date_epoch_seconds: i64,
    pub display_name: String,
    pub original_filename: Option<String>,
    pub caption: Option<String>,
    pub media_type: NativeMediaType,
    pub mime_type: Option<String>,
    pub extension: Option<String>,
    pub size_bytes: Option<u64>,
    pub duration_seconds: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub thumbnail_available: bool,
    pub thumbnail_variant: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeMediaPageRequest {
    pub folder_id: Option<i64>,
    #[serde(default)]
    pub offset_message_id: i32,
    pub limit: usize,
    pub newer_than_message_id: Option<i32>,
}

impl NativeMediaPageRequest {
    fn validate(&self) -> Result<(), &'static str> {
        if self.folder_id.is_some_and(|id| id <= 0) {
            return Err("folderId must be null or positive");
        }
        if self.offset_message_id < 0 {
            return Err("offsetMessageId must not be negative");
        }
        if !(1..=MAX_MEDIA_PAGE_LIMIT).contains(&self.limit) {
            return Err("limit must be between 1 and 200");
        }
        if self.newer_than_message_id.is_some_and(|id| id <= 0) {
            return Err("newerThanMessageId must be null or positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeMediaPageResponse {
    pub items: Vec<NativeMediaRecord>,
    pub next_offset_message_id: Option<i32>,
    pub has_more: bool,
    pub messages_scanned: usize,
    pub media_found: usize,
    pub newest_scanned_message_id: Option<i32>,
    pub oldest_scanned_message_id: Option<i32>,
    pub reached_newer_than_boundary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DocumentFacts<'a> {
    mime_type: Option<&'a str>,
    file_name: Option<&'a str>,
    has_video_attribute: bool,
    animated: bool,
    size: Option<u64>,
    duration_seconds: Option<u32>,
    resolution: Option<(u32, u32)>,
    has_thumbnail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassifiedDocument {
    media_type: NativeMediaType,
    mime_type: Option<String>,
    extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThumbnailCandidate {
    pub index: usize,
    pub variant: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub estimated_bytes: usize,
    pub downloadable: bool,
    pub static_image: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThumbnailQuery {
    target_px: Option<u32>,
}

fn auth_failure(req: &HttpRequest, token: &StreamTokenData) -> Option<HttpResponse> {
    match authenticate_bearer_request(req, &token.token) {
        AuthenticationResult::Authorized => None,
        AuthenticationResult::Missing => Some(
            HttpResponse::Unauthorized()
                .insert_header(("WWW-Authenticate", "Bearer"))
                .json(serde_json::json!({"error":"authorization required"})),
        ),
        AuthenticationResult::Invalid => Some(
            HttpResponse::Forbidden().json(serde_json::json!({"error":"authorization rejected"})),
        ),
    }
}

async fn current_client(
    data: &web::Data<Arc<TelegramState>>,
) -> Result<grammers_client::Client, HttpResponse> {
    data.client.lock().await.clone().ok_or_else(|| {
        HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({"error":"Telegram runtime unavailable"}))
    })
}

async fn account(
    req: HttpRequest,
    data: web::Data<Arc<TelegramState>>,
    token: web::Data<StreamTokenData>,
) -> HttpResponse {
    if let Some(response) = auth_failure(&req, &token) {
        return response;
    }
    let client = match current_client(&data).await {
        Ok(client) => client,
        Err(response) => return response,
    };
    match client.get_me().await {
        Ok(me) => HttpResponse::Ok().json(NativeAccountResponse {
            account_id: me.bare_id(),
            display_name: Some(me.full_name()).filter(|name| !name.trim().is_empty()),
        }),
        Err(error) => {
            log::warn!(
                "Native media account lookup failed: {}",
                redact_sensitive(&error.to_string())
            );
            HttpResponse::ServiceUnavailable()
                .json(serde_json::json!({"error":"Telegram account unavailable"}))
        }
    }
}

fn display_drive_name(name: &str) -> String {
    name.replace(" [TD]", "")
        .replace(" [td]", "")
        .replace("[TD]", "")
        .replace("[td]", "")
        .trim()
        .to_string()
}

async fn discover_drive_peers(
    client: &grammers_client::Client,
) -> Result<Vec<(NativeDrivePeer, Peer)>, String> {
    let me = client.get_me().await.map_err(|error| error.to_string())?;
    let mut result = vec![(
        NativeDrivePeer {
            peer_id: me.bare_id(),
            folder_id: None,
            name: "Saved Messages".into(),
            kind: "saved-messages".into(),
        },
        Peer::User(me),
    )];
    let mut seen = HashSet::new();
    let mut dialogs = client.iter_dialogs();
    while let Some(dialog) = dialogs.next().await.map_err(|error| error.to_string())? {
        let Peer::Channel(channel) = &dialog.peer else {
            continue;
        };
        let title_match = channel.raw.title.to_lowercase().contains("[td]");
        let mut about_match = false;
        if !title_match && channel.raw.creator {
            let input = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: channel.raw.id,
                access_hash: channel.raw.access_hash.unwrap_or(0),
            });
            if let Ok(tl::enums::messages::ChatFull::Full(full)) = client
                .invoke(&tl::functions::channels::GetFullChannel { channel: input })
                .await
            {
                if let tl::enums::ChatFull::Full(chat) = full.full_chat {
                    about_match = chat.about.contains("[telegram-drive-folder]");
                }
            }
        }
        if (title_match || about_match) && seen.insert(channel.raw.id) {
            result.push((
                NativeDrivePeer {
                    peer_id: channel.raw.id,
                    folder_id: Some(channel.raw.id),
                    name: if title_match {
                        display_drive_name(&channel.raw.title)
                    } else {
                        channel.raw.title.clone()
                    },
                    kind: "channel".into(),
                },
                dialog.peer,
            ));
        }
    }
    Ok(result)
}

async fn peers(
    req: HttpRequest,
    data: web::Data<Arc<TelegramState>>,
    token: web::Data<StreamTokenData>,
) -> HttpResponse {
    if let Some(response) = auth_failure(&req, &token) {
        return response;
    }
    let client = match current_client(&data).await {
        Ok(client) => client,
        Err(response) => return response,
    };
    match discover_drive_peers(&client).await {
        Ok(items) => HttpResponse::Ok().json(NativeDrivePeersResponse {
            items: items.into_iter().map(|(item, _)| item).collect(),
        }),
        Err(error) => {
            log::warn!(
                "Native media peer scan failed: {}",
                redact_sensitive(&error)
            );
            HttpResponse::ServiceUnavailable()
                .json(serde_json::json!({"error":"Telegram Drive peers unavailable"}))
        }
    }
}

fn normalize_mime(value: Option<&str>) -> Option<String> {
    let normalized = value?
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(match normalized.as_str() {
        "image/jpg" | "image/pjpeg" => "image/jpeg".into(),
        "video/mkv" | "application/x-matroska" => "video/x-matroska".into(),
        "video/mov" => "video/quicktime".into(),
        other => other.into(),
    })
}

fn normalize_extension(file_name: Option<&str>) -> Option<String> {
    let name = file_name?.trim();
    let extension = name
        .rsplit_once('.')?
        .1
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if extension.is_empty()
        || extension.len() > 16
        || !extension
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'-')
    {
        None
    } else {
        Some(extension)
    }
}

fn classify_document(facts: DocumentFacts<'_>) -> Option<ClassifiedDocument> {
    let mime = normalize_mime(facts.mime_type);
    let extension = normalize_extension(facts.file_name);
    let video_mime = mime.as_deref().is_some_and(|value| {
        value.starts_with("video/") || matches!(value, "application/x-matroska" | "application/mp4")
    });
    let video_extension = extension
        .as_deref()
        .is_some_and(|value| matches!(value, "mp4" | "webm" | "mov" | "mkv"));
    let image_mime = mime.as_deref().is_some_and(|value| {
        value.starts_with("image/") || matches!(value, "application/heic" | "application/heif")
    });
    let image_extension = extension.as_deref().is_some_and(|value| {
        matches!(
            value,
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "heif"
        )
    });
    let media_type = if facts.has_video_attribute || video_mime || (!image_mime && video_extension)
    {
        NativeMediaType::Video
    } else if image_mime || image_extension {
        if facts.animated
            || mime.as_deref() == Some("image/gif")
            || extension.as_deref() == Some("gif")
        {
            NativeMediaType::AnimatedImage
        } else {
            NativeMediaType::Image
        }
    } else {
        return None;
    };
    Some(ClassifiedDocument {
        media_type,
        mime_type: mime,
        extension,
    })
}

fn positive_dimension(value: i32) -> Option<u32> {
    u32::try_from(value).ok().filter(|value| *value > 0)
}

fn candidate_from_photo_size(index: usize, size: &PhotoSize) -> ThumbnailCandidate {
    match size {
        PhotoSize::Size(value) => ThumbnailCandidate {
            index,
            variant: size.photo_type(),
            width: positive_dimension(value.width),
            height: positive_dimension(value.height),
            estimated_bytes: size.size(),
            downloadable: true,
            static_image: true,
        },
        PhotoSize::Cached(value) => ThumbnailCandidate {
            index,
            variant: size.photo_type(),
            width: positive_dimension(value.width),
            height: positive_dimension(value.height),
            estimated_bytes: size.size(),
            downloadable: true,
            static_image: true,
        },
        PhotoSize::Progressive(value) => ThumbnailCandidate {
            index,
            variant: size.photo_type(),
            width: positive_dimension(value.width),
            height: positive_dimension(value.height),
            estimated_bytes: size.size(),
            downloadable: true,
            static_image: true,
        },
        PhotoSize::Stripped(_) => ThumbnailCandidate {
            index,
            variant: size.photo_type(),
            width: None,
            height: None,
            estimated_bytes: size.size(),
            downloadable: true,
            static_image: true,
        },
        PhotoSize::Path(_) | PhotoSize::Empty(_) => ThumbnailCandidate {
            index,
            variant: size.photo_type(),
            width: None,
            height: None,
            estimated_bytes: size.size(),
            downloadable: false,
            static_image: false,
        },
    }
}

fn collect_candidates(thumbs: &[PhotoSize]) -> Vec<ThumbnailCandidate> {
    thumbs
        .iter()
        .enumerate()
        .map(|(index, size)| candidate_from_photo_size(index, size))
        .collect()
}

pub(crate) fn bound_target_px(value: Option<u32>) -> u32 {
    value
        .unwrap_or(320)
        .clamp(MIN_THUMBNAIL_TARGET_PX, MAX_THUMBNAIL_TARGET_PX)
}

pub(crate) fn select_thumbnail_candidate(
    candidates: &[ThumbnailCandidate],
    target_px: u32,
) -> Option<&ThumbnailCandidate> {
    let target = bound_target_px(Some(target_px));
    let mut eligible: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.downloadable && candidate.static_image)
        .collect();
    eligible.sort_by_key(|candidate| {
        let dimension = candidate.width.max(candidate.height).unwrap_or(0);
        let undersized = dimension < target;
        (
            undersized,
            if undersized {
                u32::MAX - dimension
            } else {
                dimension - target
            },
            candidate.estimated_bytes,
            candidate.index,
        )
    });
    eligible.into_iter().next()
}

fn document_facts(document: &Document) -> DocumentFacts<'_> {
    let (has_video_attribute, animated) = match document.raw.document.as_ref() {
        Some(tl::enums::Document::Document(raw)) => (
            raw.attributes
                .iter()
                .any(|attribute| matches!(attribute, tl::enums::DocumentAttribute::Video(_))),
            raw.attributes
                .iter()
                .any(|attribute| matches!(attribute, tl::enums::DocumentAttribute::Animated)),
        ),
        _ => (false, false),
    };
    let resolution = document.resolution().and_then(|(width, height)| {
        Some((positive_dimension(width)?, positive_dimension(height)?))
    });
    DocumentFacts {
        mime_type: document.mime_type(),
        file_name: Some(document.name()).filter(|name| !name.trim().is_empty()),
        has_video_attribute,
        animated,
        size: u64::try_from(document.size()).ok().filter(|size| *size > 0),
        duration_seconds: document
            .duration()
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .map(|duration| duration.round().min(u32::MAX as f64) as u32),
        resolution,
        has_thumbnail: document
            .thumbs()
            .iter()
            .any(|thumbnail| thumbnail.size() > 0),
    }
}

fn record_from_message(
    message: &grammers_client::types::Message,
    media: Media,
    account_id: i64,
    peer_id: i64,
    folder_id: Option<i64>,
    peer_name: Option<&str>,
) -> Option<NativeMediaRecord> {
    let caption = Some(message.text().trim().to_string()).filter(|value| !value.is_empty());
    let sender_id = message.sender().map(|sender| sender.id().bare_id());
    let base = |display_name: String,
                original_filename: Option<String>,
                media_type: NativeMediaType,
                mime_type: Option<String>,
                extension: Option<String>,
                size_bytes: Option<u64>,
                duration_seconds: Option<u32>,
                width: Option<u32>,
                height: Option<u32>,
                thumbnail_variant: Option<String>| NativeMediaRecord {
        account_id,
        peer_id,
        folder_id,
        message_id: message.id(),
        peer_name: peer_name.map(ToOwned::to_owned),
        sender_id,
        date_epoch_seconds: message.date().timestamp(),
        display_name,
        original_filename,
        caption: caption.clone(),
        media_type,
        mime_type,
        extension,
        size_bytes,
        duration_seconds,
        width,
        height,
        thumbnail_available: thumbnail_variant.is_some(),
        thumbnail_variant,
    };
    match media {
        Media::Photo(photo) => {
            let thumbs = photo.thumbs();
            let candidates = collect_candidates(&thumbs);
            let selected = select_thumbnail_candidate(&candidates, 320);
            let dimensions = candidates
                .iter()
                .filter_map(|candidate| Some((candidate.width?, candidate.height?)))
                .max_by_key(|(width, height)| width.saturating_mul(*height));
            let display_name = caption
                .clone()
                .unwrap_or_else(|| format!("Photo {}.jpg", message.id()));
            Some(base(
                display_name,
                None,
                NativeMediaType::Image,
                Some("image/jpeg".into()),
                Some("jpg".into()),
                u64::try_from(photo.size()).ok().filter(|size| *size > 0),
                None,
                dimensions.map(|value| value.0),
                dimensions.map(|value| value.1),
                selected.map(|candidate| candidate.variant.clone()),
            ))
        }
        Media::Document(document) => {
            let facts = document_facts(&document);
            let classified = classify_document(facts)?;
            let original_filename = facts.file_name.map(ToOwned::to_owned);
            let display_name = original_filename
                .clone()
                .or_else(|| caption.clone())
                .unwrap_or_else(|| format!("Media {}", message.id()));
            let candidates = collect_candidates(&document.thumbs());
            let selected = select_thumbnail_candidate(&candidates, 320);
            Some(base(
                display_name,
                original_filename,
                classified.media_type,
                classified.mime_type,
                classified.extension,
                facts.size,
                facts.duration_seconds,
                facts.resolution.map(|value| value.0),
                facts.resolution.map(|value| value.1),
                selected.map(|candidate| candidate.variant.clone()),
            ))
        }
        _ => None,
    }
}

async fn media_page(
    req: HttpRequest,
    body: web::Json<NativeMediaPageRequest>,
    data: web::Data<Arc<TelegramState>>,
    token: web::Data<StreamTokenData>,
) -> HttpResponse {
    if let Some(response) = auth_failure(&req, &token) {
        return response;
    }
    if let Err(error) = body.validate() {
        return HttpResponse::BadRequest().json(serde_json::json!({"error":error}));
    }
    let client = match current_client(&data).await {
        Ok(client) => client,
        Err(response) => return response,
    };
    let account = match client.get_me().await {
        Ok(account) => account,
        Err(_) => {
            return HttpResponse::ServiceUnavailable()
                .json(serde_json::json!({"error":"Telegram account unavailable"}))
        }
    };
    let peer = match resolve_peer(&client, body.folder_id, &data.peer_cache).await {
        Ok(peer) => peer,
        Err(error) => {
            log::warn!(
                "Native media peer resolution failed: {}",
                redact_sensitive(&error)
            );
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error":"Telegram Drive peer unavailable"}));
        }
    };
    let peer_id = peer.id().bare_id();
    let peer_name = if body.folder_id.is_none() {
        Some("Saved Messages")
    } else {
        peer.name()
    };
    let mut iterator = client
        .iter_messages(&peer)
        .offset_id(body.offset_message_id)
        .limit(body.limit + 1);
    let mut items = Vec::with_capacity(body.limit);
    let mut scanned = 0usize;
    let mut newest_scanned = None;
    let mut oldest_scanned = None;
    let mut has_more = false;
    let mut reached_boundary = false;
    loop {
        let message = match iterator.next().await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(error) => {
                log::warn!(
                    "Native media page failed: {}",
                    redact_sensitive(&error.to_string())
                );
                return HttpResponse::ServiceUnavailable()
                    .json(serde_json::json!({"error":"Telegram media page unavailable"}));
            }
        };
        if body
            .newer_than_message_id
            .is_some_and(|boundary| message.id() <= boundary)
        {
            reached_boundary = true;
            break;
        }
        if scanned == body.limit {
            has_more = true;
            break;
        }
        scanned += 1;
        newest_scanned.get_or_insert(message.id());
        oldest_scanned = Some(message.id());
        if let Some(media) = message.media() {
            if let Some(record) = record_from_message(
                &message,
                media,
                account.bare_id(),
                peer_id,
                body.folder_id,
                peer_name,
            ) {
                items.push(record);
            }
        }
    }
    let response = NativeMediaPageResponse {
        media_found: items.len(),
        items,
        next_offset_message_id: oldest_scanned,
        has_more,
        messages_scanned: scanned,
        newest_scanned_message_id: newest_scanned,
        oldest_scanned_message_id: oldest_scanned,
        reached_newer_than_boundary: reached_boundary,
    };
    HttpResponse::Ok().json(response)
}

fn thumbnail_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

async fn download_thumbnail(
    client: &grammers_client::Client,
    thumbnail: &PhotoSize,
) -> Result<Vec<u8>, String> {
    let mut iterator = client.iter_download(thumbnail).chunk_size(64 * 1024);
    let mut bytes = Vec::with_capacity(thumbnail.size().min(MAX_THUMBNAIL_BYTES));
    while let Some(chunk) = iterator.next().await.map_err(|error| error.to_string())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_THUMBNAIL_BYTES {
            return Err("thumbnail exceeded the bounded response size".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        Err("thumbnail data was empty".into())
    } else {
        Ok(bytes)
    }
}

async fn thumbnail(
    req: HttpRequest,
    path: web::Path<(String, i32)>,
    query: web::Query<ThumbnailQuery>,
    data: web::Data<Arc<TelegramState>>,
    token: web::Data<StreamTokenData>,
) -> HttpResponse {
    if let Some(response) = auth_failure(&req, &token) {
        return response;
    }
    let (folder, message_id) = path.into_inner();
    if message_id <= 0 {
        return HttpResponse::BadRequest().json(serde_json::json!({"error":"invalid messageId"}));
    }
    let folder_id = if matches!(folder.as_str(), "home" | "me") {
        None
    } else {
        match folder.parse::<i64>().ok().filter(|value| *value > 0) {
            Some(value) => Some(value),
            None => {
                return HttpResponse::BadRequest()
                    .json(serde_json::json!({"error":"invalid folderId"}))
            }
        }
    };
    let target = bound_target_px(query.target_px);
    let client = match current_client(&data).await {
        Ok(client) => client,
        Err(response) => return response,
    };
    let peer = match resolve_peer(&client, folder_id, &data.peer_cache).await {
        Ok(peer) => peer,
        Err(_) => {
            return HttpResponse::NotFound()
                .json(serde_json::json!({"error":"media source not found"}))
        }
    };
    let message = match client.get_messages_by_id(&peer, &[message_id]).await {
        Ok(messages) => messages.into_iter().flatten().next(),
        Err(error) => {
            log::warn!(
                "Native thumbnail lookup failed: {}",
                redact_sensitive(&error.to_string())
            );
            return HttpResponse::ServiceUnavailable()
                .json(serde_json::json!({"error":"Telegram thumbnail unavailable"}));
        }
    };
    let Some(media) = message.and_then(|message| message.media()) else {
        return HttpResponse::NotFound().json(serde_json::json!({"error":"thumbnail not found"}));
    };
    let thumbs = match media {
        Media::Photo(photo) => photo.thumbs(),
        Media::Document(document) => document.thumbs(),
        _ => Vec::new(),
    };
    let candidates = collect_candidates(&thumbs);
    let Some(candidate) = select_thumbnail_candidate(&candidates, target) else {
        return HttpResponse::NotFound()
            .json(serde_json::json!({"error":"thumbnail not available"}));
    };
    match download_thumbnail(&client, &thumbs[candidate.index]).await {
        Ok(bytes) => HttpResponse::Ok()
            .insert_header(("Cache-Control", "private, max-age=86400"))
            .insert_header(("X-Content-Type-Options", "nosniff"))
            .content_type(thumbnail_mime(&bytes))
            .body(bytes),
        Err(error) => {
            log::warn!(
                "Native thumbnail download failed: {}",
                redact_sensitive(&error)
            );
            HttpResponse::ServiceUnavailable()
                .json(serde_json::json!({"error":"Telegram thumbnail unavailable"}))
        }
    }
}

pub fn configure_media_library_routes(config: &mut web::ServiceConfig) {
    config.service(
        web::scope("/native-media-library/v1")
            .app_data(web::JsonConfig::default().limit(4 * 1024))
            .route("/account", web::get().to(account))
            .route("/peers", web::get().to(peers))
            .route("/media-page", web::post().to(media_page))
            .route("/thumbnail/{folder}/{message_id}", web::get().to(thumbnail)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header;
    use actix_web::test::TestRequest;

    fn facts<'a>(mime: Option<&'a str>, file: Option<&'a str>) -> DocumentFacts<'a> {
        DocumentFacts {
            mime_type: mime,
            file_name: file,
            has_video_attribute: false,
            animated: false,
            size: None,
            duration_seconds: None,
            resolution: None,
            has_thumbnail: false,
        }
    }

    #[test]
    fn classifies_image_video_and_gif_documents_from_metadata() {
        assert_eq!(
            classify_document(facts(
                Some("image/HEIC; charset=binary"),
                Some("capture.HEIC")
            ))
            .unwrap()
            .media_type,
            NativeMediaType::Image
        );
        let mut video = facts(Some("application/octet-stream"), Some("clip.bin"));
        video.has_video_attribute = true;
        video.duration_seconds = Some(9);
        video.resolution = Some((1920, 1080));
        let classified_video = classify_document(video).unwrap();
        assert_eq!(classified_video.media_type, NativeMediaType::Video);
        assert_eq!(video.duration_seconds, Some(9));
        assert_eq!(video.resolution, Some((1920, 1080)));
        let mut gif = facts(Some("image/gif"), Some("animation.GIF"));
        gif.animated = true;
        assert_eq!(
            classify_document(gif).unwrap().media_type,
            NativeMediaType::AnimatedImage
        );
    }

    #[test]
    fn video_containers_are_supported_only_after_metadata_fallbacks() {
        for (mime, extension) in [
            ("video/mp4", "mp4"),
            ("video/webm", "webm"),
            ("video/quicktime", "mov"),
            ("video/x-matroska", "mkv"),
        ] {
            let file = format!("movie.{extension}");
            let value = classify_document(facts(Some(mime), Some(&file))).unwrap();
            assert_eq!(value.media_type, NativeMediaType::Video);
            assert_eq!(value.extension.as_deref(), Some(extension));
        }
    }

    #[test]
    fn normalizes_mime_and_extension_without_inventing_unknown_values() {
        assert_eq!(
            normalize_mime(Some(" IMAGE/JPG ; q=1")).as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            normalize_extension(Some("archive.photo.JPEG")).as_deref(),
            Some("jpeg")
        );
        assert_eq!(normalize_extension(Some("no-extension")), None);
        assert_eq!(normalize_mime(None), None);
        assert_eq!(facts(None, None).size, None);
    }

    #[test]
    fn validates_page_limit_and_cursor_ids() {
        let mut request = NativeMediaPageRequest {
            folder_id: None,
            offset_message_id: 0,
            limit: 200,
            newer_than_message_id: None,
        };
        assert!(request.validate().is_ok());
        request.limit = 201;
        assert!(request.validate().is_err());
        request.limit = 1;
        request.offset_message_id = -1;
        assert!(request.validate().is_err());
        request.offset_message_id = 0;
        request.newer_than_message_id = Some(0);
        assert!(request.validate().is_err());
    }

    #[test]
    fn bounds_and_ranks_static_thumbnail_candidates() {
        assert_eq!(bound_target_px(Some(1)), 64);
        assert_eq!(bound_target_px(Some(50_000)), 1_024);
        assert_eq!(bound_target_px(None), 320);
        let candidates = vec![
            ThumbnailCandidate {
                index: 0,
                variant: "s".into(),
                width: Some(90),
                height: Some(90),
                estimated_bytes: 1,
                downloadable: true,
                static_image: true,
            },
            ThumbnailCandidate {
                index: 1,
                variant: "m".into(),
                width: Some(320),
                height: Some(240),
                estimated_bytes: 2,
                downloadable: true,
                static_image: true,
            },
            ThumbnailCandidate {
                index: 2,
                variant: "v".into(),
                width: Some(640),
                height: Some(360),
                estimated_bytes: 3,
                downloadable: true,
                static_image: true,
            },
            ThumbnailCandidate {
                index: 3,
                variant: "path".into(),
                width: None,
                height: None,
                estimated_bytes: 1,
                downloadable: false,
                static_image: false,
            },
        ];
        assert_eq!(
            select_thumbnail_candidate(&candidates, 300)
                .unwrap()
                .variant,
            "m"
        );
        assert_eq!(
            select_thumbnail_candidate(&candidates, 500)
                .unwrap()
                .variant,
            "v"
        );
    }

    #[test]
    fn determines_binary_thumbnail_mime_without_paths() {
        assert_eq!(thumbnail_mime(b"\x89PNG\r\n\x1a\nrest"), "image/png");
        assert_eq!(thumbnail_mime(b"GIF89arest"), "image/gif");
        assert_eq!(thumbnail_mime(b"RIFFxxxxWEBPrest"), "image/webp");
        assert_eq!(thumbnail_mime(b"\xff\xd8\xff"), "image/jpeg");
    }

    #[test]
    fn bearer_auth_has_no_query_fallback_for_new_routes() {
        let missing = TestRequest::get()
            .uri("/native-media-library/v1/account?token=secret")
            .to_http_request();
        assert_eq!(
            authenticate_bearer_request(&missing, "secret"),
            AuthenticationResult::Missing
        );
        let token = StreamTokenData {
            token: "secret".into(),
        };
        assert_eq!(
            auth_failure(&missing, &token).unwrap().status(),
            actix_web::http::StatusCode::UNAUTHORIZED
        );
        let invalid = TestRequest::get()
            .insert_header((header::AUTHORIZATION, "Bearer wrong"))
            .to_http_request();
        assert_eq!(
            authenticate_bearer_request(&invalid, "secret"),
            AuthenticationResult::Invalid
        );
        assert_eq!(
            auth_failure(&invalid, &token).unwrap().status(),
            actix_web::http::StatusCode::FORBIDDEN
        );
        let valid = TestRequest::get()
            .insert_header((header::AUTHORIZATION, "Bearer secret"))
            .to_http_request();
        assert_eq!(
            authenticate_bearer_request(&valid, "secret"),
            AuthenticationResult::Authorized
        );
        assert!(auth_failure(&valid, &token).is_none());
    }

    #[test]
    fn serialized_responses_have_no_telegram_secrets() {
        let account = NativeAccountResponse {
            account_id: 7,
            display_name: Some("Person".into()),
        };
        let peer = NativeDrivePeersResponse {
            items: vec![NativeDrivePeer {
                peer_id: 7,
                folder_id: None,
                name: "Saved Messages".into(),
                kind: "saved-messages".into(),
            }],
        };
        for value in [
            serde_json::to_string(&account).unwrap(),
            serde_json::to_string(&peer).unwrap(),
        ] {
            let lower = value.to_ascii_lowercase();
            for forbidden in [
                "access_hash",
                "accesshash",
                "file_reference",
                "filereference",
                "authorization",
                "token",
                "phone",
            ] {
                assert!(!lower.contains(forbidden), "{forbidden} leaked in {value}");
            }
        }
    }

    #[test]
    fn caption_and_original_filename_are_separate_model_fields() {
        let record = NativeMediaRecord {
            account_id: 1,
            peer_id: 2,
            folder_id: Some(2),
            message_id: 3,
            peer_name: Some("Folder".into()),
            sender_id: None,
            date_epoch_seconds: 4,
            display_name: "photo.png".into(),
            original_filename: Some("photo.png".into()),
            caption: Some("holiday".into()),
            media_type: NativeMediaType::Image,
            mime_type: Some("image/png".into()),
            extension: Some("png".into()),
            size_bytes: None,
            duration_seconds: None,
            width: Some(10),
            height: Some(20),
            thumbnail_available: true,
            thumbnail_variant: Some("m".into()),
        };
        let value = serde_json::to_value(record).unwrap();
        assert_eq!(value["originalFilename"], "photo.png");
        assert_eq!(value["caption"], "holiday");
        assert_eq!(value["extension"], "png");
    }
}
