//! Запись звонка (перенос `CallRecorder.swift` + `CallRecordingCore.swift`): системный звук
//! (WASAPI loopback — то, что слышно в колонках, то есть собеседники) вместе с микрофоном, в моно
//! 16 кГц, с расшифровкой в историю после остановки.
//!
//! Как на Маке, функция скрытая: Shift+щелчок по значку в трее начинает и останавливает запись.
//!
//! # Подстраховка от потери записи («то, что сказано, потом никак не восстановишь»)
//!
//!   • звук пишется на диск сегментами по две минуты, каждый закрывается сразу: при сбое теряется
//!     не больше двух минут;
//!   • если микрофон перестал присылать звук дольше пяти секунд, захват пересоздаётся; три неудачи
//!     подряд — уведомление, записанное сохраняется;
//!   • незавершённая сессия (сбой, выключение) дорасшифровывается при следующем старте;
//!   • автостоп по тишине сначала спрашивает и только потом останавливает. Решает не отдельный
//!     кадр, а ДОЛЯ громкого времени в пятиминутном окне: щелчки клавиатуры в пустой комнате дают
//!     единицы процентов, живой разговор — десятки.
//!
//! Loopback молчит, пока ничего не играет (WASAPI не шлёт пустые буферы), поэтому часы записи —
//! микрофон, а системный звук подмешивается по мере прихода.

use super::import::Job;
use super::{app, hud, VoiceCmd};
use crate::audio_file::Resampler;
use crate::l10n::t;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use keyboop_core::audio_import::rms;
use keyboop_core::history::HistoryKind;
use std::collections::VecDeque;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RATE: u32 = 16_000;
/// Длина сегмента на диске.
const SEGMENT_SECONDS: usize = 120;
/// Окно, по которому решаем «звучит или нет», и доля громкого времени, ниже которой — тишина.
const SILENCE_WINDOW_SECONDS: usize = 300;
const LOUD_SHARE: f64 = 0.10;
const SILENCE_RMS: f32 = 0.006;
/// После вопроса «остановить?» ждём столько, потом останавливаем сами.
const AUTO_STOP_AFTER: Duration = Duration::from_secs(120);
/// Без звука от микрофона дольше этого захват считается умершим.
const STALL: Duration = Duration::from_secs(5);
const MAX_RESTARTS: u32 = 3;

pub static RECORDING: AtomicBool = AtomicBool::new(false);
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
/// Спросили «остановить?» — щелчок по уведомлению остановит.
pub static ASKED_TO_STOP: AtomicBool = AtomicBool::new(false);

struct Queue {
    samples: VecDeque<f32>,
    last: Instant,
}

/// Один поток захвата, приведённый к 16 кГц моно.
struct Capture {
    _stream: cpal::Stream,
    queue: Arc<Mutex<Queue>>,
}

impl Capture {
    fn start(device: &cpal::Device, loopback: bool) -> Result<Capture, String> {
        let config = if loopback {
            device.default_output_config()
        } else {
            device.default_input_config()
        }
        .map_err(|e| e.to_string())?;
        let channels = config.channels() as usize;
        let rate = config.sample_rate().0;
        let queue = Arc::new(Mutex::new(Queue {
            samples: VecDeque::new(),
            last: Instant::now(),
        }));
        let q = queue.clone();
        let mut rs = Resampler::new(rate, RATE);
        let mut mono = Vec::new();
        let mut out = Vec::new();
        let mut feed = move |data: &[f32]| {
            mono.clear();
            mono.extend(
                data.chunks(channels)
                    .map(|f| f.iter().sum::<f32>() / channels as f32),
            );
            out.clear();
            rs.process(&mono, &mut out);
            let mut q = q.lock().unwrap();
            q.samples.extend(out.iter().copied());
            q.last = Instant::now();
        };
        let cfg: cpal::StreamConfig = config.clone().into();
        let err = |_e: cpal::StreamError| {};
        // На устройстве вывода cpal строит входной поток в режиме loopback.
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(&cfg, move |d: &[f32], _| feed(d), err, None)
            }
            cpal::SampleFormat::I16 => device.build_input_stream(
                &cfg,
                move |d: &[i16], _| {
                    let f: Vec<f32> = d.iter().map(|s| *s as f32 / 32_768.0).collect();
                    feed(&f)
                },
                err,
                None,
            ),
            cpal::SampleFormat::I32 => device.build_input_stream(
                &cfg,
                move |d: &[i32], _| {
                    let f: Vec<f32> = d.iter().map(|s| *s as f32 / 2_147_483_648.0).collect();
                    feed(&f)
                },
                err,
                None,
            ),
            other => return Err(format!("формат не поддержан: {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Capture {
            _stream: stream,
            queue,
        })
    }

    fn take(&self, n: usize) -> Vec<f32> {
        let mut q = self.queue.lock().unwrap();
        let n = n.min(q.samples.len());
        q.samples.drain(..n).collect()
    }

    fn available(&self) -> usize {
        self.queue.lock().unwrap().samples.len()
    }

    fn silent_for(&self) -> Duration {
        self.queue.lock().unwrap().last.elapsed()
    }
}

