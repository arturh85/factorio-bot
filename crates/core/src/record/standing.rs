//! The world as a run's own record says it stood at a tick -- small enough
//! to check in, complete enough to replan against.
//!
//! # Why this exists
//!
//! A t=0 dump has no factory in it, and every offline baseline is taken on
//! one, so the replan path -- the second expansion meeting the first plan's
//! work as map facts -- is invisible to the whole baseline regime. A live
//! run is twenty minutes and a shared box. The record of a *finished* run is
//! neither: `map.jsonl` carries a keyframe of every non-resource entity the
//! model held, and `samples.jsonl` carries where each bot stood and what it
//! held, on a 60-tick beat. Put those on the t=0 dump and the planner meets
//! the world the run's replan met, in seconds, with no game.
//!
//! `run-1788926478-07032`'s refusal was reproduced this way byte for byte on
//! the first attempt (`crates/planner/tests/replan_haul.rs`), from 224
//! entities and four bot positions. This module is that recipe as a type, so
//! the next reproduction is a command rather than a day.
//!
//! # What a snapshot carries, and what it cannot
//!
//! Entities by name, position and direction -- the keyframe's own fields --
//! plus each bot's position and inventory from the nearest `bots` sample at
//! or before the tick, plus the recipe of every crafting machine the nearest
//! `machines` sample saw. **Not** the contents of chests and furnaces, not
//! the force's research, not the ground the run charted after t=0. A
//! keyframe is a rectangle (`bounds`), so an entity outside it is not in the
//! record and is not here either; the count of tiles on which game and model
//! disagreed rides along as `divergences`, because a snapshot taken off a
//! model that had drifted from the game is a snapshot of a belief.
//!
//! # Check it in, or the number dies with the worktree
//!
//! `workspace/runs/` is not in the repository, so a test or a note that
//! quotes a result from a run needs the snapshot in the tree beside it:
//! `crates/planner/tests/fixtures/<run>-tick<T>.json`. At roughly a hundred
//! bytes per entity the base of a twenty-minute run is a few tens of KB;
//! anything under a few thousand entities is fine as text. A snapshot of a
//! world-record base would not be, and is not this tool's job -- that world
//! is a save, and `--resume-from` is for it.

use crate::record::map::{MapKind, read_map};
use crate::record::samples::{SampleKind, read_samples};
use crate::types::Position;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// One entity as the keyframe recorded it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StandingEntity {
    pub name: String,
    pub position: Position,
    pub direction: u8,
}

/// Where a bot stood and what it held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotAt {
    pub id: u8,
    pub position: Position,
    #[serde(default)]
    pub inventory: BTreeMap<String, u32>,
}

/// A recipe a crafting machine had set, by the machine's position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeAt {
    pub position: Position,
    pub recipe: String,
}

/// The world at one tick of one run, as its record says.
///
/// The field names are those of the first hand-built fixture
/// (`run-1788926478-07032-tick56168.json`), so it reads unchanged; the
/// fields added since all default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StandingSnapshot {
    /// The run the record came from, when known.
    #[serde(default)]
    pub run: Option<String>,
    /// The keyframe's tick -- the last one at or before the tick asked for.
    pub keyframe_tick: u64,
    /// The tick of the `bots` sample the positions came from, if any.
    #[serde(default)]
    pub bots_sample_tick: Option<u64>,
    /// How many tiles the keyframe's game and model lists disagreed on. Zero
    /// means the standing world is what the game had; anything else means
    /// this is the model's belief, and says so.
    #[serde(default)]
    pub divergences: usize,
    pub entities: Vec<StandingEntity>,
    #[serde(default)]
    pub bots: Vec<BotAt>,
    #[serde(default)]
    pub recipes: Vec<RecipeAt>,
    /// Technologies to mark researched on the acting force before planning.
    /// Never read off a record -- the record does not carry research -- so
    /// this is a caller's hypothesis, stated: "the world as it stood, with
    /// `electronics` open". It is what lets a leg no archived dump reaches
    /// (an electric arm on a replan with the plant standing) be planned
    /// against a real standing world rather than a fixture.
    #[serde(default)]
    pub researched: Vec<String>,
}

