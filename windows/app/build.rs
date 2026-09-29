//! Сборочные мелочи для Windows.
//!
//! Под mingw (кросс-сборка) C++-рантайм whisper.cpp по умолчанию тянется как `libstdc++-6.dll`,
//! и exe без неё не запустится. whisper-rs-sys просит `-lstdc++` в «динамическом» режиме
//! линковщика, поэтому кладём в отдельную папку ТОЛЬКО статическую `libstdc++.a` и ставим эту
//! папку в поиск первой: линковщик найдёт в ней статическую версию раньше системной DLL-заглушки.
//! MSVC-сборку это не касается: там рантайм линкуется как обычно.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    if target != "x86_64-pc-windows-gnu" {
        return;
    }
    let cxx = env::var("CXX_x86_64_pc_windows_gnu")
        .unwrap_or_else(|_| "x86_64-w64-mingw32-g++-posix".into());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("static-cxx");
    let _ = std::fs::create_dir_all(&out);
    if let Ok(o) = Command::new(&cxx)
        .arg("-print-file-name=libstdc++.a")
        .output()
    {
        let path = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        if path.is_absolute() && path.exists() {
            let _ = std::fs::copy(&path, out.join("libstdc++.a"));
            println!("cargo:rustc-link-search=native={}", out.display());
        }
    }
    // Всё остальное (winpthread, libgcc) — тоже статически; advapi32 нужен ggml (реестр) и должен
    // стоять в строке линковки ПОСЛЕ библиотек whisper.
    println!("cargo:rustc-link-arg-bins=-static");
    println!("cargo:rustc-link-arg-bins=-ladvapi32");
    println!("cargo:rerun-if-env-changed=CXX_x86_64_pc_windows_gnu");
}
