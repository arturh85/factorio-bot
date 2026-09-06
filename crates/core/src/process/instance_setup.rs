use serde_json::Value;
use std::fs;
use std::fs::{File, read_to_string};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;

use crate::constants::{
    MAP_GEN_SETTINGS_FILENAME, MAP_SETTINGS_FILENAME, MOD_LIST_FILENAME, MODS_FOLDERNAME,
    SERVER_SETTINGS_FILENAME,
};
use crate::errors::*;
use crate::factorio::rcon::RconSettings;
use crate::factorio::util::{read_to_value, write_value_to};
use crate::process::io_utils::{
    await_lock, extract_archive, get_factorio_binary_path, get_factorio_data_path, symlink,
};
use crate::process::output_reader::read_output;
use crate::process::process_control::FactorioStartCondition;
use crate::process::spinner::Spinner;
use miette::{IntoDiagnostic, Result, miette};
use parking_lot::RwLock;
use tokio::fs::create_dir;

// The two build profiles resolve `mods/` differently, on purpose.
//
//   debug    `workspace/mods` is a real directory, and `BotBridge` inside it
//            is a SYMLINK to this checkout's `mods/BotBridge`, created or
//            repaired on every setup (`resolve_workspace_mods`). An edit to
//            `mods/BotBridge/control.lua` is therefore what the next run
//            loads, with nothing to refresh and no copy to go stale. The
//            other mods and Factorio's own `mod-list.json` /
//            `mod-settings.dat` stay real files in the workspace, because the
//            game rewrites those as it runs and must not write into the repo.
//   release  `mods/` and `scripts/` are baked into the binary at compile time
//            and extracted into the workspace on first setup, so a release
//            binary is self-contained. Two consequences, both of which have
//            cost a debugging session: editing `mods/` has no effect until
//            the binary is rebuilt, and no effect on an existing
//            `workspace/mods` at all, because extraction is skipped once that
//            directory exists. `REFRESH_MODS_ENV` is the explicit way out;
//            `asset_sync::warn_if_stale` says so when the copy has drifted.
//
// This divergence is deliberate; do not "fix" it by dropping the embedding,
// and do not extend the symlink to a release build, which has no checkout to
// point at.
//
// The debug side used to be a copy plus a drift *check* -- a byte-for-byte
// comparison against the checkout, reported on the mods line. The check was
// correct and useless: it reported through a line `silent` suppressed, so a
// `control.lua` edit went unloaded for hours with the detection sitting right
// there, unprinted. Both halves are fixed here -- the symlink removes the
// state the check was looking for, and the mods line is now unconditional.
#[cfg(not(debug_assertions))]
pub const MODS_CONTENT: include_dir::Dir = include_dir!("mods");
/// The repo's `scripts/` directory, embedded. Extracted into
/// `<workspace>/scripts` by [`crate::scripts::ensure_scripts_dir`], its only
/// consumer.
#[cfg(not(debug_assertions))]
pub const SCRIPTS_CONTENT: include_dir::Dir = include_dir!("scripts");

/// Set to any value to overwrite a stale `<workspace>/mods` with the snapshot
/// embedded in this binary. Not read automatically: refreshing on every run
/// would silently discard a workspace copy someone edited on purpose.
#[cfg(not(debug_assertions))]
pub const REFRESH_MODS_ENV: &str = "FACTORIO_BOT_REFRESH_MODS";

/// The repo's `mods/` directory as a compile-time path, with `$suffix`
/// appended -- e.g. `repo_mods_path!("/BotBridge/control.lua")`.
///
/// This is *the* single definition of "the mods directory this build was
/// compiled against", and it exists so that two things which must agree
/// cannot drift apart: `resolve_workspace_mods` below, which points a debug
/// run's `workspace/mods/BotBridge` at this directory, and
/// `factorio::rcon`'s transfer-guarantee test, which `include_str!`s the
/// mod's `control.lua` out of it. If the guard compiled in bytes from one
/// directory and the run loaded another, the green guard would say nothing
/// about the run.
///
/// Resolved from `CARGO_MANIFEST_DIR` (this crate) rather than from the
/// process's working directory, so it names the same place whatever a binary
/// is later run from. It is a *compile-time* fact: a debug binary carried
/// away from its source tree finds nothing there, which `link_bridge_mod`
/// reports as "no directory there now" while leaving whatever the workspace
/// already has alone.
// Unused in a release build, which extracts the embedded snapshot instead,
// and in any non-test build of a release binary nothing imports it.
#[allow(unused_macros)]
macro_rules! repo_mods_path {
    ($suffix:literal) => {
        // "mods" is `MODS_FOLDERNAME`, which cannot appear here: `concat!`
        // takes literals only.
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../mods", $suffix)
    };
}
#[cfg(test)]
pub(crate) use repo_mods_path;

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

/// The file [`record_map_gen_seed`] writes, beside `map-exchange-string.txt`
/// and in the same spirit: a small plain-text note saying how the level next to
/// it was made.
pub const MAP_GEN_SEED_FILENAME: &str = "map-gen-seed.txt";

/// What the file holds when Factorio chose the seed and nothing observed it.
///
/// Written rather than left absent, so that "this map was generated by a build
/// that records seeds, and the seed was not ours to know" is distinguishable
/// from "this map predates seed recording". Those are different facts and only
/// one of them can be fixed by passing `--seed`.
pub const MAP_GEN_SEED_UNKNOWN: &str = "unknown";

/// Writes down which seed this instance's level was **just** created with.
///
/// Called at every creation, including the ones with no seed, and that is the
/// point rather than an oversight. The seed belongs to the *map*, not to the
/// run, and a stale note is worse than no note: a workspace whose level was
/// regenerated with `--new` and no `--seed` would otherwise keep claiming the
/// seed of the map before it, which is precisely the false confidence this
/// whole file exists to remove.
///
/// Not fallible to the caller's detriment on purpose -- the level was created
/// successfully by the time this runs, and failing the whole setup because a
/// note could not be written would trade a working run for a missing record.
fn record_map_gen_seed(instance_path: &Path, seed: Option<&str>) -> Result<()> {
    let path = instance_path.join(MAP_GEN_SEED_FILENAME);
    let contents = seed.unwrap_or(MAP_GEN_SEED_UNKNOWN);
    if let Err(error) = fs::write(&path, contents) {
        warn!(
            "could not record the map seed at <bright-blue>{:?}</>: {}. \
             The map is fine; the run just will not be able to say which seed made it.",
            path, error
        );
    }
    Ok(())
}

/// Reads back the seed this instance's current level was created with, when a
/// build that records it made the level.
///
/// `None` for every workspace whose level predates [`record_map_gen_seed`], and
/// `None` for a level Factorio seeded itself. Both are honestly "we do not
/// know", which is the answer a run record must be able to give rather than
/// guessing at a plausible number.
pub fn read_map_gen_seed(instance_path: &Path) -> Option<String> {
    let raw = fs::read_to_string(instance_path.join(MAP_GEN_SEED_FILENAME)).ok()?;
    let seed = raw.trim();
    if seed.is_empty() || seed == MAP_GEN_SEED_UNKNOWN {
        return None;
    }
    Some(seed.to_string())
}

