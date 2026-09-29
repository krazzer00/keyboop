//! Раскладки Windows (HKL): какая включена у активного окна, переключение, цикл, и живая
//! таблица символов «латиница ↔ кириллица» из самих раскладок через `ToUnicodeEx` — аналог
//! `DynamicKeymap` (там `UCKeyTranslate`). Так «Русская», «Русская (машинопись)» и любая
//! латинская раскладка конвертируются по своим настоящим клавишам, а не по таблице US/ЙЦУКЕН.

use keyboop_core::keymap::{self, Table};
use keyboop_core::Script;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub type Hkl = isize;

/// Сколько доверяем раскладке, которую сами только что попросили включить: окно применяет
/// её асинхронно, и следующее нажатие надо декодировать уже в ней.
const OWN_SELECT_GRACE: Duration = Duration::from_millis(400);

pub struct Layouts {
    list: Vec<Hkl>,
    requested: Option<(Hkl, Instant)>,
    pub last_lat: Hkl,
    pub last_cyr: Hkl,
    table_pair: (Hkl, Hkl),
}

/// Печатные клавиши: буквы, цифры, знаки (порядок — как в ряду клавиатуры).
const PRINTABLE_VKS: &[u16] = &[
    0x41,
    0x42,
    0x43,
    0x44,
    0x45,
    0x46,
    0x47,
    0x48,
    0x49,
    0x4A,
    0x4B,
    0x4C,
    0x4D,
    0x4E,
    0x4F,
    0x50,
    0x51,
    0x52,
    0x53,
    0x54,
    0x55,
    0x56,
    0x57,
    0x58,
    0x59,
    0x5A, // A…Z
    0x30,
    0x31,
    0x32,
    0x33,
    0x34,
    0x35,
    0x36,
    0x37,
    0x38,
    0x39, // 0…9
    VK_OEM_1,
    VK_OEM_PLUS,
    VK_OEM_COMMA,
    VK_OEM_MINUS,
    VK_OEM_PERIOD,
    VK_OEM_2,
    VK_OEM_3,
    VK_OEM_4,
    VK_OEM_5,
    VK_OEM_6,
    VK_OEM_7,
    VK_OEM_8,
    VK_OEM_102,
];

fn hkl_of(p: Hkl) -> HKL {
    p as HKL
}

/// Символ клавиши в раскладке: с шифтом или без. Мёртвая клавиша отдаёт свой базовый символ.
pub fn translate(vk: u16, shift: bool, caps: bool, hkl: Hkl) -> Option<String> {
    let mut state = [0u8; 256];
    if shift {
        state[VK_SHIFT as usize] = 0x80;
        state[VK_LSHIFT as usize] = 0x80;
    }
    if caps {
        state[VK_CAPITAL as usize] = 0x01;
    }
    translate_with_state(vk, &state, hkl)
}

pub fn translate_with_state(vk: u16, state: &[u8; 256], hkl: Hkl) -> Option<String> {
    let mut buf = [0u16; 8];
    unsafe {
        let scan = MapVirtualKeyExW(vk as u32, MAPVK_VK_TO_VSC, hkl_of(hkl));
        // wFlags = 4: не трогать состояние клавиатуры ядра (Windows 10 1607+), иначе мы бы ломали
        // человеку мёртвые клавиши, вызывая ToUnicodeEx из хука.
        let n = ToUnicodeEx(
            vk as u32,
            scan,
            state.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            4,
            hkl_of(hkl),
        );
        let len = match n {
            n if n > 0 => n as usize,
            -1 => 1, // мёртвая клавиша: в буфере её «пробельный» вариант
            _ => return None,
        };
        String::from_utf16(&buf[..len])
            .ok()
            .filter(|s| !s.is_empty())
    }
}

pub fn script_of(hkl: Hkl) -> Script {
    match translate(0x41, false, false, hkl).and_then(|s| s.chars().next()) {
        Some(c) if keyboop_core::text::is_cyrillic(c) => Script::Cyrillic,
        Some(c) if c.is_ascii_alphabetic() => Script::Latin,
        _ => Script::Other,
    }
}

pub fn foreground_hkl() -> (Hkl, HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        (GetKeyboardLayout(tid) as Hkl, fg)
    }
}

fn system_list() -> Vec<Hkl> {
    unsafe {
        let n = GetKeyboardLayoutList(0, std::ptr::null_mut());
        if n <= 0 {
            return Vec::new();
        }
        let mut v: Vec<HKL> = vec![std::ptr::null_mut(); n as usize];
        let got = GetKeyboardLayoutList(n, v.as_mut_ptr());
        v.truncate(got.max(0) as usize);
        v.into_iter().map(|h| h as Hkl).collect()
    }
}

impl Layouts {
    pub fn new(last_lat: Hkl, last_cyr: Hkl) -> Self {
        let mut l = Layouts {
            list: Vec::new(),
            requested: None,
            last_lat,
            last_cyr,
            table_pair: (0, 0),
        };
        l.refresh();
        l
    }

