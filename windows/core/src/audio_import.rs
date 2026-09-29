//! Перенос `AudioImportCore.swift`: резка длинной записи на куски по тишине, сборка текста с
//! абзацами по паузам, огибающая волны по ходу чтения и формат прогресса.
//!
//! Режем сами, а не отдаём файл движку целиком: часовой звонок — это 300 МБ сэмплов одним
//! массивом и ни слова о прогрессе, пока движок не закончит. Кусками по 20–45 секунд память
//! маленькая, прогресс честный, отмена работает между кусками. Режем в тишине, а не по
//! секундомеру: граница посреди слова стоила бы искажённого слова на каждом стыке.

pub const SAMPLE_RATE: usize = 16_000;
/// Окно RMS: 20 мс, как у самого whisper.
const FRAME_SECONDS: f64 = 0.02;
/// Раньше этого куска не режем даже в тишине: слишком короткие окна хуже распознаются.
const MIN_CHUNK_SECONDS: f64 = 20.0;
/// Не позже этого режем принудительно, в самом тихом месте хвоста.
const MAX_CHUNK_SECONDS: f64 = 45.0;
/// При принудительном резе самое тихое место ищем в последних секундах куска.
const HARD_CUT_LOOKBACK_SECONDS: f64 = 6.0;
/// Тише этого кадр считается тишиной. Комнатный шум записи звонка обычно 0.002–0.005.
const SILENCE_RMS: f32 = 0.006;
/// Столько тишины подряд — граница фразы, в ней можно резать.
const SILENCE_MIN_SECONDS: f64 = 0.35;
/// Пауза длиннее — новый абзац в тексте.
pub const PARAGRAPH_GAP_SECONDS: f64 = 1.6;
pub const ENVELOPE_BUCKETS: usize = 64;

fn frame_samples() -> usize {
    (SAMPLE_RATE as f64 * FRAME_SECONDS) as usize
}

/// Кусок записи, готовый для движка.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    /// Где кусок начинается в исходном файле (секунды).
    pub start_seconds: f64,
    /// Сколько тишины было ПЕРЕД куском: по ней сборщик решает, абзац это или та же фраза.
    pub gap_before: f64,
}

impl AudioChunk {
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / SAMPLE_RATE as f64
    }
}

pub fn rms(s: &[f32]) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
}

/// Резка потока сэмплов на куски. Кормить `feed`, в конце обязательно `flush`.
#[derive(Default)]
pub struct AudioChunker {
    pending: Vec<f32>,
    buffer: Vec<f32>,
    frame_rms: Vec<f32>,
    speech_seen: bool,
    silence_run: usize,
    gap_frames: usize,
    consumed: usize,
    chunk_start: usize,
}

impl AudioChunker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, samples: &[f32]) -> Vec<AudioChunk> {
        let frame = frame_samples();
        let mut out = Vec::new();
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(samples);
        let mut i = 0;
        while i + frame <= data.len() {
            if let Some(c) = self.push(&data[i..i + frame]) {
                out.push(c);
            }
            i += frame;
        }
        self.pending = data[i..].to_vec();
        out
    }

    /// Остаток после конца файла. None, если в остатке не было речи.
    pub fn flush(&mut self) -> Option<AudioChunk> {
        let frame = frame_samples();
        if !self.pending.is_empty() {
            let mut tail = std::mem::take(&mut self.pending);
            tail.resize(frame, 0.0);
            let _ = self.push(&tail);
        }
        if !self.speech_seen || self.buffer.is_empty() {
            return None;
        }
        let chunk = self.emit(self.buffer.len() - self.silence_run * frame);
        self.buffer.clear();
        self.frame_rms.clear();
        self.speech_seen = false;
        self.silence_run = 0;
        Some(chunk)
    }

    fn push(&mut self, slice: &[f32]) -> Option<AudioChunk> {
        let frame = frame_samples();
        self.consumed += frame;
        let r = rms(slice);
        let quiet = r < SILENCE_RMS;
        if !self.speech_seen {
            if quiet {
                self.gap_frames += 1; // ведущую тишину не копим, только считаем
                return None;
            }
            self.speech_seen = true;
            self.chunk_start = self.consumed - frame;
        }
        self.buffer.extend_from_slice(slice);
        self.frame_rms.push(r);
        self.silence_run = if quiet { self.silence_run + 1 } else { 0 };
        let seconds = self.buffer.len() as f64 / SAMPLE_RATE as f64;
        if seconds >= MIN_CHUNK_SECONDS
            && self.silence_run as f64 * FRAME_SECONDS >= SILENCE_MIN_SECONDS
        {
            // Режем по началу тишины: кусок заканчивается сразу после речи, тишина уходит в gap.
            let chunk = self.emit(self.buffer.len() - self.silence_run * frame);
            self.gap_frames = self.silence_run;
            self.buffer.clear();
            self.frame_rms.clear();
            self.speech_seen = false;
            self.silence_run = 0;
            return Some(chunk);
        }
        if seconds >= MAX_CHUNK_SECONDS {
            // Тишины не дождались: режем в самом тихом кадре хвоста.
            let lookback = (HARD_CUT_LOOKBACK_SECONDS / FRAME_SECONDS) as usize;
            let n = self.frame_rms.len();
            let from = n.saturating_sub(lookback).max(1);
            let mut best = n - 1;
            for f in from..n {
                if self.frame_rms[f] < self.frame_rms[best] {
                    best = f;
                }
            }
            let cut = best * frame;
            let chunk = self.emit(cut);
            self.buffer.drain(..cut);
            self.frame_rms.drain(..best);
            self.chunk_start += cut;
            self.gap_frames = 0;
            self.silence_run = self
                .frame_rms
                .iter()
                .rev()
                .take_while(|&&x| x < SILENCE_RMS)
                .count();
            self.speech_seen = self.frame_rms.iter().any(|&x| x >= SILENCE_RMS);
            if !self.speech_seen {
                self.gap_frames = self.frame_rms.len();
                self.buffer.clear();
                self.frame_rms.clear();
                self.silence_run = 0;
            }
            return Some(chunk);
        }
        None
    }

    fn emit(&self, end: usize) -> AudioChunk {
        let n = end.min(self.buffer.len());
        AudioChunk {
            samples: self.buffer[..n].to_vec(),
            start_seconds: self.chunk_start as f64 / SAMPLE_RATE as f64,
            gap_before: self.gap_frames as f64 * FRAME_SECONDS,
        }
    }
}

