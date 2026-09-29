//! Значок в трее: показывает текущую раскладку (EN/RU) и открывает меню — аналог значка в
//! строке меню macOS. Настройки, которых нет в меню, правятся в `settings.json` (Блокнотом из
//! меню же), файл перечитывается сам.

use super::layouts;
use super::sys::{self, wide};
use super::{app, Cmd, VERSION};
use crate::l10n::t;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_TRAY: u32 = WM_APP + 1;
pub const WM_APP_REFRESH: u32 = WM_APP + 2;
pub const WM_APP_LEARN: u32 = WM_APP + 3;
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;
const TIMER_POLL: usize = 1;

const ID_AUTO: usize = 100;
const ID_LIVE: usize = 101;
const ID_SOUND: usize = 102;
const ID_TYPO: usize = 103;
const ID_TWOCAPS: usize = 104;
const ID_DEV: usize = 105;
const ID_GROUP: usize = 106;
const ID_PAUSE: usize = 110; // +0..3 — длительности, +4 — снять
const ID_LEARN: usize = 120; // +индекс предложения
const ID_APP_MODE: usize = 130; // +0 обычный, +1 мягкий, +2 выкл
const ID_APP_LAYOUT: usize = 133; // +0 не трогать, +1 en, +2 ru
const ID_OPEN_SETTINGS: usize = 140;
const ID_OPEN_EXCEPTIONS: usize = 141;
const ID_OPEN_SNIPPETS: usize = 142;
const ID_OPEN_FOLDER: usize = 143;
const ID_RELOAD: usize = 144;
const ID_AUTOSTART: usize = 150;
const ID_UI_SETTINGS: usize = 145;
const ID_UI_HISTORY: usize = 146;
const ID_UI_FEEDBACK: usize = 147;
const ID_VOICE: usize = 107;
const ID_CALL_STOP: usize = 108;
const ID_QUIT: usize = 199;

const PAUSE_MINUTES: [u32; 4] = [15, 60, 180, 300];

static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static BADGE: Mutex<String> = Mutex::new(String::new());
static BALLOON_WORD: Mutex<Option<String>> = Mutex::new(None);

fn hwnd() -> HWND {
    app().ui_hwnd.load(Ordering::Relaxed) as HWND
}

fn wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn copy_into(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

fn base_data() -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd();
    d.uID = 1;
    d
}

/// Состояние для значка: подпись раскладки и цвет.
fn badge_state() -> (String, u32, String) {
    let hkl = layouts::foreground_hkl().0;
    let script = layouts::script_of(hkl);
    let label = layouts::short_name(hkl);
    let (auto, paused) = {
        let e = app().engine.lock().unwrap();
        (e.settings.auto_enabled, e.settings.is_paused(wall()))
    };
    // COLORREF = 0x00BBGGRR.
    let color = if paused || !auto {
        0x00_3C_8C_E0 // оранжевый: авто не работает
    } else {
        match script {
            keyboop_core::Script::Cyrillic => 0x00_EB_63_25, // синий
            keyboop_core::Script::Latin => 0x00_51_41_37,    // графит
            keyboop_core::Script::Other => 0x00_88_96_0D,    // бирюзовый
        }
    };
    if super::voice::call::is_recording() {
        return (label, 0x00_2D_2D_D9, t("call.tip").to_string()); // красный: идёт запись
    }
    let status = if paused {
        t("paused").to_string()
    } else if !auto {
        format!("{} — off", t("auto"))
    } else {
        t("auto").to_string()
    };
    (label, color, format!("Keyboop {VERSION} · {status}"))
}

