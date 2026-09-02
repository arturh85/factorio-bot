use crate::constants::SERVER_SETTINGS_FILENAME;
use crate::errors::*;
use crate::factorio::rcon::{FactorioRcon, RconSettings};
use crate::factorio::world::FactorioWorld;
use crate::process::arrange_windows::arrange_windows;
use crate::process::connect_wait::{ConnectWait, ConnectWatcher, missing_clients};
use crate::process::instance_setup::setup_factorio_instance;
use crate::process::output_reader::read_output;
use crate::process::{InteractiveProcess, io_utils};
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

            // Wait for all clients to actually connect to the server
            // Clients take 20-30 seconds to load sprites and connect
            if params.client_count > 0 && !params.silent {
                info!(
                    "Waiting for {} client(s) to connect...",
                    params.client_count
                );
            }
            let wait_started = Instant::now();
            let expected_players = params.client_count as usize;
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
    async fn start_server(
        workspace_path: &str,
        rcon_settings: &RconSettings,
        factorio_port: Option<u16>,
        instance_name: &str,
        write_logs: bool,
        silent: Arc<parking_lot::RwLock<bool>>,
        wait_until: FactorioStartCondition,
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
        let saves_level_path = saves_path.join(PathBuf::from("level.zip"));
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
