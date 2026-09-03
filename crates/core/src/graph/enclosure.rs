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
//! # Why a false positive would be worse than the bug
//!
//! Fencing off a bot that was working is a worse outcome than failing to
//! notice one that was not. Two things keep that from happening:
//!
//! * The occupancy model is *incomplete in the safe direction*. `blocked_tree`
//!   holds only what the game has told us about, so uncharted ground reads as
//!   free and the fill escapes through it. A missing obstacle produces
//!   [`Escape::Open`], never [`Escape::Enclosed`].
//! * The question is only asked when the game has already answered it once —
//!   see `crates/executor/src/walk_memory.rs`. A report requires both the
//!   pathfinder refusing a route from this spot *and* this fill closing, which
//!   is a conjunction of two independent judgements about the same ground.

use crate::factorio::world::Enclosure;
use crate::graph::entity_graph::EntityGraph;
use crate::types::{PlayerId, Position, Rect};

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

/// The side of one fill cell, in tiles.
///
/// An eighth of a tile, and the two directions of error are not symmetric, so
/// the value is argued rather than tuned:
///
/// * **It cannot cross a wall.** Obstacles are inflated by the character's
///   half-box before rasterising, so the thinnest thing that can separate two
///   points is 2 × 0.199 = 0.398 tiles thick — more than three cells. Two
///   4-connected free cells are 0.125 apart, so no wall fits between them
///   without covering one of them. A fill that cannot leak is a fill that
///   cannot report [`Escape::Open`] for a bot that is actually boxed in.
/// * **It can miss a corridor narrower than one cell.** A gap exactly as wide
///   as the character has zero width in configuration space; this finds a gap
///   only once it is a cell wider than the character. That error direction
///   produces a false [`Escape::Enclosed`], which is the expensive one — and
///   it is why the trigger requires the game's own pathfinder to have refused
///   a route from the same spot first. A corridor that thin is not one the
///   game's pathfinder would have used either.
pub const CELL: f64 = 0.125;

/// Cells per axis in the searched window: the window is `2 * SEARCH_RADIUS`
/// tiles across, at [`CELL`] resolution.
const GRID: usize = (2. * SEARCH_RADIUS / CELL) as usize;

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
    /// The free region closes inside the window: every point the character can
    /// walk to is inside a pocket of this many square tiles, and nothing
    /// outside it is reachable without mining or being moved.
    ///
    /// The area is the *configuration-space* area — obstacles inflated by the
    /// character's own half-box — so it is the ground the character's centre
    /// can occupy, which is smaller than the floor area a person would measure
    /// by eye. It is reported because "boxed into 3 square tiles" and "boxed
    /// into 300" are different situations and the record should not make a
    /// reader guess which one it is looking at.
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
/// log-depth descent tens of thousands of times to answer what one windowed
/// query answers. See the module docs for what each answer claims.
///
/// The character's own position is admitted as free whatever the occupancy
/// says. A character overlapping a collision box is a real state — the mod
/// teleports one clear of a ghost it is standing in — and refusing to seed the
/// fill there would answer "unknown" for the very case most likely to be
/// stuck. If it truly cannot move, the pocket comes back a fraction of a tile.
pub fn escape_from(graph: &EntityGraph, from: &Position) -> Escape {
    let window = Rect::new(
        &Position::new(from.x() - SEARCH_RADIUS, from.y() - SEARCH_RADIUS),
        &Position::new(from.x() + SEARCH_RADIUS, from.y() + SEARCH_RADIUS),
    );
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
        return Escape::Unknown(EscapeUnknown::OutsideModel);
    }

    let (half_x, half_y) = character_half_box(graph);
    // Configuration space: grow every obstacle by the character's half-box and
    // treat the character as the single point at its centre. Testing the
    // character's whole box against every cell instead would ask the same
    // question once per cell rather than once per obstacle.
    let mut blocked = vec![false; GRID * GRID];
    let origin = (from.x() - SEARCH_RADIUS, from.y() - SEARCH_RADIUS);
    for box_ in graph.blocking_boxes_within(&window) {
        // A cell counts as blocked when its *centre* falls inside the grown
        // box. `blocking_boxes_within` is a narrowing pass that admits boxes
        // which merely come close, so every one of them is tested exactly here
        // and a box entirely outside the window clips away to nothing.
        let Some(xs) = cells_covering(
            box_.left_top.x() - half_x - origin.0,
            box_.right_bottom.x() + half_x - origin.0,
        ) else {
            continue;
        };
        let Some(ys) = cells_covering(
            box_.left_top.y() - half_y - origin.1,
            box_.right_bottom.y() + half_y - origin.1,
        ) else {
            continue;
        };
        for y in ys {
            let row = y * GRID;
            blocked[row + xs.start..row + xs.end].fill(true);
        }
    }

    let start = (GRID / 2, GRID / 2);
    let mut seen = vec![false; GRID * GRID];
    let mut stack = vec![start];
    seen[start.1 * GRID + start.0] = true;
    let mut reached = 0u32;
    while let Some((x, y)) = stack.pop() {
        reached += 1;
        if x == 0 || y == 0 || x == GRID - 1 || y == GRID - 1 {
            return Escape::Open;
        }
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            let index = ny * GRID + nx;
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
fn character_half_box(graph: &EntityGraph) -> (f64, f64) {
    graph
        .entity_prototypes()
        .get("character")
        .map(|p| (p.collision_box.width() / 2., p.collision_box.height() / 2.))
        .unwrap_or((
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
        ))
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
