use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use super::crypto::{
    decrypt_aead, encrypt_aead, random_bytes, KdfParams, WrappedSecret, KEY_LEN, NONCE_LEN,
};
use super::manifest::VaultManifest;

const BLOB_MAGIC: &[u8; 8] = b"TDV1BLB!";
const BLOB_VERSION: u32 = 1;
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    pub version: u32,
    pub vault_id: String,
    pub bucket_id: i64,
    pub kdf: KdfParams,
    pub salt: String,
    pub wrapped_master_key: WrappedSecret,
    pub latest_manifest_message_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedManifestFile {
    pub magic: String,
    pub version: u32,
    pub vault_id: String,
    pub generation: u64,
    pub encrypted_manifest: WrappedSecret,
}

pub struct EncryptedFileStats {
    pub plaintext_size: u64,
    pub ciphertext_size: u64,
    pub chunk_size: u32,
    pub chunk_count: u64,
}

pub fn header_aad(vault_id: &str) -> Vec<u8> {
    format!("td:v1:header:{}", vault_id).into_bytes()
}

pub fn manifest_aad(vault_id: &str, generation: u64) -> Vec<u8> {
    format!("td:v1:manifest:{}:{}", vault_id, generation).into_bytes()
}

pub fn file_key_aad(vault_id: &str, file_id: i64) -> Vec<u8> {
    format!("td:v1:file-key:{}:{}", vault_id, file_id).into_bytes()
}

fn chunk_aad(vault_id: &str, file_id: i64, chunk_index: u64, plain_len: u32) -> Vec<u8> {
    let mut aad = Vec::with_capacity(vault_id.len() + 64);
    aad.extend_from_slice(b"td:v1:chunk:");
    aad.extend_from_slice(vault_id.as_bytes());
    aad.extend_from_slice(&file_id.to_le_bytes());
    aad.extend_from_slice(&chunk_index.to_le_bytes());
    aad.extend_from_slice(&plain_len.to_le_bytes());
    aad
}

pub fn encrypt_manifest(
    master_key: &[u8; KEY_LEN],
    manifest: &VaultManifest,
) -> Result<EncryptedManifestFile, String> {
    let plaintext =
        serde_json::to_vec(manifest).map_err(|e| format!("Failed to serialize manifest: {}", e))?;
    let aad = manifest_aad(&manifest.vault_id, manifest.generation);
    let encrypted_manifest = encrypt_aead(master_key, &plaintext, &aad)?;
    Ok(EncryptedManifestFile {
        magic: "TDV1MANIFEST".to_string(),
        version: 1,
        vault_id: manifest.vault_id.clone(),
        generation: manifest.generation,
        encrypted_manifest,
    })
}

pub fn decrypt_manifest(
    master_key: &[u8; KEY_LEN],
    encrypted: &EncryptedManifestFile,
) -> Result<VaultManifest, String> {
    if encrypted.magic != "TDV1MANIFEST" || encrypted.version != 1 {
        return Err("Unsupported manifest format".to_string());
    }
    let aad = manifest_aad(&encrypted.vault_id, encrypted.generation);
    let plaintext = decrypt_aead(master_key, &encrypted.encrypted_manifest, &aad)?;
    serde_json::from_slice(&plaintext).map_err(|e| format!("Failed to parse manifest: {}", e))
}

pub fn read_encrypted_manifest(path: &Path) -> Result<EncryptedManifestFile, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read manifest: {}", e))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Failed to parse encrypted manifest: {}", e))
}

pub fn write_encrypted_manifest(
    path: &Path,
    encrypted: &EncryptedManifestFile,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create vault dir: {}", e))?;
    }
    let bytes = serde_json::to_vec_pretty(encrypted)
        .map_err(|e| format!("Failed to serialize encrypted manifest: {}", e))?;
    std::fs::write(path, bytes).map_err(|e| format!("Failed to write manifest: {}", e))
}

