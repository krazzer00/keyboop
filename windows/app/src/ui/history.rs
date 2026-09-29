//! Окно истории (перенос `VoiceHistoryWindow.swift`): диктовки, скопированный текст, импорт и
//! звонки; поиск, фильтр, копирование, прослушивание записи, удаление.

use super::widgets::{card, subtle};
use super::{ipc, l, Data, CORAL};
use crate::win::history_store;
use eframe::egui::{self, RichText, Ui};
use keyboop_core::audio_import;
use keyboop_core::history::{self, HistoryEntry, HistoryKind};
use std::time::{Duration, Instant};

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Удалить всю историю и записи голоса.
pub fn clear_all() {
    for e in history_store::load() {
        if let Some(a) = e.audio {
            history_store::delete_clip(&a);
        }
    }
    history_store::save(&[]);
    ipc::send("history");
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Dictation,
    Clipboard,
    Files,
}

pub struct HistoryView {
    entries: Vec<HistoryEntry>,
    loaded_at: Option<Instant>,
    query: String,
    filter: Filter,
    copied: Option<(f64, Instant)>,
    /// Состояние расшифровки файла или звонка (из `import.json` главного процесса).
    import: Option<crate::win::voice::import::Status>,
    /// Итог, который уже показали.
    seen_finished: Option<u64>,
    toast: Option<(String, Instant)>,
}

impl Default for HistoryView {
    fn default() -> Self {
        HistoryView {
            entries: Vec::new(),
            loaded_at: None,
            query: String::new(),
            filter: Filter::All,
            import: None,
            seen_finished: None,
            toast: None,
            copied: None,
        }
    }
}

fn local_offset_secs() -> i64 {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    unsafe {
        let mut tz: TIME_ZONE_INFORMATION = std::mem::zeroed();
        let r = GetTimeZoneInformation(&mut tz);
        let bias = tz.Bias + if r == 2 { tz.DaylightBias } else { 0 };
        -(bias as i64) * 60
    }
}

fn day_and_time(ts: f64) -> (i64, String) {
    let local = ts as i64 + local_offset_secs();
    let day = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    (day, format!("{:02}:{:02}", secs / 3600, (secs / 60) % 60))
}

fn day_title(day: i64) -> String {
    let today = (now() as i64 + local_offset_secs()).div_euclid(86_400);
    match today - day {
        0 => l("Сегодня", "Today").into(),
        1 => l("Вчера", "Yesterday").into(),
        _ => {
            // Дни от эпохи → дата (алгоритм Хиннанта).
            let z = day + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let y = yoe + era * 400;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let dd = doy - (153 * mp + 2) / 5 + 1;
            let mm = if mp < 10 { mp + 3 } else { mp - 9 };
            format!("{dd:02}.{mm:02}.{}", if mm <= 2 { y + 1 } else { y })
        }
    }
}

fn kind_icon(k: HistoryKind) -> &'static str {
    match k {
        HistoryKind::Dictation => "🎤",
        HistoryKind::Clipboard => "📋",
        HistoryKind::Imported => "📄",
        HistoryKind::Call => "📞",
    }
}

