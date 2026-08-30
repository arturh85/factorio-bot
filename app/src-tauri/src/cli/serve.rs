use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{value_parser, Arg, ArgMatches, Command};
use factorio_bot_core::miette::{IntoDiagnostic, Result};
use factorio_bot_core::paris::info;
use std::net::SocketAddr;

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "serve"
  }
  fn build_command(&self) -> Command {
    Command::new("serve")
      .arg(
        Arg::new("bind")
          .short('b')
          .long("bind")
          .value_name("ADDR")
          .required(false)
          .value_parser(value_parser!(String))
          .help("address to bind, defaults to 127.0.0.1 on the configured port"),
      )
      .about("serve the web UI and HTTP API")
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: &ArgMatches, context: &mut Context) -> Result<()> {
  let bind: SocketAddr = if let Some(raw) = matches.get_one::<String>("bind") {
    raw.parse().into_diagnostic()?
  } else {
    let port = context.app_settings.read().await.restapi.port;
    SocketAddr::from(([127, 0, 0, 1], u16::try_from(port).into_diagnostic()?))
  };

  // Orchestrators, systemd, and `docker stop` all send SIGTERM rather than
  // Ctrl-C's SIGINT. Waiting on ctrl_c() alone would let SIGTERM kill the
  // process without ever running the graceful-shutdown path, orphaning
  // Factorio exactly as before. `signal::unix` is Unix-only, so gate it and
  // fall back to plain ctrl_c() on other targets (e.g. Windows).
  let shutdown = async {
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
    info!("shutdown signal received, stopping ...");
  };

  // `crates/server` logs through `tracing`, which is correct for a library but
  // has no subscriber installed in CLI mode, so nothing it logs ever reaches a
  // terminal. `serve` is a long-running foreground command: it has to tell the
  // user which address it is about to bind. `paris` writes straight to stdout
  // and is what the rest of the binary already uses.
  info!("serving http://{} - press Ctrl-C to stop", bind);

  factorio_bot_server::webserver::start_with_shutdown(
    context.app_settings.clone(),
    context.instance_state.clone(),
    bind,
    shutdown,
  )
  .await?;

  info!("server stopped");

  // `run()` in lib.rs falls through after any subcommand into the REPL (or
  // GUI) start-up, which is correct for setup commands like `start` that
  // intentionally hand off to an interactive session. `serve` is different:
  // it's a long-running foreground command, and by the time
  // `start_with_shutdown` has returned here the Factorio instance has
  // already been stopped and the user has explicitly asked (via Ctrl-C) for
  // the process to end. Falling through would instead start a REPL with no
  // TTY behind it and panic. Exit explicitly instead.
  //
  // Interim: plan 4 deletes the GUI and reworks `run()`'s control flow, at
  // which point this should become proper control flow rather than a
  // process exit.
  std::process::exit(0);
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
