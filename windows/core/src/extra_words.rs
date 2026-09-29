//! Списки из `ExtraWords.swift` (генерирует build.rs) в виде множеств.

use std::collections::HashSet;
use std::sync::OnceLock;

mod raw {
    include!(concat!(env!("OUT_DIR"), "/extra_words.rs"));
}

pub use raw::*;

pub struct Sets {
    pub ru_abbr: HashSet<&'static str>,
    pub ru_short: HashSet<&'static str>,
    pub force_ru_amb: HashSet<&'static str>,
    pub force_en_amb: HashSet<&'static str>,
    pub tech_latin_tokens: HashSet<&'static str>,
    pub tech_from_cyrillic: HashSet<&'static str>,
    pub default_keep: HashSet<&'static str>,
    pub hyphen_terms: HashSet<&'static str>,
    pub ru_hyphen_terms: HashSet<&'static str>,
    pub ru_label_classifiers: HashSet<&'static str>,
    pub en_keep_short: HashSet<&'static str>,
    pub label_classifiers: HashSet<&'static str>,
    pub ru: HashSet<&'static str>,
    pub en: HashSet<&'static str>,
}

fn set(v: &[&'static str]) -> HashSet<&'static str> {
    v.iter().copied().collect()
}

pub fn sets() -> &'static Sets {
    static S: OnceLock<Sets> = OnceLock::new();
    S.get_or_init(|| Sets {
        ru_abbr: set(RU_ABBR),
        ru_short: set(RU_SHORT),
        force_ru_amb: set(FORCE_RU_AMB),
        force_en_amb: set(FORCE_EN_AMB),
        tech_latin_tokens: set(TECH_LATIN_TOKENS),
        tech_from_cyrillic: set(TECH_FROM_CYRILLIC),
        default_keep: set(DEFAULT_KEEP),
        hyphen_terms: set(HYPHEN_TERMS),
        ru_hyphen_terms: set(RU_HYPHEN_TERMS),
        ru_label_classifiers: set(RU_LABEL_CLASSIFIERS),
        en_keep_short: set(EN_KEEP_SHORT),
        label_classifiers: set(LABEL_CLASSIFIERS),
        ru: set(RU),
        en: set(EN),
    })
}
