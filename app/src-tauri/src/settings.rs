//! Re-exported from `factorio_bot_core::app_settings` so the existing
//! `crate::settings::` call sites keep compiling.
//!
//! `load_app_settings` is deliberately **not** re-exported here any more. Its
//! only remaining caller is the REPL's command builder, which runs before a
//! `Context` exists and so has no resolved settings to consult; it names the
//! core path itself. Re-exporting it from the crate every module already
//! imports made "just load the settings again" the easy thing to reach for,
//! which is how a `--settings` override came to be honoured by the server and
//! ignored by script resolution (see `crate::scripting::run_script_file`).
//! Without the `repl` feature nothing used it at all, and the unused re-export
//! warned.
pub use factorio_bot_core::app_settings::SharedAppSettings;

use factorio_bot_core::app_settings::{AppSettings as CoreAppSettings, fill_workspace_default};
use factorio_bot_core::miette::{Result, miette};
use factorio_bot_core::paths;
use std::borrow::Cow;
use std::path::PathBuf;

/// Command line overrides for values that otherwise only come from
/// `AppSettings.toml`.
///
/// Precedence is **CLI > settings file > built-in defaults**: every field that
/// is `Some` here replaces whatever the settings file said, and the settings
/// file in turn replaces the `Default` impl.
// The `_path` suffix on every field is deliberate: each name mirrors the CLI
// flag and the `AppSettings` key it overrides, and shortening them here would
// break that correspondence.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SettingsOverrides {
  /// Read the settings from this file instead of
  /// `<data dir>/AppSettings.toml`. Unlike the default location, an explicitly
  /// named file that does not exist is an error rather than a silent fallback
  /// to defaults -- a typo'd `--settings` path that quietly ran with defaults
  /// is exactly the kind of failure this flag exists to prevent.
  pub settings_path: Option<PathBuf>,
  pub workspace_path: Option<String>,
  pub factorio_archive_path: Option<String>,
}

impl SettingsOverrides {
  /// Applies the overrides on top of an already-loaded settings value.
  ///
  /// Split out from [`load_app_settings_with`] so the precedence rule can be
  /// tested without touching the user's real data directory.
  pub fn apply(&self, settings: &mut CoreAppSettings) {
    if let Some(workspace_path) = &self.workspace_path {
      settings.factorio.workspace_path = Cow::Owned(workspace_path.clone());
    }
    if let Some(archive_path) = &self.factorio_archive_path {
      settings.factorio.factorio_archive_path = Cow::Owned(archive_path.clone());
    }
  }
}

/// The settings file a command should use: the explicit `--settings` path if
/// one was given, otherwise the default location the program reads.
///
/// The single copy of this rule -- `Context::new` (read side) and
/// `cli::config::target_path` (`config init`'s write side) both call this
/// rather than restating it. Four copies of a settings rule have already
/// drifted apart in this repo once.
pub fn resolved_settings_path(overrides: &SettingsOverrides) -> PathBuf {
  overrides
    .settings_path
    .clone()
    .unwrap_or_else(paths::settings_file)
}

