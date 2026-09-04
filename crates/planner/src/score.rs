//! How good a *starting map* is, scored off a dumped world.
//!
//! This is workstream 0b of
//! `docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md`. **Walking was
//! 20.3% of the reference run** — 14,330 ticks, 3.98 minutes, for bot 1 alone,
//! against a target of researching `automation` in under nine minutes and a
//! single-player world record of 6:12. Four minutes of walking would eat two
//! thirds of that budget, so no planner improvement compensates for a spawn
//! whose ore is far away, and a comparison against a world-record time is
//! meaningless until the map is a fair one.
//!
//! # What it measures, and what it deliberately does not
//!
//! The cheap tier: **how far spawn is from the nearest charted tile of each
//! thing rung 1 needs**, priced in the planner's own currency — ticks of
//! walking at [`WALK_TILES_PER_TICK`](crate::WALK_TILES_PER_TICK) — so the
//! number is directly comparable with a makespan rather than being a distance
//! nobody can convert.
//!
//! The expensive tier is not here: it is `expand()` + `schedule()` +
//! [`PlanReport`](crate::PlanReport), which prices the actual plan instead of
//! this proxy and which the `score-map` CLI runs alongside this. Both are
//! wanted. A makespan is strictly the better number and it is also the one
//! that fails outright on a map the planner refuses, at which point the
//! distances below are what say *why*.
//!
//! # Charting bounds every number here, and that is not a caveat to skip
//!
//! `EntityGraph`'s `resources` and `tile_tree` hold what the game has
//! **charted**, not what exists. A world dumped before the bots explored
//! therefore scores what has been *seen*. Two consequences, both load-bearing:
//!
//! * A resource with no charted tile is **not charted**, which is not the same
//!   claim as **not there**. [`Verdict::Incomplete`] says the former and never
//!   the latter, and [`MapScore::charted_extent`] is reported beside it so a
//!   reader can tell "this map has no coal" from "nobody has looked yet".
//! * A distance is an **upper bound on the true distance**. Charting only
//!   grows, and a tile discovered later can only be nearer or further; a nearer
//!   one would improve the score. So a map that scores *well* here scores at
//!   least that well in truth, while a map that scores badly may only be
//!   under-explored. **The asymmetry runs the same way as
//!   `EntityGraph::resource_fingerprint`'s**, and for the same reason.
//!
//! This is why scoring is worth doing on a dump taken **after** a run has
//! charted its surroundings, or on one taken at t=0 with the charted extent
//! read off the report rather than assumed.
//!
//! # Determinism
//!
//! `crates/planner` is pure: no I/O, no async, no wall clock, ordered
//! collections only, floats compared with `total_cmp`. Everything here obeys
//! that — nearest-tile selection breaks ties on `(distance, x, y)`, exactly as
//! `EntityGraph::nearest_water_tile` and `PlanState::nearest_supply_anchor`
//! do, so two scorings of one world agree tile for tile. Reading a file and
//! printing a table is the CLI's job.

use crate::method::power::{PLANT_WATER_SCAN_RADIUS, PLANT_WATER_WIDE_SCAN_RADIUS};
use crate::schedule::WALK_TILES_PER_TICK;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::{Position, Rect};
use serde::{Deserialize, Serialize};

/// The ore rung 1 puts on the ground, in the order a report lists it.
///
/// **Verified against the planner rather than assumed.** The planner has no
/// list of resource names at all — every lookup is name-driven off the recipe
/// chain into `PlanState::resource_patches(item)` — so these four were read
/// back off what `researched:automation` actually demands from an empty start:
/// iron and copper smelt into the plates the science pack and every
/// intermediate need; coal fuels the furnaces, the drills and the boiler (and
/// is the one raw name the planner does spell literally, in
/// `method::power`'s plant bill); stone is the furnaces themselves, and is
/// also inside the boiler and the burner mining drill.
///
/// Wood is a requirement too, but not a *map* requirement — see
/// [`MapScore::wood`]. Nothing else raw is on the path: no crude oil, no
/// uranium, and rocks are never named by the planner.
pub const RUNG_1_ORES: [&str; 4] = ["iron-ore", "copper-ore", "coal", "stone"];

