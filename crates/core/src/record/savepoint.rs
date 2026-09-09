//! Milestone savepoints: the world as it stood the moment a milestone closed.
//!
//! `researched("automation")` costs about twenty minutes of run time and has
//! satisfied repeatedly and deterministically. Every experiment past it pays
//! that as a prelude, and red and green science both sit behind it. A savepoint
//! is how that prelude gets paid once: the run that reaches a milestone writes
//! the game out, and a later run starts from that file instead of re-deriving
//! the world.
//!
//! # Three things this module is careful about
//!
//! **A save is not finished when the call returns.** `game.server_save` is
//! asynchronous -- the engine writes at the end of a tick, and the RCON reply
//! comes back long before the bytes land. Factorio writes into
//! `<name>.tmp.zip` and renames it onto `<name>.zip` when it is done, which is
//! what makes [`save_is_complete`] possible; the rename is atomic within a
//! directory, so the final name never names a half-written file. That is
//! observed rather than assumed: this workspace's `saves/` held a 741,376-byte
//! `level.tmp.zip` left by a server killed mid-save, and it does not parse as a
//! zip at all while every finished save beside it does. The check here does not
//! *rely* on the rename, though -- it opens the archive and demands the
//! entries a Factorio save has, so a build that wrote in place would be caught
//! by the missing end-of-central-directory rather than trusted.
//!
//! **A save carries mod state.** `script.dat` inside the archive is
//! BotBridge's `storage`, which holds the sampling session's run id, per-player
//! walk and mining state, and the craft and research waiters keyed by
//! `ActionId`. Action ids are minted `% 1000` per run
//! (`FactorioRcon::next_action_id`), so a waiter left over from the run that
//! took the save *will* collide with a live id in the run that resumes it, and
//! settle the wrong action. Factorio only migrates `storage` on a version bump
//! and `mods/BotBridge/info.json` is pinned at `0.0.1`, so nothing in the game
//! will ever do this for us. Two defences: the resuming run calls
//! `rcon_session_reset` before it does anything (see
//! `crates/core/src/process/process_control.rs`), and a savepoint records the
//! [`ModFingerprint`] it was written under so [`check_mods`] can refuse a save
//! written by different mod code.
//!
//! **A resumed run is not benchmark-comparable.** It starts on accumulated
//! world state, so its timings measure a different thing. The run that resumes
//! records [`Savepoint::provenance_label`] in `provenance.json`'s
//! `resumed_from`, and `tools/run_analysis.py --compare` already refuses a
//! comparison across differing `resumed_from`.
//!
//! # Where savepoints live, and how they are reaped
//!
//! Inside the run directory that produced them: `runs/<run_id>/savepoints/`.
//! That makes a savepoint identifiable back to its run and milestone from its
//! path alone, keeps it beside the `provenance.json` that says which map and
//! commit it came from -- and, most of the point, makes it inherit
//! [`super::retention::reap`] rather than needing a second policy that could
//! disagree with the first. A savepoint cannot outlive the run record that
//! explains it, which is exactly the failure the nine manifest-less runs are.
//! `.keep` ([`super::retention::KEEP_MARKER`]) protects a run's savepoints
//! along with everything else in it.

use crate::record::provenance::PROVENANCE_FILE;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The schema version this build writes. Readers must accept any schema at or
/// below their own, the same rule [`super::provenance`] and [`super::samples`]
/// follow.
pub const SAVEPOINT_SCHEMA: u32 = 1;

/// Subdirectory of a run directory holding its savepoints.
pub const SAVEPOINTS_DIR: &str = "savepoints";

/// Written into the server instance directory when a run was started from a
/// savepoint, and **deleted when it was not**.
///
/// The deletion is the load-bearing half. `map-gen-seed.txt` is the cautionary
/// tale: a start-time fact left on disk by one run and read by the next
/// describes the wrong run, and reads as data rather than as staleness. See
/// [`clear_resume_marker`].
pub const RESUME_MARKER: &str = "resumed-from.json";

/// How long [`await_save`] waits for the engine to finish writing.
///
/// Generous on purpose: the save runs inside the game loop on a machine that
/// may also be running four graphical clients. A 1.5 MB save takes well under
/// a second when nothing else is happening, and the cost of waiting too long
/// is a slow milestone boundary, where the cost of giving up too early is a
/// milestone with no savepoint and a log line saying so.
pub const SAVE_TIMEOUT: Duration = Duration::from_secs(180);

/// How long [`await_save`] gives the engine to produce *anything* -- either the
/// finished file or the `.tmp.zip` it writes through -- before concluding that
/// it is not writing here at all.
///
/// The engine starts within a tick or two, so this is generous for a save that
/// is happening. It exists for the case where the save is happening
/// *somewhere else*: `factorio-bot lua --connect` drives a server this process
/// did not start, whose saves directory is on another machine's disk, and
/// without this bound every milestone of such a run would stall for
/// [`SAVE_TIMEOUT`] before admitting it. Failing in fifteen seconds with a
/// message that names the directory it watched is the useful answer there.
const APPEARANCE_TIMEOUT: Duration = Duration::from_secs(15);

/// How often [`await_save`] looks.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// What went wrong taking or resolving a savepoint.
///
/// A savepoint failing must never fail the run that was taking it -- the run's
/// job is the milestone, not the souvenir -- so every caller here is expected
/// to record this and carry on. It is an enum rather than a string so a caller
/// can tell "the game never finished writing" from "there is no such
/// savepoint".
#[derive(Debug, thiserror::Error)]
pub enum SavepointError {
    #[error("the game did not finish writing {path:?} within {waited:?}: {why}")]
    NeverFinished {
        path: PathBuf,
        waited: Duration,
        why: String,
    },
    #[error("no savepoint for run {run_id}{}", match milestone_index {
        Some(index) => format!(", milestone {index}"),
        // `None` is "the run was named without a milestone", and printing a
        // milestone number the caller never gave sends them looking for a
        // milestone 0 that was never asked about.
        None => String::new(),
    })]
    NotFound {
        run_id: String,
        milestone_index: Option<u32>,
    },
    #[error("savepoint metadata {path:?} is not readable: {source}")]
    Unreadable {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("savepoint schema {found} is newer than this build understands ({SAVEPOINT_SCHEMA})")]
    FutureSchema { found: u32 },
    #[error(
        "refusing to resume: {explain}\n\
         Resuming anyway is a real option -- pass --resume-force -- but the run will be recorded \
         as having done so."
    )]
    ModMismatch { explain: String },
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("rcon: {0}")]
    Rcon(String),
}

