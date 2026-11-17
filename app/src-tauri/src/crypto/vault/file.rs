use crate::crypto::error::{CryptoError, CryptoErrorCode, CryptoResult};
use crate::crypto::secret::SecretKey;
use crate::crypto::vault::export::{create_recovery_bundle, import_recovery_bundle};
use crate::crypto::vault::{CryptoVault, RecoveryVerification, VaultCapture};
use crate::crypto::{kdf, policy, random};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

const VAULT_MAGIC: &[u8; 6] = b"TDVLT2";
const VAULT_VERSION: u16 = 2;
const VAULT_AAD_DOMAIN: &[u8] = b"telegram-drive:persistent-vault:v2";
const VAULT_HEADER_SIZE: usize = 64;
const MAX_VAULT_CIPHERTEXT: usize = 1024 * 1024;
const PAYLOAD_MAGIC: &[u8; 6] = b"TDVPL2";
const MAX_PROFILES: usize = 64;
const MAX_PROFILE_ID_BYTES: usize = 128;
/// Replaced vault files kept beside the vault so a mistaken recovery import
/// can be undone by restoring the newest archive.
const MAX_REPLACED_VAULT_ARCHIVES: usize = 5;
const VAULT_IDENTITY_DOMAIN: &[u8] = b"telegram-drive:vault-identity:v1";

pub struct FileVault {
    path: PathBuf,
    unlocked: bool,
    vault_key: Option<SecretKey>,
    unlock_key: Option<SecretKey>,
    vault_salt: Option<[u8; 16]>,
    profiles: HashMap<String, SecretKey>,
    created_at: i64,
    preparation: Option<PreparationStorage>,
    #[cfg(feature = "native-e2e")]
    fail_cleanup: bool,
}

/// Preparation intercepts every read/write/archive entry point. Workers can
/// derive and authenticate but cannot touch the captured files.
struct PreparationStorage {
    capture: Option<FileCapture>,
    pending: std::sync::Mutex<Option<Vec<u8>>>,
    archive: std::sync::atomic::AtomicBool,
}
struct FileCapture {
    primary: Option<FileFingerprint>,
    backup: Option<FileFingerprint>,
    bytes: Option<Vec<u8>>,
    access: VaultCapture,
}
#[derive(PartialEq, Eq)]
struct FileIdentity {
    length: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
#[derive(PartialEq, Eq)]
struct FileFingerprint {
    identity: FileIdentity,
    digest: Option<[u8; 32]>,
}

struct VaultFileHeader {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: [u8; 16],
    nonce: [u8; 24],
    ciphertext_length: usize,
}

impl FileVault {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            unlocked: false,
            vault_key: None,
            unlock_key: None,
            vault_salt: None,
            profiles: HashMap::new(),
            created_at: 0,
            preparation: None,
            #[cfg(feature = "native-e2e")]
            fail_cleanup: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn capture_file(path: &Path) -> CryptoResult<Option<Vec<u8>>> {
        match std::fs::File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take((VAULT_HEADER_SIZE + MAX_VAULT_CIPHERTEXT + 1) as u64)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > VAULT_HEADER_SIZE + MAX_VAULT_CIPHERTEXT {
                    return Err(CryptoError::header_invalid("Vault file exceeds policy"));
                }
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn file_identity(path: &Path) -> CryptoResult<Option<FileIdentity>> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Some(FileIdentity {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        }))
    }
    fn fingerprint(path: &Path, hash: bool) -> CryptoResult<Option<FileFingerprint>> {
        use sha2::{Digest, Sha256};
        let Some(identity) = Self::file_identity(path)? else {
            return Ok(None);
        };
        let digest = if hash {
            let mut input = std::fs::File::open(path)?;
            let mut hasher = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            if Self::file_identity(path)?.as_ref() != Some(&identity) {
                return Err(CryptoError::new(
                    CryptoErrorCode::PolicyRejected,
                    "VAULT_CHANGED: Vault file changed during capture",
                ));
            }
            Some(hasher.finalize().into())
        } else {
            None
        };
        Ok(Some(FileFingerprint { identity, digest }))
    }
    fn matches_fingerprint(path: &Path, expected: &Option<FileFingerprint>) -> CryptoResult<bool> {
        if Self::file_identity(path)?.as_ref() != expected.as_ref().map(|file| &file.identity) {
            return Ok(false);
        }
        Ok(Self::fingerprint(
            path,
            expected.as_ref().is_some_and(|file| file.digest.is_some()),
        )? == *expected)
    }
    fn captured_bytes(&self) -> CryptoResult<Vec<u8>> {
        let bytes = if let Some(preparation) = &self.preparation {
            preparation
                .capture
                .as_ref()
                .and_then(|capture| capture.bytes.clone())
        } else if let Some(primary) = Self::capture_file(&self.path)? {
            Some(primary)
        } else {
            Self::capture_file(&self.backup_path())?
        };
        bytes.ok_or_else(|| CryptoError::new(CryptoErrorCode::KeyRequired, "Vault does not exist"))
    }

