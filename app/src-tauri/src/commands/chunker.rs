use std::fs::File;
use std::io::{Read, Write};
use sha2::{Sha256, Digest};
use tokio::fs;

/// Size limit: 2GB - 50MB buffer for Telegram overhead
pub const MAX_CHUNK_SIZE: u64 = 2_000_000_000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkInfo {
    pub part_number: u32,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkedFileMetadata {
    pub r#type: String, // "chunked_file"
    pub original_name: String,
    pub original_size: u64,
    pub chunk_count: u32,
    pub chunk_size: u64,
    pub parts_folder_id: Option<i64>,
    pub hash: String,
    pub chunks: Vec<ChunkInfo>,
    pub created_at: String,
}

impl ChunkedFileMetadata {
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json_string(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// Split a large file into chunks on disk.
/// Returns vector of temp chunk paths with metadata.
pub async fn split_file(
    source_path: &str,
    chunk_size: u64,
) -> Result<Vec<(String, ChunkInfo)>, String> {
    let source_file = tokio::fs::File::open(source_path)
        .await
        .map_err(|e| format!("Failed to open source file: {}", e))?;
    
    let file_size = source_file
        .metadata()
        .await
        .map_err(|e| format!("Failed to get file metadata: {}", e))?
        .len();

    let chunk_count = (file_size + chunk_size - 1) / chunk_size;
    log::info!(
        "Splitting {} bytes into {} chunks of {} bytes each",
        file_size,
        chunk_count,
        chunk_size
    );

    // Calculate full file hash upfront
    let full_file_hash = calculate_file_hash(source_path).await?;
    log::info!("Original file hash: {}", full_file_hash);

    let temp_dir = std::env::temp_dir();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    
    let source_name = std::path::Path::new(source_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    
    let mut chunks = Vec::new();
    let mut buffer = vec![0u8; (1024 * 1024 * 10).min(chunk_size as usize)];

    let mut source = std::fs::File::open(source_path)
        .map_err(|e| format!("Failed to open source for chunking: {}", e))?;

    for part_num in 0..chunk_count {
        let chunk_path = temp_dir.join(format!(
            "chunk_{}_{}.part{}",
            timestamp, source_name, part_num + 1
        ));
        let chunk_path_str = chunk_path.to_string_lossy().to_string();

        let mut chunk_file = File::create(&chunk_path)
            .map_err(|e| format!("Failed to create chunk file: {}", e))?;
        
        let mut bytes_written: u64 = 0;
        let remaining = file_size - (part_num * chunk_size);
        let this_chunk_size = std::cmp::min(chunk_size, remaining);

        while bytes_written < this_chunk_size {
            let to_read = std::cmp::min(
                buffer.len(),
                (this_chunk_size - bytes_written) as usize,
            );
            
            let n = source
                .read(&mut buffer[..to_read])
                .map_err(|e| format!("Failed to read from source: {}", e))?;
            
            if n == 0 {
                break;
            }

            chunk_file
                .write_all(&buffer[..n])
                .map_err(|e| format!("Failed to write to chunk: {}", e))?;
            
            bytes_written += n as u64;
        }

        chunk_file
            .sync_all()
            .map_err(|e| format!("Failed to sync chunk file: {}", e))?;
        drop(chunk_file);

        let chunk_hash = calculate_file_hash(&chunk_path_str).await?;
        log::info!(
            "Created chunk {}/{}: {} bytes, hash: {}",
            part_num + 1,
            chunk_count,
            bytes_written,
            chunk_hash
        );

        chunks.push((
            chunk_path_str,
            ChunkInfo {
                part_number: (part_num + 1) as u32,
                size: bytes_written,
                hash: chunk_hash,
            },
        ));
    }

    Ok(chunks)
}

/// Merge chunks back into a single file.
/// ⚠️ Does NOT delete temp chunk files — caller must clean up.
/// Chunks on Telegram Drive are kept permanently.
pub async fn merge_chunks(
    chunk_paths: Vec<String>,
    output_path: &str,
) -> Result<(String, String), String> {
    if chunk_paths.is_empty() {
        return Err("No chunks to merge".to_string());
    }

    let mut output_file = tokio::fs::File::create(output_path)
        .await
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    let mut hasher = Sha256::new();
    let mut total_written: u64 = 0;

    for (index, chunk_path) in chunk_paths.iter().enumerate() {
        log::info!("Merging chunk {}/{}: {}", index + 1, chunk_paths.len(), chunk_path);
        
        let mut chunk_file = tokio::fs::File::open(chunk_path)
            .await
            .map_err(|e| format!("Failed to open chunk {}: {}", chunk_path, e))?;

        let chunk_size = chunk_file
            .metadata()
            .await
            .map_err(|e| format!("Failed to get chunk metadata: {}", e))?
            .len();

        let mut buffer = vec![0u8; 1024 * 1024 * 10];
        let mut chunk_read: u64 = 0;

        loop {
            let n = chunk_file
                .read(&mut buffer)
                .await
                .map_err(|e| format!("Failed to read chunk: {}", e))?;
            
            if n == 0 {
                break;
            }

            output_file
                .write_all(&buffer[..n])
                .await
                .map_err(|e| format!("Failed to write merged file: {}", e))?;
            
            hasher.update(&buffer[..n]);
            chunk_read += n as u64;
            total_written += n as u64;
        }

        if chunk_read != chunk_size {
            return Err(format!(
                "Chunk read mismatch: expected {} bytes, read {}",
                chunk_size, chunk_read
            ));
        }
    }

    output_file
        .sync_all()
        .await
        .map_err(|e| format!("Failed to sync output file: {}", e))?;
    drop(output_file);

    let merged_hash = format!("{:x}", hasher.finalize());
    log::info!("Merge complete: {} bytes, hash: {}", total_written, merged_hash);

    Ok((output_path.to_string(), merged_hash))
}

/// Calculate SHA256 hash of a file
pub async fn calculate_file_hash(file_path: &str) -> Result<String, String> {
    let mut file = tokio::fs::File::open(file_path)
        .await
        .map_err(|e| format!("Failed to open file for hashing: {}", e))?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024 * 10];

    loop {
        let n = file
            .read(&mut buffer)
            .await
            .map_err(|e| format!("Failed to read file for hashing: {}", e))?;
        
        if n == 0 {
            break;
        }

        hasher.update(&buffer[..n]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Verify a merged file against expected hash
pub fn verify_hash(file_hash: &str, expected_hash: &str) -> Result<bool, String> {
    if file_hash.eq_ignore_ascii_case(expected_hash) {
        log::info!("Hash verification passed");
        Ok(true)
    } else {
        Err(format!(
            "Hash mismatch: expected {}, got {}",
            expected_hash, file_hash
        ))
    }
}

/// Clean up ONLY temporary local chunk files.
/// ⚠️ Chunks on Telegram Drive (in __*_parts folder) are NEVER deleted.
pub async fn cleanup_temp_chunks(chunk_paths: &[String]) {
    for path in chunk_paths {
        match tokio::fs::remove_file(path).await {
            Ok(_) => log::info!("Cleaned up temp chunk: {}", path),
            Err(e) => log::warn!("Failed to clean up temp chunk {}: {}", path, e),
        }
    }
}