/// Нарисовать значок: цветная плашка с кодом языка.
fn make_icon(label: &str, color: u32) -> HICON {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -size; // сверху вниз
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color_bmp =
            CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let old = SelectObject(dc, color_bmp);
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        const KEY: u32 = 0x00FF00FF; // «прозрачный» цвет вне плашки
        px.fill(KEY);
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_NULL, 0, 0);
        let (ob, op) = (SelectObject(dc, brush), SelectObject(dc, pen));
        let r = (size / 4).max(3);
        RoundRect(dc, 0, 0, size + 1, size + 1, r, r);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        DeleteObject(brush);
        DeleteObject(pen);
        let face = wide("Segoe UI");
        let height = -(size * if label.chars().count() > 2 { 45 } else { 60 } / 100);
        let font = CreateFontW(
            height,
            0,
            0,
            0,
            FW_BOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        );
        let of = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00FFFFFF);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        let mut text: Vec<u16> = label.encode_utf16().collect();
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            text.len() as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, of);
        DeleteObject(font);
        GdiFlush();
        for p in px.iter_mut() {
            *p = if *p & 0x00FF_FFFF == KEY {
                0
            } else {
                *p | 0xFF00_0000
            };
        }
        SelectObject(dc, old);
        let mask = CreateBitmap(size, size, 1, 1, std::ptr::null());
        let info = ICONINFO {
            fIcon: TRUE,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color_bmp,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color_bmp);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        icon
    }
}

/// Обновить значок, если что-то поменялось (или принудительно).
fn refresh(force_add: bool) {
    let (label, color, tip) = badge_state();
    let key = format!("{label}|{color}|{tip}");
    {
        let mut b = BADGE.lock().unwrap();
        if !force_add && *b == key {
            return;
        }
        *b = key;
    }
    let mut d = base_data();
    d.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    d.uCallbackMessage = WM_APP_TRAY;
    d.hIcon = make_icon(&label, color);
    copy_into(&mut d.szTip, &tip);
    unsafe {
        if force_add || Shell_NotifyIconW(NIM_MODIFY, &d) == 0 {
            Shell_NotifyIconW(NIM_ADD, &d);
        }
        DestroyIcon(d.hIcon);
    }
}

/// Всплывающее уведомление у значка.
pub fn balloon(title: &str, body: &str) {
    let mut d = base_data();
    d.uFlags = NIF_INFO;
    d.dwInfoFlags = NIIF_INFO;
    copy_into(&mut d.szInfoTitle, title);
    copy_into(&mut d.szInfo, body);
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &d);
    }
}

fn show_learn_balloon() {
    let word = app().learn_offers.lock().unwrap().last().cloned();
    if let Some(w) = word {
        *BALLOON_WORD.lock().unwrap() = Some(w.clone());
        balloon(
            t("learn.title"),
            &format!("{}{}{}", t("learn.body"), w, t("learn.end")),
        );
    }
}

fn confirm_learn(word: &str) {
    let app = app();
    app.engine.lock().unwrap().confirm_learn(word);
    app.learn_offers.lock().unwrap().retain(|w| w != word);
    app.send(Cmd::Persist);
    app.log(&format!(
        "undo-learn: слово добавлено в исключения (len {})",
        word.chars().count()
    ));
}

struct Menu(HMENU);

impl Menu {
    fn new() -> Menu {
        Menu(unsafe { CreatePopupMenu() })
    }
    fn item(&self, id: usize, text: &str, checked: bool, enabled: bool) {
        let w = wide(text);
        let mut f = MF_STRING;
        if checked {
            f |= MF_CHECKED;
        }
        if !enabled {
            f |= MF_GRAYED;
        }
        unsafe {
            AppendMenuW(self.0, f, id, w.as_ptr());
        }
    }
    fn sep(&self) {
        unsafe {
            AppendMenuW(self.0, MF_SEPARATOR, 0, std::ptr::null());
        }
    }
    fn sub(&self, text: &str, sub: Menu) {
        let w = wide(text);
        unsafe {
            AppendMenuW(self.0, MF_STRING | MF_POPUP, sub.0 as usize, w.as_ptr());
        }
        // Подменю уничтожится вместе с родителем (DestroyMenu рекурсивен), своего Drop у Menu нет.
        let _ = sub;
    }
}

