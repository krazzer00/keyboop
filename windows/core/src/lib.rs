//! Keyboop core: всё, что не зависит от операционной системы.
//!
//! Это перенос логики из `Sources/Keyboop/*.swift` (macOS). Имена модулей и функций повторяют
//! Swift-оригинал, чтобы правку можно было сверить построчно: `LayoutDetector.decide` →
//! [`detector::decide`], `Keymap.smartConvert` → [`keymap::smart_convert`] и так далее.
//! Подробные истории «почему так» живут в комментариях Swift-файлов; здесь оставлены только
//! ссылки на них и то, что отличается на Windows.

pub mod ambiguous;
pub mod audio_import;
pub mod buffer;
pub mod detector;
pub mod engine;
pub mod exceptions;
pub mod extra_words;
pub mod history;
pub mod keymap;
pub mod layout_data;
pub mod resonance;
pub mod settings;
pub mod snippets;
pub mod text;
pub mod typo;
pub mod undo;
pub mod voice;

pub use engine::{Engine, Key, KeyInput, ManualStep, Platform, Script};
pub use settings::Settings;
