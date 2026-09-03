mod config;
#[cfg(feature = "lua")]
mod lua;
mod plan;
#[cfg(debug_assertions)]
mod playground;
mod rcon;
#[cfg(feature = "repl")]
mod repl;
#[cfg(feature = "lua")]
mod roll_seed;
mod score_map;
#[cfg(feature = "restapi")]
mod serve;
mod start;

use crate::context::Context;
use crate::settings::SettingsOverrides;
use crate::{APP_ABOUT, APP_AUTHOR, APP_NAME};
use clap::{Arg, ArgMatches, Command, value_parser};
use clap_complete::{Generator, Shell, generate};
use factorio_bot_core::miette::{Result, miette};
use factorio_bot_core::record::savepoint::{ResumeMarker, list_savepoints, plan_resume};
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;

/// Explains, once, where a setting can come from. Shown under `--help` on the
/// top-level command and on every subcommand that reads settings.
pub const SETTINGS_PRECEDENCE_HELP: &str = "\
Settings precedence (highest wins):
  1. command line options (--workspace-path, --factorio-archive, ...)
  2. the settings file (--settings <path>, default: <data dir>/AppSettings.toml)
  3. built-in defaults

Run `factorio-bot config show` to print the effective merged settings, or
`factorio-bot config init` to write a settings file you can edit.";

/// The settings options every subcommand accepts. Declared once on the root
/// command as `global(true)`, so `factorio-bot --settings x lua y` and
/// `factorio-bot lua y --settings x` are the same thing.
fn settings_args() -> Vec<Arg> {
  vec![
    Arg::new("settings")
      .long("settings")
      .value_name("path")
      .global(true)
      .value_parser(value_parser!(PathBuf))
      .help("read settings from this file instead of <data dir>/AppSettings.toml"),
    Arg::new("workspace-path")
      .long("workspace-path")
      .value_name("path")
      .global(true)
      .value_parser(value_parser!(String))
      .help("override factorio.workspace_path from the settings file"),
    Arg::new("factorio-archive")
      .long("factorio-archive")
      .value_name("path")
      .global(true)
      .value_parser(value_parser!(String))
      .help("override factorio.factorio_archive_path from the settings file"),
  ]
}

/// Reads the global settings options out of any subcommand's matches.
///
/// `global(true)` args are copied into the subcommand's `ArgMatches` by clap,
/// so subcommands never need to see the root matches.
pub fn settings_overrides(matches: &ArgMatches) -> SettingsOverrides {
  SettingsOverrides {
    settings_path: matches.get_one::<PathBuf>("settings").cloned(),
    workspace_path: matches.get_one::<String>("workspace-path").cloned(),
    factorio_archive_path: matches.get_one::<String>("factorio-archive").cloned(),
  }
}

/// Resolves when the user asks this process to stop.
///
/// Ctrl-C raises SIGINT, but orchestrators, systemd and `docker stop` all send
/// SIGTERM instead. Waiting on `ctrl_c()` alone would let SIGTERM end the
/// process without ever running a shutdown path, which for the commands that
/// own a `FactorioInstance` means leaving the Factorio children behind --
/// `InteractiveProcess` has no `Drop`, so nothing else would kill them.
///
/// `signal::unix` is Unix-only, so non-Unix targets fall back to plain
/// `ctrl_c()`.
pub async fn shutdown_signal() {
  #[cfg(unix)]
  {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
      .expect("failed to install SIGTERM handler");
    tokio::select! {
      _ = tokio::signal::ctrl_c() => {}
      _ = sigterm.recv() => {}
    }
  }
  #[cfg(not(unix))]
  {
    let _ = tokio::signal::ctrl_c().await;
  }
}

