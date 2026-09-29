//! Распознавание речи Parakeet TDT 0.6B v3 (NVIDIA NeMo, 25 языков) — второй движок, как на Маке
//! (там он идёт через CoreML; здесь — ONNX Runtime на процессоре).
//!
//! Модель: ONNX-экспорт istupakov/parakeet-tdt-0.6b-v3-onnx (int8), закреплённая ревизия. Движок:
//! onnxruntime.dll из официального релиза Microsoft, скачивается вместе с моделью и грузится
//! динамически по полному пути — в exe ничего не линкуется, а системная onnxruntime.dll (её
//! ставит Windows ML) нас не касается. Всё проверяется по SHA-256.
//!
//! Язык модели задать нельзя: v3 сама решает, что звучит. Настройка «русский/английский»
//! работает как на Маке — фильтр ПИСЬМЕННОСТИ: если лучший кандидат не той письменности, берём
//! первый подходящий из ближайших. Так английская речь не запишется кириллицей.

use crate::models::{self, DownloadError};
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;

const REPO: &str = "istupakov/parakeet-tdt-0.6b-v3-onnx";
const REVISION: &str = "8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce";

/// (файл, размер, SHA-256). Мелкие первыми: сорвётся — так сорвётся быстро.
const FILES: &[(&str, u64, &str)] = &[
    (
        "vocab.txt",
        93_939,
        "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
    ),
    (
        "nemo128.onnx",
        139_764,
        "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f",
    ),
    (
        "decoder_joint-model.int8.onnx",
        18_202_004,
        "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
    ),
    (
        "encoder-model.int8.onnx",
        652_183_999,
        "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
    ),
];

const ORT_ZIP_URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-win-x64-1.22.0.zip";
const ORT_ZIP_SIZE: u64 = 72_368_545;
const ORT_ZIP_SHA: &str = "174c616efc0271194488642a72f1a514e01487da4dfe84c49296d66e40ebe0da";
const ORT_ZIP_ENTRY: &str = "onnxruntime-win-x64-1.22.0/lib/onnxruntime.dll";
const ORT_DLL: &str = "onnxruntime.dll";

pub const SIZE: &str = "750 MB";

pub fn dir() -> PathBuf {
    models::dir().join("parakeet")
}

fn runtime_path() -> PathBuf {
    // Для проверки на других системах путь к библиотеке можно подменить.
    std::env::var_os("KEYBOOP_ORT_DYLIB")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir().join(ORT_DLL))
}

pub fn is_installed() -> bool {
    let d = dir();
    FILES.iter().all(|(f, _, _)| d.join(f).is_file()) && runtime_path().is_file()
}

/// Скачать модель и движок. Прогресс 0…1 по байтам всего пакета.
pub fn download(cancel: &AtomicBool, progress: &mut dyn FnMut(f64)) -> Result<(), DownloadError> {
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| DownloadError::Failed(e.to_string()))?;
    let total: u64 = FILES.iter().map(|f| f.1).sum::<u64>() + ORT_ZIP_SIZE;
    let mut offset = 0u64;
    for (name, size, sha) in FILES {
        let dest = d.join(name);
        if !dest.is_file() {
            let url = format!("https://huggingface.co/{REPO}/resolve/{REVISION}/{name}");
            let mut sub = |x: f64| progress((offset as f64 + x * *size as f64) / total as f64);
            models::download_file(&url, &dest, Some(sha), cancel, &mut sub)?;
        }
        offset += size;
    }
    if !d.join(ORT_DLL).is_file() {
        let zip_path = d.join("onnxruntime.zip");
        let mut sub = |x: f64| progress((offset as f64 + x * ORT_ZIP_SIZE as f64) / total as f64);
        models::download_file(ORT_ZIP_URL, &zip_path, Some(ORT_ZIP_SHA), cancel, &mut sub)?;
        let r = extract(&zip_path, ORT_ZIP_ENTRY, &d.join(ORT_DLL));
        let _ = std::fs::remove_file(&zip_path);
        r.map_err(DownloadError::Failed)?;
    }
    progress(1.0);
    Ok(())
}

