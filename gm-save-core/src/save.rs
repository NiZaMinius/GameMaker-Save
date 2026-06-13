//! Slot-based save file management.
//!
//! # Binary format
//!
//! Every save file (extension `.gmsv`) has the following layout:
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │  4 bytes  magic        b"GMSV"               │
//! │  4 bytes  version      u32, little-endian     │
//! │ 16 bytes  salt         random (PBKDF2 input)  │
//! │ 12 bytes  nonce        random (ChaCha20 input)│
//! │  8 bytes  timestamp    i64 LE, Unix seconds   │
//! │  4 bytes  payload_len  u32 LE                 │
//! │  N bytes  payload      ChaCha20-Poly1305      │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! Files are written **atomically**: the data is first written to a `.tmp`
//! file in the same directory, then renamed over the target. On all major
//! operating systems, `rename` is atomic at the filesystem level, so a crash
//! mid-save will never produce a half-written slot file.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use zeroize::Zeroize;

use crate::crypto::{decrypt, derive_key, encrypt, NONCE_LEN, SALT_LEN};
use crate::error::{Result, SaveError};

// ---------------------------------------------------------------------------
// Format constants
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 4] = b"GMSV";
const FORMAT_VERSION: u32 = 1;

// Byte offsets / lengths — named so the parser never contains magic numbers.
const OFF_MAGIC: usize = 0;
const LEN_MAGIC: usize = 4;
const OFF_VERSION: usize = OFF_MAGIC + LEN_MAGIC; //  4
const LEN_VERSION: usize = 4;
const OFF_SALT: usize = OFF_VERSION + LEN_VERSION; //  8
const LEN_SALT: usize = SALT_LEN; // 16
const OFF_NONCE: usize = OFF_SALT + LEN_SALT; // 24
const LEN_NONCE: usize = NONCE_LEN; // 12
const OFF_TIMESTAMP: usize = OFF_NONCE + LEN_NONCE; // 36
const LEN_TIMESTAMP: usize = 8;
const OFF_PAYLOAD_LEN: usize = OFF_TIMESTAMP + LEN_TIMESTAMP; // 44
const LEN_PAYLOAD_LEN: usize = 4;
const HEADER_SIZE: usize = OFF_PAYLOAD_LEN + LEN_PAYLOAD_LEN; // 48

// ---------------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------------

/// Raw fields extracted from a `.gmsv` file header.
///
/// Used internally by both [`SaveManager::load`] and [`SaveManager::slot_meta`]
/// so that the binary format is parsed in exactly one place.
struct RawHeader {
    version: u32,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    timestamp: i64,
    payload_len: usize,
}

/// Parses the fixed-size header from a byte slice.
///
/// `data` must be at least [`HEADER_SIZE`] bytes long.  This function
/// validates the magic bytes but does **not** check the format version —
/// callers decide how to handle mismatches.
fn parse_header(data: &[u8]) -> Result<RawHeader> {
    if data.len() < HEADER_SIZE {
        return Err(SaveError::InvalidFormat("file is too short".into()));
    }
    if &data[OFF_MAGIC..OFF_MAGIC + LEN_MAGIC] != MAGIC {
        return Err(SaveError::InvalidFormat("bad magic bytes".into()));
    }

    let version = u32::from_le_bytes(
        data[OFF_VERSION..OFF_VERSION + LEN_VERSION]
            .try_into()
            .unwrap(),
    );

    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&data[OFF_SALT..OFF_SALT + LEN_SALT]);

    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&data[OFF_NONCE..OFF_NONCE + LEN_NONCE]);

    let timestamp = i64::from_le_bytes(
        data[OFF_TIMESTAMP..OFF_TIMESTAMP + LEN_TIMESTAMP]
            .try_into()
            .unwrap(),
    );

    let payload_len = u32::from_le_bytes(
        data[OFF_PAYLOAD_LEN..OFF_PAYLOAD_LEN + LEN_PAYLOAD_LEN]
            .try_into()
            .unwrap(),
    ) as usize;

    Ok(RawHeader {
        version,
        salt,
        nonce,
        timestamp,
        payload_len,
    })
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Metadata readable from a slot file **without** decrypting the payload.
///
/// Useful for showing save-selection screens ("Last saved 5 minutes ago").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMeta {
    /// Slot number.
    pub slot: u32,
    /// Unix timestamp (seconds since epoch) of when this slot was last saved.
    pub timestamp: i64,
    /// File format version written by the library that created this file.
    pub version: u32,
}

