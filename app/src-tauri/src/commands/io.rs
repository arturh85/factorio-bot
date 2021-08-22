#![allow(clippy::module_name_repetitions)]

use std::path::PathBuf;
use std::str::FromStr;

#[tauri::command]
pub fn file_exists(path: &str) -> Result<bool, String> {
  let path = PathBuf::from_str(path).map_err(|e| String::from("error: ") + &e.to_string())?;
  Ok(path.exists())
}
