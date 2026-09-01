//! Periodic world state captured by the mod, keyed on `game.tick`.
//!
//! Written by BotBridge into the *server's* `script-output` and ingested here.
//! This is a sibling of `events.jsonl`, not a replacement: a reader built
//! before this file existed opens a run unchanged, because it never asks for
//! it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use crate::types::Position;

/// The sample shape this build understands.
///
/// Refusing an unknown value is the point. In a debug build `workspace/mods`
/// wins over the repo checkout and there is no refresh path, and `info.json`
/// has read `0.0.1` since the project began -- so a version string cannot tell
/// a stale mod from a current one. This integer can, because it is bumped
/// deliberately whenever a field changes.
pub const SAMPLE_SCHEMA: u32 = 1;

/// One line of `samples.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Sample {
    /// The schema this line was written under. [`read_samples`] has already
    /// checked this against [`SAMPLE_SCHEMA`] via `SchemaProbe` by the time a
    /// line reaches this struct, but the field must still be a *real* member
    /// here -- not just probed and discarded -- or re-serialising an archived
    /// sample drops the stamp. A file this product produces would then be
    /// unable to trip its own schema guard on a later read: a future
    /// `SAMPLE_SCHEMA` bump would silently lose data on an old archive instead
    /// of failing loudly.
    pub schema: u32,
    pub tick: u64,
    /// The run id the mod stamped on this line, or `None` for a line written
    /// before this field existed. Absence must be distinguishable from a
    /// mismatch: [`ingest_samples`] treats a named-but-different run as a
    /// confident exclusion and an absent run as a tick guess, and those are
    /// not the same confidence level.
    ///
    /// `#[serde(default)]`, deliberately unlike the "present-and-null, never
    /// absent" fields elsewhere in this module: those describe values a
    /// *current* producer always has an answer for (even if the answer is
    /// "nothing"), where here the missing-key case is a *past* producer that
    /// never knew to ask. A key present with a `null` value and a key never
    /// written must both mean "we don't know", so both must decode to `None`.
    #[serde(default)]
    pub run: Option<String>,
    #[serde(flatten)]
    pub kind: SampleKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SampleKind {
    Bots {
        bots: Vec<BotSample>,
    },
    Force {
        /// Null when nothing is queued -- present and null, so "we looked and
        /// nothing was researching" is distinguishable from "we never asked".
        research: Option<ResearchSample>,
        techs_unlocked: u32,
        production: ProductionSample,
        power: PowerSample,
    },
    /// A kind this build does not know. Readers skip it; writers never emit it.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BotSample {
    pub id: u32,
    /// As the game reports it. A tile centre stays `-40.5`; nothing rounds.
    pub position: Position,
    /// Item name to count. Built mod-side from Factorio 2.0's
    /// `get_contents()`, which returns an array of `{name, count, quality}`.
    pub inventory: BTreeMap<String, u32>,
    /// Queue *length*, not its contents: the contents are large, change every
    /// tick, and answer no question we have.
    pub crafting_queue: u32,
    /// Name of whatever the bot's character is mining, or `None` when it
    /// isn't. Resolved mod-side from `LuaControl.mining_state.position` via
    /// `LuaSurface.find_entities_filtered` -- *not* from
    /// `LuaEntity.mining_target`, which belongs to mining drills, not
    /// characters, and raises when read off one (see `sample_bots` in
    /// `mods/BotBridge/control.lua`).
    pub mining: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResearchSample {
    pub name: String,
    /// 0.0 to 1.0.
    pub progress: f64,
    /// Always `null` today: `mods/BotBridge/control.lua`'s `sample_force`
    /// hard-codes `eta_ticks = nil` because no writer computes an estimate
    /// yet. So a `null` here means "not computed", not "the game reported
    /// none" -- there is no code path, mod-side or Rust-side, that has ever
    /// asked Factorio for this and gotten a real answer.
    pub eta_ticks: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProductionSample {
    /// Cumulative from game start, never per-interval: deltas are derivable
    /// from totals, and totals are unrecoverable from deltas once one sample
    /// is lost.
    pub made: BTreeMap<String, u64>,
    pub consumed: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PowerSample {
    pub generated_kw: f64,
    pub consumed_kw: f64,
    /// Consumed over demanded. Coverage is not capacity: an under-supplied
    /// network reads as dead rather than slow, so this is the field that makes
    /// that visible after the fact.
    pub satisfaction: f64,
}

/// What [`read_samples`] found.
#[derive(Debug)]
pub struct ReadSamples {
    pub samples: Vec<Sample>,
    /// Lines that did not parse -- in practice the truncated final line of a
    /// crashed run. Returned rather than swallowed.
    pub skipped: usize,
}

#[derive(Deserialize)]
struct SchemaProbe {
    schema: u32,
}

/// Reads a sample log, tolerating a truncated tail but never an unknown schema.
pub fn read_samples(path: &Path) -> io::Result<ReadSamples> {
    let mut samples = Vec::new();
    let mut skipped = 0usize;
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // Schema first. A wrong-shaped line must fail loudly rather than land
        // in `skipped`, where it would look like ordinary truncation.
        if let Ok(probe) = serde_json::from_str::<SchemaProbe>(&line)
            && probe.schema != SAMPLE_SCHEMA
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "sample schema {} is not the {} this build understands \
                     -- workspace/mods is probably stale",
                    probe.schema, SAMPLE_SCHEMA
                ),
            ));
        }
        match serde_json::from_str::<Sample>(&line) {
            Ok(sample) => samples.push(sample),
            Err(_) => skipped += 1,
        }
    }
    Ok(ReadSamples { samples, skipped })
}

