//! Перенос `Keymap.swift` + табличной части `DynamicKeymap.swift`.
//!
//! Статическая таблица — пары клавиш «U.S.» ↔ «Русская» (ЙЦУКЕН), запаска на первые секунды.
//! Живая таблица строится платформой из реально установленных раскладок (на Windows — через
//! `ToUnicodeEx`, на Маке — через `UCKeyTranslate`) и кладётся сюда [`set_live_tables`].

use crate::text::{is_ascii_digit, is_letter};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

pub type Table = HashMap<char, char>;

const BASE_PAIRS: &[(char, char)] = &[
    ('`', 'ё'),
    ('q', 'й'),
    ('w', 'ц'),
    ('e', 'у'),
    ('r', 'к'),
    ('t', 'е'),
    ('y', 'н'),
    ('u', 'г'),
    ('i', 'ш'),
    ('o', 'щ'),
    ('p', 'з'),
    ('[', 'х'),
    (']', 'ъ'),
    ('a', 'ф'),
    ('s', 'ы'),
    ('d', 'в'),
    ('f', 'а'),
    ('g', 'п'),
    ('h', 'р'),
    ('j', 'о'),
    ('k', 'л'),
    ('l', 'д'),
    (';', 'ж'),
    ('\'', 'э'),
    ('z', 'я'),
    ('x', 'ч'),
    ('c', 'с'),
    ('v', 'м'),
    ('b', 'и'),
    ('n', 'т'),
    ('m', 'ь'),
    (',', 'б'),
    ('.', 'ю'),
    ('/', '.'),
];

fn upper(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

/// Статическая EN→RU (буквы получают и верхний регистр, знаки — нет, как в Swift).
pub fn static_en_to_ru() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let mut d = Table::new();
        for &(e, r) in BASE_PAIRS {
            d.insert(e, r);
            if e.is_alphabetic() {
                d.insert(upper(e), upper(r));
            }
        }
        d
    })
}

/// Статическая RU→EN. При повторе ключа побеждает последняя пара, как у Swift-словаря.
pub fn static_ru_to_en() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let mut d = Table::new();
        for &(e, r) in BASE_PAIRS {
            d.insert(r, e);
            if e.is_alphabetic() {
                d.insert(upper(r), upper(e));
            }
        }
        d
    })
}

pub struct LiveTables {
    pub en_to_ru: Table,
    pub ru_to_en: Table,
}

fn live_slot() -> &'static RwLock<Option<Arc<LiveTables>>> {
    static L: OnceLock<RwLock<Option<Arc<LiveTables>>>> = OnceLock::new();
    L.get_or_init(|| RwLock::new(None))
}

/// Положить живые таблицы, снятые с раскладок системы. Типографские двойники кавычек
/// добавляются здесь же (`DynamicKeymap.addTypographicAliases`, задача 168).
pub fn set_live_tables(mut en_to_ru: Table, ru_to_en: Table) {
    if en_to_ru.is_empty() {
        return;
    }
    add_typographic_aliases(&mut en_to_ru);
    *live_slot().write().unwrap() = Some(Arc::new(LiveTables { en_to_ru, ru_to_en }));
}

pub fn clear_live_tables() {
    *live_slot().write().unwrap() = None;
}

pub fn live() -> Option<Arc<LiveTables>> {
    live_slot().read().unwrap().clone()
}

pub fn is_live_ready() -> bool {
    live_slot().read().unwrap().is_some()
}

fn add_typographic_aliases(map: &mut Table) {
    let twins: [(char, &[char]); 2] = [
        ('\'', &['\u{2019}', '\u{2018}', '\u{00B4}', '\u{2032}']),
        ('"', &['\u{201C}', '\u{201D}', '\u{201F}']),
    ];
    for (straight, curly) in twins {
        let Some(&target) = map.get(&straight) else {
            continue;
        };
        for &c in curly {
            map.entry(c).or_insert(target);
        }
    }
}

/// Типографский двойник → прямой знак (для статической таблицы).
pub fn straighten(ch: char) -> char {
    match ch {
        '\u{2019}' | '\u{2018}' | '\u{00B4}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{201F}' => '"',
        _ => ch,
    }
}

