#![allow(clippy::module_name_repetitions)]

use tauri::{AppHandle, Manager, Wry};

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn maximize_window(app_handle: AppHandle<Wry>) -> Result<(), String> {
  app_handle
    .get_window("main")
    .unwrap()
    .maximize()
    .map_err(|e| format!("error: {}", e))
}
