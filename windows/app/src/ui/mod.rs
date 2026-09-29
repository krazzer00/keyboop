//! Окна Keyboop (аналог `SettingsWindow`, `VoiceHistoryWindow`, `WelcomeWindow`, `FeedbackWindow`).
//!
//! Живут в ОТДЕЛЬНОМ процессе `keyboop.exe --ui <окно>`: у главного процесса колбэк хука
//! клавиатуры, которому нельзя ждать ни миллисекунды, а тяжёлое окно с графикой ему не сосед.
//! Закрыли окно — процесс ушёл, память вернулась. Общение с главным процессом — через те же
//! JSON-файлы (он перечитывает их сам) и короткие команды WM_COPYDATA (см. [`ipc`]).

mod feedback;
mod history;
mod settings;
mod welcome;
mod widgets;

use crate::storage::{State, Store};
use eframe::egui;
use keyboop_core::exceptions::Exceptions;
use keyboop_core::Settings;

pub const CORAL: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x7A, 0x59);
pub const GRAPHITE: egui::Color32 = egui::Color32::from_rgb(0x1C, 0x1B, 0x1A);

/// Язык интерфейса окна: русский или английский (как l10n главного процесса).
pub fn l<'a>(ru: &'a str, en: &'a str) -> &'a str {
    if crate::l10n::is_russian() {
        ru
    } else {
        en
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Settings,
    History,
    Welcome,
    Feedback,
}

impl Kind {
    fn parse(s: &str) -> Kind {
        match s {
            "history" => Kind::History,
            "welcome" => Kind::Welcome,
            "feedback" => Kind::Feedback,
            _ => Kind::Settings,
        }
    }
    fn title(self) -> String {
        match self {
            Kind::Settings => l("Keyboop — настройки", "Keyboop — Settings").into(),
            Kind::History => l("Keyboop — история", "Keyboop — History").into(),
            Kind::Welcome => l("Добро пожаловать в Keyboop", "Welcome to Keyboop").into(),
            Kind::Feedback => {
                l("Keyboop — написать разработчику", "Keyboop — Send feedback").into()
            }
        }
    }
    fn size(self) -> [f32; 2] {
        match self {
            Kind::Settings => [860.0, 640.0],
            Kind::History => [720.0, 620.0],
            Kind::Welcome => [620.0, 560.0],
            Kind::Feedback => [560.0, 520.0],
        }
    }
}

/// Команда главному процессу (WM_COPYDATA окну трея). Молча, если он не запущен.
pub mod ipc {
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub const MAGIC: usize = 0x4B42_4950; // "KBIP"

    pub fn send(cmd: &str) {
        unsafe {
            let class: Vec<u16> = "KeyboopTray".encode_utf16().chain(Some(0)).collect();
            let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
            if hwnd.is_null() {
                return;
            }
            let data = COPYDATASTRUCT {
                dwData: MAGIC,
                cbData: cmd.len() as u32,
                lpData: cmd.as_ptr() as *mut _,
            };
            let mut result = 0usize;
            SendMessageTimeoutW(
                hwnd,
                WM_COPYDATA,
                0,
                &data as *const _ as LPARAM,
                SMTO_ABORTIFHUNG,
                1000,
                &mut result,
            );
        }
    }
}

/// Всё, что окна читают и пишут на диск.
pub struct Data {
    pub store: Store,
    pub settings: Settings,
    saved_settings: Settings,
    pub exceptions: Exceptions,
    saved_exceptions: String,
    pub snippets: Vec<(String, String)>,
    saved_snippets: Vec<(String, String)>,
    pub text_snippets: Vec<(String, String)>,
    saved_text_snippets: Vec<(String, String)>,
    pub dictionary: Vec<(String, String)>,
    saved_dictionary: Vec<(String, String)>,
    pub state: State,
    pub errors: Vec<String>,
    last_write: std::time::Instant,
}

impl Data {
    fn load() -> Data {
        let store = Store::open();
        let mut errors = Vec::new();
        let settings = store.load_settings(&mut errors);
        let exceptions = store.load_exceptions(&mut errors);
        let snippets = store.load_snippets(&mut errors);
        let text_snippets = store.load_text_snippets();
        let dictionary = store.load_dictionary();
        let state = store.load_state();
        Data {
            saved_settings: settings.clone(),
            saved_exceptions: serde_json::to_string(&exceptions).unwrap_or_default(),
            saved_snippets: snippets.clone(),
            saved_text_snippets: text_snippets.clone(),
            saved_dictionary: dictionary.clone(),
            store,
            settings,
            exceptions,
            snippets,
            text_snippets,
            dictionary,
            state,
            errors,
            last_write: std::time::Instant::now(),
        }
    }

    /// Записать изменения на диск и сказать главному процессу перечитать.
    /// Не чаще раза в 300 мс: слайдер громкости не должен писать файл на каждый пиксель.
    fn flush(&mut self, force: bool) {
        if !force && self.last_write.elapsed() < std::time::Duration::from_millis(300) {
            return;
        }
        let mut changed = false;
        if self.settings != self.saved_settings {
            self.store.save_settings(&self.settings);
            self.saved_settings = self.settings.clone();
            changed = true;
        }
        let exc = serde_json::to_string(&self.exceptions).unwrap_or_default();
        if exc != self.saved_exceptions {
            self.store.save_exceptions(&self.exceptions);
            self.saved_exceptions = exc;
            changed = true;
        }
        if self.snippets != self.saved_snippets {
            let clean: Vec<_> = self
                .snippets
                .iter()
                .filter(|(t, _)| !t.trim().is_empty())
                .cloned()
                .collect();
            self.store.save_snippets(crate::storage::SNIPPETS, &clean);
            self.saved_snippets = self.snippets.clone();
            changed = true;
        }
        if self.text_snippets != self.saved_text_snippets {
            let clean: Vec<_> = self
                .text_snippets
                .iter()
                .filter(|(t, x)| !t.is_empty() || !x.is_empty())
                .cloned()
                .collect();
            self.store
                .save_snippets(crate::storage::TEXT_SNIPPETS, &clean);
            self.saved_text_snippets = self.text_snippets.clone();
            changed = true;
        }
        if self.dictionary != self.saved_dictionary {
            let clean: Vec<_> = self
                .dictionary
                .iter()
                .filter(|(h, _)| !h.trim().is_empty())
                .cloned()
                .collect();
            self.store.save_dictionary(&clean);
            self.saved_dictionary = self.dictionary.clone();
            changed = true;
        }
        if changed {
            self.last_write = std::time::Instant::now();
            ipc::send("reload");
        }
    }
}

pub struct KeyboopUi {
    kind: Kind,
    data: Data,
    settings: settings::SettingsView,
    history: history::HistoryView,
    welcome: welcome::WelcomeView,
    feedback: feedback::FeedbackView,
    theme_applied: String,
}

impl eframe::App for KeyboopUi {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.theme_applied != self.data.settings.app_theme {
            self.theme_applied = self.data.settings.app_theme.clone();
            ctx.set_theme(match self.theme_applied.as_str() {
                "light" => egui::ThemePreference::Light,
                "dark" => egui::ThemePreference::Dark,
                _ => egui::ThemePreference::System,
            });
        }
        match self.kind {
            Kind::Settings => self.settings.show(ui, &mut self.data),
            Kind::History => self.history.show(ui, &mut self.data),
            Kind::Welcome => {
                if self.welcome.show(ui, &mut self.data) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Kind::Feedback => self.feedback.show(ui, &mut self.data),
        }
        self.data.flush(false);
        // Пока что-то качается или играет — перерисовываемся сами.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.data.flush(true);
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, file: &str, family: egui::FontFamily| {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{file}")) {
            fonts.font_data.insert(
                name.into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.into());
            true
        } else {
            false
        }
    };
    add("segoe", "segoeui.ttf", egui::FontFamily::Proportional);
    add("consolas", "consola.ttf", egui::FontFamily::Monospace);
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(22.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.5, FontFamily::Monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.visuals.selection.bg_fill = CORAL;
        style.visuals.selection.stroke.color = egui::Color32::WHITE;
        style.visuals.hyperlink_color = CORAL;
        style.visuals.widgets.active.bg_fill = CORAL;
    });
}

