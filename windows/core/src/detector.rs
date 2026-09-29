//! Перенос `LayoutDetector.swift` — двусторонний детектор «слово набрано не в той раскладке».
//! Каскад: guard'ы → force-swap → словарь → триграммы. Порядок шагов и все пороги совпадают
//! с оригиналом; истории каждого правила — в комментариях Swift-файла.

use crate::exceptions::Exceptions;
use crate::extra_words::sets;
use crate::keymap::{self, is_trailing_punct, static_en_to_ru, static_ru_to_en};
use crate::layout_data::{self, LayoutData};
use crate::text::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapDecision {
    Keep,
    Convert { to_cyrillic: bool },
}

use SwapDecision::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextHint {
    None,
    Cyrillic,
    Latin,
}

impl ContextHint {
    pub fn of(word: Option<&str>) -> Self {
        match word {
            Some(w) if !w.is_empty() => {
                if has_cyrillic(w) {
                    ContextHint::Cyrillic
                } else if has_latin_letter(w) {
                    ContextHint::Latin
                } else {
                    ContextHint::None
                }
            }
            _ => ContextHint::None,
        }
    }
}

pub const MARGIN: f64 = 2.0;
pub const SHORT_EN_SWAP_FLOOR: f64 = -10.0;
pub const LIVE_IMPOSSIBLE: f64 = -13.0;
pub const LIVE_MARGIN: f64 = 6.0;

/// Аббревиатуры без гласных — статистика триграмм их не вытягивает.
pub const FORCE_SWAP: &[&str] = &[
    "http", "https", "url", "uri", "api", "rest", "json", "xml", "yaml", "csv", "html", "css",
    "sdk", "cli", "gui", "ide", "ssh", "ftp", "tcp", "udp", "ip", "dns", "vpn", "ssl", "tls",
    "smtp", "jwt", "cors", "sql", "nosql", "git", "npm", "yarn", "k8s", "aws", "gcp", "kpi", "crm",
    "seo", "smm", "mvp", "cpu", "gpu", "ram", "ssd", "hdd", "usb", "pdf", "mp3", "mp4", "png",
    "jpg", "jpeg", "svg", "gif", "ddos", "iot", "llm", "gpt", "ml", "ai", "ui", "ux", "db", "os",
    "io", "qa", "ci", "cd", "webp", "mvc", "orm", "cdn", "dom",
];

fn is_force_swap(w: &str) -> bool {
    FORCE_SWAP.contains(&w)
}

pub const COMMON_EN_TWO_LETTER: &[&str] = &[
    "am", "an", "as", "at", "be", "by", "do", "go", "he", "hi", "if", "in", "is", "it", "me", "my",
    "no", "of", "ok", "on", "or", "so", "to", "up", "us", "we", "id", "ai", "ui", "ux",
];

pub const RU_SINGLE_LETTER: &[&str] = &["а", "в", "и", "к", "о", "с", "у", "я"];

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn string(v: &[char]) -> String {
    v.iter().collect()
}

/// Символ — буква, или его клавиша в другой раскладке даёт букву (х=[, ж=;, э=', ё=`, ъ=]).
pub fn is_layout_letter(c: char) -> bool {
    if is_letter(c) {
        return true;
    }
    if static_en_to_ru().get(&c).is_some_and(|m| is_letter(*m)) {
        return true;
    }
    static_ru_to_en().get(&c).is_some_and(|m| is_letter(*m))
}

/// Схлопнуть растянутые буквы (от трёх подряд): два варианта — до двух и до одной.
pub fn de_elongated(s: &str) -> Option<Vec<String>> {
    let mut run = 0;
    let mut prev: Option<char> = None;
    let mut has = false;
    let (mut to_two, mut to_one) = (String::new(), String::new());
    for c in s.chars() {
        if Some(c) == prev {
            run += 1;
        } else {
            run = 1;
            prev = Some(c);
        }
        if run >= 3 {
            has = true;
        }
        if run <= 2 {
            to_two.push(c);
        }
        if run == 1 {
            to_one.push(c);
        }
    }
    if !has {
        return None;
    }
    Some(if to_two == to_one {
        vec![to_one]
    } else {
        vec![to_two, to_one]
    })
}

