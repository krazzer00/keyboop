//! Выбор текстового сниппета по хоткею (перенос `SnippetPicker.swift`).
//!
//! ⚠️ ПАНЕЛЬ НЕ ЗАБИРАЕТ ФОКУС — ЭТО ГЛАВНОЕ. Обычное окно выбора сделало бы активными нас, и
//! сниппет пришлось бы вставлять, возвращая фокус чужому окну, то есть гонкой. Здесь панель только
//! рисует (слоёное окно без активации, прозрачное для мыши), цифру ловит хук клавиатуры, а клик по
//! строке — хук мыши. Каретка остаётся ровно там, где была.
//!
//! Цифры: 1…9, затем Shift+1…9, затем Ctrl+1…9, дальше только мышью. `0` — нулевая строка
//! «последняя диктовка», если её есть чем вставить. Esc закрывает, любая другая клавиша закрывает
//! список и уходит приложению своим ходом: человек передумал, отбирать у него нажатие нельзя.
//!
//! Всё здесь живёт в потоке хуков (открытие из хоткея, клавиши и мышь из хуков, таймер его окна).

use super::overlay::{self, rgba};
use crate::l10n;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use tiny_skia::Pixmap;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const TIMER_PICKER: usize = 3;
/// Список сам закрывается, если им не пользуются.
const IDLE_MS: u32 = 30_000;

static OPEN: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
enum Target {
    Dictation(String),
    Snippet(String),
}

struct Row {
    label: String,
    name: String,
    text: String,
    target: Target,
}

struct Picker {
    hwnd: HWND,
    rows: Vec<Row>,
    /// Первая видимая строка (прокрутка колесом, когда всё не помещается).
    first: usize,
    visible: usize,
    hover: Option<usize>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    more_than_nine: bool,
}

thread_local! {
    static HWND_CACHE: RefCell<HWND> = const { RefCell::new(std::ptr::null_mut()) };
    static PICKER: RefCell<Option<Picker>> = const { RefCell::new(None) };
}

const ROW_H: f32 = 28.0;
const PAD_V: f32 = 12.0;
const TIP_H: f32 = 22.0;
const WIDTH: f32 = 620.0;

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

fn window() -> HWND {
    HWND_CACHE.with(|c| {
        let mut h = c.borrow_mut();
        if h.is_null() {
            *h = overlay::create("KeyboopSnippetPicker");
        }
        *h
    })
}

/// Подпись строки: 0 — диктовка, 1…9, Shift+1…9, Ctrl+1…9, дальше без цифры.
fn shortcut_label(index: usize) -> String {
    match index {
        0..=9 => index.to_string(),
        10..=18 => format!("Shift+{}", index - 9),
        19..=27 => format!("Ctrl+{}", index - 18),
        _ => String::new(),
    }
}

/// Последняя диктовка, если её есть чем вставить.
fn last_dictation() -> Option<String> {
    if std::env::var("KEYBOOP_SNIPPICK_DEMO").as_deref() == Ok("1") {
        return Some(
            "Образец: сюда попадает текст последней диктовки, длинный обрезается по ширине панели"
                .into(),
        );
    }
    let minutes = super::app()
        .engine
        .lock()
        .unwrap()
        .settings
        .voice_history_minutes;
    super::history_store::last_dictation(minutes).filter(|t| !t.is_empty())
}

/// Открыть список (хоткей). Пустой список — короткая подсказка вместо панели.
pub fn open() {
    let pairs: Vec<(String, String)> = super::app()
        .store
        .load_text_snippets()
        .into_iter()
        .filter(|(a, b)| !a.is_empty() || !b.is_empty())
        .collect();
    let dictation = last_dictation();
    if pairs.is_empty() && dictation.is_none() {
        super::hud::toast(l10n::t("snip.pickEmpty"));
        return;
    }
    hide();
    let mut rows = Vec::new();
    if let Some(d) = dictation {
        rows.push(Row {
            label: shortcut_label(0),
            name: l10n::t("snip.pickDictation").into(),
            text: d.clone(),
            target: Target::Dictation(d),
        });
    }
    let more_than_nine = pairs.len() > 9;
    for (i, (name, text)) in pairs.into_iter().enumerate() {
        rows.push(Row {
            label: shortcut_label(i + 1),
            name,
            text: text.clone(),
            target: Target::Snippet(text),
        });
    }

    let s = overlay::scale();
    let (ax, ay) = anchor();
    let work = work_area_at(ax, ay);
    let work_h = (work.bottom - work.top) as f32;
    // Хотя бы три строки видно всегда; больше 60% высоты экрана панель не занимает.
    let max_rows = (((work_h * 0.6) - (PAD_V * 2.0 + TIP_H) * s) / (ROW_H * s))
        .floor()
        .max(3.0) as usize;
    let visible = rows.len().min(max_rows);
    let w = (WIDTH * s).round() as i32;
    let h = ((PAD_V * 2.0 + TIP_H + ROW_H * visible as f32) * s).round() as i32;
    let (x, y) = origin_near(ax, ay, w, h, &work);
    let p = Picker {
        hwnd: window(),
        rows,
        first: 0,
        visible,
        hover: None,
        x,
        y,
        w,
        h,
        more_than_nine,
    };
    draw(&p);
    PICKER.with(|c| *c.borrow_mut() = Some(p));
    OPEN.store(true, Ordering::Relaxed);
    unsafe {
        SetTimer(super::hook::hook_hwnd(), TIMER_PICKER, IDLE_MS, None);
    }
}

