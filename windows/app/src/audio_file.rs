//! Чтение аудиофайла (mp3, m4a/aac, wav, flac, ogg/vorbis, alac) потоком в формат движков:
//! 16 кГц, моно, f32. Чистый Rust (symphonia), без системных кодеков. Весь файл в памяти не
//! держим: сэмплы уходят в `sink` небольшими порциями.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const RATE: u32 = 16_000;

/// Потоковый пересчёт частоты: при понижении каждое выходное значение — среднее по своему окну
/// входа (грубый, но честный фильтр от наложения), при повышении — линейная интерполяция.
pub struct Resampler {
    step: f64,
    pos: f64,
    buf: Vec<f32>,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Resampler {
            step: from as f64 / to as f64,
            pos: 0.0,
            buf: Vec::new(),
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if (self.step - 1.0).abs() < 1e-9 {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        loop {
            let a = self.pos.floor() as usize;
            if self.step > 1.0 {
                let b = (self.pos + self.step).floor() as usize;
                if b >= self.buf.len() {
                    break;
                }
                let w = &self.buf[a..b.max(a + 1)];
                out.push(w.iter().sum::<f32>() / w.len() as f32);
            } else {
                if a + 1 >= self.buf.len() {
                    break;
                }
                let f = (self.pos - a as f64) as f32;
                out.push(self.buf[a] * (1.0 - f) + self.buf[a + 1] * f);
            }
            self.pos += self.step;
        }
        let used = (self.pos.floor() as usize).min(self.buf.len());
        self.buf.drain(..used);
        self.pos -= used as f64;
    }
}

/// Длительность файла в секундах, если контейнер её знает.
pub fn duration(path: &Path) -> Option<f64> {
    let (format, _) = open(path).ok()?;
    let t = format.default_track()?;
    let rate = t.codec_params.sample_rate? as f64;
    Some(t.codec_params.n_frames? as f64 / rate)
}

fn open(path: &Path) -> Result<(Box<dyn symphonia::core::formats::FormatReader>, Hint), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("формат не распознан: {e}"))?;
    Ok((probed.format, hint))
}

/// Прочитать файл и отдавать 16 кГц моно порциями. `sink` возвращает false — остановиться.
pub fn decode(
    path: &Path,
    cancel: &AtomicBool,
    mut sink: impl FnMut(&[f32]) -> bool,
) -> Result<(), String> {
    let (mut format, _) = open(path)?;
    let track = format
        .default_track()
        .ok_or("в файле нет звуковой дорожки")?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("кодек не поддерживается: {e}"))?;
    let mut resampler: Option<(u32, Resampler)> = None;
    let mut mono = Vec::new();
    let mut out = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("отменено".into());
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // Битый пакет пропускаем: одна щель лучше, чем отказ от всего файла.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sb.copy_interleaved_ref(decoded);
        mono.clear();
        mono.extend(
            sb.samples()
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
        let rs = match &mut resampler {
            Some((rate, r)) if *rate == spec.rate => r,
            _ => {
                resampler = Some((spec.rate, Resampler::new(spec.rate, RATE)));
                &mut resampler.as_mut().unwrap().1
            }
        };
        out.clear();
        rs.process(&mono, &mut out);
        if !out.is_empty() && !sink(&out) {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_keeps_length_and_level() {
        let input: Vec<f32> = vec![0.5; 44_100];
        let mut r = Resampler::new(44_100, 16_000);
        let mut out = Vec::new();
        for c in input.chunks(1000) {
            r.process(c, &mut out);
        }
        assert!((out.len() as i64 - 16_000).abs() < 5, "{}", out.len());
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-5));
        let mut up = Resampler::new(8_000, 16_000);
        let mut out = Vec::new();
        up.process(&[0.0, 1.0, 0.0, 1.0], &mut out);
        assert_eq!(&out[..3], &[0.0, 0.5, 1.0]);
    }

    #[test]
    fn decodes_wav() {
        // WAV 22 050 Гц стерео, секунда тона.
        let rate = 22_050u32;
        let mut pcm = Vec::new();
        for i in 0..rate {
            let v = ((i as f32 * 0.05).sin() * 10_000.0) as i16;
            pcm.extend_from_slice(&v.to_le_bytes());
            pcm.extend_from_slice(&v.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(&pcm);
        let path = std::env::temp_dir().join("keyboop-decode-test.wav");
        std::fs::write(&path, wav).unwrap();
        let mut total = 0usize;
        decode(&path, &AtomicBool::new(false), |s| {
            total += s.len();
            true
        })
        .unwrap();
        let _ = std::fs::remove_file(&path);
        assert!((total as i64 - 16_000).abs() < 50, "{total}");
        assert!((duration(&std::env::temp_dir().join("нет.wav")).is_none()));
    }
}
