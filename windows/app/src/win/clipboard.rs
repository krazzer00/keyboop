//! Чтение ВЫДЕЛЕННОГО текста для ручного хоткея (когда буфер набора пуст).
//!
//! Общего способа прочитать выделение в любой программе у Windows нет, поэтому делаем как
//! запасной путь мак-версии (`SelectionText.readViaClipboard`): Ctrl+C, прочитать, а затем
//! ВЕРНУТЬ буфер обмена как был — все форматы, байт в байт. Обычная конверсия слов (авто и по
//! хоткею) буфер обмена не трогает вообще; сюда попадаем только по явному хоткею на выделении.
//!
//! Возвращаем буфер, только если после нашего Ctrl+C его никто не менял: если человек успел
//! скопировать своё, его копию не затираем.

use super::input;
use super::sys::wide;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

/// Больше этого буфер не копируем: проще отказаться от чтения выделения, чем рисковать данными.
const MAX_BACKUP_BYTES: usize = 32 * 1024 * 1024;

pub struct Selection {
    pub text: String,
    backup: Vec<(u32, Vec<u8>)>,
    seq_after: u32,
}

fn hwnd() -> HWND {
    super::app()
        .ui_hwnd
        .load(std::sync::atomic::Ordering::Relaxed) as HWND
}

struct Open;

impl Open {
    fn new() -> Option<Open> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(hwnd()) } != 0 {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

/// GDI-форматы (не HGLOBAL) скопировать байтами нельзя; система сама синтезирует их из DIB.
fn is_handle_format(f: u32) -> bool {
    matches!(f, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E)
}

fn backup() -> Option<Vec<(u32, Vec<u8>)>> {
    let _open = Open::new()?;
    let mut out = Vec::new();
    let mut total = 0usize;
    let mut f = 0u32;
    unsafe {
        loop {
            f = EnumClipboardFormats(f);
            if f == 0 {
                break;
            }
            if is_handle_format(f) {
                continue;
            }
            let h = GetClipboardData(f);
            if h.is_null() {
                continue;
            }
            let size = GlobalSize(h);
            if size == 0 {
                continue;
            }
            total += size;
            if total > MAX_BACKUP_BYTES {
                return None;
            }
            let p = GlobalLock(h) as *const u8;
            if p.is_null() {
                continue;
            }
            out.push((f, std::slice::from_raw_parts(p, size).to_vec()));
            GlobalUnlock(h);
        }
    }
    Some(out)
}

fn read_text() -> Option<String> {
    let _open = Open::new()?;
    unsafe {
        let h = GetClipboardData(CF_UNICODETEXT as u32);
        if h.is_null() {
            return None;
        }
        let p = GlobalLock(h) as *const u16;
        if p.is_null() {
            return None;
        }
        let max = GlobalSize(h) / 2;
        let mut n = 0;
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        GlobalUnlock(h);
        Some(s)
    }
}

/// Скопировать выделение и прочитать его текст. `None` — выделения нет (буфер не изменился).
pub fn read_selection() -> Option<Selection> {
    let seq0 = unsafe { GetClipboardSequenceNumber() };
    let Some(saved) = backup() else {
        super::app().log("выделение: буфер обмена слишком большой или занят — не трогаю");
        return None;
    };
    input::release_modifiers();
    std::thread::sleep(Duration::from_millis(15));
    input::ctrl_c();
    let t0 = Instant::now();
    while unsafe { GetClipboardSequenceNumber() } == seq0 {
        if t0.elapsed() > Duration::from_millis(400) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Программа сперва очищает буфер, потом кладёт данные: дадим ей дописать.
    let mut text = None;
    while t0.elapsed() < Duration::from_millis(600) {
        std::thread::sleep(Duration::from_millis(15));
        text = read_text();
        if text.is_some() {
            break;
        }
    }
    let seq_after = unsafe { GetClipboardSequenceNumber() };
    Some(Selection {
        text: text.unwrap_or_default(),
        backup: saved,
        seq_after,
    })
}

impl Selection {
    /// Вернуть буфер обмена чуть позже: программа должна успеть принять нашу печать.
    pub fn restore_later(self) {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            self.restore();
        });
    }

    fn restore(self) {
        if unsafe { GetClipboardSequenceNumber() } != self.seq_after {
            super::app().log("выделение: буфер обмена изменён кем-то ещё — не возвращаю");
            return;
        }
        let Some(_open) = Open::new() else { return };
        unsafe {
            EmptyClipboard();
            for (f, bytes) in &self.backup {
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
                if h.is_null() {
                    continue;
                }
                let p = GlobalLock(h) as *mut u8;
                if p.is_null() {
                    GlobalFree(h);
                    continue;
                }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                GlobalUnlock(h);
                if SetClipboardData(*f, h).is_null() {
                    GlobalFree(h);
                }
            }
            // Менеджеры истории буфера (Win+V) не должны запоминать наш возврат как новую копию.
            let fmt = RegisterClipboardFormatW(
                wide("ExcludeClipboardContentFromMonitorProcessing").as_ptr(),
            );
            let h = GlobalAlloc(GMEM_MOVEABLE, 4);
            if !h.is_null() && SetClipboardData(fmt, h).is_null() {
                GlobalFree(h);
            }
        }
    }
}
