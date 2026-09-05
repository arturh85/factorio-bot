# Routed Belts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `connect(from, to, item)` returns the belt path, its directions and the inserters at both ends, or a refusal naming what stopped it — so a drill can feed furnaces and furnaces can fill a chest without a bot carrying every item by hand.

**Architecture:** The grid search lives in `crates/core/src/graph/route.rs` and works on the same one-tile occupancy grid `enclosure::rasterize` already produces; the planner's `method/connect.rs` turns a route into `Place` actions and owns inserter facing. This is the split `enclosure` already uses, and it exists because when this repo had two grids the finer one found cracks the game's pathfinder cannot use and nothing detected the disagreement.

**Tech Stack:** Rust 2024, `crates/core` (graph, types), `crates/planner` (pure, deterministic, no I/O), fixture tests in `crates/core/tests/`, offline `factorio-bot plan`, live `just headless`.

**Spec:** `docs/superpowers/specs/2026-09-05-belt-routing-design.md`

## Global Constraints

- **Every cargo command needs `nix develop -c`.** `pkg-config` and Lua 5.4 come from the flake; a bare `cargo build` dies in `mlua-sys` with a message that reads like a missing system package and is not one.
- **Never `cargo fmt --all`.** Format single files: `rustfmt --edition 2024 <file>`. The edition flag is not optional — bare `rustfmt` defaults to 2015 and dies on every `async fn`.
- **Commit with explicit paths** (`git commit -m "..." -- <paths>`), never `git add -A`, never `--amend`, never `git stash`.
- **`crates/planner` is pure and deterministic**: no I/O, no async, no wall-clock, ordered collections only, floats compared via `total_cmp`. A route must come out identical for identical input.
- **Work in `.worktrees/route` on branch `belt-routing`** with `CARGO_TARGET_DIR=.worktrees/route/target`.
- **An inserter's `direction` points at the side it PICKS UP from.** Established empirically. Getting it backwards places 100% correctly and does nothing.
- **Live runs use `just headless`** (character bots, 5x, own workspace/ports) — never the default ports while another session measures.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/graph/route.rs` | **Create.** The search: A* over the rasterised grid with a turn penalty, undergrounds, and typed refusals. Pure; knows nothing about actions or the planner. |
| `crates/core/src/graph/mod.rs` | **Modify.** Register `pub mod route;`. |
| `crates/core/src/types.rs` | **Modify.** Add `FactorioEntity::new_underground_belt`, beside the existing `new_transport_belt` at :1517. |
| `crates/core/tests/route_grid.rs` | **Create.** Fixture tests over hand-built grids: straight run, detour, under a wall, refusals. |
| `crates/planner/src/method/connect.rs` | **Create.** Inserter facing (one owner, tested) and the method that emits `Place` actions with a materials bill. |
| `crates/planner/src/method/mod.rs` | **Modify.** Register `pub mod connect;`. |
| `CLAUDE.md` | **Modify.** One subsection: what `connect` does, what it refuses, and that it is belts only. |

---

### Task 1: Core — the grid search, surface only

**Files:**
- Create: `crates/core/src/graph/route.rs`
- Modify: `crates/core/src/graph/mod.rs`
- Test: `crates/core/tests/route_grid.rs`

**Interfaces:**
- Consumes: `enclosure::{CELL, GRID, cell_index, cell_to_position, rasterize, window}`, `types::{Direction, Position}`.
- Produces: `Route`, `RouteTile`, `TileKind`, `RouteError`, and
  `pub fn route_belt(blocked: &[bool], origin: (f64, f64), from: (usize, usize), to: (usize, usize), max_underground: Option<u8>) -> Result<Route, RouteError>`.
  Task 2 extends the same function; Task 4 calls it.

- [ ] **Step 1: Write the failing test.** Create `crates/core/tests/route_grid.rs`:

```rust
use factorio_bot_core::graph::enclosure::{GRID, cell_index};
use factorio_bot_core::graph::route::{RouteError, TileKind, route_belt};
use factorio_bot_core::types::Direction;

/// An empty grid: every cell free.
fn open_grid() -> Vec<bool> {
    vec![false; GRID * GRID]
}

/// Block one cell.
fn block(grid: &mut [bool], x: usize, y: usize) {
    grid[cell_index(x, y)] = true;
}

