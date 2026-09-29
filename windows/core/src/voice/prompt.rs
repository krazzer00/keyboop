//! Перенос `WhisperPrompt.swift`: затравка для whisper и срез её эха.

/// Русская фраза-затравка: запятые, «ё», «!», «?», тире. Текст не менять без нового замера.
pub const RU: &str = "Привет! Как дела? Сегодня хорошая погода, но, кажется, скоро пойдёт дождь. Ну что ж, подождём — время ещё есть.";
pub const EN: &str = "Hi! How are you? The weather is nice today, but it looks like it might rain. Well, let's wait — there's still time.";

/// С какой уверенности определения языка «Авто» берёт затравку только на этом языке.
pub const CONFIDENT_LANGUAGE: f32 = 0.5;

/// Под какой язык собирать затравку: явный выбор человека, иначе уверенно определённый язык.
pub fn prompt_language(setting: &str, detected: Option<&str>, p: f32) -> String {
    if setting != "auto" {
        return setting.to_string();
    }
    match detected {
        Some(d) if p >= CONFIDENT_LANGUAGE => d.to_string(),
        _ => "auto".into(),
    }
}

/// Блоки затравки (начало, хвост); словарь диктовки встаёт между ними. None — без затравки.
pub fn blocks(language: &str) -> Option<(String, String)> {
    match language {
        "ru" => Some((RU.into(), String::new())),
        "en" => Some((EN.into(), String::new())),
        "auto" => Some((RU.into(), format!(" {EN}"))),
        _ => None,
    }
}

/// Полная затравка: фразы + подсказка словаря; если не влезает в 600 символов — без словаря.
pub fn punctuation_prompt(language: &str, dictionary_hint: Option<&str>) -> String {
    let Some((head, tail)) = blocks(language) else {
        return String::new();
    };
    let dict = match dictionary_hint {
        Some(w) if language == "en" => format!(" Words that often come up: {w}."),
        Some(w) => format!(" Часто встречаются слова: {w}."),
        None => String::new(),
    };
    let full = format!("{head}{dict}{tail}");
    if full.chars().count() <= PROMPT_MAX_CHARS {
        full
    } else {
        format!("{head}{tail}")
    }
}

const PROMPT_MAX_CHARS: usize = 600;

/// Куски затравки: блоки фраз целиком и остаток (словарь). Длинные первыми.
pub fn echo_pieces(prompt: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = prompt.to_string();
    for block in [RU, EN] {
        if rest.contains(block) {
            pieces.push(block.to_string());
            rest = rest.replace(block, " ");
        }
    }
    let remainder = rest.trim();
    if !remainder.is_empty() {
        pieces.push(remainder.to_string());
    }
    pieces.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
    pieces
}

/// Срезать эхо затравки с начала расшифровки (только целые куски). Возвращает текст и число срезов.
pub fn strip_echo(text: &str, prompt: &str) -> (String, usize) {
    let mut result = text.trim().to_string();
    if prompt.is_empty() {
        return (result, 0);
    }
    let pieces = echo_pieces(prompt);
    let mut cut = 0;
    while let Some(p) = pieces.iter().find(|p| result.starts_with(p.as_str())) {
        result = result[p.len()..].trim().to_string();
        cut += 1;
    }
    (result, cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_is_cut_only_whole() {
        let prompt = punctuation_prompt("ru", Some("Keyboop"));
        let text = format!("{RU} Часто встречаются слова: Keyboop. Добрый день");
        assert_eq!(strip_echo(&text, &prompt).0, "Добрый день");
        assert_eq!(
            strip_echo("Привет! Как дела?", &prompt).0,
            "Привет! Как дела?"
        );
    }

    #[test]
    fn language_choice() {
        assert_eq!(prompt_language("auto", Some("ru"), 0.9), "ru");
        assert_eq!(prompt_language("auto", Some("ru"), 0.2), "auto");
        assert_eq!(prompt_language("en", Some("ru"), 0.9), "en");
        assert!(blocks("es").is_none());
    }
}
