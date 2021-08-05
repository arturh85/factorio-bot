#![allow(clippy::module_name_repetitions)]
use crate::commands::ERR_TO_STRING;
use async_std::sync::RwLock;
use factorio_bot_backend::factorio::planner::Planner;
use factorio_bot_backend::factorio::process_control::InstanceState;
use factorio_bot_backend::settings::AppSettings;
use tauri::State;

#[tauri::command]
pub async fn execute_script(
  app_settings: State<'_, RwLock<AppSettings>>,
  instance_state: State<'_, RwLock<Option<InstanceState>>>,
  code: String,
) -> Result<(), String> {
  if let Some(instance_state) = &*instance_state.read().await {
    if let Some(world) = &instance_state.world {
      info!("creating planner 1");
      let world = world.clone();
      info!("creating planner 2");
      let rcon = instance_state.rcon.clone();
      info!("creating planner 3");
      let mut planner = Planner::new(world, Some(rcon));
      info!("running {}", code);
      planner
        .plan(code, app_settings.read().await.client_count as u32)
        .map_err(ERR_TO_STRING)?;
    } else {
      warn!("execute_script called without world instance");
    }
  } else {
    warn!("execute_script called without running instance");
  }
  Ok(())
}
