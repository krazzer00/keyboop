//! Перенос `VoiceDictionary.swift`: словарь диктовки «как слышится → как пишется».
//! Точные замены (с начала слова, пробел в образце = любой разрыв, включая дефис) и нечёткие
//! по расстоянию Левенштейна с предохранителем: живое слово языка не трогаем никогда.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictPair {
    pub heard: String,
    pub written: String,
}

struct Needle {
    pattern: Vec<char>,
    replacement: String,
}

struct FuzzyNeedle {
    pattern: Vec<char>,
    words: usize,
    tol: usize,
    replacement: String,
}

pub struct VoiceDictionary {
    pairs: Vec<(String, String)>,
    needles: Vec<Needle>,
    fuzzy: Vec<FuzzyNeedle>,
    full_fuzzy: Vec<FuzzyNeedle>,
    max_fuzzy_words: usize,
    case_keepers: HashSet<String>,
    /// Слово языка (RU/EN словари) — нечёткая замена его не трогает. None — данные не готовы,
    /// нечёткий поиск выключен.
    language_guard: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
}

pub fn fold(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}

/// Заготовки словаря (засеваются один раз): то, что ломается у всех.
pub fn seed_pairs() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![
        ("кейбуп".into(), "Keyboop".into()),
        ("к-буп".into(), "Keyboop".into()),
        ("вайп".into(), "вайб".into()),
    ];
    let mut seen = HashSet::new();
    let claude = [
        "клауд",
        "клод",
        "клоуд",
        "клауде",
        "клаус",
        "клауд",
        "cloud",
        "claud",
        "clode",
    ];
    let code = ["код", "коуд", "кот", "code"];
    for c in claude {
        for k in code {
            let s = format!("{c} {k}");
            if seen.insert(s.clone()) {
                out.push((s, "Claude Code".into()));
            }
        }
        for k in code {
            let s = format!("{c}{k}");
            if seen.insert(s.clone()) {
                out.push((s, "Claude Code".into()));
            }
        }
    }
    let chat = ["чат", "чад", "чет", "chat"];
    let gpt = [
        "гпт",
        "жпт",
        "гбт",
        "жбт",
        "джипити",
        "джепити",
        "джи пи ти",
        "gpt",
        "gbt",
    ];
    let mut seen = HashSet::new();
    for c in chat {
        for g in gpt {
            for s in [format!("{c} {g}"), format!("{c}{g}")] {
                if seen.insert(s.clone()) {
                    out.push((s, "ChatGPT".into()));
                }
            }
        }
    }
    for h in [
        "юс ди ти",
        "юс дити",
        "юс ди т",
        "юс д ти",
        "юс д т",
        "юс дт",
        "юс ти",
        "юсди ти",
        "юсдити",
        "юсдт",
        "юсд ти",
        "юсд т",
        "юсд",
        "юис ди ти",
        "юис дити",
        "юис дт",
        "юис д",
        "юисдт",
        "юисд",
        "усд ти",
        "усдт",
        "усд",
        "ус ди ти",
        "ус дт",
        "ю эс ди ти",
        "ю эс дт",
        "у эс ди ти",
        "юз ди ти",
        "юзди ти",
        "юздт",
    ] {
        out.push((h.into(), "USDT".into()));
    }
    for h in [
        "впн",
        "впен",
        "впэн",
        "впиэн",
        "вэпэн",
        "випиэн",
        "vpn",
        "ви пи эн",
        "вэ пэ эн",
        "в пэ эн",
        "ви пи ен",
        "ви пиэн",
    ] {
        out.push((h.into(), "VPN".into()));
    }
    for h in Z_CODE {
        out.push((h.to_string(), "ZCode".into()));
    }
    let ai: [(&[&str], &str); 4] = [
        (
            &[
                "сидэнс",
                "сиданс",
                "сиденс",
                "сидинс",
                "сидэнц",
                "seedance",
                "seadance",
                "си дэнс",
                "си данс",
                "sea dance",
                "си dance",
            ],
            "Seedance",
        ),
        (
            &[
                "сидрим",
                "сидрем",
                "сидрым",
                "seedream",
                "seadream",
                "си дрим",
                "си дрем",
                "sea dream",
                "си dream",
            ],
            "Seedream",
        ),
        (&["клинг", "клингг", "клин г", "klink", "клинк г"], "Kling"),
        (
            &[
                "хигсфилд",
                "хиггсфилд",
                "хигсвилд",
                "хиггсвилд",
                "хиксфилд",
                "хигзфилд",
                "хигсфилт",
                "хиггсфилт",
                "higgsfield",
                "хигс филд",
                "хиггс филд",
                "хигс вилд",
            ],
            "Higgsfield",
        ),
    ];
    for (heard, written) in ai {
        for h in heard {
            out.push((h.to_string(), written.into()));
        }
    }
    out
}

