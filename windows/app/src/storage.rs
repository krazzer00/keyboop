//! Файлы настроек и данных: `%APPDATA%\Keyboop\*.json` (или `KEYBOOP_HOME`).
//!
//! Файлы человек может править руками в Блокноте, поэтому правило простое: сломанный JSON мы
//! НЕ перезаписываем. Пишем в лог, работаем на значениях по умолчанию, а файл остаётся как был —
//! иначе одна пропущенная запятая молча стёрла бы его исключения и сниппеты.

use keyboop_core::exceptions::Exceptions;
use keyboop_core::undo::UndoState;
use keyboop_core::Settings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const SETTINGS: &str = "settings.json";
pub const EXCEPTIONS: &str = "exceptions.json";
pub const SNIPPETS: &str = "snippets.json";
pub const STATE: &str = "state.json";
pub const DICTIONARY: &str = "dictionary.json";
pub const TEXT_SNIPPETS: &str = "text_snippets.json";
pub const LOG: &str = "keyboop.log";

/// Служебное состояние, которое человек руками не правит.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub rescued_count: u64,
    pub undo: UndoState,
    pub typo_personal: BTreeMap<String, u32>,
    /// Последние раскладки, которыми человек пользовался (HKL в hex): "lat" / "cyr".
    pub last_layouts: BTreeMap<String, String>,
    /// Сколько надиктовано (символов и слов) — счётчик в меню.
    pub voice_chars: u64,
    pub voice_words: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn open() -> Store {
        let dir = std::env::var_os("KEYBOOP_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Keyboop")))
            .unwrap_or_else(|| PathBuf::from(".keyboop"));
        let _ = fs::create_dir_all(&dir);
        Store { dir }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn mtime(&self, name: &str) -> Option<SystemTime> {
        fs::metadata(self.path(name))
            .and_then(|m| m.modified())
            .ok()
    }

    /// Прочитать JSON. `Ok(None)` — файла нет; `Err` — файл есть, но сломан.
    fn read<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, String> {
        let p = self.path(name);
        match fs::read(&p) {
            Err(_) => Ok(None),
            Ok(bytes) => {
                let bytes = bytes
                    .strip_prefix(b"\xEF\xBB\xBF")
                    .unwrap_or(&bytes)
                    .to_vec(); // BOM от Блокнота
                serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|e| format!("{name}: {e}"))
            }
        }
    }

    fn write<T: Serialize>(&self, name: &str, v: &T) {
        let p = self.path(name);
        let tmp = p.with_extension("json.tmp");
        if let Ok(s) = serde_json::to_string_pretty(v) {
            if fs::write(&tmp, s).is_ok() {
                let _ = fs::rename(&tmp, &p);
            }
        }
    }

    /// Значение из файла; если файла нет — создаём его со значением по умолчанию.
    fn load_or_create<T: DeserializeOwned + Serialize>(
        &self,
        name: &str,
        default: impl FnOnce() -> T,
        errors: &mut Vec<String>,
    ) -> T {
        match self.read::<T>(name) {
            Ok(Some(v)) => v,
            Ok(None) => {
                let v = default();
                self.write(name, &v);
                v
            }
            Err(e) => {
                errors.push(e);
                default()
            }
        }
    }

    pub fn load_settings(&self, errors: &mut Vec<String>) -> Settings {
        let s: Settings = self.load_or_create(SETTINGS, Settings::default, errors);
        // Новые поля (добавленные в новой версии) дописываем в файл, чтобы человек их видел.
        if errors.is_empty() {
            self.write(SETTINGS, &s);
        }
        s
    }

    pub fn save_settings(&self, s: &Settings) {
        self.write(SETTINGS, s);
    }

    pub fn load_exceptions(&self, errors: &mut Vec<String>) -> Exceptions {
        self.load_or_create(EXCEPTIONS, Exceptions::seeded, errors)
    }

    pub fn save_exceptions(&self, e: &Exceptions) {
        self.write(EXCEPTIONS, e);
    }

    pub fn load_snippets(&self, errors: &mut Vec<String>) -> Vec<(String, String)> {
        let v: Vec<Snippet> = self.load_or_create(SNIPPETS, Vec::new, errors);
        v.into_iter().map(|s| (s.trigger, s.text)).collect()
    }

    /// Словарь диктовки. Нет файла — создаём с заготовками (как засев на Маке).
    pub fn load_dictionary(&self) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        let v: Vec<keyboop_core::voice::dictionary::DictPair> = self.load_or_create(
            DICTIONARY,
            || {
                keyboop_core::voice::dictionary::seed_pairs()
                    .into_iter()
                    .map(
                        |(heard, written)| keyboop_core::voice::dictionary::DictPair {
                            heard,
                            written,
                        },
                    )
                    .collect()
            },
            &mut errors,
        );
        v.into_iter().map(|p| (p.heard, p.written)).collect()
    }

    pub fn save_dictionary(&self, pairs: &[(String, String)]) {
        let v: Vec<keyboop_core::voice::dictionary::DictPair> = pairs
            .iter()
            .map(|(h, w)| keyboop_core::voice::dictionary::DictPair {
                heard: h.clone(),
                written: w.clone(),
            })
            .collect();
        self.write(DICTIONARY, &v);
    }

    /// Текстовые сниппеты для вставки по цифре: [{"trigger": название, "text": текст}].
    pub fn load_text_snippets(&self) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        let v: Vec<Snippet> = self.load_or_create(TEXT_SNIPPETS, Vec::new, &mut errors);
        v.into_iter().map(|s| (s.trigger, s.text)).collect()
    }

    pub fn save_snippets(&self, name: &str, pairs: &[(String, String)]) {
        let v: Vec<Snippet> = pairs
            .iter()
            .map(|(t, x)| Snippet {
                trigger: t.clone(),
                text: x.clone(),
            })
            .collect();
        self.write(name, &v);
    }

    pub fn load_state(&self) -> State {
        self.read(STATE).ok().flatten().unwrap_or_default()
    }

    pub fn save_state(&self, s: &State) {
        self.write(STATE, s);
    }
}

/// Лог диагностики. Содержимое набранного сюда не попадает никогда — только длины и классы.
pub struct Log {
    path: PathBuf,
}

impl Log {
    pub fn new(dir: &Path) -> Log {
        let path = dir.join(LOG);
        // Простая ротация: больше мегабайта — начинаем заново.
        if fs::metadata(&path)
            .map(|m| m.len() > 1_000_000)
            .unwrap_or(false)
        {
            let _ = fs::rename(&path, dir.join("keyboop.old.log"));
        }
        Log { path }
    }

    pub fn write(&self, msg: &str) {
        use std::io::Write;
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(f, "{h:02}:{m:02}:{s:02} UTC  {msg}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_file_is_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("keyboop-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let store = Store { dir: dir.clone() };
        fs::write(store.path(EXCEPTIONS), "{ broken").unwrap();
        let mut errors = Vec::new();
        let e = store.load_exceptions(&mut errors);
        assert_eq!(errors.len(), 1);
        assert!(e.ignored.contains("вк"));
        assert_eq!(
            fs::read_to_string(store.path(EXCEPTIONS)).unwrap(),
            "{ broken"
        );
        let mut errors = Vec::new();
        let s = store.load_settings(&mut errors);
        assert!(s.auto_enabled && errors.is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
