use crate::cli::{settings_overrides, Subcommand, SubcommandCallback, SETTINGS_PRECEDENCE_HELP};
use crate::context::Context;
use crate::settings::{load_app_settings_with, SettingsOverrides};
use clap::{ArgAction, ArgMatches, Command};
use factorio_bot_core::app_settings::AppSettings;
use factorio_bot_core::miette::{miette, IntoDiagnostic, Result};
use std::path::PathBuf;

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "config"
  }
  fn build_command(&self) -> Command {
    Command::new("config")
      .about("Inspect and create the settings file")
      .after_help(SETTINGS_PRECEDENCE_HELP)
      .subcommand_required(true)
      .arg_required_else_help(true)
      .subcommand(
        Command::new("show")
          .about("Print the effective settings, after --settings and the CLI overrides")
          .after_help(SETTINGS_PRECEDENCE_HELP),
      )
      .subcommand(
        Command::new("init")
          .about("Write a settings file containing every key with its default value")
          .after_help(
            "The file is generated from the AppSettings struct itself, so it can never \
             drift out of shape with what the program reads. Any CLI override \
             (--workspace-path, --factorio-archive) is baked into the written file.\n\n\
             An empty workspace_path means \"<data dir>/workspace\".",
          )
          .arg(
            clap::Arg::new("force")
              .long("force")
              .short('f')
              .action(ArgAction::SetTrue)
              .help("overwrite the file if it already exists"),
          ),
      )
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

/// Serialises settings the same way `config show` prints them.
///
/// Shared with the tests so what is asserted is what is printed.
fn render(settings: &AppSettings) -> Result<String> {
  ::toml::to_string_pretty(settings).into_diagnostic()
}

/// Where `config init` writes to: the explicit `--settings` path if one was
/// given, otherwise the location the program reads by default.
fn target_path(overrides: &SettingsOverrides) -> PathBuf {
  overrides
    .settings_path
    .clone()
    .unwrap_or_else(factorio_bot_core::paths::settings_file)
}

/// The settings `config init` writes: the struct defaults with any CLI
/// override applied. Deliberately *not* seeded from an existing file, so
/// `init` always produces a complete, current template.
fn initial_settings(overrides: &SettingsOverrides) -> AppSettings {
  let mut settings = AppSettings::default();
  overrides.apply(&mut settings);
  settings
}

// Nothing here awaits, but `SubcommandCallback` is a future-returning fn
// pointer shared by every subcommand.
#[allow(clippy::unused_async)]
async fn run(matches: &ArgMatches, _context: &mut Context) -> Result<()> {
  let overrides = settings_overrides(matches);
  match matches.subcommand() {
    Some(("show", _)) => {
      let settings = load_app_settings_with(&overrides)?;
      print!("{}", render(&settings)?);
      Ok(())
    }
    Some(("init", init_matches)) => {
      let path = target_path(&overrides);
      if path.exists() && !init_matches.get_flag("force") {
        return Err(miette!(
          "{} already exists; pass --force to overwrite it",
          path.display()
        ));
      }
      if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).into_diagnostic()?;
      }
      AppSettings::save(path.clone(), &initial_settings(&overrides))?;
      println!("wrote {}", path.display());
      Ok(())
    }
    other => Err(miette!(
      "unknown config subcommand: {:?}",
      other.map(|o| o.0)
    )),
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cli::build_app;

  /// What `config init` writes must be loadable by `AppSettings::load`, with
  /// every value surviving the round trip. This is the guard against the class
  /// of bug where a hand-maintained template is flat while the struct is
  /// nested and every key silently merges into nothing.
  #[test]
  fn a_written_config_round_trips_through_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("AppSettings.toml");
    let overrides = SettingsOverrides {
      settings_path: Some(path.clone()),
      workspace_path: Some("/generated/workspace".to_owned()),
      factorio_archive_path: Some("/generated/factorio.tar.xz".to_owned()),
    };

    AppSettings::save(path.clone(), &initial_settings(&overrides)).expect("saves");
    let loaded = AppSettings::load(path).expect("loads what init wrote");

    // the CLI overrides were baked in...
    assert_eq!(loaded.factorio.workspace_path, "/generated/workspace");
    assert_eq!(
      loaded.factorio.factorio_archive_path,
      "/generated/factorio.tar.xz"
    );
    // ...and the untouched keys came back as themselves, not as blanks
    let defaults = AppSettings::default();
    assert_eq!(loaded.factorio.rcon_pass, defaults.factorio.rcon_pass);
    assert_eq!(loaded.factorio.rcon_port, defaults.factorio.rcon_port);
    assert_eq!(loaded.restapi.port, defaults.restapi.port);
  }

  /// The generated file must carry the struct's sections as tables. A flat
  /// file parses fine as TOML but merges into nothing.
  #[test]
  fn the_written_config_is_sectioned_like_the_struct() {
    let rendered = render(&AppSettings::default()).expect("renders");
    let parsed: ::toml::Value = ::toml::from_str(&rendered).expect("valid toml");
    let table = parsed.as_table().expect("a table");
    let mut sections: Vec<&str> = table.keys().map(String::as_str).collect();
    sections.sort_unstable();
    assert_eq!(sections, vec!["factorio", "gui", "restapi"]);
    for section in sections {
      assert!(table[section].is_table(), "[{section}] must be a table");
    }
  }

  /// `config show` must reflect the CLI overrides, not just the file. Rendering
  /// is asserted on the text, because that text is what the user reads.
  #[test]
  fn show_renders_the_overridden_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("AppSettings.toml");
    std::fs::write(&path, "[factorio]\nworkspace_path = \"/from/file\"\n").expect("writes");

    let settings = load_app_settings_with(&SettingsOverrides {
      settings_path: Some(path),
      workspace_path: Some("/from/cli".to_owned()),
      ..SettingsOverrides::default()
    })
    .expect("loads");
    let rendered = render(&settings).expect("renders");

    assert!(
      rendered.contains("workspace_path = \"/from/cli\""),
      "expected the CLI value in the output, got:\n{rendered}"
    );
    assert!(
      !rendered.contains("/from/file"),
      "the file value must not survive the override:\n{rendered}"
    );
  }

  /// `config init` defaults to writing the standard settings file, and honours
  /// `--settings` as its output path.
  #[test]
  fn init_writes_to_the_settings_path_when_one_is_given() {
    let matches = build_app()
      .try_get_matches_from([
        "factorio-bot",
        "--settings",
        "/tmp/custom.toml",
        "config",
        "init",
      ])
      .expect("parses");
    let sub = matches
      .subcommand_matches("config")
      .expect("config matched");
    assert_eq!(
      target_path(&settings_overrides(sub)),
      PathBuf::from("/tmp/custom.toml")
    );

    let matches = build_app()
      .try_get_matches_from(["factorio-bot", "config", "init"])
      .expect("parses");
    let sub = matches
      .subcommand_matches("config")
      .expect("config matched");
    assert_eq!(
      target_path(&settings_overrides(sub)),
      factorio_bot_core::paths::settings_file()
    );
  }

  /// The global settings options must reach a subcommand's matches whether they
  /// are written before or after the subcommand name.
  #[test]
  fn global_settings_options_are_visible_from_either_position() {
    let expected = SettingsOverrides {
      settings_path: Some(PathBuf::from("/s.toml")),
      workspace_path: Some("/w".to_owned()),
      factorio_archive_path: Some("/a.tar.xz".to_owned()),
    };
    for argv in [
      vec![
        "factorio-bot",
        "--settings",
        "/s.toml",
        "--workspace-path",
        "/w",
        "--factorio-archive",
        "/a.tar.xz",
        "config",
        "show",
      ],
      vec![
        "factorio-bot",
        "config",
        "show",
        "--settings",
        "/s.toml",
        "--workspace-path",
        "/w",
        "--factorio-archive",
        "/a.tar.xz",
      ],
    ] {
      let matches = build_app()
        .try_get_matches_from(&argv)
        .unwrap_or_else(|e| panic!("parses {argv:?}: {e}"));
      let sub = matches
        .subcommand_matches("config")
        .expect("config matched");
      assert_eq!(settings_overrides(sub), expected, "for {argv:?}");
    }
  }
}
