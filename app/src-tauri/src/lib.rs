#![warn(clippy::all, clippy::pedantic)]
// Removed because of CLI/REPL features
// // Remove console window opening on windows
#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "console"
)]
#[cfg(feature = "cli")]
mod cli;
mod context;
mod paths;
#[cfg(feature = "repl")]
mod repl;
#[cfg(feature = "lua")]
mod scripting;
mod settings;

use context::Context;
use settings::SettingsOverrides;
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const APP_ABOUT: &str = env!("CARGO_PKG_DESCRIPTION");

/// The `--settings` (and its sibling overrides) pulled out of the real
/// process argv, *before* `cli::start` does its own full parse and dispatch.
///
/// This has to happen ahead of `Context::new` below, which is what actually
/// loads settings, and `Context::new` has to happen ahead of `cli::start`
/// picking a subcommand -- `config show` and every other subcommand alike
/// read `context.app_settings`. Parsing here again means `cli::start` parses
/// the same argv a second time, which is harmless: clap parsing has no side
/// effects, and a `--settings`/`--help`/malformed-argv error surfaces here
/// exactly as it would have there, just slightly earlier.
///
/// Without the `cli` feature there is no argv to parse -- nothing else in
/// the binary reads `--settings` -- so this is just the no-override default.
#[cfg(feature = "cli")]
fn settings_overrides_from_env() -> SettingsOverrides {
  match cli::build_app().try_get_matches_from(std::env::args_os()) {
    Ok(matches) => cli::settings_overrides(&matches),
    // `.exit()` prints the usage/help/error clap would have shown and ends
    // the process -- the same outcome `cli::start`'s own `get_matches()`
    // would have produced, just before `Context::new` rather than after.
    Err(err) => err.exit(),
  }
}

#[cfg(not(feature = "cli"))]
fn settings_overrides_from_env() -> SettingsOverrides {
  SettingsOverrides::default()
}

#[allow(clippy::missing_panics_doc)]
pub fn run() {
  let overrides = settings_overrides_from_env();

  // Same treatment as `cli::start` below, and for the same reason: everything
  // `Context::new` can fail at is operational -- an unwritable data directory,
  // an `AppSettings.toml` that is not valid TOML. `.expect` turned those into
  // "The application panicked (crashed)" with a backtrace hint, and under
  // `panic = "abort"` into an abort, which is not a failure mode a
  // configuration mistake deserves.
  //
  // This runs *before* clap has chosen a subcommand, so it is also the widest
  // blast radius in the binary: whatever fails here fails `config show` and
  // `config init --force` too, i.e. the commands a user reaches for to repair
  // the configuration. That is why the workspace refusal was moved out of
  // settings load and down to the point of use -- see
  // `factorio_bot_core::app_settings::fill_workspace_default`.
  //
  // `overrides` -- pulled out of argv above -- is what makes this honour
  // `--settings` at all: previously `Context::new()` took nothing and always
  // loaded the data-dir default, so `--settings x.toml serve` read (and its
  // `PUT /api/v1/settings` route overwrote) the wrong file while reporting
  // success.
  let context = match Context::new(&overrides) {
    Ok(context) => context,
    Err(report) => {
      eprintln!("Error: {report:?}");
      std::process::exit(1);
    }
  };

  #[cfg(feature = "cli")]
  {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    // Was `.expect("failed to start cli")`, which turned every operational
    // failure a subcommand can hit -- a missing factorio archive, a mod that
    // fails to load, a port already in use -- into "The application panicked
    // (crashed)" with a backtrace hint. Those are expected conditions with
    // one-line, user-fixable causes; only genuine invariant violations should
    // still panic. `{:?}` on a miette `Report` is its rendered diagnostic.
    let app = match rt.block_on(async { cli::start(context.clone()).await }) {
      Ok(app) => app,
      Err(report) => {
        eprintln!("Error: {report:?}");
        std::process::exit(1);
      }
    };

    // If no subcommand was run, app is Some and we should show help (unless REPL will start)
    #[cfg(not(feature = "repl"))]
    {
      if app.is_none() {
        return;
      }
      app
        .expect("checked before")
        .print_help()
        .expect("failed to print_help");
      return;
    }
    #[cfg(feature = "repl")]
    {
      if let Some(mut app) = app {
        // No subcommand was run, show help and let the REPL take over
        app.print_help().expect("failed to print_help");
      }
      // If app is None, a subcommand ran - continue to the REPL
    }
  }
  #[cfg(feature = "repl")]
  {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async { repl::start(context.clone()).await })
      .expect("repl failed");
  }
  #[cfg(all(not(feature = "cli"), not(feature = "repl")))]
  {
    panic!("select at least one feature of cli or repl");
  }
}
