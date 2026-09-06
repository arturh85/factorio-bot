//! "Would this placement wall a bot in?" -- asked before the wall exists.
//!
//! # What this shares with `crates/core::graph::enclosure`, and what it adds
//!
//! That module answers the question about the *real*, already-built world,
//! from an `EntityGraph` the executor can read straight off the game. This
//! one asks it about a world that does not exist yet: a candidate cell or
//! power plant is only ever a handful of `FactorioEntity`s sitting in a
//! forked [`crate::state::PlanState`]'s own `added` map, never reported to
//! the game and never in any `EntityGraph`. What differs is therefore only
//! *where the obstacles come from* -- [`PlanState`]'s own occupancy here, the
//! graph's `blocked_tree` there. The grid, the rasteriser and the fills are
//! `crates/core`'s, called through the thin wrappers at the bottom of this
//! file, and the constants are re-exported rather than restated.
//!
//! One implementation on purpose. This file used to carry its own copy of
//! the algorithm, and a copy is a place for the two to disagree about what
//! a wall is: a placement that passed prevention here would then be found
//! enclosed by detection there, or the other way round, for no reason but
//! the two grids differing. Run `run-1788552801-73005` showed that the grid
//! *itself* was wrong -- finer than the game's pathfinder, so it found exits
//! the game refuses -- and one fix landing in one place is the point.
//!
//! # What counts as an obstacle, and what deliberately does not
//!
//! Three sources, not the six [`PlanState::is_area_clear_of`] (private to
//! `state.rs`) consults for placement:
//!
//! * the plan's own tentative entities (`added`) -- so a candidate cell's own
//!   parts, sitting in a forked state's `added` map, block the fill exactly as
//!   they will block a bot once they are real;
//! * the base world's entity tree (`find_entities_in_radius`);
//! * `blocked_tree` -- trees, cliffs, rocks, units, water.
//!
//! Left out, and each for a reason `crates/core`'s own module already
//! established or that follows from this one being asked *before* a build:
//!
//! * **characters.** `crates/core::graph::enclosure::escape_from` does not
//!   count other characters as obstacles either -- they move, buildings do
//!   not, and a bot that happens to be standing in a doorway for one tick is
//!   not a wall. Counting them here would fabricate an enclosure out of two
//!   bots' ordinary foot traffic.
//! * **refused footprints.** `PlanState::refused` is a verdict about where a
//!   *building* may go, not a fact about whether the ground is walkable --
//!   the game will happily route a character across a tile it refused a
//!   furnace on.
//! * **resource tiles.** Ore blocks neither a footstep nor a placement --
//!   nothing buildable collides with the `resource` layer, and since
//!   `ore-does-not-block` `is_area_clear_of` does not count it either, so this
//!   exclusion is no longer a difference between the two grids.
//!
//! # Two questions, one grid
//!
//! [`PlanState::escape_from`] asks the same question
//! `crates/core::graph::enclosure::escape_from` does: does the free region
//! around a point reach the edge of a bounded window. Run once against the
//! state *before* a candidate footprint exists and once against the forked
//! state that already carries it, the two calls answer "would this placement
//! newly close the fill that was open a moment ago" -- see
//! [`check`].
//!
//! [`PlanState::nearest_safe_escape`] answers the follow-on question a
//! diagnostic never has to: *where* should the bot go. It is not a second
//! guess at the same fill -- searching the "before" reachable set in
//! nearest-first order and testing each candidate against the "after" grid
//! for real would cost one fill per candidate. Both grids are built once, and
//! the "after" grid is turned into a single reachable-from-the-boundary set
//! ([`reachable_from_boundary`]) that every "before" cell is checked against
//! in O(1), so the whole search costs two fills over the same window rather
//! than one fill per tile it considers escaping to.

use crate::ids::BotId;
use crate::state::PlanState;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{Direction, FactorioEntity, Position, Rect};
use std::collections::BTreeSet;

/// How far from the point being asked about the search looks, in tiles, on
/// each axis. Identical to `crates/core::graph::enclosure::SEARCH_RADIUS`,
/// and for the same reason: the two answer the same question (a bounded
/// "no exit within N tiles"), and a prevention check that used a different
/// window than the detection check that inspired it would let a bot pass
/// prevention and still be found enclosed by detection a moment later, or the
/// other way round, for no reason but the two numbers disagreeing.
pub const SEARCH_RADIUS: f64 = factorio_bot_core::graph::enclosure::SEARCH_RADIUS;

