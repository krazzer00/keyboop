//! Лампочка Caps Lock как индикатор языка: горит — русский, погасла — английский (перенос
//! `CapsLED.swift`). Сам Caps Lock не включается: пишем только светодиод.
//!
//! На Windows отдельного канала к светодиоду нет, поэтому пишем в драйвер класса клавиатур
//! (`IOCTL_KEYBOARD_SET_INDICATORS`), как это делают утилиты индикаторов. Оговорки честные:
//!   • Windows пускает туда не всегда (на части систем нужен запуск от администратора) — тогда
//!     функция молча не работает, причина один раз уходит в лог;
//!   • при нажатии Caps/Num/Scroll Lock система перерисовывает лампочки по-своему, поэтому мы
//!     догоняем её: переписываем состояние каждые две секунды и при каждой смене языка.
//! Настройка выключена по умолчанию: лампочка перестаёт означать «включён Caps».

use super::app;
use super::sys::wide;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::IO::DeviceIoControl;

const IOCTL_KEYBOARD_SET_INDICATORS: u32 = 0x000B_0008;
const IOCTL_KEYBOARD_QUERY_INDICATORS: u32 = 0x000B_0040;
const KEYBOARD_CAPS_LOCK_ON: u16 = 4;
/// Сколько клавиатур пробуем (KeyboardClass0…N).
const MAX_KEYBOARDS: usize = 4;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct IndicatorParams {
    unit_id: u16,
    led_flags: u16,
}

struct State {
    last: Option<(bool, Instant)>,
    /// Лампочку горела наша: при выключении настройки её надо погасить.
    lit_by_us: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    last: None,
    lit_by_us: false,
});
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Открыть N-ю клавиатуру через временное имя DOS-устройства.
fn open(n: usize) -> Option<HANDLE> {
    unsafe {
        let name = wide(&format!("KeyboopKbd{n}"));
        let target = wide(&format!("\\Device\\KeyboardClass{n}"));
        if DefineDosDeviceW(DDD_RAW_TARGET_PATH, name.as_ptr(), target.as_ptr()) == 0 {
            return None;
        }
        let path = wide(&format!("\\\\.\\KeyboopKbd{n}"));
        let h = CreateFileW(
            path.as_ptr(),
            0,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        DefineDosDeviceW(
            DDD_RAW_TARGET_PATH | DDD_REMOVE_DEFINITION | DDD_EXACT_MATCH_ON_REMOVE,
            name.as_ptr(),
            target.as_ptr(),
        );
        (h != INVALID_HANDLE_VALUE).then_some(h)
    }
}

/// Записать лампочку на всех клавиатурах. true — хоть одна приняла.
fn write(on: bool) -> bool {
    let mut any = false;
    for n in 0..MAX_KEYBOARDS {
        let Some(h) = open(n) else { continue };
        unsafe {
            let mut p = IndicatorParams::default();
            let mut got = 0u32;
            let q = DeviceIoControl(
                h,
                IOCTL_KEYBOARD_QUERY_INDICATORS,
                std::ptr::null(),
                0,
                &mut p as *mut _ as *mut _,
                4,
                &mut got,
                std::ptr::null_mut(),
            );
            if q != 0 {
                // Num и Scroll оставляем как есть, меняем только Caps.
                p.led_flags = (p.led_flags & !KEYBOARD_CAPS_LOCK_ON)
                    | if on { KEYBOARD_CAPS_LOCK_ON } else { 0 };
                let ok = DeviceIoControl(
                    h,
                    IOCTL_KEYBOARD_SET_INDICATORS,
                    &p as *const _ as *const _,
                    4,
                    std::ptr::null_mut(),
                    0,
                    &mut got,
                    std::ptr::null_mut(),
                );
                any |= ok != 0;
            }
            CloseHandle(h);
        }
    }
    any
}

/// Вызывается таймером трея. `enabled` — настройка, `russian` — текущий язык.
pub fn sync(enabled: bool, russian: bool) {
    let mut st = STATE.lock().unwrap();
    if !enabled {
        if st.lit_by_us {
            // Вернуть лампочке её обычный смысл: состояние Caps Lock.
            write(super::hook::caps_on());
            st.lit_by_us = false;
            st.last = None;
        }
        return;
    }
    let due = match st.last {
        Some((v, at)) => v != russian || at.elapsed() > Duration::from_secs(2),
        None => true,
    };
    if !due {
        return;
    }
    let ok = write(russian);
    st.last = Some((russian, Instant::now()));
    st.lit_by_us = ok;
    if !ok && !REPORTED.swap(true, Ordering::Relaxed) {
        app().log("Caps LED: Windows не даёт писать в лампочки клавиатуры (возможно, нужны права администратора)");
    }
}
