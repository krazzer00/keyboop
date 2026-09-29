//! Общее для наших «плашек» поверх всех окон (индикатор диктовки, выбор сниппета): окно с
//! попиксельной прозрачностью, которое не забирает фокус и не ловит мышь, рисование через
//! tiny-skia и текст шрифтом Windows через ab_glyph.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::sync::OnceLock;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn font() -> Option<&'static FontVec> {
    static F: OnceLock<Option<FontVec>> = OnceLock::new();
    F.get_or_init(|| {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        for name in ["segoeui.ttf", "seguisb.ttf", "arial.ttf", "tahoma.ttf"] {
            if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{name}")) {
                if let Ok(f) = FontVec::try_from_vec(bytes) {
                    return Some(f);
                }
            }
        }
        None
    })
    .as_ref()
}

pub fn scale() -> f32 {
    unsafe { GetDpiForSystem() as f32 / 96.0 }.max(1.0)
}

/// Создать окно-плашку (невидимое). Колбэк окна — стандартный.
pub fn create(class_name: &str) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class: Vec<u16> = class_name.encode_utf16().chain(Some(0)).collect();
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(DefWindowProcW),
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
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        )
    }
}

/// Показать картинку в окне в точке (x, y) экрана.
pub fn present(hwnd: HWND, pm: &Pixmap, x: i32, y: i32) {
    unsafe {
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bmp.is_null() {
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
            return;
        }
        let old = SelectObject(dc, bmp);
        // tiny-skia: RGBA с предумножением; Windows ждёт BGRA с предумножением.
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
        let pt_dst = POINT { x, y };
        let size = SIZE { cx: w, cy: h };
        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            hwnd, screen, &pt_dst, &size, dc, &pt_src, 0, &blend, ULW_ALPHA,
        );
        SelectObject(dc, old);
        DeleteObject(bmp);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn hide(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

pub fn fill_round_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color) {
    let mut pb = PathBuilder::new();
    let r = r.min(w / 2.0).min(h / 2.0);
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

pub fn fill_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, color: Color) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

pub fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Color) {
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        let mut paint = Paint::default();
        paint.set_color(color);
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
}

/// Ширина строки в пикселях.
pub fn text_width(text: &str, px: f32) -> f32 {
    let Some(f) = font() else {
        return text.chars().count() as f32 * px * 0.55;
    };
    let sf = f.as_scaled(PxScale::from(px));
    let mut w = 0.0;
    let mut prev = None;
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Нарисовать строку; (x, y) — левый край и базовая линия.
pub fn draw_text(
    pm: &mut Pixmap,
    text: &str,
    x: f32,
    baseline: f32,
    px: f32,
    color: [u8; 3],
    alpha: f32,
) {
    let Some(f) = font() else { return };
    let sf = f.as_scaled(PxScale::from(px));
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let mut pen = x;
    let mut prev = None;
    let data = pm.data_mut();
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            pen += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(pen, baseline));
        pen += sf.h_advance(id);
        prev = Some(id);
        if let Some(outline) = f.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let px_ = bounds.min.x as i32 + gx as i32;
                let py_ = bounds.min.y as i32 + gy as i32;
                if px_ < 0 || py_ < 0 || px_ >= w || py_ >= h {
                    return;
                }
                let a = (cov * alpha).clamp(0.0, 1.0);
                let i = ((py_ * w + px_) * 4) as usize;
                // Смешивание «поверх» в предумноженном RGBA.
                for (k, col) in color.iter().enumerate() {
                    let src = *col as f32 * a;
                    data[i + k] = (src + data[i + k] as f32 * (1.0 - a)).round().min(255.0) as u8;
                }
                data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a))
                    .round()
                    .min(255.0) as u8;
            });
        }
    }
}

/// Прямоугольник работы (без панели задач) монитора, где активное окно.
pub fn work_area_of_foreground() -> RECT {
    unsafe {
        let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

/// Где каретка ввода (экранные координаты), если приложение её показывает.
pub fn caret_rect() -> Option<RECT> {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let mut gui: GUITHREADINFO = std::mem::zeroed();
        gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(tid, &mut gui) == 0 || gui.hwndCaret.is_null() {
            return None;
        }
        let r = gui.rcCaret;
        if r.right <= r.left && r.bottom <= r.top {
            return None;
        }
        let mut tl = POINT {
            x: r.left,
            y: r.top,
        };
        let mut br = POINT {
            x: r.right,
            y: r.bottom,
        };
        ClientToScreen(gui.hwndCaret, &mut tl);
        ClientToScreen(gui.hwndCaret, &mut br);
        Some(RECT {
            left: tl.x,
            top: tl.y,
            right: br.x,
            bottom: br.y,
        })
    }
}
