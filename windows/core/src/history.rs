//! Перенос `ClipboardHistoryCore.swift` и логики `VoiceHistory.swift`: единая история диктовок,
//! скопированного текста, импорта и звонков. Здесь только данные и политики; шифрование файла
//! (на Маке AES-GCM с ключом в Keychain, на Windows DPAPI) делает платформа.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum HistoryKind {
    Dictation,
    Clipboard,
    Imported,
    Call,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Время (секунды Unix).
    pub date: f64,
    pub text: String,
    /// Id аудиоклипа (если сохранялся).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    /// Огибающая звука для мини-волны в карточке (0…15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wave: Option<Vec<u8>>,
    #[serde(default)]
    pub kind: Option<HistoryKind>,
    /// Программа (для буфера обмена) или имя файла (для импорта).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

impl HistoryEntry {
    pub fn resolved_kind(&self) -> HistoryKind {
        self.kind.unwrap_or(HistoryKind::Dictation)
    }
    pub fn is_clipboard(&self) -> bool {
        self.resolved_kind() == HistoryKind::Clipboard
    }
    /// Импорт и звонки не подчищаются по сроку хранения: их человек положил туда сам.
    pub fn is_imported(&self) -> bool {
        matches!(
            self.resolved_kind(),
            HistoryKind::Imported | HistoryKind::Call
        )
    }
}

pub const MAX_DICTATION: usize = 3000;
pub const MAX_CLIPBOARD: usize = 100;
pub const MAX_IMPORTED: usize = 20;
/// Варианты срока хранения в минутах (0 — хранить всё).
pub const RETENTION_CHOICES: [u32; 6] = [30, 60, 480, 7 * 24 * 60, 30 * 24 * 60, 0];

/// Ограничить число записей каждого вида, выбрасывая самые старые. (оставшиеся, выброшенные)
pub fn capped(entries: &[HistoryEntry]) -> (Vec<HistoryEntry>, Vec<HistoryEntry>) {
    let (mut d, mut c, mut i, mut call) =
        (MAX_DICTATION, MAX_CLIPBOARD, MAX_IMPORTED, MAX_IMPORTED);
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for e in entries.iter().rev() {
        let slot = match e.resolved_kind() {
            HistoryKind::Dictation => &mut d,
            HistoryKind::Clipboard => &mut c,
            HistoryKind::Imported => &mut i,
            HistoryKind::Call => &mut call,
        };
        if *slot > 0 {
            *slot -= 1;
            kept.push(e.clone());
        } else {
            dropped.push(e.clone());
        }
    }
    kept.reverse();
    dropped.reverse();
    (kept, dropped)
}

pub fn last_dictation(entries: &[HistoryEntry]) -> Option<&HistoryEntry> {
    entries
        .iter()
        .rev()
        .find(|e| e.resolved_kind() == HistoryKind::Dictation)
}

pub fn last_clipboard_text(entries: &[HistoryEntry]) -> Option<&str> {
    entries
        .iter()
        .rev()
        .find(|e| e.is_clipboard())
        .map(|e| e.text.as_str())
}

/// Поиск по истории: текст или программа, без регистра.
pub fn matches(e: &HistoryEntry, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || e.text.to_lowercase().contains(&q)
        || e.app
            .as_ref()
            .is_some_and(|a| a.to_lowercase().contains(&q))
}

/// Удалить записи старше срока (кроме импорта/звонков). Возвращает id аудио удалённых записей.
pub fn prune(entries: &mut Vec<HistoryEntry>, minutes: u32, now: f64) -> Vec<String> {
    if minutes == 0 {
        return Vec::new();
    }
    let cutoff = now - minutes as f64 * 60.0;
    let old = |e: &HistoryEntry| e.date < cutoff && !e.is_imported();
    let audio: Vec<String> = entries
        .iter()
        .filter(|e| old(e))
        .filter_map(|e| e.audio.clone())
        .collect();
    entries.retain(|e| !old(e));
    audio
}

/// Огибающая записи для мини-волны: `buckets` пиков, 0…15 (корень для читаемости тихих мест).
pub fn envelope(samples: &[f32], buckets: usize) -> Vec<u8> {
    if samples.is_empty() {
        return Vec::new();
    }
    let per = (samples.len() / buckets).max(1);
    let peaks: Vec<f32> = samples
        .chunks(per)
        .take(buckets)
        .map(|c| c.iter().fold(0f32, |m, s| m.max(s.abs())))
        .collect();
    let top = peaks.iter().cloned().fold(0f32, f32::max);
    if top <= 0.0001 {
        return vec![0; peaks.len()];
    }
    peaks
        .iter()
        .map(|p| ((p / top).sqrt() * 15.0).round().clamp(0.0, 15.0) as u8)
        .collect()
}

/// Решение «класть ли скопированное в историю» (`ClipboardCapturePolicy`).
#[derive(Debug, PartialEq, Eq)]
pub enum ClipVerdict {
    Store(String),
    Skip(&'static str),
}

pub const CLIPBOARD_MAX_CHARS: usize = 20_000;

pub fn clipboard_verdict(
    text: Option<&str>,
    ours: bool,
    concealed: bool,
    last_stored: Option<&str>,
) -> ClipVerdict {
    if ours {
        return ClipVerdict::Skip("ours");
    }
    if concealed {
        return ClipVerdict::Skip("concealed");
    }
    let Some(text) = text else {
        return ClipVerdict::Skip("noText");
    };
    if text.trim().is_empty() {
        return ClipVerdict::Skip("empty");
    }
    if text.chars().count() > CLIPBOARD_MAX_CHARS {
        return ClipVerdict::Skip("tooLong");
    }
    if Some(text) == last_stored {
        return ClipVerdict::Skip("duplicate");
    }
    ClipVerdict::Store(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(date: f64, kind: HistoryKind) -> HistoryEntry {
        HistoryEntry {
            date,
            text: format!("t{date}"),
            audio: Some(format!("a{date}")),
            wave: None,
            kind: Some(kind),
            app: None,
        }
    }

    #[test]
    fn prune_keeps_imports() {
        let mut v = vec![
            e(0.0, HistoryKind::Dictation),
            e(0.0, HistoryKind::Imported),
            e(3000.0, HistoryKind::Dictation),
        ];
        let gone = prune(&mut v, 30, 3600.0);
        assert_eq!(gone, vec!["a0".to_string()]);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn caps_per_kind() {
        let v: Vec<HistoryEntry> = (0..105)
            .map(|i| e(i as f64, HistoryKind::Clipboard))
            .collect();
        let (kept, dropped) = capped(&v);
        assert_eq!(kept.len(), 100);
        assert_eq!(dropped.len(), 5);
        assert_eq!(kept[0].date, 5.0);
    }

    #[test]
    fn clipboard_rules() {
        assert_eq!(
            clipboard_verdict(Some("x"), false, false, Some("x")),
            ClipVerdict::Skip("duplicate")
        );
        assert_eq!(
            clipboard_verdict(Some("  "), false, false, None),
            ClipVerdict::Skip("empty")
        );
        assert_eq!(
            clipboard_verdict(Some("hi"), false, false, None),
            ClipVerdict::Store("hi".into())
        );
    }

    #[test]
    fn envelope_shape() {
        let s: Vec<f32> = (0..6400).map(|i| (i as f32 / 6400.0)).collect();
        let w = envelope(&s, 64);
        assert_eq!(w.len(), 64);
        assert_eq!(*w.last().unwrap(), 15);
    }
}
