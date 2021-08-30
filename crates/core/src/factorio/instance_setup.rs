use std::fs;
use std::fs::{read_to_string, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Instant;

use async_std::fs::create_dir;
use indicatif::HumanDuration;
use paris::Logger;
use serde_json::Value;

use crate::errors::*;
use crate::factorio::output_reader::read_output;
use crate::factorio::process_control::{await_lock, FactorioStartCondition};
use crate::factorio::rcon::RconSettings;
use crate::factorio::util::{read_to_value, write_value_to};
use miette::{DiagnosticResult, IntoDiagnostic};

#[cfg(not(debug_assertions))]
pub const MODS_CONTENT: include_dir::Dir = include_dir!("../../mods");
#[cfg(not(debug_assertions))]
pub const PLANS_CONTENT: include_dir::Dir = include_dir!("../../scripts");

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
) -> DiagnosticResult<()> {
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
        create_dir(instance_path)
            .await
            .into_diagnostic("factorio::instance_setup::could_not_create_instance_path")?;
    }
    let readdir = instance_path
        .read_dir()
        .into_diagnostic("factorio::instance_setup::could_not_read_instance_dir")?;
    if readdir.count() == 0 {
        let started = Instant::now();

        #[cfg(windows)]
        {
            use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
            let file = fs::File::open(factorio_archive_path)
                .into_diagnostic("factorio::instance_setup::could_not_open_archive_path")?;
            info!(
                "Extracting <bright-blue>{}</> to <magenta>{}</>",
                &factorio_archive_path,
                instance_path.to_str().unwrap()
            );

            let mut archive = zip::ZipArchive::new(file)
                .into_diagnostic("factorio::instance_setup::could_not_open_zip")?;

            let mut files: Vec<String> = vec![];
            for i in 0..archive.len() {
                files.push(
                    archive
                        .by_index(i)
                        .into_diagnostic("factorio::instance_setup::could_not_read_zip_entry")?
                        .name()
                        .into(),
                );
            }
            if workspace_data_path.exists() {
                files = files
                    .into_iter()
                    .filter(|file| !file.contains("/data/"))
                    .collect();
            }
            let bar = ProgressBar::new(files.len() as u64);
            bar.set_draw_target(ProgressDrawTarget::stdout());
            bar.set_style(
                ProgressStyle::default_spinner().template("{msg}\n{wide_bar} {pos}/{len}"),
            );
            for file in files {
                let message = format!("extracting {}", &file);
                bar.set_message(message);
                bar.tick();
                // output_path is like Factorio_0.18.36\bin\x64\factorio.exe
                let output_path = PathBuf::from(&file);
                // output_path is like bin\x64\factorio.exe
                let output_path = output_path
                    .strip_prefix(output_path.components().next().unwrap())
                    .into_diagnostic("factorio::instance_setup::strip_prefix")?;
                // output_path is like $instance_path\bin\x64\factorio.exe
                let output_path = PathBuf::from(instance_path).join(PathBuf::from(output_path));

                if (&*file).ends_with('/') {
                    fs::create_dir_all(&output_path)
                        .into_diagnostic("factorio::instance_setup::could_not_create_unpack_dir")?;
                } else {
                    if let Some(p) = output_path.parent() {
                        if !p.exists() {
                            fs::create_dir_all(&p).into_diagnostic(
                                "factorio::instance_setup::could_not_create_dir",
                            )?;
                        }
                    }

                    let mut outfile = fs::File::create(&output_path).unwrap();
                    let mut file = archive.by_name(&file).unwrap();
                    std::io::copy(&mut file, &mut outfile).unwrap();
                }
                bar.inc(1);
            }
            if !workspace_data_path.exists() {
                let instance_data_path = instance_path.join(PathBuf::from("data"));
                fs::rename(&instance_data_path, &workspace_data_path)
                    .into_diagnostic("factorio::instance_setup::could_not_rename_data")?;
            }
            bar.finish();
        }

        #[cfg(unix)]
        {
            let archive_path = PathBuf::from_str(factorio_archive_path)
                .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
            let tar_path = archive_path.with_extension("");
            if !tar_path.exists() {
                let mut logger = Logger::new();
                logger.loading(format!(
                    "Uncompressing <bright-blue>{}</> to <magenta>{}</> ...",
                    &archive_path.to_str().unwrap(),
                    tar_path.to_str().unwrap()
                ));

                let tar_gz = File::open(&archive_path)
                    .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
                let tar = xz2::read::XzDecoder::new(tar_gz);
                let mut archive = tar::Archive::new(tar);
                archive.unpack(&tar_path).expect("failed to decompress xz");
                logger.success(format!(
                    "Uncompressed <bright-blue>{}</> to <magenta>{}</>",
                    &archive_path.to_str().unwrap(),
                    tar_path.to_str().unwrap()
                ));
            }
            let mut logger = Logger::new();
            logger.loading(format!(
                "Extracting <bright-blue>{}</> to <magenta>{}</> ...",
                &tar_path.to_str().unwrap(),
                workspace_path.to_str().unwrap()
            ));
            // FIXME: what did this do ...?
            // let mut archive = archiver_rs::Tar::open(&tar_path).unwrap();
            // archive.extract(workspace_path).expect("failed to extract");
            logger.success("Extraction finished");

            let extracted_path = workspace_path.join(PathBuf::from("factorio"));
            if extracted_path.exists() {
                std::fs::remove_dir(&instance_path).expect("failed to delete empty folder");
                std::fs::rename(&extracted_path, instance_path).expect("failed to rename");
                success!("Renamed {:?} to {:?}", &extracted_path, instance_path);
            } else {
                error!("Failed to find {:?}", &extracted_path);
            }

            let instance_data_path = instance_path.join(PathBuf::from("data"));
            if !workspace_data_path.exists() {
                fs::rename(&instance_data_path, &workspace_data_path)
                    .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
            } else {
                std::fs::remove_dir_all(&instance_data_path).expect("failed to delete data folder");
            }
        }
        info!(
            "Extracting took <yellow>{}</>",
            HumanDuration(started.elapsed())
        );
    }
    #[allow(unused_mut)]
    let mut workspace_mods_path = workspace_path.join(PathBuf::from("mods"));
    if !workspace_mods_path.exists() {
        #[cfg(debug_assertions)]
        {
            workspace_mods_path = PathBuf::from("../../mods");
        }
        #[cfg(not(debug_assertions))]
        {
            std::fs::create_dir_all(&workspace_mods_path)
                .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
            if let Err(err) = MODS_CONTENT.extract(workspace_mods_path.clone()) {
                error!("failed to extract static mods content: {:?}", err);
                return Err(ModExtractFailed {}.into());
            }
        }
        if !workspace_mods_path.exists() {
            return Err(MissingModsFolder {}.into());
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let data_plans_path = workspace_path.join(PathBuf::from("plans"));
        if !data_plans_path.exists() {
            std::fs::create_dir_all(&data_plans_path)
                .into_diagnostic("factorio::output_parser::could_not_canonicalize")?;
            if let Err(err) = PLANS_CONTENT.extract(data_plans_path.clone()) {
                error!("failed to extract static plans content: {:?}", err);
                return Err(PlansExtractFailed {}.into());
            }
        }
    }

    let workspace_mods_path = std::fs::canonicalize(workspace_mods_path)
        .into_diagnostic("factorio::instance_setup::could_not_canonicalize")?;
    let mods_path = instance_path.join(PathBuf::from("mods"));
    if !mods_path.exists() {
        if !silent {
            info!("Creating Symlink for <bright-blue>{:?}</>", &mods_path);
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&workspace_mods_path, &mods_path)
                .into_diagnostic("factorio::instance_setup::could_not_create_symlink")?;
        }
        #[cfg(windows)]
        {
            let status = runas::Command::new("cmd.exe")
                .arg("/C")
                .arg("mklink")
                .arg("/D")
                .arg(&mods_path)
                .arg(&workspace_mods_path)
                .status()
                .into_diagnostic("factorio::instance_setup::could_not_create_symlink")?;
            // std::os::windows::fs::symlink_dir(&data_mods_path, &mods_path)?;
            if !status.success() {
                // return Err(anyhow!(
                //     "failed to create factorio mods symlink: {:?} -> {:?} ... {}",
                //     &mods_path,
                //     &workspace_mods_path,
                //     status.to_string()
                // ));
                return Err(ModSymlinkFailed {}.into());
            }
        }
    }
    let instance_data_path = instance_path.join(PathBuf::from("data"));
    if !instance_data_path.exists() && workspace_data_path.exists() {
        let workspace_data_path = std::fs::canonicalize(workspace_data_path)
            .into_diagnostic("factorio::instance_setup::could_not_canonicalize")?;
        if !silent {
            info!(
                "Creating Symlink for <bright-blue>{:?}</>",
                &instance_data_path
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&workspace_data_path, &instance_data_path)
                .into_diagnostic("factorio::instance_setup::could_not_create_symlink")?;
        }
        #[cfg(windows)]
        {
            let status = runas::Command::new("cmd.exe")
                .arg("/C")
                .arg("mklink")
                .arg("/D")
                .arg(&instance_data_path)
                .arg(&workspace_data_path)
                .status()
                .into_diagnostic("factorio::instance_setup::could_not_create_symlink")?;
            // std::os::windows::fs::symlink_dir(&workspace_data_path, &instance_data_path)?;
            if !status.success() {
                // return Err(anyhow!(
                //     "failed to create factorio data symlink: {:?} -> {:?} ... {}",
                //     &instance_data_path,
                //     &workspace_data_path,
                //     status.to_string()
                // ));
                return Err(ModSymlinkFailed {}.into());
            }
        }
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
        let server_settings_path = instance_path.join(PathBuf::from("server-settings.json"));
        if !server_settings_path.exists() {
            let server_settings_data = include_bytes!("../data/server-settings.json");
            let mut outfile = fs::File::create(&server_settings_path)
                .into_diagnostic("factorio::instance_setup::could_not_create_settings")?;
            if !silent {
                info!("Creating <bright-blue>{:?}</>", &server_settings_path);
            }
            // io::copy(&mut template_file, &mut outfile)?;
            outfile
                .write_all(server_settings_data)
                .into_diagnostic("factorio::instance_setup::could_not_write_settings")?;
        }

        let saves_path = instance_path.join(PathBuf::from("saves"));
        if !saves_path.exists() {
            if !silent {
                info!("Creating <bright-blue>{:?}</>", &saves_path);
            }
            create_dir(&saves_path)
                .await
                .into_diagnostic("factorio::instance_setup::could_not_create_saves_directory")?;
        }

        let saves_level_path = saves_path.join(PathBuf::from("level.zip"));
        let map_exchange_string_path = instance_path.join(PathBuf::from("map-exchange-string.txt"));
        if let Some(map_exchange_string) = &map_exchange_string {
            if !map_exchange_string_path.exists()
                || read_to_string(&map_exchange_string_path)
                    .into_diagnostic(
                        "factorio::instance_setup::could_not_read_map_exchange_string_path",
                    )?
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
                fs::File::create(&map_exchange_string_path)
                    .into_diagnostic(
                        "factorio::instance_setup::could_not_create_map_exchange_string_path",
                    )?
                    .write_all(map_exchange_string.as_ref())
                    .into_diagnostic(
                        "factorio::instance_setup::could_not_write_map_exchange_string_path",
                    )?;
            }
        }

        if saves_level_path.exists() && recreate_save {
            std::fs::remove_file(&saves_level_path).unwrap_or_else(|_| {
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
            let map_gen_settings_path =
                format!("{}/map-gen-settings.json", instance_path.to_str().unwrap());
            let map_settings_path =
                format!("{}/map-settings.json", instance_path.to_str().unwrap());
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
            let mut outfile = fs::File::create(&player_data_path)
                .into_diagnostic("factorio::instance_setup::could_not_create_data_path")?;
            outfile
                .write_all(player_data)
                .into_diagnostic("factorio::instance_setup::could_not_write_data_path")?;
            if !silent {
                info!("Created <bright-blue>{:?}</>", &player_data_path);
            }
        }
        let mut value: Value = read_to_value(&player_data_path)?;
        value["service-username"] = Value::from(instance_name);
        let player_data_file = File::create(&player_data_path)
            .into_diagnostic("factorio::instance_setup::could_not_create_file")?;
        serde_json::to_writer_pretty(player_data_file, &value)
            .into_diagnostic("factorio::instance_setup::could_not_create_writer")?;

        let config_path = instance_path.join(PathBuf::from("config"));
        if !config_path.exists() {
            create_dir(&config_path)
                .await
                .into_diagnostic("factorio::instance_setup::could_not_create_dir")?;
            let config_ini_data = include_bytes!("../data/config.ini");
            let config_ini_path = config_path.join(PathBuf::from("config.ini"));
            let mut outfile = fs::File::create(&config_ini_path)
                .into_diagnostic("factorio::instance_setup::could_not_create_file")?;
            outfile
                .write_all(config_ini_data)
                .into_diagnostic("factorio::instance_setup::could_not_create_file")?;
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
) -> DiagnosticResult<()> {
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
    let server_settings_path = instance_path.join(PathBuf::from("server-settings.json"));
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
        logger.loading(
            "Updating <bright-blue>map-settings.json</> and <bright-blue>map-gen-settings.json</>",
        );
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
    let (_, rcon) = read_output(
        reader,
        rcon_settings,
        log_path,
        // None,
        false,
        true,
        FactorioStartCondition::Initialized,
    )
    .await?;
    let map_gen_settings_filename = "map-gen-settings.json";
    let map_settings_filename = "map-settings.json";
    rcon.parse_map_exchange_string(map_gen_settings_filename, map_exchange_string)
        .await?;
    child
        .kill()
        .into_diagnostic("factorio::instance_setup::could_not_kill_child")?;
    let target_map_gen_settings_path =
        instance_path.join(PathBuf::from_str(map_gen_settings_filename).unwrap());
    let target_map_settings_path =
        instance_path.join(PathBuf::from_str(map_settings_filename).unwrap());
    let script_output_path = instance_path.join(PathBuf::from_str("script-output").unwrap());
    let source_map_gen_settings_path =
        script_output_path.join(PathBuf::from_str(map_gen_settings_filename).unwrap());
    let value: Value = read_to_value(&source_map_gen_settings_path)?;
    write_value_to(&value["map_settings"], &target_map_settings_path)?;
    write_value_to(&value["map_gen_settings"], &target_map_gen_settings_path)?;
    fs::remove_file(&source_map_gen_settings_path)
        .unwrap_or_else(|_| panic!("failed to delete {:?}", &source_map_gen_settings_path));

    if !silent {
        logger.success(
            "Updated <bright-blue>map-settings.json</> and <bright-blue>map-gen-settings.json</>",
        );
    }
    Ok(())
}