/// The item a tree yields, and the only thing trees are wanted for.
const WOOD: &str = "wood";

/// How far out a resource is looked for by default, in tiles.
///
/// Not a planner bound — the planner has none for ore, and mining anywhere on
/// the map is legal. It is the radius **this report calls "close to spawn"**,
/// and it is set at twice the water bound so that water is the tighter of the
/// two constraints and stays the one that disqualifies a map.
///
/// At [`WALK_TILES_PER_TICK`] a one-way walk of 256 tiles is 1,707 ticks
/// (28 seconds), and rung 1 walks to ore repeatedly. A map whose iron is out
/// here is already a bad map; the radius exists so that a report says
/// "nothing within 256 tiles" instead of scanning an entire charted continent
/// to name a tile no plan would ever use.
pub const DEFAULT_SEARCH_RADIUS: f64 = 256.;

/// Ticks of walking to cover `tiles`, at the planner's own walking speed.
///
/// Rounded up, matching [`crate::travel_ticks`]: a walk that takes part of a
/// tick takes the tick.
#[must_use]
pub fn walk_ticks(tiles: f64) -> u64 {
    (tiles / WALK_TILES_PER_TICK).ceil().max(0.) as u64
}

/// One resource, and how far spawn is from the nearest charted tile of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceScore {
    pub name: String,
    /// The nearest charted tile, or `None` when none was charted inside the
    /// search radius. **`None` means "not charted within the radius", never
    /// "absent from the map"** — see the module documentation.
    pub nearest: Option<Position>,
    /// Distance from the origin to [`ResourceScore::nearest`], in tiles,
    /// Euclidean (`calculate_distance`, not `Position::distance`, which is
    /// Manhattan despite the name).
    pub distance: Option<f64>,
    /// [`ResourceScore::distance`] priced as one-way walking.
    pub walk_ticks: Option<u64>,
    /// How many tiles of this resource are charted **anywhere in the dump**,
    /// not merely inside the radius. A large count with no nearest tile is a
    /// map whose ore is all far away; a zero is a map nobody has charted this
    /// resource on at all.
    ///
    /// Tiles and not patches: `PlanState::resource_patches` documents its own
    /// flood fill as partitioning one contiguous field into two or three
    /// patches non-deterministically *within a single process*, so a patch
    /// count is not a number two scorings agree on.
    pub charted_tiles: usize,
}

/// Water, which the run needs and which the planner will refuse without.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaterScore {
    /// The tile's top-left corner, as `FactorioTile::position` reports it.
    pub nearest: Option<Position>,
    pub distance: Option<f64>,
    pub walk_ticks: Option<u64>,
    /// Whether the nearest water is inside `plan_plant`'s cheap scan
    /// ([`PLANT_WATER_SCAN_RADIUS`], 64 tiles). Outside it the planner still
    /// builds, from the wide scan; this is "a good shoreline", not a bound.
    pub within_cheap_scan: bool,
    /// Whether the nearest water is inside the planner's **wide** scan
    /// ([`PLANT_WATER_WIDE_SCAN_RADIUS`], 128 tiles). Outside it,
    /// `plan_plant` raises `PlannerError::PowerPlantNeedsWater` and the goal
    /// does not expand at all — the lab never gets power, so `automation` is
    /// not researched.
    ///
    /// Measured here from the origin; the planner measures from the plant
    /// site, which it puts near the work rather than at spawn. So this is an
    /// indicator and not a proof, in both directions.
    pub within_planner_reach: bool,
}

