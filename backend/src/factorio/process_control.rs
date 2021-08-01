use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::thread::{sleep, JoinHandle};
use std::time::{Duration, Instant};

use async_std::task;
use paris::Logger;
// use crate::factorio::ws::FactorioWebSocketServer;

use crate::factorio::instance_setup::setup_factorio_instance;
use crate::factorio::output_reader::read_output;
use crate::factorio::rcon::{FactorioRcon, RconSettings};
use crate::factorio::world::FactorioWorld;
use crate::settings::AppSettings;
use std::sync::mpsc::channel;

#[allow(clippy::too_many_arguments)]
pub async fn start_factorio(
    settings: &AppSettings,
    server_host: Option<&str>,
    client_count: u8,
    recreate: bool,
    map_exchange_string: Option<String>,
    seed: Option<String>,
    // websocket_server: Option<Addr<FactorioWebSocketServer>>,
    write_logs: bool,
    silent: bool,
) -> anyhow::Result<(Option<Arc<FactorioWorld>>, Arc<FactorioRcon>)> {
    let mut world: Option<Arc<FactorioWorld>> = None;
    let rcon_settings =
        RconSettings::new(settings.rcon_port as u16, &settings.rcon_pass, server_host);
    if server_host.is_none() {
        setup_factorio_instance(
            &settings.workspace_path,
            &rcon_settings,
            None,
            "server",
            true,
            recreate,
            map_exchange_string,
            seed,
            silent,
        )
        .await?;
    }
    let settings = settings.clone();
    for instance_number in 0..client_count {
        let instance_name = format!("client{}", instance_number + 1);
        if let Err(err) = setup_factorio_instance(
            &settings.workspace_path,
            &rcon_settings,
            None,
            &instance_name,
            false,
            false,
            None,
            None,
            silent,
        )
        .await
        {
            error!("Failed to setup Factorio <red>{}</>: ", err);
            break;
        }
    }

    let rcon = match server_host {
        None => {
            let started = Instant::now();
            let (_world, rcon, child) = start_factorio_server(
                &settings.workspace_path,
                &rcon_settings,
                None,
                "server",
                // websocket_server,
                write_logs,
                silent,
                FactorioStartCondition::Initialized,
            )
            .await?;
            world = Some(_world);
            report_child_death(child);
            if !silent {
                success!(
                    "Started <bright-blue>server</> in <yellow>{:?}</>",
                    started.elapsed()
                );
            }
            rcon
        }
        Some(_) => Arc::new(
            FactorioRcon::new(&rcon_settings, silent)
                .await
                .expect("failed to connect"),
        ),
    };
    for instance_number in 0..client_count {
        let instance_name = format!("client{}", instance_number + 1);
        let started = Instant::now();
        if let Err(err) = start_factorio_client(
            &settings,
            instance_name.clone(),
            server_host,
            write_logs,
            silent,
        )
        .await
        {
            error!("Failed to start Factorio <red>{}</>", err);
            break;
        }
        success!(
            "Started <bright-blue>{}</> in <yellow>{:?}</>",
            &instance_name,
            started.elapsed()
        );
        rcon.whoami(&instance_name).await.unwrap();
        // Execute a dummy command to silence the warning about "using commands will
        // disable achievements". If we don't do this, the first command will be lost
        rcon.silent_print("").await.unwrap();
    }
    Ok((world, rcon))
}

pub fn await_lock(lock_path: PathBuf, silent: bool) {
    if lock_path.exists() {
        match std::fs::remove_file(&lock_path) {
            Ok(_) => {}
            Err(_) => {
                let mut logger = Logger::new();
                if !silent {
                    logger.loading("Waiting for .lock to disappear");
                }
                let started = Instant::now();
                for _ in 0..1000 {
                    sleep(Duration::from_millis(1));
                    if std::fs::remove_file(&lock_path).is_ok() {
                        break;
                    }
                }
                if !lock_path.exists() {
                    if !silent {
                        logger.success(format!(
                            "Successfully awaited .lock in <yellow>{:?}</>",
                            started.elapsed()
                        ));
                    }
                } else {
                    logger.done();
                    error!("Factorio instance already running!");
                    std::process::exit(1);
                }
            }
        }
    }
}

#[derive(PartialEq, Clone)]
pub enum FactorioStartCondition {
    Initialized,
    DiscoveryComplete,
}

