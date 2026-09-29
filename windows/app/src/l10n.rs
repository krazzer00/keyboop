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

pub fn is_russian() -> bool {
    RU.load(Ordering::Relaxed)
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
        "snip.pickTip" => (
            "цифра вставит · Esc закроет",
            "a digit inserts · Esc closes",
        ),
        "snip.pickTipMore" => (
            "цифра · Shift+цифра · Ctrl+цифра вставят, дальше мышью · Esc закроет",
            "digit · Shift+digit · Ctrl+digit insert, the rest by mouse · Esc closes",
        ),
        "snip.pickEmpty" => ("Список сниппетов пуст", "The snippet list is empty"),
        "snip.pickDictation" => ("Диктовка", "Dictation"),
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
        "translate.processing" => ("Перевожу", "Translating"),
        "translate.noPack" => (
            "Скачайте пакет перевода в настройках",
            "Download the translation pack in Settings",
        ),
        "translate.failed" => ("Не удалось перевести", "Translation failed"),
        "translate.nothing" => ("Выделите текст для перевода", "Select text to translate"),
        "import.saved" => (
            "Файл расшифрован и добавлен в историю",
            "The file is transcribed and added to History",
        ),
        "import.cancelled" => ("Импорт отменён", "Import cancelled"),
        "import.failed" => ("Не удалось прочитать файл", "Could not read the file"),
        "import.busy" => ("Уже идёт расшифровка", "A transcription is already running"),
        "call.saved" => (
            "Запись расшифрована и добавлена в историю",
            "The recording is transcribed and added to History",
        ),
        "call.empty" => (
            "В записи не нашлось речи, ничего не сохранено",
            "No speech in the recording, nothing saved",
        ),
        "call.started" => (
            "Запись звонка началась. Остановить: Shift+щелчок по значку",
            "Call recording started. To stop: Shift+click the icon",
        ),
        "call.tip" => (
            "● Идёт запись звонка — Shift+щелчок остановит",
            "● Recording a call — Shift+click to stop",
        ),
        "call.stop" => ("Остановить запись звонка", "Stop recording the call"),
        "call.failed" => ("Запись не началась", "Recording did not start"),
        "call.recovering" => (
            "Восстанавливаю незавершённую запись",
            "Recovering an unfinished recording",
        ),
        "call.noSystemAudioTitle" => ("Записывается только микрофон", "Only the microphone is recorded"),
        "call.noSystemAudioBody" => (
            "Звук собеседников (то, что играет в колонках или наушниках) записать не получилось, пишу только ваш голос.",
            "Could not capture the other side (what plays in your speakers or headphones), recording only your voice.",
        ),
        "call.stalledTitle" => ("Запись прервалась", "Recording interrupted"),
        "call.stalledBody" => (
            "Микрофон перестал присылать звук, и три перезапуска не помогли. Записанное сохраняю.",
            "The microphone stopped sending audio and three restarts did not help. Saving what was recorded.",
        ),
        "call.silenceTitle" => ("Пять минут тишины", "Five minutes of silence"),
        "call.silenceBody" => (
            "Звук, похоже, закончился. Нажмите на это уведомление, чтобы остановить запись. Через две минуты остановлю сам.",
            "The sound seems to be over. Click this notification to stop recording. I will stop on my own in two minutes.",
        ),
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
        "open.feedback" => ("Написать разработчику…", "Send feedback…"),
        "files" => ("Файлы настроек", "Settings files"),
        _ => ("?", "?"),
    };
    if ru {
        r
    } else {
        e
    }
}