/// The mod code a savepoint was written under.
///
/// # Why a fingerprint and not the version number
///
/// `mods/BotBridge/info.json` is pinned at `0.0.1` and has been for the whole
/// life of this project, because bumping it makes Factorio run migrations.
/// So the version says nothing about whether `control.lua` changed, and
/// `control.lua` is what decides how `storage` is shaped and read. Two saves
/// with the same version and different code is the *ordinary* case here, not
/// an exotic one.
///
/// # What the digest is, and is not
///
/// CRC-32 over every file in the mod directory, sorted by relative path, with
/// each path and length folded in so that renames and truncations move it. It
/// detects drift; it is not a cryptographic commitment and must not be
/// described as one. Nothing here is defending against an adversary -- the
/// question is only "is this the same mod I ran last week".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ModFingerprint {
    /// `version` out of the mod's `info.json`, when it could be read.
    pub version: Option<String>,
    /// Lowercase hex CRC-32 over the sorted file list. See the type docs.
    pub digest: String,
    /// How many files went into the digest. A digest over a directory that was
    /// half missing would otherwise look like an ordinary mismatch.
    pub files: usize,
}

impl ModFingerprint {
    /// Fingerprints one mod directory -- `<workspace>/mods/BotBridge`, which is
    /// a symlink to the repo copy in a debug build and a real directory in a
    /// release one. Either way this reads what the game loads.
    ///
    /// `None` when the directory cannot be read at all. That is "we do not
    /// know", and [`check_mods`] treats it as such rather than as a match.
    pub fn of(mod_dir: &Path) -> Option<Self> {
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        collect_files(mod_dir, mod_dir, &mut files).ok()?;
        if files.is_empty() {
            return None;
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut crc = flate2::Crc::new();
        for (rel, bytes) in &files {
            crc.update(rel.as_bytes());
            crc.update(&(bytes.len() as u64).to_le_bytes());
            crc.update(bytes);
        }
        let version = fs::read_to_string(mod_dir.join("info.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|json| {
                json.get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            });
        Some(ModFingerprint {
            version,
            digest: format!("{:08x}", crc.sum()),
            files: files.len(),
        })
    }
}

/// Reads every file under `dir`, relative to `root`, following the directory
/// symlink a debug workspace uses for `mods/BotBridge`.
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // `metadata` rather than `file_type`: the latter reports the symlink,
        // and `mods/BotBridge` is one in every debug workspace.
        let meta = fs::metadata(&path)?;
        if meta.is_dir() {
            collect_files(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, fs::read(&path)?));
        }
    }
    Ok(())
}

/// Whether the mod that wrote a savepoint is the mod that would load it.
///
/// The dangerous verdict is [`ModCheck::Changed`] with `version_bumped: false`:
/// Factorio migrates `storage` **only** on a version bump, so changed code
/// under an unchanged version number means the new `control.lua` reads the old
/// run's `storage` with no migration and no complaint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModCheck {
    /// Byte-for-byte the same mod.
    Same,
    /// Different code. `version_bumped` says whether Factorio would at least
    /// have run a migration -- it would not, today, since `info.json` is
    /// pinned.
    Changed {
        saved: String,
        current: String,
        version_bumped: bool,
    },
    /// One side has no fingerprint. Never reported as a match: a savepoint
    /// written before this field existed, or a mods directory that could not
    /// be read, are both "we cannot tell".
    Unknown { why: String },
}

impl ModCheck {
    /// True only for [`ModCheck::Same`]. `Unknown` is deliberately not safe --
    /// see [`crate::record::provenance`] on absent evidence.
    pub fn is_safe(&self) -> bool {
        matches!(self, ModCheck::Same)
    }

    /// One line, for a refusal message or a warning.
    pub fn explain(&self) -> String {
        match self {
            ModCheck::Same => "the savepoint was written by this exact mod code".to_string(),
            ModCheck::Changed {
                saved,
                current,
                version_bumped,
            } => format!(
                "the savepoint was written by BotBridge {saved} and this workspace has {current}. \
                 The save carries that mod's `storage` -- walk state, craft and research waiters, \
                 the sampling run id -- and Factorio migrates storage only on a version bump, \
                 which {}.",
                if *version_bumped {
                    "did happen here"
                } else {
                    "did NOT happen here: info.json is pinned at the same version"
                }
            ),
            ModCheck::Unknown { why } => format!(
                "cannot tell whether the mod changed since the savepoint was written: {why}"
            ),
        }
    }
}

/// Compares the mod a savepoint was written under against the one on disk now.
pub fn check_mods(saved: Option<&ModFingerprint>, current: Option<&ModFingerprint>) -> ModCheck {
    match (saved, current) {
        (Some(saved), Some(current)) => {
            if saved == current {
                ModCheck::Same
            } else {
                ModCheck::Changed {
                    saved: describe(saved),
                    current: describe(current),
                    version_bumped: saved.version != current.version,
                }
            }
        }
        (None, _) => ModCheck::Unknown {
            why: "the savepoint records no mod fingerprint (written before this existed)"
                .to_string(),
        },
        (_, None) => ModCheck::Unknown {
            why: "this workspace's mods directory could not be fingerprinted".to_string(),
        },
    }
}

fn describe(fp: &ModFingerprint) -> String {
    format!(
        "{} ({}, {} files)",
        fp.version.as_deref().unwrap_or("version unknown"),
        fp.digest,
        fp.files
    )
}

