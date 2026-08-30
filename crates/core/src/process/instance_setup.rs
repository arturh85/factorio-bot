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
#[cfg(not(debug_assertions))]
use crate::process::asset_sync;
use crate::process::io_utils::{await_lock, extract_archive, get_factorio_binary_path, symlink};
use crate::process::output_reader::read_output;
use crate::process::process_control::FactorioStartCondition;
use crate::process::spinner::Spinner;
use miette::{miette, IntoDiagnostic, Result};
use parking_lot::RwLock;
use tokio::fs::create_dir;

// Release builds must be self-contained, so `mods/` and `scripts/` are baked
// into the binary at compile time and extracted into the workspace on first
// setup. Debug builds instead point the mods path at `../../mods` in the repo
// (see below), which is live.
//
// Two consequences, both of which have already cost a debugging session:
//   * editing `mods/` has no effect on a release binary until it is rebuilt --
//     the embedded copy is a snapshot taken by `include_dir!`;
//   * it has no effect on an existing `workspace/mods` at all, in any build,
//     because extraction is skipped once that directory exists.
//
// This divergence is deliberate; do not "fix" it by dropping the embedding.
// What *is* fixed here: the second bullet used to fail silently. `asset_sync`
// compares the embedded snapshot against whatever is already on disk and
// warns when they differ, and `REFRESH_MODS_ENV` / `REFRESH_PLANS_ENV` are the
// explicit, opt-in way to overwrite a stale copy (see their doc comments).
#[cfg(not(debug_assertions))]
pub const MODS_CONTENT: include_dir::Dir = include_dir!("mods");
#[cfg(not(debug_assertions))]
pub const PLANS_CONTENT: include_dir::Dir = include_dir!("scripts");

/// Set to any value to overwrite a stale `<workspace>/mods` with the snapshot
/// embedded in this binary. Not read automatically: refreshing on every run
/// would silently discard a workspace copy someone edited on purpose.
#[cfg(not(debug_assertions))]
pub const REFRESH_MODS_ENV: &str = "FACTORIO_BOT_REFRESH_MODS";
/// Same as [`REFRESH_MODS_ENV`], for `<workspace>/plans`.
#[cfg(not(debug_assertions))]
pub const REFRESH_PLANS_ENV: &str = "FACTORIO_BOT_REFRESH_PLANS";

/// The mod this project ships and depends on: without it there is no RCON
/// bridge, and every other feature is unreachable.
pub const BRIDGE_MOD_NAME: &str = "BotBridge";

/// Reduces a Factorio version string to its `major.minor` prefix.
///
/// Factorio matches a mod's declared `factorio_version` against the running
/// game on `major.minor` only. Verified against the installed Factorio 2.1.17
/// by running `factorio --create` with a probe mod: `factorio_version` of
/// `2.1`, `2.1.17` and even `2.1.0` all load, while `2.0` and `1.1` are
/// rejected with `Incompatible Factorio version (current: 2.1, required: 2.0)`
/// -- note that Factorio itself reports its own version as `2.1` there. Wube's
/// own mods shipped with 2.1.17 (`space-age`, `quality`, `elevated-rails`,
/// `recycler`) all declare `"factorio_version": "2.1"`.
///
/// Returns `None` for anything that is not two leading numeric components, so
/// a malformed manifest degrades to "cannot tell" rather than a false verdict.
fn major_minor(version: &str) -> Option<String> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.trim();
    let minor = parts.next()?.trim();
    let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !numeric(major) || !numeric(minor) {
        return None;
    }
    Some(format!("{}.{}", major, minor))
}

/// Reads a top-level string field out of a JSON file, treating a missing file,
/// unreadable bytes, invalid JSON and a missing or non-string field all as
/// "not available". These are operational conditions, not bugs.
fn read_json_string_field(path: &Path, field: &str) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    Some(value.get(field)?.as_str()?.to_string())
}

