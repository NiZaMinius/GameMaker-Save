//! # gm-save-ffi
//!
//! GameMaker-compatible FFI layer for `gm-save-core`.
//!
//! ## Calling convention
//!
//! All exported functions follow GameMaker Studio 2's native extension rules:
//! - **Parameters**: either `f64` (Real) or `*const c_char` (String)
//! - **Return**:     either `f64` (Real) or `*const c_char` (String)
//! - **ABI**: `extern "C"` (cdecl)
//!
//! ## Exported functions
//!
//! | Function                         | Returns | Description                                     |
//! |----------------------------------|---------|--------------------------------------------------|
//! | `SaveSystem_Init(dir, pass)`     | `f64`   | Initialize. `1.0` = ok, `0.0` = bad args.       |
//! | `SaveSystem_Save(slot, json)`    | `f64`   | Encrypt + write. `1.0` = ok, `0.0` = error.     |
//! | `SaveSystem_Load(slot)`          | `str`   | Decrypt + read. Empty string on error.           |
//! | `SaveSystem_Exists(slot)`        | `f64`   | `1.0` if slot file exists, `0.0` otherwise.      |
//! | `SaveSystem_Delete(slot)`        | `f64`   | Delete slot. `1.0` = ok, `0.0` = error.          |
//! | `SaveSystem_ListSlots()`         | `str`   | Comma-separated slot numbers, e.g. `"1,2,3"`.   |
//! | `SaveSystem_SlotTimestamp(slot)` | `f64`   | Unix timestamp of last save. `-1.0` on error.    |
//! | `SaveSystem_LastError()`         | `str`   | Human-readable description of the last error.    |
//! | `SaveSystem_Shutdown()`          | `f64`   | Release resources. Always `1.0`.                 |

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};

use gm_save_core::SaveManager;

mod state;

// ---------------------------------------------------------------------------
// Thread-local string buffers
//
// GameMaker copies any string returned by an extension function immediately
// after the call returns, so a single per-thread buffer is sufficient.
// We keep one buffer for regular return values and one for the last error.
// ---------------------------------------------------------------------------

thread_local! {
    static STRING_BUF: RefCell<CString> = RefCell::new(CString::new("").unwrap());
    static ERROR_BUF:  RefCell<CString> = RefCell::new(CString::new("").unwrap());
}

/// Stores `s` in the thread-local string buffer and returns a raw pointer.
/// Safe to use as a return value because GameMaker copies it before the
/// next call can overwrite the buffer.
fn return_str(s: &str) -> *const c_char {
    STRING_BUF.with(|buf| {
        // Fast path: most strings have no interior null bytes.
        let cs = CString::new(s).unwrap_or_else(|_| {
            // Slow path: strip null bytes only when actually needed.
            let clean: Vec<u8> = s.bytes().filter(|&b| b != 0).collect();
            CString::new(clean).unwrap_or_else(|_| CString::new("").unwrap())
        });
        // `as_ptr` points into the heap allocation of `cs`.
        // Moving `cs` into the RefCell does NOT move that heap allocation.
        let ptr = cs.as_ptr();
        *buf.borrow_mut() = cs;
        ptr
    })
}

/// Records a human-readable error message for `SaveSystem_LastError()`.
fn set_error(msg: &str) {
    ERROR_BUF.with(|buf| {
        let cs = CString::new(msg).unwrap_or_else(|_| {
            let clean: Vec<u8> = msg.bytes().filter(|&b| b != 0).collect();
            CString::new(clean).unwrap_or_else(|_| CString::new("unknown error").unwrap())
        });
        *buf.borrow_mut() = cs;
    });
}

/// Clears the last error (called at the start of every successful operation).
fn clear_error() {
    ERROR_BUF.with(|buf| {
        let already_empty = buf.borrow().as_bytes().is_empty();
        if !already_empty {
            *buf.borrow_mut() = CString::new("").unwrap();
        }
    });
}

// ---------------------------------------------------------------------------
// GML type helpers
// ---------------------------------------------------------------------------

/// Converts a C string pointer to a `&str`.
/// Returns `None` for null pointers or invalid UTF-8.
///
/// # Safety
/// `ptr` must be a valid null-terminated C string for the duration of this call.
#[inline]
unsafe fn c_str_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok()
}

