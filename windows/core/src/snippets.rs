//! Перенос `SnippetStore.swift`: автозамена «сокращение → текст». Совпадение каноническое:
//! раскладка и регистр набора не важны («ьфшд» и «MAIL» раскрывают сниппет «mail»).

use crate::keymap;
use std::collections::HashMap;

#[derive(Default)]
pub struct SnippetStore {
    pairs: Vec<(String, String)>,
    by_canonical: HashMap<String, String>,
}

pub fn canonical(s: &str) -> String {
    keymap::convert(s, false).to_lowercase()
}

impl SnippetStore {
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        let mut s = SnippetStore::default();
        s.set_all(pairs);
        s
    }

    pub fn set_all(&mut self, pairs: Vec<(String, String)>) {
        let mut out: Vec<(String, String)> = Vec::new();
        for (t, e) in pairs {
            let trig = t.trim().to_string();
            if trig.is_empty() {
                continue;
            }
            let c = canonical(&trig);
            if let Some(p) = out.iter_mut().find(|p| canonical(&p.0) == c) {
                p.1 = e;
            } else {
                out.push((trig, e));
            }
        }
        self.pairs = out;
        self.rebuild_index();
    }

    /// Живая таблица раскладок появилась позже — канон надо пересчитать.
    pub fn rebuild_index(&mut self) {
        self.by_canonical = self
            .pairs
            .iter()
            .map(|(t, e)| (canonical(t), e.clone()))
            .filter(|(c, _)| !c.is_empty())
            .collect();
    }

    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    pub fn expansion(&self, typed: &str) -> Option<&str> {
        if self.by_canonical.is_empty() {
            return None;
        }
        self.by_canonical.get(&canonical(typed)).map(String::as_str)
    }
}

/// Управляющие символы из раскрытия режем (кроме \n и \t).
pub fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\n' || c == '\t' || c as u32 >= 0x20)
        .collect()
}
