//! Перенос `TypoFix.swift` + `NumericTypoRule.swift`: исправление опечаток (выключено по
//! умолчанию). Курируемая таблица `typo_rules.json`, механика «перестановка/удвоение» по словарю
//! и личный словарь слов, которые человек пишет по-своему (копится всегда).

use crate::exceptions::Exceptions;
use crate::layout_data::{self, TYPO_RULES};
use crate::text::*;
use std::collections::{BTreeMap, HashMap};

const PERSONAL_THRESHOLD: u32 = 2;

#[derive(Default)]
pub struct TypoFix {
    rules_ru: HashMap<String, String>,
    rules_en: HashMap<String, String>,
    pub personal: BTreeMap<String, u32>,
    pub dirty: bool,
    ready: bool,
}

/// Защиты, которые знает движок (исключения и сессионная защита отката).
pub struct Guard<'a> {
    pub exceptions: &'a Exceptions,
    pub session_protected: &'a dyn Fn(&str) -> bool,
}

impl TypoFix {
    pub fn load(personal: BTreeMap<String, u32>) -> Self {
        let all: HashMap<String, HashMap<String, String>> =
            serde_json::from_slice(TYPO_RULES).unwrap_or_default();
        let mut t = TypoFix {
            rules_ru: all.get("ru").cloned().unwrap_or_default(),
            rules_en: all.get("en").cloned().unwrap_or_default(),
            personal,
            dirty: false,
            ready: true,
        };
        let wrong: Vec<String> = t
            .personal
            .keys()
            .filter(|w| t.candidate(w).is_some() || !w.chars().all(is_letter))
            .cloned()
            .collect();
        for w in wrong {
            t.personal.remove(&w);
            t.dirty = true;
        }
        t
    }

    fn in_dictionaries(w: &str) -> bool {
        let d = layout_data::shared();
        d.words_ru.contains(w) || d.words_en.contains(w)
    }

    /// Личный словарь пополняем всегда, даже когда функция выключена.
    pub fn note_typed(&mut self, word: &str) {
        let w = word.to_lowercase();
        if !self.ready
            || char_count(&w) < 4
            || Self::in_dictionaries(&w)
            || !w.chars().all(is_letter)
        {
            return;
        }
        if self.candidate(&w).is_some() {
            return;
        }
        let n = self.personal.get(&w).copied().unwrap_or(0) + 1;
        if n > PERSONAL_THRESHOLD {
            return;
        }
        self.personal.insert(w, n);
        self.dirty = true;
    }

    pub fn is_personal(&self, w: &str) -> bool {
        self.personal.get(&w.to_lowercase()).copied().unwrap_or(0) >= PERSONAL_THRESHOLD
    }

    fn mechanical(&self, w: &str) -> Option<String> {
        let chars: Vec<char> = w.chars().collect();
        if chars.len() < 5 {
            return None;
        }
        let d = layout_data::shared();
        let dict = if has_cyrillic(w) {
            &d.words_ru
        } else {
            &d.words_en
        };
        let mut found: Option<String> = None;
        let mut offer = |s: String| -> bool {
            if !dict.contains(&s) {
                return false;
            }
            if let Some(f) = &found {
                if *f != s {
                    found = None;
                    return true;
                }
            }
            found = Some(s);
            false
        };
        let mut c = chars.clone();
        for i in 0..chars.len() - 2 {
            c.swap(i, i + 1);
            let s: String = c.iter().collect();
            c.swap(i, i + 1);
            if offer(s) {
                return None;
            }
        }
        for i in 1..chars.len() - 2 {
            if chars[i] == chars[i + 1] && chars[i] != 'н' {
                let mut c2 = chars.clone();
                c2.remove(i);
                if offer(c2.into_iter().collect()) {
                    return None;
                }
            }
        }
        found
    }

    fn table_candidate(&self, w: &str) -> Option<String> {
        if has_cyrillic(w) {
            self.rules_ru.get(w)
        } else {
            self.rules_en.get(w)
        }
        .cloned()
    }

