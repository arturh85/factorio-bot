# Run Record Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record what the world looked like during a run — bot inventories, force statistics, the entity map, and the planner's intent — and present it as both a watchable timeline and a diagnosis view.

**Architecture:** BotBridge pushes periodic samples into the server's `script-output`; the recorder ingests them into two new sibling files (`samples.jsonl`, `map.jsonl`) keyed on `game.tick`, the clock every stream already shares. The executor logs placements with intent and truth together. The viewer gains a map canvas and production panels on the existing run page, and an analysis route joining planned steps to what actually happened.

**Tech Stack:** Rust (serde, utoipa, axum), Lua 5.4 (BotBridge / Factorio 2.0 API), Vue 3 + Pinia 4 + vitest.

**Spec:** `docs/superpowers/specs/2026-09-01-run-record-enrichment-design.md`

## Global Constraints

- **Sample schema integer is `1`.** Every sample line carries `"schema":1`. The ingester refuses an unknown schema and names the value it found. Bump on any field change, including additions.
- **Bot sample interval: 60 ticks. Force sample interval: 300 ticks.** 60 UPS, so 1 s and 5 s.
- **Exactly one new `on_nth_tick` registration, on 60.** Frame capture already owns 300 and `on_nth_tick(n, f)` *replaces* the handler for `n`. The force sample is written from inside the existing 300-tick frame handler. Registering a second handler on 300 silently unregisters frame capture.
- **Sample writes are server-only**: `helpers.write_file(name, data, true, 0)`. The fourth argument restricts the write; without it every peer writes its own copy, as frames already do.
- **`production` totals are cumulative from game start**, never per-interval.
- **Present-and-null, never absent.** A field nobody could determine is written as `null`. A missing key cannot be told apart from "we never asked".
- **Resource positions are tile centres** (`-40.5`, never `-41`). `EntityGraph` keys resources by `Pos(i32, i32)`, which floors — anything reading a position back out for a keyframe must restore the half-tile offset.
- **Do not debug the mod with `rcon.print`.** Its output lands in the RCON reply body and the executor reads that reply as the action's result. Use `writeout(...)`.
- **The executor issues only legitimate player actions.** No `cheat_*` calls anywhere in this plan.
- **The OpenAPI seam fails from both ends.** A new route means: regenerate `app/src/api/openapi.snapshot.json` with `UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p factorio-bot-server --features lua --test openapi`, then mirror the type in `app/src/api/types.ts` with an `objectContract<T>` declaration.
- **Gates, every task:** `cargo fmt`, `cargo clippy --workspace --all-features --all-targets -- --deny warnings`, `cargo test --workspace`. Frontend tasks additionally `cd app && pnpm lint` and `pnpm run test:coverage` (lines 90 / statements 90 / branches 80).
- **`mlua`'s `Option::None` reaches Lua as light userdata and is TRUTHY.** `x or {}` does not substitute a default. Guard with `type(x) == "table"`.
- **Editing the mod:** in a debug build `workspace/mods` wins over the repo and there is **no refresh path**. Edit `workspace/mods/BotBridge/control.lua` directly while iterating, or delete `workspace/mods` to re-seed. Every run logs `Using mods directory <path> (<why>)` — that line, not a guess, says which copy shipped.

---

## File Structure

| File | Responsibility |
|---|---|
| `mods/BotBridge/control.lua` | Sampling handlers, `rcon_sample_bots`, schema stamp |
| `crates/core/src/record/samples.rs` | **new** — sample types, parsing, schema validation, ingestion |
| `crates/core/src/record/map.rs` | **new** — placement/removal/keyframe types, divergence |
| `crates/core/src/record/mod.rs` | `EventKind` additions, manifest counts, `finish` wiring |
| `crates/executor/src/log.rs` | `Placement` on `Attempt` |
| `crates/executor/src/rcon_actuator.rs` | Records intent and the game's reply at the place call |
| `crates/scripting_lua/src/globals/record.rs` | Carries placements and plan DAG from Lua into the recorder |
| `crates/server/src/runs.rs` | `/samples` and `/map` routes |
| `app/src/lib/runSamples.ts` | **new** — pure: sample lookup at a tick, production series |
| `app/src/lib/runMap.ts` | **new** — pure: reconstruct the entity map at a tick from keyframe + deltas |
| `app/src/lib/runDiff.ts` | **new** — pure: join planned steps to observed dispatch/settle |
| `app/src/components/MapPanel.vue` | **new** — canvas: ore, entities, bot dots and trails |
| `app/src/pages/RunsPage.vue` | Hosts the map and production panels |
| `app/src/pages/RunAnalysisPage.vue` | **new** — overrun table, divergence list, milestone forensics |

---
## Phase 1 — Tier A: state samples

### Task 1: Sample types and ingestion

**Files:**
- Create: `crates/core/src/record/samples.rs`
- Modify: `crates/core/src/record/mod.rs` (add `pub mod samples;`)
- Test: inline `#[cfg(test)]` module in `crates/core/src/record/samples.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Sample`, `SampleKind`, `BotSample`, `ResearchSample`, `PowerSample`, `SAMPLE_SCHEMA: u32`, `read_samples(path: &Path) -> io::Result<ReadSamples>`, `ingest_samples(workspace: &Path, run_dir: &Path, not_before: u64) -> io::Result<usize>`.

- [ ] **Step 1: Write the failing tests**

```rust
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
        let path = write(tmp.path(), &[
            r#"{"kind":"bots","schema":1,"tick":61500,"bots":[{"id":1,"position":{"x":-40.5,"y":-48.5},"inventory":{"iron-ore":23},"crafting_queue":0,"mining":"iron-ore"}]}"#,
        ]);
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
        let path = write(tmp.path(), &[
            r#"{"kind":"force","schema":1,"tick":61500,"research":null,"techs_unlocked":7,"production":{"made":{"iron-plate":120},"consumed":{}},"power":{"generated_kw":180.0,"consumed_kw":150.0,"satisfaction":1.0}}"#,
        ]);
        let read = read_samples(&path).unwrap();
        let SampleKind::Force { research, techs_unlocked, production, power } = &read.samples[0].kind
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
        let path = write(tmp.path(), &[
            r#"{"kind":"bots","schema":99,"tick":61500,"bots":[]}"#,
        ]);
        let err = read_samples(&path).unwrap_err();
        // A stale workspace/mods must fail loudly, naming what it found.
        assert!(err.to_string().contains("99"), "error must name the schema: {err}");
    }

    #[test]
    fn skips_a_truncated_final_line_and_counts_it() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), &[
            r#"{"kind":"bots","schema":1,"tick":61500,"bots":[]}"#,
            r#"{"kind":"bots","schema":1,"tick":6156"#,
        ]);
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
        write(&out, &[
            r#"{"kind":"bots","schema":1,"tick":100,"bots":[]}"#,
            r#"{"kind":"bots","schema":1,"tick":61500,"bots":[]}"#,
        ]);
        let run_dir = tmp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();

        let count = ingest_samples(&workspace, &run_dir, 61269).unwrap();

        // A previous run's leftover file cannot leak into this one.
        assert_eq!(count, 1);
        let written = std::fs::read_to_string(run_dir.join("samples.jsonl")).unwrap();
        assert!(written.contains("61500"));
        assert!(!written.contains(r#""tick":100"#));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p factorio-bot-core record::samples`
