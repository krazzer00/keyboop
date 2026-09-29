//! Голосовой набор (перенос `VoiceController.swift`): держишь хоткей — запись → распознавание
//! (whisper.cpp или Parakeet, по настройке) → словарь диктовки → вставка текста туда, где курсор. Локально, ни байта в сеть.
//!
//! Поток диктовки принимает команды от хука (начать/закончить/отменить). Распознавание идёт на
//! отдельном потоке: запись закончилась — можно сразу начинать новую. Вставку делает поток
//! хуков, потому что вся наша синтетика идёт через его «забор» (см. hook.rs).

pub mod audio;
mod volume;
mod whisper;

use super::{app, history_store, hook, hud, sys};
use crate::l10n::t;
use crate::models;
use crate::synth::Cue;
use keyboop_core::history::HistoryKind;
use keyboop_core::voice;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub enum VoiceCmd {
    Begin,
    End,
    /// Esc: отменить (и, по настройке, распознать в историю).
    Cancel,
    /// Отменить молча: хоткей оказался частью чужого сочетания.
    Abort,
    Toggle,
    /// Заранее загрузить модель (после старта или смены модели).
    Preload,
}

/// Идёт запись — хук глотает Esc и не считает правый Alt чужим сочетанием.
pub static ACTIVE: AtomicBool = AtomicBool::new(false);
pub static VOICE_CHARS: AtomicU64 = AtomicU64::new(0);
pub static VOICE_WORDS: AtomicU64 = AtomicU64::new(0);

static TX: OnceLock<Mutex<Sender<VoiceCmd>>> = OnceLock::new();

pub fn send(cmd: VoiceCmd) {
    if let Some(tx) = TX.get() {
        let _ = tx.lock().unwrap().send(cmd);
    }
}

/// Словарь диктовки главного процесса (перечитывается при изменении файла).
pub static DICTIONARY: OnceLock<Mutex<voice::dictionary::VoiceDictionary>> = OnceLock::new();

pub fn dictionary() -> &'static Mutex<voice::dictionary::VoiceDictionary> {
    DICTIONARY.get_or_init(|| Mutex::new(voice::dictionary::VoiceDictionary::new(Vec::new())))
}

pub fn set_dictionary(pairs: Vec<(String, String)>) {
    let mut d = dictionary().lock().unwrap();
    d.set_all(pairs);
    if keyboop_core::layout_data::is_ready() {
        d.set_language_guard(Box::new(|w| {
            let ld = keyboop_core::layout_data::shared();
            ld.words_ru.contains(w) || ld.words_en.contains(w)
        }));
    }
}

/// Загруженный движок распознавания.
enum Asr {
    Whisper(whisper::Whisper),
    Parakeet(Box<crate::parakeet::Parakeet>),
}

struct Loaded {
    /// Имя модели Whisper или "parakeet".
    key: String,
    asr: Asr,
}

const PARAKEET: &str = "parakeet";

type Model = Arc<Mutex<Option<Loaded>>>;

struct Recording {
    rec: audio::Recorder,
    restore: volume::Restore,
}

pub fn cue(kind: Cue) {
    let (on, vol) = {
        let e = app().engine.lock().unwrap();
        (
            e.settings.voice_sound_enabled,
            e.settings.voice_sound_volume,
        )
    };
    if on {
        sys::play_cue(kind, vol);
    }
}

fn settings() -> keyboop_core::Settings {
    app().engine.lock().unwrap().settings.clone()
}

/// Что распознаёт: выбранный движок, а если его модели нет на диске — любая скачанная.
fn model_to_use(s: &keyboop_core::Settings) -> Option<String> {
    let parakeet = crate::parakeet::is_installed();
    if s.voice_engine == PARAKEET && parakeet {
        return Some(PARAKEET.into());
    }
    if models::is_installed(&s.voice_model) {
        return Some(s.voice_model.clone());
    }
    models::any_installed()
        .map(str::to_string)
        .or_else(|| parakeet.then(|| PARAKEET.into()))
}

