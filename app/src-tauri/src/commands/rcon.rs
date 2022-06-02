#![allow(clippy::module_name_repetitions)]
use crate::commands::ERR_TO_STRING;
use factorio_bot_core::process::process_control::FactorioInstance;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

#[tauri::command]
pub async fn execute_rcon(
  instance_state: State<'_, Arc<RwLock<Option<FactorioInstance>>>>,
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