/// Проиграть запись (WAV в памяти). Буфер держим, пока не начнётся следующая.
fn play(bytes: Vec<u8>) {
    use std::sync::Mutex;
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    static CURRENT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    let mut cur = CURRENT.lock().unwrap();
    unsafe {
        PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0);
    }
    *cur = bytes;
    unsafe {
        PlaySoundW(
            cur.as_ptr() as *const u16,
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

impl HistoryView {
    /// Прочитать состояние расшифровки; закончилась новая — показать итог и перечитать ленту.
    fn poll_import(&mut self) {
        let path = crate::win::voice::import::status_path();
        let st: Option<crate::win::voice::import::Status> = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        if let Some(st) = &st {
            match self.seen_finished {
                None => self.seen_finished = Some(st.finished),
                Some(seen) if seen != st.finished => {
                    self.seen_finished = Some(st.finished);
                    if let Some(m) = &st.message {
                        self.toast = Some((m.clone(), Instant::now()));
                    }
                    self.loaded_at = None; // перечитать историю
                }
                _ => {}
            }
        }
        self.import = st;
    }

    fn reload(&mut self, minutes: u32) {
        let mut e = history_store::load();
        let _ = history::prune(&mut e, minutes, now());
        self.entries = e;
        self.loaded_at = Some(Instant::now());
    }

    pub fn show(&mut self, ui: &mut Ui, d: &mut Data) {
        if self
            .loaded_at
            .is_none_or(|t| t.elapsed() > Duration::from_secs(3))
        {
            self.reload(d.settings.voice_history_minutes);
        }
        // Новые диктовки и копирования появляются без действий человека: перечитываем и так.
        self.poll_import();
        let busy = self.import.as_ref().is_some_and(|s| s.running);
        ui.ctx()
            .request_repaint_after(Duration::from_millis(if busy { 500 } else { 3000 }));
        egui::Panel::top("hist-top").show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text(format!("🔍 {}", l("Поиск", "Search")))
                        .desired_width(170.0),
                );
                for (f, label) in [
                    (Filter::All, l("Всё", "All")),
                    (Filter::Dictation, l("Диктовки", "Dictations")),
                    (Filter::Clipboard, l("Буфер", "Clipboard")),
                    (Filter::Files, l("Файлы и звонки", "Files and calls")),
                ] {
                    ui.selectable_value(&mut self.filter, f, label);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(l("Импорт файла…", "Import a file…")),
                        )
                        .on_hover_text(l(
                            "Расшифровать запись (mp3, m4a, wav, flac, ogg) в историю",
                            "Transcribe a recording (mp3, m4a, wav, flac, ogg) into History",
                        ))
                        .clicked()
                    {
                        // Диалог модальный: в своём потоке, чтобы окно не замирало.
                        std::thread::spawn(|| {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter(
                                    l("Аудио", "Audio"),
                                    &["mp3", "m4a", "mp4", "aac", "wav", "flac", "ogg", "oga"],
                                )
                                .pick_file()
                            {
                                crate::ui::ipc::send(&format!("import:{}", p.display()));
                            }
                        });
                    }
                });
            });
            if let Some(st) = self.import.as_ref().filter(|s| s.running) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let what = if st.kind == Some(HistoryKind::Call) {
                        l("Расшифровываю звонок", "Transcribing the call")
                    } else {
                        l("Расшифровываю файл", "Transcribing the file")
                    };
                    let mut line = format!("{what}: {}", audio_import::clock(st.processed));
                    if st.total > 0.0 {
                        line += &format!(" / {}", audio_import::clock(st.total));
                    }
                    if let Some(rem) = audio_import::remaining(st.processed, st.total, st.elapsed) {
                        line += &format!(
                            " · {} {}",
                            l("осталось ~", "about"),
                            audio_import::clock(rem)
                        );
                        if !crate::l10n::is_russian() {
                            line += " left";
                        }
                    }
                    ui.label(line);
                    if st.total > 0.0 {
                        ui.add(
                            egui::ProgressBar::new((st.processed / st.total) as f32)
                                .desired_width(140.0),
                        );
                    }
                    if ui.small_button(l("Отменить", "Cancel")).clicked() {
                        crate::ui::ipc::send("import-cancel");
                    }
                });
            }
            if let Some((msg, at)) = &self.toast {
                if at.elapsed() < Duration::from_secs(6) {
                    subtle(ui, msg);
                }
            }
            ui.add_space(6.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if !d.settings.voice_history_enabled {
                card(ui, |ui| {
                    ui.label(l(
                        "История выключена. Включить её можно в настройках, раздел «История».",
                        "History is off. Turn it on in Settings → History.",
                    ));
                });
                return;
            }
            let mut delete: Option<(f64, String)> = None;
            let shown: Vec<&HistoryEntry> = self
                .entries
                .iter()
                .rev()
                .filter(|e| history::matches(e, &self.query))
                .filter(|e| match self.filter {
                    Filter::All => true,
                    Filter::Dictation => e.resolved_kind() == HistoryKind::Dictation,
                    Filter::Clipboard => e.is_clipboard(),
                    Filter::Files => e.is_imported(),
                })
                .collect();
            if shown.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(l("Здесь пока пусто", "Nothing here yet")).heading());
                    subtle(
                        ui,
                        l(
                            "Надиктуйте что-нибудь — текст появится здесь.",
                            "Dictate something — the text will appear here.",
                        ),
                    );
                });
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut last_day = None;
                    for e in shown {
                        let (day, time) = day_and_time(e.date);
                        if last_day != Some(day) {
                            last_day = Some(day);
                            ui.add_space(8.0);
                            ui.label(RichText::new(day_title(day)).strong().color(CORAL));
                        }
                        card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(kind_icon(e.resolved_kind()));
                                ui.label(RichText::new(&time).small());
                                if let Some(app) = &e.app {
                                    ui.label(RichText::new(app).small().weak());
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button("🗑")
                                            .on_hover_text(l("Удалить", "Delete"))
                                            .clicked()
                                        {
                                            delete = Some((e.date, e.text.clone()));
                                        }
                                        let copied = self.copied.is_some_and(|(d, t)| {
                                            d == e.date && t.elapsed() < Duration::from_secs(2)
                                        });
                                        if ui
                                            .small_button(if copied {
                                                l("Скопировано", "Copied")
                                            } else {
                                                l("Копировать", "Copy")
                                            })
                                            .clicked()
                                        {
                                            ui.ctx().copy_text(e.text.clone());
                                            self.copied = Some((e.date, Instant::now()));
                                        }
                                        if let Some(id) = &e.audio {
                                            if ui
                                                .small_button("▶")
                                                .on_hover_text(l("Прослушать", "Play"))
                                                .clicked()
                                            {
                                                if let Some(wav) = history_store::load_clip(id) {
                                                    play(wav);
                                                }
                                            }
                                        }
                                    },
                                );
                            });
                            ui.add(egui::Label::new(&e.text).wrap().selectable(true));
                        });
                    }
                });
            if let Some((date, text)) = delete {
                let mut all = history_store::load();
                for e in all.iter().filter(|e| e.date == date && e.text == text) {
                    if let Some(a) = &e.audio {
                        history_store::delete_clip(a);
                    }
                }
                all.retain(|e| !(e.date == date && e.text == text));
                history_store::save(&all);
                ipc::send("history");
                self.loaded_at = None;
            }
        });
    }
}
