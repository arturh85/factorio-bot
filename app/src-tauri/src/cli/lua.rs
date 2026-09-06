use crate::cli::{
  SETTINGS_PRECEDENCE_HELP, Subcommand, SubcommandCallback, resolve_resume, resume_args,
  settings_overrides,
};
use crate::context::Context;
use crate::scripting::run_script_file;
use crate::settings::load_app_settings_with;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::factorio::snapshot::attach_world;
use factorio_bot_core::miette::{Context as _, Result};
use factorio_bot_core::paris::{info, warn};
use factorio_bot_core::parking_lot::RwLock;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::process::process_control::{
  FactorioInstance, FactorioParams, FactorioStartCondition,
};
use std::path::Path;
use std::sync::Arc;

const LUA_AFTER_HELP: &str = "\
--clients and --bots are different numbers:

  --clients  how many *graphical Factorio client processes* to launch.
             Each one needs a display and ~26s to load sprites, and the
             server then waits up to 90s for them to connect.
  --bots     how many *bots the script plans for*. Bots the clients did not
             bring are synthesised by the planner with a default inventory,
             so goals can be planned without any client running.

--bots defaults to --clients, so existing invocations are unchanged.

Fast planning loop (no graphical client, no 90s connect wait):

  factorio-bot lua myscript.lua --clients 0 --bots 4

Note that --clients 0 plans and simulates only: nothing moves in the game
world, because there are no real players to move. Use it to iterate on goal
decomposition and task graphs, then re-run with --clients N to execute.

--connect attaches to a Factorio server this program did not start -- one you
are already playing on -- over RCON alone:

  factorio-bot lua myscript.lua --connect

The world is read with a single `world_snapshot` RCON call, plus the connected
players, plus the entities in ONE 400x400 square -- centred on the first
connected player, or on the origin when nobody is connected. One square, not
one per player: 200 tiles either side was chosen by measuring the payload
(~1.9 MB), and a square per player multiplies that.

goal.* plans against that snapshot and world.* answers from it, but five
things an owned server gives you are absent, and none of them announce
themselves:

  * No event stream. The world is a point-in-time read: research finished,
    entities built or mined, and players moved after the snapshot stay
    invisible until the script is run again.
  * Recipes are only those enabled at snapshot time; a technology that
    finishes afterwards does not appear.
  * Entities outside that one square do not exist as far as the plan is
    concerned.
  * No tiles, which means no water. world.draw still renders, and renders a
    map with no lakes in it -- a wrong picture rather than an error.
  * No graphics. The map renderer's sprite atlas is not read.

The server must have the BotBridge mod loaded and RCON enabled.

--resume-from starts on a world a previous run already reached, instead of
re-deriving it:

  factorio-bot lua myscript.lua --resume-from run-1788465258-49050:3
  factorio-bot lua myscript.lua --resume-from run-1788465258-49050   # its last

Every milestone a run satisfies writes runs/<run>/savepoints/milestone-<n>.zip,
so the twenty minutes to researched(\"automation\") is paid once rather than
before every experiment behind it. The savepoint is copied into the server
instance and started from the copy: neither the archived savepoint nor the
workspace's level.zip is written to.

Two things a resumed run is not:

  * It is NOT benchmark-comparable with a fresh-world run. It begins with
    built furnaces, charged chests and partly mined patches. `provenance.json`
    records which savepoint it came from and `just analyse --compare` refuses
    to measure the two against each other.
  * It is NOT a clean mod state. The save carries BotBridge's `storage`, and
    Factorio migrates that only on a mod version bump, which this project
    pins. A resume clears the run-scoped half of it (walk state, craft and
    research waiters, the sampling session) before anything else runs, and
    refuses outright if the mod's code has changed since the savepoint was
    written -- pass --resume-force to override that, which is recorded.";

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "lua"
  }
  fn build_command(&self) -> Command {
    resume_args(Command::new("lua").about("Start Factorio and run a Lua script"))
      .after_help(format!("{LUA_AFTER_HELP}\n\n{SETTINGS_PRECEDENCE_HELP}"))
      .arg(
        Arg::new("script")
          .help("Path to the Lua script to run (relative to scripts/ folder)")
          .required(true)
          .value_parser(value_parser!(String)),
      )
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .value_parser(value_parser!(u8))
          .help("number of graphical Factorio clients to start (0 = server only)"),
      )
      .arg(
        Arg::new("bots")
          .short('b')
          .long("bots")
          .value_name("bots")
          .required(false)
          .value_parser(value_parser!(u8))
          .help("number of bots the script plans for [default: same as --clients]"),
      )
      .arg(
        Arg::new("headless")
          .long("headless")
          .action(ArgAction::SetTrue)
          .conflicts_with("connect")
          .help(
            "bots are server-side character entities created by the mod; no graphical \
             client is started (implies --clients 0; --bots defaults to 1)",
          ),
      )
      .arg(
        Arg::new("game-speed")
          .long("game-speed")
          .value_name("speed")
          .default_value("1")
          .value_parser(value_parser!(f64))
          .help("run the world at this game.speed; every wall-clock deadline scales with it"),
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
      .arg(
        Arg::new("connect")
          .long("connect")
          .action(ArgAction::SetTrue)
          .help("Attach to an already-running Factorio server instead of starting one"),
      )
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(run(args, context))
  }
}

