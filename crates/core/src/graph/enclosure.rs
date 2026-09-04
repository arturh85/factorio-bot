//! "Can this character reach open ground at all?"
//!
//! # The condition this names
//!
//! Every individual placement in this system is already held clear of every
//! character's collision box — `crates/planner`'s `placement_clearance` does
//! that, and a test pins it. **Boxing a bot in is emergent**: each placement is
//! legitimately clear, and the *set* forms a wall. So it cannot be found by
//! looking at placements one at a time; it is a property of the resulting
//! occupancy, and the only way to see it is to ask whether the ground the
//! character stands on still connects to anywhere else.
//!
//! Run `run-1788432181-42528` is why this exists. Bots 2 and 3 reported
//! byte-identical positions from tick ~48 000 to the end of the run — bot 2
//! last moved at tick 47 940, bot 3 at 48 000 — while bot 1 worked the same
//! half of the map freely and performed 746 of the run's 831 dispatches.
//! Twenty walks failed, all on bots 2/3/4 and none on bot 1, nineteen of them
//! `failed to path find` *before dispatch*. Two of those refusals were for
//! destinations barely four tiles from where the bot stood. The run completed
//! its budget with two of four bots frozen and **nothing in the record named
//! the condition**; it was found by reading `samples.jsonl` by hand.
//!
//! # Whose notion of "reachable" this is
//!
//! **The game's pathfinder's, not geometry's.** A bot only ever moves along a
//! path `LuaSurface.request_path` returns, so the question that matters is
//! not "could a 0.4-tile-wide box slide out of here" but "would the game
//! find a route out of here". The two differ, and run
//! `run-1788552801-73005` is the difference: bot 1 stood in a 2×2-tile patch
//! whose exits were a 0.5-tile crack between an `iron-chest` and its
//! `inserter` and a 0.65-tile one between that inserter and an
//! `assembling-machine-1`. A character fits through either, geometrically.
//! The game refused it three walks from that spot, `failed to path find`
//! every time, and it stood there for 40 000 ticks. This fill, run at
//! 1/8-tile resolution in configuration space as it then was, found both
//! cracks and answered [`Escape::Open`] — so the record said nothing.
//!
//! The mod asks for every path at the pathfinder's default
//! `path_resolution_modifier` of 0, which the API documents as "a resolution
//! of `1x1` tiles, centered on tile centers". So this fill uses exactly that
//! grid: one cell per tile, anchored to the map's tile grid rather than to
//! the character, and a tile is blocked when the character's collision box
//! centred on that tile's centre overlaps an obstacle. That is what
//! [`CELL`] = 1 and the tile-anchored [`window`] encode. A tile-wide channel
//! of land stays open under this rule (its centre is clear); a crack narrower
//! than a tile between two off-grid boxes does not, whatever its width — and
//! that is the game's answer too.
//!
//! # What the answer means, and what it does not
//!
//! [`Escape::Open`] is *not* a proof of freedom. It says the free region
//! around the character reaches the edge of a window [`SEARCH_RADIUS`] tiles
//! across, so there is no enclosure *this small*. A pen wider than the window
//! looks exactly like open country from inside it. That is the deliberate
//! trade: a bounded fill answering "no exit within N tiles" is a weaker claim
//! than an unbounded one, and it is an honest one, where an unbounded fill on
//! a charted map is a walk over the whole `blocked_tree` — the water work
//! alone put ~410 000 blocking boxes in it.
//!
//! [`Escape::Unknown`] is a third answer and is never folded into either of
//! the others, for the reason the walk record already keeps `no_path` apart
//! from `pathfinder_busy`: "I could not tell" and "there is a way out" are
//! different facts, and reporting the first as the second is how a condition
//! goes unnamed.
//!
//! # Which way this can be wrong
//!
//! * The occupancy model is *incomplete*: `blocked_tree` holds only what the
//!   game has told us about, so uncharted ground reads as free and the fill
//!   escapes through it. A missing obstacle produces [`Escape::Open`], never
//!   [`Escape::Enclosed`]. Detection guards against this by asking only when
//!   the game has already refused a route from the same spot — see
//!   `crates/executor/src/walk_memory.rs` — so a report is a conjunction of
//!   two independent judgements about the same ground.
//! * The pathfinder is not a pure function of tile geometry; its abstract
//!   layer and its handling of a start position that is itself inside a box
//!   are not modelled here. Where this and the game disagree, the game's
//!   verdict is the one a walk record carries, and this is the diagnosis
//!   beside it, never a substitute for it.