#[test]
fn a_clear_line_is_straight_and_faces_the_destination() {
    let grid = open_grid();
    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), None)
        .expect("an empty grid has a route");

    assert_eq!(route.tiles.len(), 5, "five tiles inclusive of both ends");
    assert!(
        route.tiles.iter().all(|t| t.direction == Direction::East),
        "a straight eastward run faces east the whole way: {:?}",
        route.tiles.iter().map(|t| t.direction).collect::<Vec<_>>()
    );
    assert!(
        route.tiles.iter().all(|t| t.kind == TileKind::Belt),
        "no undergrounds are needed on open ground"
    );
}

#[test]
fn a_blocked_line_goes_around_and_says_so_in_its_directions() {
    let mut grid = open_grid();
    block(&mut grid, 12, 10);

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), None)
        .expect("one blocked cell has a way around it");

    assert!(
        route.tiles.iter().all(|t| t.position.x() != 12.5 || t.position.y() != 10.5),
        "the route must not cross the blocked cell"
    );
    assert!(
        route.tiles.iter().any(|t| t.direction != Direction::East),
        "going around means at least one tile turns"
    );
}

#[test]
fn a_walled_destination_is_refused_by_name() {
    let mut grid = open_grid();
    for y in 9..=11 {
        block(&mut grid, 12, y);
    }
    for x in 11..=13 {
        block(&mut grid, x, 9);
        block(&mut grid, x, 11);
    }

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (12, 10), None)
        .expect_err("a destination behind a wall has no surface route");

    match err {
        RouteError::NoPath { blocked } => assert!(
            !blocked.is_empty(),
            "a refusal must name the tiles that stopped it"
        ),
        other => panic!("expected NoPath, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run it and watch it fail.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-core --test route_grid`
Expected: FAIL — `unresolved import factorio_bot_core::graph::route`.

- [ ] **Step 3: Implement the module.** Create `crates/core/src/graph/route.rs`:

```rust
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
#[derive(PartialEq, Eq)]
struct Node {
    cost: u32,
    estimate: u32,
    cell: (usize, usize),
    facing: Direction,
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // A min-heap out of a max-heap, with a total order on every field so
        // two equal-cost nodes always resolve the same way.
        (other.cost + other.estimate, other.cell, dir_key(other.facing))
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
        heap.push(Node { cost: 0, estimate: heuristic(from, to), cell: from, facing: dir });
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
            let Some(next) = step(node.cell, dx, dy) else { continue };
            if blocked[cell_index(next.0, next.1)] && next != to {
                continue;
            }
            let cost = node.cost + STEP + if dir == node.facing { 0 } else { TURN_PENALTY };
            let next_slot = state_index(next, dir);
            if cost < best[next_slot] {
                best[next_slot] = cost;
                came[next_slot] = Some((node.cell, node.facing));
                heap.push(Node { cost, estimate: heuristic(next, to), cell: next, facing: dir });
            }
        }
    }

    Err(RouteError::NoPath { blocked: blocking_tiles(blocked, origin, to) })
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
        if let Some(n) = step(to, dx, dy) {
            if blocked[cell_index(n.0, n.1)] {
                out.push(cell_to_position(origin, n));
            }
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
        let Some(prev) = came[state_index(cursor.0, cursor.1)] else { break };
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
```

- [ ] **Step 4: Register the module.** In `crates/core/src/graph/mod.rs`, add `pub mod route;` after the `enclosure` line.

- [ ] **Step 5: Run the tests and make them pass.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-core --test route_grid`
Expected: 3 passed.

- [ ] **Step 6: Format and commit.**

```bash
nix develop -c rustfmt --edition 2024 crates/core/src/graph/route.rs
nix develop -c rustfmt --edition 2024 crates/core/tests/route_grid.rs
git commit -m "feat(core): a belt route is searched on the game's own grid, and prefers straight" -- crates/core/src/graph/route.rs crates/core/src/graph/mod.rs crates/core/tests/route_grid.rs
```

---

### Task 2: Core — undergrounds

**Files:**
- Modify: `crates/core/src/graph/route.rs`
- Modify: `crates/core/src/types.rs` (add `new_underground_belt` beside `new_transport_belt` at :1517)
- Test: `crates/core/tests/route_grid.rs`

**Interfaces:**
- Consumes: everything Task 1 produced.
- Produces: `route_belt` now honours `max_underground`, emitting `TileKind::UndergroundEntry` / `UndergroundExit` pairs; `FactorioEntity::new_underground_belt(position: &Position, direction: Direction, output: bool) -> FactorioEntity`.

- [ ] **Step 1: Write the failing tests.** Append to `crates/core/tests/route_grid.rs`:

```rust
#[test]
fn a_wall_is_crossed_underground_when_the_surface_cannot_go_round() {
    let mut grid = open_grid();
    // A full-height wall at x = 12: no surface route exists at all.
    for y in 0..GRID {
        block(&mut grid, 12, y);
    }

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(4))
        .expect("an underground pair crosses a one-tile wall");

    assert_eq!(
        route.tiles.iter().filter(|t| t.kind == TileKind::UndergroundEntry).count(),
        1,
        "exactly one entry"
    );
    assert_eq!(
        route.tiles.iter().filter(|t| t.kind == TileKind::UndergroundExit).count(),
        1,
        "exactly one exit"
    );
    assert!(
        route.tiles.iter().all(|t| t.position.x() != 12.5),
        "nothing is placed inside the wall"
    );
}

#[test]
fn a_wall_wider_than_the_prototype_allows_is_refused_by_span() {
    let mut grid = open_grid();
    for x in 12..=20 {
        for y in 0..GRID {
            block(&mut grid, x, y);
        }
    }

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (22, 10), Some(4))
        .expect_err("a nine-tile wall is wider than a span of four");

    match err {
        RouteError::SpanTooLong { max, .. } => assert_eq!(max, 4),
        other => panic!("expected SpanTooLong, got {other:?}"),
    }
}