// ---------------------------------------------------------------------------
// SaveManager
// ---------------------------------------------------------------------------

/// Manages a directory of encrypted save slot files.
///
/// Each slot is stored as an independent file `slot_NNNN.gmsv`.
/// Slots are numbered from `1` upward; there is no built-in upper limit.
///
/// # Example
///
/// ```rust
/// use gm_save_core::SaveManager;
/// # use tempfile::tempdir;
/// # let dir = tempdir().unwrap();
///
/// let mgr = SaveManager::new(dir.path(), "my-game-secret-v1");
///
/// // Save slot 1
/// mgr.save(1, r#"{"hp":100,"level":3}"#).unwrap();
///
/// // Load slot 1 back
/// let json = mgr.load(1).unwrap();
/// assert_eq!(json, r#"{"hp":100,"level":3}"#);
///
/// // Metadata without decrypting
/// let meta = mgr.slot_meta(1).unwrap();
/// assert_eq!(meta.slot, 1);
/// assert_eq!(meta.version, 1);
///
/// // List all slots
/// assert_eq!(mgr.list_slots().unwrap(), vec![1]);
///
/// // Delete
/// mgr.delete(1).unwrap();
/// assert!(!mgr.exists(1));
/// ```
pub struct SaveManager {
    save_dir: PathBuf,
    passphrase: String,
}

