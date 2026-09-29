//! Перенос `Engine.swift`: обработка нажатий, авто-переключение на границе слова, правка на
//! лету, Enter-pre, сниппеты, ручной хоткей, правки текста (две заглавные, Caps Lock, опечатки).
//!
//! Всё системное (печать синтетикой, смена раскладки, звук) спрятано за [`Platform`], поэтому
//! движок одинаков для любой ОС и целиком проверяется тестами на эмуляторе текстового поля.
//!
//! Что намеренно НЕ перенесено с Мака и почему:
//! * `muted`/Fence B — на Windows ту же гонку закрывает платформа: реальные нажатия, пришедшие
//!   раньше нашей синтетики, она перехватывает и переигрывает после неё (см. `app/src/hook.rs`).
//! * AX-проба каретки и фантомный предохранитель — на Windows нет общего аналога Accessibility
//!   для чтения текста у каретки; работает ветка «Accessibility не ответил» оригинала.
//! * Точка по двойному пробелу — системной такой функции в Windows нет, хвост не переписываем.
//! * Сверка «мнение против реальности» — раскладку платформа читает у активного окна на
//!   каждом нажатии, отдельное мнение не копится.

use crate::buffer::{ConversionItem, KeystrokeBuffer};
use crate::detector::{self, SwapDecision};
use crate::exceptions::{self, Exceptions};
use crate::keymap;
use crate::layout_data;
use crate::resonance::AntiResonanceGuard;
use crate::settings::Settings;
use crate::snippets::{self, SnippetStore};
use crate::text::*;
use crate::typo::{Guard, TypoFix};
use crate::undo::UndoLearner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Cyrillic,
    Latin,
    Other,
}

/// Всё, что движок просит у операционной системы.
pub trait Platform {
    /// Монотонное время, секунды.
    fn now(&self) -> f64;
    /// Время Unix, секунды (пауза, обучение на отмене).
    fn wall(&self) -> f64;
    /// Стереть `delete` символов перед кареткой и напечатать `text`; `then_return` — затем Enter.
    fn replace(&mut self, delete: usize, text: &str, then_return: bool);
    /// Включить латинскую или кириллическую раскладку (ту, которой человек пользуется).
    fn select_layout(&mut self, cyrillic: bool);
    /// Следующая раскладка по кругу. false — крутить нечего.
    fn cycle_layout(&mut self) -> bool;
    /// Письменность текущей раскладки активного окна.
    fn current_script(&self) -> Script;
    fn play_sound(&mut self);
    fn beep(&mut self);
    /// Раскладка могла смениться — обновить индикатор.
    fn layout_changed(&mut self);
    /// Диагностика. Содержимое набранного сюда не пишем никогда (приватность).
    fn log(&mut self, msg: &str);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// Печатный символ (уже декодированный раскладкой активного окна).
    Char(String),
    Backspace,
    /// Ctrl+Backspace: стирает слово целиком, сколько — мы не знаем.
    DeleteWord,
    Space,
    Tab,
    Enter,
    Arrow,
    /// Esc, Delete, Home/End, PageUp/PageDown.
    Nav,
    /// Сочетание с Ctrl/Alt/Win — не текст.
    Shortcut,
    /// Всё остальное (F-клавиши и т. п.): буфер не трогаем.
    Other,
}

#[derive(Clone, Debug)]
pub struct KeyInput {
    pub key: Key,
    pub shift: bool,
    /// Зажат Ctrl, Alt или Win.
    pub other_mods: bool,
}

impl KeyInput {
    pub fn plain(key: Key) -> Self {
        KeyInput {
            key,
            shift: false,
            other_mods: false,
        }
    }
}

/// Результат ручного хоткея: либо сделано, либо буфер пуст и платформе надо прочитать выделение.
#[derive(Debug, PartialEq, Eq)]
pub enum ManualStep {
    Done,
    NeedSelection,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SelectionConversion {
    /// Похоже на авто-копию строки (перевод строки, слишком длинно): не трогаем ничего.
    Refused,
    /// Конвертировать нечего.
    Nothing,
    Converted {
        text: String,
        to_cyrillic: bool,
    },
}

/// Что происходит с клавишей-границей, пока мы конвертируем слово внутри её обработки.
#[derive(Clone, Copy)]
enum Landing<'a> {
    /// Конверсия вне обработки нажатия (таймер, хоткей).
    None,
    /// Клавиша пройдёт в приложение раньше нашей замены.
    Lands(&'a str),
    /// Клавишу глотаем; её символ уже в хвосте буфера и печатается заменой.
    Swallowed(&'a str),
}

#[derive(Clone, Copy)]
struct BoundaryTask {
    due: f64,
    soft: bool,
    retried: bool,
}

/// Пауза внутри слова, после которой в мягком режиме это считается командами, а не набором.
const SOFT_MAX_INTRA_WORD_GAP: f64 = 1.0;
const TWO_CAPS_KEEP: &[&str] = &[
    "iphone", "ipad", "ipod", "imac", "icloud", "itunes", "imessage", "ibooks", "iwork", "ebay",
];
const BRAND_SPELLING: &[(&str, &str)] = &[
    ("ios", "iOS"),
    ("ipados", "iPadOS"),
    ("macos", "macOS"),
    ("watchos", "watchOS"),
    ("tvos", "tvOS"),
    ("visionos", "visionOS"),
    ("iphone", "iPhone"),
    ("ipad", "iPad"),
    ("imac", "iMac"),
    ("icloud", "iCloud"),
    ("itunes", "iTunes"),
    ("imessage", "iMessage"),
    ("ebay", "eBay"),
    ("esim", "eSIM"),
    ("iot", "IoT"),
];
const SELECTION_MAX_CHARS: usize = 300;
const CASE_CHANGE_MAX_CHARS: usize = 5000;

fn ru_validator(w: &str) -> bool {
    layout_data::shared().words_ru.contains(w)
}

fn script_class(s: &str) -> &'static str {
    match (has_cyrillic(s), has_latin_letter(s)) {
        (true, true) => "MIX",
        (true, false) => "CYR",
        (false, true) => "LAT",
        _ => "—",
    }
}

pub struct Engine {
    pub settings: Settings,
    pub exceptions: Exceptions,
    pub snippets: SnippetStore,
    pub typo: TypoFix,
    pub undo: UndoLearner,
    pub buffer: KeystrokeBuffer,
    resonance: AntiResonanceGuard,
    /// Сколько слов «расколдовано» — счётчик для меню.
    pub rescued_count: u64,
    /// Исключения поменялись (выучено слово) — платформе пора сохранить.
    pub exceptions_dirty: bool,
    /// Конвертировать на границе слова сразу, в обработчике нажатия, а не через +30 мс.
    /// На Маке задержка нужна, чтобы не держать колбэк тапа; на Windows порядок событий
    /// охраняет платформа («забор» в хуке), и синхронный путь исключает гонку со следующим словом.
    pub sync_boundary: bool,
    /// Сколько замен отправлено — чтобы понять, печатали ли мы что-то в этом вызове.
    replaced: u64,