/// One savepoint's metadata, written beside the `.zip` it describes.
///
/// Deliberately small. Seed, map fingerprint, git commit, Factorio version and
/// build profile are **not** copied here: they are start-of-run facts and they
/// are already in `provenance.json` one directory up, which this file is
/// guaranteed to sit next to. Copying them would create a second source of
/// truth for values that cannot change mid-run, and second sources of truth in
/// this archive have been wrong before.
///
/// [`ModFingerprint`] is the exception, and it is here because provenance does
/// not carry it and it is the one fact that decides whether this file can be
/// safely loaded at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Savepoint {
    pub schema: u32,
    /// The run that produced it. Also the name of the directory two levels up,
    /// so the file identifies itself even if it is copied somewhere else.
    pub run_id: String,
    /// Which milestone had just been satisfied, as numbered by the supervisor.
    pub milestone_index: u32,
    /// The game tick the save was requested at -- the tick the world in it is
    /// (very nearly) at. The engine writes at the end of a tick, so the world
    /// on disk may be a tick or two later; nothing here needs it exact and
    /// claiming it would be a lie.
    pub tick: u64,
    /// Unix seconds at which the file finished being written.
    pub created_unix: u64,
    /// Size of the `.zip`, for the reader deciding whether the archive is
    /// getting expensive.
    pub bytes: u64,
    /// Name of the save file, relative to this metadata file.
    pub file: String,
    /// The mod code the save was written under. See [`ModFingerprint`].
    pub mods: Option<ModFingerprint>,
}

impl Savepoint {
    /// What `provenance.resumed_from` records for a run started from this.
    ///
    /// Stable and comparable: two runs resumed from the same savepoint produce
    /// the same string, which is what lets `--compare` treat them as sharing a
    /// starting world, and two runs resumed from different ones differ, which
    /// is what makes it refuse.
    pub fn provenance_label(&self) -> String {
        format!("{}/milestone-{}", self.run_id, self.milestone_index)
    }

    /// `savepoints/milestone-<n>.zip` inside a run directory.
    pub fn zip_path(run_dir: &Path, milestone_index: u32) -> PathBuf {
        run_dir
            .join(SAVEPOINTS_DIR)
            .join(format!("milestone-{milestone_index}.zip"))
    }

    /// `savepoints/milestone-<n>.json` inside a run directory.
    pub fn meta_path(run_dir: &Path, milestone_index: u32) -> PathBuf {
        run_dir
            .join(SAVEPOINTS_DIR)
            .join(format!("milestone-{milestone_index}.json"))
    }

    /// Writes the metadata beside the save.
    pub fn write(&self, run_dir: &Path) -> io::Result<PathBuf> {
        let path = Self::meta_path(run_dir, self.milestone_index);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(&path, json.as_bytes())?;
        Ok(path)
    }

    /// Reads one savepoint's metadata.
    pub fn read(meta_path: &Path) -> Result<Self, SavepointError> {
        let bytes = fs::read(meta_path)?;
        let savepoint: Savepoint =
            serde_json::from_slice(&bytes).map_err(|source| SavepointError::Unreadable {
                path: meta_path.to_path_buf(),
                source,
            })?;
        if savepoint.schema > SAVEPOINT_SCHEMA {
            return Err(SavepointError::FutureSchema {
                found: savepoint.schema,
            });
        }
        Ok(savepoint)
    }
}

/// A savepoint together with where its `.zip` actually is.
#[derive(Debug, Clone, PartialEq)]
pub struct LocatedSavepoint {
    pub savepoint: Savepoint,
    /// Absolute (or caller-relative) path to the save file itself.
    pub zip: PathBuf,
    /// The run directory it belongs to, which also holds its
    /// [`PROVENANCE_FILE`].
    pub run_dir: PathBuf,
}

impl LocatedSavepoint {
    /// The `provenance.json` of the run that produced this savepoint, if it is
    /// still there. A savepoint whose run record was reaped is a savepoint
    /// nobody can identify -- which is why they live inside the run directory
    /// in the first place, so this can only be `None` for a savepoint someone
    /// moved by hand.
    pub fn provenance_path(&self) -> PathBuf {
        self.run_dir.join(PROVENANCE_FILE)
    }
}

/// Every savepoint under a runs root, newest run first, ascending milestone
/// within a run.
pub fn list_savepoints(runs_root: &Path) -> Vec<LocatedSavepoint> {
    let mut found: Vec<LocatedSavepoint> = Vec::new();
    let Ok(entries) = fs::read_dir(runs_root) else {
        return found;
    };
    let mut run_dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join(SAVEPOINTS_DIR).is_dir())
        .collect();
    run_dirs.sort();
    run_dirs.reverse();
    for run_dir in run_dirs {
        let Ok(saves) = fs::read_dir(run_dir.join(SAVEPOINTS_DIR)) else {
            continue;
        };
        let mut metas: Vec<PathBuf> = saves
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .collect();
        metas.sort();
        for meta in metas {
            let Ok(savepoint) = Savepoint::read(&meta) else {
                continue;
            };
            let zip = meta.with_file_name(&savepoint.file);
            if !zip.is_file() {
                continue;
            }
            found.push(LocatedSavepoint {
                savepoint,
                zip,
                run_dir: run_dir.clone(),
            });
        }
    }
    found
}

/// Resolves a `<run_id>` / `<run_id>:<milestone>` / path reference to one
/// savepoint.
///
/// With no milestone named, the **highest-numbered** milestone of that run is
/// taken: the furthest the run got is the state worth resuming from, and
/// asking for a run by name and getting its first milestone would be a
/// surprise. A path is taken as-is, which is how a savepoint copied out of a
/// reaped run can still be used.
pub fn resolve_savepoint(
    runs_root: &Path,
    reference: &str,
) -> Result<LocatedSavepoint, SavepointError> {
    let as_path = Path::new(reference);
    if as_path.extension().is_some_and(|ext| ext == "zip") && as_path.is_file() {
        let meta_path = as_path.with_extension("json");
        let run_dir = as_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let savepoint = Savepoint::read(&meta_path)?;
        return Ok(LocatedSavepoint {
            savepoint,
            zip: as_path.to_path_buf(),
            run_dir,
        });
    }

    let (run_id, milestone) = match reference.split_once(':') {
        Some((run, milestone)) => (
            run.to_string(),
            Some(milestone.parse::<u32>().map_err(|_| {
                SavepointError::Rcon(format!(
                    "\"{milestone}\" is not a milestone number; expected <run-id>:<n>"
                ))
            })?),
        ),
        None => (reference.to_string(), None),
    };

    let mut candidates: Vec<LocatedSavepoint> = list_savepoints(runs_root)
        .into_iter()
        .filter(|found| found.savepoint.run_id == run_id)
        .filter(|found| milestone.is_none_or(|m| found.savepoint.milestone_index == m))
        .collect();
    candidates.sort_by_key(|found| found.savepoint.milestone_index);
    candidates.pop().ok_or(SavepointError::NotFound {
        run_id,
        milestone_index: milestone,
    })
}