    /// Перечитать список раскладок. true — живая таблица символов перестроена.
    pub fn refresh(&mut self) -> bool {
        self.list = system_list();
        let lat = self.pick(Script::Latin);
        let cyr = self.pick(Script::Cyrillic);
        match (lat, cyr) {
            (Some(l), Some(c)) if (l, c) != self.table_pair => {
                self.table_pair = (l, c);
                let (e2r, r2e) = build_tables(l, c);
                keymap::set_live_tables(e2r, r2e);
                true
            }
            (Some(_), Some(_)) => false,
            _ => {
                if self.table_pair != (0, 0) {
                    self.table_pair = (0, 0);
                    keymap::clear_live_tables();
                    return true;
                }
                false
            }
        }
    }

    /// Раскладка нужной письменности: та, которой человек пользовался последней, иначе первая.
    fn pick(&self, script: Script) -> Option<Hkl> {
        let remembered = if script == Script::Latin {
            self.last_lat
        } else {
            self.last_cyr
        };
        if remembered != 0 && self.list.contains(&remembered) {
            return Some(remembered);
        }
        self.list.iter().copied().find(|&h| script_of(h) == script)
    }

    /// Раскладка, в которой декодировать нажатие: только что запрошенная нами или реальная.
    pub fn effective(&mut self) -> Hkl {
        if let Some((h, at)) = self.requested {
            if at.elapsed() < OWN_SELECT_GRACE {
                return h;
            }
            self.requested = None;
        }
        let (h, _) = foreground_hkl();
        self.observe(h);
        h
    }

    /// Запомнить раскладку, которой человек пользуется (её и включаем при конверсии).
    fn observe(&mut self, h: Hkl) {
        match script_of(h) {
            Script::Latin if h != self.last_lat => self.last_lat = h,
            Script::Cyrillic if h != self.last_cyr => self.last_cyr = h,
            _ => {}
        }
    }

    pub fn current_script(&mut self) -> Script {
        script_of(self.effective())
    }

    pub fn select(&mut self, cyrillic: bool) {
        let want = if cyrillic {
            Script::Cyrillic
        } else {
            Script::Latin
        };
        if let Some(h) = self.pick(want) {
            self.activate(h);
        }
    }

    pub fn cycle(&mut self) -> bool {
        if self.list.len() < 2 {
            return false;
        }
        let cur = self.effective();
        let i = self
            .list
            .iter()
            .position(|&h| h == cur)
            .map(|i| (i + 1) % self.list.len())
            .unwrap_or(0);
        self.activate(self.list[i]);
        true
    }

    fn activate(&mut self, h: Hkl) {
        unsafe {
            let (_, fg) = foreground_hkl();
            let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
            let mut gui: GUITHREADINFO = std::mem::zeroed();
            gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            let target = if GetGUIThreadInfo(tid, &mut gui) != 0 && !gui.hwndFocus.is_null() {
                gui.hwndFocus
            } else {
                fg
            };
            PostMessageW(target, WM_INPUTLANGCHANGEREQUEST, 0, h);
        }
        self.requested = Some((h, Instant::now()));
        self.observe(h);
    }
}

/// Таблицы EN→RU и RU→EN по всем печатным клавишам, с шифтом и без. Первая пара побеждает.
fn build_tables(lat: Hkl, cyr: Hkl) -> (Table, Table) {
    let mut e2r = Table::new();
    let mut r2e = Table::new();
    for &vk in PRINTABLE_VKS {
        for shift in [false, true] {
            let (Some(ls), Some(cs)) = (
                translate(vk, shift, false, lat),
                translate(vk, shift, false, cyr),
            ) else {
                continue;
            };
            let (mut li, mut ci) = (ls.chars(), cs.chars());
            let (Some(l), None, Some(c), None) = (li.next(), li.next(), ci.next(), ci.next())
            else {
                continue;
            };
            if l == c || l.is_control() || c.is_control() {
                continue;
            }
            e2r.entry(l).or_insert(c);
            r2e.entry(c).or_insert(l);
        }
    }
    (e2r, r2e)
}

pub fn hkl_hex(h: Hkl) -> String {
    format!("{:x}", h as usize)
}

pub fn parse_hkl(s: &str) -> Hkl {
    usize::from_str_radix(s, 16).map(|v| v as Hkl).unwrap_or(0)
}

/// Короткое имя раскладки для значка в трее: EN, RU, UK, DE…
pub fn short_name(h: Hkl) -> String {
    let lang = (h as usize & 0xFFFF) as u32;
    let mut buf = [0u16; 9];
    let n = unsafe {
        windows_sys::Win32::Globalization::GetLocaleInfoW(
            lang,
            windows_sys::Win32::Globalization::LOCALE_SISO639LANGNAME,
            buf.as_mut_ptr(),
            buf.len() as i32,
        )
    };
    if n > 1 {
        String::from_utf16_lossy(&buf[..(n - 1) as usize]).to_uppercase()
    } else {
        "??".into()
    }
}