Expected: FAIL — `unresolved import`, the module does not exist.

- [ ] **Step 3: Write the implementation**

```rust
//! Periodic world state captured by the mod, keyed on `game.tick`.
//!
//! Written by BotBridge into the *server's* `script-output` and ingested here.
//! This is a sibling of `events.jsonl`, not a replacement: a reader built
//! before this file existed opens a run unchanged, because it never asks for
//! it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
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
    pub tick: u64,
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
    pub mining: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResearchSample {
    pub name: String,
    /// 0.0 to 1.0.
    pub progress: f64,
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
        if let Ok(probe) = serde_json::from_str::<SchemaProbe>(&line) {
            if probe.schema != SAMPLE_SCHEMA {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "sample schema {} is not the {} this build understands \
                         -- workspace/mods is probably stale",
                        probe.schema, SAMPLE_SCHEMA
                    ),
                ));
            }
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
/// Filtered by `not_before` rather than by run id: unlike frames, samples have
/// no per-run sidecar, and the recorder's high-water mark is what separates
/// this run's lines from a previous run's leftovers.
pub fn ingest_samples(workspace: &Path, run_dir: &Path, not_before: u64) -> io::Result<usize> {
    let source = workspace
        .join("server")
        .join("script-output")
        .join("botbridge")
        .join("samples.jsonl");
    if !source.exists() {
        return Ok(0);
    }
    let read = read_samples(&source)?;
    let mut out = File::create(run_dir.join("samples.jsonl"))?;
    let mut count = 0usize;
    for sample in read.samples.iter().filter(|s| s.tick >= not_before) {
        writeln!(out, "{}", serde_json::to_string(sample)?)?;
        count += 1;
    }
    Ok(count)
}
```

Add to `crates/core/src/record/mod.rs`, beside the existing `pub mod frames;`:

```rust
pub mod samples;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p factorio-bot-core record::samples`
Expected: PASS, 5 tests.

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git add crates/core/src/record/samples.rs crates/core/src/record/mod.rs
git commit -m "feat(record): read and ingest world-state samples"
```

---

### Task 2: BotBridge writes the samples

**Files:**
- Modify: `mods/BotBridge/control.lua`

**Interfaces:**
- Consumes: `SAMPLE_SCHEMA = 1` from Task 1 — the mod writes it, Rust validates it.
- Produces: `script-output/botbridge/samples.jsonl` with `bots` and `force` lines; a `rcon_sample_bots()` remote callable by the executor.

- [ ] **Step 1: Add the constants and the shared writer**

Place beside the existing `FRAME_CAPTURE_*` constants:

```lua
-- Sample schema. Bumped deliberately on every field change, because
-- info.json has read 0.0.1 since the project began and cannot tell a stale
-- workspace/mods from a current one. Rust refuses a schema it does not know.
local SAMPLE_SCHEMA = 1
local SAMPLE_DIR = "botbridge"
local SAMPLE_FILE = SAMPLE_DIR .. "/samples.jsonl"
local SAMPLE_BOT_INTERVAL = 60 -- 1 s at 60 UPS

-- Appends one JSON line, on the server only.
--
-- The fourth argument is what restricts the write. `on_nth_tick` runs on every
-- peer, which is why each client writes its own frames -- but force statistics
-- are identical on every peer and bot inventories are readable from any of
-- them, so four copies would be four identical files to reconcile for nothing.
local function write_sample(line)
	helpers.write_file(SAMPLE_FILE, helpers.table_to_json(line) .. "\n", true, 0)
end
```

- [ ] **Step 2: Add the bot sampler on its own 60-tick registration**

```lua
local function sample_bots(tick)
	local bots = {}
	for _, player in pairs(game.connected_players) do
		local character = player.character
		bots[#bots + 1] = {
			id = player.index,
			position = player.position,
			-- `inventory_counts` handles Factorio 2.0's get_contents(),
			-- which returns an array of {name, count, quality}, not a dict.
			inventory = character and inventory_counts(
				character.get_inventory(defines.inventory.character_main)
			) or {},
			crafting_queue = player.crafting_queue_size or 0,
			mining = character and character.mining_state.mining
				and character.mining_target and character.mining_target.name or nil,
		}
	end
	write_sample({
		kind = "bots",
		schema = SAMPLE_SCHEMA,
		tick = tick,
		bots = bots,
	})
end

script.on_nth_tick(SAMPLE_BOT_INTERVAL, function(event)
	sample_bots(event.tick)
end)
```

- [ ] **Step 3: Fold the force sample into the existing 300-tick frame handler**

Define `power_totals` (Step 4) **above** `sample_force` in the file. Lua resolves
a `local function` at call time from the enclosing scope, so a `local` declared
later in the chunk is not visible to an earlier one — `sample_force` would call
a nil value.


Do **not** add `script.on_nth_tick(300, ...)`. `on_nth_tick(n, f)` replaces the handler registered for `n`, and frame capture already owns 300 — a second registration silently stops frame capture, producing a run with no frames and nothing to say why. Add this call inside the body of the existing `FRAME_CAPTURE_INTERVAL` handler:

```lua
local function sample_force(tick)
	local force = game.forces.player
	local research = nil
	if force.current_research then
		local tech = force.current_research
		research = {
			name = tech.name,
			progress = force.research_progress,
			eta_ticks = nil,
		}
	end
	local unlocked = 0
	for _, tech in pairs(force.technologies) do
		if tech.researched then unlocked = unlocked + 1 end
	end
	local made, consumed = {}, {}
	local stats = force.get_item_production_statistics(game.surfaces[1])
	for name, count in pairs(stats.input_counts) do made[name] = count end
	for name, count in pairs(stats.output_counts) do consumed[name] = count end

	write_sample({
		kind = "force",
		schema = SAMPLE_SCHEMA,
		tick = tick,
		research = research,
		techs_unlocked = unlocked,
		production = { made = made, consumed = consumed },
		power = power_totals(force),
	})
end
```

- [ ] **Step 4: Add `power_totals`**

Power is read from the force's electric networks. A network with no generation still reports demand, which is exactly the case that matters — coverage is not capacity.

```lua
-- Generation and demand across every electric network the force owns.
--
-- Reported in kW to match the numbers a player sees. `satisfaction` is
-- consumed over demanded, or 1.0 when nothing demands anything -- a network
-- with no load is fully satisfied, not divided by zero.
local function power_totals(force)
	local generated, demanded = 0.0, 0.0
	for _, surface in pairs(game.surfaces) do
		for _, pole in pairs(surface.find_entities_filtered({
			type = "electric-pole", force = force,
		})) do
			local network = pole.electric_network_statistics
			if network then
				for name, count in pairs(network.input_counts) do
					generated = generated + count
				end
				for name, count in pairs(network.output_counts) do
					demanded = demanded + count
				end
				break -- one pole per network is enough; statistics are shared
			end
		end
	end
	local satisfaction = 1.0
	if demanded > 0 then satisfaction = math.min(1.0, generated / demanded) end
	return {
		generated_kw = generated,
		consumed_kw = math.min(generated, demanded),
		satisfaction = satisfaction,
	}
end
```

- [ ] **Step 5: Add the settle-triggered remote**

```lua
-- Samples bots on demand, called by the executor immediately after an action
-- settles with a non-success status.
--
-- Only on failure: the 60-tick beat already carries what a bot held when
-- nothing went wrong, and sampling every settle would roughly double the
-- stream for that. The tick that matters for diagnosis is the tick something
-- failed, and by the next beat the bot has moved or handed off.
function rcon_sample_bots()
	sample_bots(game.tick)
