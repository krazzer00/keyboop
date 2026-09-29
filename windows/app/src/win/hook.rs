//! Поток хуков: низкоуровневые хуки клавиатуры и мыши, слежение за активным окном, таймер
//! отложенной авто-конверсии. ВСЯ работа движка с набором и ВСЯ наша печать (SendInput)
//! происходят здесь, в одном потоке.
//!
//! ⚠️ ПОЧЕМУ ОДИН ПОТОК. Первая версия печатала замену из рабочего потока под мьютексом движка.
//! Там, где SendInput ждёт, пока хуки обработают вставленные события (так ведёт себя Wine), это
//! взаимная блокировка: рабочий поток держит движок и ждёт хук, а хук ждёт движок. Поймано
//! сквозным тестом: второе слово зависало. В одном потоке ждать некому.
//!
//! ⚠️ «ЗАБОР» ВОКРУГ НАШЕЙ СИНТЕТИКИ. Замена слова — пачка событий SendInput, и она встаёт в
//! очередь ввода ПОСЛЕ всего, что человек уже успел нажать. Если настоящая клавиша K нажата за
//! миг до нашей пачки, приложение получит K раньше наших Backspace'ов, и они сотрут K вместо
//! буквы слова (на Маке ту же гонку ловили Fence A/B). Решение точное: пока первое событие нашей
//! пачки не прошло через хук, каждое настоящее нажатие глотается и копится, а как только пачка
//! пошла — переигрывается следом за ней в исходном порядке. Переигранные нажатия помечены
//! `MARK_REPLAY`, и хук обрабатывает их как обычные.

use super::clipboard::Selection;
use super::input::{self, MARK_REPLAY, MARK_SILENT, MARK_SYNTH};
use super::layouts;
use super::sys::wide;
use super::voice::{self, VoiceCmd};
use super::{app, Cmd, WinPlatform};
use crate::hotkey::{self, Hotkey};
use keyboop_core::engine::SelectionConversion;
use keyboop_core::{Engine, Key, KeyInput, ManualStep};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Результат чтения выделения от рабочего потока готов.
pub const WM_KB_SELECTION: u32 = WM_APP + 10;
/// Есть готовый текст для вставки (диктовка, сниппет, перевод).
pub const WM_KB_INSERT: u32 = WM_APP + 11;

static INSERT_QUEUE: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

/// Напечатать текст там, где курсор. Из любого потока: печать идёт в потоке хуков.
pub fn post_insert(text: String, then_return: bool) {
    INSERT_QUEUE.lock().unwrap().push((text, then_return));
    unsafe {
        PostMessageW(
            HOOK_HWND.load(Ordering::Relaxed) as HWND,
            WM_KB_INSERT,
            0,
            0,
        );
    }
}
/// Окно потока хуков: сюда шлют сообщения и вешают таймеры.
pub fn hook_hwnd() -> HWND {
    HOOK_HWND.load(Ordering::Relaxed) as HWND
}

const TIMER_ENGINE: usize = 1;
const TIMER_WATCHDOG: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionKind {
    Convert,
    Case,
    Translate,
}

pub static SELECTION_RESULT: Mutex<Option<(SelectionKind, Option<Selection>)>> = Mutex::new(None);

struct Fence {
    armed_at: Option<Instant>,
    queue: Vec<INPUT>,
}

static FENCE: Mutex<Fence> = Mutex::new(Fence {
    armed_at: None,
    queue: Vec::new(),
});
/// Если первое событие пачки так и не пришло (Windows не пустила ввод в окно, запущенное от
/// администратора), забор снимается сам, а накопленное переигрывается.
const FENCE_TIMEOUT: Duration = Duration::from_millis(300);

static HOOK_HWND: AtomicIsize = AtomicIsize::new(0);
static KB_HOOK: AtomicIsize = AtomicIsize::new(0);
static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
static CAPS_ON: AtomicBool = AtomicBool::new(false);

