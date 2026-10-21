//! PRIVATE state-file encryption (Android `StateTransferCrypto`): PBKDF2-HMAC-SHA256 (UTF-8
//! passphrase, 256-bit key), AES-256-GCM with a 12-byte nonce and the 128-bit tag appended to
//! the ciphertext (JCE layout), standard padded Base64.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rand::{CryptoRng, Rng};
use sha2::Sha256;

use crate::wire::{ANY_PLAYER_STATE_FORMAT, ENCRYPTION_ALGORITHM, ENCRYPTION_KDF, EncryptedStateFile};

/// Android's default PBKDF2 iteration count.
pub const DEFAULT_KDF_ITERATIONS: u32 = 150_000;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("Iteration count must be a positive number")]
    InvalidIterations,
    #[error("invalid Base64 in {0}")]
    Base64(&'static str),
    #[error("nonce must be {NONCE_LEN} bytes")]
    InvalidNonce,
    #[error("decryption failed (wrong passphrase or corrupted file)")]
    DecryptFailed,
    #[error("decrypted state is not UTF-8")]
    NotUtf8,
}

fn derive_key(passphrase: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, iterations, &mut key);
    key
}

/// Encrypts `plain_text` with fresh random salt and nonce from `rng`.
pub fn encrypt<R: Rng + CryptoRng + ?Sized>(
    plain_text: &str,
    passphrase: &str,
    iterations: u32,
    rng: &mut R,
) -> Result<EncryptedStateFile, CryptoError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill_bytes(&mut salt);
    rng.fill_bytes(&mut nonce);
    encrypt_with(plain_text, passphrase, iterations, &salt, &nonce)
}

/// Deterministic encryption with the given salt and nonce.
pub fn encrypt_with(
    plain_text: &str,
    passphrase: &str,
    iterations: u32,
    salt: &[u8],
    nonce: &[u8; NONCE_LEN],
) -> Result<EncryptedStateFile, CryptoError> {
    let kdf_iterations = i32::try_from(iterations).map_err(|_| CryptoError::InvalidIterations)?;
    if iterations == 0 {
        return Err(CryptoError::InvalidIterations);
    }
    let key = derive_key(passphrase, salt, iterations);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("32-byte key");
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(nonce), plain_text.as_bytes())
        .expect("AES-GCM encryption of in-memory data cannot fail");
    Ok(EncryptedStateFile {
        format: ANY_PLAYER_STATE_FORMAT.to_owned(),
        encrypted: true,
        algorithm: ENCRYPTION_ALGORITHM.to_owned(),
        kdf: ENCRYPTION_KDF.to_owned(),
        kdf_iterations,
        salt_base64: STANDARD.encode(salt),
        nonce_base64: STANDARD.encode(nonce),
        ciphertext_base64: STANDARD.encode(ciphertext),
    })
}

/// Decrypts a PRIVATE file back to the envelope text.
pub fn decrypt(file: &EncryptedStateFile, passphrase: &str) -> Result<String, CryptoError> {
    let iterations = u32::try_from(file.kdf_iterations)
        .ok()
        .filter(|&n| n > 0)
        .ok_or(CryptoError::InvalidIterations)?;
    let salt = STANDARD.decode(&file.salt_base64).map_err(|_| CryptoError::Base64("saltBase64"))?;
    let nonce = STANDARD.decode(&file.nonce_base64).map_err(|_| CryptoError::Base64("nonceBase64"))?;
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::InvalidNonce);
    }
    let ciphertext = STANDARD
        .decode(&file.ciphertext_base64)
        .map_err(|_| CryptoError::Base64("ciphertextBase64"))?;
    let key = derive_key(passphrase, &salt, iterations);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("32-byte key");
    let plain = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
        .map_err(|_| CryptoError::DecryptFailed)?;
    String::from_utf8(plain).map_err(|_| CryptoError::NotUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_wrong_passphrase() {
        let file = encrypt_with("{\"a\":1}", "pässwörd", 1000, &[7u8; 16], &[9u8; 12]).unwrap();
        assert_eq!(
            serde_json::to_string(&file).unwrap(),
            format!(
                r#"{{"kdfIterations":1000,"saltBase64":"BwcHBwcHBwcHBwcHBwcHBw==","nonceBase64":"CQkJCQkJCQkJCQkJ","ciphertextBase64":"{}"}}"#,
                file.ciphertext_base64
            )
        );
        assert_eq!(decrypt(&file, "pässwörd").unwrap(), "{\"a\":1}");
        assert_eq!(decrypt(&file, "nope"), Err(CryptoError::DecryptFailed));
        // 7 plaintext bytes + 16-byte tag.
        assert_eq!(STANDARD.decode(&file.ciphertext_base64).unwrap().len(), 7 + 16);
    }

    #[test]
    fn random_encrypt_round_trips() {
        let mut rng = rand::rng();
        let file = encrypt("x", "p", 1, &mut rng).unwrap();
        assert_eq!(decrypt(&file, "p").unwrap(), "x");
        assert_eq!(encrypt("x", "p", 0, &mut rng), Err(CryptoError::InvalidIterations));
    }
}
