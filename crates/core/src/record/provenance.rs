//! What a run *was*, written the moment it starts.
//!
//! # Why this is not in the manifest
//!
//! `manifest.json` is written in [`super::RunRecorder::finish`] and nowhere
//! else, so it exists only for runs that finished. Of the 24 runs archived when
//! this module was written, **15 had a manifest and 9 had none** -- and the nine
//! are exactly the runs that were killed or died, which are the runs whose
//! identity somebody most needs later. A provenance record that appears only
//! for successful runs cannot answer "were these two comparable?" about any
//! failure, and that is precisely the question that was got wrong repeatedly on
//! 2026-09-03.
//!
//! So provenance is a separate file, `provenance.json`, written once by
//! [`super::RunRecorder::record_provenance`] immediately after the run
//! directory exists and before anything can fail. It is never rewritten. A
//! single file updated at finish would reintroduce the same hole -- a process
//! killed during the rewrite loses both halves -- and there is nothing here
//! that a later moment knows better, because every field describes what was
//! *launched*.
//!
//! The division of labour is therefore:
//!
//! * `provenance.json` -- what was launched. Written at start, immutable.
//! * `manifest.json` -- what happened. Written at finish, derived from the log.
//!
//! # What cannot be recovered
//!
//! Nothing here can be backfilled onto the runs that already exist. No archived
//! run recorded its seed, and `mods/BotBridge/control.lua` never read
//! `map_gen_settings`, so the maps those runs used are permanently
//! unidentifiable. A reader comparing two pre-provenance runs is entitled to
//! know that it is comparing two unknowns, and [`Provenance`]'s absence is how
//! it finds out.

use super::map::MapKind;
use crate::graph::entity_graph::ResourceFingerprint;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// The name of the file this module writes, inside the run directory.
pub const PROVENANCE_FILE: &str = "provenance.json";

/// The only value [`GitProvenance::source`] takes today. Named so a reader can
/// match on it rather than on a spelling.
pub const SOURCE_WORKING_TREE: &str = "working-tree-at-run-start";