/// Trees: a requirement of the *plan*, but not of the *map*.
///
/// Rung 1 needs exactly one wood, for the `small-electric-pole` in
/// `method::power`'s plant bill, and every bot starts the run holding one —
/// "of which a four-bot run has exactly four and can make no more". So a map
/// with no tree in sight still researches automation, and a tree is only the
/// fallback for a run that has spent its starting wood.
///
/// It is reported because the owner wants wood gathered during idle time
/// (workstream A's filler work), and because "no trees anywhere near spawn" is
/// a thing worth knowing about a map before choosing it. It never contributes
/// to [`MapScore::walk_score`] and never makes a verdict
/// [`Verdict::Incomplete`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WoodScore {
    pub nearest: Option<Position>,
    pub distance: Option<f64>,
    pub walk_ticks: Option<u64>,
    /// Standing trees charted anywhere in the dump that yield wood.
    pub charted_sources: usize,
}

/// Where the tile tree runs out around the origin. Defined on
/// [`crate::state`] since the charting query moved onto `PlanState`; the doc
/// there says what a probe is and what a blind one means. This report reads
/// it for one reason `state` does not spell out: an incomplete disc makes
/// every "nearest" below a **lower bound**, and the verdict says so.
///
/// Two things it still cannot repair: `control.lua:1311` drops any chunk
/// outside `[-512, 512]` permanently, and the discovery pass has been observed
/// returning `{}` entities for 54 of 419 chunks whose real contents arrived
/// thousands of ticks later (`docs/superpowers/notes/2026-09-02-resource-double-count.md`).
/// So even a fully covered disc gives a resource census that is a **lower
/// bound**.
pub use crate::state::ChartingScore;

/// What the report concludes, and how strongly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// Every rung-1 requirement has a charted source inside the radius, and
    /// water is inside the planner's own reach.
    Viable,
    /// Something rung 1 needs was not found. **This is a statement about the
    /// dump, not about the map**: the named things were not charted within the
    /// search radius, which a map that genuinely lacks them and a map nobody
    /// has explored both produce.
    Incomplete { missing: Vec<String> },
}

impl Verdict {
    #[must_use]
    pub fn is_viable(&self) -> bool {
        matches!(self, Verdict::Viable)
    }
}

/// A starting map, scored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapScore {
    /// Where distances were measured from. Factorio's map spawn is the origin
    /// unless the caller says otherwise, and a dump carries no spawn point of
    /// its own — see [`MapScore::of`].
    pub origin: Position,
    pub radius: f64,
    /// One entry per name in [`RUNG_1_ORES`], in that order.
    pub resources: Vec<ResourceScore>,
    pub water: WaterScore,
    pub wood: WoodScore,
    /// The sum of the one-way walk to each rung-1 requirement, water included:
    /// **the score**, in ticks, lower being better. `None` when anything is
    /// missing, because a sum over a subset would rank an incomplete map above
    /// a complete one.
    ///
    /// It is a proxy and it is a *loose* one — rung 1 walks to each resource
    /// more than once and walks between them, not out and back from spawn each
    /// time. Its job is to separate a map with everything at 30 tiles from one
    /// with iron at 200, which it does; ranking two similar maps is the
    /// makespan's job.
    pub walk_score: Option<u64>,
    /// The axis-aligned box containing every charted resource tile of any
    /// name, or `None` when nothing at all is charted.
    ///
    /// **This is what bounds every other number in the report.** It is the
    /// region the dump has knowledge of, so a missing resource outside it is
    /// unexplored rather than absent. Resource tiles rather than terrain tiles
    /// because it is resources the verdict is about, and because a
    /// resource-free charted chunk cannot make a resource appear.
    pub charted_extent: Option<Rect>,
    /// Every resource name charted in the dump with its tile count, rung-1 or
    /// not — the same census `EntityGraph::resource_fingerprint` reports. A
    /// map with `uranium-ore` and no `coal` is visible here and nowhere else.
    pub charted: std::collections::BTreeMap<String, usize>,
    /// How much of the search disc the dump has terrain for. Read this before
    /// believing anything above it — see [`ChartingScore`].
    pub charting: ChartingScore,
    /// The dump's resource digest, for saying two dumps are the same map.
    /// Equal digests mean the same map; **unequal means unknown**, because
    /// charting grows. `None` when nothing is charted.
    pub fingerprint: Option<String>,
    pub verdict: Verdict,
}