/// How many client processes to launch, and how many bots to plan for.
///
/// These used to be one number, which meant `-c 0` planned for zero bots (every
/// goal failed with "no bots in this run") and `-c 1` on a headless machine
/// insisted on launching a graphical client. They are independent: planning
/// needs a bot count, execution needs client processes.
///
/// `--bots` defaults to `--clients` so every invocation written before the split
/// keeps its old meaning.
/// [`resolve_counts`] plus the headless flag.
///
/// `--headless` means the bots are character entities the mod creates on the
/// server, so there is nothing for `--clients` to count: it must be absent or
/// `0`, and `--bots` -- which normally defaults to `--clients` -- defaults to
/// one bot instead of zero. A run is all clients or all characters, and the
/// mix is refused here, before a process is spawned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCounts {
  pub clients: u8,
  pub bots: u8,
  pub headless: bool,
}

pub fn resolve_run_counts(matches: &ArgMatches) -> Result<RunCounts> {
  let clients = *matches.get_one::<u8>("clients").expect("defaulted by clap");
  let headless = matches
    .try_get_one::<bool>("headless")
    .ok()
    .flatten()
    .copied()
    .unwrap_or(false);
  if headless {
    if clients > 0
      && matches.value_source("clients") == Some(clap::parser::ValueSource::CommandLine)
    {
      return Err(factorio_bot_core::miette::miette!(
        "--headless spawns character bots and cannot also start {clients} graphical \
         client(s): a run is all clients or all characters (drop --clients)"
      ));
    }
    let bots = matches.get_one::<u8>("bots").copied().unwrap_or(1);
    return Ok(RunCounts {
      clients: 0,
      bots,
      headless: true,
    });
  }
  let bots = matches.get_one::<u8>("bots").copied().unwrap_or(clients);
  Ok(RunCounts {
    clients,
    bots,
    headless: false,
  })
}

