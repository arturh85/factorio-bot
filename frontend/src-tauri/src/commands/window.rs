use tauri::Manager;

#[tauri::command]
pub fn maximize_window(app_handle: tauri::AppHandle<tauri::Wry>) -> Result<(), String> {
  app_handle
    .get_window("main")
    .unwrap()
    .maximize()
    .map_err(|e| String::from("error: ") + &e.to_string())
}