/// The immutable facts about what a run was launched with.
///
/// Every field is `Option` except the identity, and each `None` is a real
/// answer: "nothing observed this". They are present-and-null rather than
/// omitted for the reason [`super::EventKind::RunStarted`]'s `seed` is -- a key
/// that is always there says "we looked", where a missing key cannot be told
/// apart from "we never asked".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Bumped when a field changes meaning, never when one is added. Readers
    /// must accept any schema at or below their own, the same rule
    /// [`super::samples`] follows, so that archived runs stay readable.
    pub schema: u32,
    pub run_id: String,
    /// Unix seconds at which the run directory was created. For "when was
    /// this", never for comparing two runs -- runs are compared on ticks.
    pub started_unix: u64,
    /// The game tick the run opened at.
    pub started_tick: u64,
    /// The map-generation seed, when this workspace's `level.zip` was created
    /// by a build that recorded one.
    ///
    /// `None` for every map generated before
    /// [`crate::process::instance_setup::record_map_gen_seed`] existed, and for
    /// every map Factorio seeded itself. Both mean "unknown", and neither may
    /// be reported as a match against anything.
    pub seed: Option<String>,
    /// The map-exchange string for the map this run played -- a **seed plus the
    /// map-gen settings**, which is what actually identifies a map. The same
    /// seed under different settings, or on a Factorio version whose defaults
    /// moved, is a different map, so the seed two fields up is necessary and
    /// not sufficient.
    ///
    /// Asked of the running game ([`crate::factorio::rcon::FactorioRcon::
    /// map_exchange_string`]) and falling back to `map-exchange-string.txt`,
    /// which setup writes only when a string was *supplied* to it. Until
    /// 2026-09-06 the file was the only source and nothing produced a string
    /// from a live map, so all 20 archived runs carry `null` here and their
    /// maps are permanently unidentifiable.
    ///
    /// **`None` means "not captured", never "default settings".** An RCON
    /// failure, an old mod without the function, and a server this process did
    /// not start all land here, and none of them is evidence about the map --
    /// the same asymmetry [`ResourceFingerprint`] documents.
    pub map_exchange_string: Option<String>,
    /// An identity for the map computed from the world itself, which is the
    /// only identity available for a map whose seed was never recorded.
    ///
    /// See [`ResourceFingerprint`] for what a match and a mismatch each mean --
    /// they are not symmetrical.
    pub map: Option<ResourceFingerprint>,
    /// The installed game's version, e.g. `"2.1.17"`.
    pub factorio: Option<String>,
    /// Where the code came from. See [`GitProvenance`].
    pub git: Option<GitProvenance>,
    /// `"debug"` or `"release"`. Worth recording because the two builds resolve
    /// mods and scripts from different places (see `CLAUDE.md`), so the same
    /// commit can run different Lua in each.
    pub profile: String,
    /// The roster the run was launched for, ascending. What was *asked for*; a
    /// run that degrades to fewer bots because clients never connected still
    /// records the request here, and `run_started.bots` records what it got.
    /// The pair is the evidence that a run degraded.
    pub roster_requested: Vec<u32>,
    /// The workspace the run used, absolute. Two runs from different
    /// workspaces used different `level.zip` files and different mod copies.
    pub workspace: Option<String>,
    /// The save, if any, this run was resumed from rather than starting on a
    /// fresh world.
    ///
    /// Always `None` today: nothing saves at a milestone yet. The field is here
    /// ahead of that feature on purpose, because it is a *start-time* fact and
    /// adding it later would mean a schema bump on a file whose whole value is
    /// that archived runs stay readable.
    ///
    /// **A resumed run is not benchmark-comparable with a fresh one.** It
    /// begins with accumulated world state -- built furnaces, charged chests,
    /// partly mined patches -- so its timings measure a different thing. A
    /// comparison tool must treat `Some(_)` against `None` the way it treats
    /// two different seeds, and for the same reason.
    #[serde(default)]
    pub resumed_from: Option<String>,
    /// `"clients"` or `"characters"`: whether the bots were graphical clients
    /// or server-side character entities (`--headless`). Play fidelity is
    /// the same, so a comparison across the two is flagged, not refused. `None`
    /// for runs older than the field and for attached servers.
    #[serde(default)]
    pub bot_mode: Option<String>,
    /// `game.speed` as set at start. Timings are in ticks and do not change
    /// with it; whether the server *kept up* with it is what a reader wants
    /// to know when a run at speed 10 looks slower per tick than one at 1.
    #[serde(default)]
    pub game_speed: Option<f64>,
}

/// Which commit the code came from, and whether it had been edited.
///
/// # Read at run start, from the working tree
///
/// Deliberately **not** stamped into the binary by `build.rs`. A build script
/// that shells out to `git` makes every build depend on git state and on git
/// being installed; `app/src-tauri/build.rs` does not do this today and this
/// module is not a reason to start.
///
/// The honest cost of reading it at run start is stated in [`Self::source`]:
/// this is the working tree *now*, which is the commit the binary was built
/// from only if nobody has moved since. `dirty` is the part that makes it
/// useful anyway -- a dirty tree means the commit hash alone does not describe
/// the code, and a comparison across two dirty runs is not a comparison across
/// two commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitProvenance {
    /// Full 40-character commit hash of `HEAD`.
    pub commit: String,
    /// Whether the working tree had uncommitted changes to tracked files.
    /// `true` makes `commit` an approximation rather than an identity.
    pub dirty: bool,
    /// How this was obtained, so a reader is never misled about its strength.
    /// Always `"working-tree-at-run-start"` today.
    pub source: String,
}

impl Provenance {
    /// The schema version this build writes.
    pub const SCHEMA: u32 = 1;
}

