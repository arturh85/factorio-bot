use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::{fs::File, sync::Arc};

// use crate::factorio::ws::FactorioWebSocketServer;
// use tokio::sync::mpsc::channel;

use crate::factorio::rcon::{FactorioRcon, RconSettings};
use crate::factorio::world::FactorioWorld;
use crate::process::output_parser::OutputParser;
use crate::process::process_control::FactorioStartCondition;
use crate::process::InteractiveProcess;
use miette::{IntoDiagnostic, Result};
use parking_lot::{Mutex, RwLock};
use std::sync::mpsc;

pub async fn read_output(
    cmd: Command,
    log_path: PathBuf,
    rcon_settings: &RconSettings,
    write_logs: bool,
    silent: Arc<RwLock<bool>>,
    wait_until: FactorioStartCondition,
) -> Result<(Arc<FactorioWorld>, InteractiveProcess, FactorioRcon)> {
    let log_file = Mutex::new(match write_logs {
        true => Some(File::create(log_path).into_diagnostic()?),
        false => None,
    });
    let mut output_parser = OutputParser::new();
    let wait_until_thread = wait_until.clone();
    let (tx1, rx1) = mpsc::channel();
    let (tx2, rx2) = mpsc::channel();
    let initialized = Mutex::new(false);
    let _world = output_parser.world();
    let _silent = silent.clone();
    let proc = InteractiveProcess::new_with_stderr(
        cmd,
        move |line| {
            match line {
                Ok(line) => {
                    let silent = *silent.read();
                    let mut initialized = initialized.lock();
                    // after we receive this line we can connect via rcon
                    if !*initialized && line.contains("my_client_id") {
                        tx1.send(()).expect("failed to send");
                        if wait_until_thread == FactorioStartCondition::Initialized {
                            *initialized = true;
                        }
                    }
                    // wait for factorio init before sending confirmation
                    if !*initialized
                        && (line.contains("initial discovery done") || line.contains("(100% done)"))
                    {
                        *initialized = true;
                        output_parser.on_init().unwrap();
                        tx2.send(()).expect("failed to send");
                    }
                    // filter out 6 million lines like 6664601 / 6665150
                    if *initialized || !line.contains(" / ") {
                        let mut log_file = log_file.lock();
                        log_file.iter_mut().for_each(|log_file| {
                            log_file
                                .write_all(line.as_bytes())
                                .expect("failed to write log file");
                            log_file.write_all(b"\n").expect("failed to write log file");
                        });

                        if !line.is_empty() && &line[0..2] == "§" {
                            if let Some(pos) = line[2..].find('§') {
                                let tick: u64 = (&line[2..pos + 2]).parse().unwrap();
                                let rest = &line[pos + 4..];
                                if let Some(pos) = rest.find('§') {
                                    let action = &rest[0..pos];
                                    let rest = &rest[pos + 2..];
                                    if !silent {
                                        match action {
                                            "on_player_changed_position"
                                            | "on_player_main_inventory_changed"
                                            | "on_player_changed_distance"
                                            | "entity_prototypes"
                                            | "recipes"
                                            | "force"
                                            | "item_prototypes"
                                            | "graphics"
                                            | "tiles"
                                            | "STATIC_DATA_END"
                                            | "entities" => {}
                                            _ => {
                                                info!(
                                                        "<cyan>server</>⮞ §{}§<bright-blue>{}</>§<green>{}</>",
                                                        tick, action, rest
                                                    );
                                            }
                                        }
                                    }

                                    // println!("get output_parser.lock {tick}: {action}");
                                    let result = output_parser.parse(tick, action, rest);
                                    // println!("output_parser.lock gotten");
                                    if let Err(err) = result {
                                        error!(
                                            "<red>failed to parse</> <bright-blue>'{}'</>",
                                            line
                                        );
                                        error!("<red>error: {:?}</>", err);
                                    }
                                }
                            }
                        } else if line.contains("Error") && !silent {
                            warn!("<cyan>server</>⮞ <red>{}</>", line);
                        } else if !silent {
                            // info!("<cyan>server</>⮞ <magenta>{}</>", line);
                            println!("{}", line);
                        }
                    }
                }
                Err(err) => {
                    error!("<red>failed to read server stdout: {:?}</>", err);
                }
            };
        },
        move |line| {
            match line {
                Ok(line) => {
                    warn!("<cyan>server</>⮞ <red>{}</>", line);
                }
                Err(err) => {
                    error!("<red>failed to read server stderr: {:?}</>", err);
                }
            };
        },
    ).into_diagnostic()?;
    rx1.recv().into_diagnostic()?;
    let rcon = FactorioRcon::new(rcon_settings, _silent)
        .await
        .expect("failed to rcon");
    rcon.initialize_server().await?;
    if wait_until == FactorioStartCondition::DiscoveryComplete {
        rx2.recv().into_diagnostic()?;
    }
    Ok((_world, proc, rcon))
}
