//! Lazy authorization of exact cache files recorded by historical applications.
use crate::workspace::{assets, store::Store, AccountGuard};
use crate::{models::FileMetadata, workspace::store::WorkspaceFile};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
enum Origin {
    ProducedCache {
        key: String,
        name: String,
        size: u64,
        media_identity: String,
    },
    Cache {
        key: String,
        thumbnail: bool,
        flat: bool,
    },
    Offline {
        pack: String,
        key: String,
    },
}
#[derive(Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub(crate) struct Proof {
    origin: Origin,
}
impl Proof {
    pub(crate) fn produced_cache(
        key: String,
        path: &Path,
        size: u64,
        media_identity: String,
    ) -> Result<Self, String> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Invalid cache filename")?
            .into();
        Ok(Self {
            origin: Origin::ProducedCache {
                key,
                name,
                size,
                media_identity,
            },
        })
    }
    pub(crate) fn matches_key(&self, expected: &str) -> bool {
        match &self.origin {
            Origin::ProducedCache { key, .. }
            | Origin::Cache { key, .. }
            | Origin::Offline { key, .. } => key == expected,
        }
    }
}
fn private(base: &Path, parts: &[&str]) -> Result<Option<PathBuf>, String> {
    let mut directory = base.to_path_buf();
    for part in std::iter::once("").chain(parts.iter().copied()) {
        if !part.is_empty() {
            directory.push(part);
        }
        match std::fs::symlink_metadata(&directory) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err("FILE_OPEN_REFUSED: Cache directory is not private".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        }
    }
    directory
        .canonicalize()
        .map(Some)
        .map_err(|e| e.to_string())
}
fn plain(file: &WorkspaceFile) -> bool {
    file.file.encryption_state == "plain"
        && !file.file.name.to_ascii_lowercase().ends_with(".tdenc")
        && !file
            .file
            .file_ext
            .as_deref()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tdenc"))
        && file.key == crate::workspace::store::file_key(file.file.folder_id, file.file.id)
}
fn historical_name(file: &WorkspaceFile, thumbnail: bool) -> String {
    if thumbnail {
        format!("{:x}.jpg", Sha256::digest(file.key.as_bytes()))
    } else {
        assets::file_name(file)
    }
}
// 3.9.x listing metadata retains the original document extension even after
// caption-based renames. Photos retain jpg; extensionless documents use MIME.
fn flat_name(account: &AccountGuard, file: &WorkspaceFile, thumbnail: bool) -> String {
    let folder = file
        .file
        .folder_id
        .map_or_else(|| "home".into(), |id| id.to_string());
    let stem = format!("{}_{folder}_{}", account.owner, file.file.id);
    if thumbnail {
        return format!("{stem}.thumb.jpg");
    }
    let extension = file
        .file
        .file_ext
        .as_deref()
        .filter(|ext| !ext.is_empty())
        .map(str::to_lowercase)
        .unwrap_or_else(|| {
            match file.file.mime_type.as_deref() {
                Some("image/jpeg") => "jpg",
                Some("image/png") => "png",
                Some("image/gif") => "gif",
                Some("image/webp") => "webp",
                Some("image/bmp") => "bmp",
                Some("application/pdf") => "pdf",
                Some("video/mp4") => "mp4",
                _ => "bin",
            }
            .into()
        });
    let extension = if extension.len() <= 12 && extension.chars().all(|c| c.is_ascii_alphanumeric())
    {
        &extension
    } else {
        "bin"
    };
    format!("{stem}.{extension}")
}
fn cache_directory(
    account: &AccountGuard,
    cache: &Path,
    thumbnail: bool,
    flat: bool,
) -> Result<Option<PathBuf>, String> {
    if flat {
        if thumbnail {
            private(&account.root, &["thumbnails"])
        } else {
            private(cache, &["previews"])
        }
    } else {
        private(
            cache,
            &[
                "previews",
                "workspace",
                &account.owner.to_string(),
                if thumbnail { "thumbnails" } else { "previews" },
            ],
        )
    }
}
fn cache_match(
    store: &Store,
    file: &WorkspaceFile,
    name: &str,
    thumbnail: bool,
) -> Result<bool, String> {
    if !plain(file) {
        return Ok(false);
    }
    if historical_name(file, thumbnail) == name {
        return Ok(true);
    }
    let identity = store.record::<String>("asset-identity-v1", &file.key)?;
    Ok(
        identity
            .is_some_and(|identity| assets::disposable_name(file, &identity, thumbnail) == name),
    )
}
fn exact_file(
    path: &Path,
    directory: &Path,
    name: &str,
    size: Option<u64>,
) -> Result<bool, String> {
    if path.parent() != Some(directory) || path.file_name().and_then(|s| s.to_str()) != Some(name) {
        return Ok(false);
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    Ok(meta.file_type().is_file()
        && size.map_or(meta.len() > 0 && meta.len() <= 1024 * 1024, |size| {
            meta.len() == size
        })
        && path.canonicalize().map_err(|e| e.to_string())? == directory.join(name))
}
pub(crate) fn current_allows(store: &Store, key: &str) -> Result<bool, String> {
    if store.file(key)?.is_some_and(|file| !plain(&file)) {
        return Ok(false);
    }
    Ok(!store
        .record::<serde_json::Value>("removal", key)?
        .is_some_and(|r| {
            ["pending", "deleting", "deleted"].contains(&r["status"].as_str().unwrap_or(""))
        }))
}
fn offline_file(
    store: &Store,
    pack: &str,
    key: Option<&str>,
    name: &str,
) -> Result<Option<WorkspaceFile>, String> {
    let Some(record) = store.record::<serde_json::Value>("offline-pack", pack)? else {
        return Ok(None);
    };
    if record["ownerId"].as_str() != Some(store.owner.to_string().as_str())
        || record["id"].as_str() != Some(pack)
        || record["status"] == "expired"
        || record["expiresAt"]
            .as_i64()
            .is_some_and(|expiry| expiry <= crate::app_time::unix_millis())
    {
        return Ok(None);
    }
    for item in record["files"].as_array().into_iter().flatten() {
        if item["status"] != "ready" {
            continue;
        }
        let file: WorkspaceFile =
            serde_json::from_value(item["file"].clone()).map_err(|e| e.to_string())?;
        if plain(&file)
            && current_allows(store, &file.key)?
            && key.is_none_or(|key| key == file.key)
            && assets::file_name(&file) == name
            && item["downloadedBytes"].as_u64() == Some(file.file.size)
        {
            return Ok(Some(file));
        }
    }
    Ok(None)
}
pub(crate) fn identify(
    account: &AccountGuard,
    cache: &Path,
    path: &Path,
) -> Result<Option<Proof>, String> {
    account.validate()?;
    let owner = account.owner.to_string();
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    for (thumbnail, flat) in [(false, false), (true, false), (false, true), (true, true)] {
        let Some(directory) = cache_directory(account, cache, thumbnail, flat)? else {
            continue;
        };
        if path.parent() != Some(directory.as_path()) {
            continue;
        }
        let store = Store::open(&account.root, account.owner)?;
        let mut rows = store
            .db
            .prepare(concat!(
                "SELECT key,metadata FROM workspace_files WHERE NOT EXISTS ",
                "(SELECT 1 FROM workspace_records r WHERE r.kind='removal' ",
                "AND r.id=workspace_files.key AND json_extract(r.value,'$.status') ",
                "IN ('pending','deleting','deleted'))"
            ))
            .map_err(|e| e.to_string())?;
        while rows.next().map_err(|e| e.to_string())? == sqlite::State::Row {
            let key = rows.read::<String, _>(0).map_err(|e| e.to_string())?;
            let file: FileMetadata =
                serde_json::from_str(&rows.read::<String, _>(1).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let file = WorkspaceFile {
                file,
                key: key.clone(),
                folder_name: String::new(),
                tags: vec![],
                collection_ids: vec![],
            };
            let matches = if flat {
                plain(&file) && flat_name(account, &file, thumbnail) == name
            } else {
                cache_match(&store, &file, name, thumbnail)?
            };
            if matches {
                let proof = Proof {
                    origin: Origin::Cache {
                        key,
                        thumbnail,
                        flat,
                    },
                };
                return if proof.check(account, cache, path)? {
                    Ok(Some(proof))
                } else {
                    Ok(None)
                };
            }
        }
        return Ok(None);
    }
    let Some(base) = private(&account.root, &["workspace", &owner, "offline"])? else {
        return Ok(None);
    };
    let Some(parent) = path
        .parent()
        .filter(|parent| parent.parent() == Some(base.as_path()))
    else {
        return Ok(None);
    };
    let Some(pack) = parent
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| uuid::Uuid::parse_str(name).is_ok())
    else {
        return Ok(None);
    };
    let store = Store::open(&account.root, account.owner)?;
    let Some(file) = offline_file(&store, pack, None, name)? else {
        return Ok(None);
    };
    let proof = Proof {
        origin: Origin::Offline {
            pack: pack.into(),
            key: file.key,
        },
    };
    Ok(proof.check(account, cache, path)?.then_some(proof))
}
impl Proof {
    pub(crate) fn check(
        &self,
        account: &AccountGuard,
        cache: &Path,
        path: &Path,
    ) -> Result<bool, String> {
        account.validate()?;
        let owner = account.owner.to_string();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Ok(false);
        };
        let store = Store::open(&account.root, account.owner)?;
        let result = match &self.origin {
            Origin::ProducedCache {
                key,
                name,
                size,
                media_identity,
            } => {
                let Some(directory) = cache_directory(account, cache, false, false)? else {
                    return Ok(false);
                };
                current_allows(&store, key)?
                    && store.record::<String>("asset-identity-v1", key)?.as_ref()
                        == Some(media_identity)
                    && exact_file(path, &directory, name, Some(*size))?
            }
            Origin::Cache {
                key,
                thumbnail,
                flat,
            } => {
                let Some(directory) = cache_directory(account, cache, *thumbnail, *flat)? else {
                    return Ok(false);
                };
                let Some(file) = store.file(key)? else {
                    return Ok(false);
                };
                let removed = store
                    .record::<serde_json::Value>("removal", key)?
                    .is_some_and(|r| {
                        ["pending", "deleting", "deleted"]
                            .contains(&r["status"].as_str().unwrap_or(""))
                    });
                let matches = if *flat {
                    plain(&file) && flat_name(account, &file, *thumbnail) == name
                } else {
                    cache_match(&store, &file, name, *thumbnail)?
                };
                !removed
                    && matches
                    && exact_file(
                        path,
                        &directory,
                        name,
                        (!thumbnail).then_some(file.file.size),
                    )?
            }
            Origin::Offline { pack, key } => {
                let Some(directory) =
                    private(&account.root, &["workspace", &owner, "offline", pack])?
                else {
                    return Ok(false);
                };
                let Some(file) = offline_file(&store, pack, Some(key), name)? else {
                    return Ok(false);
                };
                exact_file(path, &directory, name, Some(file.file.size))?
            }
        };
        account.validate()?;
        Ok(result)
    }
}
pub(crate) fn historical_asset(
    account: &AccountGuard,
    cache: &Path,
    key: &str,
    thumbnail: bool,
) -> Result<Option<(PathBuf, Proof)>, String> {
    let store = Store::open(&account.root, account.owner)?;
    // A verified newer remote identity must never be shadowed by old bytes.
    if store.record::<String>("asset-identity-v1", key)?.is_some() {
        return Ok(None);
    }
    let Some(file) = store.file(key)?.filter(plain) else {
        return Ok(None);
    };
    for flat in [false, true] {
        let Some(directory) = cache_directory(account, cache, thumbnail, flat)? else {
            continue;
        };
        let name = if flat {
            flat_name(account, &file, thumbnail)
        } else {
            historical_name(&file, thumbnail)
        };
        let path = directory.join(name);
        if !path.exists() {
            continue;
        }
        let proof = Proof {
            origin: Origin::Cache {
                key: key.into(),
                thumbnail,
                flat,
            },
        };
        if proof.check(account, cache, &path)? {
            return Ok(Some((path, proof)));
        }
    }
    Ok(None)
}
pub(crate) fn historical_preview(
    account: &AccountGuard,
    cache: &Path,
    key: &str,
) -> Result<Option<PathBuf>, String> {
    Ok(historical_asset(account, cache, key, false)?.map(|(path, _)| path))
}

