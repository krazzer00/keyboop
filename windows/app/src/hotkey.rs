//! Разбор хоткеев из настроек: "Pause", "Ctrl+Shift+K", "Alt+Pause", "CapsLock", или одиночный
//! модификатор-тап: "RCtrl", "RShift", "LAlt", "RAlt", "LWin"… (нажал и отпустил, ничего между).
//!
//! Коды — виртуальные клавиши Windows (VK_*), поэтому модуль переносимый и проверяется тестами.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    /// Обычная клавиша с набором модификаторов (набор должен совпасть точно).
    Key { vk: u32, mods: Mods },
    /// Тап одиночного модификатора: конкретная левая/правая клавиша.
    Tap { vk: u32 },
}

pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12;
pub const VK_LSHIFT: u32 = 0xA0;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_LMENU: u32 = 0xA4;
pub const VK_RMENU: u32 = 0xA5;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;

pub fn is_modifier_vk(vk: u32) -> bool {
    matches!(
        vk,
        VK_SHIFT
            | VK_CONTROL
            | VK_MENU
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_LMENU
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
    )
}

fn named_key(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    let vk = match n.as_str() {
        "pause" | "break" => 0x13,
        "capslock" | "caps" => 0x14,
        "scrolllock" | "scroll" => 0x91,
        "insert" | "ins" => 0x2D,
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "apps" | "menu" => 0x5D,
        "`" | "~" | "grave" => 0xC0,
        "-" | "minus" => 0xBD,
        "=" | "plus" => 0xBB,
        "[" => 0xDB,
        "]" => 0xDD,
        "\\" => 0xDC,
        ";" => 0xBA,
        "'" => 0xDE,
        "," | "comma" => 0xBC,
        "." | "period" => 0xBE,
        "/" | "slash" => 0xBF,
        _ => {
            if let Some(num) = n.strip_prefix('f').and_then(|x| x.parse::<u32>().ok()) {
                if (1..=24).contains(&num) {
                    return Some(0x70 + num - 1);
                }
                return None;
            }
            let mut ch = n.chars();
            match (ch.next(), ch.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u32,
                _ => return None,
            }
        }
    };
    Some(vk)
}

fn tap_key(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "lctrl" | "leftctrl" => VK_LCONTROL,
        "rctrl" | "rightctrl" => VK_RCONTROL,
        "lshift" | "leftshift" => VK_LSHIFT,
        "rshift" | "rightshift" => VK_RSHIFT,
        "lalt" | "leftalt" => VK_LMENU,
        "ralt" | "rightalt" | "altgr" => VK_RMENU,
        "lwin" => VK_LWIN,
        "rwin" => VK_RWIN,
        _ => return None,
    })
}

/// Пустая строка — хоткей выключен (`None`). Ошибка — текст для лога.
pub fn parse(spec: &str) -> Result<Option<Hotkey>, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    if parts.len() == 1 {
        if let Some(vk) = tap_key(parts[0]) {
            return Ok(Some(Hotkey::Tap { vk }));
        }
    }
    let mut mods = Mods::default();
    let mut key = None;
    for p in &parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            "win" | "meta" => mods.win = true,
            "" => return Err(format!("хоткей «{spec}»: пустая часть")),
            other => {
                if key.is_some() {
                    return Err(format!("хоткей «{spec}»: больше одной основной клавиши"));
                }
                key = Some(
                    named_key(other)
                        .ok_or_else(|| format!("хоткей «{spec}»: не знаю клавишу «{other}»"))?,
                );
            }
        }
    }
    match key {
        Some(vk) => Ok(Some(Hotkey::Key { vk, mods })),
        None => Err(format!("хоткей «{spec}»: нет основной клавиши")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(
            parse("Pause").unwrap(),
            Some(Hotkey::Key {
                vk: 0x13,
                mods: Mods::default()
            })
        );
        assert_eq!(
            parse("Ctrl+Shift+K").unwrap(),
            Some(Hotkey::Key {
                vk: 0x4B,
                mods: Mods {
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                }
            })
        );
        assert_eq!(
            parse("RCtrl").unwrap(),
            Some(Hotkey::Tap { vk: VK_RCONTROL })
        );
        assert_eq!(
            parse("F12").unwrap(),
            Some(Hotkey::Key {
                vk: 0x7B,
                mods: Mods::default()
            })
        );
        assert_eq!(parse("").unwrap(), None);
        assert!(parse("Ctrl+Nope").is_err());
    }
}