#[test]
fn undergrounds_are_not_used_when_the_surface_is_open() {
    let grid = open_grid();
    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(4))
        .expect("open ground routes on the surface");
    assert!(
        route.tiles.iter().all(|t| t.kind == TileKind::Belt),
        "an underground pair costs 2 belts' worth of iron for nothing here"
    );
}
```

- [ ] **Step 2: Run and watch them fail.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-core --test route_grid`
Expected: the three new tests FAIL (surface search ignores `max_underground`).

- [ ] **Step 3: Implement the underground move.** In `route_belt`, replace `let _ = max_underground;` and add a jump move inside the neighbour loop:

```rust
        // An underground pair: enter at `node.cell`, surface `span` tiles on
        // in the same direction. Only in the direction already faced -- a
        // pair that turns is not a thing the game has -- and only over
        // blocked ground, because on open ground it is two belts' worth of
        // iron for nothing.
        if let Some(max) = max_underground {
            for span in 2..=(max as i64 + 1) {
                let Some(exit) = step(node.cell, dx * span, dy * span) else { break };
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
                let exit_slot = state_index(exit, dir);
                if cost < best[exit_slot] {
                    best[exit_slot] = cost;
                    came[exit_slot] = Some((node.cell, node.facing));
                    heap.push(Node {
                        cost,
                        estimate: heuristic(exit, to),
                        cell: exit,
                        facing: dir,
                    });
                }
            }
        }
```

Mark the pair when reconstructing: a step longer than one tile between
consecutive cells is an underground, so in `reconstruct` compare each cell to
its predecessor and set `TileKind::UndergroundEntry` on the earlier tile and
`UndergroundExit` on the later one, leaving the tiles between them unplaced.

To distinguish "no path at all" from "the wall was too wide", record the
widest blocked run encountered on the straight line between `from` and `to`
before searching; if the search fails and that run exceeds `max_underground`,
return `SpanTooLong { needed: run, max }` instead of `NoPath`.

- [ ] **Step 4: Add the entity constructor.** In `crates/core/src/types.rs`, beside `new_transport_belt`:

```rust
    /// `output = true` is the surfacing half of the pair; Factorio calls the
    /// two halves `input` and `output` and they take the same direction.
    pub fn new_underground_belt(
        position: &Position,
        direction: Direction,
        output: bool,
    ) -> FactorioEntity {
        FactorioEntity {
            name: "underground-belt".into(),
            entity_type: "underground-belt".into(),
            position: position.clone(),
            direction: Direction::to_u8(&direction).unwrap_or(0),
            entity_data: if output { Some("output".into()) } else { Some("input".into()) },
            ..Default::default()
        }
    }
```

If `FactorioEntity` has no `entity_data` field, carry the half in the name
only and add a `// FIXME` naming the gap — do not invent a field.

