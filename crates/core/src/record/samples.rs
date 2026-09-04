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

use crate::graph::entity_graph::radius_from_origin;
use crate::types::Position;

/// The sample shape this build understands.
///
/// Refusing an unknown value is the point. In a debug build `workspace/mods`
/// wins over the repo checkout and there is no refresh path, and `info.json`
/// has read `0.0.1` since the project began -- so a version string cannot tell
/// a stale mod from a current one. This integer can, because it is bumped
/// deliberately whenever a field changes.
///
/// # 1 -> 2 (2026-09-03)
///
/// Added [`SampleKind::Machines`] and [`PowerSample::networks`]. The bump is
/// what makes a stale mod *loud*: schema 1 answers none of the questions this
/// data exists for, and a run that quietly recorded no machine state at all
/// would look exactly like a run whose machines were all fine.
///
/// The two read paths treat a mismatch differently, and deliberately:
///
/// * [`ingest_samples_incremental`] reads what the *currently loaded* mod just
///   wrote, so a mismatch there means a stale `workspace/mods` -- precisely
///   the bug this constant exists to catch. It stays strict.
/// * [`read_samples`] reads an *archive*, written by whatever mod ran at the
///   time. An older schema there is history, not staleness, and every change
///   so far has been additive (new variants decode to [`SampleKind::Unknown`],
///   new fields are `#[serde(default)]`), so it accepts anything up to this
///   value. Refusing old archives would have destroyed the ability to analyse
///   past runs to buy nothing.
pub const SAMPLE_SCHEMA: u32 = 2;

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
    /// Per-machine state, on the same 300-tick beat as [`SampleKind::Force`]
    /// and written from the same handler, so a machine's `network` can be
    /// joined to that tick's [`PowerSample::networks`] without interpolating.
    ///
    /// This is what makes "is this machine actually working?" answerable. The
    /// record used to carry bot state and force totals and nothing in between,
    /// so a run that built a cell, set both recipes and charged the chests
    /// could not say whether the assemblers had power, held ingredients or
    /// produced anything.
    Machines {
        /// Keyed by `unit_number` as a string, not an array.
        ///
        /// `helpers.table_to_json` renders an empty Lua table as `{}`, which
        /// is why a `bots` line with no connected players is unparseable and
        /// lands in [`ReadSamples::skipped`]. A run's first minutes
        /// legitimately have no machines at all, so this shape sidesteps that
        /// defect rather than reproducing it: as a map, "none yet" is `{}` and
        /// decodes.
        machines: BTreeMap<String, MachineSample>,
        /// Machines seen but not written, because the sample hit the mod's
        /// per-line cap. Zero on every run so far. Non-zero means this line is
        /// a prefix of a larger base -- a reader must be able to tell that
        /// from "this is all of them", which is why it is a counted field and
        /// not a dropped row.
        truncated: u32,
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
    /// The same figures per electric network, keyed by the smallest
    /// sub-network id under each parent network.
    ///
    /// The totals above sum every network the force owns, and that sum hides
    /// the failure it is most often consulted about. A run reading
    /// `generated_kw: 900, consumed_kw: 60` looks powered; if the 900 kW is on
    /// the network holding the lab and the assemblers sit on an island whose
    /// pole reaches no generator, the totals never mention it. Here that
    /// island is its own entry with `generated_kw: 0`, and
    /// [`MachineSample::network`] says which entry each machine is on.
    ///
    /// Keyed by a sub-network id because `LuaElectricNetwork` has no `id`
    /// attribute at all in 2.1.17; [`NetworkPower::sub_ids`] carries the real
    /// membership, and the key is only a handle.
    ///
    /// `#[serde(default)]` for archives written before this field existed --
    /// empty then means "not recorded", where on a current line it means "this
    /// force owns no electric poles". Both are honestly "no networks known".
    #[serde(default)]
    pub networks: BTreeMap<String, NetworkPower>,
}

