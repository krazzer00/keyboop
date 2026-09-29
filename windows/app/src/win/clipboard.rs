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
use keyboop_core::history::{self, ClipVerdict, HistoryKind};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
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
    _owned: Owned,
}

// ── Наши записи в буфер ──────────────────────────────────────────────────────────────────
//
// ⚠️ ОБЯЗАННОСТЬ КАЖДОЙ НАШЕЙ ЗАПИСИ В БУФЕР: сразу после неё отметить номер (`note_ours`).
// Иначе история буфера запишет нашу служебную операцию как «скопированный текст» человека.
// Многошаговые операции (Ctrl+C → чтение → возврат) ещё и держат «окно владения» (`Owned`):
// пока оно открыто, наблюдатель в буфер не смотрит.

static OWNED_DEPTH: AtomicU32 = AtomicU32::new(0);
static OWN_SEQS: Mutex<VecDeque<u32>> = Mutex::new(VecDeque::new());

/// Окно владения буфером: открыто, пока жив объект.
pub struct Owned;

impl Owned {
    pub fn begin() -> Owned {
        OWNED_DEPTH.fetch_add(1, Ordering::SeqCst);
        Owned
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        OWNED_DEPTH.fetch_sub(1, Ordering::SeqCst);
    }
}

fn note_ours() {
    let seq = unsafe { GetClipboardSequenceNumber() };
    let mut q = OWN_SEQS.lock().unwrap();
    q.push_back(seq);
    while q.len() > 32 {
        q.pop_front();
    }
}

fn is_ours(seq: u32) -> bool {
    OWN_SEQS.lock().unwrap().contains(&seq)
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
    let owned = Owned::begin();
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
        _owned: owned,
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
        if !write_formats(&self.backup) {
            super::app().log("выделение: буфер обмена занят — не вернул");
        }
    }
}

fn exclude_format() -> u32 {
    unsafe {
        RegisterClipboardFormatW(wide("ExcludeClipboardContentFromMonitorProcessing").as_ptr())
    }
}

fn set_bytes(format: u32, bytes: &[u8]) {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if h.is_null() {
            return;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        if SetClipboardData(format, h).is_null() {
            GlobalFree(h);
        }
    }
}