/// The side of one fill cell, in tiles: one tile, the game's own pathfinding
/// grid. See `crates/core::graph::enclosure::CELL` for the argument, which
/// is the game's and not ours.
pub const CELL: f64 = factorio_bot_core::graph::enclosure::CELL;

/// Cells per axis in the searched window. Only the tests below size a grid
/// by hand; everything else reads it through `crates/core`.
#[cfg(test)]
const GRID: usize = factorio_bot_core::graph::enclosure::GRID;

/// What a bounded escape search found. See the module docs for how this
/// differs from `crates/core::graph::enclosure::Escape`, which it otherwise
/// mirrors exactly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Escape {
    /// The free region reaches the edge of the searched window.
    Open,
    /// The free region closes inside the window, at this many whole tiles of
    /// the pathfinder's grid.
    Enclosed { pocket_tiles: f64 },
    /// The window is not entirely inside the region `blocked_tree` covers, so
    /// a query for it would come back short. Never folded into `Open`: an
    /// unmodelled edge is not a proof of freedom.
    Unknown,
}

/// One bot that must walk clear before a candidate placement is built.
#[derive(Debug, Clone, PartialEq)]
pub struct Evacuation {
    pub bot: BotId,
    /// The nearest tile [`PlanState::nearest_safe_escape`] could prove both
    /// reachable right now and still connected to open ground once the
    /// placement stands.
    pub to: Position,
    /// The size of the pocket this bot would otherwise be sealed into, for
    /// the label the caller writes onto the evacuation action -- see
    /// `EventKind::BotEnclosed`'s own `pocket_tiles` for the precedent this
    /// number is read the same way as.
    pub pocket_tiles: f64,
}

/// What checking a candidate placement against every nearby bot found.
#[derive(Debug, Clone, PartialEq)]
pub enum EnclosurePrevention {
    /// No bystander's status changes because of this placement.
    Clear,
    /// These bots must be walked to safety, in this order, before the
    /// placement's own steps run.
    Evacuate(Vec<Evacuation>),
    /// At least one bystander would be trapped and no safe escape could be
    /// proven -- either the search ran off the modelled map, or the only way
    /// out runs through the footprint itself. The candidate must be refused
    /// outright: an unproven escape is not a green light to build, the same
    /// discipline `crates/core::graph::enclosure::Escape::Unknown` already
    /// keeps for detection.
    Refuse,
}

/// How far outside a bot's own position an obstacle can sit and still change
/// that bot's grid, in tiles on one axis.
///
/// [`window`] is anchored to the *tile* grid rather than to the character, so
/// the searched square runs from `floor(x) - SEARCH_RADIUS` to
/// `floor(x) + SEARCH_RADIUS`: up to one whole tile further from the bot on
/// one side than [`SEARCH_RADIUS`] alone. `rasterize` then grows every
/// obstacle box by half a character before testing tile centres, and vanilla's
/// character half-side is 0.199 -- [`CHARACTER_HALF_BOX_BOUND`] is the
/// generous cap that keeps this a constant rather than a prototype lookup.
/// An obstacle whose box is further than this from the bot on either axis
/// cannot mark a single cell of that bot's window.
const WINDOW_REACH: f64 = SEARCH_RADIUS + CELL + CHARACTER_HALF_BOX_BOUND;

/// An upper bound on half the `character` collision box, in tiles. Vanilla is
/// 0.199 (`crates/core::graph::enclosure`'s
/// `VANILLA_CHARACTER_COLLISION_HALF_SIDE`) and a mod can only make this a
/// *bound* question, not an exact one, so [`WINDOW_REACH`] takes a cap rather
/// than reading the prototype: it is a narrowing radius, and being generous
/// costs one more bot's escape fill, while being short is the silent miss
/// this whole selection exists to avoid.
const CHARACTER_HALF_BOX_BOUND: f64 = 0.5;

