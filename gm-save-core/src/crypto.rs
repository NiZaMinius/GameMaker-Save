//! Cryptographic primitives used by the save system.
//!
//! # Algorithms
//! - **Key derivation**: PBKDF2-HMAC-SHA256, 100 000 iterations
//! - **Encryption**: ChaCha20-Poly1305 (authenticated — integrity is guaranteed)
//! - **Nonce**: 12 bytes, generated fresh from the OS CSPRNG on every save
//! - **Salt**: 16 bytes, generated fresh from the OS CSPRNG on every save
//!
//! A new (salt, nonce) pair per save means the same plaintext never produces
//! the same ciphertext twice, even with the same passphrase.

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    ChaCha20Poly1305, Key, Nonce,
};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

use crate::error::{Result, SaveError};

/// Length of the random salt used in PBKDF2 key derivation (bytes).
pub const SALT_LEN: usize = 16;

/// Length of the ChaCha20-Poly1305 nonce (bytes).
pub const NONCE_LEN: usize = 12;

/// Length of the derived ChaCha20 key (bytes).
pub const KEY_LEN: usize = 32;

/// Number of PBKDF2 rounds.
/// 100 000 is the OWASP minimum recommendation for PBKDF2-SHA256 (2023).
const PBKDF2_ROUNDS: u32 = 100_000;

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

/// Derives a 32-byte ChaCha20 key from `passphrase` and `salt` using
/// PBKDF2-HMAC-SHA256.
///
/// **Both `passphrase` and `salt` must be stored (or re-derived) consistently**
/// to decrypt the file later. The salt is stored in the file header; the
/// passphrase must remain constant across game versions.
pub fn derive_key(passphrase: &str, salt: &[u8; SALT_LEN]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, PBKDF2_ROUNDS, &mut key);
    key
}

// ---------------------------------------------------------------------------
// Encryption / Decryption
// ---------------------------------------------------------------------------

/// Encrypts `plaintext` with ChaCha20-Poly1305.
///
/// Generates a fresh random nonce on every call.
///
/// # Returns
/// `(nonce, ciphertext)` — the nonce must be stored alongside the ciphertext.
pub fn encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>)> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| SaveError::Crypto(e.to_string()))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(nonce.as_slice());

    Ok((nonce_bytes, ciphertext))
}

/// Decrypts and authenticates `ciphertext` with ChaCha20-Poly1305.
///
/// Returns [`SaveError::Crypto`] if the key is wrong, the nonce is wrong,
/// or the ciphertext has been tampered with.
pub fn decrypt(
    key: &[u8; KEY_LEN],
    nonce_bytes: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| SaveError::Crypto(e.to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_salt() -> [u8; SALT_LEN] {
        [0xDE; SALT_LEN]
    }

    #[test]
    fn derive_key_is_deterministic() {
        let s = fixed_salt();
        assert_eq!(derive_key("pass", &s), derive_key("pass", &s));
    }

    #[test]
    fn derive_key_differs_by_passphrase() {
        let s = fixed_salt();
        assert_ne!(derive_key("pass-a", &s), derive_key("pass-b", &s));
    }

    #[test]
    fn derive_key_differs_by_salt() {
        let s1 = [0u8; SALT_LEN];
        let s2 = [1u8; SALT_LEN];
        assert_ne!(derive_key("pass", &s1), derive_key("pass", &s2));
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = derive_key("roundtrip", &fixed_salt());
        let plaintext = b"Hello, GameMaker!";

        let (nonce, ct) = encrypt(&key, plaintext).unwrap();
        let recovered = decrypt(&key, &nonce, &ct).unwrap();

        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let key1 = derive_key("correct", &fixed_salt());
        let key2 = derive_key("wrong", &fixed_salt());

        let (nonce, ct) = encrypt(&key1, b"secret").unwrap();
        assert!(decrypt(&key2, &nonce, &ct).is_err());
    }

    #[test]
    fn decrypt_with_tampered_ciphertext_fails() {
        let key = derive_key("tamper-test", &fixed_salt());
        let (nonce, mut ct) = encrypt(&key, b"integrity").unwrap();

        // Flip the last byte — Poly1305 tag will reject this.
        let last = ct.len() - 1;
        ct[last] ^= 0xFF;

        assert!(decrypt(&key, &nonce, &ct).is_err());
    }

    #[test]
    fn nonces_are_unique_across_calls() {
        let key = derive_key("nonce-test", &fixed_salt());
        let (n1, _) = encrypt(&key, b"data").unwrap();
        let (n2, _) = encrypt(&key, b"data").unwrap();
        // Two random 12-byte nonces must not collide (probability ≈ 2⁻⁹⁶).
        assert_ne!(n1, n2);
    }

    #[test]
    fn encrypt_empty_plaintext() {
        let key = derive_key("empty", &fixed_salt());
        let (nonce, ct) = encrypt(&key, b"").unwrap();
        let recovered = decrypt(&key, &nonce, &ct).unwrap();
        assert!(recovered.is_empty());
    }
}

