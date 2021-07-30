use crate::app_settings::{AppSettings, APP_SETTINGS_DEFAULT};
use crate::constants::app_settings_path;
use serde_json::Value;
use std::path::Path;

impl AppSettings {
  pub fn load() -> anyhow::Result<AppSettings> {
    let file_path = app_settings_path();
    if Path::exists(&file_path) {
      let file_contents = ::std::fs::read_to_string(file_path)?;
      let mut app_settings = serde_json::to_value(APP_SETTINGS_DEFAULT)?;
      let result: Value = ::toml::from_str(&file_contents)?;
      AppSettings::merge(&mut app_settings, &result);
      Ok(serde_json::from_value(app_settings)?)
    } else {
      Ok(APP_SETTINGS_DEFAULT)
    }
  }

  pub fn save(app_settings: AppSettings) -> anyhow::Result<()> {
    let file_contents = ::toml::to_string(&app_settings)?;
    let filepath = app_settings_path();
    ::std::fs::write(filepath, file_contents)?;
    Ok(())
  }

  fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
      (&mut Value::Object(ref mut a), &Value::Object(ref b)) => {
        for (k, v) in b {
          AppSettings::merge(a.entry(k.clone()).or_insert(Value::Null), v);
        }
      }
      (a, b) => {
        *a = b.clone();
      }
    }
  }
}