fn ensure_loaded(model: &Model, name: &str) -> bool {
    let mut m = model.lock().unwrap();
    if m.as_ref().is_some_and(|l| l.key == name) {
        return true;
    }
    *m = None; // сначала освобождаем память прежней модели
    let t0 = Instant::now();
    let loaded = if name == PARAKEET {
        crate::parakeet::Parakeet::load(whisper::threads()).map(|p| Asr::Parakeet(Box::new(p)))
    } else {
        whisper::Whisper::load(&models::whisper_path(name), name).map(Asr::Whisper)
    };
    match loaded {
        Ok(asr) => {
            app().log(&format!(
                "voice: модель {name} загружена за {} мс",
                t0.elapsed().as_millis()
            ));
            *m = Some(Loaded {
                key: name.to_string(),
                asr,
            });
            true
        }
        Err(e) => {
            app().log(&format!("voice: модель {name} не загрузилась: {e}"));
            false
        }
    }
}

/// Распознать и доставить результат. `history_only` — отменённая Esc диктовка.
fn transcribe_and_deliver(
    model: Model,
    samples: Vec<f32>,
    history_only: bool,
    busy: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        busy.store(true, Ordering::Relaxed);
        let s = settings();
        let t0 = Instant::now();
        let text = match model_to_use(&s) {
            Some(name) if ensure_loaded(&model, &name) => {
                let hint = dictionary().lock().unwrap().recognition_hint(180);
                let mut m = model.lock().unwrap();
                match m.as_mut().map(|l| &mut l.asr) {
                    Some(Asr::Parakeet(p)) => match p.transcribe(&samples, &s.voice_language) {
                        Ok(text) => {
                            app().log(&format!(
                                "voice: parakeet → {} симв., {} мс",
                                text.chars().count(),
                                t0.elapsed().as_millis()
                            ));
                            text
                        }
                        Err(e) => {
                            app().log(&format!("voice: parakeet ошибка: {e}"));
                            String::new()
                        }
                    },
                    Some(Asr::Whisper(w)) => {
                        match w.transcribe(&samples, &s.voice_language, hint.as_deref()) {
                            Ok(tr) => {
                                app().log(&format!(
                                    "voice: whisper {} → {} симв., язык {} p={:.2}, {} мс",
                                    name,
                                    tr.text.chars().count(),
                                    tr.language,
                                    tr.probability,
                                    t0.elapsed().as_millis()
                                ));
                                tr.text
                            }
                            Err(e) => {
                                app().log(&format!("voice: whisper ошибка: {e}"));
                                String::new()
                            }
                        }
                    }
                    None => String::new(),
                }
            }
            _ => String::new(),
        };
        busy.store(false, Ordering::Relaxed);
        if s.voice_unload_after_dictation {
            *model.lock().unwrap() = None;
        }
        deliver(&s, &text, &samples, history_only);
    });
}

fn deliver(s: &keyboop_core::Settings, raw: &str, samples: &[f32], history_only: bool) {
    let clean = {
        let d = dictionary().lock().unwrap();
        voice::clean(raw, &d)
    };
    let clip = || {
        if s.voice_save_audio {
            history_store::save_clip(samples)
        } else {
            None
        }
    };
    if clean.is_empty() {
        if !history_only {
            cue(Cue::Fail);
        }
        hud::set(hud::HudState::Hidden, s.voice_hud_top);
        app().log("voice: пустой результат — ничего не вставляю");
        return;
    }
    VOICE_CHARS.fetch_add(clean.chars().count() as u64, Ordering::Relaxed);
    VOICE_WORDS.fetch_add(voice::word_count(&clean) as u64, Ordering::Relaxed);
    if s.voice_history_enabled {
        history_store::add_text(
            &clean,
            HistoryKind::Dictation,
            None,
            clip(),
            s.voice_history_minutes,
        );
    }
    if history_only {
        hud::toast(t("voice.escSaved"));
        return;
    }
    let out = {
        let d = dictionary().lock().unwrap();
        voice::apply_output_options(&clean, s.voice_output(), &d)
    };
    let out = if s.voice_trailing_space && !s.voice_auto_enter {
        format!("{out} ")
    } else {
        out
    };
    hud::set(hud::HudState::Hidden, s.voice_hud_top);
    hook::post_insert(out, s.voice_auto_enter);
    app().log(&format!("voice: вставлено {} симв.", clean.chars().count()));
}

