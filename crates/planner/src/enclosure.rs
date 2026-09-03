//! "Would this placement wall a bot in?" -- asked before the wall exists.
//!
//! # Why this is a second implementation of `crates/core::graph::enclosure`
//!
//! That module answers the same question about the *real*, already-built
//! world, from an `EntityGraph` the executor can read straight off the game.
//! This one has to answer it about a world that does not exist yet: a
//! candidate cell or power plant is only ever a handful of `FactorioEntity`s
//! sitting in a forked [`crate::state::PlanState`]'s own `added` map, never
//! reported to the game and never in any `EntityGraph`. `crates/planner`
//! cannot pull `crates/core::graph::enclosure` in and hand it that fork --
//! not because of an async/IO boundary (the function is already synchronous
//! and pure) but because it is typed against `EntityGraph` specifically and
//! has no notion of a plan's own tentative placements at all. The one thing
//! actually shared is the algorithm, which is reimplemented here against
//! [`PlanState`]'s own occupancy instead.
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
use std::collections::VecDeque;

/// How far from the point being asked about the search looks, in tiles, on
/// each axis. Identical to `crates/core::graph::enclosure::SEARCH_RADIUS`,
/// and for the same reason: the two answer the same question (a bounded
/// "no exit within N tiles"), and a prevention check that used a different
/// window than the detection check that inspired it would let a bot pass
/// prevention and still be found enclosed by detection a moment later, or the
/// other way round, for no reason but the two numbers disagreeing.
pub const SEARCH_RADIUS: f64 = 24.0;

/// The side of one fill cell, in tiles. Identical to
/// `crates/core::graph::enclosure::CELL` and argued the same way there: fine
/// enough that no wall the character's own half-box would block can leak
/// through it, coarse enough that the grid stays a few hundred thousand
/// cells rather than tens of millions.
pub const CELL: f64 = 0.125;

/// Cells per axis in the searched window.
const GRID: usize = (2. * SEARCH_RADIUS / CELL) as usize;

/// What a bounded escape search found. See the module docs for how this
/// differs from `crates/core::graph::enclosure::Escape`, which it otherwise
/// mirrors exactly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Escape {
    /// The free region reaches the edge of the searched window.
    Open,
    /// The free region closes inside the window, at this many square tiles of
    /// configuration space.
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

/// The window `SEARCH_RADIUS` tiles around `from` on every axis, and the
/// grid's own origin corner (its bottom-left, in the same coordinate frame as
/// `from`).
pub(crate) fn window(from: &Position) -> (Rect, (f64, f64)) {
    let origin = (from.x() - SEARCH_RADIUS, from.y() - SEARCH_RADIUS);
    (
        Rect::new(
            &Position::new(origin.0, origin.1),
            &Position::new(from.x() + SEARCH_RADIUS, from.y() + SEARCH_RADIUS),
        ),
        origin,
    )
}

pub(crate) fn cell_index(x: usize, y: usize) -> usize {
    y * GRID + x
}

/// The half-open range of cell indices whose centres fall in `[lo, hi]`,
/// measured in tiles from the window's own left/top edge, clipped to the
/// grid. Ported unchanged from `crates/core::graph::enclosure::cells_covering`
/// -- see that function's doc for why the range can come back empty.
fn cells_covering(lo: f64, hi: f64) -> Option<std::ops::Range<usize>> {
    let first = (lo / CELL - 0.5).ceil();
    let last = (hi / CELL - 0.5).floor();
    let first = first.max(0.) as usize;
    let last = last.min(GRID as f64 - 1.);
    if last < 0. {
        return None;
    }
    let end = last as usize + 1;
    (first < end).then_some(first..end)
}

/// Rasterise `obstacles` into a `GRID x GRID` blocked mask over the window
/// whose bottom-left corner is `origin`, growing every box by `half_box` on
/// each axis first -- configuration space, exactly as
/// `crates/core::graph::enclosure::escape_from` builds it.
pub(crate) fn rasterize(
    obstacles: impl Iterator<Item = Rect>,
    origin: (f64, f64),
    half_box: (f64, f64),
) -> Vec<bool> {
    let mut blocked = vec![false; GRID * GRID];
    for box_ in obstacles {
        let Some(xs) = cells_covering(
            box_.left_top.x() - half_box.0 - origin.0,
            box_.right_bottom.x() + half_box.0 - origin.0,
        ) else {
            continue;
        };
        let Some(ys) = cells_covering(
            box_.left_top.y() - half_box.1 - origin.1,
            box_.right_bottom.y() + half_box.1 - origin.1,
        ) else {
            continue;
        };
        for y in ys {
            let row = y * GRID;
            blocked[row + xs.start..row + xs.end].fill(true);
        }
    }
    blocked
}

/// The four axis neighbours of `(x, y)`, each still inside the grid.
fn neighbours(x: usize, y: usize) -> impl Iterator<Item = (usize, usize)> {
    [
        (x + 1, y),
        (x.wrapping_sub(1), y),
        (x, y + 1),
        (x, y.wrapping_sub(1)),
    ]
    .into_iter()
    .filter(|&(nx, ny)| nx < GRID && ny < GRID)
}

