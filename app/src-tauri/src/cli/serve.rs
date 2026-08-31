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
      .arg(
        Arg::new("web-root")
          .long("web-root")
          .value_name("DIR")
          .required(false)
          .value_parser(value_parser!(String))
          .help("directory to serve the web UI from, overrides settings.restapi.web_root"),
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

  // `Context::new` creates `workspace/`, but nothing creates
  // `workspace/scripts/`: the desktop app got that as a side effect of
  // `scripts_dir`, which the HTTP handlers deliberately do not call (it would
  // resolve `./scripts` against the server process's working directory). On a
  // fresh install that left every `/api/v1/scripts*` route answering "missing
  // scripts directory", with no way to create a first script from a browser.
  // Bootstrap it here instead — once, at startup, from the configured
  // workspace path only, never relative to the CWD.
  {
    let configured = context
      .app_settings
      .read()
      .await
      .factorio
      .workspace_path
      .to_string();
    // The shared rule, not a fourth copy of it. `resolve_workspace` already
    // rejects a relative path -- the local `is_relative` check that used to
    // follow this could no longer fire, and a guard that cannot fail is worse
    // than none, because the next reader trusts it.
    //
    // This is the *use*, which is where the refusal belongs: settings load
    // only fills the default, so `config show` can still print a relative
    // `workspace_path` and `config init --force` can still rewrite it.
    //
    // A bare `?` rather than `.map_err(|err| miette!("{err}"))?`:
    // `RelativeWorkspacePath` is a `Diagnostic`, so `From<_> for Report`
    // carries its `help` -- "set settings.factorio.workspace_path to an
    // absolute path, or leave it empty ..." -- through to the terminal.
    // Reformatting it through `miette!` drops exactly that sentence, which is
    // the only actionable one in the message.
    let workspace_path = factorio_bot_core::paths::resolve_workspace(&configured)?;
    let scripts_dir = factorio_bot_core::scripts::ensure_scripts_dir(&workspace_path)?;
    info!("scripts directory: {}", scripts_dir.display());
  }

  // `start_with_shutdown` reads `settings.restapi.web_root` back out of this
  // same `SharedAppSettings` at startup (see `webserver.rs`), so overriding
  // it here — before the call below — is what actually reaches the router;
  // setting it any later (e.g. after `build_router` has already run) would
  // be a no-op.
  if let Some(web_root) = matches.get_one::<String>("web-root") {
    context.app_settings.write().await.restapi.web_root = Some(web_root.clone());
  }

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
    factorio_bot_server::webserver::SHUTDOWN_GRACE_PERIOD,
  )
  .await?;

  info!("server stopped");

  // `run()` falls through into the REPL after any subcommand, which is right
  // for setup commands like `start` that intentionally hand off to an
  // interactive session. `serve` is different: by the time
  // `start_with_shutdown` returns, the Factorio instance is stopped and the
  // user asked (Ctrl-C / SIGTERM) for the process to end. Falling through
  // would start a REPL with no TTY behind it. Exit explicitly.
  std::process::exit(0);
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
