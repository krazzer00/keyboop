//! Импорт аудиофайла и расшифровка записи звонка в историю (перенос `AudioImporter.swift`).
//!
//! Файл читается потоком уже в формате движка, `AudioChunker` режет его по тишине, каждый кусок
//! уходит в тот же движок, что и диктовка, текст собирается с абзацами по паузам. Памяти нужно на
//! один кусок, а не на весь файл; прогресс и отмена живут между кусками. Модель берётся на время
//! одного куска, так что диктовка посреди долгого импорта не ждёт его конца.
//!
//! Прогресс окно истории (отдельный процесс) читает из `import.json` в папке данных. В лог не
//! пишется ни имя файла, ни текст: только длительности и счётчики.

use super::{app, ensure_loaded, model_to_use, recognize, settings, Model};
use crate::audio_file;
use keyboop_core::audio_import::{self, AudioChunker, EnvelopeAccumulator};
use keyboop_core::history::{HistoryEntry, HistoryKind};
use keyboop_core::voice;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub struct Job {
    /// Файлы по порядку (у звонка — сегменты по две минуты).
    pub sources: Vec<PathBuf>,
    pub kind: HistoryKind,
    /// Подпись карточки: имя файла.
    pub label: Option<String>,
    /// Что удалить после (папка сессии звонка).
    pub cleanup: Option<PathBuf>,
}

/// Состояние для окна истории.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub running: bool,
    pub kind: Option<HistoryKind>,
    /// Секунд звука пройдено / всего (0 — неизвестно).
    pub processed: f64,
    pub total: f64,
    pub elapsed: f64,
    /// Итог последнего импорта (показать тостом).
    pub message: Option<String>,
    /// Растёт с каждым завершением: окно понимает, что итог новый.
    pub finished: u64,
}

pub static CANCEL: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<Status> = Mutex::new(Status {
    running: false,
    kind: None,
    processed: 0.0,
    total: 0.0,
    elapsed: 0.0,
    message: None,
    finished: 0,
});

pub fn status_path() -> PathBuf {
    super::history_store::data_dir().join("import.json")
}

fn publish(update: impl FnOnce(&mut Status)) {
    let mut s = STATUS.lock().unwrap();
    update(&mut s);
    if let Ok(json) = serde_json::to_vec(&*s) {
        let p = status_path();
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

/// Запустить в фоне. false — импорт уже идёт (движок общий, по одному за раз).
pub(super) fn start(model: Model, job: Job) -> bool {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return false;
    }
    CANCEL.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        let kind = job.kind;
        let cleanup = job.cleanup.clone();
        let result = run(&model, job);
        let message = match &result {
            Ok(true) => crate::l10n::t(if kind == HistoryKind::Call {
                "call.saved"
            } else {
                "import.saved"
            }),
            Ok(false) => crate::l10n::t("call.empty"),
            Err(e) if e == "cancelled" => crate::l10n::t("import.cancelled"),
            Err(_) => crate::l10n::t("import.failed"),
        };
        if let Err(e) = &result {
            app().log(&format!("импорт: {e}"));
        }
        // Сегменты звонка удаляем, только если расшифровка дошла до истории или речи не было:
        // при ошибке запись остаётся на диске и доберётся при следующем старте.
        if let (Some(dir), Ok(_)) = (cleanup, &result) {
            let _ = std::fs::remove_dir_all(dir);
        }
        super::hud::toast(message);
        publish(|s| {
            s.running = false;
            s.message = Some(message.to_string());
            s.finished += 1;
        });
        RUNNING.store(false, Ordering::SeqCst);
    });
    true
}

/// Ok(true) — записано в историю, Ok(false) — речи не нашлось.
fn run(model: &Model, job: Job) -> Result<bool, String> {
    let s = settings();
    let name = model_to_use(&s).ok_or("нет скачанной модели распознавания")?;
    if !ensure_loaded(model, &name) {
        return Err("модель не загрузилась".into());
    }
    let total: f64 = job
        .sources
        .iter()
        .filter_map(|p| audio_file::duration(p))
        .sum();
    let t0 = Instant::now();
    publish(|st| {
        *st = Status {
            running: true,
            kind: Some(job.kind),
            total,
            finished: st.finished,
            ..Status::default()
        }
    });
    let mut chunker = AudioChunker::new();
    let mut envelope =
        EnvelopeAccumulator::new(((total.max(1.0)) * audio_import::SAMPLE_RATE as f64) as usize);
    let mut pieces: Vec<(String, f64)> = Vec::new();
    let mut processed = 0usize;
    let mut chunks = 0usize;
    let transcribe = |chunk: audio_import::AudioChunk, pieces: &mut Vec<(String, f64)>| {
        let raw = recognize(model, &name, &s, &chunk.samples);
        let clean = {
            let d = super::dictionary().lock().unwrap();
            voice::clean(&raw, &d)
        };
        pieces.push((clean, chunk.gap_before));
    };
    for src in &job.sources {
        audio_file::decode(src, &CANCEL, |samples| {
            envelope.feed(samples);
            processed += samples.len();
            for chunk in chunker.feed(samples) {
                chunks += 1;
                transcribe(chunk, &mut pieces);
                let done = processed as f64 / audio_import::SAMPLE_RATE as f64;
                publish(|st| {
                    st.processed = done;
                    st.elapsed = t0.elapsed().as_secs_f64();
                });
                if CANCEL.load(Ordering::Relaxed) {
                    return false;
                }
            }
            true
        })
        .map_err(|e| {
            if e == "отменено" {
                "cancelled".to_string()
            } else {
                e
            }
        })?;
        if CANCEL.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
    }
    if let Some(chunk) = chunker.flush() {
        chunks += 1;
        transcribe(chunk, &mut pieces);
    }
    let text = audio_import::join_transcript(&pieces);
    let seconds = processed as f64 / audio_import::SAMPLE_RATE as f64;
    app().log(&format!(
        "импорт: {:.0} с звука, {chunks} кусков, {} симв., {} мс",
        seconds,
        text.chars().count(),
        t0.elapsed().as_millis()
    ));
    if text.trim().is_empty() {
        return Ok(false);
    }
    super::history_store::add(
        HistoryEntry {
            date: super::history_store::now(),
            text,
            audio: None,
            wave: Some(envelope.finish()),
            kind: Some(job.kind),
            app: job.label,
        },
        s.voice_history_minutes,
    );
    Ok(true)
}