    fn backup_path(&self) -> PathBuf {
        self.path.with_extension("vault.bak")
    }

    fn part_path(&self) -> PathBuf {
        self.path.with_extension("vault.part")
    }

    fn archive_prefix(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "vault".to_string());
        format!("{name}.replaced-")
    }

    /// Copy the current vault file aside before a recovery import replaces it.
    /// Returns without error when there is nothing to archive.
    fn archive_before_replacement(&self) -> CryptoResult<()> {
        if let Some(preparation) = &self.preparation {
            preparation
                .archive
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Ok(());
        }
        let Some(source) = self.readable_path() else {
            return Ok(());
        };
        let parent = self
            .path
            .parent()
            .ok_or_else(|| CryptoError::internal("Vault has no parent directory"))?;
        let prefix = self.archive_prefix();
        let stamp = crate::app_time::utc_millis_stamp();
        let archive = parent.join(format!("{prefix}{stamp}"));
        let mut input = std::fs::File::open(&source)?;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&archive)?;
        std::io::copy(&mut input, &mut file)?;
        file.sync_all()?;
        drop(file);

        // Keep only the newest archives; names sort chronologically.
        let mut archives: Vec<PathBuf> = std::fs::read_dir(parent)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().starts_with(&prefix))
                    .unwrap_or(false)
            })
            .collect();
        archives.sort();
        while archives.len() > MAX_REPLACED_VAULT_ARCHIVES {
            let _ = std::fs::remove_file(archives.remove(0));
        }
        Ok(())
    }

    /// Compare decrypted bundle material with the unlocked vault.
    fn compare_with_unlocked(
        &self,
        bundle_key: &SecretKey,
        bundle_profiles: &HashMap<String, SecretKey>,
    ) -> CryptoResult<RecoveryVerification> {
        let vault_key = self
            .vault_key
            .as_ref()
            .ok_or_else(CryptoError::vault_locked)?;
        let matches_vault_key =
            constant_time_eq::constant_time_eq(vault_key.expose(), bundle_key.expose());
        let missing_profiles = self
            .profiles
            .iter()
            .filter(|(profile_id, key)| {
                !bundle_profiles
                    .get(*profile_id)
                    .map(|candidate| {
                        constant_time_eq::constant_time_eq(candidate.expose(), key.expose())
                    })
                    .unwrap_or(false)
            })
            .count();
        Ok(RecoveryVerification {
            matches_vault_key,
            missing_profiles,
        })
    }

    fn readable_path(&self) -> Option<PathBuf> {
        if self.path.is_file() {
            Some(self.path.clone())
        } else {
            let backup = self.backup_path();
            backup.is_file().then_some(backup)
        }
    }

    fn encode_header(header: &VaultFileHeader) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(VAULT_HEADER_SIZE);
        bytes.extend_from_slice(VAULT_MAGIC);
        bytes.extend_from_slice(&VAULT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&header.memory_kib.to_le_bytes());
        bytes.extend_from_slice(&header.iterations.to_le_bytes());
        bytes.extend_from_slice(&header.parallelism.to_le_bytes());
        bytes.extend_from_slice(&header.salt);
        bytes.extend_from_slice(&header.nonce);
        bytes.extend_from_slice(&(header.ciphertext_length as u32).to_le_bytes());
        debug_assert_eq!(bytes.len(), VAULT_HEADER_SIZE);
        bytes
    }

    fn parse_header(bytes: &[u8]) -> CryptoResult<VaultFileHeader> {
        if bytes.len() < VAULT_HEADER_SIZE + policy::AEAD_TAG_LENGTH {
            return Err(CryptoError::truncated());
        }
        if &bytes[..6] != VAULT_MAGIC {
            return Err(CryptoError::header_invalid("Invalid vault magic"));
        }
        let version = u16::from_le_bytes([bytes[6], bytes[7]]);
        if version != VAULT_VERSION {
            return Err(CryptoError::unsupported_version(version));
        }
        let memory_kib = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| CryptoError::truncated())?,
        );
        let iterations = u32::from_le_bytes(
            bytes[12..16]
                .try_into()
                .map_err(|_| CryptoError::truncated())?,
        );
        let parallelism = u32::from_le_bytes(
            bytes[16..20]
                .try_into()
                .map_err(|_| CryptoError::truncated())?,
        );
        policy::validate_argon2_params(memory_kib, iterations, parallelism)?;
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&bytes[20..36]);
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&bytes[36..60]);
        let ciphertext_length = u32::from_le_bytes(
            bytes[60..64]
                .try_into()
                .map_err(|_| CryptoError::truncated())?,
        ) as usize;
        if !(policy::AEAD_TAG_LENGTH..=MAX_VAULT_CIPHERTEXT).contains(&ciphertext_length)
            || bytes.len() != VAULT_HEADER_SIZE + ciphertext_length
        {
            return Err(CryptoError::header_invalid(
                "Invalid vault ciphertext length",
            ));
        }
        Ok(VaultFileHeader {
            memory_kib,
            iterations,
            parallelism,
            salt,
            nonce,
            ciphertext_length,
        })
    }

    fn serialize_payload(&self) -> CryptoResult<Vec<u8>> {
        let vault_key = self
            .vault_key
            .as_ref()
            .ok_or_else(CryptoError::vault_locked)?;
        if self.profiles.len() > MAX_PROFILES {
            return Err(CryptoError::new(
                CryptoErrorCode::PolicyRejected,
                "Too many encryption profiles",
            ));
        }
        let mut payload = Vec::new();
        payload.extend_from_slice(PAYLOAD_MAGIC);
        payload.extend_from_slice(&self.created_at.to_le_bytes());
        payload.extend_from_slice(vault_key.expose());
        payload.extend_from_slice(&(self.profiles.len() as u16).to_le_bytes());
        let mut profile_ids: Vec<&String> = self.profiles.keys().collect();
        profile_ids.sort();
        for profile_id in profile_ids {
            let id = profile_id.as_bytes();
            if id.is_empty() || id.len() > MAX_PROFILE_ID_BYTES {
                return Err(CryptoError::new(
                    CryptoErrorCode::PolicyRejected,
                    "Invalid encryption profile identifier",
                ));
            }
            payload.extend_from_slice(&(id.len() as u16).to_le_bytes());
            payload.extend_from_slice(id);
            payload.extend_from_slice(
                self.profiles
                    .get(profile_id)
                    .ok_or_else(|| CryptoError::internal("Profile disappeared"))?
                    .expose(),
            );
        }
        Ok(payload)
    }

    fn parse_payload(payload: &[u8]) -> CryptoResult<(i64, SecretKey, HashMap<String, SecretKey>)> {
        if payload.len() < 48 || &payload[..6] != PAYLOAD_MAGIC {
            return Err(CryptoError::wrong_key_or_corrupt());
        }
        let created_at = i64::from_le_bytes(
            payload[6..14]
                .try_into()
                .map_err(|_| CryptoError::wrong_key_or_corrupt())?,
        );
        let mut vault_key = [0u8; 32];
        vault_key.copy_from_slice(&payload[14..46]);
        let count = u16::from_le_bytes(
            payload[46..48]
                .try_into()
                .map_err(|_| CryptoError::wrong_key_or_corrupt())?,
        ) as usize;
        if count > MAX_PROFILES {
            return Err(CryptoError::wrong_key_or_corrupt());
        }
        let mut cursor = 48usize;
        let mut profiles = HashMap::with_capacity(count);
        for _ in 0..count {
            let length_end = cursor
                .checked_add(2)
                .ok_or_else(CryptoError::size_overflow)?;
            if length_end > payload.len() {
                return Err(CryptoError::truncated());
            }
            let id_length = u16::from_le_bytes(
                payload[cursor..length_end]
                    .try_into()
                    .map_err(|_| CryptoError::truncated())?,
            ) as usize;
            cursor = length_end;
            if id_length == 0 || id_length > MAX_PROFILE_ID_BYTES {
                return Err(CryptoError::wrong_key_or_corrupt());
            }
            let id_end = cursor
                .checked_add(id_length)
                .ok_or_else(CryptoError::size_overflow)?;
            let key_end = id_end
                .checked_add(32)
                .ok_or_else(CryptoError::size_overflow)?;
            if key_end > payload.len() {
                return Err(CryptoError::truncated());
            }
            let profile_id = std::str::from_utf8(&payload[cursor..id_end])
                .map_err(|_| CryptoError::wrong_key_or_corrupt())?
                .to_string();
            let mut profile_key = [0u8; 32];
            profile_key.copy_from_slice(&payload[id_end..key_end]);
            if profiles
                .insert(profile_id, SecretKey::new(profile_key))
                .is_some()
            {
                return Err(CryptoError::wrong_key_or_corrupt());
            }
            cursor = key_end;
        }
        if cursor != payload.len() {
            return Err(CryptoError::header_invalid(
                "Trailing bytes in vault payload",
            ));
        }
        Ok((created_at, SecretKey::new(vault_key), profiles))
    }

    fn encrypt_payload_with_key(
        &self,
        payload: &[u8],
        unlock_key: &SecretKey,
        salt: [u8; 16],
    ) -> CryptoResult<Vec<u8>> {
        let nonce = random::random_wrap_nonce();
        let header = VaultFileHeader {
            memory_kib: policy::ARGON2_MEMORY_FLOOR_KIB,
            iterations: policy::ARGON2_ITERATIONS_FLOOR,
            parallelism: policy::ARGON2_PARALLELISM_FLOOR,
            salt,
            nonce,
            ciphertext_length: payload
                .len()
                .checked_add(policy::AEAD_TAG_LENGTH)
                .ok_or_else(CryptoError::size_overflow)?,
        };
        let header_bytes = Self::encode_header(&header);
        let mut aad = Vec::with_capacity(VAULT_AAD_DOMAIN.len() + header_bytes.len());
        aad.extend_from_slice(VAULT_AAD_DOMAIN);
        aad.extend_from_slice(&header_bytes);
        let cipher = XChaCha20Poly1305::new_from_slice(unlock_key.expose())
            .map_err(|_| CryptoError::internal("Invalid vault unlock key"))?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: payload,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::internal("Vault encryption failed"))?;
        if ciphertext.len() != header.ciphertext_length {
            return Err(CryptoError::internal("Vault ciphertext length mismatch"));
        }
        let mut file_bytes = header_bytes;
        file_bytes.extend_from_slice(&ciphertext);
        Ok(file_bytes)
    }

    fn atomic_write(&self, bytes: &[u8]) -> CryptoResult<()> {
        if let Some(preparation) = &self.preparation {
            *preparation
                .pending
                .lock()
                .map_err(|_| CryptoError::internal("Preparation lock poisoned"))? =
                Some(bytes.to_vec());
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| CryptoError::internal("Vault has no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        let part_path = self.part_path();
        let backup_path = self.backup_path();

        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&part_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);

        if self.path.exists() {
            if backup_path.exists() {
                std::fs::remove_file(&backup_path)?;
            }
            std::fs::rename(&self.path, &backup_path)?;
        }
        if let Err(error) = std::fs::rename(&part_path, &self.path) {
            if backup_path.exists() && !self.path.exists() {
                let _ = std::fs::rename(&backup_path, &self.path);
            }
            return Err(error.into());
        }
        // The replacement rename is the commit point. A cleanup failure must
        // not leave callers holding the old unlock material for the new file.
        if backup_path.exists() {
            #[cfg(feature = "native-e2e")]
            let cleanup = if self.fail_cleanup {
                Err(std::io::Error::other("Fixture post-commit cleanup failure"))
            } else {
                std::fs::remove_file(&backup_path)
            };
            #[cfg(not(feature = "native-e2e"))]
            let cleanup = std::fs::remove_file(&backup_path);
            if cleanup.is_err() {
                log::warn!(
                    "Vault replacement committed; backup cleanup will be retried on the next write"
                );
            }
        }
        Ok(())
    }

    fn persist_unlocked(&self) -> CryptoResult<()> {
        let unlock_key = self
            .unlock_key
            .as_ref()
            .ok_or_else(CryptoError::vault_locked)?;
        let salt = self
            .vault_salt
            .ok_or_else(|| CryptoError::internal("Missing vault salt"))?;
        let mut payload = self.serialize_payload()?;
        let result = self
            .encrypt_payload_with_key(&payload, unlock_key, salt)
            .and_then(|bytes| self.atomic_write(&bytes));
        payload.zeroize();
        result
    }

    /// Replace the vault's key material and persist it under `unlock`.
    /// On any failure the previous in-memory state is restored, so a failed
    /// import leaves an unlocked vault unlocked with its original keys.
    fn replace_material(
        &mut self,
        created_at: i64,
        vault_key: SecretKey,
        profiles: HashMap<String, SecretKey>,
        unlock_key: SecretKey,
        salt: [u8; 16],
    ) -> CryptoResult<()> {
        let previous = (
            self.created_at,
            self.vault_key.take(),
            std::mem::take(&mut self.profiles),
            self.unlock_key.take(),
            self.vault_salt.take(),
            self.unlocked,
        );
        self.created_at = created_at;
        self.vault_key = Some(vault_key);
        self.profiles = profiles;
        self.unlock_key = Some(unlock_key);
        self.vault_salt = Some(salt);
        self.unlocked = true;
        if let Err(error) = self.persist_unlocked() {
            self.created_at = previous.0;
            self.vault_key = previous.1;
            self.profiles = previous.2;
            self.unlock_key = previous.3;
            self.vault_salt = previous.4;
            self.unlocked = previous.5;
            return Err(error);
        }
        Ok(())
    }
}