/// Converts a Rust `bool` to the GML boolean convention (`1.0` / `0.0`).
#[inline(always)]
fn gm_bool(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Exported functions
// ---------------------------------------------------------------------------

/// Initializes the save system for this game session.
///
/// # Parameters (GML)
/// - `save_dir`   — Directory where slot files are stored.
///                  Recommended: `working_directory + "saves/"`.
/// - `passphrase` — Secret string for key derivation.
///                  **Never change this across game versions** or existing saves
///                  will become unreadable.
///
/// # Returns
/// `1.0` on success, `0.0` if either argument is null or invalid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_Init(
    save_dir: *const c_char,
    passphrase: *const c_char,
) -> f64 {
    let Some(dir) = c_str_to_str(save_dir) else {
        set_error("save_dir is null or invalid UTF-8");
        return 0.0;
    };
    let Some(pass) = c_str_to_str(passphrase) else {
        set_error("passphrase is null or invalid UTF-8");
        return 0.0;
    };

    state::set(SaveManager::new(dir, pass));
    clear_error();
    1.0
}

/// Encrypts `json_str` and writes it to slot `slot`.
///
/// # Parameters (GML)
/// - `slot`     — Slot number (Real, treated as u32). Use `1`, `2`, `3`, …
/// - `json_str` — JSON string to save (the output of `json_stringify`).
///
/// # Returns
/// `1.0` on success, `0.0` on any error (check `SaveSystem_LastError()`).
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_Save(slot: f64, json_str: *const c_char) -> f64 {
    let Some(json) = c_str_to_str(json_str) else {
        set_error("json_str is null or invalid UTF-8");
        return 0.0;
    };

    match state::with(|mgr| mgr.save(slot as u32, json)) {
        Some(Ok(())) => {
            clear_error();
            1.0
        }
        Some(Err(e)) => {
            set_error(&e.to_string());
            0.0
        }
        None => {
            set_error("SaveSystem not initialized — call SaveSystem_Init first");
            0.0
        }
    }
}

/// Decrypts and returns the JSON stored in slot `slot`.
///
/// # Parameters (GML)
/// - `slot` — Slot number (Real, treated as u32).
///
/// # Returns
/// The JSON string on success, or `""` on error.
/// Always check with `SaveSystem_Exists` or test for `""` before parsing.
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_Load(slot: f64) -> *const c_char {
    match state::with(|mgr| mgr.load(slot as u32)) {
        Some(Ok(json)) => {
            clear_error();
            return_str(&json)
        }
        Some(Err(e)) => {
            set_error(&e.to_string());
            return_str("")
        }
        None => {
            set_error("SaveSystem not initialized");
            return_str("")
        }
    }
}

/// Returns whether slot `slot` has a save file on disk.
///
/// # Returns
/// `1.0` if the slot file exists, `0.0` otherwise (including if not initialized).
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_Exists(slot: f64) -> f64 {
    gm_bool(state::with(|mgr| mgr.exists(slot as u32)).unwrap_or(false))
}

/// Deletes the save file for slot `slot`.
///
/// Succeeds silently if the slot did not exist.
///
/// # Returns
/// `1.0` on success, `0.0` on error.
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_Delete(slot: f64) -> f64 {
    match state::with(|mgr| mgr.delete(slot as u32)) {
        Some(Ok(())) => {
            clear_error();
            1.0
        }
        Some(Err(e)) => {
            set_error(&e.to_string());
            0.0
        }
        None => {
            set_error("SaveSystem not initialized");
            0.0
        }
    }
}

/// Returns a comma-separated list of all existing slot numbers.
///
/// # Returns
/// A string such as `"1,2,3"`, or `""` if there are no saves or the system
/// is not initialized.
///
/// # GML usage
/// ```gml
/// var raw   = SaveSystem_ListSlots();
/// var parts = string_split(raw, ",");
/// for (var i = 0; i < array_length(parts); i++) {
///     var slot = real(parts[i]);
///     // …
/// }
/// ```
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_ListSlots() -> *const c_char {
    let result = state::with(|mgr| {
        mgr.list_slots().map(|v| {
            v.iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        })
    });

    match result {
        Some(Ok(list)) => {
            clear_error();
            return_str(&list)
        }
        Some(Err(e)) => {
            set_error(&e.to_string());
            return_str("")
        }
        None => {
            set_error("SaveSystem not initialized");
            return_str("")
        }
    }
}

/// Returns the Unix timestamp (seconds since epoch) of slot `slot`'s last save.
///
/// This reads only the file header — it does **not** decrypt the payload.
/// Useful for displaying save-slot metadata without the cost of a full load.
///
/// # Returns
/// Timestamp as `f64`, or `-1.0` if the slot does not exist or is not initialized.
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_SlotTimestamp(slot: f64) -> f64 {
    let result = state::with(|mgr| mgr.slot_meta(slot as u32).map(|m| m.timestamp as f64));

    match result {
        Some(Ok(ts)) => {
            clear_error();
            ts
        }
        Some(Err(e)) => {
            set_error(&e.to_string());
            -1.0
        }
        None => {
            set_error("SaveSystem not initialized");
            -1.0
        }
    }
}

