//! «Написать разработчику» (перенос `FeedbackWindow.swift`): текст, контакт, диагностика без
//! содержимого набранного. Отправка на тот же адрес, что у мак-версии.

use super::widgets::{card, subtle};
use super::{l, Data, CORAL};
use eframe::egui::{self, RichText, Ui};
use std::sync::{Arc, Mutex};

const ENDPOINT: &str = "https://keyboop.com/api/feedback";

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    Sending,
    Sent,
    Failed(String),
}

pub struct FeedbackView {
    text: String,
    contact: String,
    diag: bool,
    status: Arc<Mutex<Status>>,
}

impl Default for FeedbackView {
    fn default() -> Self {
        FeedbackView {
            text: String::new(),
            contact: String::new(),
            diag: true,
            status: Arc::new(Mutex::new(Status::Idle)),
        }
    }
}

/// Диагностика: версии, настройки хоткеев, раскладки, хвост лога. Без текста, который набирали.
fn diagnostics(d: &Data) -> String {
    let s = &d.settings;
    let log = std::fs::read_to_string(d.store.path(crate::storage::LOG)).unwrap_or_default();
    let tail: Vec<&str> = log.lines().rev().take(60).collect();
    format!(
        "Keyboop {} · Windows · интерфейс {}\n{}\nхоткеи: convert={} voice={}({}) translate={} switch={} case={}\nавто={} на лету={} dev={} голос={} модель={} язык={}\n--- лог ---\n{}",
        env!("CARGO_PKG_VERSION"),
        if crate::l10n::is_russian() { "ru" } else { "en" },
        crate::win::sys::os_version(),
        s.hotkey_convert,
        s.hotkey_voice,
        s.voice_hold_mode,
        s.hotkey_translate,
        s.hotkey_switch_layout,
        s.hotkey_case,
        s.auto_enabled,
        s.live_fix_enabled,
        s.developer_mode,
        s.voice_enabled,
        s.voice_model,
        s.voice_language,
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    )
}

impl FeedbackView {
    pub fn show(&mut self, ui: &mut Ui, d: &mut Data) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(12.0);
            ui.heading(l("Написать разработчику", "Send feedback"));
            subtle(
                ui,
                l(
                    "Идеи, баги, «не переключилось». Читаем всё.",
                    "Ideas, bugs, “didn't switch”. We read everything.",
                ),
            );
            let status = self.status.lock().unwrap().clone();
            if status == Status::Sent {
                card(ui, |ui| {
                    ui.label(
                        RichText::new(l("Улетело! Спасибо.", "Sent! Thank you."))
                            .strong()
                            .color(CORAL),
                    );
                    if !self.contact.is_empty() {
                        ui.label(l(
                            "Ответим на указанный контакт.",
                            "We'll reply to the contact you left.",
                        ));
                    }
                });
                return;
            }
            ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .desired_rows(10)
                    .desired_width(f32::INFINITY)
                    .hint_text(l(
                        "Что случилось или что хочется…",
                        "What happened or what you'd like…",
                    )),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.contact)
                    .desired_width(f32::INFINITY)
                    .hint_text(l(
                        "Почта или Telegram для ответа (не обязательно)",
                        "Email or Telegram for a reply (optional)",
                    )),
            );
            ui.checkbox(
                &mut self.diag,
                l(
                    "Приложить диагностику (без того, что вы набирали)",
                    "Attach diagnostics (without anything you typed)",
                ),
            );
            ui.horizontal(|ui| {
                let can = self.text.trim().chars().count() >= 3 && status != Status::Sending;
                if ui
                    .add_enabled(
                        can,
                        egui::Button::new(RichText::new(l("Отправить", "Send")).strong())
                            .fill(CORAL),
                    )
                    .clicked()
                {
                    let mut body = serde_json::json!({
                        "text": self.text.trim(),
                        "version": format!("win-{}", env!("CARGO_PKG_VERSION")),
                        "kind": "feedback",
                        "platform": "windows",
                    });
                    if !self.contact.trim().is_empty() {
                        body["contact"] = self.contact.trim().into();
                    }
                    if self.diag {
                        body["diag"] = diagnostics(d).into();
                    }
                    *self.status.lock().unwrap() = Status::Sending;
                    let st = self.status.clone();
                    std::thread::spawn(move || {
                        let r = ureq::post(ENDPOINT)
                            .header("Content-Type", "application/json")
                            .send(body.to_string());
                        *st.lock().unwrap() = match r {
                            Ok(resp) if resp.status() == 200 => Status::Sent,
                            Ok(resp) => Status::Failed(format!("HTTP {}", resp.status())),
                            Err(e) => Status::Failed(e.to_string()),
                        };
                    });
                }
                match &status {
                    Status::Sending => {
                        ui.spinner();
                        ui.label(l("Отправляю…", "Sending…"));
                    }
                    Status::Failed(e) => {
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 90, 40),
                            format!("{} ({e})", l("Не отправилось", "Couldn't send")),
                        );
                        ui.hyperlink_to(
                            l("Написать в Telegram", "Write on Telegram"),
                            "https://t.me/keyboop_bot?start=report",
                        );
                    }
                    _ => {}
                }
            });
        });
    }
}