pub fn hide() {
    if !OPEN.swap(false, Ordering::Relaxed) {
        return;
    }
    PICKER.with(|c| {
        if let Some(p) = c.borrow_mut().take() {
            overlay::hide(p.hwnd);
        }
    });
    unsafe {
        KillTimer(super::hook::hook_hwnd(), TIMER_PICKER);
    }
}

/// Вставить выбранное. Печать уходит в очередь потока хуков — после того, как хук вернёт
/// управление и проглоченная цифра уже не помешает.
fn pick(target: Target) {
    hide();
    match target {
        Target::Dictation(t) => super::hook::post_insert(t, false),
        Target::Snippet(t) => {
            if !t.is_empty() {
                super::app().log(&format!(
                    "сниппет по хоткею: вставлено {} симв.",
                    t.chars().count()
                ));
                super::hook::post_insert(t, false);
            }
        }
    }
}

/// Нажатие клавиши при открытом списке. `Some(true)` — проглотить, `Some(false)` — список закрыт,
/// клавиша уходит дальше, `None` — не наше (модификатор).
pub fn on_key(vk: u32, shift: bool, ctrl: bool, other_mods: bool) -> Option<bool> {
    if !is_open() || crate::hotkey::is_modifier_vk(vk) {
        return None;
    }
    if vk == 0x1B {
        hide();
        return Some(true);
    }
    let digit = match vk {
        0x30..=0x39 => Some(vk - 0x30),
        0x60..=0x69 => Some(vk - 0x60), // цифровой блок
        _ => None,
    };
    if let Some(d) = digit {
        let bank = match (shift, ctrl, other_mods) {
            (false, false, false) => Some(0),
            (true, false, false) => Some(1),
            (false, true, false) => Some(2),
            _ => None,
        };
        if let Some(bank) = bank {
            let target = PICKER.with(|c| {
                let c = c.borrow();
                let p = c.as_ref()?;
                let has_dictation = matches!(p.rows.first()?.target, Target::Dictation(_));
                if d == 0 {
                    // Нулевая строка своя: если её нет, клавиша остаётся человеку.
                    return (bank == 0 && has_dictation).then(|| p.rows[0].target.clone());
                }
                let idx = bank * 9 + d as usize - 1;
                let row = idx + usize::from(has_dictation);
                p.rows.get(row).map(|r| r.target.clone())
            });
            if let Some(t) = target {
                pick(t);
                return Some(true);
            }
        }
    }
    hide();
    Some(false)
}

