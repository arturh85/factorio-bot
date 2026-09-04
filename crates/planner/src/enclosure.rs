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
//! * **resource tiles.** Ore does not block a footstep; `is_area_clear_of`
//!   only treats it as occupancy for placement, by policy, not by collision.
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
use factorio_bot_core::types::{Position, Rect};

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

/// A generous upper bound on any cell or power plant's own half-diagonal,
/// added to [`SEARCH_RADIUS`] when selecting which bots to check against a
/// candidate footprint.
///
/// Bounded rather than "every bot in the world": a bot farther than
/// `SEARCH_RADIUS` from every point of the footprint cannot have the
/// footprint's parts land inside *its own* search window, so the two escape
/// answers cannot differ for it regardless of what gets built. The largest
/// shape sited today (the power plant: pump, three pipes, boiler, engine,
/// pole) spans under 10 tiles from its own origin; 12 tiles of pad leaves
/// room without widening the search to the whole map. If a future cell shape
/// is wider than that, this constant is the one place to widen.
const FOOTPRINT_PAD: f64 = 12.0;

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
pub fn check(state: &PlanState, trial: &PlanState, origin: &Position) -> EnclosurePrevention {
    let mut evacuations = Vec::new();
    for (player, pos) in state.characters_near(origin, SEARCH_RADIUS + FOOTPRINT_PAD) {
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
