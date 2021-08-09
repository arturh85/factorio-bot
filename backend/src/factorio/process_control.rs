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

pub struct InstanceState {
    pub world: Option<Arc<FactorioWorld>>,
    pub rcon: Arc<FactorioRcon>,
    pub server_process: Option<Child>,
    pub client_processes: Vec<Child>,
}

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
) -> anyhow::Result<InstanceState> {
    let mut world: Option<Arc<FactorioWorld>> = None;
    let rcon_settings =
        RconSettings::new(settings.rcon_port as u16, &settings.rcon_pass, server_host);
    if server_host.is_none() {
        setup_factorio_instance(
            &settings.workspace_path,
            &settings.factorio_archive_path,
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
            &settings.factorio_archive_path,
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
    let mut server_child = None;
    let mut client_children = vec![];

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
            // report_child_death(child);
            server_child = Some(child);
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
        let child = start_factorio_client(
            &settings,
            instance_name.clone(),
            server_host,
            write_logs,
            silent,
        )
        .await?;
        // report_child_death(child);
        client_children.push(child);
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

    #[cfg(windows)]
    {
        mod bindings {
            windows::include_bindings!();
        }
        use bindings::Windows::Win32::{
            Foundation::{BOOL, HWND, LPARAM, PWSTR},
            UI::WindowsAndMessaging::{
                EnumWindows, GetSystemMetrics, GetWindowTextW, MoveWindow, SM_CXMAXIMIZED,
                SM_CYMAXIMIZED,
            },
        };
        async_std::task::sleep(Duration::from_secs(client_count as u64)).await; // wait for window to be visible, hopefully

        static mut HWNDS: Vec<HWND> = Vec::new();
        // let mut factorio_hwnds: Vec<HWND> = vec![];
        extern "system" fn enum_window(window: HWND, _: LPARAM) -> BOOL {
            unsafe {
                // let foo: isize = store_hwnd.try_into().unwrap();
                // let mut pntr = foo as *const StoreHWNDs;
                // info!("DEBUG {:?}", store_hwnd.borrow());

                let mut text: [u16; 512] = [0; 512];
                let len = GetWindowTextW(window, PWSTR(text.as_mut_ptr()), text.len() as i32);

                let text = String::from_utf16_lossy(&text[..len as usize]);
                if !text.is_empty() && text.contains("Factorio ") && !text.contains("Factorio Bot")
                {
                    HWNDS.push(window);
                    info!("window {:?} {}", window, text);
                }

                BOOL(1)
            }
        }

        unsafe {
            // let pntr = &mut store as *mut StoreHWNDs;
            EnumWindows(Some(enum_window), LPARAM(0 as isize)).ok()?;
            info!("result {:?}", HWNDS);
            let max_width = GetSystemMetrics(SM_CXMAXIMIZED);
            let max_height = GetSystemMetrics(SM_CYMAXIMIZED);
            let count = HWNDS.len();
            for (index, window) in HWNDS.iter().enumerate() {
                let (x, y, w, h) = window_size(max_width, max_height, count, index);
                MoveWindow(window, x, y, w, h, BOOL(1)).unwrap();
            }
            HWNDS.clear();
        }
    }

    Ok(InstanceState {
        client_processes: client_children,
        server_process: server_child,
        world,
        rcon,
    })
}

pub fn window_size(
    width_full: i32,
    height_full: i32,
    client_count: usize,
    client_index: usize,
) -> (i32, i32, i32, i32) {
    if client_count == 1 {
        (0, 0, width_full, height_full)
    } else {
        // cut into two columns and as many rows as needed
        let cols = 2;
        let col_index = (client_index % 2) as i32;
        let rows = (client_count as f64 / 2.0).ceil() as i32;
        let row_index = (client_index / 2) as i32;
        let col_width = width_full / cols;
        let row_height = height_full / rows;
        (
            col_width * col_index,
            row_height * row_index,
            if client_count % 2 == 1 && client_index == client_count - 1 {
                width_full
            } else {
                col_width
            },
            row_height,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_size_1() {
        assert_eq!(window_size(800, 600, 1, 0), (0, 0, 800, 600));
    }

    #[test]
    fn test_window_size_2() {
        assert_eq!(window_size(800, 600, 2, 0), (0, 0, 400, 600));
        assert_eq!(window_size(800, 600, 2, 1), (400, 0, 400, 600));
    }

    #[test]
    fn test_window_size_3() {
        assert_eq!(window_size(800, 600, 3, 0), (0, 0, 400, 300));
        assert_eq!(window_size(800, 600, 3, 1), (400, 0, 400, 300));
        assert_eq!(window_size(800, 600, 3, 2), (0, 300, 800, 300));
    }

    #[test]
    fn test_window_size_4() {
        assert_eq!(window_size(800, 600, 4, 0), (0, 0, 400, 300));
        assert_eq!(window_size(800, 600, 4, 1), (400, 0, 400, 300));
        assert_eq!(window_size(800, 600, 4, 2), (0, 300, 400, 300));
        assert_eq!(window_size(800, 600, 4, 3), (400, 300, 400, 300));
    }
}

pub async fn await_lock(lock_path: PathBuf, silent: bool) -> anyhow::Result<()> {
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
                    #[cfg(windows)]
                    {
                        let mut kill_list: Vec<u32> = vec![];
                        process_list::for_each_process(|id, name| {
                            if let Some(name) = name.to_str() {
                                if name.contains("factorio.exe") {
                                    info!("killing process {}: \"{}\"", id, name);
                                    kill_list.push(id);
                                }
                            }
                        })?;
                        for id in kill_list {
                            heim::process::get(id).await?.kill().await?;
                        }
                    }
                    #[cfg(unix)]
                    {
                        return Err(anyhow!("Factorio instance already running!"));
                    }
                }
            }
        }
    }
    Ok(())
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
        return Err(anyhow!("failed to find workspace"));
    }
    let instance_path = workspace_path.join(PathBuf::from(instance_name));
    let instance_path = Path::new(&instance_path);
    if !instance_path.exists() {
        error!(
            "Failed to find instance at <bright-blue>{:?}</>",
            instance_path
        );
        return Err(anyhow!("failed to find instance"));
    }
    let binary = if cfg!(windows) {
        "bin/x64/factorio.exe"
    } else {
        "bin/x64/factorio"
    };
    let factorio_binary_path = instance_path.join(PathBuf::from(binary));
    await_lock(instance_path.join(PathBuf::from(".lock")), silent).await?;

    if !factorio_binary_path.exists() {
        error!(
            "factorio binary missing at <bright-blue>{:?}</>",
            factorio_binary_path
        );
        return Err(anyhow!("failed to find factorio binary"));
    }
    let saves_path = instance_path.join(PathBuf::from("saves"));
    if !saves_path.exists() {
        error!("saves missing at <bright-blue>{:?}</>", saves_path);
        return Err(anyhow!("failed to find factorio saves"));
    }
    let saves_level_path = saves_path.join(PathBuf::from("level.zip"));
    if !saves_level_path.exists() {
        error!(
            "save file missing at <bright-blue>{:?}</>",
            saves_level_path
        );
        return Err(anyhow!("failed to find factorio saves/level.zip"));
    }
    let server_settings_path = instance_path.join(PathBuf::from("server-settings.json"));
    if !server_settings_path.exists() {
        error!(
            "server settings missing at <bright-blue>{:?}</>",
            server_settings_path
        );
        return Err(anyhow!("server settings missing"));
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

pub fn report_child_death(mut child: Child) -> JoinHandle<ExitStatus> {
    thread::spawn(move || {
        let exit_code = child.wait().expect("failed to wait for child to end");
        if let Some(code) = exit_code.code() {
            error!("<red>server stopped</> with exit code <yellow>{}</>", code);
        } else {
            error!("<red>server stopped</> without exit code");
        }
        exit_code
    })
}

pub async fn start_factorio_client(
    settings: &AppSettings,
    instance_name: String,
    server_host: Option<&str>,
    write_logs: bool,
    silent: bool,
) -> anyhow::Result<Child> {
    let workspace_path: String = settings.workspace_path.to_string();
    let workspace_path = Path::new(&workspace_path);
    if !workspace_path.exists() {
        error!(
            "Failed to find workspace at <bright-blue>{:?}</>",
            workspace_path
        );
        return Err(anyhow!("failed to find workspace"));
    }
    let instance_path = workspace_path.join(PathBuf::from(&instance_name));
    let instance_path = Path::new(&instance_path);
    if !instance_path.exists() {
        error!(
            "Failed to find instance at <bright-blue>{:?}</>",
            instance_path
        );
        return Err(anyhow!("failed to find instance"));
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
        return Err(anyhow!("failed to find factorio binary"));
    }
    await_lock(instance_path.join(PathBuf::from(".lock")), silent).await?;
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
    Ok(child)
}
