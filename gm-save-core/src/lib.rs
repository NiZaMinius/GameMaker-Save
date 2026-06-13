//! # gm-save-core
//!
//! Core logic for the `gm-save` GameMaker Studio 2 save extension.
//!
//! Provides **encrypted, slot-based save file management** using:
//! - [ChaCha20-Poly1305] for authenticated encryption
//! - [PBKDF2-HMAC-SHA256] for key derivation from a passphrase
//! - Atomic writes (temp file → rename) to prevent corruption on power loss
//!
//! This crate contains **no FFI code**. It is a pure Rust library suitable
//! for any Rust project. The companion crate `gm-save-ffi` wraps it as a
//! native extension `.dll` / `.so` / `.dylib` callable from GameMaker.
//!
//! ## Quick start
//!
//! ```rust
//! use gm_save_core::SaveManager;
//! # use tempfile::tempdir;
//! # let dir = tempdir().unwrap();
//!
//! let mgr = SaveManager::new(dir.path(), "my-game-secret-v1");
//!
//! mgr.save(1, r#"{"hp":100,"level":3}"#).unwrap();
//! let json = mgr.load(1).unwrap();
//! assert_eq!(json, r#"{"hp":100,"level":3}"#);
//! ```
//!
//! [ChaCha20-Poly1305]: https://www.rfc-editor.org/rfc/rfc8439
//! [PBKDF2-HMAC-SHA256]: https://www.rfc-editor.org/rfc/rfc8018

pub mod crypto;
pub mod error;
pub mod save;

pub use error::{Result, SaveError};
pub use save::{SaveManager, SlotMeta};
