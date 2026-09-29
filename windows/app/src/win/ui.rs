//! Запуск окон (настройки, история, приветствие, отзыв) — отдельный процесс, см. `crate::ui`.

pub fn open(section: &str) {
    crate::ui::spawn("settings", Some(section));
}