/// One electric network's own generation and demand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NetworkPower {
    /// Every electric *sub*-network under this parent, ascending. A machine
    /// belongs to this network when its [`MachineSample::network`] is one of
    /// these. Several sub-networks share a parent when a closed power switch
    /// joins them, which is why this is a set and not a single id.
    pub sub_ids: Vec<u32>,
    pub generated_kw: f64,
    pub consumed_kw: f64,
    /// What the network's consumers asked for, as opposed to what they got.
    /// Present here and not in the force totals because the gap between demand
    /// and transfer is the whole diagnosis for a browning-out island.
    pub demanded_kw: f64,
    /// Consumed over demanded, 1.0 when nothing is demanded.
    pub satisfaction: f64,
}

/// One machine's state at the sample's tick.
///
/// Fields that do not apply to an entity's type are **absent**, not null, and
/// that is a deliberate departure from this module's usual "present-and-null,
/// never absent" rule. That rule is for a value a producer always has an
/// answer for; here `recipe`, `crafting`, `progress` and `products_finished`
/// are declared by the Factorio API for `CraftingMachine` only, so on a lab or
/// a boiler the question does not exist rather than having a null answer.
/// Reading them off one raises -- which is how a bad attribute read in
/// `sample_bots` once took a live server down mid-run -- so the mod does not
/// read them, and writes no key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MachineSample {
    /// Prototype name, e.g. `assembling-machine-1`.
    pub name: String,
    /// Entity type. The sampled set is exactly `assembling-machine`,
    /// `furnace`, `mining-drill`, `lab`, `boiler`, `generator`, `container`
    /// and `logistic-container` -- the machines a run's plan places, plus the
    /// chests it feeds them from.
    #[serde(rename = "type")]
    pub entity_type: String,
    /// As the game reports it. A tile centre stays `-40.5`; nothing rounds.
    pub position: Position,
    /// `LuaEntity.status`, by name -- the game's own verdict on why this
    /// machine is or is not running, and the single field most of this data
    /// exists for. `working`, `no_power`, `low_power`, `no_ingredients`,
    /// `full_output`, `not_enough_space_in_output`, `no_recipe`, `no_fuel`,
    /// `not_plugged_in_electric_network` and `no_minable_resources` are the
    /// ones a run of this project actually hits.
    ///
    /// The name, not the number: a `defines.entity_status` id is meaningless
    /// without the table that produced it, and an archive outlives the
    /// Factorio version that wrote it. An id the mod could not name arrives as
    /// `unmapped_<n>` rather than being dropped.
    ///
    /// `None` when the entity has no status at all -- the attribute is
    /// `optional` in the API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// `LuaEntity.electric_network_id`, or `None` for a machine connected to
    /// no electric network -- which for an assembling machine is itself the
    /// answer to "did the pole we placed actually connect it".
    ///
    /// Join against [`NetworkPower::sub_ids`] in the same tick's force sample.
    /// An id matching no sampled network means no *pole* on the force reached
    /// that network, since [`PowerSample::networks`] is enumerated from poles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<u32>,
    /// What `get_recipe()` reports, which is the only honest verdict on what a
    /// crafting machine is set to: `set_recipe` returns the items it *removed*
    /// rather than a success flag, so a machine that ignored the call reads as
    /// configured everywhere except here.
    ///
    /// Absent on a non-crafting machine; present and `None` never happens --
    /// a crafting machine with no recipe simply gets no key, and its `status`
    /// says `no_recipe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// `is_crafting()`. Absent on a non-crafting machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crafting: Option<bool>,
    /// `crafting_progress`, 0.0 to 1.0, rounded mod-side to a thousandth.
    /// Absent on a non-crafting machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// `products_finished`: lifetime completed crafts. The blunt instrument,
    /// and often the fastest one -- an assembler still reporting `0` after
    /// twenty minutes did not produce, whatever else its row says. Absent on a
    /// non-crafting machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub products_finished: Option<u64>,
    /// For a mining drill, the resource it is mining. Absent for every other
    /// type, and absent for a drill over nothing -- which is what separates
    /// `no_minable_resources` on a depleted patch from a drill that was never
    /// placed over ore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mining: Option<String>,
    /// Ingredient inventory: `crafter_input` for a crafting machine,
    /// `lab_input` for a lab, empty for anything else.
    ///
    /// **The mod omits an empty one and this struct restores it.** Most
    /// machines are idle most of the time, and `"input":{},"output":{},
    /// "fuel":{}` is forty bytes of nothing per machine per five seconds, so
    /// the wire format drops them -- hence `#[serde(default)]` on all three.
    /// They are still *serialised* unconditionally, because absence and
    /// emptiness mean the same thing here and normalising to one of them in
    /// the archive is what lets the published schema, the TypeScript
    /// declaration and this struct all say the same thing.
    #[serde(default)]
    pub input: BTreeMap<String, u32>,
    /// `get_output_inventory()`, and for a chest its whole contents -- from
    /// the run's point of view a chest is a thing the cell draws from, so
    /// "what is in this thing" stays one key rather than two that differ only
    /// by entity type.
    ///
    /// Non-empty with `status: full_output` is a cell that produced and then
    /// jammed; empty on the chest feeding a machine reporting
    /// `no_ingredients` is a cell that ran dry. Opposite repairs, and the
    /// record could previously distinguish neither.
    #[serde(default)]
    pub output: BTreeMap<String, u32>,
    /// `get_fuel_inventory()`, for burner machines -- stone furnaces, burner
    /// drills, boilers. Empty with `status: no_fuel` is the whole story.
    #[serde(default)]
    pub fuel: BTreeMap<String, u32>,
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

