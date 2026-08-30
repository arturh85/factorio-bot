use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use std::path::PathBuf;

#[derive(Clone)]
pub struct AppState {
    pub instance: SharedFactorioInstance,
    pub settings: SharedAppSettings,
    /// Where `PUT /api/v1/settings` persists. Production wiring points this
    /// at `factorio_bot_core::paths::settings_file()` via [`AppState::new`];
    /// tests that exercise persistence should build `AppState` directly with
    /// a path inside a `tempfile::TempDir` instead, so the suite never
    /// overwrites a real developer's settings file.
    pub settings_path: PathBuf,
}

impl AppState {
    /// Builds production state: `settings_path` is the real on-disk settings
    /// file. Test helpers that only exercise routes unrelated to persistence
    /// (i.e. everything but `put_settings`) should use this too, rather than
    /// repeating the struct literal, so a future field addition only touches
    /// this constructor.
    pub fn new(instance: SharedFactorioInstance, settings: SharedAppSettings) -> Self {
        AppState {
            instance,
            settings,
            settings_path: factorio_bot_core::paths::settings_file(),
        }
    }
}
