use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose, Engine as _};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedSecret {
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub algorithm: String,
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            algorithm: "argon2id".to_string(),
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::thread_rng().fill_bytes(&mut out);
    out
}

pub fn random_key() -> [u8; KEY_LEN] {
    random_bytes::<KEY_LEN>()
}

pub fn b64_encode(bytes: &[u8]) -> String {
    general_purpose::STANDARD.encode(bytes)
}

pub fn b64_decode(value: &str) -> Result<Vec<u8>, String> {
    general_purpose::STANDARD
        .decode(value)
        .map_err(|e| format!("Invalid base64: {}", e))
}

pub fn derive_unlock_key(
    password: &str,
    salt: &[u8],
    params: &KdfParams,
) -> Result<[u8; KEY_LEN], String> {
    if params.algorithm != "argon2id" {
        return Err(format!("Unsupported KDF: {}", params.algorithm));
    }

    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|e| format!("Invalid KDF params: {}", e))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| format!("Password derivation failed: {}", e))?;
    Ok(key)
}

pub fn encrypt_aead(
    key: &[u8; KEY_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<WrappedSecret, String> {
    let nonce = random_bytes::<NONCE_LEN>();
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| "Invalid encryption key length".to_string())?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| "Encryption failed".to_string())?;

    Ok(WrappedSecret {
        nonce: b64_encode(&nonce),
        ciphertext: b64_encode(&ciphertext),
    })
}

pub fn decrypt_aead(
    key: &[u8; KEY_LEN],
    wrapped: &WrappedSecret,
    aad: &[u8],
) -> Result<Vec<u8>, String> {
    let nonce = b64_decode(&wrapped.nonce)?;
    if nonce.len() != NONCE_LEN {
        return Err("Invalid nonce length".to_string());
    }
    let ciphertext = b64_decode(&wrapped.ciphertext)?;
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| "Invalid encryption key length".to_string())?;
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ciphertext.as_ref(),
                aad,
            },
        )
        .map_err(|_| "Authentication failed".to_string())
}

pub fn wrap_key(
    wrapping_key: &[u8; KEY_LEN],
    key_to_wrap: &[u8; KEY_LEN],
    aad: &[u8],
) -> Result<WrappedSecret, String> {
    encrypt_aead(wrapping_key, key_to_wrap, aad)
}

pub fn unwrap_key(
    wrapping_key: &[u8; KEY_LEN],
    wrapped: &WrappedSecret,
    aad: &[u8],
) -> Result<[u8; KEY_LEN], String> {
    let mut plaintext = decrypt_aead(wrapping_key, wrapped, aad)?;
    if plaintext.len() != KEY_LEN {
        plaintext.zeroize();
        return Err("Invalid wrapped key length".to_string());
    }
    let mut out = [0u8; KEY_LEN];
    out.copy_from_slice(&plaintext);
    plaintext.zeroize();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aead_rejects_wrong_associated_data() {
        let key = random_key();
        let wrapped = encrypt_aead(&key, b"secret", b"aad-1").unwrap();
        assert!(decrypt_aead(&key, &wrapped, b"aad-2").is_err());
    }

    #[test]
    fn key_wrap_round_trip() {
        let master = random_key();
        let file_key = random_key();
        let wrapped = wrap_key(&master, &file_key, b"file-key").unwrap();
        let unwrapped = unwrap_key(&master, &wrapped, b"file-key").unwrap();
        assert_eq!(file_key, unwrapped);
    }
}