    live_fix_last: String,
    word_edited: bool,
    pending_context_clear: bool,
    pending_soft_reset: bool,
    context_reset_at: f64,
    last_real_key_at: f64,
    caret_jumped_since_clear: bool,
    backspace_since_jump: bool,
    last_text_fix: Option<(String, String)>,
    front_app: String,
    front_app_mode: String,
    front_app_is_dev: bool,
    forced_layout_last_app: String,
    tasks: Vec<BoundaryTask>,
}

impl Engine {
    pub fn new(
        settings: Settings,
        exceptions: Exceptions,
        snippets: SnippetStore,
        typo: TypoFix,
        undo: UndoLearner,
    ) -> Self {
        Engine {
            settings,
            exceptions,
            snippets,
            typo,
            undo,
            buffer: KeystrokeBuffer::new(),
            resonance: AntiResonanceGuard::default(),
            rescued_count: 0,
            exceptions_dirty: false,
            sync_boundary: false,
            replaced: 0,
            live_fix_last: String::new(),
            word_edited: false,
            pending_context_clear: false,
            pending_soft_reset: false,
            context_reset_at: 0.0,
            last_real_key_at: 0.0,
            caret_jumped_since_clear: false,
            backspace_since_jump: false,
            last_text_fix: None,
            front_app: String::new(),
            front_app_mode: String::new(),
            front_app_is_dev: false,
            forced_layout_last_app: String::new(),
            tasks: Vec::new(),
        }
    }

    /// Все наши замены идут через эту точку.
    fn emit(&mut self, p: &mut dyn Platform, delete: usize, text: &str, then_return: bool) {
        self.replaced += 1;
        p.replace(delete, text, then_return);
    }

    fn paused(&self, p: &dyn Platform) -> bool {
        self.settings.is_paused(p.wall())
    }

    fn sound(&self, p: &mut dyn Platform) {
        if self.settings.sound_enabled {
            p.play_sound();
        }
    }

    // MARK: - Контекст

    /// Отложенная очистка после клика (см. `applyPendingContextClear` в Swift).
    fn apply_pending_context_clear(&mut self) -> bool {
        if !self.pending_context_clear {
            return false;
        }
        if self.last_real_key_at > self.context_reset_at {
            self.pending_context_clear = false;
            self.pending_soft_reset = true;
            return false;
        }
        self.live_fix_last.clear();
        self.caret_jumped_since_clear = true;
        self.backspace_since_jump = false;
        self.buffer.clear();
        self.undo.reset_context();
        self.resonance.reset_history();
        self.pending_context_clear = false;
        self.pending_soft_reset = false;
        true
    }

    /// Клик мышью или смена фокуса: каретка могла сместиться.
    pub fn context_reset(&mut self, now: f64) {
        self.context_reset_at = now;
        self.buffer.invalidate_group_history();
        self.pending_context_clear = true;
    }

    /// Активное окно сменилось. `exe` — имя исполняемого файла программы.
    pub fn foreground_changed(&mut self, exe: &str, p: &mut dyn Platform) {
        let exe = exe.to_lowercase();
        self.front_app_mode = self.exceptions.app_mode(&exe);
        self.front_app_is_dev = exceptions::is_dev_app(&exe);
        self.front_app = exe.clone();
        self.context_reset(p.now());
        // Жёсткая раскладка программы: только при входе в неё, дальше человек волен переключать.
        if exe != self.forced_layout_last_app {
            if let Some(lang) = self.exceptions.app_layout(&exe).map(str::to_string) {
                p.select_layout(lang == "ru");
                p.layout_changed();
                p.log(&format!("раскладка программы: {exe} → {lang}"));
            }
            self.forced_layout_last_app = exe;
        }
    }

    pub fn front_app(&self) -> &str {
        &self.front_app
    }

    /// Платформа знает о программе то, чего не знают исключения (например, что она запущена от
    /// администратора и нашу синтетику не примет) — тогда режим «не трогать» на время её фокуса.
    pub fn force_front_app_off(&mut self) {
        self.front_app_mode = "off".into();
    }

    // MARK: - Нажатия