/// The two `--resume-from` flags, shared by `lua` and `start`.
///
/// Both commands boot a server, and the reason to resume differs -- `lua` to
/// skip a prelude a run has already paid for, `start` to *look* at the world a
/// milestone reached -- but the flags and their hazards are identical, and a
/// second copy of them is a second place for the mod-mismatch refusal to be
/// forgotten.
pub fn resume_args(command: Command) -> Command {
  command
    .arg(
      Arg::new("resume-from")
        .long("resume-from")
        .value_name("RUN[:MILESTONE] | PATH")
        .required(false)
        .value_parser(value_parser!(String))
        .help("start from a milestone savepoint instead of the workspace map"),
    )
    .arg(
      Arg::new("resume-force")
        .long("resume-force")
        .action(clap::ArgAction::SetTrue)
        .requires("resume-from")
        .help("resume even though the savepoint was written by different mod code"),
    )
}

/// Resolves `--resume-from`, and when it cannot, says what *is* available.
///
/// A refusal that only says "no such savepoint" leaves the reader to go and
/// list a directory by hand, and the answer is one call away.
///
/// Returns `None` when the flag was not given, which is also what makes the
/// server start *clear* the resume marker -- a fresh run must never inherit
/// the previous one's answer to "which world is this".
pub fn resolve_resume(
  matches: &ArgMatches,
  workspace: &std::path::Path,
) -> Result<Option<ResumeMarker>> {
  let Some(reference) = matches.get_one::<String>("resume-from") else {
    return Ok(None);
  };
  let marker =
    plan_resume(workspace, reference, matches.get_flag("resume-force")).map_err(|err| {
      let runs_root = workspace.join("runs");
      let available: Vec<String> = list_savepoints(&runs_root)
        .into_iter()
        .map(|found| {
          format!(
            "  {}:{}  (tick {}, {} bytes)",
            found.savepoint.run_id,
            found.savepoint.milestone_index,
            found.savepoint.tick,
            found.savepoint.bytes
          )
        })
        .collect();
      if available.is_empty() {
        miette!("{err}\nNo savepoints exist under {runs_root:?} yet.")
      } else {
        miette!("{err}\nAvailable savepoints:\n{}", available.join("\n"))
      }
    })?;
  Ok(Some(marker))
}

pub fn subcommands() -> Vec<Box<dyn Subcommand>> {
  vec![
    config::build(),
    #[cfg(feature = "lua")]
    lua::build(),
    rcon::build(),
    #[cfg(feature = "lua")]
    roll_seed::build(),
    #[cfg(debug_assertions)]
    playground::build(),
    plan::build(),
    score_map::build(),
    #[cfg(feature = "repl")]
    repl::build(),
    #[cfg(feature = "restapi")]
    serve::build(),
    start::build(),
  ]
}

/// Assembles the full clap `Command`. Split out of [`start`] so tests can
/// parse argument vectors without running anything.
pub fn build_app() -> Command {
  let mut app = Command::new(APP_NAME)
    .version(env!("CARGO_PKG_VERSION"))
    .author(APP_AUTHOR)
    .about(APP_ABOUT)
    .after_help(SETTINGS_PRECEDENCE_HELP)
    .arg(
      Arg::new("shell")
        .help("generate shell-completion script")
        .long("generate")
        .value_parser(value_parser!(Shell)),
    )
    .args(settings_args());
  for subcommand in &subcommands() {
    app = app.subcommand(subcommand.build_command());
  }
  app
}

pub async fn start(mut context: Context) -> Result<Option<Command>> {
  let mut app = build_app();
  let subcommands = subcommands();
  let matches = app.clone().get_matches();
  if let Some(shell) = matches.get_one::<Shell>("shell").copied() {
    eprintln!("Generating completion file for {shell}...");
    print_completions(shell, &mut app);
    return Ok(None);
  }
  for subcommand in &subcommands {
    if let Some(matches) = matches.subcommand_matches(subcommand.name()) {
      let callback = subcommand.build_callback();
      callback(matches, &mut context).await?;
      return Ok(None);
    }
  }
  Ok(Some(app))
}

fn print_completions<G: Generator>(r#gen: G, cmd: &mut Command) {
  generate(r#gen, cmd, cmd.get_name().to_owned(), &mut io::stdout());
}

pub trait Subcommand {
  fn name(&self) -> &str;
  fn build_command(&self) -> Command;
  fn build_callback(&self) -> SubcommandCallback;
}

pub type SubcommandCallback =
  for<'a> fn(&'a ArgMatches, &'a mut Context) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;
