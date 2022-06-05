use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

// use crate::factorio::ws::FactorioWebSocketServer;
// use tokio::sync::mpsc::channel;

use crate::factorio::world::FactorioWorld;
use crate::process::output_parser::OutputParser;
use crate::process::process_control::FactorioStartCondition;
use interactive_process::InteractiveProcess;
use miette::{IntoDiagnostic, Result};
use parking_lot::Mutex;
use std::sync::mpsc::channel;

pub fn read_output(
    cmd: Command,
    log_path: PathBuf,
    // websocket_server: Option<Addr<FactorioWebSocketServer>>,
    write_logs: bool,
    silent: bool,
    wait_until: FactorioStartCondition,
) -> Result<(Arc<FactorioWorld>, InteractiveProcess)> {
    let log_file = Mutex::new(match write_logs {
        true => Some(File::create(log_path).into_diagnostic()?),
        false => None,
    });
    let output_parser: Arc<Mutex<OutputParser>> = Arc::new(Mutex::new(OutputParser::new()));
    let wait_until_thread = wait_until;
    let (tx1, rx1) = channel();
    tx1.send(()).into_diagnostic()?;
    let (tx2, rx2) = channel();
    tx2.send(()).into_diagnostic()?;
    let initialized = Mutex::new(false);
    let _output_parser = output_parser.clone();
    let proc = InteractiveProcess::new(cmd, move|line| {
        match line {
            Ok(line) => {
                let mut initialized = initialized.lock();
                // after we receive this line we can connect via rcon
                if !*initialized && line.contains("my_client_id") {
                    rx1.recv().unwrap();
                    if wait_until_thread == FactorioStartCondition::Initialized {
                        *initialized = true;
                    }
                }
                // wait for factorio init before sending confirmation
                if !*initialized
                    && (line.contains("initial discovery done") || line.contains("(100% done)"))
                {
                    *initialized = true;
                    output_parser.lock().on_init().unwrap();
                    if let Err(err) = rx2.recv() {
                        error!("recv error 1: {:?} ", err);
                    }
                    if let Err(err) = rx2.recv() {
                        error!("recv error 1: {:?} ", err);
                    }
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

                                let result = output_parser.lock().parse(tick, action, rest);
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
                error!("<red>failed to read server log: {}</>", err);
            }
        };
    })
    .unwrap();
    let world = _output_parser.lock().world();
    Ok((world, proc))
}
