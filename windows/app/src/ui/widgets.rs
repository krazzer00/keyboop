//! Общие элементы окон: переключатель-«тумблер», строка настройки с подписью, заголовок раздела,
//! поле хоткея с записью сочетания, редактор списка пар, чипы слов.

use super::{l, CORAL};
use eframe::egui::{self, Color32, Response, RichText, Sense, Ui};

/// Тумблер в стиле macOS (рисуем сами: у egui только флажок).
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = egui::vec2(38.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let off_bg = if ui.visuals().dark_mode {
            Color32::from_gray(80)
        } else {
            Color32::from_gray(200)
        };
        let bg = lerp_color(off_bg, CORAL, t);
        let r = rect.height() / 2.0;
        ui.painter().rect_filled(rect, r, bg);
        let cx = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
        ui.painter()
            .circle_filled(egui::pos2(cx, rect.center().y), r - 3.0, Color32::WHITE);
    }
    response
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

pub fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .small()
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(2.0);
}

pub fn subtle(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .small()
            .color(ui.visuals().weak_text_color()),
    );
}

/// Карточка: скруглённый фон под группой настроек.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Строка «название + пояснение ... тумблер». Возвращает true, если значение изменилось.
pub fn toggle_row(ui: &mut Ui, on: &mut bool, title: &str, subtitle: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 60.0).max(120.0));
            ui.label(title);
            if !subtitle.is_empty() {
                subtle(ui, subtitle);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = toggle(ui, on).changed();
        });
    });
    changed
}

/// Строка «название ... произвольный элемент справа».
pub fn row(ui: &mut Ui, title: &str, subtitle: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            // Справа остаётся место под поле хоткея или переключатель вариантов.
            ui.set_width((ui.available_width() - HOTKEY_FIELD_WIDTH - 10.0).max(160.0));
            ui.label(title);
            if !subtitle.is_empty() {
                subtle(ui, subtitle);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// Состояние записи хоткея (какое поле сейчас слушает клавиатуру).
#[derive(Default)]
pub struct HotkeyRecorder {
    pub listening: Option<String>,
}

fn key_name(k: egui::Key) -> Option<String> {
    use egui::Key::*;
    Some(match k {
        Space => "Space".into(),
        Tab => "Tab".into(),
        Enter => "Enter".into(),
        Escape => return None,
        Backspace => "Backspace".into(),
        Delete => "Delete".into(),
        Insert => "Insert".into(),
        Home => "Home".into(),
        End => "End".into(),
        PageUp => "PageUp".into(),
        PageDown => "PageDown".into(),
        Backtick => "`".into(),
        Minus => "-".into(),
        Equals => "=".into(),
        OpenBracket => "[".into(),
        CloseBracket => "]".into(),
        Backslash => "\\".into(),
        Semicolon => ";".into(),
        Quote => "'".into(),
        Comma => ",".into(),
        Period => ".".into(),
        Slash => "/".into(),
        other => {
            let n = other.name();
            if n.len() == 1 || n.starts_with('F') {
                n.to_string()
            } else {
                return None;
            }
        }
    })
}

const HOTKEY_FIELD_WIDTH: f32 = 290.0;

/// Поле хоткея: текущая запись, кнопка «Записать», быстрый выбор особых клавиш, «Выкл».
/// Возвращает true, если значение изменилось.
pub fn hotkey_field(
    ui: &mut Ui,
    rec: &mut HotkeyRecorder,
    id: &str,
    value: &mut String,
    presets: &[&str],
) -> bool {
    let mut changed = false;
    let listening = rec.listening.as_deref() == Some(id);
    if listening {
        // Ловим сочетание: модификаторы + клавиша. Esc — отмена записи.
        let (mods, key) = ui.input(|i| {
            let key = i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key, pressed: true, ..
                } => Some(*key),
                _ => None,
            });
            (i.modifiers, key)
        });
        if let Some(k) = key {
            if k == egui::Key::Escape {
                rec.listening = None;
                super::ipc::send("hotkeys-on");
            } else if let Some(name) = key_name(k) {
                let mut parts = Vec::new();
                if mods.ctrl {
                    parts.push("Ctrl");
                }
                if mods.shift {
                    parts.push("Shift");
                }
                if mods.alt {
                    parts.push("Alt");
                }
                let mut s = parts.join("+");
                if !s.is_empty() {
                    s.push('+');
                }
                s.push_str(&name);
                *value = s;
                changed = true;
                rec.listening = None;
                super::ipc::send("hotkeys-on");
            }
        }
    }
    // Поле, кнопка записи и выбор готовых вариантов всегда слева направо, даже в строке, которая
    // выравнивает свою правую часть по правому краю.
    // Ширина фиксирована: так поле не растягивает карточку и справа ничего не обрезается.
    let size = egui::vec2(HOTKEY_FIELD_WIDTH, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let shown = if listening {
                l("Нажмите…", "Press keys…").to_string()
            } else if value.is_empty() {
                l("выключено", "off").to_string()
            } else {
                value.clone()
            };
            let valid = value.is_empty() || crate::hotkey::parse(value).is_ok();
            let text = RichText::new(shown).monospace();
            let text = if valid {
                text
            } else {
                text.color(Color32::from_rgb(220, 60, 60))
            };
            let field = ui.add(
                egui::Button::new(text)
                    .min_size(egui::vec2(140.0, 26.0))
                    .selected(listening),
            );
            if listening {
                field.on_hover_text(l("Esc — отмена", "Esc cancels"));
            }
            if ui
                .small_button(if listening {
                    "×"
                } else {
                    l("Записать", "Record")
                })
                .clicked()
            {
                if listening {
                    rec.listening = None;
                    super::ipc::send("hotkeys-on");
                } else {
                    rec.listening = Some(id.to_string());
                    // Пока пишем сочетание, главный процесс не должен его перехватывать.
                    super::ipc::send("hotkeys-off");
                }
            }
            egui::ComboBox::from_id_salt(format!("{id}-presets"))
                .selected_text("")
                .width(28.0)
                .show_ui(ui, |ui| {
                    for p in presets {
                        if ui.selectable_label(value == p, *p).clicked() {
                            *value = p.to_string();
                            changed = true;
                        }
                    }
                    if ui
                        .selectable_label(value.is_empty(), l("Выключить", "Turn off"))
                        .clicked()
                    {
                        value.clear();
                        changed = true;
                    }
                });
        },
    );
    changed
}