use crate::factorio::world::Enclosure;
use crate::graph::entity_graph::EntityGraph;
use crate::types::{PlayerId, Position, Rect};
use std::collections::VecDeque;

/// Half the `character` collision box on each axis when the world carries no
/// `character` prototype.
///
/// The same number, for the same reason, as `crates/planner`'s
/// `VANILLA_CHARACTER_COLLISION_HALF_SIDE`: vanilla's character is
/// `{{-0.2, -0.2}, {0.2, 0.2}}` snapped to Factorio's 1/256 position grid.
/// Read from the prototype when there is one, because a mod can change it.
const VANILLA_CHARACTER_COLLISION_HALF_SIDE: f64 = 0.19921875;

/// How far from the character the search looks, in tiles, on each axis.
///
/// This is the whole bound, and [`Escape::Open`] means exactly "the free
/// region reaches this far" — no more. 24 tiles is chosen against the
/// condition rather than against a cost target: a bot boxed in by buildings
/// sits in a pocket of a few square tiles, and run `run-1788432181-42528`'s
/// two frozen bots were refused paths to points 4.2 tiles away. A window this
/// size contains that case with room to spare while staying a fixed, small
/// amount of work (see [`CELL`]).
pub const SEARCH_RADIUS: f64 = 24.0;

/// The side of one fill cell, in tiles: **one tile**, because that is the
/// grid the game's pathfinder searches on for every path the mod requests
/// (see the module docs).
///
/// This used to be an eighth of a tile, argued from geometry — a wall could
/// not leak between cells that close together. It could not, and it did not
/// have to: the fill leaked through *real* gaps that the game's own coarser
/// grid refuses, which is the opposite error, and the one run
/// `run-1788552801-73005` paid for. A finer grid than the pathfinder's is not
/// more accurate; it answers a different question.
pub const CELL: f64 = 1.0;

/// Cells per axis in the searched window: the window is `2 * SEARCH_RADIUS`
/// tiles across, at [`CELL`] resolution.
pub const GRID: usize = (2. * SEARCH_RADIUS / CELL) as usize;

/// What a bounded escape search found.
///
/// Three answers, never two. Collapsing [`Escape::Unknown`] into either of the
/// others would assert something nobody established — the same distinction
/// `WalkFailureKind` keeps between `no_path` and `pathfinder_busy`.
#[derive(Debug, Clone, PartialEq)]
pub enum Escape {
    /// The free region around the character reaches the edge of the searched
    /// window. **There is no enclosure within [`SEARCH_RADIUS`] tiles** — not
    /// "the character is free", which this cannot see far enough to say.
    Open,
    /// The free region closes inside the window: every tile the game's
    /// pathfinder would route the character to is inside a pocket of this
    /// many tiles, and nothing outside it is reachable without mining or
    /// being moved.
    ///
    /// Whole tiles, counted on the pathfinder's own grid — the tile the
    /// character stands on is always one of them. "Boxed into 4 tiles" and
    /// "boxed into 300" are different situations and the record should not
    /// make a reader guess which one it is looking at.
    Enclosed { pocket_tiles: f64 },
    /// The search could not answer. Never a synonym for either of the others.
    Unknown(EscapeUnknown),
}

