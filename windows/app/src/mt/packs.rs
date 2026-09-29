//! Пакеты перевода: что скачивать, куда класть, как проверять.
//!
//! Файлы берём с HuggingFace на ЗАКРЕПЛЁННОМ коммите (ветка с safetensors-вариантом модели) и
//! сверяем каждый по SHA-256: подменённый или битый файл в папку не попадёт.

use crate::models::{self, DownloadError};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub struct Pack {
    pub id: &'static str,
    repo: &'static str,
    revision: &'static str,
    pub size: &'static str,
    files: &'static [(&'static str, &'static str)],
}

pub const PACKS: &[Pack] = &[
    Pack {
        id: "ru-en",
        repo: "Helsinki-NLP/opus-mt-ru-en",
        revision: "7c84e70e05294db4fead6135d04585020413f78b",
        size: "310 MB",
        files: &[
            (
                "config.json",
                "5ea76c78596bce8fe005ef89e00de0924bf83e5f532ce08784ff1fcefb699f5f",
            ),
            (
                "source.spm",
                "745998e51ba5b058e38b7ac7765c25c43ed5c1c39cc92b27163b9b2e323c9d7c",
            ),
            (
                "vocab.json",
                "33e95da3be3fa3b50169c4c46693ba2f29fbf4cb29d99044bd07d72d181fa1e9",
            ),
            (
                "model.safetensors",
                "f73dc54675dc0da9ca6704098cadae3562f149b1f93c585f9d21c8502a760421",
            ),
        ],
    },
    Pack {
        id: "en-ru",
        repo: "Helsinki-NLP/opus-mt-en-ru",
        revision: "cd80d4414addb42379be7c8a75c72940372fa6c2",
        size: "310 MB",
        files: &[
            (
                "config.json",
                "d3c2614fd099bc228fc257f047e46f1c6f94cfa12dee2b1fa6c0d780e6b7de06",
            ),
            (
                "source.spm",
                "16bebef1389a0b8ab452772c4e35b9e605e5713f8ac7baa71ca701394eaa086d",
            ),
            (
                "vocab.json",
                "33e95da3be3fa3b50169c4c46693ba2f29fbf4cb29d99044bd07d72d181fa1e9",
            ),
            (
                "model.safetensors",
                "e0af2b7de8a1c4f7b83faee21c5d4391284998e693d76f3a995bfd5bc8dce7e8",
            ),
        ],
    },
];

pub fn find(id: &str) -> Option<&'static Pack> {
    PACKS.iter().find(|p| p.id == id)
}

pub fn dir(id: &str) -> PathBuf {
    models::dir().join("translate").join(id)
}

pub fn is_installed(id: &str) -> bool {
    find(id).is_some_and(|p| p.files.iter().all(|(f, _)| dir(id).join(f).is_file()))
}

/// Скачать пакет. Прогресс 0…1 — по большому файлу модели, мелкие идут первыми.
pub fn download(
    id: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<(), DownloadError> {
    let p = find(id).ok_or_else(|| DownloadError::Failed(format!("нет пакета {id}")))?;
    let d = dir(id);
    std::fs::create_dir_all(&d).map_err(|e| DownloadError::Failed(e.to_string()))?;
    for (name, sha) in p.files {
        let dest = d.join(name);
        if dest.is_file() {
            continue;
        }
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{name}",
            p.repo, p.revision
        );
        let big = *name == "model.safetensors";
        let mut sub = |x: f64| {
            if big {
                progress(0.02 + x * 0.98)
            }
        };
        models::download_file(&url, &dest, Some(sha), cancel, &mut sub)?;
    }
    progress(1.0);
    Ok(())
}

pub fn delete(id: &str) -> bool {
    let d = dir(id);
    !d.exists() || std::fs::remove_dir_all(d).is_ok()
}
