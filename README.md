<div align="center">

# gm-save

**Encrypted slot-based save system for GameMaker Studio 2, written in Rust.**

[![Crates.io](https://img.shields.io/crates/v/gm-save-core.svg)](https://crates.io/crates/gm-save-core)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license--лицензия)
[![CI](https://github.com/NiZaMinius/gm-save/actions/workflows/release.yml/badge.svg)](https://github.com/NiZaMinius/gm-save/actions)
<br/>

🌐 **Language / Язык**

[**English**](#-english) · [**Русский**](#-русский)

</div>

---

<a name="-english"></a>

## 🇬🇧 English

- [Features](#features)
- [Architecture](#architecture)
- [GameMaker Setup](#gamemaker-setup)
- [GML API Reference](#gml-api-reference)
- [File Format](#file-format)
- [Building from Source](#building-from-source)
- [License](#license--лицензия)

---

### Features

- **ChaCha20-Poly1305** authenticated encryption — fast, secure, tamper-proof
- **PBKDF2-HMAC-SHA256** key derivation with 100 000 rounds — your passphrase never appears in the file
- **Fresh (salt, nonce) on every save** — identical data always produces different ciphertext
- **Atomic writes** — temp file → rename ensures a power loss mid-save never corrupts an existing slot
- **Sandboxed save location** — defaults to `game_save_id`, GameMaker's guaranteed-writable per-game folder (never `working_directory`, which can sit in a protected install path)
- **Unlimited slots** — 1, 2, 3, … any number, fully independent of each other
- **Metadata without decryption** — read timestamps cheaply for save-selection screens
- **`SaveSystem_LastError()`** — human-readable error strings passed back to GML
- **Cross-platform** — builds to `.dll` (Windows), `.so` (Linux), `.dylib` (macOS)

---

### Architecture

The project is split into two crates intentionally:

```
gm-save/
├── gm-save-core/   # Pure Rust — encryption, slots, file format
│                   # Published to crates.io
│
├── gm-save-ffi/    # C ABI wrapper → compiles to .dll / .so / .dylib
│                   # Distributed via GitHub Releases
│
└── gml/
    └── gm_save.gml # GML helper script for GameMaker developers
```

`gm-save-core` contains zero FFI code and can be used as a normal Rust library.  
`gm-save-ffi` wraps it with `extern "C"` functions and `#[no_mangle]` exports — the only layer GameMaker needs.

---

### GameMaker Setup

#### Step 1 — Download the release

Go to [Releases](https://github.com/NiZaMinius/gm-save/releases) and download:

- `gm_save.dll` — compiled native extension (Windows x64)
- `gm_save.gml` — GML helper script

#### Step 2 — Add the Extension in GMS2

1. Open your project → **Extensions** panel → right-click → **Create Extension**
2. Name it `gm_save`
3. Click **Add File** → select `gm_save.dll`
4. For each function below, click **Add Function** and fill in the details:

| Function name              | Return type | Parameters                        |
|----------------------------|-------------|-----------------------------------|
| `SaveSystem_Init`          | Real        | String `save_dir`, String `pass`  |
| `SaveSystem_Save`          | Real        | Real `slot`, String `json`        |
| `SaveSystem_Load`          | String      | Real `slot`                       |
| `SaveSystem_Exists`        | Real        | Real `slot`                       |
| `SaveSystem_Delete`        | Real        | Real `slot`                       |
| `SaveSystem_ListSlots`     | String      | *(none)*                          |
| `SaveSystem_SlotTimestamp` | Real        | Real `slot`                       |
| `SaveSystem_LastError`     | String      | *(none)*                          |
| `SaveSystem_Shutdown`      | Real        | *(none)*                          |

#### Step 3 — Import the GML script

Drag `gm_save.gml` into your project's **Scripts** folder.

#### Step 4 — Use it

```gml
// ── Persistent controller, Game Start event ──────────────────────────
// Pick a passphrase and NEVER change it — changing it makes all
// existing saves unreadable (the derived key would differ).
gmsave_init("MY_GAME_2024_SECRET_V1");

// ── Saving ───────────────────────────────────────────────────────────
var _data = {
    hp:    global.player_hp,
    level: global.current_level,
    x:     obj_player.x,
    y:     obj_player.y,
};
if (!gmsave_save(1, _data)) {
    show_message("Save failed: " + gmsave_last_error());
}

// ── Loading ──────────────────────────────────────────────────────────
var _save = gmsave_load(1);
if (_save != undefined) {
    global.player_hp     = _save.hp;
    global.current_level = _save.level;
}

// ── Save-selection UI ────────────────────────────────────────────────
var _slots = gmsave_list_slots(); // [1, 2, 3, ...]
for (var i = 0; i < array_length(_slots); i++) {
    draw_text(32, 64 + i * 48,
        "Slot " + string(_slots[i])
        + "  —  " + gmsave_slot_age_string(_slots[i]));
}

// ── Game End event ───────────────────────────────────────────────────
gmsave_shutdown();
```

---

### GML API Reference

| Function | Returns | Description |
|---|---|---|
| `gmsave_init(passphrase)` | Bool | Initialize with `game_save_id + "saves/"` |
| `gmsave_init_dir(dir, passphrase)` | Bool | Initialize with a custom directory |
| `gmsave_shutdown()` | — | Release resources (call in Game End event) |
| `gmsave_save(slot, data)` | Bool | Serialize → encrypt → write slot |
| `gmsave_load(slot)` | Struct | Decrypt → parse → return struct |
| `gmsave_load_raw(slot)` | String | Decrypt → return raw JSON string |
| `gmsave_exists(slot)` | Bool | Check if slot file exists |
| `gmsave_delete(slot)` | Bool | Delete slot file |
| `gmsave_list_slots()` | Array | Sorted array of existing slot numbers |
| `gmsave_slot_timestamp(slot)` | Real | Unix timestamp of last save (no decryption) |
| `gmsave_slot_age_seconds(slot)` | Real | Seconds since last save |
| `gmsave_slot_age_string(slot)` | String | Human-readable: "5 min ago", "2 hr ago" |
| `gmsave_last_error()` | String | Error message from last failed operation |

---

### File Format

Every `.gmsv` file uses this binary layout:

```
 4 bytes   magic         b"GMSV"
 4 bytes   version       u32, little-endian
16 bytes   salt          random, unique per save (PBKDF2 input)
12 bytes   nonce         random, unique per save (ChaCha20 input)
 8 bytes   timestamp     i64, little-endian, Unix seconds
 4 bytes   payload_len   u32, little-endian
 N bytes   payload       ChaCha20-Poly1305 ciphertext + 16-byte Poly1305 tag
```

A fresh `(salt, nonce)` pair is generated on every save — the same data encrypted twice produces two completely different files. The Poly1305 tag guarantees that any tampering is detected on load.

---

### Building from Source

**Requirements:** Rust 1.75+ (stable)

```bash
git clone https://github.com/NiZaMinius/gm-save
cd gm-save

# Run all 46 tests
cargo test

# Build release DLL on Windows
cargo build --release -p gm-save-ffi
# Output: target/release/gm_save.dll

# Cross-compile for Windows from Linux / macOS
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p gm-save-ffi
```

---

<a name="-русский"></a>

## 🇷🇺 Русский

- [Возможности](#возможности)
- [Архитектура](#архитектура)
- [Установка в GameMaker](#установка-в-gamemaker)
- [Справочник GML API](#справочник-gml-api)
- [Формат файла](#формат-файла)
- [Сборка из исходников](#сборка-из-исходников)
- [Лицензия](#license--лицензия)

---

### Возможности

- **ChaCha20-Poly1305** — аутентифицированное шифрование: быстрое, надёжное, защищённое от подделки
- **PBKDF2-HMAC-SHA256** с 100 000 итерациями — пароль никогда не попадает в файл в открытом виде
- **Новый (salt, nonce) при каждом сохранении** — одинаковые данные всегда дают разный шифртекст
- **Атомарная запись** — сначала во временный файл, потом rename: сбой питания не повредит существующий слот
- **Изолированная папка сохранений** — по умолчанию `game_save_id`, гарантированно доступная для записи папка GameMaker (никогда `working_directory`, которая может лежать в защищённом пути установки)
- **Неограниченное число слотов** — 1, 2, 3, … любое количество, полностью независимые
- **Метаданные без расшифровки** — быстрое чтение временных меток для экранов выбора сохранения
- **`SaveSystem_LastError()`** — человекочитаемые сообщения об ошибках прямо в GML
- **Кроссплатформенность** — собирается в `.dll` (Windows), `.so` (Linux), `.dylib` (macOS)

---

### Архитектура

Проект намеренно разделён на два крейта:

```
gm-save/
├── gm-save-core/   # Чистый Rust — шифрование, слоты, формат файла
│                   # Публикуется на crates.io
│
├── gm-save-ffi/    # Обёртка с C ABI → компилируется в .dll / .so / .dylib
│                   # Распространяется через GitHub Releases
│
└── gml/
    └── gm_save.gml # GML-скрипт для разработчиков GameMaker
```

`gm-save-core` не содержит FFI-кода — это обычная Rust-библиотека.  
`gm-save-ffi` оборачивает её функциями `extern "C"` с `#[no_mangle]` — именно этот слой компилируется в DLL.

---

### Установка в GameMaker

#### Шаг 1 — Скачай релиз

Перейди в [Releases](https://github.com/NiZaMinius/gm-save/releases) и скачай:

- `gm_save.dll` — нативное расширение (Windows x64)
- `gm_save.gml` — GML-скрипт с обёртками

#### Шаг 2 — Добавь Extension в GMS2

1. Открой проект → панель **Extensions** → правой кнопкой → **Create Extension**
2. Назови его `gm_save`
3. Нажми **Add File** → выбери `gm_save.dll`
4. Для каждой функции ниже нажми **Add Function**:

| Имя функции                | Тип возврата | Параметры                         |
|----------------------------|-------------|-----------------------------------|
| `SaveSystem_Init`          | Real        | String `save_dir`, String `pass`  |
| `SaveSystem_Save`          | Real        | Real `slot`, String `json`        |
| `SaveSystem_Load`          | String      | Real `slot`                       |
| `SaveSystem_Exists`        | Real        | Real `slot`                       |
| `SaveSystem_Delete`        | Real        | Real `slot`                       |
| `SaveSystem_ListSlots`     | String      | *(нет)*                           |
| `SaveSystem_SlotTimestamp` | Real        | Real `slot`                       |
| `SaveSystem_LastError`     | String      | *(нет)*                           |
| `SaveSystem_Shutdown`      | Real        | *(нет)*                           |

#### Шаг 3 — Импортируй GML-скрипт

Перетащи `gm_save.gml` в папку **Scripts** проекта.

#### Шаг 4 — Используй

```gml
// ── Постоянный контроллер, событие Game Start ────────────────────────
// Выбери пароль и НИКОГДА его не меняй — смена сделает все
// существующие сохранения нечитаемыми.
gmsave_init("MY_GAME_2024_SECRET_V1");

// ── Сохранение ───────────────────────────────────────────────────────
var _data = {
    hp:    global.player_hp,
    level: global.current_level,
    x:     obj_player.x,
    y:     obj_player.y,
};
if (!gmsave_save(1, _data)) {
    show_message("Ошибка: " + gmsave_last_error());
}

// ── Загрузка ─────────────────────────────────────────────────────────
var _save = gmsave_load(1);
if (_save != undefined) {
    global.player_hp     = _save.hp;
    global.current_level = _save.level;
}

// ── Экран выбора сохранения ──────────────────────────────────────────
var _slots = gmsave_list_slots();
for (var i = 0; i < array_length(_slots); i++) {
    draw_text(32, 64 + i * 48,
        "Слот " + string(_slots[i])
        + "  —  " + gmsave_slot_age_string(_slots[i]));
}

// ── Событие Game End ─────────────────────────────────────────────────
gmsave_shutdown();
```

---

### Справочник GML API

| Функция | Возврат | Описание |
|---|---|---|
| `gmsave_init(passphrase)` | Bool | Инициализация в `game_save_id + "saves/"` |
| `gmsave_init_dir(dir, passphrase)` | Bool | Инициализация с произвольной папкой |
| `gmsave_shutdown()` | — | Освободить ресурсы (Game End) |
| `gmsave_save(slot, data)` | Bool | Сериализовать → зашифровать → записать |
| `gmsave_load(slot)` | Struct | Расшифровать → распарсить → вернуть struct |
| `gmsave_load_raw(slot)` | String | Расшифровать → вернуть сырую JSON-строку |
| `gmsave_exists(slot)` | Bool | Проверить существование файла слота |
| `gmsave_delete(slot)` | Bool | Удалить файл слота |
| `gmsave_list_slots()` | Array | Отсортированный массив номеров слотов |
| `gmsave_slot_timestamp(slot)` | Real | Unix-метка последнего сохранения |
| `gmsave_slot_age_seconds(slot)` | Real | Секунд с последнего сохранения |
| `gmsave_slot_age_string(slot)` | String | "5 min ago", "2 hr ago", "1 day(s) ago" |
| `gmsave_last_error()` | String | Сообщение об ошибке последней операции |

---

### Формат файла

```
 4 байта   magic         b"GMSV"
 4 байта   version       u32, little-endian
16 байт    salt          случайный, уникален для каждого сохранения
12 байт    nonce         случайный, уникален для каждого сохранения
 8 байт    timestamp     i64, little-endian, Unix-секунды
 4 байта   payload_len   u32, little-endian
 N байт    payload       шифртекст ChaCha20-Poly1305 + 16-байтовый тег Poly1305
```

---

### Сборка из исходников

**Требования:** Rust 1.75+ (stable)

```bash
git clone https://github.com/NiZaMinius/gm-save
cd gm-save

# Запустить все 46 тестов
cargo test

# Собрать DLL на Windows
cargo build --release -p gm-save-ffi
# Результат: target/release/gm_save.dll

# Кросс-компиляция под Windows с Linux / macOS
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p gm-save-ffi
```

---

<a name="license--лицензия"></a>

## License / Лицензия

Licensed under either of the following, at your option:

- **MIT License** — see [LICENSE-MIT](LICENSE-MIT)
- **Apache License, Version 2.0** — see [LICENSE-APACHE](LICENSE-APACHE)

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project shall be dual-licensed as above, without any
additional terms or conditions.+

---

Лицензировано на ваш выбор по одной из следующих лицензий:

- **Лицензия MIT** — см. [LICENSE-MIT](LICENSE-MIT)
- **Лицензия Apache 2.0** — см. [LICENSE-APACHE](LICENSE-APACHE)

<div align="center">
<br/>
Made with ❤️ by <a href="https://github.com/NiZaMinius">NiZaMinius</a>
</div>
