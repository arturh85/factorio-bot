//! Re-exported from `factorio_bot_core::app_settings` so the existing
//! `crate::settings::` call sites keep compiling.
pub use factorio_bot_core::app_settings::{load_app_settings, SharedAppSettings};
// `AppSettings` itself is only named by the GUI's settings commands; without
// the `gui` feature re-exporting it would be an unused import warning.
#[cfg(feature = "gui")]
pub use factorio_bot_core::app_settings::AppSettings;
