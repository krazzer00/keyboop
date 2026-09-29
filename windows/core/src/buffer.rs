//! Перенос `KeystrokeBuffer.swift`: локальная модель набранного (текущее слово, последнее
//! завершённое с хвостом, история слов сессии). Время передаётся снаружи (монотонные секунды),
//! чтобы буфер можно было гонять в тестах без часов.

#[derive(Clone, Debug)]
pub struct SessionWord {
    pub word: String,
    pub tail: String,
}

pub struct ConversionItem {
    pub word: String,
    pub delete_count: usize,
    pub tail: String,
}

pub struct Group {
    pub words: Vec<SessionWord>,
    pub delete_count: usize,
}

#[derive(Default)]
pub struct KeystrokeBuffer {
    pub current_word: String,
    pub last_word: String,
    pub last_tail: String,
    session_words: Vec<SessionWord>,
    last_activity: f64,
    /// Самая длинная пауза между буквами внутри слова (отличает набор от горячих клавиш).
    pub current_word_gap: f64,
    pub last_word_gap: f64,
}

const GROUP_MAX_CHARS: usize = 200;
const GROUP_MAX_IDLE: f64 = 8.0;

fn len(s: &str) -> usize {
    s.chars().count()
}

fn pop_char(s: &mut String) {
    s.pop();
}