fn show_menu() {
    let app = app();
    let (s, rescued) = {
        let e = app.engine.lock().unwrap();
        (e.settings.clone(), e.rescued_count)
    };
    let paused = s.is_paused(wall());
    let m = Menu::new();
    m.item(
        0,
        &format!("Keyboop {VERSION} · {}{}", t("rescued"), rescued),
        false,
        false,
    );
    let hk: Vec<String> = [&s.hotkey_convert, &s.hotkey_switch_layout, &s.hotkey_case]
        .into_iter()
        .filter(|h| !h.is_empty())
        .cloned()
        .collect();
    if !hk.is_empty() {
        m.item(
            0,
            &format!("{}{}", t("hotkeys"), hk.join(" · ")),
            false,
            false,
        );
    }
    m.sep();
    if super::voice::call::is_recording() {
        m.item(ID_CALL_STOP, t("call.stop"), false, true);
        m.sep();
    }
    let offers = app.learn_offers.lock().unwrap().clone();
    for (i, w) in offers.iter().enumerate().take(5) {
        m.item(
            ID_LEARN + i,
            &format!("{}{}{}", t("learn"), w, t("learn.end")),
            false,
            true,
        );
    }
    if !offers.is_empty() {
        m.sep();
    }
    m.item(ID_AUTO, t("auto"), s.auto_enabled, true);
    m.item(ID_LIVE, t("live"), s.live_fix_enabled, s.auto_enabled);
    m.item(ID_SOUND, t("sound"), s.sound_enabled, true);
    m.item(ID_VOICE, t("voice"), s.voice_enabled, true);
    m.item(ID_TYPO, t("typo"), s.typo_fix, true);
    m.item(ID_TWOCAPS, t("twocaps"), s.two_caps_fix, true);
    m.item(ID_DEV, t("dev"), s.developer_mode, true);
    m.item(ID_GROUP, t("group"), s.group_convert, !s.auto_enabled);

    let pause = Menu::new();
    for (i, min) in PAUSE_MINUTES.iter().enumerate() {
        pause.item(ID_PAUSE + i, t(&format!("pause.{min}")), false, true);
    }
    pause.item(ID_PAUSE + 4, t("pause.stop"), false, paused);
    m.sub(
        &if paused {
            format!("{} ✓", t("pause"))
        } else {
            t("pause").to_string()
        },
        pause,
    );

    let last_app = app.last_app.lock().unwrap().clone();
    if !last_app.is_empty() {
        let (mode, layout) = {
            let e = app.engine.lock().unwrap();
            (
                e.exceptions.app_mode(&last_app),
                e.exceptions.app_layout(&last_app).unwrap_or("").to_string(),
            )
        };
        let sub = Menu::new();
        sub.item(ID_APP_MODE, t("app.normal"), mode.is_empty(), true);
        sub.item(ID_APP_MODE + 1, t("app.soft"), mode == "soft", true);
        sub.item(ID_APP_MODE + 2, t("app.off"), mode == "off", true);
        sub.sep();
        sub.item(ID_APP_LAYOUT, t("app.layout.none"), layout.is_empty(), true);
        sub.item(ID_APP_LAYOUT + 1, t("app.layout.en"), layout == "en", true);
        sub.item(ID_APP_LAYOUT + 2, t("app.layout.ru"), layout == "ru", true);
        m.sub(&format!("{}{}", t("app"), last_app), sub);
    }
    m.sep();
    m.item(ID_UI_SETTINGS, t("open.ui"), false, true);
    m.item(ID_UI_HISTORY, t("open.history"), false, true);
    m.item(ID_UI_FEEDBACK, t("open.feedback"), false, true);
    // Файлы — для тех, кто правит настройки руками.
    let files = Menu::new();
    files.item(ID_OPEN_SETTINGS, t("open.settings"), false, true);
    files.item(ID_OPEN_EXCEPTIONS, t("open.exceptions"), false, true);
    files.item(ID_OPEN_SNIPPETS, t("open.snippets"), false, true);
    files.item(ID_OPEN_FOLDER, t("open.folder"), false, true);
    files.item(ID_RELOAD, t("reload"), false, true);
    m.sub(t("files"), files);
    m.sep();
    m.item(ID_AUTOSTART, t("autostart"), sys::autostart_enabled(), true);
    m.item(ID_QUIT, t("quit"), false, true);

    let cmd = unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd());
        let cmd = TrackPopupMenu(
            m.0,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd(),
            std::ptr::null(),
        );
        PostMessageW(hwnd(), WM_NULL, 0, 0);
        DestroyMenu(m.0);
        cmd as usize
    };
    if cmd != 0 {
        on_command(cmd, &offers, &last_app);
    }
}

