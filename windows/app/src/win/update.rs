//! Установка обновлений (см. `crate::update` про соглашение о релизах).
//!
//! Работающий exe в Windows нельзя перезаписать, но можно переименовать: кладём новый рядом,
//! текущий переименовываем в `keyboop.old.exe`, новый — на его место, и перезапускаемся. Новый
//! процесс ждёт, пока старый выйдет, и удаляет `.old`. Не получилось на любом шаге — возвращаем
//! как было и открываем страницу релиза, чтобы человек скачал сам.

use super::{app, tray};
use crate::l10n::t;
use crate::update::{self, Release};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Предложили обновиться уведомлением: щелчок по нему установит.
pub static OFFERED: AtomicBool = AtomicBool::new(false);
static OFFER: Mutex<Option<Release>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);

/// Проверить (в своём потоке: сеть не должна задерживать ничего другого).
pub fn check(manual: bool) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let (silent, beta) = {
            let e = app().engine.lock().unwrap();
            (e.settings.silent_auto_update, e.settings.beta_channel)
        };
        match update::fetch() {
            Ok(list) => match update::newest(&list, &update::current(), beta) {
                None => {
                    app().log("обновления: новее нет");
                    if manual {
                        tray::balloon(t("update.latest"), "");
                    }
                }
                Some(r) => {
                    let v = r
                        .tag_name
                        .trim_start_matches(update::TAG_PREFIX)
                        .to_string();
                    app().log(&format!("обновления: доступна {v}"));
                    if silent {
                        install(r.clone());
                    } else {
                        *OFFER.lock().unwrap() = Some(r.clone());
                        OFFERED.store(true, Ordering::Relaxed);
                        tray::balloon(&format!("{}{v}", t("update.available")), t("update.click"));
                    }
                }
            },
            Err(e) => {
                app().log(&format!("обновления: не проверить: {e}"));
                if manual {
                    tray::balloon(t("update.failed"), "");
                }
            }
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// Щелчок по уведомлению «доступна версия».
pub fn install_offer() {
    if let Some(r) = OFFER.lock().unwrap().take() {
        std::thread::spawn(move || install(r));
    }
}

fn install(r: Release) {
    match try_install(&r) {
        Ok(()) => {
            app().log(&format!(
                "обновления: установлена {}, перезапуск",
                r.tag_name
            ));
            restart();
        }
        Err(e) => {
            app().log(&format!("обновления: не установилось: {e}"));
            tray::balloon(t("update.installFailed"), t("update.manual"));
            if !r.html_url.is_empty() {
                super::sys::open_url(&r.html_url);
            }
        }
    }
}

fn try_install(r: &Release) -> Result<(), String> {
    let asset = |name: &str| {
        r.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
    };
    let zip_url = asset(update::ASSET).ok_or("в релизе нет архива")?;
    let sha_url = asset(&format!("{}.sha256", update::ASSET)).ok_or("в релизе нет хеша")?;
    let sha_text = ureq::get(&sha_url)
        .call()
        .map_err(|e| e.to_string())?
        .into_body()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let sha = update::parse_sha(&sha_text).ok_or("хеш не читается")?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("нет папки программы")?.to_path_buf();
    let zip_path = std::env::temp_dir().join(update::ASSET);
    crate::models::download_file(
        &zip_url,
        &zip_path,
        Some(&sha),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .map_err(|e| format!("{e:?}"))?;
    let new_exe = dir.join("keyboop.new.exe");
    let extracted = extract_exe(&zip_path, &new_exe);
    let _ = std::fs::remove_file(&zip_path);
    extracted?;
    swap(&exe, &new_exe, &dir.join("keyboop.old.exe"))
}

fn extract_exe(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let name = (0..z.len())
        .filter_map(|i| z.by_index(i).ok().map(|e| e.name().to_string()))
        .find(|n| n.to_ascii_lowercase().ends_with("keyboop.exe"))
        .ok_or("в архиве нет keyboop.exe")?;
    let mut src = z.by_name(&name).map_err(|e| e.to_string())?;
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut src, &mut out).map_err(|e| e.to_string())?;
    Ok(())
}

/// current → old, new → current; при сбое — назад.
fn swap(current: &Path, new: &Path, old: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(old);
    std::fs::rename(current, old).map_err(|e| format!("нет прав на папку программы: {e}"))?;
    if let Err(e) = std::fs::rename(new, current) {
        let _ = std::fs::rename(old, current);
        let _ = std::fs::remove_file(new);
        return Err(e.to_string());
    }
    Ok(())
}

/// Запустить новую версию и выйти. Новый процесс дождётся нашего выхода (`--after-update`).
fn restart() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let pid = std::process::id().to_string();
    if std::process::Command::new(&exe)
        .args(["--after-update", &pid])
        .spawn()
        .is_ok()
    {
        unsafe {
            PostMessageW(
                app().ui_hwnd.load(Ordering::Relaxed) as HWND,
                WM_CLOSE,
                0,
                0,
            );
        }
    }
}

/// При старте: если нас запустила прежняя версия — дождаться её выхода; убрать `.old`.
pub fn on_start(args: &[String]) {
    if let Some(i) = args.iter().position(|a| a == "--after-update") {
        if let Some(pid) = args.get(i + 1).and_then(|p| p.parse::<u32>().ok()) {
            unsafe {
                let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
                if !h.is_null() {
                    WaitForSingleObject(h, 20_000);
                    CloseHandle(h);
                }
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let old: PathBuf = exe.with_file_name("keyboop.old.exe");
        for _ in 0..10 {
            if !old.exists() || std::fs::remove_file(&old).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}
