//! Isolated backend process for native E2E journeys, excluded from normal builds.
//!
//! The protocol drives production storage, crypto and loopback HTTP services.
//! It does not simulate a Tauri window or claim authenticated Telegram coverage.
use crate::commands::{download_destination, TelegramState};
use crate::crypto::{
    self,
    envelope::encrypt_reader::{EncryptingReader, EncryptionSession},
    envelope::EnvelopeHeader,
    secret::SecretKey,
    state::CryptoState,
    vault::FileVault,
};
use crate::sync_engine::{
    self, config as sync_config, native_fixture::NativeSyncFixture, policy::StoredPairPolicy,
    SyncEngine,
};
use crate::transfer_engine::{TransferJob, TransferStore};
use crate::workspace::{store::Store, AccountGuard};
use grammers_session::{storages::SqliteSession, types::PeerInfo, Session};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, AtomicU64},
        Arc,
    },
};
use tauri::{Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

// Allow bounded source work while lifecycle journeys exercise notifications and retention.
const INVENTORY_FIXTURE_RECONCILIATION: Duration = Duration::from_secs(30);

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request[key]
        .as_str()
        .ok_or_else(|| format!("Missing {key}"))
}

fn child(root: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || Path::new(name).components().count() != 1 || name == "." || name == ".." {
        return Err("Expected a fixture filename".into());
    }
    Ok(root.join(name))
}
/// Stands in for Telegram's part storage: every part is kept as a file, and
/// one chosen part can be refused to interrupt the upload.
struct FixtureSink {
    directory: PathBuf,
    fail_at: Option<i32>,
    saved: std::sync::Mutex<Vec<i32>>,
}

impl crate::resumable_upload::PartSink for FixtureSink {
    async fn save_part(
        &self,
        file_id: i64,
        part: i32,
        _total_parts: i32,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        if self.fail_at == Some(part) {
            return Err("Upload failed: connection reset by peer".into());
        }
        tokio::fs::write(self.directory.join(format!("{file_id}-{part}.part")), bytes)
            .await
            .map_err(error)?;
        self.saved
            .lock()
            .map_err(|_| "Fixture sink lock poisoned".to_string())?
            .push(part);
        Ok(())
    }
}

struct FixtureProxySecret(PathBuf);
impl crate::vpn_optimizer::ProxySecretStore for FixtureProxySecret {
    fn read(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(&self.0) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
    fn write(&self, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(value) => std::fs::write(&self.0, value).map_err(error),
            None => match std::fs::remove_file(&self.0) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            },
        }
    }
}

struct SlowFixtureProxySecret {
    root: PathBuf,
}

impl crate::vpn_optimizer::ProxySecretStore for SlowFixtureProxySecret {
    fn read(&self) -> Result<Option<String>, String> {
        std::fs::write(self.root.join("proxy-io-started"), b"started").map_err(error)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !self.root.join("proxy-io-release").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        crate::vpn_optimizer::ProxySecretStore::read(&FixtureProxySecret(
            self.root.join("proxy-credential"),
        ))
    }
    fn write(&self, value: Option<&str>) -> Result<(), String> {
        crate::vpn_optimizer::ProxySecretStore::write(
            &FixtureProxySecret(self.root.join("proxy-credential")),
            value,
        )
    }
}

/// The folder every upload journey targets.
const UPLOAD_FOLDER: i64 = 4242;

fn disconnected() -> Arc<TelegramState> {
    Arc::new(TelegramState {
        client: Arc::new(Mutex::new(None)),
        session: Arc::new(Mutex::new(None)),
        phone_login: Arc::new(Mutex::new(None)),
        password_token: Arc::new(Mutex::new(None)),
        api_id: Arc::new(Mutex::new(None)),
        auth_attempt_counter: Arc::new(AtomicU64::new(0)),
        sync_readiness: Default::default(),
        runner_shutdown: Arc::new(std::sync::Mutex::new(None)),
        runner_count: Arc::new(AtomicU32::new(0)),
        peer_cache: Arc::new(RwLock::new(HashMap::new())),
        active_file_loads: Arc::new(RwLock::new(HashMap::new())),
        cancelled_transfers: Arc::new(RwLock::new(HashSet::new())),
    })
}

#[derive(Clone)]
struct MediaFixture {
    bandwidth: Arc<crate::bandwidth::BandwidthManager>,
    network: Arc<crate::vpn_optimizer::NetworkConfig>,
    root: PathBuf,
    source: PathBuf,
    filename: String,
    resolutions: Arc<AtomicU64>,
    cache: Arc<crate::server::MediaResolutionCache<PathBuf>>,
    resolution_gate: Arc<tokio::sync::Notify>,
}

fn fixture_download(
    source: PathBuf,
    start: u64,
    fail_after: Option<u64>,
    delay_ms: u64,
) -> impl futures::Stream<Item = Result<actix_web::web::Bytes, actix_web::Error>> {
    async_stream::stream! {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let mut file = match tokio::fs::File::open(source).await {
            Ok(file) => file,
            Err(error) => { yield Err(actix_web::error::ErrorBadGateway(error)); return; }
        };
        if let Err(error) = file.seek(std::io::SeekFrom::Start(start / 524_288 * 524_288)).await {
            yield Err(actix_web::error::ErrorBadGateway(error)); return;
        }
        let mut delivered = 0;
        loop {
            if fail_after.is_some_and(|limit| delivered >= limit) {
                yield Err(actix_web::error::ErrorBadGateway("Fixture download interrupted")); return;
            }
            let mut bytes = vec![0; 65_536];
            let count = match file.read(&mut bytes).await {
                Ok(0) => return,
                Ok(count) => count,
                Err(error) => { yield Err(actix_web::error::ErrorBadGateway(error)); return; }
            };
            bytes.truncate(count);
            if delay_ms > 0 { tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await; }
            delivered += count as u64;
            yield Ok(actix_web::web::Bytes::from(bytes));
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaFixtureQuery {
    owner: String,
    fail_after: Option<u64>,
    declared_size: Option<u64>,
    #[serde(default)]
    delay_ms: u64,
    #[serde(default)]
    protected: bool,
    credential: Option<u64>,
    #[serde(default)]
    resolve_failure: bool,
    #[serde(default)]
    message_id: i32,
    #[serde(default)]
    wait_for_resolution: bool,
}

async fn fixture_media(
    request: actix_web::HttpRequest,
    query: actix_web::web::Query<MediaFixtureQuery>,
    fixture: actix_web::web::Data<MediaFixture>,
    crypto: actix_web::web::Data<CryptoState>,
) -> actix_web::HttpResponse {
    let account = match AccountGuard::open(&fixture.root, Some(&query.owner)) {
        Ok(account) => account,
        Err(_) => return actix_web::HttpResponse::NotFound().finish(),
    };
    let source = match fixture
        .cache
        .resolve(&account, (None, query.message_id), || async {
            fixture
                .resolutions
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if query.wait_for_resolution {
                fixture.resolution_gate.notified().await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            if query.resolve_failure {
                return Err("Fixture resolution interrupted".into());
            }
            Ok(fixture.source.clone())
        })
        .await
    {
        Ok(source) => source,
        Err(error) if error.starts_with("ACCOUNT_") => {
            return actix_web::HttpResponse::NotFound().finish()
        }
        Err(_) => return actix_web::HttpResponse::BadGateway().finish(),
    };
    let size = match tokio::fs::metadata(&source).await {
        Ok(metadata) => query.declared_size.unwrap_or(metadata.len()),
        Err(_) => return actix_web::HttpResponse::NotFound().finish(),
    };
    if query.protected {
        let Some(credential) = query.credential else {
            return actix_web::HttpResponse::Locked().finish();
        };
        let key = match crypto
            .operation_wrapping_key(credential, crypto::state::OperationClass::MediaStream)
        {
            Ok(key) => key,
            Err(_) => return actix_web::HttpResponse::Locked().finish(),
        };
        use tokio::io::AsyncReadExt;
        let mut file = tokio::fs::File::open(&source).await.unwrap();
        let mut bytes = vec![0; crypto::policy::MAX_HEADER_LENGTH];
        let count = file.read(&mut bytes).await.unwrap();
        bytes.truncate(count);
        let header = EnvelopeHeader::parse(&bytes).unwrap();
        let record = crate::server::EncryptedStreamRecord {
            header: bytes[..header.core.header_length as usize].to_vec(),
            plaintext_size: header.core.total_plaintext_length,
        };
        return crate::server::build_encrypted_media_response_from_source(
            &request,
            record,
            &key,
            &account,
            crate::server::ProtectedStreamAccess {
                bandwidth: fixture.bandwidth.clone(),
                network: fixture.network.clone(),
                account: account.clone(),
                state: crypto.get_ref().clone(),
                credential,
            },
            move |start| fixture_download(source, start, query.fail_after, query.delay_ms),
        )
        .await;
    }
    crate::server::build_media_response_from_source(
        size,
        &request,
        "application/octet-stream",
        Some(&fixture.filename),
        crate::server::StreamingExtras {
            extra_headers: vec![],
            log_label: "Fixture media",
            bandwidth: fixture.bandwidth.clone(),
            network: fixture.network.clone(),
        },
        Some(account),
        move |start| fixture_download(source, start, query.fail_after, query.delay_ms),
    )
}

fn archive_fixture_chunks(
    source: PathBuf,
) -> impl futures::Stream<Item = Result<bytes::Bytes, String>> {
    async_stream::stream! {
        use tokio::io::AsyncReadExt;
        let mut file=match tokio::fs::File::open(source).await { Ok(file)=>file,Err(error)=>{yield Err(error.to_string());return;} };
        loop {
            let mut bytes=vec![0;8192];
            match file.read(&mut bytes).await {
                Ok(0)=>break,
                Ok(count)=>{bytes.truncate(count);yield Ok(bytes::Bytes::from(bytes));}
                Err(error)=>{yield Err(error.to_string());return;}
            }
        }
    }
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct InventoryRow {
    id: i32,
    name: String,
    #[serde(default)]
    protected: bool,
}

impl crate::file_inventory::Row for InventoryRow {
    fn id(&self) -> i32 {
        self.id
    }
    fn same(&self, other: &Self) -> bool {
        self == other
    }
}
#[derive(Clone)]
struct RawInventoryRow(grammers_tl_types::enums::Message);
impl crate::file_inventory::Row for RawInventoryRow {
    fn id(&self) -> i32 {
        self.0.id()
    }
    fn same(&self, other: &Self) -> bool {
        crate::file_inventory::same_listing_message(&self.0, &other.0)
    }
}
fn raw_inventory_row(value: &Value) -> RawInventoryRow {
    let document = grammers_tl_types::types::Document {
        id: value["documentId"].as_i64().unwrap_or(1),
        access_hash: 1,
        file_reference: vec![value["reference"].as_u64().unwrap_or(1) as u8],
        date: 1,
        mime_type: value["mime"].as_str().unwrap_or("application/pdf").into(),
        size: value["size"].as_i64().unwrap_or(100),
        thumbs: None,
        video_thumbs: None,
        dc_id: 1,
        attributes: vec![grammers_tl_types::types::DocumentAttributeFilename {
            file_name: value["name"].as_str().unwrap_or("Report.pdf").into(),
        }
        .into()],
    };
    let media = grammers_tl_types::types::MessageMediaDocument {
        nopremium: false,
        spoiler: false,
        video: false,
        round: false,
        voice: false,
        document: Some(document.into()),
        alt_documents: None,
        video_cover: None,
        video_timestamp: None,
        ttl_seconds: None,
    };
    RawInventoryRow(
        grammers_tl_types::types::Message {
            out: false,
            mentioned: false,
            media_unread: false,
            silent: false,
            post: false,
            from_scheduled: false,
            legacy: false,
            edit_hide: false,
            pinned: false,
            noforwards: false,
            invert_media: false,
            offline: false,
            video_processing_pending: false,
            paid_suggested_post_stars: false,
            paid_suggested_post_ton: false,
            id: value["id"].as_i64().unwrap_or(1) as i32,
            from_id: None,
            from_boosts_applied: None,
            peer_id: grammers_tl_types::types::PeerUser { user_id: 101 }.into(),
            saved_peer_id: None,
            fwd_from: None,
            via_bot_id: None,
            via_business_bot_id: None,
            reply_to: None,
            date: value["date"].as_i64().unwrap_or(1) as i32,
            message: value["caption"].as_str().unwrap_or("").to_string(),
            media: Some(media.into()),
            reply_markup: None,
            entities: None,
            views: Some(value["views"].as_i64().unwrap_or(0) as i32),
            forwards: Some(value["forwards"].as_i64().unwrap_or(0) as i32),
            replies: None,
            edit_date: None,
            post_author: None,
            grouped_id: None,
            reactions: None,
            restriction_reason: None,
            ttl_period: None,
            quick_reply_shortcut_id: None,
            effect: None,
            factcheck: None,
            report_delivery_until_date: None,
            paid_message_stars: None,
            suggested_post: None,
            schedule_repeat_period: None,
        }
        .into(),
    )
}
#[derive(Clone)]
struct RawInventorySource(PathBuf);
impl crate::file_inventory::Source<RawInventoryRow> for RawInventorySource {
    fn history(
        &self,
        after: i32,
    ) -> futures::future::BoxFuture<'_, Result<(i32, Vec<RawInventoryRow>), String>> {
        Box::pin(async move {
            let value: Value =
                serde_json::from_slice(&tokio::fs::read(&self.0).await.map_err(error)?)
                    .map_err(error)?;
            Ok((
                value["highwater"].as_i64().unwrap_or(1) as i32,
                value["rows"]
                    .as_array()
                    .ok_or("Missing raw rows")?
                    .iter()
                    .map(raw_inventory_row)
                    .filter(|row| crate::file_inventory::Row::id(row) > after)
                    .collect(),
            ))
        })
    }
    fn lookup<'a>(
        &'a self,
        ids: &'a [i32],
    ) -> futures::future::BoxFuture<'a, Result<Vec<Option<RawInventoryRow>>, String>> {
        Box::pin(async move {
            let value: Value =
                serde_json::from_slice(&tokio::fs::read(&self.0).await.map_err(error)?)
                    .map_err(error)?;
            let rows = value["rows"].as_array().ok_or("Missing raw rows")?;
            Ok(ids
                .iter()
                .map(|id| {
                    rows.iter()
                        .find(|row| row["id"].as_i64() == Some(i64::from(*id)))
                        .map(raw_inventory_row)
                })
                .collect())
        })
    }
}

#[derive(Clone)]
struct InventorySource {
    path: PathBuf,
    history: Arc<AtomicU64>,
    calls: Arc<AtomicU64>,
    lookups: Arc<AtomicU64>,
    lookup_times: Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
    cached: Arc<std::sync::Mutex<Option<Arc<InventoryRemote>>>>,
    started: Arc<tokio::sync::Notify>,
}

struct InventoryRemote {
    highwater: i32,
    rows: std::collections::BTreeMap<i32, InventoryRow>,
    history_error: bool,
    lookup_error: bool,
    race: Option<Value>,
    delay_ms: u64,
    history_gate: bool,
}

impl InventorySource {
    async fn data(&self) -> Result<Arc<InventoryRemote>, String> {
        let cached = self.cached.lock().map_err(error)?.clone();
        if let Some(data) = cached {
            return Ok(data);
        }
        let data: Value =
            serde_json::from_slice(&tokio::fs::read(&self.path).await.map_err(error)?)
                .map_err(error)?;
        let rows: Vec<InventoryRow> =
            serde_json::from_value(data["rows"].clone()).map_err(error)?;
        let data = Arc::new(InventoryRemote {
            highwater: data["highwater"].as_i64().unwrap_or(0) as i32,
            rows: rows.into_iter().map(|row| (row.id, row)).collect(),
            history_error: data["historyError"] == true,
            lookup_error: data["lookupError"] == true,
            race: data.get("bootstrapRace").cloned(),
            delay_ms: data["historyDelayMs"].as_u64().unwrap_or(0),
            history_gate: data["historyGate"] == true,
        });
        *self.cached.lock().map_err(error)? = Some(data.clone());
        Ok(data)
    }
}

impl crate::file_inventory::Source<InventoryRow> for InventorySource {
    fn history(
        &self,
        after: i32,
    ) -> futures::future::BoxFuture<'_, Result<(i32, Vec<InventoryRow>), String>> {
        Box::pin(async move {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let data = self.data().await?;
            if data.history_error {
                return Err("history fixture unavailable".into());
            }
            if after == 0 {
                self.history
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            self.started.notify_one();
            if data.history_gate {
                let release = self.path.with_extension("release");
                let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
                while !release.is_file() {
                    if tokio::time::Instant::now() >= deadline {
                        return Err("Inventory fixture gate deadline".into());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            if data.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(data.delay_ms)).await;
            }
            if after == 0 {
                if let Some(race) = &data.race {
                    tokio::fs::write(&self.path, serde_json::to_vec(race).map_err(error)?)
                        .await
                        .map_err(error)?;
                    self.cached.lock().map_err(error)?.take();
                }
            }
            Ok((
                data.highwater.max(after),
                data.rows
                    .range((std::ops::Bound::Excluded(after), std::ops::Bound::Unbounded))
                    .map(|(_, row)| row.clone())
                    .collect(),
            ))
        })
    }
    fn lookup<'a>(
        &'a self,
        ids: &'a [i32],
    ) -> futures::future::BoxFuture<'a, Result<Vec<Option<InventoryRow>>, String>> {
        Box::pin(async move {
            if ids.len() > 100 {
                return Err("oversized verification batch".into());
            }
            self.lookup_times
                .lock()
                .map_err(error)?
                .push(std::time::Instant::now());
            self.lookups
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let data = self.data().await?;
            if data.lookup_error {
                return Err("verification fixture unavailable".into());
            }
            Ok(ids.iter().map(|id| data.rows.get(id).cloned()).collect())
        })
    }
}

#[derive(Clone)]
struct FileAssetSource {
    info: crate::workspace::assets::AssetInfo,
    body: PathBuf,
    thumbnail: Option<PathBuf>,
    downloads: Arc<AtomicU64>,
    fail: bool,
    delay: u64,
}

impl crate::workspace::assets::AssetSource for FileAssetSource {
    fn info(&self) -> &crate::workspace::assets::AssetInfo {
        &self.info
    }
    fn download<'a>(
        &'a self,
        target: &'a Path,
        thumbnail: bool,
        _request: &'a crate::workspace::assets::Request,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> futures::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            self.downloads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let input = if thumbnail {
                self.thumbnail.as_ref().ok_or("Missing fixture thumbnail")?
            } else {
                &self.body
            };
            let mut input = tokio::fs::File::open(input).await.map_err(error)?;
            let mut options = tokio::fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                options.mode(0o600);
            }
            let mut output = options.open(target).await.map_err(error)?;
            let mut bytes = vec![0; 64 * 1024];
            loop {
                if cancelled() {
                    return Err("CANCELLED".into());
                }
                let count = input.read(&mut bytes).await.map_err(error)?;
                if count == 0 {
                    break;
                }
                output.write_all(&bytes[..count]).await.map_err(error)?;
                tokio::time::sleep(std::time::Duration::from_millis(self.delay)).await;
            }
            output.sync_all().await.map_err(error)?;
            if self.fail {
                return Err("NETWORK_UNAVAILABLE: Fixture transport failed at completion".into());
            }
            Ok(())
        })
    }
}

fn fixture_asset(
    root: &Path,
    account: &AccountGuard,
    request: &Value,
    downloads: Arc<AtomicU64>,
) -> Result<
    (
        crate::workspace::store::WorkspaceFile,
        Option<Box<dyn crate::workspace::assets::AssetSource>>,
    ),
    String,
> {
    use sha2::{Digest, Sha256};
    let body = child(root, text(request, "source")?)?;
    let bytes = std::fs::read(&body).map_err(error)?;
    let thumbnail = request["thumbnailSource"]
        .as_str()
        .map(|path| child(root, path))
        .transpose()?;
    let thumbnail_size = thumbnail
        .as_ref()
        .map(|path| {
            std::fs::metadata(path)
                .map(|meta| meta.len())
                .map_err(error)
        })
        .transpose()?;
    let fallback = if request["mime"] == "image/heic" || request["mime"] == "image/heif" {
        Some(crate::workspace::assets::ThumbnailInput::Heic(
            heic_fixture_tools(root, request)?,
        ))
    } else if request["video"].as_bool().unwrap_or(false) {
        Some(crate::workspace::assets::ThumbnailInput::Video {
            executable: child(root, text(request, "ffmpeg")?)?,
            format: "mov",
            fixture: request["fixtureVideoClock"]
                .as_bool()
                .unwrap_or(false)
                .then(|| crate::workspace::assets::VideoFixtureClock {
                    ready: root.join(format!("video-{}.ready", uuid::Uuid::new_v4())),
                    delay_ms: request["fixtureVideoReadyDelayMs"]
                        .as_u64()
                        .unwrap_or(0)
                        .min(30_000),
                }),
        })
    } else {
        Some(crate::workspace::assets::ThumbnailInput::Image)
    };
    let size = bytes.len() as u64;
    let info = crate::workspace::assets::AssetInfo {
        owner: request["sourceOwner"].as_i64().unwrap_or(account.owner),
        identity: request["sourceIdentity"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("file:{:x}", Sha256::digest(&bytes))),
        size,
        thumbnail_size,
        fallback,
    };
    let id = request["id"].as_i64().unwrap_or(1);
    let folder = request["folder"].as_i64();
    let file = crate::workspace::store::WorkspaceFile {
        key: crate::workspace::store::file_key(folder, id),
        folder_name: "Fixture folder".into(),
        tags: Vec::new(),
        collection_ids: Vec::new(),
        file: crate::models::FileMetadata {
            id,
            folder_id: folder,
            name: request["filename"]
                .as_str()
                .unwrap_or("Unindexed fixture file.png")
                .into(),
            size,
            mime_type: Some(request["mime"].as_str().unwrap_or("image/png").into()),
            file_ext: Some(request["extension"].as_str().unwrap_or("png").into()),
            created_at: "2026-10-01T00:00:00Z".into(),
            icon_type: "file".into(),
            encryption_state: request["protection"].as_str().unwrap_or("plain").into(),
            is_favorite: false,
            is_pinned: false,
        },
    };
    let source = FileAssetSource {
        info,
        body,
        thumbnail,
        downloads,
        fail: request["fail"].as_bool().unwrap_or(false),
        delay: request["delayMs"].as_u64().unwrap_or(0),
    };
    Ok((
        file,
        if request["noSource"] == true {
            None
        } else {
            Some(Box::new(source))
        },
    ))
}

struct StoreSearchSource {
    folders: Vec<i64>,
    credential: Option<crate::crypto::state::UnlockSessionId>,
    names: std::collections::HashMap<String, String>,
}

impl crate::local_search::Source for StoreSearchSource {
    fn snapshot<'a>(
        &'a self,
        account: &'a AccountGuard,
        folder: Option<&'a str>,
    ) -> futures::future::BoxFuture<'a, Result<crate::local_search::Snapshot, String>> {
        Box::pin(async move {
            let owner = account.clone();
            let folders = self.folders.clone();
            let folder = folder.map(str::to_string);
            let credential = self.credential;
            let names = self.names.clone();
            tokio::task::spawn_blocking(move || {
                owner.validate()?;
                let (mut rows, _) = Store::open(&owner.root, owner.owner)?
                    .search_rows(&folders, folder.as_deref())?;
                for row in &mut rows {
                    if let Some(name) = names.get(&row.key) {
                        row.file.name = name.clone();
                        row.file.encryption_state = "encrypted_unlocked".into();
                    }
                }
                Ok(crate::local_search::Snapshot {
                    rows,
                    folders: folders.clone(),
                    credential,
                    complete: false,
                    offline: true,
                    inventory: folders
                        .iter()
                        .copied()
                        .map(Some)
                        .chain(std::iter::once(None))
                        .map(|folder| {
                            crate::local_search::InventoryStamp::capture(
                                &crate::local_search::inventory_generation(&owner, folder),
                            )
                        })
                        .collect(),
                })
            })
            .await
            .map_err(error)?
        })
    }
}