/// Fails before launching Factorio when the bridge mod targets a different
/// Factorio major.minor than the installed game.
///
/// Factorio surfaces this mismatch only as a mod load failure buried in its own
/// output, after which the run dies with a misleading "failed to create
/// factorio level". Nothing in the Rust build reads the mod manifest, so no
/// test can catch it -- hence this preflight.
///
/// When either manifest is missing or malformed the check cannot reach a
/// verdict; it warns and lets the run continue rather than blocking on an
/// unrelated problem.
pub fn preflight_mod_factorio_version(
    mods_path: &Path,
    data_path: &Path,
    silent: bool,
) -> Result<()> {
    let mod_info_path = mods_path.join(BRIDGE_MOD_NAME).join("info.json");
    let base_info_path = data_path.join("base").join("info.json");

    let declared = read_json_string_field(&mod_info_path, "factorio_version");
    let installed = read_json_string_field(&base_info_path, "version");
    let (Some(declared), Some(installed)) = (declared, installed) else {
        if !silent {
            warn!(
                "skipping Factorio version preflight: could not read `factorio_version` from <bright-blue>{:?}</> or `version` from <bright-blue>{:?}</>",
                mod_info_path, base_info_path
            );
        }
        return Ok(());
    };

    let (Some(mod_major_minor), Some(game_major_minor)) =
        (major_minor(&declared), major_minor(&installed))
    else {
        if !silent {
            warn!(
                "skipping Factorio version preflight: cannot parse mod version <bright-blue>{}</> ({:?}) or game version <bright-blue>{}</> ({:?}) as major.minor",
                declared, mod_info_path, installed, base_info_path
            );
        }
        return Ok(());
    };

    if mod_major_minor != game_major_minor {
        return Err(ModFactorioVersionMismatch {
            mod_name: BRIDGE_MOD_NAME.into(),
            mod_factorio_version: declared,
            mod_major_minor,
            game_version: installed,
            game_major_minor,
            mod_info_path: mod_info_path.to_string_lossy().into_owned(),
            base_info_path: base_info_path.to_string_lossy().into_owned(),
        }
        .into());
    }
    Ok(())
}

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
        // `extract_archive` is a plain synchronous `fn`: on a first run it
        // decompresses and writes out the entire Factorio archive, which
        // takes 8-10 minutes and never yields. Awaiting it directly parks
        // whatever thread runs it -- on the server that is one of a small
        // number of async workers, and a start plus a couple of long scripts
        // is enough to stop `/api/v1/health` answering. `spawn_blocking` puts
        // it on the blocking pool, which exists for exactly this. Fixed here
        // rather than at the caller so the CLI, the REPL and `roll_best_seed`
        // -- which all reach the extraction through this function -- get it
        // too.
        let archive = factorio_archive_path.to_owned();
        let target = instance_path.to_path_buf();
        let workspace = workspace_path.to_path_buf();
        tokio::task::spawn_blocking(move || extract_archive(&archive, &target, &workspace))
            .await
            // A `JoinError` means the extraction panicked or was cancelled.
            // Turn it into an ordinary error: resuming the panic on this
            // thread would abort the whole process under `panic = "abort"`.
            .map_err(|err| miette!("archive extraction task failed: {err}"))??;
    }
    #[allow(unused_mut)]
    let mut workspace_mods_path = workspace_path.join(PathBuf::from(MODS_FOLDERNAME));
    // Which of the several possible mod sources we ended up on. Logged below,
    // because "I edited mods/ and the game kept loading the old copy" has
    // already cost a live debugging session.
    #[allow(unused_mut, unused_assignments)]
    #[cfg(not(debug_assertions))]
    let mut mods_source = "pre-existing workspace copy; editing mods/ does NOT update it -- see the staleness warning below, or set FACTORIO_BOT_REFRESH_MODS=1 to refresh it";
    #[allow(unused_mut, unused_assignments)]
    #[cfg(debug_assertions)]
    let mut mods_source =
        "pre-existing workspace copy; editing mods/ does NOT update it, delete it to re-extract";
    if !workspace_mods_path.exists() {
        #[cfg(debug_assertions)]
        {
            workspace_mods_path = PathBuf::from(format!("../../{}", MODS_FOLDERNAME));
            mods_source = "repo checkout (debug build); edits apply on the next run";
        }
        #[cfg(not(debug_assertions))]
        {
            std::fs::create_dir_all(&workspace_mods_path).into_diagnostic()?;
            if let Err(err) = MODS_CONTENT.extract(workspace_mods_path.clone()) {
                error!("failed to extract static mods content: {:?}", err);
                return Err(ModExtractFailed {}.into());
            }
            mods_source =
                "compile-time snapshot embedded in this release binary; edits to mods/ need a rebuild";
        }
        if !workspace_mods_path.exists() {
            workspace_mods_path = PathBuf::from(MODS_FOLDERNAME);
            mods_source = "mods/ relative to the current working directory";
            if !workspace_mods_path.exists() {
                return Err(MissingModsFolder {}.into());
            }
        }
    } else {
        // The directory already existed, so nothing above extracted into it.
        // In a release build that copy can only ever be refreshed explicitly
        // -- see `asset_sync` -- so check it for drift from the embedded
        // snapshot rather than staying silent about it.
        #[cfg(not(debug_assertions))]
        {
            if asset_sync::refresh_if_requested(
                &MODS_CONTENT,
                &workspace_mods_path,
                REFRESH_MODS_ENV,
            )
            .into_diagnostic()?
            {
                mods_source =
                    "refreshed from the compile-time snapshot embedded in this release binary";
            } else {
                asset_sync::warn_if_stale(
                    &MODS_CONTENT,
                    &workspace_mods_path,
                    "mods",
                    REFRESH_MODS_ENV,
                );
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
        } else if asset_sync::refresh_if_requested(
            &PLANS_CONTENT,
            &data_plans_path,
            REFRESH_PLANS_ENV,
        )
        .into_diagnostic()?
        {
            if !silent {
                info!(
                    "Refreshed <bright-blue>{:?}</> from the embedded snapshot ({}=1 was set)",
                    data_plans_path, REFRESH_PLANS_ENV
                );
            }
        } else {
            asset_sync::warn_if_stale(&PLANS_CONTENT, &data_plans_path, "plans", REFRESH_PLANS_ENV);
        }
    }

    let workspace_mods_path = fs::canonicalize(workspace_mods_path).into_diagnostic()?;
    if !silent {
        info!(
            "Using mods directory <bright-blue>{:?}</> ({})",
            &workspace_mods_path, mods_source
        );
    }
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
    // Refuse to launch a game that will silently drop the bridge mod. Prefer the
    // workspace data directory, falling back to the instance's own copy.
    let base_data_path = {
        let candidate = workspace_path.join(PathBuf::from("data"));
        if candidate.join("base").join("info.json").exists() {
            candidate
        } else {
            instance_data_path.clone()
        }
    };
    preflight_mod_factorio_version(&workspace_mods_path, &base_data_path, silent)?;
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
                    let factorio_binary_path = get_factorio_binary_path(instance_path);
                    if !factorio_binary_path.exists() {
                        error!(
                            "factorio binary missing at <bright-blue>{:?}</>",
                            factorio_binary_path
                        );
                        return Err(FactorioBinaryNotFound {}.into());
                    }
                    // only used to point macOS Factorio at the instance mods directory
                    #[cfg(target_os = "macos")]
                    let mods_path = instance_path.join("mods");
                    #[cfg(target_os = "macos")]
                    let mods_path_str = mods_path.to_str().unwrap().to_string();
                    let mut args = vec!["--create", saves_level_path.to_str().unwrap()];
                    if let Some(seed) = seed.as_ref() {
                        args.push("--map-gen-seed");
                        args.push(seed);
                    }
                    // macOS Factorio uses ~/Library/Application Support/factorio/mods by default
                    #[cfg(target_os = "macos")]
                    {
                        args.push("--mod-directory");
                        args.push(&mods_path_str);
                    }
                    let output = Command::new(&factorio_binary_path)
                        .args(&args)
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
                panic!("failed to delete {}", saves_level_path.to_str().unwrap())
            });
        }
        if !saves_level_path.exists() {
            let mut logger = Spinner::new();
            let factorio_binary_path = get_factorio_binary_path(instance_path);
            if !factorio_binary_path.exists() {
                error!(
                    "factorio binary missing at <bright-blue>{:?}</>",
                    factorio_binary_path
                );
                return Err(FactorioBinaryNotFound {}.into());
            }
            // only used to point macOS Factorio at the instance mods directory
            #[cfg(target_os = "macos")]
            let mods_path = instance_path.join("mods");
            #[cfg(target_os = "macos")]
            let mods_path_str = mods_path.to_str().unwrap().to_string();
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
            // macOS Factorio uses ~/Library/Application Support/factorio/mods by default
            #[cfg(target_os = "macos")]
            {
                args.push("--mod-directory");
                args.push(&mods_path_str);
            }
            await_lock(instance_path.join(PathBuf::from(".lock")), silent).await?;
            if !silent {
                logger.loading(format!(
                    "Creating Level at <bright-blue>{:?}</>...",
                    saves_level_path
                ));
            }

            let output = Command::new(&factorio_binary_path)
                .args(&args)
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
                    saves_level_path
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

        // CRITICAL: Check if config.ini FILE exists, not just the config/ directory
        // Bug fix (Jan 2026): Archive extraction creates an empty config/ directory,
        // so checking `!config_path.exists()` would skip config.ini creation, causing
        // clients to crash with "Error: Specified config file doesn't exist"
        let config_path = instance_path.join(PathBuf::from("config"));
        let config_ini_path = config_path.join(PathBuf::from("config.ini"));
        if !config_ini_path.exists() {
            if !config_path.exists() {
                create_dir(&config_path).await.into_diagnostic()?;
            }
            let config_ini_data = include_bytes!("../data/config.ini");
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
    let factorio_binary_path = get_factorio_binary_path(instance_path);
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
    let mut logger = Spinner::new();
    if !silent {
        logger.loading(format!(
            "Updating <bright-blue>{}</> and <bright-blue>{}</>",
            MAP_SETTINGS_FILENAME, MAP_GEN_SETTINGS_FILENAME
        ));
    }
    // only used to point macOS Factorio at the instance mods directory
    #[cfg(target_os = "macos")]
    let mods_path = instance_path.join("mods");
    #[cfg(target_os = "macos")]
    let mods_path_str = mods_path.to_str().unwrap().to_string();
    let port_str = factorio_port.unwrap_or(34197).to_string();
    let rcon_port_str = rcon_settings.port.to_string();
    // pushed to in the macOS-only block below
    #[allow(unused_mut)]
    let mut args = vec![
        "--start-server",
        saves_level_path.to_str().unwrap(),
        "--port",
        &port_str,
        "--rcon-port",
        &rcon_port_str,
        "--rcon-password",
        &rcon_settings.pass,
        "--server-settings",
        server_settings_path.to_str().unwrap(),
    ];
    // macOS Factorio uses ~/Library/Application Support/factorio/mods by default
    #[cfg(target_os = "macos")]
    {
        args.push("--mod-directory");
        args.push(&mods_path_str);
    }
    let log_path = workspace_path.join(PathBuf::from_str("server-log.txt").unwrap());
    let mut command = Command::new(&factorio_binary_path);
    command.args(&args);
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
        .unwrap_or_else(|_| panic!("failed to delete {:?}", source_map_gen_settings_path));

    if !silent {
        logger.success(format!(
            "Updated <bright-blue>{}</> and <bright-blue>{}</>",
            MAP_SETTINGS_FILENAME, MAP_GEN_SETTINGS_FILENAME
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_json(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// Lays out a mods dir and a data dir. `mod_manifest` / `base_manifest` are
    /// written verbatim so malformed input can be exercised; `None` means the
    /// file is absent entirely.
    fn fixture(mod_manifest: Option<&str>, base_manifest: Option<&str>) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        if let Some(contents) = mod_manifest {
            write_json(
                &dir.path()
                    .join("mods")
                    .join(BRIDGE_MOD_NAME)
                    .join("info.json"),
                contents,
            );
        }
        if let Some(contents) = base_manifest {
            write_json(
                &dir.path().join("data").join("base").join("info.json"),
                contents,
            );
        }
        dir
    }

    fn run(dir: &tempfile::TempDir) -> Result<()> {
        preflight_mod_factorio_version(&dir.path().join("mods"), &dir.path().join("data"), true)
    }

    #[test]
    fn major_minor_drops_the_patch_component() {
        assert_eq!(major_minor("2.1.17").as_deref(), Some("2.1"));
        assert_eq!(major_minor("2.1").as_deref(), Some("2.1"));
        assert_eq!(major_minor(" 2.1.17 ").as_deref(), Some("2.1"));
        assert_eq!(major_minor("2"), None);
        assert_eq!(major_minor(""), None);
        assert_eq!(major_minor("two.one"), None);
        assert_eq!(major_minor("2."), None);
    }

    #[test]
    fn mismatched_major_minor_fails_and_names_both_versions() {
        // The exact shape that broke a live run: the mod still declared 2.0
        // while the installed game was 2.1.17.
        let dir = fixture(
            Some(r#"{"name":"BotBridge","factorio_version":"2.0"}"#),
            Some(r#"{"name":"base","version":"2.1.17"}"#),
        );
        let err = run(&dir).expect_err("2.0 mod against a 2.1.17 game must be refused");
        let rendered = err.to_string();
        assert!(
            rendered.contains("2.0"),
            "error must name the mod's version, got: {rendered}"
        );
        assert!(
            rendered.contains("2.1.17"),
            "error must name the installed version, got: {rendered}"
        );
        let typed = err
            .downcast_ref::<ModFactorioVersionMismatch>()
            .expect("expected ModFactorioVersionMismatch");
        assert_eq!(typed.mod_factorio_version, "2.0");
        assert_eq!(typed.mod_major_minor, "2.0");
        assert_eq!(typed.game_version, "2.1.17");
        assert_eq!(typed.game_major_minor, "2.1");
        assert!(typed.mod_info_path.ends_with("BotBridge/info.json"));
        assert!(typed.base_info_path.ends_with("base/info.json"));
    }

    #[test]
    fn differing_patch_level_is_accepted() {
        // Verified against the real binary: a mod declaring 2.1 loads on 2.1.17.
        let dir = fixture(
            Some(r#"{"name":"BotBridge","factorio_version":"2.1"}"#),
            Some(r#"{"name":"base","version":"2.1.17"}"#),
        );
        run(&dir).expect("major.minor match must pass regardless of patch level");
    }

    #[test]
    fn differing_major_version_fails() {
        let dir = fixture(
            Some(r#"{"name":"BotBridge","factorio_version":"1.1"}"#),
            Some(r#"{"name":"base","version":"2.1.17"}"#),
        );
        run(&dir).expect_err("1.1 mod against a 2.1.17 game must be refused");
    }

    #[test]
    fn missing_manifests_are_operational_not_fatal() {
        run(&fixture(None, Some(r#"{"version":"2.1.17"}"#)))
            .expect("a missing mod manifest must not fail the run");
        run(&fixture(Some(r#"{"factorio_version":"2.1"}"#), None))
            .expect("a missing base manifest must not fail the run");
        run(&fixture(None, None)).expect("both missing must not fail the run");
    }

    #[test]
    fn malformed_manifests_are_operational_not_fatal() {
        run(&fixture(Some("{not json"), Some(r#"{"version":"2.1.17"}"#)))
            .expect("unparseable mod manifest must not fail the run");
        run(&fixture(
            Some(r#"{"factorio_version":"2.1"}"#),
            Some("[1, 2, 3]"),
        ))
        .expect("unexpected base manifest shape must not fail the run");
        run(&fixture(
            Some(r#"{"factorio_version":17}"#),
            Some(r#"{"version":"2.1.17"}"#),
        ))
        .expect("non-string factorio_version must not fail the run");
        run(&fixture(
            Some(r#"{"factorio_version":"latest"}"#),
            Some(r#"{"version":"2.1.17"}"#),
        ))
        .expect("unparseable version string must not fail the run");
    }
}

/// Runtime behaviour of [`setup_factorio_instance`], as opposed to the
/// manifest checks above. A separate module because the file already has a
/// `tests` module (a second one with the same name would not compile) and
/// because this one needs a tokio runtime with a very specific shape.
#[cfg(test)]
mod extraction_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// When the sampler below reads the tick counter: far enough into the
    /// extraction that a working runtime has ticked many times, early enough
    /// that the extraction is certainly still running.
    const SAMPLE_AT: Duration = Duration::from_millis(300);

    /// Builds a `.tar.xz` in the shape `extract_archive` expects on unix: a
    /// single top-level `factorio/` directory containing `data/`.
    ///
    /// Slow-by-file-count rather than slow-by-size: 100 000 tiny files make
    /// `Archive::unpack` do 100 000 file creations, which is syscall-bound and
    /// takes a couple of seconds, while compressing to almost nothing and
    /// costing the test no meaningful disk. 20 000 was measured at ~400ms on
    /// this machine, too close to `SAMPLE_AT` for the guard below to accept.
    fn build_slow_archive(path: &Path) {
        let file = File::create(path).expect("create archive");
        let encoder = xz2::write::XzEncoder::new(file, 1);
        let mut builder = tar::Builder::new(encoder);
        let payload = [b'x'; 32];
        for index in 0..100_000 {
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    format!("factorio/data/file-{index}.bin"),
                    &payload[..],
                )
                .expect("append file");
        }
        builder
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish xz");
    }

    /// The defect this guards against: `extract_archive` is a synchronous
    /// `fn` doing minutes of file IO on a first run. Awaited directly it parks
    /// whatever thread runs it -- on the server that is one of a small number
    /// of async workers, so a single first-run start takes a worker out of
    /// service for the whole extraction and `/api/v1/health` stops answering.
    ///
    /// Three details are what make this decisive rather than lucky:
    ///
    ///   * `worker_threads = 1`. `spawn_blocking` uses the blocking pool,
    ///     which exists independently of the worker count, so the ticker keeps
    ///     running; a direct call occupies the one and only worker and the
    ///     ticker cannot tick at all.
    ///   * The call under test is `tokio::spawn`ed rather than awaited in the
    ///     test body. On the multi-threaded runtime the body runs on the
    ///     `block_on` thread, which is *not* a worker: blocking it leaves the
    ///     worker free and the ticker ticks merrily with the defect fully
    ///     present. Verified -- the body-local version of this test passes
    ///     against the unfixed code.
    ///   * The tick count is snapshotted by a plain OS thread *during* the
    ///     extraction, not read after the call returns. `setup_factorio_instance`
    ///     has further `await` points after the extraction, so a reading taken
    ///     afterwards picks up a tick or two even when the extraction blocked
    ///     the whole way through.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn extracting_an_archive_does_not_park_the_async_worker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        let archive = dir.path().join("factorio.tar.xz");
        build_slow_archive(&archive);

        let ticks = Arc::new(AtomicUsize::new(0));
        let ticker = {
            let ticks = ticks.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    ticks.fetch_add(1, Ordering::SeqCst);
                }
            })
        };
        let sampled = Arc::new(AtomicUsize::new(usize::MAX));
        let sampler = {
            let ticks = ticks.clone();
            let sampled = sampled.clone();
            std::thread::spawn(move || {
                std::thread::sleep(SAMPLE_AT);
                sampled.store(ticks.load(Ordering::SeqCst), Ordering::SeqCst);
            })
        };

        let workspace_arg = workspace.to_str().expect("utf-8 workspace path").to_owned();
        let archive_arg = archive.to_str().expect("utf-8 archive path").to_owned();
        let rcon_settings = RconSettings::new(4321, "foobar", None);
        let started = Instant::now();
        // The call fails partway through -- there is no Factorio binary in
        // this synthetic archive -- and that is fine: extraction happens
        // early, and the assertion is about the runtime, not the result.
        let _ = tokio::spawn(async move {
            setup_factorio_instance(
                &workspace_arg,
                &archive_arg,
                &rcon_settings,
                None,
                "server",
                true,
                false,
                None,
                None,
                true,
            )
            .await
        })
        .await;
        let elapsed = started.elapsed();
        ticker.abort();
        sampler.join().expect("sampler thread");
        let observed = sampled.load(Ordering::SeqCst);

        assert!(
            elapsed > SAMPLE_AT * 2,
            "the whole call took {elapsed:?}, so the tick count sampled at {SAMPLE_AT:?} says \
             nothing about what happened during the extraction; raise the file count in \
             build_slow_archive"
        );
        assert!(
            observed >= 3,
            "the runtime made no progress while the archive extracted ({observed} ticks in the \
             first {SAMPLE_AT:?}); the extraction is blocking an async worker"
        );
    }
}
