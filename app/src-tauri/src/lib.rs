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
#[cfg(feature = "gui")]
mod gui;
mod paths;
#[cfg(feature = "repl")]
mod repl;
#[cfg(feature = "lua")]
mod scripting;
mod settings;

use context::Context;
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const APP_ABOUT: &str = env!("CARGO_PKG_DESCRIPTION");

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::missing_panics_doc)]
pub fn run() {
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
  let context = match Context::new() {
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

    // If no subcommand was run, app is Some and we should show help (unless GUI/REPL will start)
    #[cfg(all(not(feature = "gui"), not(feature = "repl")))]
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
    // With GUI or REPL features, continue even after a subcommand runs
    #[cfg(any(feature = "gui", feature = "repl"))]
    {
      if let Some(mut app) = app {
        // No subcommand was run, show help and let GUI/REPL take over
        app.print_help().expect("failed to print_help");
      }
      // If app is None, a subcommand ran - continue to GUI/REPL
    }
  }
  #[cfg(feature = "gui")]
  {
    gui::start(context.clone()).expect("failed to start gui");
  }
  #[cfg(not(feature = "gui"))]
  {
    #[cfg(feature = "repl")]
    {
      let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
      rt.block_on(async { repl::start(context.clone()).await })
        .expect("repl failed");
    }
    #[cfg(all(not(feature = "cli"), not(feature = "repl")))]
    {
      panic!("select at least one feature of cli, repl, gui");
    }
  }
}
