use thiserror::Error;

/// All errors that can occur in the save system.
#[derive(Debug, Error)]
pub enum SaveError {
    /// An I/O error occurred while reading or writing a file.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Decryption or authentication failed.
    /// This usually means a wrong passphrase or a tampered/corrupted file.
    #[error("Crypto error: {0}")]
    Crypto(String),

    /// The file header does not start with the expected magic bytes (`GMSV`).
    #[error("Invalid file format: {0}")]
    InvalidFormat(String),

    /// The file was written with a different format version than this library expects.
    #[error("Version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },

    /// The requested slot number has no save file on disk.
    #[error("Slot {0} does not exist")]
    SlotNotFound(u32),
}

/// Convenience `Result` alias for this crate.
pub type Result<T> = std::result::Result<T, SaveError>;