async fn run(matches: &ArgMatches, _context: &mut Context) -> Result<()> {
  let app_settings = load_app_settings_with(&settings_overrides(matches))?;
  let script_path = matches
    .get_one::<String>("script")
    .expect("required by clap");
  let RunCounts {
    clients,
    bots,
    headless,
  } = resolve_run_counts(matches)?;
  let game_speed = *matches
    .get_one::<f64>("game-speed")
    .expect("defaulted by clap");
  let connect_mode = matches.get_flag("connect");
  let server_host = matches.get_one::<String>("server").cloned();
  warn_if_server_flags_are_ignored(connect_mode || server_host.is_some(), headless, game_speed);

  if connect_mode {
    // Fast iteration mode: connect to already-running Factorio
    info!(
      "Connecting to running Factorio to run script: {}",
      script_path
    );

    let rcon_settings = RconSettings::new(
      app_settings.factorio.rcon_port,
      &app_settings.factorio.rcon_pass,
      server_host,
    );
    // Was `.expect(..)`: a refused connection is an ordinary operational
    // failure, not a broken invariant, and printing it as a panic buried the
    // cause under a backtrace hint.
    let rcon = FactorioRcon::new(&rcon_settings, Arc::new(RwLock::new(false)))
      .await
      .wrap_err_with(|| {
        format!(
          "failed to connect to RCON on port {} - is Factorio running?",
          app_settings.factorio.rcon_port
        )
      })?;

    // Read the world over RCON rather than leaving it empty.
    //
    // This used to hand the planner a `FactorioSurface::new()` -- no recipes, no
    // prototypes, no entity graph -- because all of that arrived by parsing the
    // stdout of a server *this process spawned*, which an attached session does
    // not have. Every world.* and goal.* call therefore found nothing, which is
    // what the old help text meant by "only rcon.*". `attach_world` asks the
    // mod for the same records over RCON instead.
    let rcon = Arc::new(rcon);
    let world = attach_world(&rcon, None).await?;
    info!(
      "Attached: {} entity prototypes, {} recipes, {} player(s)",
      world.entity_prototypes.len(),
      world.recipes.len(),
      world.players.len()
    );
    // Attached: this is someone else's server, and `goal.plan` must not
    // stop its clock (see `ServerOwnership`).
    let mut planner = Planner::attached(world, Some(rcon));

    let (stdout, stderr) =
      run_script_file(&mut planner, &app_settings, script_path, bots, None).await?;
    print_script_output(&stdout, &stderr);

    info!("Script completed");
  } else {
    // Full mode: start Factorio server + clients -- unless `--server <host>`
    // named one, in which case nothing is started and the run is attached.
    let attached_server = server_host.is_some();
    let params = start_params(
      matches,
      &app_settings,
      headless,
      clients,
      bots,
      game_speed,
      server_host,
    )?;
    info!("Starting Factorio to run script: {}", script_path);

    // Was `.expect("failed to start factorio")`. A missing archive, a mod that
    // fails to load or an occupied port are all expected conditions and are
    // reported as errors, not as "The application panicked (crashed)".
    let instance_state = FactorioInstance::start(&app_settings.factorio, params)
      .await
      .wrap_err("failed to start Factorio")?;

    // The script result is captured rather than `?`-propagated: returning early
    // here would drop `instance_state` without stopping it, and
    // `FactorioInstance` has no `Drop`, so a failing script leaked a Factorio
    // server holding the factorio and rcon ports. The next run then failed with
    // "Host address is already in use" instead of the real error.
    let script_result = match instance_state.surface() {
      Some(world) => {
        info!("Factorio started, running script...");
        let mut planner = if attached_server {
          Planner::attached(world.clone(), Some(instance_state.rcon.clone()))
        } else {
          Planner::new(world.clone(), Some(instance_state.rcon.clone()))
        };
        if clients == 0 && !headless {
          // The one mode entitled to bots the game does not have. `--clients 0`
          // starts no Factorio client at all, so the world has no players and
          // `Planner::roster` -- which is what every other run gets -- would
          // hand the script an empty roster. Seeding them here, where the
          // intent to simulate is stated, keeps that intent out of the roster
          // itself: a run that asked for four clients and got three still
          // plans for three, instead of a phantom bot at the origin.
          planner.initiate_missing_players_with_default_inventory(bots);
        }
        run_script_file(&mut planner, &app_settings, script_path, bots, None).await
      }
      None => Err(factorio_bot_core::miette::miette!(
        "Failed to start Factorio (no world available)"
      )),
    };

    // Clean up Factorio processes (clients first, then server) on every path.
    if let Err(err) = instance_state.stop() {
      warn!("failed to stop Factorio cleanly: {:?}", err);
    }

    let (stdout, stderr) = script_result?;
    print_script_output(&stdout, &stderr);
    info!("Script completed");
  }

  Ok(())
}

/// `--headless` and `--game-speed` decide how a server this process starts is
/// set up. With `--connect` or `--server` it starts none, so both would be
/// silently ignored; say so instead. Not gated on `silent`, for the reason the
/// `--seed` warning is not: a flag that looks honoured and is not is worse
/// than one that is loud.
fn warn_if_server_flags_are_ignored(attached: bool, headless: bool, game_speed: f64) {
  let speed_requested = (game_speed - 1.0).abs() > f64::EPSILON;
  if attached && (headless || speed_requested) {
    warn!(
      "--headless / --game-speed are IGNORED with --connect or --server: this process \
       does not start the server and cannot decide what its bots are"
    );
  }
}

