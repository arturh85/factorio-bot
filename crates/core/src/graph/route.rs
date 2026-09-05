//! A belt run between two tiles, on the grid the game's pathfinder uses.
//!
//! **Why this is not in the planner.** `enclosure` already learned this here:
//! when prevention and detection had two different grids, the finer one found
//! cracks the game cannot route through and nothing detected the
//! disagreement. A belt is placed on the same one-tile lattice, so it is
//! searched on the same one, from the same `rasterize`.
//!
//! Pure: no I/O, no clock, and a given grid always produces the same route.
//! Ties are broken by a fixed cell order, never by hash iteration.

use crate::graph::enclosure::{GRID, cell_index, cell_to_position};
use crate::types::{Direction, Position};
use num_traits::ToPrimitive;
use std::collections::BinaryHeap;

/// What a tile in a route is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileKind {
    Belt,
    /// Task 2 fills these in; a surface-only route never emits them.
    UndergroundEntry,
    UndergroundExit,
}

#[derive(Debug, Clone)]
pub struct RouteTile {
    pub position: Position,
    pub direction: Direction,
    pub kind: TileKind,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub tiles: Vec<RouteTile>,
}

#[derive(Debug, Clone)]
pub enum RouteError {
    /// No route on the surface, and none underground either. `blocked` names
    /// the occupied tiles adjacent to the frontier the search died on -- the
    /// reader wants to know *what* stopped it, not merely that something did.
    NoPath { blocked: Vec<Position> },
    /// An underground span longer than the prototype allows.
    SpanTooLong { needed: u32, max: u8 },
}

/// One step costs this; a step that turns costs this plus [`TURN_PENALTY`].
const STEP: u32 = 10;

/// **Straightness is not cosmetic.** Every corner is a belt whose direction
/// differs from its neighbours', and direction is the part a caller gets
/// wrong. A penalty of six tenths of a step makes the search prefer a longer
/// straight run to a shorter staircase without ever preferring a detour that
/// costs more than one extra tile per corner.
const TURN_PENALTY: u32 = 6;

/// Search state: a cell plus the direction we entered it from, because the
/// cost of leaving depends on it, plus whether this cell is the surfacing
/// end of an underground pair (`surfaced`). A surfaced cell cannot launch
/// another jump -- see the note on the underground move below -- so it is
/// part of the state, not just an annotation on the route afterwards.
struct Node {
    cost: u32,
    estimate: u32,
    cell: (usize, usize),
    facing: Direction,
    surfaced: bool,
}

// `PartialEq` is written in terms of `Ord`, not derived. A derived one
// compares `cost` and `estimate` separately, while `Ord` below compares only
// their *sum* alongside the other three fields -- so two nodes with the same
// total but a different split (cost 10/estimate 20 against 20/10) are
// `Ord`-equal and derived-`PartialEq`-unequal at the same time. Nothing in
// this file invokes `==` on a `Node`, so that inconsistency has never had a
// chance to matter, but `BinaryHeap` is entitled to assume the two agree and
// the cheapest fix is to make them one definition.
//
// `Direction` derives `PartialEq` only (see `types.rs`), not `Eq`, so `Node`
// cannot derive either trait. `cmp` never compares a float (cost/estimate are
// `u32`, cell is `(usize, usize)`, facing goes through `dir_key` and surfaced
// is a bool), so it is a genuine total order and this is a legitimate manual
// `Eq`.
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // A min-heap out of a max-heap, with a total order on every field so
        // two equal-cost nodes always resolve the same way.
        (
            other.cost + other.estimate,
            other.cell,
            dir_key(other.facing),
            other.surfaced,
        )
            .cmp(&(
                self.cost + self.estimate,
                self.cell,
                dir_key(self.facing),
                self.surfaced,
            ))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn dir_key(d: Direction) -> u8 {
    Direction::to_u8(&d).unwrap_or(0)
}

/// One search-state predecessor: the cell and facing it came from, plus
/// whether *that* predecessor cell was itself the surfacing end of a jump.
type Predecessor = ((usize, usize), Direction, bool);

const DIRECTIONS: [(Direction, (i64, i64)); 4] = [
    (Direction::North, (0, -1)),
    (Direction::East, (1, 0)),
    (Direction::South, (0, 1)),
    (Direction::West, (-1, 0)),
];