- [ ] **Step 5: Run the tests and make them pass.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-core --test route_grid`
Expected: 6 passed.

- [ ] **Step 6: Format and commit.**

```bash
nix develop -c rustfmt --edition 2024 crates/core/src/graph/route.rs
nix develop -c rustfmt --edition 2024 crates/core/src/types.rs
git commit -m "feat(core): a route goes under a wall it cannot go round, and refuses a span it cannot cross" -- crates/core/src/graph/route.rs crates/core/src/types.rs crates/core/tests/route_grid.rs
```

---

### Task 3: Planner — one owner for inserter facing

**Files:**
- Create: `crates/planner/src/method/connect.rs`
- Modify: `crates/planner/src/method/mod.rs`

**Interfaces:**
- Produces: `pub fn inserter_facing(from: &Position, to: &Position) -> Option<Direction>` — the direction an inserter standing between `from` and `to` must carry to move an item **from** `from` **to** `to`. Task 4 calls it twice per connection.

- [ ] **Step 1: Write the failing test.** Create `crates/planner/src/method/connect.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::types::{Direction, Position};

    /// **The whole reason this function exists.** An inserter's `direction`
    /// names the side it PICKS UP from, not the side it drops into --
    /// established empirically here (chest / burner-inserter / chest, then
    /// machine / inserter / chest). `direction = 12` ("west") is what moves
    /// items *west to east*. Getting it backwards produces a layout that
    /// places 100% correctly, passes every geometry check, and does
    /// absolutely nothing.
    #[test]
    fn an_inserter_faces_the_side_it_picks_up_from() {
        let west = Position::new(0.5, 0.5);
        let east = Position::new(2.5, 0.5);
        assert_eq!(
            inserter_facing(&west, &east),
            Some(Direction::West),
            "moving items west -> east means facing WEST, the pickup side"
        );
        assert_eq!(
            inserter_facing(&east, &west),
            Some(Direction::East),
            "and the reverse faces east"
        );
    }

    #[test]
    fn a_diagonal_has_no_inserter_facing() {
        assert_eq!(
            inserter_facing(&Position::new(0.5, 0.5), &Position::new(2.5, 2.5)),
            None,
            "an inserter is cardinal; a diagonal is a caller bug, not a default"
        );
    }
}
```

- [ ] **Step 2: Run and watch it fail.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner connect`
Expected: FAIL — `cannot find function inserter_facing`.

- [ ] **Step 3: Implement it** at the top of the same file:

```rust
//! Connecting two entities with a belt, and the inserters at each end.

use factorio_bot_core::types::{Direction, Position};

/// The `direction` an inserter must carry to move an item from `from` to `to`.
///
/// **It names the side it picks up from.** See the test; this is the one
/// place in the planner that knows it, so that no caller has to.
pub fn inserter_facing(from: &Position, to: &Position) -> Option<Direction> {
    let dx = to.x() - from.x();
    let dy = to.y() - from.y();
    if dx.abs() > f64::EPSILON && dy.abs() > f64::EPSILON {
        return None;
    }
    if dx.abs() > f64::EPSILON {
        // Moving east means picking up from the west.
        return Some(if dx > 0.0 { Direction::West } else { Direction::East });
    }
    if dy.abs() > f64::EPSILON {
        // Screen coordinates: +y is south. Moving south picks up from north.
        return Some(if dy > 0.0 { Direction::North } else { Direction::South });
    }
    None
}
```

- [ ] **Step 4: Register the module.** Add `pub mod connect;` to `crates/planner/src/method/mod.rs`.

