//! Connecting two entities with a belt, and the inserters at each end.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::enclosure;
use crate::goal::{Goal, Holder};
use crate::ids::ItemId;
use crate::method::have::PLACE_TICKS;
use crate::method::{ExpansionCtx, Step};
use factorio_bot_core::blueprint::UndergroundHalf;
use factorio_bot_core::graph::enclosure::GRID;
use factorio_bot_core::graph::route::{
    RouteError, TileKind, route_belt_with_tunnels, tunnel_axis, tunnel_cells,
};
use factorio_bot_core::types::{Direction, FactorioEntity, Position, Rect};

/// The belt this module lays, and the item whose bill it states.
const BELT: &str = "transport-belt";
/// The underground pair that carries [`BELT`] beneath an obstacle. Paired
/// with `BELT` by hand because the prototype does not name its partner: the
/// day this module lays a faster belt, this is the second name to change.
const UNDERGROUND: &str = "underground-belt";
/// The inserter at each end when a caller does not name one.
///
/// **Not craftable at stage 1**, which is the whole reason
/// [`connect_steps_with`] exists. `inserter`'s recipe takes an
/// `electronic-circuit` and reads `enabled: false` on a freeplay force —
/// checked against seed 31337's own t=0 dump, not assumed — so a plan that
/// places one before `electronics` is researched refuses on the bill. A
/// `burner-inserter` is 1 iron plate and 1 gear and is enabled from the start.
const INSERTER: &str = "inserter";

/// Half the collision box of the *largest* thing this module places, on each
/// axis: a `transport-belt` is `0.796875` tiles across
/// (`crates/core/tests/entity-prototype-fixtures.json`), and
/// `FactorioEntity::new_transport_belt` already rounds that to `0.8`.
///
/// **This is a placement clearance, not a pathfinding one, and passing zero
/// here was a real defect.** `enclosure::rasterize`'s contract is "a cell is
/// blocked when its centre falls inside the obstacle *grown by `half_box`*",
/// and its only other caller passes the character's half-box because it asks
/// whether a character can walk there. This asks whether a belt can be
/// *built* there, so the right clearance is the belt's own. With zero, a cell
/// counted as blocked only when an obstacle covered its exact centre:
/// grid-aligned buildings were accidentally safe (their boxes straddle a tile
/// centre anyway), but a **tree or a rock sits at an arbitrary sub-tile
/// position** and a box 0.8 wide can miss a tile centre entirely while
/// leaving no room for a belt. The route was then planned straight through
/// it and the build failed partway -- breaking this module's own promise,
/// stated on [`ConnectRefusal`], that a refusal always comes before anything
/// is placed.
///
/// One number for both entities rather than one each, because there is one
/// grid: an `inserter`'s box is fractionally smaller (`0.78` in
/// `FactorioEntity::new_inserter`), so using the belt's is conservative for
/// the inserter and exact for the belt. The cost is that a tile whose only
/// obstacle clears an inserter but not a belt reads as blocked for both;
/// against a plan that refuses cleanly, that is the direction to be wrong in.
const PLACEMENT_HALF_BOX: f64 = 0.4;

/// The `direction` an inserter must carry to move an item from `from` to `to`.
///
/// **It names the side it picks up from.** See the test.
///
/// # Who else encodes this convention
///
/// This is the one place **new** planner code should compute it -- but it is
/// not the only encoding in the tree, and calling it "the one place that
/// knows" (as an earlier version of this comment and of CLAUDE.md did) is
/// overstated. Two others exist and agree with it today:
///
/// * [`crate::state::PlanState::delivers_into`] derives the same fact from an
///   inserter's `pickup_position` / `drop_position` rather than from a pair of
///   machine positions, which is the *reading* side of the same convention;
/// * `crate::method::assemble` places its cell's inserters from fixed
///   north-frame offsets with the directions written out as constants (see
///   that module's own "each one points at what it PICKS UP from" note), which
///   predates this function.
///
/// Neither is wrong and neither is changed here. The claim worth making is
/// the narrow one: a caller deriving a facing from two positions should call
/// this rather than re-derive it.
pub fn inserter_facing(from: &Position, to: &Position) -> Option<Direction> {
    let dx = to.x() - from.x();
    let dy = to.y() - from.y();
    if dx.abs() > f64::EPSILON && dy.abs() > f64::EPSILON {
        return None;
    }
    if dx.abs() > f64::EPSILON {
        // Moving east means picking up from the west.
        return Some(if dx > 0.0 {
            Direction::West
        } else {
            Direction::East
        });
    }
    if dy.abs() > f64::EPSILON {
        // Screen coordinates: +y is south. Moving south picks up from north.
        return Some(if dy > 0.0 {
            Direction::North
        } else {
            Direction::South
        });
    }
    None
}

/// Which `UndergroundHalf` a `route_belt` tile of each underground
/// `TileKind` must build as, `None` for an ordinary surface `Belt` tile.
///
/// Called by [`connect_steps_with`] for every tile the route emits, since
/// 2026-09-09. Until then the underground arm of that loop was an
/// `unreachable!()` behind a `max_underground: None`, on the stated ground
/// that "neither `FactorioEntity` nor the mod's `rcon_place_entity` can
/// express which half" -- which had stopped being true at every layer
/// (`FactorioEntity::underground_half`, `new_underground_belt`, the mod's
/// fifth argument, the executor threading it through) while the comment and
/// the `None` stayed. This function and its test were written first so that
/// the wiring had a pinned answer to consult rather than a chance to
/// transpose the two halves and reproduce "places perfectly, connects
/// nothing" with types instead of without them.
///
/// The mapping itself comes from `route_belt`'s own comment
/// (`crates/core/src/graph/route.rs`): "the earlier tile is where the pair
/// DIVES" (`UndergroundEntry`) and "the later one is where it SURFACES"
/// (`UndergroundExit`). Factorio's own two terms for those same moments are
/// `input` (an item leaves the surface belt into the tunnel there) and
/// `output` (it returns to one there) -- which is exactly `UndergroundHalf`'s
/// two variants.
pub fn underground_half_for_tile_kind(kind: TileKind) -> Option<UndergroundHalf> {
    match kind {
        TileKind::Belt => None,
        TileKind::UndergroundEntry => Some(UndergroundHalf::Input),
        TileKind::UndergroundExit => Some(UndergroundHalf::Output),
    }
}

/// Why a connection could not be made. **Every variant is returned before
/// anything is placed** -- a half-built belt run is worse than no belt run,
/// because the items sit on it and the bot that would carry them is gone.
#[derive(Debug, Clone)]
pub enum ConnectRefusal {
    /// No inserter tile, no belt endpoint next to it, or no belt route
    /// between the two endpoints could be found. `blocked` names whichever
    /// positions stood in the way -- the occupied tiles that stopped a
    /// perimeter search, or the obstacles `route_belt` itself reports.
    NoRoute { blocked: Vec<Position> },
    /// An underground span longer than the belt prototype allows: the wall
    /// on the direct line is wider than [`UNDERGROUND`]'s
    /// `max_underground_distance` can bridge. Both numbers are the
    /// prototype's unit, entry-to-exit distance.
    SpanTooLong { needed: u32, max: u8 },
    /// An inserter's own machine and its belt tile were not aligned on one
    /// axis -- an inserter only ever moves items between two tiles directly
    /// opposite each other, and this pair is not that.
    ///
    /// **Unreachable by construction since the footprint rewrite**, and left
    /// in place as an assertion rather than as an expected outcome: both
    /// facings are now computed between two cell centres of the *same* grid
    /// that lie two cells apart on one axis (machine footprint cell,
    /// inserter cell, belt cell -- collinear by the way they are chosen), so
    /// the difference is an exact integer on one axis and exactly zero on
    /// the other. Before that rewrite this fired on **open ground for every
    /// even-footprint machine**: a `stone-furnace` covers two tiles per axis
    /// and therefore sits at an *integer*, while every position
    /// `enclosure::cell_to_position` produces is a half-integer, so both `dx`
    /// and `dy` came out non-zero half-integers and [`inserter_facing`]
    /// answered `None` every time.
    NotCardinal,
}

impl std::fmt::Display for ConnectRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // An empty list is honest, not a missing detail: it means nothing
            // on the searched grid stopped the route -- every candidate tile
            // fell outside the window, or the search reached the window's
            // edge without ever bordering an obstacle. Both mean the two
            // machines are further apart than one window models.
            ConnectRefusal::NoRoute { blocked } if blocked.is_empty() => write!(
                f,
                "no belt route, and nothing on the searched grid blocked it: the two \
                 machines are further apart than one search window reaches"
            ),
            ConnectRefusal::NoRoute { blocked } => {
                write!(f, "no belt route, blocked by {} tile(s):", blocked.len())?;
                for pos in blocked.iter().take(8) {
                    write!(f, " {pos}")?;
                }
                if blocked.len() > 8 {
                    write!(f, " ... and {} more", blocked.len() - 8)?;
                }
                Ok(())
            }
            ConnectRefusal::SpanTooLong { needed, max } => write!(
                f,
                "the obstacle needs an underground span of {needed} tiles and the belt \
                 allows {max}"
            ),
            ConnectRefusal::NotCardinal => write!(
                f,
                "an inserter's machine and its belt tile are not opposite each other \
                 on one axis"
            ),
        }
    }
}