/// Where the engine writes a named save, and where it writes it *first*.
///
/// `game.server_save("x")` produces `<instance>/saves/x.zip`, via
/// `<instance>/saves/x.tmp.zip`.
pub fn save_paths(saves_dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    (
        saves_dir.join(format!("{name}.zip")),
        saves_dir.join(format!("{name}.tmp.zip")),
    )
}

/// Why a file is not (yet) a finished Factorio save.
///
/// Returned rather than collapsed to a bool so that a timeout can say what it
/// was still waiting for -- "the file never appeared" and "the file is there
/// but has no central directory" are different failures with different causes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotYet {
    Missing,
    /// The engine's own temp file is still there.
    StillWriting,
    /// The zip's central directory could not be read: the last thing a zip
    /// writer writes, so its absence means the writer has not finished.
    NotAZip(String),
    /// It is a zip, but not a save: no `level.dat*` and no `level-init.dat`.
    NotASave,
}

impl std::fmt::Display for NotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotYet::Missing => write!(f, "the file does not exist"),
            NotYet::StillWriting => write!(f, "the engine's .tmp.zip is still present"),
            NotYet::NotAZip(why) => write!(f, "not a readable zip yet ({why})"),
            NotYet::NotASave => write!(f, "a zip, but with no level.dat inside"),
        }
    }
}

/// Whether `path` is a complete Factorio save, and how big it is.
///
/// # What "complete" is established from
///
/// A zip's end-of-central-directory record is written **last**, so an archive
/// that opens and lists its entries is an archive whose writer finished. On
/// top of that we demand the entries a save actually has (`level.dat*` and
/// `level-init.dat`, both under the save's own top-level directory), so that a
/// zip which is merely well-formed cannot pass for a world.
///
/// This is deliberately not "the log said `Saving finished`". Nineteen of
/// twenty walk failures in this project were once archived as `kind: "other"`
/// because a classifier matched on wording the build had changed; a check that
/// reads the artefact cannot go stale that way.
pub fn save_is_complete(path: &Path) -> Result<u64, NotYet> {
    let meta = fs::metadata(path).map_err(|_| NotYet::Missing)?;
    let file = fs::File::open(path).map_err(|err| NotYet::NotAZip(err.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| NotYet::NotAZip(err.to_string()))?;
    let mut has_level = false;
    let mut has_init = false;
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else {
            return Err(NotYet::NotAZip(format!("entry {i} is unreadable")));
        };
        let name = entry.name().rsplit('/').next().unwrap_or_default();
        // `level.dat0`, `level.dat1`, `level.datmetadata` in a 2.x save; plain
        // `level.dat` in older ones. Prefix, so both pass.
        if name.starts_with("level.dat") {
            has_level = true;
        }
        if name == "level-init.dat" {
            has_init = true;
        }
        if has_level && has_init {
            return Ok(meta.len());
        }
    }
    Err(NotYet::NotASave)
}

/// Waits for the engine to finish writing a named save, and returns its path.
///
/// Polls the artefact rather than the log. Gives up after `timeout` with a
/// message naming what it was still waiting for -- a savepoint that never
/// appeared must say so out loud, because a milestone with no savepoint looks
/// exactly like a milestone nobody asked to save.
pub async fn await_save(
    saves_dir: &Path,
    name: &str,
    timeout: Duration,
) -> Result<(PathBuf, u64), SavepointError> {
    await_save_bounded(saves_dir, name, timeout, APPEARANCE_TIMEOUT).await
}