/// The installed game's version, e.g. `"2.1.17"`, read from the base mod's own
/// manifest -- the same source [`preflight_mod_factorio_version`] trusts.
///
/// `None` rather than a guess when the file is missing or malformed: a run
/// record that names the wrong game version is worse than one that admits it
/// does not know, because only the second prompts anybody to look.
pub fn installed_factorio_version(data_path: &Path) -> Option<String> {
    read_json_string_field(&data_path.join("base").join("info.json"), "version")
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

/// Resolves `<workspace>/mods` into the directory the game will really load
/// and one sentence saying how it got there, which the caller prints.
///
/// Debug build: `workspace/mods` is a real directory holding the other mods
/// and Factorio's own state files, and `BotBridge` inside it is a **symlink**
/// to the checkout this binary was compiled against, created or repaired on
/// every setup. So the mod the game loads is the mod in `mods/BotBridge`, by
/// construction, and there is no copy left that could go stale.
///
/// Three things ruled out first, each because it was tried:
///
///   * *Point the mods directory itself at the checkout.* Factorio rewrites
///     `mod-list.json` and `mod-settings.dat` in the mods directory as it
///     runs, so those writes would land in the repo -- and `workspace/mods`
///     also holds mods the checkout does not (`creative-mod`, `YARM`).
///   * *Copy the checkout over the workspace copy on every run.* That is what
///     `FACTORIO_BOT_REFRESH_MODS` does for a release build, and it silently
///     discards a workspace copy someone edited on purpose to unblock a run.
///   * *Delete the workspace copy and let it be re-seeded.* Every instance's
///     `mods` is a symlink to `workspace/mods`, so a missing
///     `workspace/mods/BotBridge` leaves the server with no bridge mod: it
///     hangs at `start waiting` forever, having first written a `level.zip`
///     with no bridge state, which then poisons every later run because
///     Factorio only migrates on a version bump and `info.json` is pinned.
///
/// This replaced a drift *check* -- a byte-for-byte comparison of the copy
/// against the checkout, reported on the mods line -- which was correct and
/// useless: the line it reported through was suppressed by `silent`, so a
/// stale copy was loaded for hours with the detection sitting right there.
/// Detecting a state that can no longer happen is not worth a branch; making
/// the state impossible is.
#[cfg(debug_assertions)]
fn resolve_workspace_mods(workspace_mods_path: PathBuf) -> Result<(PathBuf, String)> {
    std::fs::create_dir_all(&workspace_mods_path).into_diagnostic()?;
    let repo_mods = Path::new(repo_mods_path!(""));
    // Cosmetic only, and deliberately infallible: the compile-time path
    // contains `../..`, which is noise in a log line. If it cannot be
    // canonicalized the checkout is gone, and the raw path is still the right
    // thing to name -- `link_bridge_mod` reports that as "nothing there now"
    // rather than as a failure.
    let repo_mods = fs::canonicalize(repo_mods).unwrap_or_else(|_| repo_mods.to_path_buf());
    seed_missing_mods_from_checkout(&repo_mods, &workspace_mods_path);
    let bridge = link_bridge_mod(&repo_mods, &workspace_mods_path)?;
    Ok((workspace_mods_path, format!("debug build; {bridge}")))
}

/// Release counterpart: the mods directory is extracted once from the
/// snapshot `include_dir!` baked into this binary, and after that only an
/// explicit `FACTORIO_BOT_REFRESH_MODS` overwrites it. Unchanged from before
/// the debug side grew a symlink -- a release binary has no checkout to point
/// at, and being self-contained is the point of it.
#[cfg(not(debug_assertions))]
fn resolve_workspace_mods(workspace_mods_path: PathBuf) -> Result<(PathBuf, String)> {
    // Scoped to this function rather than imported at the top of the file:
    // the debug half of this pair does not use it, and a top-level import
    // would be an unused-import warning in the profile this project builds by
    // default.
    use crate::process::asset_sync;

    let mut workspace_mods_path = workspace_mods_path;
    if !workspace_mods_path.exists() {
        std::fs::create_dir_all(&workspace_mods_path).into_diagnostic()?;
        if let Err(err) = MODS_CONTENT.extract(workspace_mods_path.clone()) {
            error!("failed to extract static mods content: {:?}", err);
            return Err(ModExtractFailed {}.into());
        }
        let mut mods_source = String::from(
            "compile-time snapshot embedded in this release binary; edits to mods/ need a rebuild",
        );
        if !workspace_mods_path.exists() {
            workspace_mods_path = PathBuf::from(MODS_FOLDERNAME);
            mods_source = String::from("mods/ relative to the current working directory");
            if !workspace_mods_path.exists() {
                return Err(MissingModsFolder {}.into());
            }
        }
        return Ok((workspace_mods_path, mods_source));
    }

    // The directory already existed, so nothing above extracted into it. In a
    // release build that copy can only ever be refreshed explicitly -- see
    // `asset_sync` -- so check it for drift from the embedded snapshot rather
    // than staying silent about it.
    if asset_sync::refresh_if_requested(&MODS_CONTENT, &workspace_mods_path, REFRESH_MODS_ENV)
        .into_diagnostic()?
    {
        return Ok((
            workspace_mods_path,
            String::from(
                "refreshed from the compile-time snapshot embedded in this release binary",
            ),
        ));
    }
    asset_sync::warn_if_stale(
        &MODS_CONTENT,
        &workspace_mods_path,
        "mods",
        REFRESH_MODS_ENV,
    );
    Ok((
        workspace_mods_path,
        String::from(
            "pre-existing workspace copy; editing mods/ does NOT update it -- see the staleness warning above, or set FACTORIO_BOT_REFRESH_MODS=1 to refresh it",
        ),
    ))
}

/// Creates a directory symlink without asking for elevation.
///
/// Not [`crate::process::io_utils::symlink`], which on Windows shells out to
/// `mklink` through `runas` and pops a UAC prompt. That is a reasonable trade
/// for the instance `mods` link, which the run cannot proceed without; it is
/// the wrong one for a developer convenience that has a working fallback. An
/// unprivileged Windows failure here is expected and handled by copying.
#[cfg(debug_assertions)]
fn symlink_dir(original: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(original, link)
    }
}

/// Copies a directory tree: the fallback when a symlink cannot be created,
/// and how `scripts::ensure_scripts_dir` seeds a workspace from the checkout.
#[cfg(debug_assertions)]
pub(crate) fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Fills a `workspace/mods` that is missing entries the checkout has --
/// `creative-mod`, `YARM`, and the initial `mod-list.json` /
/// `mod-settings.dat` -- so a fresh workspace comes up with the same mod set
/// as before this function existed.
///
/// Only where nothing is there yet. Factorio rewrites `mod-list.json` and
/// `mod-settings.dat` in place as it runs, so once a workspace has them, the
/// workspace's are the live ones and the checkout's are a stale seed.
/// `BotBridge` is skipped: it gets a symlink, not a copy.
///
/// Never fails the run. A mod that could not be seeded is a mod the game will
/// not load, which is visible; refusing to start over it is not better.
#[cfg(debug_assertions)]
fn seed_missing_mods_from_checkout(repo_mods: &Path, workspace_mods_path: &Path) {
    let Ok(entries) = std::fs::read_dir(repo_mods) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == BRIDGE_MOD_NAME {
            continue;
        }
        let target = workspace_mods_path.join(&name);
        if target.exists() {
            continue;
        }
        let copied = match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => copy_dir_recursive(&entry.path(), &target),
            Ok(_) => std::fs::copy(entry.path(), &target).map(|_| ()),
            Err(err) => Err(err),
        };
        if let Err(err) = copied {
            warn!(
                "could not seed <bright-blue>{:?}</> from the checkout: {}",
                target, err
            );
        }
    }
}

/// Makes `<workspace>/mods/BotBridge` be the checkout's directory, and
/// returns the sentence the mods line carries about how that turned out.
///
/// Idempotent and self-repairing: a correct symlink is left alone, and
/// anything else in that slot -- a stale copied directory, a symlink to some
/// other checkout, a leftover file -- is removed and replaced.
///
/// The symlink is a convenience, so a machine that cannot make one (Windows
/// without the developer-mode or `SeCreateSymbolicLink` privilege) gets a
/// copy and is *told* it got a copy, rather than a failed run. Only when
/// neither works is this an error: `workspace/mods/BotBridge` would then be
/// absent, and a Factorio server with no bridge mod does not fail -- it hangs
/// at `start waiting` forever while writing a save with no bridge state.
#[cfg(debug_assertions)]
fn link_bridge_mod(repo_mods: &Path, workspace_mods_path: &Path) -> Result<String> {
    let target = repo_mods.join(BRIDGE_MOD_NAME);
    let link = workspace_mods_path.join(BRIDGE_MOD_NAME);
    if !target.is_dir() {
        // A debug binary carried away from its source tree. Say what is
        // missing and leave whatever is in the workspace alone -- it is the
        // only bridge mod this run has.
        return Ok(format!(
            "{BRIDGE_MOD_NAME} left as it is at {link:?}: this binary was compiled against \
             {target:?}, and there is no directory there now"
        ));
    }
    let linked = format!(
        "{BRIDGE_MOD_NAME} is a symlink to {target:?}, so an edit there is what the game loads"
    );
    if fs::read_link(&link).is_ok_and(|existing| existing == target) {
        return Ok(linked);
    }
    if let Ok(metadata) = fs::symlink_metadata(&link) {
        // `symlink_metadata` does not follow, so a symlink reports as one
        // however it is pointed; only a real directory needs `remove_dir_all`.
        let removed = if metadata.file_type().is_dir() {
            fs::remove_dir_all(&link)
        } else {
            fs::remove_file(&link)
        };
        removed.into_diagnostic().map_err(|err| {
            miette!("could not replace {link:?} with a symlink to {target:?}: {err}")
        })?;
    }
    match symlink_dir(&target, &link) {
        Ok(()) => Ok(linked),
        Err(symlink_err) => match copy_dir_recursive(&target, &link) {
            Ok(()) => Ok(format!(
                "{BRIDGE_MOD_NAME} was COPIED from {target:?} because no symlink could be created \
                 ({symlink_err}); a later edit to the checkout reaches the game only on the next setup"
            )),
            Err(copy_err) => Err(miette!(
                "{BRIDGE_MOD_NAME} could not be put at {link:?}: neither a symlink \
                 ({symlink_err}) nor a copy ({copy_err}) of {target:?} could be made"
            )),
        },
    }
}