/// Picks the map-exchange string to record, from what the live game answered
/// and what setup left on disk.
///
/// The live answer wins: it describes the map that was actually loaded, while
/// the file records only what setup was *handed* -- and setup writes no file at
/// all when no string was supplied, which is every run in the archive.
///
/// Every empty or whitespace-only candidate is dropped rather than stored.
/// `None` here has one meaning, **"not captured"**, and an empty string would
/// be a second one that reads as a value. That distinction is the same one
/// [`ResourceFingerprint`] keeps: an answer and the absence of an answer are
/// not interchangeable.
pub fn choose_map_exchange_string(
    from_game: Option<String>,
    from_setup_file: Option<String>,
) -> Option<String> {
    fn usable(value: Option<String>) -> Option<String> {
        value
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    usable(from_game).or_else(|| usable(from_setup_file))
}

/// Reads `HEAD` and the dirty flag from the working tree the process is running
/// in.
///
/// Every failure collapses to `None`: no git, no repository, a detached or
/// broken checkout, or a `git` that takes too long are all "we do not know",
/// and a run must not fail because its provenance could not be established.
///
/// `git status --porcelain` is limited to tracked files (`--untracked-files=no`)
/// on purpose. An untracked scratch file, a `target/` directory or an editor
/// swap file says nothing about the code that ran, and counting them would
/// report every real checkout as dirty -- a flag that is always true carries no
/// information and would be ignored within a day.
pub fn git_provenance(repo: &Path) -> Option<GitProvenance> {
    let rev = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !rev.status.success() {
        return None;
    }
    let commit = String::from_utf8(rev.stdout).ok()?.trim().to_string();
    if commit.is_empty() {
        return None;
    }
    let status = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()?;
    // A `git status` that fails leaves the dirty flag unknown, and the
    // conservative reading of unknown here is `true`: claiming a clean tree we
    // did not verify would assert the commit describes the code exactly, which
    // is the stronger claim of the two.
    let dirty = !status.status.success() || !status.stdout.is_empty();
    Some(GitProvenance {
        commit,
        dirty,
        source: SOURCE_WORKING_TREE.to_string(),
    })
}

/// Reads a run's provenance back, or `None` for a run recorded before this
/// existed.
///
/// The absence is meaningful and callers must preserve it: a run with no
/// provenance file is a run whose comparability is **unknown**, which is a
/// different answer from "comparable" and from "not comparable".
pub fn read_provenance(dir: &Path) -> Option<Provenance> {
    let bytes = std::fs::read(dir.join(PROVENANCE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Counts the entities in a keyframe by name, for a quick "what was on this
/// map" line that does not need the whole map record parsed twice.
///
/// Lives here rather than in [`super::map`] because it exists for the
/// comparison question, not for drawing: two runs whose opening keyframes hold
/// different resources were not on the same map, whatever else agrees.
pub fn keyframe_entity_counts(kind: &MapKind) -> Option<BTreeMap<String, usize>> {
    let MapKind::Keyframe { game, .. } = kind else {
        return None;
    };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entity in game {
        *counts.entry(entity.name.clone()).or_default() += 1;
    }
    Some(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::map::{Bounds, EntitySnapshot};
    use crate::types::Position;

    #[test]
    fn git_provenance_reads_this_very_checkout() {
        // Runs against the repository the test itself lives in, which is the
        // same shape as the real call: a process started inside a checkout.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(git) = git_provenance(repo) else {
            // A source tarball with no `.git` is a legitimate way to build this
            // crate, and the function is specified to answer `None` there.
            return;
        };
        assert_eq!(git.commit.len(), 40, "a full hash, not an abbreviation");
        assert!(git.commit.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(git.source, SOURCE_WORKING_TREE);
    }

    #[test]
    fn git_provenance_of_a_directory_that_is_not_a_repository_is_none() {
        // Not an error and not a panic: a run launched outside a checkout still
        // has to start.
        let dir = std::env::temp_dir().join(format!("fb-prov-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // `/tmp` is not inside this repository, so `git -C` finds no repo --
        // unless the temp dir happens to be, which no CI layout does.
        assert!(git_provenance(&dir).is_none() || cfg!(windows));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_live_map_exchange_string_beats_the_file_setup_was_handed() {
        // The file says what setup was *asked* for; the game says what it
        // actually loaded. When they disagree the game is right.
        assert_eq!(
            choose_map_exchange_string(Some(">>>live<<<".into()), Some(">>>file<<<".into())),
            Some(">>>live<<<".to_string())
        );
        assert_eq!(
            choose_map_exchange_string(None, Some(">>>file<<<".into())),
            Some(">>>file<<<".to_string())
        );
        assert_eq!(
            choose_map_exchange_string(Some(">>>live<<<".into()), None),
            Some(">>>live<<<".to_string())
        );
    }

    #[test]
    fn a_failure_to_capture_is_none_and_never_an_empty_string() {
        // Both sources absent -- an RCON failure and no setup file, which is
        // every archived run.
        assert_eq!(choose_map_exchange_string(None, None), None);
        // An empty or blank answer is not a value. If it were stored, a reader
        // could not tell "this map has no settings" (meaningless) from "nobody
        // asked", and the whole point of the field is that it can.
        assert_eq!(choose_map_exchange_string(Some(String::new()), None), None);
        assert_eq!(choose_map_exchange_string(Some("   \n".into()), None), None);
        assert_eq!(choose_map_exchange_string(None, Some(String::new())), None);
        // A blank live answer must not shadow a real file answer either.
        assert_eq!(
            choose_map_exchange_string(Some("  ".into()), Some(">>>file<<<".into())),
            Some(">>>file<<<".to_string())
        );
    }

    #[test]
    fn provenance_round_trips_and_tolerates_a_newer_reader() {
        let dir = std::env::temp_dir().join(format!("fb-prov-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let provenance = Provenance {
            schema: Provenance::SCHEMA,
            run_id: "run-1-2".into(),
            started_unix: 1_788_000_000,
            started_tick: 4330,
            seed: Some("12345".into()),
            map_exchange_string: None,
            map: None,
            factorio: Some("2.1.17".into()),
            git: Some(GitProvenance {
                commit: "0".repeat(40),
                dirty: true,
                source: SOURCE_WORKING_TREE.to_string(),
            }),
            profile: "debug".to_string(),
            roster_requested: vec![1, 2, 3, 4],
            workspace: Some("/tmp/ws".into()),
            resumed_from: None,
            bot_mode: Some("characters".into()),
            game_speed: Some(5.0),
        };
        std::fs::write(
            dir.join(PROVENANCE_FILE),
            serde_json::to_vec_pretty(&provenance).unwrap(),
        )
        .unwrap();
        assert_eq!(read_provenance(&dir).as_ref(), Some(&provenance));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_without_a_provenance_file_reads_as_unknown_not_as_a_default() {
        // The nine archived runs with no manifest are also the nine with no
        // provenance, and this is the call that must not invent one for them.
        let dir = std::env::temp_dir().join(format!("fb-prov-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_provenance(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keyframe_entity_counts_group_by_name() {
        let kind = MapKind::Keyframe {
            bounds: Bounds {
                left: 0.,
                top: 0.,
                right: 1.,
                bottom: 1.,
            },
            game: vec![
                EntitySnapshot {
                    name: "iron-ore".into(),
                    position: Position { x: 0., y: 0. },
                    direction: 0,
                },
                EntitySnapshot {
                    name: "iron-ore".into(),
                    position: Position { x: 1., y: 0. },
                    direction: 0,
                },
                EntitySnapshot {
                    name: "stone-furnace".into(),
                    position: Position { x: 2., y: 0. },
                    direction: 0,
                },
            ],
            model: vec![],
            divergence: vec![],
        };
        let counts = keyframe_entity_counts(&kind).expect("a keyframe counts");
        assert_eq!(counts.get("iron-ore"), Some(&2));
        assert_eq!(counts.get("stone-furnace"), Some(&1));
    }

    #[test]
    fn a_placement_row_is_not_a_keyframe_and_counts_nothing() {
        assert_eq!(keyframe_entity_counts(&MapKind::Unknown), None);
    }
}

#[cfg(test)]
mod run_mode_fields {
    use super::*;

    #[test]
    fn an_archived_provenance_without_the_run_mode_fields_still_reads() {
        let json = r#"{"schema":1,"run_id":"run-1","started_unix":0,"started_tick":0,
            "seed":null,"map_exchange_string":null,"map":null,"factorio":null,"git":null,
            "profile":"debug","roster_requested":[1],"workspace":null}"#;
        let p: Provenance = serde_json::from_str(json).unwrap();
        assert_eq!(p.bot_mode, None);
        assert_eq!(p.game_speed, None);
        assert_eq!(p.resumed_from, None);
    }
}