/// Loads the effective settings: the file (either `--settings` or the default
/// location) with the CLI overrides applied on top.
pub fn load_app_settings_with(overrides: &SettingsOverrides) -> Result<CoreAppSettings> {
  let mut settings = match &overrides.settings_path {
    Some(path) => {
      if !path.exists() {
        return Err(miette!("settings file not found: {}", path.display()));
      }
      CoreAppSettings::load(path.clone())?
    }
    None => CoreAppSettings::load(paths::settings_file())?,
  };
  overrides.apply(&mut settings);
  // Applied after the override so an explicit `--workspace-path` is never
  // second-guessed, and shared with `load_app_settings` rather than restated:
  // four copies of this rule had drifted apart, and the start route ended up
  // resolving a workspace differently from the route listing its scripts.
  //
  // Filling the default is *all* this does. Whether the resulting path is
  // usable is decided at the point of use (`paths::resolve_workspace`), never
  // here: `config show` and `config init --force` both load before they can
  // report or rewrite a bad `workspace_path`, so a load that refuses one
  // refuses its own repair.
  fill_workspace_default(&mut settings)?;
  Ok(settings)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn write_settings(dir: &std::path::Path, contents: &str) -> PathBuf {
    let file_path = dir.join("AppSettings.toml");
    std::fs::write(&file_path, contents).expect("writes the settings file");
    file_path
  }

  /// CLI beats the settings file. Both keys are set in the file *and* on the
  /// command line, with different values, so a test that passed by accident
  /// (override ignored, file value happening to match) is impossible.
  #[test]
  fn cli_overrides_beat_the_settings_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = write_settings(
      dir.path(),
      r#"
[factorio]
workspace_path = "/from/file/workspace"
factorio_archive_path = "/from/file/archive.tar.xz"
"#,
    );

    let overrides = SettingsOverrides {
      settings_path: Some(file_path),
      workspace_path: Some("/from/cli/workspace".to_owned()),
      factorio_archive_path: Some("/from/cli/archive.tar.xz".to_owned()),
    };
    let settings = load_app_settings_with(&overrides).expect("loads");

    assert_eq!(settings.factorio.workspace_path, "/from/cli/workspace");
    assert_eq!(
      settings.factorio.factorio_archive_path,
      "/from/cli/archive.tar.xz"
    );
  }

  /// ...and the settings file beats the defaults, for keys the CLI did not
  /// mention. Without this half, "CLI wins" could be satisfied by ignoring the
  /// file entirely.
  #[test]
  fn the_settings_file_beats_the_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = write_settings(
      dir.path(),
      r#"
[factorio]
workspace_path = "/from/file/workspace"
factorio_archive_path = "/from/file/archive.tar.xz"
rcon_port = 4444
"#,
    );

    let overrides = SettingsOverrides {
      settings_path: Some(file_path),
      ..SettingsOverrides::default()
    };
    let settings = load_app_settings_with(&overrides).expect("loads");

    assert_eq!(settings.factorio.workspace_path, "/from/file/workspace");
    assert_eq!(
      settings.factorio.factorio_archive_path,
      "/from/file/archive.tar.xz"
    );
    assert_eq!(settings.factorio.rcon_port, 4444);
    // a key neither the CLI nor the file mentioned keeps its default
    assert_eq!(settings.factorio.rcon_pass, "foobar");
  }

  /// A `--settings` path that does not exist is a clean error, not a silent
  /// run against the built-in defaults.
  #[test]
  fn a_missing_explicit_settings_file_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let overrides = SettingsOverrides {
      settings_path: Some(dir.path().join("nope.toml")),
      ..SettingsOverrides::default()
    };
    let report = load_app_settings_with(&overrides).expect_err("missing file must fail");
    assert!(
      report.to_string().contains("settings file not found"),
      "unexpected error: {report}"
    );
  }

  /// An empty `workspace_path` still falls back to the data directory, and an
  /// explicit `--workspace-path` is never replaced by that fallback.
  #[test]
  fn workspace_path_falls_back_only_when_nothing_set_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = write_settings(dir.path(), "[factorio]\nworkspace_path = \"\"\n");

    let defaulted = load_app_settings_with(&SettingsOverrides {
      settings_path: Some(file_path.clone()),
      ..SettingsOverrides::default()
    })
    .expect("loads");
    assert_eq!(
      defaulted.factorio.workspace_path,
      paths::workspace_dir().to_string_lossy()
    );

    let overridden = load_app_settings_with(&SettingsOverrides {
      settings_path: Some(file_path),
      workspace_path: Some("/explicit".to_owned()),
      ..SettingsOverrides::default()
    })
    .expect("loads");
    assert_eq!(overridden.factorio.workspace_path, "/explicit");
  }
}
