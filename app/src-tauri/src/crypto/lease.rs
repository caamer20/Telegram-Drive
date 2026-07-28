use crate::crypto::error::{CryptoError, CryptoResult};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Policy for temporary plaintext files on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempPolicy {
    /// Allow temporary plaintext files with strict controls.
    Balanced,
    /// Reject any operation requiring plaintext-on-disk.
    Strict,
}

/// A lease for a temporary plaintext file on disk.
///
/// Leases are tracked, have a maximum lifetime (TTL), and are cleaned up
/// on lock, exit, or expiry. The file is stored in an app-private directory
/// with a randomized name and owner-only permissions.
pub struct PlaintextLease {
    path: PathBuf,
    created_at: Instant,
    ttl: Duration,
    size_bytes: u64,
}

/// Manages temporary plaintext file leases for operations that require
/// plaintext on disk (e.g., RAR/7z extraction, FFmpeg input).
pub struct LeaseManager {
    policy: TempPolicy,
    leases: HashMap<String, PlaintextLease>,
    base_dir: PathBuf,
    max_total_bytes: u64,
    current_total_bytes: u64,
}

impl LeaseManager {
    pub fn new(base_dir: PathBuf, policy: TempPolicy) -> Self {
        Self {
            policy,
            leases: HashMap::new(),
            base_dir,
            max_total_bytes: 512 * 1024 * 1024, // 512 MiB default
            current_total_bytes: 0,
        }
    }

    /// Set the temporary plaintext policy.
    pub fn set_policy(&mut self, policy: TempPolicy) {
        self.policy = policy;
    }

    /// Check if an operation requiring a plaintext lease is allowed.
    pub fn check_policy(&self) -> CryptoResult<()> {
        match self.policy {
            TempPolicy::Balanced => Ok(()),
            TempPolicy::Strict => Err(CryptoError::new(
                crate::crypto::error::CryptoErrorCode::TempPolicyBlocked,
                "Operation requires temporary plaintext on disk which is blocked by strict policy",
            )),
        }
    }

    /// Create a new lease for plaintext data.
    /// Returns the path where plaintext data should be written.
    pub fn create_lease(
        &mut self,
        expected_size: u64,
        ttl: Duration,
    ) -> CryptoResult<PathBuf> {
        self.check_policy()?;

        // Check total limit
        if self.current_total_bytes + expected_size > self.max_total_bytes {
            // Try to clean up expired leases
            self.cleanup_expired();
            if self.current_total_bytes + expected_size > self.max_total_bytes {
                return Err(CryptoError::internal("Lease size limit exceeded"));
            }
        }

        let id = format!("lease_{}", crate::crypto::random::random_u64());
        let path = self.base_dir.join(&id);

        let lease = PlaintextLease {
            path: path.clone(),
            created_at: Instant::now(),
            ttl,
            size_bytes: 0,
        };

        self.leases.insert(id, lease);
        self.current_total_bytes += expected_size;

        Ok(path)
    }

    /// Release a lease and clean up its file.
    pub fn release_lease(&mut self, id: &str) {
        if let Some(lease) = self.leases.remove(id) {
            self.current_total_bytes = self.current_total_bytes.saturating_sub(lease.size_bytes);
            let _ = std::fs::remove_file(&lease.path);
        }
    }

    /// Clean up all expired leases.
    pub fn cleanup_expired(&mut self) {
        let now = Instant::now();
        let expired: Vec<String> = self
            .leases
            .iter()
            .filter(|(_, lease)| now.duration_since(lease.created_at) > lease.ttl)
            .map(|(id, _)| id.clone())
            .collect();

        for id in expired {
            self.release_lease(&id);
        }
    }

    /// Revoke all leases (called on lock/exit).
    pub fn revoke_all(&mut self) {
        let ids: Vec<String> = self.leases.keys().cloned().collect();
        for id in ids {
            self.release_lease(&id);
        }
    }

    /// Clean up any orphaned lease files in the base directory.
    pub fn startup_cleanup(&self) {
        if let Ok(entries) = std::fs::read_dir(&self.base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("lease_") {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }
}

impl Drop for LeaseManager {
    fn drop(&mut self) {
        self.revoke_all();
    }
}