impl CryptoVault for FileVault {
    fn preparation_snapshot(&self) -> CryptoResult<Box<dyn CryptoVault>> {
        if self.preparation.is_some() {
            return Err(CryptoError::internal("Nested vault preparation"));
        }
        Ok(Box::new(Self {
            path: self.path.clone(),
            unlocked: self.unlocked,
            vault_key: self.vault_key.clone(),
            unlock_key: self.unlock_key.clone(),
            vault_salt: self.vault_salt,
            profiles: self.profiles.clone(),
            created_at: self.created_at,
            preparation: Some(PreparationStorage {
                capture: None,
                pending: std::sync::Mutex::new(None),
                archive: std::sync::atomic::AtomicBool::new(false),
            }),
            #[cfg(feature = "native-e2e")]
            fail_cleanup: self.fail_cleanup,
        }))
    }
    fn capture_preparation_files(&mut self, access: VaultCapture) -> CryptoResult<()> {
        let capture = if access == VaultCapture::Resident {
            FileCapture {
                primary: None,
                backup: None,
                bytes: None,
                access,
            }
        } else if access == VaultCapture::Read {
            use sha2::{Digest, Sha256};
            let has_primary = Self::file_identity(&self.path)?.is_some();
            let selected = if has_primary {
                self.path.clone()
            } else {
                self.backup_path()
            };
            let identity = Self::file_identity(&selected)?;
            let bytes = Self::capture_file(&selected)?;
            if Self::file_identity(&selected)? != identity {
                return Err(CryptoError::new(
                    CryptoErrorCode::PolicyRejected,
                    "VAULT_CHANGED: Vault changed during capture",
                ));
            }
            let selected_fingerprint = match (&bytes, identity) {
                (Some(bytes), Some(identity)) => Some(FileFingerprint {
                    identity,
                    digest: Some(Sha256::digest(bytes).into()),
                }),
                (None, None) => None,
                _ => {
                    return Err(CryptoError::new(
                        CryptoErrorCode::PolicyRejected,
                        "VAULT_CHANGED: Vault changed during capture",
                    ))
                }
            };
            let other_path = if has_primary {
                self.backup_path()
            } else {
                self.path.clone()
            };
            let other = Self::fingerprint(&other_path, false)?;
            let (primary, backup) = if has_primary {
                (selected_fingerprint, other)
            } else {
                (other, selected_fingerprint)
            };
            FileCapture {
                primary,
                backup,
                bytes,
                access,
            }
        } else {
            let has_primary = Self::file_identity(&self.path)?.is_some();
            FileCapture {
                primary: Self::fingerprint(
                    &self.path,
                    access == VaultCapture::Replace && has_primary,
                )?,
                backup: Self::fingerprint(
                    &self.backup_path(),
                    access == VaultCapture::Replace && !has_primary,
                )?,
                bytes: None,
                access,
            }
        };
        self.preparation
            .as_mut()
            .ok_or_else(|| CryptoError::internal("Missing preparation"))?
            .capture = Some(capture);
        Ok(())
    }
    fn commit_prepared(&mut self) -> CryptoResult<()> {
        let preparation = self
            .preparation
            .take()
            .ok_or_else(|| CryptoError::internal("Vault was not prepared"))?;
        let capture = preparation
            .capture
            .ok_or_else(|| CryptoError::internal("Preparation disk input was not captured"))?;
        if capture.access == VaultCapture::Resident {
            return Err(CryptoError::internal("Read-only preparation cannot commit"));
        }
        if !Self::matches_fingerprint(&self.path, &capture.primary)?
            || !Self::matches_fingerprint(&self.backup_path(), &capture.backup)?
        {
            return Err(CryptoError::new(
                CryptoErrorCode::PolicyRejected,
                "VAULT_CHANGED: Vault files changed during preparation",
            ));
        }
        let pending = preparation
            .pending
            .into_inner()
            .map_err(|_| CryptoError::internal("Preparation lock poisoned"))?;
        if let Some(bytes) = pending {
            if preparation
                .archive
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                self.archive_before_replacement()?;
            }
            self.atomic_write(&bytes)?;
        }
        Ok(())
    }
    fn material_fingerprint(&self) -> CryptoResult<[u8; 32]> {
        use sha2::{Digest, Sha256};
        let mut payload = self.serialize_payload()?;
        let digest = Sha256::digest(&payload).into();
        payload.zeroize();
        Ok(digest)
    }

    #[cfg(feature = "native-e2e")]
    fn test_cleanup_failure(&mut self, fail: bool) {
        self.fail_cleanup = fail;
    }
    fn exists(&self) -> bool {
        if let Some(preparation) = &self.preparation {
            preparation
                .capture
                .as_ref()
                .is_some_and(|capture| capture.primary.is_some() || capture.backup.is_some())
        } else {
            self.path.is_file() || self.backup_path().is_file()
        }
    }

    fn create(&mut self, passphrase: &[u8]) -> CryptoResult<()> {
        if self.exists() {
            return Err(CryptoError::new(
                CryptoErrorCode::PolicyRejected,
                "A vault already exists",
            ));
        }
        if passphrase.len() < 8 {
            return Err(CryptoError::new(
                CryptoErrorCode::PolicyRejected,
                "Vault passphrase is too short",
            ));
        }
        self.created_at = crate::app_time::unix_seconds();
        self.vault_key = Some(SecretKey::new(random::random_key()));
        self.profiles.clear();
        let salt = random::random_salt();
        self.unlock_key = Some(kdf::derive_passphrase_key(
            passphrase,
            &salt,
            policy::ARGON2_MEMORY_FLOOR_KIB,
            policy::ARGON2_ITERATIONS_FLOOR,
            policy::ARGON2_PARALLELISM_FLOOR,
        )?);
        self.vault_salt = Some(salt);
        self.unlocked = true;
        if let Err(error) = self.persist_unlocked() {
            self.lock();
            return Err(error);
        }
        Ok(())
    }

    fn unlock(&mut self, passphrase: &[u8]) -> CryptoResult<()> {
        let mut bytes = self.captured_bytes()?;
        let header = Self::parse_header(&bytes)?;
        let unlock_key = kdf::derive_passphrase_key(
            passphrase,
            &header.salt,
            header.memory_kib,
            header.iterations,
            header.parallelism,
        )?;
        let header_bytes = &bytes[..VAULT_HEADER_SIZE];
        let mut aad = Vec::with_capacity(VAULT_AAD_DOMAIN.len() + VAULT_HEADER_SIZE);
        aad.extend_from_slice(VAULT_AAD_DOMAIN);
        aad.extend_from_slice(header_bytes);
        let cipher = XChaCha20Poly1305::new_from_slice(unlock_key.expose())
            .map_err(|_| CryptoError::internal("Invalid vault unlock key"))?;
        let decrypted = cipher.decrypt(
            XNonce::from_slice(&header.nonce),
            Payload {
                msg: &bytes[VAULT_HEADER_SIZE..],
                aad: &aad,
            },
        );
        bytes.zeroize();
        let mut payload = decrypted.map_err(|_| CryptoError::wrong_key_or_corrupt())?;
        let parsed = Self::parse_payload(&payload);
        payload.zeroize();
        let (created_at, vault_key, profiles) = parsed?;
        self.created_at = created_at;
        self.vault_key = Some(vault_key);
        self.profiles = profiles;
        self.unlock_key = Some(unlock_key);
        self.vault_salt = Some(header.salt);
        self.unlocked = true;
        Ok(())
    }

    fn lock(&mut self) {
        self.unlocked = false;
        self.vault_key = None;
        self.unlock_key = None;
        self.vault_salt = None;
        self.profiles.clear();
    }

    fn is_unlocked(&self) -> bool {
        self.unlocked
    }

    fn wrapping_key(&self) -> CryptoResult<&SecretKey> {
        if !self.unlocked {
            return Err(CryptoError::vault_locked());
        }
        self.vault_key
            .as_ref()
            .ok_or_else(CryptoError::vault_locked)
    }

    fn save_profile(&mut self, profile_id: &str, wrapping_key: SecretKey) -> CryptoResult<()> {
        if !self.unlocked {
            return Err(CryptoError::vault_locked());
        }
        if profile_id.is_empty() || profile_id.len() > MAX_PROFILE_ID_BYTES {
            return Err(CryptoError::new(
                CryptoErrorCode::PolicyRejected,
                "Invalid profile identifier",
            ));
        }
        self.profiles.insert(profile_id.to_string(), wrapping_key);
        self.persist_unlocked()
    }

    fn load_wrapping_key(&self, profile_id: &str) -> CryptoResult<SecretKey> {
        if !self.unlocked {
            return Err(CryptoError::vault_locked());
        }
        self.profiles
            .get(profile_id)
            .cloned()
            .ok_or_else(CryptoError::key_required)
    }

    fn change_passphrase(&mut self, new_passphrase: &[u8]) -> CryptoResult<()> {
        if !self.unlocked {
            return Err(CryptoError::vault_locked());
        }
        if new_passphrase.len() < 8 || new_passphrase.len() > 1024 {
            return Err(CryptoError::new(
                CryptoErrorCode::PolicyRejected,
                "Vault passphrase must be between 8 and 1024 bytes",
            ));
        }
        let salt = random::random_salt();
        let new_unlock_key = kdf::derive_passphrase_key(
            new_passphrase,
            &salt,
            policy::ARGON2_MEMORY_FLOOR_KIB,
            policy::ARGON2_ITERATIONS_FLOOR,
            policy::ARGON2_PARALLELISM_FLOOR,
        )?;
        let mut payload = self.serialize_payload()?;
        let encrypted = self.encrypt_payload_with_key(&payload, &new_unlock_key, salt);
        payload.zeroize();
        let file_bytes = encrypted?;
        self.atomic_write(&file_bytes)?;
        self.unlock_key = Some(new_unlock_key);
        self.vault_salt = Some(salt);
        Ok(())
    }

    fn export_bundle(&self, recovery_passphrase: &[u8]) -> CryptoResult<Vec<u8>> {
        if !self.unlocked {
            return Err(CryptoError::vault_locked());
        }
        let mut payload = self.serialize_payload()?;
        let result = create_recovery_bundle(&payload, recovery_passphrase);
        payload.zeroize();
        result
    }

    fn verify_bundle(
        &self,
        bundle: &[u8],
        recovery_passphrase: &[u8],
    ) -> CryptoResult<RecoveryVerification> {
        if !self.unlocked {
            return Err(CryptoError::vault_locked());
        }
        let mut payload = import_recovery_bundle(bundle, recovery_passphrase)?;
        let parsed = Self::parse_payload(&payload);
        payload.zeroize();
        let (_, bundle_key, bundle_profiles) = parsed?;
        self.compare_with_unlocked(&bundle_key, &bundle_profiles)
    }

    fn import_bundle(
        &mut self,
        bundle: &[u8],
        recovery_passphrase: &[u8],
        vault_passphrase: Option<&[u8]>,
        allow_key_replacement: bool,
    ) -> CryptoResult<()> {
        let mut payload = import_recovery_bundle(bundle, recovery_passphrase)?;
        let parsed = Self::parse_payload(&payload);
        payload.zeroize();
        let (created_at, bundle_key, bundle_profiles) = parsed?;

        if self.unlocked && !allow_key_replacement {
            let comparison = self.compare_with_unlocked(&bundle_key, &bundle_profiles)?;
            if !comparison.is_complete() {
                return Err(CryptoError::new(
                    CryptoErrorCode::PolicyRejected,
                    "[RECOVERY_BUNDLE_MISMATCH] This recovery bundle does not contain every key the current vault uses. Importing it would make files protected by the missing keys unreadable.",
                ));
            }
        }

        let (unlock_key, salt) = match vault_passphrase {
            Some(passphrase) => {
                if passphrase.len() < 8 || passphrase.len() > 1024 {
                    return Err(CryptoError::new(
                        CryptoErrorCode::PolicyRejected,
                        "Vault passphrase must be between 8 and 1024 bytes",
                    ));
                }
                let salt = random::random_salt();
                let unlock_key = kdf::derive_passphrase_key(
                    passphrase,
                    &salt,
                    policy::ARGON2_MEMORY_FLOOR_KIB,
                    policy::ARGON2_ITERATIONS_FLOOR,
                    policy::ARGON2_PARALLELISM_FLOOR,
                )?;
                (unlock_key, salt)
            }
            None => {
                // Keep the passphrase the user already unlocks this vault with.
                let unlock_key = self.unlock_key.clone().filter(|_| self.unlocked);
                match (unlock_key, self.vault_salt) {
                    (Some(unlock_key), Some(salt)) => (unlock_key, salt),
                    _ => {
                        return Err(CryptoError::new(
                            CryptoErrorCode::KeyRequired,
                            "[RECOVERY_VAULT_PASSPHRASE_REQUIRED] Choose the passphrase that will unlock the restored vault",
                        ));
                    }
                }
            }
        };

        self.archive_before_replacement()?;
        self.replace_material(created_at, bundle_key, bundle_profiles, unlock_key, salt)
    }

    fn identity(&self) -> CryptoResult<String> {
        use sha2::{Digest, Sha256};
        let vault_key = self
            .vault_key
            .as_ref()
            .filter(|_| self.unlocked)
            .ok_or_else(CryptoError::vault_locked)?;
        let mut hasher = Sha256::new();
        hasher.update(VAULT_IDENTITY_DOMAIN);
        hasher.update(vault_key.expose());
        let digest = hasher.finalize();
        Ok(digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

impl Drop for FileVault {
    fn drop(&mut self) {
        self.lock();
    }
}
