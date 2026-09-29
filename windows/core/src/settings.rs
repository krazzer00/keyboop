//! Перенос `AppSettings.swift`. Значения по умолчанию те же, что регистрирует мак-версия;
//! хранится в `%APPDATA%\Keyboop\settings.json`. Хоткеи — строки вида "Pause", "Ctrl+Shift+K",
//! "RCtrl" (тап/удержание одиночного модификатора); пустая строка — выключен.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    // ── Раскладка ──────────────────────────────────────────────────────────────
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
    /// Хоткей конвертирует все слова фразы (работает только при выключенном авто).
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
    /// Хоткей «перевести последнее слово / выделенное».
    pub hotkey_convert: String,
    /// Хоткей «только сменить раскладку» (как 🌐 на Маке), например "CapsLock".
    pub hotkey_switch_layout: String,
    /// Хоткей «сменить регистр выделенного».
    pub hotkey_case: String,
    /// Лампочка Caps Lock показывает язык: горит — русский.
    pub caps_led_indicator: bool,

    // ── Голосовой набор ────────────────────────────────────────────────────────
    pub voice_enabled: bool,
    /// Хоткей диктовки. По умолчанию правый Alt (аналог правого ⌥ на Маке).
    pub hotkey_voice: String,
    /// "hold" — говорить, пока держишь; "toggle" — нажал, говоришь, нажал ещё раз.
    pub voice_hold_mode: String,
    /// "whisper" | "parakeet".
    pub voice_engine: String,
    /// Модель Whisper: tiny | base | small | medium | large-v3-turbo.
    pub voice_model: String,
    /// "auto" | "ru" | "en".
    pub voice_language: String,
    /// Имя микрофона; пусто — системный по умолчанию.
    pub voice_mic: String,
    pub voice_sound_enabled: bool,
    pub voice_sound_volume: f64,
    pub voice_no_capital: bool,
    pub voice_no_final_period: bool,
    pub voice_no_em_dash: bool,
    pub voice_trailing_space: bool,
    /// После вставки нажать Enter (отправить в чате).
    pub voice_auto_enter: bool,
    /// Приглушать звук системы на время диктовки.
    pub voice_duck: bool,
    /// До скольких процентов приглушать (0…77).
    pub voice_duck_level: u32,
    /// Поднимать уровень микрофона на время диктовки.
    pub voice_mic_gain: bool,
    pub voice_mic_gain_level: u32,
    /// Сохранять аудио диктовок рядом с текстом в истории.
    pub voice_save_audio: bool,
    pub voice_history_enabled: bool,
    /// Сколько минут хранить историю диктовок.
    pub voice_history_minutes: u32,
    pub clipboard_history_enabled: bool,
    /// Esc во время диктовки отменяет её.
    pub esc_cancels_dictation: bool,
    /// Отменённую диктовку всё равно распознать и положить в историю.
    pub esc_save_to_history: bool,
    /// Выгружать модель сразу после диктовки (экономия памяти).
    pub voice_unload_after_dictation: bool,
    /// Через сколько минут простоя выгружать модель (0 — никогда).
    pub voice_model_idle_minutes: u32,
    /// Плашка диктовки у верхнего края экрана, а не у курсора.
    pub voice_hud_top: bool,
    /// Вставить последнюю диктовку ещё раз.
    pub hotkey_paste_dictation: String,

    // ── Буфер обмена и сниппеты ────────────────────────────────────────────────
    /// Вставка без форматирования.
    pub plain_paste: bool,
    pub hotkey_plain_paste: String,
    /// Выбор текстового сниппета по цифре.
    pub hotkey_snippet_pick: String,

    // ── Перевод ────────────────────────────────────────────────────────────────
    pub translate_enabled: bool,
    pub hotkey_translate: String,
    pub translate_sound_enabled: bool,

    // ── Прочее ─────────────────────────────────────────────────────────────────
    /// Пауза «не мешать» до этого момента (секунды Unix).
    pub paused_until: f64,
    /// Язык интерфейса: "auto" | "ru" | "en".
    pub language: String,
    /// Оформление окон: "system" | "light" | "dark".
    pub app_theme: String,
    /// Что делает щелчок по значку в трее: "menu" | "settings" | "history" | "dictate" | "pause".
    pub tray_click: String,
    pub silent_auto_update: bool,
    pub beta_channel: bool,
    pub did_show_welcome: bool,
    pub last_run_version: String,
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
            group_convert: true,
            two_caps_fix: false,
            typo_fix: false,
            learn_on_undo: true,
            snippet_expand_space: true,
            snippet_expand_enter: true,
            snippet_expand_tab: true,
            hotkey_convert: "Pause".into(),
            hotkey_switch_layout: String::new(),
            hotkey_case: "Alt+Pause".into(),
            caps_led_indicator: false,

            voice_enabled: true,
            hotkey_voice: "RAlt".into(),
            voice_hold_mode: "hold".into(),
            voice_engine: "whisper".into(),
            voice_model: "small".into(),
            voice_language: "auto".into(),
            voice_mic: String::new(),
            voice_sound_enabled: true,
            voice_sound_volume: 0.6,
            voice_no_capital: false,
            voice_no_final_period: false,
            voice_no_em_dash: false,
            voice_trailing_space: true,
            voice_auto_enter: false,
            voice_duck: false,
            voice_duck_level: 20,
            voice_mic_gain: true,
            voice_mic_gain_level: 90,
            voice_save_audio: false,
            voice_history_enabled: true,
            voice_history_minutes: 60,
            clipboard_history_enabled: false,
            esc_cancels_dictation: true,
            esc_save_to_history: true,
            voice_unload_after_dictation: false,
            voice_model_idle_minutes: 60,
            voice_hud_top: false,
            hotkey_paste_dictation: String::new(),

            plain_paste: false,
            hotkey_plain_paste: "Ctrl+Shift+V".into(),
            hotkey_snippet_pick: String::new(),

            translate_enabled: true,
            hotkey_translate: "Ctrl+Alt+T".into(),
            translate_sound_enabled: true,

            paused_until: 0.0,
            language: "auto".into(),
            app_theme: "system".into(),
            tray_click: "menu".into(),
            silent_auto_update: false,
            beta_channel: false,
            did_show_welcome: false,
            last_run_version: String::new(),
        }
    }
}

impl Settings {
    pub fn is_paused(&self, wall_now: f64) -> bool {
        self.paused_until > wall_now
    }

    pub fn voice_output(&self) -> crate::voice::OutputOptions {
        crate::voice::OutputOptions {
            no_final_period: self.voice_no_final_period,
            no_capital: self.voice_no_capital,
            no_em_dash: self.voice_no_em_dash,
        }
    }
}