/// The cell `at` falls in, in the window whose origin corner is `origin` --
/// or `None` when it lies outside that window's `GRID x GRID` cells
/// altogether, which is what it means to ask `connect_steps` to join two
/// points too far apart for one `enclosure::window` (centred on `from`) to
/// model both of.
fn cell_of(origin: (f64, f64), at: &Position) -> Option<(usize, usize)> {
    let x = (at.x() - origin.0) / enclosure::CELL;
    let y = (at.y() - origin.1) / enclosure::CELL;
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    (x < GRID && y < GRID).then_some((x, y))
}

/// `(x, y)` as a grid cell, or `None` when it falls outside the window.
fn in_grid(x: i64, y: i64) -> Option<(usize, usize)> {
    if x < 0 || y < 0 || x >= GRID as i64 || y >= GRID as i64 {
        return None;
    }
    Some((x as usize, y as usize))
}

/// The inclusive cell range `[lo, hi]` covers on one axis, measured in tiles
/// from the window's edge, or `None` when it covers no cell centre at all.
///
/// The same rule `enclosure::rasterize` uses -- a cell counts when its centre
/// falls inside the span -- restated here because that module's own helper is
/// private and because this one must answer in **unclamped** `i64`: a
/// footprint's perimeter is one cell outside it, and clamping first would
/// silently move that perimeter inwards onto the machine itself.
fn cell_span(lo: f64, hi: f64) -> Option<(i64, i64)> {
    let first = (lo / enclosure::CELL - 0.5).ceil();
    let last = (hi / enclosure::CELL - 0.5).floor();
    if last < first {
        return None;
    }
    Some((first as i64, last as i64))
}

/// The rectangle of cells a machine actually stands on.
///
/// **The whole point of the rewrite this belongs to.** `connect_steps` used
/// to treat `from` and `to` as 1x1 entities whose position was a tile centre,
/// and derived its inserter tile from the four neighbours of the machine's
/// *centre cell*. Nothing the spec names is 1x1: a 3x3 (assembling machine,
/// lab, electric mining drill) has all four of those neighbours inside its
/// own footprint, so every such connection refused `NoRoute`; a 2x2
/// (stone furnace, burner drill) had two of four inside it, so the chain bent
/// and the belt endpoint ended up diagonal from the machine.
///
/// Read from the entity's own `bounding_box`, which every `FactorioEntity`
/// constructor fills in and the game reports directly. A degenerate box (a
/// hand-built entity that never set one) falls back to the single cell the
/// position lands in, which is the old behaviour and is right for a 1x1.
#[derive(Debug, Clone, Copy)]
struct Footprint {
    x: (i64, i64),
    y: (i64, i64),
}

fn footprint_of(origin: (f64, f64), entity: &FactorioEntity) -> Option<Footprint> {
    let box_ = &entity.bounding_box;
    if box_.width() > 0.0 && box_.height() > 0.0 {
        let x = cell_span(
            box_.left_top.x() - origin.0,
            box_.right_bottom.x() - origin.0,
        )?;
        let y = cell_span(
            box_.left_top.y() - origin.1,
            box_.right_bottom.y() - origin.1,
        )?;
        return Some(Footprint { x, y });
    }
    let (x, y) = cell_of(origin, &entity.position)?;
    Some(Footprint {
        x: (x as i64, x as i64),
        y: (y as i64, y as i64),
    })
}

/// Mark every cell of `footprint` blocked.
///
/// Stated rather than relied upon: a machine the *plan* placed is in
/// `PlanState`'s overlay and not in the base `entity_graph` that
/// [`connect_steps`] rasterises, so its tiles would otherwise read as free
/// and the belt would be routed straight over it.
fn claim_footprint(blocked: &mut [bool], footprint: &Footprint) {
    for y in footprint.y.0..=footprint.y.1 {
        for x in footprint.x.0..=footprint.x.1 {
            if let Some((x, y)) = in_grid(x, y) {
                blocked[enclosure::cell_index(x, y)] = true;
            }
        }
    }
}

/// Every cell cardinally outside `footprint`, paired with the outward
/// direction that reaches it, in fixed North / East / South / West order and
/// ascending along each side.
///
/// Fixed so that the tile a caller gets for a given obstacle layout is always
/// the same tile, never an artefact of iteration order. Diagonal corners are
/// deliberately absent: an inserter only ever moves items along one axis.
fn perimeter(footprint: &Footprint) -> Vec<((i64, i64), (i64, i64))> {
    let mut out = Vec::new();
    for x in footprint.x.0..=footprint.x.1 {
        out.push(((x, footprint.y.0 - 1), (0, -1)));
    }
    for y in footprint.y.0..=footprint.y.1 {
        out.push(((footprint.x.1 + 1, y), (1, 0)));
    }
    for x in footprint.x.0..=footprint.x.1 {
        out.push(((x, footprint.y.1 + 1), (0, 1)));
    }
    for y in footprint.y.0..=footprint.y.1 {
        out.push(((footprint.x.0 - 1, y), (-1, 0)));
    }
    out
}

/// One machine's end of a connection: three collinear cells, outward from the
/// machine.
#[derive(Debug, Clone, Copy)]
struct Endpoint {
    /// The footprint cell the inserter reaches into. **This, not the
    /// machine's `position`, is what a facing is computed against** -- it is
    /// a cell centre on the same grid as `inserter` and `belt`, so the three
    /// are collinear with integer separations and [`inserter_facing`] can
    /// never see the mixed-parity diagonal that used to refuse every
    /// even-footprint machine.
    anchor: (usize, usize),
    inserter: (usize, usize),
    belt: (usize, usize),
}

/// The first free `(inserter, belt)` pair on `footprint`'s perimeter.
///
/// Both cells are required free together: an inserter with nowhere to put the
/// belt is not a usable end, and taking the inserter tile anyway is what made
/// the old chain bend into a diagonal.
///
/// `Err` names every occupied tile that stopped it, deduplicated and in
/// ascending cell order -- a fixed order rather than an artefact of the scan.
/// An off-grid candidate is skipped rather than named: it has no position
/// this window can state.
fn first_free_perimeter(
    blocked: &[bool],
    origin: (f64, f64),
    footprint: &Footprint,
) -> Result<Endpoint, Vec<Position>> {
    let mut stopped: Vec<(usize, usize)> = Vec::new();
    for ((x, y), (dx, dy)) in perimeter(footprint) {
        let (Some(inserter), Some(belt), Some(anchor)) = (
            in_grid(x, y),
            in_grid(x + dx, y + dy),
            in_grid(x - dx, y - dy),
        ) else {
            continue;
        };
        if blocked[enclosure::cell_index(inserter.0, inserter.1)] {
            stopped.push(inserter);
            continue;
        }
        if blocked[enclosure::cell_index(belt.0, belt.1)] {
            stopped.push(belt);
            continue;
        }
        return Ok(Endpoint {
            anchor,
            inserter,
            belt,
        });
    }
    stopped.sort_unstable();
    stopped.dedup();
    Err(stopped
        .into_iter()
        .map(|cell| enclosure::cell_to_position(origin, cell))
        .collect())
}