/// Мышь при открытом списке. true — событие наше (клик по строке, колесо над панелью).
pub fn on_mouse(msg: u32, x: i32, y: i32, wheel: i16) -> bool {
    if !is_open() {
        return false;
    }
    let hit = PICKER.with(|c| {
        let c = c.borrow();
        let p = c.as_ref()?;
        let inside = x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h;
        inside.then(|| row_at(p, y))
    });
    match msg {
        WM_MOUSEMOVE => {
            let row = hit.flatten();
            PICKER.with(|c| {
                if let Some(p) = c.borrow_mut().as_mut() {
                    if p.hover != row {
                        p.hover = row;
                        draw(p);
                    }
                }
            });
            false
        }
        WM_MOUSEWHEEL if hit.is_some() => {
            PICKER.with(|c| {
                if let Some(p) = c.borrow_mut().as_mut() {
                    let max_first = p.rows.len().saturating_sub(p.visible);
                    let step = if wheel > 0 { -1i32 } else { 1 };
                    let first = (p.first as i32 + step * 3).clamp(0, max_first as i32) as usize;
                    if first != p.first {
                        p.first = first;
                        draw(p);
                    }
                }
            });
            true
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => match hit {
            Some(Some(row)) if msg == WM_LBUTTONDOWN => {
                let t = PICKER.with(|c| c.borrow().as_ref().map(|p| p.rows[row].target.clone()));
                if let Some(t) = t {
                    pick(t);
                }
                true
            }
            Some(_) => true, // по панели мимо строк: ничего не делаем, но и приложению не отдаём
            None => {
                hide(); // клик мимо списка закрывает его
                false
            }
        },
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => hit.is_some(),
        _ => false,
    }
}

fn row_at(p: &Picker, y: i32) -> Option<usize> {
    let s = overlay::scale();
    let rel = (y - p.y) as f32 - PAD_V * s;
    if rel < 0.0 {
        return None;
    }
    let i = (rel / (ROW_H * s)) as usize;
    (i < p.visible)
        .then_some(p.first + i)
        .filter(|&r| r < p.rows.len())
}

fn anchor() -> (i32, i32) {
    if let Some(r) = overlay::caret_rect() {
        return (r.left, r.bottom);
    }
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

fn work_area_at(x: i32, y: i32) -> RECT {
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

/// Под кареткой со сдвигом; не влезает вниз — над ней; и всегда в пределах экрана.
fn origin_near(ax: i32, ay: i32, w: i32, h: i32, work: &RECT) -> (i32, i32) {
    let gap = (16.0 * overlay::scale()) as i32;
    let mut x = ax + gap;
    let mut y = ay + gap;
    if y + h > work.bottom - 8 {
        y = ay - h - gap;
    }
    x = x.min(work.right - w - 8).max(work.left + 8);
    y = y.min(work.bottom - h - 8).max(work.top + 8);
    (x, y)
}

/// Обрезать строку по ширине, с многоточием.
fn ellipsize(text: &str, px: f32, max_w: f32) -> String {
    if overlay::text_width(text, px) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if overlay::text_width(&s, px) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_string()
        + "…"
}

fn draw(p: &Picker) {
    if let Some(pm) = render(p) {
        overlay::present(p.hwnd, &pm, p.x, p.y);
    }
}

fn render(p: &Picker) -> Option<Pixmap> {
    let s = overlay::scale();
    let mut pm = Pixmap::new(p.w as u32, p.h as u32)?;
    let (w, h) = (p.w as f32, p.h as f32);
    overlay::fill_round_rect(&mut pm, 0.0, 0.0, w, h, 10.0 * s, rgba(32, 31, 30, 242));

    let px = 12.5 * s;
    let label_w = p
        .rows
        .iter()
        .map(|r| overlay::text_width(&r.label, px))
        .fold(18.0 * s, f32::max);
    let name_w = p
        .rows
        .iter()
        .map(|r| overlay::text_width(&r.name, px))
        .fold(0.0, f32::max)
        .clamp(44.0 * s, 120.0 * s)
        + 8.0 * s;
    let x_label = 14.0 * s;
    let x_name = x_label + label_w + 10.0 * s;
    let x_text = x_name + name_w + 10.0 * s;
    let row_h = ROW_H * s;
    let top = PAD_V * s;

    for (vi, row) in p.rows.iter().enumerate().skip(p.first).take(p.visible) {
        let y = top + (vi - p.first) as f32 * row_h;
        if p.hover == Some(vi) {
            overlay::fill_round_rect(
                &mut pm,
                6.0 * s,
                y,
                w - 12.0 * s,
                row_h,
                6.0 * s,
                rgba(0, 0, 0, 90),
            );
        }
        let base = y + row_h / 2.0 + px * 0.36;
        overlay::draw_text(
            &mut pm,
            &row.label,
            x_label,
            base,
            px,
            [0xFF, 0x7A, 0x59],
            1.0,
        );
        let name = ellipsize(&row.name, px, name_w - 4.0 * s);
        overlay::draw_text(&mut pm, &name, x_name, base, px, [240, 238, 235], 1.0);
        let flat = row.text.replace(['\r', '\n'], " ");
        let text = ellipsize(&flat, px, w - x_text - 14.0 * s);
        overlay::draw_text(&mut pm, &text, x_text, base, px, [170, 168, 165], 1.0);
        let last = vi + 1 == p.first + p.visible || vi + 1 == p.rows.len();
        if !last {
            let line = rgba(255, 255, 255, 26);
            overlay::fill_rect(&mut pm, 14.0 * s, y + row_h - 1.0, w - 28.0 * s, 1.0, line);
        }
    }
    let tip = l10n::t(if p.more_than_nine {
        "snip.pickTipMore"
    } else {
        "snip.pickTip"
    });
    let tip_px = 11.0 * s;
    overlay::draw_text(
        &mut pm,
        tip,
        14.0 * s,
        h - (TIP_H * s - tip_px) / 2.0 - 3.0 * s,
        tip_px,
        [130, 128, 125],
        1.0,
    );
    Some(pm)
}

#[cfg(test)]
mod tests {
    #[test]
    fn labels() {
        assert_eq!(super::shortcut_label(0), "0");
        assert_eq!(super::shortcut_label(9), "9");
        assert_eq!(super::shortcut_label(10), "Shift+1");
        assert_eq!(super::shortcut_label(27), "Ctrl+9");
        assert_eq!(super::shortcut_label(28), "");
    }
}