    /// Нажатие реальной клавиши. true — клавишу надо проглотить (её заменит наша синтетика).
    pub fn key_down(&mut self, k: KeyInput, p: &mut dyn Platform) -> bool {
        let now = p.now();
        if !self.apply_pending_context_clear() && self.pending_soft_reset {
            self.buffer.soft_context_reset();
            self.resonance.reset_history();
            self.pending_soft_reset = false;
        }
        match &k.key {
            Key::Backspace => {
                self.live_fix_last.clear();
                self.buffer.backspace(now);
                self.word_edited = true;
                if self.caret_jumped_since_clear {
                    self.backspace_since_jump = true;
                }
                let cur = self.buffer.current_word.clone();
                self.undo.observe(&cur, &self.exceptions.learned, p.wall());
            }
            Key::DeleteWord => {
                self.live_fix_last.clear();
                self.buffer.clear();
                self.word_edited = false;
                self.undo.reset_context();
            }
            Key::Space | Key::Tab | Key::Enter => return self.boundary(&k, now, p),
            Key::Arrow => {
                self.live_fix_last.clear();
                self.word_edited = false;
                self.buffer.invalidate_group_history();
                if k.shift || self.settings.arrows_cancel {
                    self.buffer.clear();
                    self.undo.reset_context();
                }
            }
            Key::Nav => {
                self.live_fix_last.clear();
                self.word_edited = false;
                self.buffer.clear();
                self.undo.reset_context();
            }
            Key::Shortcut => {
                self.live_fix_last.clear();
                self.word_edited = false;
                self.buffer.clear();
            }
            Key::Char(s) => {
                if !s
                    .chars()
                    .next()
                    .is_some_and(|c| c as u32 >= 0x20 && c as u32 != 0x7F)
                {
                    return false;
                }
                self.last_real_key_at = now;
                let s = s.clone();
                if self.try_inline_live_fix(&s, &k, p) {
                    return true;
                }
                self.buffer.append(&s, now);
                let cur = self.buffer.current_word.clone();
                self.undo.observe(&cur, &self.exceptions.learned, p.wall());
            }
            Key::Other => {}
        }
        false
    }

    fn boundary(&mut self, k: &KeyInput, now: f64, p: &mut dyn Platform) -> bool {
        // Прошлое слово ещё ждёт своей конверсии (быстрый набор обогнал +30 мс): делаем её сейчас,
        // пока новая граница не сделала «последним» уже другое слово и то не осталось без починки.
        if !self.tasks.is_empty() {
            let pending: Vec<BoundaryTask> = self.tasks.drain(..).collect();
            if self.settings.auto_enabled && !self.paused(p) {
                let soft = pending.iter().any(|t| t.soft);
                // Эта граница не проглочена: её символ окажется на экране РАНЬШЕ нашей замены.
                let ws = match k.key {
                    Key::Tab => "\t",
                    Key::Enter => "\n",
                    _ => " ",
                };
                self.convert_word_ex(false, soft, false, Landing::Lands(ws), p);
            }
        }
        self.live_fix_last.clear();
        self.word_edited = false;
        self.last_real_key_at = now;
        let (ws, space, enter, tab) = match k.key {
            Key::Tab => ("\t", false, false, true),
            Key::Enter => ("\n", false, true, false),
            _ => (" ", true, false, false),
        };
        let st = &self.settings;
        let snip_key_ok = (space && st.snippet_expand_space)
            || (enter && st.snippet_expand_enter)
            || (tab && st.snippet_expand_tab);
        let snip_allowed =
            self.front_app_mode != "off" && !(st.developer_mode && self.front_app_is_dev);
        // Глотать и печатать границу сами можно, только если она «голая»: Shift+Enter, Ctrl+Enter,
        // Shift+Tab — это другие команды, а мы бы отдали приложению простой Enter/Tab.
        // Shift+пробел остаётся пробелом, его можно.
        let can_swallow = !k.other_mods && (space || !k.shift);
        // Сниппет: глотаем границу и раскрываем сами.
        if snip_key_ok
            && snip_allowed
            && can_swallow
            && !self.paused(p)
            && !self.buffer.current_word.is_empty()
        {
            if let Some(exp) = self
                .snippets
                .expansion(&self.buffer.current_word)
                .map(str::to_string)
            {
                self.expand_snippet(&exp, ws, now, p);
                return true;
            }
        }
        if enter && self.convert_before_return(k, p) {
            return true;
        }
        self.buffer.boundary(ws, now);
        let st = &self.settings;
        let mut auto =
            (space && st.trigger_space) || (enter && st.trigger_enter) || (tab && st.trigger_tab);
        if st.developer_mode && self.front_app_is_dev {
            auto = false;
        }
        if self.front_app_mode == "off" {
            auto = false;
        }
        if auto && st.auto_enabled && !self.paused(p) {
            let soft = self.front_app_mode == "soft";
            if self.sync_boundary && can_swallow {
                // Границу глотаем и печатаем сами внутри замены: так порядок «клавиша или наша
                // пачка первой» не важен вовсе (в Windows и в Wine он разный).
                let before = self.replaced;
                self.convert_word_ex(false, soft, false, Landing::Swallowed(ws), p);
                if self.replaced != before {
                    return true;
                }
            } else {
                self.tasks.push(BoundaryTask {
                    due: now + 0.03,
                    soft,
                    retried: false,
                });
            }
        }
        false
    }

    /// Ближайший момент, когда движку надо дать поработать ([`Engine::run_due`]).
    pub fn next_deadline(&self) -> Option<f64> {
        self.tasks.iter().map(|t| t.due).reduce(f64::min)
    }

    /// Выполнить отложенные задачи (авто-конверсия на границе слова, +30 мс).
    pub fn run_due(&mut self, p: &mut dyn Platform) {
        let now = p.now();
        let (due, rest): (Vec<_>, Vec<_>) = self.tasks.drain(..).partition(|t| t.due <= now);
        self.tasks = rest;
        for t in due {
            if !self.settings.auto_enabled || self.paused(p) {
                continue;
            }
            // Fence A: человек уже печатает следующее слово — одна отсрочка.
            if !t.retried && now - self.last_real_key_at < 0.025 {
                self.tasks.push(BoundaryTask {
                    due: now + 0.04,
                    soft: t.soft,
                    retried: true,
                });
                continue;
            }
            self.convert_word(false, t.soft, false, p);
        }
    }

    /// Выполнить все задачи сразу, не дожидаясь срока (для тестов и выхода).
    pub fn flush(&mut self, p: &mut dyn Platform) {
        for t in &mut self.tasks {
            t.due = f64::NEG_INFINITY;
            t.retried = true;
        }
        self.run_due(p);
    }

    // MARK: - Ручные действия

    /// Хоткей «переключить»: отмена нашей правки → слово из буфера → (буфер пуст) выделение.
    pub fn manual_hotkey(&mut self, p: &mut dyn Platform) -> ManualStep {
        self.apply_pending_context_clear();
        self.live_fix_last.clear();
        if self.undo_last_text_fix(p) {
            return ManualStep::Done;
        }
        if self.buffer.word_for_conversion(false).is_none() {
            return ManualStep::NeedSelection;
        }
        self.convert_word(true, false, false, p);
        ManualStep::Done
    }

