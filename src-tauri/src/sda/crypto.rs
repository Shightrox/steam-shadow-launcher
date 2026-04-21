//! Cryptographic helpers for SDA login flow and local vault encryption.
//!
//! - RSA-PKCS#1v1.5 password encryption for `BeginAuthSessionViaCredentials`.
//! - PBKDF2-SHA1 + AES-256-CBC decrypt (SDA-compatible maFile import).
//! - Argon2id + AES-256-GCM encrypt/decrypt (our own vault format).

use crate::error::{AppError, AppResult};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
use rsa::{BigUint, Pkcs1v15Encrypt, RsaPublicKey};

/// Decode a Steam-style hex-encoded big-endian integer (as returned by
/// `GetPasswordRSAPublicKey`).
fn parse_hex_biguint(s: &str) -> AppResult<BigUint> {
    let clean = s.trim();
    let bytes = (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|e| AppError::Other(format!("RSA hex decode: {e}")))?;
    Ok(BigUint::from_bytes_be(&bytes))
}

/// Encrypt a password with Steam's RSA public key; result is base64.
pub fn encrypt_password(password: &str, mod_hex: &str, exp_hex: &str) -> AppResult<String> {
    let n = parse_hex_biguint(mod_hex)?;
    let e = parse_hex_biguint(exp_hex)?;
    let pk = RsaPublicKey::new(n, e)
        .map_err(|e| AppError::Other(format!("RSA key build: {e}")))?;
    let mut rng = rand::thread_rng();
    let cipher = pk
        .encrypt(&mut rng, Pkcs1v15Encrypt, password.as_bytes())
        .map_err(|e| AppError::Other(format!("RSA encrypt: {e}")))?;
    Ok(B64.encode(cipher))
}

// ── SDA-encrypted maFile decrypt ──────────────────────────────────────────
//
// SDA encrypts its `.maFile` with:
//   passcode -> PBKDF2-SHA1(passcode, salt, 50_000 iters, 32-byte key)
//   IV is base64-encoded 16 bytes, stored in manifest entry (encryption_iv).
//   Ciphertext = AES-256-CBC(plaintext with PKCS7 padding).
//   Both ciphertext and IV are base64 in the file.
//
// We only support decrypt (for import); our own format is Argon2id+AES-GCM.

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// Decrypt the body of an SDA-encrypted maFile.
///
/// `salt_b64` and `iv_b64` come from the SDA `manifest.json` entry
/// (`encryption_salt` / `encryption_iv`). `ciphertext_b64` is the content
/// of the `.maFile` itself (which is base64 wrapped around AES-CBC output).
pub fn sda_decrypt(
    passcode: &str,
    salt_b64: &str,
    iv_b64: &str,
    ciphertext_b64: &str,
) -> AppResult<Vec<u8>> {
    let salt = B64
        .decode(salt_b64.trim())
        .map_err(|e| AppError::Other(format!("SDA salt: {e}")))?;
    let iv = B64
        .decode(iv_b64.trim())
        .map_err(|e| AppError::Other(format!("SDA iv: {e}")))?;
    if iv.len() != 16 {
        return Err(AppError::Other("SDA iv must be 16 bytes".into()));
    }
    let mut ciphertext = B64
        .decode(ciphertext_b64.trim())
        .map_err(|e| AppError::Other(format!("SDA body: {e}")))?;
    // PBKDF2-SHA1 50_000 iters → 32-byte AES-256 key.
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(passcode.as_bytes(), &salt, 50_000, &mut key);
    let cipher = Aes256CbcDec::new_from_slices(&key, &iv)
        .map_err(|e| AppError::Other(format!("AES-CBC init: {e}")))?;
    let plain = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|_| AppError::Other("SDA_BAD_PASSWORD".into()))?;
    Ok(plain.to_vec())
}

// ── Local vault: Argon2id + AES-256-GCM ──────────────────────────────────
//
// File layout:
//   magic         : 5 bytes  = "SSLv1"
//   salt          : 16 bytes
//   nonce         : 12 bytes
//   ciphertext+tag: remainder
//
// KDF params: Argon2id m=19 MiB, t=2, p=1 (OWASP 2024 recommendation).

pub const VAULT_MAGIC: &[u8; 5] = b"SSLv1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

pub fn derive_key(password: &str, salt: &[u8]) -> AppResult<[u8; KEY_LEN]> {
    let params = Params::new(19 * 1024, 2, 1, Some(KEY_LEN))
        .map_err(|e| AppError::Other(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| AppError::Other(format!("argon2 derive: {e}")))?;
    Ok(out)
}

/// Encrypt plaintext with a random salt+nonce, returning the framed blob.
pub fn vault_encrypt(password: &str, plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let mut rng = rand::thread_rng();
    let mut salt = [0u8; SALT_LEN];
    rng.fill_bytes(&mut salt);
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill_bytes(&mut nonce);
    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AppError::Other(format!("AES-GCM init: {e}")))?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| AppError::Other(format!("AES-GCM encrypt: {e}")))?;
    let mut out = Vec::with_capacity(VAULT_MAGIC.len() + SALT_LEN + NONCE_LEN + ct.len());
    out.extend_from_slice(VAULT_MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt a vault blob. `VAULT_BAD_PASSWORD` on auth-tag failure.
pub fn vault_decrypt(password: &str, blob: &[u8]) -> AppResult<Vec<u8>> {
    if blob.len() < VAULT_MAGIC.len() + SALT_LEN + NONCE_LEN + 16 {
        return Err(AppError::Other("VAULT_TRUNCATED".into()));
    }
    if &blob[..VAULT_MAGIC.len()] != VAULT_MAGIC {
        return Err(AppError::Other("VAULT_BAD_MAGIC".into()));
    }
    let salt = &blob[VAULT_MAGIC.len()..VAULT_MAGIC.len() + SALT_LEN];
    let nonce = &blob[VAULT_MAGIC.len() + SALT_LEN..VAULT_MAGIC.len() + SALT_LEN + NONCE_LEN];
    let ct = &blob[VAULT_MAGIC.len() + SALT_LEN + NONCE_LEN..];
    let key = derive_key(password, salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AppError::Other(format!("AES-GCM init: {e}")))?;
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| AppError::Other("VAULT_BAD_PASSWORD".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let data = b"secret-mafile-content";
        let pw = "correct-horse";
        let blob = vault_encrypt(pw, data).unwrap();
        assert_eq!(&blob[..5], VAULT_MAGIC);
        let out = vault_decrypt(pw, &blob).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn bad_password() {
        let blob = vault_encrypt("a", b"x").unwrap();
        let err = vault_decrypt("b", &blob).unwrap_err();
        assert!(err.to_string().contains("VAULT_BAD_PASSWORD"));
    }
}