/// Every collision box the *plan* has added inside `area`, so a second
/// connection cannot be routed over the first one's belt.
///
/// # Why this had to change, and what it fixes
///
/// This module's own doc used to state as a limitation that obstacles came
/// from the base world alone and that "a caller chaining two connections in
/// the same plan must not treat the first call's output as ground truth for
/// the second". That limitation was survivable while nothing called this
/// function at all. A cell that feeds itself needs **four** connections in one
/// expansion — the coal drill's own refuel loop, and the runs to the smelting
/// cell's two burner machines — and without this every one of them would be
/// routed against an empty grid and place belts on top of each other. They
/// would place perfectly and move nothing, which is this project's defining
/// failure.
///
/// # The box a plan-placed entity is measured by
///
/// **Its prototype's, not its `bounding_box` field.** `method::produce`'s
/// `machine()` builds a cell's drill and furnace with `bounding_box:
/// Default::default()` — a degenerate zero box — because nothing had ever
/// asked it for one. Trusting that field would make a stone furnace this same
/// plan just placed occupy nothing at all, and the belt would be routed
/// straight through it. So the prototype is consulted first
/// ([`crate::state::PlanState::collision_area_facing`]) and the entity's own
/// box is the fallback for the things that carry a real one (the belts and
/// inserters this module itself emits).
fn overlay_boxes(ctx: &ExpansionCtx, area: &Rect) -> Vec<Rect> {
    let centre = Position::new(
        (area.left_top.x() + area.right_bottom.x()) / 2.,
        (area.left_top.y() + area.right_bottom.y()) / 2.,
    );
    // The circumradius of the window, so nothing inside the rectangle is
    // missed by a radius query: half the diagonal of a square of this side.
    let radius = (area.width() / 2.).hypot(area.height() / 2.);
    ctx.state
        .entities_within(&centre, radius)
        .into_iter()
        .filter_map(|entity| {
            <Direction as factorio_bot_core::num_traits::FromPrimitive>::from_u8(entity.direction)
                .and_then(|facing| {
                    ctx.state
                        .collision_area_facing(&entity.name, &entity.position, facing)
                })
                .or_else(|| {
                    let box_ = &entity.bounding_box;
                    (box_.width() > 0. && box_.height() > 0.).then(|| box_.clone())
                })
        })
        .collect()
}

/// Which cells of this window a charted enemy structure's standoff covers, or
/// `None` when no threat reaches the window at all.
///
/// **`None` is the zero-cost answer and it is the usual one.** Every map this
/// project plans on today has its early build sites far from anything charted,
/// so this returns `None` after one [`crate::state::PlanState::threat_covering`]-shaped
/// query and [`connect_steps_with`] never builds a second grid or runs a second
/// search. That is what keeps the guard off the planning-cost budget on the
/// four baselines.
///
/// # Why the threat list is taken once and not per cell
///
/// `EntityGraph::threats_from` clones and *sorts* the whole threat table on
/// every call, so asking it per cell would be `GRID * GRID` sorts -- 2,304 of
/// them for one belt. It is asked once, about the window's centre, for
/// everything that could possibly reach the window; the per-cell test is then
/// a distance against that short list. This is the shape this repo has already
/// paid for once, when a per-candidate threat lookup doubled the oil goal's
/// planning wall time.
///
/// The radius asked for is the window's circumradius plus the widest standoff
/// any threat here claims, so a nest sitting outside the window whose reach
/// extends into it is still counted -- the failure mode being guarded against
/// is precisely a threat you cannot see from the tile you are standing on.
fn threatened_cells(ctx: &ExpansionCtx, origin: (f64, f64)) -> Option<Vec<bool>> {
    let centre = enclosure::cell_to_position(origin, (GRID / 2, GRID / 2));
    let reaching: Vec<(Position, f64)> = ctx
        .state
        .base()
        .entity_graph
        .threats_from(&centre)
        .into_iter()
        .filter_map(|(name, at, distance)| {
            let standoff = ctx.state.threat_standoff(&name).tiles;
            // Half the window's diagonal: nothing inside the square is further
            // from its centre than this, so a threat further away than
            // `standoff + that` cannot cover a single cell of it.
            let circumradius = (GRID as f64 / 2.).hypot(GRID as f64 / 2.);
            (distance <= standoff + circumradius).then_some((at, standoff))
        })
        .collect();
    if reaching.is_empty() {
        return None;
    }
    let mut cells = vec![false; GRID * GRID];
    for y in 0..GRID {
        for x in 0..GRID {
            let at = enclosure::cell_to_position(origin, (x, y));
            cells[enclosure::cell_index(x, y)] = reaching.iter().any(|(threat, standoff)| {
                factorio_bot_core::factorio::util::calculate_distance(&at, threat) < *standoff
            });
        }
    }
    Some(cells)
}

/// [`UNDERGROUND`]'s reach, read from the prototype the world carries --
/// `None` when the world has no such prototype, which makes the search
/// surface-only rather than guessing a number. On this install the field
/// reads 5 for `underground-belt`, 7 for `fast-` and 11 for `turbo-`; a
/// constant here would be right for one of them on one mod set.
fn underground_reach(ctx: &ExpansionCtx) -> Option<u8> {
    ctx.state
        .base()
        .globals
        .entity_prototypes
        .get(UNDERGROUND)
        .and_then(|proto| proto.max_underground_distance)
}

/// Two positions on the same tile.
fn same_tile(a: &Position, b: &Position) -> bool {
    (a.x() - b.x()).abs() < 1e-6 && (a.y() - b.y()).abs() < 1e-6
}

/// The eight cells an arm-and-belt could stand on around every `container`
/// inside `area` -- each of its four cardinal neighbours and the tile beyond
/// -- paired with the chest's position, so the caller can pick out its own
/// two endpoints' sides (the only ones it closes).
///
/// **A chest's sides are the scarce thing.** A 1x1 chest has four of them,
/// and each run in or out of it takes one: an arm on the neighbour and a
/// belt on the tile beyond. A route that merely passes a chest at one or
/// two tiles' distance spends a side just as surely, and it did (see the
/// caller). Only containers, by the prototype's `entity_type`, and only the
/// cardinal lines: a furnace's or a drill's perimeter is where its arms go
/// too, but those are sited by the method that owns them, with a room check
/// of its own, and closing their rings here made a one-cell arrangement
/// unbuildable in the fixtures. Read off the prototype rather than the
/// entity's `entity_type`, which a plan-built entity may leave empty.
fn container_sides(
    ctx: &ExpansionCtx,
    area: &Rect,
    origin: (f64, f64),
) -> Vec<((usize, usize), Position)> {
    let centre = Position::new(
        (area.left_top.x() + area.right_bottom.x()) / 2.,
        (area.left_top.y() + area.right_bottom.y()) / 2.,
    );
    let radius = (area.width() / 2.).hypot(area.height() / 2.);
    let is_container = |name: &str| {
        ctx.state
            .base()
            .globals
            .entity_prototypes
            .get(name)
            .is_some_and(|p| p.entity_type == "container")
    };
    let mut out = Vec::new();
    for chest in ctx
        .state
        .entities_within(&centre, radius)
        .into_iter()
        .filter(|e| is_container(&e.name))
    {
        for (dx, dy) in [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)] {
            for reach in [1., 2.] {
                let at = Position::new(
                    chest.position.x() + dx * reach,
                    chest.position.y() + dy * reach,
                );
                if let Some(cell) = cell_of(origin, &at) {
                    out.push((cell, chest.position.clone()));
                }
            }
        }
    }
    out
}

/// The ground every existing [`UNDERGROUND`] pair inside `area` runs beneath
/// -- base world and this plan's overlay alike -- as a tunnel grid for
/// `route_belt_with_tunnels` plus the same cells as a list to mark blocked.
///
/// **A tunnel is an entity the grid cannot see.** `overlay_boxes` and the
/// base graph report collision boxes, and the four tiles between an entry
/// half and its exit half have none: nothing stands there. The game still
/// pairs through them, so a second pair of the same belt laid along that
/// line would connect to the wrong half, and this module's own rule that
/// nothing else of this plan's is built on a tunnel keeps the span readable
/// as one thing. So the span is reserved twice: as an axis bit, which only
/// a same-axis jump respects, and as a blocked cell, which everything does.
///
/// Pairing follows the game: an input half faces its tunnel, and the first
/// same-name half it meets on that line within reach is its partner if it is
/// an output facing the same way, and a dead end otherwise. A half nothing
/// pairs with reserves only its own cell, which its collision box does
/// already.
fn tunnel_grid(
    ctx: &ExpansionCtx,
    area: &Rect,
    origin: (f64, f64),
    reach: u8,
) -> (Vec<u8>, Vec<(usize, usize)>) {
    use factorio_bot_core::num_traits::FromPrimitive;
    let centre = Position::new(
        (area.left_top.x() + area.right_bottom.x()) / 2.,
        (area.left_top.y() + area.right_bottom.y()) / 2.,
    );
    let radius = (area.width() / 2.).hypot(area.height() / 2.);
    let halves: Vec<FactorioEntity> = ctx
        .state
        .entities_within(&centre, radius)
        .into_iter()
        .filter(|e| e.name == UNDERGROUND)
        .collect();
    let mut tunnels = vec![0u8; GRID * GRID];
    let mut reserved: Vec<(usize, usize)> = Vec::new();
    for input in halves
        .iter()
        .filter(|e| e.underground_half == Some(UndergroundHalf::Input))
    {
        let Some(facing) = Direction::from_u8(input.direction) else {
            continue;
        };
        let (dx, dy) = match facing {
            Direction::North => (0., -1.),
            Direction::East => (1., 0.),
            Direction::South => (0., 1.),
            Direction::West => (-1., 0.),
            _ => continue,
        };
        let partner = (1..=i64::from(reach)).find_map(|i| {
            let at = Position::new(
                input.position.x() + dx * i as f64,
                input.position.y() + dy * i as f64,
            );
            halves
                .iter()
                .find(|e| {
                    (e.position.x() - at.x()).abs() < 1e-6 && (e.position.y() - at.y()).abs() < 1e-6
                })
                .map(|e| {
                    (e.underground_half == Some(UndergroundHalf::Output)
                        && e.direction == input.direction)
                        .then(|| e.position.clone())
                })
        });
        let Some(Some(exit)) = partner else {
            continue;
        };
        let axis = tunnel_axis(facing);
        for tile in tunnel_cells(&input.position, &exit) {
            if let Some(cell) = cell_of(origin, &tile) {
                tunnels[enclosure::cell_index(cell.0, cell.1)] |= axis;
                reserved.push(cell);
            }
        }
    }
    (tunnels, reserved)
}

