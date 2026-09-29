//! Громкость системы и микрофона на время диктовки (перенос `SystemVolume.swift`, `MicVolume.swift`):
//! приглушить вывод до N % от текущего и поднять уровень микрофона, а после — вернуть как было.
//! Через Core Audio (IAudioEndpointVolume) устройства по умолчанию.

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

fn endpoint(flow: EDataFlow) -> Option<IAudioEndpointVolume> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let e: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = e.GetDefaultAudioEndpoint(flow, eConsole).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

fn get(flow: EDataFlow) -> Option<f32> {
    unsafe { endpoint(flow)?.GetMasterVolumeLevelScalar().ok() }
}

fn set(flow: EDataFlow, v: f32) {
    if let Some(ep) = endpoint(flow) {
        unsafe {
            let _ = ep.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
        }
    }
}

/// Что поменяли — чтобы вернуть ровно то, что было.
#[derive(Default)]
pub struct Restore {
    output: Option<f32>,
    input: Option<f32>,
}

/// Приглушить вывод до `duck_percent` % от текущего уровня (если `duck`) и поставить микрофону
/// `gain_percent` % (если `gain` и сейчас ниже).
pub fn apply(duck: bool, duck_percent: u32, gain: bool, gain_percent: u32) -> Restore {
    let mut r = Restore::default();
    if duck {
        if let Some(cur) = get(eRender) {
            r.output = Some(cur);
            set(eRender, cur * duck_percent.min(77) as f32 / 100.0);
        }
    }
    if gain {
        if let Some(cur) = get(eCapture) {
            let want = gain_percent.clamp(10, 100) as f32 / 100.0;
            if cur < want {
                r.input = Some(cur);
                set(eCapture, want);
            }
        }
    }
    r
}

pub fn restore(r: Restore) {
    if let Some(v) = r.output {
        set(eRender, v);
    }
    if let Some(v) = r.input {
        set(eCapture, v);
    }
}