impl SaveManager {
    /// Creates a `SaveManager`.
    ///
    /// - `save_dir`   — Directory where slot files live. Created on first save.
    /// - `passphrase` — Secret string used to derive the encryption key.
    ///                  **Must remain identical** across game updates so that
    ///                  existing save files stay decryptable.
    pub fn new(save_dir: impl Into<PathBuf>, passphrase: impl Into<String>) -> Self {
        Self {
            save_dir: save_dir.into(),
            passphrase: passphrase.into(),
        }
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    fn slot_path(&self, slot: u32) -> PathBuf {
        self.save_dir.join(format!("slot_{slot:04}.gmsv"))
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Encrypts `json` and writes it to slot `slot`.
    ///
    /// - Creates the save directory if it does not exist.
    /// - Uses an atomic write (tmp → rename) to prevent partial files.
    /// - Generates a fresh (salt, nonce) pair; each save is cryptographically
    ///   independent of the previous one.
    pub fn save(&self, slot: u32, json: &str) -> Result<()> {
        std::fs::create_dir_all(&self.save_dir)?;

        // Fresh randomness for this save.
        let mut salt = [0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);

        let mut key = derive_key(&self.passphrase, &salt);
        let result = encrypt(&key, json.as_bytes());
        key.zeroize();
        let (nonce, payload) = result?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Assemble the binary file in one allocation.
        let mut buf = Vec::with_capacity(HEADER_SIZE + payload.len());
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&salt);
        buf.extend_from_slice(&nonce);
        buf.extend_from_slice(&timestamp.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&payload);

        // Atomic write: tmp → rename.
        // If rename fails (e.g. another process holds a lock on the target
        // file on Windows), clean up the orphaned .tmp so it doesn't
        // accumulate on disk.
        let path = self.slot_path(slot);
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &buf)?;

        if let Err(e) = std::fs::rename(&tmp_path, &path) {
            // Best-effort cleanup — ignore remove errors.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }

        Ok(())
    }

    /// Decrypts slot `slot` and returns the original JSON string.
    ///
    /// # Errors
    /// - [`SaveError::SlotNotFound`] — no file for this slot.
    /// - [`SaveError::InvalidFormat`] — file is corrupt / not a `.gmsv` file.
    /// - [`SaveError::VersionMismatch`] — file was written with an incompatible version.
    /// - [`SaveError::Crypto`] — wrong passphrase, or the file has been tampered with.
    pub fn load(&self, slot: u32) -> Result<String> {
        let path = self.slot_path(slot);

        // Read the entire file in one syscall.  We skip a separate
        // `path.exists()` check to eliminate a TOCTOU race — the
        // `NotFound` error from `read` is handled directly.
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SaveError::SlotNotFound(slot));
            }
            Err(e) => return Err(e.into()),
        };

        let hdr = parse_header(&data)?;

        if hdr.version != FORMAT_VERSION {
            return Err(SaveError::VersionMismatch {
                expected: FORMAT_VERSION,
                found: hdr.version,
            });
        }

        if data.len() < HEADER_SIZE + hdr.payload_len {
            return Err(SaveError::InvalidFormat("payload is truncated".into()));
        }
        let ciphertext = &data[HEADER_SIZE..HEADER_SIZE + hdr.payload_len];

        let mut key = derive_key(&self.passphrase, &hdr.salt);
        let result = decrypt(&key, &hdr.nonce, ciphertext);
        key.zeroize();
        let plaintext = result?;

        String::from_utf8(plaintext)
            .map_err(|_| SaveError::InvalidFormat("payload is not valid UTF-8".into()))
    }

    /// Returns `true` if slot `slot` has a save file on disk.
    pub fn exists(&self, slot: u32) -> bool {
        self.slot_path(slot).exists()
    }

    /// Deletes the save file for slot `slot`.
    ///
    /// Succeeds silently if the slot does not exist.
    pub fn delete(&self, slot: u32) -> Result<()> {
        let path = self.slot_path(slot);
        // Skip the separate `exists()` check — removing directly and
        // treating `NotFound` as success avoids a TOCTOU race.
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Returns a **sorted** list of all existing slot numbers.
    pub fn list_slots(&self) -> Result<Vec<u32>> {
        if !self.save_dir.exists() {
            return Ok(vec![]);
        }

        let mut slots = Vec::new();

        for entry in std::fs::read_dir(&self.save_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if let Some(num_str) = name
                .strip_prefix("slot_")
                .and_then(|s| s.strip_suffix(".gmsv"))
            {
                if let Ok(n) = num_str.parse::<u32>() {
                    slots.push(n);
                }
            }
        }

        slots.sort_unstable();
        Ok(slots)
    }

    /// Reads the metadata of slot `slot` **without** decrypting the payload.
    ///
    /// Only the 48-byte file header is read from disk — the (potentially
    /// large) encrypted payload is never touched.  Much faster than
    /// [`load`](Self::load) and suitable for save-selection screens where
    /// you only need timestamps.
    pub fn slot_meta(&self, slot: u32) -> Result<SlotMeta> {
        let path = self.slot_path(slot);

        // Open the file and read only the fixed-size header (48 bytes)
        // instead of loading the entire file into memory.  On a
        // save-selection screen with many slots this avoids megabytes of
        // unnecessary allocations.
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SaveError::SlotNotFound(slot));
            }
            Err(e) => return Err(e.into()),
        };

        let mut header_buf = [0u8; HEADER_SIZE];
        file.read_exact(&mut header_buf)
            .map_err(|_| SaveError::InvalidFormat("file is too short".into()))?;

        let hdr = parse_header(&header_buf)?;

        Ok(SlotMeta {
            slot,
            timestamp: hdr.timestamp,
            version: hdr.version,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mgr(dir: &std::path::Path) -> SaveManager {
        SaveManager::new(dir, "test-passphrase-XYZ")
    }

    // --- Basic roundtrip ---

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        let json = r#"{"hp":100,"level":3,"x":512.0}"#;
        m.save(1, json).unwrap();
        assert_eq!(m.load(1).unwrap(), json);
    }

    #[test]
    fn multiple_slots_are_independent() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(1, r#"{"slot":1}"#).unwrap();
        m.save(2, r#"{"slot":2}"#).unwrap();
        m.save(3, r#"{"slot":3}"#).unwrap();
        assert_eq!(m.load(1).unwrap(), r#"{"slot":1}"#);
        assert_eq!(m.load(2).unwrap(), r#"{"slot":2}"#);
        assert_eq!(m.load(3).unwrap(), r#"{"slot":3}"#);
    }

    #[test]
    fn overwriting_slot_stores_new_data() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(1, r#"{"v":1}"#).unwrap();
        m.save(1, r#"{"v":2}"#).unwrap();
        assert_eq!(m.load(1).unwrap(), r#"{"v":2}"#);
    }

    // --- exists / delete ---

    #[test]
    fn exists_returns_false_before_first_save() {
        let dir = tempdir().unwrap();
        assert!(!mgr(dir.path()).exists(1));
    }

    #[test]
    fn exists_returns_true_after_save() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(1, "{}").unwrap();
        assert!(m.exists(1));
    }

    #[test]
    fn delete_removes_slot_file() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(1, "{}").unwrap();
        m.delete(1).unwrap();
        assert!(!m.exists(1));
    }

    #[test]
    fn delete_nonexistent_slot_is_ok() {
        let dir = tempdir().unwrap();
        assert!(mgr(dir.path()).delete(99).is_ok());
    }

    // --- list_slots ---

    #[test]
    fn list_slots_empty_directory() {
        let dir = tempdir().unwrap();
        assert_eq!(mgr(dir.path()).list_slots().unwrap(), vec![]);
    }

    #[test]
    fn list_slots_returns_sorted_numbers() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(3, "{}").unwrap();
        m.save(1, "{}").unwrap();
        m.save(2, "{}").unwrap();
        assert_eq!(m.list_slots().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn list_slots_after_delete_excludes_deleted() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(1, "{}").unwrap();
        m.save(2, "{}").unwrap();
        m.delete(1).unwrap();
        assert_eq!(m.list_slots().unwrap(), vec![2]);
    }

    // --- Error paths ---

    #[test]
    fn load_nonexistent_slot_is_slot_not_found() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            mgr(dir.path()).load(1),
            Err(SaveError::SlotNotFound(1))
        ));
    }

    #[test]
    fn load_with_wrong_passphrase_fails() {
        let dir = tempdir().unwrap();
        let good = SaveManager::new(dir.path(), "correct-pass");
        let bad = SaveManager::new(dir.path(), "wrong-pass");
        good.save(1, r#"{"secret":42}"#).unwrap();
        assert!(bad.load(1).is_err());
    }

    #[test]
    fn load_tampered_file_fails() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(1, r#"{"data":"hello"}"#).unwrap();

        let path = dir.path().join("slot_0001.gmsv");
        let mut data = std::fs::read(&path).unwrap();
        let last = data.len() - 1;
        data[last] ^= 0xFF;
        std::fs::write(&path, &data).unwrap();

        assert!(m.load(1).is_err());
    }

    #[test]
    fn load_truncated_file_fails() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        m.save(1, "{}").unwrap();

        let path = dir.path().join("slot_0001.gmsv");
        let data = std::fs::read(&path).unwrap();
        std::fs::write(&path, &data[..HEADER_SIZE / 2]).unwrap();

        assert!(m.load(1).is_err());
    }

    // --- Metadata ---

    #[test]
    fn slot_meta_timestamp_is_recent() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        let before = now_unix();
        m.save(1, "{}").unwrap();
        let after = now_unix();
        let meta = m.slot_meta(1).unwrap();
        assert!(meta.timestamp >= before && meta.timestamp <= after);
        assert_eq!(meta.version, FORMAT_VERSION);
        assert_eq!(meta.slot, 1);
    }

    #[test]
    fn slot_meta_nonexistent_is_slot_not_found() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            mgr(dir.path()).slot_meta(5),
            Err(SaveError::SlotNotFound(5))
        ));
    }

    // --- Large payload ---

    #[test]
    fn large_json_payload_roundtrip() {
        let dir = tempdir().unwrap();
        let m = mgr(dir.path());
        let json = format!(r#"{{"data":"{}"}}"#, "x".repeat(50_000));
        m.save(1, &json).unwrap();
        assert_eq!(m.load(1).unwrap(), json);
    }

    // --- Helpers ---

    fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }
}