    fn candidate(&self, w: &str) -> Option<String> {
        if let Some(f) = self.table_candidate(w) {
            return Some(f);
        }
        if !w.chars().all(is_letter) {
            return None;
        }
        self.mechanical(w)
    }

    fn is_protected(w: &str, g: &Guard) -> bool {
        g.exceptions.is_ignored(w) || g.exceptions.is_learned(w) || (g.session_protected)(w)
    }

    fn restoring_leading_case(original: &str, fixed: &str) -> String {
        if original.chars().next().is_some_and(is_upper) {
            upper_first(fixed)
        } else {
            fixed.to_string()
        }
    }

    fn punctuation_wrapped_core(word: &str) -> Option<(String, String, String)> {
        let c: Vec<char> = word.chars().collect();
        let mut start = 0;
        while start < c.len() && is_punctuation(c[start]) {
            start += 1;
        }
        let mut end = c.len();
        while end > start && is_punctuation(c[end - 1]) {
            end -= 1;
        }
        if start >= end {
            return None;
        }
        Some((
            c[..start].iter().collect(),
            c[start..end].iter().collect(),
            c[end..].iter().collect(),
        ))
    }

    /// `1ю8` → `1.8` (отзыв #218).
    pub fn numeric_suggestion(&self, word: &str, g: &Guard) -> Option<String> {
        let fixed = numeric_rule(word)?;
        if Self::is_protected(&word.to_lowercase(), g) {
            return None;
        }
        Some(fixed)
    }

    pub fn curated_suggestion(&self, word: &str, g: &Guard) -> Option<String> {
        let (prefix, core, suffix) = Self::punctuation_wrapped_core(word)?;
        let w = core.to_lowercase();
        if char_count(&w) < 4 || has_cyrillic(&core) == has_latin_letter(&core) {
            return None;
        }
        let whole = word.to_lowercase();
        if Self::in_dictionaries(&w)
            || self.is_personal(&w)
            || self.is_personal(&whole)
            || Self::is_protected(&w, g)
            || Self::is_protected(&whole, g)
        {
            return None;
        }
        let fixed = self.table_candidate(&w)?;
        Some(prefix + &Self::restoring_leading_case(&core, &fixed) + &suffix)
    }

    pub fn suggest(&self, word: &str, g: &Guard) -> Option<String> {
        if let Some(f) = self.numeric_suggestion(word, g) {
            return Some(f);
        }
        if let Some(f) = self.curated_suggestion(word, g) {
            return Some(f);
        }
        let w = word.to_lowercase();
        if char_count(&w) < 4
            || !w.chars().all(is_letter)
            || has_cyrillic(word) == has_latin_letter(word)
        {
            return None;
        }
        if Self::in_dictionaries(&w) || self.is_personal(&w) || Self::is_protected(&w, g) {
            return None;
        }
        let fixed = self.mechanical(&w)?;
        Some(Self::restoring_leading_case(word, &fixed))
    }
}

/// «ю» между двумя цифрами — это точка, набранная в русской раскладке.
pub fn numeric_rule(text: &str) -> Option<String> {
    let mut c: Vec<char> = text.chars().collect();
    if c.len() < 3 {
        return None;
    }
    let mut changed = false;
    for i in 1..c.len() - 1 {
        if c[i] == 'ю' && c[i - 1].is_ascii_digit() && c[i + 1].is_ascii_digit() {
            c[i] = '.';
            changed = true;
        }
    }
    changed.then(|| c.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes() {
        let t = TypoFix::load(BTreeMap::new());
        let e = Exceptions::default();
        let g = Guard {
            exceptions: &e,
            session_protected: &|_| false,
        };
        assert_eq!(t.suggest("1ю8", &g).as_deref(), Some("1.8"));
        assert_eq!(t.suggest("acheive", &g).as_deref(), Some("achieve"));
        assert_eq!(t.suggest("hello", &g), None);
    }
}