/// Достать один файл из zip.
fn extract(zip_path: &Path, entry: &str, dest: &Path) -> Result<(), String> {
    let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut src = z.by_name(entry).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("part");
    let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    std::io::copy(&mut src, &mut out).map_err(|e| e.to_string())?;
    drop(out);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

pub fn delete() -> bool {
    let d = dir();
    !d.exists() || std::fs::remove_dir_all(d).is_ok()
}

static INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Библиотека движка уже подгружена в процесс.
pub fn runtime_loaded() -> bool {
    INIT.get().is_some_and(|r| r.is_ok())
}

/// Подключить onnxruntime.dll (один раз за процесс).
fn init_runtime() -> Result<(), String> {
    INIT.get_or_init(|| {
        let p = runtime_path();
        ort::init_from(&p)
            .map_err(|e| format!("{}: {e}", p.display()))?
            .with_name("keyboop")
            .commit();
        Ok(())
    })
    .clone()
}

const DURATIONS: usize = 5;
const MAX_TOKENS_PER_STEP: usize = 10;
const STATE: usize = 2 * 640;

#[derive(PartialEq, Clone, Copy, Debug)]
enum Script {
    Cyrillic,
    Latin,
    Neutral,
}

fn script(piece: &str) -> Script {
    for c in piece.chars() {
        if ('\u{0400}'..='\u{04FF}').contains(&c) {
            return Script::Cyrillic;
        }
        if c.is_alphabetic() {
            return Script::Latin;
        }
    }
    Script::Neutral
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn argmax(v: &[f32]) -> usize {
    v.iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

pub struct Parakeet {
    pre: Session,
    enc: Session,
    dec: Session,
    vocab: Vec<String>,
    blank: usize,
}

impl Parakeet {
    pub fn load(threads: usize) -> Result<Parakeet, String> {
        Self::load_from(&dir(), threads)
    }

    pub fn load_from(d: &Path, threads: usize) -> Result<Parakeet, String> {
        init_runtime()?;
        let open = |f: &str| -> Result<Session, String> {
            Session::builder()
                .map_err(err)?
                .with_intra_threads(threads)
                .map_err(err)?
                .commit_from_file(d.join(f))
                .map_err(err)
        };
        let vocab_txt = std::fs::read_to_string(d.join("vocab.txt")).map_err(err)?;
        let mut vocab = Vec::new();
        for line in vocab_txt.lines() {
            let Some((tok, id)) = line.rsplit_once(' ') else {
                continue;
            };
            let id: usize = id.parse().map_err(err)?;
            if vocab.len() <= id {
                vocab.resize(id + 1, String::new());
            }
            vocab[id] = tok.to_string();
        }
        let blank = vocab
            .iter()
            .position(|t| t == "<blk>")
            .ok_or("в словаре нет <blk>")?;
        Ok(Parakeet {
            pre: open("nemo128.onnx")?,
            enc: open("encoder-model.int8.onnx")?,
            dec: open("decoder_joint-model.int8.onnx")?,
            vocab,
            blank,
        })
    }

    /// 16 кГц моно → текст. Длинная запись режется на куски по минуте в тихих местах.
    pub fn transcribe(&mut self, samples: &[f32], language: &str) -> Result<String, String> {
        let mut parts = Vec::new();
        for (a, b) in chunks(samples) {
            let t = self.transcribe_chunk(&samples[a..b], language)?;
            if !t.is_empty() {
                parts.push(t);
            }
        }
        Ok(parts.join(" "))
    }

    fn transcribe_chunk(&mut self, chunk: &[f32], language: &str) -> Result<String, String> {
        // Секунда тишины в конце: без неё модель не дописывает последние слова.
        let mut samples = chunk.to_vec();
        samples.extend(std::iter::repeat_n(0.0, 16_000));
        let n = samples.len();
        let wave = Tensor::from_array(([1usize, n], samples.into_boxed_slice())).map_err(err)?;
        let lens =
            Tensor::from_array(([1usize], vec![n as i64].into_boxed_slice())).map_err(err)?;
        let out = self
            .pre
            .run(ort::inputs!["waveforms" => wave, "waveforms_lens" => lens])
            .map_err(err)?;
        let (fshape, feats) = out["features"].try_extract_tensor::<f32>().map_err(err)?;
        let fshape: Vec<usize> = fshape.iter().map(|&d| d as usize).collect();
        let (_, flen) = out["features_lens"]
            .try_extract_tensor::<i64>()
            .map_err(err)?;
        let feats = Tensor::from_array((fshape, feats.to_vec().into_boxed_slice())).map_err(err)?;
        let flen = Tensor::from_array(([1usize], vec![flen[0]].into_boxed_slice())).map_err(err)?;
        drop(out);

        let out = self
            .enc
            .run(ort::inputs!["audio_signal" => feats, "length" => flen])
            .map_err(err)?;
        let (eshape, encoded) = out["outputs"].try_extract_tensor::<f32>().map_err(err)?;
        let (_, elen) = out["encoded_lengths"]
            .try_extract_tensor::<i64>()
            .map_err(err)?;
        let (dim, stride) = (eshape[1] as usize, eshape[2] as usize);
        let frames = stride.min(elen[0].max(0) as usize);
        let encoded = encoded.to_vec();
        drop(out);

        let want = match language {
            "ru" => Some(Script::Cyrillic),
            "en" => Some(Script::Latin),
            _ => None,
        };
        let vsize = self.vocab.len();
        let (mut s1, mut s2) = (vec![0f32; STATE], vec![0f32; STATE]);
        let mut tokens: Vec<usize> = Vec::new();
        let (mut t, mut emitted) = (0usize, 0usize);
        while t < frames {
            let frame: Vec<f32> = (0..dim).map(|d| encoded[d * stride + t]).collect();
            let prev = *tokens.last().unwrap_or(&self.blank) as i32;
            let inputs = ort::inputs![
                "encoder_outputs" => Tensor::from_array(([1usize, dim, 1], frame.into_boxed_slice())).map_err(err)?,
                "targets" => Tensor::from_array(([1usize, 1], vec![prev].into_boxed_slice())).map_err(err)?,
                "target_length" => Tensor::from_array(([1usize], vec![1i32].into_boxed_slice())).map_err(err)?,
                "input_states_1" => Tensor::from_array(([2usize, 1, 640], s1.clone().into_boxed_slice())).map_err(err)?,
                "input_states_2" => Tensor::from_array(([2usize, 1, 640], s2.clone().into_boxed_slice())).map_err(err)?,
            ];
            let out = self.dec.run(inputs).map_err(err)?;
            let (_, logits) = out["outputs"].try_extract_tensor::<f32>().map_err(err)?;
            if logits.len() < vsize + DURATIONS {
                return Err("неожиданный размер выхода декодера".into());
            }
            let (tok_logits, dur_logits) = logits.split_at(vsize);
            let token = pick(&self.vocab, self.blank, tok_logits, want);
            let step = argmax(&dur_logits[..DURATIONS]);
            if token != self.blank {
                let (_, n1) = out["output_states_1"]
                    .try_extract_tensor::<f32>()
                    .map_err(err)?;
                let (_, n2) = out["output_states_2"]
                    .try_extract_tensor::<f32>()
                    .map_err(err)?;
                s1.copy_from_slice(n1);
                s2.copy_from_slice(n2);
                tokens.push(token);
                emitted += 1;
            }
            if step > 0 {
                t += step;
                emitted = 0;
            } else if token == self.blank || emitted >= MAX_TOKENS_PER_STEP {
                t += 1;
                emitted = 0;
            }
        }
        let text: String = tokens
            .iter()
            .map(|&i| self.vocab[i].as_str())
            .filter(|p| !(p.starts_with('<') && p.ends_with('>')))
            .collect();
        Ok(text.replace('\u{2581}', " ").trim().to_string())
    }
}

/// Лучший токен с фильтром письменности.
fn pick(vocab: &[String], blank: usize, logits: &[f32], want: Option<Script>) -> usize {
    let best = argmax(logits);
    let Some(want) = want else { return best };
    let ok = |i: usize| i == blank || script(&vocab[i]) != other(want);
    if ok(best) {
        return best;
    }
    let mut idx: Vec<usize> = (0..logits.len()).collect();
    idx.sort_unstable_by(|&a, &b| logits[b].total_cmp(&logits[a]));
    idx.into_iter().take(8).find(|&i| ok(i)).unwrap_or(best)
}

fn other(s: Script) -> Script {
    match s {
        Script::Cyrillic => Script::Latin,
        Script::Latin => Script::Cyrillic,
        Script::Neutral => Script::Neutral,
    }
}

/// Границы кусков: до полутора минут — одним куском, дальше по минуте, разрез в самом тихом
/// месте ±5 секунд от минутной отметки (чтобы не резать слово пополам).
fn chunks(samples: &[f32]) -> Vec<(usize, usize)> {
    const SR: usize = 16_000;
    let n = samples.len();
    if n <= 90 * SR {
        return vec![(0, n)];
    }
    let win = SR * 3 / 10;
    let mut out = Vec::new();
    let mut start = 0;
    while n - start > 90 * SR {
        let target = start + 60 * SR;
        let (lo, hi) = (target - 5 * SR, (target + 5 * SR).min(n - win));
        let mut best = (f32::MAX, target);
        let mut i = lo;
        while i < hi {
            let e: f32 = samples[i..i + win].iter().map(|x| x * x).sum();
            if e < best.0 {
                best = (e, i + win / 2);
            }
            i += SR / 20;
        }
        out.push((start, best.1));
        start = best.1;
    }
    out.push((start, n));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking() {
        assert_eq!(chunks(&vec![0.0; 16_000 * 10]), vec![(0, 160_000)]);
        let long = vec![0.1; 16_000 * 200];
        let c = chunks(&long);
        assert!(c.len() >= 3);
        assert_eq!(c.first().unwrap().0, 0);
        assert_eq!(c.last().unwrap().1, long.len());
        assert!(c.windows(2).all(|w| w[0].1 == w[1].0));
    }

    #[test]
    fn scripts() {
        assert_eq!(script("▁привет"), Script::Cyrillic);
        assert_eq!(script("▁hello"), Script::Latin);
        assert_eq!(script(","), Script::Neutral);
    }

    /// Сквозной тест: KEYBOOP_PARAKEET_DIR (модель), KEYBOOP_ORT_DYLIB (libonnxruntime),
    /// KEYBOOP_PARAKEET_WAV (16 кГц, 16 бит) и KEYBOOP_PARAKEET_EXPECT (подстрока результата).
    #[test]
    fn transcribes_with_real_model() {
        let (Some(d), Some(wav), Some(expect)) = (
            std::env::var_os("KEYBOOP_PARAKEET_DIR"),
            std::env::var_os("KEYBOOP_PARAKEET_WAV"),
            std::env::var("KEYBOOP_PARAKEET_EXPECT").ok(),
        ) else {
            return;
        };
        let bytes = std::fs::read(wav).unwrap();
        let (samples, rate) = crate::synth::f32_from_wav(&bytes).unwrap();
        assert_eq!(rate, 16_000);
        let mut p = Parakeet::load_from(Path::new(&d), 4).unwrap();
        let text = p.transcribe(&samples, "auto").unwrap();
        assert!(text.contains(&expect), "{text}");
    }
}
