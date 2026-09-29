//! Запись микрофона (WASAPI через cpal) → 16 кГц моно float для whisper. Аналог `AudioRecorder.swift`.
//!
//! Пишем в родной частоте устройства, а в 16 кГц переводим один раз в конце: так в колбэке звука
//! нет ничего тяжелее сложения. Уровень громкости (для волны на плашке) считается блоками ~30 мс.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const TARGET_RATE: u32 = 16_000;

/// Последние уровни громкости для волны на плашке (0…1, по блоку ~30 мс).
pub struct Levels {
    ring: Mutex<Vec<f32>>,
}

impl Levels {
    pub fn new() -> Self {
        Levels {
            ring: Mutex::new(Vec::new()),
        }
    }
    fn push(&self, v: f32) {
        let mut r = self.ring.lock().unwrap();
        r.push(v);
        if r.len() > 64 {
            let extra = r.len() - 64;
            r.drain(..extra);
        }
    }
    /// Последние `n` уровней (старые слева).
    pub fn last(&self, n: usize) -> Vec<f32> {
        let r = self.ring.lock().unwrap();
        let start = r.len().saturating_sub(n);
        let mut v = vec![0.0; n - (r.len() - start)];
        v.extend_from_slice(&r[start..]);
        v
    }
    pub fn clear(&self) {
        self.ring.lock().unwrap().clear();
    }
}

struct Shared {
    samples: Vec<f32>,
    block_sum: f32,
    block_n: usize,
}

pub struct Recorder {
    stream: cpal::Stream,
    shared: Arc<Mutex<Shared>>,
    rate: u32,
    started: Instant,
}

/// Имена микрофонов для настроек.
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn pick_device(name: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if !name.is_empty() {
        if let Ok(mut it) = host.input_devices() {
            if let Some(d) = it.find(|d| d.name().map(|n| n == name).unwrap_or(false)) {
                return Some(d);
            }
        }
    }
    host.default_input_device()
}

/// Сработавший раз: 0 — нет ошибки, иначе номер ошибки потока (для лога).
pub static STREAM_ERRORS: AtomicU32 = AtomicU32::new(0);

impl Recorder {
    /// Начать запись. `levels` получает уровни для плашки.
    pub fn start(device_name: &str, levels: Arc<Levels>) -> Result<Recorder, String> {
        let device = pick_device(device_name).ok_or("нет микрофона")?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let block = (rate as usize * 30 / 1000).max(1) * channels;
        let shared = Arc::new(Mutex::new(Shared {
            samples: Vec::with_capacity(rate as usize * 30),
            block_sum: 0.0,
            block_n: 0,
        }));
        let s2 = shared.clone();
        let err = |e: cpal::StreamError| {
            let _ = e;
            STREAM_ERRORS.fetch_add(1, Ordering::Relaxed);
        };
        let feed = move |data: &[f32]| {
            let mut sh = s2.lock().unwrap();
            for frame in data.chunks(channels) {
                let v = frame.iter().sum::<f32>() / channels as f32;
                sh.samples.push(v);
                sh.block_sum += v * v;
                sh.block_n += channels;
                if sh.block_n >= block {
                    let rms = (sh.block_sum / (sh.block_n / channels) as f32).sqrt();
                    levels.push((rms * 6.0).min(1.0));
                    sh.block_sum = 0.0;
                    sh.block_n = 0;
                }
            }
        };
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(&stream_config, move |d: &[f32], _| feed(d), err, None)
            }
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |d: &[i16], _| {
                    let f: Vec<f32> = d.iter().map(|s| *s as f32 / 32_768.0).collect();
                    feed(&f)
                },
                err,
                None,
            ),
            cpal::SampleFormat::I32 => device.build_input_stream(
                &stream_config,
                move |d: &[i32], _| {
                    let f: Vec<f32> = d.iter().map(|s| *s as f32 / 2_147_483_648.0).collect();
                    feed(&f)
                },
                err,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &stream_config,
                move |d: &[u16], _| {
                    let f: Vec<f32> = d
                        .iter()
                        .map(|s| (*s as f32 - 32_768.0) / 32_768.0)
                        .collect();
                    feed(&f)
                },
                err,
                None,
            ),
            other => return Err(format!("формат микрофона не поддержан: {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Recorder {
            stream,
            shared,
            rate,
            started: Instant::now(),
        })
    }

    pub fn duration(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Остановить и отдать запись в 16 кГц моно.
    pub fn stop(self) -> Vec<f32> {
        let _ = self.stream.pause();
        drop(self.stream); // микрофон освобождаем сразу: не держим индикатор записи в простое
        let samples = std::mem::take(&mut self.shared.lock().unwrap().samples);
        resample(&samples, self.rate, TARGET_RATE)
    }
}

/// Перевод частоты: усреднение по окну (сглаживает наложение при понижении) + линейная интерполяция.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    let half = if ratio > 1.0 {
        (ratio / 2.0).floor() as isize
    } else {
        0
    };
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as isize;
        if half > 0 {
            let lo = (idx - half).max(0) as usize;
            let hi = ((idx + half) as usize).min(input.len() - 1);
            let sum: f32 = input[lo..=hi].iter().sum();
            out.push(sum / (hi - lo + 1) as f32);
        } else {
            let a = input[(idx as usize).min(input.len() - 1)];
            let b = input[(idx as usize + 1).min(input.len() - 1)];
            let t = (pos - idx as f64) as f32;
            out.push(a + (b - a) * t);
        }
    }
    out
}