/// Поток диктовки.
pub fn run() {
    let (tx, rx): (Sender<VoiceCmd>, Receiver<VoiceCmd>) = channel();
    let _ = TX.set(Mutex::new(tx));
    let levels = Arc::new(audio::Levels::new());
    hud::set_levels(levels.clone());
    let model: Model = Arc::new(Mutex::new(None));
    let busy = Arc::new(AtomicBool::new(false));
    let mut recording: Option<Recording> = None;
    let mut last_used = Instant::now();
    loop {
        let cmd = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => {
                // Модель, которой давно не пользовались, выгружаем: это сотни мегабайт памяти.
                let idle = settings().voice_model_idle_minutes;
                if idle > 0
                    && !busy.load(Ordering::Relaxed)
                    && last_used.elapsed() > Duration::from_secs(idle as u64 * 60)
                {
                    let mut m = model.lock().unwrap();
                    if m.is_some() {
                        *m = None;
                        app().log(&format!("voice: модель выгружена после {idle} мин простоя"));
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        last_used = Instant::now();
        let s = settings();
        let cmd = match cmd {
            VoiceCmd::Toggle => {
                if recording.is_some() {
                    VoiceCmd::End
                } else {
                    VoiceCmd::Begin
                }
            }
            c => c,
        };
        match cmd {
            VoiceCmd::Begin => {
                if recording.is_some() || !s.voice_enabled {
                    continue;
                }
                let Some(name) = model_to_use(&s) else {
                    hud::toast(t("voice.noModel"));
                    cue(Cue::Fail);
                    super::ui::open("voice");
                    continue;
                };
                cue(Cue::Start);
                let restore = volume::apply(
                    s.voice_duck,
                    s.voice_duck_level,
                    s.voice_mic_gain,
                    s.voice_mic_gain_level,
                );
                levels.clear();
                match audio::Recorder::start(&s.voice_mic, levels.clone()) {
                    Ok(rec) => {
                        ACTIVE.store(true, Ordering::Relaxed);
                        hud::set(hud::HudState::Recording, s.voice_hud_top);
                        recording = Some(Recording { rec, restore });
                        app().log("voice: запись началась");
                        // Пока человек говорит, модель уже грузится.
                        let m = model.clone();
                        std::thread::spawn(move || {
                            ensure_loaded(&m, &name);
                        });
                    }
                    Err(e) => {
                        volume::restore(restore);
                        app().log(&format!("voice: микрофон не открылся: {e}"));
                        hud::toast(t("voice.noMic"));
                        cue(Cue::Fail);
                    }
                }
            }
            VoiceCmd::End | VoiceCmd::Cancel | VoiceCmd::Abort => {
                let Some(r) = recording.take() else { continue };
                ACTIVE.store(false, Ordering::Relaxed);
                let duration = r.rec.duration();
                let samples = r.rec.stop();
                volume::restore(r.restore);
                if matches!(cmd, VoiceCmd::Abort) {
                    hud::set(hud::HudState::Hidden, s.voice_hud_top);
                    app().log("voice: отменено — хоткей оказался частью сочетания");
                    continue;
                }
                let cancel = matches!(cmd, VoiceCmd::Cancel);
                if !cancel {
                    cue(Cue::Stop);
                }
                let level = voice::rms(&samples);
                app().log(&format!(
                    "voice: {} сэмплов, {:.1} с, RMS {:.4}",
                    samples.len(),
                    duration,
                    level
                ));
                if duration < 0.3 {
                    hud::set(hud::HudState::Hidden, s.voice_hud_top);
                    continue;
                }
                if level <= 0.001 {
                    hud::toast(t("voice.silent"));
                    cue(Cue::Fail);
                    continue;
                }
                if cancel && !s.esc_save_to_history {
                    hud::toast(t("voice.cancelled"));
                    continue;
                }
                if cancel {
                    hud::set(hud::HudState::Hidden, s.voice_hud_top);
                } else {
                    hud::set(
                        hud::HudState::Processing("voice.processing"),
                        s.voice_hud_top,
                    );
                }
                transcribe_and_deliver(model.clone(), samples, cancel, busy.clone());
            }
            VoiceCmd::Preload => {
                if let Some(name) = model_to_use(&s) {
                    if !s.voice_unload_after_dictation {
                        let m = model.clone();
                        std::thread::spawn(move || {
                            ensure_loaded(&m, &name);
                        });
                    }
                }
            }
            VoiceCmd::Toggle => {}
        }
    }
}