/// Reads a sample log, tolerating a truncated tail and an older schema, but
/// never a newer one.
///
/// This reads *archives*, so the asymmetry is the whole point (see
/// [`SAMPLE_SCHEMA`]). A line written under an older schema is a past run,
/// and every change to this schema has been additive -- unknown variants
/// decode to [`SampleKind::Unknown`], absent fields to their defaults -- so it
/// decodes into exactly the subset of state that run actually captured. A line
/// written under a *newer* schema is genuinely undecodable: fields this build
/// has never heard of may have changed meaning, not merely appeared, so it
/// still fails loudly. [`ingest_samples_incremental`], which reads the live
/// mod's output rather than an archive, keeps the strict equality that catches
/// a stale `workspace/mods`.
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
            && probe.schema > SAMPLE_SCHEMA
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "sample schema {} is newer than the {} this build \
                     understands -- the archive was written by a later build",
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

/// The furthest a bot was observed from the map origin, and which bot it was.
///
/// # Why the archive computes this and nothing else does
///
/// "How far did a bot actually get" is the honest half of the free-vision
/// disclosure (see [`crate::graph::entity_graph::VisionExtent`]), and the only
/// record that holds a bot's position over the whole run is this sample
/// stream. Reading it back later means re-parsing every sample line; the
/// ingest already parses each line exactly once on its way into the archive,
/// so the high-water mark is taken there and carried on
/// [`crate::record::RunRecorder`].
///
/// Distance is **Euclidean from the map origin**, via
/// [`crate::graph::entity_graph::radius_from_origin`] -- deliberately not
/// [`Position::distance`], which is Manhattan and would report a bot 45 tiles
/// out diagonally as 63.
#[derive(Debug, Clone, PartialEq)]
pub struct TravelObservation {
    /// Euclidean distance from `(0, 0)`, in tiles.
    pub tiles: f64,
    /// Which bot was that far out. A single number for a whole run hides
    /// which bot earned it, and "one bot went and three sat still" is a
    /// finding this project has already had to reconstruct by hand once.
    pub bot: u32,
    /// The tick of the sample this came from.
    pub tick: u64,
}