/// Route a belt from `from` to `to` across `blocked`.
///
/// `origin` is the window origin `enclosure::window` returned for the same
/// grid; it is only used to turn cells back into positions.
///
/// `max_underground` is the belt prototype's `max_underground_distance`.
/// `None` means undergrounds are not available and the search is surface-only.
pub fn route_belt(
    blocked: &[bool],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    max_underground: Option<u8>,
) -> Result<Route, RouteError> {
    let mut best: Vec<u32> = vec![u32::MAX; GRID * GRID * 8];
    let mut came: Vec<Option<Predecessor>> = vec![None; GRID * GRID * 8];
    let mut heap = BinaryHeap::new();
    // Every cell the search actually visited, for an honest refusal message
    // if it never reaches `to`.
    let mut reached: Vec<bool> = vec![false; GRID * GRID];

    for (dir, _) in DIRECTIONS {
        let slot = state_index(from, dir, false);
        best[slot] = 0;
        heap.push(Node {
            cost: 0,
            estimate: heuristic(from, to),
            cell: from,
            facing: dir,
            surfaced: false,
        });
    }

    while let Some(node) = heap.pop() {
        reached[cell_index(node.cell.0, node.cell.1)] = true;
        if node.cell == to {
            return Ok(reconstruct(
                &came,
                origin,
                from,
                to,
                node.facing,
                node.surfaced,
            ));
        }
        let slot = state_index(node.cell, node.facing, node.surfaced);
        if node.cost > best[slot] {
            continue;
        }
        for (dir, (dx, dy)) in DIRECTIONS {
            // The normal move and the underground move are independent
            // options out of `node.cell` in this direction, not a fallback
            // chain: a blocked (or off-grid) immediate neighbour must not
            // suppress an underground attempt in the same direction -- that
            // is exactly the case where a jump is needed, when the wall
            // starts on the very next tile. Each is therefore its own `if`,
            // never a shared early `continue`.
            if let Some(next) = step(node.cell, dx, dy)
                && !blocked[cell_index(next.0, next.1)]
            {
                let cost = node.cost + STEP + if dir == node.facing { 0 } else { TURN_PENALTY };
                // A normal step always lands on an ordinary tile: whether or
                // not `node` itself was a jump's exit, the tile we are
                // stepping onto now is ground, not another underground
                // entity, so `surfaced` resets to `false` here regardless of
                // `node.surfaced`.
                let next_slot = state_index(next, dir, false);
                if cost < best[next_slot] {
                    best[next_slot] = cost;
                    came[next_slot] = Some((node.cell, node.facing, node.surfaced));
                    heap.push(Node {
                        cost,
                        estimate: heuristic(next, to),
                        cell: next,
                        facing: dir,
                        surfaced: false,
                    });
                }
            }

            // An underground pair: enter at `node.cell`, surface `span` tiles
            // on in the same direction. The pair itself never turns -- that
            // is not a thing the game has -- and it is only worth taking
            // over ground that is actually blocked, because on open ground a
            // pair costs two belts' worth of iron for a run a single surface
            // tile would cover for free.
            //
            // Gated on `!node.surfaced`: `node.cell` already holds a real
            // underground-belt entity (the *output* half of the previous
            // jump) whenever `node.surfaced` is true, and a tile can only
            // ever hold one entity. Launching a second jump from that same
            // tile would need it to simultaneously be an output and an
            // input, which the game cannot build and which this search must
            // not either -- without this gate, two jumps separated by
            // nothing at all silently collapse onto one shared tile during
            // reconstruction, and one of the two roles (`UndergroundEntry` /
            // `UndergroundExit`) is overwritten and lost. A normal surface
            // step first (handled above, and it always clears `surfaced`)
            // is what makes a fresh launch tile legal again.
            if !node.surfaced
                && let Some(max) = max_underground
            {
                for span in 2..=(max as i64 + 1) {
                    let Some(exit) = step(node.cell, dx * span, dy * span) else {
                        break;
                    };
                    if blocked[cell_index(exit.0, exit.1)] {
                        continue;
                    }
                    let crosses_blocked = (1..span).any(|i| {
                        step(node.cell, dx * i, dy * i)
                            .map(|c| blocked[cell_index(c.0, c.1)])
                            .unwrap_or(false)
                    });
                    if !crosses_blocked {
                        continue;
                    }
                    let cost = node.cost
                        + STEP * span as u32
                        + if dir == node.facing { 0 } else { TURN_PENALTY };
                    let exit_slot = state_index(exit, dir, true);
                    if cost < best[exit_slot] {
                        best[exit_slot] = cost;
                        came[exit_slot] = Some((node.cell, node.facing, node.surfaced));
                        heap.push(Node {
                            cost,
                            estimate: heuristic(exit, to),
                            cell: exit,
                            facing: dir,
                            surfaced: true,
                        });
                    }
                }
            }
        }
    }

    if let Some(max) = max_underground {
        let widest = widest_blocked_run(blocked, from, to);
        if widest > max as u32 {
            return Err(RouteError::SpanTooLong {
                needed: widest,
                max,
            });
        }
    }

    Err(RouteError::NoPath {
        blocked: blocking_tiles(blocked, origin, &reached),
    })
}

