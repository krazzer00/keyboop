//! Мелкие системные вещи: один экземпляр, DPI, звук, имя программы окна, автозапуск, открыть файл.

use std::sync::Mutex;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn set_dpi_awareness() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Второй экземпляр только мешал бы первому (два хука — двойные замены).
pub fn single_instance() -> bool {
    unsafe {
        let name = wide("Local\\KeyboopSingleInstance");
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

pub fn message_box(text: &str) {
    unsafe {
        let t = wide(text);
        let c = wide("Keyboop");
        MessageBoxW(
            std::ptr::null_mut(),
            t.as_ptr(),
            c.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

pub fn system_is_russian() -> bool {
    let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    lang & 0x3FF == 0x19
}

/// Готовый WAV звука переключения. `PlaySound(SND_MEMORY | SND_ASYNC)` читает память, пока
/// играет, поэтому старый буфер не освобождаем (громкость меняется редко — это копейки).
static CUE: Mutex<Option<&'static [u8]>> = Mutex::new(None);

pub fn set_sound_volume(volume: f64) {
    let wav: &'static [u8] = Box::leak(crate::synth::switch_cue(volume).into_boxed_slice());
    *CUE.lock().unwrap() = Some(wav);
}

pub fn play_switch_sound() {
    let cue = *CUE.lock().unwrap();
    if let Some(wav) = cue {
        unsafe {
            PlaySoundW(
                wav.as_ptr() as *const u16,
                std::ptr::null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
}

/// Звуки диктовки и перевода: WAV держим в памяти (кэш по виду и громкости).
pub fn play_cue(kind: crate::synth::Cue, volume: f64) {
    static CUES: Mutex<Vec<(crate::synth::Cue, u64, &'static [u8])>> = Mutex::new(Vec::new());
    let key = volume.to_bits();
    let wav = {
        let mut c = CUES.lock().unwrap();
        match c.iter().find(|(k, v, _)| *k == kind && *v == key) {
            Some((_, _, w)) => *w,
            None => {
                let w: &'static [u8] =
                    Box::leak(crate::synth::cue(kind, volume).into_boxed_slice());
                c.push((kind, key, w));
                w
            }
        }
    };
    unsafe {
        PlaySoundW(
            wav.as_ptr() as *const u16,
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

pub fn beep() {
    unsafe {
        MessageBeep(MB_OK);
    }
}

/// Имя exe процесса, которому принадлежит окно (в нижнем регистре), например `code.exe`.
pub fn exe_name_of_window(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return String::new();
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase()
    }
}

/// Окно принадлежит процессу с более высокими правами (запущен от администратора), а мы — нет.
/// Такому окну Windows не даёт присылать ввод (UIPI): наша замена туда не дойдёт, а проглоченная
/// клавиша потерялась бы. Если права процесса даже прочитать нельзя, считаем его таким же.
pub fn is_elevated_foreign(hwnd: HWND) -> bool {
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    unsafe fn elevated(process: HANDLE) -> Option<bool> {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut e as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        (ok != 0).then_some(e.TokenIsElevated != 0)
    }
    unsafe {
        if elevated(GetCurrentProcess()) == Some(true) {
            return false; // мы сами администратор — UIPI нам не мешает
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return false;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return true;
        }
        let r = elevated(h);
        CloseHandle(h);
        r.unwrap_or(true)
    }
}

pub fn is_own_window(hwnd: HWND) -> bool {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid == GetCurrentProcessId()
    }
}

pub fn open_in_notepad(path: &std::path::Path) {
    unsafe {
        let op = wide("open");
        let exe = wide("notepad.exe");
        let arg = wide(&format!("\"{}\"", path.display()));
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            exe.as_ptr(),
            arg.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn open_folder(path: &std::path::Path) {
    unsafe {
        let op = wide("open");
        let p = wide(&path.display().to_string());
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            p.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Keyboop";

fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

pub fn autostart_enabled() -> bool {
    unsafe {
        let key = wide(RUN_KEY);
        let val = wide(RUN_VALUE);
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        );
        if r != ERROR_SUCCESS {
            return false;
        }
        let s = String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]);
        s.trim_matches('"').eq_ignore_ascii_case(&exe_path())
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        let key = wide(RUN_KEY);
        let val = wide(RUN_VALUE);
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_SET_VALUE, &mut hkey)
            != ERROR_SUCCESS
        {
            return;
        }
        if on {
            let data = wide(&format!("\"{}\"", exe_path()));
            RegSetValueExW(
                hkey,
                val.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                (data.len() * 2) as u32,
            );
        } else {
            RegDeleteValueW(hkey, val.as_ptr());
        }
        RegCloseKey(hkey);
    }
}
