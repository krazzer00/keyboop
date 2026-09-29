//! Перенос `UndoLearner.swift`: обучение на отмене. Если человек сразу откатил нашу авто-
//! конверсию и точь-в-точь восстановил исходник, слово получает защиту на сессию, а после трёх
//! таких откатов за неделю мы предлагаем больше его не трогать.
//!
//! Постоянная часть (отказы и счётчики) — [`UndoState`], её сохраняет платформа.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const UNDO_WINDOW: f64 = 4.0;
const STRIKE_THRESHOLD: u32 = 3;
const STRIKE_DECAY: f64 = 7.0 * 24.0 * 3600.0;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UndoState {
    pub declined: BTreeSet<String>,
    pub strike_count: BTreeMap<String, u32>,
    pub strike_time: BTreeMap<String, f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Fresh,
    Deleting,
    Retyping,
}

struct Candidate {
    original: String,
    converted: String,
    created_at: f64,
    stage: Stage,
}

#[derive(Default)]
pub struct UndoLearner {
    pub enabled: bool,
    pub state: UndoState,
    candidate: Option<Candidate>,
    session_protected: HashSet<String>,
    pending: HashSet<String>,
    /// Слова, по которым пора спросить «больше не переключать?» — забирает движок.
    pub suggestions: Vec<String>,
    pub dirty: bool,
}

impl UndoLearner {
    pub fn new(state: UndoState, enabled: bool) -> Self {
        Self {
            enabled,
            state,
            ..Default::default()
        }
    }

    fn live_candidate(&mut self, now: f64) -> Option<&mut Candidate> {
        if let Some(c) = &self.candidate {
            if now - c.created_at > UNDO_WINDOW {
                self.candidate = None;
            }
        }
        self.candidate.as_mut()
    }

    fn register_strike(&mut self, w: &str, now: f64) -> bool {
        if let Some(&last) = self.state.strike_time.get(w) {
            if now - last > STRIKE_DECAY {
                self.state.strike_count.insert(w.to_string(), 0);
            }
        }
        let c = self.state.strike_count.get(w).copied().unwrap_or(0) + 1;
        self.state.strike_count.insert(w.to_string(), c);
        self.state.strike_time.insert(w.to_string(), now);
        self.dirty = true;
        c >= STRIKE_THRESHOLD
    }

    fn clear_strikes(&mut self, w: &str) {
        self.state.strike_count.remove(w);
        self.state.strike_time.remove(w);
        self.dirty = true;
    }

    fn record_relevant_undo(&mut self, w: &str, learned: &BTreeSet<String>, now: f64) {
        self.session_protected.insert(w.to_string());
        if self.pending.contains(w) {
            return;
        }
        if self.register_strike(w, now) && self.suggest_learn(w, learned) {
            self.clear_strikes(w);
        }
    }

    fn suggest_learn(&mut self, w: &str, learned: &BTreeSet<String>) -> bool {
        if !self.enabled
            || learned.contains(w)
            || self.state.declined.contains(w)
            || self.pending.contains(w)
        {
            return false;
        }
        self.pending.insert(w.to_string());
        self.suggestions.push(w.to_string());
        true
    }

    /// Авто-конверсия произошла — кандидат на откат.
    pub fn note_conversion(
        &mut self,
        original: &str,
        converted: &str,
        learned: &BTreeSet<String>,
        now: f64,
    ) {
        if !self.enabled {
            self.candidate = None;
            return;
        }
        let (o, c) = (original.to_lowercase(), converted.to_lowercase());
        if o == c
            || o.is_empty()
            || c.is_empty()
            || learned.contains(&o)
            || self.state.declined.contains(&o)
        {
            self.candidate = None;
            return;
        }
        self.candidate = Some(Candidate {
            original: o,
            converted: c,
            created_at: now,
            stage: Stage::Fresh,
        });
    }

    /// Ручной ре-флип нашей недавней авто-конверсии — это откат.
    pub fn note_manual_convert(
        &mut self,
        from: &str,
        to: &str,
        learned: &BTreeSet<String>,
        now: f64,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let (f, t) = (from.to_lowercase(), to.to_lowercase());
        let Some(c) = self.live_candidate(now) else {
            return false;
        };
        if f != c.converted || t != c.original {
            return false;
        }
        let orig = c.original.clone();
        self.candidate = None;
        self.record_relevant_undo(&orig, learned, now);
        true
    }

    /// Следим за стиранием нашего вывода и перенабором оригинала.
    pub fn observe(&mut self, current: &str, learned: &BTreeSet<String>, now: f64) -> bool {
        if !self.enabled {
            return false;
        }
        let cur = current.to_lowercase();
        let Some(c) = self.live_candidate(now) else {
            return false;
        };
        match c.stage {
            Stage::Fresh | Stage::Deleting => {
                if cur.is_empty() {
                    c.stage = Stage::Retyping;
                } else if c.converted.starts_with(&cur)
                    && cur.chars().count() < c.converted.chars().count()
                {
                    c.stage = Stage::Deleting;
                } else {
                    self.candidate = None;
                }
                false
            }
            Stage::Retyping => {
                if cur == c.original {
                    let orig = c.original.clone();
                    self.candidate = None;
                    self.record_relevant_undo(&orig, learned, now);
                    true
                } else if c.original.starts_with(&cur) {
                    false
                } else {
                    self.candidate = None;
                    false
                }
            }
        }
    }

    pub fn should_suppress(&mut self, current: &str, now: f64) -> bool {
        if !self.enabled {
            return false;
        }
        let cur = current.to_lowercase();
        match self.live_candidate(now) {
            Some(c) if c.stage == Stage::Retyping => {
                !cur.is_empty() && c.original.starts_with(&cur) && cur != c.original
            }
            _ => false,
        }
    }

    pub fn is_session_protected(&self, word: &str) -> bool {
        self.session_protected.contains(&word.to_lowercase())
    }

    pub fn protect(&mut self, word: &str) {
        let w = word.to_lowercase();
        if !w.is_empty() {
            self.session_protected.insert(w);
        }
    }

    pub fn reset_context(&mut self) {
        self.candidate = None;
        self.session_protected.clear();
    }

    /// Человек согласился: слово переезжает в выученные (это делает движок), счётчик чистим.
    pub fn confirm(&mut self, word: &str) {
        let w = word.to_lowercase();
        self.pending.remove(&w);
        self.clear_strikes(&w);
    }

    pub fn decline(&mut self, word: &str) {
        let w = word.to_lowercase();
        self.pending.remove(&w);
        self.state.declined.insert(w.clone());
        self.clear_strikes(&w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retype_after_delete_is_undo() {
        let learned = BTreeSet::new();
        let mut u = UndoLearner::new(UndoState::default(), true);
        u.note_conversion("гифки", "ubarb", &learned, 0.0);
        for cur in ["ubar", "uba", "ub", "u", ""] {
            assert!(!u.observe(cur, &learned, 0.5));
        }
        for cur in ["г", "ги", "гиф", "гифк"] {
            assert!(!u.observe(cur, &learned, 1.0));
        }
        assert!(u.observe("гифки", &learned, 1.5));
        assert!(u.is_session_protected("гифки"));
    }

    #[test]
    fn three_strikes_suggest() {
        let learned = BTreeSet::new();
        let mut u = UndoLearner::new(UndoState::default(), true);
        for i in 0..3 {
            let t = i as f64 * 10.0;
            u.note_conversion("гифки", "ubarb", &learned, t);
            assert!(u.note_manual_convert("ubarb", "гифки", &learned, t + 1.0));
        }
        assert_eq!(u.suggestions, vec!["гифки".to_string()]);
    }
}