/// Refuses the run when `<mods>/BotBridge` does not resolve to a readable
/// directory, instead of letting Factorio hang on it.
///
/// **This is the failure the rest of this module's warnings cannot catch, and
/// the only one that costs a whole night.** A server with no bridge mod does
/// not fail: it hangs at `start waiting` forever, and writes a `level.zip`
/// with no bridge state, which poisons every later run on that workspace
/// because Factorio migrates only on a version bump and `info.json` is pinned
/// at 0.0.1. So the check has to happen *before* a process is spawned, and it
/// has to be an error rather than a warning -- a warning here scrolls past
/// and the operator learns about it from a hang twenty minutes later.
///
/// # Why the self-repairing symlink is not enough
///
/// `link_bridge_mod` re-points this at the running binary's own checkout on
/// every setup, so a debug run always repairs whatever the last one left. But
/// it is `#[cfg(debug_assertions)]`: **a release build has no repair path at
/// all**, and the release branch of `resolve_workspace_mods` skips extraction
/// entirely when the directory already exists. That asymmetry is exactly the
/// hole the observed sequence falls through:
///
/// 1. a debug run launched from `.worktrees/x` points the link at
///    `.worktrees/x/mods/BotBridge` -- correct, and by design, for that run;
/// 2. the worktree is removed once its branch merges. `git worktree remove`
///    succeeds cleanly; the damage lands somewhere it does not look;
/// 3. the next **release** run -- which is what every measured run uses --
///    finds a populated `workspace/mods`, extracts nothing, repairs nothing,
///    and hands Factorio a dangling symlink.
///
/// Nothing in that sequence is a mistake anybody makes twice on purpose, and
/// it happened twice in two days here. A `readlink` before a run catches a bad
/// state; it does not stop the next run creating one.
///
/// # What counts as resolving
///
/// `info.json` must be readable, because that is what Factorio itself reads to
/// decide a directory is a mod. Checking `is_dir()` alone would pass a symlink
/// pointing at some unrelated surviving directory, and checking only that the
/// link resolves would pass an empty one left by a half-finished copy.
fn ensure_bridge_mod_resolves(workspace_mods_path: &Path) -> Result<()> {
    let bridge = workspace_mods_path.join(BRIDGE_MOD_NAME);
    let info = bridge.join("info.json");
    if fs::metadata(&info).is_ok_and(|meta| meta.is_file()) {
        return Ok(());
    }

    // Name the link's target when there is one: a dangling symlink is the
    // common case here, and the path it points at is the whole diagnosis.
    let detail = match fs::read_link(&bridge) {
        Ok(target) if !target.exists() => format!(
            "it is a symlink to {target:?}, and there is nothing there -- most likely a git \
             worktree that has since been removed"
        ),
        Ok(target) => {
            format!("it is a symlink to {target:?}, which exists but holds no readable info.json")
        }
        Err(_) if bridge.exists() => {
            String::from("it exists but holds no readable info.json, so Factorio will not load it")
        }
        Err(_) => String::from("there is nothing at that path at all"),
    };

    // Naming the profile is not decoration. The operator who meets this is on
    // release, and the run that broke it was a debug run they may not have
    // made -- so "this build does not repair the link" is the sentence that
    // turns a puzzle into an instruction.
    let profile = if cfg!(debug_assertions) {
        "This is a debug build, which normally repairs this link itself on every setup, so \
         something is wrong beyond a stale link"
    } else {
        "This is a RELEASE build, which never repairs this link -- only a debug run does, and \
         a debug run launched from a git worktree is what points it into one"
    };

    Err(miette!(
        "{BRIDGE_MOD_NAME} does not resolve at {bridge:?}: {detail}. {profile}. Refusing to \
         start, because a Factorio server with no bridge mod does not fail -- it hangs at \
         `start waiting` forever and writes a save with no bridge state, which poisons every \
         later run on this workspace. Repair it by pointing {bridge:?} at a checkout's \
         mods/{BRIDGE_MOD_NAME}, or re-extract the workspace copy with \
         FACTORIO_BOT_REFRESH_MODS=1 on a release build."
    ))
}

/// Makes sure `mod-list.json` still enables the bridge mod, returning a note
/// for the mods line when it had to change something.
///
/// Factorio owns this file and rewrites it on every start, listing what it
/// found. A run started while `workspace/mods/BotBridge` was absent -- which
/// is exactly what "just delete the workspace copy and let it re-seed" used
/// to produce -- comes back with the entry gone, and once the files are
/// restored the mod is still not loaded, silently, because a mod present on
/// disk but absent from an existing `mod-list.json` is a disabled mod. The
/// symlink above cannot fix that on its own.
///
/// A missing `mod-list.json` is left missing on purpose: Factorio writes one
/// from scratch and enables what it finds, which is the outcome we want.
/// Everything already in the file is preserved -- `creative-mod` and `YARM`
/// are listed and deliberately disabled.
fn ensure_bridge_mod_enabled(workspace_mods_path: &Path) -> Option<String> {
    let list_path = workspace_mods_path.join(MOD_LIST_FILENAME);
    if !list_path.exists() {
        return None;
    }
    let mut list = match read_to_value(&list_path) {
        Ok(list) => list,
        Err(err) => {
            return Some(format!(
                "WARNING: {list_path:?} could not be read as JSON ({err}), so whether \
                 {BRIDGE_MOD_NAME} is enabled could not be checked"
            ));
        }
    };
    let Some(mods) = list.get_mut("mods").and_then(Value::as_array_mut) else {
        return Some(format!(
            "WARNING: {list_path:?} has no `mods` array, so whether {BRIDGE_MOD_NAME} is enabled \
             could not be checked"
        ));
    };
    let existing = mods
        .iter_mut()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(BRIDGE_MOD_NAME));
    let note = match existing {
        Some(entry) if entry.get("enabled").and_then(Value::as_bool) == Some(true) => return None,
        Some(entry) => {
            entry["enabled"] = Value::Bool(true);
            format!("{BRIDGE_MOD_NAME} was DISABLED in {list_path:?} and has been re-enabled")
        }
        None => {
            mods.push(serde_json::json!({ "name": BRIDGE_MOD_NAME, "enabled": true }));
            format!("{BRIDGE_MOD_NAME} was MISSING from {list_path:?} and has been added, enabled")
        }
    };
    match write_value_to(&list, &list_path) {
        Ok(()) => Some(note),
        Err(err) => Some(format!(
            "WARNING: {BRIDGE_MOD_NAME} is not enabled in {list_path:?} and it could not be \
             rewritten ({err})"
        )),
    }
}

