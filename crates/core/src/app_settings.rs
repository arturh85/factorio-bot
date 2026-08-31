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

/// Writes the data-local default into an unconfigured `workspace_path`.
///
/// The only workspace question settings **load** is allowed to ask. Whether a
/// configured path is *usable* -- absolute, and therefore not dependent on the
/// process's working directory -- is decided at the point of use, by
/// `paths::resolve_workspace`.
///
/// Asking it here instead is the regression this function exists to close.
///
/// **The set of points of use is NOT complete, and this comment used to claim
/// it was.** Four sites refuse a relative path: the scripts bootstrap in
/// `serve`, `manage::scripts::scripts_root_path`, `POST /api/v1/instance/start`
/// and `setup_factorio_instance`.
///
/// Two sites still pass the raw configured string to
/// `scripts::ensure_scripts_dir`, where it is joined against the process CWD:
///
/// - `app/src-tauri/src/scripting.rs:27` -- `PathBuf::from(app_settings
///   .factorio.workspace_path.to_string())`, no resolution
/// - `app/src-tauri/src/repl/run_script.rs:47` -- `Path::new(&workspace_path)`
///   from the same raw string
///
/// With a relative `workspace_path` those two read and write
/// `<cwd>/<relative>/scripts` while `start` refuses the same value -- the app
/// writes into a workspace it will not start in.
///
/// Re-measured after the tauri removal (plan 5 task 14), which shrank this
/// list rather than changing it: the seven call sites in
/// `gui/command/script.rs` went with that file, and `cli/lua.rs` no longer
/// reaches `ensure_scripts_dir` at all, so both are struck. `cli/serve.rs:78`
/// is not on the list because it resolves first, through
/// `paths::resolve_workspace`.
///
/// Do not read the list above as exhaustive either -- it was wrong once by
/// claiming completeness and once by naming a file that no longer existed. The
/// durable fix is to make an unresolved path unable to reach
/// `ensure_scripts_dir` at all, rather than to enumerate callers.
/// `Context::new` loads the settings before clap has even chosen a subcommand,
/// so a load that fails on a relative `workspace_path` fails `config show` and
/// `config init --force` too -- the two commands whose whole job is to report
/// and rewrite that setting. Every documented repair path went through the
/// thing being repaired.
///
/// Shared with `app/src-tauri`'s `load_app_settings_with` rather than restated
/// there: two copies of one rule is how the loaders would come to disagree.
pub fn fill_workspace_default(settings: &mut AppSettings) -> Result<()> {
    // Not `to_string_lossy`: a non-UTF-8 data-local directory would be renamed
    // by the replacement characters, and every caller would then run in a
    // directory that is not the one the default names. `?` rather than
    // `.into_diagnostic()` so the diagnostic's `help` reaches the user.
    let filled = paths::fill_workspace_default(&settings.factorio.workspace_path);
    settings.factorio.workspace_path = Cow::from(paths::workspace_to_string(filled)?);
    Ok(())
}

#[allow(clippy::module_name_repetitions)]
pub fn load_app_settings() -> Result<AppSettings> {
    let mut app_settings = AppSettings::load(paths::settings_file())?;
    fill_workspace_default(&mut app_settings)?;
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

    /// Settings load fills the default and asks nothing else. A relative
    /// `workspace_path` has to survive it verbatim: `Context::new` loads the
    /// settings before clap picks a subcommand, so a load that refuses one
    /// takes `config show` and `config init --force` down with it and leaves
    /// hand-editing TOML as the only way back.
    #[test]
    fn a_relative_workspace_path_loads_unchanged() {
        let mut settings = AppSettings::default();
        settings.factorio.workspace_path = Cow::from("relative-ws");
        fill_workspace_default(&mut settings).expect("a relative workspace_path must still load");
        assert_eq!(settings.factorio.workspace_path, "relative-ws");
    }

    /// The half that load *does* own: an unset workspace means the data-local
    /// one. Without this, "never touch workspace_path" would pass the test
    /// above and leave the empty default to be joined against whatever
    /// directory the process happened to start in.
    #[test]
    fn an_unset_workspace_path_is_filled_with_the_data_local_one() {
        let mut settings = AppSettings::default();
        assert!(settings.factorio.workspace_path.is_empty(), "the default");
        fill_workspace_default(&mut settings).expect("fills");
        assert_eq!(
            settings.factorio.workspace_path,
            paths::workspace_dir().to_string_lossy()
        );
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