/// [`await_save`] with both bounds named, so a test can exercise the
/// appearance bound without spending [`APPEARANCE_TIMEOUT`] of wall clock on
/// it. Tokio's paused clock is no help here: the loop is bounded on
/// `std::time::Instant`, which a paused runtime does not move.
async fn await_save_bounded(
    saves_dir: &Path,
    name: &str,
    timeout: Duration,
    appearance: Duration,
) -> Result<(PathBuf, u64), SavepointError> {
    let (final_path, tmp_path) = save_paths(saves_dir, name);
    let started = Instant::now();
    let mut last;
    loop {
        // The temp file is checked first only so the *reason* reads correctly
        // in a timeout: `save_is_complete` alone is the verdict, since a
        // `.tmp.zip` left behind by some earlier killed server would otherwise
        // block a save that has in fact landed.
        match save_is_complete(&final_path) {
            Ok(bytes) => return Ok((final_path, bytes)),
            Err(why) => {
                last = if tmp_path.exists() && matches!(why, NotYet::Missing) {
                    NotYet::StillWriting
                } else {
                    why
                };
            }
        }
        let waited = started.elapsed();
        let nothing_yet = matches!(last, NotYet::Missing);
        if waited >= timeout || (nothing_yet && waited >= appearance) {
            return Err(SavepointError::NeverFinished {
                path: final_path,
                waited,
                why: if nothing_yet {
                    format!(
                        "{last} -- nothing at all appeared in {}, so the server is very likely \
                         writing somewhere else (an attached server saves on its own machine)",
                        saves_dir.display()
                    )
                } else {
                    last.to_string()
                },
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// The name the engine is asked to save under.
///
/// Includes the run id so two runs saving the same milestone at the same time
/// -- or a stale file from a run that crashed after asking -- cannot be
/// mistaken for one another. Restricted to the characters
/// `rcon_savepoint` accepts, which is what keeps the name inside the saves
/// directory: everything here is already `[A-Za-z0-9._-]`, but a run id is
/// data and is sanitised rather than trusted.
pub fn engine_save_name(run_id: &str, milestone_index: u32) -> String {
    let safe: String = run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("savepoint-{safe}-m{milestone_index}")
}

/// The BotBridge directory the game loads, given a workspace.
///
/// `<workspace>/mods` is what `resolve_workspace_mods`
/// (`crate::process::instance_setup`) hands the instance in both build
/// profiles, and every instance's own `mods` is a symlink to it. In a debug
/// workspace `BotBridge` under it is itself a symlink to the checkout, which
/// is exactly what should be fingerprinted: it is the code that ran.
pub fn bridge_mod_dir(workspace: &Path) -> PathBuf {
    workspace.join("mods").join("BotBridge")
}

/// Everything [`capture`] needs to know that it cannot work out for itself.
#[derive(Debug, Clone)]
pub struct CaptureRequest<'a> {
    /// The server instance directory, `<workspace>/server`. Its `saves/` is
    /// where the engine will put the file.
    pub instance_dir: &'a Path,
    /// The mod directory the game has loaded, `<workspace>/mods/BotBridge` --
    /// a symlink to the checkout in a debug workspace, a real directory in a
    /// release one. Whichever it is, it is what wrote the `storage` inside the
    /// save, which is why the save records its fingerprint.
    pub mod_dir: &'a Path,
    /// This run's directory, which the savepoint goes inside.
    pub run_dir: &'a Path,
    pub run_id: &'a str,
    pub milestone_index: u32,
    pub timeout: Duration,
}

/// Takes one savepoint: asks the engine to save, waits for the file to be
/// finished, and moves it into the run directory with its metadata beside it.
///
/// Returns the savepoint and how long the engine took, so a caller can record
/// both. Never panics and never leaves a half-move behind: the metadata is
/// written only after the `.zip` is in place, so a metadata file always
/// describes a save that exists (and [`list_savepoints`] skips one that does
/// not, for the case where somebody deletes the zip afterwards).
pub async fn capture(
    rcon: &crate::factorio::rcon::FactorioRcon,
    request: CaptureRequest<'_>,
) -> Result<(Savepoint, Duration), SavepointError> {
    let saves_dir = request.instance_dir.join("saves");
    let name = engine_save_name(request.run_id, request.milestone_index);
    let (staging, staging_tmp) = save_paths(&saves_dir, &name);
    // A previous attempt that crashed between asking and moving would leave a
    // file under exactly this name, and finding it instantly would report a
    // save that is one milestone old as this milestone's. The name is unique
    // to (run, milestone), so nothing else can be behind it.
    let _ = fs::remove_file(&staging);
    let _ = fs::remove_file(&staging_tmp);

    let started = Instant::now();
    let tick = rcon
        .savepoint(&name)
        .await
        // `{err}` and not `{err:?}`: a miette `Report`'s Debug is the whole
        // fancy multi-line diagnostic, and this string goes into a one-line
        // event in `events.jsonl`.
        .map_err(|err| SavepointError::Rcon(format!("{err}")))?
        .unwrap_or_default();
    let (written, bytes) = await_save(&saves_dir, &name, request.timeout).await?;
    let elapsed = started.elapsed();

    let savepoint = install(&written, bytes, tick, &request)?;
    Ok((savepoint, elapsed))
}

/// Moves a finished save out of the engine's saves directory and into the run
/// record, writing the metadata that identifies it.
///
/// Split from [`capture`] because everything above it needs a live game and
/// everything here does not: this half is where the file ends up under a name
/// a later `--resume-from` can find, and it is testable on its own.
///
/// The metadata is written **after** the move, so a metadata file always
/// describes a save that exists.
fn install(
    written: &Path,
    bytes: u64,
    tick: u64,
    request: &CaptureRequest<'_>,
) -> io::Result<Savepoint> {
    let destination = Savepoint::zip_path(request.run_dir, request.milestone_index);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    // A rename when the archive and the instance share a filesystem, which is
    // the ordinary case since both are under the workspace; a copy when they
    // do not. Either way the engine's saves directory does not accumulate.
    if fs::rename(written, &destination).is_err() {
        fs::copy(written, &destination)?;
        fs::remove_file(written)?;
    }

    let savepoint = Savepoint {
        schema: SAVEPOINT_SCHEMA,
        run_id: request.run_id.to_string(),
        milestone_index: request.milestone_index,
        tick,
        created_unix: now_unix(),
        bytes,
        file: format!("milestone-{}.zip", request.milestone_index),
        mods: ModFingerprint::of(request.mod_dir),
    };
    savepoint.write(request.run_dir)?;
    Ok(savepoint)
}

/// Marker written into the server instance directory naming the savepoint a
/// run was started from.
///
/// Read by `record.start()` when it fills in `provenance.resumed_from`. A
/// marker rather than a parameter threaded through the Lua API because
/// "which world did this server load" is a fact about the *instance*, known by
/// the code that started it and by nothing in between -- exactly like
/// `map-gen-seed.txt`, which is read the same way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeMarker {
    /// [`Savepoint::provenance_label`] -- what lands in `resumed_from`.
    pub label: String,
    /// Where the savepoint was read from, so a reader can go and look at it.
    pub source: String,
    /// The savepoint's own metadata, copied so a reaped run still leaves an
    /// explanation behind in the instance that resumed it.
    pub savepoint: Savepoint,
    /// What [`check_mods`] said at resume time, as a sentence. Recorded even
    /// when it was a mismatch that the operator overrode: a run that resumed
    /// across changed mod code must say so somewhere a reader will find it.
    pub mod_check: String,
    /// Unix seconds the marker was written.
    pub resumed_unix: u64,
}

/// Resolves a savepoint reference against a workspace and decides whether it
/// is safe to load, producing the marker the server start needs.
///
/// `force` overrides a mod mismatch. It does not hide one: the verdict is
/// written into the marker either way, so a run resumed across changed mod
/// code says so in a file a later reader will find, rather than only in a
/// terminal nobody kept.
pub fn plan_resume(
    workspace: &Path,
    reference: &str,
    force: bool,
) -> Result<ResumeMarker, SavepointError> {
    let found = resolve_savepoint(&workspace.join("runs"), reference)?;
    let current = ModFingerprint::of(&bridge_mod_dir(workspace));
    let check = check_mods(found.savepoint.mods.as_ref(), current.as_ref());
    if !check.is_safe() && !force {
        return Err(SavepointError::ModMismatch {
            explain: check.explain(),
        });
    }
    Ok(ResumeMarker {
        label: found.savepoint.provenance_label(),
        source: found.zip.to_string_lossy().into_owned(),
        mod_check: if check.is_safe() {
            check.explain()
        } else {
            format!("OVERRIDDEN with --resume-force: {}", check.explain())
        },
        savepoint: found.savepoint,
        resumed_unix: now_unix(),
    })
}

/// Writes the marker into `<workspace>/<instance>`.
pub fn write_resume_marker(instance_dir: &Path, marker: &ResumeMarker) -> io::Result<PathBuf> {
    fs::create_dir_all(instance_dir)?;
    let path = instance_dir.join(RESUME_MARKER);
    let json = serde_json::to_string_pretty(marker).map_err(io::Error::other)?;
    fs::write(&path, json.as_bytes())?;
    Ok(path)
}

/// Removes the marker, so a run that is **not** resuming cannot inherit the
/// previous run's answer.
///
/// Called unconditionally on every non-resuming server start. This is the
/// mistake `--seed` made -- a start-time fact that stayed true on disk after it
/// stopped being true of the run -- and it is cheaper to prevent than to detect.
pub fn clear_resume_marker(instance_dir: &Path) -> io::Result<()> {
    match fs::remove_file(instance_dir.join(RESUME_MARKER)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Reads the marker, or `None` when the instance was not resumed.
pub fn read_resume_marker(instance_dir: &Path) -> Option<ResumeMarker> {
    let bytes = fs::read(instance_dir.join(RESUME_MARKER)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Unix seconds now, or 0 if the clock is before the epoch.
pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fb-savepoint-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip shaped like a Factorio save: a top-level directory named after the
    /// save, holding the entries `save_is_complete` demands.
    fn write_fake_save(path: &Path, root: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        for entry in ["level.dat0", "level.dat1", "level-init.dat", "script.dat"] {
            zip.start_file(format!("{root}/{entry}"), options).unwrap();
            zip.write_all(b"not really a world").unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn a_finished_save_is_recognised_and_a_truncated_one_is_not() {
        let dir = scratch("complete");
        let good = dir.join("good.zip");
        write_fake_save(&good, "good");
        assert!(save_is_complete(&good).is_ok());

        // Chop the tail off, which is where a zip keeps the central directory
        // it is read through -- the same shape as the 741,376-byte
        // `level.tmp.zip` this workspace was found holding.
        let bytes = fs::read(&good).unwrap();
        let truncated = dir.join("truncated.zip");
        fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
        assert!(matches!(
            save_is_complete(&truncated),
            Err(NotYet::NotAZip(_))
        ));

        assert_eq!(
            save_is_complete(&dir.join("absent.zip")),
            Err(NotYet::Missing)
        );
    }

    /// A well-formed zip that is not a world must not pass. Without this the
    /// check would accept anything with a central directory, including a
    /// half-formed save whose data files had not been added yet.
    #[test]
    fn a_zip_that_is_not_a_save_is_refused() {
        let dir = scratch("notasave");
        let path = dir.join("empty.zip");
        let file = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();
        assert_eq!(save_is_complete(&path), Err(NotYet::NotASave));
    }

    #[tokio::test]
    async fn await_save_gives_up_and_says_what_it_was_waiting_for() {
        let dir = scratch("timeout");
        let err = await_save(&dir, "never", Duration::from_millis(300))
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("did not finish writing") && message.contains("does not exist"),
            "a timeout must name the file and the reason, got: {message}"
        );
        assert!(
            message.contains("writing somewhere else"),
            "nothing appearing at all is the attached-server case, and the message has to say \
             so rather than leaving a reader to guess: {message}"
        );
    }

    /// A server that is not writing here must be found out in seconds, not in
    /// [`SAVE_TIMEOUT`]. Without the appearance bound, every milestone of an
    /// `--connect` run would stall for three minutes.
    #[tokio::test]
    async fn a_save_that_never_starts_is_given_up_on_early() {
        let dir = scratch("appearance");
        let started = Instant::now();
        let err = await_save_bounded(
            &dir,
            "elsewhere",
            Duration::from_secs(600),
            Duration::from_millis(200),
        )
        .await
        .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "gave up after {:?}, which is the whole-save timeout, not the appearance one",
            started.elapsed()
        );
        assert!(err.to_string().contains(&dir.display().to_string()));
    }

    #[tokio::test]
    async fn await_save_returns_once_the_file_lands() {
        let dir = scratch("lands");
        let saves = dir.clone();
        let name = "savepoint-run-1-m2";
        let (final_path, _) = save_paths(&saves, name);
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            write_fake_save(&final_path, name);
        });
        let (path, bytes) = await_save(&saves, name, Duration::from_secs(5))
            .await
            .unwrap();
        writer.await.unwrap();
        assert!(path.ends_with("savepoint-run-1-m2.zip"));
        assert!(bytes > 0);
    }

    /// The `.tmp.zip` a killed server leaves behind must not block a later
    /// save that did land. The timeout message may mention it; the verdict may
    /// not depend on it.
    #[test]
    fn a_leftover_temp_file_does_not_hide_a_finished_save() {
        let dir = scratch("leftover");
        let name = "savepoint-run-9-m1";
        let (final_path, tmp_path) = save_paths(&dir, name);
        fs::write(&tmp_path, b"garbage from a killed server").unwrap();
        write_fake_save(&final_path, name);
        assert!(save_is_complete(&final_path).is_ok());
    }

    #[test]
    fn a_fingerprint_moves_when_the_mod_does() {
        let dir = scratch("fingerprint");
        let mod_dir = dir.join("BotBridge");
        fs::create_dir_all(mod_dir.join("prototypes")).unwrap();
        fs::write(mod_dir.join("info.json"), br#"{"version":"0.0.1"}"#).unwrap();
        fs::write(mod_dir.join("control.lua"), b"function on_init() end").unwrap();
        fs::write(mod_dir.join("prototypes/x.lua"), b"return {}").unwrap();

        let first = ModFingerprint::of(&mod_dir).unwrap();
        assert_eq!(first.version.as_deref(), Some("0.0.1"));
        assert_eq!(first.files, 3);
        assert_eq!(ModFingerprint::of(&mod_dir).unwrap(), first, "stable");

        // The case this exists for: `control.lua` changes and `info.json` does
        // not, so Factorio runs no migration and the old `storage` survives.
        fs::write(
            mod_dir.join("control.lua"),
            b"function on_init() end -- edited",
        )
        .unwrap();
        let second = ModFingerprint::of(&mod_dir).unwrap();
        assert_ne!(first.digest, second.digest);
        assert_eq!(first.version, second.version);

        match check_mods(Some(&first), Some(&second)) {
            ModCheck::Changed { version_bumped, .. } => assert!(
                !version_bumped,
                "an unchanged info.json means no migration ran"
            ),
            other => panic!("expected a mismatch, got {other:?}"),
        }
        assert!(!check_mods(Some(&first), Some(&second)).is_safe());
        assert!(check_mods(Some(&first), Some(&first)).is_safe());
    }

    /// Absent evidence is not a match. A savepoint written before fingerprints
    /// existed, or an unreadable mods directory, must both stop a resume from
    /// claiming the mod is unchanged.
    #[test]
    fn a_missing_fingerprint_is_never_a_match() {
        let fp = ModFingerprint {
            version: Some("0.0.1".into()),
            digest: "deadbeef".into(),
            files: 3,
        };
        assert!(!check_mods(None, Some(&fp)).is_safe());
        assert!(!check_mods(Some(&fp), None).is_safe());
        assert!(matches!(check_mods(None, None), ModCheck::Unknown { .. }));
    }

    #[test]
    fn savepoints_are_found_by_run_and_milestone_and_default_to_the_last() {
        let root = scratch("list");
        for (run, milestones) in [("run-100-1", vec![1u32, 2, 5]), ("run-200-2", vec![1])] {
            let run_dir = root.join(run);
            for m in milestones {
                let zip = Savepoint::zip_path(&run_dir, m);
                write_fake_save(&zip, "level");
                let savepoint = Savepoint {
                    schema: SAVEPOINT_SCHEMA,
                    run_id: run.to_string(),
                    milestone_index: m,
                    tick: u64::from(m) * 1000,
                    created_unix: 17,
                    bytes: fs::metadata(&zip).unwrap().len(),
                    file: format!("milestone-{m}.zip"),
                    mods: None,
                };
                savepoint.write(&run_dir).unwrap();
            }
        }
        assert_eq!(list_savepoints(&root).len(), 4);

        let latest = resolve_savepoint(&root, "run-100-1").unwrap();
        assert_eq!(
            latest.savepoint.milestone_index, 5,
            "a run named without a milestone resolves to the furthest it got"
        );
        assert_eq!(latest.savepoint.provenance_label(), "run-100-1/milestone-5");

        let exact = resolve_savepoint(&root, "run-100-1:2").unwrap();
        assert_eq!(exact.savepoint.milestone_index, 2);
        assert!(exact.provenance_path().ends_with("provenance.json"));

        assert!(matches!(
            resolve_savepoint(&root, "run-100-1:9"),
            Err(SavepointError::NotFound { .. })
        ));
        assert!(matches!(
            resolve_savepoint(&root, "run-does-not-exist"),
            Err(SavepointError::NotFound { .. })
        ));
    }

    /// A savepoint whose `.zip` is gone -- deleted by hand, or a metadata file
    /// written for a save that never landed -- must not be offered as
    /// resumable.
    #[test]
    fn metadata_without_a_save_file_is_not_listed() {
        let root = scratch("orphan");
        let run_dir = root.join("run-1-1");
        Savepoint {
            schema: SAVEPOINT_SCHEMA,
            run_id: "run-1-1".into(),
            milestone_index: 1,
            tick: 5,
            created_unix: 6,
            bytes: 7,
            file: "milestone-1.zip".into(),
            mods: None,
        }
        .write(&run_dir)
        .unwrap();
        assert!(list_savepoints(&root).is_empty());
    }

    /// The location decision, pinned: savepoints live inside the run
    /// directory, so the archive reaper is the only retention policy there is.
    ///
    /// A second policy would be a second source of truth about what may be
    /// deleted, and the two would eventually disagree -- the failure mode
    /// being a savepoint whose run record is gone, which is a save nobody can
    /// identify. `.keep` protects a run's savepoints along with the rest of it.
    #[test]
    fn reaping_a_run_takes_its_savepoints_and_keep_protects_them() {
        let root = scratch("reap");
        for (id, protected) in [("run-100-1", false), ("run-200-2", true)] {
            let run_dir = root.join(id);
            fs::create_dir_all(&run_dir).unwrap();
            // The reaper's own safety gate: no event log, not a run.
            fs::write(run_dir.join("events.jsonl"), b"{}\n").unwrap();
            write_fake_save(&Savepoint::zip_path(&run_dir, 1), "level");
            Savepoint {
                schema: SAVEPOINT_SCHEMA,
                run_id: id.to_string(),
                milestone_index: 1,
                tick: 1,
                created_unix: 1,
                bytes: 1,
                file: "milestone-1.zip".into(),
                mods: None,
            }
            .write(&run_dir)
            .unwrap();
            if protected {
                fs::write(run_dir.join(super::super::retention::KEEP_MARKER), b"").unwrap();
            }
        }
        let reaped = super::super::retention::reap(&root, 0).unwrap();
        assert_eq!(reaped.deleted, vec!["run-100-1"]);
        assert_eq!(reaped.protected, vec!["run-200-2"]);
        assert!(
            list_savepoints(&root)
                .iter()
                .map(|found| found.savepoint.run_id.as_str())
                .eq(["run-200-2"]),
            "the reaped run's savepoint went with its record, and the kept one stayed"
        );
    }

    /// The whole round trip either side of the one step that needs a live
    /// game: a finished save in the engine's directory becomes a savepoint in
    /// the run record, which `--resume-from` then resolves back to a file.
    ///
    /// This is the seam the feature actually rests on. `capture` above it
    /// differs only by the RCON call that produced the file.
    #[tokio::test]
    async fn a_finished_save_becomes_a_resumable_savepoint() {
        let workspace = scratch("install");
        let instance = workspace.join("server");
        let saves = instance.join("saves");
        let run_dir = workspace.join("runs").join("run-1788465258-49050");
        let mod_dir = bridge_mod_dir(&workspace);
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("info.json"), br#"{"version":"0.0.1"}"#).unwrap();
        fs::write(mod_dir.join("control.lua"), b"-- the mod that ran").unwrap();

        // What the engine leaves behind, under the name it was asked for.
        let name = engine_save_name("run-1788465258-49050", 3);
        let (staged, _) = save_paths(&saves, &name);
        write_fake_save(&staged, &name);
        let bytes = save_is_complete(&staged).unwrap();

        let request = CaptureRequest {
            instance_dir: &instance,
            mod_dir: &mod_dir,
            run_dir: &run_dir,
            run_id: "run-1788465258-49050",
            milestone_index: 3,
            timeout: SAVE_TIMEOUT,
        };
        let savepoint = install(&staged, bytes, 70_616, &request).unwrap();

        assert!(
            !staged.exists(),
            "the engine's saves directory must not accumulate a copy of every milestone"
        );
        assert_eq!(savepoint.file, "milestone-3.zip");
        assert!(
            savepoint.mods.is_some(),
            "the mod that wrote it is recorded"
        );
        assert!(
            save_is_complete(&Savepoint::zip_path(&run_dir, 3)).is_ok(),
            "and what landed in the run record is still a readable save"
        );

        // The other end: a later run naming this savepoint gets the file back,
        // together with the verdict on whether its mod code still matches.
        let marker = plan_resume(&workspace, "run-1788465258-49050:3", false).unwrap();
        assert_eq!(marker.label, "run-1788465258-49050/milestone-3");
        assert_eq!(marker.savepoint.tick, 70_616);
        assert!(Path::new(&marker.source).is_file());
        assert!(marker.mod_check.contains("this exact mod code"));
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_misread() {
        let dir = scratch("schema");
        let meta = dir.join("milestone-1.json");
        fs::write(
            &meta,
            br#"{"schema":99,"run_id":"r","milestone_index":1,"tick":1,"created_unix":1,"bytes":1,"file":"milestone-1.zip","mods":null}"#,
        )
        .unwrap();
        assert!(matches!(
            Savepoint::read(&meta),
            Err(SavepointError::FutureSchema { found: 99 })
        ));
    }

    /// The hazard this whole module is shaped around: a save written by one
    /// `control.lua` and loaded by another, with `info.json` unchanged so
    /// Factorio runs no migration and the old `storage` survives intact.
    #[test]
    fn resuming_across_changed_mod_code_is_refused_and_the_override_is_recorded() {
        let workspace = scratch("plan-resume");
        let mod_dir = bridge_mod_dir(&workspace);
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("info.json"), br#"{"version":"0.0.1"}"#).unwrap();
        fs::write(mod_dir.join("control.lua"), b"-- as it was").unwrap();
        let written_under = ModFingerprint::of(&mod_dir).unwrap();

        let run_dir = workspace.join("runs").join("run-7-7");
        write_fake_save(&Savepoint::zip_path(&run_dir, 2), "level");
        Savepoint {
            schema: SAVEPOINT_SCHEMA,
            run_id: "run-7-7".into(),
            milestone_index: 2,
            tick: 70_616,
            created_unix: 1,
            bytes: 10,
            file: "milestone-2.zip".into(),
            mods: Some(written_under),
        }
        .write(&run_dir)
        .unwrap();

        let marker = plan_resume(&workspace, "run-7-7:2", false).unwrap();
        assert_eq!(marker.label, "run-7-7/milestone-2");
        assert!(marker.mod_check.contains("this exact mod code"));

        fs::write(mod_dir.join("control.lua"), b"-- edited since").unwrap();
        let refused = plan_resume(&workspace, "run-7-7:2", false).unwrap_err();
        assert!(matches!(refused, SavepointError::ModMismatch { .. }));
        assert!(
            refused.to_string().contains("--resume-force"),
            "a refusal has to name the way past it: {refused}"
        );

        let forced = plan_resume(&workspace, "run-7-7:2", true).unwrap();
        assert!(
            forced.mod_check.starts_with("OVERRIDDEN"),
            "an override must survive into the record, not just the terminal: {}",
            forced.mod_check
        );
    }

    #[test]
    fn the_resume_marker_is_written_read_and_cleared() {
        let instance = scratch("marker");
        assert!(read_resume_marker(&instance).is_none());
        let marker = ResumeMarker {
            label: "run-1-1/milestone-3".into(),
            source: "/runs/run-1-1/savepoints/milestone-3.zip".into(),
            savepoint: Savepoint {
                schema: SAVEPOINT_SCHEMA,
                run_id: "run-1-1".into(),
                milestone_index: 3,
                tick: 70_616,
                created_unix: 1,
                bytes: 1_458_383,
                file: "milestone-3.zip".into(),
                mods: None,
            },
            mod_check: "the savepoint was written by this exact mod code".into(),
            resumed_unix: 2,
        };
        write_resume_marker(&instance, &marker).unwrap();
        assert_eq!(read_resume_marker(&instance).unwrap(), marker);
        clear_resume_marker(&instance).unwrap();
        assert!(
            read_resume_marker(&instance).is_none(),
            "a fresh run must not inherit the previous run's resume marker"
        );
        clear_resume_marker(&instance).unwrap();
    }

    #[test]
    fn an_engine_save_name_cannot_leave_the_saves_directory() {
        let name = engine_save_name("../../etc/pass wd", 2);
        assert_eq!(name, "savepoint-------etc-pass-wd-m2");
        assert!(!name.contains('/') && !name.contains('.'));
        assert_eq!(
            engine_save_name("run-1788465258-49050", 3),
            "savepoint-run-1788465258-49050-m3"
        );
    }
}