    /// Продолжение ручного хоткея, когда выделение прочитать не удалось или оно отклонено.
    pub fn manual_without_selection(&mut self, selection_refused: bool, p: &mut dyn Platform) {
        self.convert_word(true, false, selection_refused, p);
    }

    /// Что сделать с выделенным текстом по хоткею (чистая функция, печатает платформа).
    pub fn convert_selection_text(text: &str) -> SelectionConversion {
        if text.contains('\n') || text.contains('\r') || char_count(text) > SELECTION_MAX_CHARS {
            return SelectionConversion::Refused;
        }
        let to_cyr = if has_cyrillic(text) {
            false
        } else if has_latin_letter(text) {
            true
        } else if let Some(d) = keymap::unambiguous_symbol_direction(text) {
            d
        } else {
            return SelectionConversion::Nothing;
        };
        let out = keymap::convert(text, to_cyr);
        if out == text {
            SelectionConversion::Nothing
        } else {
            SelectionConversion::Converted {
                text: out,
                to_cyrillic: to_cyr,
            }
        }
    }

    /// Платформа напечатала конвертированное выделение.
    pub fn selection_converted(&mut self, original: &str, to_cyrillic: bool, p: &mut dyn Platform) {
        self.rescued_count += word_count(original).max(1) as u64;
        self.buffer.clear();
        p.select_layout(to_cyrillic);
        p.layout_changed();
        self.sound(p);
        p.log(&format!(
            "convert-selection: {} симв. → {}",
            char_count(original),
            if to_cyrillic { "RU" } else { "EN" }
        ));
    }

    /// Смена регистра выделенного: есть строчные — всё в ЗАГЛАВНЫЕ, иначе в строчные.
    pub fn case_changed_text(text: &str) -> Option<String> {
        if text.is_empty() || char_count(text) > CASE_CHANGE_MAX_CHARS {
            return None;
        }
        let out = if text.chars().any(is_lower) {
            text.to_uppercase()
        } else {
            text.to_lowercase()
        };
        (out != text).then_some(out)
    }

    /// Хоткей «только сменить раскладку»: набранное не трогаем, слово обрываем.
    pub fn layout_switch_only(&mut self, p: &mut dyn Platform) {
        if !p.cycle_layout() {
            p.log("switch: среди включённых нет раскладок для цикла");
            return;
        }
        self.live_fix_last.clear();
        self.buffer.clear();
        p.layout_changed();
    }

