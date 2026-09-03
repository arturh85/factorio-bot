use crate::cli::{
  SETTINGS_PRECEDENCE_HELP, Subcommand, SubcommandCallback, resolve_resume, resume_args,
  settings_overrides,
};
use crate::context::Context;
use crate::settings::load_app_settings_with;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use factorio_bot_core::miette::{Context as _, Result};
use factorio_bot_core::paris::{info, warn};
use factorio_bot_core::process::process_control::{
  FactorioInstance, FactorioParams, FactorioStartCondition,
};

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "start"
  }
  fn build_command(&self) -> Command {
    // `start` resumes for a different reason than `lua` does: not to skip a
    // prelude, but to *look* at the world a milestone reached -- open it, walk
    // around it, query it over RCON. Same flags, same refusal on changed mod
    // code.
    resume_args(Command::new("start").about("start the factorio server and clients + web server"))
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .value_parser(value_parser!(u8))
          .help("number of clients to start in addition to the server"),
      )
      .arg(
        Arg::new("server")
          .short('s')
          .long("server")
          .value_name("server")
          .required(false)
          .value_parser(value_parser!(String))
          .help("connect to server instead of starting a server"),
      )
      .arg(
        Arg::new("seed")
          .long("seed")
          .value_name("seed")
          .required(false)
          .value_parser(value_parser!(String))
          .help("use given seed to recreate level"),
      )
      .arg(
        Arg::new("map")
          .long("map")
          .value_name("map")
          .required(false)
          .value_parser(value_parser!(String))
          .help("use given map exchange string"),
      )
      .arg(
        Arg::new("new")
          .long("new")
          .short('n')
          .action(ArgAction::SetTrue)
          .help("recreate level by deleting server map if exists"),
      )
      .arg(
        Arg::new("logs")
          .short('l')
          .long("logs")
          .action(ArgAction::SetTrue)
          .help("enabled writing server & client logs to workspace"),
      )
      .arg(
        Arg::new("verbose")
          .short('v')
          .long("verbose")
          .action(ArgAction::SetTrue)
          .help("Log server output to console"),
      )
      .about("start the factorio server and clients, and keep them running")
      .after_help(format!(
        "`start` only launches processes; it never plans, so it has no bot count \
         to conflate with --clients. Use `lua --clients N --bots M` for that.\n\n\
         The command stays in the foreground and keeps the server and clients \
         alive until you interrupt it with Ctrl-C, at which point it stops \
         them. It does not run them in the background: nothing survives the \
         command exiting.\n\n{SETTINGS_PRECEDENCE_HELP}"
      ))
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