- [ ] **Step 5: Run the tests and make them pass.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner connect`
Expected: 2 passed.

- [ ] **Step 6: Format and commit.**

```bash
nix develop -c rustfmt --edition 2024 crates/planner/src/method/connect.rs
git commit -m "feat(planner): one owner for inserter facing, and it names the pickup side" -- crates/planner/src/method/connect.rs crates/planner/src/method/mod.rs
```

---

### Task 4: Planner — emit the connection

**Files:**
- Modify: `crates/planner/src/method/connect.rs`

**Interfaces:**
- Consumes: `route::{route_belt, Route, RouteTile, TileKind, RouteError}`, `enclosure::{window, rasterize}`, `inserter_facing` from Task 3, `PlanState::base()` and `base.entity_graph.blocking_boxes_within(area)`, `FactorioEntity::{new_transport_belt, new_underground_belt, new_inserter}`, `ActionKind::Place`.
- Produces: `pub fn connect_steps(state: &PlanState, from: &Position, to: &Position, item: &ItemId) -> Result<Vec<ActionKind>, ConnectRefusal>` and `pub enum ConnectRefusal { NoRoute { blocked: Vec<Position> }, SpanTooLong { needed: u32, max: u8 }, NotCardinal }`.

- [ ] **Step 1: Write the failing test.** Append to the test module in `connect.rs`:

```rust
    /// The bill is the deliverable: a caller has to be able to refuse the
    /// whole plan before a single belt is placed, which is what the
    /// preconditions on these actions are for.
    #[test]
    fn a_connection_places_belts_and_an_inserter_at_each_end() {
        let state = crate::test_world::open_world_with_two_machines();
        let steps = connect_steps(
            &state,
            &Position::new(0.5, 0.5),
            &Position::new(6.5, 0.5),
            &"iron-ore".into(),
        )
        .expect("open ground between two machines connects");

        let placed: Vec<&str> = steps
            .iter()
            .filter_map(|k| match k {
                ActionKind::Place { entity } => Some(entity.name.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(
            placed.iter().filter(|n| **n == "inserter").count(),
            2,
            "one to load the belt and one to unload it: {placed:?}"
        );
        assert!(
            placed.iter().any(|n| *n == "transport-belt"),
            "and belt between them: {placed:?}"
        );
    }

    #[test]
    fn a_walled_destination_refuses_and_places_nothing() {
        let state = crate::test_world::two_machines_behind_a_wall();
        let refusal = connect_steps(
            &state,
            &Position::new(0.5, 0.5),
            &Position::new(6.5, 0.5),
            &"iron-ore".into(),
        )
        .expect_err("a walled destination has no route");
        assert!(matches!(refusal, ConnectRefusal::NoRoute { .. }));
    }
```

If `crate::test_world` has no such fixtures, add them there following the
existing constructors in that file — an open world with two `stone-furnace`
entities six tiles apart, and the same with a `stone-wall` column between
them. Do not invent a new fixture module.

- [ ] **Step 2: Run and watch it fail.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner connect`
Expected: FAIL — `cannot find function connect_steps`.

- [ ] **Step 3: Implement it:**

```rust
/// Why a connection could not be made. **Every variant is returned before
/// anything is placed** -- a half-built belt run is worse than no belt run,
/// because the items sit on it and the bot that would carry them is gone.
#[derive(Debug, Clone)]
pub enum ConnectRefusal {
    NoRoute { blocked: Vec<Position> },
    SpanTooLong { needed: u32, max: u8 },
    NotCardinal,
}

pub fn connect_steps(
    state: &PlanState,
    from: &Position,
    to: &Position,
    item: &ItemId,
) -> Result<Vec<ActionKind>, ConnectRefusal> {
    let _ = item; // the item decides nothing about geometry; it is the caller's label
    let (area, origin) = enclosure::window(from);
    let blocked = enclosure::rasterize(
        state.base().entity_graph.blocking_boxes_within(&area),
        origin,
        (0.0, 0.0),
    );
    let max_underground = state
        .base()
        .entity_prototypes
        .get("underground-belt")
        .and_then(|p| p.max_underground_distance);

    let start = cell_of(origin, from);
    let end = cell_of(origin, to);
    let route = route_belt(&blocked, origin, start, end, max_underground).map_err(|e| match e {
        RouteError::NoPath { blocked } => ConnectRefusal::NoRoute { blocked },
        RouteError::SpanTooLong { needed, max } => ConnectRefusal::SpanTooLong { needed, max },
    })?;

    let mut steps = Vec::new();
    // Load: an inserter between the source and the first belt tile.
    let first = route.tiles.first().ok_or(ConnectRefusal::NotCardinal)?;
    let facing = inserter_facing(from, &first.position).ok_or(ConnectRefusal::NotCardinal)?;
    steps.push(ActionKind::Place {
        entity: Box::new(FactorioEntity::new_inserter(from, facing)),
    });
    for tile in &route.tiles {
        let entity = match tile.kind {
            TileKind::Belt => FactorioEntity::new_transport_belt(&tile.position, tile.direction),
            TileKind::UndergroundEntry => {
                FactorioEntity::new_underground_belt(&tile.position, tile.direction, false)
            }
            TileKind::UndergroundExit => {
                FactorioEntity::new_underground_belt(&tile.position, tile.direction, true)
            }
        };
        steps.push(ActionKind::Place { entity: Box::new(entity) });
    }
    // Unload: an inserter between the last belt tile and the destination.
    let last = route.tiles.last().expect("checked non-empty above");
    let facing = inserter_facing(&last.position, to).ok_or(ConnectRefusal::NotCardinal)?;
    steps.push(ActionKind::Place {
        entity: Box::new(FactorioEntity::new_inserter(to, facing)),
    });
    Ok(steps)
}

/// Which cell of the window a position falls in.
fn cell_of(origin: (f64, f64), at: &Position) -> (usize, usize) {
    (
        ((at.x() - origin.0) / enclosure::CELL) as usize,
        ((at.y() - origin.1) / enclosure::CELL) as usize,
    )
}
```

- [ ] **Step 4: Run the tests and make them pass.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner connect`
Expected: 4 passed.

- [ ] **Step 5: Run the whole planner suite** — it is pure and fast, and this is where a determinism break shows up.

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test -p factorio-bot-planner`
Expected: all pass.

- [ ] **Step 6: Format and commit.**

```bash
nix develop -c rustfmt --edition 2024 crates/planner/src/method/connect.rs
git commit -m "feat(planner): connect emits a belt run and its two inserters, or refuses before placing" -- crates/planner/src/method/connect.rs
```

---

### Task 5: Verify offline, then live, then write it down

**Files:**
- Modify: `CLAUDE.md`
- Create: `docs/superpowers/notes/2026-09-05-belt-routing-first-run.md`

- [ ] **Step 1: Check the whole workspace is green.**

Run: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings`
Then: `CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test --workspace`
Expected: both clean. Fix anything that is not before going near a game.

- [ ] **Step 2: Look at a route offline, against the baseline dump.** Seconds, no game:

```bash
nix develop -c ./target/debug/factorio-bot plan \
  --world workspace/scripts/map-t0-baseline.json \
  --goal producing:iron-plate:60 --bots 1,2,3,4 --steps
```

Expected: the step list now contains `place transport-belt` and `place inserter` entries. **Record the action count and makespan before and after** — a routed connection that makes the plan longer is a finding, not a failure, and it belongs in the note.

- [ ] **Step 3: Run it live, headless.** A run is ~3 minutes now.

```bash
nix develop -c ./target/debug/factorio-bot lua factory_stage2.lua \
  --headless --bots 4 --game-speed 5 --seed 31337 --new \
  --settings <your own settings file on ports 34200/4324>
```

**Check the ports are free and no other session is measuring first.**

- [ ] **Step 4: Confirm the belts actually moved something.** A placement count is not evidence — `only_ghosts` validates nothing and a correct-looking layout with a backwards inserter does nothing at all. Read the run record:

```bash
python3 tools/run_analysis.py <run dir>
grep -o '"name":"transport-belt"' <run dir>/map.jsonl | wc -l
```

Expected: belts in the entity map, and the destination chest's contents rising in `samples.jsonl` across consecutive samples.

- [ ] **Step 5: Write the note.** `docs/superpowers/notes/2026-09-05-belt-routing-first-run.md`: what the offline plan changed, what the live run did, and **anything that did not work**. If the inserters faced the wrong way, say so and say how it was seen — that failure is silent by construction.

- [ ] **Step 6: Document it in CLAUDE.md**, one subsection under the planner crate: what `connect` does, that it is belts only, that it refuses rather than half-builds, and that inserter facing has exactly one owner.

- [ ] **Step 7: Commit and merge.**

```bash
git commit -m "docs: routed belts, measured" -- CLAUDE.md docs/superpowers/notes/2026-09-05-belt-routing-first-run.md
# from the main checkout, after confirming nobody has uncommitted work in the touched files:
git merge --no-ff belt-routing
```

---

## Self-Review

**Spec coverage.** Grid search in core (Task 1), undergrounds bounded by the prototype (Task 2), one owner for inserter facing (Task 3), action emission with refusal-before-build (Task 4), fixture tests (Tasks 1-2), offline plan and live headless run and docs (Task 5). The spec's "not in this version" list — splitters, balancing, throughput sizing, fluid pipes — appears in no task, which is correct.

**Known soft spots, stated rather than hidden.** Task 2's `SpanTooLong` detection is described rather than written out, because the exact form depends on how the search fails; the step says what it must distinguish and why. Task 4 assumes `crate::test_world` gains two fixtures and says to follow the existing constructors rather than invent a module. Task 2 says explicitly not to invent an `entity_data` field if it does not exist.

**Type consistency.** `route_belt` keeps one signature across Tasks 1, 2 and 4. `TileKind` variants are named identically in the core module, the tests and the planner match. `inserter_facing` returns `Option<Direction>` in Task 3 and is consumed as an `Option` in Task 4.
