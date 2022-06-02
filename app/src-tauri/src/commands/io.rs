#![allow(clippy::module_name_repetitions)]

use std::path::PathBuf;
use std::str::FromStr;

#[tauri::command]
pub fn file_exists(path: &str) -> Result<bool, String> {
  let path = PathBuf::from_str(path).map_err(|e| format!("error: {}", e))?;
  Ok(path.exists())
}

#[tauri::command]
pub fn is_port_available(port: u16) -> bool {
  port_scanner::local_port_available(port)
}

#[tauri::command]
pub fn open_in_browser(url: String) -> Result<(), String> {
  open::that(url).map_err(|e| format!("error: {}", e))
}
