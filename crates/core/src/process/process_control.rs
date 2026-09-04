use crate::constants::SERVER_SETTINGS_FILENAME;
use crate::errors::*;
use crate::factorio::rcon::{FactorioRcon, RconSettings};
use crate::factorio::world::FactorioWorld;
use crate::process::arrange_windows::arrange_windows;
use crate::process::connect_wait::{ConnectWait, ConnectWatcher, missing_clients};
use crate::process::instance_setup::setup_factorio_instance;
use crate::process::output_reader::read_output;
use crate::process::{InteractiveProcess, io_utils};
use crate::record::run_mode::{BotMode, RunMode, write_run_mode};
use crate::record::savepoint::{ResumeMarker, clear_resume_marker, write_resume_marker};
use crate::settings::FactorioSettings;
use crate::types::PlayerId;
use miette::{IntoDiagnostic, Result};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

pub type SharedFactorioInstance = Arc<RwLock<Option<FactorioInstance>>>;

pub struct FactorioInstance {
    pub world: Option<Arc<FactorioWorld>>,
    pub rcon: Arc<FactorioRcon>,
    pub server_process: Option<InteractiveProcess>,
    pub client_processes: Vec<InteractiveProcess>,

    pub silent: Arc<parking_lot::RwLock<bool>>,
    pub server_host: Option<String>,
    pub server_port: Option<u16>,
    pub rcon_port: u16,
    pub client_count: u8,
    pub map_exchange_string: Option<String>,
    pub seed: Option<String>,
}

pub struct FactorioParams {
    pub server_host: Option<String>,
    pub client_count: u8,
    pub recreate: bool,
    pub instance_name: Option<String>,
    pub factorio_port: Option<u16>,
    pub map_exchange_string: Option<String>,
    pub seed: Option<String>,
    pub write_logs: bool,
    pub silent: bool,
    pub wait_until: FactorioStartCondition,
    /// A savepoint to start this server from instead of the instance's own
    /// `level.zip`.
    ///
    /// The file is **copied** into the instance's saves directory and the
    /// server is started from the copy, so the archived savepoint is never the
    /// file a running Factorio holds -- a resumed run that autosaves, or that
    /// is asked for a nameless `/server-save`, cannot write back over the
    /// record of the run that produced it. `level.zip` is not touched either:
    /// in this workspace it is the only copy of the map every measurement so
    /// far was taken on, and it predates `--seed`, so nothing could recreate
    /// it.
    ///
    /// See [`crate::record::savepoint`] for what a resumed run costs: it is
    /// not benchmark-comparable with a fresh one, and the marker this writes
    /// is what makes `--compare` refuse to pretend otherwise.
    ///
    /// The whole marker rather than a path, because the caller is the only one
    /// that can decide the two policy questions -- which savepoint, and what
    /// to do when the mod code has changed since it was written -- and the
    /// answer to the second has to be recorded whichever way it went.
    pub resume_from: Option<ResumeMarker>,
    /// How many server-side character bots to create instead of clients.
    ///
    /// `0` is the ordinary run: bots are the graphical clients that connect.
    /// Anything else is `--headless`: the mod creates that many `character`
    /// entities (ids `1..=n`) on the server the moment it is up, no client is
    /// spawned, and `client_count` must be `0` -- a run is all clients or all
    /// characters, and [`FactorioInstance::start`] refuses the mix before
    /// touching a process. See `crates/core/src/record/run_mode.rs` for why
    /// the mode is recorded.
    pub character_bots: u8,
    /// `game.speed` to set once the roster is up; `1.0` leaves it alone.
    pub game_speed: f64,
}

impl Default for FactorioParams {
    fn default() -> Self {
        FactorioParams {
            server_host: None,
            client_count: 0,
            recreate: false,
            instance_name: None,
            factorio_port: None,
            map_exchange_string: None,
            seed: None,
            write_logs: false,
            silent: true,
            wait_until: FactorioStartCondition::Initialized,
            resume_from: None,
            character_bots: 0,
            game_speed: 1.0,
        }
    }
}

