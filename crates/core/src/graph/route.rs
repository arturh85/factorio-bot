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

use crate::graph::enclosure::{CELL, GRID, cell_index, cell_to_position};
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
/// cost of leaving depends on it.
#[derive(PartialEq)]
struct Node {
    cost: u32,
    estimate: u32,
    cell: (usize, usize),
    facing: Direction,
}

// `Direction` derives `PartialEq` only (see `types.rs`), not `Eq`, so `Node`
// cannot derive `Eq` either. The derived `PartialEq` above never compares a
// float (cost/estimate are `u32`, cell is `(usize, usize)`, facing is a
// fieldless-discriminant enum), so it is already reflexive, symmetric and
// transitive -- a legitimate manual `Eq`.
impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // A min-heap out of a max-heap, with a total order on every field so
        // two equal-cost nodes always resolve the same way.
        (
            other.cost + other.estimate,
            other.cell,
            dir_key(other.facing),
        )
            .cmp(&(self.cost + self.estimate, self.cell, dir_key(self.facing)))
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
    let _ = max_underground; // Task 2.
    let mut best: Vec<u32> = vec![u32::MAX; GRID * GRID * 4];
    let mut came: Vec<Option<((usize, usize), Direction)>> = vec![None; GRID * GRID * 4];
    let mut heap = BinaryHeap::new();

    for (dir, _) in DIRECTIONS {
        let slot = state_index(from, dir);
        best[slot] = 0;
        heap.push(Node {
            cost: 0,
            estimate: heuristic(from, to),
            cell: from,
            facing: dir,
        });
    }

    while let Some(node) = heap.pop() {
        if node.cell == to {
            return Ok(reconstruct(&came, origin, from, to, node.facing));
        }
        let slot = state_index(node.cell, node.facing);
        if node.cost > best[slot] {
            continue;
        }
        for (dir, (dx, dy)) in DIRECTIONS {
            let Some(next) = step(node.cell, dx, dy) else {
                continue;
            };
            if blocked[cell_index(next.0, next.1)] {
                continue;
            }
            let cost = node.cost + STEP + if dir == node.facing { 0 } else { TURN_PENALTY };
            let next_slot = state_index(next, dir);
            if cost < best[next_slot] {
                best[next_slot] = cost;
                came[next_slot] = Some((node.cell, node.facing));
                heap.push(Node {
                    cost,
                    estimate: heuristic(next, to),
                    cell: next,
                    facing: dir,
                });
            }
        }
    }

    Err(RouteError::NoPath {
        blocked: blocking_tiles(blocked, origin, to),
    })
}

fn state_index(cell: (usize, usize), facing: Direction) -> usize {
    cell_index(cell.0, cell.1) * 4 + (dir_key(facing) / 4) as usize
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

/// The occupied tiles touching the destination, for the refusal message.
fn blocking_tiles(blocked: &[bool], origin: (f64, f64), to: (usize, usize)) -> Vec<Position> {
    let mut out = Vec::new();
    for (_, (dx, dy)) in DIRECTIONS {
        if let Some(n) = step(to, dx, dy)
            && blocked[cell_index(n.0, n.1)]
        {
            out.push(cell_to_position(origin, n));
        }
    }
    out
}

fn reconstruct(
    came: &[Option<((usize, usize), Direction)>],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    facing: Direction,
) -> Route {
    let mut cells = vec![(to, facing)];
    let mut cursor = (to, facing);
    while cursor.0 != from {
        let Some(prev) = came[state_index(cursor.0, cursor.1)] else {
            break;
        };
        cursor = prev;
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
    let _ = CELL;
    Route { tiles }
}
