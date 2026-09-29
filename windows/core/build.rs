//! Списки слов берём ПРЯМО из `Sources/Keyboop/ExtraWords.swift`, а не копируем руками.
//!
//! Мак-версия и Windows-версия обязаны решать одинаково: каждый список там выстрадан отзывом
//! и замером. Ручная копия разошлась бы с оригиналом с первой же правки, и Windows молча
//! начала бы ломать то, что на Маке давно починено. Поэтому источник один, а здесь только
//! разбор Swift-литералов `static let имя: Set<String> = [ "…", … ]` в Rust-срезы.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let swift = manifest.join("../../Sources/Keyboop/ExtraWords.swift");
    println!("cargo:rerun-if-changed={}", swift.display());
    let src = fs::read_to_string(&swift)
        .unwrap_or_else(|e| panic!("не читается {}: {e}", swift.display()));

    let sets = parse_sets(&src);
    assert!(
        sets.len() >= 10,
        "в ExtraWords.swift найдено подозрительно мало списков: {}",
        sets.len()
    );

    let mut out = String::from(
        "// Сгенерировано build.rs из Sources/Keyboop/ExtraWords.swift. Не править руками.\n\n",
    );
    for (name, words) in &sets {
        out.push_str(&format!("pub static {}: &[&str] = &[\n", screaming(name)));
        for w in words {
            out.push_str(&format!("    {:?},\n", w));
        }
        out.push_str("];\n\n");
    }
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("extra_words.rs");
    fs::write(dest, out).unwrap();
}

/// Разбирает `static let NAME: Set<String> = [ ... ]`. Комментарии `//` вырезаются с учётом строк.
fn parse_sets(src: &str) -> Vec<(String, Vec<String>)> {
    let marker = "static let ";
    let mut res = Vec::new();
    let mut pos = 0;
    while let Some(i) = src[pos..].find(marker) {
        let start = pos + i + marker.len();
        pos = start;
        let rest = &src[start..];
        let Some(colon) = rest.find(':') else { break };
        let name = rest[..colon].trim().to_string();
        let after = &rest[colon + 1..];
        let Some(eq) = after.find('=') else { continue };
        if !after[..eq].contains("Set<String>") {
            continue;
        }
        let body = &after[eq + 1..];
        let Some(open) = body.find('[') else { continue };
        let (words, consumed) = parse_literal(&body[open + 1..]);
        pos = start + colon + 1 + eq + 1 + open + 1 + consumed;
        res.push((name, words));
    }
    res
}

/// Читает строки до закрывающей `]`. Возвращает слова и число прочитанных байт.
fn parse_literal(s: &str) -> (Vec<String>, usize) {
    let bytes: Vec<char> = s.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    let mut consumed = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
            while i < bytes.len() && bytes[i] != '\n' {
                consumed += bytes[i].len_utf8();
                i += 1;
            }
            continue;
        }
        if c == '"' {
            let mut w = String::new();
            consumed += 1;
            i += 1;
            while i < bytes.len() && bytes[i] != '"' {
                if bytes[i] == '\\' && i + 1 < bytes.len() {
                    consumed += 1;
                    i += 1;
                }
                w.push(bytes[i]);
                consumed += bytes[i].len_utf8();
                i += 1;
            }
            consumed += 1;
            i += 1;
            words.push(w);
            continue;
        }
        consumed += c.len_utf8();
        i += 1;
        if c == ']' {
            break;
        }
    }
    (words, consumed)
}

fn screaming(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}
