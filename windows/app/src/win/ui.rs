//! Окна (настройки, история, приветствие, отзыв) — отдельный процесс `keyboop.exe --ui <окно>`.
//! Этап 2: сами окна; здесь только запуск.

pub fn open(section: &str) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .args(["--ui", "settings", "--section", section])
            .spawn();
    }
}