impl MapScore {
    /// Scores the world behind `state`, measuring from `origin`.
    ///
    /// # Why the origin is an argument and not read off the world
    ///
    /// **A `FactorioWorld` carries no spawn point.** Nothing in the dump says
    /// where the game would put a fresh character, so a scorer that "found"
    /// one would be inventing it. The caller passes Factorio's map origin
    /// `(0, 0)` for a fresh map, or a player's position, or the site a run
    /// actually worked from — and the answer says which, because the origin is
    /// in the report.
    ///
    /// This matters most for a **mid-run dump**, where every player has walked
    /// away from spawn and using a player position would score the map from
    /// wherever bot 1 happened to be standing when the dump was written.
    #[must_use]
    pub fn of(state: &PlanState, origin: &Position, radius: f64) -> MapScore {
        let graph = &state.base().entity_graph;
        let fingerprint = graph.resource_fingerprint();
        let charted = fingerprint
            .as_ref()
            .map(|print| print.tiles.clone())
            .unwrap_or_default();

        let mut extent: Option<Rect> = None;
        let mut resources = Vec::with_capacity(RUNG_1_ORES.len());
        for name in RUNG_1_ORES {
            let mut nearest: Option<(f64, Position)> = None;
            for patch in state.resource_patches(name) {
                for tile in &patch.elements {
                    extent = Some(match extent {
                        None => Rect::new(tile, tile),
                        Some(box_so_far) => grow(&box_so_far, tile),
                    });
                    let distance = calculate_distance(tile, origin);
                    if distance.total_cmp(&radius).is_gt() {
                        continue;
                    }
                    let better = match &nearest {
                        None => true,
                        Some((best, at)) => distance
                            .total_cmp(best)
                            .then(tile.x.total_cmp(&at.x))
                            .then(tile.y.total_cmp(&at.y))
                            .is_lt(),
                    };
                    if better {
                        nearest = Some((distance, tile.clone()));
                    }
                }
            }
            resources.push(ResourceScore {
                name: name.to_owned(),
                nearest: nearest.as_ref().map(|(_, at)| at.clone()),
                distance: nearest.as_ref().map(|(distance, _)| *distance),
                walk_ticks: nearest.as_ref().map(|(distance, _)| walk_ticks(*distance)),
                charted_tiles: charted.get(name).copied().unwrap_or(0),
            });
        }

        // `nearest_water_tile` measures to the tile's *centre* and returns the
        // tile with its corner position untouched — the half-tile offset lives
        // in exactly that one place, and re-deriving a distance from the
        // returned corner here would put every shoreline 0.7 tiles nearer than
        // the planner believes it is. So the distance is taken the same way.
        let water_tile = state.nearest_water_tile(origin, radius);
        let water_distance = water_tile.as_ref().map(|tile| {
            calculate_distance(
                &Position::new(tile.position.x() + 0.5, tile.position.y() + 0.5),
                origin,
            )
        });
        let water = WaterScore {
            nearest: water_tile.as_ref().map(|tile| tile.position.clone()),
            distance: water_distance,
            walk_ticks: water_distance.map(walk_ticks),
            within_cheap_scan: water_distance
                .is_some_and(|distance| distance.total_cmp(&PLANT_WATER_SCAN_RADIUS).is_le()),
            within_planner_reach: water_distance
                .is_some_and(|distance| distance.total_cmp(&PLANT_WATER_WIDE_SCAN_RADIUS).is_le()),
        };

        let sources = state.minable_sources(WOOD);
        let mut nearest_wood: Option<(f64, Position)> = None;
        for (_, at, _) in &sources {
            let distance = calculate_distance(at, origin);
            if distance.total_cmp(&radius).is_gt() {
                continue;
            }
            let better = match &nearest_wood {
                None => true,
                Some((best, best_at)) => distance
                    .total_cmp(best)
                    .then(at.x.total_cmp(&best_at.x))
                    .then(at.y.total_cmp(&best_at.y))
                    .is_lt(),
            };
            if better {
                nearest_wood = Some((distance, at.clone()));
            }
        }
        let wood = WoodScore {
            nearest: nearest_wood.as_ref().map(|(_, at)| at.clone()),
            distance: nearest_wood.as_ref().map(|(distance, _)| *distance),
            walk_ticks: nearest_wood
                .as_ref()
                .map(|(distance, _)| walk_ticks(*distance)),
            charted_sources: sources.len(),
        };

        let charting = state.charting(origin, radius);

        let mut missing: Vec<String> = resources
            .iter()
            .filter(|entry| entry.nearest.is_none())
            .map(|entry| entry.name.clone())
            .collect();
        // Water short of the planner's own wide scan is reported as missing
        // even though a tile was found, because a plant it cannot reach is a
        // plant that does not get built: `plan_plant` raises
        // `PowerPlantNeedsWater` and the whole goal fails to expand. "Found,
        // but too far to use" and "not found" are the same outcome for a run.
        if !water.within_planner_reach {
            missing.push("water".to_owned());
        }
        let walk_score = if missing.is_empty() {
            let ore: u64 = resources
                .iter()
                .filter_map(|entry| entry.walk_ticks)
                .sum::<u64>();
            water.walk_ticks.map(|ticks| ore + ticks)
        } else {
            None
        };
        let verdict = if missing.is_empty() {
            Verdict::Viable
        } else {
            Verdict::Incomplete { missing }
        };

        MapScore {
            origin: origin.clone(),
            radius,
            resources,
            water,
            wood,
            walk_score,
            charted_extent: extent,
            charted,
            charting,
            fingerprint: fingerprint.map(|print| print.digest),
            verdict,
        }
    }

