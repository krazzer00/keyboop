//! Окно настроек (перенос `SettingsWindow.swift`): «Основное» с тремя главными сочетаниями и
//! подробные разделы слева.

use super::widgets::{
    card, chips, hotkey_field, pair_list, row, section_title, subtle, toggle_row, HotkeyRecorder,
};
use super::{ipc, l, Data, CORAL};
use crate::models;
use eframe::egui::{self, RichText, Ui};
use keyboop_core::ambiguous::{self, Choice};
use keyboop_core::history::RETENTION_CHOICES;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Essentials,
    Layout,
    Exceptions,
    Snippets,
    Voice,
    Translate,
    History,
    General,
    About,
}

impl Section {
    const ALL: [Section; 9] = [
        Section::Essentials,
        Section::Layout,
        Section::Exceptions,
        Section::Snippets,
        Section::Voice,
        Section::Translate,
        Section::History,
        Section::General,
        Section::About,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Essentials => l("Основное", "Essentials"),
            Section::Layout => l("Раскладка", "Layout"),
            Section::Exceptions => l("Исключения", "Exceptions"),
            Section::Snippets => l("Сниппеты", "Snippets"),
            Section::Voice => l("Голосовой набор", "Voice typing"),
            Section::Translate => l("Перевод", "Translation"),
            Section::History => l("История", "History"),
            Section::General => l("Общее", "General"),
            Section::About => l("О программе", "About"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Section::Essentials => "★",
            Section::Layout => "⌨",
            Section::Exceptions => "⛔",
            Section::Snippets => "✂",
            Section::Voice => "🎤",
            Section::Translate => "🌐",
            Section::History => "🕘",
            Section::General => "⚙",
            Section::About => "ℹ",
        }
    }

    fn parse(s: &str) -> Section {
        match s {
            "layout" => Section::Layout,
            "exceptions" => Section::Exceptions,
            "snippets" => Section::Snippets,
            "voice" => Section::Voice,
            "translate" => Section::Translate,
            "history" => Section::History,
            "general" => Section::General,
            "about" => Section::About,
            _ => Section::Essentials,
        }
    }
}

/// Скачивание модели в фоне.
pub struct Download {
    pub progress: AtomicU32,
    pub done: AtomicBool,
    pub cancel: AtomicBool,
    pub error: Mutex<Option<String>>,
}

pub fn start_download(
    name: String,
    job: impl FnOnce(&AtomicBool, &mut dyn FnMut(f64)) -> Result<(), models::DownloadError>
        + Send
        + 'static,
) -> Arc<Download> {
    let d = Arc::new(Download {
        progress: AtomicU32::new(0),
        done: AtomicBool::new(false),
        cancel: AtomicBool::new(false),
        error: Mutex::new(None),
    });
    let d2 = d.clone();
    std::thread::spawn(move || {
        let r = job(&d2.cancel, &mut |p| {
            d2.progress.store((p as f32).to_bits(), Ordering::Relaxed)
        });
        if let Err(e) = r {
            *d2.error.lock().unwrap() = Some(match e {
                models::DownloadError::Cancelled => l("отменено", "cancelled").into(),
                models::DownloadError::Failed(m) => m,
            });
        }
        d2.done.store(true, Ordering::Relaxed);
        let _ = name;
        ipc::send("preload");
    });
    d
}

pub struct SettingsView {
    section: Section,
    rec: HotkeyRecorder,
    new_ignored: String,
    new_forced: String,
    new_app: String,
    downloads: HashMap<String, Arc<Download>>,
    mics: Option<Vec<String>>,
    autostart: Option<bool>,
    confirm_clear_history: bool,
}

impl SettingsView {
    pub fn new(section: Option<&str>) -> Self {
        SettingsView {
            section: section.map(Section::parse).unwrap_or(Section::Essentials),
            rec: HotkeyRecorder::default(),
            new_ignored: String::new(),
            new_forced: String::new(),
            new_app: String::new(),
            downloads: HashMap::new(),
            mics: None,
            autostart: None,
            confirm_clear_history: false,
        }
    }

