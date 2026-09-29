//! Перенос нужной Windows-версии части `AppSettings.swift`. Значения по умолчанию те же, что
//! регистрирует мак-версия; хранится в `%APPDATA%\Keyboop\settings.json`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Авто-переключение на границе слова — флагманская функция, включена.
    pub auto_enabled: bool,
    /// Чинить на лету, посреди слова.
    pub live_fix_enabled: bool,
    pub sound_enabled: bool,
    /// Громкость звука переключения, 0…1.
    pub sound_volume: f64,
    /// Какие клавиши завершают слово для авто-переключения.
    pub trigger_space: bool,
    pub trigger_enter: bool,
    pub trigger_tab: bool,
    /// Чинить слово ДО того, как Enter уйдёт в приложение (чаты отправляют мгновенно).
    pub enter_pre_convert: bool,
    /// Стрелки сбрасывают контекст набора.
    pub arrows_cancel: bool,
    /// Режим разработчика: в IDE и терминалах авто молчит целиком.
    pub developer_mode: bool,
    /// Хоткей конвертирует все слова фразы (только при выключенном авто).
    pub group_convert: bool,
    /// «КОгда» → «Когда».
    pub two_caps_fix: bool,
    /// Исправление опечаток и следа Caps Lock.
    pub typo_fix: bool,
    /// Обучение на отмене.
    pub learn_on_undo: bool,
    /// Какими клавишами раскрываются сниппеты.
    pub snippet_expand_space: bool,
    pub snippet_expand_enter: bool,
    pub snippet_expand_tab: bool,
    /// Хоткей «перевести последнее слово / выделенное». Формат: "Pause", "Ctrl+Shift+K",
    /// одиночный модификатор-тап: "RCtrl", "RShift", "LAlt"... Пусто — выключен.
    pub hotkey_convert: String,
    /// Хоткей «только сменить раскладку» (как 🌐 на Маке). Например "CapsLock". Пусто — выключен.
    pub hotkey_switch_layout: String,
    /// Хоткей «сменить регистр выделенного». Пусто — выключен.
    pub hotkey_case: String,
    /// Пауза «не мешать» до этого момента (секунды Unix).
    pub paused_until: f64,
    /// Язык интерфейса: "auto" | "ru" | "en".
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_enabled: true,
            live_fix_enabled: true,
            sound_enabled: true,
            sound_volume: 0.6,
            trigger_space: true,
            trigger_enter: true,
            trigger_tab: false,
            enter_pre_convert: true,
            arrows_cancel: true,
            developer_mode: false,
            group_convert: false,
            two_caps_fix: false,
            typo_fix: false,
            learn_on_undo: true,
            snippet_expand_space: true,
            snippet_expand_enter: true,
            snippet_expand_tab: true,
            hotkey_convert: "Pause".into(),
            hotkey_switch_layout: String::new(),
            hotkey_case: "Alt+Pause".into(),
            paused_until: 0.0,
            language: "auto".into(),
        }
    }
}

impl Settings {
    pub fn is_paused(&self, wall_now: f64) -> bool {
        self.paused_until > wall_now
    }
}