/// Why an escape search declined to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeUnknown {
    /// The window is not entirely inside the region the occupancy index
    /// covers, so a query for it would come back short and the missing part
    /// would read as open ground.
    ///
    /// `EntityGraph`'s quad trees are built over a fixed `max_area` of ±5120
    /// tiles from the origin. What the tree does with an entity outside that
    /// is not established anywhere in this workspace, so a window touching the
    /// edge is treated as unknown rather than as free — "I could not tell" and
    /// "there is a way out" must not be the same answer.
    OutsideModel,
}

/// Whether the character standing at `from` can reach ground outside a window
/// [`SEARCH_RADIUS`] tiles around it.
///
/// One quad-tree query, then a flood fill over a fixed grid. The tree is asked
/// once for the window rather than once per cell: `blocked_tree` is indexed
/// for exactly this narrowing, and a per-cell `custom_query` would pay a
/// log-depth descent many times to answer what one windowed query answers.
/// See the module docs for what each answer claims.
///
/// The character's own tile is admitted as free whatever the occupancy says.
/// A character overlapping a collision box is a real state — the mod
/// teleports one clear of a ghost it is standing in — and refusing to seed
/// the fill there would answer "unknown" for the very case most likely to be
/// stuck. If it truly cannot move, the pocket comes back as that one tile.
pub fn escape_from(graph: &EntityGraph, from: &Position) -> Escape {
    escape_with(graph, from, &[])
}

/// [`escape_from`], with `extra` boxes counted as obstacles on top of what
/// the graph holds.
///
/// This is how a placement is asked about *before it exists*: the executor
/// hands in the footprint it is about to build and asks whether the
/// character it is about to build it with would still have a way out. Run
/// `run-1788552801-73005` is the case — the bot stood on the one patch of
/// ground whose every exit ran across the footprint it then placed.
pub fn escape_with(graph: &EntityGraph, from: &Position, extra: &[Rect]) -> Escape {
    match grid_for(graph, from, extra) {
        None => Escape::Unknown(EscapeUnknown::OutsideModel),
        Some((blocked, _)) => fill_from_center(&blocked),
    }
}

/// The nearest tile a character at `from` can reach *now* that would still
/// connect to open ground once `extra` stands, restricted to tiles `admit`
/// accepts — or `None` when there is no such tile in the window, which is
/// the honest answer when a footprint's only escape route runs through the
/// footprint itself.
///
/// Two fills over the same window rather than one fill per candidate: the
/// "before" grid gives every reachable tile in nearest-first order, the
/// "after" grid gives the set still connected to the window's own edge, and
/// the first tile in the first list that is in the second and that `admit`
/// accepts is the answer. `admit` is where a caller says what else the tile
/// has to be — within building reach of the placement, say, so that stepping
/// aside does not put the placement itself out of reach.
pub fn step_aside_target(
    graph: &EntityGraph,
    from: &Position,
    extra: &[Rect],
    admit: impl Fn(&Position) -> bool,
) -> Option<Position> {
    let (before, origin) = grid_for(graph, from, &[])?;
    let (after, _) = grid_for(graph, from, extra)?;
    let safe = reachable_from_boundary(&after);
    bfs_order_from_center(&before)
        .into_iter()
        .filter(|cell| safe[cell_index(cell.0, cell.1)])
        .map(|cell| cell_to_position(origin, cell))
        .find(|position| admit(position))
}

/// The blocked mask for the window around `from`, over the graph's own
/// obstacles plus `extra`, and the window's origin corner -- or `None` when
/// the window is not entirely inside the region the graph models.
fn grid_for(
    graph: &EntityGraph,
    from: &Position,
    extra: &[Rect],
) -> Option<(Vec<bool>, (f64, f64))> {
    let (window, origin) = window(from);
    let modelled = {
        let tree = graph.blocked_tree();
        let bounds = tree.bounding_box();
        (
            bounds.origin.x as f64,
            bounds.origin.y as f64,
            (bounds.origin.x + bounds.size.width) as f64,
            (bounds.origin.y + bounds.size.height) as f64,
        )
    };
    if window.left_top.x() < modelled.0
        || window.left_top.y() < modelled.1
        || window.right_bottom.x() > modelled.2
        || window.right_bottom.y() > modelled.3
    {
        return None;
    }
    let half_box = character_half_box(graph);
    // `blocking_boxes_within` is a narrowing pass that admits boxes which
    // merely come close; every one of them is tested exactly in `rasterize`,
    // and a box entirely outside the window clips away to nothing.
    let obstacles = graph
        .blocking_boxes_within(&window)
        .into_iter()
        .chain(extra.iter().cloned());
    Some((rasterize(obstacles, origin, half_box), origin))
}

