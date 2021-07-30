#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]
mod app_settings;
mod commands;
mod constants;

use crate::app_settings::AppSettings;
use crate::constants::default_app_dir;
use std::time::Duration;
use tauri::Manager;

#[tauri::command]
async fn my_custom_command(app_handle: tauri::AppHandle<tauri::Wry>) {
  println!("I was invoked from JS!");
  async_std::task::sleep(Duration::from_secs(1)).await;
  println!("I was invoked after 1000ms!");
  app_handle.emit_all("the_event", "foo").unwrap();
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
  std::fs::create_dir_all(default_app_dir())?;
  let settings = AppSettings::load()?;

  // tauri::api::dir::read_dir
  let app_data_dir = tauri::api::path::local_data_dir()
    .expect("no local data directory available")
    .join(env!("CARGO_PKG_NAME"));

  println!("local data dir: {:?}", app_data_dir);

  // get_matches()
  // match get_matches() {
  //     Some(matches) => {
  //         // `matches` here is a Struct with { args, subcommand }
  //         // where args is the HashMap mapping each arg's name to it's { value, occurrences }
  //         // and subcommand is an Option of { name, matches }
  //     }
  // }
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      my_custom_command,
      crate::commands::config::load_config,
      crate::commands::config::save_config
    ])
    .manage(settings)
    .run(tauri::generate_context!())
    .expect("failed to run app");
  Ok(())
}