/// Copies the run's samples out of the server's `script-output`.
///
/// Filtered on the run id *first*, tick second -- not on tick alone. Unlike
/// frames, sample lines carry no per-run sidecar file to gate a whole
/// directory; the mod stamps `run` on each line instead, and a line naming a
/// *different* run is excluded regardless of its tick, because a named
/// mismatch is a known fact and a tick comparison is only ever a guess. Every
/// run starts near tick 0 on a freshly generated map, so two runs' tick
/// ranges overlap almost entirely (see `frames.rs`, which hit this same
/// problem and solved it with a sidecar) -- two runs can both pass through
/// tick 61,500, and a leftover `samples.jsonl` from the older one would
/// otherwise be ingested as this run's data.
///
/// A line with no `run` at all -- written before this field existed -- falls
/// back to `not_before`, the recorder's high-water mark, because that is the
/// only signal such a line can carry.
pub fn ingest_samples(
    workspace: &Path,
    run_dir: &Path,
    run_id: &str,
    not_before: u64,
) -> io::Result<usize> {
    let source = workspace
        .join("server")
        .join("script-output")
        .join("botbridge")
        .join("samples.jsonl");
    if !source.exists() {
        return Ok(0);
    }
    let read = read_samples(&source)?;
    if read.skipped > 0 {
        // Counted rather than swallowed in `read_samples`, and it must not be
        // thrown away again here. Concrete case: the mod writes `bots = {}`
        // when no player is connected, and `helpers.table_to_json({})` yields
        // `"{}"` rather than `"[]"`, so that line fails to deserialise as a
        // `Sample` and would otherwise vanish with nothing to show for it.
        tracing::warn!(
            skipped = read.skipped,
            source = %source.display(),
            "some sample lines did not parse and were skipped"
        );
    }
    let mut out = File::create(run_dir.join("samples.jsonl"))?;
    let mut count = 0usize;
    for sample in read.samples.iter().filter(|s| match &s.run {
        Some(run) => run == run_id,
        None => s.tick >= not_before,
    }) {
        writeln!(out, "{}", serde_json::to_string(sample)?)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &Path, lines: &[&str]) -> std::path::PathBuf {
        let path = dir.join("samples.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
        path
    }

    #[test]
    fn reads_a_bot_sample() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"bots","schema":1,"tick":61500,"run":"r1","bots":[{"id":1,"position":{"x":-40.5,"y":-48.5},"inventory":{"iron-ore":23},"crafting_queue":0,"mining":"iron-ore"}]}"#,
            ],
        );
        let read = read_samples(&path).unwrap();
        assert_eq!(read.samples.len(), 1);
        assert_eq!(read.samples[0].tick, 61500);
        let SampleKind::Bots { bots } = &read.samples[0].kind else {
            panic!("expected a bots sample");
        };
        // The position is a tile centre and survives unrounded.
        assert_eq!(bots[0].position.x, -40.5);
        assert_eq!(bots[0].inventory["iron-ore"], 23);
        assert_eq!(bots[0].mining.as_deref(), Some("iron-ore"));
    }

    #[test]
    fn reads_a_force_sample_with_no_research_queued() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"force","schema":1,"tick":61500,"run":"r1","research":null,"techs_unlocked":7,"production":{"made":{"iron-plate":120},"consumed":{}},"power":{"generated_kw":180.0,"consumed_kw":150.0,"satisfaction":1.0}}"#,
            ],
        );
        let read = read_samples(&path).unwrap();
        let SampleKind::Force {
            research,
            techs_unlocked,
            production,
            power,
        } = &read.samples[0].kind
        else {
            panic!("expected a force sample");
        };
        // Present-and-null: we looked, and nothing was researching.
        assert!(research.is_none());
        assert_eq!(*techs_unlocked, 7);
        assert_eq!(production.made["iron-plate"], 120);
        assert!(production.consumed.is_empty());
        assert_eq!(power.satisfaction, 1.0);
    }

    #[test]
    fn refuses_an_unknown_schema_and_names_it() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[r#"{"kind":"bots","schema":99,"tick":61500,"bots":[]}"#],
        );
        let err = read_samples(&path).unwrap_err();
        // A stale workspace/mods must fail loudly, naming what it found.
        assert!(
            err.to_string().contains("99"),
            "error must name the schema: {err}"
        );
    }

    #[test]
    fn skips_a_truncated_final_line_and_counts_it() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"bots","schema":1,"tick":61500,"run":"r1","bots":[]}"#,
                r#"{"kind":"bots","schema":1,"tick":6156"#,
            ],
        );
        let read = read_samples(&path).unwrap();
        assert_eq!(read.samples.len(), 1);
        assert_eq!(read.skipped, 1);
    }

    #[test]
    fn ingestion_excludes_samples_from_before_the_run() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        write(
            &out,
            &[
                r#"{"kind":"bots","schema":1,"tick":100,"run":null,"bots":[]}"#,
                r#"{"kind":"bots","schema":1,"tick":61500,"run":null,"bots":[]}"#,
            ],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let count = ingest_samples(&workspace, &run_dir, "ours", 61269).unwrap();

        // A previous run's leftover file cannot leak into this one.
        assert_eq!(count, 1);
        let written = std::fs::read_to_string(run_dir.join("samples.jsonl")).unwrap();
        assert!(written.contains("61500"));
        assert!(!written.contains(r#""tick":100"#));
    }

    #[test]
    fn ingestion_excludes_a_line_stamped_with_a_different_run_even_in_tick_range() {
        // The decoy case this correction exists for: two runs on freshly
        // generated maps both pass through the same tick, so a tick check
        // alone would let a stale line through. A named mismatch is a known
        // fact and must win over any tick guess.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        write(
            &out,
            &[r#"{"kind":"bots","schema":1,"tick":61500,"run":"STALE-RUN","bots":[]}"#],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let count = ingest_samples(&workspace, &run_dir, "ours", 100).unwrap();

        assert_eq!(
            count, 0,
            "a line naming another run is excluded regardless of tick"
        );
    }

    #[test]
    fn ingestion_includes_a_line_stamped_with_this_run_even_below_not_before() {
        // A line naming this run is a known fact, stronger than the
        // high-water-mark guess `not_before` represents. It must be kept even
        // when its tick predates the mark.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        write(
            &out,
            &[r#"{"kind":"bots","schema":1,"tick":100,"run":"ours","bots":[]}"#],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let count = ingest_samples(&workspace, &run_dir, "ours", 61269).unwrap();

        assert_eq!(
            count, 1,
            "a line naming this run is kept regardless of tick"
        );
    }

    #[test]
    fn ingestion_preserves_the_schema_stamp() {
        // `Sample` is deserialised and then re-serialised on its way into the
        // run's archive. If `schema` were probed-and-discarded rather than a
        // real field, the archived line would come out without it, and a
        // later `SchemaProbe` reading that archive back would silently accept
        // whatever it found instead of refusing an unknown shape.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        write(
            &out,
            &[r#"{"kind":"bots","schema":1,"tick":100,"run":"ours","bots":[]}"#],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let count = ingest_samples(&workspace, &run_dir, "ours", 0).unwrap();
        assert_eq!(count, 1);

        let archived = read_samples(&run_dir.join("samples.jsonl")).unwrap();
        assert_eq!(archived.samples.len(), 1);
        assert_eq!(
            archived.samples[0].schema, SAMPLE_SCHEMA,
            "the archived line must still carry the schema stamp after a round trip"
        );
    }
}