impl FactorioInstance {
    pub fn new_shared() -> SharedFactorioInstance {
        Arc::new(RwLock::new(None))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        settings: &FactorioSettings,
        params: FactorioParams,
    ) -> Result<FactorioInstance> {
        if params.character_bots > 0 && params.client_count > 0 {
            return Err(miette::miette!(
                "--headless spawns {} character bot(s) and cannot also start {} graphical \
                 client(s): a run is all clients or all characters",
                params.character_bots,
                params.client_count
            ));
        }
        let mut world: Option<Arc<FactorioWorld>> = None;
        let silent = Arc::new(parking_lot::RwLock::new(params.silent));
        let instance_name = params.instance_name.unwrap_or_else(|| "server".to_owned());
        let rcon_settings = RconSettings::new(
            settings.rcon_port,
            &settings.rcon_pass,
            params.server_host.clone(),
        );
        let mut factorio_port = params.factorio_port;
        if params.server_host.is_none() {
            setup_factorio_instance(
                &settings.workspace_path,
                &settings.factorio_archive_path,
                &rcon_settings,
                factorio_port,
                &instance_name,
                true,
                params.recreate,
                params.map_exchange_string.clone(),
                params.seed.clone(),
                params.silent,
            )
            .await?;
        }
        let settings = settings.clone();
        for instance_number in 0..params.client_count {
            let instance_name = format!("client{}", instance_number + 1);
            if let Err(err) = setup_factorio_instance(
                &settings.workspace_path,
                &settings.factorio_archive_path,
                &rcon_settings,
                None,
                &instance_name,
                false,
                false,
                None,
                None,
                params.silent,
            )
            .await
            {
                error!("Failed to setup Factorio <red>{:?}</>: ", err);
                break;
            }
        }
        let mut server_child = None;
        let mut client_children = vec![];

        // Which world the server is about to load, and saying so on disk.
        //
        // Both halves matter. Writing the marker is what lets `record.start()`
        // fill in `provenance.resumed_from` without the fact being threaded
        // through the Lua API; *clearing* it is what stops the next fresh run
        // inheriting this one's answer, which is precisely the mistake
        // `map-gen-seed.txt` made and the reason every run before 2026-09-03
        // silently ran on an uncontrolled map.
        let instance_dir =
            Path::new(settings.workspace_path.as_ref()).join(PathBuf::from(&instance_name));
        let resume_save = match (&params.server_host, &params.resume_from) {
            (None, Some(marker)) => Some(Self::prepare_resume(&instance_dir, marker)?),
            (None, None) => {
                clear_resume_marker(&instance_dir).into_diagnostic()?;
                None
            }
            // An attached server loaded whatever it loaded; this process did
            // not choose it and must not claim to know.
            (Some(_), _) => None,
        };
        let resumed = resume_save.is_some();
        // The run mode is a start-time fact of the instance, written next to
        // the resume marker for `record.start()` to read into provenance.
        if params.server_host.is_none() {
            let mode = RunMode {
                bot_mode: if params.character_bots > 0 {
                    BotMode::Characters
                } else {
                    BotMode::Clients
                },
                game_speed: params.game_speed,
            };
            write_run_mode(&instance_dir, &mode).into_diagnostic()?;
        }

        let rcon = match params.server_host {
            None => {
                let started = Instant::now();
                let (_world, rcon, child, used_factorio_port) = Self::start_server(
                    &settings.workspace_path,
                    &rcon_settings,
                    None,
                    &instance_name,
                    // websocket_server,
                    params.write_logs,
                    silent.clone(),
                    params.wait_until,
                    resume_save,
                )
                .await?;
                factorio_port = Some(used_factorio_port);
                world = Some(_world);
                // report_child_death(child);
                server_child = Some(child);
                if !params.silent {
                    success!(
                        "Started <bright-blue>server</> in <yellow>{:?}</>",
                        started.elapsed()
                    );
                }
                rcon
            }
            Some(_) => Arc::new(FactorioRcon::new(&rcon_settings, silent.clone()).await?),
        };
        if params.server_host.is_none() {
            // Before anything else touches the game, on every server this
            // process starts -- not only on a resumed one.
            //
            // A save carries `script.dat`, which is BotBridge's `storage`, and
            // Factorio migrates that only on a mod version bump; `info.json`
            // is pinned at 0.0.1 exactly so that it does not. So a world loaded
            // from a savepoint arrives holding the previous run's walk state,
            // craft and research waiters and sampling session. Those waiters
            // are keyed by `ActionId`, minted `% 1000` from zero every run, so
            // leaving them lets the previous run's action 7 settle this run's
            // action 7 -- a plausible-looking success for something that never
            // happened.
            //
            // Unconditional because a savepoint is not the only way a world
            // arrives with someone else's `storage` in it: `POST
            // /api/v1/game/server-save` writes over the instance's own
            // `level.zip`, so an ordinary "fresh" start can load a mid-run
            // save without anything saying so. On a genuinely fresh world this
            // drops nothing and reports as much.
            //
            // Narrated rather than fatal: a mod too old to know
            // `session_reset` is a real situation, and saying which one it is
            // beats refusing to start.
            let what = if resumed {
                "resumed from a savepoint"
            } else {
                "started"
            };
            match rcon.session_reset().await {
                Ok(dropped) => info!("{}; cleared the mod's run-scoped state: {}", what, dropped),
                Err(err) => warn!(
                    "{}, but could NOT clear the mod's run-scoped state ({:?}). If this world \
                     came from a save, the previous run's craft and research waiters are still \
                     in `storage` and share an action-id space with this run's.",
                    what, err
                ),
            }
        }
        // Everything past this point runs with a server process (or a remote
        // server) already live. Failures here used to propagate straight out,
        // dropping `server_child` without killing it -- `InteractiveProcess` has
        // no `Drop` -- so the orphaned server kept holding the factorio and rcon
        // ports and the *next* run failed with "Host address is already in use"
        // instead of reporting the real problem. The result is captured so the
        // children can be killed before the error leaves this function.
        let startup: Result<()> = async {
            // Spawn all clients first
            for instance_number in 0..params.client_count {
                let instance_name = format!("client{}", instance_number + 1);
                let instance_path =
                    Path::new(settings.workspace_path.as_ref()).join(PathBuf::from(&instance_name));
                let lock_path = instance_path.join(PathBuf::from(".lock"));

                // Diagnostic logging
                if !params.silent {
                    info!(
                        "Attempting to spawn <bright-blue>{}</> at {:?}",
                        instance_name, instance_path
                    );
                    info!("  Instance directory exists: {}", instance_path.exists());
                    info!("  Lock file exists: {}", lock_path.exists());
                }

                let started = Instant::now();
                let child = Self::start_client(
                    &settings,
                    instance_name.clone(),
                    params.server_host.clone(),
                    params.write_logs,
                    true,
                )
                .await?;

                // Log process ID
                if !params.silent {
                    info!(
                        "  Successfully spawned <bright-blue>{}</> (PID: {})",
                        instance_name,
                        child.pid()
                    );
                }

                client_children.push(child);
                if !params.silent {
                    success!(
                        "Spawned <bright-blue>{}</> in <yellow>{:?}</>",
                        &instance_name,
                        started.elapsed()
                    );
                }
            }

            // Character bots: created by the mod, on the server, now. They
            // show up in `rcon_players` at once, so the connect watcher below
            // finds them on its first poll; it is kept for its logging and
            // for the case where the mod created fewer than asked.
            if params.character_bots > 0 {
                let ids = rcon.spawn_bots(params.character_bots).await?;
                // Deliberately NOT gated on `silent`: this is the one line
                // that says what the bots are, the same way the mods
                // directory line says which mod shipped.
                info!(
                    "Using bot mode <bright-blue>characters</> ({} requested, ids {:?})",
                    params.character_bots, ids
                );
            }
            // Wait for all clients to actually connect to the server
            // Clients take 20-30 seconds to load sprites and connect
            if params.client_count > 0 && !params.silent {
                info!(
                    "Waiting for {} client(s) to connect...",
                    params.client_count
                );
            }
            let wait_started = Instant::now();
            let expected_players = if params.character_bots > 0 {
                params.character_bots as usize
            } else {
                params.client_count as usize
            };
            // Give up on a *stall*, not on a clock: clients take 25-30 s to
            // load sprites and arrive one after another, so an absolute budget
            // abandons a run that is still assembling itself. See
            // `connect_wait::CONNECT_STALL_TIMEOUT`.
            let mut watcher = ConnectWatcher::new(expected_players, wait_started);
            loop {
                let verdict = match rcon.connected_player_count().await {
                    Ok(count) => {
                        let verdict = watcher.observe(count, Instant::now());
                        if verdict == ConnectWait::Waiting && !params.silent {
                            info!("Clients: {}/{} connected", count, expected_players);
                        }
                        verdict
                    }
                    Err(e) => {
                        if !params.silent {
                            warn!("Error checking player count: {:?}", e);
                        }
                        // An unanswered poll is not evidence of a stall, but it
                        // must not stop the clock either: re-ask the watcher
                        // with what we already know.
                        watcher.observe(0, Instant::now())
                    }
                };
                match verdict {
                    ConnectWait::Satisfied => {
                        if !params.silent {
                            success!(
                                "All {} client(s) connected in <yellow>{:?}</>",
                                params.client_count,
                                wait_started.elapsed()
                            );
                        }
                        break;
                    }
                    ConnectWait::Stalled => {
                        // Best-effort naming: the poll above deliberately
                        // counts without deserialising, so ask once more for
                        // the ids. A failure here loses the names, not the
                        // warning.
                        let present: Vec<PlayerId> = match rcon.connected_players().await {
                            Ok(players) => players.iter().map(|p| p.player_id).collect(),
                            Err(e) => {
                                warn!("could not list the players that did connect: {:?}", e);
                                vec![]
                            }
                        };
                        let missing = missing_clients(expected_players, &present);
                        error!(
                            "Gave up waiting for clients after {:?} with no further progress: \
                             {}/{} have a character. Missing client(s): {:?}. \
                             The run continues without them.",
                            wait_started.elapsed(),
                            watcher.best(),
                            expected_players,
                            missing
                        );
                        break;
                    }
                    ConnectWait::Waiting => {}
                }
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }

            // Register client names with BotBridge after they're connected
            for instance_number in 0..params.client_count {
                let instance_name = format!("client{}", instance_number + 1);
                rcon.whoami(&instance_name).await?;
                // Execute a dummy command to silence the warning about "using commands will
                // disable achievements". If we don't do this, the first command will be lost
                rcon.silent_print("").await?;
            }

            arrange_windows(params.client_count).await?;
            if params.game_speed != 1.0 {
                rcon.set_game_speed(params.game_speed).await?;
                info!(
                    "Using game speed <bright-blue>{}</> (wall-clock deadlines scale with it)",
                    params.game_speed
                );
            }
            Ok(())
        }
        .await;
        if let Err(err) = startup {
            Self::kill_children(server_child, client_children);
            return Err(err);
        }
        Ok(FactorioInstance {
            client_processes: client_children,
            server_process: server_child,
            silent,
            world,
            rcon,
            map_exchange_string: params.map_exchange_string,
            seed: params.seed,
            server_host: params.server_host,
            server_port: factorio_port,
            rcon_port: rcon_settings.port,
            client_count: params.client_count,
        })
    }

