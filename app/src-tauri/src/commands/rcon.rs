#![allow(clippy::module_name_repetitions)]
use crate::commands::ERR_TO_STRING;
use async_std::sync::RwLock;
use factorio_bot_core::factorio::process_control::InstanceState;
use tauri::State;

#[tauri::command]
pub async fn execute_rcon(
  instance_state: State<'_, RwLock<Option<InstanceState>>>,
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
