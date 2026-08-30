//! Re-exported from `factorio_bot_core::paths` so the ~10 existing
//! `crate::paths::` call sites keep compiling.
pub use factorio_bot_core::paths::{data_local_dir, workspace_dir};
// `settings_file` is only written to from the GUI's settings commands; without
// the `gui` feature re-exporting it would be an unused import warning.
#[cfg(feature = "gui")]
pub use factorio_bot_core::paths::settings_file;
