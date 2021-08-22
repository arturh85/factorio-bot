use crate::settings::{AppSettings, APP_SETTINGS_DEFAULT};
use serde_json::Value;
use std::path::{Path, PathBuf};

impl AppSettings {
    pub fn load(file_path: PathBuf) -> anyhow::Result<AppSettings> {
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

    pub fn save(file_path: PathBuf, app_settings: &AppSettings) -> anyhow::Result<()> {
        let file_contents = ::toml::to_string(app_settings)?;
        ::std::fs::write(file_path, file_contents)?;
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