/// One `Place` action, with the preconditions and effects every other method
/// in this crate emits for one -- see `method::power`'s plant parts, which
/// this deliberately mirrors field for field.
fn place_step(ctx: &mut ExpansionCtx, entity: FactorioEntity, build: f64, note: &str) -> Step {
    let min_radius = ctx.state.placement_clearance(&entity.name).unwrap_or(0.0);
    let step = Step::Act(Box::new(Action {
        id: ctx.ids.next(),
        kind: ActionKind::Place {
            entity: Box::new(entity.clone()),
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: entity.position.clone(),
                radius: build,
                min_radius,
            },
            Condition::AreaFree {
                pos: entity.position.clone(),
                entity: entity.name.as_str().into(),
                direction: entity.direction,
            },
            Condition::HasItem {
                who: Actor::Role,
                item: entity.name.as_str().into(),
                count: 1,
            },
        ],
        eff: vec![
            Effect::LoseItem {
                who: Actor::Role,
                item: entity.name.as_str().into(),
                count: 1,
            },
            Effect::CreateEntity(Box::new(entity.clone())),
        ],
        duration: PLACE_TICKS,
        pinned: None,
        label: format!("place {} at {} -- {note}", entity.name, entity.position),
    }));
    // The overlay half, exactly as `power.rs` does it: the next tile's
    // `AreaFree` must see what this one took.
    ctx.state.create_entity(entity);
    step
}

/// Connect `from` to `to` with a belt run and the inserter at each end that
/// loads and unloads it.
///
/// # It is not restricted to machines, and saying it was cost this project a
/// stated gap
///
/// This doc said "between two **machines**" and `method::assemble`'s said
/// chest-to-chest "is not a shape it has"; three sessions named that as the
/// biggest thing standing between a charged cell and a factory. Nothing here
/// ever restricted it. [`footprint_of`] reads a `bounding_box`, a 1x1
/// container has one, and `tests::a_run_between_two_chests_is_routed` is the
/// measurement rather than the argument. What *is* true of a container is
/// narrower and is stated on [`container_sides`]: it has four sides, each run
/// in or out of it spends one, and they go quickly.
///
/// # The geometry
///
/// `from` and `to` are the two **entities**, and they are taken at their real
/// size: [`footprint_of`] reads each one's `bounding_box`, every cell of both
/// is marked occupied, and each end's inserter is placed on the machine's
/// **footprint perimeter** rather than beside its centre. Three collinear
/// cells make up an end -- the footprint cell the inserter reaches into, the
/// inserter's own cell, and the belt cell beyond it -- chosen as the first
/// perimeter position where the latter two are both free, scanned North,
/// East, South, West and ascending along each side. Only the two belt cells
/// are routed between; the built line reads `machine | inserter | belt ...
/// belt | inserter | machine`. Deterministic by construction: the same
/// obstacle layout always yields the same tiles.
///
/// # The bill
///
/// The materials are stated as `Goal::Have` subgoals *and* as `HasItem`
/// preconditions on each `Place`, so the shortfall machinery that already
/// exists refuses the plan before the first belt goes down. That is the third
/// of the spec's four refusal paths; the first two are [`ConnectRefusal`]'s
/// own, and the fourth (a connection point that does not exist) is what
/// `first_free_perimeter`'s `Err` reports.
///
/// `item` is what the belt is expected to carry. It decides nothing about the
/// geometry and nothing about the bill -- a belt run costs belts and
/// inserters whatever flows along it -- and appears only in the labels, which
/// is where a reader of a plan needs it.
///
/// # Scope: obstacles are the base world, this plan's overlay, and tunnels
///
/// Obstacles come from `state.base().entity_graph.blocking_boxes_within`,
/// from every collision box this plan has already added ([`overlay_boxes`]),
/// from the two footprints stated above, and from the ground beneath every
/// existing underground pair ([`tunnel_grid`]). A route that has to cross
/// something goes under it with an [`UNDERGROUND`] pair, sized by the
/// prototype's `max_underground_distance` and refused as
/// [`ConnectRefusal::SpanTooLong`] when the wall is wider than that; a world
/// with no such prototype is routed on the surface only.
pub fn connect_steps(
    ctx: &mut ExpansionCtx,
    from: &FactorioEntity,
    to: &FactorioEntity,
    item: &ItemId,
) -> Result<Vec<Step>, ConnectRefusal> {
    connect_steps_with(ctx, from, to, item, INSERTER)
}

