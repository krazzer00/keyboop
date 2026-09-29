//! Плашка диктовки у курсора (перенос `VoiceIndicator.swift`): «слушаю» с живой оранжевой
//! волной громкости, «распознаю…» с бегущими точками и короткие подсказки-тосты.
//! Своё окно на своём потоке: анимация не зависит ни от хука, ни от распознавания.

use super::overlay;
use crate::l10n::t;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_skia::Pixmap;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug, PartialEq)]
pub enum HudState {
    Hidden,
    Recording,
    /// «Распознаю…», «Перевожу…» — ключ строки и бегущие точки.
    Processing(&'static str),
    Toast(String),
}

struct Hud {
    state: HudState,
    since: Option<Instant>,
    top: bool,
    anchor: Option<(i32, i32)>,
}

static HUD: Mutex<Hud> = Mutex::new(Hud {
    state: HudState::Hidden,
    since: None,
    top: false,
    anchor: None,
});
static HWND_HUD: AtomicIsize = AtomicIsize::new(0);
static LEVELS: Mutex<Option<Arc<super::voice::audio::Levels>>> = Mutex::new(None);
const WM_HUD_WAKE: u32 = WM_APP + 20;
const TIMER_FRAME: usize = 1;
const TOAST_FOR: Duration = Duration::from_millis(2600);

pub fn set_levels(l: Arc<super::voice::audio::Levels>) {
    *LEVELS.lock().unwrap() = Some(l);
}

/// Сменить состояние плашки. `top` — у верхнего края экрана, а не у курсора.
pub fn set(state: HudState, top: bool) {
    {
        let mut h = HUD.lock().unwrap();
        if h.state == state {
            return;
        }
        // Привязку к каретке берём в момент показа: дальше плашка не прыгает за курсором.
        if h.state == HudState::Hidden {
            h.anchor = overlay::caret_rect().map(|r| (r.left, r.bottom));
        }
        h.state = state;
        h.since = Some(Instant::now());
        h.top = top;
    }
    let hwnd = HWND_HUD.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            PostMessageW(hwnd as HWND, WM_HUD_WAKE, 0, 0);
        }
    }
}

pub fn toast(text: &str) {
    let top = HUD.lock().unwrap().top;
    set(HudState::Toast(text.to_string()), top);
}

fn elapsed(h: &Hud) -> Duration {
    h.since.map(|t| t.elapsed()).unwrap_or_default()
}

fn render(h: &Hud) -> Option<Pixmap> {
    let s = overlay::scale();
    let height = (40.0 * s).round();
    let pad = 14.0 * s;
    let text_px = 14.0 * s;
    let (label, bars) = match &h.state {
        HudState::Hidden => return None,
        HudState::Recording => (t("voice.listening").to_string(), true),
        HudState::Processing(key) => {
            let dots = ((elapsed(h).as_millis() / 350) % 4) as usize;
            (format!("{}{}", t(key), ".".repeat(dots)), false)
        }
        HudState::Toast(msg) => (msg.clone(), false),
    };
    let bar_count = 18;
    let bars_w = if bars {
        bar_count as f32 * 4.0 * s
    } else {
        0.0
    };
    // Ширина «Распознаю…» считается с тремя точками, чтобы плашка не дёргалась в такт точкам.
    let label_w = match h.state {
        HudState::Processing(key) => overlay::text_width(&format!("{}...", t(key)), text_px),
        _ => overlay::text_width(&label, text_px),
    };
    let dot = 10.0 * s;
    let width =
        (pad + dot + 8.0 * s + bars_w + if bars { 10.0 * s } else { 0.0 } + label_w + pad).round();
    let mut pm = Pixmap::new(width as u32, height as u32)?;
    overlay::fill_round_rect(
        &mut pm,
        0.0,
        0.0,
        width,
        height,
        height / 2.0,
        overlay::rgba(28, 28, 30, 235),
    );
    let cy = height / 2.0;
    let mut x = pad;
    // Индикатор: красная точка записи, серая — обработка, оранжевая — подсказка.
    let dot_color = match h.state {
        HudState::Recording => {
            let pulse = 0.65 + 0.35 * ((elapsed(h).as_secs_f32() * 4.0).sin() * 0.5 + 0.5);
            overlay::rgba(255, 69, 58, (255.0 * pulse) as u8)
        }
        HudState::Processing(_) => overlay::rgba(160, 160, 165, 255),
        _ => overlay::rgba(255, 149, 0, 255),
    };
    overlay::fill_circle(&mut pm, x + dot / 2.0, cy, dot / 2.0, dot_color);
    x += dot + 8.0 * s;
    if bars {
        let levels = LEVELS
            .lock()
            .unwrap()
            .as_ref()
            .map(|l| l.last(bar_count))
            .unwrap_or_else(|| vec![0.0; bar_count]);
        for (i, lv) in levels.iter().enumerate() {
            let bh = (4.0 * s + lv * (height - 16.0 * s)).min(height - 12.0 * s);
            overlay::fill_round_rect(
                &mut pm,
                x + i as f32 * 4.0 * s,
                cy - bh / 2.0,
                2.5 * s,
                bh,
                1.25 * s,
                overlay::rgba(255, 122, 26, 255),
            );
        }
        x += bars_w + 10.0 * s;
    }
    overlay::draw_text(
        &mut pm,
        &label,
        x,
        cy + text_px * 0.35,
        text_px,
        [240, 240, 245],
        1.0,
    );
    Some(pm)
}

fn position(h: &Hud, w: i32, hgt: i32) -> (i32, i32) {
    let area = overlay::work_area_of_foreground();
    if !h.top {
        if let Some((cx, cy)) = h.anchor {
            let x = (cx - w / 2).clamp(area.left + 4, area.right - w - 4);
            let y = if cy + 12 + hgt < area.bottom {
                cy + 12
            } else {
                cy - hgt - 40
            };
            return (x, y.clamp(area.top + 4, area.bottom - hgt - 4));
        }
    }
    let x = area.left + (area.right - area.left - w) / 2;
    let y = if h.top {
        area.top + 24
    } else {
        area.bottom - hgt - 60
    };
    (x, y)
}

fn frame(hwnd: HWND) -> bool {
    let mut h = HUD.lock().unwrap();
    if let HudState::Toast(_) = h.state {
        if elapsed(&h) > TOAST_FOR {
            h.state = HudState::Hidden;
        }
    }
    match render(&h) {
        None => {
            overlay::hide(hwnd);
            false
        }
        Some(pm) => {
            let (x, y) = position(&h, pm.width() as i32, pm.height() as i32);
            overlay::present(hwnd, &pm, x, y);
            true
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_HUD_WAKE | WM_TIMER => {
            let visible = frame(hwnd);
            if visible {
                SetTimer(hwnd, TIMER_FRAME, 33, None);
            } else {
                KillTimer(hwnd, TIMER_FRAME);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Поток плашки. Зовётся один раз при старте.
pub fn run() {
    unsafe {
        let hwnd = overlay::create("KeyboopHud");
        let proc_: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT = wndproc;
        SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc_ as usize as isize);
        HWND_HUD.store(hwnd as isize, Ordering::Relaxed);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