/// Концевая пунктуация: одинакова в обеих раскладках, в конце слова её не конвертируем.
pub const TRAILING_PUNCTUATION: &[char] = &['.', ',', '!', '?', ';', ':', '…'];

pub fn is_trailing_punct(c: char) -> bool {
    TRAILING_PUNCTUATION.contains(&c)
}

/// Конвертирует строку по живой таблице, если она есть, иначе по статической.
pub fn convert(text: &str, to_cyrillic: bool) -> String {
    let live = live();
    let table = live.as_ref().map(|t| {
        if to_cyrillic {
            &t.en_to_ru
        } else {
            &t.ru_to_en
        }
    });
    convert_with(text, to_cyrillic, table)
}

/// Чистое ядро конверсии (`table = None` — статическая запаска). Знак между цифрами
/// не отправляем в таблицу вовсе (задача 263): см. [`number_separator`].
pub fn convert_with(text: &str, to_cyrillic: bool, table: Option<&Table>) -> String {
    let fallback = if to_cyrillic {
        static_en_to_ru()
    } else {
        static_ru_to_en()
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut run_has_separator = false;
    for i in 0..chars.len() {
        let ch = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        if (ch == ',' || ch == '.')
            && prev.is_some_and(is_ascii_digit)
            && next.is_some_and(is_ascii_digit)
        {
            out.push(number_separator(
                ch,
                to_cyrillic,
                run_has_separator,
                &chars[i + 2..],
            ));
            run_has_separator = true;
        } else {
            match table {
                Some(t) => out.push(*t.get(&ch).unwrap_or(&ch)),
                None => {
                    let m = fallback.get(&ch).or_else(|| fallback.get(&straighten(ch)));
                    out.push(*m.unwrap_or(&ch));
                }
            }
            if !is_ascii_digit(ch) {
                run_has_separator = false;
            }
        }
    }
    out
}

/// Запятая/точка между цифрами: в английскую единственная запятая становится точкой,
/// всё остальное остаётся как набрано. `rest` начинается сразу за цифрой после знака.
fn number_separator(ch: char, to_cyrillic: bool, left_has_separator: bool, rest: &[char]) -> char {
    if to_cyrillic || ch == '.' || left_has_separator {
        return ch;
    }
    let mut i = 0;
    while i < rest.len() {
        let c = rest[i];
        if is_ascii_digit(c) {
            i += 1;
            continue;
        }
        if (c == ',' || c == '.') && rest.get(i + 1).is_some_and(|&a| is_ascii_digit(a)) {
            return ch;
        }
        break;
    }
    '.'
}

/// Направление для ручной конверсии, когда букв нет вообще (задача 268). Только по живой таблице.
pub fn unambiguous_symbol_direction(text: &str) -> Option<bool> {
    let live = live()?;
    symbol_direction(text, &live.en_to_ru, &live.ru_to_en)
}

pub fn symbol_direction(text: &str, en_to_ru: &Table, ru_to_en: &Table) -> Option<bool> {
    let (mut to_cyr, mut to_lat) = (false, false);
    for ch in text.chars() {
        if is_letter(ch) {
            return None;
        }
        if ch.is_whitespace() || ch.is_numeric() {
            continue;
        }
        let c = straighten(ch);
        let en = en_to_ru.contains_key(&c);
        let ru = ru_to_en.contains_key(&c);
        if en && !ru {
            to_cyr = true;
        } else if ru && !en {
            to_lat = true;
        }
    }
    if to_cyr == to_lat {
        None
    } else {
        Some(to_cyr)
    }
}

/// Умная конвертация: концевую пунктуацию оставляем, ядро конвертируем; но если полное слово
/// (с `,` `.` `;` как буквами б/ю/ж) валидно по словарю — конвертируем целиком.
pub fn smart_convert(
    word: &str,
    to_cyrillic: bool,
    is_valid_target: Option<&dyn Fn(&str) -> bool>,
) -> String {
    let live = live();
    let table = live.as_ref().map(|t| {
        if to_cyrillic {
            &t.en_to_ru
        } else {
            &t.ru_to_en
        }
    });
    smart_convert_with(word, to_cyrillic, is_valid_target, table)
}

pub fn smart_convert_with(
    word: &str,
    to_cyrillic: bool,
    is_valid_target: Option<&dyn Fn(&str) -> bool>,
    table: Option<&Table>,
) -> String {
    if to_cyrillic {
        if let Some(valid) = is_valid_target {
            let full = convert_with(word, true, table);
            if valid(&full.to_lowercase()) {
                return full;
            }
        }
    }
    let mut core: Vec<char> = word.chars().collect();
    let mut trailing: Vec<char> = Vec::new();
    while let Some(&last) = core.last() {
        if !is_trailing_punct(last) {
            break;
        }
        trailing.insert(0, peeled_mark(last, to_cyrillic, table));
        core.pop();
    }
    if core.is_empty() {
        return word.to_string();
    }
    let core: String = core.into_iter().collect();
    convert_with(&core, to_cyrillic, table) + &trailing.into_iter().collect::<String>()
}

/// Срезанный концевой знак: меняем, только если таблица переводит его в другой концевой знак
/// (задача 263, «Русская — ПК»: «Lf?» → «Да,»). Только в сторону кириллицы.
fn peeled_mark(p: char, to_cyrillic: bool, table: Option<&Table>) -> char {
    if !to_cyrillic {
        return p;
    }
    let mapped: Vec<char> = convert_with(&p.to_string(), true, table).chars().collect();
    if mapped.len() == 1 && mapped[0] != p && is_trailing_punct(mapped[0]) {
        mapped[0]
    } else {
        p
    }
}

/// Конверсия для правки на лету: `None` — «ждём следующую клавишу» (задача 262).
pub fn live_convert(
    word: &str,
    to_cyrillic: bool,
    is_valid_target: Option<&dyn Fn(&str) -> bool>,
) -> Option<String> {
    let live = live();
    let table = live.as_ref().map(|t| {
        if to_cyrillic {
            &t.en_to_ru
        } else {
            &t.ru_to_en
        }
    });
    live_convert_with(word, to_cyrillic, is_valid_target, table)
}

pub fn live_convert_with(
    word: &str,
    to_cyrillic: bool,
    is_valid_target: Option<&dyn Fn(&str) -> bool>,
    table: Option<&Table>,
) -> Option<String> {
    let smart = smart_convert_with(word, to_cyrillic, is_valid_target, table);
    if smart == word {
        return Some(smart);
    }
    let full = convert_with(word, to_cyrillic, table);
    if smart == full {
        return Some(smart);
    }
    for (s, f) in smart.chars().zip(full.chars()) {
        if s != f && is_letter(f) {
            return None;
        }
    }
    Some(smart)
}

/// Ядро слова без концевой пунктуации.
pub fn core_of(word: &str) -> String {
    let mut s: Vec<char> = word.chars().collect();
    while s.last().is_some_and(|&c| is_trailing_punct(c)) {
        s.pop();
    }
    s.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_conversion() {
        assert_eq!(convert_with("ghbdtn", true, None), "привет");
        assert_eq!(convert_with("Ghbdtn", true, None), "Привет");
        assert_eq!(convert_with("руддщ", false, None), "hello");
    }

    #[test]
    fn number_separators() {
        assert_eq!(convert_with("5,5", false, None), "5.5");
        assert_eq!(convert_with("1,000,000", false, None), "1,000,000");
        assert_eq!(convert_with("17.2", true, None), "17.2");
    }

    #[test]
    fn smart_keeps_trailing_punctuation() {
        assert_eq!(smart_convert_with("ghbdtn.", true, None, None), "привет.");
        let valid = |w: &str| w == "нож";
        assert_eq!(smart_convert_with("yj;", true, Some(&valid), None), "нож");
    }

    #[test]
    fn live_waits_for_letter_like_mark() {
        assert_eq!(live_convert_with("gthtrk.", true, None, None), None);
        assert_eq!(
            live_convert_with("gthtrk.x", true, None, None).as_deref(),
            Some("переключ")
        );
    }
}