    /// Предложения «больше не переключать это слово» от обучения на отмене.
    pub fn take_learn_suggestions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.undo.suggestions)
    }

    pub fn confirm_learn(&mut self, word: &str) {
        self.exceptions.add_learned(word);
        self.undo.confirm(word);
        self.exceptions_dirty = true;
    }

    pub fn decline_learn(&mut self, word: &str) {
        self.undo.decline(word);
    }

    /// Живая таблица раскладок обновилась — канонические триггеры сниппетов пересчитать.
    pub fn keymap_changed(&mut self) {
        self.snippets.rebuild_index();
    }

    // MARK: - Конверсия

    fn convert_group(&mut self, p: &mut dyn Platform) -> bool {
        let Some(g) = self.buffer.group_for_conversion(p.now()) else {
            return false;
        };
        let mut out = String::new();
        let mut any = false;
        let mut n = 0;
        let mut last_to_cyr = false;
        let mut prev: Option<String> = None;
        for sw in &g.words {
            match detector::decide(&sw.word, &self.exceptions, prev.as_deref(), None, false) {
                SwapDecision::Convert { to_cyrillic } => {
                    let conv = keymap::smart_convert(&sw.word, to_cyrillic, Some(&ru_validator));
                    out.push_str(&conv);
                    out.push_str(&sw.tail);
                    any = true;
                    n += 1;
                    last_to_cyr = to_cyrillic;
                    prev = Some(conv);
                }
                SwapDecision::Keep => {
                    out.push_str(&sw.word);
                    out.push_str(&sw.tail);
                    prev = Some(sw.word.clone());
                }
            }
        }
        if !any {
            return true;
        }
        if char_count(&out) != g.delete_count {
            p.log("convert-group: длина разошлась — отказ");
            return true;
        }
        p.log(&format!(
            "convert-group(хоткей): {} слов, {} симв.",
            g.words.len(),
            g.delete_count
        ));
        self.emit(p, g.delete_count, &out, false);
        self.rescued_count += n;
        self.buffer.clear();
        p.select_layout(last_to_cyr);
        p.layout_changed();
        self.sound(p);
        true
    }

    fn convert_word(
        &mut self,
        manual: bool,
        soft: bool,
        selection_refused: bool,
        p: &mut dyn Platform,
    ) {
        self.convert_word_ex(manual, soft, selection_refused, Landing::None, p);
    }

    fn convert_word_ex(
        &mut self,
        manual: bool,
        soft: bool,
        selection_refused: bool,
        landing: Landing,
        p: &mut dyn Platform,
    ) {
        if manual
            && self.settings.group_convert
            && !self.settings.auto_enabled
            && self.convert_group(p)
        {
            return;
        }
        let Some(item) = self.buffer.word_for_conversion(!manual) else {
            if manual {
                if selection_refused {
                    p.log("хоткей: выделение отклонено — ничего не делаю");
                    return;
                }
                let cycled = p.cycle_layout();
                p.layout_changed();
                p.log(if cycled {
                    "хоткей при пустом буфере: переключил раскладку циклом"
                } else {
                    "хоткей при пустом буфере: циклить нечего"
                });
            }
            return;
        };
        let item = match landing {
            Landing::None => item,
            Landing::Lands(ws) => ConversionItem {
                delete_count: item.delete_count + char_count(ws),
                tail: format!("{}{ws}", item.tail),
                word: item.word,
            },
            // Хвост уже содержит границу (buffer.boundary), а на экране её ещё нет.
            Landing::Swallowed(ws) => ConversionItem {
                delete_count: item.delete_count - char_count(ws),
                tail: item.tail,
                word: item.word,
            },
        };
        let word = item.word.clone();
        let mut auto_prop: Option<(String, bool, bool)> = None;
        let to_cyr = if manual {
            if has_cyrillic(&word) {
                false
            } else if has_latin_letter(&word) {
                true
            } else {
                p.current_script() != Script::Cyrillic
            }
        } else {
            match self.auto_conversion_proposal(&word, soft, true, p) {
                Some(prop) => {
                    let d = prop.1;
                    auto_prop = Some(prop);
                    d
                }
                None => {
                    self.fix_two_leading_caps(&word, &item, p);
                    self.fix_typo(&word, &item, p);
                    return;
                }
            }
        };
        let converted = match &auto_prop {
            Some(pr) => pr.0.clone(),
            None => keymap::convert(&word, to_cyr),
        };
        if converted == word {
            if manual {
                p.beep();
            }
            return;
        }
        if !manual && !self.resonance.allow(&word, &converted, p.now()) {
            p.log("авто молчит: анти-резонанс заморозил конверсию");
            self.live_fix_last.clear();
            self.buffer.clear();
            return;
        }
        let rescue = auto_prop.as_ref().is_some_and(|x| x.2);
        p.log(&format!(
            "{}{}: {} симв. {} → {}",
            if rescue {
                "mixed-rescue"
            } else {
                "convert-word"
            },
            if manual {
                "(хоткей)"
            } else {
                "(авто)"
            },
            item.delete_count,
            script_class(&word),
            if to_cyr { "RU" } else { "EN" }
        ));
        self.emit(
            p,
            item.delete_count,
            &format!("{converted}{}", item.tail),
            false,
        );
        let wall = p.wall();
        if manual {
            self.buffer.apply_conversion(&converted);
            self.undo
                .note_manual_convert(&word, &converted, &self.exceptions.learned, wall);
            self.undo.protect(&converted);
        } else {
            self.buffer.apply_completed_conversion(&converted);
            if !rescue {
                self.undo
                    .note_conversion(&word, &converted, &self.exceptions.learned, wall);
            }
        }
        self.rescued_count += 1;
        p.select_layout(to_cyr);
        p.layout_changed();
        self.sound(p);
    }

    /// Единая точка авто-решения для границы слова и Enter-pre: (текст, в кириллицу, rescue).
    fn auto_conversion_proposal(
        &mut self,
        word: &str,
        soft: bool,
        completed: bool,
        p: &mut dyn Platform,
    ) -> Option<(String, bool, bool)> {
        if !layout_data::is_ready() {
            return None;
        }
        if p.current_script() == Script::Other {
            return None;
        }
        if self.undo.is_session_protected(word) {
            p.log("авто молчит: слово под session-защитой");
            return None;
        }
        if let SwapDecision::Convert { to_cyrillic } = detector::mixed_rescue(word) {
            return Some((keymap::convert(word, to_cyrillic), to_cyrillic, true));
        }
        let ctx_current = !completed && !self.buffer.current_word.is_empty();
        let prev = self.buffer.context_word(ctx_current);
        let earlier = self.buffer.earlier_context_word(ctx_current);
        let mut after_jump = self.caret_jumped_since_clear;
        self.caret_jumped_since_clear = false;
        // Accessibility-пробы каретки на Windows нет: судим по Backspace (ветка .unknown оригинала).
        if after_jump && char_count(word) == 1 && !self.backspace_since_jump {
            after_jump = false;
        }
        match detector::decide(
            word,
            &self.exceptions,
            prev.as_deref(),
            earlier.as_deref(),
            after_jump,
        ) {
            SwapDecision::Keep => None,
            SwapDecision::Convert { to_cyrillic } => {
                if soft {
                    let core = keymap::core_of(word).to_lowercase();
                    let mut uniq: Vec<char> = core.chars().collect();
                    uniq.sort_unstable();
                    uniq.dedup();
                    if char_count(&core) <= 2 || uniq.len() == 1 {
                        return None;
                    }
                    let gap = if completed || self.buffer.current_word.is_empty() {
                        self.buffer.last_word_gap
                    } else {
                        self.buffer.current_word_gap
                    };
                    if gap > SOFT_MAX_INTRA_WORD_GAP {
                        return None;
                    }
                }
                let converted = keymap::smart_convert(word, to_cyrillic, Some(&ru_validator));
                (converted != word).then_some((converted, to_cyrillic, false))
            }
        }
    }

    /// Правка на лету прямо в обработчике нажатия: клавиша глотается и входит в замену.
    fn try_inline_live_fix(&mut self, pending: &str, k: &KeyInput, p: &mut dyn Platform) -> bool {
        let st = &self.settings;
        if !st.live_fix_enabled || !st.auto_enabled || self.paused(p) {
            return false;
        }
        if !layout_data::is_ready() || self.word_edited {
            return false;
        }
        if self.front_app_is_dev && self.settings.developer_mode {
            return false;
        }
        if !self.front_app_mode.is_empty() || k.shift || k.other_mods {
            return false;
        }
        if p.current_script() == Script::Other {
            return false;
        }
        let on_screen = self.buffer.current_word.clone();
        let candidate = format!("{on_screen}{pending}");
        let n = char_count(&candidate);
        if !(4..=16).contains(&n) || candidate == self.live_fix_last {
            return false;
        }
        if has_cyrillic(&candidate) && has_latin_letter(&candidate) {
            return false;
        }
        let SwapDecision::Convert { to_cyrillic } =
            detector::live_decide(&candidate, &self.exceptions)
        else {
            return false;
        };
        let Some(converted) = keymap::live_convert(&candidate, to_cyrillic, Some(&ru_validator))
        else {
            return false;
        };
        if converted == candidate {
            return false;
        }
        let now = p.now();
        if !self.resonance.allow(&candidate, &converted, now) {
            self.live_fix_last.clear();
            self.buffer.clear();
            return false;
        }
        if self.undo.should_suppress(&candidate, p.wall())
            || self.undo.is_session_protected(&candidate)
        {
            return false;
        }
        self.emit(p, char_count(&on_screen), &converted, false);
        self.buffer.append(pending, now);
        self.buffer.apply_conversion(&converted);
        self.live_fix_last = converted.clone();
        p.select_layout(to_cyrillic);
        self.undo
            .note_conversion(&candidate, &converted, &self.exceptions.learned, p.wall());
        self.rescued_count += 1;
        p.layout_changed();
        self.sound(p);
        p.log(&format!(
            "inline-fix: {} симв. {}→{}",
            n,
            script_class(&candidate),
            script_class(&converted)
        ));
        true
    }

    /// Enter-pre: чиним слово, пока оно ещё на экране, и отпускаем Enter синтетикой после замены.
    fn convert_before_return(&mut self, k: &KeyInput, p: &mut dyn Platform) -> bool {
        if !layout_data::is_ready() {
            return false;
        }
        let st = &self.settings;
        if !(st.enter_pre_convert && st.auto_enabled && st.trigger_enter) || self.paused(p) {
            return false;
        }
        if k.shift || k.other_mods {
            return false;
        }
        let word = self.buffer.current_word.clone();
        if word.is_empty() {
            return false;
        }
        if self.settings.developer_mode && self.front_app_is_dev {
            return false;
        }
        if self.front_app_mode == "off" {
            return false;
        }
        if self.settings.typo_fix {
            if let Some(fixed) = self.typo.numeric_suggestion(&word, &self.typo_guard()) {
                return self.apply_typo_before_return(&word, &fixed, p);
            }
        }
        let prop = self.auto_conversion_proposal(&word, self.front_app_mode == "soft", false, p);
        if prop.is_none() && self.settings.typo_fix {
            if let Some(fixed) = self.typo.curated_suggestion(&word, &self.typo_guard()) {
                return self.apply_typo_before_return(&word, &fixed, p);
            }
        }
        let Some((text, to_cyr, rescue)) = prop else {
            return false;
        };
        if !self.resonance.allow(&word, &text, p.now()) {
            self.live_fix_last.clear();
            self.buffer.clear();
            return false;
        }
        p.log(&format!(
            "{}(enter-pre): {} симв. → {}",
            if rescue {
                "mixed-rescue"
            } else {
                "convert-word"
            },
            char_count(&word),
            if to_cyr { "RU" } else { "EN" }
        ));
        self.emit(p, char_count(&word), &text, true);
        self.buffer.apply_conversion(&text);
        self.buffer.boundary("\n", p.now());
        if !rescue {
            self.undo
                .note_conversion(&word, &text, &self.exceptions.learned, p.wall());
        }
        p.select_layout(to_cyr);
        self.rescued_count += 1;
        p.layout_changed();
        self.sound(p);
        true
    }

    fn apply_typo_before_return(&mut self, word: &str, fixed: &str, p: &mut dyn Platform) -> bool {
        p.log(&format!(
            "опечатка(enter-pre): {} симв. исправлено",
            char_count(word)
        ));
        self.last_text_fix = None;
        self.emit(p, char_count(word), fixed, true);
        self.buffer.apply_conversion(fixed);
        self.buffer.boundary("\n", p.now());
        true
    }

    fn expand_snippet(&mut self, expansion: &str, ws: &str, now: f64, p: &mut dyn Platform) {
        let trigger_len = char_count(&self.buffer.current_word);
        let body = snippets::sanitize(expansion);
        let glue = match body.chars().last() {
            Some(l) if (l == ' ' || l == '\t') && l.to_string() == ws => "",
            _ => ws,
        };
        self.emit(p, trigger_len, &format!("{body}{glue}"), false);
        self.buffer.commit_snippet(expansion, ws, now);
        self.buffer.invalidate_group_history();
        self.sound(p);
        p.log(&format!(
            "snippet: {} → {} симв., граница проглочена",
            trigger_len,
            char_count(&body)
        ));
    }

    // MARK: - Правки текста (не раскладки)

    fn typo_guard(&self) -> Guard<'_> {
        Guard {
            exceptions: &self.exceptions,
            session_protected: &|_| false,
        }
    }

    /// «КОгда» → «Когда» (выключено по умолчанию).
    fn fix_two_leading_caps(&mut self, word: &str, item: &ConversionItem, p: &mut dyn Platform) {
        if !self.settings.two_caps_fix {
            return;
        }
        let ch: Vec<char> = word.chars().collect();
        if ch.len() < 3 || !ch.iter().all(|c| is_letter(*c)) {
            return;
        }
        if !(is_upper(ch[0]) && is_upper(ch[1]) && is_lower(ch[2])) {
            return;
        }
        if has_cyrillic(word) == has_latin_letter(word) {
            return;
        }
        let fixed: String = std::iter::once(ch[0])
            .chain(ch[1].to_lowercase())
            .chain(ch[2..].iter().copied())
            .collect();
        let lw = word.to_lowercase();
        if fixed == word || self.exceptions.is_ignored(&lw) || TWO_CAPS_KEEP.contains(&lw.as_str())
        {
            return;
        }
        p.log(&format!(
            "две заглавные: {} симв. исправлено",
            item.delete_count
        ));
        self.emit(
            p,
            item.delete_count,
            &format!("{fixed}{}", item.tail),
            false,
        );
        self.buffer.apply_completed_conversion(&fixed);
    }

    /// След Caps Lock: «пРИВЕТ» → «Привет» (от пяти букв, известные имена — канонически).
    fn fix_caps_lock_word(
        &mut self,
        word: &str,
        item: &ConversionItem,
        p: &mut dyn Platform,
    ) -> bool {
        if !self.settings.typo_fix {
            return false;
        }
        let ch: Vec<char> = word.chars().collect();
        if ch.len() < 5 || !ch.iter().all(|c| is_letter(*c)) {
            return false;
        }
        if !(is_lower(ch[0]) && ch[1..].iter().all(|c| is_upper(*c))) {
            return false;
        }
        if has_cyrillic(word) == has_latin_letter(word) {
            return false;
        }
        let lw = word.to_lowercase();
        if self.exceptions.is_ignored(&lw) {
            return false;
        }
        let fixed = BRAND_SPELLING
            .iter()
            .find(|(k, _)| *k == lw)
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| upper_first(&lw));
        if fixed == word {
            return false;
        }
        p.log(&format!(
            "caps lock: {} симв. развёрнуто",
            item.delete_count
        ));
        self.undo
            .note_conversion(word, &fixed, &self.exceptions.learned, p.wall());
        self.last_text_fix = Some((word.to_string(), fixed.clone()));
        self.emit(
            p,
            item.delete_count,
            &format!("{fixed}{}", item.tail),
            false,
        );
        self.buffer.apply_completed_conversion(&fixed);
        true
    }

    fn fix_typo(&mut self, word: &str, item: &ConversionItem, p: &mut dyn Platform) {
        if self.fix_caps_lock_word(word, item, p) {
            return;
        }
        self.typo.note_typed(word);
        if !self.settings.typo_fix {
            return;
        }
        let protected: Vec<String> = [word.to_lowercase()]
            .into_iter()
            .filter(|w| self.undo.is_session_protected(w))
            .collect();
        let guard = Guard {
            exceptions: &self.exceptions,
            session_protected: &|w: &str| protected.iter().any(|x| x == w),
        };
        let Some(fixed) = self.typo.suggest(word, &guard) else {
            return;
        };
        p.log(&format!("опечатка: {} симв. исправлено", item.delete_count));
        self.undo
            .note_conversion(word, &fixed, &self.exceptions.learned, p.wall());
        self.last_text_fix = Some((word.to_string(), fixed.clone()));
        self.emit(
            p,
            item.delete_count,
            &format!("{fixed}{}", item.tail),
            false,
        );
        self.buffer.apply_completed_conversion(&fixed);
    }

    /// Первое нажатие хоткея после нашей правки текста возвращает слово как было.
    fn undo_last_text_fix(&mut self, p: &mut dyn Platform) -> bool {
        let Some((original, fixed)) = self.last_text_fix.clone() else {
            return false;
        };
        let Some(item) = self.buffer.word_for_conversion(false) else {
            return false;
        };
        if item.word != fixed {
            self.last_text_fix = None;
            return false;
        }
        p.log(&format!(
            "отмена правки: {} симв. возвращено",
            item.delete_count
        ));
        self.undo
            .note_manual_convert(&fixed, &original, &self.exceptions.learned, p.wall());
        self.undo.protect(&original);
        self.last_text_fix = None;
        self.emit(
            p,
            item.delete_count,
            &format!("{original}{}", item.tail),
            false,
        );
        self.buffer.apply_completed_conversion(&original);
        self.sound(p);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typo::TypoFix;
    use crate::undo::{UndoLearner, UndoState};
    use std::collections::BTreeMap;

    /// Эмулятор текстового поля: экран, раскладка, часы.
    struct Field {
        screen: String,
        cyr: bool,
        t: f64,
        sounds: usize,
        /// Идёт обработка нажатия: как на Windows, наша синтетика встаёт в очередь ПОСЛЕ него.
        in_key: bool,
        queued: Vec<(usize, String, bool)>,
    }

    impl Field {
        fn apply(&mut self, delete: usize, text: &str, then_return: bool) {
            for _ in 0..delete {
                self.screen.pop();
            }
            self.screen.push_str(text);
            if then_return {
                self.screen.push('\n');
            }
        }
    }

    impl Platform for Field {
        fn now(&self) -> f64 {
            self.t
        }
        fn wall(&self) -> f64 {
            1_000_000.0 + self.t
        }
        fn replace(&mut self, delete: usize, text: &str, then_return: bool) {
            if self.in_key {
                self.queued.push((delete, text.to_string(), then_return));
            } else {
                self.apply(delete, text, then_return);
            }
        }
        fn select_layout(&mut self, cyrillic: bool) {
            self.cyr = cyrillic;
        }
        fn cycle_layout(&mut self) -> bool {
            self.cyr = !self.cyr;
            true
        }
        fn current_script(&self) -> Script {
            if self.cyr {
                Script::Cyrillic
            } else {
                Script::Latin
            }
        }
        fn play_sound(&mut self) {
            self.sounds += 1;
        }
        fn beep(&mut self) {}
        fn layout_changed(&mut self) {}
        fn log(&mut self, _: &str) {}
    }

    struct Rig {
        e: Engine,
        f: Field,
    }

    impl Rig {
        fn new(live_fix: bool) -> Self {
            layout_data::warm_up();
            let settings = Settings {
                live_fix_enabled: live_fix,
                ..Settings::default()
            };
            let e = Engine::new(
                settings,
                Exceptions::seeded(),
                SnippetStore::new(vec![("mail".into(), "me@example.com".into())]),
                TypoFix::load(BTreeMap::new()),
                UndoLearner::new(UndoState::default(), true),
            );
            Rig {
                e,
                f: Field {
                    screen: String::new(),
                    cyr: false,
                    t: 0.0,
                    sounds: 0,
                    in_key: false,
                    queued: Vec::new(),
                },
            }
        }

        fn key(&mut self, key: Key) -> bool {
            self.key_after(key, 0.08)
        }

        fn key_after(&mut self, key: Key, dt: f64) -> bool {
            self.f.t += dt;
            self.e.run_due(&mut self.f);
            let echo = match &key {
                Key::Char(s) => Some(s.clone()),
                Key::Space => Some(" ".into()),
                Key::Enter => Some("\n".into()),
                Key::Tab => Some("\t".into()),
                _ => None,
            };
            self.f.in_key = true;
            let swallowed = self.e.key_down(KeyInput::plain(key.clone()), &mut self.f);
            self.f.in_key = false;
            if !swallowed {
                if key == Key::Backspace {
                    self.f.screen.pop();
                } else if let Some(s) = echo {
                    self.f.screen.push_str(&s);
                }
            }
            for (d, t, r) in std::mem::take(&mut self.f.queued) {
                self.f.apply(d, &t, r);
            }
            swallowed
        }

        /// Печатает физическими клавишами US-раскладки: в кириллице они дают русские буквы.
        fn type_keys(&mut self, keys: &str) {
            self.type_keys_every(keys, 0.08);
        }

        fn type_keys_every(&mut self, keys: &str, dt: f64) {
            for c in keys.chars() {
                let key = match c {
                    ' ' => Key::Space,
                    '\n' => Key::Enter,
                    _ => Key::Char(if self.f.cyr {
                        keymap::convert_with(&c.to_string(), true, None)
                    } else {
                        c.to_string()
                    }),
                };
                self.key_after(key, dt);
            }
        }

        fn settle(&mut self) {
            self.f.t += 1.0;
            self.e.flush(&mut self.f);
        }
    }

    #[test]
    fn boundary_converts_gibberish() {
        let mut r = Rig::new(false);
        r.type_keys("ghbdtn ");
        r.settle();
        assert_eq!(r.f.screen, "привет ");
        assert!(r.f.cyr);
        r.type_keys("vbh ");
        r.settle();
        assert_eq!(r.f.screen, "привет мир ");
    }

    /// Набор быстрее +30 мс: каждое слово всё равно чинится (и в асинхронном режиме).
    #[test]
    fn fast_typing_converts_every_word() {
        for sync in [false, true] {
            for live in [false, true] {
                let mut r = Rig::new(live);
                r.e.sync_boundary = sync;
                r.type_keys_every("ghbdtn vbh ntrcn ", 0.005);
                r.settle();
                assert_eq!(r.f.screen, "привет мир текст ", "sync={sync} live={live}");
            }
        }
    }

    /// Shift+Enter (перенос строки в чатах) не глотаем и не подменяем простым Enter.
    #[test]
    fn modified_enter_is_never_swallowed() {
        let mut r = Rig::new(false);
        r.e.sync_boundary = true;
        r.type_keys("ghbdtn");
        let k = KeyInput { key: Key::Enter, shift: true, other_mods: false };
        assert!(!r.e.key_down(k, &mut r.f));
        r.f.screen.push('\n');
        r.settle();
        assert_eq!(r.f.screen, "привет\n");
    }

    #[test]
    fn valid_words_untouched() {
        let mut r = Rig::new(false);
        r.type_keys("hello world ");
        r.settle();
        assert_eq!(r.f.screen, "hello world ");
        assert!(!r.f.cyr);
    }

    #[test]
    fn live_fix_converts_mid_word() {
        let mut r = Rig::new(true);
        r.type_keys("ghbdtn ");
        r.settle();
        assert_eq!(r.f.screen, "привет ");
        assert!(r.f.cyr);
    }

    #[test]
    fn enter_pre_converts_before_return() {
        let mut r = Rig::new(false);
        r.type_keys("ghbdtn");
        assert!(r.key(Key::Enter));
        assert_eq!(r.f.screen, "привет\n");
    }

    #[test]
    fn snippet_expands() {
        let mut r = Rig::new(false);
        r.type_keys("mail ");
        assert_eq!(r.f.screen, "me@example.com ");
    }

    #[test]
    fn manual_hotkey_flips_word() {
        let mut r = Rig::new(false);
        r.type_keys("hello");
        assert_eq!(r.e.manual_hotkey(&mut r.f), ManualStep::Done);
        assert_eq!(r.f.screen, "руддщ");
        // Повтор возвращает обратно.
        r.e.manual_hotkey(&mut r.f);
        assert_eq!(r.f.screen, "hello");
    }

    #[test]
    fn manual_hotkey_empty_buffer_asks_selection() {
        let mut r = Rig::new(false);
        assert_eq!(r.e.manual_hotkey(&mut r.f), ManualStep::NeedSelection);
        r.e.manual_without_selection(false, &mut r.f);
        assert!(r.f.cyr);
    }

    #[test]
    fn manual_after_auto_is_undo_and_protects() {
        let mut r = Rig::new(false);
        r.type_keys("ghbdtn ");
        r.settle();
        assert_eq!(r.f.screen, "привет ");
        r.e.manual_hotkey(&mut r.f);
        assert_eq!(r.f.screen, "ghbdtn ");
        assert!(r.e.undo.is_session_protected("ghbdtn"));
    }

    #[test]
    fn click_clears_context() {
        let mut r = Rig::new(false);
        r.type_keys("ghbd");
        r.f.t += 0.5;
        r.e.context_reset(r.f.t);
        r.type_keys("tn ");
        r.settle();
        // После клика «tn» — огрызок чужого слова, его не трогаем.
        assert_eq!(r.f.screen, "ghbdtn ");
    }

    #[test]
    fn off_app_is_left_alone() {
        let mut r = Rig::new(false);
        r.e.foreground_changed("WindowsTerminal.exe", &mut r.f);
        r.type_keys("ghbdtn ");
        r.settle();
        assert_eq!(r.f.screen, "ghbdtn ");
    }

    #[test]
    fn selection_rules() {
        assert_eq!(
            Engine::convert_selection_text("ghbdtn vbh"),
            SelectionConversion::Converted {
                text: "привет мир".into(),
                to_cyrillic: true
            }
        );
        assert_eq!(
            Engine::convert_selection_text("line\n"),
            SelectionConversion::Refused
        );
        assert_eq!(
            Engine::convert_selection_text("123"),
            SelectionConversion::Nothing
        );
        assert_eq!(Engine::case_changed_text("Hello").as_deref(), Some("HELLO"));
        assert_eq!(Engine::case_changed_text("HELLO").as_deref(), Some("hello"));
    }

    #[test]
    fn typo_fix_and_undo_by_hotkey() {
        let mut r = Rig::new(false);
        r.e.settings.typo_fix = true;
        r.type_keys("acheive ");
        r.settle();
        assert_eq!(r.f.screen, "achieve ");
        r.e.manual_hotkey(&mut r.f);
        assert_eq!(r.f.screen, "acheive ");
    }
}
