pub mod command;

use crate::context::Context;
use factorio_bot_core::miette::{IntoDiagnostic, Report, Result};

#[allow(clippy::items_after_statements)]
pub fn start(context: Context) -> Result<()> {
  let mut app = tauri::Builder::default()
    .any_thread()
    .manage(context.app_settings)
    .manage(context.instance_state)
    .manage(context.restapi_handle)
    .invoke_handler(tauri::generate_handler![
      crate::gui::command::is_restapi_started,
      crate::gui::command::is_instance_started,
      crate::gui::command::is_port_available,
      crate::gui::command::save_script,
      crate::gui::command::load_script,
      crate::gui::command::load_scripts_in_directory,
      crate::gui::command::execute_rcon,
      crate::gui::command::execute_script,
      crate::gui::command::execute_code,
      crate::gui::command::update_settings,
      crate::gui::command::load_settings,
      crate::gui::command::save_settings,
      crate::gui::command::start_instances,
      crate::gui::command::stop_instances,
      crate::gui::command::start_restapi,
      crate::gui::command::stop_restapi,
      crate::gui::command::maximize_window,
      crate::gui::command::file_exists,
      crate::gui::command::open_in_browser,
    ])
    .build(tauri::generate_context!())
    .into_diagnostic()?;
  loop {
    let iteration = app.run_iteration();
    if iteration.window_count == 0 {
      break;
    }
  }
  Ok(())
}

pub const ERR_TO_STRING: fn(Report) -> String = |e| String::from("error: ") + &*format!("{:?}", e);
