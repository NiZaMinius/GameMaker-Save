/// @file     gm_save.gml
/// @desc     Helper wrappers for the gm-save native extension.
///
///           DROP THIS FILE into your project's Scripts folder.
///           The compiled DLL must be set up as an Extension in the GMS2 IDE
///           (see README.md for step-by-step instructions).
///
/// @version  0.1.0
/// @author   NiZaMinius
/// @license  MIT OR Apache-2.0

// ============================================================
// INTERNAL HELPERS
// ============================================================

/// @func   __gmsave_unix_now()
/// @desc   Returns the current system time as a Unix timestamp (seconds
///         since Jan 1, 1970 UTC).
///
///         GML dates are counted in days since Dec 30, 1899.
///         Unix epoch starts Jan 1, 1970 — which is 25569 days later.
///         Multiplying the day-delta by 86400 converts days to seconds.
///
///         IMPORTANT: date_current_datetime() returns LOCAL time by
///         default. The Rust side stores timestamps in UTC
///         (SystemTime::now()), so we must temporarily force the
///         GML timezone to UTC here, or every player outside UTC+0
///         would see a wrong "age" for their saves (off by their
///         UTC offset). The original timezone is restored immediately
///         after the read so it never leaks into the rest of the game.
///
///         NOTE: current_time is NOT used here because it counts
///         milliseconds since game launch, not since the system epoch.
///         That would make slot age wrong after a game restart.
/// @return {Real}  Unix timestamp in whole seconds (UTC).
function __gmsave_unix_now() {
    var _previous_tz = date_get_timezone();
    date_set_timezone(timezone_utc);
    var _now = floor((date_current_datetime() - 25569) * 86400);
    date_set_timezone(_previous_tz);
    return _now;
}

// ============================================================
// INITIALIZATION
// ============================================================

/// @func   gmsave_init(passphrase)
/// @desc   Initializes the save system. Call ONCE — e.g. in the Game Start
///         event of a persistent controller object.
///         Uses `game_save_id + "saves/"` as the save directory.
///
///         IMPORTANT: game_save_id (not working_directory) is used
///         deliberately. working_directory points at the install
///         folder of the game — if installed under a protected path
///         like "C:\Program Files\", writes can silently fail or get
///         redirected by Windows UAC virtualization without the
///         player noticing. game_save_id always points to a writable,
///         per-game sandboxed folder under %localappdata% on Windows
///         (and the equivalent safe location on other platforms).
///
/// @param  {String} passphrase
///         Secret string used to derive the encryption key.
///
///         IMPORTANT: This value must NEVER change across game updates,
///         otherwise all existing save files become unreadable.
///         Do NOT use `game_display_name` or any other value that can
///         change. Use a hardcoded string constant like:
///             gmsave_init("MY_GAME_2024_SECRET_V1");
///
/// @return {Bool}  true if initialized successfully.
function gmsave_init(passphrase) {
    var _dir = game_save_id + "saves/";
    return SaveSystem_Init(_dir, string(passphrase)) == 1;
}

/// @func   gmsave_init_dir(save_dir, passphrase)
/// @desc   Like gmsave_init but with a custom save directory.
///         Use this when you need saves in a specific location
///         (e.g. a shared cloud sync folder).
///
/// @param  {String} save_dir    Absolute path to the save directory.
/// @param  {String} passphrase  See gmsave_init for passphrase rules.
/// @return {Bool}   true if initialized successfully.
function gmsave_init_dir(save_dir, passphrase) {
    return SaveSystem_Init(string(save_dir), string(passphrase)) == 1;
}

/// @func   gmsave_shutdown()
/// @desc   Releases the internal save manager.
///         Call in the Game End event of your persistent controller.
function gmsave_shutdown() {
    SaveSystem_Shutdown();
}

// ============================================================
// SAVE / LOAD
// ============================================================

/// @func   gmsave_save(slot, data)
/// @desc   Serializes `data` to JSON, encrypts it, and writes it to `slot`.
///
/// @param  {Real}        slot  Slot number (1, 2, 3, …). No upper limit.
/// @param  {Struct|Any}  data  Any value accepted by json_stringify().
/// @return {Bool}        true on success. On failure, call gmsave_last_error().
function gmsave_save(slot, data) {
    var _json = json_stringify(data);
    var _ok   = SaveSystem_Save(real(slot), _json) == 1;
    if (!_ok) {
        show_debug_message(
            "[gm-save] Save failed (slot " + string(slot) + "): "
            + SaveSystem_LastError()
        );
    }
    return _ok;
}

/// @func   gmsave_load(slot)
/// @desc   Decrypts slot `slot` and returns the parsed data as a struct.
///
///         json_parse() is wrapped in try/catch: the Rust side already
///         authenticates ciphertext (Poly1305), so corrupted decrypts
///         are rare — but a manually-edited file, a future format bug,
///         or a cross-process race could still hand back malformed
///         JSON. Without this guard a single bad save would crash the
///         whole game instead of just failing to load.
///
/// @param  {Real}    slot  Slot number.
/// @return {Struct}  Parsed save data, or `undefined` on failure.
function gmsave_load(slot) {
    var _raw = SaveSystem_Load(real(slot));
    if (_raw == "") {
        show_debug_message(
            "[gm-save] Load failed (slot " + string(slot) + "): "
            + SaveSystem_LastError()
        );
        return undefined;
    }
    try {
        return json_parse(_raw);
    } catch (_ex) {
        show_debug_message(
            "[gm-save] JSON parse error (slot " + string(slot) + "): "
            + string(_ex.message)
        );
        return undefined;
    }
}

