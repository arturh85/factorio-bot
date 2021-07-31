use crate::commands::ERR_TO_STRING;
use crate::constants::app_workspace_path;
use async_std::sync::Mutex;
use factorio_bot_backend::factorio::instance_setup::setup_factorio_instance;
use factorio_bot_backend::factorio::rcon::RconSettings;
use factorio_bot_backend::settings::AppSettings;
use tauri::State;

#[tauri::command]
pub async fn start_instances(
  _app_handle: tauri::AppHandle<tauri::Wry>,
  app_settings: State<'_, Mutex<AppSettings>>,
) -> Result<(), String> {
  let app_settings = app_settings.lock().await;
  let workspace_path = app_workspace_path();
  let workspace_path_str = workspace_path.to_str().unwrap();
  std::fs::create_dir_all(&workspace_path).map_err(|e| String::from("error: ") + &e.to_string())?;

  let rcon_settings =
    RconSettings::new(app_settings.rcon_port as u16, &app_settings.rcon_pass, None);
  setup_factorio_instance(
    workspace_path_str,
    &rcon_settings,
    Some(4711),
    "server",
    true,
    false,
    None,
    None,
    false,
  )
  .await
  .map_err(ERR_TO_STRING)?;
  Ok(())
}
