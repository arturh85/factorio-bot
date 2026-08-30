use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;

#[derive(Clone)]
pub struct AppState {
    pub instance: SharedFactorioInstance,
    pub settings: SharedAppSettings,
}