/// Записать в буфер набор форматов строго в исходном порядке (программа берёт первый
/// подходящий) и пометить запись служебной: менеджеры истории (Win+V) и наш наблюдатель её
/// не запоминают.
fn write_formats(formats: &[(u32, Vec<u8>)]) -> bool {
    {
        let Some(_open) = Open::new() else {
            return false;
        };
        unsafe {
            EmptyClipboard();
        }
        for (f, bytes) in formats {
            set_bytes(*f, bytes);
        }
        set_bytes(exclude_format(), &[0; 4]);
    }
    note_ours();
    true
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn formats_now() -> Vec<u32> {
    let Some(_open) = Open::new() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut f = 0u32;
    unsafe {
        loop {
            f = EnumClipboardFormats(f);
            if f == 0 {
                break;
            }
            out.push(f);
        }
    }
    out
}

/// Сколько ждём, пока программа заберёт содержимое, прежде чем вернуть буфер. У чтения буфера
/// нет уведомления; вернуть рано значит вставить форматированный текст, то есть не сделать ровно
/// то, ради чего звали.
const SETTLE: Duration = Duration::from_millis(300);

/// Вставка без форматирования (перенос `PlainPaste.swift`): подменяем богатое содержимое голым
/// текстом, шлём Ctrl+V и возвращаем буфер как было — только если его за это время никто не
/// сменил. Уже голый текст не трогаем вовсе, просто вставляем.
pub fn plain_paste() {
    let app = super::app();
    let Some(plain) = read_text().filter(|t| !t.is_empty()) else {
        app.log("вставка без форматирования: в буфере нет текста — ничего не делаю");
        super::sys::beep();
        return;
    };
    // CF_TEXT, CF_OEMTEXT, CF_UNICODETEXT, CF_LOCALE: система синтезирует их друг из друга.
    let only_plain = formats_now().iter().all(|f| matches!(f, 1 | 7 | 13 | 16));
    input::release_modifiers();
    if only_plain {
        app.log("вставка без форматирования: в буфере и так голый текст — буфер не трогаю");
        input::ctrl_v();
        return;
    }
    let _owned = Owned::begin();
    let Some(saved) = backup() else {
        app.log("вставка без форматирования: буфер слишком большой или занят — не трогаю");
        return;
    };
    if !write_formats(&[(CF_UNICODETEXT as u32, utf16z(&plain))]) {
        app.log("вставка без форматирования: буфер занят — ничего не делаю");
        return;
    }
    let ours = unsafe { GetClipboardSequenceNumber() };
    input::ctrl_v();
    std::thread::sleep(SETTLE);
    if unsafe { GetClipboardSequenceNumber() } != ours {
        // Человек или другая программа положили в буфер своё: оно новее нашего.
        app.log("вставка без форматирования: буфер сменился на чужой — не восстанавливаю");
        return;
    }
    if !write_formats(&saved) {
        app.log("вставка без форматирования: буфер занят — не восстановил");
        return;
    }
    app.log(&format!(
        "вставка без форматирования: {} симв., буфер восстановлен (форматов было {})",
        plain.chars().count(),
        saved.len()
    ));
}

// ── История буфера обмена (перенос `ClipboardWatcher.swift`) ───────────────────────────────
//
// Опрашиваем номер буфера на таймере окна трея: одно целое число, дешевле любого чтения.
// Содержимое читаем, только когда номер сдвинулся, и принимаем изменение на следующем тике, если
// он не сдвинулся снова: так пропускаются промежуточные записи программы в несколько шагов, а
// наши собственные операции успевают отметиться. Выключенный (по умолчанию) захват не стоит ни
// одного вызова. То, что лежало в буфере до включения, не забираем: согласия на это не давали.

static WATCHING: AtomicBool = AtomicBool::new(false);
static LAST_SEEN: AtomicU32 = AtomicU32::new(0);
static PENDING: AtomicU32 = AtomicU32::new(0);
static LAST_SKIP: Mutex<Option<(&'static str, Instant)>> = Mutex::new(None);

/// Пароли и прочее, что программа просит не запоминать (менеджеры паролей ставят эти форматы).
fn concealed() -> bool {
    unsafe {
        for name in [
            "ExcludeClipboardContentFromMonitorProcessing",
            "Clipboard Viewer Ignore",
        ] {
            if IsClipboardFormatAvailable(RegisterClipboardFormatW(wide(name).as_ptr())) != 0 {
                return true;
            }
        }
        let f = RegisterClipboardFormatW(wide("CanIncludeInClipboardHistory").as_ptr());
        if IsClipboardFormatAvailable(f) != 0 {
            let Some(_open) = Open::new() else {
                return true;
            };
            let h = GetClipboardData(f);
            if !h.is_null() && GlobalSize(h) >= 4 {
                let p = GlobalLock(h) as *const u32;
                if !p.is_null() {
                    let v = p.read_unaligned();
                    GlobalUnlock(h);
                    return v == 0;
                }
            }
        }
    }
    false
}

/// Тик наблюдателя (таймер окна трея).
pub fn watch_tick() {
    let app = super::app();
    let (on, minutes) = {
        let e = app.engine.lock().unwrap();
        (
            e.settings.voice_history_enabled && e.settings.clipboard_history_enabled,
            e.settings.voice_history_minutes,
        )
    };
    let seq = unsafe { GetClipboardSequenceNumber() };
    if !on {
        if WATCHING.swap(false, Ordering::Relaxed) {
            app.log("буфер: история буфера выключена");
        }
        return;
    }
    if !WATCHING.swap(true, Ordering::Relaxed) {
        LAST_SEEN.store(seq, Ordering::Relaxed);
        PENDING.store(0, Ordering::Relaxed);
        app.log("буфер: история буфера включена");
        return;
    }
    if seq == LAST_SEEN.load(Ordering::Relaxed) || OWNED_DEPTH.load(Ordering::SeqCst) > 0 {
        PENDING.store(0, Ordering::Relaxed);
        return;
    }
    if PENDING.swap(seq, Ordering::Relaxed) != seq {
        return; // подождём тик: вдруг программа ещё пишет
    }
    PENDING.store(0, Ordering::Relaxed);
    LAST_SEEN.store(seq, Ordering::Relaxed);
    let ours = is_ours(seq);
    let hidden = !ours && concealed();
    let text = if ours || hidden { None } else { read_text() };
    let last = super::history_store::last_clipboard_text();
    match history::clipboard_verdict(text.as_deref(), ours, hidden, last.as_deref()) {
        ClipVerdict::Store(t) => {
            let exe = app.last_app.lock().unwrap().clone();
            let src = (!exe.is_empty()).then_some(exe);
            super::history_store::add_text(&t, HistoryKind::Clipboard, src, None, minutes);
            // Только длина, не текст.
            app.log(&format!("буфер: записано {} симв.", t.chars().count()));
        }
        ClipVerdict::Skip(reason) => note_skip(reason),
    }
}

/// Причины пропуска в лог без спама: «наше» и «повтор» — норма; прочие не чаще раза в 10 с.
fn note_skip(reason: &'static str) {
    if matches!(reason, "ours" | "duplicate") {
        return;
    }
    let mut last = LAST_SKIP.lock().unwrap();
    if let Some((r, at)) = *last {
        if r == reason && at.elapsed() < Duration::from_secs(10) {
            return;
        }
    }
    *last = Some((reason, Instant::now()));
    super::app().log(&format!("буфер: пропущено ({reason})"));
}
