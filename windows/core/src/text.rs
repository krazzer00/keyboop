//! Мелкие свойства строк и символов, которые в Swift даёт `Character`/`String`.

/// Есть ли в строке кириллица (U+0400…U+04FF) — `String.hasCyrillic`.
pub fn has_cyrillic(s: &str) -> bool {
    s.chars().any(is_cyrillic)
}

/// Есть ли латинская буква ASCII — `String.hasLatinLetter`.
pub fn has_latin_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
}

pub fn is_cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

/// `Character.isLetter`.
pub fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

/// `Character.isNumber`.
pub fn is_number(c: char) -> bool {
    c.is_numeric()
}

pub fn is_ascii_digit(c: char) -> bool {
    c.is_ascii_digit()
}

/// `Character.isPunctuation` (категории P*). Покрываем ASCII и типографику, которая реально
/// встречается в набранном тексте.
pub fn is_punctuation(c: char) -> bool {
    if c.is_ascii() {
        return matches!(
            c,
            '!' | '"'
                | '#'
                | '%'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | ','
                | '-'
                | '.'
                | '/'
                | ':'
                | ';'
                | '?'
                | '@'
                | '['
                | '\\'
                | ']'
                | '_'
                | '{'
                | '}'
        );
    }
    matches!(
        c,
        '«' | '»'
            | '„'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '‚'
            | '‹'
            | '›'
            | '…'
            | '–'
            | '—'
            | '‐'
            | '‑'
            | '‒'
            | '¡'
            | '¿'
            | '§'
            | '¶'
            | '·'
            | '•'
            | '′'
            | '″'
            | '‟'
    )
}

pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

pub fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Первый символ в верхнем регистре, остальное как есть.
pub fn upper_first(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(f) => f.to_uppercase().collect::<String>() + it.as_str(),
        None => String::new(),
    }
}

pub fn is_upper(c: char) -> bool {
    c.is_uppercase()
}

pub fn is_lower(c: char) -> bool {
    c.is_lowercase()
}

/// Сколько пробельных «слов» (для счётчика спасённых слов при конверсии выделения).
pub fn word_count(s: &str) -> usize {
    s.split(' ').filter(|w| !w.is_empty()).count()
}