/// Настоящее слово перед концевой пунктуацией (`it.` ≠ `шею`).
pub fn has_valid_source_before_trailing_punctuation(raw_core: &str, forced: &Exceptions) -> bool {
    let whole = raw_core.to_lowercase();
    let semantic = keymap::core_of(&whole);
    if semantic == whole || char_count(&semantic) < 2 {
        return false;
    }
    let cyr = has_cyrillic(&semantic);
    let lat = has_latin_letter(&semantic);
    if cyr == lat {
        return false;
    }
    if is_force_swap(&semantic) || forced.is_forced(&semantic) {
        return true;
    }
    let s = sets();
    if lat && s.tech_latin_tokens.contains(semantic.as_str()) {
        return true;
    }
    if lat && s.force_ru_amb.contains(semantic.as_str()) {
        return false;
    }
    let d = layout_data::shared();
    let words = if cyr { &d.words_ru } else { &d.words_en };
    if words.contains(&semantic) {
        return true;
    }
    de_elongated(&semantic).is_some_and(|v| v.iter().any(|x| words.contains(x)))
}

/// Смайлик западного типа (`:D`, `;P`, `XD`) — никогда не слово.
pub fn is_emoticon(raw: &str) -> bool {
    let mut s = chars(raw);
    while s.last().is_some_and(|c| ".,!?…".contains(*c)) {
        s.pop();
    }
    if s.is_empty() {
        return false;
    }
    let eyes = s.remove(0);
    if s.len() > 1 && "-~^'".contains(s[0]) {
        s.remove(0);
    }
    if s.len() != 1 {
        return false;
    }
    let mouth = s[0];
    let mouth_symbols = ")(|/\\*$@[]{}3";
    match eyes {
        ':' | '=' => is_letter(mouth) || mouth_symbols.contains(mouth),
        ';' | '8' => (is_letter(mouth) && is_upper(mouth)) || mouth_symbols.contains(mouth),
        'X' | 'x' => mouth == 'D' || mouth == 'd',
        _ => false,
    }
}

/// Кириллический токен переводится в `swapped` ровно по ЙЦУКЕН.
pub fn is_jcuken_swap(w: &str, swapped: &str) -> bool {
    let t = static_ru_to_en();
    let mut out = String::new();
    for ch in w.chars() {
        match t.get(&ch) {
            Some(m) => out.push(*m),
            None => return false,
        }
    }
    out.to_lowercase() == swapped
}

pub fn is_tech_token_from_cyrillic(w: &str, swapped: &str) -> bool {
    sets().tech_from_cyrillic.contains(swapped) && is_jcuken_swap(w, swapped)
}

/// Числовой токен слева («5 г», «100 гб»).
pub fn is_numeric_token(s: Option<&str>) -> bool {
    let Some(s) = s else { return false };
    if s.is_empty() {
        return false;
    }
    let mut digit = false;
    for ch in s.chars() {
        if is_number(ch) {
            digit = true;
            continue;
        }
        if ",.-–/".contains(ch) {
            continue;
        }
        return false;
    }
    digit
}

const RU_VOWELS: &[char] = &['а', 'е', 'ё', 'и', 'о', 'у', 'ы', 'э', 'ю', 'я'];

/// Кириллица, невозможная в русской орфографии (ЬФСИЩЩЛ = MACBOOK).
pub fn is_impossible_russian_spelling(w: &str) -> bool {
    let c = chars(&w.to_lowercase());
    if c.len() < 2 {
        return false;
    }
    if c.iter().all(|x| *x == c[0]) {
        return false;
    }
    if c[0] == 'ь' || c[0] == 'ы' {
        return true;
    }
    for i in 0..c.len() - 1 {
        let (a, b) = (c[i], c[i + 1]);
        if a == b && "ьыйщ".contains(a) {
            return true;
        }
        if RU_VOWELS.contains(&a) && (b == 'ь' || b == 'ы') {
            return true;
        }
    }
    false
}

/// Буквенное ядро: срезаем ведущие/концевые не-буквы (кроме цифр).
pub fn letter_core(raw: &str) -> String {
    let c = chars(raw);
    let keep = |x: char| is_layout_letter(x) || is_number(x);
    let Some(first) = c.iter().position(|&x| keep(x)) else {
        return String::new();
    };
    let last = c.iter().rposition(|&x| keep(x)).unwrap();
    string(&c[first..=last])
}