/// What one call to [`ingest_samples_incremental`] found and consumed.
///
/// Not `Eq`: [`IngestProgress::travel`] carries a distance in tiles, and a
/// float has no total equality. `PartialEq` is what the tests compare with and
/// all any caller has ever needed.
#[derive(Debug, PartialEq)]
pub struct IngestProgress {
    /// Byte offset into the *source* file this call read up to. The next
    /// call should resume from here, not from 0 -- that is what makes
    /// repeated ingestion append-only rather than a rewrite.
    pub offset: u64,
    /// Sample lines appended to the run's `samples.jsonl` by this call.
    pub appended: usize,
    /// Lines in the newly-read portion that did not parse as a [`Sample`].
    pub skipped: usize,
    /// The highest tick among the samples this call *appended*, or `None` when
    /// it appended none. Reported rather than left to the caller to recompute,
    /// because the caller would have to reread the archive it just wrote to
    /// learn it -- and it is what tells a run whether its sample stream
    /// actually reached the end of the run.
    pub high_tick: Option<u64>,
    /// The furthest any bot in the lines this call *appended* stood from the
    /// map origin.
    ///
    /// `None` when this call appended no bot position at all -- which is the
    /// ordinary case for a call that appended only `force` and `machines`
    /// lines, and is **not** a claim that no bot moved. The caller keeps the
    /// high-water mark across calls; see [`crate::record::RunRecorder`].
    pub travel: Option<TravelObservation>,
    /// How many [`SampleKind::Bots`] entries this call appended a position
    /// from -- the denominator behind `travel`.
    ///
    /// Reported because a null `travel` has two very different causes: no bot
    /// sample was read at all (this is zero, and the run's travel is genuinely
    /// unknown), or bot samples were read and none of them is further out than
    /// what a previous call already saw. Without the count the two are
    /// indistinguishable, and the first is a broken instrument while the
    /// second is a quiet run.
    pub bot_positions: usize,
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
/// Filtered on the run id *first*, tick second -- not on tick alone. The mod
/// stamps `run` on each line, and a line naming a *different* run is excluded
/// regardless of its tick, because a named mismatch is a known fact and a tick
/// comparison is only ever a guess. Every run starts near tick 0 on a freshly
/// generated map, so two runs' tick ranges overlap almost entirely -- two runs
/// can both pass through tick 61,500, and a leftover `samples.jsonl` from the
/// older one would otherwise be ingested as this run's data.
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
            high_tick: None,
            travel: None,
            bot_positions: 0,
        });
    }

    let mut file = File::open(&source)?;
    let len = file.metadata()?.len();
    // The mod truncates this file when a *new* run's sampling session starts.
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
            high_tick: None,
            travel: None,
            bot_positions: 0,
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
        // Strict equality here, unlike [`read_samples`]'s "not newer". This
        // path reads what the mod running *right now* just wrote, so an older
        // schema means a stale `workspace/mods` -- which is the exact bug
        // [`SAMPLE_SCHEMA`] exists to catch, and which would otherwise present
        // as a run that silently recorded less than it was asked to.
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
    let mut high_tick: Option<u64> = None;
    // Measured over the lines that are actually archived, never over `parsed`:
    // a line belonging to another run is excluded from the archive and must be
    // excluded from this run's travel too, for exactly the reason the run-id
    // filter exists at all.
    let mut travel: Option<TravelObservation> = None;
    let mut bot_positions = 0usize;
    for sample in parsed.iter().filter(|s| match &s.run {
        Some(run) => run == run_id,
        None => s.tick >= not_before,
    }) {
        writeln!(out, "{}", serde_json::to_string(sample)?)?;
        appended += 1;
        high_tick = Some(high_tick.map_or(sample.tick, |t: u64| t.max(sample.tick)));
        if let SampleKind::Bots { bots } = &sample.kind {
            for bot in bots {
                bot_positions += 1;
                let tiles = radius_from_origin(&bot.position);
                // Ties broken on the bot id so a run reports the same bot each
                // time two are the same distance out; nothing about the sample
                // order is stable enough to rely on.
                let better = travel.as_ref().is_none_or(|best| {
                    matches!(
                        tiles.total_cmp(&best.tiles).then(bot.id.cmp(&best.bot)),
                        std::cmp::Ordering::Greater
                    )
                });
                if better {
                    travel = Some(TravelObservation {
                        tiles,
                        bot: bot.id,
                        tick: sample.tick,
                    });
                }
            }
        }
    }

    Ok(IngestProgress {
        offset: start + complete_len as u64,
        appended,
        skipped,
        high_tick,
        travel,
        bot_positions,
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
                r#"{"kind":"bots","schema":2,"tick":61500,"run":"r1","bots":[{"id":1,"position":{"x":-40.5,"y":-48.5},"inventory":{"iron-ore":23},"crafting_queue":0,"mining":"iron-ore"}]}"#,
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
                r#"{"kind":"force","schema":2,"tick":61500,"run":"r1","research":null,"techs_unlocked":7,"production":{"made":{"iron-plate":120},"consumed":{}},"power":{"generated_kw":180.0,"consumed_kw":150.0,"satisfaction":1.0}}"#,
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
        // An archive from a *later* build is genuinely undecodable -- a field
        // this build has never heard of may have changed meaning rather than
        // merely appeared -- so it must fail loudly, naming what it found.
        assert!(
            err.to_string().contains("99"),
            "error must name the schema: {err}"
        );
    }

    #[test]
    fn reads_an_archive_written_under_an_older_schema() {
        // The counterpart to the test above, and the reason the archive guard
        // is "not newer" rather than "equal". Every run recorded before
        // schema 2 is stamped 1; refusing them would have made the whole
        // archive unreadable to buy nothing, since the change was additive.
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"force","schema":1,"tick":61500,"run":"r1","research":null,"techs_unlocked":7,"production":{"made":{},"consumed":{}},"power":{"generated_kw":900.0,"consumed_kw":60.0,"satisfaction":1.0}}"#,
            ],
        );
        let read = read_samples(&path).unwrap();
        assert_eq!(read.samples.len(), 1);
        // The stamp survives the read, so a later bump can still refuse it.
        assert_eq!(read.samples[0].schema, 1);
        let SampleKind::Force { power, .. } = &read.samples[0].kind else {
            panic!("expected a force sample");
        };
        // A schema-1 line knew nothing about individual networks. Empty here
        // means "not recorded", and the reader must not invent one.
        assert!(power.networks.is_empty());
    }

    #[test]
    fn reads_a_machines_sample() {
        let tmp = tempfile::tempdir().unwrap();
        // The shape that answers the question this kind exists for: an
        // assembling machine with its recipe set, its ingredients present, and
        // the game itself reporting `no_power` -- on a network the force
        // sample shows generating nothing. Force totals cannot express this.
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"machines","schema":2,"tick":61500,"run":"r1","truncated":0,"machines":{"312":{"name":"assembling-machine-1","type":"assembling-machine","position":{"x":-12.5,"y":-58.5},"status":"no_power","network":7,"recipe":"iron-gear-wheel","crafting":false,"progress":0.0,"products_finished":0,"input":{"iron-plate":40}},"77":{"name":"stone-furnace","type":"furnace","position":{"x":-58.0,"y":13.0},"status":"no_fuel","recipe":"iron-plate","crafting":false,"progress":0.5,"products_finished":12,"input":{"iron-ore":8},"output":{"iron-plate":3}}}}"#,
            ],
        );
        let read = read_samples(&path).unwrap();
        let SampleKind::Machines {
            machines,
            truncated,
        } = &read.samples[0].kind
        else {
            panic!("expected a machines sample");
        };
        assert_eq!(*truncated, 0);
        assert_eq!(machines.len(), 2);

        let asm = &machines["312"];
        assert_eq!(asm.entity_type, "assembling-machine");
        // The three fields the whole feature is for: the recipe really is set,
        // the ingredients really are there, and the machine still is not
        // running -- because it has no power, not because it lacks either.
        assert_eq!(asm.recipe.as_deref(), Some("iron-gear-wheel"));
        assert_eq!(asm.input["iron-plate"], 40);
        assert_eq!(asm.status.as_deref(), Some("no_power"));
        assert_eq!(asm.network, Some(7));
        assert_eq!(asm.products_finished, Some(0));
        // Absent, not empty-and-present, and both decode the same way.
        assert!(asm.output.is_empty());
        assert!(asm.fuel.is_empty());

        // A stone furnace carries no `network` at all -- it is not an electric
        // machine -- and that absence is not the same fact as network 7.
        let furnace = &machines["77"];
        assert_eq!(furnace.network, None);
        assert_eq!(furnace.status.as_deref(), Some("no_fuel"));
        assert!(furnace.fuel.is_empty());
        assert_eq!(furnace.output["iron-plate"], 3);
    }

    #[test]
    fn a_machines_sample_with_no_machines_yet_still_parses() {
        // The `bots = {}` defect, not repeated: `helpers.table_to_json` writes
        // an empty Lua table as `{}`, so a `bots` line from a run with no
        // connected players is unparseable and lands in `skipped`. `machines`
        // is a map for exactly this reason, and a run's first minutes really
        // do have no machines.
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"machines","schema":2,"tick":300,"run":"r1","machines":{},"truncated":0}"#,
            ],
        );
        let read = read_samples(&path).unwrap();
        assert_eq!(read.skipped, 0);
        let SampleKind::Machines { machines, .. } = &read.samples[0].kind else {
            panic!("expected a machines sample");
        };
        assert!(machines.is_empty());
    }

    #[test]
    fn per_network_power_separates_an_unpowered_island_from_the_force_total() {
        // The evening this data was added to settle: the force reads 900 kW
        // generated against 60 kW consumed and looks entirely healthy, while
        // the cell's assemblers sit on a second network that generates
        // nothing. The totals cannot say this; the per-network split can.
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"force","schema":2,"tick":61500,"run":"r1","research":null,"techs_unlocked":7,"production":{"made":{},"consumed":{}},"power":{"generated_kw":900.0,"consumed_kw":60.0,"satisfaction":1.0,"networks":{"3":{"sub_ids":[3],"generated_kw":900.0,"consumed_kw":60.0,"demanded_kw":60.0,"satisfaction":1.0},"7":{"sub_ids":[7,9],"generated_kw":0.0,"consumed_kw":0.0,"demanded_kw":189.0,"satisfaction":0.0}}}}"#,
            ],
        );
        let read = read_samples(&path).unwrap();
        let SampleKind::Force { power, .. } = &read.samples[0].kind else {
            panic!("expected a force sample");
        };
        assert_eq!(power.generated_kw, 900.0);
        // Healthy force-wide, and one of its two networks is dead.
        assert_eq!(power.satisfaction, 1.0);
        let island = &power.networks["7"];
        assert_eq!(island.generated_kw, 0.0);
        assert_eq!(island.demanded_kw, 189.0);
        assert_eq!(island.satisfaction, 0.0);
        // A machine's `network` is matched against `sub_ids`, not the key: a
        // power switch can put several sub-networks under one parent, and the
        // key is only the smallest of them.
        assert!(island.sub_ids.contains(&9));
    }

    #[test]
    fn skips_a_truncated_final_line_and_counts_it() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(
            tmp.path(),
            &[
                r#"{"kind":"bots","schema":2,"tick":61500,"run":"r1","bots":[]}"#,
                r#"{"kind":"bots","schema":2,"tick":6156"#,
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
                r#"{"kind":"bots","schema":2,"tick":100,"run":null,"bots":[]}"#,
                r#"{"kind":"bots","schema":2,"tick":61500,"run":null,"bots":[]}"#,
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
            &[r#"{"kind":"bots","schema":2,"tick":61500,"run":"STALE-RUN","bots":[]}"#],
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
            &[r#"{"kind":"bots","schema":2,"tick":100,"run":"ours","bots":[]}"#],
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
    fn ingestion_reports_the_furthest_bot_and_counts_the_positions_behind_it() {
        // The travel half of the free-vision disclosure, taken on the way past
        // rather than by re-parsing the archive later.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        write(
            &out,
            &[
                r#"{"kind":"bots","schema":2,"tick":100,"run":"ours","bots":[{"id":1,"position":{"x":-3.5,"y":4.5},"inventory":{},"crafting_queue":0,"mining":null},{"id":2,"position":{"x":30.5,"y":40.5},"inventory":{},"crafting_queue":0,"mining":null}]}"#,
                r#"{"kind":"force","schema":2,"tick":100,"run":"ours","research":null,"techs_unlocked":1,"production":{"made":{},"consumed":{}},"power":{"generated_kw":0.0,"consumed_kw":0.0,"satisfaction":1.0}}"#,
            ],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 0, 0).unwrap();

        let travel = progress.travel.expect("a bot position was archived");
        assert_eq!(travel.bot, 2, "the furthest one, not the last one read");
        assert_eq!(travel.tick, 100);
        assert!(
            (travel.tiles - 30.5f64.hypot(40.5)).abs() < 1e-9,
            "Euclidean from the origin, got {}",
            travel.tiles
        );
        assert_eq!(
            progress.bot_positions, 2,
            "both bots counted; the force line contributes none"
        );
    }

    /// **A call that archived no bot position says so with a zero, not with a
    /// distance of zero.**
    ///
    /// The two are opposite facts -- "nothing looked" and "a bot was at the
    /// origin" -- and this project has already shipped four instruments that
    /// reported the second while meaning the first.
    #[test]
    fn ingestion_reports_no_travel_at_all_rather_than_a_travel_of_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        write(
            &out,
            &[
                r#"{"kind":"force","schema":2,"tick":100,"run":"ours","research":null,"techs_unlocked":1,"production":{"made":{},"consumed":{}},"power":{"generated_kw":0.0,"consumed_kw":0.0,"satisfaction":1.0}}"#,
            ],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 0, 0).unwrap();
        assert_eq!(progress.appended, 1);
        assert!(progress.travel.is_none());
        assert_eq!(progress.bot_positions, 0);
    }

    #[test]
    fn a_bot_position_from_another_run_does_not_count_as_this_run_s_travel() {
        // The run-id filter that keeps a stale line out of the archive has to
        // keep it out of the measurement too, or a previous run's expedition
        // is disclosed as this run's.
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let out = workspace.join("server/script-output/botbridge");
        std::fs::create_dir_all(&out).unwrap();
        write(
            &out,
            &[
                r#"{"kind":"bots","schema":2,"tick":100,"run":"STALE-RUN","bots":[{"id":1,"position":{"x":300.5,"y":400.5},"inventory":{},"crafting_queue":0,"mining":null}]}"#,
                r#"{"kind":"bots","schema":2,"tick":101,"run":"ours","bots":[{"id":1,"position":{"x":3.5,"y":4.5},"inventory":{},"crafting_queue":0,"mining":null}]}"#,
            ],
        );
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let progress = ingest_samples_incremental(&workspace, &run_dir, "ours", 0, 0).unwrap();
        let travel = progress.travel.expect("our own line was archived");
        assert!(
            (travel.tiles - 3.5f64.hypot(4.5)).abs() < 1e-9,
            "the stale run's 500-tile expedition must not be ours: {}",
            travel.tiles
        );
        assert_eq!(progress.bot_positions, 1);
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
            &[r#"{"kind":"bots","schema":2,"tick":100,"run":"ours","bots":[]}"#],
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
            &[r#"{"kind":"bots","schema":2,"tick":100,"run":"ours","bots":[]}"#],
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
            r#"{{"kind":"bots","schema":2,"tick":200,"run":"ours","bots":[]}}"#
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
            r#"{"kind":"bots","schema":2,"tick":100,"run":"ours","bots":[]}"#,
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