/// An upper bound on half a single entity's collision-box *diagonal*, in
/// tiles.
///
/// Used only to *narrow* the query below: an entity whose centre is further
/// from the bot than the window's own half-diagonal plus this cannot have its
/// box touch the window, so it need not be looked at. The exact test is the
/// box against the window rect, done afterwards, so this being generous costs
/// one comparison and being short is the only way a part could be missed.
/// Vanilla's largest collision box is the rocket silo's, under 5 tiles on a
/// half-side and so about 7 on a half-diagonal; 16 admits an entity of
/// 22x22 tiles, which is not an entity.
///
/// The same narrowing-then-exact-test discipline
/// `crates/core::graph::enclosure::grid_for` uses: `blocking_boxes_within`
/// "admits boxes which merely come close; every one of them is tested exactly
/// in `rasterize`".
const MAX_ENTITY_HALF_SPAN: f64 = 16.0;

/// Whether the candidate placement -- everything `trial` holds that `state`
/// does not -- would touch the escape window of a bot standing at `at`.
///
/// # This is the selection, and it does not know how big a footprint is
///
/// It used to: `check` picked bots within `SEARCH_RADIUS + FOOTPRINT_PAD` of
/// the placement's *origin*, and `FOOTPRINT_PAD = 12` was justified in its
/// own doc by enumerating "the largest shape sited today (the power plant:
/// pump, three pipes, boiler, **engine**, pole)" as spanning "under 10 tiles
/// from its own origin". That enumeration was singular. On 2026-09-06
/// `method::power::plan_plant` began sizing the engine row from demand up to
/// `MAX_ENGINES_PER_BOILER` engines at a five-tile pitch, and the second
/// engine reaches **14.008** tiles from the pump -- outside the 12 the pad
/// admitted. Nothing could have noticed: the pad is read here and written
/// there, and no test in either file computed one from the other. A bot only
/// the second engine would wall in fell outside the window, was never
/// examined, and was walled in with nothing reported.
///
/// So the question is asked the other way round. Rather than guessing how far
/// a footprint reaches and padding a radius by it, each bot is asked whether
/// the parts that actually exist touch the window that is actually searched.
/// **No term of this depends on the shape being placed**, so no future
/// resizing of a cell or a plant can make it wrong.
///
/// `trial` is `state.fork()` with the candidate created in it (see [`check`]),
/// so the two share a base world and differ only by the candidate. Both are
/// asked for the entities around `at` and the answers differenced -- two
/// range queries and a set difference, rather than a point lookup per entity.
fn candidate_touches_window(state: &PlanState, trial: &PlanState, at: &Position) -> bool {
    let (searched, _) = window(at);
    // A box touching the window has its centre no further than the window's
    // own half-diagonal plus the box's own half-span.
    let probe = WINDOW_REACH * std::f64::consts::SQRT_2 + MAX_ENTITY_HALF_SPAN;
    let before: BTreeSet<(u64, u64, String)> = state
        .entities_within(at, probe)
        .iter()
        .map(key_of)
        .collect();
    trial
        .entities_within(at, probe)
        .iter()
        .filter(|entity| !before.contains(&key_of(entity)))
        .any(|entity| touches(&searched, &footprint_of(trial, entity)))
}

/// Whether `area`, grown by the character half-box `rasterize` will grow it
/// by, overlaps the searched `window`.
///
/// Conservative on purpose: overlapping the window is necessary for marking
/// one of its cells and not quite sufficient (a box can clip a window's edge
/// without covering any tile centre). The cost of the difference is one
/// bot's escape fill returning the same answer twice.
fn touches(window: &Rect, area: &Rect) -> bool {
    area.left_top.x() - CHARACTER_HALF_BOX_BOUND <= window.right_bottom.x()
        && area.right_bottom.x() + CHARACTER_HALF_BOX_BOUND >= window.left_top.x()
        && area.left_top.y() - CHARACTER_HALF_BOX_BOUND <= window.right_bottom.y()
        && area.right_bottom.y() + CHARACTER_HALF_BOX_BOUND >= window.left_top.y()
}

/// The identity two states agree on for one entity: where it stands and what
/// it is. Positions come from the same clones on both sides, so their bits
/// compare exactly; the name is carried because a `create_entity` may replace
/// what stood on a tile with something else.
fn key_of(entity: &FactorioEntity) -> (u64, u64, String) {
    (
        entity.position.x().to_bits(),
        entity.position.y().to_bits(),
        entity.name.clone(),
    )
}

