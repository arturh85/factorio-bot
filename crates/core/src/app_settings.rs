use crate::paths;
use crate::settings::{FactorioSettings, RestApiSettings};
use miette::{IntoDiagnostic, Result};
use serde_json::Value;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

#[allow(clippy::module_name_repetitions)]
#[derive(Default, Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GuiSettings {
    pub enable_autostart: bool,
    pub enable_restapi: bool,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Default, Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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
    // One resolution rule, in `paths::resolve_workspace`. This used to inline
    // it -- empty means the data-local workspace -- and four copies of that
    // inline had drifted apart, which is how a start could resolve a workspace
    // differently from the route that lists its scripts.
    //
    // `to_str().unwrap()` also went with it: a non-UTF-8 data-local directory
    // panicked here, and with `panic = "abort"` in release that is a crash at
    // load rather than an error.
    let resolved = paths::resolve_workspace(&app_settings.factorio.workspace_path)
        .into_diagnostic()?;
    app_settings.factorio.workspace_path = Cow::from(resolved.to_string_lossy().into_owned());
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

    /// A binary built without the `restapi` feature wrote an `AppSettings.toml`
    /// with no `[restapi]` section at all. Such a file must still load: the
    /// values it does carry survive, and every section it omits falls back to
    /// its default. This is the backward-compatibility claim the settings move
    /// rests on.
    #[test]
    fn loads_a_toml_written_without_a_restapi_section() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("AppSettings.toml");
        std::fs::write(
            &file_path,
            r#"
[factorio]
client_count = 4
rcon_pass = "hunter2"
rcon_port = 4444
workspace_path = "/tmp/some-workspace"
recreate = true
"#,
        )
        .expect("writes the settings file");

        let settings = AppSettings::load(file_path).expect("loads a partial settings file");

        // the values the old file carried survive
        assert_eq!(settings.factorio.client_count, 4);
        assert_eq!(settings.factorio.rcon_pass, "hunter2");
        assert_eq!(settings.factorio.rcon_port, 4444);
        assert_eq!(settings.factorio.workspace_path, "/tmp/some-workspace");
        assert!(settings.factorio.recreate);
        // factorio keys the file did not mention keep their defaults
        assert_eq!(
            settings.factorio.seed,
            FactorioSettings::default().seed,
            "unmentioned keys must not be blanked"
        );

        // the whole missing section comes back as its default
        assert_eq!(settings.restapi.port, 7492);
        assert_eq!(settings.restapi.web_root, None);
        assert!(!settings.gui.enable_autostart);
        assert!(!settings.gui.enable_restapi);
    }

    /// A file that does not exist is not an error; the defaults stand in.
    #[test]
    fn load_falls_back_to_defaults_when_the_file_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = AppSettings::load(dir.path().join("missing.toml"))
            .expect("absent file is not an error");
        assert_eq!(settings.restapi.port, 7492);
        assert_eq!(
            settings.factorio.client_count,
            FactorioSettings::default().client_count
        );
    }

    /// `save` then `load` must be lossless, so the GUI's settings page and the
    /// server read the same values.
    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("AppSettings.toml");
        let mut settings = AppSettings::default();
        settings.factorio.client_count = 7;
        settings.restapi.port = 8123;
        settings.restapi.web_root = Some("/srv/www".to_owned());
        settings.gui.enable_autostart = true;

        AppSettings::save(file_path.clone(), &settings).expect("saves");
        let restored = AppSettings::load(file_path).expect("loads");

        assert_eq!(restored.factorio.client_count, 7);
        assert_eq!(restored.restapi.port, 8123);
        assert_eq!(restored.restapi.web_root, Some("/srv/www".to_owned()));
        assert!(restored.gui.enable_autostart);
    }

    /// The template shipped in `src/data/AppSettings.toml` is what a user copies
    /// into the data directory. It used to be flat while `AppSettings` is
    /// nested, so every key in it merged into nothing: a `factorio_archive_path`
    /// set there stayed empty and the run failed with "no factorio archive
    /// configured".
    const SHIPPED_TEMPLATE: &str = include_str!("data/AppSettings.toml");

    #[test]
    fn the_shipped_template_is_sectioned_like_the_struct() {
        let parsed: toml::Value = toml::from_str(SHIPPED_TEMPLATE).expect("template is valid toml");
        let table = parsed.as_table().expect("template is a table");
        let sections: Vec<&str> = table.keys().map(String::as_str).collect();
        assert_eq!(
            sections,
            vec!["factorio", "gui", "restapi"],
            "the template must only contain the struct's sections"
        );
        for section in sections {
            assert!(
                table[section].is_table(),
                "[{section}] must be a table of settings"
            );
        }
    }

    /// Structure alone is not enough: the values have to arrive in the struct.
    /// Every key here is edited away from its default first, because a template
    /// whose values happen to equal the defaults cannot tell a working merge
    /// from a broken one.
    #[test]
    fn values_edited_in_the_shipped_template_reach_the_struct() {
        let edited = SHIPPED_TEMPLATE
            .replace(
                r#"factorio_archive_path = """#,
                r#"factorio_archive_path = "/opt/factorio.tar.xz""#,
            )
            .replace("client_count = 2", "client_count = 5")
            .replace("port = 7492", "port = 9001")
            .replace("enable_autostart = false", "enable_autostart = true");
        assert_ne!(edited, SHIPPED_TEMPLATE, "the edits must apply");

        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("AppSettings.toml");
        std::fs::write(&file_path, edited).expect("writes the settings file");
        let settings = AppSettings::load(file_path).expect("loads the template");

        assert_eq!(
            settings.factorio.factorio_archive_path,
            "/opt/factorio.tar.xz"
        );
        assert_eq!(settings.factorio.client_count, 5);
        assert_eq!(settings.restapi.port, 9001);
        assert!(settings.gui.enable_autostart);
        // and an untouched key still carries the template's own value
        assert_eq!(settings.factorio.rcon_pass, "foobar");
    }
}