const Z_CODE: [&str; 4] = ["z код", "зет код", "z code", "зет code"];

pub fn fuzzy_tolerance(n: usize) -> usize {
    if n < 5 {
        0
    } else if n <= 8 {
        1
    } else {
        2
    }
}

/// Левенштейн с отсечкой: больше `limit` — возвращаем `limit + 1`.
pub fn edit_distance(a: &[char], b: &[char], limit: usize) -> usize {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > limit {
        return limit + 1;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = i;
        let lo = 1.max(i.saturating_sub(limit));
        let hi = m.min(i + limit);
        if lo > 1 {
            cur[lo - 1] = limit + 1;
        }
        for j in lo..=hi {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(cur[j]);
        }
        if hi < m {
            cur[hi + 1] = limit + 1;
        }
        if row_min > limit {
            return limit + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

fn is_break(c: char) -> bool {
    c.is_whitespace() || c == '-' || c == '–'
}

fn word_count(p: &[char]) -> usize {
    p.split(|c| *c == ' ')
        .filter(|w| !w.is_empty())
        .count()
        .max(1)
}

impl VoiceDictionary {
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        let mut d = VoiceDictionary {
            pairs: Vec::new(),
            needles: Vec::new(),
            fuzzy: Vec::new(),
            full_fuzzy: Vec::new(),
            max_fuzzy_words: 0,
            case_keepers: HashSet::new(),
            language_guard: None,
        };
        d.set_all(pairs);
        d
    }

    /// Включить нечёткий поиск: `is_word` — «это слово русского или английского словаря».
    pub fn set_language_guard(&mut self, is_word: Box<dyn Fn(&str) -> bool + Send + Sync>) {
        self.language_guard = Some(is_word);
    }

    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    pub fn set_all(&mut self, pairs: Vec<(String, String)>) {
        let mut out: Vec<(String, String)> = Vec::new();
        for (h, w) in pairs {
            let heard = h.trim().to_string();
            let written = w.trim().to_string();
            if heard.is_empty() {
                continue;
            }
            let c = fold(&heard);
            if let Some(p) = out.iter_mut().find(|p| fold(&p.0) == c) {
                p.1 = written;
            } else {
                out.push((heard, written));
            }
        }
        self.pairs = out;
        self.rebuild_index();
    }

    /// Дополнить список новыми заготовками (которых ещё нет), не трогая пользовательские.
    pub fn merge_seed(&mut self) -> usize {
        if self.pairs.is_empty() {
            return 0;
        }
        let have: HashSet<String> = self.pairs.iter().map(|p| fold(&p.0)).collect();
        let add: Vec<(String, String)> = seed_pairs()
            .into_iter()
            .filter(|p| !have.contains(&fold(&p.0)))
            .collect();
        let n = add.len();
        if n > 0 {
            let mut all = self.pairs.clone();
            all.extend(add);
            self.set_all(all);
        }
        n
    }

    fn rebuild_index(&mut self) {
        let exact_only: HashSet<String> = Z_CODE.iter().map(|s| fold(s)).collect();
        let indexed: Vec<(Vec<char>, String, usize)> = self
            .pairs
            .iter()
            .enumerate()
            .filter_map(|(i, (h, w))| {
                let f = fold(h.trim());
                (!f.is_empty() && !w.is_empty()).then(|| (f.chars().collect(), w.clone(), i))
            })
            .collect();
        let mut sorted = indexed.clone();
        sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.2.cmp(&b.2)));
        self.needles = sorted
            .iter()
            .map(|(p, r, _)| Needle {
                pattern: p.clone(),
                replacement: r.clone(),
            })
            .collect();

        let fuzzy_indexed: Vec<&(Vec<char>, String, usize)> = indexed
            .iter()
            .filter(|n| !exact_only.contains(&n.0.iter().collect::<String>()))
            .collect();
        self.full_fuzzy = fuzzy_indexed
            .iter()
            .filter_map(|(p, r, _)| {
                let tol = fuzzy_tolerance(p.len());
                (tol > 0).then(|| FuzzyNeedle {
                    pattern: p.clone(),
                    words: word_count(p),
                    tol,
                    replacement: r.clone(),
                })
            })
            .collect();

        // Опоры для быстрого предфильтра (сжатый нечёткий индекс, см. Swift).
        let mut groups: HashMap<String, Vec<&(Vec<char>, String, usize)>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for n in &fuzzy_indexed {
            if !groups.contains_key(&n.1) {
                order.push(n.1.clone());
            }
            groups.entry(n.1.clone()).or_default().push(n);
        }
        let mut compact: Vec<(Vec<char>, usize, usize, String, usize)> = Vec::new();
        for replacement in order {
            let mut candidates = groups.remove(&replacement).unwrap_or_default();
            candidates.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.2.cmp(&b.2)));
            let mut anchors: Vec<&(Vec<char>, String, usize)> = Vec::new();
            for c in candidates {
                let covered = anchors.iter().any(|a| {
                    if word_count(&a.0) != word_count(&c.0) {
                        return false;
                    }
                    let tol = fuzzy_tolerance(a.0.len());
                    tol > 0 && edit_distance(&a.0, &c.0, tol) <= tol
                });
                if !covered {
                    anchors.push(c);
                }
            }
            for a in anchors {
                let tol = fuzzy_tolerance(a.0.len());
                if tol > 0 {
                    compact.push((a.0.clone(), word_count(&a.0), tol, a.1.clone(), a.2));
                }
            }
        }
        compact.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.4.cmp(&b.4)));
        self.fuzzy = compact
            .into_iter()
            .map(|(pattern, words, tol, replacement, _)| FuzzyNeedle {
                pattern,
                words,
                tol,
                replacement,
            })
            .collect();
        self.max_fuzzy_words = self.fuzzy.iter().map(|n| n.words).max().unwrap_or(0);
        self.case_keepers = self
            .pairs
            .iter()
            .filter_map(|(_, w)| {
                let first = w.split(' ').next()?;
                first
                    .chars()
                    .any(char::is_uppercase)
                    .then(|| first.to_string())
            })
            .collect();
    }

    /// Слово с намеренным регистром из словаря («Keyboop») — «не начинать с заглавной» его не трогает.
    pub fn keeps_case(&self, word: &str) -> bool {
        self.case_keepers.contains(word)
    }

    /// Подсказка распознаванию: написания из словаря, сперва имена собственные.
    pub fn recognition_hint(&self, max_chars: usize) -> Option<String> {
        let mut seen = HashSet::new();
        let words: Vec<&String> = self
            .pairs
            .iter()
            .map(|p| &p.1)
            .filter(|w| !w.is_empty() && seen.insert(w.as_str()))
            .collect();
        let named = words.iter().filter(|w| w.chars().any(char::is_uppercase));
        let plain = words.iter().filter(|w| !w.chars().any(char::is_uppercase));
        let mut out: Vec<&str> = Vec::new();
        let mut used = 0;
        for w in named.chain(plain) {
            let cost = w.chars().count() + 2;
            if used + cost > max_chars {
                continue;
            }
            out.push(w);
            used += cost;
        }
        (!out.is_empty()).then(|| out.join(", "))
    }

    /// Применить словарь к распознанному тексту.
    pub fn apply(&self, text: &str) -> String {
        if self.needles.is_empty() || text.is_empty() {
            return text.to_string();
        }
        let src: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len() + 16);
        let mut i = 0;
        while i < src.len() {
            let at_word_start = i == 0 || !is_word_char(src[i - 1]);
            if at_word_start {
                if let Some((end, rep)) = self.exact_match(&src, i) {
                    out.push_str(&cased(rep, &src, i));
                    i = end;
                    continue;
                }
                if let Some((end, rep)) = self.fuzzy_match(&src, i) {
                    out.push_str(&cased(&rep, &src, i));
                    i = end;
                    continue;
                }
            }
            out.push(src[i]);
            i += 1;
        }
        out
    }

    fn exact_match(&self, src: &[char], from: usize) -> Option<(usize, &str)> {
        'needles: for n in &self.needles {
            let (mut pi, mut si) = (0, from);
            while pi < n.pattern.len() {
                let pc = n.pattern[pi];
                if pc == ' ' {
                    if si >= src.len() || !is_break(src[si]) {
                        continue 'needles;
                    }
                    while si < src.len() && is_break(src[si]) {
                        si += 1;
                    }
                    pi += 1;
                    continue;
                }
                if si >= src.len() || fold(&src[si].to_string()) != pc.to_string() {
                    continue 'needles;
                }
                si += 1;
                pi += 1;
            }
            return Some((si, &n.replacement));
        }
        None
    }

    fn all_words_of_language(&self, text: &[char]) -> bool {
        let Some(guard) = &self.language_guard else {
            return true;
        };
        let parts: Vec<String> = text
            .split(|c| *c == ' ')
            .filter(|p| !p.is_empty())
            .map(|p| p.iter().collect())
            .collect();
        !parts.is_empty() && parts.iter().all(|p| guard(p))
    }

    fn words_ahead(&self, src: &[char], from: usize, max: usize) -> Vec<(Vec<char>, usize)> {
        let mut out = Vec::new();
        let mut acc: Vec<char> = Vec::new();
        let mut si = from;
        while out.len() < max {
            let mut w: Vec<char> = Vec::new();
            while si < src.len() && is_word_char(src[si]) {
                w.extend(fold(&src[si].to_string()).chars());
                si += 1;
            }
            if w.is_empty() {
                break;
            }
            if !acc.is_empty() {
                acc.push(' ');
            }
            acc.extend(w);
            out.push((acc.clone(), si));
            let mut bi = si;
            while bi < src.len() && is_break(src[bi]) {
                bi += 1;
            }
            if bi == si {
                break;
            }
            si = bi;
        }
        out
    }

    fn fuzzy_match(&self, src: &[char], from: usize) -> Option<(usize, String)> {
        if self.language_guard.is_none() || self.fuzzy.is_empty() {
            return None;
        }
        let ahead = self.words_ahead(src, from, self.max_fuzzy_words);
        if ahead.is_empty() {
            return None;
        }
        let possible = self.fuzzy.iter().any(|n| {
            if n.words > ahead.len() {
                return false;
            }
            let cand = &ahead[n.words - 1].0;
            if self.all_words_of_language(cand) {
                return false;
            }
            let pre = n.tol + 2;
            cand.len().abs_diff(n.pattern.len()) <= pre
                && edit_distance(&n.pattern, cand, pre) <= pre
        });
        if !possible {
            return None;
        }
        let mut best: Option<(usize, usize, usize, &str)> = None;
        let mut ambiguous = false;
        for n in &self.full_fuzzy {
            if n.words > ahead.len() {
                continue;
            }
            let (cand, end) = &ahead[n.words - 1];
            if cand.len().abs_diff(n.pattern.len()) > n.tol || self.all_words_of_language(cand) {
                continue;
            }
            let d = edit_distance(&n.pattern, cand, n.tol);
            if d > n.tol {
                continue;
            }
            match best {
                None => {
                    best = Some((d, n.pattern.len(), *end, &n.replacement));
                    ambiguous = false;
                }
                Some((bd, bl, _, br)) => {
                    if d < bd || (d == bd && n.pattern.len() > bl) {
                        best = Some((d, n.pattern.len(), *end, &n.replacement));
                        ambiguous = false;
                    } else if d == bd && n.pattern.len() == bl && n.replacement != br {
                        ambiguous = true;
                    }
                }
            }
        }
        match best {
            Some((_, _, end, r)) if !ambiguous => Some((end, r.to_string())),
            _ => None,
        }
    }
}

