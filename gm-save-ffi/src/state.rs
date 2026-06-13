//! Global [`SaveManager`] state for the FFI layer.
//!
//! GameMaker calls DLL functions as independent C calls — there is no Rust
//! object the caller can hold onto. We keep a single `SaveManager` alive
//! for the lifetime of the game session inside a `Mutex<Option<…>>` wrapped
//! in a `OnceLock`.
//!
//! Layout:
//! ```text
//! OnceLock  →  Mutex  →  Option<SaveManager>
//!  (init once)  (thread-safe access)  (None = not initialized)
//! ```
//!
//! If the Mutex becomes poisoned due to a panic, the layer recovers the lock
//! automatically. This prevents a poisoned state from crashing GameMaker on
//! subsequent calls.

use gm_save_core::SaveManager;
use std::sync::{Mutex, OnceLock};

static MANAGER: OnceLock<Mutex<Option<SaveManager>>> = OnceLock::new();

/// Returns a reference to the global mutex (creates it once with `None`).
fn global() -> &'static Mutex<Option<SaveManager>> {
    MANAGER.get_or_init(|| Mutex::new(None))
}

/// Installs a new [`SaveManager`], replacing any previous one.
/// Called by `SaveSystem_Init`.
pub fn set(manager: SaveManager) {
    *global().lock().unwrap_or_else(|e| e.into_inner()) = Some(manager);
}

/// Calls `f` with an immutable reference to the active [`SaveManager`].
///
/// Returns `None` if [`set`] has not been called yet (i.e., before
/// `SaveSystem_Init` is called from GML).
pub fn with<F, T>(f: F) -> Option<T>
where
    F: FnOnce(&SaveManager) -> T,
{
    global()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(f)
}

/// Drops the active [`SaveManager`].
/// Called by `SaveSystem_Shutdown`.
pub fn clear() {
    *global().lock().unwrap_or_else(|e| e.into_inner()) = None;
}