    /// The report as lines a person reads, one per line, no trailing newline.
    ///
    /// Same contract as [`PlanReport::lines`](crate::PlanReport::lines), and
    /// here for the same reason: the shape is testable without a CLI, and
    /// every caller prints the same one.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!(
                "origin         ({:.1}, {:.1})   radius {:.0} tiles",
                self.origin.x(),
                self.origin.y(),
                self.radius
            ),
            "resource       distance  walk    charted tiles".to_string(),
        ];
        for entry in &self.resources {
            out.push(match (entry.distance, entry.walk_ticks) {
                (Some(distance), Some(ticks)) => format!(
                    "{:<14} {distance:>7.1}  {ticks:>5}   {}",
                    entry.name, entry.charted_tiles
                ),
                _ => format!(
                    "{:<14} {:>7}  {:>5}   {}",
                    entry.name, "none", "-", entry.charted_tiles
                ),
            });
        }
        out.push(match (self.water.distance, self.water.walk_ticks) {
            (Some(distance), Some(ticks)) => format!(
                "{:<14} {distance:>7.1}  {ticks:>5}   {}",
                "water",
                if self.water.within_cheap_scan {
                    "inside the plant's cheap scan (64)"
                } else if self.water.within_planner_reach {
                    "inside the planner's wide scan (128)"
                } else {
                    "TOO FAR: plan_plant refuses beyond 128"
                }
            ),
            _ => format!("{:<14} {:>7}  {:>5}   none charted", "water", "none", "-"),
        });
        out.push(match (self.wood.distance, self.wood.walk_ticks) {
            (Some(distance), Some(ticks)) => format!(
                "{:<14} {distance:>7.1}  {ticks:>5}   {} standing (bonus)",
                "wood", self.wood.charted_sources
            ),
            _ => format!(
                "{:<14} {:>7}  {:>5}   none charted (bonus, not required)",
                "wood", "none", "-"
            ),
        });
        out.push(match self.walk_score {
            Some(score) => format!(
                "walk score     {score} ticks ({}) -- lower is better",
                crate::render::ticks_to_timestamp(score.min(u64::from(u32::MAX)) as u32)
            ),
            None => "walk score     n/a -- something rung 1 needs was not found".to_string(),
        });
        out.push(match &self.charted_extent {
            Some(extent) => format!(
                "charted        resources span x {:.0}..{:.0}, y {:.0}..{:.0}",
                extent.left_top.x(),
                extent.right_bottom.x(),
                extent.left_top.y(),
                extent.right_bottom.y()
            ),
            None => "charted        nothing -- this dump has no resource tiles at all".to_string(),
        });
        out.push(if self.charting.is_complete() {
            format!(
                "charting       {}/{} probes on charted ground -- the search disc is covered",
                self.charting.covered, self.charting.probes
            )
        } else {
            format!(
                "charting       {}/{} probes on charted ground -- READ THIS FIRST: {} \
                 direction(s) of the search disc are unexplored, so a resource \
                 missing below may simply not have been looked at",
                self.charting.covered,
                self.charting.probes,
                self.charting.blind.len()
            )
        });
        if let Some(digest) = &self.fingerprint {
            out.push(format!("fingerprint    {digest}"));
        }
        match &self.verdict {
            Verdict::Viable => out.push("verdict        VIABLE for rung 1".to_string()),
            Verdict::Incomplete { missing } => {
                out.push(format!(
                    "verdict        INCOMPLETE -- not charted within the radius: {}",
                    missing.join(", ")
                ));
                out.push(
                    "               (that is a fact about this dump, not about the map: \
                     charting grows as bots explore)"
                        .to_string(),
                );
            }
        }
        out
    }
}