/// Включён ли Caps Lock (по нашему слежению за клавишей).
pub fn caps_on() -> bool {
    CAPS_ON.load(Ordering::Relaxed)
}
/// Время (GetTickCount) последнего колбэка хука — для сторожа, который переустанавливает хуки.
static LAST_HOOK_TICK: AtomicU32 = AtomicU32::new(0);
/// Модификатор, который может оказаться «тапом»-хоткеем (нажат один, ничего между).
static TAP_CANDIDATE: AtomicU32 = AtomicU32::new(0);
/// Клик мышью (время в битах f64; 0 — не было). Хук мыши движок не трогает, клик применяется
/// при следующем обращении к движку — ровно как ленивая очистка в мак-версии.
static CLICK_AT: AtomicU64 = AtomicU64::new(0);
/// Хоткей с модификаторами ждёт, пока модификаторы отпустят: печатать Backspace при зажатом
/// Ctrl значило бы стирать словами.
static PENDING_ACTION: AtomicU8 = AtomicU8::new(0);

pub const ACT_CONVERT: u8 = 1;
pub const ACT_SWITCH: u8 = 2;
pub const ACT_CASE: u8 = 3;
pub const ACT_VOICE_TOGGLE: u8 = 4;
pub const ACT_PASTE_DICTATION: u8 = 5;
pub const ACT_PLAIN_PASTE: u8 = 6;
pub const ACT_SNIPPET_PICK: u8 = 7;
pub const ACT_TRANSLATE: u8 = 8;
/// Окно настроек записывает сочетание: наши хоткеи не перехватываем.
pub static HOTKEYS_SUSPENDED: AtomicBool = AtomicBool::new(false);
/// Клавиша диктовки, которую сейчас держат (0 — не держат).
static VOICE_HOLD: AtomicU32 = AtomicU32::new(0);

struct KeyState {
    down: [bool; 256],
    /// Когда клавиша нажата (время события, мс) — чтобы отличить «вместе с хоткеем» от «до него».
    down_at: [u32; 256],
    /// Клавиши, чьё нажатие мы проглотили: их отпускание тоже глотаем.
    swallowed_up: Vec<u32>,
}

static KEYS: Mutex<KeyState> = Mutex::new(KeyState {
    down: [false; 256],
    down_at: [0; 256],
    swallowed_up: Vec::new(),
});

thread_local! {
    /// Поток держит мьютекс движка. Некоторые реализации (Wine) доставляют колбэк хука прямо
    /// изнутри нашей SendInput, то есть повторным входом; ждать мьютекс там значит зависнуть.
    static ENGINE_HELD: Cell<bool> = const { Cell::new(false) };
}

/// Доступ к движку с платформой. Только на потоке хуков.
fn with_engine<R>(f: impl FnOnce(&mut Engine, &mut WinPlatform) -> R) -> R {
    let app = app();
    let mut e = app.engine.lock().unwrap();
    ENGINE_HELD.with(|h| h.set(true));
    let click = CLICK_AT.swap(0, Ordering::Relaxed);
    if click != 0 {
        e.context_reset(f64::from_bits(click));
    }
    let mut p = WinPlatform { app };
    let r = f(&mut e, &mut p);
    let deadline = e.next_deadline();
    let offers = e.take_learn_suggestions();
    ENGINE_HELD.with(|h| h.set(false));
    drop(e);
    schedule(deadline);
    if !offers.is_empty() {
        app.learn_offers.lock().unwrap().extend(offers);
        app.notify_ui(super::tray::WM_APP_LEARN);
    }
    r
}

fn schedule(deadline: Option<f64>) {
    let hwnd = HOOK_HWND.load(Ordering::Relaxed) as HWND;
    unsafe {
        match deadline {
            Some(d) => {
                let ms = ((d - app().now()) * 1000.0).ceil().max(1.0) as u32;
                SetTimer(hwnd, TIMER_ENGINE, ms, None);
            }
            None => {
                KillTimer(hwnd, TIMER_ENGINE);
            }
        }
    }
}