/// The box this entity blocks with, the same three ways `PlanState`'s own
/// obstacle collection resolves it: the reported bounding box, else the
/// prototype's collision box at the entity's facing, else the tile it stands
/// on.
fn footprint_of(state: &PlanState, entity: &FactorioEntity) -> Rect {
    if entity.bounding_box.width() > 0. && entity.bounding_box.height() > 0. {
        return entity.bounding_box.clone();
    }
    Direction::from_u8(entity.direction)
        .and_then(|facing| state.collision_area_facing(&entity.name, &entity.position, facing))
        .unwrap_or_else(|| {
            let (x, y) = (entity.position.x().floor(), entity.position.y().floor());
            Rect::new(&Position::new(x, y), &Position::new(x + 1., y + 1.))
        })
}

/// The furthest of a box's four corners from `point`, in tiles. Only the
/// measurement test below uses it: the selection above compares boxes against
/// the window rather than distances against a radius.
#[cfg(test)]
fn corner_reach(point: &Position, area: &Rect) -> f64 {
    let xs = [area.left_top.x(), area.right_bottom.x()];
    let ys = [area.left_top.y(), area.right_bottom.y()];
    let mut furthest: f64 = 0.;
    for x in xs {
        for y in ys {
            furthest = furthest.max(distance(point, &Position::new(x, y)));
        }
    }
    furthest
}

#[cfg(test)]
fn distance(a: &Position, b: &Position) -> f64 {
    (a.x() - b.x()).hypot(a.y() - b.y())
}

/// Check a candidate placement, already forked into `trial`, against every
/// bot near `origin` known to `state`.
///
/// `state` is the world as it stands before the placement; `trial` is the
/// same state with the candidate's own parts already created in it (`fit`, in
/// both `method::assemble` and `method::power`, already builds exactly this
/// fork to check `Feeds`/`Powered`, so this is the last predicate asked of a
/// candidate rather than a fresh walk of the state). Only a bot whose escape
/// answer flips from [`Escape::Open`] in `state` to [`Escape::Enclosed`] in
/// `trial` is this placement's doing; a bot already enclosed, or still open
/// either way, is left alone.
///
/// # Which bots are examined, and why no constant decides it
///
/// Every character `state` knows, filtered by whether this candidate could
/// possibly change that bot's own answer -- [`candidate_touches_window`],
/// which compares the parts `trial` actually holds against the window that
/// bot is actually searched in. A bot the candidate cannot reach is skipped
/// before either fill, so the cheap case stays cheap.
///
/// This replaced a radius, `SEARCH_RADIUS + FOOTPRINT_PAD` = 36 tiles from
/// `origin`, whose pad was sized by hand against the shapes that existed the
/// day it was written and went stale the moment `method::power` grew a second
/// steam engine. See [`candidate_touches_window`] for that account. Two
/// things about the replacement are worth stating in the negative:
///
/// * **it is not "every bot", though it iterates every bot.** The filter is
///   exact rather than generous: it asks the question the two fills would
///   have answered, cheaply, and only skips a bot whose two answers are
///   provably identical.
/// * **it no longer has a term for the footprint's size at all**, so nothing
///   about a future cell or plant shape can make it wrong. The old form also
///   compared a straight-line distance against a *square* window with no
///   `sqrt(2)` anywhere in it, which was short by that factor for a bot on
///   the diagonal; that error has no counterpart here, because nothing is
///   compared against a radius any more.
///
/// [`PlanState::characters_near`] is passed an infinite radius rather than a
/// large one because there is no honest finite number: any bound would be the
/// same kind of claim about somebody else's geometry that this fix removes.
pub fn check(state: &PlanState, trial: &PlanState, origin: &Position) -> EnclosurePrevention {
    let mut evacuations = Vec::new();
    for (player, pos) in state.characters_near(origin, f64::INFINITY) {
        if !candidate_touches_window(state, trial, &pos) {
            continue;
        }
        let before = state.escape_from(&pos);
        let after = trial.escape_from(&pos);
        match (before, after) {
            (Escape::Open, Escape::Enclosed { pocket_tiles }) => {
                match state.nearest_safe_escape(trial, &pos) {
                    Some(to) => evacuations.push(Evacuation {
                        bot: BotId(player),
                        to,
                        pocket_tiles,
                    }),
                    // The only way out of the pocket this placement creates
                    // runs through the placement itself. Walking somewhere
                    // now cannot fix that; only not building here can.
                    None => return EnclosurePrevention::Refuse,
                }
            }
            // An answer of `Unknown` on either side means the window ran off
            // the modelled map. That is not evidence the bot is safe.
            (Escape::Unknown, _) | (_, Escape::Unknown) => return EnclosurePrevention::Refuse,
            _ => {}
        }
    }
    if evacuations.is_empty() {
        EnclosurePrevention::Clear
    } else {
        EnclosurePrevention::Evacuate(evacuations)
    }
}

