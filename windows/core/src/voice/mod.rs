//! Голосовой набор: всё, что не зависит от ОС и от движка распознавания. Перенос чистых частей
//! мак-версии: `WhisperPrompt`, `WhisperGhosts`, `VoiceDictionary`, `VoiceOutputCase` и
//! оформление результата из `VoiceController.deliver/applyOutputOptions`.

pub mod dictionary;
pub mod ghosts;
pub mod output_case;
pub mod prompt;

use dictionary::VoiceDictionary;

/// Параметры оформления готового текста (из настроек).
#[derive(Clone, Copy, Debug, Default)]
pub struct OutputOptions {
    pub no_final_period: bool,
    pub no_capital: bool,
    pub no_em_dash: bool,
}

/// Очистка распознанного: призраки Whisper → словарь диктовки. Это то, что идёт в историю.
pub fn clean(raw: &str, dict: &VoiceDictionary) -> String {
    dict.apply(ghosts::clean(raw).trim())
}

/// Оформление для вставки (`VoiceController.applyOutputOptions`).
pub fn apply_output_options(text: &str, o: OutputOptions, dict: &VoiceDictionary) -> String {
    let mut out = text.to_string();
    if o.no_final_period && out.ends_with('.') && !out.ends_with("..") {
        out.pop();
        let trimmed = out.trim_end_matches(' ').len();
        out.truncate(trimmed);
    }
    if o.no_capital {
        out = output_case::lowercased_sentence_starts(&out, &|t| dict.keeps_case(t));
    }
    if o.no_em_dash {
        out = out.replace(['—', '–'], "-");
    }
    out
}

/// Сколько слов в тексте (счётчик в меню).
pub fn word_count(s: &str) -> usize {
    s.split([' ', '\n', '\t']).filter(|w| !w.is_empty()).count()
}

/// RMS сигнала: тишину (микрофон молчал) не отправляем в распознавание.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_options() {
        let d = VoiceDictionary::new(vec![]);
        let o = OutputOptions {
            no_final_period: true,
            no_capital: true,
            no_em_dash: true,
        };
        assert_eq!(apply_output_options("Привет — мир.", o, &d), "привет - мир");
        assert_eq!(apply_output_options("Ну и что..", o, &d), "ну и что..");
    }
}
