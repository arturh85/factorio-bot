use crate::settings::RestApiSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub instance: SharedFactorioInstance,
    pub settings: Arc<RwLock<RestApiSettings>>,
}
