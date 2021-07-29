#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

// use tauri::cli::get_matches;

use std::time::Duration;
use tauri::Manager;

#[tauri::command]
async fn my_custom_command(app_handle: tauri::AppHandle) {
  println!("I was invoked from JS!");
  async_std::task::sleep(Duration::from_secs(1)).await;
  println!("I was invoked after 1000ms!");
  app_handle.emit_all("the_event", "foo").unwrap();
}

#[async_std::main]
async fn main() {
  // match get_matches() {
  //     Some(matches) => {
  //         // `matches` here is a Struct with { args, subcommand }
  //         // where args is the HashMap mapping each arg's name to it's { value, occurrences }
  //         // and subcommand is an Option of { name, matches }
  //     }
  // }
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![my_custom_command])
    .run(tauri::generate_context!())
    .expect("failed to run app");
}