fn trim_digits(s: &str) -> String {
    let c = chars(s);
    let first = c.iter().position(|x| !is_number(*x));
    match first {
        None => String::new(),
        Some(f) => {
            let l = c.iter().rposition(|x| !is_number(*x)).unwrap();
            string(&c[f..=l])
        }
    }
}

/// Форма слова через ASCII-дефис (`HyphenTokenShape.linguisticCore`).
pub fn hyphen_linguistic_core(raw: &str) -> Option<String> {
    let c = chars(raw);
    let ok = |x: char| is_layout_letter(x) || is_number(x);
    let first = c.iter().position(|&x| ok(x))?;
    let last = c.iter().rposition(|&x| ok(x))?;
    if c[..first].contains(&'-') || c[last + 1..].contains(&'-') {
        return None;
    }
    let mut seg = 0usize;
    let mut segments = 0;
    for &ch in &c[first..=last] {
        if ch == '-' {
            if seg == 0 {
                return None;
            }
            seg = 0;
            segments += 1;
        } else {
            if !is_layout_letter(ch) {
                return None;
            }
            seg += 1;
        }
    }
    if segments < 1 || seg == 0 {
        return None;
    }
    Some(string(&c[first..=last]))
}

/// Решение для режима «чинить на лету» (мид-слово): строже обычного.
pub fn live_decide(raw: &str, exc: &Exceptions) -> SwapDecision {
    let lc = letter_core(raw);
    let typed = keymap::core_of(&lc).to_lowercase();
    let core = trim_digits(&lc);
    let w = core.to_lowercase();
    if typed != w && is_exception_or_prefix(&typed, has_cyrillic(&typed), exc) {
        return Keep;
    }
    if has_valid_source_before_trailing_punctuation(&core, exc) {
        return Keep;
    }
    if char_count(&w) < 4 || !w.chars().all(is_layout_letter) {
        return Keep;
    }
    let (src_cyr, src_lat) = (has_cyrillic(&w), has_latin_letter(&w));
    if src_cyr == src_lat {
        return Keep;
    }
    let to_cyr = !src_cyr;
    let swapped = keymap::convert(&core, to_cyr).to_lowercase();
    if swapped == w || !swapped.chars().all(is_letter) {
        return Keep;
    }
    let d = layout_data::shared();
    if src_lat && d.words_en.contains(&w) {
        return Keep;
    }
    if !src_lat && d.words_ru.contains(&w) {
        return Keep;
    }
    if is_exception_or_prefix(&w, src_cyr, exc) {
        return Keep;
    }
    let orig = d.plausibility(&w, src_cyr);
    let swap = d.plausibility(&swapped, to_cyr);
    if orig <= LIVE_IMPOSSIBLE && swap > orig + LIVE_MARGIN {
        return Convert {
            to_cyrillic: to_cyr,
        };
    }
    Keep
}

/// `w` — слово-исключение или его префикс (только для live-fix).
pub fn is_exception_or_prefix(w: &str, cyrillic: bool, exc: &Exceptions) -> bool {
    let n = char_count(w);
    if n < 2 {
        return false;
    }
    fn hit<'a>(mut it: impl Iterator<Item = &'a str>, w: &str, n: usize) -> bool {
        it.any(|x| x == w || (char_count(x) > n && x.starts_with(w)))
    }
    if hit(exc.learned.iter().map(String::as_str), w, n)
        || hit(exc.ignored.iter().map(String::as_str), w, n)
        || hit(sets().default_keep.iter().copied(), w, n)
    {
        return true;
    }
    let s = sets();
    if cyrillic {
        hit(s.ru.iter().copied(), w, n)
            || hit(s.ru_abbr.iter().copied(), w, n)
            || hit(s.ru_short.iter().copied(), w, n)
    } else {
        hit(s.en.iter().copied(), w, n) || hit(s.en_keep_short.iter().copied(), w, n)
    }
}

/// Спасение смешанного слова (кир+лат): ровно одна сторона даёт словарное слово.
pub fn mixed_rescue(raw: &str) -> SwapDecision {
    if !(has_cyrillic(raw) && has_latin_letter(raw)) {
        return Keep;
    }
    let core = letter_core(raw);
    if char_count(&core) < 2 || !core.chars().all(is_layout_letter) {
        return Keep;
    }
    let to_ru = keymap::convert(&core, true);
    let to_en = keymap::convert(&core, false);
    let d = layout_data::shared();
    let ru_ok = !has_latin_letter(&to_ru) && d.words_ru.contains(&to_ru.to_lowercase());
    let en_ok = !has_cyrillic(&to_en) && d.words_en.contains(&to_en.to_lowercase());
    match (ru_ok, en_ok) {
        (true, false) => Convert { to_cyrillic: true },
        (false, true) => Convert { to_cyrillic: false },
        _ => Keep,
    }
}