impl StandingSnapshot {
    /// The snapshot of `run_dir` as it stood at `tick`: the last keyframe at
    /// or before it, the last `bots` sample at or before it, and the last
    /// `machines` sample at or before it.
    ///
    /// `keep` says which keyframe entities belong in the snapshot, by name.
    /// A keyframe lists **every** entity in its rectangle, ore tiles
    /// included -- 1,803 of `run-1788926478-07032`'s at tick 56,168 against
    /// 224 that were built -- and the dump already carries the ore, so a
    /// caller with prototypes passes "not a resource" and a snapshot stays
    /// the size of what was built. The predicate is a parameter rather than
    /// a name heuristic because this module has no prototypes and `-ore` is
    /// not what every resource is called.
    ///
    /// `map.jsonl` must exist and hold a keyframe no later than `tick`;
    /// `samples.jsonl` is optional (a run killed before its first sample
    /// still has a keyframe), and its absence leaves `bots` empty and says
    /// so through `bots_sample_tick: None` rather than inventing positions.
    pub fn from_run(run_dir: &Path, tick: u64, keep: impl Fn(&str) -> bool) -> io::Result<Self> {
        let map = read_map(&run_dir.join("map.jsonl"))?;
        let keyframe = map
            .records
            .iter()
            .filter(|r| r.tick <= tick)
            .filter_map(|r| match &r.kind {
                MapKind::Keyframe {
                    model, divergence, ..
                } => Some((r.tick, model, divergence)),
                _ => None,
            })
            .next_back()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "{} has no keyframe at or before tick {tick} ({} records read, {} skipped)",
                        run_dir.join("map.jsonl").display(),
                        map.records.len(),
                        map.skipped
                    ),
                )
            })?;
        let (keyframe_tick, model, divergence) = keyframe;
        let entities = model
            .iter()
            .filter(|e| keep(&e.name))
            .map(|e| StandingEntity {
                name: e.name.clone(),
                position: e.position.clone(),
                direction: e.direction,
            })
            .collect();

        let run = run_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());

        let mut bots = Vec::new();
        let mut bots_sample_tick = None;
        let mut recipes = Vec::new();
        let samples_path = run_dir.join("samples.jsonl");
        if samples_path.exists() {
            let samples = read_samples(&samples_path)?;
            if let Some(sample) = samples
                .samples
                .iter()
                .rfind(|s| s.tick <= tick && matches!(s.kind, SampleKind::Bots { .. }))
                && let SampleKind::Bots { bots: seen } = &sample.kind
            {
                bots_sample_tick = Some(sample.tick);
                for bot in seen {
                    let Ok(id) = u8::try_from(bot.id) else {
                        continue;
                    };
                    bots.push(BotAt {
                        id,
                        position: bot.position.clone(),
                        inventory: bot.inventory.clone(),
                    });
                }
            }
            if let Some(sample) = samples
                .samples
                .iter()
                .rfind(|s| s.tick <= tick && matches!(s.kind, SampleKind::Machines { .. }))
                && let SampleKind::Machines { machines, .. } = &sample.kind
            {
                for machine in machines.values() {
                    if let Some(recipe) = &machine.recipe {
                        recipes.push(RecipeAt {
                            position: machine.position.clone(),
                            recipe: recipe.clone(),
                        });
                    }
                }
            }
        }
        Ok(StandingSnapshot {
            run,
            keyframe_tick,
            bots_sample_tick,
            divergences: divergence.len(),
            entities,
            bots,
            recipes,
            researched: Vec::new(),
        })
    }

    pub fn read_from(path: &Path) -> io::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        serde_json::from_str(&raw).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a standing snapshot: {err}", path.display()),
            )
        })
    }

    pub fn write_to(&self, path: &Path) -> io::Result<()> {
        let raw = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(path, raw)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;

    fn not_ore(name: &str) -> bool {
        !name.ends_with("-ore")
    }

    fn run_dir(map: &str, samples: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("map.jsonl"), map).unwrap();
        if let Some(samples) = samples {
            fs::write(dir.path().join("samples.jsonl"), samples).unwrap();
        }
        dir
    }

    const KEYFRAMES: &str = concat!(
        r#"{"tick":100,"kind":"keyframe","bounds":{"left":0,"top":0,"right":1,"bottom":1},"#,
        r#""game":[{"name":"stone-furnace","position":{"x":2,"y":2},"direction":0}],"#,
        r#""model":[{"name":"stone-furnace","position":{"x":2,"y":2},"direction":0}],"divergence":[]}"#,
        "\n",
        r#"{"tick":300,"kind":"placed","bot":1,"intent":{"name":"x","position":{"x":0,"y":0},"direction":0},"#,
        r#""actual":{"name":"x","position":{"x":0,"y":0},"direction":0},"drift":null}"#,
        "\n",
        r#"{"tick":500,"kind":"keyframe","bounds":{"left":0,"top":0,"right":1,"bottom":1},"#,
        r#""game":[{"name":"stone-furnace","position":{"x":2,"y":2},"direction":0}],"#,
        r#""model":[{"name":"stone-furnace","position":{"x":2,"y":2},"direction":0},"#,
        r#"{"name":"iron-ore","position":{"x":0.5,"y":0.5},"direction":0},"#,
        r#"{"name":"iron-chest","position":{"x":4.5,"y":4.5},"direction":0}],"#,
        r#""divergence":[{"entity":{"name":"iron-chest","position":{"x":4.5,"y":4.5},"direction":0},"only_in":"model"}]}"#,
        "\n",
    );

    const SAMPLES: &str = concat!(
        r#"{"schema":3,"tick":120,"kind":"bots","bots":[{"id":1,"position":{"x":1.5,"y":1.5},"#,
        r#""inventory":{"iron-plate":3},"crafting_queue":0,"mining":null}]}"#,
        "\n",
        r#"{"schema":3,"tick":480,"kind":"bots","bots":[{"id":1,"position":{"x":9.5,"y":9.5},"#,
        r#""inventory":{"iron-plate":7},"crafting_queue":0,"mining":null}]}"#,
        "\n",
        r#"{"schema":3,"tick":480,"kind":"machines","machines":{"7":{"name":"assembling-machine-1","#,
        r#""type":"assembling-machine","position":{"x":6.5,"y":6.5},"recipe":"iron-gear-wheel","#,
        r#""input":{},"output":{},"fuel":{}}},"truncated":0}"#,
        "\n",
    );

    /// The LAST keyframe at or before the tick, not the first, and the bots
    /// sample nearest below it.
    #[test]
    fn the_snapshot_is_the_last_keyframe_and_sample_at_or_before_the_tick() {
        let dir = run_dir(KEYFRAMES, Some(SAMPLES));
        let snap = StandingSnapshot::from_run(dir.path(), 500, not_ore).unwrap();
        assert_eq!(snap.keyframe_tick, 500);
        assert_eq!(
            snap.entities.len(),
            2,
            "the model list, which is what the planner saw"
        );
        assert_eq!(
            snap.divergences, 1,
            "and the disagreement is counted, not hidden"
        );
        assert_eq!(snap.bots_sample_tick, Some(480));
        assert_eq!(snap.bots[0].position, Position::new(9.5, 9.5));
        assert_eq!(snap.bots[0].inventory.get("iron-plate"), Some(&7));
        assert_eq!(snap.recipes.len(), 1);
        assert_eq!(snap.recipes[0].recipe, "iron-gear-wheel");

        let earlier = StandingSnapshot::from_run(dir.path(), 250, not_ore).unwrap();
        assert_eq!(earlier.keyframe_tick, 100);
        assert_eq!(earlier.entities.len(), 1);
        assert_eq!(earlier.bots_sample_tick, Some(120));
        assert_eq!(earlier.bots[0].inventory.get("iron-plate"), Some(&3));
    }

    /// A tick before the first keyframe is a named refusal, not an empty
    /// world that looks like t=0.
    #[test]
    fn a_tick_before_any_keyframe_is_refused_by_name() {
        let dir = run_dir(KEYFRAMES, None);
        let err = StandingSnapshot::from_run(dir.path(), 50, not_ore).unwrap_err();
        assert!(
            err.to_string().contains("no keyframe at or before tick 50"),
            "{err}"
        );
    }

    /// No samples file: entities still come through, and the missing bots
    /// are said to be missing.
    #[test]
    fn a_run_without_samples_has_entities_and_no_invented_bots() {
        let dir = run_dir(KEYFRAMES, None);
        let snap = StandingSnapshot::from_run(dir.path(), 1_000, not_ore).unwrap();
        assert_eq!(snap.entities.len(), 2);
        assert!(snap.bots.is_empty());
        assert_eq!(snap.bots_sample_tick, None);
    }

    /// Round trip through the file form the fixtures use.
    #[test]
    fn a_snapshot_reads_back_what_it_wrote() {
        let dir = run_dir(KEYFRAMES, Some(SAMPLES));
        let snap = StandingSnapshot::from_run(dir.path(), 500, not_ore).unwrap();
        let path = dir.path().join("snap.json");
        snap.write_to(&path).unwrap();
        assert_eq!(StandingSnapshot::read_from(&path).unwrap(), snap);
    }

    /// The first hand-built fixture predates `divergences` and `recipes`,
    /// and must keep reading.
    #[test]
    fn the_original_fixture_shape_still_parses() {
        let raw = r#"{"run":"run-x","keyframe_tick":56168,"bots_sample_tick":56160,
            "entities":[{"name":"stone-furnace","position":{"x":-26.0,"y":-13.0},"direction":0}],
            "bots":[{"id":1,"position":{"x":1.0,"y":2.0},"inventory":{"coal":5}}]}"#;
        let snap: StandingSnapshot = serde_json::from_str(raw).unwrap();
        assert_eq!(snap.entities.len(), 1);
        assert_eq!(snap.bots[0].id, 1);
        assert_eq!(snap.divergences, 0);
        assert!(snap.recipes.is_empty());
    }
}
