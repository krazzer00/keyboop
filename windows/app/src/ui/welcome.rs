//! Окно-приветствие при первом запуске (перенос `WelcomeWindow.swift`): что умеет, какие
//! сочетания, модель для голоса, автозапуск.

use super::settings::{start_download, Download};
use super::widgets::{card, subtle, toggle_row};
use super::{l, Data, CORAL};
use crate::models;
use eframe::egui::{self, RichText, Ui};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Default)]
pub struct WelcomeView {
    page: usize,
    download: Option<Arc<Download>>,
    autostart: Option<bool>,
}

impl WelcomeView {
    /// Возвращает true, когда человек нажал «Готово».
    pub fn show(&mut self, ui: &mut Ui, d: &mut Data) -> bool {
        let mut finished = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Keyboop").size(34.0).strong().color(CORAL));
                subtle(ui, l("wrong layout? keyboop.", "wrong layout? keyboop."));
            });
            ui.add_space(16.0);
            match self.page {
                0 => {
                    card(ui, |ui| {
                        ui.label(RichText::new(l("Набрали «ghbdtn» вместо «привет»?", "Typed “ghbdtn” instead of “привет”?")).strong());
                        ui.label(l("Keyboop заметит и исправит сам — на пробеле или прямо посреди слова. Буфер обмена не трогается.", "Keyboop notices and fixes it — on Space or right mid-word. Your clipboard stays untouched."));
                    });
                    card(ui, |ui| {
                        ui.label(RichText::new(l("Сочетания", "Shortcuts")).strong());
                        egui::Grid::new("hk").num_columns(2).spacing([20.0, 6.0]).show(ui, |ui| {
                            ui.monospace(&d.settings.hotkey_convert);
                            ui.label(l("исправить последнее слово или выделенное", "fix the last word or the selection"));
                            ui.end_row();
                            ui.monospace(&d.settings.hotkey_voice);
                            ui.label(l("зажать и диктовать", "hold and dictate"));
                            ui.end_row();
                            ui.monospace(&d.settings.hotkey_translate);
                            ui.label(l("перевести выделенное", "translate the selection"));
                            ui.end_row();
                            ui.monospace("Esc");
                            ui.label(l("отменить диктовку", "cancel dictation"));
                            ui.end_row();
                        });
                    });
                }
                1 => {
                    card(ui, |ui| {
                        ui.label(RichText::new(l("Голосовой набор", "Voice typing")).strong());
                        ui.label(l("Речь распознаётся прямо на этом компьютере: звук никуда не уходит. Нужна модель — скачайте её один раз.", "Speech is recognized right on this computer: audio goes nowhere. It needs a model — download it once."));
                        let name = "small";
                        if models::is_installed(name) || models::any_installed().is_some() {
                            ui.label(RichText::new(format!("✔ {}", l("Модель уже на месте", "The model is ready"))).color(CORAL));
                        } else {
                            match &self.download {
                                Some(dl) if !dl.done.load(Ordering::Relaxed) => {
                                    let p = f32::from_bits(dl.progress.load(Ordering::Relaxed));
                                    ui.add(egui::ProgressBar::new(p).show_percentage());
                                }
                                other => {
                                    if let Some(err) = other.as_ref().and_then(|x| x.error.lock().unwrap().clone()) {
                                        ui.colored_label(egui::Color32::from_rgb(220, 90, 40), err);
                                    }
                                    if ui.button(format!("{} (small, 466 MB)", l("Скачать модель", "Download the model"))).clicked() {
                                        d.settings.voice_model = name.into();
                                        self.download = Some(start_download(name.into(), move |c, p| models::download_whisper(name, c, p)));
                                    }
                                    subtle(ui, l("Можно и позже: в настройках, раздел «Голосовой набор».", "You can do it later too: Settings → Voice typing."));
                                }
                            }
                        }
                    });
                }
                _ => {
                    card(ui, |ui| {
                        let mut auto = *self.autostart.get_or_insert_with(crate::win::sys::autostart_enabled);
                        if toggle_row(ui, &mut auto, l("Запускать вместе с Windows", "Start with Windows"), l("Keyboop живёт в трее у часов", "Keyboop lives in the tray by the clock")) {
                            crate::win::sys::set_autostart(auto);
                            self.autostart = Some(auto);
                        }
                    });
                    card(ui, |ui| {
                        ui.label(l("Всё настраивается по щелчку на значке в трее. Приятного набора!", "Everything is set up from the tray icon. Happy typing!"));
                    });
                }
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if self.page > 0 && ui.button(l("‹ Назад", "‹ Back")).clicked() {
                    self.page -= 1;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let last = self.page >= 2;
                    let label = if last { l("Готово", "Done") } else { l("Дальше ›", "Next ›") };
                    if ui.add(egui::Button::new(RichText::new(label).strong()).fill(CORAL)).clicked() {
                        if last {
                            d.settings.did_show_welcome = true;
                            finished = true;
                        } else {
                            self.page += 1;
                        }
                    }
                });
            });
        });
        finished
    }
}