/// Everything `FactorioInstance::start` needs for a run this process starts,
/// resolved and judged before Factorio is touched: a refusal after the server
/// is up costs a minute of startup to say something that was knowable from
/// two files.
#[allow(clippy::too_many_arguments)]
fn start_params(
  matches: &ArgMatches,
  app_settings: &factorio_bot_core::app_settings::AppSettings,
  headless: bool,
  clients: u8,
  bots: u8,
  game_speed: f64,
  server_host: Option<String>,
) -> Result<FactorioParams> {
  let write_logs = matches.get_flag("logs");
  let verbose = matches.get_flag("verbose");
  let seed = matches.get_one::<String>("seed").cloned();
  let map_exchange_string = matches.get_one::<String>("map").cloned();
  let recreate = matches.get_flag("new");

  if headless {
    info!(
      "Headless run: {} character bot(s), no graphical client",
      bots
    );
  } else if clients == 0 {
    info!("Planning-only run: no graphical clients, {} bot(s)", bots);
  }

  let resume_from = resolve_resume(
    matches,
    Path::new(app_settings.factorio.workspace_path.as_ref()),
  )?;

  Ok(FactorioParams {
    seed,
    resume_from,
    server_host,
    client_count: clients,
    character_bots: if headless { bots } else { 0 },
    game_speed,
    factorio_port: app_settings.factorio.factorio_port,
    recreate,
    write_logs,
    map_exchange_string,
    wait_until: FactorioStartCondition::DiscoveryComplete,
    silent: !verbose,
    ..FactorioParams::default()
  })
}

