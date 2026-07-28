use crate::crypto::error::CryptoResult;
use crate::crypto::secret::SecretKey;
use std::collections::HashMap;
use std::path::PathBuf;

/// Encrypted derived cache for thumbnails, previews, and other derivatives.
///
/// All persistent derivatives of encrypted files must be encrypted with a
/// domain-separated cache key. Raw plaintext derivatives must never be stored
/// in the same directories used for plaintext file caches.
pub struct EncryptedDerivedCache {
    /// Base directory for encrypted derivative storage.
    /// Used by store()/load() once the full caching pipeline is implemented.
    #[allow(dead_code)]
    base_dir: PathBuf,
    cache_key: Option<SecretKey>,
    /// Track active cache entries for cleanup on lock.
    active_entries: HashMap<String, CacheEntry>,
}

/// Metadata for a cached derivative file on disk.
/// Fields will be consumed by the planned LRU eviction logic.
#[allow(dead_code)]
struct CacheEntry {
    path: PathBuf,
    size_bytes: u64,
}

impl EncryptedDerivedCache {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            cache_key: None,
            active_entries: HashMap::new(),
        }
    }

    /// Set the domain-separated cache encryption key (called after vault unlock).
    pub fn set_cache_key(&mut self, key: SecretKey) {
        self.cache_key = Some(key);
    }

    /// Clear the cache key and all active entries (called on lock).
    pub fn clear(&mut self) {
        self.cache_key = None;
        self.active_entries.clear();
    }

    /// Check if the cache is ready (key is set).
    pub fn is_ready(&self) -> bool {
        self.cache_key.is_some()
    }

    /// Store an encrypted derivative.
    pub fn store(
        &mut self,
        _file_uuid: &[u8; 16],
        _derivative_type: &str,
        _data: &[u8],
    ) -> CryptoResult<PathBuf> {
        if self.cache_key.is_none() {
            return Err(crate::crypto::error::CryptoError::vault_locked());
        }
        // Full implementation would:
        // 1. Derive a derivative-specific key from the cache key
        // 2. Encrypt the data
        // 3. Write to a randomized path in base_dir
        // 4. Track the entry
        Err(crate::crypto::error::CryptoError::internal(
            "EncryptedDerivedCache::store not yet implemented",
        ))
    }

    /// Load and decrypt a derivative.
    pub fn load(
        &self,
        _file_uuid: &[u8; 16],
        _derivative_type: &str,
    ) -> CryptoResult<Vec<u8>> {
        if self.cache_key.is_none() {
            return Err(crate::crypto::error::CryptoError::vault_locked());
        }
        Err(crate::crypto::error::CryptoError::internal(
            "EncryptedDerivedCache::load not yet implemented",
        ))
    }
}
