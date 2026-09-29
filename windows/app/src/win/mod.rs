//! Windows-оболочка Keyboop. Три потока:
//!
//! * **хук** (`hook.rs`) — низкоуровневые хуки клавиатуры и мыши и слежение за активным окном.
//!   Колбэки обязаны быть быстрыми: Windows молча снимает хук, который отвечает дольше порога.
//! * **рабочий** (здесь) — отложенная авто-конверсия (+30 мс после границы слова), ручные
//!   хоткеи с чтением выделения, сохранение данных.
//! * **интерфейс** (`tray.rs`) — значок в трее с текущей раскладкой и меню.
//!
//! Движок (`keyboop_core::Engine`) общий и живёт под мьютексом.

mod clipboard;
mod hook;
mod input;
mod layouts;
mod sys;
mod tray;

use crate::storage::{State, Store};
use crate::{hotkey, l10n, storage};
use keyboop_core::snippets::SnippetStore;
use keyboop_core::typo::TypoFix;
use keyboop_core::undo::UndoLearner;
use keyboop_core::{Engine, Platform, Script};
use layouts::Layouts;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub enum Cmd {
    /// Прочитать выделенный текст (буфер обмена) и вернуть его потоку хуков.
    ReadSelection(hook::SelectionKind),
    /// Сохранить всё, что поменялось.
    Persist,
    Quit,
}

pub struct Hotkeys {
    pub convert: Option<hotkey::Hotkey>,
    pub switch_layout: Option<hotkey::Hotkey>,
    pub case: Option<hotkey::Hotkey>,
}

pub struct App {
    pub engine: Mutex<Engine>,
    pub layouts: Mutex<Layouts>,
    pub hotkeys: Mutex<Hotkeys>,
    pub store: Store,
    pub log: storage::Log,
    pub tx: Mutex<Sender<Cmd>>,
    pub ui_hwnd: AtomicIsize,
    /// Последняя «чужая» программа на переднем плане (для пункта меню исключений).
    pub last_app: Mutex<String>,
    /// Слова, которые предлагаем выучить (баннер + пункт меню).
    pub learn_offers: Mutex<Vec<String>>,
    pub saved_mtimes: Mutex<[Option<SystemTime>; 3]>,
    start: Instant,
    last_rescued_saved: AtomicU64,
}

static APP: OnceLock<App> = OnceLock::new();

pub fn app() -> &'static App {
    APP.get().expect("App не инициализирован")
}

impl App {
    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.lock().unwrap().send(cmd);
    }

    pub fn log(&self, msg: &str) {
        self.log.write(msg);
    }

    /// Сообщение окну трея (перерисовать значок, показать баннер и т. п.).
    pub fn notify_ui(&self, msg: u32) {
        let h = self.ui_hwnd.load(Ordering::Relaxed);
        if h != 0 {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(h as _, msg, 0, 0);
            }
        }
    }

    /// Применить то из настроек, что живёт вне движка: хоткеи и громкость звука.
    pub fn apply_settings(&self, s: &keyboop_core::Settings) {
        sys::set_sound_volume(s.sound_volume);
        let parse = |spec: &str| match hotkey::parse(spec) {
            Ok(h) => h,
            Err(e) => {
                self.log(&e);
                None
            }
        };
        *self.hotkeys.lock().unwrap() = Hotkeys {
            convert: parse(&s.hotkey_convert),
            switch_layout: parse(&s.hotkey_switch_layout),
            case: parse(&s.hotkey_case),
        };
    }

    pub fn save_settings(&self) {
        let s = self.engine.lock().unwrap().settings.clone();
        self.store.save_settings(&s);
        self.remember_mtimes();
    }

    pub fn remember_mtimes(&self) {
        *self.saved_mtimes.lock().unwrap() = [
            self.store.mtime(storage::SETTINGS),
            self.store.mtime(storage::EXCEPTIONS),
            self.store.mtime(storage::SNIPPETS),
        ];
    }

    /// Файлы поменяли снаружи (Блокнот) — перечитать. Зовётся таймером интерфейса.
    pub fn reload_if_changed(&self, force: bool) {
        let now = [
            self.store.mtime(storage::SETTINGS),
            self.store.mtime(storage::EXCEPTIONS),
            self.store.mtime(storage::SNIPPETS),
        ];
        let changed = force || *self.saved_mtimes.lock().unwrap() != now;
        if !changed {
            return;
        }
        let mut errors = Vec::new();
        let settings = self.store.load_settings(&mut errors);
        let exceptions = self.store.load_exceptions(&mut errors);
        let snippets = self.store.load_snippets(&mut errors);
        if !errors.is_empty() {
            for e in &errors {
                self.log(&format!("настройки не перечитаны: {e}"));
            }
            tray::balloon(l10n::t("config.error"), l10n::t("config.error.body"));
            self.remember_mtimes();
            return;
        }
        l10n::set_language(&settings.language, sys::system_is_russian());
        self.apply_settings(&settings);
        {
            let mut e = self.engine.lock().unwrap();
            e.settings = settings;
            e.exceptions = exceptions;
            e.snippets.set_all(snippets);
            e.undo.enabled = e.settings.learn_on_undo;
            let front = e.front_app().to_string();
            let mut p = WinPlatform { app: self };
            e.foreground_changed(&front, &mut p);
        }
        self.remember_mtimes();
        self.log("настройки перечитаны");
        self.notify_ui(tray::WM_APP_REFRESH);
    }

    /// Сохранить изменившиеся данные движка.
    pub fn persist(&self) {
        let (exc, state) = {
            let mut e = self.engine.lock().unwrap();
            let exc = e.exceptions_dirty.then(|| e.exceptions.clone());
            e.exceptions_dirty = false;
            let dirty = e.undo.dirty
                || e.typo.dirty
                || e.rescued_count != self.last_rescued_saved.load(Ordering::Relaxed);
            let state = dirty.then(|| {
                e.undo.dirty = false;
                e.typo.dirty = false;
                let l = self.layouts.lock().unwrap();
                let mut last_layouts = std::collections::BTreeMap::new();
                last_layouts.insert("lat".to_string(), layouts::hkl_hex(l.last_lat));
                last_layouts.insert("cyr".to_string(), layouts::hkl_hex(l.last_cyr));
                State {
                    rescued_count: e.rescued_count,
                    undo: e.undo.state.clone(),
                    typo_personal: e.typo.personal.clone(),
                    last_layouts,
                }
            });
            (exc, state)
        };
        if let Some(x) = exc {
            self.store.save_exceptions(&x);
            self.remember_mtimes();
        }
        if let Some(s) = state {
            self.last_rescued_saved
                .store(s.rescued_count, Ordering::Relaxed);
            self.store.save_state(&s);
        }
    }
}

