use crate::constants::SERVER_SETTINGS_FILENAME;
use crate::errors::*;
use crate::factorio::rcon::{FactorioRcon, RconSettings};
use crate::factorio::world::FactorioWorld;
use crate::process::arrange_windows::arrange_windows;
use crate::process::instance_setup::setup_factorio_instance;
use crate::process::output_reader::read_output;
use crate::process::{io_utils, InteractiveProcess};
use crate::settings::FactorioSettings;
use miette::{IntoDiagnostic, Result};
use parking_lot::Mutex;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::mpsc::channel;
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
            settings.rcon_port as u16,
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
            Some(_) => Arc::new(
                FactorioRcon::new(&rcon_settings, silent.clone())
                    .await
                    .expect("failed to connect"),
            ),
        };
        for instance_number in 0..params.client_count {
            let instance_name = format!("client{}", instance_number + 1);
            let started = Instant::now();
            let child = Self::start_client(
                &settings,
                instance_name.clone(),
                params.server_host.clone(),
                params.write_logs,
                true,
            )
            .await?;
            // report_child_death(child);
            client_children.push(child);
            if !params.silent {
                success!(
                    "Started <bright-blue>{}</> in <yellow>{:?}</>",
                    &instance_name,
                    started.elapsed()
                );
            }
            rcon.whoami(&instance_name).await.unwrap();
            // Execute a dummy command to silence the warning about "using commands will
            // disable achievements". If we don't do this, the first command will be lost
            rcon.silent_print("").await.unwrap();
        }
        arrange_windows(params.client_count).await?;
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
        let binary = if cfg!(windows) {
            "bin/x64/factorio.exe"
        } else {
            "bin/x64/factorio"
        };
        let current_silent = *silent.read();
        let factorio_binary_path = instance_path.join(PathBuf::from(binary));
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
        let args = &[
            "--start-server",
            saves_level_path.to_str().unwrap(),
            "--port",
            &factorio_port.to_string(),
            "--rcon-port",
            &rcon_settings.port.to_string(),
            "--rcon-password",
            &rcon_settings.pass,
            "--server-settings",
            server_settings_path.to_str().unwrap(),
        ];
        if !current_silent {
            info!(
                "Starting <bright-blue>server</> at {:?} with {:?}",
                &instance_path, &args
            );
        }
        let mut command = Command::new(&factorio_binary_path);
        command.args(args);
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
        let binary = if cfg!(windows) {
            "bin/x64/factorio.exe"
        } else {
            "bin/x64/factorio"
        };
        let factorio_binary_path = instance_path.join(PathBuf::from(binary));
        if !factorio_binary_path.exists() {
            error!(
                "factorio binary missing at <bright-blue>{:?}</>",
                factorio_binary_path
            );
            return Err(FactorioBinaryNotFound {}.into());
        }
        io_utils::await_lock(instance_path.join(PathBuf::from(".lock")), silent).await?;
        let args = &[
            "--mp-connect",
            &server_host
                .clone()
                .unwrap_or_else(|| "localhost".to_owned()),
            "--graphics-quality",
            "low",
            // "--force-graphics-preset", "very-low",
            // "--video-memory-usage", "low",

            // "--gfx-safe-mode",
            // "--low-vram",
            "--disable-audio",
        ];
        if !silent {
            info!(
                "Starting <bright-blue>{}</> at {:?} with {:?}",
                &instance_name, &instance_path, &args
            );
        }

        let mut command = Command::new(&factorio_binary_path);
        command.args(args);
        let instance_name = instance_name;
        let log_instance_name = instance_name.clone();
        // let stdout = child.stdout.take().unwrap();
        // let reader = BufReader::new(stdout);
        let log_filename = format!(
            "{}/{}-log.txt",
            workspace_path.to_str().unwrap(),
            instance_name
        );
        let log_file: Arc<Mutex<Option<File>>> = Arc::new(Mutex::new(match write_logs {
            true => Some(File::create(log_filename).into_diagnostic()?),
            false => None,
        }));
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
        let is_client = server_host.is_some();

        let initialized = Mutex::new(false);
        let (tx, rx) = channel();
        tx.send(()).into_diagnostic()?;

        let proc = InteractiveProcess::new_with_stderr(
            command,
            move |line| {
                match line {
                    Ok(line) => {
                        let mut initialized = initialized.lock();
                        // wait for factorio init before sending confirmation
                        if !*initialized && line.contains("my_client_id") {
                            *initialized = true;
                            rx.recv().unwrap();
                            rx.recv().unwrap();
                        }
                        let mut log_file = log_file.lock();
                        log_file.iter_mut().for_each(|log_file| {
                            // filter out 6.6 million lines like 6664601 / 6665150...
                            if *initialized || !line.contains(" / ") {
                                log_file
                                    .write_all(line.as_bytes())
                                    .expect("failed to write log file");
                                log_file.write_all(b"\n").expect("failed to write log file");
                            }
                        });
                        if is_client && !line.contains(" / ") && !line.starts_with('§') {
                            info!("<cyan>{}</>⮞ <magenta>{}</>", &log_instance_name, line);
                        }
                    }
                    Err(err) => {
                        error!("<red>failed to read client stdout: {:?}</>", err);
                    }
                };
            },
            move |line| {
                match line {
                    Ok(line) => {
                        warn!("<cyan>client</>⮞ <red>{}</>", line);
                    }
                    Err(err) => {
                        error!("<red>failed to read client stderr: {:?}</>", err);
                    }
                };
            },
        )
        .unwrap();
        tx.send(()).into_diagnostic()?;
        Ok(proc)
    }

    pub fn stop(mut self) -> Result<()> {
        for child in self.client_processes {
            if child.close().kill().is_err() {
                error!("failed to kill client");
            }
        }
        if let Some(server) = self.server_process.take() {
            if server.close().kill().is_err() {
                error!("failed to kill server");
            }
        }
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
