use crate::paths;
use factorio_bot_core::settings::{FactorioSettings, FACTORIO_SETTINGS_DEFAULT};
#[cfg(feature = "restapi")]
use factorio_bot_restapi::settings::{RestApiSettings, RESTAPI_SETTINGS_DEFAULT};
use miette::{IntoDiagnostic, Result};
use serde_json::Value;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

#[allow(clippy::module_name_repetitions)]
#[derive(
  Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize,
)]
pub struct GuiSettings {
  pub enable_autostart: bool,
  pub enable_restapi: bool,
}

#[allow(clippy::module_name_repetitions)]
#[derive(
  Debug, Clone, typescript_definitions::TypeScriptify, serde::Serialize, serde::Deserialize,
)]
pub struct AppSettings {
  pub factorio: FactorioSettings,
  #[cfg(feature = "restapi")]
  pub restapi: RestApiSettings,
  pub gui: GuiSettings,
}

#[allow(clippy::module_name_repetitions)]
pub type SharedAppSettings = Arc<RwLock<AppSettings>>;

#[allow(dead_code)]
pub const APP_SETTINGS_DEFAULT: AppSettings = AppSettings {
  factorio: FACTORIO_SETTINGS_DEFAULT,
  #[cfg(feature = "restapi")]
  restapi: RESTAPI_SETTINGS_DEFAULT,
  gui: GUI_SETTINGS_DEFAULT,
};

#[allow(dead_code)]
pub const GUI_SETTINGS_DEFAULT: GuiSettings = GuiSettings {
  enable_autostart: false,
  enable_restapi: false,
};

impl AppSettings {
  pub fn into_shared(self) -> SharedAppSettings {
    Arc::new(RwLock::new(self))
  }
  #[allow(dead_code)]
  pub fn load(file_path: PathBuf) -> Result<AppSettings> {
    if Path::exists(&file_path) {
      let file_contents = ::std::fs::read_to_string(file_path).into_diagnostic()?;
      let mut app_settings = serde_json::to_value(APP_SETTINGS_DEFAULT).into_diagnostic()?;
      let result: Value = ::toml::from_str(&file_contents).into_diagnostic()?;
      AppSettings::merge(&mut app_settings, &result);
      Ok(serde_json::from_value(app_settings).into_diagnostic()?)
    } else {
      Ok(APP_SETTINGS_DEFAULT)
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

#[allow(clippy::module_name_repetitions, unused)]
pub fn load_app_settings() -> Result<AppSettings> {
  let mut app_settings = AppSettings::load(paths::settings_file())?;
  if app_settings.factorio.workspace_path == "" {
    let s: String = paths::workspace_dir().to_str().unwrap().into();
    app_settings.factorio.workspace_path = Cow::from(s);
  }
  Ok(app_settings)
}