/// The window `SEARCH_RADIUS` tiles around `from` on every axis, anchored to
/// the tile grid, and the grid's own origin corner. One implementation, in
/// `crates/core` -- see the module docs for why the two crates must not each
/// have their own.
pub(crate) fn window(from: &Position) -> (Rect, (f64, f64)) {
    factorio_bot_core::graph::enclosure::window(from)
}

pub(crate) fn cell_index(x: usize, y: usize) -> usize {
    factorio_bot_core::graph::enclosure::cell_index(x, y)
}

/// Rasterise `obstacles` into a blocked mask over the window whose origin
/// corner is `origin`, growing every box by `half_box` on each axis first --
/// the game's own tile-centre test, as `crates/core` implements it.
pub(crate) fn rasterize(
    obstacles: impl Iterator<Item = Rect>,
    origin: (f64, f64),
    half_box: (f64, f64),
) -> Vec<bool> {
    factorio_bot_core::graph::enclosure::rasterize(obstacles, origin, half_box)
}

/// Flood fill from the centre cell, admitted free whatever `blocked` says
/// there, translated into this crate's own [`Escape`].
pub(crate) fn fill_from_center(blocked: &[bool]) -> Escape {
    match factorio_bot_core::graph::enclosure::fill_from_center(blocked) {
        factorio_bot_core::graph::enclosure::Escape::Open => Escape::Open,
        factorio_bot_core::graph::enclosure::Escape::Enclosed { pocket_tiles } => {
            Escape::Enclosed { pocket_tiles }
        }
        factorio_bot_core::graph::enclosure::Escape::Unknown(_) => Escape::Unknown,
    }
}

/// Every cell reachable from the centre, nearest first.
pub(crate) fn bfs_order_from_center(blocked: &[bool]) -> Vec<(usize, usize)> {
    factorio_bot_core::graph::enclosure::bfs_order_from_center(blocked)
}

/// Every free cell still connected to the window's own edge.
pub(crate) fn reachable_from_boundary(blocked: &[bool]) -> Vec<bool> {
    factorio_bot_core::graph::enclosure::reachable_from_boundary(blocked)
}

/// The tile centre `(x, y)` names, in the window whose origin corner is
/// `origin`.
pub(crate) fn cell_to_position(origin: (f64, f64), cell: (usize, usize)) -> Position {
    factorio_bot_core::graph::enclosure::cell_to_position(origin, cell)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The one fact this file has to keep true on its own: its answers are
    /// the same answers `crates/core` gives, cell for cell, so a placement
    /// that passes prevention here is not found enclosed by detection there.
    #[test]
    fn prevention_and_detection_share_one_grid() {
        assert_eq!(
            SEARCH_RADIUS,
            factorio_bot_core::graph::enclosure::SEARCH_RADIUS
        );
        assert_eq!(CELL, factorio_bot_core::graph::enclosure::CELL);
        assert_eq!(GRID, factorio_bot_core::graph::enclosure::GRID);
    }

    #[test]
    fn an_empty_grid_is_open() {
        let blocked = vec![false; GRID * GRID];
        assert_eq!(fill_from_center(&blocked), Escape::Open);
    }

    #[test]
    fn a_closed_ring_encloses() {
        let mut blocked = vec![false; GRID * GRID];
        let (cx, cy) = (GRID / 2, GRID / 2);
        let r = 2usize;
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if x == cx - r || x == cx + r || y == cy - r || y == cy + r {
                    blocked[cell_index(x, y)] = true;
                }
            }
        }
        // Two tiles out on every side leaves a 3x3 interior, whole tiles.
        assert_eq!(
            fill_from_center(&blocked),
            Escape::Enclosed { pocket_tiles: 9. }
        );
    }
}

