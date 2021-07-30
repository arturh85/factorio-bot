#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]
pub mod app_settings;

use app_settings::types::{AppSettings, APP_SETTINGS_DEFAULT};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::Manager;

fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (&mut Value::Object(ref mut a), &Value::Object(ref b)) => {
            for (k, v) in b {
                merge(a.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (a, b) => {
            *a = b.clone();
        }
    }
}

#[tauri::command]
async fn my_custom_command(app_handle: tauri::AppHandle<tauri::Wry>) {
    println!("I was invoked from JS!");
    async_std::task::sleep(Duration::from_secs(1)).await;
    println!("I was invoked after 1000ms!");
    app_handle.emit_all("the_event", "foo").unwrap();
}

// settings: State<'_, AppSettings>
#[tauri::command]
async fn load_config() -> AppSettings {
    // app_settings.inner().clone()
    load_app_settings().expect("aa")
}
#[tauri::command]
async fn save_config(_app_handle: tauri::AppHandle<tauri::Wry>) {
    save_app_settings(load_app_settings().expect("config failed to load"))
        .expect("config failed to save");
}

const APP_SETTINGS_FILENAME: &str = "AppSettings.toml";

pub fn default_app_dir() -> PathBuf {
    tauri::api::path::local_data_dir()
        .expect("no local data directory available")
        .join(env!("CARGO_PKG_NAME"))
}

pub fn app_settings_path() -> PathBuf {
    default_app_dir().join(APP_SETTINGS_FILENAME)
}

pub fn load_app_settings() -> anyhow::Result<AppSettings> {
    let file_path = app_settings_path();
    if Path::exists(&file_path) {
        let file_contents = ::std::fs::read_to_string(file_path)?;
        let mut app_settings = serde_json::to_value(APP_SETTINGS_DEFAULT)?;
        let result: Value = ::toml::from_str(&file_contents)?;
        merge(&mut app_settings, &result);
        Ok(serde_json::from_value(app_settings)?)
    } else {
        Ok(APP_SETTINGS_DEFAULT)
    }
}

pub fn save_app_settings(app_settings: AppSettings) -> anyhow::Result<()> {
    let file_contents = ::toml::to_string(&app_settings)?;
    let filepath = app_settings_path();
    ::std::fs::write(filepath, file_contents)?;
    Ok(())
}

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    std::fs::create_dir_all(default_app_dir())?;
    let settings = load_app_settings()?;

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
            load_config,
            save_config
        ])
        .manage(settings)
        .run(tauri::generate_context!())
        .expect("failed to run app");
    Ok(())
}
