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
    /// The half where a pair dives (Factorio's `input`); a surface-only
    /// route (`max_underground_distance: None`) never emits one.
    UndergroundEntry,
    /// The half where it surfaces (`output`).
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
    /// An underground span longer than the prototype allows. Both numbers are
    /// in the prototype's own unit, the distance from the entry half to the
    /// exit half (`max_underground_distance`, see [`route_belt`]): `needed`
    /// is the widest wall on the direct line plus one, i.e. the shortest
    /// pair that could cross it.
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

/// What an underground pair costs over and above the tiles it spans.
///
/// A pair is a last resort, not a shortcut: in the base recipes one
/// `underground-belt` craft is 10 iron plates and 5 belts for two halves --
/// about 25 plates -- against 1.5 plates per surface belt, so a pair is worth
/// roughly sixteen belt tiles of iron, and it also reserves the ground it
/// runs beneath for nothing else. Sixteen steps' worth makes the search
/// prefer any detour shorter than that to tunnelling, and step around a
/// tree rather than under it, while still crossing a wall the surface
/// cannot pass. This is a preference weight for a search, not a game
/// constant: a mod that reprices the recipe moves where the search's
/// indifference point sits, not whether it can route.
const UNDERGROUND_PENALTY: u32 = 16 * STEP;

/// A hidden tile costs this on top of [`STEP`], so that of two jumps that
/// both clear the wall the shorter one wins: a longer span reserves more
/// ground beneath it and is otherwise the same price.
const HIDDEN_TILE_COST: u32 = 1;

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

