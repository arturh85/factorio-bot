use crate::settings::load_app_settings;
use factorio_bot_core::process::process_control::FactorioInstance;
use miette::{IntoDiagnostic, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

#[allow(clippy::items_after_statements)]
pub async fn start() -> Result<()> {
  let instance_state: Option<FactorioInstance> = None;
  let restapi_handle: Option<JoinHandle<Result<()>>> = None;
  let app_settings = load_app_settings()?;
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      crate::commands::is_restapi_started,
      crate::commands::is_instance_started,
      crate::commands::is_port_available,
      crate::commands::load_script,
      crate::commands::load_scripts_in_directory,
      crate::commands::execute_rcon,
      crate::commands::execute_script,
      crate::commands::execute_code,
      crate::commands::update_settings,
      crate::commands::load_settings,
      crate::commands::save_settings,
      crate::commands::start_instances,
      crate::commands::stop_instances,
      crate::commands::start_restapi,
      crate::commands::stop_restapi,
      crate::commands::maximize_window,
      crate::commands::file_exists,
      crate::commands::open_in_browser,
    ])
    .manage(Arc::new(RwLock::new(app_settings)))
    .manage(Arc::new(RwLock::new(instance_state)))
    .manage(RwLock::new(restapi_handle))
    .run(tauri::generate_context!())
    .into_diagnostic()
}
