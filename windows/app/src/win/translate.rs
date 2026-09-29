//! Перевод выделенного по хоткею (аналог `TranslationEngine.swift` + обработчика в `Engine`).
//!
//! Выделение читает рабочий поток (Ctrl+C с возвратом буфера), перевод идёт на своём потоке —
//! модель думает полсекунды на предложение, и держать хук всё это время нельзя. Готовый перевод
//! печатается поверх всё ещё выделенного текста, как на Маке.

use super::clipboard::Selection;
use super::{app, hud};
use crate::l10n::t;
use crate::mt::{self, packs, Translator};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Модель перевода выгружается после простоя: 300 МБ в памяти ради редкого хоткея — дорого.
const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);

fn sender() -> &'static Mutex<Sender<String>> {
    static TX: OnceLock<Mutex<Sender<String>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("translate".into())
            .spawn(move || run(rx))
            .expect("поток перевода");
        Mutex::new(tx)
    })
}

/// Выделение прочитано (поток хуков): проверить и отдать на перевод.
pub fn apply(sel: Option<&Selection>) {
    let Some(text) = sel
        .map(|s| s.text.clone())
        .filter(|t| t.chars().any(char::is_alphabetic))
    else {
        hud::toast(t("translate.nothing"));
        return;
    };
    let dir = mt::direction(&text);
    if !packs::is_installed(dir) {
        app().log(&format!("перевод: нет пакета {dir}"));
        hud::toast(t("translate.noPack"));
        crate::ui::spawn("settings", Some("translate"));
        return;
    }
    let _ = sender().lock().unwrap().send(text);
}

fn run(rx: Receiver<String>) {
    let mut loaded: HashMap<&'static str, Translator> = HashMap::new();
    loop {
        let text = match rx.recv_timeout(IDLE_UNLOAD) {
            Ok(t) => t,
            Err(RecvTimeoutError::Timeout) => {
                if !loaded.is_empty() {
                    loaded.clear();
                    app().log("перевод: модель выгружена после простоя");
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let (top, sound, volume) = {
            let e = app().engine.lock().unwrap();
            (
                e.settings.voice_hud_top,
                e.settings.translate_sound_enabled,
                e.settings.sound_volume,
            )
        };
        hud::set(hud::HudState::Processing("translate.processing"), top);
        let dir = mt::direction(&text);
        let t0 = Instant::now();
        let result = (|| {
            if !loaded.contains_key(dir) {
                let tr = Translator::load(&packs::dir(dir))?;
                app().log(&format!(
                    "перевод: модель {dir} загружена за {} мс",
                    t0.elapsed().as_millis()
                ));
                loaded.insert(dir, tr);
            }
            loaded.get_mut(dir).unwrap().translate(&text)
        })();
        hud::set(hud::HudState::Hidden, top);
        match result {
            Ok(out) if !out.trim().is_empty() => {
                // Только длины, не текст.
                app().log(&format!(
                    "перевод {dir}: {} → {} симв. за {} мс",
                    text.chars().count(),
                    out.chars().count(),
                    t0.elapsed().as_millis()
                ));
                if sound {
                    super::sys::play_cue(crate::synth::Cue::Translate, volume);
                }
                super::hook::post_insert(out, false);
            }
            Ok(_) => hud::toast(t("translate.failed")),
            Err(e) => {
                app().log(&format!("перевод: ошибка {e}"));
                loaded.remove(dir);
                hud::toast(t("translate.failed"));
            }
        }
    }
}