pub fn encrypt_file_to_path(
    input_path: &Path,
    output_path: &Path,
    vault_id: &str,
    file_id: i64,
    file_key: &[u8; KEY_LEN],
) -> Result<EncryptedFileStats, String> {
    let input = File::open(input_path).map_err(|e| format!("Failed to open source file: {}", e))?;
    let mut reader = BufReader::new(input);
    let output = File::create(output_path)
        .map_err(|e| format!("Failed to create encrypted temp file: {}", e))?;
    let mut writer = BufWriter::new(output);
    let cipher = XChaCha20Poly1305::new_from_slice(file_key)
        .map_err(|_| "Invalid file key length".to_string())?;

    writer.write_all(BLOB_MAGIC).map_err(|e| e.to_string())?;
    writer
        .write_all(&BLOB_VERSION.to_le_bytes())
        .map_err(|e| e.to_string())?;
    writer
        .write_all(&file_id.to_le_bytes())
        .map_err(|e| e.to_string())?;
    writer
        .write_all(&(DEFAULT_CHUNK_SIZE as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;

    let mut plaintext_size = 0u64;
    let mut chunk_count = 0u64;
    let mut buffer = vec![0u8; DEFAULT_CHUNK_SIZE];

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Failed to read source file: {}", e))?;
        if read == 0 {
            break;
        }

        let plain_len = read as u32;
        let nonce = random_bytes::<NONCE_LEN>();
        let aad = chunk_aad(vault_id, file_id, chunk_count, plain_len);
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &buffer[..read],
                    aad: &aad,
                },
            )
            .map_err(|_| "File chunk encryption failed".to_string())?;

        writer.write_all(&nonce).map_err(|e| e.to_string())?;
        writer
            .write_all(&plain_len.to_le_bytes())
            .map_err(|e| e.to_string())?;
        writer
            .write_all(&(ciphertext.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        writer.write_all(&ciphertext).map_err(|e| e.to_string())?;

        plaintext_size += read as u64;
        chunk_count += 1;
    }

    buffer.zeroize();
    writer.flush().map_err(|e| e.to_string())?;
    let ciphertext_size = std::fs::metadata(output_path)
        .map_err(|e| format!("Failed to stat encrypted temp file: {}", e))?
        .len();

    Ok(EncryptedFileStats {
        plaintext_size,
        ciphertext_size,
        chunk_size: DEFAULT_CHUNK_SIZE as u32,
        chunk_count,
    })
}

pub fn decrypt_file_to_path(
    input_path: &Path,
    output_path: &Path,
    vault_id: &str,
    expected_file_id: i64,
    file_key: &[u8; KEY_LEN],
) -> Result<(), String> {
    let input =
        File::open(input_path).map_err(|e| format!("Failed to open encrypted file: {}", e))?;
    let mut reader = BufReader::new(input);
    let output =
        File::create(output_path).map_err(|e| format!("Failed to create output file: {}", e))?;
    let mut writer = BufWriter::new(output);

    let mut magic = [0u8; 8];
    reader
        .read_exact(&mut magic)
        .map_err(|e| format!("Invalid encrypted file header: {}", e))?;
    if &magic != BLOB_MAGIC {
        return Err("Unsupported encrypted file magic".to_string());
    }

    let version = read_u32(&mut reader)?;
    if version != BLOB_VERSION {
        return Err(format!("Unsupported encrypted file version: {}", version));
    }

    let file_id = read_i64(&mut reader)?;
    if file_id != expected_file_id {
        return Err("Encrypted file identity mismatch".to_string());
    }

    let _chunk_size = read_u32(&mut reader)?;
    let cipher = XChaCha20Poly1305::new_from_slice(file_key)
        .map_err(|_| "Invalid file key length".to_string())?;

    let mut chunk_index = 0u64;
    loop {
        let mut nonce = [0u8; NONCE_LEN];
        match reader.read_exact(&mut nonce) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Failed to read encrypted chunk nonce: {}", e)),
        }

        let plain_len = read_u32(&mut reader)?;
        let cipher_len = read_u32(&mut reader)? as usize;
        let mut ciphertext = vec![0u8; cipher_len];
        reader
            .read_exact(&mut ciphertext)
            .map_err(|e| format!("Failed to read encrypted chunk: {}", e))?;

        let aad = chunk_aad(vault_id, file_id, chunk_index, plain_len);
        let mut plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext.as_ref(),
                    aad: &aad,
                },
            )
            .map_err(|_| "Encrypted chunk authentication failed".to_string())?;

        if plaintext.len() != plain_len as usize {
            plaintext.zeroize();
            return Err("Encrypted chunk length mismatch".to_string());
        }
        writer.write_all(&plaintext).map_err(|e| e.to_string())?;
        plaintext.zeroize();
        ciphertext.zeroize();
        chunk_index += 1;
    }

    writer.flush().map_err(|e| e.to_string())
}

fn read_u32(reader: &mut BufReader<File>) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_i64(reader: &mut BufReader<File>) -> Result<i64, String> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(i64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip_and_tamper_rejection() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("plain.txt");
        let encrypted = tmp.path().join("cipher.bin");
        let output = tmp.path().join("out.txt");
        std::fs::write(&input, b"hello encrypted telegram drive").unwrap();
        let key = super::super::crypto::random_key();

        encrypt_file_to_path(&input, &encrypted, "vault-test", 42, &key).unwrap();
        decrypt_file_to_path(&encrypted, &output, "vault-test", 42, &key).unwrap();
        assert_eq!(
            std::fs::read(&input).unwrap(),
            std::fs::read(&output).unwrap()
        );

        let mut bytes = std::fs::read(&encrypted).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&encrypted, bytes).unwrap();
        assert!(decrypt_file_to_path(&encrypted, &output, "vault-test", 42, &key).is_err());
    }
}
