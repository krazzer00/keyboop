//! Перевод выделенного (аналог `TranslationEngine.swift`). Реализация — этап 5.

pub fn apply(_sel: Option<&super::clipboard::Selection>) {
    super::app().log("перевод: ещё не перенесено");
}