/// Отправить пачку замены, подняв забор. Мьютекс забора на время SendInput не держим: колбэк
/// нашей же синтетики может прийти раньше, чем SendInput вернётся.
pub fn send_replacement(events: &[INPUT]) {
    FENCE.lock().unwrap().armed_at = Some(Instant::now());
    if input::send(events) == 0 {
        FENCE.lock().unwrap().armed_at = None; // SendInput отказал целиком — ждать нечего
    }
}

fn to_raw(kb: &KBDLLHOOKSTRUCT, up: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if kb.flags & LLKHF_EXTENDED != 0 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if kb.vkCode == VK_PACKET as u32 {
        return input::raw(
            0,
            kb.scanCode as u16,
            flags | KEYEVENTF_UNICODE,
            MARK_REPLAY,
        );
    }
    input::raw(kb.vkCode as u16, kb.scanCode as u16, flags, MARK_REPLAY)
}

/// Первое событие нашей пачки пришло: снимаем забор и переигрываем накопленное за ней.
fn fence_on_synth() {
    let q = {
        let mut f = FENCE.lock().unwrap();
        if f.armed_at.take().is_none() {
            return;
        }
        std::mem::take(&mut f.queue)
    };
    input::send(&q);
}

/// Настоящее событие при поднятом заборе: в очередь. true — событие проглочено.
fn fence_hold(kb: &KBDLLHOOKSTRUCT, up: bool) -> bool {
    let mut f = FENCE.lock().unwrap();
    let Some(at) = f.armed_at else { return false };
    f.queue.push(to_raw(kb, up));
    if at.elapsed() > FENCE_TIMEOUT {
        f.armed_at = None;
        let q = std::mem::take(&mut f.queue);
        drop(f);
        input::send(&q);
    }
    true
}

fn mods() -> crate::hotkey::Mods {
    use input::is_down;
    crate::hotkey::Mods {
        ctrl: is_down(VK_LCONTROL) || is_down(VK_RCONTROL),
        shift: is_down(VK_LSHIFT) || is_down(VK_RSHIFT),
        alt: is_down(VK_LMENU) || is_down(VK_RMENU),
        win: is_down(VK_LWIN) || is_down(VK_RWIN),
    }
}

fn any_modifier_down() -> bool {
    let ks = KEYS.lock().unwrap();
    ks.down
        .iter()
        .enumerate()
        .any(|(i, d)| *d && hotkey::is_modifier_vk(i as u32))
}

/// Хоткей сработал: сразу, либо когда отпустят модификаторы.
fn fire(action: u8) {
    if any_modifier_down() {
        PENDING_ACTION.store(action, Ordering::Relaxed);
    } else {
        run_action(action);
    }
}

fn run_action(action: u8) {
    match action {
        ACT_CONVERT => {
            if with_engine(|e, p| e.manual_hotkey(p)) == ManualStep::NeedSelection {
                app().send(Cmd::ReadSelection(SelectionKind::Convert));
            }
        }
        ACT_SWITCH => with_engine(|e, p| e.layout_switch_only(p)),
        ACT_CASE => app().send(Cmd::ReadSelection(SelectionKind::Case)),
        ACT_VOICE_TOGGLE => voice::send(VoiceCmd::Toggle),
        ACT_PASTE_DICTATION => {
            let minutes = app().engine.lock().unwrap().settings.voice_history_minutes;
            match super::history_store::last_dictation(minutes) {
                Some(text) => with_engine(|e, p| e.insert_text(&text, false, p)),
                None => super::sys::beep(),
            }
        }
        ACT_PLAIN_PASTE => app().send(Cmd::PlainPaste),
        ACT_SNIPPET_PICK => super::picker::open(),
        ACT_TRANSLATE => app().send(Cmd::ReadSelection(SelectionKind::Translate)),
        _ => {}
    }
}

