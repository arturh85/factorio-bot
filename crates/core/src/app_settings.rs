use crate::paths;
use crate::settings::{FactorioSettings, RestApiSettings};
use miette::{IntoDiagnostic, Result};
use serde_json::Value;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

#[allow(clippy::module_name_repetitions)]
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct GuiSettings {
    pub enable_autostart: bool,
    pub enable_restapi: bool,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub factorio: FactorioSettings,
    pub restapi: RestApiSettings,
    pub gui: GuiSettings,
}

#[allow(clippy::module_name_repetitions)]
pub type SharedAppSettings = Arc<RwLock<AppSettings>>;

impl AppSettings {
    pub fn into_shared(self) -> SharedAppSettings {
        Arc::new(RwLock::new(self))
    }
    pub fn load(file_path: PathBuf) -> Result<AppSettings> {
        if Path::exists(&file_path) {
            let file_contents = ::std::fs::read_to_string(file_path).into_diagnostic()?;
            let mut app_settings =
                serde_json::to_value(AppSettings::default()).into_diagnostic()?;
            let result: Value = ::toml::from_str(&file_contents).into_diagnostic()?;
            AppSettings::merge(&mut app_settings, &result);
            Ok(serde_json::from_value(app_settings).into_diagnostic()?)
        } else {
            Ok(AppSettings::default())
        }
    }

    #[allow(dead_code)]
    pub fn save(file_path: PathBuf, app_settings: &AppSettings) -> Result<()> {
        let file_contents = ::toml::to_string(app_settings).into_diagnostic()?;
        ::std::fs::write(file_path, file_contents).into_diagnostic()?;
        Ok(())
    }

    fn merge(a: &mut Value, b: &Value) {
        match (a, b) {
            (&mut Value::Object(ref mut a), Value::Object(b)) => {
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

#[allow(clippy::module_name_repetitions)]
pub fn load_app_settings() -> Result<AppSettings> {
    let mut app_settings = AppSettings::load(paths::settings_file())?;
    if app_settings.factorio.workspace_path.is_empty() {
        let s: String = paths::workspace_dir().to_str().unwrap().into();
        app_settings.factorio.workspace_path = Cow::from(s);
    }
    Ok(app_settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_serialisable_and_round_trip() {
        let settings = AppSettings::default();
        let json = serde_json::to_value(&settings).expect("serialises");
        let restored: AppSettings = serde_json::from_value(json).expect("deserialises");
        assert_eq!(restored.restapi.port, settings.restapi.port);
        assert_eq!(restored.gui.enable_autostart, settings.gui.enable_autostart);
    }

    /// The restapi section is no longer feature-gated, so it is present in the
    /// serialised form regardless of how the binary was built.
    #[test]
    fn restapi_section_is_always_present() {
        let json = serde_json::to_value(AppSettings::default()).expect("serialises");
        assert!(
            json.get("restapi").is_some(),
            "expected a restapi section, got: {json}"
        );
    }
}