fn on_command(cmd: usize, offers: &[String], last_app: &str) {
    let app = app();
    let toggle = |f: fn(&mut keyboop_core::Settings)| {
        f(&mut app.engine.lock().unwrap().settings);
        app.save_settings();
    };
    match cmd {
        ID_AUTO => toggle(|s| s.auto_enabled = !s.auto_enabled),
        ID_LIVE => toggle(|s| s.live_fix_enabled = !s.live_fix_enabled),
        ID_SOUND => toggle(|s| s.sound_enabled = !s.sound_enabled),
        ID_TYPO => toggle(|s| s.typo_fix = !s.typo_fix),
        ID_TWOCAPS => toggle(|s| s.two_caps_fix = !s.two_caps_fix),
        ID_DEV => toggle(|s| s.developer_mode = !s.developer_mode),
        ID_GROUP => toggle(|s| s.group_convert = !s.group_convert),
        c if (ID_PAUSE..ID_PAUSE + 4).contains(&c) => {
            let until = wall() + PAUSE_MINUTES[c - ID_PAUSE] as f64 * 60.0;
            app.engine.lock().unwrap().settings.paused_until = until;
            app.save_settings();
            app.log(&format!("пауза: {} мин", PAUSE_MINUTES[c - ID_PAUSE]));
        }
        c if c == ID_PAUSE + 4 => toggle(|s| s.paused_until = 0.0),
        c if (ID_LEARN..ID_LEARN + 5).contains(&c) => {
            if let Some(w) = offers.get(c - ID_LEARN) {
                confirm_learn(w);
            }
        }
        c if (ID_APP_MODE..ID_APP_MODE + 3).contains(&c) => {
            let mode = ["normal", "soft", "off"][c - ID_APP_MODE];
            let mut e = app.engine.lock().unwrap();
            e.exceptions
                .app_modes
                .insert(last_app.to_string(), mode.to_string());
            e.exceptions_dirty = true;
            drop(e);
            app.send(Cmd::Persist);
        }
        c if (ID_APP_LAYOUT..ID_APP_LAYOUT + 3).contains(&c) => {
            let mut e = app.engine.lock().unwrap();
            match c - ID_APP_LAYOUT {
                0 => {
                    e.exceptions.app_layouts.remove(last_app);
                }
                1 => {
                    e.exceptions
                        .app_layouts
                        .insert(last_app.to_string(), "en".into());
                }
                _ => {
                    e.exceptions
                        .app_layouts
                        .insert(last_app.to_string(), "ru".into());
                }
            }
            e.exceptions_dirty = true;
            drop(e);
            app.send(Cmd::Persist);
        }
        ID_VOICE => toggle(|s| s.voice_enabled = !s.voice_enabled),
        ID_CALL_STOP => {
            std::thread::spawn(super::voice::call::stop);
        }
        ID_UI_SETTINGS => crate::ui::spawn("settings", None),
        ID_UI_HISTORY => crate::ui::spawn("history", None),
        ID_UI_FEEDBACK => crate::ui::spawn("feedback", None),
        ID_OPEN_SETTINGS => sys::open_in_notepad(&app.store.path(crate::storage::SETTINGS)),
        ID_OPEN_EXCEPTIONS => {
            app.persist();
            sys::open_in_notepad(&app.store.path(crate::storage::EXCEPTIONS))
        }
        ID_OPEN_SNIPPETS => sys::open_in_notepad(&app.store.path(crate::storage::SNIPPETS)),
        ID_OPEN_FOLDER => sys::open_folder(&app.store.dir),
        ID_RELOAD => app.reload_if_changed(true),
        ID_AUTOSTART => sys::set_autostart(!sys::autostart_enabled()),
        ID_QUIT => unsafe {
            DestroyWindow(hwnd());
        },
        _ => {}
    }
    refresh(false);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            match lparam as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(),
                // Shift+щелчок — запись звонка (на Маке ⌥-щелчок), скрытая функция.
                WM_LBUTTONUP
                    if super::input::is_down(
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT,
                    ) =>
                {
                    super::voice::call::toggle();
                    refresh(false);
                }
                WM_LBUTTONUP => tray_click(),
                NIN_BALLOONUSERCLICK => {
                    if super::voice::call::ASKED_TO_STOP.swap(false, Ordering::Relaxed) {
                        std::thread::spawn(super::voice::call::stop);
                    } else if super::update::OFFERED.swap(false, Ordering::Relaxed) {
                        super::update::install_offer();
                    } else if let Some(w) = BALLOON_WORD.lock().unwrap().take() {
                        confirm_learn(&w);
                    }
                }
                _ => {}
            }
            0
        }
        WM_APP_REFRESH => {
            refresh(false);
            0
        }
        WM_COPYDATA => {
            let cds = &*(lparam as *const windows_sys::Win32::System::DataExchange::COPYDATASTRUCT);
            if cds.dwData == crate::ui::ipc::MAGIC && !cds.lpData.is_null() {
                let bytes =
                    std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                on_ipc(&String::from_utf8_lossy(bytes));
            }
            1
        }
        WM_APP_LEARN => {
            show_learn_balloon();
            0
        }
        WM_TIMER => {
            static TICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            refresh(false);
            super::clipboard::watch_tick();
            let led = app().engine.lock().unwrap().settings.caps_led_indicator;
            let russian =
                layouts::script_of(layouts::foreground_hkl().0) == keyboop_core::Script::Cyrillic;
            super::caps_led::sync(led, russian);
            if TICKS.fetch_add(1, Ordering::Relaxed) % 5 == 4 {
                app().reload_if_changed(false);
            }
            0
        }
        WM_DESTROY => {
            let d = base_data();
            Shell_NotifyIconW(NIM_DELETE, &d);
            PostQuitMessage(0);
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            // Проводник перезапустился — значок надо добавить заново.
            refresh(true);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Команда из окна (отдельный процесс).
fn on_ipc(cmd: &str) {
    let app = app();
    match cmd {
        "reload" => app.reload_if_changed(true),
        "hotkeys-off" => super::hook::HOTKEYS_SUSPENDED.store(true, Ordering::Relaxed),
        "hotkeys-on" => super::hook::HOTKEYS_SUSPENDED.store(false, Ordering::Relaxed),
        "preload" => super::voice::send(super::voice::VoiceCmd::Preload),
        "history" => super::history_store::reload(),
        "check-updates" => app.send(super::Cmd::CheckUpdates),
        "import-cancel" => super::voice::import::CANCEL.store(true, Ordering::Relaxed),
        c if c.starts_with("import:") => {
            let path = std::path::PathBuf::from(&c["import:".len()..]);
            let label = path.file_name().map(|n| n.to_string_lossy().into_owned());
            super::voice::send(super::voice::VoiceCmd::Import(super::voice::import::Job {
                sources: vec![path],
                kind: keyboop_core::history::HistoryKind::Imported,
                label,
                cleanup: None,
            }));
        }
        _ => app.log(&format!("ipc: неизвестная команда {cmd}")),
    }
    refresh(false);
}

/// Щелчок левой кнопкой по значку: действие из настроек (по умолчанию — меню).
fn tray_click() {
    let action = app().engine.lock().unwrap().settings.tray_click.clone();
    match action.as_str() {
        "settings" => crate::ui::spawn("settings", None),
        "history" => crate::ui::spawn("history", None),
        "dictate" => super::voice::send(super::voice::VoiceCmd::Toggle),
        "pause" => {
            app().engine.lock().unwrap().settings.paused_until = wall() + 15.0 * 60.0;
            app().save_settings();
            refresh(false);
        }
        _ => show_menu(),
    }
}

/// Поток интерфейса: окно-невидимка, значок, таймер опроса раскладки. Возвращается при выходе.
pub fn run(config_errors: bool) {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide("KeyboopTray");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let title = wide("Keyboop");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        app().ui_hwnd.store(hwnd as isize, Ordering::Relaxed);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            Ordering::Relaxed,
        );
        refresh(true);
        if config_errors {
            balloon(t("config.error"), t("config.error.body"));
        }
        // Первый запуск — окно-приветствие.
        if !app().engine.lock().unwrap().settings.did_show_welcome {
            crate::ui::spawn("welcome", None);
        }
        // Опрос раскладки активного окна: человек переключает её и сам (Alt+Shift, Win+Space).
        SetTimer(hwnd, TIMER_POLL, 400, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
