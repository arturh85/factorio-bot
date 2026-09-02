//! Periodic world state captured by the mod, keyed on `game.tick`.
//!
//! Written by BotBridge into the *server's* `script-output` and ingested here.
//! This is a sibling of `events.jsonl`, not a replacement: a reader built
//! before this file existed opens a run unchanged, because it never asks for
//! it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
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

/// What one call to [`ingest_samples_incremental`] found and consumed.
#[derive(Debug, PartialEq, Eq)]
pub struct IngestProgress {
    /// Byte offset into the *source* file this call read up to. The next
    /// call should resume from here, not from 0 -- that is what makes
    /// repeated ingestion append-only rather than a rewrite.
    pub offset: u64,
    /// Sample lines appended to the run's `samples.jsonl` by this call.
    pub appended: usize,
    /// Lines in the newly-read portion that did not parse as a [`Sample`].
    pub skipped: usize,
}

/// Copies whatever is new in the run's samples source since `offset`,
/// appending it to `run_dir/samples.jsonl` rather than rewriting the file.
///
/// This is what lets [`crate::record::RunRecorder`] call ingestion at every
/// milestone boundary *and* at `finish` without re-reading and re-writing
/// however much has already accumulated on a long run, and without archiving
/// any line twice: each call only looks past the byte offset the previous
/// call returned, and the caller is expected to persist that offset (see
/// `RunRecorder::samples_offset`) and pass it back in.
///
/// Only *complete* lines are consumed. The mod appends a line and the OS may
/// still be mid-write when this reads the file, so the last chunk since the
/// final `\n` is left for the next call rather than treated as gospel --
/// consuming a partial line would advance the offset past bytes that have
/// not finished landing on disk, silently dropping the rest of that sample
/// forever.
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
///
/// An unrecognised `schema` still fails the whole call loudly, exactly like
/// [`read_samples`] -- and nothing is appended to the archive on that path:
/// validation happens before any line is written, so a call that errors
/// leaves both the output file and `offset` untouched, and retrying it later
/// (say, at the next milestone) reports the same error instead of silently
/// re-admitting lines a prior partial write already archived.
pub fn ingest_samples_incremental(
    workspace: &Path,
    run_dir: &Path,
    run_id: &str,
    not_before: u64,
    offset: u64,
) -> io::Result<IngestProgress> {
    let source = workspace
        .join("server")
        .join("script-output")
        .join("botbridge")
        .join("samples.jsonl");
    if !source.exists() {
        return Ok(IngestProgress {
            offset,
            appended: 0,
            skipped: 0,
        });
    }

    let mut file = File::open(&source)?;
    let len = file.metadata()?.len();
    // The mod truncates this file when a *new* run's frame capture starts.
    // That should never happen while this run is still going -- but if it
    // does, the remembered offset now points past the end of a shorter file,
    // and seeking there would read nothing forever rather than catching up.
    // Restarting from the top is safe: the run-id filter below still keeps
    // any line belonging to the run that did the truncating out of this
    // run's archive.
    let start = if offset > len { 0 } else { offset };
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let complete_len = match buf.iter().rposition(|&b| b == b'\n') {
        Some(pos) => pos + 1,
        None => 0,
    };
    if complete_len == 0 {
        // Nothing new, or only an in-progress line since last time.
        return Ok(IngestProgress {
            offset: start,
            appended: 0,
            skipped: 0,
        });
    }

    // Parsed fully before anything is written -- see the doc comment above
    // on why a schema failure must not leave a partial append behind.
    let mut parsed = Vec::new();
    let mut skipped = 0usize;
    for line in buf[..complete_len].split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            skipped += 1;
            continue;
        };
        if let Ok(probe) = serde_json::from_str::<SchemaProbe>(text)
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
        match serde_json::from_str::<Sample>(text) {
            Ok(sample) => parsed.push(sample),
            Err(_) => {
                // Concrete case: the mod writes `bots = {}` when no player is
                // connected, and `helpers.table_to_json({})` yields `"{}"`
                // rather than `"[]"`, so that line fails to deserialise as a
                // `Sample`. Counted, not silently dropped.
                skipped += 1
            }
        }
    }

    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("samples.jsonl"))?;
    let mut appended = 0usize;
    for sample in parsed.iter().filter(|s| match &s.run {
        Some(run) => run == run_id,
        None => s.tick >= not_before,
    }) {
        writeln!(out, "{}", serde_json::to_string(sample)?)?;
        appended += 1;
    }

    Ok(IngestProgress {
        offset: start + complete_len as u64,
        appended,
        skipped,
    })
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

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 61269, 0).unwrap();

        // A previous run's leftover file cannot leak into this one.
        assert_eq!(progress.appended, 1);
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

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 100, 0).unwrap();

        assert_eq!(
            progress.appended, 0,
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

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 61269, 0).unwrap();

        assert_eq!(
            progress.appended, 1,
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

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 0, 0).unwrap();
        assert_eq!(progress.appended, 1);

        let archived = read_samples(&run_dir.join("samples.jsonl")).unwrap();
        assert_eq!(archived.samples.len(), 1);
        assert_eq!(
            archived.samples[0].schema, SAMPLE_SCHEMA,
            "the archived line must still carry the schema stamp after a round trip"
        );
    }

    #[test]
    fn repeated_ingestion_from_the_returned_offset_does_not_duplicate_lines() {
        // Simulates what `RunRecorder::ingest_samples` does across a
        // milestone boundary and then `finish`: call once, let the mod append
        // more, call again with the offset the first call returned.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        let path = write(
            &out,
            &[r#"{"kind":"bots","schema":1,"tick":100,"run":"ours","bots":[]}"#],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let first = ingest_samples_incremental(&workspace, &run_dir, "ours", 0, 0).unwrap();
        assert_eq!(first.appended, 1);

        // The mod appends -- never rewrites -- while the run is live.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(
            f,
            r#"{{"kind":"bots","schema":1,"tick":200,"run":"ours","bots":[]}}"#
        )
        .unwrap();
        drop(f);

        // Resuming from `first.offset` must only pick up the new line, not
        // re-read the one already archived.
        let second =
            ingest_samples_incremental(&workspace, &run_dir, "ours", 0, first.offset).unwrap();
        assert_eq!(
            second.appended, 1,
            "only the newly-appended line is picked up"
        );

        // Calling again from the same (now current) offset with nothing new
        // written must append nothing further.
        let third =
            ingest_samples_incremental(&workspace, &run_dir, "ours", 0, second.offset).unwrap();
        assert_eq!(third.appended, 0);

        let lines: Vec<_> = std::fs::read_to_string(run_dir.join("samples.jsonl"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            lines.len(),
            2,
            "each source line must be archived exactly once: {lines:?}"
        );
        assert!(lines[0].contains(r#""tick":100"#));
        assert!(lines[1].contains(r#""tick":200"#));
    }

    #[test]
    fn an_in_progress_final_line_is_left_for_the_next_call() {
        // A write racing a read can leave the last line incomplete. Consuming
        // it anyway would advance the offset past bytes that have not
        // actually landed yet, silently losing the rest of that sample.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        let path = out.join("samples.jsonl");
        // No trailing newline: this line is "in progress".
        std::fs::write(
            &path,
            r#"{"kind":"bots","schema":1,"tick":100,"run":"ours","bots":[]}"#,
        )
        .unwrap();
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 0, 0).unwrap();
        assert_eq!(progress.appended, 0, "an incomplete line is not consumed");
        assert_eq!(progress.offset, 0, "the offset must not advance past it");
        assert!(!run_dir.join("samples.jsonl").exists());

        // Once the writer finishes the line, the next call picks it up.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f).unwrap();
        drop(f);
        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 0, 0).unwrap();
        assert_eq!(progress.appended, 1);
    }
}
