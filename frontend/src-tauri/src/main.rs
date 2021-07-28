#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use crate::plugins::instance_plugin::InstancePlugin;

mod cmd;
mod plugins;

fn main() {
  let instance_plugin = InstancePlugin::new();
  tauri::AppBuilder::new()
      .plugin(instance_plugin)
    .invoke_handler(|_webview, arg| {
      use cmd::Cmd::*;
      match serde_json::from_str(arg) {
        Err(e) => {
          Err(e.to_string())
        }
        Ok(command) => {
          match command {
            Start { } => {
              println!("Start");
            },
            Stop { } => {
              println!("Stop");
            },
            LoadConfiguration { } => {
              println!("LoadConfiguration");
            },
            #[allow(unused_variables)]
            UpdateConfiguration { key, value } => {
              println!("UpdateConfiguration");
            }
          }
          Ok(())
        }
      }
    })
    .build()
    .run();
}