/// The longest run of consecutive blocked cells on the straight (Bresenham)
/// line from `from` to `to`.
///
/// This is what tells a refusal whether the obstacle was a wall an
/// underground pair simply is not long enough for, versus some other reason
/// the search never reached `to` -- a chokepoint the wall check would not
/// see, or a walled-off destination reachable in a straight line but sealed
/// on every side. Only the first of those is `SpanTooLong`; getting this
/// wrong by *under*-reporting (falling back to `NoPath`) is far cheaper than
/// blaming a span that was never the problem, so this only ever looks at the
/// direct line, never claims a run it did not actually measure, and a caller
/// still gets `NoPath` whenever this comes back at or under `max`.
fn widest_blocked_run(blocked: &[bool], from: (usize, usize), to: (usize, usize)) -> u32 {
    let (mut x, mut y) = (from.0 as i64, from.1 as i64);
    let (x1, y1) = (to.0 as i64, to.1 as i64);
    let adx = (x1 - x).abs();
    let ady = -(y1 - y).abs();
    let sx: i64 = if x < x1 { 1 } else { -1 };
    let sy: i64 = if y < y1 { 1 } else { -1 };
    let mut err = adx + ady;

    let mut widest = 0u32;
    let mut current = 0u32;
    loop {
        if blocked[cell_index(x as usize, y as usize)] {
            current += 1;
            widest = widest.max(current);
        } else {
            current = 0;
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= ady {
            err += ady;
            x += sx;
        }
        if e2 <= adx {
            err += adx;
            y += sy;
        }
    }
    widest
}

fn state_index(cell: (usize, usize), facing: Direction, surfaced: bool) -> usize {
    cell_index(cell.0, cell.1) * 8 + (dir_key(facing) / 4) as usize * 2 + surfaced as usize
}

fn step(cell: (usize, usize), dx: i64, dy: i64) -> Option<(usize, usize)> {
    let x = cell.0 as i64 + dx;
    let y = cell.1 as i64 + dy;
    if x < 0 || y < 0 || x >= GRID as i64 || y >= GRID as i64 {
        return None;
    }
    Some((x as usize, y as usize))
}

fn heuristic(a: (usize, usize), b: (usize, usize)) -> u32 {
    let dx = a.0.abs_diff(b.0) as u32;
    let dy = a.1.abs_diff(b.1) as u32;
    (dx + dy) * STEP
}

/// The occupied tiles touching the region the search actually reached, for
/// an honest refusal message.
///
/// The naive version of this only looked at the four neighbours of `to`,
/// which is wrong whenever the wall that stopped the search is further out,
/// or the failure is a chokepoint mid-route rather than a walled
/// destination: the destination's own neighbours can all be free while the
/// search still never got there. Scanning every reached cell's neighbours
/// names the tiles that actually stopped the frontier, wherever they are.
///
/// A `(usize, usize)` set, not a `Position` one: `Position` holds `f64` and
/// cannot be deduplicated or ordered, so cells are deduplicated on the grid
/// first and only converted to positions -- in ascending `(x, y)` order, a
/// fixed order rather than an artefact of traversal -- once that is done. An
/// empty result is honest here: it means the search reached the map edge
/// without ever bordering a blocked cell, i.e. nothing on the grid stopped
/// it.
fn blocking_tiles(blocked: &[bool], origin: (f64, f64), reached: &[bool]) -> Vec<Position> {
    let mut cells: Vec<(usize, usize)> = Vec::new();
    for y in 0..GRID {
        for x in 0..GRID {
            if !reached[cell_index(x, y)] {
                continue;
            }
            for (_, (dx, dy)) in DIRECTIONS {
                if let Some(n) = step((x, y), dx, dy)
                    && blocked[cell_index(n.0, n.1)]
                {
                    cells.push(n);
                }
            }
        }
    }
    cells.sort_unstable();
    cells.dedup();
    cells
        .into_iter()
        .map(|c| cell_to_position(origin, c))
        .collect()
}

fn reconstruct(
    came: &[Option<Predecessor>],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    facing: Direction,
    surfaced: bool,
) -> Route {
    let mut cells = vec![(to, facing)];
    let mut cursor = (to, facing);
    let mut cursor_surfaced = surfaced;
    while cursor.0 != from {
        let Some((prev_cell, prev_facing, prev_surfaced)) =
            came[state_index(cursor.0, cursor.1, cursor_surfaced)]
        else {
            break;
        };
        cursor = (prev_cell, prev_facing);
        cursor_surfaced = prev_surfaced;
        cells.push(cursor);
    }
    cells.reverse();
    // A belt faces the way the item leaves it, so each tile takes the
    // direction of the step *out* of it; the last tile keeps the one it
    // arrived with.
    let mut tiles = Vec::with_capacity(cells.len());
    for (i, (cell, _)) in cells.iter().enumerate() {
        let direction = cells.get(i + 1).map(|(_, d)| *d).unwrap_or(facing);
        tiles.push(RouteTile {
            position: cell_to_position(origin, *cell),
            direction,
            kind: TileKind::Belt,
        });
    }
    // A step further than one tile from its predecessor is an underground
    // jump: the earlier tile is where the pair dives, the later one is where
    // it surfaces, and everything in between is never placed at all.
    for i in 1..cells.len() {
        let (prev, _) = cells[i - 1];
        let (cur, _) = cells[i];
        let dist = prev.0.abs_diff(cur.0) + prev.1.abs_diff(cur.1);
        if dist > 1 {
            tiles[i - 1].kind = TileKind::UndergroundEntry;
            tiles[i].kind = TileKind::UndergroundExit;
        }
    }
    Route { tiles }
}