/// @func   gmsave_load_raw(slot)
/// @desc   Like gmsave_load but returns the raw JSON string.
///         Useful when you need to pass the string somewhere else
///         instead of parsing it immediately.
///
/// @param  {Real}    slot  Slot number.
/// @return {String}  Raw JSON string, or "" on failure.
function gmsave_load_raw(slot) {
    return SaveSystem_Load(real(slot));
}

// ============================================================
// SLOT MANAGEMENT
// ============================================================

/// @func   gmsave_exists(slot)
/// @desc   Returns whether a save file exists for `slot`.
/// @param  {Real}  slot  Slot number.
/// @return {Bool}
function gmsave_exists(slot) {
    return SaveSystem_Exists(real(slot)) == 1;
}

/// @func   gmsave_delete(slot)
/// @desc   Permanently deletes the save file for `slot`.
///         Safe to call on a slot that does not exist.
/// @param  {Real}  slot  Slot number.
/// @return {Bool}  true on success.
function gmsave_delete(slot) {
    return SaveSystem_Delete(real(slot)) == 1;
}

/// @func   gmsave_list_slots()
/// @desc   Returns an array of all existing slot numbers, sorted ascending.
/// @return {Array<Real>}  e.g. [1, 2, 3], or [] if no saves exist.
function gmsave_list_slots() {
    var _raw = SaveSystem_ListSlots();
    if (_raw == "") {
        return [];
    }
    var _parts  = string_split(_raw, ",");
    var _count  = array_length(_parts);
    var _result = array_create(_count);
    for (var i = 0; i < _count; i++) {
        _result[i] = real(_parts[i]);
    }
    return _result;
}

/// @func   gmsave_slot_timestamp(slot)
/// @desc   Returns the Unix timestamp (seconds since Jan 1, 1970) of the
///         last save for `slot`, WITHOUT decrypting the payload.
///         Ideal for save-selection screens.
///
/// @param  {Real}  slot  Slot number.
/// @return {Real}  Unix timestamp in seconds, or -1 if slot not found.
function gmsave_slot_timestamp(slot) {
    return SaveSystem_SlotTimestamp(real(slot));
}

/// @func   gmsave_slot_age_seconds(slot)
/// @desc   Returns how many seconds have passed since `slot` was last saved.
///
///         Uses the system clock (not game uptime), so the result stays
///         accurate after a game restart.
///
/// @param  {Real}  slot  Slot number.
/// @return {Real}  Age in whole seconds, or -1 if slot not found.
function gmsave_slot_age_seconds(slot) {
    var _ts = gmsave_slot_timestamp(slot);
    if (_ts < 0) return -1;

    // __gmsave_unix_now() returns current Unix time in seconds.
    // The slot timestamp from Rust is also Unix seconds — so the
    // subtraction is always correct, even across game restarts.
    return __gmsave_unix_now() - _ts;
}

/// @func   gmsave_slot_age_string(slot)
/// @desc   Returns a human-readable age string for a slot.
///         e.g. "2 minutes ago", "3 hours ago", "1 day ago".
///         Useful for save-selection UIs.
///
/// @param  {Real}    slot  Slot number.
/// @return {String}  Human-readable age, or "Unknown" if slot not found.
function gmsave_slot_age_string(slot) {
    var _secs = gmsave_slot_age_seconds(slot);
    if (_secs < 0)    return "Unknown";
    if (_secs < 60)   return string(floor(_secs)) + " sec ago";
    if (_secs < 3600) return string(floor(_secs / 60)) + " min ago";
    if (_secs < 86400) return string(floor(_secs / 3600)) + " hr ago";
    return string(floor(_secs / 86400)) + " day(s) ago";
}

// ============================================================
// ERROR HANDLING
// ============================================================

/// @func   gmsave_last_error()
/// @desc   Returns the error message from the last failed operation.
///         Returns "" if the last operation succeeded.
/// @return {String}
function gmsave_last_error() {
    return SaveSystem_LastError();
}

// ============================================================
// EXAMPLE USAGE
// ============================================================
//
// // ── Persistent controller object, Game Start event ──────────
// gmsave_init("MY_GAME_2024_SECRET_V1");  // never change this string
//
// // ── Saving ──────────────────────────────────────────────────
// var _data = {
//     hp:    global.player_hp,
//     level: global.current_level,
//     x:     obj_player.x,
//     y:     obj_player.y,
// };
// if (!gmsave_save(1, _data)) {
//     show_message("Save failed: " + gmsave_last_error());
// }
//
// // ── Loading ─────────────────────────────────────────────────
// var _save = gmsave_load(1);
// if (_save != undefined) {
//     global.player_hp     = _save.hp;
//     global.current_level = _save.level;
//     obj_player.x         = _save.x;
//     obj_player.y         = _save.y;
// }
//
// // ── Save-selection UI ────────────────────────────────────────
// var _slots = gmsave_list_slots();
// for (var i = 0; i < array_length(_slots); i++) {
//     var _slot = _slots[i];
//     draw_text(32, 64 + i * 48,
//         "Slot " + string(_slot)
//         + "  —  " + gmsave_slot_age_string(_slot));
// }
//
// // ── Game End event ───────────────────────────────────────────
// gmsave_shutdown();
