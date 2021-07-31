use std::time::Duration;
use tauri::Manager;

#[tauri::command]
pub async fn my_custom_command(app_handle: tauri::AppHandle<tauri::Wry>) {
  println!("I was invoked from JS!");
  async_std::task::sleep(Duration::from_secs(1)).await;
  println!("I was invoked after 1000ms!");
  app_handle.emit_all("the_event", "foo").unwrap();
}