async fn run(matches: &ArgMatches, context: &mut Context) -> Result<()> {
  let app_settings = load_app_settings_with(&settings_overrides(matches))?;
  // Unlike `lua`, this count feeds exactly one thing: how many client processes
  // to launch. Nothing here plans, so there is no bot count to conflate it with.
  let clients = *matches.get_one::<u8>("clients").expect("defaulted by clap");
  let write_logs = matches.get_flag("logs");
  let verbose = matches.get_flag("verbose");
  let seed = matches.get_one::<String>("seed").cloned();
  let map_exchange_string = matches.get_one::<String>("map").cloned();
  let recreate = matches.get_flag("new");
  let server_host = matches.get_one::<String>("server").cloned();
  // let websocket_server = FactorioWebSocketServer { listeners: vec![] }.start();

  let resume_from = resolve_resume(
    matches,
    std::path::Path::new(app_settings.factorio.workspace_path.as_ref()),
  )?;

  let params = FactorioParams {
    seed,
    resume_from,
    server_host,
    client_count: clients,
    recreate,
    write_logs,
    map_exchange_string,
    wait_until: FactorioStartCondition::DiscoveryComplete,
    silent: !verbose,
    ..FactorioParams::default()
  };
  // Was `.expect("failed to start factorio")`: a missing archive, a mod that
  // fails to load or an occupied port are expected conditions, not panics.
  let instance_state = FactorioInstance::start(&app_settings.factorio, params)
    .await
    .wrap_err("failed to start Factorio")?;

  // `FactorioInstance::start` logs a client-connect timeout with `error!` and
  // then returns the instance anyway -- that is deliberate, and `lua` relies on
  // it. What was not deliberate is that `start` printed a bare "started!"
  // directly underneath "Timeout waiting for clients to connect", so a run
  // where no client ever joined still read as a clean success. Ask RCON how
  // many players are actually in the game and report that instead of assuming.
  let connected = instance_state
    .rcon
    .connected_player_count()
    .await
    .unwrap_or(0);
  match start_outcome(clients, connected) {
    StartOutcome::Ready => info!("started!"),
    StartOutcome::ClientsMissing {
      expected,
      connected,
    } => warn!(
      "started the server, but only <yellow>{connected}</>/<yellow>{expected}</> client(s) connected"
    ),
  }

  // Hold the instance in the shared slot and block until the user asks us to
  // stop.
  //
  // This `run` used to return here, and that is the whole bug. `cli::start`
  // answered `Ok(None)`, and `lib::run` then either returned outright (a build
  // without the `repl` feature) or fell through to `repl::start`, which panics
  // with `failed to read_line` the moment stdin is not a TTY -- and under
  // `panic = "abort"` that is a SIGABRT. Either way the parent died seconds
  // after printing "started!".
  //
  // Nothing in this codebase killed the children on that path:
  // `InteractiveProcess` has no `Drop` and `FactorioInstance::stop` was never
  // called. The server died anyway, ~2s later, because its stdout and stderr
  // are pipes back to `read_output`'s reader threads -- once the parent is
  // gone the read ends close and Factorio takes SIGPIPE on its next write. The
  // client got `Stdio::null()` (the macOS GUI workaround in `start_client`), so
  // it had no pipe to break: it was reparented to init and sat there
  // indefinitely on Factorio's "server left" modal, a graphical window that
  // looks like a working setup but is attached to nothing.
  //
  // So: stay up, and on the way out stop the instance explicitly, which kills
  // the clients and then the server.
  *context.instance_state.write().await = Some(instance_state);
  info!("running - press Ctrl-C to stop");
  crate::cli::shutdown_signal().await;
  info!("shutdown signal received, stopping ...");
  if let Some(instance) = context.instance_state.write().await.take() {
    instance.stop()?;
  }
  info!("stopped");

  // Exit rather than return, for the same reason `serve` does: returning would
  // hand a `repl`-enabled build to `repl::start` after the user has already
  // asked the process to end, and that REPL panics without a TTY.
  std::process::exit(0);
}

/// What `start` actually brought up, as against what was asked for.
#[derive(Debug, PartialEq, Eq)]
enum StartOutcome {
  Ready,
  ClientsMissing { expected: u8, connected: u8 },
}

/// Compares the number of clients that joined the game against the number
/// requested, so the closing message can describe the world rather than assume
/// it.
fn start_outcome(expected: u8, connected: usize) -> StartOutcome {
  let connected = u8::try_from(connected).unwrap_or(u8::MAX);
  if connected >= expected {
    StartOutcome::Ready
  } else {
    StartOutcome::ClientsMissing {
      expected,
      connected,
    }
  }
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}

#[cfg(test)]
mod tests {
  use super::{StartOutcome, start_outcome};

  /// Every requested client joined, so the run is what the user asked for.
  #[test]
  fn every_requested_client_connected_is_ready() {
    assert_eq!(start_outcome(2, 2), StartOutcome::Ready);
  }

  /// `-c 0` asks for a server and no clients, which is a complete success.
  #[test]
  fn a_server_with_no_clients_requested_is_ready() {
    assert_eq!(start_outcome(0, 0), StartOutcome::Ready);
  }

  /// The case that used to print "started!" directly underneath
  /// `FactorioInstance::start`'s own "Timeout waiting for clients to
  /// connect": the server is up but the client never joined, so the report
  /// must not read as success.
  #[test]
  fn a_client_that_never_connected_is_reported_missing() {
    assert_eq!(
      start_outcome(1, 0),
      StartOutcome::ClientsMissing {
        expected: 1,
        connected: 0
      }
    );
  }
}