/// `a` is the exact opposite of `b`, by the deltas in [`DIRECTIONS`].
fn reverses(a: Direction, b: Direction) -> bool {
    let delta = |d: Direction| DIRECTIONS.iter().find(|(x, _)| *x == d).map(|(_, v)| *v);
    match (delta(a), delta(b)) {
        (Some((ax, ay)), Some((bx, by))) => ax == -bx && ay == -by,
        _ => false,
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

/// Bit set in a tunnel grid cell for a tunnel running east-west.
pub const TUNNEL_EW: u8 = 0b01;
/// Bit set in a tunnel grid cell for a tunnel running north-south.
pub const TUNNEL_NS: u8 = 0b10;

/// The tunnel-grid bit an underground pair running in `direction` occupies.
pub fn tunnel_axis(direction: Direction) -> u8 {
    match direction {
        Direction::East | Direction::West => TUNNEL_EW,
        _ => TUNNEL_NS,
    }
}

/// Route a belt from `from` to `to` across `blocked`, with no existing
/// tunnels to keep clear of. See [`route_belt_with_tunnels`].
pub fn route_belt(
    blocked: &[bool],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    max_underground_distance: Option<u8>,
) -> Result<Route, RouteError> {
    let tunnels = vec![0u8; GRID * GRID];
    route_belt_with_tunnels(
        blocked,
        &tunnels,
        origin,
        from,
        to,
        max_underground_distance,
    )
}

/// Route a belt from `from` to `to` across `blocked`.
///
/// `origin` is the window origin `enclosure::window` returned for the same
/// grid; it is only used to turn cells back into positions.
///
/// `max_underground_distance` is the belt prototype's field of that name,
/// **in the prototype's own unit and untranslated**: the distance from the
/// entry half to the exit half, so a value of 5 (`underground-belt` on this
/// install; `fast-` is 7 and `turbo-` 11) lets a pair dive at `x` and
/// surface at `x + 5`, hiding four tiles. An earlier version of this
/// parameter counted the hidden tiles instead, which is that same number
/// minus one, under a name and a doc that said it was the prototype's
/// field -- exactly the mismatch a caller reading the prototype would then
/// pass through unconverted and overshoot by a tile. `None` means
/// undergrounds are not available and the search is surface-only.
///
/// `tunnels` is a `GRID x GRID` grid of [`TUNNEL_EW`] / [`TUNNEL_NS`] bits
/// naming the cells an existing underground pair of this same belt already
/// runs beneath, its two halves included. A new pair may cross one of those
/// at right angles but never along the same axis: the game pairs an input
/// with the first same-type half it meets on its line, so a jump laid along
/// an existing tunnel would connect to the wrong half. Belt weaving works
/// only across tiers for the same reason, and this search has one tier.
///
/// # Two shape rules the game imposes on a pair, both enforced in the search
///
/// * **A jump is only launched straight.** The entry half faces the tunnel,
///   and the belt feeding it must arrive from behind: a belt arriving from
///   the side would side-load the entry and fill one lane. So a jump in
///   direction `d` is offered only from a state already facing `d`; the tile
///   before the entry may itself be a corner (a corner belt keeps both
///   lanes), the entry may not.
/// * **The tile after an exit is straight too.** The exit half emits onto
///   the tile in front of it and nowhere else, so from a surfaced state the
///   only move is one more step in the same direction. Without this, the
///   search would happily surface at a tile and turn, leaving an output half
///   pointing at ground no belt stands on -- a route that places perfectly
///   and moves nothing.
///
/// Neither rule changes a surface-only search: both only gate moves that
/// involve a jump.
///
/// # A route that meets itself is repaired, not refused
///
/// The state space is `(cell, facing, surfaced)`, so A* is free to return to
/// a cell it already stands on with a different facing -- and with a jump on
/// offer it will, because a jump is only launched straight: a run that
/// needs to dive west out of a corridor it entered heading south turns a
/// three-tile hook to arrive facing west, and the hook that turns back
/// north lands its entry half on a tile the run already laid a belt on.
/// Measured on `run-1788926478-07032`: the haul out of a sealed coal pocket
/// came back as ten tiles with `[30.5, -39.5]` both a south-facing belt and
/// a west-facing entry, and the refusal named that one tile as "blocked" --
/// a tile the record showed EMPTY, and one the planner's own route was
/// standing on twice. The replan died on it with eighty steps abandoned.
///
/// So a route whose own tiles collide ([`self_crossing`]) is not the answer
/// and not a refusal either. **The move that doubled the tile is forbidden
/// and the search runs again.** Not the tile: closing the cell was tried
/// first and moved the hook one row south per round, five rounds without
/// converging, because the hook that turns *south* instead is exactly as
/// cheap and only loses the tie -- forbidding the one step that lands on
/// the doubled cell is what lets the tied legal hook win. Each round
/// forbids a move the last route took, so every round's route is new and
/// [`SELF_CROSSING_REPAIRS`] bounds the work. When the rounds run out, or
/// a round finds nothing at all, the refusal names the obstacles the last
/// frontier touched on the caller's grid -- never the route's own cells,
/// which are not obstacles and send a reader looking for one that is not
/// there.
pub fn route_belt_with_tunnels(
    blocked: &[bool],
    tunnels: &[u8],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    max_underground_distance: Option<u8>,
) -> Result<Route, RouteError> {
    route_with(
        blocked,
        tunnels,
        origin,
        from,
        to,
        max_underground_distance,
        None,
    )
}

/// [`route_belt_with_tunnels`], with the first tile's facing FIXED.
///
/// The route's first tile is `from`, and it faces `launch`: the only move
/// offered out of `from` is the step (or the jump) in that direction, so a
/// turn on the first tile is never searched, never repaired round, and never
/// returned. Everything after the first tile is the ordinary search.
///
/// # Who needs this
///
/// A caller whose `from` is the OUTPUT of something that emits in a fixed
/// direction -- a splitter's second output, in `method::connect`'s belt tap.
/// A splitter pushes items straight ahead onto whatever stands on its output
/// tile; a belt there facing sideways is side-loaded, which takes one lane
/// and is the same "places perfectly and moves nothing" shape the underground
/// exit's straight-tile rule exists for (see [`route_belt`]'s doc). The
/// plain search seeds all four facings at `from` at cost zero and takes
/// whichever turn is cheapest, so it cannot promise the first tile continues
/// the emitter's direction; this one can, by construction rather than by
/// checking afterwards and refusing a route that was otherwise fine.
///
/// `from == to` is answered with the one tile facing `launch`.
pub fn route_belt_launching(
    blocked: &[bool],
    tunnels: &[u8],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    max_underground_distance: Option<u8>,
    launch: Direction,
) -> Result<Route, RouteError> {
    route_with(
        blocked,
        tunnels,
        origin,
        from,
        to,
        max_underground_distance,
        Some(launch),
    )
}

/// The one search behind [`route_belt_with_tunnels`] and
/// [`route_belt_launching`]: `launch` is `None` for the free start every
/// route had before the launching form existed, and the fixed first facing
/// otherwise.
fn route_with(
    blocked: &[bool],
    tunnels: &[u8],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    max_underground_distance: Option<u8>,
    launch: Option<Direction>,
) -> Result<Route, RouteError> {
    debug_assert_eq!(tunnels.len(), GRID * GRID, "one tunnel byte per cell");
    // Empty on the first round, so a route that never meets itself -- every
    // route this function returned before repairs existed -- is the first
    // search's answer, byte-identical.
    let mut forbidden: Vec<Move> = Vec::new();
    let mut reached: Vec<bool> = vec![false; GRID * GRID];
    for _ in 0..=SELF_CROSSING_REPAIRS {
        let (found, frontier) = search_once(
            blocked,
            tunnels,
            origin,
            from,
            to,
            max_underground_distance,
            &forbidden,
            launch,
        );
        reached = frontier;
        let Some(route) = found else {
            break;
        };
        let Some(hits) = self_crossing(&route, origin) else {
            return Ok(route);
        };
        let before = forbidden.len();
        forbidden.extend(doubling_moves(&route, origin, &hits));
        if forbidden.len() == before {
            // Nothing new to forbid, so the next round would be this one
            // again. Unreachable while `doubling_moves` names a move for
            // every hit; stated so a change there cannot loop here.
            break;
        }
    }

    // A destination with nothing free beside it is sealed on ITS side, and
    // the search just proved no jump lands on it either. Name its walls,
    // never a span: the span below is read off the straight line from
    // `from`, which says nothing about which end is shut. Three live runs
    // read "an underground span of 7 tiles" as a wall round the source
    // (`docs/superpowers/notes/2026-09-09-belt-banding-and-reachability-live.md`)
    // when it was the sink's own four neighbours, one of them water.
    let walls = walls_around(blocked, to);
    if walls.len() == DIRECTIONS.len() {
        return Err(RouteError::NoPath {
            blocked: walls
                .into_iter()
                .map(|c| cell_to_position(origin, c))
                .collect(),
        });
    }

    if let Some(max) = max_underground_distance {
        // A wall `w` tiles wide needs a pair `w + 1` apart, in the
        // prototype's unit.
        let needed = widest_blocked_run(blocked, from, to) + 1;
        if needed > u32::from(max) {
            return Err(RouteError::SpanTooLong { needed, max });
        }
    }

    Err(RouteError::NoPath {
        blocked: blocking_tiles(blocked, origin, &reached),
    })
}

/// How many times a self-crossing route is repaired and re-searched before
/// the search gives up. Each round forbids a move the last route took, so
/// this is a bound on work, not a condition for termination; the measured
/// case needs one round, and eight keeps a pathological grid from spending a
/// plan's budget here.
const SELF_CROSSING_REPAIRS: usize = 8;

/// A step or jump the search may not take: leaving `cell` in `direction`.
type Move = ((usize, usize), Direction);

/// The moves that put a route onto a cell it already uses: for every hit
/// cell, each step or jump in `route` that lands on it after its first use,
/// and each jump that runs beneath it. Deduplicated, in route order.
fn doubling_moves(route: &Route, origin: (f64, f64), hits: &[(usize, usize)]) -> Vec<Move> {
    let cell = |p: &Position| -> (usize, usize) {
        let x = ((p.x() - origin.0) / crate::graph::enclosure::CELL) as usize;
        let y = ((p.y() - origin.1) / crate::graph::enclosure::CELL) as usize;
        (x, y)
    };
    let mut out: Vec<Move> = Vec::new();
    let mut push = |m: Move| {
        if !out.iter().any(|o| o.0 == m.0 && o.1 == m.1) {
            out.push(m);
        }
    };
    for hit in hits {
        let mut seen = false;
        for (i, tile) in route.tiles.iter().enumerate() {
            if cell(&tile.position) != *hit {
                continue;
            }
            if seen && i > 0 {
                push((cell(&route.tiles[i - 1].position), tile.direction));
            }
            seen = true;
        }
        for (entry, exit) in route.underground_pairs() {
            let spans = tunnel_cells(&route.tiles[entry].position, &route.tiles[exit].position)
                .iter()
                .any(|p| cell(p) == *hit);
            if spans {
                push((
                    cell(&route.tiles[entry].position),
                    route.tiles[entry].direction,
                ));
            }
        }
    }
    out
}

/// One A* pass over `blocked`: the cheapest route in the state space, which
/// may meet itself -- [`route_belt_with_tunnels`] checks -- or `None`, and
/// beside it every cell the search visited, for the refusal's honest naming.
#[allow(clippy::too_many_arguments)]
fn search_once(
    blocked: &[bool],
    tunnels: &[u8],
    origin: (f64, f64),
    from: (usize, usize),
    to: (usize, usize),
    max_underground_distance: Option<u8>,
    forbidden: &[Move],
    launch: Option<Direction>,
) -> (Option<Route>, Vec<bool>) {
    let max_underground = max_underground_distance;
    let mut best: Vec<u32> = vec![u32::MAX; GRID * GRID * 8];
    let mut came: Vec<Option<Predecessor>> = vec![None; GRID * GRID * 8];
    let mut heap = BinaryHeap::new();
    // Every cell the search actually visited, for an honest refusal message
    // if it never reaches `to`.
    let mut reached: Vec<bool> = vec![false; GRID * GRID];

    // A launched route arrives at `from` already facing its way, so only
    // that facing is seeded; the free start seeds all four at cost zero, as
    // it always has.
    for (dir, _) in DIRECTIONS {
        if launch.is_some_and(|launch| launch != dir) {
            continue;
        }
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
            let route = reconstruct(&came, origin, from, to, node.facing, node.surfaced);
            return (Some(route), reached);
        }
        let slot = state_index(node.cell, node.facing, node.surfaced);
        if node.cost > best[slot] {
            continue;
        }
        for (dir, (dx, dy)) in DIRECTIONS {
            // A move a repair round forbade: neither the step nor the jump
            // out of this cell in this direction is offered.
            if forbidden.iter().any(|(c, d)| *c == node.cell && *d == dir) {
                continue;
            }
            // A launched route leaves `from` in its launch direction and no
            // other: the first tile's facing is the emitter's, by
            // construction. (A route that wanders back onto `from` is a
            // self-crossing, repaired above like any other; the rule holding
            // there too costs nothing.)
            if node.cell == from && launch.is_some_and(|launch| launch != dir) {
                continue;
            }
            // The normal move and the underground move are independent
            // options out of `node.cell` in this direction, not a fallback
            // chain: a blocked (or off-grid) immediate neighbour must not
            // suppress an underground attempt in the same direction -- that
            // is exactly the case where a jump is needed, when the wall
            // starts on the very next tile. Each is therefore its own `if`,
            // never a shared early `continue`.
            // The second clause is the "straight after an exit" rule from
            // the doc above: a surfaced node offers only the one step that
            // continues the tunnel's direction.
            //
            // The third clause: a belt run never reverses. The state space
            // is (cell, facing, surfaced), so stepping back onto the cell
            // just left is a *different* state and A* is happy to take it
            // -- and did, surfacing at an exit, stepping one tile on and
            // stepping straight back onto the exit tile as a surface belt
            // to turn there. A belt fed from the tile it points at is not a
            // thing, so the reverse of `node.facing` is never offered.
            if let Some(next) = step(node.cell, dx, dy)
                && !blocked[cell_index(next.0, next.1)]
                && (!node.surfaced || dir == node.facing)
                && !reverses(dir, node.facing)
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
            // is not a thing the game has -- and it is only offered over
            // ground that is actually blocked, and then priced by
            // `UNDERGROUND_PENALTY` so that a detour the surface can make is
            // taken first.
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
            //
            // Gated on `dir == node.facing` too: the "launched straight"
            // rule from the doc above. Every start state is seeded in all
            // four facings at cost zero, so a jump straight out of `from`
            // is still free in every direction.
            if !node.surfaced
                && dir == node.facing
                && let Some(max) = max_underground
            {
                let axis = tunnel_axis(dir);
                // `span` is the entry-to-exit distance, the prototype's unit.
                for span in 2..=(max as i64) {
                    let Some(exit) = step(node.cell, dx * span, dy * span) else {
                        break;
                    };
                    if blocked[cell_index(exit.0, exit.1)] {
                        continue;
                    }
                    if tunnels[cell_index(exit.0, exit.1)] & axis != 0 {
                        // Surfacing on top of an existing same-axis tunnel
                        // would pair with it; so would any longer jump.
                        break;
                    }
                    let under: Vec<(usize, usize)> = (1..span)
                        .filter_map(|i| step(node.cell, dx * i, dy * i))
                        .collect();
                    if under
                        .iter()
                        .any(|c| tunnels[cell_index(c.0, c.1)] & axis != 0)
                    {
                        break;
                    }
                    let crosses_blocked = under.iter().any(|c| blocked[cell_index(c.0, c.1)]);
                    if !crosses_blocked {
                        continue;
                    }
                    let cost = node.cost
                        + STEP * span as u32
                        + UNDERGROUND_PENALTY
                        + HIDDEN_TILE_COST * (span as u32 - 1);
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

    (None, reached)
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
/// The orthogonal neighbours of `cell` that are blocked or off the grid
/// (an edge counts as a wall: nothing enters from beyond the window). Four
/// of them means no belt can ever be laid into `cell` on the surface.
fn walls_around(blocked: &[bool], cell: (usize, usize)) -> Vec<(usize, usize)> {
    DIRECTIONS
        .iter()
        .filter_map(|(_, (dx, dy))| match step(cell, *dx, *dy) {
            Some(n) if blocked[cell_index(n.0, n.1)] => Some(n),
            Some(_) => None,
            // Off the grid: name the cell itself rather than invent a
            // position outside the window.
            None => Some(cell),
        })
        .collect()
}

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

impl Route {
    /// The underground pairs in a route, as `(entry, exit)` indices into
    /// `tiles`, in route order.
    pub fn underground_pairs(&self) -> Vec<(usize, usize)> {
        let mut pairs = Vec::new();
        let mut open: Option<usize> = None;
        for (i, tile) in self.tiles.iter().enumerate() {
            match tile.kind {
                TileKind::UndergroundEntry => open = Some(i),
                TileKind::UndergroundExit => {
                    if let Some(entry) = open.take() {
                        pairs.push((entry, i));
                    }
                }
                TileKind::Belt => {}
            }
        }
        pairs
    }
}

/// Every position strictly between the two halves of a pair, plus the
/// halves themselves: the cells a tunnel occupies and the game will pair
/// through. `entry` and `exit` must be collinear on one axis, which
/// `route_belt` guarantees for the pairs it emits.
pub fn tunnel_cells(entry: &Position, exit: &Position) -> Vec<Position> {
    let (dx, dy) = (exit.x() - entry.x(), exit.y() - entry.y());
    let steps = dx.abs().max(dy.abs()).round() as i64;
    let unit = |d: f64| if d == 0.0 { 0.0 } else { d.signum() };
    let (sx, sy) = (unit(dx), unit(dy));
    (0..=steps)
        .map(|i| Position::new(entry.x() + sx * i as f64, entry.y() + sy * i as f64))
        .collect()
}

/// The cells a route wants twice: a tile it stands on more than once (the
/// state space lets A* return to a cell with a different facing, and a tile
/// holds one entity), a surface tile of its own lying beneath one of its own
/// tunnels, or two of its own same-axis tunnels sharing ground. `None` when
/// the route is clean.
///
/// The doubled-tile case is stated on its own since 2026-09-09. The tunnel
/// cases were what this check was written for, and they happened to catch
/// the measured instance -- a belt and the entry half of a pair on one cell
/// -- only because [`tunnel_cells`] counts the halves themselves; a route
/// doubling a tile some other way would have passed. A surface-only search
/// cannot double a tile (a loop is never cheaper), so every hit here
/// involves a jump.
fn self_crossing(route: &Route, origin: (f64, f64)) -> Option<Vec<(usize, usize)>> {
    let mut reserved: Vec<(usize, usize)> = Vec::new();
    let mut hits: Vec<(usize, usize)> = Vec::new();
    let cell = |p: &Position| -> (usize, usize) {
        let x = ((p.x() - origin.0) / crate::graph::enclosure::CELL) as usize;
        let y = ((p.y() - origin.1) / crate::graph::enclosure::CELL) as usize;
        (x, y)
    };
    let mut stood: Vec<(usize, usize)> = Vec::new();
    for tile in &route.tiles {
        let c = cell(&tile.position);
        if stood.contains(&c) {
            hits.push(c);
        }
        stood.push(c);
    }
    let pairs = route.underground_pairs();
    let mut axes: Vec<((usize, usize), u8)> = Vec::new();
    for (entry, exit) in &pairs {
        let axis = tunnel_axis(route.tiles[*entry].direction);
        for p in tunnel_cells(&route.tiles[*entry].position, &route.tiles[*exit].position) {
            let c = cell(&p);
            if axes.iter().any(|(other, a)| *other == c && *a == axis) {
                hits.push(c);
            }
            axes.push((c, axis));
            reserved.push(c);
        }
    }
    for (i, tile) in route.tiles.iter().enumerate() {
        if pairs.iter().any(|(e, x)| *e == i || *x == i) {
            continue;
        }
        let c = cell(&tile.position);
        if reserved.contains(&c) {
            hits.push(c);
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort_unstable();
    hits.dedup();
    Some(hits)
}