/// [`escape_from`], turned into the ledger row a run record names the
/// condition with — or `None` when there is nothing to name.
///
/// The conversion lives here rather than at the call site so that the two
/// answers that are *not* a finding, [`Escape::Open`] and [`Escape::Unknown`],
/// are dropped in one place. Neither is a condition: the first says the
/// character can move, and the second says nobody knows, and recording either
/// as an enclosure would put a claim in the archive that this fill never made.
pub fn enclosure_at(
    graph: &EntityGraph,
    player: PlayerId,
    at: &Position,
    tick: Option<u64>,
) -> Option<Enclosure> {
    match escape_from(graph, at) {
        Escape::Enclosed { pocket_tiles } => Some(Enclosure {
            tick,
            player,
            at: at.clone(),
            pocket_tiles,
            searched_tiles: SEARCH_RADIUS,
        }),
        Escape::Open | Escape::Unknown(_) => None,
    }
}

/// Half the `character` collision box on each axis, as the graph's prototypes
/// report it, falling back to
/// [`VANILLA_CHARACTER_COLLISION_HALF_SIDE`] on both axes.
///
/// Read rather than hardcoded for the reason `crates/planner`'s
/// `character_half_box` gives: it is prototype data a mod can change. A world
/// with no `character` prototype at all is the ordinary case for a fixture.
pub fn character_half_box(graph: &EntityGraph) -> (f64, f64) {
    graph
        .entity_prototypes()
        .get("character")
        .map(|p| (p.collision_box.width() / 2., p.collision_box.height() / 2.))
        .unwrap_or((
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
        ))
}

// ---------------------------------------------------------------------------
// The grid itself. Shared with `crates/planner::enclosure`, which asks the
// same question of a plan's tentative occupancy: one implementation, so that
// prevention and detection cannot disagree about what a wall is.
// ---------------------------------------------------------------------------

/// The window [`SEARCH_RADIUS`] tiles around `from` on every axis, **anchored
/// to the tile grid**, and its origin corner (left/top).
///
/// Anchored so that every cell *is* a map tile and its centre is that tile's
/// centre — the points the game's pathfinder tests. A window anchored on the
/// character instead would sample the character's box at points offset from
/// the tile centres by whatever fraction of a tile it happens to stand at,
/// which is a grid the game never searches.
pub fn window(from: &Position) -> (Rect, (f64, f64)) {
    let origin = (
        from.x().floor() - SEARCH_RADIUS,
        from.y().floor() - SEARCH_RADIUS,
    );
    (
        Rect::new(
            &Position::new(origin.0, origin.1),
            &Position::new(origin.0 + 2. * SEARCH_RADIUS, origin.1 + 2. * SEARCH_RADIUS),
        ),
        origin,
    )
}

/// Row-major index of cell `(x, y)`.
pub fn cell_index(x: usize, y: usize) -> usize {
    y * GRID + x
}

/// The tile centre cell `(x, y)` names, in the window whose left/top corner
/// is `origin`.
pub fn cell_to_position(origin: (f64, f64), cell: (usize, usize)) -> Position {
    Position::new(
        origin.0 + (cell.0 as f64 + 0.5) * CELL,
        origin.1 + (cell.1 as f64 + 0.5) * CELL,
    )
}