#[allow(clippy::too_many_arguments)]
pub async fn start_factorio_server(
    workspace_path: &str,
    rcon_settings: &RconSettings,
    factorio_port: Option<u16>,
    instance_name: &str,
    // websocket_server: Option<Addr<FactorioWebSocketServer>>,
    write_logs: bool,
    silent: bool,
    wait_until: FactorioStartCondition,
) -> anyhow::Result<(Arc<FactorioWorld>, Arc<FactorioRcon>, Child)> {
    let workspace_path = Path::new(&workspace_path);
    if !workspace_path.exists() {
        error!(
            "Failed to find workspace at <bright-blue>{:?}</>",
            workspace_path
        );
        std::process::exit(1);
    }
    let instance_path = workspace_path.join(PathBuf::from(instance_name));
    let instance_path = Path::new(&instance_path);
    if !instance_path.exists() {
        error!(
            "Failed to find instance at <bright-blue>{:?}</>",
            instance_path
        );
        std::process::exit(1);
    }
    let binary = if cfg!(windows) {
        "bin/x64/factorio.exe"
    } else {
        "bin/x64/factorio"
    };
    let factorio_binary_path = instance_path.join(PathBuf::from(binary));
    await_lock(instance_path.join(PathBuf::from(".lock")), silent);

    if !factorio_binary_path.exists() {
        error!(
            "factorio binary missing at <bright-blue>{:?}</>",
            factorio_binary_path
        );
        std::process::exit(1);
    }
    let saves_path = instance_path.join(PathBuf::from("saves"));
    if !saves_path.exists() {
        error!("saves missing at <bright-blue>{:?}</>", saves_path);
        std::process::exit(1);
    }
    let saves_level_path = saves_path.join(PathBuf::from("level.zip"));
    if !saves_level_path.exists() {
        error!(
            "save file missing at <bright-blue>{:?}</>",
            saves_level_path
        );
        std::process::exit(1);
    }
    let server_settings_path = instance_path.join(PathBuf::from("server-settings.json"));
    if !server_settings_path.exists() {
        error!(
            "server settings missing at <bright-blue>{:?}</>",
            server_settings_path
        );
        std::process::exit(1);
    }
    let args = &[
        "--start-server",
        saves_level_path.to_str().unwrap(),
        "--port",
        &factorio_port.unwrap_or(34197).to_string(),
        "--rcon-port",
        &rcon_settings.port.to_string(),
        "--rcon-password",
        &rcon_settings.pass,
        "--server-settings",
        server_settings_path.to_str().unwrap(),
    ];
    if !silent {
        info!(
            "Starting <bright-blue>server</> at {:?} with {:?}",
            &instance_path, &args
        );
    }
    let mut child = Command::new(&factorio_binary_path)
        .args(args)
        // .stdout(Stdio::from(outputs))
        // .stderr(Stdio::from(errors))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start server");

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let log_path = workspace_path.join(PathBuf::from_str("server-log.txt").unwrap());
    let (world, rcon) = read_output(
        reader,
        rcon_settings,
        log_path,
        // websocket_server,
        write_logs,
        silent,
        wait_until,
    )
    .await?;
    // await for factorio to start before returning

    Ok((world, rcon, child))
}

pub fn report_child_death(mut child: Child) {
    thread::spawn(move || {
        let exit_code = child.wait().expect("failed to wait for server");
        if let Some(code) = exit_code.code() {
            error!("<red>server stopped</> with exit code <yellow>{}</>", code);
        } else {
            error!("<red>server stopped</> without exit code");
        }
    });
}

pub async fn start_factorio_client(
    settings: &AppSettings,
    instance_name: String,
    server_host: Option<&str>,
    write_logs: bool,
    silent: bool,
) -> anyhow::Result<JoinHandle<ExitStatus>> {
    let workspace_path: String = settings.workspace_path.to_string();
    let workspace_path = Path::new(&workspace_path);
    if !workspace_path.exists() {
        error!(
            "Failed to find workspace at <bright-blue>{:?}</>",
            workspace_path
        );
        std::process::exit(1);
    }
    let instance_path = workspace_path.join(PathBuf::from(&instance_name));
    let instance_path = Path::new(&instance_path);
    if !instance_path.exists() {
        error!(
            "Failed to find instance at <bright-blue>{:?}</>",
            instance_path
        );
        std::process::exit(1);
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
        std::process::exit(1);
    }
    await_lock(instance_path.join(PathBuf::from(".lock")), silent);
    let args = &[
        "--mp-connect",
        server_host.unwrap_or("localhost"),
        "--graphics-quality",
        "low",
        // "--force-graphics-preset", "very-low",
        // "--video-memory-usage", "low",

        // "--gfx-safe-mode",
        // "--low-vram",
        "--disable-audio",
    ];
    info!(
        "Starting <bright-blue>{}</> at {:?} with {:?}",
        &instance_name, &instance_path, &args
    );
    let mut child = Command::new(&factorio_binary_path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start client");
    let instance_name = instance_name;
    let log_instance_name = instance_name.clone();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let log_filename = format!(
        "{}/{}-log.txt",
        workspace_path.to_str().unwrap(),
        instance_name
    );
    let mut log_file = match write_logs {
        true => Some(File::create(log_filename)?),
        false => None,
    };
    let handle = thread::spawn(move || {
        let exit_code = child.wait().expect("failed to wait for client");
        if let Some(code) = exit_code.code() {
            error!(
                "<red>{} stopped</> with exit code <yellow>{}</>",
                &instance_name, code
            );
        } else {
            error!("<red>{} stopped</> without exit code", &instance_name);
        }
        exit_code
    });
    let is_client = server_host.is_some();
    let (tx, rx) = channel();
    tx.send(())?;
    std::thread::spawn(move || {
        task::spawn(async move {
            let mut initialized = false;
            for line in reader.lines() {
                if let Ok(line) = line {
                    // wait for factorio init before sending confirmation
                    if !initialized && line.contains("my_client_id") {
                        initialized = true;
                        rx.recv().unwrap();
                        rx.recv().unwrap();
                    }
                    log_file.iter_mut().for_each(|log_file| {
                        // filter out 6.6 million lines like 6664601 / 6665150...
                        if initialized || !line.contains(" / ") {
                            log_file
                                .write_all(line.as_bytes())
                                .expect("failed to write log file");
                            log_file.write_all(b"\n").expect("failed to write log file");
                        }
                    });
                    if is_client && !line.contains(" / ") && !line.starts_with('§') {
                        info!("<cyan>{}</>⮞ <magenta>{}</>", &log_instance_name, line);
                    }
                } else {
                    error!("failed to read client log");
                    break;
                }
            }
        });
    });
    tx.send(())?;
    Ok(handle)
}