end
```

- [ ] **Step 6: Verify against a live run**

```bash
rm -rf workspace/mods                 # re-seed from the repo checkout
cargo build --no-default-features --features cli,lua
timeout 180 target/debug/factorio-bot lua goal_smoke.lua --clients 1 --bots 2
```

Confirm the run log names the repo's mods directory (`Using mods directory … `), then:

```bash
head -2 workspace/server/script-output/botbridge/samples.jsonl
```

Expected: two JSON lines, one `"kind":"bots"`, `"schema":1`, with a non-empty `bots` array. If `samples.jsonl` is missing, the mod that shipped was the stale copy — check the `Using mods directory` line before changing any code.

Confirm frames still exist, since this task touches the handler that writes them:

```bash
ls workspace/client1/script-output/frames | head -3
```

- [ ] **Step 7: Commit**

```bash
git add mods/BotBridge/control.lua
git commit -m "feat(mod): push world-state samples on the server only"
```

---
### Task 3: Archive samples with the run, and serve them

**Files:**
- Modify: `crates/core/src/record/mod.rs` (`Manifest`, `finish`)
- Modify: `crates/server/src/runs.rs`
- Modify: `app/src/api/types.ts`, `app/src/api/client.ts`
- Test: `crates/core/src/record/mod.rs` inline tests; `app/src/api/openapi.contract.spec.ts`

**Interfaces:**
- Consumes: `samples::ingest_samples(workspace, run_dir, not_before) -> io::Result<usize>` from Task 1.
- Produces: `Manifest.samples: usize`; route `GET /api/v1/runs/{id}/samples` returning `{"samples": [Sample]}`; TypeScript `RunSamplesResponse`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn finish_counts_the_samples_it_archived() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let out = workspace.join("server/script-output/botbridge");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("samples.jsonl"),
        "{\"kind\":\"bots\",\"schema\":1,\"tick\":900,\"bots\":[]}\n",
    )
    .unwrap();

    let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-1").unwrap();
    rec.record(800, EventKind::MilestoneStarted { index: 1, goal: "g".into() })
        .unwrap();
    let (manifest, _) = rec.finish(1000, "done", Some(&workspace), 5).unwrap();

    assert_eq!(manifest.samples, 1);
}

#[test]
fn a_run_with_no_sample_file_reports_zero_rather_than_failing() {
    // The mod may not have shipped, or the run may predate sampling. Neither
    // is a reason to lose the events the run did produce.
    let tmp = tempfile::tempdir().unwrap();
    let mut rec = RunRecorder::start(&tmp.path().join("runs"), "run-2").unwrap();
    let (manifest, _) = rec
        .finish(1000, "done", Some(&tmp.path().join("workspace")), 5)
        .unwrap();
    assert_eq!(manifest.samples, 0);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p factorio-bot-core finish_counts_the_samples`
Expected: FAIL — `Manifest` has no field `samples`.

- [ ] **Step 3: Add the field and wire ingestion**

In `Manifest`, beside `frames`:

```rust
    /// How many samples were archived. A count, not a size: a reader deciding
    /// whether to fetch the stream cares how many records it will get.
    #[serde(default)]
    pub samples: usize,
```

In `finish`, after the existing `archive_frames` call and guarded the same way:

```rust
        let samples = match workspace {
            Some(workspace) => {
                samples::ingest_samples(workspace, &self.dir, self.not_before(0))?
            }
            None => 0,
        };
```

and set `samples` on the constructed `Manifest`.

