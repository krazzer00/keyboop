//! Обёртка над whisper.cpp (перенос `WhisperBridge.swift`). Локально, без сети.

use keyboop_core::voice::prompt;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct Whisper {
    ctx: WhisperContext,
}

/// Потоков на распознавание: все ядра, кроме двух (системе и интерфейсу), не больше 8.
pub fn threads() -> usize {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    n.saturating_sub(2).clamp(1, 8)
}

pub struct Transcript {
    pub text: String,
    /// Язык и уверенность (для лога и выбора затравки).
    pub language: String,
    pub probability: f32,
}

impl Whisper {
    pub fn load(path: &std::path::Path) -> Result<Whisper, String> {
        let p = path.to_str().ok_or("путь к модели")?;
        let ctx = WhisperContext::new_with_params(p, WhisperContextParameters::default())
            .map_err(|e| format!("{e:?}"))?;
        Ok(Whisper { ctx })
    }

    /// Распознать 16 кГц моно. `language`: "auto" | "ru" | "en". `hint` — слова словаря диктовки.
    pub fn transcribe(
        &self,
        samples: &[f32],
        language: &str,
        hint: Option<&str>,
    ) -> Result<Transcript, String> {
        let mut state = self.ctx.create_state().map_err(|e| format!("{e:?}"))?;
        let threads = threads();
        let mut decode_language = language.to_string();
        let mut probability = 0.0;
        let mut detected = false;
        if language == "auto" && state.pcm_to_mel(samples, threads).is_ok() {
            if let Ok((id, probs)) = state.lang_detect(0, threads) {
                if let Some(name) = whisper_rs::get_lang_str(id) {
                    decode_language = name.to_string();
                    probability = probs.get(id as usize).copied().unwrap_or(0.0);
                    detected = true;
                }
            }
        }
        let prompt_lang = prompt::prompt_language(
            language,
            detected.then_some(decode_language.as_str()),
            probability,
        );
        let initial = prompt::punctuation_prompt(&prompt_lang, hint);

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        params.set_no_timestamps(true);
        params.set_translate(false);
        params.set_suppress_blank(true);
        params.set_no_context(true); // не тащить контекст между диктовками
        params.set_single_segment(false);
        params.set_temperature(0.0);
        params.set_temperature_inc(0.2);
        params.set_entropy_thold(2.4);
        params.set_no_speech_thold(0.6);
        params.set_n_threads(threads as i32);
        let lang_for_decode = if decode_language == "auto" {
            None
        } else {
            Some(decode_language.as_str())
        };
        params.set_language(lang_for_decode);
        if !initial.is_empty() {
            params.set_initial_prompt(&initial);
        }
        state.full(params, samples).map_err(|e| format!("{e:?}"))?;
        let mut out = String::new();
        for i in 0..state.full_n_segments() {
            if let Some(seg) = state.get_segment(i) {
                if let Ok(t) = seg.to_str_lossy() {
                    out.push_str(&t);
                }
            }
        }
        let (text, _cut) = prompt::strip_echo(&out, &initial);
        Ok(Transcript {
            text,
            language: decode_language,
            probability,
        })
    }
}
