use crate::commands::ERR_TO_STRING;
use crate::constants;
use async_std::sync::RwLock;
use factorio_bot_backend::factorio::process_control::start_factorio;
use factorio_bot_backend::factorio::rcon::FactorioRcon;
use factorio_bot_backend::factorio::world::FactorioWorld;
use factorio_bot_backend::settings::AppSettings;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn start_instances(
  _app_handle: tauri::AppHandle<tauri::Wry>,
  app_settings: State<'_, RwLock<AppSettings>>,
  world: State<'_, RwLock<Option<Arc<FactorioWorld>>>>,
  rcon: State<'_, RwLock<Option<Arc<FactorioRcon>>>>,
) -> Result<(), String> {
  if world.read().await.is_some() || rcon.read().await.is_some() {
    return Result::Err("already started".into());
  }

  let app_settings = app_settings.read().await;
  let workspace_path = constants::app_workspace_path();
  std::fs::create_dir_all(&workspace_path).map_err(|e| String::from("error: ") + &e.to_string())?;

  let map_exchange_string = app_settings.map_exchange_string.to_string();
  let seed = app_settings.seed.to_string();

  let (started_world, started_rcon) = start_factorio(
    &app_settings,
    None,
    app_settings.client_count as u8,
    app_settings.recreate,
    if map_exchange_string.len() > 0 {
      Some(map_exchange_string)
    } else {
      None
    },
    if seed.len() > 0 { Some(seed) } else { None },
    false,
    false,
  )
  .await
  .map_err(ERR_TO_STRING)?;

  let mut world = world.write().await;
  *world = Some(started_world.unwrap());
  let mut rcon = rcon.write().await;
  *rcon = Some(started_rcon);

  Ok(())
}