/// Returns a human-readable description of the last error that occurred.
///
/// Returns `""` if the last operation succeeded.
///
/// # GML usage
/// ```gml
/// if (!SaveSystem_Save(1, json_stringify(data))) {
///     show_message("Save failed: " + SaveSystem_LastError());
/// }
/// ```
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_LastError() -> *const c_char {
    ERROR_BUF.with(|buf| buf.borrow().as_ptr())
}

/// Releases the internal [`SaveManager`] and frees its resources.
///
/// Call this in your game's `Game End` event.
///
/// # Returns
/// Always `1.0`.
#[no_mangle]
pub unsafe extern "C" fn SaveSystem_Shutdown() -> f64 {
    state::clear();
    clear_error();
    1.0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// All tests that touch global state are marked `#[serial]` via the
// `serial_test` crate. This prevents data races when `cargo test` runs
// tests on multiple threads simultaneously.

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::ffi::{CStr, CString};
    use tempfile::tempdir;

    // --- Helpers ---

    fn cs(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    unsafe fn last_error() -> String {
        CStr::from_ptr(SaveSystem_LastError())
            .to_string_lossy()
            .into_owned()
    }

    unsafe fn str_result(ptr: *const c_char) -> String {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }

    unsafe fn init_in(dir: &std::path::Path) {
        let d = cs(dir.to_str().unwrap());
        let p = cs("test-passphrase");
        assert_eq!(SaveSystem_Init(d.as_ptr(), p.as_ptr()), 1.0);
    }

    // --- Init ---

    #[test]
    #[serial]
    fn init_with_valid_args_returns_1() {
        let dir = tempdir().unwrap();
        unsafe { init_in(dir.path()) };
        unsafe { SaveSystem_Shutdown() };
    }

    #[test]
    #[serial]
    fn init_with_null_dir_returns_0() {
        let p = cs("pass");
        let result = unsafe { SaveSystem_Init(std::ptr::null(), p.as_ptr()) };
        assert_eq!(result, 0.0);
        assert!(!unsafe { last_error() }.is_empty());
        unsafe { SaveSystem_Shutdown() };
    }

    #[test]
    #[serial]
    fn init_with_null_passphrase_returns_0() {
        let d = cs("/tmp");
        let result = unsafe { SaveSystem_Init(d.as_ptr(), std::ptr::null()) };
        assert_eq!(result, 0.0);
        unsafe { SaveSystem_Shutdown() };
    }

    // --- Save / Load ---

    #[test]
    #[serial]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let json = r#"{"hp":100,"x":512}"#;
        let slot = 1.0_f64;
        let j = cs(json);

        unsafe {
            init_in(dir.path());
            assert_eq!(SaveSystem_Save(slot, j.as_ptr()), 1.0);
            let loaded = str_result(SaveSystem_Load(slot));
            assert_eq!(loaded, json);
            SaveSystem_Shutdown();
        }
    }

    #[test]
    #[serial]
    fn load_before_init_returns_empty_string() {
        unsafe {
            SaveSystem_Shutdown(); // ensure cleared
            let result = str_result(SaveSystem_Load(1.0));
            assert_eq!(result, "");
            assert!(!last_error().is_empty());
        }
    }

    #[test]
    #[serial]
    fn load_nonexistent_slot_returns_empty_string() {
        let dir = tempdir().unwrap();
        unsafe {
            init_in(dir.path());
            let result = str_result(SaveSystem_Load(99.0));
            assert_eq!(result, "");
            assert!(!last_error().is_empty());
            SaveSystem_Shutdown();
        }
    }

    #[test]
    #[serial]
    fn save_with_null_json_returns_0() {
        let dir = tempdir().unwrap();
        unsafe {
            init_in(dir.path());
            let result = SaveSystem_Save(1.0, std::ptr::null());
            assert_eq!(result, 0.0);
            SaveSystem_Shutdown();
        }
    }

    // --- Exists ---

    #[test]
    #[serial]
    fn exists_returns_0_before_save() {
        let dir = tempdir().unwrap();
        unsafe {
            init_in(dir.path());
            assert_eq!(SaveSystem_Exists(1.0), 0.0);
            SaveSystem_Shutdown();
        }
    }

    #[test]
    #[serial]
    fn exists_returns_1_after_save() {
        let dir = tempdir().unwrap();
        let json = cs("{}");
        unsafe {
            init_in(dir.path());
            SaveSystem_Save(1.0, json.as_ptr());
            assert_eq!(SaveSystem_Exists(1.0), 1.0);
            SaveSystem_Shutdown();
        }
    }

    // --- Delete ---

    #[test]
    #[serial]
    fn delete_removes_slot() {
        let dir = tempdir().unwrap();
        let json = cs("{}");
        unsafe {
            init_in(dir.path());
            SaveSystem_Save(1.0, json.as_ptr());
            assert_eq!(SaveSystem_Exists(1.0), 1.0);
            assert_eq!(SaveSystem_Delete(1.0), 1.0);
            assert_eq!(SaveSystem_Exists(1.0), 0.0);
            SaveSystem_Shutdown();
        }
    }

    #[test]
    #[serial]
    fn delete_nonexistent_slot_returns_1() {
        let dir = tempdir().unwrap();
        unsafe {
            init_in(dir.path());
            assert_eq!(SaveSystem_Delete(99.0), 1.0);
            SaveSystem_Shutdown();
        }
    }

    // --- ListSlots ---

    #[test]
    #[serial]
    fn list_slots_returns_sorted_csv() {
        let dir = tempdir().unwrap();
        let json = cs("{}");
        unsafe {
            init_in(dir.path());
            SaveSystem_Save(3.0, json.as_ptr());
            SaveSystem_Save(1.0, json.as_ptr());
            SaveSystem_Save(2.0, json.as_ptr());
            let list = str_result(SaveSystem_ListSlots());
            assert_eq!(list, "1,2,3");
            SaveSystem_Shutdown();
        }
    }

    #[test]
    #[serial]
    fn list_slots_empty_returns_empty_string() {
        let dir = tempdir().unwrap();
        unsafe {
            init_in(dir.path());
            assert_eq!(str_result(SaveSystem_ListSlots()), "");
            SaveSystem_Shutdown();
        }
    }

    // --- Timestamp ---

    #[test]
    #[serial]
    fn slot_timestamp_is_positive_after_save() {
        let dir = tempdir().unwrap();
        let json = cs("{}");
        unsafe {
            init_in(dir.path());
            SaveSystem_Save(1.0, json.as_ptr());
            let ts = SaveSystem_SlotTimestamp(1.0);
            assert!(ts > 0.0, "timestamp should be a positive unix time");
            SaveSystem_Shutdown();
        }
    }

    #[test]
    #[serial]
    fn slot_timestamp_nonexistent_returns_minus_one() {
        let dir = tempdir().unwrap();
        unsafe {
            init_in(dir.path());
            assert_eq!(SaveSystem_SlotTimestamp(99.0), -1.0);
            SaveSystem_Shutdown();
        }
    }

    // --- Shutdown ---

    #[test]
    #[serial]
    fn shutdown_always_returns_1() {
        unsafe {
            assert_eq!(SaveSystem_Shutdown(), 1.0);
            // Calling shutdown again is safe.
            assert_eq!(SaveSystem_Shutdown(), 1.0);
        }
    }

    #[test]
    #[serial]
    fn operations_after_shutdown_return_error_values() {
        let dir = tempdir().unwrap();
        let json = cs("{}");
        unsafe {
            init_in(dir.path());
            SaveSystem_Save(1.0, json.as_ptr());
            SaveSystem_Shutdown();

            // Everything should gracefully return an error value.
            assert_eq!(SaveSystem_Exists(1.0), 0.0);
            assert_eq!(SaveSystem_Delete(1.0), 0.0);
            assert_eq!(SaveSystem_SlotTimestamp(1.0), -1.0);
            assert_eq!(str_result(SaveSystem_Load(1.0)), "");
            assert_eq!(str_result(SaveSystem_ListSlots()), "");
        }
    }

    // --- LastError ---

    #[test]
    #[serial]
    fn last_error_is_empty_after_success() {
        let dir = tempdir().unwrap();
        let json = cs(r#"{"ok":true}"#);
        unsafe {
            init_in(dir.path());
            SaveSystem_Save(1.0, json.as_ptr());
            assert_eq!(last_error(), "");
            SaveSystem_Shutdown();
        }
    }

    #[test]
    #[serial]
    fn last_error_is_set_after_failure() {
        let dir = tempdir().unwrap();
        unsafe {
            init_in(dir.path());
            SaveSystem_Load(999.0); // slot doesn't exist
            assert!(!last_error().is_empty());
            SaveSystem_Shutdown();
        }
    }
}
