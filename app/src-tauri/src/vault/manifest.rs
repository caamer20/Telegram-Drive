use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::crypto::WrappedSecret;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultManifest {
    pub version: u32,
    pub vault_id: String,
    pub generation: u64,
    pub bucket_id: i64,
    pub folders: Vec<FolderRecord>,
    pub files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderRecord {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub size: u64,
    pub mime_type: Option<String>,
    pub file_ext: Option<String>,
    pub created_at: String,
    pub modified_at: String,
    pub blob_message_id: i32,
    pub ciphertext_size: u64,
    pub chunk_size: u32,
    pub chunk_count: u64,
    pub wrapped_file_key: WrappedSecret,
}

impl VaultManifest {
    pub fn new(vault_id: String, bucket_id: i64) -> Self {
        Self {
            version: 1,
            vault_id,
            generation: 0,
            bucket_id,
            folders: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn touch_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }

    pub fn now() -> String {
        Utc::now().to_rfc3339()
    }

    pub fn folder_exists(&self, folder_id: Option<i64>) -> bool {
        match folder_id {
            None => true,
            Some(id) => self.folders.iter().any(|folder| folder.id == id),
        }
    }
}