/// Замена без заглавных, а в тексте слово с заглавной (начало фразы) — поднимаем первую букву.
fn cased(replacement: &str, src: &[char], i: usize) -> String {
    if replacement.chars().any(char::is_uppercase) || !src[i].is_uppercase() {
        return replacement.to_string();
    }
    crate::text::upper_first(replacement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> VoiceDictionary {
        let mut d = VoiceDictionary::new(seed_pairs());
        d.set_language_guard(Box::new(|w| {
            ["сидение", "когда", "код", "сиденье"].contains(&w)
        }));
        d
    }

    #[test]
    fn exact_and_breaks() {
        let d = dict();
        assert_eq!(d.apply("открой клауд код"), "открой Claude Code");
        assert_eq!(d.apply("открой клауд-код"), "открой Claude Code");
        assert_eq!(d.apply("Вайп хороший"), "Вайб хороший");
        assert_eq!(d.apply("кейбупом"), "Keyboopом");
    }

    #[test]
    fn fuzzy_but_not_real_words() {
        let d = dict();
        assert_eq!(d.apply("сидэнз"), "Seedance");
        assert_eq!(d.apply("сидение"), "сидение");
    }

    #[test]
    fn hint_and_case() {
        let d = dict();
        assert!(d.recognition_hint(180).unwrap().starts_with("Keyboop"));
        assert!(d.keeps_case("Keyboop"));
        assert!(!d.keeps_case("вайб"));
    }

    #[test]
    fn levenshtein() {
        let a: Vec<char> = "kitten".chars().collect();
        let b: Vec<char> = "sitting".chars().collect();
        assert_eq!(edit_distance(&a, &b, 5), 3);
        assert_eq!(edit_distance(&a, &b, 1), 2);
    }
}