/// Рабочий поток прочитал выделение: применяем здесь, где живёт вся наша печать.
fn apply_selection() {
    let Some((kind, sel)) = SELECTION_RESULT.lock().unwrap().take() else {
        return;
    };
    match (kind, &sel) {
        (SelectionKind::Convert, Some(s)) => match Engine::convert_selection_text(&s.text) {
            SelectionConversion::Converted { text, to_cyrillic } => with_engine(|e, p| {
                keyboop_core::Platform::replace(p, 0, &text, false);
                e.selection_converted(&s.text, to_cyrillic, p);
            }),
            SelectionConversion::Refused => with_engine(|e, p| e.manual_without_selection(true, p)),
            SelectionConversion::Nothing => {
                with_engine(|e, p| e.manual_without_selection(false, p))
            }
        },
        (SelectionKind::Convert, None) => with_engine(|e, p| e.manual_without_selection(false, p)),
        (SelectionKind::Case, Some(s)) => match Engine::case_changed_text(&s.text) {
            Some(out) => with_engine(|e, p| {
                keyboop_core::Platform::replace(p, 0, &out, false);
                e.buffer.clear();
                if e.settings.sound_enabled {
                    keyboop_core::Platform::play_sound(p);
                }
                keyboop_core::Platform::log(
                    p,
                    &format!("регистр: {} симв.", s.text.chars().count()),
                );
            }),
            None => super::sys::beep(),
        },
        (SelectionKind::Case, None) => super::sys::beep(),
        (SelectionKind::Translate, _) => super::translate::apply(sel.as_ref()),
    }
    if let Some(s) = sel {
        s.restore_later();
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    LAST_HOOK_TICK.store(GetTickCount(), Ordering::Relaxed);
    if code != HC_ACTION as i32 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
    let up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
    match kb.dwExtraInfo {
        MARK_SILENT => return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam),
        MARK_SYNTH => {
            fence_on_synth();
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }
        _ => {}
    }
    if ENGINE_HELD.with(|h| h.get()) {
        // Повторный вход изнутри нашей же SendInput: забор поднят, копим — переиграем после.
        return if fence_hold(kb, up) {
            1
        } else {
            CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
        };
    }
    let swallow = std::panic::catch_unwind(|| handle_key(kb, up)).unwrap_or(false);
    if swallow {
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

/// Обработка настоящего (или переигранного) нажатия. true — проглотить.
fn handle_key(kb: &KBDLLHOOKSTRUCT, up: bool) -> bool {
    if fence_hold(kb, up) {
        return true;
    }
    let vk = kb.vkCode;
    let repeat;
    {
        let mut ks = KEYS.lock().unwrap();
        let i = (vk & 0xFF) as usize;
        repeat = !up && ks.down[i];
        ks.down[i] = !up;
        if !up && !repeat {
            ks.down_at[i] = kb.time;
        }
    }
    let hk = if HOTKEYS_SUSPENDED.load(Ordering::Relaxed) {
        super::Hotkeys::default()
    } else {
        app().hotkeys.lock().unwrap().clone()
    };

    // ── Диктовка: удержание хоткея, Esc, чужое сочетание ────────────────────────────────
    let held = VOICE_HOLD.load(Ordering::Relaxed);
    if held != 0 {
        if up && vk == held {
            VOICE_HOLD.store(0, Ordering::Relaxed);
            voice::send(VoiceCmd::End);
        } else if !up && !repeat && vk == VK_ESCAPE as u32 && hk.esc_cancels {
            VOICE_HOLD.store(0, Ordering::Relaxed);
            voice::send(VoiceCmd::Cancel);
            KEYS.lock().unwrap().swallowed_up.push(vk);
            return true;
        } else if !up && !repeat && vk != held {
            // Хоткей диктовки оказался частью чужого сочетания (правый Alt + буква): не диктовка.
            VOICE_HOLD.store(0, Ordering::Relaxed);
            voice::send(VoiceCmd::Abort);
        }
    } else if !up
        && !repeat
        && vk == VK_ESCAPE as u32
        && hk.esc_cancels
        && voice::ACTIVE.load(Ordering::Relaxed)
    {
        voice::send(VoiceCmd::Cancel);
        KEYS.lock().unwrap().swallowed_up.push(vk);
        return true;
    }

    if up {
        let mut ks = KEYS.lock().unwrap();
        if let Some(i) = ks.swallowed_up.iter().position(|&x| x == vk) {
            ks.swallowed_up.swap_remove(i);
            drop(ks);
            maybe_run_pending();
            return true;
        }
    }

    let entries = hk.actions();

    if up {
        // Тап модификатора: отпущен тот же, что был нажат, и между ними ничего.
        let cand = TAP_CANDIDATE.swap(0, Ordering::Relaxed);
        if cand != 0 && cand == vk {
            for (h, action) in entries {
                if h == Some(Hotkey::Tap { vk }) {
                    fire(action);
                }
            }
        }
        maybe_run_pending();
        return false;
    }

    // «Нажат только этот модификатор»: другие модификаторы, нажатые почти одновременно с ним, не в
    // счёт. Так приходят составные клавиши: AltGr присылает фальшивый левый Ctrl, а Wine правый
    // Alt — парой с левым. Модификатор, зажатый заранее, — это уже сочетание.
    let only_this_down = || {
        let ks = KEYS.lock().unwrap();
        ks.down.iter().enumerate().all(|(i, d)| {
            !*d || i as u32 == vk
                || (hotkey::is_modifier_vk(i as u32) && kb.time.wrapping_sub(ks.down_at[i]) < 100)
        })
    };

    // Диктовка удержанием модификатора (по умолчанию правый Alt): начинаем сразу при нажатии.
    if hk.voice_hold && hk.voice == Some(Hotkey::Tap { vk }) && !repeat && only_this_down() {
        VOICE_HOLD.store(vk, Ordering::Relaxed);
        voice::send(VoiceCmd::Begin);
        // Отпущенный «в одиночку» Alt открыл бы меню окна: гасим нейтральной клавишей.
        if matches!(
            vk,
            crate::hotkey::VK_LMENU
                | crate::hotkey::VK_RMENU
                | crate::hotkey::VK_LWIN
                | crate::hotkey::VK_RWIN
        ) {
            send_mask();
        }
        return false;
    }

    if hotkey::is_modifier_vk(vk) {
        if !repeat {
            let tap = entries.iter().any(|(h, _)| *h == Some(Hotkey::Tap { vk }));
            TAP_CANDIDATE.store(
                if tap && only_this_down() { vk } else { 0 },
                Ordering::Relaxed,
            );
        }
        return false;
    }
    TAP_CANDIDATE.store(0, Ordering::Relaxed);

    let m = mods();

    // Открыт список сниппетов: цифра выбирает, Esc закрывает, остальное закрывает и идёт дальше.
    if let Some(swallow) = super::picker::on_key(vk, m.shift, m.ctrl, m.alt || m.win) {
        if swallow {
            KEYS.lock().unwrap().swallowed_up.push(vk);
            return true;
        }
    }

    // Хоткеи-сочетания (набор модификаторов должен совпасть точно).
    if let Some(Hotkey::Key { vk: hvk, mods: hm }) = hk.voice {
        if hvk == vk && hm == m {
            KEYS.lock().unwrap().swallowed_up.push(vk);
            if m.alt || m.win {
                send_mask();
            }
            if !repeat {
                if hk.voice_hold {
                    VOICE_HOLD.store(vk, Ordering::Relaxed);
                    voice::send(VoiceCmd::Begin);
                } else {
                    voice::send(VoiceCmd::Toggle);
                }
            }
            return true;
        }
    }
    for (h, action) in entries {
        if let Some(Hotkey::Key { vk: hvk, mods: hm }) = h {
            if hvk == vk && hm == m {
                KEYS.lock().unwrap().swallowed_up.push(vk);
                // Alt или Win, отпущенные «в одиночку» (саму клавишу мы проглотили), открыли бы
                // меню окна или «Пуск». Гасим это нейтральной клавишей, как делает AutoHotkey.
                if m.alt || m.win {
                    send_mask();
                }
                if !repeat {
                    fire(action);
                }
                return true;
            }
        }
    }

    if vk == VK_CAPITAL as u32 {
        if !repeat {
            CAPS_ON.fetch_xor(true, Ordering::Relaxed);
        }
        return false;
    }

    let Some(key) = classify(kb, m) else {
        return false;
    };
    let input = KeyInput {
        key,
        shift: m.shift,
        other_mods: m.ctrl || m.alt || m.win,
    };
    let swallow = with_engine(|e, p| e.key_down(input, p));
    if swallow {
        KEYS.lock().unwrap().swallowed_up.push(vk);
    }
    swallow
}

fn send_mask() {
    input::send(&[
        input::key(input::VK_MASK, false, MARK_SILENT),
        input::key(input::VK_MASK, true, MARK_SILENT),
    ]);
}

fn maybe_run_pending() {
    if PENDING_ACTION.load(Ordering::Relaxed) != 0 && !any_modifier_down() {
        let a = PENDING_ACTION.swap(0, Ordering::Relaxed);
        run_action(a);
    }
}

/// Что это за нажатие для движка.
fn classify(kb: &KBDLLHOOKSTRUCT, m: crate::hotkey::Mods) -> Option<Key> {
    let vk = kb.vkCode as u16;
    let cmd = m.ctrl || m.alt || m.win;
    let altgr = m.ctrl && m.alt && !m.win;
    Some(match vk {
        VK_BACK => {
            if m.ctrl {
                Key::DeleteWord
            } else if cmd {
                Key::Shortcut
            } else {
                Key::Backspace
            }
        }
        VK_RETURN => Key::Enter,
        VK_SPACE | VK_TAB if cmd => Key::Shortcut,
        VK_SPACE => Key::Space,
        VK_TAB => Key::Tab,
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => Key::Arrow,
        VK_ESCAPE | VK_DELETE | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_INSERT => Key::Nav,
        VK_PACKET => match char::from_u32(kb.scanCode) {
            Some(c) if !c.is_control() => Key::Char(c.to_string()),
            _ => Key::Other,
        },
        _ => {
            if cmd && !altgr {
                return Some(Key::Shortcut);
            }
            let hkl = app().layouts.lock().unwrap().effective();
            let mut state = [0u8; 256];
            if m.shift {
                state[VK_SHIFT as usize] = 0x80;
                state[VK_LSHIFT as usize] = 0x80;
            }
            if CAPS_ON.load(Ordering::Relaxed) {
                state[VK_CAPITAL as usize] = 0x01;
            }
            if altgr {
                state[VK_CONTROL as usize] = 0x80;
                state[VK_MENU as usize] = 0x80;
            }
            match layouts::translate_with_state(vk, &state, hkl) {
                Some(s) if s.chars().all(|c| !c.is_control()) => Key::Char(s),
                _ if altgr => Key::Shortcut,
                _ => Key::Other,
            }
        }
    })
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    LAST_HOOK_TICK.store(GetTickCount(), Ordering::Relaxed);
    if code == HC_ACTION as i32 {
        let ms = &*(lparam as *const MSLLHOOKSTRUCT);
        let click = matches!(
            wparam as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        );
        if ms.dwExtraInfo != MARK_SYNTH && super::picker::is_open() {
            let wheel = (ms.mouseData >> 16) as i16;
            let ours = std::panic::catch_unwind(|| {
                super::picker::on_mouse(wparam as u32, ms.pt.x, ms.pt.y, wheel)
            })
            .unwrap_or(false);
            if ours {
                return 1;
            }
        }
        if click && ms.dwExtraInfo != MARK_SYNTH {
            TAP_CANDIDATE.store(0, Ordering::Relaxed);
            let t = app().now().max(f64::MIN_POSITIVE);
            CLICK_AT.store(t.to_bits(), Ordering::Relaxed);
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn foreground_proc(
    _: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let _ = std::panic::catch_unwind(|| {
        let exe = super::sys::exe_name_of_window(hwnd);
        if exe.is_empty() || super::sys::is_own_window(hwnd) {
            return; // своё меню в трее не считается сменой программы
        }
        super::picker::hide(); // человек ушёл в другую программу: список сниппетов уже не к месту
        let app = app();
        *app.last_app.lock().unwrap() = exe.clone();
        let elevated = super::sys::is_elevated_foreign(hwnd);
        with_engine(|e, p| {
            if app.layouts.lock().unwrap().refresh() {
                e.keymap_changed();
                app.log("раскладки: таблица символов перестроена");
            }
            e.foreground_changed(&exe, p);
            if elevated {
                e.force_front_app_off();
            }
        });
        if elevated {
            app.log(&format!("{exe}: запущена от администратора — в ней не переключаю (Windows не пускает туда ввод)"));
        }
        app.notify_ui(super::tray::WM_APP_REFRESH);
    });
}

fn install() {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        for slot in [&KB_HOOK, &MOUSE_HOOK] {
            let old = slot.swap(0, Ordering::Relaxed);
            if old != 0 {
                UnhookWindowsHookEx(old as HHOOK);
            }
        }
        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hinst, 0);
        let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hinst, 0);
        KB_HOOK.store(kb as isize, Ordering::Relaxed);
        MOUSE_HOOK.store(ms as isize, Ordering::Relaxed);
        LAST_HOOK_TICK.store(GetTickCount(), Ordering::Relaxed);
        if kb.is_null() {
            app().log(&format!(
                "хук клавиатуры не установлен: ошибка {}",
                GetLastError()
            ));
        }
    }
}

/// Сторож: Windows молча снимает хук, если колбэк однажды ответил дольше порога. Если человек
/// что-то вводит, а наши колбэки молчат — переустанавливаем.
fn watchdog() {
    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut lii) == 0 {
            return;
        }
        let now = GetTickCount();
        let since_hook = lii
            .dwTime
            .wrapping_sub(LAST_HOOK_TICK.load(Ordering::Relaxed));
        if now.wrapping_sub(lii.dwTime) < 2000 && since_hook > 1500 && since_hook < 0x8000_0000 {
            app().log("сторож: хуки молчат при живом вводе — переустанавливаю");
            install();
        }
    }
}

