//! Офлайн-перевод русский ↔ английский (аналог `TranslationEngine.swift`).
//!
//! На Маке переводит системный Apple Translation; у Windows такого нет, поэтому переводим сами:
//! модели OPUS-MT (Helsinki-NLP, Marian) на candle — чистый Rust, без сторонних DLL. Пакет языка
//! (~300 МБ на направление) скачивается только по кнопке в настройках; в работе сети нет.
//!
//! Текст переводится построчно: переносы строк, отступы и пустые строки остаются на местах.
//! Длинные строки режутся на предложения — модель обучена на предложениях, и так она не теряет
//! хвост длинного абзаца.

pub mod packs;
pub mod spm;

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::marian::{Config, MTModel};
use std::collections::HashMap;
use std::path::Path;

/// Направление по содержимому: кириллицы больше латиницы — с русского, иначе на русский.
pub fn direction(text: &str) -> &'static str {
    let (mut cyr, mut lat) = (0usize, 0usize);
    for c in text.chars() {
        if ('\u{0400}'..='\u{04FF}').contains(&c) {
            cyr += 1;
        } else if c.is_ascii_alphabetic() {
            lat += 1;
        }
    }
    if cyr > lat {
        "ru-en"
    } else {
        "en-ru"
    }
}

pub struct Translator {
    model: MTModel,
    cfg: Config,
    src: spm::Spm,
    vocab: HashMap<String, u32>,
    rev: Vec<String>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Translator {
    pub fn load(dir: &Path) -> Result<Translator, String> {
        let cfg: Config =
            serde_json::from_slice(&std::fs::read(dir.join("config.json")).map_err(err)?)
                .map_err(err)?;
        // Веса отображаются в память, а не читаются целиком: загрузка за доли секунды.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                &[dir.join("model.safetensors")],
                DType::F32,
                &Device::Cpu,
            )
        }
        .map_err(err)?;
        let model = MTModel::new(&cfg, vb).map_err(err)?;
        let src = spm::Spm::from_bytes(&std::fs::read(dir.join("source.spm")).map_err(err)?)?;
        let vocab: HashMap<String, u32> =
            serde_json::from_slice(&std::fs::read(dir.join("vocab.json")).map_err(err)?)
                .map_err(err)?;
        let mut rev = vec![String::new(); vocab.len()];
        for (k, &v) in &vocab {
            if let Some(slot) = rev.get_mut(v as usize) {
                *slot = k.clone();
            }
        }
        Ok(Translator {
            model,
            cfg,
            src,
            vocab,
            rev,
        })
    }

    /// Перевести текст целиком, сохранив строки и отступы.
    pub fn translate(&mut self, text: &str) -> Result<String, String> {
        let mut out = Vec::new();
        for line in text.split('\n') {
            let (body, cr) = match line.strip_suffix('\r') {
                Some(b) => (b, "\r"),
                None => (line, ""),
            };
            let trimmed = body.trim();
            if trimmed.is_empty() || !trimmed.chars().any(char::is_alphabetic) {
                out.push(line.to_string());
                continue;
            }
            let lead = &body[..body.len() - body.trim_start().len()];
            let tail = &body[body.trim_end().len()..];
            let mut parts = Vec::new();
            for s in sentences(trimmed) {
                parts.push(self.sentence(s).map_err(err)?);
            }
            out.push(format!("{lead}{}{tail}{cr}", parts.join(" ")));
        }
        Ok(out.join("\n"))
    }

    /// Одно предложение: кодировщик один раз, дальше жадный выбор слова за словом с кешем.
    fn sentence(&mut self, text: &str) -> candle_core::Result<String> {
        let unk = self.vocab.get("<unk>").copied().unwrap_or(1);
        let mut ids: Vec<u32> = self
            .src
            .encode(text)
            .iter()
            .map(|p| self.vocab.get(p).copied().unwrap_or(unk))
            .collect();
        ids.truncate(400);
        ids.push(self.cfg.eos_token_id);
        self.model.reset_kv_cache();
        let input = Tensor::new(ids.as_slice(), &Device::Cpu)?.unsqueeze(0)?;
        let enc = self.model.encoder().forward(&input, 0)?;
        let mut tokens = vec![self.cfg.decoder_start_token_id];
        let max = (ids.len() * 3 + 10).min(500);
        for index in 0..max {
            let ctx = if index == 0 {
                &tokens[..]
            } else {
                &tokens[tokens.len() - 1..]
            };
            let start = tokens.len() - ctx.len();
            let inp = Tensor::new(ctx, &Device::Cpu)?.unsqueeze(0)?;
            let logits = self.model.decode(&inp, &enc, start)?.squeeze(0)?;
            let last = logits.i(logits.dim(0)? - 1)?;
            let mut v: Vec<f32> = last.to_vec1()?;
            if let Some(p) = v.get_mut(self.cfg.pad_token_id as usize) {
                *p = f32::NEG_INFINITY; // <pad> модель выдавать не должна
            }
            let next = v
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i as u32)
                .unwrap_or(self.cfg.eos_token_id);
            if next == self.cfg.eos_token_id {
                break;
            }
            tokens.push(next);
        }
        let pieces: Vec<String> = tokens[1..]
            .iter()
            .filter_map(|&t| self.rev.get(t as usize))
            .filter(|p| !matches!(p.as_str(), "<pad>" | "</s>" | "<unk>"))
            .cloned()
            .collect();
        Ok(spm::Spm::decode(&pieces))
    }
}

/// Разрезать строку на предложения (короткую — не режем вовсе).
fn sentences(line: &str) -> Vec<&str> {
    if line.chars().count() < 200 {
        return vec![line];
    }
    let mut out = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let next_space = chars.get(k + 1).is_some_and(|&(_, n)| n == ' ');
        if matches!(c, '.' | '!' | '?' | '…') && next_space {
            let end = i + c.len_utf8();
            let s = line[start..end].trim();
            if !s.is_empty() {
                out.push(s);
            }
            start = end;
        }
    }
    let rest = line[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_direction() {
        assert_eq!(direction("Привет, мир"), "ru-en");
        assert_eq!(direction("Hello world"), "en-ru");
        assert_eq!(direction("Keyboop — переключатель"), "ru-en");
    }

    #[test]
    fn splits_long_lines_only() {
        assert_eq!(sentences("Коротко. Ещё."), vec!["Коротко. Ещё."]);
        let long = "Первое предложение довольно длинное, чтобы строка превысила порог. ".repeat(4);
        let parts = sentences(long.trim());
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().all(|p| p.ends_with('.')));
    }

    /// Сквозной тест на настоящих моделях: KEYBOOP_MT_DIR=папка с ru-en и en-ru.
    #[test]
    fn translates_with_real_models() {
        let Some(dir) = std::env::var_os("KEYBOOP_MT_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let mut ru_en = Translator::load(&dir.join("ru-en")).unwrap();
        let out = ru_en
            .translate("Кошка спит на диване.\n\n  Привет!")
            .unwrap();
        assert_eq!(out, "The cat sleeps on the couch.\n\n  Hey!");
        let mut en_ru = Translator::load(&dir.join("en-ru")).unwrap();
        assert_eq!(
            en_ru
                .translate("Please send me the report by Friday.")
                .unwrap(),
            "Пожалуйста, пришлите мне отчет к пятнице."
        );
    }
}