impl KeystrokeBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, s: &str, now: f64) {
        if !self.current_word.is_empty() {
            self.current_word_gap = self.current_word_gap.max(now - self.last_activity);
        }
        self.current_word.push_str(s);
        self.last_activity = now;
    }

    pub fn backspace(&mut self, now: f64) {
        self.last_activity = now;
        if !self.current_word.is_empty() {
            pop_char(&mut self.current_word);
            if self.current_word.is_empty() {
                self.last_word.clear();
                self.last_tail.clear();
                self.session_words.clear();
                self.current_word_gap = 0.0;
            }
        } else if !self.last_tail.is_empty() {
            pop_char(&mut self.last_tail);
            if let Some(l) = self.session_words.last_mut() {
                l.tail = self.last_tail.clone();
            }
        } else if !self.last_word.is_empty() {
            // Backspace вошёл в завершённое слово: возвращаем его в текущее.
            self.current_word = std::mem::take(&mut self.last_word);
            self.current_word_gap = self.last_word_gap;
            self.session_words.pop();
            self.last_word = self
                .session_words
                .last()
                .map(|w| w.word.clone())
                .unwrap_or_default();
            self.last_tail = self
                .session_words
                .last()
                .map(|w| w.tail.clone())
                .unwrap_or_default();
            pop_char(&mut self.current_word);
            if self.current_word.is_empty() {
                self.last_word.clear();
                self.last_tail.clear();
                self.session_words.clear();
                self.current_word_gap = 0.0;
            }
        } else {
            self.clear();
        }
    }

    /// Граница слова (пробел/таб/ввод).
    pub fn boundary(&mut self, ws: &str, now: f64) {
        self.last_activity = now;
        if !self.current_word.is_empty() {
            let w = std::mem::take(&mut self.current_word);
            self.session_words.push(SessionWord {
                word: w.clone(),
                tail: ws.to_string(),
            });
            self.last_word = w;
            self.last_tail = ws.to_string();
            self.last_word_gap = self.current_word_gap;
            self.current_word_gap = 0.0;
        } else if !self.last_word.is_empty() {
            self.last_tail.push_str(ws);
            if let Some(l) = self.session_words.last_mut() {
                l.tail.push_str(ws);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.current_word.is_empty() && self.last_word.is_empty()
    }

    pub fn clear(&mut self) {
        self.current_word.clear();
        self.last_word.clear();
        self.last_tail.clear();
        self.session_words.clear();
        self.current_word_gap = 0.0;
        self.last_word_gap = 0.0;
    }

    /// Мягкий сброс: забываем завершённый контекст, но не набираемое слово.
    pub fn soft_context_reset(&mut self) {
        self.last_word.clear();
        self.last_tail.clear();
        self.session_words.clear();
    }

    pub fn invalidate_group_history(&mut self) {
        self.session_words.clear();
    }

    /// Группа для конвертации нескольких слов одним хоткеем.
    pub fn group_for_conversion(&self, now: f64) -> Option<Group> {
        if now - self.last_activity > GROUP_MAX_IDLE {
            return None;
        }
        let mut words = self.session_words.clone();
        if !self.current_word.is_empty() {
            words.push(SessionWord {
                word: self.current_word.clone(),
                tail: String::new(),
            });
        }
        if words.len() < 2 {
            return None;
        }
        if words
            .iter()
            .any(|w| w.tail.contains('\n') || w.tail.contains('\t'))
        {
            return None;
        }
        let total: usize = words.iter().map(|w| len(&w.word) + len(&w.tail)).sum();
        if total == 0 || total > GROUP_MAX_CHARS {
            return None;
        }
        Some(Group {
            words,
            delete_count: total,
        })
    }

    /// Что конвертировать: текущее слово, иначе последнее с хвостом. `completed_only` — строго
    /// завершённое слово, начатое следующее уходит в хвост (аудит C2).
    pub fn word_for_conversion(&self, completed_only: bool) -> Option<ConversionItem> {
        if completed_only {
            if self.last_word.is_empty() {
                return None;
            }
            return Some(ConversionItem {
                word: self.last_word.clone(),
                delete_count: len(&self.last_word) + len(&self.last_tail) + len(&self.current_word),
                tail: format!("{}{}", self.last_tail, self.current_word),
            });
        }
        if !self.current_word.is_empty() {
            return Some(ConversionItem {
                word: self.current_word.clone(),
                delete_count: len(&self.current_word),
                tail: String::new(),
            });
        }
        if !self.last_word.is_empty() {
            return Some(ConversionItem {
                word: self.last_word.clone(),
                delete_count: len(&self.last_word) + len(&self.last_tail),
                tail: self.last_tail.clone(),
            });
        }
        None
    }

    pub fn apply_completed_conversion(&mut self, converted: &str) {
        if self.last_word.is_empty() {
            return;
        }
        self.last_word = converted.to_string();
        if let Some(l) = self.session_words.last_mut() {
            l.word = converted.to_string();
        }
    }

    pub fn commit_snippet(&mut self, expansion: &str, ws: &str, now: f64) {
        self.current_word = expansion.to_string();
        self.boundary(ws, now);
    }

    pub fn apply_conversion(&mut self, converted: &str) {
        if !self.current_word.is_empty() {
            self.current_word = converted.to_string();
        } else if !self.last_word.is_empty() {
            self.last_word = converted.to_string();
            if let Some(l) = self.session_words.last_mut() {
                l.word = converted.to_string();
            }
        }
    }

    fn breaks_line(t: &str) -> bool {
        t.contains('\n') || t.contains('\r')
    }

    /// Слово перед решаемым (как на экране); не пересекает Enter.
    pub fn context_word(&self, for_current: bool) -> Option<String> {
        let i = self.session_words.len() as isize - if for_current { 1 } else { 2 };
        if i < 0 {
            return None;
        }
        let w = &self.session_words[i as usize];
        if Self::breaks_line(&w.tail) {
            return None;
        }
        Some(w.word.clone())
    }

    pub fn earlier_context_word(&self, for_current: bool) -> Option<String> {
        let immediate = self.session_words.len() as isize - if for_current { 1 } else { 2 };
        let earlier = immediate - 1;
        if earlier < 0 {
            return None;
        }
        let (im, ea) = (
            &self.session_words[immediate as usize],
            &self.session_words[earlier as usize],
        );
        if Self::breaks_line(&im.tail) || Self::breaks_line(&ea.tail) {
            return None;
        }
        Some(ea.word.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_only_includes_started_next_word() {
        let mut b = KeystrokeBuffer::new();
        b.append("g", 0.0);
        b.append("h", 0.1);
        b.boundary(" ", 0.2);
        b.append("x", 0.3);
        let it = b.word_for_conversion(true).unwrap();
        assert_eq!(it.word, "gh");
        assert_eq!(it.delete_count, 4);
        assert_eq!(it.tail, " x");
    }

    #[test]
    fn backspace_reenters_completed_word() {
        let mut b = KeystrokeBuffer::new();
        for (i, c) in "ghbdtn".chars().enumerate() {
            b.append(&c.to_string(), i as f64 * 0.1);
        }
        b.boundary(" ", 1.0);
        b.backspace(1.1);
        b.backspace(1.2);
        assert_eq!(b.current_word, "ghbdt");
    }
}
