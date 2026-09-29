//! Минимальный SentencePiece (unigram) для моделей OPUS-MT: разбор `.spm` (protobuf ModelProto),
//! нормализация, сегментация Витерби и обратная склейка. Ровно то, что делает MarianTokenizer:
//! куски из исходного `.spm`, номера — из общего `vocab.json`.

use std::collections::HashMap;

const SPACE: char = '\u{2581}'; // ▁

pub struct Spm {
    pieces: HashMap<String, f32>,
    max_len: usize,
    unk_score: f32,
}

/// Протобуф без схемы: достаём только нужные поля.
struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let byte = *self.b.get(self.i)?;
            self.i += 1;
            v |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }

    /// Следующее поле: (номер, тип, данные для типа 2 / значение для 0, 1, 5).
    fn field(&mut self) -> Option<(u64, Field<'a>)> {
        if self.i >= self.b.len() {
            return None;
        }
        let key = self.varint()?;
        let (num, wire) = (key >> 3, key & 7);
        let f = match wire {
            0 => Field::Varint(self.varint()?),
            1 => {
                let s = self.b.get(self.i..self.i + 8)?;
                self.i += 8;
                Field::Fixed64(u64::from_le_bytes(s.try_into().ok()?))
            }
            2 => {
                let n = self.varint()? as usize;
                let s = self.b.get(self.i..self.i + n)?;
                self.i += n;
                Field::Bytes(s)
            }
            5 => {
                let s = self.b.get(self.i..self.i + 4)?;
                self.i += 4;
                Field::Fixed32(u32::from_le_bytes(s.try_into().ok()?))
            }
            _ => return None,
        };
        Some((num, f))
    }
}

enum Field<'a> {
    Varint(u64),
    Fixed64(#[allow(dead_code)] u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

impl Spm {
    pub fn from_bytes(bytes: &[u8]) -> Result<Spm, String> {
        let mut r = Reader { b: bytes, i: 0 };
        let mut pieces = HashMap::new();
        let mut min_score = f32::MAX;
        let mut max_len = 1;
        while let Some((num, f)) = r.field() {
            // ModelProto.pieces = 1: SentencePiece { piece = 1, score = 2, type = 3 }.
            let (1, Field::Bytes(b)) = (num, f) else {
                continue;
            };
            let mut p = Reader { b, i: 0 };
            let (mut piece, mut score, mut kind) = (String::new(), 0f32, 1u64);
            while let Some((n, f)) = p.field() {
                match (n, f) {
                    (1, Field::Bytes(s)) => piece = String::from_utf8_lossy(s).into_owned(),
                    (2, Field::Fixed32(v)) => score = f32::from_bits(v),
                    (3, Field::Varint(v)) => kind = v,
                    _ => {}
                }
            }
            // NORMAL = 1, USER_DEFINED = 4. Служебные (<unk>, <s>, </s>) текст не режут.
            if kind == 1 || kind == 4 {
                min_score = min_score.min(score);
                max_len = max_len.max(piece.chars().count());
                pieces.insert(piece, score);
            }
        }
        if pieces.is_empty() {
            return Err("пустая модель SentencePiece".into());
        }
        Ok(Spm {
            pieces,
            max_len,
            unk_score: min_score - 10.0,
        })
    }

    /// Нормализация как у nmt_nfkc: NFKC, лишние пробелы прочь, пробел → ▁ и ▁ в начале.
    fn normalize(text: &str) -> Vec<char> {
        use unicode_normalization::UnicodeNormalization;
        let mut out = vec![SPACE];
        let mut space = true;
        for c in text.nfkc() {
            if c.is_whitespace() {
                if !space {
                    out.push(SPACE);
                    space = true;
                }
            } else if !c.is_control() {
                out.push(c);
                space = false;
            }
        }
        if out.len() > 1 && *out.last().unwrap() == SPACE {
            out.pop();
        }
        out
    }

    /// Разрезать текст на куски с наибольшей суммарной оценкой. Неизвестный символ — отдельный
    /// кусок (станет <unk>).
    pub fn encode(&self, text: &str) -> Vec<String> {
        let chars = Self::normalize(text);
        let n = chars.len();
        let mut best = vec![f32::NEG_INFINITY; n + 1];
        let mut back = vec![0usize; n + 1];
        best[0] = 0.0;
        let mut buf = String::new();
        for start in 0..n {
            if best[start] == f32::NEG_INFINITY {
                continue;
            }
            let mut any = false;
            buf.clear();
            for end in start + 1..=(start + self.max_len).min(n) {
                buf.push(chars[end - 1]);
                if let Some(&s) = self.pieces.get(buf.as_str()) {
                    any = true;
                    if best[start] + s > best[end] {
                        best[end] = best[start] + s;
                        back[end] = start;
                    }
                }
            }
            if !any && best[start] + self.unk_score > best[start + 1] {
                best[start + 1] = best[start] + self.unk_score;
                back[start + 1] = start;
            }
        }
        let mut out = Vec::new();
        let mut end = n;
        while end > 0 {
            let start = back[end];
            out.push(chars[start..end].iter().collect());
            end = start;
        }
        out.reverse();
        out
    }

    /// Склеить куски обратно в текст.
    pub fn decode(pieces: &[String]) -> String {
        let s: String = pieces.concat();
        s.replace(SPACE, " ").trim().to_string()
    }
}