/// Редактор списка пар «слева → справа» с добавлением и удалением.
pub fn pair_list(
    ui: &mut Ui,
    id: &str,
    pairs: &mut Vec<(String, String)>,
    left_hint: &str,
    right_hint: &str,
    multiline_right: bool,
) {
    let mut remove = None;
    let width = ui.available_width();
    for (i, (a, b)) in pairs.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(a)
                    .hint_text(left_hint)
                    .desired_width(width * 0.3),
            );
            ui.label("→");
            if multiline_right {
                ui.add(
                    egui::TextEdit::multiline(b)
                        .hint_text(right_hint)
                        .desired_rows(1)
                        .desired_width(width * 0.55),
                );
            } else {
                ui.add(
                    egui::TextEdit::singleline(b)
                        .hint_text(right_hint)
                        .desired_width(width * 0.55),
                );
            }
            if ui
                .small_button("🗑")
                .on_hover_text(l("Удалить", "Delete"))
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        pairs.remove(i);
    }
    if ui.button(format!("+ {}", l("Добавить", "Add"))).clicked() {
        pairs.push((String::new(), String::new()));
    }
    let _ = id;
}

/// Слова чипами с крестиком + поле добавления. Возвращает true, если список изменился.
pub fn chips(
    ui: &mut Ui,
    words: &mut std::collections::BTreeSet<String>,
    input: &mut String,
    hint: &str,
) -> bool {
    let mut changed = false;
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for w in words.iter() {
            let r = ui.add(egui::Button::new(format!("{w}  ×")).corner_radius(12.0));
            if r.clicked() {
                remove = Some(w.clone());
            }
        }
    });
    if let Some(w) = remove {
        words.remove(&w);
        changed = true;
    }
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(input)
                .hint_text(hint)
                .desired_width(220.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button(l("Добавить", "Add")).clicked() || enter) && !input.trim().is_empty()
        {
            for w in input
                .split([' ', ',', ';'])
                .map(|w| w.trim().to_lowercase())
                .filter(|w| !w.is_empty())
            {
                words.insert(w);
            }
            input.clear();
            changed = true;
        }
    });
    changed
}
