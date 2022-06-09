#![allow(clippy::module_name_repetitions)]
use crate::gui::ERR_TO_STRING;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use tauri::State;

#[tauri::command]
pub async fn execute_rcon(
  instance_state: State<'_, SharedFactorioInstance>,
  command: String,
) -> Result<(), String> {
  if let Some(instance_state) = &*instance_state.read().await {
    instance_state
      .rcon
      .send(&command)
      .await
      .map_err(ERR_TO_STRING)?;
  } else {
    warn!("execute_rcon called without running instance");
  }
  Ok(())
}