fn russian_nj_after_latin_label(
    word: &str,
    raw_core: &str,
    prev: Option<&str>,
    earlier: Option<&str>,
) -> bool {
    if word != "nj" || raw_core != word {
        return false;
    }
    let (Some(prev), Some(earlier)) = (prev, earlier) else {
        return false;
    };
    if !prev.ends_with(',') || !has_cyrillic(earlier) || has_latin_letter(earlier) {
        return false;
    }
    let label: Vec<char> = prev.chars().collect();
    let label = &label[..label.len() - 1];
    if label.is_empty() || !label.iter().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    label.iter().filter(|c| c.is_uppercase()).count() >= 2
}

/// Решение для авто-режима (граница слова). Ручной хоткей сюда не заходит.
pub fn decide(
    raw: &str,
    exc: &Exceptions,
    prev: Option<&str>,
    earlier: Option<&str>,
    after_caret_jump: bool,
) -> SwapDecision {
    let context = ContextHint::of(prev);
    let lcore = letter_core(raw);
    let literal = lcore.to_lowercase();
    let typed = keymap::core_of(&lcore).to_lowercase();
    let whole = raw.to_lowercase();
    for k in [&literal, &typed, &whole] {
        if exc.is_ignored(k) || exc.is_learned(k) {
            return Keep;
        }
    }
    if is_emoticon(raw) {
        return Keep;
    }
    let had_digits = lcore.chars().any(is_number);
    let core_raw = if had_digits {
        trim_digits(&lcore)
    } else {
        lcore.clone()
    };
    let w = core_raw.to_lowercase();
    let wn = char_count(&w);
    let s = sets();
    let d: &LayoutData = layout_data::shared();

    // Дефисные слова: разбор по частям.
    if w.contains('-') {
        let Some(hyphen_raw) = hyphen_linguistic_core(raw) else {
            return Keep;
        };
        let hyphen_word = hyphen_raw.to_lowercase();
        if hyphen_word != w {
            return Keep;
        }
        if has_cyrillic(&hyphen_word) != has_latin_letter(&hyphen_word) {
            let to_cyr = !has_cyrillic(&hyphen_word);
            let swapped = keymap::convert(&hyphen_raw, to_cyr).to_lowercase();
            if s.hyphen_terms.contains(hyphen_word.as_str())
                || s.ru_hyphen_terms.contains(hyphen_word.as_str())
            {
                return Keep;
            }
            if s.hyphen_terms.contains(swapped.as_str())
                || s.ru_hyphen_terms.contains(swapped.as_str())
            {
                return Convert {
                    to_cyrillic: to_cyr,
                };
            }
            let src: Vec<&str> = hyphen_word.split('-').collect();
            let dst: Vec<&str> = swapped.split('-').collect();
            if src.len() == dst.len()
                && src.iter().all(|x| char_count(x) >= 2)
                && dst
                    .iter()
                    .all(|x| char_count(x) >= 2 && x.chars().all(is_letter))
            {
                let latin = has_latin_letter(&hyphen_word);
                let (dict_src, dict_dst) = if latin {
                    (&d.words_en, &d.words_ru)
                } else {
                    (&d.words_ru, &d.words_en)
                };
                if src.iter().all(|x| dict_src.contains(*x)) {
                    return Keep;
                }
                if dst.iter().all(|x| dict_dst.contains(*x)) {
                    return Convert {
                        to_cyrillic: to_cyr,
                    };
                }
            }
        }
        return Keep;
    }

    if wn < if had_digits { 4 } else { 1 } || !w.chars().all(is_layout_letter) {
        return Keep;
    }
    if exc.is_ignored(&w) || exc.is_learned(&w) || s.default_keep.contains(w.as_str()) {
        return Keep;
    }
    let src_cyr = has_cyrillic(&w);
    let src_lat = has_latin_letter(&w);
    if src_cyr == src_lat {
        return Keep;
    }
    let to_cyr = !src_cyr;
    let swapped = keymap::convert(&core_raw, to_cyr).to_lowercase();

    // Запятая/точка, набранные в русской раскладке, приезжают буквами («лунищщзб»).
    if src_cyr && !d.words_ru.contains(&w) && swapped.chars().last().is_some_and(is_trailing_punct)
    {
        let mut trimmed = chars(&core_raw);
        while let Some(&l) = trimmed.last() {
            let mapped = keymap::convert(&l.to_string(), false);
            if mapped.chars().last().is_some_and(is_trailing_punct) {
                trimmed.pop();
            } else {
                break;
            }
        }
        if trimmed.is_empty() {
            return Keep;
        }
        return decide(&string(&trimmed), exc, prev, earlier, after_caret_jump);
    }
    if swapped == w || !swapped.chars().all(|c| is_letter(c) || c == '\'') {
        return Keep;
    }

    let source_is_real_word = has_valid_source_before_trailing_punctuation(&core_raw, exc)
        || d.words_ru.contains(&w)
        || d.words_en.contains(&w)
        || de_elongated(&w).is_some_and(|v| {
            v.iter()
                .any(|x| d.words_ru.contains(x) || d.words_en.contains(x))
        });

    // 1. Force-swap: встроенные аббревиатуры — только из каши.
    if is_force_swap(&swapped) && !source_is_real_word {
        return Convert {
            to_cyrillic: to_cyr,
        };
    }
    if src_cyr
        && !source_is_real_word
        && !after_caret_jump
        && is_tech_token_from_cyrillic(&w, &swapped)
    {
        return Convert { to_cyrillic: false };
    }
    if exc.is_forced(&swapped) {
        return Convert {
            to_cyrillic: to_cyr,
        };
    }
    if is_force_swap(&w) || exc.is_forced(&w) {
        return Keep;
    }
    if src_lat && s.tech_latin_tokens.contains(w.as_str()) {
        return Keep;
    }

    // ★ Strict-gate: слово, валидное в языке набора, не переключаем никогда.
    if wn >= 2 && source_is_real_word && !(src_lat && s.force_ru_amb.contains(w.as_str())) {
        return Keep;
    }

    // Заглавная посреди фразы — имя собственное («по Уфе»).
    let cr = chars(&core_raw);
    if prev.is_some()
        && cr.first().is_some_and(|c| is_upper(*c))
        && cr[1..].iter().any(|c| is_lower(*c))
    {
        return Keep;
    }

    // 2. Одиночные буквы.
    if wn == 1 {
        if after_caret_jump && context == ContextHint::None {
            return Keep;
        }
        if src_lat && RU_SINGLE_LETTER.contains(&swapped.as_str()) {
            if let Some(p) = prev.map(|p| p.to_lowercase()) {
                if context == ContextHint::Latin && s.label_classifiers.contains(p.as_str()) {
                    return Keep;
                }
                if context == ContextHint::Cyrillic && s.ru_label_classifiers.contains(p.as_str()) {
                    return Keep;
                }
            }
            return Convert { to_cyrillic: true };
        }
        if src_cyr
            && context != ContextHint::Cyrillic
            && ["i", "u", "a"].contains(&swapped.as_str())
        {
            if is_numeric_token(prev) {
                return Keep;
            }
            return Convert { to_cyrillic: false };
        }
        return Keep;
    }

    let en_swap_not_junk = || -> bool {
        if wn == 2 {
            return COMMON_EN_TWO_LETTER.contains(&swapped.as_str());
        }
        wn >= 4 || d.plausibility(&swapped, false) > SHORT_EN_SWAP_FLOOR
    };

    // 3. Словарь (+ контекст фразы для коллизий).
    if src_lat {
        if d.words_en.contains(&w) && !s.force_ru_amb.contains(w.as_str()) {
            if context == ContextHint::Cyrillic
                && d.words_ru.contains(&swapped)
                && !s.en_keep_short.contains(w.as_str())
            {
                return Convert { to_cyrillic: true };
            }
            return Keep;
        }
        if d.words_ru.contains(&swapped) {
            if context == ContextHint::Latin
                && s.en_keep_short.contains(w.as_str())
                && !russian_nj_after_latin_label(&w, &core_raw, prev, earlier)
            {
                return Keep;
            }
            return Convert { to_cyrillic: true };
        }
    } else {
        if s.force_en_amb.contains(swapped.as_str()) && !d.words_ru.contains(&w) {
            return Convert { to_cyrillic: false };
        }
        if d.words_ru.contains(&w) {
            if context == ContextHint::Latin && d.words_en.contains(&swapped) {
                return Convert { to_cyrillic: false };
            }
            return Keep;
        }
        if d.words_en.contains(&swapped) && en_swap_not_junk() {
            return Convert { to_cyrillic: false };
        }
    }

    // 4. Триграммы — только от 4 букв.
    if wn < 4 {
        return Keep;
    }
    // Русская аббревиатура заглавными остаётся как набрана (задача 258).
    if src_cyr
        && !cr.iter().any(|c| is_lower(*c))
        && cr.iter().filter(|c| is_letter(**c)).count() >= 2
        && !is_impossible_russian_spelling(&w)
    {
        return Keep;
    }
    let orig = d.plausibility(&w, src_cyr);
    let swap = d.plausibility(&swapped, to_cyr);
    if swap > orig + MARGIN {
        return Convert {
            to_cyrillic: to_cyr,
        };
    }
    if orig <= -19.0 && swap > orig + 1.0 {
        return Convert {
            to_cyrillic: to_cyr,
        };
    }
    Keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(w: &str) -> SwapDecision {
        decide(w, &Exceptions::seeded(), None, None, false)
    }

    fn dec_after(w: &str, prev: &str) -> SwapDecision {
        decide(w, &Exceptions::seeded(), Some(prev), None, false)
    }

    const RU: SwapDecision = Convert { to_cyrillic: true };
    const EN: SwapDecision = Convert { to_cyrillic: false };

    #[test]
    fn gibberish_converts() {
        assert_eq!(dec("ghbdtn"), RU);
        assert_eq!(dec("Ghbdtn"), RU);
        assert_eq!(dec("руддщ"), EN);
        assert_eq!(dec("ghbdtn."), RU);
        assert_eq!(dec("(tckb"), RU);
    }

    #[test]
    fn real_words_kept() {
        for w in [
            "hello",
            "привет",
            "her",
            "here",
            "iOS",
            "sql",
            "api",
            ":D",
            "вк",
            "ok",
        ] {
            assert_eq!(dec(w), Keep, "{w}");
        }
    }

    #[test]
    fn abbreviations_forced() {
        assert_eq!(dec("фзш"), EN); // api
        assert_eq!(dec("ыйд"), EN); // sql
    }

    #[test]
    fn single_letters() {
        assert_eq!(dec("d"), RU);
        assert_eq!(dec_after("d", "vitamin"), Keep);
        assert_eq!(dec("г"), EN);
        assert_eq!(dec_after("г", "5"), Keep);
        assert_eq!(decide("d", &Exceptions::seeded(), None, None, true), Keep);
    }

    #[test]
    fn context_resolves_collisions() {
        assert_eq!(dec_after("yt", "привет"), RU);
        // Strict-gate стоит выше контекста: «шт» — настоящее русское сокращение, его не трогаем.
        assert_eq!(dec_after("шт", "hello"), Keep);
    }

    #[test]
    fn hyphen_words() {
        assert_eq!(dec("xnj-nj"), RU);
        assert_eq!(dec("dry-run"), Keep);
        assert_eq!(dec("--xnj-nj"), Keep);
    }

    #[test]
    fn russian_abbreviation_in_caps_kept() {
        assert_eq!(dec("УФНС"), Keep);
        assert_eq!(dec("ЬФСИЩЩЛ"), EN);
    }

    #[test]
    fn name_mid_sentence_kept() {
        assert_eq!(dec_after("Уфе", "по"), Keep);
    }

    #[test]
    fn russian_comma_typed_as_letter() {
        assert_eq!(dec("лунищщзб"), EN);
    }

    #[test]
    fn elongated_words_kept() {
        assert_eq!(dec("круууто"), Keep);
    }

    #[test]
    fn mixed_rescue_works() {
        assert_eq!(mixed_rescue("привtn"), RU);
        assert_eq!(mixed_rescue("helloмир"), Keep);
    }

    #[test]
    fn live_decide_is_strict() {
        let e = Exceptions::seeded();
        assert_eq!(live_decide("ghbd", &e), RU);
        assert_eq!(live_decide("hell", &e), Keep);
        assert_eq!(live_decide("it.", &e), Keep);
    }
}