/// Rasterise `obstacles` into a `GRID x GRID` blocked mask over the window
/// whose left/top corner is `origin`.
///
/// A cell is blocked when its centre falls inside the obstacle grown by
/// `half_box` on each axis — which is the same test as "the character's box,
/// centred on this tile's centre, overlaps the obstacle", the one the game's
/// pathfinder makes at its default resolution. Inclusive at the edges: a box
/// that exactly touches the character's box is counted as blocking, the
/// conservative direction, and one the 1/256 position grid makes rare.
pub fn rasterize(
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

/// The half-open range of cell indices whose centres fall in `[lo, hi]`,
/// measured in tiles from the window's left/top edge, clipped to the grid.
///
/// `None` when that range is empty after clipping — which is the ordinary
/// answer for a box the quad tree admitted because it came close to the
/// window without reaching it.
fn cells_covering(lo: f64, hi: f64) -> Option<std::ops::Range<usize>> {
    // Cell `i`'s centre sits at `(i + 0.5) * CELL`.
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

/// Flood fill from the centre cell — the tile `from` stands on — admitted
/// free whatever `blocked` says there: the character's own ground is real
/// ground, occupancy model or not.
pub fn fill_from_center(blocked: &[bool]) -> Escape {
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
pub fn bfs_order_from_center(blocked: &[bool]) -> Vec<(usize, usize)> {
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

/// Every free cell still connected to the window's own edge — seeded from
/// every free cell on the boundary at once, rather than from the centre, so a
/// pocket that has sealed the centre off can still be told apart from ground
/// that stays open at the rim.
pub fn reachable_from_boundary(blocked: &[bool]) -> Vec<bool> {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_anchored_to_the_tile_grid() {
        // Wherever inside its tile the character stands, the window's origin
        // is the same tile corner, and the centre cell is that tile.
        for x in [34.0, 34.41796875, 34.99] {
            let (rect, origin) = window(&Position::new(x, -4.62890625));
            assert_eq!(origin, (34. - SEARCH_RADIUS, -5. - SEARCH_RADIUS));
            assert_eq!(rect.left_top, Position::new(origin.0, origin.1));
            let centre = cell_to_position(origin, (GRID / 2, GRID / 2));
            assert_eq!(centre, Position::new(34.5, -4.5), "the tile's own centre");
        }
    }

    #[test]
    fn a_tile_is_blocked_when_the_character_box_at_its_centre_overlaps() {
        let origin = (0., 0.);
        let half = (0.19921875, 0.19921875);
        // An iron chest at (33.5, -6.5) relative to a window at the origin:
        // shift it into the window so tile (3, 3) is the one it sits on.
        let chest = Rect::new(
            &Position::new(3.5 - 0.34765625, 3.5 - 0.34765625),
            &Position::new(3.5 + 0.34765625, 3.5 + 0.34765625),
        );
        let blocked = rasterize([chest].into_iter(), origin, half);
        assert!(blocked[cell_index(3, 3)], "the tile the chest sits on");
        // Its neighbours' centres are a full tile away; a 0.35 + 0.2 reach
        // does not get there.
        assert!(!blocked[cell_index(4, 3)]);
        assert!(!blocked[cell_index(3, 2)]);
    }

    #[test]
    fn an_empty_grid_is_open() {
        let blocked = vec![false; GRID * GRID];
        assert_eq!(fill_from_center(&blocked), Escape::Open);
    }

    #[test]
    fn a_closed_ring_encloses_a_whole_number_of_tiles() {
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
        // A ring two tiles out on every side leaves a 3x3 interior.
        assert_eq!(
            fill_from_center(&blocked),
            Escape::Enclosed { pocket_tiles: 9. }
        );
    }

    #[test]
    fn a_sealed_pocket_is_unreachable_from_the_boundary() {
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
        let safe = reachable_from_boundary(&blocked);
        assert!(!safe[cell_index(cx, cy)]);
        assert!(safe[cell_index(cx + r + 1, cy)]);
    }

    #[test]
    fn bfs_order_is_nearest_first() {
        let blocked = vec![false; GRID * GRID];
        let order = bfs_order_from_center(&blocked);
        assert_eq!(order.len(), GRID * GRID);
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
