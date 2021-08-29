use crate::settings::{AppSettings, APP_SETTINGS_DEFAULT};
use miette::{DiagnosticResult, IntoDiagnostic};
use serde_json::Value;
use std::path::{Path, PathBuf};

impl AppSettings {
    pub fn load(file_path: PathBuf) -> DiagnosticResult<AppSettings> {
        if Path::exists(&file_path) {
            let file_contents = ::std::fs::read_to_string(file_path)
                .into_diagnostic("factorio::output_parser::could_not_read_to_string")?;
            let mut app_settings = serde_json::to_value(APP_SETTINGS_DEFAULT)
                .into_diagnostic("factorio::output_parser::could_not_create_value")?;
            let result: Value = ::toml::from_str(&file_contents)
                .into_diagnostic("factorio::output_parser::could_not_parse_toml")?;
            AppSettings::merge(&mut app_settings, &result);
            Ok(serde_json::from_value(app_settings)
                .into_diagnostic("factorio::output_parser::could_not_create_value")?)
        } else {
            Ok(APP_SETTINGS_DEFAULT)
        }
    }

    pub fn save(file_path: PathBuf, app_settings: &AppSettings) -> DiagnosticResult<()> {
        let file_contents = ::toml::to_string(app_settings)
            .into_diagnostic("factorio::output_parser::could_not_parse_json")?;
        ::std::fs::write(file_path, file_contents)
            .into_diagnostic("factorio::output_parser::could_not_parse_json")?;
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
