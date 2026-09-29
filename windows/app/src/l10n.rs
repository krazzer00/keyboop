//! Строки интерфейса (меню в трее, уведомления). Русский и английский, как у мак-версии.

use std::sync::atomic::{AtomicBool, Ordering};

static RU: AtomicBool = AtomicBool::new(true);

/// `language`: "ru" | "en" | "auto" (auto — по языку интерфейса системы).
pub fn set_language(language: &str, system_is_russian: bool) {
    let ru = match language {
        "ru" => true,
        "en" => false,
        _ => system_is_russian,
    };
    RU.store(ru, Ordering::Relaxed);
}

pub fn t(key: &str) -> &'static str {
    let ru = RU.load(Ordering::Relaxed);
    let (r, e) = match key {
        "rescued" => ("Расколдовано слов: ", "Words rescued: "),
        "auto" => ("Авто-переключение", "Auto-switch layout"),
        "live" => ("Чинить на лету", "Fix while typing"),
        "sound" => ("Звук", "Sound"),
        "typo" => ("Исправлять опечатки", "Fix typos"),
        "twocaps" => ("Исправлять ДВе заглавные", "Fix TWo capitals"),
        "dev" => (
            "Режим разработчика (не трогать IDE и терминалы)",
            "Developer mode (leave IDEs and terminals alone)",
        ),
        "group" => (
            "Хоткей переводит всю фразу",
            "Hotkey converts the whole phrase",
        ),
        "pause" => ("Пауза «не мешать»", "Pause"),
        "pause.15" => ("15 минут", "15 minutes"),
        "pause.60" => ("1 час", "1 hour"),
        "pause.180" => ("3 часа", "3 hours"),
        "pause.300" => ("5 часов", "5 hours"),
        "pause.stop" => ("Снять паузу", "Resume now"),
        "paused" => ("На паузе", "Paused"),
        "app" => ("Программа: ", "App: "),
        "app.normal" => ("Переключать как обычно", "Switch as usual"),
        "app.soft" => (
            "Мягко (только очевидные слова)",
            "Softly (only obvious words)",
        ),
        "app.off" => ("Не переключать", "Don't switch"),
        "app.layout.none" => ("Раскладку не трогать", "Don't force a layout"),
        "app.layout.en" => ("Всегда включать английскую", "Always switch to English"),
        "app.layout.ru" => ("Всегда включать русскую", "Always switch to Russian"),
        "learn" => ("Больше не переключать «", "Stop converting “"),
        "learn.end" => ("»", "”"),
        "learn.title" => (
            "Keyboop: вы уже трижды вернули это слово",
            "Keyboop: you reverted this word three times",
        ),
        "learn.body" => (
            "Нажмите, чтобы больше не переключать «",
            "Click to stop converting “",
        ),
        "open.settings" => ("Настройки (settings.json)…", "Settings (settings.json)…"),
        "open.exceptions" => (
            "Исключения (exceptions.json)…",
            "Exceptions (exceptions.json)…",
        ),
        "open.snippets" => ("Сниппеты (snippets.json)…", "Snippets (snippets.json)…"),
        "open.folder" => ("Папка данных и лог", "Data folder and log"),
        "reload" => ("Перечитать настройки", "Reload settings"),
        "autostart" => ("Запускать вместе с Windows", "Start with Windows"),
        "quit" => ("Выйти", "Quit"),
        "hotkeys" => ("Хоткеи: ", "Hotkeys: "),
        "already" => (
            "Keyboop уже запущен — значок в трее у часов.",
            "Keyboop is already running — see the tray icon.",
        ),
        "config.error" => (
            "Keyboop: файл настроек не прочитан",
            "Keyboop: settings file could not be read",
        ),
        "config.error.body" => (
            "Файл сохранён как есть, работаю на значениях по умолчанию. Подробности в keyboop.log.",
            "The file was left untouched; running with defaults. See keyboop.log for details.",
        ),
        "voice.listening" => ("Слушаю", "Listening"),
        "voice.processing" => ("Распознаю", "Transcribing"),
        "voice.escSaved" => (
            "Отменено — текст в истории",
            "Cancelled — text saved to history",
        ),
        "voice.cancelled" => ("Диктовка отменена", "Dictation cancelled"),
        "voice.noModel" => (
            "Скачайте модель распознавания в настройках",
            "Download a speech model in Settings",
        ),
        "voice.noMic" => ("Микрофон недоступен", "Microphone is unavailable"),
        "voice.silent" => (
            "Тишина — микрофон ничего не услышал",
            "Silence — the microphone heard nothing",
        ),
        "voice" => ("Голосовой набор", "Voice typing"),
        "open.ui" => ("Настройки…", "Settings…"),
        "open.history" => ("История…", "History…"),
        _ => ("?", "?"),
    };
    if ru {
        r
    } else {
        e
    }
}
