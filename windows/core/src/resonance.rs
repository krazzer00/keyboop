//! Перенос `AntiResonanceGuard.swift`: предохранитель от циклического переключения A→B→A.

pub struct AntiResonanceGuard {
    window: f64,
    max_flips: usize,
    freeze_for: f64,
    recent: Vec<(String, f64)>,
    frozen_until: f64,
}

impl Default for AntiResonanceGuard {
    fn default() -> Self {
        Self {
            window: 0.7,
            max_flips: 6,
            freeze_for: 2.5,
            recent: Vec::new(),
            frozen_until: 0.0,
        }
    }
}

impl AntiResonanceGuard {
    pub fn allow(&mut self, word: &str, produced: &str, now: f64) -> bool {
        if now < self.frozen_until {
            return false;
        }
        let window = self.window;
        self.recent.retain(|(_, at)| now - at <= window);
        let oscillation = self.recent.iter().any(|(p, _)| p == word);
        self.recent.push((produced.to_string(), now));
        if oscillation || self.recent.len() > self.max_flips {
            self.frozen_until = now + self.freeze_for;
            self.recent.clear();
            return false;
        }
        true
    }

    pub fn reset_history(&mut self) {
        self.recent.clear();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn freezes_on_oscillation() {
        let mut g = super::AntiResonanceGuard::default();
        assert!(g.allow("ghbdtn", "привет", 0.0));
        assert!(!g.allow("привет", "ghbdtn", 0.1));
        assert!(!g.allow("abc", "фис", 0.2));
        assert!(g.allow("abc", "фис", 3.0));
    }
}