/// [`connect_steps`], with the inserter prototype named by the caller.
///
/// The only reason this is a parameter: `inserter` is not craftable on a
/// freeplay force at t=0 (see [`INSERTER`]), and stage 1 has no electricity to
/// run one with even if it were. A stage-1 caller passes `burner-inserter`,
/// which is enabled from the start, costs 1 iron plate and 1 gear, and — the
/// property this arrangement rests on — **takes its own fuel out of the coal
/// it is moving**, so a coal belt needs no separate supply for the arms that
/// unload it.
///
/// That last sentence is a claim about the *game*, not about this crate, and
/// nothing here can check it. It is measured in the live run, by reading the
/// placed inserters' `fuel_inventory` over RCON.
///
/// Geometry, bill and refusals are identical either way: both prototypes have
/// the same collision box, the same one-tile reach, and the same convention
/// that `direction` names the side the arm picks up from
/// (`crate::state`'s `vanilla_inserter_reach` lists them together).
///
/// # A ONE-TILE REACH IS A PRECONDITION HERE, AND NOTHING CHECKS IT
///
/// That sentence is true of `inserter` and `burner-inserter`, the only two
/// names this module has ever been given, and **false of
/// `long-handed-inserter`**, which reaches two
/// (`crate::state::inserter_reach`, prototype-derived since 2026-09-08).
/// This parameter takes any name.
///
/// [`Endpoint`] is three *collinear adjacent* cells -- anchor, inserter, belt
/// -- built as `inserter = anchor + d` and `belt = inserter + d` in
/// [`first_free_perimeter`], with no reach anywhere in the construction. Hand
/// it a long inserter and it places the belt **one** tile from an arm that
/// picks up **two** tiles away: the arm reaches straight over the belt this
/// module just laid, and drops two tiles into the machine rather than one. The
/// belt would stand, the inserter would stand, every refusal would stay
/// silent, and nothing would move -- this crate's most expensive failure
/// shape, arrived at from the one direction its geometry does not check.
///
/// So: **this is belts-and-one-tile-inserters, and a caller wanting reach 2
/// needs [`Endpoint`] to carry a reach rather than assume one.** Scoped, not
/// built -- see the report on `inserter_pickup_position`. Until then, pass
/// only a prototype whose `inserter_reach` is 1.
pub fn connect_steps_with(
    ctx: &mut ExpansionCtx,
    from: &FactorioEntity,
    to: &FactorioEntity,
    item: &ItemId,
    inserter: &str,
) -> Result<Vec<Step>, ConnectRefusal> {
    let (area, origin) = enclosure::window(&from.position);
    // `mut`: the two machine footprints and the six tiles derived below (an
    // anchor, an inserter and a belt cell at each end) all claim their cells
    // onto this same grid -- see the comments there.
    let mut blocked = enclosure::rasterize(
        ctx.state
            .base()
            .entity_graph
            .blocking_boxes_within(&area)
            .into_iter()
            .chain(overlay_boxes(ctx, &area)),
        origin,
        (PLACEMENT_HALF_BOX, PLACEMENT_HALF_BOX),
    );

    let from_footprint = footprint_of(origin, from).ok_or_else(|| ConnectRefusal::NoRoute {
        blocked: vec![from.position.clone()],
    })?;
    let to_footprint = footprint_of(origin, to).ok_or_else(|| ConnectRefusal::NoRoute {
        blocked: vec![to.position.clone()],
    })?;
    claim_footprint(&mut blocked, &from_footprint);
    claim_footprint(&mut blocked, &to_footprint);

    // UNDERGROUNDS: the reach is the prototype's, or `None` -- surface-only
    // -- when the world carries no `underground-belt` prototype at all.
    // Existing pairs (base world or this plan's) reserve the ground beneath
    // them on two grids: the tunnel grid, which stops a same-axis jump from
    // pairing with them, and `blocked`, which keeps everything else off --
    // applied before either end is chosen, so a perimeter pair is never
    // picked on top of a tunnel and then argued with.
    //
    // This branch used to pass `None` unconditionally under a "RULING 2"
    // that said the half of a pair could not be expressed by `FactorioEntity`
    // or by the mod. Every layer had since learned to carry it -- see
    // `underground_half_for_tile_kind`'s doc -- and the stuck belt-fed
    // sustain path (one cell, its coal route "blocked by 4 tiles") was the
    // cost of the comment outliving its reason.
    let reach = underground_reach(ctx);
    let tunnels = match reach {
        Some(reach) => {
            let (tunnels, reserved) = tunnel_grid(ctx, &area, origin, reach);
            for cell in reserved {
                blocked[enclosure::cell_index(cell.0, cell.1)] = true;
            }
            tunnels
        }
        None => vec![0u8; GRID * GRID],
    };

    // Each end is claimed onto `blocked` as soon as it is chosen, so the
    // second search sees what the first took. Without this, both ends are
    // "the first free perimeter pair of X" against the same static grid and
    // neither knows what the other claimed -- in tight geometry the two could
    // pick the same tile, and this function would go on to emit two `Place`
    // actions for it. Claiming turns that collision into a refusal instead:
    // the tile is no longer free for whichever search asks next, so it keeps
    // looking, and if nothing is left it refuses via `first_free_perimeter`'s
    // `Err` exactly as an ordinary blocked tile would.
    let source = first_free_perimeter(&blocked, origin, &from_footprint)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    blocked[enclosure::cell_index(source.inserter.0, source.inserter.1)] = true;
    blocked[enclosure::cell_index(source.belt.0, source.belt.1)] = true;

    let sink = first_free_perimeter(&blocked, origin, &to_footprint)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    blocked[enclosure::cell_index(sink.inserter.0, sink.inserter.1)] = true;

    // A CHEST'S OTHER SIDES ARE RESERVED, not routed over. Measured
    // 2026-09-09 on seed 31337: the haul into a cell's coal chest ended on
    // its north side and ran its last belts down the chest's EAST side on
    // the way in, so the chest's fourth side -- the one the next cell
    // needed -- was spent by a belt that had no business there, and the
    // next run refused with all four neighbours named. `method::sustain`'s
    // doc had already said where the fix belonged: "a real fix reserves the
    // perimeter in `method::connect`". So once the two ends are chosen, a
    // chest at either end closes every other side to this route.
    //
    // **Only this call's own two machines, and only if they are chests.**
    // The first version closed every chest in the window, and the one-cell
    // sustain arrangement in the fixtures -- three chests within a few
    // tiles -- lost every surface route and reached for a tunnel it cannot
    // craft. A bystander's sides are the bystander's own call's business.
    for (cell, owner) in container_sides(ctx, &area, origin) {
        let ours = same_tile(&owner, &from.position) || same_tile(&owner, &to.position);
        let chosen = [source.inserter, source.belt, sink.inserter, sink.belt].contains(&cell);
        if ours && !chosen {
            blocked[enclosure::cell_index(cell.0, cell.1)] = true;
        }
    }

    // THREATS: a belt is a standing structure, so the route prefers to keep
    // out of a charted enemy structure's reach -- but never at the price of
    // the route itself. `threatened_cells` is OR-ed onto a *copy* of the grid
    // and tried first; a refusal there falls through to the plain grid, which
    // is the search this function has always run. So the guard can move a
    // belt and can never delete one, the same prefer-then-fall-back shape
    // `method::util::free_area_near_where` uses for siting, and for the same
    // measured reason (a standing thing cannot walk out of range).
    //
    // The endpoints are deliberately NOT part of it: they are fixed by the
    // machines, and blocking them would refuse every route on the first
    // attempt and make the whole pass a wasted search.
    //
    // ORDER: surface first, on both grids, and only then a tunnel. A pair
    // is a last resort -- it costs the iron of some sixteen belts, needs a
    // recipe that is disabled at t=0, and reserves the ground beneath it --
    // so a route the surface can make, however long its detour, is the
    // route. This is what keeps every plan that routed before undergrounds
    // existed byte-for-byte the same plan.
    let avoiding = threatened_cells(ctx, origin).map(|threatened| {
        let mut avoiding = blocked.clone();
        for (cell, is_threatened) in threatened.iter().enumerate() {
            if *is_threatened {
                avoiding[cell] = true;
            }
        }
        avoiding[enclosure::cell_index(source.belt.0, source.belt.1)] = false;
        avoiding[enclosure::cell_index(sink.belt.0, sink.belt.1)] = false;
        avoiding
    });
    let search = |grid: &[bool], reach: Option<u8>| {
        route_belt_with_tunnels(grid, &tunnels, origin, source.belt, sink.belt, reach)
    };
    let route = avoiding
        .as_ref()
        .and_then(|grid| search(grid, None).ok())
        .or_else(|| search(&blocked, None).ok())
        .or_else(|| reach.and_then(|_| avoiding.as_ref().and_then(|grid| search(grid, reach).ok())))
        .map_or_else(|| search(&blocked, reach), Ok)
        .map_err(|error| match error {
            RouteError::NoPath { blocked } => ConnectRefusal::NoRoute { blocked },
            RouteError::SpanTooLong { needed, max } => ConnectRefusal::SpanTooLong { needed, max },
        })?;

    let source_anchor_pos = enclosure::cell_to_position(origin, source.anchor);
    let sink_anchor_pos = enclosure::cell_to_position(origin, sink.anchor);
    let belt_start_pos = enclosure::cell_to_position(origin, source.belt);
    let belt_end_pos = enclosure::cell_to_position(origin, sink.belt);
    let src_inserter_pos = enclosure::cell_to_position(origin, source.inserter);
    let dst_inserter_pos = enclosure::cell_to_position(origin, sink.inserter);

    // Load: moves items from the source machine onto the belt. Measured
    // between two cell centres two cells apart on one axis, never between a
    // machine's raw position and a cell centre -- see `ConnectRefusal::NotCardinal`.
    let load_facing =
        inserter_facing(&source_anchor_pos, &belt_start_pos).ok_or(ConnectRefusal::NotCardinal)?;
    // Unload: moves items off the belt into the destination machine.
    let unload_facing =
        inserter_facing(&belt_end_pos, &sink_anchor_pos).ok_or(ConnectRefusal::NotCardinal)?;

    // Nothing above this line has touched `ctx`; every refusal is returned
    // before a single action is emitted or a single entity added to the
    // overlay, which is the promise `ConnectRefusal` makes.
    let undergrounds = u32::try_from(
        route
            .tiles
            .iter()
            .filter(|t| t.kind != TileKind::Belt)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let belts = u32::try_from(route.tiles.len())
        .unwrap_or(u32::MAX)
        .saturating_sub(undergrounds);
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);

    let mut steps = Vec::with_capacity(route.tiles.len() + 5);
    if belts > 0 {
        steps.push(Step::Subgoal(Goal::Have {
            item: BELT.into(),
            count: belts,
            whose: Holder::Share(ctx.chain_actor),
            via: None,
        }));
    }
    if undergrounds > 0 {
        steps.push(Step::Subgoal(Goal::Have {
            item: UNDERGROUND.into(),
            count: undergrounds,
            whose: Holder::Share(ctx.chain_actor),
            via: None,
        }));
    }
    steps.push(Step::Subgoal(Goal::Have {
        item: inserter.into(),
        count: 2,
        whose: Holder::Share(ctx.chain_actor),
        via: None,
    }));

    let load =
        FactorioEntity::new_named_inserter(inserter.to_string(), &src_inserter_pos, load_facing);
    let note = format!("load {item} out of {}", from.name);
    steps.push(place_step(ctx, load, build, &note));

    let note = format!("carry {item} from {} to {}", from.name, to.name);
    for tile in &route.tiles {
        let entity = match underground_half_for_tile_kind(tile.kind) {
            None => FactorioEntity::new_transport_belt(&tile.position, tile.direction),
            // Both halves carry the tunnel's direction; `route_belt` keeps
            // the exit's step straight, so `tile.direction` is that for both.
            Some(half) => {
                FactorioEntity::new_underground_belt(&tile.position, tile.direction, half)
            }
        };
        steps.push(place_step(ctx, entity, build, &note));
    }

    let unload =
        FactorioEntity::new_named_inserter(inserter.to_string(), &dst_inserter_pos, unload_facing);
    let note = format!("unload {item} into {}", to.name);
    steps.push(place_step(ctx, unload, build, &note));

    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use factorio_bot_core::num_traits::ToPrimitive;

    /// `Direction` as the `u8` an emitted entity carries, for tests that
    /// assert a facing.
    fn dir(d: Direction) -> u8 {
        d.to_u8()
            .expect("every cardinal direction is representable")
    }

    fn placements(steps: &[Step], name: &str) -> Vec<(Position, u8)> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) => match &action.kind {
                    ActionKind::Place { entity } if entity.name == name => {
                        Some((entity.position.clone(), entity.direction))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

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

    /// **Pins the mapping `underground_half_for_tile_kind` documents but
    /// nothing calls yet**, so a future edit wiring it into `connect_steps`
    /// cannot silently transpose Entry/Exit and Input/Output -- the exact
    /// shape of mistake that would place two `output` halves (or two
    /// `input`s) and reproduce this project's defining failure with types
    /// instead of without them.
    #[test]
    fn underground_tile_kinds_map_to_the_correct_half() {
        assert_eq!(
            underground_half_for_tile_kind(TileKind::UndergroundEntry),
            Some(UndergroundHalf::Input),
            "the tile where the pair DIVES is the input half"
        );
        assert_eq!(
            underground_half_for_tile_kind(TileKind::UndergroundExit),
            Some(UndergroundHalf::Output),
            "the tile where the pair SURFACES is the output half"
        );
        assert_eq!(
            underground_half_for_tile_kind(TileKind::Belt),
            None,
            "an ordinary surface belt tile is neither half"
        );
    }

    /// **Defends the sign convention.** Screen coordinates here use +y for
    /// south, not north. A future edit that inverts the north/south directions
    /// would pass the horizontal tests but fail this one.
    #[test]
    fn an_inserter_picks_up_from_north_when_moving_south_in_screen_coordinates() {
        let north = Position::new(0.5, 0.5);
        let south = Position::new(0.5, 2.5);
        assert_eq!(
            inserter_facing(&north, &south),
            Some(Direction::North),
            "moving items north -> south (positive y) means facing NORTH, the pickup side"
        );
        assert_eq!(
            inserter_facing(&south, &north),
            Some(Direction::South),
            "and the reverse faces south"
        );
    }

    /// **The test the four earlier reviews did not have.** Both machines here
    /// are real Factorio shapes at *legal* Factorio positions, and neither is
    /// 1x1:
    ///
    /// * a `stone-furnace` built by the production constructor
    ///   (`FactorioEntity::new_stone_furnace`, box 1.8) at the **integer**
    ///   `(5.0, 5.0)` -- an even footprint covers two tiles per axis and so
    ///   sits on a tile *boundary*, which `method::util::tile_alignment`
    ///   spells out and which the old fixture's `(0.5, 0.5)` violated;
    /// * a `lab` at the **half-integer** `(12.5, 5.5)` with its real
    ///   2.3984375 box -- an odd 3x3 footprint sits on a tile centre.
    ///
    /// Against the pre-rewrite code the furnace refused `NotCardinal` (its
    /// integer position against a half-integer belt tile gave a diagonal) and
    /// the lab refused `NoRoute` (all four neighbours of its centre cell are
    /// inside its own 3x3), so this test fails loudly if the footprint
    /// geometry is reverted.
    ///
    /// **Asserts position AND direction, not just "an inserter exists at
    /// each end".** A name-only count cannot catch a regression that swaps
    /// the two `inserter_facing` calls, or feeds either the wrong pair of
    /// positions: that would still place one inserter at each end and pass a
    /// count-only check, while producing exactly the layout that places
    /// perfectly and moves nothing.
    ///
    /// The numbers are traced by hand. The window origin is
    /// `(floor(5) - 24, floor(5) - 24) = (-19, -19)`. The furnace covers
    /// cells x 23..=24, y 23..=24; its first free perimeter pair is North at
    /// x = 23, so the inserter takes cell (23, 22) = `(4.5, 3.5)` and the
    /// belt starts at (23, 21) = `(4.5, 2.5)`, with the anchor -- the
    /// footprint cell the inserter reaches into -- at (23, 23) = `(4.5,
    /// 4.5)`. Moving items south -> north picks up from the SOUTH. The lab
    /// covers x 30..=32, y 23..=25; its North pair at x = 30 gives inserter
    /// (30, 22) = `(11.5, 3.5)` and belt end (30, 21) = `(11.5, 2.5)`, anchor
    /// (30, 23) = `(11.5, 4.5)`; that inserter takes from the belt to its
    /// north, so it faces NORTH. Between the two belt cells the row y = 21 is
    /// open, so the route is eight tiles running east.
    #[test]
    fn a_connection_places_belts_and_an_inserter_at_each_end() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_on_open_ground();
        let steps = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("open ground between a 2x2 furnace and a 3x3 lab connects");

        assert_eq!(
            placements(&steps, "inserter"),
            vec![
                (Position::new(4.5, 3.5), dir(Direction::South)),
                (Position::new(11.5, 3.5), dir(Direction::North)),
            ],
            "load inserter north of the furnace facing South (picks up from \
             the machine), unload inserter north of the lab facing North \
             (picks up from the belt)"
        );

        let belts = placements(&steps, "transport-belt");
        assert_eq!(
            belts,
            (0..8)
                .map(|i| (Position::new(4.5 + f64::from(i), 2.5), dir(Direction::East)))
                .collect::<Vec<_>>(),
            "eight belt tiles running east along the open row between the two \
             inserters"
        );
    }

    /// The bill is the deliverable: a caller has to be able to refuse the
    /// whole plan before a single belt is placed, which is what these
    /// preconditions are for. This asserts the *shape* every other method in
    /// this crate emits -- `power.rs`'s plant parts are the reference -- so a
    /// regression back to bare `ActionKind`s with no id, no preconditions and
    /// no effects fails here rather than in a live run.
    #[test]
    fn every_placement_states_its_own_materials() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_on_open_ground();
        let steps = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("open ground between a 2x2 furnace and a 3x3 lab connects");

        let bill: Vec<(String, u32)> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Subgoal(Goal::Have { item, count, .. }) => Some((item.to_string(), *count)),
                _ => None,
            })
            .collect();
        assert_eq!(
            bill,
            vec![
                ("transport-belt".to_string(), 8),
                ("inserter".to_string(), 2)
            ],
            "eight belts and two inserters, stated as goals the shortfall \
             machinery can refuse"
        );

        let mut ids = std::collections::BTreeSet::new();
        for step in &steps {
            let Step::Act(action) = step else { continue };
            assert!(
                ids.insert(action.id),
                "every emitted action carries its own id: {:?} repeats",
                action.id
            );
            let ActionKind::Place { entity } = &action.kind else {
                panic!("connect emits only placements, got {:?}", action.kind)
            };
            assert!(
                action.pre.iter().any(|c| matches!(
                    c,
                    Condition::HasItem { item, count: 1, .. } if *item == entity.name
                )),
                "{} states it needs one of itself in hand",
                entity.name
            );
            assert!(
                action
                    .pre
                    .iter()
                    .any(|c| matches!(c, Condition::AreaFree { .. })),
                "{} states the ground it needs",
                entity.name
            );
            assert!(
                action
                    .pre
                    .iter()
                    .any(|c| matches!(c, Condition::AtPosition { .. })),
                "{} states the bot must be in build range",
                entity.name
            );
            assert!(
                action.eff.iter().any(|e| matches!(
                    e,
                    Effect::LoseItem { item, count: 1, .. } if *item == entity.name
                )),
                "{} spends the item it placed",
                entity.name
            );
            assert!(
                action
                    .eff
                    .iter()
                    .any(|e| matches!(e, Effect::CreateEntity(_))),
                "{} creates the entity it placed",
                entity.name
            );
            assert_eq!(
                action.duration, PLACE_TICKS,
                "{} takes a placement",
                entity.name
            );
        }
        assert_eq!(
            ids.len(),
            10,
            "eight belts and two inserters are ten actions"
        );
    }

    /// A wall wider than the prototype's reach still refuses, and refuses by
    /// the number: five columns need a pair six apart and the fixture's
    /// `underground-belt` reads 5. Nothing is placed and no id is spent.
    #[test]
    fn a_wall_wider_than_the_reach_refuses_by_span_and_places_nothing() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_behind_a_wide_wall();
        let before = ctx.ids.next();
        let refusal = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect_err("a five-wide wall is one wider than a reach of five can cross");
        assert!(
            matches!(refusal, ConnectRefusal::SpanTooLong { needed: 6, max: 5 }),
            "the refusal names both numbers in the prototype's unit: {refusal}"
        );
        assert_eq!(
            ctx.ids.next().0,
            before.0 + 1,
            "a refusal allocates no action id, because it emits no action"
        );
        assert!(
            ctx.state
                .entities_within(&from.position, 30.0)
                .iter()
                .all(|e| e.name != "transport-belt" && e.name != "underground-belt"),
            "a refusal leaves nothing in the overlay"
        );
    }

    /// **The unblocking test.** The one-wide wall at `x = 8.5` that used to
    /// refuse is crossed with an underground pair: an `input` half west of
    /// it, an `output` half east of it, both facing east, nothing placed in
    /// the wall, and the bill states the pair as `underground-belt` and the
    /// rest as `transport-belt`. Each half is asserted by position, half AND
    /// direction -- two `input`s, or an `output` facing the next turn,
    /// would place perfectly and move nothing.
    #[test]
    fn a_one_tile_wall_is_crossed_by_an_underground_pair() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_behind_a_wall();
        let steps = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("a one-wide wall is what an underground pair is for");

        let halves: Vec<(Position, u8, Option<UndergroundHalf>)> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) => match &action.kind {
                    ActionKind::Place { entity } if entity.name == "underground-belt" => Some((
                        entity.position.clone(),
                        entity.direction,
                        entity.underground_half,
                    )),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            halves,
            vec![
                (
                    Position::new(7.5, 2.5),
                    dir(Direction::East),
                    Some(UndergroundHalf::Input)
                ),
                (
                    Position::new(9.5, 2.5),
                    dir(Direction::East),
                    Some(UndergroundHalf::Output)
                ),
            ],
            "the pair dives at 7.5 and surfaces at 9.5, both facing the tunnel's way"
        );
        assert!(
            placements(&steps, "transport-belt")
                .iter()
                .all(|(p, _)| p.x() != 8.5),
            "nothing is placed inside the wall"
        );

        let bill: Vec<(String, u32)> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Subgoal(Goal::Have { item, count, .. }) => Some((item.to_string(), *count)),
                _ => None,
            })
            .collect();
        assert_eq!(
            bill,
            vec![
                ("transport-belt".to_string(), 5),
                ("underground-belt".to_string(), 2),
                ("inserter".to_string(), 2),
            ],
            "eight cells between the inserters less the wall cell: five belts and one \
             pair, stated as goals the shortfall machinery can refuse"
        );
    }

    /// The ground beneath an existing pair is off limits to a same-axis jump.
    /// A pair already stands across the wall on the route's own row --
    /// `input` at 7.5, `output` at 9.5, facing east -- so the straight
    /// route's natural move is a jump from 6.5 over the three cells 7.5,
    /// 8.5, 9.5 to 10.5: four apart, well within reach, and it would pair
    /// with the standing halves instead of with itself. With the tunnel
    /// reserved the search must cross on another row with its own pair and
    /// put nothing on the reserved cells.
    #[test]
    fn a_route_keeps_off_the_ground_beneath_an_existing_pair() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_behind_a_wall();
        let standing_in = FactorioEntity::new_underground_belt(
            &Position::new(7.5, 2.5),
            Direction::East,
            UndergroundHalf::Input,
        );
        let standing_out = FactorioEntity::new_underground_belt(
            &Position::new(9.5, 2.5),
            Direction::East,
            UndergroundHalf::Output,
        );
        let reserved = tunnel_cells(&standing_in.position, &standing_out.position);
        ctx.state.create_entity(standing_in);
        ctx.state.create_entity(standing_out);

        let steps = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("the wall is still crossable on a row of its own");
        let laid: Vec<(String, Position)> = steps
            .iter()
            .filter_map(|s| match s {
                Step::Act(a) => match &a.kind {
                    ActionKind::Place { entity }
                        if entity.name == "underground-belt" || entity.name == "transport-belt" =>
                    {
                        Some((entity.name.clone(), entity.position.clone()))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let pair: Vec<&Position> = laid
            .iter()
            .filter(|(n, _)| n == "underground-belt")
            .map(|(_, p)| p)
            .collect();
        assert_eq!(
            pair.len(),
            2,
            "the route still needs its own pair: {laid:?}"
        );
        assert!(
            pair.iter().all(|p| p.y() != 2.5),
            "the new pair is on a different row from the standing one: {laid:?}"
        );
        for (_, tile) in &laid {
            assert!(
                !reserved.iter().any(|p| p == tile),
                "{tile} lies on the standing pair's tunnel {reserved:?}"
            );
        }
    }

    /// A route may not spend the sides of the chest it serves. The fixture's
    /// cheapest route runs down the chest's west side to reach its south
    /// arm -- the shape the measured haul took on seed 31337, leaving the
    /// next cell no side to load from. Both west cells must stay empty, and
    /// the run must still be made: the long way round exists.
    #[test]
    fn a_route_keeps_off_the_other_sides_of_the_chest_it_serves() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_chest_hugged_on_the_way_in();
        let steps = connect_steps(&mut ctx, &from, &to, &"coal".into())
            .expect("the long way round is open");
        let unload = placements(&steps, "inserter");
        assert_eq!(
            unload.last().map(|(p, _)| p.clone()),
            Some(Position::new(12.5, 3.5)),
            "fixture precondition: the chest is loaded from its south side"
        );
        let laid: Vec<Position> = placements(&steps, "transport-belt")
            .into_iter()
            .map(|(p, _)| p)
            .chain(
                placements(&steps, "underground-belt")
                    .into_iter()
                    .map(|(p, _)| p),
            )
            .collect();
        for side in [Position::new(11.5, 2.5), Position::new(10.5, 2.5)] {
            assert!(
                !laid.contains(&side),
                "the run hugs the chest's west side at {side}: {laid:?}"
            );
        }
        assert!(
            laid.len() > 10,
            "the detour is the long way round, not a shorter run through the wall: {laid:?}"
        );
    }

    /// The ground beneath a standing pair is not free ground, even where it
    /// is empty. The lab's only open north pair sits on the span between a
    /// standing `input` and `output` (see the fixture); the run must not end
    /// there, on either free span cell, and must still be made from another
    /// side. The tunnel-axis bit alone cannot enforce this -- it gates jumps,
    /// not perimeter choice -- so this is the reservation-as-blocked rule's
    /// own test.
    #[test]
    fn a_run_does_not_end_on_the_ground_beneath_a_standing_pair() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_beside_a_standing_span();
        let standing_in = FactorioEntity::new_underground_belt(
            &Position::new(6.5, 2.5),
            Direction::East,
            UndergroundHalf::Input,
        );
        let standing_out = FactorioEntity::new_underground_belt(
            &Position::new(10.5, 2.5),
            Direction::East,
            UndergroundHalf::Output,
        );
        let span = tunnel_cells(&standing_in.position, &standing_out.position);
        ctx.state.create_entity(standing_in);
        ctx.state.create_entity(standing_out);
        let steps = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("the lab has other sides");
        let laid: Vec<Position> = placements(&steps, "transport-belt")
            .into_iter()
            .chain(placements(&steps, "inserter"))
            .chain(placements(&steps, "underground-belt"))
            .map(|(p, _)| p)
            .collect();
        for tile in &span {
            assert!(
                !laid.contains(tile),
                "{tile} lies beneath the standing pair {span:?}: {laid:?}"
            );
        }
    }

    /// **The IMPORTANT-2 regression test.** Each end is found by a search
    /// that does not know what the other claimed. In tight geometry the two
    /// can land on the very same tiles, and without the claim-as-you-go grid
    /// `connect_steps` would emit two `Place` actions for one cell: a belt
    /// from one machine's route and an inserter from the other's.
    ///
    /// `furnaces_sharing_a_perimeter` engineers exactly that: the second
    /// furnace's only unwalled perimeter pair is the very inserter/belt pair
    /// the first furnace already claimed. With the two ends aware of each
    /// other, the second furnace's search finds them taken, has nowhere else
    /// to go, and refuses -- it must not silently accept the shared tiles.
    #[test]
    fn machines_close_enough_to_share_a_derived_tile_refuse_instead_of_colliding() {
        let (mut ctx, from, to) = crate::test_world::furnaces_sharing_a_perimeter();
        let refusal = connect_steps(&mut ctx, &from, &to, &"iron-plate".into()).expect_err(
            "every perimeter pair of the second furnace is either walled or \
             already claimed by the first furnace's own end",
        );
        assert!(matches!(refusal, ConnectRefusal::NoRoute { .. }));
    }

    /// A tree at an arbitrary sub-tile position blocks the tile it stands on.
    ///
    /// **The zero-half-box defect, pinned.** `rasterize` was called with
    /// `(0.0, 0.0)`, so a cell counted as blocked only when an obstacle
    /// covered its exact centre. A 0.8-wide tree at `(9.1, 2.9)` covers
    /// `x 8.7..9.5`, `y 2.5..3.3` and misses the centre of cell `(9.5, 2.5)`
    /// on the y axis by 0.2 tiles -- so the route went straight through it and
    /// the build failed partway. Grown by the belt's own half-box it blocks
    /// that cell, and the route detours.
    #[test]
    fn a_tree_off_the_tile_centre_still_blocks_a_belt() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_with_a_tree();
        let steps = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("a single tree is routed around, not refused");
        let belts = placements(&steps, "transport-belt");
        assert!(
            !belts.iter().any(|(pos, _)| pos == &Position::new(9.5, 2.5)),
            "the belt must not run through the tree's tile: {belts:?}"
        );
        assert!(
            belts.len() > 8,
            "and the detour costs it at least one extra tile: {belts:?}"
        );
    }

    /// The tile the straight belt runs through, between the two machines of
    /// `furnace_and_lab_on_open_ground`. `a_tree_off_the_tile_centre_still_
    /// blocks_a_belt` above asserts the detour against the same cell, from an
    /// obstacle instead of a threat, so the two guards are measured against
    /// one geometry.
    const ON_THE_STRAIGHT_ROUTE: Position = Position { x: 9.5, y: 2.5 };

    /// `furnace_and_lab_on_open_ground` plus one enemy structure whose
    /// standoff is `reach` tiles.
    ///
    /// **The reach is stated by a PROTOTYPE, not by the worm table**, which is
    /// the only way to get a small one: `PlanState::threat_standoff` reads
    /// `FactorioEntityPrototype::attack_range` first and falls back to
    /// `WORM_ATTACK_RANGE`'s 25 tiles, and a 25-tile disc swallows a 48-tile
    /// window's endpoints along with everything else -- there would be no
    /// detour to find. A small reach is also the honest shape of the question
    /// this tests: whether the router *prefers* clear ground, not whether it
    /// can escape a whole nest.
    ///
    /// The threat stands two tiles off the belt row rather than on it, so its
    /// own collision box blocks nothing and the only thing that can move the
    /// route is the standoff. That distinction cost this session one failing
    /// test on the siting side, where a worm parked on the origin made the
    /// ground unbuildable and the guard was never involved.
    fn furnace_and_lab_with_a_threat(reach: f64) -> (ExpansionCtx, FactorioEntity, FactorioEntity) {
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let world = fixture_world();
        let mut prototype = world
            .globals
            .entity_prototypes
            .get("stone-furnace")
            .expect("the fixture ships a stone-furnace prototype")
            .clone();
        prototype.attack_range = Some(reach);
        world
            .globals
            .entity_prototypes
            .insert("small-worm-turret".into(), prototype);

        let furnace = FactorioEntity::new_stone_furnace(&Position::new(5.0, 5.0), Direction::North);
        let lab_position = Position::new(12.5, 5.5);
        let mut lab = FactorioEntity::new_stone_furnace(&lab_position, Direction::North);
        lab.name = "lab".to_owned();
        lab.entity_type = "lab".to_owned();
        lab.bounding_box = factorio_bot_core::factorio::util::rect_floor_ceil(
            &factorio_bot_core::factorio::util::add_to_rect(
                &Rect::new(
                    &Position::new(-1.199_218_75, -1.199_218_75),
                    &Position::new(1.199_218_75, 1.199_218_75),
                ),
                &lab_position,
            ),
        );
        let mut worm = FactorioEntity::new_stone_furnace(
            &Position::new(ON_THE_STRAIGHT_ROUTE.x, ON_THE_STRAIGHT_ROUTE.y - 2.),
            Direction::North,
        );
        worm.name = "small-worm-turret".to_owned();
        worm.entity_type = "turret".to_owned();

        world
            .update_chunk_entities(vec![furnace.clone(), lab.clone(), worm])
            .expect("a fixture world accepts these entities");
        let ctx = ExpansionCtx::new(
            crate::state::PlanState::from_world(Arc::new(world), &[]),
            BotId(1),
        );
        (ctx, furnace, lab)
    }

    /// A belt is a **standing** structure, so the route prefers to keep out of
    /// a charted enemy structure's reach.
    ///
    /// That preference is worth having because standing exposure is what a
    /// worm punishes: measured live on 2026-09-08
    /// (`scripts/threat_pass_probe.sh`), a character *standing* 24 tiles from
    /// a `small-worm-turret` lost 153 of 250 health in 300 ticks while one
    /// *walking past* at the same distance lost none.
    #[test]
    fn a_belt_route_prefers_ground_outside_a_threats_standoff() {
        let (mut ctx, from, to) = furnace_and_lab_with_a_threat(2.5);
        assert!(
            ctx.state.threat_covering(&ON_THE_STRAIGHT_ROUTE).is_some(),
            "fixture precondition: the threat must cover the straight route's own tile"
        );
        let steps = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("a small standoff is routed around, not refused");
        let belts = placements(&steps, "transport-belt");
        assert!(
            !belts.iter().any(|(pos, _)| pos == &ON_THE_STRAIGHT_ROUTE),
            "the belt must not run through the threatened tile: {belts:?}"
        );
        for (pos, _) in &belts {
            assert!(
                ctx.state.threat_covering(pos).is_none(),
                "and no belt tile at all may sit inside the standoff: {pos}"
            );
        }
    }

    /// **And the preference never costs a route.** With a standoff wide enough
    /// to cover the whole search window there is no clear path, and the answer
    /// must be the belt run this function laid before the guard existed --
    /// not a refusal. A refusal where a plan used to exist is a defect, not
    /// caution; the same fallback shape `method::util::free_area_near_where`
    /// uses for siting.
    #[test]
    fn a_threat_over_the_whole_window_still_yields_the_unguarded_route() {
        let (mut ctx, from, to) = furnace_and_lab_with_a_threat(200.);
        assert!(
            ctx.state
                .threat_covering(&Position::new(5.0, 5.0))
                .is_some(),
            "fixture precondition: the standoff must cover even the source machine, or the first \
             pass would succeed and this would not be testing the fallback"
        );
        let guarded = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect("a threat must not delete a route that exists");

        let (mut plain, from, to) = crate::test_world::furnace_and_lab_on_open_ground();
        let unguarded = connect_steps(&mut plain, &from, &to, &"iron-plate".into())
            .expect("the control routes on open ground");
        assert_eq!(
            placements(&guarded, "transport-belt"),
            placements(&unguarded, "transport-belt"),
            "with nowhere clear to go, the route must be exactly the one laid before the guard"
        );
    }

    /// A `BotId` with no bot behind it is the fixture's ordinary state; the
    /// build radius falls back to 10 tiles, as every other method's does.
    #[test]
    fn the_chain_actor_is_the_one_the_context_carries() {
        let (ctx, _, _) = crate::test_world::furnace_and_lab_on_open_ground();
        assert_eq!(ctx.chain_actor, BotId(1));
    }
    /// **Chest to chest is a shape this module already has**, and this test
    /// is the measurement that says so.
    ///
    /// Both this module's doc and `method::assemble`'s said the opposite --
    /// that `connect_steps` "routes a belt run between two **machines**" and
    /// that chest-to-chest "is not a shape it has" -- and three separate
    /// sessions named that as the gap between a charged cell and a factory.
    /// Nothing in the code ever restricted it: `footprint_of` reads a
    /// `bounding_box`, and a 1x1 container has one. The two ends here are
    /// `iron-chest`s eight tiles apart, and the run that comes back is the
    /// whole arrangement -- an arm on each chest, a belt row between them,
    /// and both facings naming the side each arm PICKS UP from.
    ///
    /// What *is* scarce is a chest's sides: it has four, and this run spends
    /// one at each end (see `container_sides`).
    #[test]
    fn a_run_between_two_chests_is_routed() {
        let (mut ctx, source, sink) = crate::test_world::two_chests_on_open_ground();
        let steps = connect_steps_with(&mut ctx, &source, &sink, &"iron-plate".into(), INSERTER)
            .expect("two chests eight tiles apart on open ground");
        assert_eq!(
            placements(&steps, INSERTER),
            vec![
                // On the source chest's north side, picking up from the
                // SOUTH -- which is the chest.
                (Position::new(4.5, 4.5), dir(Direction::South)),
                // On the sink chest's north side, picking up from the
                // NORTH -- which is the belt.
                (Position::new(12.5, 4.5), dir(Direction::North)),
            ],
            "an arm at each end, each facing what it picks up from"
        );
        let belts = placements(&steps, BELT);
        assert_eq!(belts.len(), 9, "nine belt tiles from x=4.5 to x=12.5");
        assert!(
            belts
                .iter()
                .all(|(at, facing)| at.y() == 3.5 && *facing == dir(Direction::East)),
            "one straight row, running east: {belts:?}"
        );
    }
}