/// Makes sure the resolved workspace directory exists, creating it when only
/// the leaf is missing.
///
/// Everything under a workspace is derived -- the server and client instances
/// are extracted from the archive, `mods` is populated below, `scripts` is
/// seeded by `scripts::ensure_scripts_dir`, `runs/` appears on the first run --
/// so a `workspace_path` naming a directory that does not exist yet is a new
/// instance to set up, not a setting to correct. Researchers running headless
/// instances side by side point each settings file at its own workspace, and
/// every one of those starts out missing.
///
/// What the old unconditional refusal was actually guarding against is a
/// typo: a path pointing somewhere else entirely. That protection is kept
/// where it is real -- when the **parent** does not exist either, the path is
/// refused with `WorkspaceNotFound`, because creating it (and `create_dir_all`
/// would) turns a mistyped setting into a silently populated stray tree.
///
/// The creation line is narration deliberately not gated on `silent`: every
/// CLI path sets `silent`, which is how the `Using mods directory` line came to
/// print on no run at all, and "where did my workspace go" is exactly the
/// question a user asks later.
pub(crate) fn ensure_workspace_dir(workspace: &crate::paths::ResolvedWorkspace) -> Result<&Path> {
    let workspace_path = workspace.as_path();
    if workspace_path.is_dir() {
        return Ok(workspace_path);
    }
    if workspace_path.exists() {
        error!(
            "Workspace path <bright-blue>{:?}</> exists but is not a directory",
            workspace_path
        );
        return Err(WorkspaceNotFound {}.into());
    }
    match workspace_path.parent() {
        Some(parent) if parent.is_dir() => {
            std::fs::create_dir(workspace_path).into_diagnostic()?;
            info!(
                "Created workspace <bright-blue>{:?}</> (new instance; server, mods and scripts are set up on first run)",
                workspace_path
            );
            Ok(workspace_path)
        }
        _ => {
            error!(
                "Failed to find workspace at <bright-blue>{:?}</>: its parent directory does not exist",
                workspace_path
            );
            Err(WorkspaceNotFound {}.into())
        }
    }
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
    // The point of use for the workspace rule, and the one the CLI reaches:
    // `factorio-bot start` and `factorio-bot lua` hand `settings.factorio`
    // straight to `FactorioInstance::start`, and settings load deliberately no
    // longer refuses a relative `workspace_path` (that refusal used to run
    // inside `Context::new`, ahead of subcommand dispatch, and took `config
    // show` and `config init --force` down with it). Without this check the
    // relative path would be joined against whatever directory the process
    // happened to start in -- the exact hazard `resolve_workspace` exists for.
    //
    // A bare `?`: `RelativeWorkspacePath` is a `Diagnostic`, so the `help`
    // naming the setting and its fix survives into the report.
    let resolved_workspace = crate::paths::resolve_workspace(workspace_path_str)?;
    let workspace_path = ensure_workspace_dir(&resolved_workspace)?;
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
    // Which directory the game is about to load its mods from, and why.
    //
    // Deliberately not gated on `silent`. This is the line CLAUDE.md tells a
    // reader to trust when asking "did my edit ship", and `silent` is true on
    // every path that matters by default: the CLI passes `silent: !verbose`
    // (`cli/lua.rs`, `cli/start.rs`, `repl/factorio_control.rs`) and the
    // server takes `FactorioParams::default()`, which sets it outright. So
    // the one authoritative answer printed only for someone who had already
    // guessed the question and re-run with `--verbose`. It printed zero times
    // across a whole night of runs that were failing for exactly this reason.
    let (workspace_mods_path, mut mods_source) =
        resolve_workspace_mods(workspace_path.join(PathBuf::from(MODS_FOLDERNAME)))?;
    let workspace_mods_path = fs::canonicalize(workspace_mods_path).into_diagnostic()?;
    // Before `ensure_bridge_mod_enabled`, deliberately: enabling a mod in
    // `mod-list.json` whose files do not resolve produces exactly the silent
    // hang this refuses, and would report "re-enabled" while doing it.
    ensure_bridge_mod_resolves(&workspace_mods_path)?;
    if let Some(note) = ensure_bridge_mod_enabled(&workspace_mods_path) {
        mods_source = format!("{mods_source}; {note}");
    }
    info!(
        "Using mods directory <bright-blue>{:?}</> ({})",
        &workspace_mods_path, mods_source
    );
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
    ensure_instance_config_ini(workspace_path, instance_path, silent)?;
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
        // Whether *this call* generated the level. Not the same question as
        // "does level.zip exist" by the time the seed is checked below: the
        // map-exchange branch immediately after this can create it, and a seed
        // consumed there was honoured, not ignored.
        let mut created_level = false;
        if let Some(map_exchange_string) = &map_exchange_string
            && (!map_exchange_string_path.exists()
                || read_to_string(&map_exchange_string_path)
                    .into_diagnostic()?
                    .ne(map_exchange_string))
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
                created_level = true;
                record_map_gen_seed(instance_path, seed.as_deref())?;
            }
            update_map_gen_settings(
                &resolved_workspace,
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

        if saves_level_path.exists() && recreate_save {
            fs::remove_file(&saves_level_path).unwrap_or_else(|_| {
                panic!("failed to delete {}", saves_level_path.to_str().unwrap())
            });
        }
        // The seed only ever reaches Factorio on a `--create`, and `--create`
        // only happens when there is no `level.zip`. So a seed passed against
        // an existing save does *nothing*, and used to do nothing in total
        // silence -- the worst possible outcome for the one flag that makes two
        // runs comparable, because the run then looks controlled and is not.
        // Every experiment run before 2026-09-03 was on whatever map the
        // workspace happened to hold, and no record of any of them names a
        // seed.
        //
        // Deliberately not gated on `silent`. `silent` is set by every CLI path
        // (`silent: !verbose`), which is exactly how the `Using mods directory`
        // line came to print on no run at all. A warning nobody sees is the
        // defect, not the fix.
        if !created_level && saves_level_path.exists() && seed.is_some() {
            warn!(
                "--seed was IGNORED: <bright-blue>{:?}</> already exists, so no map was generated. \
                 The run will use whatever map that save holds. Pass --new to delete it and \
                 generate the seeded map, or point --workspace at an empty directory.",
                saves_level_path
            );
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
            record_map_gen_seed(instance_path, seed.as_deref())?;
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
    }
    Ok(())
}

pub async fn update_map_gen_settings(
    workspace: &crate::paths::ResolvedWorkspace,
    instance_name: &str,
    factorio_port: Option<u16>,
    rcon_settings: &RconSettings,
    map_exchange_string: &str,
    silent: bool,
) -> Result<()> {
    // Takes the resolved type rather than a `&str` it re-derives a `Path`
    // from. `setup_factorio_instance` had already resolved the path and then
    // passed this function the ORIGINAL raw string, so the guarantee stopped
    // one function short of the filesystem work it was meant to cover. That
    // was harmless only because of the order of two checks upstream -- which
    // is the convention this newtype exists to replace.
    let workspace_path = workspace.as_path();
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

/// The `config.ini` shipped with this crate, placeholders and all. Never write
/// it to disk verbatim -- see [`render_config_ini`].
const CONFIG_INI_TEMPLATE: &str = include_str!("../data/config.ini");
/// Stands in for `path.read-data` in [`CONFIG_INI_TEMPLATE`].
const READ_DATA_PLACEHOLDER: &str = "__FACTORIO_BOT_READ_DATA__";
/// Stands in for `path.write-data` in [`CONFIG_INI_TEMPLATE`].
const WRITE_DATA_PLACEHOLDER: &str = "__FACTORIO_BOT_WRITE_DATA__";

/// Renders the config template for one instance by substituting absolute
/// `read-data` / `write-data` paths.
///
/// Absolute, rather than the `__PATH__executable__/../..` form Factorio writes
/// for itself, because that form is unshareable. Factorio resolves it against
/// the *binary's* directory, and this project supports three layouts where the
/// binary sits at different depths: `MacOS/factorio` on macOS,
/// `bin/x64/factorio` on Linux and `bin/x64/factorio.exe` on Windows. The
/// template used to carry the macOS shape, so every generated instance on
/// Linux resolved `read-data` to `<instance>/bin/data` and died at startup
/// with `Error configuring paths: There is no package core in ...` -- before
/// writing a log, and with its stdio nulled, so entirely silently. Counting
/// `../` per platform would work, but absolute paths remove the whole class of
/// bug and were verified against a real Factorio 2.1.17 run, which reports
/// them back as `Read data path:` / `Write data path:`.
///
/// Fails rather than emitting a half-substituted config: a leftover
/// placeholder would reach Factorio as a literal directory name.
fn render_config_ini(read_data: &Path, write_data: &Path) -> Result<String> {
    fn as_value<'a>(path: &'a Path, what: &str) -> Result<&'a str> {
        if !path.is_absolute() {
            return Err(miette!(
                "refusing to write a relative {what} path into config.ini: {path:?}"
            ));
        }
        path.to_str()
            .ok_or_else(|| miette!("{what} path is not valid UTF-8: {path:?}"))
    }
    let rendered = CONFIG_INI_TEMPLATE
        .replace(READ_DATA_PLACEHOLDER, as_value(read_data, "read-data")?)
        .replace(WRITE_DATA_PLACEHOLDER, as_value(write_data, "write-data")?);
    if rendered.contains(READ_DATA_PLACEHOLDER) || rendered.contains(WRITE_DATA_PLACEHOLDER) {
        return Err(miette!(
            "config.ini template still contains a path placeholder after substitution"
        ));
    }
    Ok(rendered)
}

