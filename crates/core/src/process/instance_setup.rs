use paris::Logger;
use serde_json::Value;
use std::fs;
use std::fs::{read_to_string, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;

use crate::constants::{
    MAP_GEN_SETTINGS_FILENAME, MAP_SETTINGS_FILENAME, MODS_FOLDERNAME, SERVER_SETTINGS_FILENAME,
};
use crate::errors::*;
use crate::factorio::rcon::RconSettings;
use crate::factorio::util::{read_to_value, write_value_to};
use crate::process::io_utils::{await_lock, extract_archive, symlink};
use crate::process::output_reader::read_output;
use crate::process::process_control::FactorioStartCondition;
use miette::{miette, IntoDiagnostic, Result};
use parking_lot::RwLock;
use tokio::fs::create_dir;

#[cfg(not(debug_assertions))]
pub const MODS_CONTENT: include_dir::Dir = include_dir!("mods");
#[cfg(not(debug_assertions))]
pub const PLANS_CONTENT: include_dir::Dir = include_dir!("scripts");

#[allow(clippy::too_many_arguments)]
pub async fn setup_factorio_instance(
    workspace_path_str: &str,
    factorio_archive_path: &str,
    rcon_settings: &RconSettings,
    factorio_port: Option<u16>,
    instance_name: &str,
    is_server: bool,
    recreate_save: bool,
    map_exchange_string: Option<String>,
    seed: Option<String>,
    silent: bool,
) -> Result<()> {
    if workspace_path_str.is_empty() {
        return Err(miette!("no workspace configured"));
    }
    if factorio_archive_path.is_empty() {
        return Err(miette!("no factorio archive configured"));
    }
    let workspace_path = Path::new(&workspace_path_str);
    if !workspace_path.exists() {
        error!(
            "Failed to find workspace at <bright-blue>{:?}</>",
            workspace_path
        );
        return Err(WorkspaceNotFound {}.into());
    }
    let workspace_data_path = workspace_path.join(PathBuf::from("data"));
    let instance_path = workspace_path.join(PathBuf::from(instance_name));
    let instance_path = Path::new(&instance_path);
    if !instance_path.exists() {
        if !silent {
            info!("Creating <bright-blue>{:?}</>", &instance_path);
        }
        create_dir(instance_path).await.into_diagnostic()?;
    }
    let readdir = instance_path.read_dir().into_diagnostic()?;
    if readdir.count() == 0 {
        extract_archive(factorio_archive_path, instance_path, workspace_path)?;
    }
    #[allow(unused_mut)]
    let mut workspace_mods_path = workspace_path.join(PathBuf::from(MODS_FOLDERNAME));
    if !workspace_mods_path.exists() {
        #[cfg(debug_assertions)]
        {
            workspace_mods_path = PathBuf::from(format!("../../{}", MODS_FOLDERNAME));
        }
        #[cfg(not(debug_assertions))]
        {
            std::fs::create_dir_all(&workspace_mods_path).into_diagnostic()?;
            if let Err(err) = MODS_CONTENT.extract(workspace_mods_path.clone()) {
                error!("failed to extract static mods content: {:?}", err);
                return Err(ModExtractFailed {}.into());
            }
        }
        if !workspace_mods_path.exists() {
            workspace_mods_path = PathBuf::from(MODS_FOLDERNAME);
            if !workspace_mods_path.exists() {
                return Err(MissingModsFolder {}.into());
            }
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let data_plans_path = workspace_path.join(PathBuf::from("plans"));
        if !data_plans_path.exists() {
            std::fs::create_dir_all(&data_plans_path).into_diagnostic()?;
            if let Err(err) = PLANS_CONTENT.extract(data_plans_path.clone()) {
                error!("failed to extract static plans content: {:?}", err);
                return Err(PlansExtractFailed {}.into());
            }
        }
    }

    let workspace_mods_path = fs::canonicalize(workspace_mods_path).into_diagnostic()?;
    let mods_path = instance_path.join(PathBuf::from(MODS_FOLDERNAME));
    if !mods_path.exists() {
        if !silent {
            info!("Creating Symlink for <bright-blue>{:?}</>", &mods_path);
        }
        symlink(&workspace_mods_path, &mods_path)?;
    }
    let instance_data_path = instance_path.join(PathBuf::from("data"));
    if !instance_data_path.exists() && workspace_data_path.exists() {
        let workspace_data_path = fs::canonicalize(workspace_data_path).into_diagnostic()?;
        if !silent {
            info!(
                "Creating Symlink for <bright-blue>{:?}</>",
                &instance_data_path
            );
        }
        symlink(&workspace_data_path, &instance_data_path)?;
    }
    // delete server/script-output/*
    // let script_output_put = instance_path.join(PathBuf::from("script-output"));
    // if script_output_put.exists() {
    //     for entry in fs::read_dir(script_output_put)? {
    //         let entry = entry.unwrap();
    //         std::fs::remove_file(entry.path())
    //             .unwrap_or_else(|_| panic!("failed to delete {}", entry.path().to_str().unwrap()));
    //     }
    // }
    if is_server {
        let server_settings_path = instance_path.join(PathBuf::from(SERVER_SETTINGS_FILENAME));
        if !server_settings_path.exists() {
            let server_settings_data = include_bytes!("../data/server-settings.json");
            let mut outfile = File::create(&server_settings_path).into_diagnostic()?;
            if !silent {
                info!("Creating <bright-blue>{:?}</>", &server_settings_path);
            }
            // io::copy(&mut template_file, &mut outfile)?;
            outfile.write_all(server_settings_data).into_diagnostic()?;
        }

        let saves_path = instance_path.join(PathBuf::from("saves"));
        if !saves_path.exists() {
            if !silent {
                info!("Creating <bright-blue>{:?}</>", &saves_path);
            }
            create_dir(&saves_path).await.into_diagnostic()?;
        }

        let saves_level_path = saves_path.join(PathBuf::from("level.zip"));
        let map_exchange_string_path = instance_path.join(PathBuf::from("map-exchange-string.txt"));
        if let Some(map_exchange_string) = &map_exchange_string {
            if !map_exchange_string_path.exists()
                || read_to_string(&map_exchange_string_path)
                    .into_diagnostic()?
                    .ne(map_exchange_string)
            {
                if !saves_level_path.exists() {
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
                    let mut args = vec!["--create", saves_level_path.to_str().unwrap()];
                    if let Some(seed) = seed.as_ref() {
                        args.push("--map-gen-seed");
                        args.push(seed);
                    }
                    let output = Command::new(&factorio_binary_path)
                        .args(args)
                        .output()
                        .expect("failed to run factorio --create");
                    if !saves_level_path.exists() {
                        error!(
                            "failed to create factorio level. Output: \n\n{}\n\n{}",
                            std::str::from_utf8(&output.stdout).unwrap(),
                            std::str::from_utf8(&output.stderr).unwrap()
                        );
                        return Err(FactorioLevelFailed {}.into());
                    }
                }
                update_map_gen_settings(
                    workspace_path_str,
                    instance_name,
                    factorio_port,
                    rcon_settings,
                    map_exchange_string,
                    silent,
                )
                .await?;
                File::create(&map_exchange_string_path)
                    .into_diagnostic()?
                    .write_all(map_exchange_string.as_ref())
                    .into_diagnostic()?;
            }
        }

        if saves_level_path.exists() && recreate_save {
            fs::remove_file(&saves_level_path).unwrap_or_else(|_| {
                panic!("failed to delete {}", &saves_level_path.to_str().unwrap())
            });
        }
        if !saves_level_path.exists() {
            let mut logger = Logger::new();
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
            let mut args = vec!["--create", saves_level_path.to_str().unwrap()];
            if let Some(seed) = &seed {
                args.push("--map-gen-seed");
                args.push(seed);
            }
            let map_gen_settings_path = format!(
                "{}/{}",
                instance_path.to_str().unwrap(),
                MAP_GEN_SETTINGS_FILENAME
            );
            let map_settings_path = format!(
                "{}/{}",
                instance_path.to_str().unwrap(),
                MAP_SETTINGS_FILENAME
            );
            if map_exchange_string.is_some() {
                args.push("--map-gen-settings");
                args.push(&map_gen_settings_path);
                args.push("--map-settings");
                args.push(&map_settings_path);
            }
            await_lock(instance_path.join(PathBuf::from(".lock")), silent).await?;
            if !silent {
                logger.loading(format!(
                    "Creating Level at <bright-blue>{:?}</>...",
                    &saves_level_path
                ));
            }

            let output = Command::new(&factorio_binary_path)
                .args(args)
                .output()
                .expect("failed to run factorio --create");

            if !saves_level_path.exists() {
                error!(
                    "failed to create factorio level. Output: \n\n{}\n\n{}",
                    std::str::from_utf8(&output.stdout).unwrap(),
                    std::str::from_utf8(&output.stderr).unwrap()
                );
                return Err(FactorioLevelFailed {}.into());
            }
            if !silent {
                logger.success(format!(
                    "Created Level at <bright-blue>{:?}</>",
                    &saves_level_path
                ));
            }
        }
    } else {
        let player_data_path = instance_path.join(PathBuf::from("player-data.json"));
        if !player_data_path.exists() {
            let player_data = include_bytes!("../data/player-data.json");
            let mut outfile = File::create(&player_data_path).into_diagnostic()?;
            outfile.write_all(player_data).into_diagnostic()?;
            if !silent {
                info!("Created <bright-blue>{:?}</>", &player_data_path);
            }
        }
        let mut value: Value = read_to_value(&player_data_path)?;
        value["service-username"] = Value::from(instance_name);
        let player_data_file = File::create(&player_data_path).into_diagnostic()?;
        serde_json::to_writer_pretty(player_data_file, &value).into_diagnostic()?;

        let config_path = instance_path.join(PathBuf::from("config"));
        if !config_path.exists() {
            create_dir(&config_path).await.into_diagnostic()?;
            let config_ini_data = include_bytes!("../data/config.ini");
            let config_ini_path = config_path.join(PathBuf::from("config.ini"));
            let mut outfile = File::create(&config_ini_path).into_diagnostic()?;
            outfile.write_all(config_ini_data).into_diagnostic()?;
            if !silent {
                info!("Created <bright-blue>{:?}</>", &config_ini_path);
            }
        }
    }
    Ok(())
}

pub async fn update_map_gen_settings(
    workspace_path: &str,
    instance_name: &str,
    factorio_port: Option<u16>,
    rcon_settings: &RconSettings,
    map_exchange_string: &str,
    silent: bool,
) -> Result<()> {
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
    let factorio_binary_path = instance_path.join(PathBuf::from(binary));
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
    let server_settings_path = instance_path.join(PathBuf::from(SERVER_SETTINGS_FILENAME));
    if !server_settings_path.exists() {
        error!(
            "server settings missing at <bright-blue>{:?}</>",
            server_settings_path
        );
        return Err(FactorioSettingsNotFound {}.into());
    }
    await_lock(instance_path.join(PathBuf::from(".lock")), silent).await?;
    let mut logger = Logger::new();
    if !silent {
        logger.loading(format!(
            "Updating <bright-blue>{}</> and <bright-blue>{}</>",
            MAP_SETTINGS_FILENAME, MAP_GEN_SETTINGS_FILENAME
        ));
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
    let log_path = workspace_path.join(PathBuf::from_str("server-log.txt").unwrap());
    let mut command = Command::new(&factorio_binary_path);
    command.args(args);
    let (_, proc, rcon) = read_output(
        command,
        log_path,
        rcon_settings,
        false,
        Arc::new(RwLock::new(true)),
        FactorioStartCondition::Initialized,
    )
    .await?;
    rcon.parse_map_exchange_string(MAP_GEN_SETTINGS_FILENAME, map_exchange_string)
        .await?;
    proc.close().kill().into_diagnostic()?;
    let target_map_gen_settings_path =
        instance_path.join(PathBuf::from_str(MAP_GEN_SETTINGS_FILENAME).unwrap());
    let target_map_settings_path =
        instance_path.join(PathBuf::from_str(MAP_SETTINGS_FILENAME).unwrap());
    let script_output_path = instance_path.join(PathBuf::from_str("script-output").unwrap());
    let source_map_gen_settings_path =
        script_output_path.join(PathBuf::from_str(MAP_GEN_SETTINGS_FILENAME).unwrap());
    let value: Value = read_to_value(&source_map_gen_settings_path)?;
    write_value_to(&value["map_settings"], &target_map_settings_path)?;
    write_value_to(&value["map_gen_settings"], &target_map_gen_settings_path)?;
    fs::remove_file(&source_map_gen_settings_path)
        .unwrap_or_else(|_| panic!("failed to delete {:?}", &source_map_gen_settings_path));

    if !silent {
        logger.success(format!(
            "Updated <bright-blue>{}</> and <bright-blue>{}</>",
            MAP_SETTINGS_FILENAME, MAP_GEN_SETTINGS_FILENAME
        ));
    }
    Ok(())
}