`#[serde(default)]` is what lets a manifest written before this field existed still deserialize — the same forward-compatibility rule the rest of the format follows.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p factorio-bot-core record::`
Expected: PASS.

- [ ] **Step 5: Add the route**

In `crates/server/src/runs.rs`, following the shape of the existing `get_run_frames`:

```rust
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RunSamplesResponse {
    pub samples: Vec<Sample>,
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}/samples",
    params(("id" = String, Path, description = "Run id")),
    responses(
        (status = 200, body = RunSamplesResponse),
        (status = 404, description = "No such run"),
    ),
    tag = "runs",
)]
pub async fn get_run_samples(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RunSamplesResponse>, StatusCode> {
    let dir = run_dir(&state, &id).ok_or(StatusCode::NOT_FOUND)?;
    let path = dir.join("samples.jsonl");
    // A run recorded before sampling existed is not an error; it has none.
    if !path.exists() {
        return Ok(Json(RunSamplesResponse { samples: Vec::new() }));
    }
    let read = read_samples(&path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(RunSamplesResponse { samples: read.samples }))
}
```

Register it on the router beside the other run routes, and add both `RunSamplesResponse` and the sample types to the utoipa `components(schemas(...))` list.

- [ ] **Step 6: Regenerate the snapshot and mirror the type**

```bash
UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p factorio-bot-server --features lua --test openapi
```

Then in `app/src/api/types.ts`, declare `Sample`, `BotSample`, `ResearchSample`, `ProductionSample`, `PowerSample` and `RunSamplesResponse`, each tied to its schema with `objectContract<T>` in `app/src/api/openapi.contract.spec.ts`, and add `getRunSamples(id: string)` to `app/src/api/client.ts`.

- [ ] **Step 7: Verify both halves of the seam**

Run: `cargo test -p factorio-bot-server --features lua --test openapi` then `cd app && pnpm run test:coverage`
Expected: PASS on both. A field present in Rust and missing in TypeScript fails the contract spec; that is the seam working, not a flake.

- [ ] **Step 8: Commit**

```bash
cargo fmt
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git add crates/core/src/record/mod.rs crates/server/src/runs.rs \
        app/src/api/openapi.snapshot.json app/src/api/types.ts \
        app/src/api/client.ts app/src/api/openapi.contract.spec.ts
git commit -m "feat(server): archive and serve run samples"
```

---

### Task 4: Production, research and inventory panels

**Files:**
- Create: `app/src/lib/runSamples.ts`, `app/src/lib/runSamples.spec.ts`
- Modify: `app/src/store/runsStore.ts`, `app/src/pages/RunsPage.vue`

**Interfaces:**
- Consumes: `getRunSamples(id)` and the `Sample` types from Task 3.
- Produces: `botSampleAt(samples, tick)`, `forceSampleAt(samples, tick)`, `productionSeries(samples, items)`, `inventoryOf(sample, bot)`; store getters `botState`, `forceState`, `production`.

- [ ] **Step 1: Write the failing tests**

```ts
describe('botSampleAt', () => {
    it('takes the latest sample at or before the cursor', () => {
        // At-or-before, never exact: samples land on a 60-tick beat and the
        // cursor does not, so an exact match would blank the panel almost
        // always.
        const s = [botSample(61500), botSample(61560), botSample(61620)];
        expect(botSampleAt(s, 61590)?.tick).toBe(61560);
    });

    it('is null before the first sample', () => {
        expect(botSampleAt([botSample(61500)], 61400)).toBeNull();
    });

    it('ignores force samples when looking for bot state', () => {
        // Both kinds share one file; a lookup that ignored `kind` would
        // return a force line and read its missing `bots` as an empty roster.
        const s = [botSample(61500), forceSample(61560)];
        expect(botSampleAt(s, 61590)?.tick).toBe(61500);
    });
});

describe('productionSeries', () => {
    it('reports cumulative totals as recorded', () => {
        const s = [forceSample(61500, {'iron-plate': 10}), forceSample(61800, {'iron-plate': 45})];
        expect(productionSeries(s, ['iron-plate'])).toEqual([
            {item: 'iron-plate', points: [{tick: 61500, made: 10}, {tick: 61800, made: 45}]}
        ]);
    });

    it('carries the last known total forward for an item a sample omits', () => {
        // The mod omits an item with no production rather than writing a zero.
        // Reading absence as zero would draw a cumulative curve that drops.
        const s = [forceSample(61500, {'iron-plate': 10}), forceSample(61800, {})];
        expect(productionSeries(s, ['iron-plate'])[0].points[1]).toEqual({tick: 61800, made: 10});
    });
});
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd app && pnpm exec vitest run src/lib/runSamples.spec.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

```ts
/**
 * Reading world-state samples at a tick.
 *
 * Pure, and kept out of the components for the same reason `runTimeline.ts`
 * is: this is the part that can be wrong in a way you would not notice by
 * looking at the screen.
 */
import {BotSample, Sample} from '@/api/types';

export interface ProductionPoint {
    tick: number;
    made: number;
}

export interface ProductionSeries {
    item: string;
    points: ProductionPoint[];
}

/** The latest `bots` sample at or before `tick`, or null before the first. */
export function botSampleAt(samples: Sample[], tick: number): Sample | null {
    let best: Sample | null = null;
    for (const sample of samples) {
        if (sample.kind !== 'bots' || sample.tick > tick) continue;
        if (best === null || sample.tick > best.tick) best = sample;
    }
    return best;
}

/** The latest `force` sample at or before `tick`, or null before the first. */
export function forceSampleAt(samples: Sample[], tick: number): Sample | null {
    let best: Sample | null = null;
    for (const sample of samples) {
        if (sample.kind !== 'force' || sample.tick > tick) continue;
        if (best === null || sample.tick > best.tick) best = sample;
    }
    return best;
}

/** What one bot held in a sample, or null when it is not in that sample. */
export function inventoryOf(sample: Sample | null, bot: number): BotSample | null {
    if (sample === null || sample.kind !== 'bots') return null;
    return sample.bots.find((b) => b.id === bot) ?? null;
}

/**
 * Cumulative production curves.
 *
 * An item absent from a sample carries its previous total forward rather than
 * reading as zero. The mod omits an item with no production, and a cumulative
 * curve that drops to zero and back is a graph of the file format, not of the
 * factory.
 */
export function productionSeries(samples: Sample[], items: string[]): ProductionSeries[] {
    const last = new Map<string, number>(items.map((item) => [item, 0]));
    const series: ProductionSeries[] = items.map((item) => ({item, points: []}));
    const force = samples.filter((s) => s.kind === 'force').sort((a, b) => a.tick - b.tick);
    for (const sample of force) {
        if (sample.kind !== 'force') continue;
        for (const [i, item] of items.entries()) {
            const made = sample.production.made[item] ?? last.get(item) ?? 0;
            last.set(item, made);
            series[i].points.push({tick: sample.tick, made});
        }
    }
    return series;
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cd app && pnpm exec vitest run src/lib/runSamples.spec.ts`
Expected: PASS, 5 tests.

- [ ] **Step 5: Load samples in the store and render the panels**

Add `samples: Sample[]` to the store state, fetch it in `openRun` alongside the existing `getRun`/`getRunFrames`/`getRunLanes` calls, and add getters `botState`, `forceState` and `production` reading the functions above at `this.cursor`.

In `RunsPage.vue`, add three elements beside the frame panel: a research line showing the current technology name and `progress` as a percentage (or "no research queued" when `research` is null), a production block listing each tracked item's cumulative total at the cursor, and an inventory list for the selected view's bot from `inventoryOf`.

Items tracked are those named by the run's milestone goals; when none parse, fall back to every item that appears in any sample's `made`.

- [ ] **Step 6: Gates and commit**

```bash
cd app && pnpm lint && pnpm run test:coverage && pnpm run build:web
git add app/src/lib/runSamples.ts app/src/lib/runSamples.spec.ts \
        app/src/store/runsStore.ts app/src/store/runsStore.spec.ts \
        app/src/pages/RunsPage.vue
git commit -m "feat(runs): show research, production and inventory at the cursor"
```

---
## Phase 2 — Tier B: the world map

### Task 5: Placement log with drift

**Files:**
- Create: `crates/core/src/record/map.rs`
- Modify: `crates/core/src/record/mod.rs` (`pub mod map;`), `crates/executor/src/log.rs`, `crates/executor/src/rcon_actuator.rs`
- Test: inline `#[cfg(test)]` in `crates/core/src/record/map.rs`

**Interfaces:**
- Consumes: `crate::types::{Position, FactorioEntity}`.
- Produces: `MapRecord`, `MapKind`, `EntitySnapshot`, `Placement`, `drift_between(intent: &EntitySnapshot, actual: &EntitySnapshot) -> Option<Vec<String>>`; `Attempt.placed: Option<Placement>` on the executor's log.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str, x: f64, y: f64, direction: u8) -> EntitySnapshot {
        EntitySnapshot {
            name: name.to_string(),
            position: Position::new(x, y),
            direction,
        }
    }

    #[test]
    fn agreement_is_no_drift() {
        let a = snap("stone-furnace", -12.0, 8.0, 0);
        assert_eq!(drift_between(&a, &a), None);
    }

    #[test]
    fn a_differing_field_is_named() {
        // The inserter-direction trap is exactly this shape: a layout that
        // places 100% correctly and does nothing, because placement and
        // function are separate concerns.
        let intent = snap("burner-inserter", 4.0, 2.0, 0);
        let actual = snap("burner-inserter", 4.0, 2.0, 12);
        assert_eq!(drift_between(&intent, &actual), Some(vec!["direction".to_string()]));
    }

    #[test]
    fn several_differing_fields_are_all_named() {
        let intent = snap("stone-furnace", -12.0, 8.0, 0);
        let actual = snap("steel-furnace", -12.5, 8.0, 0);
        assert_eq!(
            drift_between(&intent, &actual),
            Some(vec!["name".to_string(), "position".to_string()])
        );
    }

    #[test]
    fn a_placed_record_round_trips() {
        let record = MapRecord {
            tick: 62010,
            kind: MapKind::Placed {
                bot: 3,
                intent: snap("stone-furnace", -12.0, 8.0, 0),
                actual: snap("stone-furnace", -12.0, 8.0, 0),
                drift: None,
            },
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains(r#""kind":"placed""#));
        assert_eq!(serde_json::from_str::<MapRecord>(&json).unwrap(), record);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p factorio-bot-core record::map`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement**

```rust
//! What the bots built, and whether the game agreed.
//!
//! Two line kinds share `map.jsonl`. `placed`/`removed` are deltas carrying
//! the map; `keyframe` bounds how far a reconstruction from those deltas can
//! drift. Deltas are exact and tiny; keyframes at delta frequency would be
//! megabytes of repetition.

use serde::{Deserialize, Serialize};

use crate::types::Position;

/// One line of `map.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MapRecord {
    pub tick: u64,
    #[serde(flatten)]
    pub kind: MapKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapKind {
    Placed {
        bot: u32,
        /// What the executor asked for.
        intent: EntitySnapshot,
        /// What the game reports it created.
        actual: EntitySnapshot,
        /// Field names that differ, or null when they agree. Null and empty
        /// would mean the same thing; only one of them is written.
        drift: Option<Vec<String>>,
    },
    Removed {
        bot: u32,
        entity: EntitySnapshot,
    },
    Keyframe {
        bounds: Bounds,
        /// What the game reports inside `bounds`.
        game: Vec<EntitySnapshot>,
        /// What our `EntityGraph` believes is inside `bounds`.
        model: Vec<EntitySnapshot>,
        /// Entities present in exactly one of them.
        divergence: Vec<Divergence>,
    },
    /// A kind this build does not know. Readers skip it; writers never emit it.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EntitySnapshot {
    pub name: String,
    /// Unrounded. A resource sits at a tile centre and stays there.
    pub position: Position,
    /// Factorio 2.0 uses 16 values. For an inserter this is the side it PICKS
    /// UP from, not the side it drops into.
    pub direction: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Bounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Divergence {
    pub entity: EntitySnapshot,
    /// Which side has it: `"game"` or `"model"`.
    pub only_in: String,
}

/// The placement the executor performed, kept on its `Attempt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub intent: EntitySnapshot,
    pub actual: EntitySnapshot,
    pub drift: Option<Vec<String>>,
}

/// Which fields of a placement the game did not honour.
///
/// `None` when they agree. Compares position exactly: the executor asks for a
/// position the game either accepts or snaps, and a tolerance here would hide
/// exactly the snapping we want to see.
pub fn drift_between(intent: &EntitySnapshot, actual: &EntitySnapshot) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    if intent.name != actual.name {
        fields.push("name".to_string());
    }
    if intent.position != actual.position {
        fields.push("position".to_string());
    }
    if intent.direction != actual.direction {
        fields.push("direction".to_string());
    }
    (!fields.is_empty()).then_some(fields)
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p factorio-bot-core record::map`
Expected: PASS, 4 tests.

- [ ] **Step 5: Record the placement in the executor**

Add to `Attempt` in `crates/executor/src/log.rs`:

```rust
    /// What this attempt placed, when it placed anything.
    pub placed: Option<factorio_bot_core::record::map::Placement>,
```

Default it to `None` wherever `Attempt` is constructed.

In `crates/executor/src/rcon_actuator.rs`, at the `place_entity_timed` call around line 309, the returned `FactorioEntity` is the game's truth and the arguments are the intent. Build both snapshots there, compute `drift_between`, and store the `Placement` on the attempt. Nothing else in the executor reads it; it exists to be carried out.

- [ ] **Step 6: Verify the executor still passes its own suite**

Run: `cargo test -p factorio-bot-executor`
Expected: PASS. `replay.rs` constructs `Attempt` values in tests; each needs `placed: None`.

- [ ] **Step 7: Gates and commit**

```bash
cargo fmt
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git add crates/core/src/record/map.rs crates/core/src/record/mod.rs \
        crates/executor/src/log.rs crates/executor/src/rcon_actuator.rs
git commit -m "feat(record): log placements with intent, truth and drift"
```

---

### Task 6: Keyframes and EntityGraph divergence

**Files:**
- Modify: `crates/core/src/record/map.rs`
- Test: inline `#[cfg(test)]` in `crates/core/src/record/map.rs`

**Interfaces:**
- Consumes: `EntitySnapshot`, `Divergence`, `Bounds` from Task 5; `EntityGraph::find_entities_in_radius`.
- Produces: `divergence_between(game: &[EntitySnapshot], model: &[EntitySnapshot]) -> Vec<Divergence>`, `resource_position_from_pos(pos: Pos) -> Position`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn identical_sides_do_not_diverge() {
    let e = vec![snap("stone-furnace", -12.0, 8.0, 0)];
    assert!(divergence_between(&e, &e).is_empty());
}

#[test]
fn an_entity_only_the_game_has_is_reported_as_such() {
    let game = vec![snap("stone-furnace", -12.0, 8.0, 0)];
    assert_eq!(
        divergence_between(&game, &[]),
        vec![Divergence { entity: game[0].clone(), only_in: "game".to_string() }]
    );
}

#[test]
fn an_entity_only_the_model_has_is_reported_as_such() {
    let model = vec![snap("stone-furnace", -12.0, 8.0, 0)];
    assert_eq!(
        divergence_between(&[], &model),
        vec![Divergence { entity: model[0].clone(), only_in: "model".to_string() }]
    );
}

#[test]
fn a_resource_read_out_of_the_graph_keeps_its_tile_centre() {
    // EntityGraph keys resources by Pos(i32, i32), which FLOORS. Reading one
    // back without restoring the half tile puts every resource 0.5 off, and
    // the divergence list becomes pure noise instead of a signal.
    //
    // This exact round trip once made mining fail with "no entity to mine" for
    // every ore on every map, while every test passed -- the test fixture
    // built ore at integer positions, the one input for which the lossy round
    // trip is lossless.
    let pos = Pos(-41, -49);
    assert_eq!(resource_position_from_pos(pos), Position::new(-40.5, -48.5));

    let game = vec![snap("iron-ore", -40.5, -48.5, 0)];
    let model = vec![EntitySnapshot {
        name: "iron-ore".to_string(),
        position: resource_position_from_pos(pos),
        direction: 0,
    }];
    assert!(divergence_between(&game, &model).is_empty());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p factorio-bot-core record::map`
Expected: FAIL — `divergence_between` and `resource_position_from_pos` are not defined.

- [ ] **Step 3: Implement**

```rust
use crate::types::Pos;

/// The position a resource actually occupies, given the floored key
/// `EntityGraph` stores it under.
///
/// Every real resource entity sits at a tile centre -- `(-40.5, -48.5)`, never
/// `(-41, -49)` -- and the mod's `surface.find_entity(name, position)` matches
/// exactly. `Pos(i32, i32)` floors, so reading one back out must add the half
/// tile the key threw away.
pub fn resource_position_from_pos(pos: Pos) -> Position {
    Position::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5)
}

/// Entities present on exactly one side.
///
/// Order is `game`-only first, then `model`-only, each in the order given, so
/// the list is stable across runs and a diff of two runs is readable.
pub fn divergence_between(
    game: &[EntitySnapshot],
    model: &[EntitySnapshot],
) -> Vec<Divergence> {
    let mut out = Vec::new();
    for entity in game {
        if !model.contains(entity) {
            out.push(Divergence { entity: entity.clone(), only_in: "game".to_string() });
        }
    }
    for entity in model {
        if !game.contains(entity) {
            out.push(Divergence { entity: entity.clone(), only_in: "model".to_string() });
        }
    }
    out
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p factorio-bot-core record::map`
Expected: PASS, 8 tests.

- [ ] **Step 5: Emit keyframes**

A keyframe is written at every milestone boundary and every 9,000 ticks (30 force samples, 2.5 minutes), whichever comes first. `bounds` is the bounding box of every entity placed this run plus a 16-tile margin; a run that has placed nothing writes no keyframe rather than a zero-area box.

`game` comes from `EntityGraph::find_entities_in_radius` over that box after a refresh from the game; `model` from the same graph without one. Resources read out of the graph go through `resource_position_from_pos`.

- [ ] **Step 6: Gates and commit**

```bash
cargo fmt
cargo clippy --workspace --all-features --all-targets -- --deny warnings
git add crates/core/src/record/map.rs
git commit -m "feat(record): keyframe the built area and flag model divergence"
```

---

### Task 7: Serve the map, and reconstruct it in the viewer

**Files:**
- Modify: `crates/core/src/record/mod.rs`, `crates/server/src/runs.rs`
- Create: `app/src/lib/runMap.ts`, `app/src/lib/runMap.spec.ts`
- Modify: `app/src/api/types.ts`, `app/src/api/client.ts`

**Interfaces:**
- Consumes: `MapRecord` from Task 5, `Manifest` from Task 3.
- Produces: `Manifest.map: usize`; `GET /api/v1/runs/{id}/map` returning `{"map": [MapRecord]}`; `entitiesAt(records: MapRecord[], tick: number): EntitySnapshot[]`.

- [ ] **Step 1: Write the failing test**

```ts
describe('entitiesAt', () => {
    it('applies placements up to the cursor', () => {
        const r = [placed(100, 'stone-furnace', -12, 8), placed(200, 'stone-furnace', -10, 8)];
        expect(entitiesAt(r, 150)).toHaveLength(1);
        expect(entitiesAt(r, 250)).toHaveLength(2);
    });

    it('drops an entity removed before the cursor', () => {
        const r = [placed(100, 'stone-furnace', -12, 8), removed(200, 'stone-furnace', -12, 8)];
        expect(entitiesAt(r, 250)).toHaveLength(0);
    });

    it('restarts from the latest keyframe at or before the cursor', () => {
        // The keyframe is the truth; replaying deltas from tick zero would
        // carry forward anything the keyframe corrected.
        const r = [
            placed(100, 'stone-furnace', -12, 8),
            keyframe(300, [{name: 'steel-furnace', position: {x: -12, y: 8}, direction: 0}]),
            placed(400, 'stone-furnace', -8, 8)
        ];
        const at = entitiesAt(r, 450);
        expect(at.map((e) => e.name).sort()).toEqual(['steel-furnace', 'stone-furnace']);
    });

    it('is empty before anything was placed', () => {
        expect(entitiesAt([placed(100, 'stone-furnace', -12, 8)], 50)).toEqual([]);
    });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd app && pnpm exec vitest run src/lib/runMap.spec.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `entitiesAt`**

Start from the latest `keyframe` at or before `tick` (using its `game` array, which is the truth), or from empty when there is none, then apply every `placed` and `removed` record strictly after that keyframe and at or before `tick`. Match entities for removal on name and position together — position alone would remove a replaced entity of a different type at the same tile.

- [ ] **Step 4: Run to verify it passes**

Run: `cd app && pnpm exec vitest run src/lib/runMap.spec.ts`
Expected: PASS, 4 tests.

- [ ] **Step 5: Add `Manifest.map`, the route, and the contract**

Mirroring Task 3 exactly: `#[serde(default)] pub map: usize` on `Manifest`, ingestion in `finish`, `get_run_map` in `crates/server/src/runs.rs` returning an empty list for a run that has no `map.jsonl`, then

```bash
UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p factorio-bot-server --features lua --test openapi
```

and the matching `objectContract<T>` declarations in TypeScript.

- [ ] **Step 6: Gates and commit**

```bash
cargo fmt && cargo clippy --workspace --all-features --all-targets -- --deny warnings
cargo test --workspace
cd app && pnpm lint && pnpm run test:coverage
git add crates/core/src/record/mod.rs crates/server/src/runs.rs \
        app/src/lib/runMap.ts app/src/lib/runMap.spec.ts \
        app/src/api/openapi.snapshot.json app/src/api/types.ts \
        app/src/api/client.ts app/src/api/openapi.contract.spec.ts
git commit -m "feat(runs): serve the entity map and reconstruct it at a tick"
```

---

### Task 8: The map panel

**Files:**
- Create: `app/src/components/MapPanel.vue`, `app/src/components/MapPanel.spec.ts`
- Modify: `app/src/store/runsStore.ts`, `app/src/pages/RunsPage.vue`

**Interfaces:**
- Consumes: `entitiesAt` from Task 7, `botSampleAt` from Task 4.
- Produces: `<MapPanel>` taking `entities`, `bots`, `trail` and `bounds` props.

- [ ] **Step 1: Write the failing test**

```ts
describe('MapPanel', () => {
    it('renders a canvas sized to the bounds it is given', () => {
        const wrapper = mount(MapPanel, {
            props: {
                entities: [{name: 'stone-furnace', position: {x: -12, y: 8}, direction: 0}],
                bots: [{id: 1, position: {x: 0, y: 0}}],
                trail: {1: [{x: -1, y: 0}, {x: 0, y: 0}]},
                bounds: {left: -64, top: -64, right: 64, bottom: 64}
            }
        });
        expect(wrapper.find('canvas').exists()).toBe(true);
    });

    it('reports an empty map rather than drawing an empty canvas', () => {
        // A blank canvas and "nothing was built yet" look identical, and one
        // of them is a bug.
        const wrapper = mount(MapPanel, {
            props: {entities: [], bots: [], trail: {}, bounds: null}
        });
        expect(wrapper.text()).toContain('Nothing placed yet');
    });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd app && pnpm exec vitest run src/components/MapPanel.spec.ts`
Expected: FAIL — component not found.

- [ ] **Step 3: Implement the panel**

A `<canvas>` drawn in a `watchEffect` over the props: entities as filled cells coloured by name hash, bot dots on top, and each bot's trail as a polyline of its last 30 seconds (1,800 ticks) of positions. World coordinates map to canvas coordinates through `bounds`, preserving aspect ratio so a wide base is not stretched.

When `bounds` is null, render the text `Nothing placed yet` instead of a canvas.

- [ ] **Step 4: Run to verify it passes**

Run: `cd app && pnpm exec vitest run src/components/MapPanel.spec.ts`
Expected: PASS, 2 tests.

- [ ] **Step 5: Add a `trail` getter and mount the panel**

Add a store getter returning, per bot, the positions from every `bots` sample in the 1,800 ticks up to `cursor`. Mount `<MapPanel>` in `RunsPage.vue` beside the frame panel, driven by the same cursor.

- [ ] **Step 6: Gates and commit**

```bash
cd app && pnpm lint && pnpm run test:coverage && pnpm run build:web
git add app/src/components/MapPanel.vue app/src/components/MapPanel.spec.ts \
        app/src/store/runsStore.ts app/src/store/runsStore.spec.ts \
        app/src/pages/RunsPage.vue
git commit -m "feat(runs): draw the entity map and bot trails at the cursor"
```

---
## Phase 3 — Tier C: planner introspection

### Task 9: Plan DAG, satisfaction reason, structured failure

**Files:**
- Modify: `crates/core/src/record/mod.rs`
- Test: inline `#[cfg(test)]` in `crates/core/src/record/mod.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `PlannedStep`, `SatisfiedReason`, `FailureKind`, `ActionFailure`; `EventKind::PlanCreated` gains `plan: Vec<PlannedStep>`; `EventKind::MilestoneSatisfied` gains `reason: SatisfiedReason`; `EventKind::ActionSettled` gains `failure: Option<ActionFailure>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_milestone_satisfied_without_doing_anything_says_which_kind() {
    // run-1788277287-11819 closed milestone 4 in zero ticks and filed the run
    // as done. Either the bots already had the plates or the planner returned
    // an empty plan, and the record could not tell you which.
    let json = serde_json::to_string(&EventKind::MilestoneSatisfied {
        index: 4,
        iterations: 0,
        reason: SatisfiedReason::PlanEmpty,
    })
    .unwrap();
    assert!(json.contains(r#""reason":"plan_empty""#));
}

#[test]
fn an_old_event_without_a_reason_still_reads() {
    // Every run recorded before this field existed must stay openable.
    let old = r#"{"kind":"milestone_satisfied","index":4,"iterations":0}"#;
    let kind: EventKind = serde_json::from_str(old).unwrap();
    let EventKind::MilestoneSatisfied { reason, .. } = kind else {
        panic!("expected milestone_satisfied");
    };
    assert_eq!(reason, SatisfiedReason::Unknown);
}

#[test]
fn a_plan_carries_its_steps_and_their_dependencies() {
    let kind = EventKind::PlanCreated {
        milestone_index: 1,
        steps: 2,
        makespan: 400,
        bots: vec![1, 2],
        plan: vec![
            PlannedStep {
                id: 0,
                bot: 1,
                action: "mine 10 iron-ore".into(),
                deps: vec![],
                planned_start: 0,
                planned_duration: 300,
            },
            PlannedStep {
                id: 1,
                bot: 2,
                action: "craft iron-gear-wheel".into(),
                deps: vec![0],
                planned_start: 300,
                planned_duration: 100,
            },
        ],
    };
    let round: EventKind = serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();
    assert_eq!(round, kind);
}

#[test]
fn a_failure_keeps_its_string_beside_its_kind() {
    // The string is what a person reads; the kind is what a query groups by.
    // Replacing one with the other loses an audience.
    let failure = ActionFailure {
        kind: FailureKind::MissingItem,
        detail: Some("iron-plate".into()),
    };
    let settled = EventKind::ActionSettled {
        id: 3,
        bot: 1,
        status: "failed".into(),
        elapsed_ticks: Some(120),
        error: Some("not enough iron-plate in inventory".into()),
        failure: Some(failure),
    };
    let json = serde_json::to_string(&settled).unwrap();
    assert!(json.contains("not enough iron-plate"));
    assert!(json.contains(r#""kind":"missing_item""#));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p factorio-bot-core record::`
Expected: FAIL — the fields and types do not exist.

- [ ] **Step 3: Implement**

```rust
/// One scheduled step, as the planner intended it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PlannedStep {
    pub id: u32,
    pub bot: u32,
    pub action: String,
    /// Ids this step waits on.
    pub deps: Vec<u32>,
    /// Ticks from the plan's start, **not** absolute game ticks: a plan is
    /// computed before it is dispatched and does not know its own origin. The
    /// viewer converts planned to observed in exactly one place; this keeps
    /// that true.
    pub planned_start: u64,
    pub planned_duration: u64,
}

/// Why a milestone needed no work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SatisfiedReason {
    /// The world already met the goal.
    AlreadySatisfied,
    /// The planner produced no steps. Indistinguishable from success in the
    /// record until this field existed, and not the same thing at all.
    PlanEmpty,
    /// Recorded before this field existed. Readers must not treat it as
    /// either of the above.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    MissingItem,
    Unreachable,
    Blocked,
    Rejected,
    Timeout,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ActionFailure {
    pub kind: FailureKind,
    pub detail: Option<String>,
}
```

Then on the variants: `PlanCreated` gains `#[serde(default)] pub plan: Vec<PlannedStep>`, `MilestoneSatisfied` gains `#[serde(default = "SatisfiedReason::unknown")] pub reason: SatisfiedReason`, and `ActionSettled` gains `#[serde(default)] pub failure: Option<ActionFailure>`.

`#[serde(default)]` on each is what keeps every run already on disk readable. `SatisfiedReason::Unknown` is a distinct third value rather than defaulting to `AlreadySatisfied`, because guessing which of the two an old run meant is precisely the guess this task exists to remove.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p factorio-bot-core record::`
Expected: PASS.

- [ ] **Step 5: Fix the call sites**

`crates/scripting_lua/src/globals/record.rs` constructs `MilestoneSatisfied` and `ActionSettled`; both need the new fields. Compilation will name each site.

- [ ] **Step 6: Gates and commit**

```bash
cargo fmt && cargo clippy --workspace --all-features --all-targets -- --deny warnings
cargo test --workspace
git add crates/core/src/record/mod.rs crates/scripting_lua/src/globals/record.rs
git commit -m "feat(record): record planner intent, satisfaction reason and failure kind"
```

---

### Task 10: Emit the new fields from the supervisor

**Files:**
- Modify: `crates/scripting_lua/src/globals/record.rs`, `scripts/supervisor.lua`
- Test: existing supervisor tests in `crates/scripting_lua/src/supervisor_lib.rs`

**Interfaces:**
- Consumes: `PlannedStep`, `SatisfiedReason`, `ActionFailure` from Task 9.
- Produces: a `record.plan_created(index, plan)` Lua binding; `record.milestone_satisfied(index, iterations, reason)` gains its third argument.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_milestone_satisfied_by_an_empty_plan_records_that_reason() {
    // The supervisor treats an empty plan as satisfaction. It must say so,
    // because "already done" and "the planner gave up" are the same line
    // otherwise.
    let (lua, _) = harness_with_stub_goal_returning_empty_plan();
    lua.load(include_str!("../../../scripts/supervisor.lua"))
        .exec()
        .unwrap();
    let events = recorded_events(&lua);
    assert_eq!(events[events.len() - 2].reason(), "plan_empty");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p factorio-bot-scripting-lua supervisor`
Expected: FAIL — `record.milestone_satisfied` takes two arguments.

- [ ] **Step 3: Implement**

Add `plan_created` to the `record` table in `globals/record.rs`, taking the milestone index and a Lua array of step tables (`id`, `bot`, `action`, `deps`, `planned_start`, `planned_duration`), and extend `milestone_satisfied` with a reason string parsed into `SatisfiedReason`.

In `scripts/supervisor.lua`, call `record.plan_created(index, plan)` immediately after the planner returns, and pass `"plan_empty"` or `"already_satisfied"` to `milestone_satisfied` based on whether the goal was already met before planning.

**`Option::None` reaches Lua as light userdata and is truthy.** A step field the planner did not set is not absent — guard with `type(x) == "table"` before iterating `deps`.

Tests run the shipped file through `include_str!` in the **sandboxed** interpreter; `clippy.toml` bans `mlua::Lua::new`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p factorio-bot-scripting-lua supervisor`
Expected: PASS.

- [ ] **Step 5: Verify against a live run**

```bash
cargo build --no-default-features --features cli,lua
timeout 180 target/debug/factorio-bot lua goal_smoke.lua --clients 0 --bots 4
grep -c plan_created workspace/runs/*/events.jsonl | tail -1
```

Expected: one `plan_created` per milestone that planned. Zero means the binding is registered but unreached — the defect this whole task exists to fix, and it would recur silently.

- [ ] **Step 6: Commit**

```bash
git add crates/scripting_lua/src/globals/record.rs scripts/supervisor.lua
git commit -m "feat(supervisor): emit the plan DAG and why a milestone was satisfied"
```

---

### Task 11: The analysis view

**Files:**
- Create: `app/src/lib/runDiff.ts`, `app/src/lib/runDiff.spec.ts`, `app/src/pages/RunAnalysisPage.vue`
- Modify: `app/src/router.ts` (or wherever routes are declared), `app/src/pages/RunsPage.vue` (link)

**Interfaces:**
- Consumes: `PlannedStep` and `ActionFailure` from Task 9 via the run detail endpoint; `MapRecord` from Task 7; `Sample` from Task 3.
- Produces: `joinPlanToOutcome(plan, events): OutcomeRow[]` where `OutcomeRow` is `{id, bot, action, plannedDuration, actualDuration, delta, status, failure}`.

- [ ] **Step 1: Write the failing tests**

```ts
describe('joinPlanToOutcome', () => {
    it('pairs a planned step with its settle by id', () => {
        const rows = joinPlanToOutcome(
            [step(0, 1, 'mine 10 iron-ore', 300)],
            [dispatched(0, 1, 61637), settled(0, 1, 'success', 362)]
        );
        expect(rows[0]).toMatchObject({id: 0, plannedDuration: 300, actualDuration: 362, delta: 62});
    });

    it('sorts by overrun, worst first', () => {
        const rows = joinPlanToOutcome(
            [step(0, 1, 'a', 300), step(1, 2, 'b', 100)],
            [settled(0, 1, 'success', 310), settled(1, 2, 'success', 400)]
        );
        expect(rows.map((r) => r.id)).toEqual([1, 0]);
    });

    it('keeps a planned step that never ran, with a null actual', () => {
        // A step that was never dispatched is the most interesting row on the
        // page. Dropping it because it has no settle hides the failure.
        const rows = joinPlanToOutcome([step(0, 1, 'a', 300)], []);
        expect(rows[0].actualDuration).toBeNull();
        expect(rows[0].status).toBe('never dispatched');
    });

    it('keeps a settle with no planned step, with a null planned', () => {
        // Recovery actions are dispatched outside the plan. They belong on the
        // page too, and a join that only walks the plan loses them.
        const rows = joinPlanToOutcome([], [settled(9, 1, 'success', 50)]);
        expect(rows[0].plannedDuration).toBeNull();
    });
});
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd app && pnpm exec vitest run src/lib/runDiff.spec.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the join**

A full outer join on `id`: every planned step and every settle appears exactly once. `delta` is `actualDuration - plannedDuration` when both are present and `null` otherwise; rows sort by `delta` descending with `null` deltas last, so a step that never ran does not sort as if it were on time.

- [ ] **Step 4: Run to verify they pass**

Run: `cd app && pnpm exec vitest run src/lib/runDiff.spec.ts`
Expected: PASS, 4 tests.

- [ ] **Step 5: Build the page**

Route `/runs/:id/analysis`, linked from the run page. Four sections:

1. **Overrun table** — `joinPlanToOutcome` output, worst delta first.
2. **Divergence list** — every keyframe's `divergence` with its tick, each row seeking the map panel to that tick.
3. **Milestone forensics** — per milestone, the plan DAG beside what ran, and the `reason` when it was satisfied with zero iterations.
4. **Inventory at failure** — for each failed action, the settle-triggered bot sample nearest its tick, showing what that bot was carrying.

- [ ] **Step 6: Gates and commit**

```bash
cd app && pnpm lint && pnpm run test:coverage && pnpm run build:web
git add app/src/lib/runDiff.ts app/src/lib/runDiff.spec.ts \
        app/src/pages/RunAnalysisPage.vue app/src/router.ts app/src/pages/RunsPage.vue
git commit -m "feat(runs): add the analysis view joining plan to outcome"
```

---

### Task 12: A live run that exercises all of it

**Files:** none — this task produces a recording, not code.

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Rebuild and re-seed the mod**

```bash
rm -rf workspace/mods
cargo build --release --no-default-features --features cli,lua
```

A release build embeds `mods/` at compile time and extracts once; deleting `workspace/mods` is what makes the extraction happen. Confirm the `Using mods directory <path> (<why>)` line in the run log names the extracted copy.

- [ ] **Step 2: Run with four bots and a research goal**

```bash
timeout 1800 target/release/factorio-bot lua supervisor.lua -c 4
```

Four clients means archive extraction on first use — 8-10 minutes per new client instance, then ~26 s each to load sprites. Budget 20 minutes before the run proper starts if these clients are new.

- [ ] **Step 3: Check every stream is populated**

```bash
RUN=$(ls -td workspace/runs/*/ | head -1)
wc -l "$RUN"/events.jsonl "$RUN"/samples.jsonl "$RUN"/map.jsonl
python3 -c "
import json,collections,sys
for f in ['events','samples','map']:
    ks=collections.Counter(json.loads(l)['kind'] for l in open('$RUN/'+f+'.jsonl'))
    print(f, dict(ks))"
```

Expected: `plan_created` present in `events`; both `bots` and `force` in `samples`; `placed` and `keyframe` in `map`. A zero for any of them names exactly which task did not land.

- [ ] **Step 4: Check the record answers the question that started this**

```bash
python3 -c "
import json
for l in open('$RUN/events.jsonl'):
    e=json.loads(l)
    if e['kind']=='milestone_satisfied': print(e['index'], e['iterations'], e.get('reason'))"
```

Expected: every satisfied milestone names a reason. A milestone satisfied with zero iterations and `plan_empty` is the failure that used to be filed as `done`.

- [ ] **Step 5: Look at both views**

```bash
cd app && pnpm run build:web && cd ..
target/release/factorio-bot serve --bind 127.0.0.1:7500 --web-root app/dist
```

Open `/#/runs`, confirm the map panel draws the entities the run placed and the bot trails move as you scrub, then open the analysis route and confirm the overrun table has rows.

**Never `pkill -f factorio-bot` to stop the server** — the pattern matches the shell running it. Match `pgrep -x factorio-bot` and check `/proc/$p/cmdline` for the port.

- [ ] **Step 6: Commit the findings**

Write what the run showed to `docs/superpowers/notes/2026-09-01-enriched-run-findings.md` and commit it, naming any stream that came back empty and why.

---

## Self-Review

**Spec coverage.** §1 file layout → Tasks 3, 7. §2.1 two beats and the single registration → Task 2 Steps 2-3. §2.2 schema stamp → Task 1 (refusal) and Task 2 Step 1 (emission). §2.3 sample shapes → Tasks 1, 2. §2.4 settle-triggered → Task 2 Step 5. §3.1 ingestion → Tasks 1, 3. §3.2 placement log → Task 5. §3.3 keyframes and the half-tile trap → Task 6. §3.4 planner introspection → Tasks 9, 10. §4.1 video view → Tasks 4, 8. §4.2 analysis view → Task 11. §4.3 contract → Tasks 3, 7. §6 testing → each task's own steps plus Task 12. No gaps.

**Type consistency.** `EntitySnapshot`, `Placement`, `Divergence`, `Bounds` defined in Task 5 and used unchanged in 6, 7, 8. `Sample`/`SampleKind` defined in Task 1, consumed in 3, 4, 11. `PlannedStep` defined in Task 9, consumed in 10 and 11. `SAMPLE_SCHEMA` is `1` in Task 1 and written as `1` in Task 2.

**Known risk carried forward.** Task 2 Step 3 edits the handler that also writes frames. Its Step 6 verifies frames still appear, because a mistake there produces a run with no frames and nothing to explain why.
