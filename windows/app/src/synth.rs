//! Звуки Keyboop — тот же синтез, что `CueSynth.swift` на Маке: WAV собирается в памяти.
//! * переключение раскладки — два сухих «пыка» по 34 мс на 440 Гц;
//! * старт/стоп диктовки — «тук-тук» вверх (330→440 и 440→587);
//! * «не вышло» — вниз (300→225);
//! * перевод — мажорное арпеджио до-ми-соль.

const SAMPLE_RATE: f64 = 44_100.0;
const BASE_HZ: f64 = 330.0;
const RATIO: f64 = 440.0 / 330.0;
const TONE_DUR: f64 = 0.050;
const GAP_DUR: f64 = 0.030;
const AMP: f64 = 0.38;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    Switch,
    Start,
    Stop,
    Fail,
    Translate,
}

/// WAV звука. `volume` 0…1; 0.6 — громкость по умолчанию, при ней амплитуда как на Маке.
pub fn cue(kind: Cue, volume: f64) -> Vec<u8> {
    let amp = (AMP * volume.clamp(0.0, 1.0) / 0.6).min(0.95);
    let mut s = Vec::new();
    match kind {
        Cue::Switch => {
            dry(&mut s, amp);
            silence(&mut s, 0.022);
            dry(&mut s, amp);
        }
        Cue::Start => two_tone(&mut s, BASE_HZ, BASE_HZ * RATIO, amp),
        Cue::Stop => two_tone(&mut s, BASE_HZ * RATIO, BASE_HZ * RATIO * RATIO, amp),
        Cue::Fail => two_tone(&mut s, 300.0, 225.0, amp),
        Cue::Translate => {
            tone(&mut s, 330.0, amp);
            silence(&mut s, GAP_DUR * 0.6);
            tone(&mut s, 415.0, amp);
            silence(&mut s, GAP_DUR * 0.6);
            tone(&mut s, 523.0, amp);
        }
    }
    wav(&s)
}

/// Совместимость со старым вызовом: звук переключения.
pub fn switch_cue(volume: f64) -> Vec<u8> {
    cue(Cue::Switch, volume)
}

fn two_tone(s: &mut Vec<i16>, f1: f64, f2: f64, amp: f64) {
    tone(s, f1, amp);
    silence(s, GAP_DUR);
    tone(s, f2, amp);
}

fn tone(out: &mut Vec<i16>, freq: f64, amp: f64) {
    let n = (SAMPLE_RATE * TONE_DUR) as usize;
    for i in 0..n {
        let t = i as f64 / SAMPLE_RATE;
        let attack = (t / 0.007).min(1.0);
        let decay = (-t / (TONE_DUR * 0.5)).exp();
        let v = (2.0 * std::f64::consts::PI * freq * t).sin() * attack * decay * amp;
        out.push((v.clamp(-1.0, 1.0) * 32_767.0) as i16);
    }
}

fn dry(out: &mut Vec<i16>, amp: f64) {
    let dur = 0.034;
    let n = (SAMPLE_RATE * dur) as usize;
    for i in 0..n {
        let t = i as f64 / SAMPLE_RATE;
        let attack = (t / 0.007).min(1.0);
        let decay = (-t / (dur * 0.38)).exp();
        let v = (2.0 * std::f64::consts::PI * 440.0 * t).sin() * attack * decay * amp;
        out.push((v.clamp(-1.0, 1.0) * 32_767.0) as i16);
    }
}

fn silence(out: &mut Vec<i16>, dur: f64) {
    out.extend(std::iter::repeat_n(0, (SAMPLE_RATE * dur) as usize));
}

/// PCM 16 бит, моно.
pub fn wav_with_rate(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

fn wav(samples: &[i16]) -> Vec<u8> {
    wav_with_rate(samples, SAMPLE_RATE as u32)
}

/// 16 кГц float → WAV (клипы диктовок).
pub fn wav_from_f32(samples: &[f32], rate: u32) -> Vec<u8> {
    let s: Vec<i16> = samples
        .iter()
        .map(|v| (v.clamp(-1.0, 1.0) * 32_767.0) as i16)
        .collect();
    wav_with_rate(&s, rate)
}

/// WAV (PCM16 моно) → float. None — не наш формат.
#[cfg_attr(not(test), allow(dead_code))] // сквозной тест Parakeet
pub fn f32_from_wav(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let rate = u32::from_le_bytes(bytes[24..28].try_into().ok()?);
    let data = &bytes[44..];
    Some((
        data.chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32_768.0)
            .collect(),
        rate,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header() {
        let w = switch_cue(0.6);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize,
            w.len() - 8
        );
        for c in [Cue::Start, Cue::Stop, Cue::Fail, Cue::Translate] {
            assert!(cue(c, 0.6).len() > 1000);
        }
    }

    #[test]
    fn wav_roundtrip() {
        let s = vec![0.0f32, 0.5, -0.5];
        let (back, rate) = f32_from_wav(&wav_from_f32(&s, 16_000)).unwrap();
        assert_eq!(rate, 16_000);
        assert!((back[1] - 0.5).abs() < 0.001);
    }
}