/// Реализация [`Platform`] для Windows. Создаётся на время вызова движка.
pub struct WinPlatform<'a> {
    pub app: &'a App,
}

impl Platform for WinPlatform<'_> {
    fn now(&self) -> f64 {
        self.app.now()
    }
    fn wall(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
    fn replace(&mut self, delete: usize, text: &str, then_return: bool) {
        hook::send_replacement(&input::replacement(delete, text, then_return));
    }
    fn select_layout(&mut self, cyrillic: bool) {
        self.app.layouts.lock().unwrap().select(cyrillic);
    }
    fn cycle_layout(&mut self) -> bool {
        self.app.layouts.lock().unwrap().cycle()
    }
    fn current_script(&self) -> Script {
        self.app.layouts.lock().unwrap().current_script()
    }
    fn play_sound(&mut self) {
        sys::play_switch_sound();
    }
    fn beep(&mut self) {
        sys::beep();
    }
    fn layout_changed(&mut self) {
        self.app.notify_ui(tray::WM_APP_REFRESH);
    }
    fn log(&mut self, msg: &str) {
        self.app.log(msg);
    }
}

pub fn run() {
    sys::set_dpi_awareness();
    if !sys::single_instance() {
        sys::message_box(l10n::t("already"));
        return;
    }
    let store = Store::open();
    let log = storage::Log::new(&store.dir);
    log.write(&format!("Keyboop {VERSION} для Windows запущен"));

    let mut errors = Vec::new();
    let settings = store.load_settings(&mut errors);
    let exceptions = store.load_exceptions(&mut errors);
    let snippets = store.load_snippets(&mut errors);
    for e in &errors {
        log.write(&format!(
            "файл не прочитан, работаю на значениях по умолчанию: {e}"
        ));
    }
    let state = store.load_state();
    l10n::set_language(&settings.language, sys::system_is_russian());

    // Языковые данные (~5 МБ JSON) грузим в фоне: до готовности движок просто молчит.
    std::thread::spawn(keyboop_core::layout_data::warm_up);

    let last_lat = state
        .last_layouts
        .get("lat")
        .map(|s| layouts::parse_hkl(s))
        .unwrap_or(0);
    let last_cyr = state
        .last_layouts
        .get("cyr")
        .map(|s| layouts::parse_hkl(s))
        .unwrap_or(0);
    let layouts = Layouts::new(last_lat, last_cyr);

    let learn_on_undo = settings.learn_on_undo;
    let mut engine = Engine::new(
        settings,
        exceptions,
        SnippetStore::new(snippets),
        TypoFix::load(state.typo_personal.clone()),
        UndoLearner::new(state.undo.clone(), learn_on_undo),
    );
    engine.rescued_count = state.rescued_count;
    // Граница слова конвертируется прямо в колбэке хука: порядок с нашей синтетикой охраняет
    // забор (hook.rs), а гонки «следующее слово обогнало +30 мс» нет по построению.
    engine.sync_boundary = true;
    engine.keymap_changed();

    let (tx, rx) = channel();
    let state_app = App {
        engine: Mutex::new(engine),
        layouts: Mutex::new(layouts),
        hotkeys: Mutex::new(Hotkeys {
            convert: None,
            switch_layout: None,
            case: None,
        }),
        store,
        log,
        tx: Mutex::new(tx),
        ui_hwnd: AtomicIsize::new(0),
        last_app: Mutex::new(String::new()),
        learn_offers: Mutex::new(Vec::new()),
        saved_mtimes: Mutex::new([None, None, None]),
        start: Instant::now(),
        last_rescued_saved: AtomicU64::new(state.rescued_count),
    };
    let settings = state_app.engine.lock().unwrap().settings.clone();
    state_app.apply_settings(&settings);
    state_app.remember_mtimes();
    if APP.set(state_app).is_err() {
        return;
    }

    std::thread::spawn(move || worker(rx));
    std::thread::spawn(hook::run);
    tray::run(!errors.is_empty());

    // Интерфейс закрылся — выходим, но сперва сохраняемся.
    app().send(Cmd::Quit);
    app().persist();
    app().log("Keyboop завершён");
}

/// Рабочий поток: всё медленное, что нельзя делать в потоке хуков (там каждая миллисекунда
/// задерживает ввод всей системы): чтение выделения через буфер обмена и запись файлов.
fn worker(rx: Receiver<Cmd>) {
    let app = app();
    loop {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Cmd::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Cmd::ReadSelection(kind)) => {
                let sel = clipboard::read_selection();
                *hook::SELECTION_RESULT.lock().unwrap() = Some((kind, sel));
                hook::post_selection_ready();
            }
            Ok(Cmd::Persist) | Err(RecvTimeoutError::Timeout) => app.persist(),
        }
    }
}