struct InventorySearchSource {
    folders: Vec<i64>,
    cache: Arc<crate::file_inventory::InventoryCache<InventoryRow>>,
    source: InventorySource,
}

impl crate::local_search::Source for InventorySearchSource {
    fn snapshot<'a>(
        &'a self,
        account: &'a AccountGuard,
        folder: Option<&'a str>,
    ) -> futures::future::BoxFuture<'a, Result<crate::local_search::Snapshot, String>> {
        Box::pin(async move {
            let mut rows = Vec::new();
            let mut inventory = vec![crate::local_search::InventoryStamp::capture(
                &crate::local_search::inventory_generation(account, Some(i64::MIN)),
            )];
            let mut complete = true;
            let mut text_bytes = 0usize;
            let store = Store::open(&account.root, account.owner)?;
            for id in &self.folders {
                if rows.len() >= 100_000 {
                    complete = false;
                    break;
                }
                if folder.is_some_and(|value| value != id.to_string()) {
                    continue;
                }
                let listing = match self
                    .cache
                    .read_with_ticket(account, Some(*id), &self.source, false, false)
                    .await
                {
                    Ok(listing) => listing,
                    Err(error) if error.starts_with("INVENTORY_BUSY:") => {
                        complete = false;
                        break;
                    }
                    Err(error) => return Err(error),
                };
                if !listing.cached {
                    complete = false;
                    break;
                }
                complete &= listing.complete;
                let mut candidates = Vec::new();
                for remote in listing.rows.iter() {
                    if rows.len() + candidates.len() >= 100_000 {
                        complete = false;
                        break;
                    }
                    let key = crate::workspace::store::file_key(Some(*id), i64::from(remote.id));
                    let overlays = store.search_overlays(std::slice::from_ref(&key))?;
                    let overlay = overlays.get(&key).ok_or("Missing overlay")?;
                    if overlay.hidden {
                        continue;
                    }
                    candidates.push(crate::workspace::store::WorkspaceFile {
                        key,
                        folder_name: id.to_string(),
                        tags: overlay.tags.clone(),
                        collection_ids: overlay.collections.clone(),
                        file: crate::models::FileMetadata {
                            id: i64::from(remote.id),
                            folder_id: Some(*id),
                            name: remote.name.clone(),
                            size: 1000,
                            mime_type: Some("application/pdf".into()),
                            file_ext: Some("pdf".into()),
                            created_at: chrono::Utc::now().to_rfc3339(),
                            icon_type: "file".into(),
                            encryption_state: "plain".into(),
                            is_favorite: overlay.favorite,
                            is_pinned: overlay.pinned,
                        },
                    });
                }
                complete &= crate::commands::search::append_search_folder(
                    &mut rows,
                    candidates,
                    &store,
                    &mut text_bytes,
                )?;
                listing.ticket.validate(account)?;
                inventory.push(listing.generation);
            }
            Ok(crate::local_search::Snapshot {
                rows,
                folders: self.folders.clone(),
                credential: None,
                complete,
                offline: false,
                inventory,
            })
        })
    }
}

struct ReaderPartial(PathBuf);
impl Drop for ReaderPartial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct FixtureNotificationHost {
    summary: crate::desktop_tray::TraySummaryState,
    tray_sink: std::sync::Mutex<()>,
    language: crate::native_localization::NativeLanguageState,
    root: PathBuf,
    deliveries: std::sync::Mutex<Vec<Value>>,
}