/// What a real power plant's own geometry costs the selection window.
///
/// Separate from the tests above because these are the only ones that reach
/// out of this module and site an actual plant: the window is here and the
/// shape it has to cover is in `method::power`, and the whole defect this
/// module was carrying was that no test in either file compared the two.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod plant_reach {
    use super::*;
    use crate::ids::BotId;
    use crate::method::power::{ENGINE, entity_for, plan_plant_for};
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    /// The retired constant. `FOOTPRINT_PAD = 12` was justified by enumerating
    /// "the largest shape sited today (the power plant: pump, three pipes,
    /// boiler, **engine**, pole)" as spanning "under 10 tiles from its own
    /// origin". Kept here as the literal the measurement is compared against.
    const RETIRED_FOOTPRINT_PAD: f64 = 12.0;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    /// A plant, sited on the fixture's lake, forked into a trial exactly as
    /// `plan_plant_for` and `complete_plant` fork one before calling
    /// [`check`], plus the pump it is measured from.
    fn plant_trial(kw: f64) -> (PlanState, PlanState, Position, usize) {
        let before = state();
        let plant = plan_plant_for(&before, &Position::new(0., 0.), kw).expect("the fixture lake");
        let pump = plant
            .parts
            .iter()
            .find(|part| part.name == crate::method::power::PUMP)
            .map(|part| part.position.clone())
            .expect("a plant sited from scratch places its own pump");
        let engines = plant.parts.iter().filter(|p| p.name == ENGINE).count();
        let mut trial = before.fork();
        for part in &plant.parts {
            trial.create_entity(entity_for(&trial, part));
        }
        (before, trial, pump, engines)
    }

    /// The furthest corner of any entity the trial holds and the base world
    /// does not, from `pump` -- the plant's own reach, measured off the
    /// entities `plan_plant_for` sited.
    fn reach_of(trial: &PlanState, pump: &Position) -> f64 {
        let base = state();
        let mut furthest: f64 = 0.;
        for entity in trial.entities_within(pump, 64.) {
            if base.entity_at(&entity.position).is_some() {
                continue;
            }
            furthest = furthest.max(corner_reach(pump, &footprint_of(trial, &entity)));
        }
        furthest
    }

    /// The measurement the review of 2026-09-06 asked for, as two literals.
    ///
    /// A one-engine plant reaches 9.64 tiles from its pump and a two-engine
    /// plant reaches 14.01 -- so the `12` that used to bound the selection
    /// window covered the first and **not** the second, and a bot only the
    /// second engine would wall in was never examined.
    ///
    /// Both numbers are asserted against literals rather than against
    /// anything the code computes, so that a change to the engine pitch, the
    /// engine prototype or the pole siting moves them and fails here. The
    /// tolerance is a hundredth of a tile, which is smaller than any layout
    /// change could be.
    #[test]
    fn a_second_engine_takes_a_plant_past_the_pad_that_used_to_bound_the_window() {
        let (_, trial, pump, engines) = plant_trial(0.);
        assert_eq!(engines, 1, "a plant sized against no demand has one engine");
        let one = reach_of(&trial, &pump);
        assert!(
            (one - 9.639).abs() < 0.01,
            "a one-engine plant reaches 9.639 tiles from its pump, got {one}"
        );
        assert!(
            one < RETIRED_FOOTPRINT_PAD,
            "which is what made `FOOTPRINT_PAD = 12` look generous"
        );

        let (_, trial, pump, engines) = plant_trial(1000.);
        assert_eq!(engines, 2, "1,000 kW needs two 900 kW engines");
        let two = reach_of(&trial, &pump);
        assert!(
            (two - 14.008).abs() < 0.01,
            "a two-engine plant reaches 14.008 tiles from its pump, got {two}"
        );
        assert!(
            two > RETIRED_FOOTPRINT_PAD,
            "and 14.008 is outside the 12 tiles the retired constant admitted: \
             this is the defect, measured"
        );
    }
}