/// Hydrate proof for a previously registered disposable preview, including
/// legitimate unindexed media. Stream metadata once; never inspect siblings' bytes.
pub(crate) fn identify_registered(
    account: &AccountGuard,
    cache: &Path,
    path: &Path,
) -> Result<Option<Proof>, String> {
    if let Some(proof) = identify(account, cache, path)? {
        return Ok(Some(proof));
    }
    let Some(directory) = cache_directory(account, cache, false, false)? else {
        return Ok(None);
    };
    if path.parent() != Some(directory.as_path()) {
        return Ok(None);
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let Some((stem, extension)) = name.split_once('.') else {
        return Ok(None);
    };
    if extension.is_empty()
        || extension.len() > 12
        || !extension.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Ok(None);
    }
    let store = Store::open(&account.root, account.owner)?;
    let mut rows = store
        .db
        .prepare("SELECT id,value FROM workspace_records WHERE kind='asset-identity-v1'")
        .map_err(|e| e.to_string())?;
    while rows.next().map_err(|e| e.to_string())? == sqlite::State::Row {
        let key = rows.read::<String, _>(0).map_err(|e| e.to_string())?;
        let identity: String =
            serde_json::from_str(&rows.read::<String, _>(1).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if format!(
            "{:x}",
            Sha256::digest(format!("raster-v2:{key}:{identity}:false").as_bytes())
        ) != stem
        {
            continue;
        }
        let size = std::fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .len();
        let proof = Proof::produced_cache(key, path, size, identity)?;
        return Ok(proof.check(account, cache, path)?.then_some(proof));
    }
    Ok(None)
}