fn load_icon() -> Option<egui::IconData> {
    let png = include_bytes!("../../../../Sources/Keyboop/Resources/menubar-mark.png");
    eframe::icon_data::from_png_bytes(png).ok()
}

static INSTANCE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Одно окно каждого вида: повторный запуск поднимает уже открытое.
fn already_open(kind: Kind) -> bool {
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let name: Vec<u16> = format!("Local\\KeyboopUi-{kind:?}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if h.is_null() || GetLastError() != ERROR_ALREADY_EXISTS {
            INSTANCE.store(h as isize, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        let title: Vec<u16> = kind.title().encode_utf16().chain(Some(0)).collect();
        let w = FindWindowW(std::ptr::null(), title.as_ptr());
        if !w.is_null() {
            ShowWindow(w, SW_RESTORE);
            SetForegroundWindow(w);
        }
        true
    }
}

/// Открыть окно отдельным процессом.
pub fn spawn(kind: &str, section: Option<&str>) {
    if let Ok(exe) = std::env::current_exe() {
        let mut c = std::process::Command::new(exe);
        c.args(["--ui", kind]);
        if let Some(s) = section {
            c.args(["--section", s]);
        }
        let _ = c.spawn();
    }
}

/// «Что нового» Windows-версии.
pub fn whats_new() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![(
        "0.4.10",
        vec![
            l(
                "Keyboop для Windows: тот же детектор раскладки и словари, что на Маке",
                "Keyboop for Windows: the same layout detector and dictionaries as on the Mac",
            ),
            l(
                "Голосовой набор на whisper.cpp — локально, без интернета",
                "Voice typing on whisper.cpp — local, offline",
            ),
            l(
                "История диктовок и буфера обмена, зашифрованная для вашего пользователя",
                "Dictation and clipboard history, encrypted for your Windows user",
            ),
            l(
                "Сниппеты, выбор текста по цифре, вставка без форматирования",
                "Snippets, pick a text by number, paste as plain text",
            ),
        ],
    )]
}