impl crate::desktop_notifications::NotificationHost for FixtureNotificationHost {
    fn language(&self) -> &'static str {
        self.language.get()
    }
    fn preferences(&self) -> crate::desktop_preferences::DesktopPreferences {
        std::fs::read(self.root.join("notification-preferences.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    fn visible_and_focused(&self) -> bool {
        self.root.join("notification-visible").exists()
    }
    fn update_tray(&self, summary: crate::desktop_tray::TransferSummary, revision: u64) {
        let held = summary.active == 1
            && summary.paused == 1
            && self.root.join("notification-tray-hold").exists();
        if held {
            let _ = std::fs::write(self.root.join("notification-tray-entered"), b"entered");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while self.root.join("notification-tray-hold").exists()
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        // This serial sink stands in for the desktop's GUI thread.
        let _sink = self.tray_sink.lock().unwrap();
        if self.summary.update(summary, revision) {
            let _ = crate::desktop_preferences::persist_json_atomically(
                &self.root.join("notification-tray.json"),
                &crate::desktop_tray::project(self.summary.snapshot(), self.language.get()),
            );
        }
        if held {
            let _ = std::fs::write(self.root.join("notification-tray-returned"), b"returned");
        }
    }
    fn deliver(&self, title: String, body: String) -> Result<(), String> {
        let mut delivered = self.deliveries.lock().map_err(error)?;
        delivered.push(json!({"title":title,"body":body}));
        crate::desktop_preferences::persist_json_atomically(
            &self.root.join("notification-deliveries.json"),
            &*delivered,
        )
    }
    fn before_receipt_write(&self, count: usize) {
        if count != 1 || !self.root.join("notification-receipt-hold").exists() {
            return;
        }
        let _ = std::fs::write(self.root.join("notification-receipt-entered"), b"entered");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.root.join("notification-receipt-hold").exists()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    fn flush_complete(&self) {
        let _ = std::fs::write(self.root.join("notification-flush-complete"), b"complete");
    }
    fn after_receipt_write(&self, count: usize) {
        if count == 2 {
            let _ = std::fs::write(
                self.root.join("notification-receipt-second-written"),
                b"written",
            );
        }
    }
    fn pending_drained(&self) {
        if !self.root.join("notification-drain-hold").exists() {
            return;
        }
        let _ = std::fs::write(self.root.join("notification-drain-entered"), b"entered");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.root.join("notification-drain-hold").exists()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

struct FixtureNotifications {
    host: Arc<FixtureNotificationHost>,
    coordinator: Arc<crate::desktop_notifications::DesktopNotificationCoordinator>,
    store: crate::transfer_engine::TransferStore,
    jobs: HashMap<String, crate::transfer_engine::TransferJob>,
}

/// The controlled Telegram boundary records bytes it actually receives and the
/// actual production publication. It never fabricates a prepared thumbnail.
struct FixtureVideoUploadTransport {
    root: PathBuf,
    block_thumbnail: bool,
    block_main_upload: bool,
    fail_thumbnail: bool,
    fail_publish: bool,
    release: tokio::sync::Notify,
    thumbnail_requests: AtomicU64,
    preparation_completed: std::sync::atomic::AtomicBool,
    main_upload_requests: AtomicU64,
    send_attempts: AtomicU64,
    capture: std::sync::Mutex<Value>,
}
impl crate::desktop_video_upload::Transport for FixtureVideoUploadTransport {
    type Receipt = String;
    async fn upload_thumbnail(
        &self,
        mut reader: crate::desktop_video_upload::ThumbnailReader,
        len: usize,
    ) -> Result<grammers_client::types::media::Uploaded, String> {
        use tokio::io::AsyncReadExt;
        self.thumbnail_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.block_thumbnail {
            self.release.notified().await;
        }
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(error)?;
        if bytes.len() != len {
            return Err("Thumbnail transport received incomplete bytes".into());
        }
        std::fs::write(self.root.join("video-upload-thumbnail.jpg"), &bytes).map_err(error)?;
        if self.fail_thumbnail {
            return Err("Controlled Telegram thumbnail upload failure".into());
        }
        Ok(grammers_client::types::media::Uploaded::from_raw(
            grammers_tl_types::types::InputFile {
                id: 202,
                parts: 1,
                name: "video-upload-thumbnail.jpg".into(),
                md5_checksum: "controlled-telegram-thumbnail".into(),
            }
            .into(),
        ))
    }
    async fn publish(
        &self,
        payload: crate::desktop_video_upload::Publication,
    ) -> Result<Self::Receipt, String> {
        use grammers_tl_types::Serializable;
        self.send_attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let raw: grammers_tl_types::enums::InputMedia = payload.media.into();
        let encoded = raw
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        *self.capture.lock().map_err(error)? =
            json!({"serializedTl":encoded,"caption":payload.caption});
        if self.fail_publish {
            return Err("Controlled Telegram publication failure".into());
        }
        Ok("controlled-telegram-message-303".into())
    }
}
struct FixtureVideoUploadJourney {
    transport: Arc<FixtureVideoUploadTransport>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<Value, String>>,
}

struct TransferRetryDriver {
    store: TransferStore,
    jobs: HashMap<String, TransferJob>,
    pending: HashMap<String, TransferJob>,
    prompt_tokens: HashMap<String, u64>,
}

struct Driver {
    sync_app: Option<tauri::App>,
    transfer_retry: Option<TransferRetryDriver>,
    video_upload: Option<FixtureVideoUploadJourney>,
    notifications: Option<FixtureNotifications>,
    proxy_update_task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    reader_tasks: Vec<tokio::task::JoinHandle<Result<u64, String>>>,
    network: Arc<crate::vpn_optimizer::NetworkConfig>,
    bandwidth: Arc<crate::bandwidth::BandwidthManager>,
    bandwidth_hold: Option<crate::bandwidth::BandwidthReservation>,
    search_task: Option<tokio::task::JoinHandle<Result<crate::local_search::Reply, String>>>,
    search_index: Option<crate::local_search::Index>,
    native_preview: Option<crate::workspace::device_cache::Reservation>,
    legacy_pin_task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    asset_downloads: Arc<AtomicU64>,
    asset_task: Option<tokio::task::JoinHandle<Result<String, String>>>,
    inventory_task: Option<tokio::task::JoinHandle<Result<Vec<InventoryRow>, String>>>,
    publication_task:
        Option<tokio::task::JoinHandle<Result<Vec<crate::models::FileMetadata>, String>>>,
    publication_continue: Option<tokio::sync::oneshot::Sender<()>>,
    inventories: Arc<crate::file_inventory::InventoryCache<InventoryRow>>,
    raw_inventory: crate::file_inventory::InventoryCache<RawInventoryRow>,
    inventory_history: Arc<AtomicU64>,
    inventory_calls: Arc<AtomicU64>,
    inventory_lookups: Arc<AtomicU64>,
    inventory_lookup_times: Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
    root: PathBuf,
    crypto: CryptoState,
    vault_tasks: HashMap<String, tokio::task::JoinHandle<Result<Value, String>>>,
    guard: Option<AccountGuard>,
    /// The cycle planned by `sync_cycle`, kept for the following `sync_record`.
    planned_sync: Option<(i64, sync_engine::PreparedReconciliation)>,
    archive_task:
        Option<tokio::task::JoinHandle<Result<crate::commands::archive::ArchiveStaging, String>>>,
    archive_continue: Option<tokio::sync::oneshot::Sender<()>>,
    store_task: Option<tokio::task::JoinHandle<Result<(), String>>>,
    store_continue: Option<std::sync::mpsc::Sender<()>>,
    scan_peers: Arc<RwLock<HashMap<i64, String>>>,
    scan_task: Option<tokio::task::JoinHandle<Result<usize, String>>>,
    scan_continue: Option<tokio::sync::oneshot::Sender<()>>,
    servers: Vec<(
        actix_web::dev::ServerHandle,
        tokio::task::JoinHandle<std::io::Result<()>>,
    )>,
    retained_sessions: Vec<Arc<TelegramState>>,
}

impl Driver {
    fn sync_app(&mut self) -> Result<&tauri::App, String> {
        if self.sync_app.is_none() {
            let mut context = crate::application_context();
            context.config_mut().app.windows.clear();
            context.config_mut().app.tray_icon = None;
            let app = tauri::Builder::default()
                .enable_macos_default_menu(false)
                .build(context)
                .map_err(error)?;
            let db = crate::db::init_db_at(&self.root)?;
            app.manage(disconnected().as_ref().clone());
            app.manage(db.clone());
            app.manage(self.crypto.clone());
            app.manage(Arc::new(NativeSyncFixture::new(self.root.clone())));
            app.manage(SyncEngine::new(db, app.handle().clone()));
            NativeSyncFixture::install_listener(app.handle());
            self.sync_app = Some(app);
        }
        Ok(self.sync_app.as_ref().unwrap())
    }

    async fn dispatch_sync_lifecycle(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "sync_seed" => {
                let owner = text(&request, "owner")?.to_string();
                let scanner = request["scanner"].as_str().unwrap_or("full").to_string();
                if !matches!(scanner.as_str(), "full" | "incremental") {
                    return Err("Unknown fixture scanner".into());
                }
                let db = crate::db::init_db_at(&self.root)?;
                let root = self.root.clone();
                crate::db::with_connection(db.clone(), move |connection| {
                    connection.execute(concat!(
                        "INSERT OR REPLACE INTO sync_settings(key,value) VALUES",
                        "('sync_enabled','true'),('sync_debounce_ms','250')"
                    )).map_err(error)?;
                    let mut scanner_setting = connection.prepare(
                        "INSERT OR REPLACE INTO sync_settings(key,value) VALUES('sync_scanner',?)"
                    ).map_err(error)?;
                    scanner_setting.bind((1,scanner.as_str())).map_err(error)?;
                    scanner_setting.next().map_err(error)?;
                    for id in 1..=3 {
                        let path = root.join(format!("mapping-{id}"));
                        std::fs::create_dir_all(&path).map_err(error)?;
                        let mut insert = connection.prepare(concat!(
                            "INSERT OR IGNORE INTO sync_pairs(",
                            "id,local_path,channel_id,folder_key,label,sync_direction,is_active,created_at)",
                            " VALUES(?,?,?,?,?,'bidirectional',?,1)"
                        )).map_err(error)?;
                        insert.bind((1,id)).map_err(error)?;
                        insert.bind((2,path.to_str().ok_or("Invalid fixture path")?)).map_err(error)?;
                        insert.bind((3,100+id)).map_err(error)?;
                        insert.bind((4,format!("folder-{id}").as_str())).map_err(error)?;
                        insert.bind((5,format!("Mapping {id}").as_str())).map_err(error)?;
                        insert.bind((6,i64::from(id != 3))).map_err(error)?;
                        insert.next().map_err(error)?;
                        let policy = StoredPairPolicy {
                            account_owner:Some(owner.clone()),
                            ..Default::default()
                        };
                        sync_config::write_pair_policy(connection,id,&policy)?;
                    }
                    let data = b"Keep the existing synced file across startup and reconnect.\n";
                    std::fs::write(root.join("mapping-1/keep.txt"),data).map_err(error)?;
                    let hash = format!("{:x}",Sha256::digest(data));
                    let mut insert = connection.prepare(concat!(
                        "INSERT OR REPLACE INTO sync_state(",
                        "pair_id,relative_path,local_hash,remote_hash,file_size,",
                        "local_mtime,remote_date,message_id,sync_status)",
                        " VALUES(1,'keep.txt',?,?,?,1,1,42,'synced')"
                    )).map_err(error)?;
                    insert.bind((1,hash.as_str())).map_err(error)?;
                    insert.bind((2,hash.as_str())).map_err(error)?;
                    insert.bind((3,data.len() as i64)).map_err(error)?;
                    insert.next().map_err(error)?;
                    let remote = json!({"1":{"keep.txt":{
                        "relativePath":"keep.txt","hash":hash,"fileSize":data.len(),
                        "modifiedAt":1,"messageId":42
                    }}});
                    std::fs::write(
                        root.join("sync-fixture-remote.json"),
                        serde_json::to_vec(&remote).map_err(error)?
                    ).map_err(error)?;
                    Ok(())
                }).await?;
                Ok(json!(true))
            }
            "sync_start" => {
                let app = self.sync_app()?;
                if let Some(owner) = request["readyOwner"].as_i64() {
                    app.state::<Arc<NativeSyncFixture>>().set_owner(Some(owner));
                    let readiness = &app.state::<TelegramState>().sync_readiness;
                    readiness.mark_ready(readiness.generation(), owner);
                }
                app.state::<SyncEngine>().start().await?;
                let settings =
                    sync_config::load_settings(app.state::<SyncEngine>().db.clone()).await?;
                Ok(json!({"scanner":settings.scanner}))
            }
            "sync_status" => {
                let app = self.sync_app()?;
                app.state::<Arc<NativeSyncFixture>>()
                    .snapshot(app.handle(), request["waitFor"].as_str())
                    .await
            }
            "sync_telegram" => {
                let app = self.sync_app()?;
                let fixture = app.state::<Arc<NativeSyncFixture>>();
                let readiness = &app.state::<TelegramState>().sync_readiness;
                let owner = request["owner"].as_i64();
                fixture.set_owner(owner);
                let Some(owner) = owner else {
                    return Ok(json!({"generation":readiness.invalidate()}));
                };
                let accepted = request["silent"] != true
                    && readiness.mark_ready(readiness.generation(), owner);
                Ok(json!({"accepted":accepted,"generation":readiness.generation()}))
            }
            "sync_ready_reply" => {
                let app = self.sync_app()?;
                let readiness = &app.state::<TelegramState>().sync_readiness;
                let generation = request["generation"]
                    .as_u64()
                    .ok_or("Missing reply generation")?;
                let owner = request["owner"].as_i64().ok_or("Missing reply owner")?;
                Ok(json!({"accepted":readiness.mark_ready(generation,owner)}))
            }
            "sync_wake" => {
                self.sync_app()?.emit("vault-unlocked", ()).map_err(error)?;
                Ok(json!(true))
            }
            "sync_seed_cleanup" => {
                let db = crate::db::init_db_at(&self.root)?;
                sync_config::save_pending_cleanup(
                    db,
                    2,
                    &[sync_config::PendingCleanup {
                        relative_path: "replacement.txt".into(),
                        old_message_id: 41,
                        new_message_id: 42,
                        old_remote_hash: Some("preserve-the-existing-remote-version".into()),
                    }],
                )
                .await?;
                Ok(json!(true))
            }
            "sync_pending_cleanup" => {
                let db = crate::db::init_db_at(&self.root)?;
                serde_json::to_value(sync_config::load_pending_cleanup(db, 2).await?).map_err(error)
            }
            "sync_stop" => {
                self.sync_app()?
                    .state::<SyncEngine>()
                    .shutdown_and_wait()
                    .await?;
                Ok(json!(true))
            }
            _ => Err("Unknown sync lifecycle fixture command".into()),
        }
    }

    fn transfer_retry(&mut self) -> Result<&mut TransferRetryDriver, String> {
        if self.transfer_retry.is_none() {
            let (store, jobs) = TransferStore::open(&self.root.join("transfers.db"))?;
            self.transfer_retry = Some(TransferRetryDriver {
                store,
                jobs: jobs.into_iter().map(|job| (job.id.clone(), job)).collect(),
                pending: HashMap::new(),
                prompt_tokens: HashMap::new(),
            });
        }
        Ok(self.transfer_retry.as_mut().unwrap())
    }

    async fn dispatch(&mut self, request: Value) -> Result<Value, String> {
        let command = text(&request, "command")?.to_owned();
        match command.as_str() {
            "sync_seed"
            | "sync_start"
            | "sync_status"
            | "sync_telegram"
            | "sync_ready_reply"
            | "sync_wake"
            | "sync_seed_cleanup"
            | "sync_pending_cleanup"
            | "sync_stop" => self.dispatch_sync_lifecycle(request).await,
            "notifications_start"
            | "notification_language"
            | "notification_tray"
            | "notification_transition"
            | "notification_replay"
            | "notification_deliveries" => self.dispatch_notifications(request).await,
            "workspace_suspend"
            | "workspace_resume"
            | "workspace_seed_files"
            | "workspace_page"
            | "workspace_marker"
            | "workspace_tx_start"
            | "workspace_tx_read"
            | "workspace_tx_finish"
            | "database_mode"
            | "database"
            | "seed_account"
            | "account_validation_status"
            | "capture_account"
            | "validate_account"
            | "save_collection"
            | "workspace" => self.dispatch_workspace(request).await,
            "archive_zip_fixture"
            | "archive_stage_start"
            | "archive_stage_cancel"
            | "archive_stage_finish"
            | "archive_delete_fixture"
            | "archive_stage_fixture" => self.dispatch_archives(request).await,
            "publication_start"
            | "publication_finish"
            | "listing_rename"
            | "inventory_upload_published"
            | "storage_insight_inventory"
            | "external_file_start"
            | "external_file_forget"
            | "external_file_identity_mode"
            | "external_file_old_registration"
            | "external_file_identity_status"
            | "external_file_status"
            | "external_file_open"
            | "proxy_transport_fixture"
            | "grammers_proxy_connect"
            | "inventory_refresh_overlap"
            | "inventory_detection_bound"
            | "inventory_audit_rounds"
            | "inventory_raw_list"
            | "inventory_list"
            | "inventory_parallel"
            | "inventory_waiter"
            | "inventory_touch"
            | "inventory_invalidate"
            | "inventory_watch_start"
            | "inventory_watch_finish"
            | "inventory_timer_lifecycle"
            | "inventory_deadline"
            | "legacy_inventory_write" => self.dispatch_inventory(request).await,
            "heic_pin_gate"
            | "heic_pin_start"
            | "heic_expected_pixels"
            | "asset_display_local"
            | "heic_status"
            | "asset_read"
            | "asset_display_read"
            | "asset_start"
            | "workspace_asset_read"
            | "native_preview_prepare"
            | "native_preview_finish"
            | "native_preview_clear"
            | "asset_pin"
            | "asset_cached"
            | "asset_delete"
            | "api_thumbnail_seed"
            | "asset_clear_all"
            | "asset_status"
            | "asset_offline_seed"
            | "legacy_external_seed"
            | "legacy_external_resolve"
            | "legacy_external_resolve_start"
            | "legacy_external_clear_start"
            | "legacy_external_status"
            | "asset_offline_read"
            | "asset_metadata_finished"
            | "asset_metadata_started"
            | "asset_core_busy"
            | "legacy_pin_start"
            | "legacy_pin_finish"
            | "reader_copy_start"
            | "reader_copy_cancel"
            | "reader_copy_finish"
            | "asset_limits"
            | "asset_abort"
            | "asset_finish"
            | "asset_cancel"
            | "asset_clear"
            | "thumbnail_batch" => self.dispatch_assets(request).await,
            "traffic_clock"
            | "traffic_proxy_start_slow"
            | "traffic_proxy_finish_slow"
            | "traffic_proxy_patch"
            | "traffic_proxy_matches"
            | "traffic_pending"
            | "traffic_snapshot"
            | "traffic_configure"
            | "bandwidth_read"
            | "bandwidth_set_limit"
            | "bandwidth_set_date"
            | "bandwidth_resize"
            | "bandwidth_commit_other"
            | "bandwidth_hold"
            | "bandwidth_commit"
            | "bandwidth_cancel" => self.dispatch_traffic(request).await,
            "search_inventory"
            | "search_runtime"
            | "search_start"
            | "search_abort"
            | "search_available"
            | "search_tag"
            | "search_assign"
            | "search_inventory_changed"
            | "search_record"
            | "search_index_build"
            | "search_index_query"
            | "search_index_saved" => self.dispatch_search(request).await,
            "webdav_upload_fixture"
            | "start_media_fixture"
            | "start_ad_server"
            | "stop_ad_server"
            | "sponsor_load_verified"
            | "start_stream_server"
            | "api_seed_catalog"
            | "supporter_verify"
            | "start_webdav"
            | "release_retained_sessions"
            | "start_api"
            | "start_http" => self.dispatch_servers(request).await,
            "peer_queued_clear" | "peer_scan_seed" | "peer_scan_start" | "peer_scan_read"
            | "peer_scan_finish" => self.dispatch_peers(request).await,
            "vault_cleanup_fault"
            | "vault_prepare_start"
            | "vault_prepare_abort"
            | "vault_prepare_finish"
            | "vault_status_responsive"
            | "vault_create"
            | "vault_unlock"
            | "vault_auto_timeout"
            | "vault_auto_due"
            | "vault_lock"
            | "vault_change_passphrase"
            | "vault_export"
            | "vault_recover"
            | "vault_verify_recovery"
            | "vault_identity"
            | "vault_save_profile"
            | "envelope_known_answer"
            | "fixture_encrypt"
            | "read_envelope" => self.dispatch_crypto(request).await,
            "folder_create" | "folder_delete" | "folder_rename" | "folder_layout"
            | "folder_scan" | "group_create" | "folder_assign" | "folder_sign_out"
            | "folder_rows" | "seed_folder_row" => self.dispatch_folders(request).await,
            "staging_root" | "log_start" | "log_emit" | "startup_failure_message" => {
                self.dispatch_diagnostics(request).await
            }
            "sync_seed_pair"
            | "sync_set_preferences"
            | "sync_cycle"
            | "sync_record"
            | "sync_state" => self.dispatch_sync(request).await,
            "video_upload_prepare"
            | "video_upload_start"
            | "video_upload_observe"
            | "video_upload_release"
            | "video_upload_cancel"
            | "video_upload_finish"
            | "upload_resumable"
            | "upload_session"
            | "upload_assemble"
            | "publish_download"
            | "save_transfer"
            | "transfers"
            | "retry_failed_upload"
            | "transfer_retry_snapshot"
            | "record_transfer_result"
            | "fail_transfer"
            | "remove_transfer" => self.dispatch_transfers(request).await,
            _ => Err("Unknown native E2E command".into()),
        }
    }

    async fn dispatch_notifications(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "notifications_start" => {
                let owner = crate::workspace::current_owner(&self.root)?.to_string();
                let (store, jobs) =
                    crate::transfer_engine::TransferStore::open(&self.root.join("transfers.db"))?;
                let jobs: Vec<_> = jobs
                    .into_iter()
                    .filter(|job| job.owner_id.as_deref() == Some(owner.as_str()))
                    .collect();
                let delivered = std::fs::read(self.root.join("notification-deliveries.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .unwrap_or_default();
                let host = Arc::new(FixtureNotificationHost {
                    summary: Default::default(),
                    tray_sink: std::sync::Mutex::new(()),
                    language: crate::native_localization::NativeLanguageState::load(
                        &self.root,
                        request["systemLanguage"].as_str(),
                    ),
                    root: self.root.clone(),
                    deliveries: std::sync::Mutex::new(delivered),
                });
                let coordinator =
                    crate::desktop_notifications::DesktopNotificationCoordinator::with_host(
                        host.clone(),
                        self.root.join("desktop-notification-receipts.v1.json"),
                    );
                coordinator.seed(jobs.clone());
                self.notifications = Some(FixtureNotifications {
                    host,
                    coordinator,
                    store,
                    jobs: jobs.into_iter().map(|job| (job.id.clone(), job)).collect(),
                });
                Ok(json!(true))
            }
            "notification_language" => {
                let notifications = self
                    .notifications
                    .as_ref()
                    .ok_or("Notifications are not started")?;
                notifications
                    .host
                    .language
                    .set(text(&request, "language")?)?;
                notifications.coordinator.refresh_tray();
                Ok(json!(true))
            }
            "notification_tray" => serde_json::from_slice(
                &std::fs::read(self.root.join("notification-tray.json")).map_err(error)?,
            )
            .map_err(error),
            "notification_transition" => {
                let notifications = self
                    .notifications
                    .as_mut()
                    .ok_or("Notifications are not started")?;
                let mut job = notifications
                    .jobs
                    .get(text(&request, "id")?)
                    .cloned()
                    .ok_or("Unknown notification transfer")?;
                AccountGuard::open(&self.root, job.owner_id.as_deref())?.validate()?;
                if let Some(reason) = request["error"].as_str() {
                    crate::transfer_engine::apply_failure(
                        &mut job,
                        reason.to_owned(),
                        chrono::Utc::now().timestamp_millis(),
                    );
                } else {
                    job.status =
                        serde_json::from_value(request["status"].clone()).map_err(error)?;
                    if job.status == crate::transfer_engine::TransferStatus::Completed {
                        job.progress = 100;
                        job.transferred_bytes = job.total_bytes;
                        job.speed_bytes_per_sec = 0;
                    }
                }
                if let Some(origin) = request["origin"].as_str() {
                    job.origin = Some(origin.to_owned());
                }
                job.revision = job.revision.saturating_add(1);
                notifications.store.upsert(&job).await?;
                notifications.jobs.insert(job.id.clone(), job.clone());
                if request["background"].as_bool().unwrap_or(false) {
                    let coordinator = notifications.coordinator.clone();
                    let event = job.clone();
                    tauri::async_runtime::spawn_blocking(move || coordinator.record(event));
                } else {
                    notifications.coordinator.record(job.clone());
                }
                serde_json::to_value(job).map_err(error)
            }
            "notification_replay" => {
                let notifications = self
                    .notifications
                    .as_ref()
                    .ok_or("Notifications are not started")?;
                let job = notifications
                    .jobs
                    .get(text(&request, "id")?)
                    .cloned()
                    .ok_or("Unknown notification transfer")?;
                let mut previous = job.clone();
                previous.status = crate::transfer_engine::TransferStatus::Paused;
                notifications.coordinator.seed(vec![previous]);
                notifications.coordinator.record(job);
                Ok(json!(true))
            }
            "notification_deliveries" => Ok(std::fs::read(
                self.root.join("notification-deliveries.json"),
            )
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(json!([]))),
            _ => Err("Unknown notifications E2E command".into()),
        }
    }

    async fn dispatch_workspace(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "workspace_suspend" => {
                crate::workspace::suspend();
                Ok(json!(true))
            }
            "workspace_resume" => {
                crate::workspace::resume();
                Ok(json!(true))
            }
            "workspace_seed_files" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let store = Store::open(&self.root, account.owner)?;
                let files: Vec<crate::models::FileMetadata> = serde_json::from_value(request["files"].clone()).map_err(error)?;
                for file in files { account.validate()?; store.remember_local_file(&file)?; }
                if let Some(hidden) = request["hidden"].as_array() {
                    for key in hidden {
                        let key = key.as_str().ok_or("Invalid hidden key")?;
                        store.put_record("removal", key, &json!({"key":key,"status":"pending"}))?;
                    }
                }
                if let Some(tagged) = request["tagged"].as_str() { store.tag(&[tagged.into()], "page-tag", true)?; }
                account.validate()?;
                Ok(json!(true))
            }
            "workspace_page" => {
                let page = crate::workspace::read_page_at(
                    &self.root,
                    text(&request, "owner")?,
                    request["cursor"].as_str().map(str::to_string),
                    request["limit"].as_u64().unwrap_or(256) as usize,
                )?;
                serde_json::to_value(page).map_err(error)
            }
            "workspace_marker" => {
                let root = if let Some(name) = request["root"].as_str() { child(&self.root, name)? } else { self.root.clone() };
                let store = Store::open(&root, request["owner"].as_i64().ok_or("Missing owner")?)?;
                if request["create"].as_bool().unwrap_or(false) {
                    store.db.execute("CREATE TEMP TABLE connection_marker(value TEXT); INSERT INTO connection_marker VALUES('lease')").map_err(error)?;
                    Ok(json!(true))
                } else {
                    let mut query = store.db.prepare("SELECT value FROM connection_marker").map_err(error)?;
                    query.next().map_err(error)?;
                    Ok(json!(query.read::<String,_>(0).map_err(error)?))
                }
            }
            "workspace_tx_start" => {
                if self.store_task.is_some() { return Err("Transaction already running".into()); }
                let root = self.root.clone();
                let (ready_send, ready) = tokio::sync::oneshot::channel();
                let (resume, continued) = std::sync::mpsc::channel();
                let fail = request["fail"].as_bool().unwrap_or(false);
                let metadata = request["metadata"].as_bool().unwrap_or(false);
                self.store_continue = Some(resume);
                self.store_task = Some(tokio::task::spawn_blocking(move || {
                    let store = Store::open(&root, 101)?;
                    store.transaction(|| {
                        if metadata { store.put_record("favorite", "42:1", &true)?; }
                        else { store.put_record("lease-test", "first", &1)?; }
                        let _ = ready_send.send(());
                        continued.recv().map_err(error)?;
                        if fail { return Err("Transaction interrupted".into()); }
                        store.put_record("lease-test", "second", &2)
                    })
                }));
                ready.await.map_err(error)?;
                Ok(json!(true))
            }
            "workspace_tx_read" => {
                let store = Store::open(&self.root, 101)?;
                Ok(json!([store.record::<u64>("lease-test", "first")?, store.record::<u64>("lease-test", "second")?]))
            }
            "workspace_tx_finish" => {
                self.store_continue.take().ok_or("No transaction")?.send(()).map_err(error)?;
                self.store_task.take().ok_or("No transaction")?.await.map_err(error)??;
                Ok(json!(true))
            }
            "database_mode" => {
                crate::db::with_connection(crate::db::init_db_at(&self.root)?, |connection| {
                    let mut mode = connection.prepare("PRAGMA journal_mode").map_err(error)?;
                    mode.next().map_err(error)?;
                    let mut busy = connection.prepare("PRAGMA busy_timeout").map_err(error)?;
                    busy.next().map_err(error)?;
                    Ok(json!({"journal":mode.read::<String,_>(0).map_err(error)?, "busyMs":busy.read::<i64,_>(0).map_err(error)?}))
                }).await
            }
            "database" => {
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, |connection| {
                    let mut query = connection
                        .prepare("SELECT MAX(version) FROM app_schema_migrations")
                        .map_err(error)?;
                    query.next().map_err(error)?;
                    Ok(json!({"version": query.read::<i64,_>(0).map_err(error)?}))
                })
                .await
            }
            "account_validation_status" => {
                let counts = crate::workspace::test_account_validation_counts();
                Ok(json!({"fallbackReads":counts.0,"validations":counts.1}))
            },
            "seed_account" => {
                let owner = request["owner"]
                    .as_i64()
                    .filter(|value| *value > 0)
                    .ok_or("Invalid fixture owner")?;
                let root=if let Some(root)=request["root"].as_str() {child(&self.root,root)?} else {self.root.clone()};
                if let Ok(previous) = crate::workspace::current_owner(&root) {
                    if previous != owner { crate::external_files::invalidate_account(&root, previous)?; }
                }
                let path = root.join("telegram.session");
                let staged = root.join(format!(".fixture-session-{}.sqlite", uuid::Uuid::new_v4()));
                let prepared = (|| {
                    // Populate a new synthetic login privately; owner guards
                    // can keep reading the previous account until replacement.
                    let session = SqliteSession::open(&staged).map_err(error)?;
                    session.cache_peer(&PeerInfo::User {id:owner,auth:None,bot:Some(false),is_self:Some(true)});
                    drop(session);
                    crate::desktop_preferences::atomic_replace(&staged,&path).map_err(error)?;
                    // Fence any registered old session before reporting the new owner.
                    let session=crate::workspace::open_session(&root).map_err(error)?;
                    crate::workspace::register_session(&root,&session)?;
                    Ok::<(),String>(())
                })();
                if prepared.is_err() {let _ = std::fs::remove_file(&staged);}
                prepared?;
                Ok(json!({"owner": crate::workspace::current_owner(&root)?}))
            }
            "capture_account" => {
                self.guard = Some(AccountGuard::open(&self.root, request["owner"].as_str())?);
                Ok(json!({"owner": self.guard.as_ref().unwrap().owner}))
            }
            "validate_account" => {
                self.guard
                    .as_ref()
                    .ok_or("No captured account")?
                    .validate()?;
                Ok(json!(true))
            }
            "save_collection" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let store = Store::open(&self.root, account.owner)?;
                store.save_collection(
                    &serde_json::from_value(request["collection"].clone()).map_err(error)?,
                )?;
                account.validate()?;
                serde_json::to_value(store.snapshot()?).map_err(error)
            }
            "workspace" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let snapshot = Store::open(&self.root, account.owner)?.snapshot()?;
                account.validate()?;
                serde_json::to_value(snapshot).map_err(error)
            }
            _ => Err("Unknown workspace E2E command".into()),
        }
    }

    async fn dispatch_archives(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "archive_zip_fixture" => {
                let source = child(&self.root, text(&request, "source")?)?;
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let setup = crate::commands::archive::ArchiveDownload {
                    root: crate::temp_artifacts::staging_root_in(&self.root).map_err(error)?,
                    max_bytes: 10_000_000,
                    expected_bytes: Some(std::fs::metadata(&source).map_err(error)?.len()),
                    account: Some(account),
                };
                serde_json::to_value(
                    crate::commands::archive::zip_from_chunks(
                        archive_fixture_chunks(source),
                        setup,
                        request["entry"].as_u64().map(|index| index as usize),
                        "fixture.zip".into(),
                    )
                    .await?,
                )
                .map_err(error)
            }
            "archive_stage_start" => {
                if self.archive_task.is_some() {
                    return Err("Archive staging already running".into());
                }
                let account = AccountGuard::open(&self.root, None)?;
                let setup = crate::commands::archive::ArchiveDownload {
                    root: crate::temp_artifacts::staging_root_in(&self.root).map_err(error)?,
                    max_bytes: 1_000_000,
                    expected_bytes: None,
                    account: Some(account),
                };
                let (ready_send, ready) = tokio::sync::oneshot::channel();
                let (resume, continued) = tokio::sync::oneshot::channel();
                self.archive_continue = Some(resume);
                self.archive_task = Some(tokio::spawn(async move {
                    let chunks = async_stream::stream! { yield Ok(bytes::Bytes::from(vec![5;8192])); let _=ready_send.send(()); let _=continued.await; yield Ok(bytes::Bytes::from(vec![6;8192])); };
                    crate::commands::archive::stage_archive_chunks(chunks, &setup, "zip").await
                }));
                ready.await.map_err(error)?;
                Ok(json!(true))
            }
            "archive_stage_cancel" => {
                let task = self.archive_task.take().ok_or("No archive staging")?;
                task.abort();
                match task.await {
                    Err(error) if error.is_cancelled() => {}
                    _ => return Err("Archive task did not cancel".into()),
                };
                self.archive_continue = None;
                Ok(json!(true))
            }
            "archive_stage_finish" => {
                self.archive_continue
                    .take()
                    .ok_or("No archive staging")?
                    .send(())
                    .map_err(|_| "Archive stopped")?;
                let staged = self
                    .archive_task
                    .take()
                    .ok_or("No archive staging")?
                    .await
                    .map_err(error)??;
                drop(staged);
                Ok(json!(true))
            }
            "archive_delete_fixture" => {
                let path = PathBuf::from(text(&request, "path")?);
                if path.parent()
                    != Some(
                        crate::temp_artifacts::staging_root_in(&self.root)
                            .map_err(error)?
                            .as_path(),
                    )
                {
                    return Err("Outside fixture staging".into());
                }
                crate::temp_artifacts::delete_registered(&path)?;
                Ok(json!(true))
            }
            "archive_stage_fixture" => {
                let source = child(&self.root, text(&request, "source")?)?;
                let fail = request["fail"].as_bool().unwrap_or(false);
                let staging = crate::temp_artifacts::staging_root_in(&self.root).map_err(error)?;
                let chunks = async_stream::stream! {
                    use tokio::io::AsyncReadExt;
                    let mut file = match tokio::fs::File::open(source).await {
                        Ok(file) => file,
                        Err(error) => { yield Err(error.to_string()); return; }
                    };
                    loop {
                        let mut bytes = vec![0;8192];
                        match file.read(&mut bytes).await {
                            Ok(0) => break,
                            Ok(count) => { bytes.truncate(count); yield Ok(bytes::Bytes::from(bytes)); }
                            Err(error) => { yield Err(error.to_string()); return; }
                        }
                    }
                    if fail { yield Err("Archive transport interrupted".to_string()); }
                };
                let setup = crate::commands::archive::ArchiveDownload {
                    root: staging,
                    max_bytes: request["maxBytes"].as_u64().unwrap_or(1_000_000),
                    expected_bytes: None,
                    account: None,
                };
                let staged =
                    crate::commands::archive::stage_archive_chunks(chunks, &setup, "zip").await?;
                let length = std::fs::metadata(&staged.archive_path)
                    .map_err(error)?
                    .len();
                Ok(json!(length))
            }
            _ => Err("Unknown archives E2E command".into()),
        }
    }

    async fn dispatch_inventory(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "publication_start" => {
                if self.publication_task.is_some() {
                    return Err("Publication already running".into());
                }
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let file: crate::models::FileMetadata =
                    serde_json::from_value(request["file"].clone()).map_err(error)?;
                let publication = crate::file_inventory::Publication {
                    ticket: crate::file_inventory::RevisionTicket::capture(&account),
                    credential: self.crypto.current_session(),
                    account,
                };
                let crypto = self.crypto.clone();
                let (proceed, gate) = tokio::sync::oneshot::channel();
                let (ready_send, ready) = tokio::sync::oneshot::channel();
                self.publication_continue = Some(proceed);
                self.publication_task =
                    Some(tokio::spawn(async move {
                        let _ = ready_send.send(());
                        let _ = gate.await;
                        let files = vec![file];
                        let saved = files.clone();
                        let scope = publication.clone();
                        tokio::task::spawn_blocking(move || {
                            scope.persist(|| {
                                Store::open(&scope.account.root, scope.account.owner)?
                                    .remember_files(&saved, "Saved Messages", "publication-fixture")
                            })
                        })
                        .await
                        .map_err(error)??;
                        publication.output(&crypto, &files, || Ok(files.clone()))
                    }));
                ready.await.map_err(error)?;
                Ok(json!(true))
            }
            "publication_finish" => {
                self.publication_continue
                    .take()
                    .ok_or("No publication")?
                    .send(())
                    .map_err(|_| "Publication canceled")?;
                serde_json::to_value(
                    self.publication_task
                        .take()
                        .ok_or("No publication")?
                        .await
                        .map_err(error)??,
                )
                .map_err(error)
            }
            "listing_rename" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                crate::workspace::remote_changes::record(
                    &account,
                    vec![crate::workspace::remote_changes::Change::Rename {
                        folder: request["folder"].as_i64(),
                        message: 1,
                        name: text(&request, "name")?.into(),
                    }],
                )
                .await?;
                Ok(json!(true))
            }
            "inventory_upload_published" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                crate::file_inventory::changed(&account, request["folder"].as_i64(), &[])?;
                Ok(json!(true))
            }
            "external_file_forget" => {
                let account = AccountGuard::open(&self.root, None)?;
                let path = produced_fixture_path(&self.root, text(&request, "path")?)?;
                use sha2::{Digest, Sha256};
                let key = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
                Store::open(&account.root, account.owner)?
                    .remove_record("external-produced-v1", &key)?;
                Ok(json!(true))
            }
            "external_file_old_registration" => {
                let account = AccountGuard::open(&self.root, None)?;
                let path = produced_fixture_path(&self.root, text(&request, "path")?)?;
                let key = format!(
                    "{:x}",
                    sha2::Sha256::digest(path.to_string_lossy().as_bytes())
                );
                let store = Store::open(&self.root, account.owner)?;
                let mut record = store
                    .record::<Value>("external-produced-v1", &key)?
                    .ok_or("No registration")?;
                for name in ["reuse", "access"] {
                    record
                        .as_object_mut()
                        .ok_or("Bad registration")?
                        .remove(name);
                }
                for name in ["stable", "changed_nanos"] {
                    record["identity"]
                        .as_object_mut()
                        .ok_or("Bad identity")?
                        .remove(name);
                }
                store.put_record("external-produced-v1", &key, &record)?;
                Ok(json!(true))
            }
            "external_file_identity_status" => {
                let path = produced_fixture_path(&self.root, text(&request, "path")?)?;
                Ok(
                    json!({"eligible":crate::external_files::test_reuse_supported(&path)?,"bytes":std::fs::metadata(&path).map_err(error)?.len()}),
                )
            }
            "external_file_identity_mode" => {
                let path = produced_fixture_path(&self.root, text(&request, "path")?)?;
                crate::external_files::test_unstable_identity(path, request["unstable"] == true);
                Ok(json!(true))
            }
            "external_file_status" => Ok(json!(crate::external_files::test_status())),
            "external_file_start" => {
                let id = text(&request, "id")?.to_string();
                let account = AccountGuard::open(&self.root, None)?;
                let path = produced_fixture_path(&self.root, text(&request, "path")?)?;
                let started = child(&self.root, &format!("file-{id}.started"))?;
                let release = child(&self.root, &format!("file-{id}.release"))?;
                let post_hash = request["postHash"].as_bool().unwrap_or(false);
                let reuse_cache = request["reuseCache"].as_bool().unwrap_or(false);
                let hold = request["hold"].as_bool().unwrap_or(true);
                let legacy = request["legacy"].as_bool().unwrap_or(false);
                let cache = self.root.join("asset-cache");
                if !hold {
                    // Start a second real worker without installing another global gate.
                } else if post_hash {
                    crate::external_files::test_hold_validated(started.clone(), release);
                } else {
                    crate::external_files::test_hold(started.clone(), release);
                }
                self.vault_tasks.insert(
                    id,
                    tokio::spawn(async move {
                        if reuse_cache {
                            return crate::external_files::reuse_cached(account, path)
                                .await
                                .map(|value| json!(value));
                        }
                        let file = if legacy {
                            crate::external_files::open_async(account.clone(), cache, path).await?
                        } else {
                            crate::external_files::validate_async(account.clone(), path).await?
                        };
                        if post_hash {
                            file.checked(&account)?;
                        }
                        Ok(json!(true))
                    }),
                );
                let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                while hold && !started.is_file() {
                    if tokio::time::Instant::now() >= deadline {
                        return Err("File worker did not start".into());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(json!(true))
            }
            "external_file_open" => {
                let account = AccountGuard::open(&self.root, None)?;
                let authenticated = crate::external_files::open_async(
                    account.clone(),
                    request["cacheRoot"]
                        .as_str()
                        .map(|path| child(&self.root, path).map(|root| root.join("asset-cache")))
                        .transpose()?
                        .unwrap_or_else(|| self.root.join("asset-cache")),
                    produced_fixture_path(&self.root, text(&request, "path")?)?,
                )
                .await?;
                let path = authenticated.checked(&account)?;
                std::fs::write(
                    self.root.join("opened-path"),
                    path.to_string_lossy().as_bytes(),
                )
                .map_err(error)?;
                Ok(json!(true))
            }
            "grammers_proxy_connect" => {
                let address: std::net::SocketAddr = text(&request, "address")?
                    .parse()
                    .map_err(|_| "Invalid proxy fixture target address")?;
                if !address.ip().is_loopback() {
                    return Err("Proxy fixture targets must be loopback addresses".into());
                }
                let proxy = text(&request, "proxy")?.to_string();
                let parsed =
                    reqwest::Url::parse(&proxy).map_err(|_| "Invalid proxy fixture URL")?;
                let local_proxy = parsed.host_str().is_some_and(|host| {
                    host == "localhost"
                        || host.ends_with(".localhost")
                        || host
                            .trim_start_matches('[')
                            .trim_end_matches(']')
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|address| address.is_loopback())
                });
                if !local_proxy {
                    return Err("Proxy fixture endpoints must be loopback addresses".into());
                }
                // Exercise the same Grammers transport as production without
                // contacting Telegram or starting an authorization handshake.
                let connected = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    grammers_mtsender::Sender::connect(
                        grammers_mtproto::transport::Full::new(),
                        grammers_mtproto::mtp::Plain::new(),
                        grammers_mtsender::ServerAddr::Proxied { address, proxy },
                    ),
                )
                .await;
                Ok(match connected {
                    Ok(Ok(sender)) => {
                        drop(sender);
                        json!({"connected":true})
                    }
                    Ok(Err(error)) => json!({
                        "connected":false,
                        "error":error.to_string(),
                        "kind":format!("{:?}", error.kind()),
                    }),
                    Err(_) => json!({"connected":false,"error":"Proxy connection timed out"}),
                })
            }
            "proxy_transport_fixture" => {
                let url = text(&request, "url")?.to_string();
                let works = crate::commands::network::proxy_transport_works(async move {
                    let response = reqwest::get(url).await.map_err(|error| {
                        grammers_mtsender::InvocationError::Io(std::io::Error::other(error))
                    })?;
                    let body: Value = response.json().await.map_err(|error| {
                        grammers_mtsender::InvocationError::Io(std::io::Error::other(error))
                    })?;
                    if let Some(rpc) = body["rpc"].as_str() {
                        Err::<(), _>(grammers_mtsender::InvocationError::Rpc(
                            grammers_mtsender::RpcError {
                                code: 400,
                                name: rpc.into(),
                                value: None,
                                caused_by: None,
                            },
                        ))
                    } else {
                        Err(grammers_mtsender::InvocationError::Io(
                            std::io::Error::other("Malformed fixture transport"),
                        ))
                    }
                })
                .await;
                Ok(json!({"transportWorks":works}))
            }
            "storage_insight_inventory" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let folders: Vec<i64> =
                    serde_json::from_value(request["folders"].clone()).map_err(error)?;
                crate::commands::search::verified_folders(&account, &folders)?;
                let source = InventorySearchSource {
                    folders,
                    cache: self.inventories.clone(),
                    source: InventorySource {
                        path: child(&self.root, text(&request, "source")?)?,
                        history: self.inventory_history.clone(),
                        calls: self.inventory_calls.clone(),
                        lookups: self.inventory_lookups.clone(),
                        lookup_times: self.inventory_lookup_times.clone(),
                        cached: Arc::new(std::sync::Mutex::new(None)),
                        started: Arc::new(tokio::sync::Notify::new()),
                    },
                };
                let mut reply = serde_json::to_value(
                    crate::commands::storage_insights::from_source(
                        account,
                        self.crypto.clone(),
                        &source,
                        text(&request, "view")?.into(),
                        Some(1),
                        Some(1),
                    )
                    .await?,
                )
                .map_err(error)?;
                reply["historyCalls"] = json!(self
                    .inventory_calls
                    .load(std::sync::atomic::Ordering::SeqCst));
                reply["lookupBatches"] = json!(self
                    .inventory_lookups
                    .load(std::sync::atomic::Ordering::SeqCst));
                Ok(reply)
            }
            "inventory_refresh_overlap" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let source = InventorySource {
                    path: child(&self.root, text(&request, "source")?)?,
                    history: self.inventory_history.clone(),
                    calls: self.inventory_calls.clone(),
                    lookups: self.inventory_lookups.clone(),
                    lookup_times: self.inventory_lookup_times.clone(),
                    cached: Arc::new(std::sync::Mutex::new(None)),
                    started: Arc::new(tokio::sync::Notify::new()),
                };
                let cache = self.inventories.clone();
                let owner = account.clone();
                let first_source = source.clone();
                let first = tokio::spawn(async move {
                    cache.read(&owner, None, &first_source, false, false).await
                });
                source.started.notified().await;
                let refreshed = self
                    .inventories
                    .read(&account, None, &source, false, true)
                    .await?;
                let first = first.await.map_err(error)??;
                Ok(json!(
                    {
                        "first":first,
                        "refreshed":refreshed,
                        "historyCalls":self.inventory_calls.load(std::sync::atomic::Ordering::SeqCst),
                        "lookupBatches":self.inventory_lookups.load(std::sync::atomic::Ordering::SeqCst)
                    }
                ))
            }
            "inventory_detection_bound" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let cache = crate::file_inventory::InventoryCache::default();
                cache.advance_test_clock(Duration::ZERO);
                let mut sources = HashMap::new();
                let large_folders = request["largeFolders"].as_i64().unwrap_or(2);
                let small_folders = request["smallFolders"].as_i64().unwrap_or(5);
                let ticks = request["ticks"].as_u64().unwrap_or(184);
                let initial_folders = large_folders + small_folders;
                for id in 1..=initial_folders {
                    let path = if id <= large_folders {
                        text(&request, "large")?
                    } else {
                        text(&request, "small")?
                    };
                    let source = InventorySource {
                        path: child(&self.root, path)?,
                        history: self.inventory_history.clone(),
                        calls: self.inventory_calls.clone(),
                        lookups: self.inventory_lookups.clone(),
                        lookup_times: self.inventory_lookup_times.clone(),
                        cached: Arc::new(std::sync::Mutex::new(None)),
                        started: Arc::new(tokio::sync::Notify::new()),
                    };
                    cache
                        .read_with_ticket(&account, Some(id), &source, false, false)
                        .await?;
                    sources.insert(Some(id), source);
                }
                let cold_history = self
                    .inventory_calls
                    .load(std::sync::atomic::Ordering::SeqCst);
                let cold_lookups = self
                    .inventory_lookups
                    .load(std::sync::atomic::Ordering::SeqCst);
                let db =
                    sqlite::open(self.root.join("workspace/101/workspace.db")).map_err(error)?;
                let version = || -> Result<i64, String> {
                    let mut query = db.prepare("PRAGMA data_version").map_err(error)?;
                    query.next().map_err(error)?;
                    query.read::<i64, _>(0).map_err(error)
                };
                let before = version()?;
                let mut max_window = 0;
                let mut lookup_window = std::collections::VecDeque::new();
                let mut prior = self
                    .inventory_lookups
                    .load(std::sync::atomic::Ordering::SeqCst);
                for tick in 1..=ticks {
                    cache.advance_test_clock(Duration::from_secs(30));
                    if tick == 90 && request["grow"] != false {
                        for id in (initial_folders + 1)..=(large_folders + 20) {
                            let source = sources
                                .get(&Some(large_folders + 1))
                                .ok_or("Missing small source")?
                                .clone();
                            cache
                                .read_with_ticket(&account, Some(id), &source, false, false)
                                .await?;
                            sources.insert(Some(id), source);
                        }
                    }
                    for folder in sources.keys() {
                        cache.touch(&account, *folder)?;
                    }
                    cache
                        .poll_recent(|_, folder| {
                            let source = sources
                                .get(&folder)
                                .cloned()
                                .ok_or_else(|| "Missing source".to_string());
                            async move { source }
                        })
                        .await;
                    let total = self
                        .inventory_lookups
                        .load(std::sync::atomic::Ordering::SeqCst);
                    lookup_window.push_back(total - prior);
                    prior = total;
                    if lookup_window.len() > 2 {
                        lookup_window.pop_front();
                    }
                    max_window = max_window.max(lookup_window.iter().sum::<u64>());
                }
                let (seen, repeated, max_gap, minimum_checks, original_checks) =
                    cache.test_detection_stats(initial_folders);
                Ok(json!(
                    {
                        "coldHistoryCalls":cold_history,
                        "coldLookupBatches":cold_lookups,
                        "historyCalls":self.inventory_calls.load(std::sync::atomic::Ordering::SeqCst)-cold_history,
                        "lookupBatches":self.inventory_lookups.load(std::sync::atomic::Ordering::SeqCst)-cold_lookups,
                        "writesChanged":version()?!=before,
                        "seen":seen,
                        "repeated":repeated,
                        "maxGapSeconds":max_gap.as_secs(),
                        "maxBatchesPerMinute":max_window,
                        "minimumChecks":minimum_checks,
                        "minimumOriginalChecks":original_checks
                    }
                ))
            }
            "inventory_audit_rounds" => {
                use crate::file_inventory::{InventoryCache, Timing};
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let cache = InventoryCache::with_timing(Timing {
                    poll: Duration::from_millis(30),
                    audit: Duration::from_millis(240),
                    reconciliation: Duration::from_secs(5),
                    retention: Duration::from_secs(10),
                    audit_window: Duration::from_millis(60),
                });
                let mut sources = HashMap::new();
                for (id, file) in [(1, "large"), (2, "small"), (3, "failure")] {
                    let Some(path) = request[file].as_str() else {
                        continue;
                    };
                    let source = InventorySource {
                        path: child(&self.root, path)?,
                        history: self.inventory_history.clone(),
                        calls: self.inventory_calls.clone(),
                        lookups: self.inventory_lookups.clone(),
                        lookup_times: self.inventory_lookup_times.clone(),
                        cached: Arc::new(std::sync::Mutex::new(None)),
                        started: Arc::new(tokio::sync::Notify::new()),
                    };
                    cache
                        .read(&account, Some(id), &source, false, false)
                        .await?;
                    sources.insert(Some(id), source);
                }
                let cold_calls = self
                    .inventory_calls
                    .load(std::sync::atomic::Ordering::SeqCst);
                let cold_lookups = self
                    .inventory_lookups
                    .load(std::sync::atomic::Ordering::SeqCst);
                let db =
                    sqlite::open(self.root.join("workspace/101/workspace.db")).map_err(error)?;
                let version = |db: &sqlite::Connection| -> Result<i64, String> {
                    let mut query = db.prepare("PRAGMA data_version").map_err(error)?;
                    query.next().map_err(error)?;
                    query.read::<i64, _>(0).map_err(error)
                };
                let before = version(&db)?;
                let started = std::time::Instant::now();
                let duration = Duration::from_millis(request["durationMs"].as_u64().unwrap_or(600));
                let mut events = 0;
                let cold_times = self.inventory_lookup_times.lock().map_err(error)?.len();
                let mut edited = false;
                let mut recovered = false;
                while started.elapsed() < duration {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    if request["editTwice"] == true
                        && !edited
                        && started.elapsed() > Duration::from_millis(650)
                    {
                        let source = sources.get(&Some(2)).ok_or("Missing small source")?;
                        let mut data: Value = serde_json::from_slice(
                            &tokio::fs::read(&source.path).await.map_err(error)?,
                        )
                        .map_err(error)?;
                        data["rows"][0]["name"] = json!("Second outside rename");
                        tokio::fs::write(&source.path, data.to_string())
                            .await
                            .map_err(error)?;
                        source.cached.lock().map_err(error)?.take();
                        edited = true;
                    }
                    if request["recoverLarge"] == true
                        && !recovered
                        && started.elapsed() > Duration::from_millis(650)
                    {
                        let source = sources.get(&Some(1)).ok_or("Missing large source")?;
                        let mut data: Value = serde_json::from_slice(
                            &tokio::fs::read(&source.path).await.map_err(error)?,
                        )
                        .map_err(error)?;
                        data["lookupError"] = json!(false);
                        let last = data["rows"]
                            .as_array_mut()
                            .ok_or("Missing rows")?
                            .last_mut()
                            .ok_or("Empty rows")?;
                        last["name"] = json!("Recovered last row");
                        tokio::fs::write(&source.path, data.to_string())
                            .await
                            .map_err(error)?;
                        source.cached.lock().map_err(error)?.take();
                        recovered = true;
                    }
                    if let Some(source) = sources.get(&Some(2)) {
                        for _ in 0..5 {
                            cache.read(&account, Some(2), source, false, false).await?;
                        }
                    }
                    let changes = cache
                        .poll_recent(|_, folder| {
                            let source = sources
                                .get(&folder)
                                .cloned()
                                .ok_or_else(|| "Missing source".to_string());
                            async move { source }
                        })
                        .await;
                    events += changes.len();
                }
                let times = self.inventory_lookup_times.lock().map_err(error)?.clone();
                let mut max_batches = 0;
                let mut window = std::collections::VecDeque::new();
                for at in &times[cold_times..] {
                    while window.front().is_some_and(|previous| {
                        at.duration_since(*previous) >= Duration::from_millis(60)
                    }) {
                        window.pop_front();
                    }
                    window.push_back(*at);
                    max_batches = max_batches.max(window.len());
                }
                let small = if let Some(source) = sources.get(&Some(2)) {
                    cache
                        .read(&account, Some(2), source, false, false)
                        .await?
                        .first()
                        .map(|row| row.name.clone())
                } else {
                    None
                };
                let large = if let Some(source) = sources.get(&Some(1)) {
                    let listing = cache
                        .read_with_ticket(&account, Some(1), source, false, false)
                        .await?;
                    Some(
                        json!({"last":listing.rows.first().map(|row| row.name.clone()), "complete":listing.complete}),
                    )
                } else {
                    None
                };
                Ok(json!(
                    {
                        "coldHistoryCalls":cold_calls,
                        "coldLookupBatches":cold_lookups,
                        "historyCalls":self.inventory_calls.load(std::sync::atomic::Ordering::SeqCst)-cold_calls,
                        "lookupBatches":self.inventory_lookups.load(std::sync::atomic::Ordering::SeqCst)-cold_lookups,
                        "maxBatchesPerWindow":max_batches,
                        "writesChanged":version(&db)?!=before,
                        "events":events,
                        "smallName":small,
                        "large":large,
                        "elapsedMs":started.elapsed().as_millis()
                    }
                ))
            }
            "inventory_raw_list" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let source = RawInventorySource(child(&self.root, text(&request, "source")?)?);
                let listing = self
                    .raw_inventory
                    .read_with_ticket(&account, None, &source, request["audit"] == true, false)
                    .await?;
                let rows: Vec<_> = listing.rows.iter().map(|row| {
                    let grammers_tl_types::enums::Message::Message(message) = &row.0 else { unreachable!() };
                    let Some(grammers_tl_types::enums::MessageMedia::Document(media)) = &message.media else { unreachable!() };
                    let Some(grammers_tl_types::enums::Document::Document(document)) = &media.document else { unreachable!() };
                    json!({"id": message.id, "caption": message.message, "views": message.views, "reference": document.file_reference})
                }).collect();
                Ok(json!({"serial": listing.serial, "rows": rows}))
            }
            "inventory_list" | "inventory_parallel" | "inventory_waiter" => {
                let root = if let Some(root) = request["root"].as_str() {
                    child(&self.root, root)?
                } else {
                    self.root.clone()
                };
                let account = AccountGuard::open(&root, Some(text(&request, "owner")?))?;
                let folder = request["folder"].as_i64();
                let source = InventorySource {
                    path: child(&self.root, text(&request, "source")?)?,
                    history: self.inventory_history.clone(),
                    calls: self.inventory_calls.clone(),
                    lookups: self.inventory_lookups.clone(),
                    lookup_times: self.inventory_lookup_times.clone(),
                    cached: Arc::new(std::sync::Mutex::new(None)),
                    started: Arc::new(tokio::sync::Notify::new()),
                };
                let audit = request["audit"].as_bool().unwrap_or(false);
                let poll = request["poll"].as_bool().unwrap_or(false);
                if request["completeBeforeWait"].as_bool().unwrap_or(false) {
                    let timing = crate::file_inventory::Timing {
                        audit: std::time::Duration::from_millis(1),
                        poll: std::time::Duration::from_millis(1),
                        ..Default::default()
                    };
                    self.inventories =
                        Arc::new(crate::file_inventory::InventoryCache::with_timing(timing));
                }
                let listing = if request["command"] == "inventory_waiter" {
                    let mut first = Box::pin(
                        self.inventories
                            .read_with_ticket(&account, folder, &source, audit, poll),
                    );
                    tokio::select! {
                        _=source.started.notified()=>{},
                        result=&mut first=>{result?;return Err("Fixture source did not pause".into());}
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    drop(first); // abandoning one subscriber must not stop shared reconciliation
                    if !self.inventories.pending(&account, folder)? {
                        return Err("Abandoned job is not pending".into());
                    }
                    if request["completeBeforeWait"].as_bool().unwrap_or(false) {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    self.inventories
                        .read_with_ticket(&account, folder, &source, audit, poll)
                        .await?
                } else if request["command"] == "inventory_parallel" {
                    let (a, b) = tokio::join!(
                        self.inventories
                            .read_with_ticket(&account, folder, &source, audit, poll),
                        self.inventories
                            .read_with_ticket(&account, folder, &source, audit, poll)
                    );
                    let a = a?;
                    let b = b?;
                    if serde_json::to_value(
                        a.rows.iter().map(|row| row.as_ref()).collect::<Vec<_>>(),
                    )
                    .map_err(error)?
                        != serde_json::to_value(
                            b.rows.iter().map(|row| row.as_ref()).collect::<Vec<_>>(),
                        )
                        .map_err(error)?
                    {
                        return Err("Concurrent listings diverged".into());
                    }
                    a
                } else {
                    self.inventories
                        .read_with_ticket(&account, folder, &source, audit, poll)
                        .await?
                };
                let complete = listing.complete;
                let rows = listing
                    .rows
                    .iter()
                    .map(|row| row.as_ref())
                    .collect::<Vec<_>>();
                Ok(json!(
                    {
                        "complete":complete,
                        "rows":rows,
                        "historyWalks":self.inventory_history.load(std::sync::atomic::Ordering::SeqCst),
                        "historyCalls":self.inventory_calls.load(std::sync::atomic::Ordering::SeqCst),
                        "lookupBatches":self.inventory_lookups.load(std::sync::atomic::Ordering::SeqCst)
                    }
                ))
            }
            "inventory_touch" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                Ok(json!(self
                    .inventories
                    .touch(&account, request["folder"].as_i64())?))
            }
            "inventory_invalidate" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                crate::file_inventory::invalidate(account.owner);
                Ok(json!(true))
            }
            "inventory_watch_start" => {
                if self.inventory_task.is_some() {
                    return Err("Inventory watch already active".into());
                }
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let source = InventorySource {
                    path: child(&self.root, text(&request, "source")?)?,
                    history: self.inventory_history.clone(),
                    calls: self.inventory_calls.clone(),
                    lookups: self.inventory_lookups.clone(),
                    lookup_times: self.inventory_lookup_times.clone(),
                    cached: Arc::new(std::sync::Mutex::new(None)),
                    started: Arc::new(tokio::sync::Notify::new()),
                };
                let started = source.started.clone();
                let cache = self.inventories.clone();
                self.inventory_task = Some(tokio::spawn(async move {
                    cache.read(&account, None, &source, false, false).await
                }));
                started.notified().await;
                Ok(json!(true))
            }
            "inventory_watch_finish" => serde_json::to_value(
                self.inventory_task
                    .take()
                    .ok_or("No inventory watch")?
                    .await
                    .map_err(error)??,
            )
            .map_err(error),
            "inventory_timer_lifecycle" => {
                use crate::file_inventory::{InventoryCache, Timing};
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let live = child(&self.root, "inventory-timer-live.json")?;
                tokio::fs::copy(child(&self.root, text(&request, "initial")?)?, &live)
                    .await
                    .map_err(error)?;
                let source = |path: PathBuf| InventorySource {
                    path,
                    history: self.inventory_history.clone(),
                    calls: self.inventory_calls.clone(),
                    lookups: self.inventory_lookups.clone(),
                    lookup_times: self.inventory_lookup_times.clone(),
                    cached: Arc::new(std::sync::Mutex::new(None)),
                    started: Arc::new(tokio::sync::Notify::new()),
                };
                let timing = Timing {
                    poll: std::time::Duration::from_millis(30),
                    audit: std::time::Duration::from_millis(60),
                    reconciliation: INVENTORY_FIXTURE_RECONCILIATION,
                    retention: std::time::Duration::from_secs(2),
                    audit_window: std::time::Duration::from_secs(60),
                };
                let cache = InventoryCache::with_timing(timing);
                cache
                    .read(&account, None, &source(live.clone()), false, false)
                    .await?;
                let unchanged = cache
                    .poll_recent(|_, _| {
                        let source = source(live.clone());
                        async move { Ok(source) }
                    })
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                tokio::fs::copy(child(&self.root, text(&request, "changed")?)?, &live)
                    .await
                    .map_err(error)?;
                if request["consumerBeforePoll"].as_bool().unwrap_or(false) {
                    cache
                        .read(&account, None, &source(live.clone()), false, false)
                        .await?;
                }
                let changed = cache
                    .poll_recent(|_, _| {
                        let source = source(live.clone());
                        async move { Ok(source) }
                    })
                    .await;
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                tokio::fs::copy(child(&self.root, text(&request, "cold")?)?, &live)
                    .await
                    .map_err(error)?;
                let before = self
                    .inventory_lookups
                    .load(std::sync::atomic::Ordering::SeqCst);
                let cold = cache
                    .read(&account, None, &source(live), false, false)
                    .await?;
                Ok(json!(
                    {
                        "unchangedEvents":unchanged.len(),
                        "changedEvents":changed.len(),
                        "changedName":changed.first().and_then(|(_,
                        _,
                        listing)|listing.rows.first()).map(|row|row.name.clone()),
                        "coldName":cold.first().map(|row|row.name.clone()),
                        "coldLookupBatches":self.inventory_lookups.load(std::sync::atomic::Ordering::SeqCst)-before
                    }
                ))
            }
            "inventory_deadline" => {
                use crate::file_inventory::{InventoryCache, Timing};
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                drop(Store::open(&self.root, account.owner)?);
                let source = InventorySource {
                    path: child(&self.root, text(&request, "source")?)?,
                    history: self.inventory_history.clone(),
                    calls: self.inventory_calls.clone(),
                    lookups: self.inventory_lookups.clone(),
                    lookup_times: self.inventory_lookup_times.clone(),
                    cached: Arc::new(std::sync::Mutex::new(None)),
                    started: Arc::new(tokio::sync::Notify::new()),
                };
                let cache = InventoryCache::with_timing(Timing {
                    poll: std::time::Duration::from_millis(1),
                    audit: std::time::Duration::from_millis(1),
                    reconciliation: std::time::Duration::from_millis(20),
                    retention: std::time::Duration::from_secs(1),
                    audit_window: std::time::Duration::from_secs(60),
                });
                let outcome = cache.read(&account, None, &source, false, false).await;
                let cursor = Store::open(&self.root, account.owner)?
                    .record::<Value>("file-inventory-v1", "saved")?;
                Ok(json!({"error":outcome.err(),"cursor":cursor}))
            }
            "legacy_inventory_write" => {
                let file: crate::models::FileMetadata =
                    serde_json::from_value(request["file"].clone()).map_err(error)?;
                crate::commands::file_inventory::upsert_inventory_chunk(
                    crate::db::init_db_at(&self.root)?,
                    "home".into(),
                    "fixture".into(),
                    vec![file],
                    None,
                )
                .await?;
                Ok(json!(true))
            }
            _ => Err("Unknown inventory E2E command".into()),
        }
    }

    async fn dispatch_assets(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "heic_pin_gate" => {
                crate::workspace::assets::test_heic_pin_gate(
                    self.root.join("heic-pin.checked"),
                    self.root.join("heic-pin.release"),
                );
                Ok(json!(true))
            }
            "heic_pin_start" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let root = self.root.clone();
                let key = crate::workspace::store::file_key(None, 1);
                self.vault_tasks.insert(
                    "heic-pin".into(),
                    tokio::spawn(async move {
                        let result = crate::workspace::assets::set_pinned_at(
                            root.join("asset-cache"),
                            account,
                            key,
                            false,
                        )
                        .await?;
                        std::fs::write(root.join("heic-pin.done"), b"done").map_err(error)?;
                        Ok(json!(result))
                    }),
                );
                Ok(json!(true))
            }
            "heic_expected_pixels" => {
                let rendered =
                    image::open(produced_fixture_path(&self.root, text(&request, "path")?)?)
                        .map_err(error)?
                        .to_rgb8();
                let reference = image::open(produced_fixture_path(
                    &self.root,
                    text(&request, "reference")?,
                )?)
                .map_err(error)?;
                let expected = match request["transform"].as_str() {
                    Some("rotation") => reference.rotate270(),
                    Some("mirror") => reference.flipv(),
                    _ => reference,
                }
                .to_rgb8();
                if rendered.dimensions() != expected.dimensions() {
                    return Err("Orientation dimensions differ".into());
                }
                let total: u64 = rendered
                    .as_raw()
                    .iter()
                    .zip(expected.as_raw())
                    .map(|(a, b)| u64::from(a.abs_diff(*b)))
                    .sum();
                Ok(
                    json!({"dimensions":rendered.dimensions(), "meanAbsoluteChannelError":total as f64 / rendered.as_raw().len() as f64}),
                )
            }
            "asset_display_local" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let key = crate::workspace::store::file_key(
                    request["folder"].as_i64(),
                    request["id"].as_i64().unwrap_or(1),
                );
                Ok(json!(
                    crate::workspace::assets::display_rendition_at(
                        self.root.join("asset-cache"),
                        account,
                        key,
                        produced_fixture_path(&self.root, text(&request, "path")?)?,
                        heic_fixture_tools(&self.root, &request)?,
                        None
                    )
                    .await?
                ))
            }
            "heic_status" => Ok(
                json!({"decodes":crate::heic::reports(), "processes":crate::process_budget::observations(),"decoderEntered":self.root.join("heic-pid").is_file()}),
            ),
            "asset_read" | "asset_display_read" | "asset_start" | "workspace_asset_read" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let cache = self.root.join("asset-cache");
                let root = self.root.clone();
                let downloads = self.asset_downloads.clone();
                let key = crate::workspace::store::file_key(
                    request["folder"].as_i64(),
                    request["id"].as_i64().unwrap_or(1),
                );
                let thumbnail = request["thumbnail"].as_bool().unwrap_or(false);
                let request_id = request["requestId"].as_str().map(str::to_string);
                if request["command"] == "asset_start" {
                    if self.asset_task.is_some() {
                        return Err("Asset already running".into());
                    }
                    let (started, wait) = tokio::sync::oneshot::channel();
                    let scope = account.clone();
                    let display = request["display"].as_bool().unwrap_or(false);
                    let display_tools = if display {
                        heic_fixture_tools(&root, &request)?
                    } else {
                        crate::heic::Tools::default()
                    };
                    let display_scope = (
                        cache.clone(),
                        account.clone(),
                        key.clone(),
                        request_id.clone(),
                    );
                    self.asset_task = Some(tokio::spawn(async move {
                        let original = crate::workspace::assets::asset_at(
                            cache,
                            account,
                            key,
                            thumbnail,
                            request_id,
                            move || async move {
                                let _ = started.send(());
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    request["lookupDelayMs"].as_u64().unwrap_or(0),
                                ))
                                .await;
                                fixture_asset(&root, &scope, &request, downloads)
                            },
                        )
                        .await?;
                        if display {
                            crate::workspace::assets::display_rendition_at(
                                display_scope.0,
                                display_scope.1,
                                display_scope.2,
                                original.into(),
                                display_tools,
                                display_scope.3,
                            )
                            .await
                        } else {
                            Ok(original)
                        }
                    }));
                    wait.await.map_err(error)?;
                    Ok(json!(true))
                } else {
                    let scope = account.clone();
                    let display = request["command"] == "asset_display_read";
                    let display_scope = (cache.clone(), account.clone(), key.clone());
                    let display_tools = if display {
                        heic_fixture_tools(&root, &request)?
                    } else {
                        crate::heic::Tools::default()
                    };
                    let path = crate::workspace::assets::asset_at(
                        cache,
                        account,
                        key,
                        thumbnail,
                        request_id,
                        move || async move {
                            let lookup = if request["command"] == "workspace_asset_read" {
                                Some(
                                    crate::workspace::assets::lookup_file_at(
                                        &scope,
                                        &crate::workspace::store::file_key(
                                            request["folder"].as_i64(),
                                            request["id"].as_i64().unwrap_or(1),
                                        ),
                                    )
                                    .await?,
                                )
                            } else {
                                None
                            };
                            let (file, source) = fixture_asset(&root, &scope, &request, downloads)?;
                            match lookup {
                                Some(crate::workspace::assets::AssetLookup::Indexed(file)) => {
                                    if file.file.encryption_state != "plain" {
                                        return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
                                    }
                                    Ok((*file, source))
                                }
                                _ => Ok((file, source)),
                            }
                        },
                    )
                    .await?;
                    let path = if display {
                        crate::workspace::assets::display_rendition_at(
                            display_scope.0,
                            display_scope.1,
                            display_scope.2,
                            path.into(),
                            display_tools,
                            None,
                        )
                        .await?
                    } else {
                        path
                    };
                    Ok(
                        json!({"path":path,"downloads":self.asset_downloads.load(std::sync::atomic::Ordering::SeqCst)}),
                    )
                }
            }
            "native_preview_prepare" => {
                crate::workspace::device_cache::set_limit_bytes(
                    request["limit"].as_u64().ok_or("Missing limit")?,
                );
                match crate::workspace::device_cache::prepare(
                    &self.root,
                    &self.root.join("asset-cache"),
                    101,
                    text(&request, "filename")?,
                    request["size"].as_u64().ok_or("Missing size")?,
                )? {
                    crate::workspace::device_cache::Prepared::Cached(path) => {
                        Ok(json!({"cached":path}))
                    }
                    crate::workspace::device_cache::Prepared::Download(lease) => {
                        let path = lease.partial.clone();
                        self.native_preview = Some(lease);
                        Ok(json!({"partial":path}))
                    }
                }
            }
            "native_preview_finish" => {
                let lease = self.native_preview.take().ok_or("No native preview")?;
                Ok(json!(lease.finish()?))
            }
            "native_preview_clear" => Ok(json!(crate::workspace::device_cache::clear(
                &self.root,
                &self.root.join("asset-cache")
            )?)),
            "asset_pin" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                Ok(json!(
                    crate::workspace::assets::set_pinned_at(
                        self.root.join("asset-cache"),
                        account,
                        crate::workspace::store::file_key(
                            request["folder"].as_i64(),
                            request["id"].as_i64().unwrap_or(1)
                        ),
                        request["pinned"].as_bool().unwrap_or(true)
                    )
                    .await?
                ))
            }
            "asset_cached" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                Ok(json!(crate::workspace::assets::cached_preview_at(
                    &self.root.join("asset-cache"),
                    &account,
                    &crate::workspace::store::file_key(
                        request["folder"].as_i64(),
                        request["id"].as_i64().unwrap_or(1)
                    )
                )?))
            }
            "asset_delete" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                crate::workspace::assets::delete_cached_at(
                    self.root.join("asset-cache"),
                    account,
                    crate::workspace::store::file_key(
                        request["folder"].as_i64(),
                        request["id"].as_i64().unwrap_or(1),
                    ),
                    request["thumbnail"].as_bool().unwrap_or(false),
                )
                .await?;
                Ok(json!(true))
            }
            "api_thumbnail_seed" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let (file, source) =
                    fixture_asset(&self.root, &account, &request, self.asset_downloads.clone())?;
                crate::workspace::assets::seed_thumbnail_fixture(
                    &account,
                    file,
                    source.ok_or("No source")?,
                );
                Ok(json!(true))
            }
            "asset_clear_all" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let cache = self.root.join("asset-cache");
                crate::workspace::assets::clear_at(&self.root, &cache, account.owner, "previews")
                    .await?;
                crate::workspace::storage::clear_legacy_previews(&cache)?;
                crate::workspace::device_cache::clear(&self.root, &cache)?;
                account.validate()?;
                Ok(json!(true))
            }
            "asset_status" => serde_json::to_value(
                crate::commands::preview::preview_cache_status(
                    &self.root.join("asset-cache/previews"),
                )
                .await?,
            )
            .map_err(error),
            "legacy_external_status" => {
                let account = AccountGuard::open(&self.root, None)?;
                let values = Store::open(&account.root, account.owner)?
                    .records::<Value>("external-cache-migration-v1")?;
                Ok(json!({"migrated": values.len()}))
            }
            "legacy_external_resolve_start" => {
                let account = AccountGuard::open(&self.root, None)?;
                let id = text(&request, "task")?.to_string();
                let cache = self.root.join("asset-cache");
                let started = self.root.join(format!("file-{id}.started"));
                crate::external_files::test_hold(
                    started.clone(),
                    self.root.join(format!("file-{id}.release")),
                );
                self.vault_tasks.insert(
                    id,
                    tokio::spawn(async move {
                        crate::workspace::assets::legacy_asset_at(
                            cache,
                            account,
                            "saved:1".into(),
                            false,
                            || async {
                                Err("NETWORK_UNAVAILABLE: Telegram fixture is offline".into())
                            },
                        )
                        .await
                        .map(|path| json!(path))
                    }),
                );
                let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                while !started.is_file() {
                    if tokio::time::Instant::now() >= deadline {
                        return Err("Legacy hash did not start".into());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(json!(true))
            }
            "legacy_external_clear_start" => {
                let account = AccountGuard::open(&self.root, None)?;
                let id = text(&request, "task")?.to_string();
                let root = self.root.clone();
                let started = root.join("legacy-clear.started");
                let signal = started.clone();
                self.vault_tasks.insert(
                    id,
                    tokio::spawn(async move {
                        std::fs::write(signal, b"ready").map_err(error)?;
                        let cache = root.join("asset-cache");
                        crate::workspace::assets::clear_at(
                            &root,
                            &cache,
                            account.owner,
                            "previews",
                        )
                        .await?;
                        crate::workspace::storage::clear_legacy_previews(&cache)?;
                        std::fs::write(root.join("legacy-clear.done"), b"done").map_err(error)?;
                        Ok(json!(true))
                    }),
                );
                let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                while !started.is_file() {
                    if tokio::time::Instant::now() >= deadline {
                        return Err("Legacy clear did not start".into());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok(json!(true))
            }
            "legacy_external_resolve" => {
                let account = AccountGuard::open(&self.root, None)?;
                let key =
                    crate::workspace::store::file_key(None, request["id"].as_i64().unwrap_or(1));
                crate::workspace::assets::legacy_asset_at(
                    self.root.join("asset-cache"),
                    account,
                    key,
                    request["thumbnail"].as_bool().unwrap_or(false),
                    || async { Err("NETWORK_UNAVAILABLE: Telegram fixture is offline".into()) },
                )
                .await
                .map(|path| json!(path))
            }
            "legacy_external_seed" => {
                use sha2::{Digest, Sha256};
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let source = child(&self.root, text(&request, "source")?)?;
                let size = std::fs::metadata(&source).map_err(error)?.len();
                let id = request["id"].as_i64().unwrap_or(1);
                let file = crate::workspace::store::WorkspaceFile {
                    key: crate::workspace::store::file_key(None, id),
                    folder_name: "Historical fixture".into(),
                    tags: Vec::new(),
                    collection_ids: Vec::new(),
                    file: crate::models::FileMetadata {
                        id,
                        folder_id: None,
                        name: request["filename"].as_str().unwrap_or("fixture.png").into(),
                        size,
                        mime_type: Some(request["mime"].as_str().unwrap_or("image/png").into()),
                        file_ext: Some(request["extension"].as_str().unwrap_or("png").into()),
                        created_at: "2026-10-01T00:00:00Z".into(),
                        icon_type: "file".into(),
                        encryption_state: request["protection"].as_str().unwrap_or("plain").into(),
                        is_favorite: false,
                        is_pinned: false,
                    },
                };
                let store = Store::open(&account.root, account.owner)?;
                store.remember_local_file(&file.file)?;
                store.put_record(
                    "opened",
                    &file.key,
                    &json!({"folder_id":null,"message_id":id,"last_opened_at":100,"open_count":1}),
                )?;
                if request["removeMetadata"] == true {
                    store.complete_scan(None, "missing-current-inventory")?;
                }
                if request["updateOnly"] == true {
                    return Ok(json!(true));
                }
                let category = request["category"].as_str().unwrap_or("previews");
                let pack_id = uuid::Uuid::new_v4().to_string();
                let directory = if category == "flat-preview" {
                    self.root.join("asset-cache/previews")
                } else if category == "flat-thumbnail" {
                    self.root.join("thumbnails")
                } else if category == "offline" {
                    let pack = json!({"id": pack_id, "ownerId": account.owner.to_string(), "status":"ready",
                        "files":[{"file":file,"status":"ready","downloadedBytes":size}], "expiresAt":null});
                    store.put_record("offline-pack", &pack_id, &pack)?;
                    self.root
                        .join("workspace")
                        .join(account.owner.to_string())
                        .join("offline")
                        .join(&pack_id)
                } else {
                    if !["previews", "thumbnails"].contains(&category) {
                        return Err("Invalid fixture category".into());
                    }
                    self.root
                        .join("asset-cache/previews/workspace")
                        .join(account.owner.to_string())
                        .join(category)
                };
                std::fs::create_dir_all(&directory).map_err(error)?;
                let mut key = file.key.clone();
                if request["current"] == true {
                    let identity = "recorded-current-fixture";
                    store.put_record("asset-identity-v1", &key, &identity)?;
                    key = format!("raster-v2:{key}:{identity}:{}", category == "thumbnails");
                }
                let name = if category == "flat-preview" {
                    format!("{}_home_{id}.png", account.owner)
                } else if category == "flat-thumbnail" {
                    format!("{}_home_{id}.thumb.jpg", account.owner)
                } else if category == "thumbnails" {
                    format!("{:x}.jpg", Sha256::digest(key.as_bytes()))
                } else {
                    let mut named = file.clone();
                    named.key = key;
                    crate::workspace::assets::file_name(&named)
                };
                let target = directory.join(name);
                std::fs::hard_link(source, &target).map_err(error)?;
                let canonical = target.canonicalize().map_err(error)?;
                let relative = canonical
                    .strip_prefix(self.root.canonicalize().map_err(error)?)
                    .map_err(error)?;
                Ok(json!({"path":relative.to_string_lossy(),"pack":pack_id}))
            }
            "asset_offline_seed" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let (file, _) =
                    fixture_asset(&self.root, &account, &request, self.asset_downloads.clone())?;
                let store = Store::open(&account.root, account.owner)?;
                store.remember_local_file(&file.file)?;
                store.put_record("opened",&file.key,&json!({"folder_id":file.file.folder_id,"message_id":file.file.id,"last_opened_at":100,"open_count":1}))?;
                Ok(json!(true))
            }
            "asset_offline_read" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                serde_json::to_value(
                    crate::commands::preview::read_offline_files(
                        &account,
                        &self.root.join("asset-cache/previews"),
                        Some(250),
                    )
                    .await?,
                )
                .map_err(error)
            }
            "asset_metadata_finished" => Ok(json!(crate::workspace::assets::metadata_finished())),
            "asset_metadata_started" => Ok(json!(crate::workspace::assets::metadata_started())),
            "asset_core_busy" => Ok(json!(crate::workspace::assets::core_busy())),
            "legacy_pin_start" => {
                let started = self.root.join("legacy-pin-started");
                let release = self.root.join("legacy-pin-release");
                crate::commands::preview::install_pin_gate(started.clone(), release);
                let cache = self.root.join("asset-cache/previews");
                self.legacy_pin_task = Some(tokio::task::spawn_blocking(move || {
                    crate::commands::preview::set_preview_pinned(&cache, "101_home_9", true)
                }));
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                while !started.is_file() {
                    if tokio::time::Instant::now() >= deadline {
                        return Err("Pin did not start".into());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                Ok(json!(true))
            }
            "legacy_pin_finish" => {
                self.legacy_pin_task
                    .take()
                    .ok_or("No pin task")?
                    .await
                    .map_err(error)??;
                Ok(json!(true))
            }
            "reader_copy_start" => {
                if !self.reader_tasks.is_empty() {
                    return Err("Reader copies already started".into());
                }
                let copies = request["copies"].as_u64().unwrap_or(1);
                if !(1..=4).contains(&copies) {
                    return Err("Invalid copy count".into());
                }
                let source = child(&self.root, text(&request, "source")?)?;
                let expected = std::fs::metadata(&source).map_err(error)?.len();
                let destinations = (0..copies)
                    .map(|index| {
                        child(
                            &self.root,
                            &format!("{}-{index}", text(&request, "destination")?),
                        )
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let account = AccountGuard::open(&self.root, None)?;
                let holds = (0..copies)
                    .map(|_| {
                        crate::bandwidth::BandwidthReservation::upload(
                            self.bandwidth.clone(),
                            expected,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for (destination, mut hold) in destinations.into_iter().zip(holds) {
                    let account = account.clone();
                    let source = source.clone();
                    let network = self.network.clone();
                    self.reader_tasks.push(tokio::spawn(async move {
                        let partial =
                            destination.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
                        let _partial = ReaderPartial(partial.clone());
                        let input = tokio::fs::File::open(source).await.map_err(error)?;
                        let mut reader = crate::traffic::Reader::new(
                            input,
                            crate::traffic::Traffic {
                                network,
                                account: account.clone(),
                                direction: crate::traffic::Direction::Upload,
                            },
                        );
                        let mut options = std::fs::OpenOptions::new();
                        options.write(true).create_new(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            options.mode(0o600);
                        }
                        let mut output =
                            tokio::fs::File::from_std(options.open(&partial).map_err(error)?);
                        let bytes = tokio::io::copy(&mut reader, &mut output)
                            .await
                            .map_err(error)?;
                        if bytes != expected {
                            return Err("Copy source changed".into());
                        }
                        output.sync_all().await.map_err(error)?;
                        drop(output);
                        account.validate()?;
                        tokio::fs::rename(&partial, &destination)
                            .await
                            .map_err(error)?;
                        hold.commit();
                        Ok(bytes)
                    }));
                }
                Ok(json!(true))
            }
            "reader_copy_cancel" => {
                let tasks = std::mem::take(&mut self.reader_tasks);
                for task in &tasks {
                    task.abort();
                }
                for task in tasks {
                    let _ = task.await;
                }
                Ok(json!(self.bandwidth.get_stats()))
            }
            "reader_copy_finish" => {
                let mut sizes = Vec::new();
                let mut failure = None;
                for task in std::mem::take(&mut self.reader_tasks) {
                    match task.await {
                        Ok(Ok(size)) => sizes.push(size),
                        Ok(Err(error)) => failure = Some(error),
                        Err(error) => failure = Some(error.to_string()),
                    }
                }
                if let Some(error) = failure {
                    return Err(error);
                }
                Ok(json!(sizes))
            }
            "asset_limits" => {
                crate::workspace::assets::configure_limits(
                    request["previews"]
                        .as_u64()
                        .ok_or("Missing preview limit")?,
                    request["thumbnails"]
                        .as_u64()
                        .ok_or("Missing thumbnail limit")?,
                );
                Ok(json!(true))
            }
            "asset_abort" => {
                let task = self.asset_task.take().ok_or("No asset task")?;
                task.abort();
                let _ = task.await;
                Ok(json!(true))
            }
            "asset_finish" => Ok(json!(self
                .asset_task
                .take()
                .ok_or("No asset task")?
                .await
                .map_err(error)??)),
            "asset_cancel" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                Ok(json!(crate::workspace::assets::cancel_request(
                    &account,
                    text(&request, "requestId")?
                )?))
            }
            "asset_clear" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                crate::workspace::assets::clear_at(
                    &self.root,
                    &self.root.join("asset-cache"),
                    account.owner,
                    text(&request, "category")?,
                )
                .await?;
                account.validate()?;
                Ok(json!(true))
            }
            "thumbnail_batch" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let source = child(&self.root, text(&request, "source")?)?;
                let results = futures::future::join_all((0..6).map(|index| {
                    let source = source.clone();
                    let account = account.clone();
                    let target = self.root.join(format!("rendered-{index}.jpg"));
                    async move {
                        crate::commands::preview::create_resized_thumbnail(
                            source,
                            target,
                            Some(account),
                        )
                        .await
                    }
                }))
                .await;
                let mut sizes = Vec::new();
                for result in results {
                    let path = result?;
                    let (width, height) = image::image_dimensions(path).map_err(error)?;
                    sizes.push(json!({"width":width,"height":height}));
                }
                Ok(json!({"peak":crate::workspace::assets::decode_peak(),"sizes":sizes}))
            }
            _ => Err("Unknown assets E2E command".into()),
        }
    }

    async fn dispatch_traffic(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "traffic_clock" => {
                self.network.pacer.set_clock(
                    chrono::NaiveDateTime::parse_from_str(
                        text(&request, "time")?,
                        "%Y-%m-%d %H:%M",
                    )
                    .map_err(error)?,
                );
                Ok(json!(true))
            }
            "traffic_proxy_start_slow" => {
                if self.proxy_update_task.is_some() {
                    return Err("Proxy update already started".into());
                }
                let root = self.root.clone();
                let network = self.network.clone();
                self.proxy_update_task = Some(tokio::task::spawn_blocking(move || {
                    network.update_credentials_at(
                        &root.join("network_settings.json"),
                        |snapshot| {
                            snapshot.proxy.host = "new.fixture".into();
                            Ok(())
                        },
                        Some(Some("synthetic proxy credential".into())),
                        &SlowFixtureProxySecret { root },
                    )
                }));
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                while !self.root.join("proxy-io-started").is_file() {
                    if tokio::time::Instant::now() > deadline {
                        return Err("Proxy IO did not start".into());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                Ok(json!(true))
            }
            "traffic_proxy_finish_slow" => {
                self.proxy_update_task
                    .take()
                    .ok_or("No proxy update")?
                    .await
                    .map_err(error)??;
                Ok(json!(true))
            }
            "traffic_proxy_patch" => {
                let change = if request["clear"].as_bool().unwrap_or(false) {
                    Some(None)
                } else {
                    request["password"]
                        .as_str()
                        .map(|password| Some(password.to_string()))
                };
                self.network.update_credentials_at(
                    &self.root.join("network_settings.json"),
                    |snapshot| {
                        snapshot.proxy.host =
                            request["host"].as_str().unwrap_or("localhost").to_string();
                        if request["invalidWindow"].as_bool().unwrap_or(false) {
                            snapshot.vpn.bandwidth_windows = vec![crate::traffic::Window {
                                days: vec![],
                                start_minute: 0,
                                end_minute: 1440,
                                up_kbs: 0,
                                down_kbs: 0,
                                pause: true,
                            }];
                        }
                        Ok(())
                    },
                    change,
                    &FixtureProxySecret(self.root.join("proxy-credential")),
                )?;
                Ok(json!(true))
            }
            "traffic_proxy_matches" => Ok(json!(
                self.network.proxy.read().map_err(error)?.password == text(&request, "password")?
            )),
            "traffic_pending" => Ok(json!(self.network.pacer.pending())),
            "traffic_snapshot" => Ok(json!(self.network.snapshot())),
            "traffic_configure" => {
                self.network
                    .update_at(&self.root.join("network_settings.json"), |snapshot| {
                        let vpn = &mut snapshot.vpn;
                        vpn.enabled = request["vpn"].as_bool().unwrap_or(false);
                        vpn.bandwidth_limit_down_kbs = request["downKbs"]
                            .as_u64()
                            .unwrap_or(0)
                            .try_into()
                            .map_err(error)?;
                        vpn.bandwidth_limit_up_kbs = request["upKbs"]
                            .as_u64()
                            .unwrap_or(0)
                            .try_into()
                            .map_err(error)?;
                        vpn.bandwidth_schedule =
                            request["bandwidth_schedule"].as_bool().unwrap_or(false);
                        vpn.bandwidth_windows = serde_json::from_value(
                            request["windows"]
                                .as_array()
                                .map(|windows| json!(windows))
                                .unwrap_or(json!([])),
                        )
                        .map_err(error)?;
                        Ok(())
                    })?;
                Ok(json!(true))
            }
            "bandwidth_read" => Ok(json!(self.bandwidth.get_stats())),
            "bandwidth_set_limit" => Ok(json!(self
                .bandwidth
                .set_limit(request["bytes"].as_u64().ok_or("Missing limit")?)?)),
            "bandwidth_set_date" => {
                self.bandwidth.set_date(
                    chrono::NaiveDate::parse_from_str(text(&request, "date")?, "%Y-%m-%d")
                        .map_err(error)?,
                );
                Ok(json!(self.bandwidth.get_stats()))
            }
            "bandwidth_resize" => {
                self.bandwidth_hold
                    .as_mut()
                    .ok_or("No quota hold")?
                    .resize(request["bytes"].as_u64().ok_or("Missing bytes")?)?;
                Ok(json!(self.bandwidth.get_stats()))
            }
            "bandwidth_commit_other" => {
                let mut held = crate::bandwidth::BandwidthReservation::download(
                    self.bandwidth.clone(),
                    request["bytes"].as_u64().ok_or("Missing bytes")?,
                )?;
                held.commit();
                Ok(json!(self.bandwidth.get_stats()))
            }
            "bandwidth_hold" => {
                if self.bandwidth_hold.is_some() {
                    return Err("Already holding quota".into());
                }
                let bytes = request["bytes"].as_u64().ok_or("Missing quota bytes")?;
                self.bandwidth_hold = Some(if request["upload"].as_bool().unwrap_or(false) {
                    crate::bandwidth::BandwidthReservation::upload(self.bandwidth.clone(), bytes)?
                } else {
                    crate::bandwidth::BandwidthReservation::download(self.bandwidth.clone(), bytes)?
                });
                Ok(json!(self.bandwidth.get_stats()))
            }
            "bandwidth_commit" => {
                self.bandwidth_hold.take().ok_or("No quota hold")?.commit();
                Ok(json!(self.bandwidth.get_stats()))
            }
            "bandwidth_cancel" => {
                self.bandwidth_hold.take().ok_or("No quota hold")?;
                Ok(json!(self.bandwidth.get_stats()))
            }
            _ => Err("Unknown traffic E2E command".into()),
        }
    }

    async fn dispatch_search(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "search_inventory" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let source = InventorySource {
                    path: child(&self.root, text(&request, "source")?)?,
                    history: self.inventory_history.clone(),
                    calls: self.inventory_calls.clone(),
                    lookups: self.inventory_lookups.clone(),
                    lookup_times: self.inventory_lookup_times.clone(),
                    cached: Arc::new(std::sync::Mutex::new(None)),
                    started: Arc::new(tokio::sync::Notify::new()),
                };
                let folders: Vec<i64> =
                    serde_json::from_value(request["folders"].clone()).map_err(error)?;
                crate::commands::search::verified_folders(&account, &folders)?;
                let source = InventorySearchSource {
                    folders,
                    cache: self.inventories.clone(),
                    source,
                };
                let query = serde_json::from_value(request["query"].clone()).map_err(error)?;
                let mut reply = serde_json::to_value(
                    crate::local_search::search(account, self.crypto.clone(), query, &source)
                        .await?,
                )
                .map_err(error)?;
                reply["historyCalls"] = json!(self
                    .inventory_calls
                    .load(std::sync::atomic::Ordering::SeqCst));
                reply["lookupBatches"] = json!(self
                    .inventory_lookups
                    .load(std::sync::atomic::Ordering::SeqCst));
                Ok(reply)
            }
            "search_runtime" | "search_start" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let folders: Vec<i64> =
                    serde_json::from_value(request["folders"].clone()).map_err(error)?;
                let query = if let Some(id) = request["saved"].as_str() {
                    crate::local_search::Query::saved(
                        &Store::open(&account.root, account.owner)?
                            .record("search", id)?
                            .ok_or("Search not found")?,
                    )?
                } else {
                    serde_json::from_value(request["query"].clone()).map_err(error)?
                };
                let source = StoreSearchSource {
                    folders,
                    credential: self.crypto.current_credential().ok().map(|(id, _)| id),
                    names: request
                        .get("names")
                        .map(|value| serde_json::from_value(value.clone()))
                        .transpose()
                        .map_err(error)?
                        .unwrap_or_default(),
                };
                if request["command"] == "search_start" {
                    crate::local_search::install_build_gate(
                        self.root.join("search-started"),
                        self.root.join("search-release"),
                    );
                    let crypto = self.crypto.clone();
                    self.search_task = Some(tokio::spawn(async move {
                        crate::local_search::search(account, crypto, query, &source).await
                    }));
                    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                    while !self.root.join("search-started").is_file() {
                        if tokio::time::Instant::now() > deadline {
                            return Err("Search did not start".into());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                    Ok(json!(true))
                } else {
                    serde_json::to_value(
                        crate::local_search::search(account, self.crypto.clone(), query, &source)
                            .await?,
                    )
                    .map_err(error)
                }
            }
            "search_abort" => {
                let task = self.search_task.take().ok_or("No search task")?;
                task.abort();
                let _ = task.await;
                Ok(json!(crate::local_search::available_builds()))
            }
            "search_available" => Ok(json!(crate::local_search::available_builds())),
            "search_tag" => {
                Store::open(&self.root, 101)?.tag(
                    &[text(&request, "key")?.into()],
                    text(&request, "tag")?,
                    request["add"].as_bool().unwrap_or(true),
                )?;
                Ok(json!(true))
            }
            "search_assign" => {
                Store::open(&self.root, 101)?.assign(
                    &[text(&request, "key")?.into()],
                    text(&request, "collection")?,
                    true,
                )?;
                Ok(json!(true))
            }
            "search_inventory_changed" => {
                let account = AccountGuard::open(&self.root, Some("101"))?;
                let generation = crate::local_search::inventory_generation(&account, Some(42));
                crate::local_search::inventory_changed(&account, Some(42), &generation);
                Ok(json!(true))
            }
            "search_record" => {
                let store = Store::open(&self.root, 101)?;
                store.put_record(
                    text(&request, "kind")?,
                    text(&request, "id")?,
                    &request["value"],
                )?;
                Ok(json!(true))
            }
            "search_index_build" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let store = Store::open(&account.root, account.owner)?;
                let folders: Vec<i64> =
                    serde_json::from_value(request["folders"].clone()).map_err(error)?;
                self.search_index = Some(crate::local_search::Index::build(
                    account,
                    store.files()?,
                    &folders,
                    None,
                )?);
                Ok(json!(true))
            }
            "search_index_query" => {
                let query = serde_json::from_value(request["query"].clone()).map_err(error)?;
                serde_json::to_value(
                    self.search_index
                        .as_ref()
                        .ok_or("No search index")?
                        .search(&query, &self.crypto, true, true)?,
                )
                .map_err(error)
            }
            "search_index_saved" => {
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let value: crate::workspace::store::SavedSearch =
                    Store::open(&account.root, account.owner)?
                        .record("search", text(&request, "id")?)?
                        .ok_or("Search not found")?;
                let query = crate::local_search::Query::saved(&value)?;
                serde_json::to_value(
                    self.search_index
                        .as_ref()
                        .ok_or("No search index")?
                        .search(&query, &self.crypto, true, true)?,
                )
                .map_err(error)
            }
            _ => Err("Unknown search E2E command".into()),
        }
    }

    async fn dispatch_servers(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "webdav_upload_fixture" => {
                use tokio::io::AsyncReadExt;
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                let source = child(&self.root, text(&request, "source")?)?;
                let target = child(&self.root, text(&request, "target")?)?;
                let count = std::sync::atomic::AtomicU32::new(0);
                let fail = request["failAttempts"].as_u64().unwrap_or(0) as u32;
                let forbidden = request["forbidden"].as_bool().unwrap_or(false);
                let outcome = crate::webdav::retry_upload(
                    2,
                    || async {
                        account
                            .validate()
                            .map_err(|_| dav_server::fs::FsError::Forbidden)?;
                        let attempt = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let mut file = tokio::fs::File::open(&source)
                            .await
                            .map_err(|_| dav_server::fs::FsError::NotFound)?;
                        let mut bytes = Vec::new();
                        file.read_to_end(&mut bytes)
                            .await
                            .map_err(|_| dav_server::fs::FsError::GeneralFailure)?;
                        if forbidden {
                            return Err(dav_server::fs::FsError::Forbidden);
                        }
                        if attempt < fail {
                            return Err(dav_server::fs::FsError::IsRemote);
                        }
                        // Controlled Telegram upload boundary: one committed file.
                        tokio::fs::write(&target, bytes)
                            .await
                            .map_err(|_| dav_server::fs::FsError::GeneralFailure)?;
                        Ok(())
                    },
                    || {
                        account
                            .validate()
                            .map_err(|_| dav_server::fs::FsError::Forbidden)
                    },
                    1,
                    2,
                )
                .await;
                Ok(
                    json!({"attempts":count.load(std::sync::atomic::Ordering::SeqCst),"error":outcome.err().map(|error|format!("{error:?}"))}),
                )
            }
            "start_media_fixture" => {
                let owner = crate::workspace::current_owner(&self.root)?;
                let fixture = MediaFixture {
                    bandwidth: self.bandwidth.clone(),
                    network: self.network.clone(),
                    root: self.root.clone(),
                    source: child(&self.root, text(&request, "source")?)?,
                    filename: text(&request, "filename")?.to_string(),
                    resolutions: Arc::new(AtomicU64::new(0)),
                    resolution_gate: Arc::new(tokio::sync::Notify::new()),
                    cache: Arc::new(crate::server::MediaResolutionCache::new(
                        request["cacheEntries"].as_u64().unwrap_or(64) as usize,
                        std::time::Duration::from_millis(
                            request["cacheTtlMs"].as_u64().unwrap_or(120_000),
                        ),
                    )),
                };
                let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(error)?;
                let address = listener.local_addr().map_err(error)?;
                let crypto = self.crypto.clone();
                let credential = crypto.current_session().and_then(|session| {
                    crypto
                        .create_operation_handle(
                            session,
                            crypto::state::OperationClass::MediaStream,
                        )
                        .ok()
                });
                let server = actix_web::HttpServer::new(move || {
                    actix_web::App::new()
                        .app_data(actix_web::web::Data::new(crypto.clone()))
                        .app_data(actix_web::web::Data::new(fixture.clone()))
                        .route("/media", actix_web::web::get().to(fixture_media))
                        .route(
                            "/resolve-continue",
                            actix_web::web::post().to(
                                |fixture: actix_web::web::Data<MediaFixture>| async move {
                                    fixture.resolution_gate.notify_one();
                                    actix_web::HttpResponse::NoContent().finish()
                                },
                            ),
                        )
                        .route(
                            "/resolutions",
                            actix_web::web::get().to(
                                |fixture: actix_web::web::Data<MediaFixture>| async move {
                                    actix_web::HttpResponse::Ok().json(
                                        fixture
                                            .resolutions
                                            .load(std::sync::atomic::Ordering::SeqCst),
                                    )
                                },
                            ),
                        )
                })
                .workers(2)
                .listen(listener)
                .map_err(error)?
                .run();
                self.servers.push((server.handle(), tokio::spawn(server)));
                Ok(
                    json!({"url": format!("http://{address}/media?owner={owner}"), "credential":credential.map(|value| value.to_string())}),
                )
            }
            "start_ad_server" => {
                let listener = crate::server::bind_ad_listener().map_err(error)?;
                let address = listener.local_addr().map_err(error)?;
                let server = if let Some(url) = request["fixtureUrl"].as_str() {
                    crate::server::start_ad_fixture(listener, url.to_string())
                } else {
                    crate::server::start_ad_server(listener)
                }
                .map_err(error)?;
                self.servers.push((server.handle(), tokio::spawn(server)));
                Ok(
                    json!({"url":format!("http://{address}"),"port":address.port(),"publishedStreamPort":crate::stream_port()}),
                )
            }
            "stop_ad_server" => {
                let (handle, task) = self.servers.pop().ok_or("No separate server")?;
                handle.stop(true).await;
                task.await.map_err(error)?.map_err(error)?;
                Ok(json!({"sponsorPort":crate::server::sponsor_port()}))
            }
            "sponsor_load_verified" => {
                crate::commands::supporter::load_sponsor_fixture(
                    text(&request, "token")?,
                    text(&request, "devicePublicKey")?,
                    text(&request, "servicePublicKey")?,
                    request["now"].as_i64().ok_or("Missing now")?,
                )
                .await?;
                Ok(json!(true))
            }
            "start_stream_server" => {
                // Production bind path: the preferred port, or an assigned one
                // when another process already holds it.
                let preferred = request["preferredPort"]
                    .as_u64()
                    .and_then(|port| u16::try_from(port).ok())
                    .ok_or("Missing preferredPort")?;
                let listener = crate::server::bind_stream_listener(preferred).map_err(error)?;
                let address = listener.local_addr().map_err(error)?;
                let server = crate::server::start_server_with_listener(
                    disconnected(),
                    "synthetic-e2e-stream-token".into(),
                    crate::db::init_db_at(&self.root)?,
                    Arc::new(crate::transcode::TranscodeManager::new(
                        self.root.join("transcode"),
                    )),
                    self.crypto.clone(),
                    self.root.clone(),
                    self.network.clone(),
                    self.bandwidth.clone(),
                    listener,
                )
                .map_err(error)?;
                self.servers.push((server.handle(), tokio::spawn(server)));
                Ok(json!({
                    "url": format!("http://{address}"),
                    "port": address.port(),
                    "publishedPort": crate::stream_port(),
                }))
            }
            "api_seed_catalog" => {
                // Controlled Telegram boundary: what walking the signed-in
                // account's folders would have returned.
                let owner = crate::workspace::current_owner(&self.root)?;
                let folders = request["folders"]
                    .as_array()
                    .ok_or("Missing folders")?
                    .iter()
                    .map(|folder| crate::api_catalog::Folder {
                        id: folder["id"].as_i64(),
                        name: folder["name"].as_str().unwrap_or_default().to_string(),
                    })
                    .collect();
                let files = request["files"]
                    .as_array()
                    .ok_or("Missing files")?
                    .iter()
                    .map(|file| {
                        let created = chrono::DateTime::parse_from_rfc3339(
                            file["created"].as_str().ok_or("Missing created")?,
                        )
                        .map_err(error)?
                        .with_timezone(&chrono::Utc);
                        let name = file["name"].as_str().ok_or("Missing name")?.to_string();
                        Ok(crate::api_catalog::ApiFile {
                            id: file["id"].as_i64().ok_or("Missing id")?,
                            folder_id: file["folder"].as_i64(),
                            document_name: file["uploadedAs"].as_str().unwrap_or(&name).to_string(),
                            name,
                            size: file["size"].as_u64().ok_or("Missing size")?,
                            mime_type: file["mime"].as_str().map(str::to_string),
                            created_at: crate::api_catalog::rfc3339(created),
                            encrypted: file["encrypted"].as_bool().unwrap_or(false),
                            timestamp: created.timestamp(),
                            is_document: true,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                crate::api_catalog::seed(
                    owner,
                    folders,
                    files,
                    request["complete"].as_bool().unwrap_or(true),
                );
                Ok(json!(true))
            }
            "supporter_verify" => crate::commands::supporter::verify_issued_token(
                text(&request, "token")?,
                text(&request, "devicePublicKey")?,
                text(&request, "servicePublicKey")?,
                request["now"].as_i64().ok_or("Missing now")?,
            ),
            "start_webdav" => {
                // The WebDAV server exactly as the application serves it;
                // only the Telegram connection is absent.
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(error)?;
                let address = listener.local_addr().map_err(error)?;
                let staging = self.root.join("webdav-staging");
                std::fs::create_dir_all(&staging).map_err(error)?;
                let state = disconnected();
                if request["retainSession"] == true {
                    // Match a live runner's session lifetime, without a Telegram client.
                    let session = crate::workspace::open_session(&self.root).map_err(error)?;
                    crate::workspace::register_session(&self.root, &session)?;
                    *state.session.lock().await = Some(session);
                    self.retained_sessions.push(state.clone());
                }
                let filesystem = crate::webdav::TelegramDavFs::new(
                    state,
                    self.bandwidth.clone(),
                    self.network.clone(),
                    request["writeEnabled"].as_bool().unwrap_or(false),
                    staging,
                    self.root.clone(),
                )
                .with_remote_error(request["remoteError"].as_str().map(str::to_string))
                .with_catalog_fixture(request["catalogFixture"] == true);
                let server = crate::webdav::serve(
                    listener,
                    filesystem,
                    crate::commands::webdav_settings::hash_token(text(&request, "token")?),
                )
                .map_err(error)?;
                self.servers.push((server.handle(), tokio::spawn(server)));
                Ok(json!({"url":format!("http://{address}")}))
            }
            "release_retained_sessions" => {
                let mut released = 0;
                for state in self.retained_sessions.drain(..) {
                    if state.session.lock().await.take().is_some() {
                        released += 1;
                    }
                }
                Ok(json!(released))
            }
            "start_api" => {
                // The REST API server exactly as the application serves it;
                // only the Telegram connection is absent.
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(error)?;
                let address = listener.local_addr().map_err(error)?;
                let server = crate::api_routes::serve(
                    listener,
                    crate::api_routes::ApiServerParts {
                        telegram: disconnected(),
                        key_hash: request["key"]
                            .as_str()
                            .map(crate::commands::api_settings::hash_key),
                        cache_dirs: crate::api_routes::CacheDirs {
                            account_root: self.root.clone(),
                            thumbnail_dir: self.root.join("thumbnails"),
                            preview_dir: self.root.join("previews"),
                        },
                        bandwidth: self.bandwidth.clone(),
                        network: self.network.clone(),
                        database: crate::db::init_db_at(&self.root)?,
                    },
                )
                .map_err(error)?;
                self.servers.push((server.handle(), tokio::spawn(server)));
                Ok(json!({"url":format!("http://{address}")}))
            }
            "start_http" => {
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(error)?;
                let address = listener.local_addr().map_err(error)?;
                let server = crate::server::start_server_with_listener(
                    disconnected(),
                    "synthetic-e2e-stream-token".into(),
                    crate::db::init_db_at(&self.root)?,
                    Arc::new(crate::transcode::TranscodeManager::new(
                        self.root.join("transcode"),
                    )),
                    self.crypto.clone(),
                    self.root.clone(),
                    self.network.clone(),
                    self.bandwidth.clone(),
                    listener,
                )
                .map_err(error)?;
                self.servers.push((server.handle(), tokio::spawn(server)));
                Ok(json!({"url":format!("http://{address}")}))
            }
            _ => Err("Unknown servers E2E command".into()),
        }
    }

    async fn dispatch_peers(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "peer_queued_clear" => {
                let cache = Arc::new(RwLock::new(HashMap::<i64, String>::new()));
                let (started, ready) = tokio::sync::oneshot::channel();
                let (resume, continued) = tokio::sync::oneshot::channel();
                let first_cache = cache.clone();
                let first = tokio::spawn(async move {
                    crate::commands::utils::resolve_cached_peer(&first_cache, 1, || async {
                        let _ = started.send(());
                        let _ = continued.await;
                        Ok(HashMap::from([(1, "old first".to_string())]))
                    })
                    .await
                });
                ready.await.map_err(error)?;
                let second_cache = cache.clone();
                let second = tokio::spawn(async move {
                    crate::commands::utils::resolve_cached_peer(&second_cache, 2, || async {
                        Ok(HashMap::from([(2, "old queued".to_string())]))
                    })
                    .await
                });
                tokio::time::sleep(Duration::from_millis(20)).await;
                crate::commands::utils::clear_cached_peers(&cache).await;
                let _ = resume.send(());
                Ok(json!({"first":first.await.map_err(error)?.is_err(),
                    "second":second.await.map_err(error)?.is_err(), "cache":*cache.read().await}))
            }
            "peer_scan_seed" => {
                self.scan_peers.write().await.insert(1, "existing".into());
                Ok(json!(true))
            }
            "peer_scan_start" => {
                if self.scan_task.is_some() {
                    return Err("Scan already running".into());
                }
                let account = AccountGuard::open(&self.root, None)?;
                let cache = self.scan_peers.clone();
                let fail = request["fail"].as_bool().unwrap_or(false);
                let resolve = request["resolve"] == true;
                let target = request["target"].as_i64().unwrap_or(3);
                let (ready_send, ready) = tokio::sync::oneshot::channel();
                let (resume, continued) = tokio::sync::oneshot::channel();
                self.scan_continue = Some(resume);
                self.scan_task = Some(tokio::spawn(async move {
                    if resolve {
                        return crate::commands::utils::resolve_cached_peer(
                            &cache,
                            target,
                            || async {
                                let _ = ready_send.send(());
                                let _ = continued.await;
                                account.validate()?;
                                if fail {
                                    return Err("Dialog walk interrupted".into());
                                }
                                Ok(HashMap::from([
                                    (2, "discovered".into()),
                                    (3, "complete".into()),
                                ]))
                            },
                        )
                        .await
                        .map(|_| 1);
                    }
                    let dialogs = async_stream::stream! {
                        yield Ok((2, "discovered".to_string()));
                        let _ = ready_send.send(());
                        let _ = continued.await;
                        if fail { yield Err("Dialog walk interrupted".into()); }
                        else { yield Ok((3, "complete".to_string())); }
                    };
                    crate::commands::utils::scan_peer_snapshot(&cache, dialogs, Some, &account)
                        .await
                }));
                ready.await.map_err(error)?;
                Ok(json!(true))
            }
            "peer_scan_read" => {
                let peers = tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    self.scan_peers.read(),
                )
                .await
                .map_err(|_| "Peer cache blocked by dialog walk".to_string())?;
                Ok(json!(*peers))
            }
            "peer_scan_finish" => {
                self.scan_continue
                    .take()
                    .ok_or("No scan")?
                    .send(())
                    .map_err(|_| "Scan stopped".to_string())?;
                let result = self
                    .scan_task
                    .take()
                    .ok_or("No scan")?
                    .await
                    .map_err(error)??;
                Ok(json!(result))
            }
            _ => Err("Unknown peers E2E command".into()),
        }
    }

    async fn dispatch_crypto(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "vault_cleanup_fault" => {
                self.crypto.test_cleanup_failure(request["fail"] == true);
                Ok(json!(true))
            }
            "vault_prepare_start" => {
                let id = text(&request, "id")?.to_string();
                if self.vault_tasks.contains_key(&id) {
                    return Err("Preparation already running".into());
                }
                let started = child(&self.root, &format!("prepare-{id}.started"))?;
                let release = child(&self.root, &format!("prepare-{id}.release"))?;
                if request["held"] == true {
                    self.crypto.install_prepare_gate(started.clone(), release);
                }
                let state = self.crypto.clone();
                let passphrase = crypto::secret::SecretBytes::from_slice(
                    text(&request, "passphrase")?.as_bytes(),
                );
                let current = crypto::secret::SecretBytes::from_slice(
                    request["currentPassphrase"]
                        .as_str()
                        .unwrap_or_default()
                        .as_bytes(),
                );
                let vault_passphrase = request["vaultPassphrase"]
                    .as_str()
                    .map(|value| crypto::secret::SecretBytes::from_slice(value.as_bytes()));
                let replace_existing = request["replaceExisting"].as_bool().unwrap_or(true);
                let operation = request["operation"]
                    .as_str()
                    .unwrap_or("unlock")
                    .to_string();
                let bundle = if matches!(operation.as_str(), "import" | "verify") {
                    std::fs::read(self.root.join("recovery.bundle")).map_err(error)?
                } else {
                    Vec::new()
                };
                let task = tokio::spawn(async move {
                    match operation.as_str() {
                        "unlock" => state.unlock(passphrase.expose()).await.map(|_| json!(true)),
                        "create" => state
                            .create_vault(passphrase.expose())
                            .await
                            .map(|_| json!(true)),
                        "change" => state
                            .change_vault_passphrase(current.expose(), passphrase.expose())
                            .await
                            .map(|_| json!(true)),
                        "import" => state
                            .import_recovery(
                                &bundle,
                                passphrase.expose(),
                                vault_passphrase
                                    .as_ref()
                                    .map(crypto::secret::SecretBytes::expose),
                                false,
                                replace_existing,
                            )
                            .await
                            .map(|_| json!(true)),
                        "export" => state
                            .export_recovery(passphrase.expose())
                            .await
                            .map(|bytes| json!({"bytes":bytes.len()})),
                        "verify" => state
                            .verify_recovery(&bundle, passphrase.expose())
                            .await
                            .map(|value| json!(value)),
                        _ => Err(crypto::error::CryptoError::internal(
                            "Unknown preparation operation",
                        )),
                    }
                    .map_err(error)
                });
                self.vault_tasks.insert(id, task);
                if request["held"] == true {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                    while !started.is_file() {
                        if tokio::time::Instant::now() >= deadline {
                            return Err("Preparation did not start".into());
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
                Ok(json!(true))
            }
            "vault_prepare_abort" => {
                let task = self
                    .vault_tasks
                    .remove(text(&request, "id")?)
                    .ok_or("No preparation")?;
                task.abort();
                let _ = task.await;
                Ok(json!(true))
            }
            "vault_prepare_finish" => self
                .vault_tasks
                .remove(text(&request, "id")?)
                .ok_or("No preparation")?
                .await
                .map_err(error)?,
            "vault_status_responsive" => {
                let state = self.crypto.clone();
                let status = tokio::time::timeout(
                    Duration::from_millis(400),
                    tokio::task::spawn_blocking(move || state.is_locked()),
                )
                .await;
                Ok(json!({"responsive":status.is_ok()}))
            }
            "vault_create" => {
                self.crypto
                    .create_vault(text(&request, "passphrase")?.as_bytes())
                    .await
                    .map_err(error)?;
                Ok(json!(true))
            }
            "vault_unlock" => {
                self.crypto
                    .unlock(text(&request, "passphrase")?.as_bytes())
                    .await
                    .map_err(error)?;
                Ok(json!(true))
            }
            "vault_auto_timeout" => {
                self.crypto
                    .set_auto_lock_timeout(Some(std::time::Duration::from_millis(
                        request["milliseconds"].as_u64().ok_or("Missing timeout")?,
                    )));
                Ok(json!(true))
            }
            "vault_auto_due" => Ok(json!(self.crypto.lock_if_auto_lock_due())),
            "vault_lock" => {
                self.crypto.lock();
                Ok(json!(true))
            }
            "vault_change_passphrase" => {
                self.crypto
                    .change_vault_passphrase(
                        request["currentPassphrase"]
                            .as_str()
                            .unwrap_or_default()
                            .as_bytes(),
                        text(&request, "passphrase")?.as_bytes(),
                    )
                    .await
                    .map_err(error)?;
                Ok(json!(true))
            }
            "vault_export" => {
                let bundle = self
                    .crypto
                    .export_recovery(text(&request, "passphrase")?.as_bytes())
                    .await
                    .map_err(error)?;
                std::fs::write(self.root.join("recovery.bundle"), bundle).map_err(error)?;
                Ok(json!(true))
            }
            "vault_recover" => {
                let bundle = std::fs::read(self.root.join("recovery.bundle")).map_err(error)?;
                self.crypto
                    .import_recovery(
                        &bundle,
                        text(&request, "passphrase")?.as_bytes(),
                        request["vaultPassphrase"].as_str().map(str::as_bytes),
                        request["allowKeyReplacement"].as_bool().unwrap_or(false),
                        request["replaceExisting"].as_bool().unwrap_or(true),
                    )
                    .await
                    .map_err(error)?;
                Ok(json!(true))
            }
            "vault_verify_recovery" => {
                let bundle = std::fs::read(self.root.join("recovery.bundle")).map_err(error)?;
                let verification = self
                    .crypto
                    .verify_recovery(&bundle, text(&request, "passphrase")?.as_bytes())
                    .await
                    .map_err(error)?;
                serde_json::to_value(verification).map_err(error)
            }
            "vault_identity" => Ok(json!(self.crypto.vault_identity())),
            "vault_save_profile" => {
                // Adds a profile wrapping key the way a protected upload does,
                // so a bundle exported earlier no longer covers every key.
                self.crypto
                    .save_profile(
                        text(&request, "profile")?,
                        SecretKey::new(crypto::random::random_key()),
                    )
                    .map_err(error)?;
                Ok(json!(true))
            }
            "envelope_known_answer" => self.known_answer(&request).await,
            "fixture_encrypt" => self.create_encrypted_fixture(&request).await,
            "read_envelope" => self.read_envelope(&request).await,
            _ => Err("Unknown crypto E2E command".into()),
        }
    }

    async fn dispatch_folders(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "folder_create" | "folder_delete" | "folder_rename" => {
                // Production folder commands against a state with no Telegram
                // client; they must fail without touching the local database.
                let state = disconnected();
                let db = crate::db::init_db_at(&self.root)?;
                let id = request["id"].as_i64().unwrap_or(0);
                match text(&request, "command")? {
                    "folder_create" => crate::commands::fs::create_folder(
                        text(&request, "name")?.to_string(),
                        &state,
                        db,
                    )
                    .await
                    .map(|folder| json!({"id": folder.id})),
                    "folder_delete" => crate::commands::fs::delete_folder(id, &state, db)
                        .await
                        .map(|done| json!(done)),
                    _ => crate::commands::fs::rename_folder(
                        id,
                        text(&request, "name")?.to_string(),
                        &state,
                        db,
                    )
                    .await
                    .map(|done| json!(done)),
                }
            }
            "folder_layout" => {
                // What the signed-in account sees: its own folders and groups.
                let owner = crate::workspace::current_owner(&self.root).ok();
                let scope = Some((self.root.clone(), owner));
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, move |connection| {
                    let readable =
                        crate::commands::folder_groups::prepare_layout(connection, scope.as_ref())?;
                    if !readable {
                        return Ok(json!({"folders": [], "groups": []}));
                    }
                    let folders = crate::commands::folder_groups::enriched_folders(connection)?;
                    let groups = crate::commands::folder_groups::list_groups(connection)?;
                    Ok(json!({
                        "folders": folders.iter().map(|folder| json!({"id": folder.id, "name": folder.name, "group": folder.group_id})).collect::<Vec<_>>(),
                        "groups": groups.iter().map(|group| group.name.clone()).collect::<Vec<_>>(),
                    }))
                })
                .await
            }
            "folder_scan" => {
                // A Telegram folder scan for the signed-in account; the list of
                // channels is the controlled fixture.
                let owner = crate::workspace::current_owner(&self.root).ok();
                let scope = Some((self.root.clone(), owner));
                let folders: Vec<crate::models::FolderMetadata> = request["folders"]
                    .as_array()
                    .ok_or("Missing folders")?
                    .iter()
                    .map(|folder| crate::models::FolderMetadata {
                        id: folder["id"].as_i64().unwrap_or_default(),
                        parent_id: None,
                        name: folder["name"].as_str().unwrap_or_default().to_string(),
                        username: None,
                        is_public: false,
                        group_id: None,
                        display_order: 0,
                    })
                    .collect();
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, move |connection| {
                    crate::commands::folder_groups::prepare_layout(connection, scope.as_ref())?;
                    crate::commands::folder_groups::get_enriched_folders_internal(
                        connection, folders,
                    )
                    .map(|folders| json!(folders.len()))
                })
                .await
            }
            "group_create" => {
                let name = text(&request, "name")?.to_string();
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, move |connection| {
                    crate::commands::folder_groups::create_group(connection, &name, "#3B82F6")
                        .map(|id| json!(id))
                })
                .await
            }
            "folder_assign" => {
                let channel = request["folder"].as_i64().ok_or("Missing folder")?;
                let group = request["group"].as_i64().map(|group| group as i32);
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, move |connection| {
                    crate::commands::folder_groups::assign_folder_to_group(
                        connection, channel, group,
                    )
                    .map(|()| json!(true))
                })
                .await
            }
            "folder_sign_out" => {
                // The folder-layout part of sign-out.
                let owner = crate::workspace::current_owner(&self.root).ok();
                let root = self.root.clone();
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, move |connection| {
                    crate::folder_layout::deactivate(connection, &root, owner).map(|()| json!(true))
                })
                .await
            }
            "folder_rows" => {
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, |connection| {
                    let mut query = connection
                        .prepare("SELECT channel_id, name FROM folder_metadata ORDER BY channel_id")
                        .map_err(error)?;
                    let mut rows = Vec::new();
                    while let sqlite::State::Row = query.next().map_err(error)? {
                        rows.push(json!({
                            "id": query.read::<i64, _>(0).map_err(error)?,
                            "name": query.read::<String, _>(1).map_err(error)?,
                        }));
                    }
                    Ok(json!(rows))
                })
                .await
            }
            "seed_folder_row" => {
                let db = crate::db::init_db_at(&self.root)?;
                let id = request["id"].as_i64().ok_or("Missing id")?;
                let name = text(&request, "name")?.to_string();
                crate::db::with_connection(db, move |connection| {
                    let mut insert = connection
                        .prepare("INSERT INTO folder_metadata (channel_id, name, username, is_public, display_order, group_id) VALUES (?, ?, NULL, 0, 0, NULL)")
                        .map_err(error)?;
                    insert.bind((1, id)).map_err(error)?;
                    insert.bind((2, name.as_str())).map_err(error)?;
                    insert.next().map_err(error)?;
                    Ok(json!(true))
                })
                .await
            }
            _ => Err("Unknown folders E2E command".into()),
        }
    }

    async fn dispatch_diagnostics(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "staging_root" => {
                // The production staging directory rules, rooted in the fixture.
                let parent = child(&self.root, text(&request, "parent")?)?;
                std::fs::create_dir_all(&parent).map_err(error)?;
                let root = crate::temp_artifacts::staging_root_in(&parent).map_err(error)?;
                if let Some(seconds) = request["sweepOlderThanSeconds"].as_u64() {
                    crate::temp_artifacts::sweep_stale_staging_in(
                        &root,
                        std::time::Duration::from_secs(seconds),
                    );
                }
                Ok(json!({"name": root.file_name().and_then(|name| name.to_str())}))
            }
            "log_start" => {
                // The production logger writing into the fixture directory.
                crate::app_log::init();
                let path = crate::app_log::attach_file(&self.root.join("logs"))?;
                Ok(json!({"path": path.file_name().and_then(|name| name.to_str())}))
            }
            "log_emit" => {
                let message = text(&request, "message")?;
                match text(&request, "level")? {
                    "error" => log::error!("{message}"),
                    "warn" => log::warn!("{message}"),
                    "info" => log::info!("{message}"),
                    _ => return Err("Unknown log level".into()),
                }
                for _ in 1..request["repeat"].as_u64().unwrap_or(1) {
                    log::warn!("{message}");
                }
                log::logger().flush();
                Ok(json!(true))
            }
            "startup_failure_message" => {
                // The text of the startup-failure report for a real setup error.
                let log_file = crate::app_log::file_path();
                Ok(json!(crate::startup_failure::describe(
                    text(&request, "error")?,
                    log_file.as_deref(),
                )))
            }
            _ => Err("Unknown diagnostics E2E command".into()),
        }
    }

    async fn dispatch_sync(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "sync_seed_pair" => {
                // A mapping row exactly as the application stores it. The local
                // folder is a real directory inside the fixture.
                let local = child(&self.root, text(&request, "folder")?)?;
                std::fs::create_dir_all(&local).map_err(error)?;
                let local_path = local.canonicalize().map_err(error)?;
                let local_path = local_path
                    .to_str()
                    .ok_or("Fixture path is not UTF-8")?
                    .to_string();
                let direction = request["direction"]
                    .as_str()
                    .unwrap_or("bidirectional")
                    .to_string();
                let policy = StoredPairPolicy {
                    account_owner: Some("101".into()),
                    preferences: serde_json::from_value::<sync_engine::policy::SyncPreferences>(
                        request
                            .get("preferences")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    )
                    .map_err(error)?
                    .validated()?,
                };
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, move |connection| {
                    let mut insert = connection
                        .prepare("INSERT INTO sync_pairs (local_path, channel_id, folder_key, label, sync_direction, is_active, created_at) VALUES (?, 4242, '4242', NULL, ?, 1, 1)")
                        .map_err(error)?;
                    insert.bind((1, local_path.as_str())).map_err(error)?;
                    insert.bind((2, direction.as_str())).map_err(error)?;
                    insert.next().map_err(error)?;
                    drop(insert);
                    let mut id = connection.prepare("SELECT last_insert_rowid()").map_err(error)?;
                    id.next().map_err(error)?;
                    let id = id.read::<i64, _>(0).map_err(error)?;
                    sync_config::write_pair_policy(connection, id, &policy)?;
                    Ok(json!({"pairId": id}))
                })
                .await
            }
            "sync_set_preferences" => {
                let pair_id = request["pairId"].as_i64().ok_or("Missing pairId")?;
                let policy = StoredPairPolicy {
                    account_owner: Some("101".into()),
                    preferences: serde_json::from_value::<sync_engine::policy::SyncPreferences>(
                        request["preferences"].clone(),
                    )
                    .map_err(error)?
                    .validated()?,
                };
                sync_config::save_pair_policy(crate::db::init_db_at(&self.root)?, pair_id, &policy)
                    .await?;
                Ok(json!(true))
            }
            "sync_cycle" => {
                // Production scanning, baselining and planning against real
                // files and SQLite. Only the list of Telegram file messages is
                // supplied by the journey.
                let pair_id = request["pairId"].as_i64().ok_or("Missing pairId")?;
                let db = crate::db::init_db_at(&self.root)?;
                let pair = sync_config::load_pairs(db.clone(), false)
                    .await?
                    .into_iter()
                    .find(|pair| pair.id == pair_id)
                    .ok_or("Sync mapping was not found")?;
                let mut files: Vec<sync_engine::RemoteFile> =
                    serde_json::from_value(request["remoteFiles"].clone()).map_err(error)?;
                // A protected file is placed by the path inside its envelope,
                // read with the vault exactly as a scan does. The journey
                // supplies the envelope a scan would fetch from Telegram.
                let vault_key = self.crypto.get_current_wrapping_key().ok();
                for (file, described) in files.iter_mut().zip(
                    request["remoteFiles"]
                        .as_array()
                        .ok_or("Missing remoteFiles")?,
                ) {
                    let (Some(envelope), Some(vault_key)) =
                        (described["envelope"].as_str(), vault_key.as_ref())
                    else {
                        continue;
                    };
                    let bytes = tokio::fs::read(child(&self.root, envelope)?)
                        .await
                        .map_err(error)?;
                    let header = EnvelopeHeader::parse(&bytes).map_err(error)?;
                    let metadata = crate::commands::fs::protected_sync_metadata(
                        &bytes[..header.core.header_length as usize],
                        vault_key,
                    )?;
                    file.sync_path = metadata.sync_path;
                    file.plaintext_size = Some(metadata.plaintext_size);
                }
                let mode = match request["scanner"].as_str().unwrap_or("full") {
                    "full" => sync_engine::LocalScanMode::HashEveryFile,
                    "incremental" => sync_engine::LocalScanMode::ReuseRecordedHashes,
                    _ => return Err("Unknown scanner".into()),
                };
                let synced = sync_engine::load_synced_tree(&db, pair_id).await?;
                let remote = sync_engine::build_remote_tree(&files, &synced, &pair.preferences)?;
                let mut prepared =
                    sync_engine::prepare_reconciliation(&db, &pair, mode, synced, remote).await?;
                sync_engine::drop_settled_skips(&mut prepared);
                let summary = json!({
                    "operations": serde_json::to_value(&prepared.operations).map_err(error)?,
                    "conflicts": prepared.conflicts,
                    "hashed": prepared.hashed,
                    "reused": prepared.reused,
                    "baselined": prepared.baselined,
                    "localFiles": prepared.local.len(),
                    "remoteFiles": prepared.remote.len(),
                });
                self.planned_sync = Some((pair_id, prepared));
                Ok(summary)
            }
            "sync_record" => {
                // Persist the outcome of the planned cycle. The executor's
                // results and the read-back uploaded messages stand in for
                // Telegram; an empty `uploadedFiles` models an engine that
                // stopped before it could verify its uploads.
                let (pair_id, prepared) = self.planned_sync.take().ok_or("Run sync_cycle first")?;
                let db = crate::db::init_db_at(&self.root)?;
                let results: Vec<sync_engine::executor::ExecutionResult> =
                    serde_json::from_value(request["results"].clone()).map_err(error)?;
                let uploads =
                    sync_engine::journal_uploads(&db, pair_id, &prepared.local, &results).await?;
                let uploaded_files: Vec<sync_engine::RemoteFile> = serde_json::from_value(
                    request
                        .get("uploadedFiles")
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                )
                .map_err(error)?;
                let uploaded = sync_engine::uploaded_tree(&uploads, &uploaded_files);
                sync_engine::record_results(&db, pair_id, &prepared, results, &uploaded).await?;
                Ok(json!({"journaledUploads": uploads.len(), "verifiedUploads": uploaded.len()}))
            }
            "sync_state" => {
                let pair_id = request["pairId"].as_i64().ok_or("Missing pairId")?;
                let db = crate::db::init_db_at(&self.root)?;
                crate::db::with_connection(db, move |connection| {
                    let mut query = connection
                        .prepare("SELECT relative_path, sync_status, message_id, remote_hash, local_mtime, file_size FROM sync_state WHERE pair_id = ? ORDER BY relative_path")
                        .map_err(error)?;
                    query.bind((1, pair_id)).map_err(error)?;
                    let mut rows = Vec::new();
                    while let sqlite::State::Row = query.next().map_err(error)? {
                        rows.push(json!({
                            "path": query.read::<String, _>(0).map_err(error)?,
                            "status": query.read::<String, _>(1).map_err(error)?,
                            "messageId": query.read::<Option<i64>, _>(2).map_err(error)?,
                            "remoteHashRecorded": query.read::<Option<String>, _>(3).map_err(error)?.is_some_and(|hash| !hash.is_empty()),
                            "localMtimeRecorded": query.read::<Option<i64>, _>(4).map_err(error)?.is_some(),
                            "fileSize": query.read::<i64, _>(5).map_err(error)?,
                        }));
                    }
                    Ok(json!(rows))
                })
                .await
            }
            _ => Err("Unknown sync E2E command".into()),
        }
    }

    async fn finish_video_upload(&mut self) -> Result<Value, String> {
        let journey = self
            .video_upload
            .take()
            .ok_or("No video upload is running")?;
        // Keep the sender alive while the production operation is awaiting I/O.
        let _cancel = journey.cancel;
        journey.task.await.map_err(error)?
    }

    async fn dispatch_transfers(&mut self, request: Value) -> Result<Value, String> {
        match text(&request, "command")? {
            "video_upload_prepare" | "video_upload_start" => {
                if self.video_upload.is_some() {
                    return Err("A video upload is already running".into());
                }
                let account = AccountGuard::open(&self.root, Some(text(&request, "owner")?))?;
                account.validate()?;
                let source = child(&self.root, text(&request, "source")?)?;
                let name = text(&request, "source")?.to_string();
                let mode = text(&request, "mode")? == "media";
                let filename_caption = request["filenameCaption"].as_bool();
                let transport = Arc::new(FixtureVideoUploadTransport {
                    root: self.root.clone(),
                    block_thumbnail: request["blockThumbnail"] == true,
                    block_main_upload: request["blockMainUpload"] == true,
                    fail_thumbnail: request["failThumbnail"] == true,
                    fail_publish: request["failPublish"] == true,
                    release: tokio::sync::Notify::new(),
                    thumbnail_requests: AtomicU64::new(0),
                    preparation_completed: std::sync::atomic::AtomicBool::new(false),
                    main_upload_requests: AtomicU64::new(0),
                    send_attempts: AtomicU64::new(0),
                    capture: std::sync::Mutex::new(Value::Null),
                });
                let (cancel_tx, mut cancel) = tokio::sync::oneshot::channel();
                let network = self.network.clone();
                let bandwidth = self.bandwidth.clone();
                let boundary = transport.clone();
                let task = tokio::spawn(async move {
                    let result = async {
                        let Some(video) = crate::desktop_video_upload::prepare_for_account(
                            source.to_str().ok_or("Invalid source path")?,
                            &name,
                            mode,
                            "video/mp4",
                            None,
                            bandwidth.clone(),
                            &account,
                            &mut cancel,
                        )
                        .await?
                        else {
                            return Ok(json!({"video":false}));
                        };
                        boundary
                            .preparation_completed
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        // Hold the controlled main Telegram upload boundary after
                        // production preparation has admitted its preview. No file
                        // part handle or publication exists until that boundary returns.
                        boundary.main_upload_requests.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                        if boundary.block_main_upload {
                            tokio::select! {
                                biased;
                                _ = &mut cancel => return Err("Transfer cancelled".into()),
                                result = async {loop {
                                    account.validate()?;
                                    tokio::time::sleep(Duration::from_millis(50)).await;
                                } #[allow(unreachable_code)] Ok::<(),String>(())} => {result?; unreachable!()},
                                _ = boundary.release.notified() => {},
                            }
                        }
                        let length = std::fs::metadata(&source).map_err(error)?.len();
                        // File-part upload is a controlled external boundary; the
                        // main payload need not be transferred again to inspect publication.
                        let raw: grammers_tl_types::enums::InputFile = if length > 10 * 1024 * 1024
                        {
                            grammers_tl_types::types::InputFileBig {
                                id: 101,
                                parts: i32::try_from(length.div_ceil(512 * 1024)).map_err(error)?,
                                name: name.clone(),
                            }
                            .into()
                        } else {
                            grammers_tl_types::types::InputFile {
                                id: 101,
                                parts: 1,
                                name: name.clone(),
                                md5_checksum: "controlled-telegram-part".into(),
                            }
                            .into()
                        };
                        let receipt = crate::desktop_video_upload::publish(
                            &video,
                            grammers_client::types::media::Uploaded::from_raw(raw),
                            crate::desktop_video_upload::caption(&name, filename_caption),
                            boundary.as_ref(),
                            &account,
                            bandwidth.clone(),
                            network.clone(),
                            &mut cancel,
                        )
                        .await?;
                        Ok::<_, String>(json!({"video":true,"receipt":receipt}))
                    }
                    .await;
                    let mut capture = boundary.capture.lock().map_err(error)?.clone();
                    if capture.is_null() {
                        capture = json!({});
                    }
                    capture["result"] = match result {
                        Ok(value) => value,
                        Err(error) => json!({"error":error}),
                    };
                    capture["preparationCompleted"] = json!(boundary
                        .preparation_completed
                        .load(std::sync::atomic::Ordering::SeqCst));
                    capture["mainUploadRequests"] = json!(boundary
                        .main_upload_requests
                        .load(std::sync::atomic::Ordering::SeqCst));
                    capture["thumbnailRequests"] = json!(boundary
                        .thumbnail_requests
                        .load(std::sync::atomic::Ordering::SeqCst));
                    capture["sendAttempts"] = json!(boundary
                        .send_attempts
                        .load(std::sync::atomic::Ordering::SeqCst));
                    capture["thumbnail"] =
                        if boundary.root.join("video-upload-thumbnail.jpg").exists() {
                            json!("video-upload-thumbnail.jpg")
                        } else {
                            Value::Null
                        };
                    capture["upBytes"] = json!(bandwidth.checked_stats()?.up_bytes);
                    capture["pacerPending"] = json!(network.pacer.pending());
                    Ok(capture)
                });
                self.video_upload = Some(FixtureVideoUploadJourney {
                    transport,
                    cancel: Some(cancel_tx),
                    task,
                });
                if text(&request, "command")? == "video_upload_start" {
                    Ok(json!(true))
                } else {
                    self.finish_video_upload().await
                }
            }
            "video_upload_observe" => {
                let journey = self
                    .video_upload
                    .as_ref()
                    .ok_or("No video upload is running")?;
                Ok(
                    json!({"thumbnailRequests":journey.transport.thumbnail_requests.load(std::sync::atomic::Ordering::SeqCst),
                    "mainUploadRequests":journey.transport.main_upload_requests.load(std::sync::atomic::Ordering::SeqCst),
                    "preparationCompleted":journey.transport.preparation_completed.load(std::sync::atomic::Ordering::SeqCst),
                    "sendAttempts":journey.transport.send_attempts.load(std::sync::atomic::Ordering::SeqCst),
                    "finished":journey.task.is_finished(),"upBytes":self.bandwidth.checked_stats()?.up_bytes}),
                )
            }
            "video_upload_release" => {
                self.video_upload
                    .as_ref()
                    .ok_or("No video upload is running")?
                    .transport
                    .release
                    .notify_one();
                Ok(json!(true))
            }
            "video_upload_cancel" => {
                if let Some(sender) = self
                    .video_upload
                    .as_mut()
                    .ok_or("No video upload is running")?
                    .cancel
                    .take()
                {
                    let _ = sender.send(());
                }
                Ok(json!(true))
            }
            "video_upload_finish" => self.finish_video_upload().await,
            "upload_resumable" => self.upload_resumable(&request).await,
            "upload_session" => {
                let resumable = self.resumable(&request).await?;
                Ok(match resumable.session().await? {
                    Some(session) => json!({
                        "fileId": session.file_id,
                        "totalParts": session.total_parts,
                        "completedParts": session.completed_parts,
                        "remoteName": session.remote_name,
                        "protected": session.envelope_header.is_some(),
                    }),
                    None => Value::Null,
                })
            }
            "upload_assemble" => {
                // Join the stored parts in order, as Telegram does when the
                // file is attached to a message.
                let directory = child(&self.root, text(&request, "sink")?)?;
                let file_id = request["fileId"].as_i64().ok_or("Missing fileId")?;
                let total_parts = request["totalParts"].as_i64().ok_or("Missing totalParts")?;
                let mut assembled = Vec::new();
                for part in 0..total_parts {
                    assembled.extend(
                        tokio::fs::read(directory.join(format!("{file_id}-{part}.part")))
                            .await
                            .map_err(|_| format!("Part {part} was never stored"))?,
                    );
                }
                let bytes = assembled.len();
                tokio::fs::write(
                    child(&self.root, text(&request, "destination")?)?,
                    assembled,
                )
                .await
                .map_err(error)?;
                Ok(json!({"bytes": bytes}))
            }
            "publish_download" => {
                let source = child(&self.root, text(&request, "source")?)?;
                let destination = child(&self.root, text(&request, "destination")?)?;
                let policy = serde_json::from_value(request["policy"].clone()).map_err(error)?;
                let result = download_destination::publish_download_file(
                    source,
                    destination,
                    policy,
                    self.guard.clone(),
                )
                .await?;
                serde_json::to_value(result).map_err(error)
            }
            "save_transfer" => {
                let (store, _) =
                    crate::transfer_engine::TransferStore::open(&self.root.join("transfers.db"))?;
                let job = serde_json::from_value(request["job"].clone()).map_err(error)?;
                store.upsert(&job).await?;
                Ok(json!(true))
            }
            "transfers" => {
                let (_, jobs) =
                    crate::transfer_engine::TransferStore::open(&self.root.join("transfers.db"))?;
                serde_json::to_value(jobs).map_err(error)
            }
            "fail_transfer" => {
                // The engine's own failure policy applied to a stored job, as
                // it is when a running transfer returns an error.
                let (store, jobs) =
                    crate::transfer_engine::TransferStore::open(&self.root.join("transfers.db"))?;
                let id = text(&request, "id")?;
                let mut job = jobs
                    .into_iter()
                    .find(|job| job.id == id)
                    .ok_or("Transfer was not found")?;
                let now = request["now"].as_i64().ok_or("Missing now")?;
                crate::transfer_engine::apply_failure(
                    &mut job,
                    text(&request, "error")?.to_string(),
                    now,
                );
                job.revision = job.revision.saturating_add(1);
                store.upsert(&job).await?;
                // What a part of the application waiting on this transfer
                // would be told now.
                let supervision = match crate::transfer_engine::supervised_outcome(&job) {
                    None => json!("working"),
                    Some(Ok(())) => json!("completed"),
                    Some(Err(reason)) => json!({"stopped": reason}),
                };
                let mut reply = serde_json::to_value(job).map_err(error)?;
                reply["supervision"] = supervision;
                Ok(reply)
            }
            "remove_transfer" => {
                let (store, _) =
                    crate::transfer_engine::TransferStore::open(&self.root.join("transfers.db"))?;
                store
                    .delete_many(&[text(&request, "id")?.to_string()])
                    .await?;
                Ok(json!(true))
            }
            "retry_failed_upload" => {
                let account = if request["capturedAccount"].as_bool().unwrap_or(false) {
                    self.guard.clone().ok_or("No captured account")?
                } else {
                    AccountGuard::open(&self.root, Some(text(&request, "owner")?))?
                };
                let id = text(&request, "id")?;
                let state = self.transfer_retry()?;
                let result = crate::transfer_engine::retry_failed_upload(
                    &state.store,
                    &mut state.jobs,
                    &mut state.pending,
                    &mut state.prompt_tokens,
                    &account,
                    id,
                    request["promptToken"].as_u64(),
                )
                .await?;
                Ok(
                    json!({"job":result.job,"retried":result.retried,"hasPromptToken":state.prompt_tokens.contains_key(id)}),
                )
            }
            "transfer_retry_snapshot" => {
                let id = text(&request, "id")?;
                let state = self.transfer_retry()?;
                let job = state.jobs.get(id).ok_or("Transfer was not found")?;
                Ok(json!({"job":job,"hasPromptToken":state.prompt_tokens.contains_key(id)}))
            }
            "record_transfer_result" => {
                // The controlled Telegram boundary supplies a finished result;
                // production SQLite checkpoint/recovery behavior remains real.
                let job = serde_json::from_value(request["job"].clone()).map_err(error)?;
                let state = self.transfer_retry()?;
                let result = crate::transfer_engine::commit_result(
                    &state.store,
                    &mut state.jobs,
                    &mut state.pending,
                    job,
                )
                .await;
                serde_json::to_value(result).map_err(error)
            }
            _ => Err("Unknown transfers E2E command".into()),
        }
    }

    /// Public fixed inputs are isolated to this native fixture process.
    async fn known_answer(&self, request: &Value) -> Result<Value, String> {
        use crypto::envelope::header::KeySlotEntry;
        use crypto::envelope::key_slot::{wrap_dek_with_nonce, KeySlotContext};
        let uuid = [17; 16];
        let salt = [11; 16];
        let nonce = [13; 24];
        let dek = SecretKey::new([23; 32]);
        let master = SecretKey::new([7; 32]);
        let wrapping =
            crypto::kdf::derive_file_wrapping_key(&master, &uuid, &salt, 1, 0).map_err(error)?;
        let context = KeySlotContext {
            file_uuid: &uuid,
            format_version: 2,
        };
        let wrapped =
            wrap_dek_with_nonce(&context, &dek, &wrapping, 1, 0, 2, 0, 0, 0, &salt, nonce)
                .map_err(error)?;
        let slot = KeySlotEntry {
            kind: 1,
            slot_id: 0,
            kdf_algorithm: 2,
            argon2_memory_kib: 0,
            argon2_iterations: 0,
            argon2_parallelism: 0,
            salt,
            wrap_nonce: nonce,
            wrapped_dek: wrapped,
        };
        let metadata = crate::commands::fs::protected_metadata_bytes(
            "fixed-answer.bin",
            true,
            (request["syncPath"] == true).then_some("docs/2026/fixed-answer.bin"),
        )?;
        let mut session =
            EncryptionSession::new_with_keys(1_048_593, vec![slot], metadata, dek, uuid, [19; 16])
                .map_err(error)?;
        if request["continue"] == true {
            session = EncryptionSession::from_header(session.header_bytes.clone(), session.dek)
                .map_err(error)?;
        }
        let input = tokio::fs::File::open(self.root.join("fixed-answer.bin"))
            .await
            .map_err(error)?;
        let mut reader = EncryptingReader::new(input, session);
        let destination = child(&self.root, text(request, "destination")?)?;
        let mut output = tokio::fs::File::create(destination).await.map_err(error)?;
        let bytes = tokio::io::copy(&mut reader, &mut output)
            .await
            .map_err(error)?;
        output.sync_all().await.map_err(error)?;
        Ok(json!({"ciphertextBytes":bytes}))
    }

    /// Generate fixture ciphertext with production slot construction and encoding.
    async fn create_encrypted_fixture(&self, request: &Value) -> Result<Value, String> {
        let source = child(&self.root, text(request, "source")?)?;
        let destination = child(&self.root, text(request, "destination")?)?;
        let input = tokio::fs::File::open(source).await.map_err(error)?;
        let size = input.metadata().await.map_err(error)?.len();
        let vault_key = self.crypto.get_current_wrapping_key().map_err(error)?;
        let dek = SecretKey::new(crypto::random::random_key());
        let uuid = crypto::random::random_uuid();
        let slot = crate::commands::fs::vault_encryption_slot(&vault_key, &uuid, &dek)?;
        let session = EncryptionSession::new_with_keys(
            size,
            vec![slot],
            br#"{"schema_version":1,"original_name":"private-e2e-document.bin","mime_type":"application/octet-stream"}"#.to_vec(),
            dek,
            uuid,
            crypto::random::random_nonce_prefix(),
        )
        .map_err(error)?;
        let mut reader = EncryptingReader::new(input, session);
        let mut output = tokio::fs::File::create(destination).await.map_err(error)?;
        let bytes = tokio::io::copy(&mut reader, &mut output)
            .await
            .map_err(error)?;
        output.sync_all().await.map_err(error)?;
        Ok(json!({"ciphertextBytes":bytes}))
    }
    /// Read a fixture envelope to prove persisted vault material still decrypts it.
    /// This does not exercise Telegram download staging or final publication.
    async fn resumable(
        &self,
        request: &Value,
    ) -> Result<crate::resumable_upload::ResumableUpload, String> {
        let source = child(&self.root, text(request, "source")?)?;
        let variant = if request["protected"].as_bool().unwrap_or(false) {
            "protected:vault:true"
        } else {
            "plain"
        };
        Ok(crate::resumable_upload::ResumableUpload::new(
            &AccountGuard::open(&self.root, None)?,
            crate::resumable_upload::session_key(Some(UPLOAD_FOLDER), &source, variant),
            crate::resumable_upload::SourceFingerprint::of(&source).await?,
        ))
    }

    /// One attempt of a large upload through the production uploader, with the
    /// production envelope for a protected file. Only where the parts go is a
    /// stand-in.
    async fn upload_resumable(&self, request: &Value) -> Result<Value, String> {
        use crate::resumable_upload::UploadError;
        let source = child(&self.root, text(request, "source")?)?;
        let directory = child(&self.root, text(request, "sink")?)?;
        tokio::fs::create_dir_all(&directory).await.map_err(error)?;
        let sink = FixtureSink {
            directory,
            fail_at: request["failAtPart"]
                .as_i64()
                .and_then(|part| i32::try_from(part).ok()),
            saved: std::sync::Mutex::new(Vec::new()),
        };
        let sink = crate::resumable_upload::PacedSink {
            sink,
            traffic: crate::traffic::Traffic {
                network: self.network.clone(),
                account: AccountGuard::open(&self.root, None)?,
                direction: crate::traffic::Direction::Upload,
            },
        };
        let resumable = self.resumable(request).await?;
        let size = tokio::fs::metadata(&source).await.map_err(error)?.len();
        let plaintext = tokio::fs::File::open(&source).await.map_err(error)?;
        let mut continued_envelope = false;
        let outcome = if request["protected"].as_bool().unwrap_or(false) {
            let mode = request["protectionMode"].as_str().unwrap_or("vault");
            let vault_key = if mode == "passphrase" {
                None
            } else {
                Some(self.crypto.get_current_wrapping_key().map_err(error)?)
            };
            let passphrase = request["filePassphrase"]
                .as_str()
                .map(|value| crypto::secret::SecretBytes::from_slice(value.as_bytes()));
            let recorded_header = resumable
                .session()
                .await?
                .and_then(|session| session.envelope_header_bytes());
            let mut stream = crate::commands::fs::protected_upload_stream_off_worker(
                plaintext,
                size,
                mode,
                vault_key.as_ref(),
                passphrase.as_ref(),
                crate::commands::fs::protected_metadata_bytes(
                    &source.to_string_lossy(),
                    true,
                    request["syncPath"].as_str(),
                )?,
                recorded_header.as_deref(),
            )
            .await?;
            continued_envelope = stream.continued;
            if recorded_header.is_some() && !stream.continued {
                resumable.discard().await?;
            }
            resumable
                .upload(
                    &sink,
                    &mut stream.reader,
                    stream.ciphertext_size,
                    &stream.remote_name,
                    Some(&stream.header),
                )
                .await
        } else {
            let mut plaintext = plaintext;
            resumable
                .upload(&sink, &mut plaintext, size, "synthetic-upload.bin", None)
                .await
        };
        let mut saved = sink
            .sink
            .saved
            .lock()
            .map_err(|_| "Fixture sink lock poisoned".to_string())?
            .clone();
        saved.sort_unstable();
        match outcome {
            Ok(parts) => {
                // Published: the application forgets the finished upload.
                if request["publish"].as_bool().unwrap_or(false) {
                    resumable.discard().await?;
                }
                Ok(json!({
                    "fileId": parts.file_id,
                    "totalParts": parts.total_parts,
                    "resumedParts": parts.resumed_parts,
                    "savedParts": saved,
                    "remoteName": parts.name,
                    "continuedEnvelope": continued_envelope,
                }))
            }
            Err(UploadError::Stale(reason)) => Ok(json!({"startsOver": reason})),
            Err(UploadError::Failed(reason)) => Err(reason),
        }
    }

    async fn read_envelope(&self, request: &Value) -> Result<Value, String> {
        let source = child(&self.root, text(request, "source")?)?;
        let destination = child(&self.root, text(request, "destination")?)?;
        let bytes = tokio::fs::read(source).await.map_err(error)?;
        let header = EnvelopeHeader::parse(&bytes).map_err(error)?;
        let passphrase = request["filePassphrase"]
            .as_str()
            .map(|value| crypto::secret::SecretBytes::from_slice(value.as_bytes()));
        let key = if passphrase.is_some() {
            self.crypto.get_current_wrapping_key().ok()
        } else {
            Some(self.crypto.get_current_wrapping_key().map_err(error)?)
        };
        let (mut decoder, _) = crate::commands::fs::initialize_tdenc2_decryptor_off_worker(
            &bytes[..header.core.header_length as usize],
            key.as_ref(),
            passphrase.as_ref(),
        )
        .await?;
        let mut plaintext = Vec::new();
        for chunk in bytes[header.core.header_length as usize..].chunks(8191) {
            plaintext.extend(decoder.feed(chunk).map_err(error)?);
        }
        decoder.finish().map_err(error)?;
        tokio::fs::write(destination, &plaintext)
            .await
            .map_err(error)?;
        Ok(json!({"plaintextBytes":plaintext.len()}))
    }
}

