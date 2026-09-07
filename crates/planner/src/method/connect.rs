//! Connecting two entities with a belt, and the inserters at each end.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::enclosure;
use crate::goal::{Goal, Holder};
use crate::ids::ItemId;
use crate::method::have::PLACE_TICKS;
use crate::method::{ExpansionCtx, Step};
use factorio_bot_core::blueprint::UndergroundHalf;
use factorio_bot_core::graph::enclosure::GRID;
use factorio_bot_core::graph::route::{RouteError, TileKind, route_belt};
use factorio_bot_core::types::{Direction, FactorioEntity, Position, Rect};

/// The belt this module lays, and the item whose bill it states.
const BELT: &str = "transport-belt";
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
/// **Not called by `connect_steps` yet** -- its `TileKind::UndergroundEntry |
/// TileKind::UndergroundExit` arm still panics on purpose, because
/// `route_belt` is always called with `max_underground: None` and so can
/// never produce one (see the `unreachable!()` there, and RULING 2 above
/// `route_belt`'s call). Wiring that up -- threading a real
/// `max_underground` through and replacing the panic with a call to this
/// function -- is follow-on work. This function and its test exist now, on
/// their own, so that day's edit has a pinned answer to consult rather than
/// a chance to silently transpose the two halves and reproduce the "places
/// perfectly, connects nothing" failure with types instead of without them.
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
    /// An underground span longer than the belt prototype allows.
    ///
    /// Structurally unreachable while `connect_steps` always calls
    /// `route_belt` with `max_underground: None` (RULING 2 of the task-4
    /// brief) -- with `None`, `route_belt` never attempts an underground
    /// move at all, so it can never report one being too long either. Kept
    /// so this type matches `RouteError` one-to-one rather than silently
    /// dropping a variant a future change to that call might need.
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
/// # The geometry
///
/// `from` and `to` are the two **machines**, and they are taken at their real
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
/// # Scope: only the base world plus these two machines is an obstacle
///
/// Obstacles come from `state.base().entity_graph.blocking_boxes_within`
/// plus the two footprints stated above -- the world as the game (or a
/// fixture) reported it, never the rest of this plan's `added` overlay. A
/// caller chaining two connections in the same plan must not treat the first
/// call's output as ground truth for the second: this function cannot see a
/// machine, inserter or belt that an *earlier* action in the same plan
/// placed, so nothing here stops the two from overlapping. Widening this to
/// read the whole overlay is future work, not a guarantee this function
/// already makes.
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
/// (`crate::state`'s `inserter_reach` already lists them together).
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
    // `sink.belt` itself is deliberately NOT claimed: it is the route's
    // destination, and `route_belt` consults `blocked` for every cell it
    // steps *into*, including that one. `source.belt` is safe to claim
    // because `route_belt` seeds its start state unconditionally and only
    // ever leaves that cell.
    //
    // RULING 2: undergrounds are never emitted on this branch, always and
    // deliberately -- do not "fix" this by threading a real
    // `max_underground` through. A Factorio underground-belt pair needs one
    // half `input` and one half `output`; `FactorioEntity` has no field to
    // record which, and the mod's `rcon_place_entity(player_id, item_name,
    // position, direction)` has no argument for it either. Emitting two
    // identical halves would place both perfectly and move nothing -- this
    // project's defining failure. A route that would need to go underground
    // must refuse instead, which passing `None` here guarantees: `route_belt`
    // never attempts an underground move without a `max_underground`.
    let route =
        route_belt(&blocked, origin, source.belt, sink.belt, None).map_err(
            |error| match error {
                RouteError::NoPath { blocked } => ConnectRefusal::NoRoute { blocked },
                RouteError::SpanTooLong { needed, max } => {
                    ConnectRefusal::SpanTooLong { needed, max }
                }
            },
        )?;

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
    let belts = u32::try_from(route.tiles.len()).unwrap_or(u32::MAX);
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);

    let mut steps = Vec::with_capacity(route.tiles.len() + 4);
    steps.push(Step::Subgoal(Goal::Have {
        item: BELT.into(),
        count: belts,
        whose: Holder::Share(ctx.chain_actor),
        via: None,
    }));
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
        let entity = match tile.kind {
            TileKind::Belt => FactorioEntity::new_transport_belt(&tile.position, tile.direction),
            // See the comment at the `route_belt` call above: with
            // `max_underground: None` the search can never take the branch
            // that produces these, so this arm can never run.
            TileKind::UndergroundEntry | TileKind::UndergroundExit => unreachable!(
                "connect_steps always calls route_belt with max_underground: None, \
                 so it can never emit an underground route tile"
            ),
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

    #[test]
    fn a_walled_destination_refuses_and_places_nothing() {
        let (mut ctx, from, to) = crate::test_world::furnace_and_lab_behind_a_wall();
        let before = ctx.ids.next();
        let refusal = connect_steps(&mut ctx, &from, &to, &"iron-plate".into())
            .expect_err("a walled destination has no route");
        assert!(matches!(refusal, ConnectRefusal::NoRoute { .. }));
        assert_eq!(
            ctx.ids.next().0,
            before.0 + 1,
            "a refusal allocates no action id, because it emits no action"
        );
        assert!(
            !refusal.to_string().is_empty(),
            "a refusal a caller can surface"
        );
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

    /// A `BotId` with no bot behind it is the fixture's ordinary state; the
    /// build radius falls back to 10 tiles, as every other method's does.
    #[test]
    fn the_chain_actor_is_the_one_the_context_carries() {
        let (ctx, _, _) = crate::test_world::furnace_and_lab_on_open_ground();
        assert_eq!(ctx.chain_actor, BotId(1));
    }
}
