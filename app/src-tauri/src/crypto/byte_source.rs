use crate::crypto::error::{CryptoError, CryptoResult};
use std::future::Future;
use std::pin::Pin;

/// Represents the media source as either plaintext or encrypted ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRepresentation {
    Plaintext,
    Ciphertext,
}

/// Logical metadata about a byte source (plaintext or encrypted).
#[derive(Debug, Clone)]
pub struct LogicalMediaMetadata {
    pub representation: MediaRepresentation,
    pub logical_size: u64,
    pub ciphertext_size: Option<u64>,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
    pub encryption_state: EncryptionState,
}

/// The encryption state of a file as visible to the UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionState {
    Plain,
    EncryptedUnlocked,
    EncryptedLocked,
    EncryptedKeyMissing,
    EncryptedUnsupportedVersion,
    EncryptedCorrupt,
    EncryptedVerifying,
}

/// Type alias for boxed async results used in trait object-safe methods.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A logical byte source that abstracts over plaintext and encrypted files.
///
/// All media access (previews, streaming, downloads, archives, transcode) should
/// route through this trait rather than implementing decryption per-path.
pub trait LogicalMediaSource: Send + Sync {
    /// Get metadata about this source.
    fn metadata(&self) -> &LogicalMediaMetadata;

    /// Read a range of plaintext bytes from the source.
    /// For encrypted sources, this maps the plaintext range to ciphertext records,
    /// fetches and authenticates them, and returns the plaintext slice.
    fn read_range(
        &self,
        plaintext_start: u64,
        plaintext_end: u64,
    ) -> BoxFuture<'_, CryptoResult<Vec<u8>>>;

    /// Stream all plaintext bytes sequentially.
    fn stream_all(&self) -> BoxFuture<'_, CryptoResult<Vec<u8>>>;

    /// Check if this source is currently accessible (e.g., vault unlocked).
    fn is_accessible(&self) -> bool;
}

/// Concrete implementation for plaintext sources.
pub struct PlaintextSource {
    metadata: LogicalMediaMetadata,
    data: Vec<u8>,
}

impl PlaintextSource {
    pub fn new(data: Vec<u8>, mime_type: Option<String>, filename: Option<String>) -> Self {
        let len = data.len() as u64;
        Self {
            metadata: LogicalMediaMetadata {
                representation: MediaRepresentation::Plaintext,
                logical_size: len,
                ciphertext_size: None,
                mime_type,
                filename,
                encryption_state: EncryptionState::Plain,
            },
            data,
        }
    }
}

impl LogicalMediaSource for PlaintextSource {
    fn metadata(&self) -> &LogicalMediaMetadata {
        &self.metadata
    }

    fn read_range(
        &self,
        start: u64,
        end: u64,
    ) -> BoxFuture<'_, CryptoResult<Vec<u8>>> {
        Box::pin(async move {
            let start = start as usize;
            let end = (end as usize).min(self.data.len() - 1);
            if start > end || start >= self.data.len() {
                return Err(CryptoError::header_invalid("Range out of bounds"));
            }
            Ok(self.data[start..=end].to_vec())
        })
    }

    fn stream_all(&self) -> BoxFuture<'_, CryptoResult<Vec<u8>>> {
        Box::pin(async move {
            Ok(self.data.clone())
        })
    }

    fn is_accessible(&self) -> bool {
        true
    }
}