    pub fn show(&mut self, ui: &mut Ui, d: &mut Data) {
        egui::Panel::left("sections")
            .exact_size(200.0)
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Keyboop").heading().strong().color(CORAL));
                });
                ui.add_space(12.0);
                for s in Section::ALL {
                    let selected = self.section == s;
                    let text = RichText::new(format!("{}  {}", s.icon(), s.title()));
                    let button =
                        egui::Button::selectable(selected, text).min_size(egui::vec2(188.0, 30.0));
                    if ui.add(button).clicked() {
                        self.section = s;
                    }
                }
            });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(12.0);
                ui.heading(self.section.title());
                ui.add_space(6.0);
                if !d.errors.is_empty() {
                    card(ui, |ui| {
                        ui.colored_label(egui::Color32::from_rgb(220, 90, 40), l("Файл настроек не прочитан — показаны значения по умолчанию. Исправьте JSON или удалите файл.", "A settings file could not be read — showing defaults. Fix the JSON or delete the file."));
                        for e in &d.errors {
                            subtle(ui, e);
                        }
                    });
                }
                match self.section {
                    Section::Essentials => self.essentials(ui, d),
                    Section::Layout => self.layout(ui, d),
                    Section::Exceptions => self.exceptions(ui, d),
                    Section::Snippets => self.snippets(ui, d),
                    Section::Voice => self.voice(ui, d),
                    Section::Translate => self.translate(ui, d),
                    Section::History => self.history(ui, d),
                    Section::General => self.general(ui, d),
                    Section::About => self.about(ui, d),
                }
                ui.add_space(24.0);
            });
        });
    }

    fn essentials(&mut self, ui: &mut Ui, d: &mut Data) {
        let s = &mut d.settings;
        card(ui, |ui| {
            row(
                ui,
                l("Исправить раскладку", "Fix the layout"),
                l(
                    "Последнее слово или выделенный текст. Тем же сочетанием — обратно",
                    "The last word or the selection. Press again to undo",
                ),
                |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "convert",
                        &mut s.hotkey_convert,
                        &["Pause", "Shift+Pause", "RCtrl", "Ctrl+Shift+Space", "F12"],
                    );
                },
            );
            ui.separator();
            let sub = if s.voice_hold_mode == "toggle" {
                l(
                    "Нажмите и говорите, нажмите ещё раз, чтобы закончить",
                    "Press and speak, press again to finish",
                )
            } else {
                l(
                    "Зажмите и говорите, отпустите, чтобы закончить",
                    "Hold and speak, release to finish",
                )
            };
            row(
                ui,
                l("Начать диктовку", "Start dictation"),
                sub,
                |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "voice",
                        &mut s.hotkey_voice,
                        &["RAlt", "RCtrl", "F9", "Ctrl+Space"],
                    );
                },
            );
            ui.separator();
            row(
                ui,
                l("Перевести выделенное", "Translate the selection"),
                l(
                    "Перевод встанет на место оригинала",
                    "The translation replaces the original",
                ),
                |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "translate",
                        &mut s.hotkey_translate,
                        &["Ctrl+Alt+T", "Ctrl+Shift+T"],
                    );
                },
            );
        });
        section_title(ui, l("Главное", "Main"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.auto_enabled,
                l(
                    "Переключать раскладку сама",
                    "Switch the layout automatically",
                ),
                l(
                    "Чинит слово, набранное не в той раскладке, на пробеле и Enter",
                    "Fixes a word typed in the wrong layout on Space and Enter",
                ),
            );
            ui.separator();
            toggle_row(
                ui,
                &mut s.voice_enabled,
                l("Голосовой набор", "Voice typing"),
                l(
                    "Распознавание на этом компьютере, без интернета",
                    "Recognized on this computer, no internet",
                ),
            );
            ui.separator();
            toggle_row(
                ui,
                &mut s.sound_enabled,
                l("Звук переключения", "Switch sound"),
                "",
            );
        });
        let wall = crate::ui::history::now();
        if s.is_paused(wall) {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "⏸ {}",
                        l(
                            "На паузе — авто-переключение молчит",
                            "Paused — auto-switching is off"
                        )
                    ));
                    if ui.button(l("Снять паузу", "Resume")).clicked() {
                        s.paused_until = 0.0;
                    }
                });
            });
        }
        ui.add_space(8.0);
        subtle(
            ui,
            &format!(
                "{} {} · {} {}",
                l("Расколдовано слов:", "Words rescued:"),
                d.state.rescued_count,
                l("надиктовано слов:", "words dictated:"),
                d.state.voice_words
            ),
        );
    }

    fn layout(&mut self, ui: &mut Ui, d: &mut Data) {
        let s = &mut d.settings;
        section_title(ui, l("Когда переключать", "When to switch"));
        card(ui, |ui| {
            toggle_row(ui, &mut s.trigger_space, l("На пробеле", "On Space"), "");
            toggle_row(ui, &mut s.trigger_enter, l("На Enter", "On Enter"), "");
            toggle_row(ui, &mut s.trigger_tab, l("На Tab", "On Tab"), "");
            ui.separator();
            toggle_row(
                ui,
                &mut s.live_fix_enabled,
                l("Чинить на лету", "Fix while typing"),
                l(
                    "Посреди слова, как только сочетание букв невозможно в текущем языке",
                    "Mid-word, as soon as the letters are impossible in the current language",
                ),
            );
            toggle_row(
                ui,
                &mut s.enter_pre_convert,
                l("Чинить до отправки по Enter", "Fix before Enter sends"),
                l(
                    "В чатах Enter отправляет сразу — слово успеет исправиться до отправки",
                    "Chats send on Enter instantly — the word is fixed first",
                ),
            );
            toggle_row(
                ui,
                &mut s.arrows_cancel,
                l("Стрелки сбрасывают слово", "Arrows reset the word"),
                "",
            );
        });
        section_title(ui, l("Осторожность", "Caution"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.developer_mode,
                l("Режим разработчика", "Developer mode"),
                l(
                    "В редакторах кода и терминалах авто-переключение молчит целиком",
                    "Auto-switching stays off in code editors and terminals",
                ),
            );
            toggle_row(
                ui,
                &mut s.learn_on_undo,
                l("Учиться на отменах", "Learn from undo"),
                l(
                    "Трижды вернули слово — предложу больше его не трогать",
                    "Revert a word three times and Keyboop offers to leave it alone",
                ),
            );
            ui.add_enabled_ui(!s.auto_enabled, |ui| {
                toggle_row(
                    ui,
                    &mut s.group_convert,
                    l(
                        "Хоткей переводит всю фразу",
                        "Hotkey converts the whole phrase",
                    ),
                    l(
                        "Только когда авто-переключение выключено",
                        "Only when auto-switching is off",
                    ),
                );
            });
        });
        section_title(ui, l("Правка текста", "Text fixes"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.typo_fix,
                l("Исправлять опечатки", "Fix typos"),
                l(
                    "«тедефон» → «телефон», след Caps Lock «пРИВЕТ» → «Привет»",
                    "“acheive” → “achieve”, Caps Lock trace “hELLO” → “Hello”",
                ),
            );
            toggle_row(
                ui,
                &mut s.two_caps_fix,
                l("Две заглавные в начале", "Two leading capitals"),
                l("«КОгда» → «Когда»", "“THis” → “This”"),
            );
        });
        section_title(ui, l("Сочетания", "Shortcuts"));
        card(ui, |ui| {
            row(
                ui,
                l("Только сменить раскладку", "Just switch the layout"),
                l(
                    "Как 🌐 на Маке: набранное не трогается",
                    "Like 🌐 on a Mac: typed text stays",
                ),
                |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "switch",
                        &mut s.hotkey_switch_layout,
                        &["CapsLock", "RCtrl", "RShift", "LAlt"],
                    );
                },
            );
            row(
                ui,
                l("Регистр выделенного", "Selection case"),
                l("ЗАГЛАВНЫЕ ↔ строчные", "UPPER ↔ lower"),
                |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "case",
                        &mut s.hotkey_case,
                        &["Alt+Pause", "Ctrl+Shift+U"],
                    );
                },
            );
        });
        section_title(ui, l("Звук и индикатор", "Sound and indicator"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.sound_enabled,
                l("Звук переключения", "Switch sound"),
                "",
            );
            row(ui, l("Громкость", "Volume"), "", |ui| {
                ui.add(egui::Slider::new(&mut s.sound_volume, 0.0..=1.0).show_value(false));
            });
            toggle_row(
                ui,
                &mut s.caps_led_indicator,
                l(
                    "Лампочка Caps Lock показывает язык",
                    "Caps Lock light shows the language",
                ),
                l(
                    "Горит — русский, погасла — английский",
                    "On — Russian, off — English",
                ),
            );
        });
    }

    fn exceptions(&mut self, ui: &mut Ui, d: &mut Data) {
        let e = &mut d.exceptions;
        section_title(
            ui,
            l("Не переключать эти слова", "Never switch these words"),
        );
        card(ui, |ui| {
            chips(
                ui,
                &mut e.ignored,
                &mut self.new_ignored,
                l(
                    "слово или несколько через пробел",
                    "a word or several, space-separated",
                ),
            );
        });
        section_title(ui, l("Всегда переключать", "Always switch"));
        card(ui, |ui| {
            chips(
                ui,
                &mut e.force_swap,
                &mut self.new_forced,
                l("слово", "word"),
            );
        });
        section_title(ui, l("Выученные на отмене", "Learned from undo"));
        card(ui, |ui| {
            if e.learned.is_empty() {
                subtle(ui, l("Пока пусто. Слова попадают сюда, когда вы трижды возвращаете их после переключения.", "Empty for now. Words land here after you revert them three times."));
            } else {
                let mut dummy = String::new();
                chips(ui, &mut e.learned, &mut dummy, "");
            }
        });
        section_title(ui, l("Спорные пары", "Ambiguous pairs"));
        card(ui, |ui| {
            subtle(ui, l("Одни и те же клавиши дают слово в обоих языках. Выберите, что для вас важнее, или оставьте «по контексту».", "The same keys give a word in both languages. Pick which matters to you, or leave “by context”."));
            // Каждый вариант в своей колонке, чтобы кнопки стояли ровно друг под другом.
            egui::Grid::new("amb")
                .num_columns(4)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    for (en, ru) in ambiguous::PAIRS {
                        let cur = ambiguous::choice(e, en, ru);
                        ui.label(format!("{en} ↔ {ru}"));
                        for (c, label) in [
                            (Choice::En, *en),
                            (Choice::Auto, l("по контексту", "by context")),
                            (Choice::Ru, *ru),
                        ] {
                            if ui.selectable_label(cur == c, label).clicked() && cur != c {
                                ambiguous::choose(e, en, ru, c);
                            }
                        }
                        ui.end_row();
                    }
                });
        });
        section_title(ui, l("Программы", "Apps"));
        card(ui, |ui| {
            subtle(ui, l("Терминалы и программы монтажа по умолчанию не трогаются, редакторы кода — мягко. Здесь можно поменять это для любой программы (имя exe).", "Terminals and editing suites are left alone by default, code editors are handled softly. Change it for any app (exe name) here."));
            let mut remove = None;
            egui::Grid::new("apps")
                .num_columns(4)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    let names: Vec<String> = e
                        .app_modes
                        .keys()
                        .chain(e.app_layouts.keys())
                        .cloned()
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    for app in names {
                        ui.label(&app);
                        let mut mode = e
                            .app_modes
                            .get(&app)
                            .cloned()
                            .unwrap_or_else(|| "normal".into());
                        egui::ComboBox::from_id_salt(format!("mode-{app}"))
                            .selected_text(mode_label(&mode))
                            .show_ui(ui, |ui| {
                                for m in ["normal", "soft", "off"] {
                                    ui.selectable_value(&mut mode, m.to_string(), mode_label(m));
                                }
                            });
                        e.app_modes.insert(app.clone(), mode);
                        let mut lay = e.app_layouts.get(&app).cloned().unwrap_or_default();
                        egui::ComboBox::from_id_salt(format!("lay-{app}"))
                            .selected_text(layout_label(&lay))
                            .show_ui(ui, |ui| {
                                for v in ["", "en", "ru"] {
                                    ui.selectable_value(&mut lay, v.to_string(), layout_label(v));
                                }
                            });
                        if lay.is_empty() {
                            e.app_layouts.remove(&app);
                        } else {
                            e.app_layouts.insert(app.clone(), lay);
                        }
                        if ui.small_button("🗑").clicked() {
                            remove = Some(app.clone());
                        }
                        ui.end_row();
                    }
                });
            if let Some(a) = remove {
                e.app_modes.remove(&a);
                e.app_layouts.remove(&a);
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_app)
                        .hint_text("resolve.exe")
                        .desired_width(220.0),
                );
                if ui.button(l("Добавить программу", "Add app")).clicked()
                    && !self.new_app.trim().is_empty()
                {
                    let mut name = self.new_app.trim().to_lowercase();
                    if !name.ends_with(".exe") {
                        name.push_str(".exe");
                    }
                    e.app_modes.insert(name, "off".into());
                    self.new_app.clear();
                }
            });
        });
    }

    fn snippets(&mut self, ui: &mut Ui, d: &mut Data) {
        section_title(ui, l("Автозамена", "Auto-replace"));
        card(ui, |ui| {
            subtle(
                ui,
                l(
                    "Набрали сокращение — получили текст. Раскладка и регистр набора не важны.",
                    "Type a shortcut, get the text. Layout and case don't matter.",
                ),
            );
            let s = &mut d.settings;
            ui.horizontal(|ui| {
                ui.label(l("Раскрывать на:", "Expand on:"));
                ui.checkbox(&mut s.snippet_expand_space, l("пробеле", "Space"));
                ui.checkbox(&mut s.snippet_expand_enter, "Enter");
                ui.checkbox(&mut s.snippet_expand_tab, "Tab");
            });
            pair_list(
                ui,
                "snip",
                &mut d.snippets,
                l("сокращение", "shortcut"),
                l("текст", "text"),
                true,
            );
        });
        section_title(ui, l("Тексты по цифре", "Texts by number"));
        card(ui, |ui| {
            subtle(
                ui,
                l(
                    "Сочетание открывает список у курсора, цифра вставляет текст.",
                    "A shortcut opens a list at the cursor; a digit inserts the text.",
                ),
            );
            row(
                ui,
                l("Открыть список", "Open the list"),
                "",
                |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "pick",
                        &mut d.settings.hotkey_snippet_pick,
                        &["Ctrl+Alt+S", "Ctrl+Shift+Space"],
                    );
                },
            );
            pair_list(
                ui,
                "texts",
                &mut d.text_snippets,
                l("название", "name"),
                l("текст", "text"),
                true,
            );
            if ui
                .button(l("Скопировать из автозамены", "Copy from auto-replace"))
                .clicked()
            {
                let have: std::collections::HashSet<String> =
                    d.text_snippets.iter().map(|p| p.0.clone()).collect();
                let add: Vec<_> = d
                    .snippets
                    .iter()
                    .filter(|p| !have.contains(&p.0))
                    .cloned()
                    .collect();
                d.text_snippets.extend(add);
            }
        });
        section_title(ui, l("Вставка без форматирования", "Paste as plain text"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut d.settings.plain_paste,
                l("Вставлять только текст", "Paste text only"),
                l(
                    "Сочетание вставляет содержимое буфера без шрифтов, цветов и ссылок",
                    "The shortcut pastes the clipboard without fonts, colors and links",
                ),
            );
            ui.add_enabled_ui(d.settings.plain_paste, |ui| {
                row(ui, l("Сочетание", "Shortcut"), "", |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "plain",
                        &mut d.settings.hotkey_plain_paste,
                        &["Ctrl+Shift+V", "Ctrl+Alt+V"],
                    );
                });
            });
        });
    }

    fn voice(&mut self, ui: &mut Ui, d: &mut Data) {
        let s = &mut d.settings;
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.voice_enabled,
                l("Голосовой набор", "Voice typing"),
                l(
                    "Речь распознаётся на этом компьютере: звук никуда не отправляется",
                    "Speech is recognized on this computer: audio never leaves it",
                ),
            );
            row(ui, l("Сочетание", "Shortcut"), "", |ui| {
                hotkey_field(
                    ui,
                    &mut self.rec,
                    "voice2",
                    &mut s.hotkey_voice,
                    &["RAlt", "RCtrl", "F9", "Ctrl+Space"],
                );
            });
            row(ui, l("Как держать", "How to hold"), "", |ui| {
                ui.selectable_value(
                    &mut s.voice_hold_mode,
                    "toggle".into(),
                    l("нажал / нажал", "press / press"),
                );
                ui.selectable_value(
                    &mut s.voice_hold_mode,
                    "hold".into(),
                    l("зажал и говорю", "hold to talk"),
                );
            });
            row(ui, l("Язык речи", "Speech language"), "", |ui| {
                for (v, label) in [
                    ("en", "English"),
                    ("ru", "Русский"),
                    ("auto", l("авто", "auto")),
                ] {
                    ui.selectable_value(&mut s.voice_language, v.to_string(), label);
                }
            });
        });
        section_title(ui, l("Модель распознавания", "Speech model"));
        card(ui, |ui| {
            subtle(
                ui,
                l(
                    "Модель скачивается один раз. Чем больше, тем точнее и медленнее.",
                    "The model is downloaded once. Bigger is more accurate and slower.",
                ),
            );
            for m in models::CATALOG {
                ui.horizontal(|ui| {
                    let installed = models::is_installed(m.name);
                    let selected = s.voice_model == m.name;
                    if ui
                        .add_enabled(installed, egui::RadioButton::new(selected, m.name))
                        .clicked()
                    {
                        s.voice_model = m.name.to_string();
                        ipc::send("preload");
                    }
                    ui.label(RichText::new(m.size).small());
                    if m.name == "small" {
                        ui.label(
                            RichText::new(l("рекомендуем", "recommended"))
                                .small()
                                .color(CORAL),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dl = self.downloads.get(m.name).cloned();
                        match dl {
                            Some(dl) if !dl.done.load(Ordering::Relaxed) => {
                                if ui.small_button("×").clicked() {
                                    dl.cancel.store(true, Ordering::Relaxed);
                                }
                                let p = f32::from_bits(dl.progress.load(Ordering::Relaxed));
                                ui.add(
                                    egui::ProgressBar::new(p)
                                        .desired_width(160.0)
                                        .show_percentage(),
                                );
                            }
                            other => {
                                if let Some(err) =
                                    other.as_ref().and_then(|d| d.error.lock().unwrap().clone())
                                {
                                    ui.colored_label(egui::Color32::from_rgb(220, 90, 40), err);
                                }
                                if installed {
                                    if ui.small_button(l("Удалить", "Delete")).clicked() {
                                        models::delete_whisper(m.name);
                                    }
                                    ui.label("✔");
                                } else if ui.button(l("Скачать", "Download")).clicked() {
                                    let name = m.name.to_string();
                                    let n2 = name.clone();
                                    let dl =
                                        start_download(name.clone(), move |cancel, progress| {
                                            models::download_whisper(&n2, cancel, progress)
                                        });
                                    self.downloads.insert(name.clone(), dl);
                                    if !models::is_installed(&s.voice_model) {
                                        s.voice_model = name;
                                    }
                                }
                            }
                        }
                    });
                });
            }
        });
        section_title(ui, l("Микрофон", "Microphone"));
        card(ui, |ui| {
            let mics = self
                .mics
                .get_or_insert_with(crate::win::voice::audio::input_devices);
            row(ui, l("Микрофон", "Microphone"), "", |ui| {
                let shown = if s.voice_mic.is_empty() {
                    l("системный по умолчанию", "system default").to_string()
                } else {
                    s.voice_mic.clone()
                };
                egui::ComboBox::from_id_salt("mic")
                    .selected_text(shown)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut s.voice_mic,
                            String::new(),
                            l("системный по умолчанию", "system default"),
                        );
                        for m in mics.iter() {
                            ui.selectable_value(&mut s.voice_mic, m.clone(), m);
                        }
                    });
            });
            toggle_row(
                ui,
                &mut s.voice_mic_gain,
                l("Поднимать уровень микрофона", "Raise microphone level"),
                l(
                    "На время диктовки, потом вернуть",
                    "During dictation, then restore",
                ),
            );
            if s.voice_mic_gain {
                ui.add(egui::Slider::new(&mut s.voice_mic_gain_level, 10..=100).suffix(" %"));
            }
            toggle_row(
                ui,
                &mut s.voice_duck,
                l("Приглушать звук системы", "Lower system audio"),
                l(
                    "Музыка и видео не мешают распознаванию",
                    "Music and video won't get in the way",
                ),
            );
            if s.voice_duck {
                ui.add(egui::Slider::new(&mut s.voice_duck_level, 0..=77).suffix(" %"));
            }
        });
        section_title(ui, l("Текст после диктовки", "Text after dictation"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.voice_trailing_space,
                l("Пробел в конце", "Trailing space"),
                "",
            );
            toggle_row(
                ui,
                &mut s.voice_no_final_period,
                l("Без точки в конце", "No final period"),
                "",
            );
            toggle_row(
                ui,
                &mut s.voice_no_capital,
                l("Не начинать с заглавной", "Don't capitalize sentences"),
                l(
                    "Имена и аббревиатуры остаются как есть",
                    "Names and acronyms stay as they are",
                ),
            );
            toggle_row(
                ui,
                &mut s.voice_no_em_dash,
                l("Дефис вместо длинного тире", "Hyphen instead of em dash"),
                "",
            );
            toggle_row(
                ui,
                &mut s.voice_auto_enter,
                l("Нажать Enter после вставки", "Press Enter after inserting"),
                l(
                    "Чтобы сообщение сразу ушло в чате",
                    "To send the message right away in chats",
                ),
            );
        });
        section_title(ui, l("Поведение", "Behavior"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.voice_sound_enabled,
                l("Звуки начала и конца записи", "Start and stop sounds"),
                "",
            );
            toggle_row(
                ui,
                &mut s.esc_cancels_dictation,
                l("Esc отменяет диктовку", "Esc cancels dictation"),
                "",
            );
            toggle_row(
                ui,
                &mut s.esc_save_to_history,
                l("Отменённое — в историю", "Save cancelled to history"),
                l(
                    "Распознать и сохранить, но не вставлять",
                    "Transcribe and save, but don't insert",
                ),
            );
            toggle_row(
                ui,
                &mut s.voice_hud_top,
                l("Плашка вверху экрана", "Indicator at the top of the screen"),
                l("Иначе — у курсора", "Otherwise near the cursor"),
            );
            toggle_row(
                ui,
                &mut s.voice_unload_after_dictation,
                l(
                    "Выгружать модель после диктовки",
                    "Unload the model after dictation",
                ),
                l(
                    "Меньше памяти, но первая диктовка дольше",
                    "Less memory, slower first dictation",
                ),
            );
            row(
                ui,
                l("Вставить последнюю диктовку", "Paste the last dictation"),
                "",
                |ui| {
                    hotkey_field(
                        ui,
                        &mut self.rec,
                        "pastelast",
                        &mut s.hotkey_paste_dictation,
                        &["Ctrl+Alt+V", "Ctrl+Shift+D"],
                    );
                },
            );
        });
        section_title(ui, l("Словарь диктовки", "Dictation dictionary"));
        card(ui, |ui| {
            subtle(
                ui,
                l(
                    "Как слышится → как пишется. Слова справа ещё и подсказываются распознаванию.",
                    "Heard as → written as. Words on the right are also hinted to the recognizer.",
                ),
            );
            pair_list(
                ui,
                "dict",
                &mut d.dictionary,
                l("как слышится", "heard as"),
                l("как пишется", "written as"),
                false,
            );
        });
    }

    fn translate(&mut self, ui: &mut Ui, d: &mut Data) {
        let s = &mut d.settings;
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.translate_enabled,
                l("Перевод выделенного", "Translate the selection"),
                l(
                    "Русский ↔ английский, на этом компьютере, без интернета",
                    "Russian ↔ English, on this computer, offline",
                ),
            );
            row(ui, l("Сочетание", "Shortcut"), "", |ui| {
                hotkey_field(
                    ui,
                    &mut self.rec,
                    "translate2",
                    &mut s.hotkey_translate,
                    &["Ctrl+Alt+T", "Ctrl+Shift+T"],
                );
            });
            toggle_row(
                ui,
                &mut s.translate_sound_enabled,
                l("Звук перевода", "Translation sound"),
                "",
            );
        });
        section_title(ui, l("Языковые пакеты", "Language packs"));
        card(ui, |ui| {
            crate::ui::settings::translate_packs(ui, &mut self.downloads);
        });
    }

    fn history(&mut self, ui: &mut Ui, d: &mut Data) {
        let s = &mut d.settings;
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.voice_history_enabled,
                l("Хранить историю диктовок", "Keep dictation history"),
                l(
                    "Зашифрована: прочитать её можете только вы на этом компьютере",
                    "Encrypted: only you on this computer can read it",
                ),
            );
            row(
                ui,
                l("Сколько хранить", "Keep for"),
                "",
                |ui| {
                    egui::ComboBox::from_id_salt("ret")
                        .selected_text(retention_label(s.voice_history_minutes))
                        .show_ui(ui, |ui| {
                            for m in RETENTION_CHOICES {
                                ui.selectable_value(
                                    &mut s.voice_history_minutes,
                                    m,
                                    retention_label(m),
                                );
                            }
                        });
                },
            );
            toggle_row(
                ui,
                &mut s.voice_save_audio,
                l("Сохранять запись голоса", "Save voice recordings"),
                l("Можно переслушать в истории", "Replay them in the history"),
            );
            toggle_row(
                ui,
                &mut s.clipboard_history_enabled,
                l("История буфера обмена", "Clipboard history"),
                l(
                    "Скопированный текст тоже попадает в историю (пароли — нет)",
                    "Copied text goes to the history too (passwords don't)",
                ),
            );
        });
        ui.horizontal(|ui| {
            if ui.button(l("Открыть историю", "Open history")).clicked() {
                crate::ui::spawn("history", None);
            }
            if !self.confirm_clear_history {
                if ui
                    .button(l("Очистить историю…", "Clear history…"))
                    .clicked()
                {
                    self.confirm_clear_history = true;
                }
            } else {
                ui.label(l(
                    "Удалить всё безвозвратно?",
                    "Delete everything for good?",
                ));
                if ui.button(l("Удалить", "Delete")).clicked() {
                    crate::ui::history::clear_all();
                    self.confirm_clear_history = false;
                }
                if ui.button(l("Отмена", "Cancel")).clicked() {
                    self.confirm_clear_history = false;
                }
            }
        });
    }

    fn general(&mut self, ui: &mut Ui, d: &mut Data) {
        let s = &mut d.settings;
        card(ui, |ui| {
            let mut auto = *self
                .autostart
                .get_or_insert_with(crate::win::sys::autostart_enabled);
            if toggle_row(
                ui,
                &mut auto,
                l("Запускать вместе с Windows", "Start with Windows"),
                "",
            ) {
                crate::win::sys::set_autostart(auto);
                self.autostart = Some(auto);
            }
            row(
                ui,
                l("Язык интерфейса", "Interface language"),
                "",
                |ui| {
                    for (v, label) in [
                        ("en", "English"),
                        ("ru", "Русский"),
                        ("auto", l("как в системе", "system")),
                    ] {
                        ui.selectable_value(&mut s.language, v.to_string(), label);
                    }
                },
            );
            row(ui, l("Оформление", "Appearance"), "", |ui| {
                for (v, label) in [
                    ("dark", l("тёмное", "dark")),
                    ("light", l("светлое", "light")),
                    ("system", l("как в системе", "system")),
                ] {
                    ui.selectable_value(&mut s.app_theme, v.to_string(), label);
                }
            });
            row(
                ui,
                l("Щелчок по значку в трее", "Tray icon click"),
                "",
                |ui| {
                    egui::ComboBox::from_id_salt("tray")
                        .selected_text(tray_label(&s.tray_click))
                        .show_ui(ui, |ui| {
                            for v in ["menu", "settings", "history", "dictate", "pause"] {
                                ui.selectable_value(
                                    &mut s.tray_click,
                                    v.to_string(),
                                    tray_label(v),
                                );
                            }
                        });
                },
            );
        });
        section_title(ui, l("Обновления", "Updates"));
        card(ui, |ui| {
            toggle_row(
                ui,
                &mut s.silent_auto_update,
                l("Ставить обновления сами", "Install updates automatically"),
                l(
                    "Иначе — только сообщить о новой версии",
                    "Otherwise just notify about a new version",
                ),
            );
            toggle_row(
                ui,
                &mut s.beta_channel,
                l("Бета-версии", "Beta versions"),
                "",
            );
            if ui.button(l("Проверить сейчас", "Check now")).clicked() {
                ipc::send("check-updates");
            }
        });
        section_title(ui, l("Данные", "Data"));
        ui.horizontal(|ui| {
            if ui
                .button(l("Папка настроек и лог", "Settings folder and log"))
                .clicked()
            {
                crate::win::sys::open_folder(&d.store.dir);
            }
            if ui.button(l("Папка моделей", "Models folder")).clicked() {
                crate::win::sys::open_folder(&models::dir());
            }
        });
    }

    fn about(&mut self, ui: &mut Ui, d: &mut Data) {
        card(ui, |ui| {
            ui.label(
                RichText::new(format!("Keyboop {} · Windows", env!("CARGO_PKG_VERSION"))).strong(),
            );
            ui.label(l("Бесплатный open-source переключатель раскладки, который не трогает буфер обмена. И набирает текст голосом.", "A free, open-source layout switcher that doesn't break your clipboard. And it types by voice."));
            ui.horizontal(|ui| {
                ui.hyperlink_to("keyboop.com", "https://keyboop.com");
                ui.hyperlink_to("Telegram", "https://t.me/keyboop");
                ui.hyperlink_to("GitHub", "https://github.com/krazzer00/keyboop");
            });
            if ui
                .button(l("Написать разработчику", "Send feedback"))
                .clicked()
            {
                crate::ui::spawn("feedback", None);
            }
        });
        section_title(ui, l("Что нового", "What's new"));
        card(ui, |ui| {
            for (ver, items) in crate::ui::whats_new() {
                ui.label(RichText::new(ver).strong());
                for it in items {
                    ui.label(format!("• {it}"));
                }
            }
        });
        section_title(ui, l("Сторонние компоненты", "Third-party components"));
        card(ui, |ui| {
            subtle(ui, "whisper.cpp (MIT) · egui (MIT/Apache-2.0) · cpal (Apache-2.0) · keyswitcher data (MIT) · tiny-skia (BSD-3)");
        });
        let _ = d;
    }
}

