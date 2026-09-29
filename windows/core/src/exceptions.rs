//! Перенос `ExceptionStore.swift`: слова «не переключать», «всегда переключать», выученные на
//! отмене, и режимы программ. На Windows программа определяется именем exe (`code.exe`), а не
//! bundle id; ключи храним в нижнем регистре.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Exceptions {
    /// Не переключать никогда.
    pub ignored: BTreeSet<String>,
    /// Всегда переключать (в том числе выбор победителя спорной пары).
    pub force_swap: BTreeSet<String>,
    /// Выученные на отмене (см. `undo`).
    pub learned: BTreeSet<String>,
    /// exe → "off" | "soft" | "normal" ("normal" отменяет встроенный режим программы).
    pub app_modes: BTreeMap<String, String>,
    /// exe → "en" | "ru": при переходе в программу включать эту раскладку.
    pub app_layouts: BTreeMap<String, String>,
}

pub enum AddOutcome {
    Added,
    AlreadyThere,
    AlreadyBuiltin,
    Rejected,
}

impl Exceptions {
    /// Новый список с посевом по умолчанию: «вк», «тг», «vk» (решение автора, 04.08.2026).
    pub fn seeded() -> Self {
        let mut e = Exceptions::default();
        for w in ["вк", "тг", "vk"] {
            e.ignored.insert(w.to_string());
        }
        e
    }

    pub fn add_ignored(&mut self, word: &str) -> AddOutcome {
        let w = word.to_lowercase();
        if w.is_empty() || w.chars().any(char::is_whitespace) {
            return AddOutcome::Rejected;
        }
        let builtin = crate::extra_words::sets().default_keep.contains(w.as_str());
        let had = !self.ignored.insert(w.clone());
        self.force_swap.remove(&w);
        if builtin {
            AddOutcome::AlreadyBuiltin
        } else if had {
            AddOutcome::AlreadyThere
        } else {
            AddOutcome::Added
        }
    }

    pub fn add_force_swap(&mut self, word: &str) {
        let w = word.to_lowercase();
        if w.is_empty() {
            return;
        }
        self.ignored.remove(&w);
        self.force_swap.insert(w);
    }

    pub fn add_learned(&mut self, word: &str) {
        let w = word.to_lowercase();
        if !w.is_empty() {
            self.learned.insert(w);
        }
    }

    pub fn is_ignored(&self, w: &str) -> bool {
        self.ignored.contains(w)
    }
    pub fn is_learned(&self, w: &str) -> bool {
        self.learned.contains(w)
    }
    pub fn is_forced(&self, w: &str) -> bool {
        self.force_swap.contains(w)
    }

    /// Режим программы: явный выбор человека, иначе встроенный.
    pub fn app_mode(&self, exe: &str) -> String {
        let exe = exe.to_lowercase();
        match self.app_modes.get(&exe).map(String::as_str) {
            Some("normal") => String::new(),
            Some(m) => m.to_string(),
            None => builtin_app_mode(&exe).to_string(),
        }
    }

    pub fn app_layout(&self, exe: &str) -> Option<&str> {
        self.app_layouts
            .get(&exe.to_lowercase())
            .map(String::as_str)
    }
}

/// Терминалы и программы с голыми горячими клавишами (монтаж, звук, 3D) — «off»;
/// редакторы кода — «soft». Аналог `Engine.builtinAppMode` для имён exe Windows.
pub const TERMINAL_APPS: &[&str] = &[
    "windowsterminal.exe",
    "wt.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "openconsole.exe",
    "mintty.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "putty.exe",
    "kitty.exe",
    "tabby.exe",
    "hyper.exe",
    "warp.exe",
];

pub const DEFAULT_OFF_APPS: &[&str] = &[
    "adobe premiere pro.exe",
    "afterfx.exe",
    "adobe audition.exe",
    "resolve.exe",
    "blender.exe",
    "ableton live 12 suite.exe",
    "ableton live 11 suite.exe",
    "protools.exe",
    "cinema 4d.exe",
    "fl64.exe",
    "reaper.exe",
];

pub const DEFAULT_SOFT_APPS: &[&str] = &[
    "code.exe",
    "code - insiders.exe",
    "cursor.exe",
    "windsurf.exe",
    "devenv.exe",
    "zed.exe",
    "sublime_text.exe",
    "idea64.exe",
    "pycharm64.exe",
    "webstorm64.exe",
    "clion64.exe",
    "rider64.exe",
    "goland64.exe",
    "phpstorm64.exe",
    "rustrover64.exe",
    "datagrip64.exe",
    "rubymine64.exe",
    "studio64.exe",
    "gvim.exe",
    "nvim-qt.exe",
];

pub fn builtin_app_mode(exe: &str) -> &'static str {
    let exe = exe.to_lowercase();
    if TERMINAL_APPS.contains(&exe.as_str())
        || DEFAULT_OFF_APPS.contains(&exe.as_str())
        || exe.starts_with("ableton live")
    {
        "off"
    } else if DEFAULT_SOFT_APPS.contains(&exe.as_str()) {
        "soft"
    } else {
        ""
    }
}

/// Программа для «режима разработчика» (авто там молчит целиком): редакторы кода и терминалы.
pub fn is_dev_app(exe: &str) -> bool {
    let exe = exe.to_lowercase();
    DEFAULT_SOFT_APPS.contains(&exe.as_str()) || TERMINAL_APPS.contains(&exe.as_str())
}