fn print_script_output(stdout: &str, stderr: &str) {
  if !stdout.is_empty() {
    print!("{stdout}");
  }
  if !stderr.is_empty() {
    eprint!("{stderr}");
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

  fn counts_for(argv: &[&str]) -> (u8, u8) {
    let matches = build_app()
      .try_get_matches_from(argv)
      .unwrap_or_else(|e| panic!("parses {argv:?}: {e}"));
    let sub = matches.subcommand_matches("lua").expect("lua matched");
    let counts = resolve_run_counts(sub).expect("no --headless in these argv");
    (counts.clients, counts.bots)
  }

  fn run_counts_for(argv: &[&str]) -> Result<RunCounts> {
    let matches = build_app()
      .try_get_matches_from(argv)
      .unwrap_or_else(|e| panic!("parses {argv:?}: {e}"));
    let sub = matches.subcommand_matches("lua").expect("lua matched");
    resolve_run_counts(sub)
  }

  #[test]
  fn headless_means_no_clients_and_the_bots_asked_for() {
    let counts =
      run_counts_for(&["factorio-bot", "lua", "x.lua", "--headless", "--bots", "4"]).unwrap();
    assert_eq!(
      counts,
      RunCounts {
        clients: 0,
        bots: 4,
        headless: true
      }
    );
  }

  /// `--bots` normally defaults to `--clients`, which would be zero here and
  /// plan for nobody.
  #[test]
  fn headless_alone_is_one_bot() {
    let counts = run_counts_for(&["factorio-bot", "lua", "x.lua", "--headless"]).unwrap();
    assert_eq!(
      counts,
      RunCounts {
        clients: 0,
        bots: 1,
        headless: true
      }
    );
  }

  #[test]
  fn headless_with_clients_is_refused_by_name() {
    let err = run_counts_for(&[
      "factorio-bot",
      "lua",
      "x.lua",
      "--headless",
      "--clients",
      "2",
    ])
    .unwrap_err();
    let message = err.to_string();
    assert!(
      message.contains("--headless") && message.contains("2 graphical"),
      "{message}"
    );
  }

  #[test]
  fn without_headless_nothing_changes() {
    let counts = run_counts_for(&["factorio-bot", "lua", "x.lua", "--clients", "3"]).unwrap();
    assert_eq!(
      counts,
      RunCounts {
        clients: 3,
        bots: 3,
        headless: false
      }
    );
  }

  /// Backwards compatibility: with no `--bots`, the two numbers stay equal, so
  /// every invocation written before the split means what it always meant.
  #[test]
  fn bots_defaults_to_clients() {
    assert_eq!(counts_for(&["factorio-bot", "lua", "s.lua"]), (1, 1));
    assert_eq!(
      counts_for(&["factorio-bot", "lua", "s.lua", "-c", "3"]),
      (3, 3)
    );
    assert_eq!(
      counts_for(&["factorio-bot", "lua", "s.lua", "-c", "0"]),
      (0, 0)
    );
  }

  /// The half that proves the split actually happened: a defaulting test alone
  /// passes just as well against the old code, where one value fed both uses.
  /// Here the two numbers must come out *different*.
  #[test]
  fn bots_and_clients_can_differ() {
    assert_eq!(
      counts_for(&["factorio-bot", "lua", "s.lua", "-c", "2", "-b", "5"]),
      (2, 5)
    );
    assert_eq!(
      counts_for(&[
        "factorio-bot",
        "lua",
        "s.lua",
        "--clients",
        "4",
        "--bots",
        "1"
      ]),
      (4, 1)
    );
  }

  /// The planning-only invocation from `--help`: a server, no graphical client
  /// to launch and wait for, and a non-zero bot count so goals can be planned.
  /// Under the old conflation this combination was unreachable -- `-c 0` gave
  /// zero bots and every goal failed with "no bots in this run".
  #[test]
  fn planning_only_mode_has_no_clients_but_does_have_bots() {
    let (clients, bots) = counts_for(&[
      "factorio-bot",
      "lua",
      "s.lua",
      "--clients",
      "0",
      "--bots",
      "4",
    ]);
    assert_eq!(clients, 0, "no graphical client process may be launched");
    assert_eq!(bots, 4, "the planner must still get bots to plan for");
  }

  /// `--connect` used to advertise its own limitation -- "world.* functions
  /// won't work, only rcon.*" -- and that sentence is now false: the world is
  /// read over RCON with `world_snapshot`. A stale help string is worse than
  /// none, so the old claim must be gone *and* the new behaviour, including the
  /// point-in-time caveat that replaces it, must be stated.
  #[test]
  fn help_describes_connect_as_attaching_not_as_rcon_only() {
    let mut lua = build_app()
      .find_subcommand("lua")
      .expect("lua subcommand exists")
      .clone();
    let help = lua.render_long_help().to_string();
    assert!(
      !help.contains("only rcon."),
      "the old --connect limitation is still advertised:\n{help}"
    );
    assert!(
      help.contains("goal.* plans against that snapshot"),
      "missing what --connect now permits:\n{help}"
    );
    assert!(
      help.contains("point-in-time"),
      "missing the snapshot caveat that replaces the old limitation:\n{help}"
    );
  }

  /// `attach_world` centres its one entity read on `players.first()`. The help
  /// used to say "within 200 tiles of them", which reads as a square per
  /// player, and a reader who believed it would expect every bot's
  /// surroundings to be loaded.
  #[test]
  fn help_says_the_read_area_is_one_square_around_one_player() {
    let mut lua = build_app()
      .find_subcommand("lua")
      .expect("lua subcommand exists")
      .clone();
    let help = lua.render_long_help().to_string();
    assert!(
      help.contains("first\nconnected player") || help.contains("first connected player"),
      "the help does not say which player the square is centred on:\n{help}"
    );
    assert!(
      help.contains("not\none per player") || help.contains("not one per player"),
      "the help still leaves a square per player as a plausible reading:\n{help}"
    );
  }

  /// The five things an attached session gives up are all stated in
  /// `attach_world`'s doc; the help used to state two. The tile one is the
  /// dangerous omission, because `world.draw` reads the blocked-tile tree and
  /// an attached world has no tiles -- so it draws a map with no water and
  /// says nothing. A user must not have to discover that from the picture.
  #[test]
  fn help_admits_that_an_attached_world_has_no_water() {
    let mut lua = build_app()
      .find_subcommand("lua")
      .expect("lua subcommand exists")
      .clone();
    let help = lua.render_long_help().to_string();
    assert!(
      help.contains("no water"),
      "the help does not warn that an attached world has no tiles:\n{help}"
    );
    assert!(
      help.contains("world.draw"),
      "the help does not name the command that silently draws the wrong map:\n{help}"
    );
  }

  /// `--bots` is documented, and so is the planning-only loop it enables. The
  /// flag is useless to anyone who cannot find out that it exists.
  #[test]
  fn help_documents_the_split_and_the_planning_only_loop() {
    let mut lua = build_app()
      .find_subcommand("lua")
      .expect("lua subcommand exists")
      .clone();
    let help = lua.render_long_help().to_string();
    assert!(help.contains("--bots"), "missing --bots:\n{help}");
    assert!(
      help.contains("--clients 0 --bots 4"),
      "missing the planning-only example:\n{help}"
    );
    assert!(
      help.contains("Settings precedence"),
      "missing the precedence note:\n{help}"
    );
  }
}