pub async fn run() -> Result<(), String> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("Explicit private fixture directory required")?,
    )
    .canonicalize()
    .map_err(error)?;
    if std::fs::read_to_string(root.join(".native-e2e-fixture")).map_err(error)?
        != "telegram-drive-synthetic-e2e\n"
    {
        return Err("Refusing to access a non-fixture directory".into());
    }
    let mut initial_network = std::fs::read_to_string(root.join("network_settings.json"))
        .ok()
        .and_then(|bytes| {
            serde_json::from_str::<crate::vpn_optimizer::NetworkConfigSnapshot>(&bytes).ok()
        })
        .unwrap_or(crate::vpn_optimizer::NetworkConfigSnapshot {
            proxy: Default::default(),
            vpn: Default::default(),
        });
    initial_network.proxy.password =
        std::fs::read_to_string(root.join("proxy-credential")).unwrap_or_default();
    let mut driver = Driver {
        sync_app: None,
        transfer_retry: None,
        video_upload: None,
        notifications: None,
        proxy_update_task: None,
        reader_tasks: Vec::new(),
        network: Arc::new(crate::vpn_optimizer::NetworkConfig::new_with_config(
            initial_network,
        )),
        bandwidth: Arc::new(crate::bandwidth::BandwidthManager::at(&root)),
        bandwidth_hold: None,
        crypto: CryptoState::new(Box::new(FileVault::new(root.join("crypto.vault")))),
        vault_tasks: HashMap::new(),
        root,
        guard: None,
        planned_sync: None,
        servers: Vec::new(),
        retained_sessions: Vec::new(),
        archive_task: None,
        archive_continue: None,
        asset_downloads: Arc::new(AtomicU64::new(0)),
        asset_task: None,
        native_preview: None,
        search_task: None,
        search_index: None,
        legacy_pin_task: None,
        inventory_task: None,
        publication_task: None,
        publication_continue: None,
        inventories: Arc::new(crate::file_inventory::InventoryCache::default()),
        raw_inventory: Default::default(),
        inventory_history: Arc::new(AtomicU64::new(0)),
        inventory_calls: Arc::new(AtomicU64::new(0)),
        inventory_lookups: Arc::new(AtomicU64::new(0)),
        inventory_lookup_times: Arc::new(std::sync::Mutex::new(Vec::new())),
        store_task: None,
        store_continue: None,
        scan_peers: Arc::new(RwLock::new(HashMap::new())),
        scan_task: None,
        scan_continue: None,
    };
    println!("{}", json!({"ready":true}));
    std::io::stdout().flush().map_err(error)?;
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.map_err(error)?).map_err(error)?;
        if request["command"] == "shutdown" {
            break;
        }
        let response = match driver.dispatch(request).await {
            Ok(value) => json!({"ok":true,"value":value}),
            Err(message) => json!({"ok":false,"error":message}),
        };
        println!("{response}");
        std::io::stdout().flush().map_err(error)?;
    }
    if let Some(app) = &driver.sync_app {
        app.state::<SyncEngine>().shutdown_and_wait().await?;
    }
    for (handle, task) in driver.servers {
        handle.stop(true).await;
        task.await.map_err(error)?.map_err(error)?;
    }
    Ok(())
}