fn mic_device(name: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if !name.is_empty() {
        if let Ok(mut it) = host.input_devices() {
            if let Some(d) = it.find(|d| d.name().is_ok_and(|n| n == name)) {
                return Some(d);
            }
        }
    }
    host.default_input_device()
}

/// WAV 16 бит, 16 кГц, моно; размеры в заголовке дописываются при закрытии.
struct WavWriter {
    file: std::fs::File,
    samples: u32,
}

impl WavWriter {
    fn create(path: &Path) -> std::io::Result<WavWriter> {
        let mut file = std::fs::File::create(path)?;
        file.write_all(&wav_header(0))?;
        Ok(WavWriter { file, samples: 0 })
    }

    fn write(&mut self, s: &[f32]) -> std::io::Result<()> {
        let bytes: Vec<u8> = s
            .iter()
            .flat_map(|v| ((v.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes())
            .collect();
        self.file.write_all(&bytes)?;
        self.samples += s.len() as u32;
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&wav_header(self.samples))?;
        self.file.sync_all()
    }
}

fn wav_header(samples: u32) -> Vec<u8> {
    let data = samples * 2;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&RATE.to_le_bytes());
    h.extend_from_slice(&(RATE * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

/// Сегмент, не закрытый из-за сбоя: дописать размеры по длине файла.
fn repair_wav(path: &Path) {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
        return;
    };
    if len <= 44 {
        let _ = std::fs::remove_file(path);
        return;
    }
    let samples = ((len - 44) / 2) as u32;
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.write_all(&wav_header(samples));
    }
}

fn calls_dir() -> PathBuf {
    super::history_store::data_dir().join("calls")
}

fn segments(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "wav"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

struct Session {
    stop: Arc<AtomicBool>,
    worker: std::thread::JoinHandle<()>,
    dir: PathBuf,
}

pub fn is_recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// Shift+щелчок по значку.
pub fn toggle() {
    if is_recording() {
        stop();
    } else {
        start();
    }
}

fn start() {
    let mut slot = SESSION.lock().unwrap();
    if slot.is_some() {
        return;
    }
    let dir = calls_dir().join(format!("{}", super::history_store::now() as u64));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app().log(&format!("звонок: папка не создана: {e}"));
        hud::toast(t("call.failed"));
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (stop2, dir2) = (stop.clone(), dir.clone());
    let mic_name = app().engine.lock().unwrap().settings.voice_mic.clone();
    let worker = std::thread::spawn(move || record(dir2, stop2, mic_name));
    *slot = Some(Session { stop, worker, dir });
    RECORDING.store(true, Ordering::Relaxed);
    ASKED_TO_STOP.store(false, Ordering::Relaxed);
    app().log("звонок: запись началась");
    super::cue(crate::synth::Cue::Start);
    hud::toast(t("call.started"));
    app().notify_ui(super::super::tray::WM_APP_REFRESH);
}

/// Остановить и отдать сегменты на расшифровку.
pub fn stop() {
    let Some(s) = SESSION.lock().unwrap().take() else {
        return;
    };
    s.stop.store(true, Ordering::Relaxed);
    let _ = s.worker.join();
    RECORDING.store(false, Ordering::Relaxed);
    ASKED_TO_STOP.store(false, Ordering::Relaxed);
    super::cue(crate::synth::Cue::Stop);
    app().notify_ui(super::super::tray::WM_APP_REFRESH);
    submit(s.dir);
}

fn submit(dir: PathBuf) {
    let sources = segments(&dir);
    if sources.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
        hud::toast(t("call.empty"));
        return;
    }
    app().log(&format!("звонок: {} сегм. на расшифровку", sources.len()));
    super::send(VoiceCmd::Import(Job {
        sources,
        kind: HistoryKind::Call,
        label: None,
        cleanup: Some(dir),
    }));
}

/// При старте: дорасшифровать сессии, оставшиеся от сбоя.
pub fn recover() {
    let Ok(it) = std::fs::read_dir(calls_dir()) else {
        return;
    };
    for dir in it
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
    {
        for seg in segments(&dir) {
            repair_wav(&seg);
        }
        app().log("звонок: нашлась незавершённая запись — расшифровываю");
        super::super::tray::balloon(t("call.recovering"), "");
        submit(dir);
    }
}

/// Поток записи: смешивание, сегменты, сторож, тишина.
fn record(dir: PathBuf, stop: Arc<AtomicBool>, mic_name: String) {
    let host = cpal::default_host();
    let open_mic = || mic_device(&mic_name).and_then(|d| Capture::start(&d, false).ok());
    let mut mic = open_mic();
    let sys = host
        .default_output_device()
        .and_then(|d| match Capture::start(&d, true) {
            Ok(c) => Some(c),
            Err(e) => {
                app().log(&format!("звонок: системный звук недоступен: {e}"));
                None
            }
        });
    if sys.is_none() {
        super::super::tray::balloon(t("call.noSystemAudioTitle"), t("call.noSystemAudioBody"));
    }
    if mic.is_none() {
        app().log("звонок: микрофон не открылся, пишу только системный звук");
    }
    let seg_len = SEGMENT_SECONDS * RATE as usize;
    let frame = RATE as usize / 50; // 20 мс
    let window_frames = SILENCE_WINDOW_SECONDS * 50;
    let mut loud: VecDeque<bool> = VecDeque::with_capacity(window_frames);
    let mut loud_count = 0usize;
    let mut asked_at: Option<Instant> = None;
    let mut restarts = 0u32;
    let mut seg_index = 0usize;
    let mut writer: Option<WavWriter> = None;
    let mut in_segment = 0usize;
    let mut pending: Vec<f32> = Vec::new();
    let started = Instant::now();
    let mut last_tick = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let stopping = stop.load(Ordering::Relaxed);
        // Сколько взять: по часам микрофона; без микрофона — по реальному времени.
        let n = match &mic {
            Some(m) => m.available(),
            None => {
                let n = (last_tick.elapsed().as_secs_f64() * RATE as f64) as usize;
                n.max(sys.as_ref().map_or(0, Capture::available))
            }
        };
        last_tick = Instant::now();
        let a = mic.as_ref().map(|m| m.take(n)).unwrap_or_default();
        let mut b = sys.as_ref().map(|s| s.take(n)).unwrap_or_default();
        // Системный звук не должен уходить вперёд больше чем на секунду: лишнее отбрасываем.
        if let Some(s) = &sys {
            let extra = s.available().saturating_sub(RATE as usize);
            if extra > 0 {
                let _ = s.take(extra);
            }
        }
        b.resize(n, 0.0);
        let mixed: Vec<f32> = (0..n)
            .map(|i| ((a.get(i).copied().unwrap_or(0.0) + b[i]) * 0.8).clamp(-1.0, 1.0))
            .collect();

        // Громкость по кадрам 20 мс для сторожа тишины.
        pending.extend_from_slice(&mixed);
        let mut i = 0;
        while i + frame <= pending.len() {
            let is_loud = rms(&pending[i..i + frame]) >= SILENCE_RMS;
            loud.push_back(is_loud);
            loud_count += usize::from(is_loud);
            if loud.len() > window_frames && loud.pop_front() == Some(true) {
                loud_count -= 1;
            }
            i += frame;
        }
        pending.drain(..i);

        // Запись сегментами.
        let mut rest: &[f32] = &mixed;
        while !rest.is_empty() {
            if writer.is_none() {
                seg_index += 1;
                match WavWriter::create(&dir.join(format!("seg-{seg_index:05}.wav"))) {
                    Ok(w) => writer = Some(w),
                    Err(e) => {
                        app().log(&format!("звонок: сегмент не создан: {e}"));
                        break;
                    }
                }
                in_segment = 0;
            }
            let take = (seg_len - in_segment).min(rest.len());
            if let Some(w) = writer.as_mut() {
                let _ = w.write(&rest[..take]);
            }
            in_segment += take;
            rest = &rest[take..];
            if in_segment >= seg_len {
                if let Some(w) = writer.take() {
                    let _ = w.finish();
                }
            }
        }
        if stopping {
            break;
        }

        // Сторож: микрофон замолчал совсем — пересоздать захват.
        if mic.as_ref().is_some_and(|m| m.silent_for() > STALL) {
            restarts += 1;
            app().log(&format!("звонок: микрофон замолчал, перезапуск {restarts}"));
            drop(mic.take()); // старый захват освобождаем до нового
            if restarts > MAX_RESTARTS {
                super::super::tray::balloon(t("call.stalledTitle"), t("call.stalledBody"));
                std::thread::spawn(self::stop);
                break;
            }
            mic = open_mic();
        }

        // Тишина: окно заполнено, громкого меньше порога — спросить, потом остановить.
        let full = loud.len() >= window_frames;
        let quiet = full && (loud_count as f64) < LOUD_SHARE * loud.len() as f64;
        match (quiet, asked_at) {
            (true, None) => {
                asked_at = Some(Instant::now());
                ASKED_TO_STOP.store(true, Ordering::Relaxed);
                super::super::tray::balloon(t("call.silenceTitle"), t("call.silenceBody"));
                app().log("звонок: пять минут тишины — спрашиваю");
            }
            (true, Some(at)) if at.elapsed() > AUTO_STOP_AFTER => {
                app().log("звонок: тишина не кончилась — останавливаю сам");
                std::thread::spawn(self::stop);
                break;
            }
            (false, Some(_)) => {
                asked_at = None;
                ASKED_TO_STOP.store(false, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    if let Some(w) = writer.take() {
        let _ = w.finish();
    }
    app().log(&format!(
        "звонок: запись остановлена, {:.0} с, {seg_index} сегм.",
        started.elapsed().as_secs_f64()
    ));
}
