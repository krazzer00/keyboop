//! Перенос `WhisperGhosts.swift`: фразы-призраки Whisper (обрывки субтитров из обучающих данных).
//! Главное правило: удалить чужое слово хуже, чем пропустить призрак — срезаем только ЦЕЛЫЕ
//! предложения и только с краёв.

use regex::Regex;
use std::sync::OnceLock;

const PATTERNS: &[&str] = &[
    r"^субтитры (создал|создавал|сделал|делал|подготовил|подготовлены|предоставлены|by)\b.*$",
    r"^редактор субтитров\b.*$",
    r"^продолжение следует$",
    r"^спасибо за просмотр$",
    r"^спасибо за внимание$",
    r"^подписывайтесь на канал$",
    r"^подписывайся на канал$",
    r"^ставьте лайк$",
    r"^не забудьте подписаться$",
    r"^thanks? (you )?for watching$",
    r"^subtitles by\b.*$",
    r"^subtitles? and translation by\b.*$",
    r"^please subscribe$",
    r"^amara\.org\b.*$",
    r"^www\.[a-z0-9.-]+$",
];

fn regexes() -> &'static Vec<Regex> {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| {
        PATTERNS
            .iter()
            .filter_map(|p| Regex::new(&format!("(?i){p}")).ok())
            .collect()
    })
}

/// Убрать призраки. Может вернуть пустую строку — значит, речи не было вовсе.
pub fn clean(text: &str) -> String {
    let sentences = split(text);
    if sentences.is_empty() {
        return text.to_string();
    }
    let mut kept: Vec<String> = sentences.clone();
    while kept.first().is_some_and(|s| is_ghost(s)) {
        kept.remove(0);
    }
    while kept.last().is_some_and(|s| is_ghost(s)) {
        kept.pop();
    }
    kept.retain(|s| s.chars().any(|c| c.is_alphanumeric()));
    if kept.len() == sentences.len() {
        return text.to_string();
    }
    kept.join(" ").trim().to_string()
}

/// Предложения: граница — знак конца плюс пробел (иначе «А.Синецкая» порвалась бы).
fn split(text: &str) -> Vec<String> {
    static BOUNDARY: OnceLock<Regex> = OnceLock::new();
    let re = BOUNDARY.get_or_init(|| Regex::new(r"([.!?…]+)\s+").unwrap());
    text.lines()
        .flat_map(|line| {
            let marked = re.replace_all(line, "$1\u{1}").to_string();
            marked
                .split('\u{1}')
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_ghost(sentence: &str) -> bool {
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let edge: &[char] = &[
        ' ', '\t', '\n', '.', ',', '!', '?', '…', '-', '–', '—', ':', ';', '"', '\'', '«', '»',
        '(', ')',
    ];
    let s = sentence.trim_matches(edge);
    let s = SPACES
        .get_or_init(|| Regex::new(r"\s+").unwrap())
        .replace_all(s, " ");
    if s.is_empty() {
        return false;
    }
    regexes().iter().any(|r| r.is_match(&s))
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn ghosts_are_cut_only_at_edges() {
        assert_eq!(clean("Привет. Субтитры сделал DimaTorzok"), "Привет.");
        assert_eq!(clean("Продолжение следует..."), "");
        assert_eq!(
            clean("Субтитры к фильму мы сделаем сами"),
            "Субтитры к фильму мы сделаем сами"
        );
        assert_eq!(
            clean("Продолжение следует за вступлением, так устроена книга"),
            "Продолжение следует за вступлением, так устроена книга"
        );
        assert_eq!(
            clean("Спасибо за просмотр. Это важно. Спасибо за просмотр."),
            "Это важно."
        );
    }
}