fn mode_label(m: &str) -> &'static str {
    match m {
        "off" => l("не переключать", "don't switch"),
        "soft" => l("мягко", "softly"),
        _ => l("как обычно", "as usual"),
    }
}

fn layout_label(v: &str) -> &'static str {
    match v {
        "en" => l("всегда английская", "always English"),
        "ru" => l("всегда русская", "always Russian"),
        _ => l("раскладку не трогать", "keep layout"),
    }
}

fn tray_label(v: &str) -> &'static str {
    match v {
        "settings" => l("открыть настройки", "open settings"),
        "history" => l("открыть историю", "open history"),
        "dictate" => l("начать диктовку", "start dictation"),
        "pause" => l("пауза на 15 минут", "pause for 15 minutes"),
        _ => l("показать меню", "show the menu"),
    }
}

fn retention_label(m: u32) -> String {
    match m {
        0 => l("всегда", "forever").into(),
        30 => l("30 минут", "30 minutes").into(),
        60 => l("1 час", "1 hour").into(),
        480 => l("8 часов", "8 hours").into(),
        10080 => l("неделю", "a week").into(),
        43200 => l("месяц", "a month").into(),
        n => format!("{n} {}", l("мин", "min")),
    }
}

/// Языковые пакеты перевода — заполняется на этапе перевода.
pub fn translate_packs(ui: &mut Ui, _downloads: &mut HashMap<String, Arc<Download>>) {
    subtle(
        ui,
        l(
            "Пакеты перевода появятся здесь.",
            "Translation packs will appear here.",
        ),
    );
}
