//! Хранилище истории (перенос `VoiceHistory.swift` + `VoiceClips.swift`) для Windows.
//!
//! Файл истории и аудиоклипы шифруются DPAPI (CryptProtectData): расшифровать их может только
//! этот пользователь Windows на этом компьютере — тот же смысл, что ключ в Keychain на Маке.
//! Окно истории (отдельный процесс) читает тот же файл тем же способом.

use keyboop_core::history::{self, HistoryEntry, HistoryKind};
use std::path::PathBuf;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        if CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        ) == 0
        {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as HLOCAL);
        Some(v)
    }
}

pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        if CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        ) == 0
        {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as HLOCAL);
        Some(v)
    }
}

pub fn data_dir() -> PathBuf {
    let d = std::env::var_os("LOCALAPPDATA")
        .map(|a| PathBuf::from(a).join("Keyboop"))
        .unwrap_or_else(|| PathBuf::from(".keyboop-local"));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn file() -> PathBuf {
    data_dir().join("history.bin")
}

fn clips_dir() -> PathBuf {
    let d = data_dir().join("voice");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn load() -> Vec<HistoryEntry> {
    let Ok(raw) = std::fs::read(file()) else {
        return Vec::new();
    };
    let Some(json) = unprotect(&raw) else {
        return Vec::new();
    };
    serde_json::from_slice(&json).unwrap_or_default()
}

pub fn save(entries: &[HistoryEntry]) {
    if let Ok(json) = serde_json::to_vec(entries) {
        if let Some(sealed) = protect(&json) {
            let tmp = file().with_extension("tmp");
            if std::fs::write(&tmp, sealed).is_ok() {
                let _ = std::fs::rename(tmp, file());
            }
        }
    }
}

/// Кэш в памяти главного процесса; окно истории меняет файл — `reload()`.
static CACHE: Mutex<Option<Vec<HistoryEntry>>> = Mutex::new(None);

fn with_cache<R>(f: impl FnOnce(&mut Vec<HistoryEntry>) -> R) -> R {
    let mut c = CACHE.lock().unwrap();
    if c.is_none() {
        *c = Some(load());
    }
    f(c.as_mut().unwrap())
}

pub fn reload() {
    *CACHE.lock().unwrap() = Some(load());
}

/// Добавить запись (с подчисткой по сроку и лимитам). `minutes` — срок хранения (0 — всё).
pub fn add(entry: HistoryEntry, minutes: u32) {
    with_cache(|c| {
        c.push(entry);
        for a in history::prune(c, minutes, now()) {
            delete_clip(&a);
        }
        let (kept, dropped) = history::capped(c);
        for d in dropped {
            if let Some(a) = d.audio {
                delete_clip(&a);
            }
        }
        *c = kept;
        save(c);
    });
}

pub fn add_text(
    text: &str,
    kind: HistoryKind,
    app: Option<String>,
    audio: Option<(String, Vec<u8>)>,
    minutes: u32,
) {
    let (audio, wave) = match audio {
        Some((id, w)) => (Some(id), Some(w)),
        None => (None, None),
    };
    add(
        HistoryEntry {
            date: now(),
            text: text.to_string(),
            audio,
            wave,
            kind: Some(kind),
            app,
        },
        minutes,
    );
}

pub fn last_dictation(minutes: u32) -> Option<String> {
    with_cache(|c| {
        let e = history::last_dictation(c)?;
        if minutes > 0 && e.date < now() - minutes as f64 * 60.0 {
            return None;
        }
        Some(e.text.clone())
    })
}

pub fn last_clipboard_text() -> Option<String> {
    with_cache(|c| history::last_clipboard_text(c).map(str::to_string))
}

pub fn all() -> Vec<HistoryEntry> {
    with_cache(|c| c.clone())
}

// ── Аудиоклипы ───────────────────────────────────────────────────────────────

/// Сохранить запись диктовки (WAV 16 кГц, зашифровано). Возвращает id и огибающую.
pub fn save_clip(samples: &[f32]) -> Option<(String, Vec<u8>)> {
    if samples.is_empty() {
        return None;
    }
    let wav = crate::synth::wav_from_f32(samples, 16_000);
    let sealed = protect(&wav)?;
    let id = format!("{:x}{:08x}", (now() * 1000.0) as u64, rand_u32());
    std::fs::write(clips_dir().join(format!("{id}.bin")), sealed).ok()?;
    Some((id, history::envelope(samples, 64)))
}

pub fn load_clip(id: &str) -> Option<Vec<u8>> {
    let raw = std::fs::read(clips_dir().join(format!("{id}.bin"))).ok()?;
    unprotect(&raw)
}

pub fn delete_clip(id: &str) {
    let _ = std::fs::remove_file(clips_dir().join(format!("{id}.bin")));
}

fn rand_u32() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(now().to_bits());
    h.finish() as u32
}