/// Огибающая волны для карточки, по ходу чтения: 64 корзины, как у клипов диктовки.
pub struct EnvelopeAccumulator {
    per_bucket: usize,
    peaks: Vec<f32>,
    index: usize,
}

impl EnvelopeAccumulator {
    pub fn new(total_samples: usize) -> Self {
        EnvelopeAccumulator {
            per_bucket: (total_samples / ENVELOPE_BUCKETS).max(1),
            peaks: vec![0.0; ENVELOPE_BUCKETS],
            index: 0,
        }
    }

    pub fn feed(&mut self, samples: &[f32]) {
        for v in samples {
            let b = (self.index / self.per_bucket).min(ENVELOPE_BUCKETS - 1);
            self.peaks[b] = self.peaks[b].max(v.abs());
            self.index += 1;
        }
    }

    pub fn finish(&self) -> Vec<u8> {
        let top = self.peaks.iter().cloned().fold(0.0, f32::max);
        if top <= 0.0001 {
            return vec![0; ENVELOPE_BUCKETS];
        }
        self.peaks
            .iter()
            .map(|p| ((p / top).sqrt() * 15.0).round().clamp(0.0, 15.0) as u8)
            .collect()
    }
}

/// Сборка кусков текста в документ: пауза длиннее порога — пустая строка между абзацами.
pub fn join_transcript(pieces: &[(String, f64)]) -> String {
    let mut out = String::new();
    for (text, gap) in pieces {
        let t = text.trim();
        if t.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str(if *gap >= PARAGRAPH_GAP_SECONDS {
                "\n\n"
            } else {
                " "
            });
        }
        out.push_str(t);
    }
    out
}

/// 83 → «1:23», 4523 → «1:15:23».
pub fn clock(seconds: f64) -> String {
    let s = seconds.round().max(0.0) as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// Оценка остатка по уже пройденному. None, пока пройдено меньше пяти секунд.
pub fn remaining(processed: f64, total: f64, elapsed: f64) -> Option<f64> {
    (processed >= 5.0 && total > processed && elapsed > 0.0)
        .then(|| (total - processed) * elapsed / processed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f64) -> Vec<f32> {
        (0..(seconds * SAMPLE_RATE as f64) as usize)
            .map(|i| (i as f32 * 0.05).sin() * 0.3)
            .collect()
    }

    fn silence(seconds: f64) -> Vec<f32> {
        vec![0.0; (seconds * SAMPLE_RATE as f64) as usize]
    }

    #[test]
    fn cuts_at_silence_after_min_length() {
        let mut c = AudioChunker::new();
        let mut s = silence(1.0);
        s.extend(tone(25.0));
        s.extend(silence(2.0));
        s.extend(tone(10.0));
        let mut chunks = c.feed(&s);
        chunks.extend(c.flush());
        assert_eq!(chunks.len(), 2);
        assert!((chunks[0].gap_before - 1.0).abs() < 0.05);
        assert!((chunks[0].seconds() - 25.0).abs() < 0.05);
        assert!(chunks[1].gap_before >= PARAGRAPH_GAP_SECONDS);
        assert!((chunks[1].start_seconds - 28.0).abs() < 0.05);
    }

    #[test]
    fn hard_cut_without_pauses() {
        let mut c = AudioChunker::new();
        let mut chunks = c.feed(&tone(100.0));
        chunks.extend(c.flush());
        assert!(chunks.len() >= 3);
        assert!(chunks
            .iter()
            .all(|ch| ch.seconds() <= MAX_CHUNK_SECONDS + 0.1));
        let total: f64 = chunks.iter().map(AudioChunk::seconds).sum();
        assert!((total - 100.0).abs() < 0.1);
    }

    #[test]
    fn silent_file_gives_nothing() {
        let mut c = AudioChunker::new();
        assert!(c.feed(&silence(30.0)).is_empty());
        assert!(c.flush().is_none());
    }

    #[test]
    fn assembles_paragraphs() {
        let t = join_transcript(&[
            ("Привет.".into(), 0.0),
            ("Как дела?".into(), 0.5),
            ("  ".into(), 0.0),
            ("Новая мысль.".into(), 2.0),
        ]);
        assert_eq!(t, "Привет. Как дела?\n\nНовая мысль.");
    }

    #[test]
    fn progress_format() {
        assert_eq!(clock(83.0), "1:23");
        assert_eq!(clock(4523.0), "1:15:23");
        assert_eq!(remaining(4.0, 100.0, 1.0), None);
        assert_eq!(remaining(10.0, 100.0, 2.0), Some(18.0));
    }

    #[test]
    fn envelope_is_normalised() {
        let mut e = EnvelopeAccumulator::new(6400);
        e.feed(&(0..6400).map(|i| i as f32 / 6400.0).collect::<Vec<_>>());
        let w = e.finish();
        assert_eq!(w.len(), ENVELOPE_BUCKETS);
        assert_eq!(*w.last().unwrap(), 15);
    }
}