/// Flood fill from the centre cell, admitted free whatever `blocked` says
/// there -- the character's own ground is real ground, occupancy model or
/// not. Mirrors `crates/core::graph::enclosure::escape_from`'s own fill
/// exactly.
pub(crate) fn fill_from_center(blocked: &[bool]) -> Escape {
    let start = (GRID / 2, GRID / 2);
    let mut seen = vec![false; GRID * GRID];
    let mut stack = vec![start];
    seen[cell_index(start.0, start.1)] = true;
    let mut reached = 0u32;
    while let Some((x, y)) = stack.pop() {
        reached += 1;
        if x == 0 || y == 0 || x == GRID - 1 || y == GRID - 1 {
            return Escape::Open;
        }
        for (nx, ny) in neighbours(x, y) {
            let index = cell_index(nx, ny);
            if seen[index] || blocked[index] {
                continue;
            }
            seen[index] = true;
            stack.push((nx, ny));
        }
    }
    Escape::Enclosed {
        pocket_tiles: f64::from(reached) * CELL * CELL,
    }
}

/// Every cell reachable from the centre, admitted free there whatever
/// `blocked` says, in breadth-first (nearest-first) order.
pub(crate) fn bfs_order_from_center(blocked: &[bool]) -> Vec<(usize, usize)> {
    let start = (GRID / 2, GRID / 2);
    let mut seen = vec![false; GRID * GRID];
    let mut queue = VecDeque::from([start]);
    seen[cell_index(start.0, start.1)] = true;
    let mut order = Vec::new();
    while let Some((x, y)) = queue.pop_front() {
        order.push((x, y));
        for (nx, ny) in neighbours(x, y) {
            let index = cell_index(nx, ny);
            if seen[index] || blocked[index] {
                continue;
            }
            seen[index] = true;
            queue.push_back((nx, ny));
        }
    }
    order
}

/// Every free cell still connected to the window's own edge -- seeded from
/// every free cell on the boundary at once, rather than from the centre, so a
/// pocket that has sealed the centre off can still be told apart from ground
/// that stays open at the rim.
pub(crate) fn reachable_from_boundary(blocked: &[bool]) -> Vec<bool> {
    let mut seen = vec![false; GRID * GRID];
    let mut queue = VecDeque::new();
    for i in 0..GRID {
        for (x, y) in [(i, 0), (i, GRID - 1), (0, i), (GRID - 1, i)] {
            let index = cell_index(x, y);
            if !blocked[index] && !seen[index] {
                seen[index] = true;
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        for (nx, ny) in neighbours(x, y) {
            let index = cell_index(nx, ny);
            if seen[index] || blocked[index] {
                continue;
            }
            seen[index] = true;
            queue.push_back((nx, ny));
        }
    }
    seen
}

/// The tile centre `(x, y)` names, in the window whose bottom-left corner is
/// `origin`.
pub(crate) fn cell_to_position(origin: (f64, f64), cell: (usize, usize)) -> Position {
    Position::new(
        origin.0 + (cell.0 as f64 + 0.5) * CELL,
        origin.1 + (cell.1 as f64 + 0.5) * CELL,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_grid_is_open() {
        let blocked = vec![false; GRID * GRID];
        assert_eq!(fill_from_center(&blocked), Escape::Open);
    }

    #[test]
    fn a_closed_ring_encloses() {
        let mut blocked = vec![false; GRID * GRID];
        let (cx, cy) = (GRID / 2, GRID / 2);
        // A ring of blocked cells a few cells out on every side, thick enough
        // that a 4-connected fill cannot leak through it.
        let r = 4usize;
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if x == cx - r || x == cx + r || y == cy - r || y == cy + r {
                    blocked[cell_index(x, y)] = true;
                }
            }
        }
        match fill_from_center(&blocked) {
            Escape::Enclosed { pocket_tiles } => assert!(pocket_tiles > 0.),
            other => panic!("expected an enclosure, got {other:?}"),
        }
    }

    #[test]
    fn boundary_and_center_fills_agree_on_an_open_grid() {
        let blocked = vec![false; GRID * GRID];
        let safe = reachable_from_boundary(&blocked);
        // Every cell the centre-seeded fill can reach on an open grid is a
        // cell the boundary fill reaches too, and vice versa: with nothing
        // blocked the two are the same connected region.
        assert!(safe.iter().all(|&s| s));
        let order = bfs_order_from_center(&blocked);
        assert_eq!(order.len(), GRID * GRID);
    }

    #[test]
    fn a_sealed_pocket_is_unreachable_from_the_boundary() {
        let mut blocked = vec![false; GRID * GRID];
        let (cx, cy) = (GRID / 2, GRID / 2);
        let r = 4usize;
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if x == cx - r || x == cx + r || y == cy - r || y == cy + r {
                    blocked[cell_index(x, y)] = true;
                }
            }
        }
        let safe = reachable_from_boundary(&blocked);
        assert!(
            !safe[cell_index(cx, cy)],
            "the centre is sealed inside the ring and must not read as connected to the edge"
        );
        // A cell just outside the ring is still connected to the boundary.
        assert!(safe[cell_index(cx + r + 1, cy)]);
    }

    #[test]
    fn bfs_order_is_nearest_first() {
        let blocked = vec![false; GRID * GRID];
        let order = bfs_order_from_center(&blocked);
        let (cx, cy) = (GRID / 2, GRID / 2);
        let dist = |(x, y): (usize, usize)| -> i64 {
            (x as i64 - cx as i64).abs() + (y as i64 - cy as i64).abs()
        };
        let mut last = 0;
        for cell in order {
            let d = dist(cell);
            assert!(d >= last, "bfs order must be non-decreasing in distance");
            last = d;
        }
    }
}
