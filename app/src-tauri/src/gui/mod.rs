pub mod command;

use crate::context::Context;
use factorio_bot_core::miette::{Report, Result};

#[allow(clippy::items_after_statements)]
pub fn start(context: Context) -> Result<()> {
  let app = tauri::Builder::default()
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_http::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_updater::Builder::new().build())
    .manage(context.app_settings)
    .manage(context.instance_state)
    .manage(context.restapi_handle)
    .invoke_handler(tauri::generate_handler![
      command::is_restapi_started,
      command::is_instance_started,
      command::is_port_available,
      command::save_script,
      command::load_script,
      command::load_scripts_in_directory,
      command::execute_rcon,
      command::execute_script,
      command::execute_code,
      command::update_settings,
      command::load_settings,
      command::save_settings,
      command::start_instances,
      command::stop_instances,
      command::start_restapi,
      command::stop_restapi,
      command::maximize_window,
      command::file_exists,
      command::open_in_browser,
    ]);
  app
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
  Ok(())
}

pub const ERR_TO_STRING: fn(Report) -> String = |e| String::from("error: ") + &*format!("{e:?}");