    #[allow(clippy::too_many_arguments)]
    /// Puts a savepoint where the server can load it, and records that it
    /// did.
    ///
    /// The savepoint is **copied**, never loaded in place: the archived file
    /// lives inside the run record that explains it, and a running Factorio
    /// writing back over it -- an autosave, a nameless `/server-save` -- would
    /// destroy the only identifiable copy of that world. The copy is also not
    /// `level.zip`: that file is the map this workspace's whole measurement
    /// history rests on, it predates `--seed`, and nothing could recreate it.
    fn prepare_resume(instance_dir: &Path, marker: &ResumeMarker) -> Result<PathBuf> {
        let source = Path::new(&marker.source);
        if !source.is_file() {
            error!("savepoint missing at <bright-blue>{:?}</>", source);
            return Err(FactorioSavesNotFound {}.into());
        }
        let saves_path = instance_dir.join(PathBuf::from("saves"));
        std::fs::create_dir_all(&saves_path).into_diagnostic()?;
        let destination = saves_path.join(PathBuf::from("resumed.zip"));
        std::fs::copy(source, &destination).into_diagnostic()?;
        write_resume_marker(instance_dir, marker).into_diagnostic()?;
        info!(
            "Resuming from savepoint <bright-blue>{}</> ({:?}); this run is NOT comparable with a \
             fresh-world run, and --compare will refuse to try",
            marker.label, source
        );
        Ok(destination)
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_server(
        workspace_path: &str,
        rcon_settings: &RconSettings,
        factorio_port: Option<u16>,
        instance_name: &str,
        write_logs: bool,
        silent: Arc<parking_lot::RwLock<bool>>,
        wait_until: FactorioStartCondition,
        // The save to start from, already copied into this instance's saves
        // directory by `FactorioInstance::start`. `None` means the instance's
        // own `level.zip`, which is every ordinary run.
        resume_save: Option<PathBuf>,
    ) -> Result<(
        Arc<FactorioWorld>,
        Arc<FactorioRcon>,
        InteractiveProcess,
        u16,
    )> {
        let workspace_path = Path::new(&workspace_path);
        if !workspace_path.exists() {
            error!(
                "Failed to find workspace at <bright-blue>{:?}</>",
                workspace_path
            );
            return Err(WorkspaceNotFound {}.into());
        }
        let instance_path = workspace_path.join(PathBuf::from(instance_name));
        let instance_path = Path::new(&instance_path);
        if !instance_path.exists() {
            error!(
                "Failed to find instance at <bright-blue>{:?}</>",
                instance_path
            );
            return Err(FactorioInstanceNotFound {}.into());
        }
        let current_silent = *silent.read();
        let factorio_binary_path = io_utils::get_factorio_binary_path(instance_path);
        io_utils::await_lock(instance_path.join(PathBuf::from(".lock")), current_silent).await?;

        if !factorio_binary_path.exists() {
            error!(
                "factorio binary missing at <bright-blue>{:?}</>",
                factorio_binary_path
            );
            return Err(FactorioBinaryNotFound {}.into());
        }
        let saves_path = instance_path.join(PathBuf::from("saves"));
        if !saves_path.exists() {
            error!("saves missing at <bright-blue>{:?}</>", saves_path);
            return Err(FactorioSavesNotFound {}.into());
        }
        // The savepoint when one was named, otherwise the instance's own map.
        // Named explicitly rather than by overwriting `level.zip`, which is
        // this workspace's only copy of the map every measurement rests on and
        // which no seed can recreate.
        let saves_level_path = match resume_save {
            Some(path) => path,
            None => saves_path.join(PathBuf::from("level.zip")),
        };
        if !saves_level_path.exists() {
            error!(
                "save file missing at <bright-blue>{:?}</>",
                saves_level_path
            );
            // return Err(anyhow!("failed to find factorio saves/level.zip"));
            return Err(FactorioSavesNotFound {}.into());
        }
        let server_settings_path = instance_path.join(PathBuf::from(SERVER_SETTINGS_FILENAME));
        if !server_settings_path.exists() {
            error!(
                "server settings missing at <bright-blue>{:?}</>",
                server_settings_path
            );
            return Err(FactorioSettingsNotFound {}.into());
        }

        let factorio_port = factorio_port.unwrap_or(34197);
        let factorio_port_str = factorio_port.to_string();
        let rcon_port_str = rcon_settings.port.to_string();
        // only used to point macOS Factorio at the instance mods directory
        #[cfg(target_os = "macos")]
        let mods_path = instance_path.join("mods");
        #[cfg(target_os = "macos")]
        let mods_path_str = mods_path.to_str().unwrap();
        let config_path = instance_path.join("config").join("config.ini");
        let config_path_str = config_path.to_str().unwrap().to_string();
        // pushed to in the macOS-only block below
        #[allow(unused_mut)]
        let mut args = vec![
            "--start-server",
            saves_level_path.to_str().unwrap(),
            "--port",
            &factorio_port_str,
            "--rcon-port",
            &rcon_port_str,
            "--rcon-password",
            &rcon_settings.pass,
            "--server-settings",
            server_settings_path.to_str().unwrap(),
            "--config",
            &config_path_str,
        ];
        // macOS Factorio uses ~/Library/Application Support/factorio/mods by default
        // We need to explicitly point it to our instance's mods directory
        #[cfg(target_os = "macos")]
        {
            args.push("--mod-directory");
            args.push(mods_path_str);
        }
        if !current_silent {
            info!(
                "Starting <bright-blue>server</> at {:?} with {:?}",
                &instance_path, &args
            );
        }
        let mut command = Command::new(&factorio_binary_path);
        command.args(&args);
        let log_path = workspace_path.join(PathBuf::from_str("server-log.txt").unwrap());
        info!("start waiting");
        let (world, proc, rcon) = read_output(
            command,
            log_path,
            rcon_settings,
            write_logs,
            silent.clone(),
            wait_until,
        )
        .await?;
        info!("waiting finished");
        // await for factorio to start before returning

        Ok((world, Arc::new(rcon), proc, factorio_port))
    }

    async fn start_client(
        settings: &FactorioSettings,
        instance_name: String,
        server_host: Option<String>,
        write_logs: bool,
        silent: bool,
    ) -> Result<InteractiveProcess> {
        let workspace_path = settings.workspace_path.to_string();
        let workspace_path = Path::new(&workspace_path);
        if !workspace_path.exists() {
            error!(
                "Failed to find workspace at <bright-blue>{:?}</>",
                workspace_path
            );
            return Err(WorkspaceNotFound {}.into());
        }
        let instance_path = workspace_path.join(PathBuf::from(&instance_name));
        let instance_path = Path::new(&instance_path);
        if !instance_path.exists() {
            error!(
                "Failed to find instance at <bright-blue>{:?}</>",
                instance_path
            );
            return Err(FactorioInstanceNotFound {}.into());
        }
        let factorio_binary_path = io_utils::get_factorio_binary_path(instance_path);
        if !factorio_binary_path.exists() {
            error!(
                "factorio binary missing at <bright-blue>{:?}</>",
                factorio_binary_path
            );
            return Err(FactorioBinaryNotFound {}.into());
        }
        io_utils::await_lock(instance_path.join(PathBuf::from(".lock")), silent).await?;
        let mods_path = workspace_path.join("mods");
        let mods_path_str = mods_path.to_str().unwrap().to_string();
        let config_path = instance_path.join("config").join("config.ini");
        let config_path_str = config_path.to_str().unwrap().to_string();
        let server_host_str = server_host
            .clone()
            .unwrap_or_else(|| "localhost".to_owned());
        let args = &[
            "--mp-connect",
            &server_host_str,
            "--graphics-quality",
            "low",
            // "--force-graphics-preset", "very-low",
            // "--video-memory-usage", "low",

            // "--gfx-safe-mode",
            // "--low-vram",
            "--disable-audio",
            "--mod-directory",
            &mods_path_str,
            "--config",
            &config_path_str,
        ];
        if !silent {
            info!(
                "Starting <bright-blue>{}</> at {:?} with {:?}",
                &instance_name, &instance_path, &args
            );
        }

        let mut command = Command::new(&factorio_binary_path);
        command.args(args);

        // For macOS graphical clients, we need to use null stdio to avoid
        // interfering with GUI rendering. InteractiveProcess with piped stdio
        // causes GUI apps to fail silently on macOS.
        //
        // Nulling *stderr* as well is what made a client that dies during
        // startup -- the whole symptom of the broken `config.ini` this sits
        // next to -- indistinguishable from one that is merely slow to
        // connect. `--logs` promised a client log and never delivered one
        // (`write_logs` was accepted and dropped here), so under `-l` the two
        // output streams go to a file instead. A file is not a pipe, so the
        // macOS GUI workaround still holds.
        use std::process::Stdio;
        command.stdin(Stdio::null());
        let log_path = workspace_path.join(format!("{instance_name}-log.txt"));
        let log_file = if write_logs {
            match File::create(&log_path) {
                Ok(file) => Some(file),
                Err(err) => {
                    // Losing the log must not lose the client.
                    error!("failed to open <bright-blue>{log_path:?}</>: {err}");
                    None
                }
            }
        } else {
            None
        };
        match log_file {
            Some(file) => {
                let stderr = file.try_clone().into_diagnostic()?;
                command.stdout(Stdio::from(file));
                command.stderr(Stdio::from(stderr));
                if !silent {
                    info!("Writing <bright-blue>{:?}</>", &log_path);
                }
            }
            None => {
                command.stdout(Stdio::null());
                command.stderr(Stdio::null());
            }
        }

        // Spawn the child process directly
        let child = command.spawn().into_diagnostic()?;

        // Create a minimal InteractiveProcess wrapper for compatibility
        // We don't actually read from the process since GUI apps don't write to stdout
        let proc = InteractiveProcess::from_child(child);

        Ok(proc)
    }

    /// Kills the clients, then the server. Shared by [`FactorioInstance::stop`]
    /// and by the error path of [`FactorioInstance::start`], so a run that dies
    /// half-way through startup releases the ports exactly like a clean shutdown
    /// does.
    fn kill_children(server: Option<InteractiveProcess>, clients: Vec<InteractiveProcess>) {
        for child in clients {
            if child.close().kill().is_err() {
                error!("failed to kill client");
            }
        }
        if let Some(server) = server
            && server.close().kill().is_err()
        {
            error!("failed to kill server");
        }
    }

    pub fn stop(mut self) -> Result<()> {
        Self::kill_children(
            self.server_process.take(),
            std::mem::take(&mut self.client_processes),
        );
        Ok(())
    }
}

#[derive(PartialEq, Clone)]
pub enum FactorioStartCondition {
    Initialized,
    DiscoveryComplete,
}

// let handle = thread::spawn(move || {
//     let exit_code = child.wait().expect("failed to wait for client");
//     if let Some(code) = exit_code.code() {
//         error!(
//             "<red>{} stopped</> with exit code <yellow>{}</>",
//             &instance_name, code
//         );
//     } else {
//         error!("<red>{} stopped</> without exit code", &instance_name);
//     }
//     exit_code
// });
