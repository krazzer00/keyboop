//! Синтетический ввод через `SendInput`: Backspace'ы и печать Unicode-символов.
//! Буфер обмена для этого НЕ нужен — главное отличие Keyboop от Punto Switcher.
//!
//! Каждое наше событие помечено в `dwExtraInfo`, чтобы хук отличал его от настоящих нажатий.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

/// Наша синтетика замены: хук её пропускает и по первой такой снимает «забор» (см. hook.rs).
pub const MARK_SYNTH: usize = 0x4B42_5331; // "KBS1"
/// Переигранные настоящие нажатия: хук обрабатывает их как настоящие.
pub const MARK_REPLAY: usize = 0x4B42_5250; // "KBRP"
/// Неназначенная клавиша: гасит одиночное отпускание Alt/Win (меню окна, «Пуск»).
pub const VK_MASK: u16 = 0xE8;

/// Служебные нажатия (Ctrl+C для чтения выделения, отпускание модификаторов): хук их не видит.
pub const MARK_SILENT: usize = 0x4B42_534C; // "KBSL"

pub fn key(vk: u16, up: bool, mark: usize) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if is_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: mark,
            },
        },
    }
}

/// Событие с точными полями исходного нажатия (для переигрывания).
pub fn raw(vk: u16, scan: u16, flags: u32, mark: usize) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: mark,
            },
        },
    }
}

fn unicode(unit: u16, up: bool, mark: usize) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: unit,
                dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: mark,
            },
        },
    }
}

fn is_extended(vk: u16) -> bool {
    matches!(
        vk,
        VK_LEFT
            | VK_RIGHT
            | VK_UP
            | VK_DOWN
            | VK_HOME
            | VK_END
            | VK_PRIOR
            | VK_NEXT
            | VK_INSERT
            | VK_DELETE
            | VK_RCONTROL
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
            | VK_APPS
            | VK_DIVIDE
            | VK_NUMLOCK
    )
}

/// События замены: `delete` Backspace'ов, затем `text`, затем (по желанию) Enter.
pub fn replacement(delete: usize, text: &str, then_return: bool) -> Vec<INPUT> {
    let mut v = Vec::with_capacity(delete * 2 + text.len() * 2 + 2);
    for _ in 0..delete {
        v.push(key(VK_BACK, false, MARK_SYNTH));
        v.push(key(VK_BACK, true, MARK_SYNTH));
    }
    for unit in text.encode_utf16() {
        // \n печатаем клавишей Enter, \t — клавишей Tab: так их понимает любое поле.
        match unit {
            0x0A => {
                v.push(key(VK_RETURN, false, MARK_SYNTH));
                v.push(key(VK_RETURN, true, MARK_SYNTH));
            }
            0x09 => {
                v.push(key(VK_TAB, false, MARK_SYNTH));
                v.push(key(VK_TAB, true, MARK_SYNTH));
            }
            0x0D => {}
            _ => {
                v.push(unicode(unit, false, MARK_SYNTH));
                v.push(unicode(unit, true, MARK_SYNTH));
            }
        }
    }
    if then_return {
        v.push(key(VK_RETURN, false, MARK_SYNTH));
        v.push(key(VK_RETURN, true, MARK_SYNTH));
    }
    v
}

pub fn send(events: &[INPUT]) -> u32 {
    if events.is_empty() {
        return 0;
    }
    unsafe {
        SendInput(
            events.len() as u32,
            events.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    }
}

pub fn is_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// Отпустить зажатые модификаторы (перед нашим Ctrl+C: Shift+Ctrl+C в редакторах — другая
/// команда). Возвращает, что отпустили, чтобы после вернуть как было.
pub fn release_modifiers() -> Vec<u16> {
    let mods = [
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
    ];
    let held: Vec<u16> = mods.into_iter().filter(|&vk| is_down(vk)).collect();
    let mut ev: Vec<INPUT> = held.iter().map(|&vk| key(vk, true, MARK_SILENT)).collect();
    // Отпущенный Alt без другой клавиши открывает меню окна: гасим его нейтральной клавишей.
    if held.iter().any(|&vk| vk == VK_LMENU || vk == VK_RMENU) {
        ev.insert(0, key(VK_MASK, false, MARK_SILENT));
        ev.insert(1, key(VK_MASK, true, MARK_SILENT));
    }
    send(&ev);
    held
}

pub fn ctrl_c() {
    ctrl_key(b'C' as u16);
}

pub fn ctrl_v() {
    ctrl_key(b'V' as u16);
}

/// Ctrl+клавиша. Помечено «тихим»: хук его не разбирает, и если человек назначит на наш хоткей
/// сочетание с Ctrl+V, петли не будет.
fn ctrl_key(vk: u16) {
    send(&[
        key(VK_LCONTROL, false, MARK_SILENT),
        key(vk, false, MARK_SILENT),
        key(vk, true, MARK_SILENT),
        key(VK_LCONTROL, true, MARK_SILENT),
    ]);
}