unsafe extern "system" fn hook_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER if wparam == TIMER_ENGINE => {
            KillTimer(hwnd, TIMER_ENGINE);
            let _ = std::panic::catch_unwind(|| with_engine(|e, p| e.run_due(p)));
            0
        }
        WM_TIMER if wparam == super::picker::TIMER_PICKER => {
            super::picker::hide();
            0
        }
        WM_TIMER if wparam == TIMER_WATCHDOG => {
            watchdog();
            0
        }
        WM_KB_SELECTION => {
            let _ = std::panic::catch_unwind(apply_selection);
            0
        }
        WM_KB_INSERT => {
            let items = std::mem::take(&mut *INSERT_QUEUE.lock().unwrap());
            for (text, enter) in items {
                with_engine(|e, p| e.insert_text(&text, enter, p));
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn post_selection_ready() {
    unsafe {
        PostMessageW(
            HOOK_HWND.load(Ordering::Relaxed) as HWND,
            WM_KB_SELECTION,
            0,
            0,
        );
    }
}

/// Поток хуков: окно для таймеров и сообщений, хуки, очередь сообщений (колбэки приходят сюда).
pub fn run() {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide("KeyboopHook");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(hook_wndproc),
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
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        HOOK_HWND.store(hwnd as isize, Ordering::Relaxed);

        CAPS_ON.store(GetKeyState(VK_CAPITAL as i32) & 1 != 0, Ordering::Relaxed);
        install();
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(foreground_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        // Стартовое окно тоже программа: применяем её режим сразу.
        foreground_proc(std::ptr::null_mut(), 0, GetForegroundWindow(), 0, 0, 0, 0);
        SetTimer(hwnd, TIMER_WATCHDOG, 3000, None);
        app().log("хуки установлены");

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
