//! Keyboop для Windows. Ядро (детектор раскладки и правила) — крейт `keyboop-core`, общий
//! с логикой мак-версии; здесь только системная оболочка.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Переносимые модули: вне Windows их используют только тесты.
#[cfg_attr(not(windows), allow(dead_code))]
mod hotkey;
#[cfg_attr(not(windows), allow(dead_code))]
mod l10n;
#[cfg_attr(not(windows), allow(dead_code))]
mod models;
#[cfg_attr(not(windows), allow(dead_code))]
mod storage;
#[cfg_attr(not(windows), allow(dead_code))]
mod synth;

#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn win_system_is_russian() -> bool {
    win::sys::system_is_russian()
}

#[cfg(windows)]
fn main() {
    // `keyboop.exe --ui <окно> [--section <раздел>]` — окно настроек/истории/… отдельным процессом.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--ui") {
        let kind = args.get(i + 1).cloned().unwrap_or_default();
        let section = args
            .iter()
            .position(|a| a == "--section")
            .and_then(|j| args.get(j + 1).cloned());
        ui::run(&kind, section);
        return;
    }
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Keyboop для Windows собирается под Windows: cargo build --release --target x86_64-pc-windows-gnu (или msvc).");
    eprintln!("На macOS используйте основное приложение из корня репозитория.");
    std::process::exit(1);
}
