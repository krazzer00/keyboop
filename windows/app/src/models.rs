//! Модели распознавания речи: каталог, пути и скачивание (перенос `ModelDownloader.swift`).
//!
//! Сеть — только по явному действию человека (кнопка «Скачать»). Сначала зеркало keyboop.com,
//! если не отдало — HuggingFace на закреплённой ревизии. Файл проверяется по SHA-256 и только
//! потом занимает своё место: битая или подменённая модель в папку не попадёт.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const PINNED_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub name: &'static str,
    pub size: &'static str,
    pub sha256: &'static str,
}

pub const CATALOG: &[Model] = &[
    Model {
        name: "base",
        size: "142 MB",
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Model {
        name: "small",
        size: "466 MB",
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
    Model {
        name: "medium",
        size: "1.5 GB",
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
    },
    Model {
        name: "large-v3-turbo",
        size: "1.6 GB",

        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    },
];

pub fn find(name: &str) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.name == name)
}

/// `%LOCALAPPDATA%\Keyboop\models` (большие файлы — в Local, а не в перемещаемом профиле).
pub fn dir() -> PathBuf {
    let base = std::env::var_os("KEYBOOP_MODELS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(|a| PathBuf::from(a).join("Keyboop").join("models"))
        })
        .unwrap_or_else(|| PathBuf::from(".keyboop-models"));
    let _ = std::fs::create_dir_all(&base);
    base
}

pub fn whisper_path(name: &str) -> PathBuf {
    dir().join(format!("ggml-{name}.bin"))
}

pub fn is_installed(name: &str) -> bool {
    whisper_path(name).is_file()
}

/// Первая скачанная модель из каталога (если выбранной нет на диске).
pub fn any_installed() -> Option<&'static str> {
    CATALOG.iter().map(|m| m.name).find(|n| is_installed(n))
}

fn mirror_url(name: &str) -> String {
    format!("https://keyboop.com/models/whisper/ggml-{name}.bin")
}

fn hf_url(name: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/{PINNED_REVISION}/ggml-{name}.bin"
    )
}

#[derive(Debug)]
pub enum DownloadError {
    Cancelled,
    Failed(String),
}

/// Скачать файл по URL в `dest` с проверкой SHA-256 (если задан). Прогресс 0…1.
pub fn download_file(
    url: &str,
    dest: &std::path::Path,
    sha256: Option<&str>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<(), DownloadError> {
    let part = dest.with_extension("part");
    let resp = ureq::get(url)
        .call()
        .map_err(|e| DownloadError::Failed(e.to_string()))?;
    let total: u64 = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.into_body().into_reader();
    let mut file =
        std::fs::File::create(&part).map_err(|e| DownloadError::Failed(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Cancelled);
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| DownloadError::Failed(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| DownloadError::Failed(e.to_string()))?;
        done += n as u64;
        if total > 0 {
            progress(done as f64 / total as f64);
        }
    }
    drop(file);
    if let Some(expected) = sha256 {
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != expected {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Failed("хеш не сошёлся".into()));
        }
    }
    let _ = std::fs::remove_file(dest);
    std::fs::rename(&part, dest).map_err(|e| DownloadError::Failed(e.to_string()))
}

/// Скачать модель Whisper: зеркало, затем HuggingFace.
pub fn download_whisper(
    name: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<(), DownloadError> {
    let m = find(name).ok_or_else(|| DownloadError::Failed(format!("нет модели {name}")))?;
    let dest = whisper_path(name);
    match download_file(&mirror_url(name), &dest, Some(m.sha256), cancel, progress) {
        Ok(()) => Ok(()),
        Err(DownloadError::Cancelled) => Err(DownloadError::Cancelled),
        Err(_) => download_file(&hf_url(name), &dest, Some(m.sha256), cancel, progress),
    }
}

pub fn delete_whisper(name: &str) -> bool {
    let p = whisper_path(name);
    !p.exists() || std::fs::remove_file(p).is_ok()
}
