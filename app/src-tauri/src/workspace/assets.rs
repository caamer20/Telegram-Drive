use super::{
    cache_core::{self, state as cache_state, Clearing},
    store::{Store, WorkspaceFile},
    AccountGuard,
};
use crate::{
    bandwidth::{BandwidthManager, BandwidthReservation},
    commands::{
        utils::{media_size, resolve_peer},
        TelegramState,
    },
    vpn_optimizer::NetworkConfig,
};
use grammers_client::{
    types::{Downloadable, Media},
    Client,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, SystemTime},
};
use tauri::Manager;
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, RwLock, Semaphore},
};

static LOCKS: OnceLock<Mutex<HashMap<String, std::sync::Weak<Mutex<()>>>>> = OnceLock::new();
static READERS: OnceLock<Semaphore> = OnceLock::new();
static ASSET_READERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static CATEGORY_LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<RwLock<()>>>>> =
    OnceLock::new();
const THUMB_OUTPUT_LIMIT: u64 = 1024 * 1024;
const ORPHAN_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const THUMB_SOURCE_LIMIT: u64 = 16 * 1024 * 1024;
static REQUESTS: OnceLock<std::sync::Mutex<HashMap<String, PendingRequest>>> = OnceLock::new();

#[cfg(feature = "native-e2e")]
static METADATA_STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "native-e2e")]
static METADATA_FINISHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "native-e2e")]
struct MetadataWriteObservation;
#[cfg(feature = "native-e2e")]
impl MetadataWriteObservation {
    fn new() -> Self {
        METADATA_STARTED.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
#[cfg(feature = "native-e2e")]
impl Drop for MetadataWriteObservation {
    fn drop(&mut self) {
        METADATA_FINISHED.fetch_add(1, Ordering::SeqCst);
    }
}
#[cfg(feature = "native-e2e")]
pub(crate) fn metadata_finished() -> u64 {
    METADATA_FINISHED.load(Ordering::SeqCst)
}
#[cfg(feature = "native-e2e")]
pub(crate) fn metadata_started() -> u64 {
    METADATA_STARTED.load(Ordering::SeqCst)
}
#[cfg(feature = "native-e2e")]
pub(crate) fn core_busy() -> bool {
    cache_core::busy()
}
struct PendingRequest {
    cancelled: Arc<AtomicBool>,
    directory: PathBuf,
}
pub fn active_paths() -> HashSet<PathBuf> {
    cache_core::active_paths()
}
fn category_lock(directory: &Path) -> Arc<RwLock<()>> {
    CATEGORY_LOCKS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(directory.into())
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone()
}

pub(crate) struct ActivePath {
    path: PathBuf,
    token: String,
    disposable: AtomicBool,
    cleanup_pin: AtomicBool,
}
impl ActivePath {
    fn new(path: PathBuf, token: String, disposable: bool) -> Self {
        cache_state().paths.insert(path.clone(), token.clone());
        Self {
            path,
            token,
            disposable: AtomicBool::new(disposable),
            cleanup_pin: AtomicBool::new(false),
        }
    }
}
impl Drop for ActivePath {
    fn drop(&mut self) {
        let mut state = cache_state();
        let owned = state.paths.get(&self.path) == Some(&self.token);
        if owned {
            state.paths.remove(&self.path);
        }
        let disposable = self.disposable.load(Ordering::SeqCst);
        let cleanup_pin = self.cleanup_pin.load(Ordering::SeqCst);
        if owned && disposable && cleanup_pin {
            // Keep pin mutation serialized, including canceled publication.
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(self.path.with_extension("pin"));
        }
        drop(state);
        if owned && disposable && !cleanup_pin {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

struct CacheReservation {
    token: String,
    directory: PathBuf,
    epoch: u64,
}
impl CacheReservation {
    async fn reserve(
        directory: &Path,
        token: &str,
        bytes: u64,
        _thumbnail: bool,
    ) -> Result<Self, String> {
        let directory = directory.to_path_buf();
        let token = token.to_string();
        tokio::task::spawn_blocking(move || {
            let epoch = cache_state().reserve(&directory, &token, bytes)?;
            Ok(Self {
                token,
                directory,
                epoch,
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    fn cancelled(&self) -> bool {
        cache_state()
            .check(&self.directory, &self.token, self.epoch)
            .is_err()
    }
}
impl Drop for CacheReservation {
    fn drop(&mut self) {
        cache_state().reservations.remove(&self.token);
    }
}
fn kept(path: &Path) -> bool {
    cache_core::kept(path)
}

fn temporary(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == "part" || ext == "source")
}
fn orphan_age(meta: &std::fs::Metadata) -> bool {
    meta.modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= ORPHAN_AGE)
}

fn private_directory(base: &Path, components: &[&str], create: bool) -> Result<PathBuf, String> {
    if create {
        std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    }
    let mut path = base.to_path_buf();
    for component in components {
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err("STORAGE_UNAVAILABLE: Unexpected cache directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if create {
                    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(path)
}
fn cache_entries(directory: &Path) -> Result<Vec<(PathBuf, std::fs::Metadata)>, String> {
    cache_core::entries(directory)
}
pub fn configure_limits(previews: u64, thumbnails: u64) {
    cache_core::configure(previews, thumbnails);
}
pub fn limits() -> (u64, u64) {
    cache_core::limits()
}
fn requests() -> std::sync::MutexGuard<'static, HashMap<String, PendingRequest>> {
    REQUESTS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
pub(crate) struct Request {
    key: String,
    token: String,
    cancelled: Arc<AtomicBool>,
    directory: PathBuf,
    epoch: u64,
}
impl Request {
    fn new(owner: &str, id: Option<String>, directory: PathBuf) -> Result<Self, String> {
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid preview request".into());
        }
        let key = format!("{owner}:{id}");
        let state = cache_state();
        let cancelled = Arc::new(AtomicBool::new(state.clearing.contains_key(&directory)));
        if let Some(old) = requests().insert(
            key.clone(),
            PendingRequest {
                cancelled: cancelled.clone(),
                directory: directory.clone(),
            },
        ) {
            old.cancelled.store(true, Ordering::SeqCst);
        }
        let epoch = state.epoch(&directory);
        drop(state);
        Ok(Self {
            key,
            directory,
            epoch,
            token: uuid::Uuid::new_v4().to_string(),
            cancelled,
        })
    }
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || !cache_state().valid(&self.directory, self.epoch)
    }
    async fn interrupted(&self, account: &AccountGuard) -> String {
        loop {
            if self.cancelled() {
                return "CANCELLED".into();
            }
            if let Err(error) = account.validate() {
                return error;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let mut active = requests();
        if active
            .get(&self.key)
            .is_some_and(|value| Arc::ptr_eq(&value.cancelled, &self.cancelled))
        {
            active.remove(&self.key);
        }
    }
}
pub fn cancel_owner(owner: i64) {
    let prefix = format!("{owner}:");
    for (key, request) in requests().iter() {
        if key.starts_with(&prefix) {
            request.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

#[tauri::command]
pub async fn cmd_workspace_cancel_asset(
    app: tauri::AppHandle,
    owner_id: String,
    request_id: String,
) -> Result<(), String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, Some(&owner_id))?;
    cancel_request(&account, &request_id).map(|_| ())
}

pub(crate) fn cancel_request(account: &AccountGuard, id: &str) -> Result<bool, String> {
    account.validate()?;
    if let Some(request) = requests().get(&format!("{}:{id}", account.owner)) {
        request.cancelled.store(true, Ordering::SeqCst);
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn clear_owner(app: &tauri::AppHandle, owner: i64, category: &str) -> Result<(), String> {
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    clear_at(&root, &cache, owner, category).await
}
pub(crate) async fn clear_at(
    root: &Path,
    cache: &Path,
    owner: i64,
    category: &str,
) -> Result<(), String> {
    cache_core::register(cache, None)?;
    if !["previews", "thumbnails", "staging"].contains(&category) {
        return Err("Unknown cache category".into());
    }
    if category != "staging" {
        crate::external_files::invalidate_account(root, owner)?;
    }
    let categories = if category == "staging" {
        vec!["previews", "thumbnails"]
    } else {
        vec![category]
    };
    for category_name in categories {
        let directory = private_directory(
            cache,
            &["previews", "workspace", &owner.to_string(), category_name],
            false,
        )?;
        clear_directory(&directory, category == "staging").await?;
    }
    Ok(())
}

async fn clear_directory(directory: &Path, staging_only: bool) -> Result<(), String> {
    let directory = directory.to_path_buf();
    if staging_only {
        return tokio::task::spawn_blocking(move || {
            let active = cache_state();
            for (path, meta) in cache_entries(&directory)? {
                if temporary(&path) && orphan_age(&meta) && !active.paths.contains_key(&path) {
                    std::fs::remove_file(path).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    // Invalidate synchronously before waiting; retain both gates through deletion.
    let clearing = Clearing::new(&directory);
    for request in requests().values() {
        if request.directory == directory {
            request.cancelled.store(true, Ordering::SeqCst);
        }
    }
    let exclusive = category_lock(&directory).write_owned().await;
    tokio::task::spawn_blocking(move || {
        let _clearing = clearing;
        let _exclusive = exclusive;
        let _mutation = cache_state();
        for (path, _) in cache_entries(&directory)? {
            if !kept(&path) {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn file_lock(key: String) -> Arc<Mutex<()>> {
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .await;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(|lock| lock.upgrade()) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

pub fn file_name(file: &WorkspaceFile) -> String {
    let ext = file
        .file
        .file_ext
        .as_deref()
        .unwrap_or("bin")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    format!(
        "{:x}.{}",
        Sha256::digest(file.key.as_bytes()),
        if ext.is_empty() { "bin" } else { &ext }
    )
}

pub fn cache_root(app: &tauri::AppHandle, owner: i64) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("previews")
        .join("workspace")
        .join(owner.to_string()))
}

pub fn stored_file(account: &AccountGuard, key: &str) -> Result<WorkspaceFile, String> {
    account.validate()?;
    Store::open(&account.root, account.owner)?
        .file(key)?
        .ok_or_else(|| "FILE_NOT_INDEXED: Scan this folder to add it to your library".into())
}

pub async fn remote_media(
    app: &tauri::AppHandle,
    account: &AccountGuard,
    file: &WorkspaceFile,
) -> Result<(Client, Media), String> {
    account.validate()?;
    if file.file.encryption_state != "plain" {
        return Err(
            "ENCRYPTED_PREVIEW_UNAVAILABLE: Open protected files through the unlocked vault".into(),
        );
    }
    let state = app.state::<TelegramState>();
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or("NETWORK_UNAVAILABLE: Reconnect to Telegram")?;
    account.validate_client(&client).await?;
    let peer = resolve_peer(&client, file.file.folder_id, &state.peer_cache).await?;
    let id = i32::try_from(file.file.id).map_err(|_| "Invalid message identifier")?;
    let message = client
        .get_messages_by_id(&peer, &[id])
        .await
        .map_err(|_| "NETWORK_UNAVAILABLE: Could not read this Telegram file")?
        .into_iter()
        .flatten()
        .next()
        .ok_or("FILE_NOT_FOUND: This file is no longer available in Telegram")?;
    let media = message
        .media()
        .ok_or("FILE_NOT_FOUND: This message has no file")?;
    if message.text() == "TDENC2"
        || matches!(&media,Media::Document(d) if d.name().to_ascii_lowercase().ends_with(".tdenc"))
    {
        return Err(
            "ENCRYPTED_PREVIEW_UNAVAILABLE: Open protected files through the unlocked vault".into(),
        );
    }
    if crate::commands::fs::resolve_remote_envelope(
        account,
        &client,
        file.file.folder_id,
        message.id(),
        &media,
        message.text(),
    )
    .await?
    .is_some()
    {
        return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
    }
    if media_size(&media) != file.file.size {
        return Err("FILE_CHANGED: Refresh the folder before saving this file".into());
    }
    account.validate()?;
    Ok((client, media))
}

pub struct DownloadSource<'a> {
    pub app: &'a tauri::AppHandle,
    pub account: &'a AccountGuard,
    pub client: &'a Client,
}

struct ConfiguredDownloadSource<'a> {
    account: &'a AccountGuard,
    client: &'a Client,
    bandwidth: Arc<BandwidthManager>,
    config: Arc<NetworkConfig>,
}
/// Private temporary file, bounded streaming and exact-length publication.
/// Used by both disposable previews and durable offline-pack downloads.
pub async fn download<D: Downloadable>(
    source: DownloadSource<'_>,
    media: &D,
    expected: u64,
    target: &Path,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    download_inner(
        ConfiguredDownloadSource {
            account: source.account,
            client: source.client,
            bandwidth: source.app.state::<Arc<BandwidthManager>>().inner().clone(),
            config: source.app.state::<Arc<NetworkConfig>>().inner().clone(),
        },
        media,
        expected,
        target,
        cancelled,
        &mut progress,
        None,
    )
    .await
}

async fn download_inner<D: Downloadable>(
    source: ConfiguredDownloadSource<'_>,
    media: &D,
    expected: u64,
    target: &Path,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(u64),
    request: Option<&Request>,
) -> Result<(), String> {
    let ConfiguredDownloadSource {
        bandwidth,
        config,
        account,
        client,
    } = source;
    let _permit = READERS
        .get_or_init(|| Semaphore::new(3))
        .acquire()
        .await
        .map_err(|_| "Download service stopped")?;
    if cancelled() {
        return Err("CANCELLED".into());
    }
    account.validate()?;
    let parent = target.parent().ok_or("Invalid media destination")?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| e.to_string())?;
    let temporary = target.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
    let _temporary = ActivePath::new(
        temporary.clone(),
        request
            .map(|request| request.token.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        true,
    );
    crate::workspace::device_cache::ensure_free_space(parent, expected)
        .map_err(|e| format!("STORAGE_UNAVAILABLE: {e}"))?;
    let mut reservation = BandwidthReservation::download(bandwidth, expected)?;
    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut output = options
            .open(&temporary)
            .await
            .map_err(|e| format!("STORAGE_UNAVAILABLE: {e}"))?;
        let chunk_size = (config.chunk_size_bytes().clamp(4096, 512 * 1024) / 4096 * 4096) as i32;
        let mut stream = client.iter_download(media).chunk_size(chunk_size);
        let mut count = 0u64;
        while let Some(chunk) = stream
            .next()
            .await
            .map_err(|_| "NETWORK_UNAVAILABLE: Download interrupted; retry to continue")?
        {
            if cancelled() {
                return Err("CANCELLED: Download paused".into());
            }
            account.validate()?;
            count = count
                .checked_add(chunk.len() as u64)
                .ok_or("FILE_CHANGED: Download exceeded expected size")?;
            verify_length(count, expected, false)?;
            crate::workspace::device_cache::ensure_free_space(parent, chunk.len() as u64)
                .map_err(|e| format!("STORAGE_UNAVAILABLE: {e}"))?;
            config
                .pacer
                .wait(
                    &config,
                    crate::traffic::Direction::Download,
                    chunk.len(),
                    || {
                        if cancelled() {
                            return Err("CANCELLED: Download paused".into());
                        }
                        account.validate()
                    },
                )
                .await?;
            output
                .write_all(&chunk)
                .await
                .map_err(|e| format!("STORAGE_UNAVAILABLE: {e}"))?;
            progress(count);
        }
        verify_length(count, expected, true)?;
        output
            .sync_all()
            .await
            .map_err(|e| format!("STORAGE_UNAVAILABLE: {e}"))?;
        drop(output);
        if cancelled() {
            return Err("CANCELLED: Download paused".into());
        }
        account.validate()?;
        tokio::fs::rename(&temporary, target)
            .await
            .map_err(|e| e.to_string())?;
        reservation.commit();
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

pub fn verify_length(actual: u64, expected: u64, complete: bool) -> Result<(), String> {
    if actual > expected || (complete && actual != expected) {
        Err("INCOMPLETE_DOWNLOAD: File length could not be verified".into())
    } else {
        Ok(())
    }
}

pub fn tree_size(root: &Path) -> u64 {
    walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

pub fn prune_directory(root: &Path, limit: u64, preserve: Option<&Path>) -> Result<u64, String> {
    cache_state().prune(root, limit, 0, preserve)
}

#[derive(Clone)]
pub(crate) struct AssetInfo {
    pub owner: i64,
    pub identity: String,
    pub size: u64,
    pub thumbnail_size: Option<u64>,
    pub fallback: Option<ThumbnailInput>,
}
pub(crate) trait AssetSource: Send + Sync {
    fn info(&self) -> &AssetInfo;
    fn download<'a>(
        &'a self,
        target: &'a Path,
        thumbnail: bool,
        request: &'a Request,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> futures::future::BoxFuture<'a, Result<(), String>>;
}
struct TelegramAssetSource {
    bandwidth: Arc<BandwidthManager>,
    network: Arc<NetworkConfig>,
    account: AccountGuard,
    client: Client,
    media: Media,
    info: AssetInfo,
}
fn chosen_thumbnail(media: &Media) -> Option<grammers_client::types::photo_sizes::PhotoSize> {
    let thumbs = match media {
        Media::Photo(p) => p.thumbs(),
        Media::Document(d) => d.thumbs(),
        _ => Vec::new(),
    };
    thumbs
        .into_iter()
        .filter(|t| t.size() > 0 && t.size() as u64 <= THUMB_SOURCE_LIMIT)
        .min_by_key(|t| t.size().abs_diff(60_000))
}
impl TelegramAssetSource {
    async fn new(
        app: tauri::AppHandle,
        account: AccountGuard,
        client: Client,
        media: Media,
        thumbnail: bool,
    ) -> Result<Self, String> {
        let needs_video =
            matches!(&media, Media::Document(d) if d.mime_type().and_then(video_format).is_some());
        let executable = if thumbnail && chosen_thumbnail(&media).is_none() && needs_video {
            crate::transcode::detect_ffmpeg(&app).await
        } else {
            None
        };
        let source = Self::configured(
            account,
            client,
            media,
            thumbnail,
            app.state::<Arc<BandwidthManager>>().inner().clone(),
            app.state::<Arc<NetworkConfig>>().inner().clone(),
            executable,
        )?;
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let mut source = source;
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if matches!(source.info.fallback, Some(ThumbnailInput::Heic(_))) {
            source.info.fallback = Some(ThumbnailInput::Heic(crate::heic::Tools::platform(
                app.path().resource_dir().ok().as_deref(),
            )));
        }
        Ok(source)
    }
    fn configured(
        account: AccountGuard,
        client: Client,
        media: Media,
        thumbnail: bool,
        bandwidth: Arc<BandwidthManager>,
        network: Arc<NetworkConfig>,
        executable: Option<PathBuf>,
    ) -> Result<Self, String> {
        let identity = match &media {
            Media::Document(d) => format!("document:{}", d.id()),
            Media::Photo(p) => format!("photo:{}", p.id()),
            _ => return Err("THUMBNAIL_UNAVAILABLE".into()),
        };
        let size = media_size(&media);
        let thumbnail_size = chosen_thumbnail(&media).map(|t| t.size() as u64);
        let mime = match &media {
            Media::Photo(_) => Some("image/jpeg"),
            Media::Document(d) => d.mime_type(),
            _ => None,
        };
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let heic = crate::heic::is_mime(mime)
            || matches!(&media, Media::Document(d) if crate::heic::is_name(d.name()));
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let heic = false;
        let fallback = if heic {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                Some(ThumbnailInput::Heic(crate::heic::Tools::platform(None)))
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            {
                None
            }
        } else if mime.is_some_and(|m| m.starts_with("image/")) {
            Some(ThumbnailInput::Image)
        } else if thumbnail && thumbnail_size.is_none() && size <= THUMB_SOURCE_LIMIT {
            match mime.and_then(video_format) {
                Some(format) => executable.map(|executable| ThumbnailInput::Video {
                    executable,
                    format,
                    #[cfg(feature = "native-e2e")]
                    fixture: None,
                }),
                None => None,
            }
        } else {
            None
        };
        let info = AssetInfo {
            owner: account.owner,
            identity,
            size,
            thumbnail_size,
            fallback,
        };
        Ok(Self {
            bandwidth,
            network,
            account,
            client,
            media,
            info,
        })
    }
}
impl AssetSource for TelegramAssetSource {
    fn info(&self) -> &AssetInfo {
        &self.info
    }
    fn download<'a>(
        &'a self,
        target: &'a Path,
        thumbnail: bool,
        request: &'a Request,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> futures::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let source = ConfiguredDownloadSource {
                bandwidth: self.bandwidth.clone(),
                config: self.network.clone(),
                account: &self.account,
                client: &self.client,
            };
            if thumbnail {
                let thumb = chosen_thumbnail(&self.media).ok_or("THUMBNAIL_UNAVAILABLE")?;
                download_inner(
                    source,
                    &thumb,
                    thumb.size() as u64,
                    target,
                    || cancelled(),
                    |_| {},
                    Some(request),
                )
                .await
            } else {
                download_inner(
                    source,
                    &self.media,
                    self.info.size,
                    target,
                    || cancelled(),
                    |_| {},
                    Some(request),
                )
                .await
            }
        })
    }
}
pub(crate) fn disposable_name(file: &WorkspaceFile, identity: &str, thumbnail: bool) -> String {
    let mut disposable = file.clone();
    disposable.key = format!("raster-v2:{}:{identity}:{thumbnail}", file.key);
    if thumbnail {
        format!("{:x}.jpg", Sha256::digest(disposable.key.as_bytes()))
    } else {
        file_name(&disposable)
    }
}
#[tauri::command]
pub async fn cmd_workspace_asset(
    app: tauri::AppHandle,
    owner_id: String,
    key: String,
    thumbnail: bool,
    request_id: Option<String>,
    display: Option<bool>,
) -> Result<String, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, Some(&owner_id))?;
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    let scope = account.clone();
    let lookup_key = key.clone();
    let display_scope = (
        cache.clone(),
        account.clone(),
        key.clone(),
        request_id.clone(),
    );
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let tools = crate::heic::Tools::platform(app.path().resource_dir().ok().as_deref());
    let original = asset_at(
        cache,
        account,
        key,
        thumbnail,
        request_id,
        move || async move {
            match lookup_file_at(&scope, &lookup_key).await? {
                AssetLookup::Indexed(file) => {
                    resolve_asset(app, scope, *file, thumbnail, None).await
                }
                AssetLookup::Unindexed(folder, message) => {
                    resolve_unindexed(app, scope, folder, message, thumbnail).await
                }
            }
        },
    )
    .await?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if display == Some(true) && !thumbnail && crate::heic::is_name(&original) {
        return display_rendition_at(
            display_scope.0,
            display_scope.1,
            display_scope.2,
            original.into(),
            tools,
            display_scope.3,
        )
        .await;
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let _ = (display, display_scope);
    Ok(original)
}
pub(crate) enum AssetLookup {
    Indexed(Box<WorkspaceFile>),
    Unindexed(Option<i64>, i32),
}
pub(crate) async fn lookup_file_at(
    account: &AccountGuard,
    key: &str,
) -> Result<AssetLookup, String> {
    account.validate()?;
    match indexed_file_at(account, key).await {
        Ok(file) => Ok(AssetLookup::Indexed(Box::new(file))),
        Err(error) if error.starts_with("FILE_NOT_INDEXED") => {
            let (folder, message) = key.split_once(':').ok_or("INVALID_FILE_KEY")?;
            let folder = if folder == "saved" {
                None
            } else {
                Some(
                    folder
                        .parse::<i64>()
                        .ok()
                        .filter(|id| *id > 0)
                        .ok_or("INVALID_FILE_KEY")?,
                )
            };
            let message = message
                .parse::<i32>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or("INVALID_FILE_KEY")?;
            if super::store::file_key(folder, i64::from(message)) != key {
                return Err("INVALID_FILE_KEY".into());
            }
            account.validate()?;
            Ok(AssetLookup::Unindexed(folder, message))
        }
        Err(error) => Err(error),
    }
}
async fn indexed_file_at(account: &AccountGuard, key: &str) -> Result<WorkspaceFile, String> {
    let account = account.clone();
    let key = key.to_string();
    tokio::task::spawn_blocking(move || stored_file(&account, &key))
        .await
        .map_err(|e| e.to_string())?
}
async fn resolve_asset(
    app: tauri::AppHandle,
    account: AccountGuard,
    file: WorkspaceFile,
    thumbnail: bool,
    resolved: Option<(Client, Media)>,
) -> Result<(WorkspaceFile, Option<Box<dyn AssetSource>>), String> {
    if file.file.encryption_state != "plain" {
        return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
    }
    let remote = match resolved {
        Some(value) => Ok(value),
        None => tokio::time::timeout(Duration::from_secs(60), remote_media(&app, &account, &file))
            .await
            .map_err(|_| "NETWORK_UNAVAILABLE: Preview lookup timed out".to_string())
            .and_then(|value| value),
    };
    let source = match remote {
        Ok((client, media)) => Some(
            TelegramAssetSource::new(app.clone(), account.clone(), client, media, thumbnail)
                .await?,
        ),
        Err(error) if error.starts_with("NETWORK_UNAVAILABLE") => None,
        Err(error) => return Err(error),
    };
    Ok((
        file,
        source.map(|source| Box::new(source) as Box<dyn AssetSource>),
    ))
}
/// Compatibility entry point also accepts messages which have not been indexed.
/// It uses current Telegram media without adding that message to the inventory.
pub(crate) async fn legacy_asset(
    app: tauri::AppHandle,
    folder: Option<i64>,
    message: i32,
    thumbnail: bool,
) -> Result<String, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let account = AccountGuard::open(&root, None)?;
    let key = super::store::file_key(folder, i64::from(message));
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    let lookup_key = key.clone();
    let scope = account.clone();
    legacy_asset_at(cache, account, key, thumbnail, move || async move {
        let account = scope;
        let key = lookup_key;
        match lookup_file_at(&account, &key).await? {
            AssetLookup::Indexed(file) => resolve_asset(app, account, *file, thumbnail, None).await,
            AssetLookup::Unindexed(folder, message) => {
                resolve_unindexed(app, account, folder, message, thumbnail).await
            }
        }
    })
    .await
}
/// Shared command path, with the Telegram lookup supplied by the caller.
pub(crate) async fn legacy_asset_at<F, Fut>(
    cache: PathBuf,
    account: AccountGuard,
    key: String,
    thumbnail: bool,
    resolve: F,
) -> Result<String, String>
where
    F: FnOnce() -> Fut,
    Fut:
        std::future::Future<Output = Result<(WorkspaceFile, Option<Box<dyn AssetSource>>), String>>,
{
    asset_at_inner(cache, account, key, thumbnail, None, true, resolve).await
}

async fn resolve_unindexed(
    app: tauri::AppHandle,
    account: AccountGuard,
    folder: Option<i64>,
    message: i32,
    thumbnail: bool,
) -> Result<(WorkspaceFile, Option<Box<dyn AssetSource>>), String> {
    let key = super::store::file_key(folder, i64::from(message));
    let state = app.state::<TelegramState>();
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or("NETWORK_UNAVAILABLE: Reconnect to Telegram")?;
    account.validate_client(&client).await?;
    let peer = resolve_peer(&client, folder, &state.peer_cache).await?;
    let message = client
        .get_messages_by_id(&peer, &[message])
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .flatten()
        .next()
        .ok_or("FILE_NOT_FOUND")?;
    let media = message.media().ok_or("FILE_NOT_FOUND")?;
    if crate::commands::fs::resolve_remote_envelope(
        &account,
        &client,
        folder,
        message.id(),
        &media,
        message.text(),
    )
    .await?
    .is_some()
    {
        return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
    }
    let (name, mime) = match &media {
        Media::Document(d) => (d.name().to_string(), d.mime_type().map(str::to_string)),
        Media::Photo(_) => ("Photo.jpg".into(), Some("image/jpeg".into())),
        _ => return Err("THUMBNAIL_UNAVAILABLE".into()),
    };
    let extension = Path::new(&name)
        .extension()
        .map(|e| e.to_string_lossy().into_owned());
    let file = WorkspaceFile {
        key,
        folder_name: String::new(),
        tags: Vec::new(),
        collection_ids: Vec::new(),
        file: crate::models::FileMetadata {
            id: i64::from(message.id()),
            folder_id: folder,
            name,
            size: media_size(&media),
            mime_type: mime,
            file_ext: extension,
            created_at: message.date().to_rfc3339(),
            icon_type: "file".into(),
            encryption_state: "plain".into(),
            is_favorite: false,
            is_pinned: false,
        },
    };
    resolve_asset(app, account, file, thumbnail, Some((client, media))).await
}
#[cfg(feature = "native-e2e")]
type FixtureThumbnail = (WorkspaceFile, Arc<dyn AssetSource>);
#[cfg(feature = "native-e2e")]
type FixtureThumbnails = HashMap<(PathBuf, i64, String), FixtureThumbnail>;
#[cfg(feature = "native-e2e")]
static THUMBNAIL_FIXTURES: OnceLock<std::sync::Mutex<FixtureThumbnails>> = OnceLock::new();
#[cfg(feature = "native-e2e")]
pub(crate) fn seed_thumbnail_fixture(
    account: &AccountGuard,
    file: WorkspaceFile,
    source: Box<dyn AssetSource>,
) {
    THUMBNAIL_FIXTURES
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            (account.root.clone(), account.owner, file.key.clone()),
            (file, Arc::from(source)),
        );
}
#[cfg(feature = "native-e2e")]
struct FixtureThumbnailSource(Arc<dyn AssetSource>);
#[cfg(feature = "native-e2e")]
impl AssetSource for FixtureThumbnailSource {
    fn info(&self) -> &AssetInfo {
        self.0.info()
    }
    fn download<'a>(
        &'a self,
        target: &'a Path,
        thumbnail: bool,
        request: &'a Request,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> futures::future::BoxFuture<'a, Result<(), String>> {
        self.0.download(target, thumbnail, request, cancelled)
    }
}

pub(crate) async fn remote_thumbnail_at(
    cache: PathBuf,
    account: AccountGuard,
    state: Arc<TelegramState>,
    transport: (Arc<BandwidthManager>, Arc<NetworkConfig>),
    folder: Option<i64>,
    message: i32,
) -> Result<String, String> {
    let (bandwidth, network) = transport;
    let key = super::store::file_key(folder, i64::from(message));
    #[cfg(feature = "native-e2e")]
    {
        let fixture = THUMBNAIL_FIXTURES
            .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(account.root.clone(), account.owner, key.clone()))
            .cloned();
        if let Some((file, source)) = fixture {
            return asset_at(cache, account, key, true, None, move || async move {
                Ok((
                    file,
                    Some(Box::new(FixtureThumbnailSource(source)) as Box<dyn AssetSource>),
                ))
            })
            .await;
        }
    }
    let scope = account.clone();
    let lookup_key = key.clone();
    asset_at(cache, account, key, true, None, move || async move {
        let client = state
            .client
            .lock()
            .await
            .clone()
            .ok_or("NETWORK_UNAVAILABLE")?;
        scope.validate_client(&client).await?;
        let peer = resolve_peer(&client, folder, &state.peer_cache).await?;
        let message = client
            .get_messages_by_id(&peer, &[message])
            .await
            .map_err(|e| format!("NETWORK_UNAVAILABLE: {e}"))?
            .into_iter()
            .flatten()
            .next()
            .ok_or("FILE_NOT_FOUND")?;
        let media = message.media().ok_or("FILE_NOT_FOUND")?;
        if crate::commands::fs::resolve_remote_envelope(
            &scope,
            &client,
            folder,
            message.id(),
            &media,
            message.text(),
        )
        .await?
        .is_some()
        {
            return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
        }
        let (name, mime) = match &media {
            Media::Document(d) => (d.name().to_string(), d.mime_type().map(str::to_string)),
            Media::Photo(_) => ("Photo.jpg".into(), Some("image/jpeg".into())),
            _ => return Err("THUMBNAIL_UNAVAILABLE".into()),
        };
        let extension = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().into_owned());
        let file = WorkspaceFile {
            key: lookup_key,
            folder_name: String::new(),
            tags: Vec::new(),
            collection_ids: Vec::new(),
            file: crate::models::FileMetadata {
                id: i64::from(message.id()),
                folder_id: folder,
                name,
                size: media_size(&media),
                mime_type: mime,
                file_ext: extension,
                created_at: message.date().to_rfc3339(),
                icon_type: "file".into(),
                encryption_state: "plain".into(),
                is_favorite: false,
                is_pinned: false,
            },
        };
        let needs_video =
            matches!(&media, Media::Document(d) if d.mime_type().and_then(video_format).is_some());
        let executable = if chosen_thumbnail(&media).is_none() && needs_video {
            crate::transcode::detect_path_ffmpeg().await
        } else {
            None
        };
        let source = TelegramAssetSource::configured(
            scope, client, media, true, bandwidth, network, executable,
        )?;
        Ok((file, Some(Box::new(source) as Box<dyn AssetSource>)))
    })
    .await
}

pub(crate) async fn asset_at<F, Fut>(
    cache: PathBuf,
    account: AccountGuard,
    key: String,
    thumbnail: bool,
    request_id: Option<String>,
    resolve: F,
) -> Result<String, String>
where
    F: FnOnce() -> Fut,
    Fut:
        std::future::Future<Output = Result<(WorkspaceFile, Option<Box<dyn AssetSource>>), String>>,
{
    asset_at_inner(cache, account, key, thumbnail, request_id, false, resolve).await
}

pub(crate) async fn asset_at_inner<F, Fut>(
    cache: PathBuf,
    account: AccountGuard,
    key: String,
    thumbnail: bool,
    request_id: Option<String>,
    historical: bool,
    resolve: F,
) -> Result<String, String>
where
    F: FnOnce() -> Fut,
    Fut:
        std::future::Future<Output = Result<(WorkspaceFile, Option<Box<dyn AssetSource>>), String>>,
{
    account.validate()?;
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let cache = cache.canonicalize().map_err(|e| e.to_string())?;
    cache_core::register(&cache, Some(&account.root))?;
    let owner_id = account.owner.to_string();
    let category = if thumbnail { "thumbnails" } else { "previews" };
    let directory = private_directory(
        &cache,
        &["previews", "workspace", &owner_id, category],
        false,
    )?;
    let request = Request::new(&owner_id, request_id, directory.clone())?;
    let semaphore = ASSET_READERS
        .get_or_init(|| Arc::new(Semaphore::new(3)))
        .clone();
    let capacity = Arc::new(tokio::select! {
        value=semaphore.acquire_owned()=>value.map_err(|_|"Preview service stopped")?,
        error=request.interrupted(&account)=>return Err(error),
    });
    let lock = file_lock(format!("{}:{owner_id}:{key}", cache.to_string_lossy())).await;
    let file_guard = Arc::new(
        tokio::select! {value=lock.lock_owned()=>value,error=request.interrupted(&account)=>return Err(error)},
    );
    let category_guard = Arc::new(category_lock(&directory).read_owned().await);
    if request.cancelled() || cache_state().clearing.contains_key(&directory) {
        return Err("CANCELLED".into());
    }
    account.validate()?;
    private_directory(
        &cache,
        &["previews", "workspace", &owner_id, category],
        true,
    )?;
    if historical {
        let scope = account.clone();
        let base = cache.clone();
        let record_key = key.clone();
        let legacy = tokio::task::spawn_blocking(move || {
            crate::legacy_external::historical_asset(&scope, &base, &record_key, thumbnail)
        })
        .await
        .map_err(|e| e.to_string())??;
        if let Some((path, proof)) = legacy {
            // Retain admission/category guards through the full digest. Clear
            // invalidates the request before it waits for the same category.
            let legacy = Arc::new(crate::commands::preview::LegacyPreviewUse::new(&path));
            // Tuple fields drop in order. Unregister the active legacy path
            // before releasing the category lock that lets clear proceed.
            let lease = (
                legacy.clone(),
                capacity.clone(),
                file_guard.clone(),
                category_guard.clone(),
            );
            crate::external_files::reuse_cached_retained(account.clone(), path.clone(), lease)
                .await?;
            let scope = account.clone();
            let base = cache.clone();
            let candidate = path.clone();
            let holds = (capacity.clone(), file_guard.clone(), category_guard.clone());
            let valid = tokio::task::spawn_blocking(move || {
                let _holds = holds;
                proof.check(&scope, &base, &candidate)
            })
            .await
            .map_err(|e| e.to_string())??;
            account.validate()?;
            if request.cancelled() || !cache_state().valid(&directory, request.epoch) {
                return Err("CANCELLED".into());
            }
            if !valid {
                return Err("FILE_OPEN_REFUSED: Legacy cache record changed".into());
            }
            return Ok(path.to_string_lossy().into_owned());
        }
    }
    let (file, source) = tokio::select! {
        result=tokio::time::timeout(Duration::from_secs(60),resolve())=>result.map_err(|_|"NETWORK_UNAVAILABLE: Preview lookup timed out")??,
        error=request.interrupted(&account)=>return Err(error),
    };
    account.validate()?;
    if file.key != key || file.file.encryption_state != "plain" {
        return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
    }
    if source
        .as_ref()
        .is_some_and(|source| source.info().owner != account.owner)
    {
        return Err("ACCOUNT_CHANGED".into());
    }
    if source
        .as_ref()
        .is_some_and(|source| source.info().size != file.file.size)
    {
        return Err("FILE_CHANGED: Refresh this file".into());
    }
    let scope = account.clone();
    let record_key = key.clone();
    let identity = source.as_ref().map(|source| source.info().identity.clone());
    let flag = request.cancelled.clone();
    let epoch = request.epoch;
    let metadata_directory = directory.clone();
    let holds = (capacity.clone(), file_guard.clone(), category_guard.clone());
    let metadata = tokio::task::spawn_blocking(move || {
        let _holds = holds;
        let check = || -> Result<(), String> {
            scope.validate()?;
            if flag.load(Ordering::SeqCst) || !cache_state().valid(&metadata_directory, epoch) {
                return Err("CANCELLED".into());
            }
            Ok(())
        };
        check()?;
        let store = Store::open(&scope.root, scope.owner)?;
        match identity {
            Some(identity) => {
                #[cfg(feature = "native-e2e")]
                let _metadata = MetadataWriteObservation::new();
                store.transaction(|| {
                    check()?;
                    store.put_record("asset-identity-v1", &record_key, &identity)?;
                    check()?;
                    Ok(identity)
                })
            }
            None => {
                let identity = store
                    .record::<String>("asset-identity-v1", &record_key)?
                    .ok_or_else(|| "NETWORK_UNAVAILABLE: No verified cached media".to_string())?;
                check()?;
                Ok(identity)
            }
        }
    });
    let identity = tokio::select! {result=metadata=>result.map_err(|e|e.to_string())??,error=request.interrupted(&account)=>return Err(error)};
    let target = directory.join(disposable_name(&file, &identity, thumbnail));
    let target_guard = Arc::new(ActivePath::new(
        target.clone(),
        request.token.clone(),
        false,
    ));
    let cap = if thumbnail { limits().1 } else { limits().0 };
    match std::fs::symlink_metadata(&target) {
        Ok(metadata)
            if metadata.file_type().is_file()
                && (metadata.len() <= cap || kept(&target))
                && (if thumbnail {
                    metadata.len() > 0
                } else {
                    metadata.len() == file.file.size
                }) =>
        {
            let reusable = if thumbnail {
                let handle = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&target)
                    .map_err(|e| e.to_string())?;
                handle
                    .set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()))
                    .map_err(|e| e.to_string())?;
                true
            } else {
                crate::external_files::reuse_cached(account.clone(), target.clone()).await?
            };
            account.validate()?;
            if request.cancelled() {
                return Err("CANCELLED".into());
            }
            // Preserve legacy internal availability without granting external
            // opening. Rejected registered files are never deleted here.
            if reusable || !thumbnail {
                return Ok(target.to_string_lossy().into_owned());
            }
        }
        Ok(metadata) if metadata.file_type().is_file() => {
            std::fs::remove_file(&target).map_err(|e| e.to_string())?
        }
        Ok(_) => return Err("STORAGE_UNAVAILABLE: Invalid preview file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    target_guard.disposable.store(true, Ordering::SeqCst);
    let source = source.ok_or("NETWORK_UNAVAILABLE: Reconnect to Telegram")?;
    if !thumbnail && file.file.size > cap {
        return Err("PREVIEW_TOO_LARGE: Keep this file offline or increase the cache limit".into());
    }
    let operation = async {
        if thumbnail {
            let input = target.with_extension(format!("{}.source", uuid::Uuid::new_v4()));
            let source_guard =
                Arc::new(ActivePath::new(input.clone(), request.token.clone(), true));
            let small = source.info().thumbnail_size.is_some();
            let size = source.info().thumbnail_size.unwrap_or(source.info().size);
            let kind = if small {
                ThumbnailInput::Image
            } else {
                source
                    .info()
                    .fallback
                    .clone()
                    .ok_or("THUMBNAIL_UNAVAILABLE")?
            };
            if size > THUMB_SOURCE_LIMIT {
                return Err("THUMBNAIL_UNAVAILABLE".into());
            }
            let reservation = Arc::new(
                CacheReservation::reserve(
                    &directory,
                    &request.token,
                    size + THUMB_OUTPUT_LIMIT,
                    true,
                )
                .await?,
            );
            let flag = request.cancelled.clone();
            let lease = reservation.clone();
            source
                .download(
                    &input,
                    small,
                    &request,
                    Arc::new(move || flag.load(Ordering::SeqCst) || lease.cancelled()),
                )
                .await?;
            verify_length(
                std::fs::metadata(&input).map_err(|e| e.to_string())?.len(),
                size,
                true,
            )?;
            let holds = (
                capacity.clone(),
                file_guard.clone(),
                category_guard.clone(),
                source_guard,
                target_guard.clone(),
                reservation.clone(),
            );
            let flag = request.cancelled.clone();
            render_thumbnail(
                input,
                target.clone(),
                Some(account.clone()),
                kind,
                Arc::new(move || flag.load(Ordering::SeqCst) || reservation.cancelled()),
                holds,
                Some(request.token.clone()),
            )
            .await?;
        } else {
            let reservation = Arc::new(
                CacheReservation::reserve(&directory, &request.token, file.file.size, false)
                    .await?,
            );
            let flag = request.cancelled.clone();
            let lease = reservation.clone();
            source
                .download(
                    &target,
                    false,
                    &request,
                    Arc::new(move || flag.load(Ordering::SeqCst) || lease.cancelled()),
                )
                .await?;
            verify_length(
                std::fs::metadata(&target).map_err(|e| e.to_string())?.len(),
                file.file.size,
                true,
            )?;
        }
        account.validate()?;
        if request.cancelled() {
            let _ = tokio::fs::remove_file(&target).await;
            return Err("CANCELLED".into());
        }
        let cap = if thumbnail { limits().1 } else { limits().0 };
        if std::fs::metadata(&target).map_err(|e| e.to_string())?.len() > cap {
            std::fs::remove_file(&target).map_err(|e| e.to_string())?;
            return Err("PREVIEW_TOO_LARGE".into());
        }
        let prune_root = directory.clone();
        let preserve = target.clone();
        tokio::task::spawn_blocking(move || prune_directory(&prune_root, cap, Some(&preserve)))
            .await
            .map_err(|e| e.to_string())??;
        if !thumbnail {
            crate::external_files::register_cache_async(
                account.clone(),
                target.clone(),
                cache.clone(),
                file.key.clone(),
                identity.clone(),
            )
            .await?;
        }
        Ok(target.to_string_lossy().into_owned())
    };
    let result =
        tokio::select! {result=operation=>result,error=request.interrupted(&account)=>Err(error)};
    if result.is_ok() {
        let mutation = cache_state();
        if !mutation.valid(&request.directory, request.epoch)
            || request.cancelled.load(Ordering::SeqCst)
        {
            return Err("CANCELLED".into());
        }
        account.validate()?;
        target_guard.disposable.store(false, Ordering::SeqCst);
    }
    result
}

#[cfg(feature = "native-e2e")]
static DECODING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "native-e2e")]
static DECODE_PEAK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "native-e2e")]
pub(crate) struct DecodeObservation;
#[cfg(feature = "native-e2e")]
impl DecodeObservation {
    pub(crate) fn new() -> Self {
        let active = DECODING.fetch_add(1, Ordering::SeqCst) + 1;
        DECODE_PEAK.fetch_max(active, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        Self
    }
}
#[cfg(feature = "native-e2e")]
impl Drop for DecodeObservation {
    fn drop(&mut self) {
        DECODING.fetch_sub(1, Ordering::SeqCst);
    }
}
#[cfg(feature = "native-e2e")]
pub(crate) fn decode_peak() -> usize {
    DECODE_PEAK.load(Ordering::SeqCst)
}

fn video_format(mime: &str) -> Option<&'static str> {
    match mime {
        "video/mp4" | "video/quicktime" | "video/x-m4v" => Some("mov"),
        "video/webm" | "video/x-matroska" => Some("matroska"),
        "video/x-msvideo" => Some("avi"),
        _ => None,
    }
}

static THUMBNAIL_DECODERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
#[cfg(feature = "native-e2e")]
#[derive(Clone)]
pub(crate) struct VideoFixtureClock {
    pub ready: PathBuf,
    pub delay_ms: u64,
}
#[derive(Clone)]
pub(crate) enum ThumbnailInput {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    Heic(crate::heic::Tools),
    Image,
    Video {
        executable: PathBuf,
        format: &'static str,
        #[cfg(feature = "native-e2e")]
        fixture: Option<VideoFixtureClock>,
    },
}
/// The permit and all caller leases move into the blocking job. Dropping a
/// waiting caller cannot permit another decode or a clear before this one ends.
pub(crate) async fn render_thumbnail<H: Send + 'static>(
    input: PathBuf,
    destination: PathBuf,
    account: Option<AccountGuard>,
    kind: ThumbnailInput,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    holds: H,
    cache_token: Option<String>,
) -> Result<PathBuf, String> {
    let permit = THUMBNAIL_DECODERS
        .get_or_init(|| Arc::new(Semaphore::new(3)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "Thumbnail service stopped")?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let heic_permit = if matches!(kind, ThumbnailInput::Heic(_)) {
        Some(
            crate::heic::DECODERS
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| "Decoder stopped")?,
        )
    } else {
        None
    };
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _holds = holds;
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let _heic_permit = heic_permit;
        #[cfg(feature = "native-e2e")]
        let _observation = DecodeObservation::new();
        let check = || -> Result<(), String> {
            if cancelled() {
                return Err("CANCELLED".into());
            }
            if let Some(account) = &account {
                account.validate()?;
            }
            Ok(())
        };
        check()?;
        let metadata = std::fs::symlink_metadata(&input).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || metadata.len() > THUMB_SOURCE_LIMIT {
            return Err("THUMBNAIL_UNAVAILABLE".into());
        }
        let token = cache_token.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let temporary = destination.with_extension(format!("{token}.part"));
        let _partial = ActivePath::new(temporary.clone(), token.clone(), true);
        let image_input = match kind {
            ThumbnailInput::Image => input.clone(),
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            ThumbnailInput::Heic(tools) => {
                crate::heic::decode(&input, &temporary, &tools, (480, 360), check)?;
                temporary.clone()
            }
            ThumbnailInput::Video {
                executable,
                format,
                #[cfg(feature = "native-e2e")]
                fixture,
            } => {
                let mut options = std::fs::OpenOptions::new();
                options.create_new(true).write(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                drop(options.open(&temporary).map_err(|e| e.to_string())?);
                let mut command = std::process::Command::new(executable);
                command.args([
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-max_alloc",
                    "67108864",
                    "-threads",
                    "1",
                    "-max_pixels",
                    "16777216",
                    "-protocol_whitelist",
                    "file,pipe",
                    "-probesize",
                    "1048576",
                    "-analyzeduration",
                    "1000000",
                    "-f",
                    format,
                ]);
                if format == "mov" {
                    command.args(["-enable_drefs", "0", "-use_absolute_path", "0"]);
                }
                command
                    .arg("-i")
                    .arg(&input)
                    .args([
                        "-map",
                        "0:v:0",
                        "-an",
                        "-sn",
                        "-dn",
                        "-frames:v",
                        "1",
                        "-vf",
                        "scale=480:360:force_original_aspect_ratio=decrease",
                        "-threads",
                        "1",
                        "-c:v",
                        "mjpeg",
                        "-f",
                        "image2",
                        "-update",
                        "1",
                        "-fs",
                        "1048576",
                    ])
                    .arg(&temporary)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(feature = "native-e2e")]
                if let Some(clock) = &fixture {
                    command.env("TD_E2E_VIDEO_READY", &clock.ready)
                        .env("TD_E2E_VIDEO_READY_DELAY_MS", clock.delay_ms.to_string());
                }
                crate::process_util::hide_console_blocking(&mut command);
                #[cfg(feature = "native-e2e")]
                let spawning = std::time::Instant::now();
                let mut child = command
                    .spawn()
                    .map_err(|_| "THUMBNAIL_UNAVAILABLE: FFmpeg unavailable")?;
                let deadline = std::time::Instant::now() + Duration::from_secs(20);
                #[cfg(feature = "native-e2e")]
                let fixture_started = std::time::Instant::now();
                #[cfg(feature = "native-e2e")]
                let fixture_spawn_seconds = spawning.elapsed().as_secs_f64();
                #[cfg(feature = "native-e2e")]
                let mut fixture_ready = None;
                loop {
                    #[cfg(feature = "native-e2e")]
                    let current_deadline = if let Some(clock) = &fixture {
                        if fixture_ready.is_none() && std::fs::read_to_string(&clock.ready)
                            .is_ok_and(|value| value == child.id().to_string()) {
                            let ready = std::time::Instant::now();
                            fixture_ready = Some(ready);
                            eprintln!("Controlled video timing: spawn_seconds={fixture_spawn_seconds:.6}, readiness_seconds={:.6}", fixture_started.elapsed().as_secs_f64());
                        }
                        fixture_ready.unwrap_or(fixture_started) + Duration::from_secs(120)
                    } else { deadline };
                    #[cfg(not(feature = "native-e2e"))]
                    let current_deadline = deadline;
                    if check().is_err() || std::time::Instant::now() >= current_deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("CANCELLED: Thumbnail extraction stopped".into());
                    }
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if !status.success() {
                                return Err("THUMBNAIL_UNAVAILABLE".into());
                            }
                            break;
                        }
                        Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                        Err(error) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(error.to_string());
                        }
                    }
                }
                if std::fs::metadata(&temporary)
                    .map_err(|e| e.to_string())?
                    .len()
                    > THUMB_OUTPUT_LIMIT
                {
                    return Err("THUMBNAIL_UNAVAILABLE".into());
                }
                temporary.clone()
            }
        };
        let mut reader = image::ImageReader::open(&image_input)
            .map_err(|e| e.to_string())?
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(64 * 1024 * 1024);
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| "THUMBNAIL_UNAVAILABLE")?
            .thumbnail(480, 360)
            .to_rgb8();
        if image_input == temporary {
            std::fs::remove_file(&temporary).map_err(|e| e.to_string())?;
        }
        // Frame extraction used the private partial; encode to a separate private
        // output so reading it cannot conflict with truncating the same file.
        let output_path = destination.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
        let _output_partial = ActivePath::new(output_path.clone(), token, true);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(&output_path).map_err(|e| e.to_string())?;
        image
            .write_to(&mut output, image::ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        if output.metadata().map_err(|e| e.to_string())?.len() > THUMB_OUTPUT_LIMIT {
            return Err("THUMBNAIL_UNAVAILABLE".into());
        }
        drop(output);
        check()?;
        std::fs::rename(&output_path, &destination).map_err(|e| e.to_string())?;
        if let Err(error) = check() {
            let _ = std::fs::remove_file(&destination);
            return Err(error);
        }
        Ok(destination)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn cached_preview_at(
    cache: &Path,
    account: &AccountGuard,
    key: &str,
) -> Result<Option<PathBuf>, String> {
    account.validate()?;
    let Some(identity) =
        Store::open(&account.root, account.owner)?.record::<String>("asset-identity-v1", key)?
    else {
        return crate::legacy_external::historical_preview(account, cache, key);
    };
    let directory = private_directory(
        cache,
        &[
            "previews",
            "workspace",
            &account.owner.to_string(),
            "previews",
        ],
        false,
    )?;
    let prefix = format!(
        "{:x}.",
        Sha256::digest(format!("raster-v2:{key}:{identity}:false").as_bytes())
    );
    let newest = cache_entries(&directory)?
        .into_iter()
        .filter(|(path, meta)| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
                && meta.len() > 0
                && !temporary(path)
                && path.extension().is_none_or(|extension| extension != "pin")
        })
        .max_by_key(|(_, meta)| meta.modified().unwrap_or(SystemTime::UNIX_EPOCH))
        .map(|(path, _)| path);
    account.validate()?;
    Ok(newest)
}
pub(crate) async fn set_pinned_at(
    cache: PathBuf,
    account: AccountGuard,
    key: String,
    pinned: bool,
) -> Result<bool, String> {
    account.validate()?;
    let cache = cache.canonicalize().unwrap_or(cache);
    let directory = private_directory(
        &cache,
        &[
            "previews",
            "workspace",
            &account.owner.to_string(),
            "previews",
        ],
        false,
    )?;
    let _category = category_lock(&directory).read_owned().await;
    tokio::task::spawn_blocking(move || {
        let Some(path) = cached_preview_at(&cache, &account, &key)? else {
            return Ok(false);
        };
        account.validate()?;
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let store = Store::open(&account.root, account.owner)?;
        let _mutation = cache_state();
        if !std::fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.file_type().is_file() && meta.len() > 0)
        {
            return Ok(false);
        }
        let paired = vec![path.clone()];
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let mut paired = paired;
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if let Some(saved) = store.record::<HeicRendition>(
            "heic-rendition-v1",
            &heic_association_key(&key, &path.canonicalize().map_err(|e| e.to_string())?),
        )? {
            if saved.source == path
                && saved.rendition.parent() == Some(directory.as_path())
                && std::fs::symlink_metadata(&saved.rendition)
                    .is_ok_and(|meta| meta.file_type().is_file())
            {
                paired.push(saved.rendition);
            }
        }
        for path in &paired {
            let marker = path.with_extension("pin");
            if pinned {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create(true).truncate(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                use std::io::Write;
                let mut file = options.open(&marker).map_err(|e| e.to_string())?;
                file.write_all(b"pinned").map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
            } else if let Err(error) = std::fs::remove_file(&marker) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error.to_string());
                }
            }
        }
        drop(_mutation);
        if let Err(error) = account.validate() {
            if pinned {
                for path in paired {
                    let _ = std::fs::remove_file(path.with_extension("pin"));
                }
            }
            return Err(error);
        }
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) async fn delete_cached_at(
    cache: PathBuf,
    account: AccountGuard,
    key: String,
    thumbnail: bool,
) -> Result<(), String> {
    account.validate()?;
    cache_core::register(&cache, Some(&account.root))?;
    let owner = account.owner.to_string();
    let _file = file_lock(format!(
        "{}:{owner}:{key}",
        cache
            .canonicalize()
            .unwrap_or_else(|_| cache.clone())
            .to_string_lossy()
    ))
    .await
    .lock_owned()
    .await;
    let directory = private_directory(
        &cache,
        &[
            "previews",
            "workspace",
            &owner,
            if thumbnail { "thumbnails" } else { "previews" },
        ],
        false,
    )?;
    let _category = category_lock(&directory).read_owned().await;
    tokio::task::spawn_blocking(move || {
        account.validate()?;
        let Some(identity) = Store::open(&account.root, account.owner)?
            .record::<String>("asset-identity-v1", &key)?
        else {
            return Ok(());
        };
        let prefix = format!(
            "{:x}.",
            Sha256::digest(format!("raster-v2:{key}:{identity}:{thumbnail}").as_bytes())
        );
        let state = cache_state();
        for (path, _) in cache_entries(&directory)? {
            account.validate()?;
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
                && !kept(&path)
                && !state.paths.contains_key(&path)
            {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        account.validate()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(serde::Deserialize, serde::Serialize)]
struct HeicRendition {
    source: PathBuf,
    fingerprint: String,
    rendition: PathBuf,
}
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn heic_association_key(key: &str, source: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!("heic-source-pair-v1:{key}:{}", source.to_string_lossy()).as_bytes()
        )
    )
}
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn heic_source_allowed(
    account: &AccountGuard,
    cache: &Path,
    key: &str,
    original: &Path,
) -> Result<(), String> {
    account.validate()?;
    let store = Store::open(&account.root, account.owner)?;
    if !crate::legacy_external::current_allows(&store, key)? {
        return Err("ENCRYPTED_PREVIEW_UNAVAILABLE".into());
    }
    // Fresh previews need not create a plaintext inventory record. Their exact
    // raster identity and private account directory bind them to this file.
    let directory = private_directory(
        cache,
        &[
            "previews",
            "workspace",
            &account.owner.to_string(),
            "previews",
        ],
        false,
    )?;
    if let Some(identity) = store.record::<String>("asset-identity-v1", key)? {
        let prefix = format!(
            "{:x}.",
            Sha256::digest(format!("raster-v2:{key}:{identity}:false").as_bytes())
        );
        if original.parent() == Some(directory.as_path())
            && original
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
        {
            return Ok(());
        }
    }
    if let Some(proof) = crate::legacy_external::identify(account, cache, original)? {
        if proof.matches_key(key) && proof.check(account, cache, original)? {
            return Ok(());
        }
    }
    Err("HEIC_SOURCE_REFUSED".into())
}
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) async fn display_rendition_at(
    cache: PathBuf,
    account: AccountGuard,
    key: String,
    original: PathBuf,
    tools: crate::heic::Tools,
    request_id: Option<String>,
) -> Result<String, String> {
    if !crate::heic::is_name(original.to_str().unwrap_or_default()) {
        return Ok(original.to_string_lossy().into_owned());
    }
    account.validate()?;
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let cache = cache.canonicalize().map_err(|e| e.to_string())?;
    cache_core::register(&cache, Some(&account.root))?;
    let owner = account.owner.to_string();
    let directory =
        private_directory(&cache, &["previews", "workspace", &owner, "previews"], true)?;
    let request = Request::new(&owner, request_id, directory.clone())?;
    let capacity = ASSET_READERS
        .get_or_init(|| Arc::new(Semaphore::new(3)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "Preview stopped")?;
    let file_guard = file_lock(format!("{}:{owner}:{key}", cache.to_string_lossy()))
        .await
        .lock_owned()
        .await;
    let category = category_lock(&directory).read_owned().await;
    let source_guard = ActivePath::new(
        original.clone(),
        format!("heic-source:{}", request.token),
        false,
    );
    let legacy_guard = crate::commands::preview::LegacyPreviewUse::new(&original);
    // Clear must see both source markers released once the category lock opens,
    // including when only a detached blocking worker still owns this lease.
    let holds = Arc::new((legacy_guard, source_guard, capacity, file_guard, category));
    let scope = account.clone();
    let base = cache.clone();
    let source = original.clone();
    let record_key = key.clone();
    let flag = request.cancelled.clone();
    let validated = crate::external_files::verify_retained(
        account.clone(),
        cache.clone(),
        original.clone(),
        holds.clone(),
        move || {
            scope.validate()?;
            if flag.load(Ordering::SeqCst) {
                return Err("CANCELLED".into());
            }
            let metadata = std::fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
            if !metadata.file_type().is_file()
                || metadata.len() == 0
                || metadata.len() > crate::heic::SOURCE_LIMIT
            {
                return Err("HEIC_PREVIEW_UNAVAILABLE: Invalid or oversized source".into());
            }
            heic_source_allowed(&scope, &base, &record_key, &source)
        },
    )
    .await?;
    let fingerprint = validated.fingerprint.clone();
    let original = validated.checked(&account)?;
    let name = format!(
        "{:x}.jpg",
        Sha256::digest(
            format!(
                "heic-display-v1:{key}:{}:{fingerprint}",
                original.to_string_lossy()
            )
            .as_bytes()
        )
    );
    let target = directory.join(name);
    let target_guard = ActivePath::new(target.clone(), request.token.clone(), false);
    target_guard.cleanup_pin.store(true, Ordering::SeqCst);
    let reservation = if target.exists() {
        None
    } else {
        Some(Arc::new(
            CacheReservation::reserve(
                &directory,
                &request.token,
                crate::heic::OUTPUT_LIMIT * 2,
                false,
            )
            .await?,
        ))
    };
    let permit = if reservation.is_some() {
        Some(
            crate::heic::DECODERS
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| "Decoder stopped")?,
        )
    } else {
        None
    };
    let flag = request.cancelled.clone();
    let epoch = request.epoch;
    // Every lease is retained by the worker even when its awaiting caller aborts.
    tokio::task::spawn_blocking(move || {
        // Tuple fields drop in order: clean target/marker before releasing
        // the source, file and category leases, including error/unwind paths.
        let leases = (target_guard, holds, permit);
        let check = || -> Result<(), String> {
            account.validate()?;
            if flag.load(Ordering::SeqCst)
                || !cache_state().valid(&directory, epoch)
                || reservation.as_ref().is_some_and(|lease| lease.cancelled())
            {
                return Err("CANCELLED".into());
            }
            heic_source_allowed(&account, &cache, &key, &original)
        };
        check()?;
        if target.is_file() {
            crate::external_files::validate_lease(&account, &target)?.checked(&account)?;
            let after = crate::external_files::validate_lease(&account, &original)?;
            if after.fingerprint != fingerprint {
                return Err("FILE_CHANGED: Preview source changed".into());
            }
            after.checked(&account)?;
            check()?;
            return Ok(target.to_string_lossy().into_owned());
        }
        leases.0.disposable.store(true, Ordering::SeqCst);
        crate::heic::decode(&original, &target, &tools, (4096, 4096), check)?;
        let after = crate::external_files::validate_lease(&account, &original)?;
        if after.fingerprint != fingerprint {
            return Err("FILE_CHANGED: Preview source changed".into());
        }
        after.checked(&account)?;
        check()?;
        crate::external_files::register(&account, &target)?;
        let store = Store::open(&account.root, account.owner)?;
        store.transaction(|| {
            check()?;
            store.put_record(
                "heic-rendition-v1",
                &heic_association_key(&key, &original),
                &HeicRendition {
                    source: original.clone(),
                    fingerprint,
                    rendition: target.clone(),
                },
            )?;
            check()
        })?;
        {
            let _mutation = cache_state();
            let inherit_pin = kept(&original);
            #[cfg(feature = "native-e2e")]
            wait_heic_pin_gate()?;
            if inherit_pin {
                // Replace a marker entry rather than following a planted link.
                let marker = target.with_extension("pin");
                if let Err(error) = std::fs::remove_file(&marker) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(error.to_string());
                    }
                }
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                use std::io::Write;
                let mut file = options.open(&marker).map_err(|e| e.to_string())?;
                file.write_all(b"pinned").map_err(|e| e.to_string())?;
            }
        }
        check()?;
        leases.0.disposable.store(false, Ordering::SeqCst);
        Ok(target.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn track_decoder_partial(path: &Path, destination: &Path) -> Option<ActivePath> {
    let token = cache_state().paths.get(destination).cloned()?;
    Some(ActivePath::new(path.to_path_buf(), token, true))
}

#[cfg(feature = "native-e2e")]
static HEIC_PIN_GATE: std::sync::Mutex<Option<(PathBuf, PathBuf)>> = std::sync::Mutex::new(None);
#[cfg(feature = "native-e2e")]
pub(crate) fn test_heic_pin_gate(started: PathBuf, release: PathBuf) {
    *HEIC_PIN_GATE.lock().unwrap_or_else(|e| e.into_inner()) = Some((started, release));
}
#[cfg(feature = "native-e2e")]
fn wait_heic_pin_gate() -> Result<(), String> {
    let gate = HEIC_PIN_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some((started, release)) = gate {
        std::fs::write(started, b"ready").map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while !release.is_file() {
            if std::time::Instant::now() >= deadline {
                return Err("HEIC pin fixture deadline".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}