fn produced_fixture_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("Produced fixture path must remain inside the private root".into());
    }
    let path = root.join(path);
    if !path.canonicalize().map_err(error)?.starts_with(root) {
        return Err("Produced fixture path escaped its root".into());
    }
    Ok(path)
}

fn heic_fixture_tools(root: &Path, request: &Value) -> Result<crate::heic::Tools, String> {
    match request["heicTools"].as_str() {
        Some("sips") => Ok(crate::heic::Tools {
            sips: Some(PathBuf::from("/usr/bin/sips")),
            ffmpeg: vec![],
            ..Default::default()
        }),
        Some("ffmpeg") => Ok(crate::heic::Tools {
            sips: None,
            ffmpeg: vec![PathBuf::from("/usr/local/bin/ffmpeg")],
            ..Default::default()
        }),
        Some("missing") => Ok(crate::heic::Tools::default()),
        _ => {
            let name = if cfg!(windows) {
                "heic-helper.exe"
            } else {
                "heic-helper"
            };
            let helper = root.join(name);
            if !helper.exists() {
                let executable =
                    std::env::current_exe()
                        .map_err(error)?
                        .with_file_name(if cfg!(windows) {
                            "native-e2e-heic-helper.exe"
                        } else {
                            "native-e2e-heic-helper"
                        });
                if std::fs::hard_link(&executable, &helper).is_err() {
                    std::fs::copy(executable, &helper).map_err(error)?;
                }
            }
            // Prepare synthetic JPEG payloads outside the conversion clock. The small
            // helper only copies them; real HEIF decoding remains a separate boundary.
            let frame = root.join("heic-frame.jpg");
            let bytes = if frame.is_file() {
                std::fs::read(frame).map_err(error)?
            } else {
                include_bytes!("../test-support/fixtures/heic/tiled-12mp-reference.jpg").to_vec()
            };
            let dimensions = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(error)?
                .into_dimensions()
                .map_err(error)?;
            let maximum = if request["thumbnail"].as_bool().unwrap_or(false) {
                (480, 360)
            } else {
                (4096, 4096)
            };
            let scale = (f64::from(maximum.0) / f64::from(dimensions.0))
                .min(f64::from(maximum.1) / f64::from(dimensions.1))
                .min(1.0);
            let expected = (
                (f64::from(dimensions.0) * scale).round() as u32,
                (f64::from(dimensions.1) * scale).round() as u32,
            );
            let path = root.join(format!("heic-payload-{}x{}.jpg", expected.0, expected.1));
            let tile = std::fs::read_to_string(root.join("heic-mode"))
                .is_ok_and(|value| value.trim() == "tile");
            if tile {
                let image = image::load_from_memory(&bytes).map_err(error)?;
                image
                    .crop_imm(0, 0, 512.min(image.width()), 512.min(image.height()))
                    .save_with_format(root.join("heic-payload-tile.jpg"), image::ImageFormat::Jpeg)
                    .map_err(error)?;
            } else if expected == dimensions {
                std::fs::write(path, &bytes).map_err(error)?;
            } else {
                image::load_from_memory(&bytes)
                    .map_err(error)?
                    .thumbnail(expected.0, expected.1)
                    .save_with_format(path, image::ImageFormat::Jpeg)
                    .map_err(error)?;
            }
            Ok(crate::heic::Tools {
                sips: None,
                ffmpeg: vec![helper],
                fixture: Some(crate::process_budget::FixtureClock {
                    ready: root.join(format!("heic-conversion-{}.ready", uuid::Uuid::new_v4())),
                    delay_ms: request["fixtureReadyDelayMs"]
                        .as_u64()
                        .unwrap_or(0)
                        .min(5000),
                    duration: Duration::from_millis(
                        request["fixtureDeadlineMs"]
                            .as_u64()
                            .unwrap_or(120_000)
                            .clamp(100, 120_000),
                    ),
                    monitor_loss: match request["fixtureMonitorLoss"].as_str() {
                        Some("before-completion") => {
                            Some(crate::process_budget::FixtureMonitorLoss::BeforeCompletion)
                        }
                        Some("after-completion") => {
                            Some(crate::process_budget::FixtureMonitorLoss::AfterCompletion)
                        }
                        _ => None,
                    },
                    exit_delay_ms: request["fixtureExitDelayMs"]
                        .as_u64()
                        .unwrap_or(0)
                        .min(1000),
                    exit_failure: request["fixtureExitFailure"].as_bool().unwrap_or(false),
                    sample_gate: (request["fixtureSampleGate"] == true).then(|| {
                        (
                            root.join("heic-sample.started"),
                            root.join("heic-sample.release"),
                        )
                    }),
                }),
            })
        }
    }
}