/// Writes `<instance>/config/config.ini` unless it is already there.
///
/// Applies to server and client instances alike: both are launched with an
/// explicit `--config` pointing here, and Factorio refuses to start when that
/// file is missing.
///
/// The "unless it is already there" is deliberate. An instance that was
/// extracted from a real Factorio archive carries the game's own config, which
/// already works; rewriting it would discard whatever the user or the game put
/// in it. The check is on the *file*, not the `config/` directory, because
/// archive extraction leaves that directory behind empty.
fn ensure_instance_config_ini(
    workspace_path: &Path,
    instance_path: &Path,
    silent: bool,
) -> Result<()> {
    let config_path = instance_path.join("config");
    let config_ini_path = config_path.join("config.ini");
    if config_ini_path.exists() {
        return Ok(());
    }
    // `<instance>/data` is a symlink to the shared `<workspace>/data` whenever
    // the latter exists; fall back to the workspace copy for the layouts where
    // the symlink was never made, and to the instance path when neither is
    // present so Factorio reports the missing directory itself.
    let instance_data_path = get_factorio_data_path(instance_path);
    let read_data = if instance_data_path.exists() {
        instance_data_path
    } else {
        let workspace_data_path = workspace_path.join("data");
        if workspace_data_path.exists() {
            workspace_data_path
        } else {
            instance_data_path
        }
    };
    let rendered = render_config_ini(&read_data, instance_path)?;
    std::fs::create_dir_all(&config_path).into_diagnostic()?;
    File::create(&config_ini_path)
        .into_diagnostic()?
        .write_all(rendered.as_bytes())
        .into_diagnostic()?;
    if !silent {
        info!("Created <bright-blue>{:?}</>", &config_ini_path);
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

    /// Reads back the `[path]` values of a rendered config.
    fn path_values(rendered: &str) -> Vec<(String, String)> {
        rendered
            .lines()
            .filter(|line| line.starts_with("read-data=") || line.starts_with("write-data="))
            .map(|line| {
                let (key, value) = line.split_once('=').unwrap();
                (key.to_string(), value.to_string())
            })
            .collect()
    }

    #[test]
    fn the_shipped_template_still_carries_both_placeholders() {
        // If someone pastes a Factorio-generated config over the template,
        // every generated instance silently inherits that machine's layout.
        assert!(CONFIG_INI_TEMPLATE.contains(READ_DATA_PLACEHOLDER));
        assert!(CONFIG_INI_TEMPLATE.contains(WRITE_DATA_PLACEHOLDER));
        // Comments may name the old form; no *setting* may use it -- resolving
        // it against the binary's directory is what broke every Linux client.
        let settings = CONFIG_INI_TEMPLATE
            .lines()
            .filter(|line| !line.trim_start().starts_with(';'));
        for line in settings {
            assert!(
                !line.contains("__PATH__executable__"),
                "platform-relative path setting in the template: {line}"
            );
        }
    }

    #[test]
    fn render_substitutes_absolute_paths() {
        let rendered =
            render_config_ini(Path::new("/ws/client1/data"), Path::new("/ws/client1")).unwrap();
        assert_eq!(
            path_values(&rendered),
            vec![
                ("read-data".to_string(), "/ws/client1/data".to_string()),
                ("write-data".to_string(), "/ws/client1".to_string()),
            ]
        );
        assert!(!rendered.contains(READ_DATA_PLACEHOLDER));
        assert!(!rendered.contains(WRITE_DATA_PLACEHOLDER));
        // The rest of the template has to survive intact.
        assert!(rendered.contains("show-tips-and-tricks=false"));
        assert!(rendered.starts_with("; version=9"));
    }

    #[test]
    fn render_rejects_relative_paths() {
        let err = render_config_ini(Path::new("data"), Path::new("/ws/client1")).unwrap_err();
        assert!(
            err.to_string().contains("read-data"),
            "unexpected error: {err}"
        );
        let err = render_config_ini(Path::new("/ws/client1/data"), Path::new("..")).unwrap_err();
        assert!(
            err.to_string().contains("write-data"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn ensure_writes_a_config_pointing_at_the_instance() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();
        let instance = workspace.join("client1");
        std::fs::create_dir_all(instance.join("data")).unwrap();
        ensure_instance_config_ini(workspace, &instance, true).unwrap();
        let written = std::fs::read_to_string(instance.join("config").join("config.ini")).unwrap();
        assert_eq!(
            path_values(&written),
            vec![
                (
                    "read-data".to_string(),
                    instance.join("data").to_str().unwrap().to_string()
                ),
                (
                    "write-data".to_string(),
                    instance.to_str().unwrap().to_string()
                ),
            ]
        );
    }

    #[test]
    fn ensure_falls_back_to_the_workspace_data_directory() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();
        let instance = workspace.join("client1");
        std::fs::create_dir_all(&instance).unwrap();
        std::fs::create_dir_all(workspace.join("data")).unwrap();
        ensure_instance_config_ini(workspace, &instance, true).unwrap();
        let written = std::fs::read_to_string(instance.join("config").join("config.ini")).unwrap();
        assert_eq!(
            path_values(&written)[0].1,
            workspace.join("data").to_str().unwrap()
        );
    }

    #[test]
    fn ensure_fills_an_empty_config_directory_left_by_extraction() {
        // Archive extraction creates `config/` but no `config.ini`; a check on
        // the directory rather than the file used to skip the write here and
        // leave the instance unable to start.
        let dir = tempdir().unwrap();
        let workspace = dir.path();
        let instance = workspace.join("client1");
        std::fs::create_dir_all(instance.join("config")).unwrap();
        ensure_instance_config_ini(workspace, &instance, true).unwrap();
        assert!(instance.join("config").join("config.ini").exists());
    }

    #[test]
    fn ensure_leaves_an_existing_config_untouched() {
        // `workspace/server` is a real extracted install carrying Factorio's
        // own config. Overwriting it would be a regression, not a fix.
        let dir = tempdir().unwrap();
        let workspace = dir.path();
        let instance = workspace.join("server");
        std::fs::create_dir_all(instance.join("config")).unwrap();
        let existing = "; version=13\n[path]\nread-data=__PATH__executable__/../../data\n";
        std::fs::write(instance.join("config").join("config.ini"), existing).unwrap();
        ensure_instance_config_ini(workspace, &instance, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(instance.join("config").join("config.ini")).unwrap(),
            existing
        );
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

/// `ensure_workspace_dir`: a missing workspace is a first run when its parent
/// exists, and a typo when it does not.
#[cfg(test)]
mod workspace_dir_tests {
    use super::*;
    use crate::paths::resolve_workspace;
    use tempfile::tempdir;

    fn resolved(path: &Path) -> crate::paths::ResolvedWorkspace {
        resolve_workspace(path.to_str().expect("utf-8 temp path")).expect("absolute")
    }

    /// The hl-05 case: `--settings` naming a `workspace_path` that nobody has
    /// created yet, beside a parent that is there. Created and returned.
    #[test]
    fn a_missing_leaf_under_an_existing_parent_is_created() {
        let parent = tempdir().unwrap();
        let leaf = parent.path().join("headless-c");
        assert!(!leaf.exists());

        let workspace = resolved(&leaf);
        let got = ensure_workspace_dir(&workspace).expect("leaf must be created");

        assert_eq!(got, leaf.as_path());
        assert!(leaf.is_dir(), "the workspace directory must now exist");
    }

    /// A parent that does not exist is the typo the refusal guards against:
    /// nothing is created, and the error is the same `WorkspaceNotFound`.
    #[test]
    fn a_missing_parent_is_refused_and_nothing_is_created() {
        let root = tempdir().unwrap();
        let leaf = root.path().join("no-such-parent").join("headless-c");

        let err = ensure_workspace_dir(&resolved(&leaf)).expect_err("missing parent must fail");

        assert!(
            err.downcast_ref::<WorkspaceNotFound>().is_some(),
            "expected WorkspaceNotFound, got {err:?}"
        );
        assert!(
            !leaf.exists(),
            "nothing may be created under a missing parent"
        );
        assert!(
            !leaf.parent().unwrap().exists(),
            "the missing parent must not be created either"
        );
        let help = miette::Diagnostic::help(&WorkspaceNotFound {})
            .map(|h| h.to_string())
            .unwrap_or_default();
        assert!(
            help.contains("parent exists"),
            "help must say the parent has to exist, got: {help}"
        );
    }

    /// An existing workspace is returned as is, contents untouched.
    #[test]
    fn an_existing_directory_is_returned_unchanged() {
        let dir = tempdir().unwrap();
        let marker = dir.path().join("runs");
        std::fs::create_dir(&marker).unwrap();

        let workspace = resolved(dir.path());
        let got = ensure_workspace_dir(&workspace).expect("existing dir is fine");

        assert_eq!(got, dir.path());
        assert!(marker.is_dir(), "existing contents must survive");
    }

    /// A file where the workspace should be is neither a typo nor a first run;
    /// it is refused rather than clobbered.
    #[test]
    fn a_file_at_the_workspace_path_is_refused() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("workspace");
        std::fs::write(&file, b"").unwrap();

        let err = ensure_workspace_dir(&resolved(&file)).expect_err("a file is not a workspace");
        assert!(err.downcast_ref::<WorkspaceNotFound>().is_some());
        assert!(file.is_file(), "the file must be left alone");
    }
}

/// `ensure_bridge_mod_resolves` is the pre-flight that turns a silent hang
/// into a refusal, so these tests are about the *refusal*, in both profiles.
///
/// It is deliberately not in `mods_source_tests`: that module is
/// `#[cfg(all(test, debug_assertions))]` because it is about the symlink a
/// debug build maintains, and **the release build is the one with no repair
/// path** -- gating these the same way would leave the profile that actually
/// needs the guard untested.
#[cfg(test)]
mod bridge_resolves_tests {
    use super::*;
    use tempfile::tempdir;

    /// A mods directory holding a real bridge mod, as a run expects to find.
    fn mods_dir_with_bridge(root: &Path) -> PathBuf {
        let mods = root.join("mods");
        let bridge = mods.join(BRIDGE_MOD_NAME);
        fs::create_dir_all(&bridge).unwrap();
        fs::write(bridge.join("info.json"), br#"{"name":"BotBridge"}"#).unwrap();
        mods
    }

    #[test]
    fn a_real_bridge_directory_resolves() {
        let dir = tempdir().unwrap();
        let mods = mods_dir_with_bridge(dir.path());

        ensure_bridge_mod_resolves(&mods).expect("a directory with info.json is a mod");
    }

    /// The observed failure, end to end: a worktree run points the link into
    /// the worktree, the worktree is removed, the link outlives it.
    ///
    /// The assertion is on the *target being named*. A refusal that says only
    /// "BotBridge does not resolve" sends the reader looking at the mod; the
    /// path is what says a removed worktree took it.
    #[cfg(unix)]
    #[test]
    fn a_symlink_into_a_removed_worktree_is_refused_and_names_it() {
        let dir = tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir_all(&mods).unwrap();

        let worktree_bridge = dir.path().join(".worktrees/ghosts/mods/BotBridge");
        fs::create_dir_all(&worktree_bridge).unwrap();
        fs::write(worktree_bridge.join("info.json"), b"{}").unwrap();
        std::os::unix::fs::symlink(&worktree_bridge, mods.join(BRIDGE_MOD_NAME)).unwrap();

        // It resolves while the worktree stands -- the link is not the defect.
        ensure_bridge_mod_resolves(&mods).expect("a live worktree link is fine");

        fs::remove_dir_all(dir.path().join(".worktrees")).unwrap();

        let err = ensure_bridge_mod_resolves(&mods).expect_err("a dangling link must be refused");
        let msg = format!("{err}");
        assert!(msg.contains("ghosts"), "must name the dead target: {msg}");
        assert!(
            msg.contains("worktree"),
            "must say what that path usually means: {msg}"
        );
    }

    #[test]
    fn a_missing_bridge_is_refused() {
        let dir = tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir_all(&mods).unwrap();

        let err = ensure_bridge_mod_resolves(&mods).expect_err("no bridge mod at all");
        assert!(format!("{err}").contains(BRIDGE_MOD_NAME));
    }

    /// The case `is_dir()` alone would wave through: the directory is there
    /// and empty, which is what a half-finished copy leaves behind.
    #[test]
    fn a_directory_without_info_json_is_refused() {
        let dir = tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir_all(mods.join(BRIDGE_MOD_NAME)).unwrap();

        let err = ensure_bridge_mod_resolves(&mods).expect_err("an empty directory is not a mod");
        assert!(format!("{err}").contains("info.json"));
    }

    /// The refusal has to say what happens if it is ignored, because the
    /// symptom (a server sitting at `start waiting`) names nothing that would
    /// lead anybody back here.
    #[test]
    fn the_refusal_explains_the_hang_it_prevents() {
        let dir = tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir_all(&mods).unwrap();

        let err = ensure_bridge_mod_resolves(&mods).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("start waiting"), "{msg}");
        assert!(msg.contains("FACTORIO_BOT_REFRESH_MODS"), "{msg}");
    }

    /// The refusal names the build profile, because the operator who meets it
    /// is on release and the run that broke the link was a debug run they may
    /// not have made. Asserted per-profile so neither branch can rot.
    #[test]
    fn the_refusal_names_the_build_profile() {
        let dir = tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir_all(&mods).unwrap();

        let msg = format!("{}", ensure_bridge_mod_resolves(&mods).unwrap_err());
        if cfg!(debug_assertions) {
            assert!(msg.contains("debug build"), "{msg}");
        } else {
            assert!(msg.contains("RELEASE build"), "{msg}");
            assert!(msg.contains("worktree"), "must point at the cause: {msg}");
        }
    }
}

/// The debug build's mods resolution: `workspace/mods/BotBridge` is a symlink
/// to the checkout, repaired on every setup, and the "Using mods directory
/// ... (...)" line that says so is printed whether or not the caller asked
/// for silence.
///
/// Two shapes of test here, and the split is not stylistic.
///
/// The filesystem tests call `setup_factorio_instance` directly and then look
/// at the directory it produced -- what the game will load is a fact about
/// the disk, not about a log line, and asserting on the disk is what makes
/// these tests about the actual defect (an edit to `mods/BotBridge` not
/// reaching the game).
///
/// The *printing* tests need a subprocess. `cargo test`'s default per-test
/// capturing redirects `print!`/`println!` -- which is how `paris`'s `info!`
/// ultimately writes -- to an in-memory buffer rather than the real fd, and
/// `std::thread::spawn` does NOT escape it: `thread::Builder::spawn`
/// explicitly propagates the capture setting to the new thread precisely so
/// multi-threaded tests are captured too (tried first here, and confirmed
/// empirically). A `print!` reaches the real stdout fd only in a process
/// where nothing asked libtest to redirect it, which is true of a fresh child
/// invoked with `--nocapture`. Once it is a real child process, its stdout is
/// captured the ordinary way via `Command::output()`.
///
/// The `worker_*` functions are the child-side bodies and are `#[ignore]`d so
/// a normal run never executes them directly.
#[cfg(all(test, debug_assertions))]
mod mods_source_tests {
    use super::*;
    use tempfile::tempdir;

    /// Runs `setup_factorio_instance` far enough to resolve the mods
    /// directory and log the line, and no further.
    ///
    /// `is_server: false` and a fake, never-opened archive path keep this to
    /// filesystem bookkeeping: no real Factorio binary or archive is needed
    /// as long as `instance_path` is pre-populated so the "first run"
    /// extraction branch is skipped.
    ///
    /// `silent: true` is the point, not an oversight. Every CLI path passes
    /// `!verbose` and the server passes `FactorioParams::default()`, so this
    /// is the value real runs use -- and the value under which the mods line
    /// printed on no run at all before this change.
    async fn run_setup(workspace: &Path) {
        let instance_path = workspace.join("client1");
        std::fs::create_dir_all(&instance_path).expect("create instance dir");
        // Non-empty, so `setup_factorio_instance` skips archive extraction.
        std::fs::write(instance_path.join(".placeholder"), b"").expect("write placeholder");

        let rcon_settings = RconSettings::new(4321, "foobar", None);
        setup_factorio_instance(
            workspace.to_str().expect("utf-8 workspace path"),
            "/nonexistent/archive.tar.xz",
            &rcon_settings,
            None,
            "client1",
            false,
            false,
            None,
            None,
            true,
        )
        .await
        .expect("setup must succeed with a pre-populated instance dir");
    }

    /// The checkout this binary was compiled against, canonicalized the same
    /// way `resolve_workspace_mods` canonicalizes it, so a symlink target
    /// compares equal.
    fn repo_mods() -> PathBuf {
        fs::canonicalize(Path::new(repo_mods_path!("")))
            .expect("this test needs the repo checkout it was compiled against")
    }

    fn bridge_link(workspace: &Path) -> PathBuf {
        workspace.join(MODS_FOLDERNAME).join(BRIDGE_MOD_NAME)
    }

    /// A fresh workspace: nothing in it at all. The state a new checkout is in.
    #[tokio::test]
    async fn a_fresh_workspace_gets_a_symlink_to_the_checkout() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");

        run_setup(&workspace).await;

        let link = bridge_link(&workspace);
        assert_eq!(
            fs::read_link(&link).expect("BotBridge must be a symlink"),
            repo_mods().join(BRIDGE_MOD_NAME)
        );
        // The directory holding it must stay a real directory: Factorio
        // rewrites `mod-list.json` and `mod-settings.dat` in there, and every
        // instance's `mods` is a symlink to it.
        assert!(
            fs::symlink_metadata(workspace.join(MODS_FOLDERNAME))
                .expect("workspace/mods must exist")
                .is_dir()
        );
    }

    /// The defect this whole change exists for: an edit to the checkout must
    /// be what the game loads, with no refresh step and no second run.
    #[tokio::test]
    async fn an_edit_to_the_checkout_is_readable_through_the_workspace() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");

        run_setup(&workspace).await;

        // Read through the workspace path the game is handed, and compare
        // against the checkout's bytes as they are right now. A copy taken at
        // setup time would pass this only until the checkout changed, so the
        // real assertion is the symlink above; this one proves the path the
        // game uses actually resolves to those bytes.
        let through_workspace =
            std::fs::read(bridge_link(&workspace).join("control.lua")).expect("read via workspace");
        let in_checkout = std::fs::read(repo_mods().join(BRIDGE_MOD_NAME).join("control.lua"))
            .expect("read via checkout");
        assert_eq!(through_workspace, in_checkout);
    }

    /// The state every existing workspace is in: a real directory copied out
    /// of the checkout at some earlier moment, which is exactly what goes
    /// stale. It must be replaced, not compared and complained about.
    #[tokio::test]
    async fn a_stale_copied_directory_is_replaced_by_the_symlink() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        copy_dir_recursive(&repo_mods(), &workspace_mods).expect("seed a copied workspace");
        let drifted = workspace_mods.join(BRIDGE_MOD_NAME).join("control.lua");
        std::fs::write(&drifted, b"-- a stale copy\n").expect("write the stale copy");

        run_setup(&workspace).await;

        assert_eq!(
            fs::read_link(bridge_link(&workspace)).expect("the copy must become a symlink"),
            repo_mods().join(BRIDGE_MOD_NAME)
        );
        assert_ne!(
            std::fs::read(&drifted).expect("read through the link"),
            b"-- a stale copy\n",
            "the stale bytes survived the repair"
        );
    }

    /// A symlink is not automatically the *right* symlink -- a workspace
    /// carried between checkouts, or a hand-made link, points somewhere else.
    #[tokio::test]
    async fn a_symlink_to_somewhere_else_is_repointed() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        std::fs::create_dir_all(&workspace_mods).expect("create workspace/mods");
        let elsewhere = dir.path().join("some-other-checkout");
        std::fs::create_dir_all(&elsewhere).expect("create the wrong target");
        symlink_dir(&elsewhere, &workspace_mods.join(BRIDGE_MOD_NAME)).expect("wrong symlink");

        run_setup(&workspace).await;

        assert_eq!(
            fs::read_link(bridge_link(&workspace)).expect("still a symlink"),
            repo_mods().join(BRIDGE_MOD_NAME)
        );
    }

    /// A correct symlink must be left alone rather than churned on every
    /// setup -- four clients plus a server means five setups per run.
    #[tokio::test]
    async fn an_already_correct_symlink_is_left_alone() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");

        run_setup(&workspace).await;
        let first = fs::symlink_metadata(bridge_link(&workspace)).expect("symlink metadata");
        run_setup(&workspace).await;
        let second = fs::symlink_metadata(bridge_link(&workspace)).expect("symlink metadata");

        // The inode, not the creation time: `created()` is `None` on several
        // Linux filesystems, and two `None`s compare equal, so that version
        // of this test would pass against a symlink deleted and remade every
        // single setup.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                first.ino(),
                second.ino(),
                "the symlink was deleted and remade rather than recognised as already correct"
            );
        }
        #[cfg(not(unix))]
        {
            let _ = (first, second);
        }
        assert_eq!(
            fs::read_link(bridge_link(&workspace)).expect("still a symlink"),
            repo_mods().join(BRIDGE_MOD_NAME)
        );
    }

    /// The other mods have to come along, or a fresh workspace comes up with
    /// a different mod set than before the symlink existed.
    #[tokio::test]
    async fn the_other_mods_are_seeded_from_the_checkout() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");

        run_setup(&workspace).await;

        for entry in std::fs::read_dir(repo_mods()).expect("read checkout") {
            let name = entry.expect("dir entry").file_name();
            if name == BRIDGE_MOD_NAME {
                continue;
            }
            assert!(
                workspace.join(MODS_FOLDERNAME).join(&name).exists(),
                "{name:?} was not seeded into a fresh workspace"
            );
        }
    }

    /// Factorio owns `mod-list.json` and `mod-settings.dat` and rewrites them
    /// as it runs. Seeding must not put the checkout's versions back over the
    /// live ones -- that would silently undo whatever the game recorded.
    #[tokio::test]
    async fn game_written_state_is_not_overwritten_by_seeding() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        std::fs::create_dir_all(&workspace_mods).expect("create workspace/mods");
        std::fs::write(
            workspace_mods.join("mod-settings.dat"),
            b"written by the game",
        )
        .expect("write mod-settings.dat");

        run_setup(&workspace).await;

        assert_eq!(
            std::fs::read(workspace_mods.join("mod-settings.dat")).expect("read"),
            b"written by the game"
        );
    }

    /// The second half of the failure, and the one a symlink cannot fix on
    /// its own: a run started while `workspace/mods/BotBridge` was absent
    /// comes back with the entry dropped from `mod-list.json`, and a mod
    /// missing from an existing list is a disabled mod however present its
    /// files are.
    #[tokio::test]
    async fn a_mod_list_that_dropped_the_bridge_mod_gets_it_back_enabled() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        std::fs::create_dir_all(&workspace_mods).expect("create workspace/mods");
        std::fs::write(
            workspace_mods.join(MOD_LIST_FILENAME),
            br#"{"mods":[{"name":"base","enabled":true}]}"#,
        )
        .expect("write mod-list.json");

        run_setup(&workspace).await;

        let list = read_to_value(&workspace_mods.join(MOD_LIST_FILENAME)).expect("read list");
        let mods = list["mods"].as_array().expect("mods array");
        let bridge = mods
            .iter()
            .find(|entry| entry["name"] == BRIDGE_MOD_NAME)
            .expect("BotBridge must be listed");
        assert_eq!(bridge["enabled"], Value::Bool(true));
        assert!(
            mods.iter().any(|entry| entry["name"] == "base"),
            "the rest of the list must survive"
        );
    }

    /// The same, one step less obvious: the entry is there and says `false`.
    /// Present on disk, listed, and not loaded.
    #[tokio::test]
    async fn a_mod_list_that_disables_the_bridge_mod_re_enables_it() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        std::fs::create_dir_all(&workspace_mods).expect("create workspace/mods");
        std::fs::write(
            workspace_mods.join(MOD_LIST_FILENAME),
            br#"{"mods":[{"name":"BotBridge","enabled":false},{"name":"YARM","enabled":false}]}"#,
        )
        .expect("write mod-list.json");

        run_setup(&workspace).await;

        let list = read_to_value(&workspace_mods.join(MOD_LIST_FILENAME)).expect("read list");
        let mods = list["mods"].as_array().expect("mods array");
        assert_eq!(
            mods.iter()
                .find(|entry| entry["name"] == BRIDGE_MOD_NAME)
                .expect("BotBridge listed")["enabled"],
            Value::Bool(true)
        );
        // A mod deliberately disabled must stay disabled: "make sure the
        // bridge mod is on" is not "turn everything on".
        assert_eq!(
            mods.iter()
                .find(|entry| entry["name"] == "YARM")
                .expect("YARM listed")["enabled"],
            Value::Bool(false)
        );
    }

    /// An absent `mod-list.json` is left absent: Factorio writes one from
    /// scratch and enables what it finds, which is the outcome we want.
    /// Writing a partial one ourselves would *disable* every mod we did not
    /// think to list.
    #[tokio::test]
    async fn an_absent_mod_list_is_not_invented() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        std::fs::create_dir_all(&workspace_mods).expect("create workspace/mods");
        // Seeded from the checkout otherwise, which ships one.
        std::fs::write(workspace_mods.join(MOD_LIST_FILENAME), b"").expect("placeholder");
        std::fs::remove_file(workspace_mods.join(MOD_LIST_FILENAME)).expect("remove");

        run_setup(&workspace).await;

        // Seeding put the checkout's list back, which is fine -- it enables
        // BotBridge. What must not happen is a list this code invented.
        let path = workspace_mods.join(MOD_LIST_FILENAME);
        if path.exists() {
            let list = read_to_value(&path).expect("read list");
            assert!(
                list["mods"]
                    .as_array()
                    .expect("mods array")
                    .iter()
                    .any(|entry| entry["name"] == BRIDGE_MOD_NAME),
                "the seeded list must enable BotBridge"
            );
        }
    }

    /// Spawns this same test binary as a child process, running only
    /// `worker_name` (which must be `#[ignore]`d so the outer, uninstructed
    /// run never executes it), with `--nocapture` so its real stdout reaches
    /// the pipe `Command::output()` reads back.
    fn run_worker_and_capture_stdout(worker_name: &str) -> String {
        let exe = std::env::current_exe().expect("current test binary path");
        let output = std::process::Command::new(exe)
            .args([
                "--exact",
                worker_name,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .output()
            .expect("spawn worker subprocess");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "worker {worker_name} failed (status {:?}); stdout:\n{stdout}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    }

    /// Not run directly -- see `names_the_workspace_directory_and_the_symlink`.
    #[tokio::test]
    #[ignore]
    async fn worker_symlinked_workspace_case() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        // The marker the driver test (a different process, which does not
        // otherwise know this randomly-named tempdir path) checks the printed
        // line against.
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        std::fs::create_dir_all(&workspace_mods).expect("create workspace/mods");
        println!(
            "WORKSPACE_MODS={}",
            fs::canonicalize(&workspace_mods)
                .expect("canonicalize")
                .display()
        );
        run_setup(&workspace).await;
    }

    /// The line that CLAUDE.md tells a reader to trust must print under
    /// `silent: true` -- the value every real run uses -- and must name both
    /// the directory in use and the checkout the bridge mod now points at.
    #[test]
    fn names_the_workspace_directory_and_the_symlink() {
        let repo_bridge = repo_mods().join(BRIDGE_MOD_NAME);

        let output = run_worker_and_capture_stdout(
            "process::instance_setup::mods_source_tests::worker_symlinked_workspace_case",
        );

        assert!(
            output.contains("Using mods directory"),
            "no mods-source line printed at all under silent: true, which is what every real \
             run passes: {output}"
        );
        // Not `strip_prefix`: under `--nocapture` libtest prints `test <name>
        // ... ` immediately before the test's own output starts, on the same
        // line, so the marker is not necessarily at the start of its line.
        let workspace_mods = output
            .lines()
            .find_map(|line| {
                line.rsplit_once("WORKSPACE_MODS=")
                    .map(|(_, rest)| rest.trim())
            })
            .unwrap_or_else(|| panic!("worker did not print its WORKSPACE_MODS marker: {output}"));
        // The directory named as *in use* must be the workspace one, not the
        // checkout -- the checkout appears later in the same line as the
        // symlink target, so this looks at the name, not at the whole line.
        let named_as_in_use = output
            .split_once("Using mods directory ")
            .map(|(_, rest)| rest.split_once(" (").map_or(rest, |(dir, _)| dir))
            .expect("the mods-source line names a directory");
        assert!(
            named_as_in_use.contains(workspace_mods),
            "the directory named as in use is {named_as_in_use}, not the workspace copy"
        );
        assert!(
            output.contains("is a symlink to"),
            "the line must say the bridge mod is a symlink, got: {output}"
        );
        assert!(
            output.contains(repo_bridge.to_str().expect("utf-8 checkout path")),
            "the line must name the checkout {repo_bridge:?} it points at, got: {output}"
        );
    }

    /// Not run directly -- see `announces_a_repaired_mod_list`.
    #[tokio::test]
    #[ignore]
    async fn worker_disabled_mod_list_case() {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        let workspace_mods = workspace.join(MODS_FOLDERNAME);
        std::fs::create_dir_all(&workspace_mods).expect("create workspace/mods");
        std::fs::write(
            workspace_mods.join(MOD_LIST_FILENAME),
            br#"{"mods":[{"name":"BotBridge","enabled":false}]}"#,
        )
        .expect("write mod-list.json");
        run_setup(&workspace).await;
    }

    /// Repairing `mod-list.json` silently would hide the fact that a previous
    /// run disabled the mod -- which is a symptom of something, not a normal
    /// state. It is reported on the same line as the directory.
    #[test]
    fn announces_a_repaired_mod_list() {
        let output = run_worker_and_capture_stdout(
            "process::instance_setup::mods_source_tests::worker_disabled_mod_list_case",
        );

        assert!(
            output.contains("DISABLED") && output.contains("re-enabled"),
            "a mod-list repair must be announced, got: {output}"
        );
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
        let encoder = liblzma::write::XzEncoder::new(file, 1);
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

    /// Settings load no longer refuses a relative `workspace_path` -- it ran
    /// inside `Context::new`, ahead of subcommand dispatch, and so refused
    /// `config show` and `config init --force` as well. The refusal lives here
    /// instead, at the point of use, which is what `factorio-bot start` and
    /// `factorio-bot lua` reach: they hand `settings.factorio` straight to
    /// `FactorioInstance::start`. Without it a relative path is joined against
    /// the process's working directory, which is the whole reason the rule
    /// exists.
    ///
    /// The `help` is asserted too, not just the message: it is the only
    /// sentence that says what to do, and it is dropped by any conversion that
    /// reformats the error (`miette!("{err}")`, `.into_diagnostic()`) instead
    /// of letting `?` carry the `Diagnostic` through.
    #[tokio::test]
    async fn a_relative_workspace_is_refused_before_anything_is_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("factorio.tar.xz");
        std::fs::write(&archive, b"not really an archive").expect("writes the archive");
        let rcon_settings = RconSettings::new(4321, "foobar", None);

        let report = setup_factorio_instance(
            "relative-ws",
            archive.to_str().expect("utf-8 archive path"),
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
        .expect_err("a relative workspace_path must be refused");

        assert!(
            report.to_string().contains("must be absolute"),
            "unexpected error: {report:?}"
        );
        let diagnostic: &dyn miette::Diagnostic = report.as_ref();
        let help = diagnostic
            .help()
            .map(|help| help.to_string())
            .expect("the report must keep the help that says how to fix it");
        assert!(
            help.contains("absolute path"),
            "unexpected help text: {help}"
        );
        assert!(
            !std::path::Path::new("relative-ws").exists(),
            "the refusal must happen before anything is created next to the cwd"
        );
    }
}
