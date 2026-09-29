//! Звук переключения — тот же синтез, что `CueSynth.makeSwitchCue` на Маке: два сухих «пыка»
//! по 34 мс на 440 Гц с паузой 22 мс. Собираем WAV в памяти, файлов не нужно.

const SAMPLE_RATE: u32 = 44_100;

fn append_dry(out: &mut Vec<i16>, amp: f64) {
    let dur = 0.034;
    let freq = 440.0;
    let n = (SAMPLE_RATE as f64 * dur) as usize;
    for i in 0..n {
        let t = i as f64 / SAMPLE_RATE as f64;
        let attack = (t / 0.007).min(1.0);
        let decay = (-t / (dur * 0.38)).exp();
        let v = (2.0 * std::f64::consts::PI * freq * t).sin() * attack * decay * amp;
        out.push((v.clamp(-1.0, 1.0) * 32_767.0) as i16);
    }
}

/// WAV (PCM 16 бит, моно) звука переключения. `volume` 0…1.
pub fn switch_cue(volume: f64) -> Vec<u8> {
    // 0.6 — громкость по умолчанию, при ней амплитуда та же, что на Маке (0.38).
    let amp = (0.38 * volume.clamp(0.0, 1.0) / 0.6).min(0.95);
    let mut s = Vec::new();
    append_dry(&mut s, amp);
    s.extend(std::iter::repeat_n(
        0,
        (SAMPLE_RATE as f64 * 0.022) as usize,
    ));
    append_dry(&mut s, amp);
    wav(&s)
}

fn wav(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // моно
    w.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    w.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_header() {
        let w = super::switch_cue(0.6);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize,
            w.len() - 8
        );
    }
}
