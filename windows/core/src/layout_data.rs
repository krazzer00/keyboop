//! Перенос `LayoutData.swift`: триграммы и словари RU/EN.
//!
//! Файлы данных те же, что у мак-версии (`Sources/Keyboop/Resources`), и вшиваются прямо в
//! бинарник: у Windows-сборки нет бандла, а один exe без папки с данными проще раздавать.
//! Разбор ~5 МБ JSON занимает десятки миллисекунд, поэтому платформа зовёт [`warm_up`] в
//! фоне при старте, а движок до готовности молчит (`Warm.isReady` в Swift).

use crate::extra_words as ew;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

macro_rules! res {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../Sources/Keyboop/Resources/",
            $name
        ))
    };
}

static TRIGRAMS_RU: &[u8] = res!("trigrams_ru.json");
static TRIGRAMS_EN: &[u8] = res!("trigrams_en.json");
static WORDS_RU: &[u8] = res!("words_ru.json");
static WORDS_EN: &[u8] = res!("words_en.json");
pub static TYPO_RULES: &[u8] = res!("typo_rules.json");

pub struct LayoutData {
    pub trigrams_ru: HashMap<String, f64>,
    pub trigrams_en: HashMap<String, f64>,
    pub words_ru: HashSet<String>,
    pub words_en: HashSet<String>,
}

static DATA: OnceLock<LayoutData> = OnceLock::new();

/// Загрузить данные (идемпотентно). Зовётся в фоне при старте.
pub fn warm_up() {
    shared();
}

/// Данные готовы — движку можно принимать решения.
pub fn is_ready() -> bool {
    DATA.get().is_some()
}

pub fn shared() -> &'static LayoutData {
    DATA.get_or_init(LayoutData::load)
}

impl LayoutData {
    fn load() -> Self {
        let trigrams_ru: HashMap<String, f64> =
            serde_json::from_slice(TRIGRAMS_RU).unwrap_or_default();
        let trigrams_en: HashMap<String, f64> =
            serde_json::from_slice(TRIGRAMS_EN).unwrap_or_default();
        let mut words_ru: HashSet<String> = serde_json::from_slice::<Vec<String>>(WORDS_RU)
            .unwrap_or_default()
            .into_iter()
            .collect();
        for list in [
            ew::RU,
            ew::RU_DEV,
            ew::RU_ABBR,
            ew::RU_SHORT,
            ew::RU_COMMON_FORMS,
            ew::RU_LOAN_NAMES,
        ] {
            words_ru.extend(list.iter().map(|s| s.to_string()));
        }
        let mut words_en: HashSet<String> = serde_json::from_slice::<Vec<String>>(WORDS_EN)
            .unwrap_or_default()
            .into_iter()
            .collect();
        words_en.extend(ew::EN.iter().map(|s| s.to_string()));
        LayoutData {
            trigrams_ru,
            trigrams_en,
            words_ru,
            words_en,
        }
    }

    /// Средняя лог-вероятность триграмм слова (с паддингом пробелами). Штраф −20 за отсутствие.
    pub fn plausibility(&self, word: &str, cyrillic: bool) -> f64 {
        let table = if cyrillic {
            &self.trigrams_ru
        } else {
            &self.trigrams_en
        };
        let mut chars: Vec<char> = vec![' '];
        chars.extend(word.to_lowercase().chars());
        chars.push(' ');
        if chars.len() < 3 {
            return f64::NEG_INFINITY;
        }
        let mut sum = 0.0;
        let mut key = String::with_capacity(12);
        for w in chars.windows(3) {
            key.clear();
            key.extend(w.iter());
            sum += table.get(&key).copied().unwrap_or(-20.0);
        }
        sum / (chars.len() - 2) as f64
    }

    pub fn is_ru_word(&self, w: &str) -> bool {
        self.words_ru.contains(w)
    }

    pub fn is_en_word(&self, w: &str) -> bool {
        self.words_en.contains(w)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn loads() {
        let d = super::shared();
        assert!(d.trigrams_ru.len() > 1000);
        assert!(d.words_ru.contains("привет"));
        assert!(d.words_en.contains("hello"));
    }
}