/// Точка входа `keyboop.exe --ui <окно> [--section <раздел>]`.
pub fn run(kind: &str, section: Option<String>) {
    let kind = Kind::parse(kind);
    let data = Data::load();
    crate::l10n::set_language(&data.settings.language, crate::win_system_is_russian());
    if already_open(kind) {
        return;
    }
    let renderer = match std::env::var("KEYBOOP_RENDERER").as_deref() {
        Ok("glow") => eframe::Renderer::Glow,
        _ => eframe::Renderer::Wgpu,
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(kind.title())
        .with_inner_size(kind.size())
        .with_min_inner_size([480.0, 360.0]);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer,
        centered: true,
        ..Default::default()
    };
    let title = kind.title();
    let result = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(KeyboopUi {
                kind,
                settings: settings::SettingsView::new(section.as_deref()),
                history: history::HistoryView::default(),
                welcome: welcome::WelcomeView::default(),
                feedback: feedback::FeedbackView::default(),
                theme_applied: String::new(),
                data,
            }))
        }),
    );
    // Direct3D не завёлся (виртуальная машина, старый драйвер) — пробуем OpenGL новым процессом:
    // второй цикл событий в том же процессе создать нельзя.
    if result.is_err() && renderer == eframe::Renderer::Wgpu {
        // Отпускаем «одно окно», иначе новый процесс решит, что окно уже открыто.
        let h = INSTANCE.swap(0, std::sync::atomic::Ordering::Relaxed);
        if h != 0 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(h as _);
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut args: Vec<String> = std::env::args().skip(1).collect();
            args.retain(|a| a != "--single");
            let _ = std::process::Command::new(exe)
                .args(args)
                .env("KEYBOOP_RENDERER", "glow")
                .spawn();
        }
    }
}