/// `box_so_far` widened to contain `point`.
///
/// `Rect::new` normalises nothing, so growing is done explicitly rather than
/// by handing it two corners and hoping they are in the right order.
fn grow(box_so_far: &Rect, point: &Position) -> Rect {
    Rect::new(
        &Position::new(
            box_so_far.left_top.x().min(point.x()),
            box_so_far.left_top.y().min(point.y()),
        ),
        &Position::new(
            box_so_far.right_bottom.x().max(point.x()),
            box_so_far.right_bottom.y().max(point.y()),
        ),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use factorio_bot_core::factorio::world::FactorioWorld;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn score_of(world: FactorioWorld, origin: Position) -> MapScore {
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        MapScore::of(&state, &origin, DEFAULT_SEARCH_RADIUS)
    }

    fn fixture_score() -> MapScore {
        score_of(fixture_world(), Position::new(0., 0.))
    }

    /// The shared fixture has all four ores, water and trees, so it is the
    /// control for "a viable map scores viable".
    #[test]
    fn a_map_with_everything_is_viable() {
        let score = fixture_score();
        assert_eq!(score.verdict, Verdict::Viable, "{:#?}", score.verdict);
        assert!(score.walk_score.is_some());
        for entry in &score.resources {
            assert!(
                entry.distance.is_some(),
                "{} was not found on a map that has it",
                entry.name
            );
            assert!(entry.charted_tiles > 0, "{} charted nothing", entry.name);
        }
        // Wood is deliberately *not* asserted here. The shared fixture's 100
        // trees are `tree-42`, a name its prototype table has no entry for, so
        // `minables_yielding("wood")` reads no `mine_result` off them and the
        // model has no wood source at all. That is a fixture quirk rather than
        // a scoring one -- and it is the reason wood must never be able to
        // turn a verdict, which this asserts by scoring `Viable` anyway.
        assert_eq!(score.wood.charted_sources, 0);
    }

    /// Trees are found and priced when the map has ones the planner can read.
    ///
    /// `tree-01` is the name whose prototype carries `mine_result {wood: 4}`;
    /// see `test_world::with_trees` for why the shared fixture's trees do not.
    #[test]
    fn trees_are_reported_when_the_prototypes_say_they_yield_wood() {
        let world = crate::test_world::with_trees(
            fixture_world(),
            &[Position::new(12., 0.), Position::new(30., 0.)],
        );
        let score = score_of(world, Position::new(0., 0.));
        assert_eq!(score.wood.charted_sources, 2);
        assert_eq!(score.wood.nearest, Some(Position::new(12., 0.)));
        assert_eq!(score.wood.walk_ticks, Some(walk_ticks(12.)));
        assert!(
            score.verdict.is_viable(),
            "trees are a bonus and must not change a verdict"
        );
    }

    /// The whole point of the scorer: a nearer map scores lower.
    ///
    /// Asserted by moving the *origin* rather than the ore, because moving ore
    /// would change the map and this must show the score tracks distance and
    /// not tile count. The fixture's copper sits around `(-40, 0)`, so
    /// measuring from `(-40, 0)` is measuring from on top of it.
    #[test]
    fn a_spawn_nearer_the_ore_scores_lower() {
        let far = fixture_score();
        let near = score_of(fixture_world(), Position::new(-40., 0.));
        let far_score = far.walk_score.expect("the fixture is viable");
        let near_score = near.walk_score.expect("still viable from the ore field");
        assert!(
            near_score < far_score,
            "standing on the copper scored {near_score}, spawn scored {far_score}"
        );
    }

    /// Distance is priced in the planner's own currency, not in tiles.
    #[test]
    fn a_distance_is_priced_as_walking_at_the_planners_own_speed() {
        assert_eq!(walk_ticks(0.), 0);
        assert_eq!(walk_ticks(15.), 100, "0.15 tiles a tick");
        assert_eq!(walk_ticks(0.01), 1, "part of a tick is a tick");
        let score = fixture_score();
        for entry in &score.resources {
            let (distance, ticks) = (entry.distance.unwrap(), entry.walk_ticks.unwrap());
            assert_eq!(ticks, walk_ticks(distance), "{} disagrees", entry.name);
        }
    }

    /// A map with no water is refused, and named as such.
    ///
    /// Not a style point: `plan_plant` raises `PowerPlantNeedsWater` when the
    /// wide scan finds nothing, so the goal does not expand and no makespan
    /// exists to compare. The distance report is the only thing that says why.
    #[test]
    fn a_map_with_no_water_is_incomplete_and_says_water() {
        let world = FactorioWorld::new();
        let score = score_of(world, Position::new(0., 0.));
        match &score.verdict {
            Verdict::Incomplete { missing } => {
                assert!(missing.contains(&"water".to_owned()), "{missing:?}");
                for ore in RUNG_1_ORES {
                    assert!(missing.contains(&ore.to_owned()), "{missing:?}");
                }
            }
            other => panic!("an empty world scored {other:?}"),
        }
        assert_eq!(score.walk_score, None, "an incomplete map has no score");
        assert_eq!(score.charted_extent, None);
        assert_eq!(score.fingerprint, None);
    }

    /// Water beyond the planner's wide scan is missing even though it exists.
    ///
    /// The fixture's water is at `(40, 40)`, ~57 tiles from the origin. Scored
    /// from far enough away it is past 128 tiles, which is exactly the
    /// distinction that matters: the tile is charted, and the plant still
    /// cannot be built.
    #[test]
    fn water_the_planner_cannot_reach_counts_as_missing() {
        let score = score_of(fixture_world(), Position::new(240., 40.));
        assert!(
            score.water.nearest.is_some(),
            "the water is charted and inside the search radius"
        );
        assert!(!score.water.within_planner_reach);
        assert!(!score.water.within_cheap_scan);
        match &score.verdict {
            Verdict::Incomplete { missing } => assert!(missing.contains(&"water".to_owned())),
            other => panic!("unreachable water scored {other:?}"),
        }
    }

    /// The search radius bounds the answer, and shrinking it hides ore.
    #[test]
    fn the_radius_is_what_close_to_spawn_means() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let tight = MapScore::of(&state, &Position::new(0., 0.), 10.);
        assert!(!tight.verdict.is_viable(), "nothing is within 10 tiles");
        for entry in &tight.resources {
            assert!(entry.nearest.is_none(), "{} is not within 10", entry.name);
            assert!(
                entry.charted_tiles > 0,
                "{} is charted even though it is out of range -- that \
                 distinction is the point",
                entry.name
            );
        }
    }

    /// Two scorings of one world agree exactly, floats included.
    ///
    /// The planner's determinism rule, asserted rather than assumed: the
    /// resource maps behind this are `DashMap`s and the flood fill that
    /// partitions them is documented as unstable *within* a process, so
    /// nearest-tile selection has to be order-independent to survive it.
    #[test]
    fn scoring_one_world_twice_gives_the_same_answer() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let once = MapScore::of(&state, &Position::new(0., 0.), DEFAULT_SEARCH_RADIUS);
        let twice = MapScore::of(&state, &Position::new(0., 0.), DEFAULT_SEARCH_RADIUS);
        assert_eq!(once, twice);
        assert_eq!(once.lines(), twice.lines());
    }

    /// An incomplete verdict must not read as a claim about the map.
    #[test]
    fn an_incomplete_report_says_it_is_about_the_dump() {
        let lines = score_of(FactorioWorld::new(), Position::new(0., 0.)).lines();
        let rendered = lines.join("\n");
        assert!(rendered.contains("INCOMPLETE"), "{rendered}");
        assert!(
            rendered.contains("charting grows"),
            "a reader must be told what a miss means: {rendered}"
        );
    }

    /// The charting probe is what stops a blind report reading as a verdict.
    ///
    /// The shared fixture charts a 4x4 block of water at `(40, 40)` and
    /// nothing else, so almost every probe misses — which is the finding this
    /// must surface loudly, not something to smooth over.
    #[test]
    fn a_barely_charted_dump_says_so_before_it_says_anything_else() {
        let score = fixture_score();
        assert_eq!(score.charting.probes, 17, "origin plus 8 dirs at 2 radii");
        assert!(
            !score.charting.is_complete(),
            "the fixture charts 16 tiles; a complete answer would be wrong"
        );
        let rendered = score.lines().join("\n");
        assert!(rendered.contains("READ THIS FIRST"), "{rendered}");
    }

    /// A dump that has terrain everywhere the disc reaches says the disc is
    /// covered, and then the resource answers can be believed.
    #[test]
    fn a_fully_charted_disc_is_reported_as_covered() {
        let world = fixture_world();
        // A t=0 dump's charted square is `[-320, 320)`; this is the same shape
        // at a size a test can build, and the radius is scaled to match.
        let mut tiles = Vec::new();
        factorio_bot_core::test_utils::spawn_water(
            &mut tiles,
            factorio_bot_core::types::Rect::new(
                &Position::new(-30., -30.),
                &Position::new(30., 30.),
            ),
        );
        world.update_chunk_tiles(tiles).expect("tiles are accepted");
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        let score = MapScore::of(&state, &Position::new(0., 0.), 20.);
        assert!(
            score.charting.is_complete(),
            "blind at {:?}",
            score.charting.blind
        );
        assert_eq!(score.charting.covered, score.charting.probes);
        let rendered = score.lines().join("\n");
        assert!(rendered.contains("disc is covered"), "{rendered}");
        assert!(!rendered.contains("READ THIS FIRST"), "{rendered}");
    }

    #[test]
    fn every_rung_one_requirement_has_a_row() {
        let lines = fixture_score().lines();
        let rendered = lines.join("\n");
        for name in RUNG_1_ORES {
            assert!(rendered.contains(name), "{name} has no row: {rendered}");
        }
        assert!(rendered.contains("water"), "{rendered}");
        assert!(rendered.contains("wood"), "{rendered}");
        assert!(rendered.contains("VIABLE"), "{rendered}");
    }
}
