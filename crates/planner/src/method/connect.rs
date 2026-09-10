//! Connecting two entities with a belt, and the inserters at each end.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::enclosure;
use crate::goal::{Goal, Holder};
use crate::ids::{BotId, ItemId, Ticks};
use crate::method::have::{HANDOVER_WALK_TICKS, PLACE_TICKS, participants_that_can_work};
use crate::method::produce::{CRAFT_TICKS_MAX_DEPTH, craft_ticks};
use crate::method::util::{mine_bill, mining_ticks};
use crate::method::{ExpansionCtx, Step};
use crate::state::PlanState;
use factorio_bot_core::blueprint::UndergroundHalf;
use factorio_bot_core::graph::enclosure::GRID;
use factorio_bot_core::graph::route::{
    Route, RouteError, RouteTile, TileKind, route_belt_launching, route_belt_with_tunnels,
    tunnel_axis, tunnel_cells,
};
use factorio_bot_core::types::{Direction, FactorioEntity, Pos, Position, Rect};
use std::collections::BTreeMap;

/// The belt this module lays, and the item whose bill it states.
const BELT: &str = "transport-belt";
/// The underground pair that carries [`BELT`] beneath an obstacle. Paired
/// with `BELT` by hand because the prototype does not name its partner: the
/// day this module lays a faster belt, this is the second name to change.
pub(crate) const UNDERGROUND: &str = "underground-belt";
/// The inserter at each end when a caller does not name one.
///
/// **Not craftable at stage 1**, which is the whole reason
/// [`connect_steps_with`] exists. `inserter`'s recipe takes an
/// `electronic-circuit` and reads `enabled: false` on a freeplay force —
/// checked against seed 31337's own t=0 dump, not assumed — so a plan that
/// places one before `electronics` is researched refuses on the bill. A
/// `burner-inserter` is 1 iron plate and 1 gear and is enabled from the start.
const INSERTER: &str = "inserter";

/// The prototype a [`tap_standing_run`] splices into a standing belt run.
///
/// **Its shape, read off the prototype and off `EntityGraph`'s own splitter
/// arm, not off a picture of one**: a splitter is **two tiles wide across
/// the direction of travel and one tile long along it** -- collision box
/// 1.796875 x 0.796875 facing north (`crates/core/tests/entity-prototype-
/// fixtures.json`), so it straddles two side-by-side belt lanes and takes
/// one step of each. Both lanes enter on its back edge and both leave on
/// its front edge; `entity_graph.rs` derives the two output tiles as
/// `(-0.5, -1)` and `(0.5, -1)` turned to the facing, and a splitter's
/// `position` is the midpoint of its two tiles (a half-integer on one axis
/// and a whole number on the other). It divides what arrives across the two
/// outputs and sends everything to whichever one is not backed up, so a
/// belt run tapped by one keeps its whole flow when the branch is full and
/// loses nothing when the original destination is.
///
/// Its recipe is disabled at t=0 -- it needs `logistics`, like
/// [`UNDERGROUND`] -- so a plan that taps carries that research; the
/// `Goal::Have` for it says so through the ordinary shortfall machinery.
const SPLITTER: &str = "splitter";

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
    /// The source's perimeter has no free `(inserter, belt)` pair -- the
    /// [`NoRoute`](Self::NoRoute) refusal, `blocked` naming the same tiles --
    /// AND the fallback that a boxed-in source gets, tapping a belt run that
    /// already leaves it with a splitter ([`tap_standing_run`]), refused
    /// too. `why` is the tap's own sentence: no run leaves the source, no
    /// straight stretch of it has a free side, or no branch could be routed
    /// from any splice to the destination.
    ///
    /// A variant of its own rather than a longer `NoRoute`, so a reader of
    /// the refusal can tell "nothing was tried" from "the tap was tried and
    /// this is why it failed" -- silence about a fallback is how a fallback
    /// goes unmeasured.
    TapRefused { blocked: Vec<Position>, why: String },
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
            ConnectRefusal::TapRefused { blocked, why } => {
                write!(
                    f,
                    "{}; and the run already leaving it could not be tapped with a \
                     splitter: {why}",
                    ConnectRefusal::NoRoute {
                        blocked: blocked.clone()
                    }
                )
            }
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

/// The first free `(inserter, belt)` pair on `footprint`'s perimeter -- a
/// pair the plan has KEPT for this end taken before any other.
///
/// Both cells are required free together: an inserter with nowhere to put the
/// belt is not a usable end, and taking the inserter tile anyway is what made
/// the old chain bend into a diagonal.
///
/// # The kept pair goes first, and only a whole pair counts
///
/// `kept` is the ground `PlanState::reserve_ground` holds for the run that
/// leaves this machine (`method::sustain`'s product exit: the arm's tile and
/// the belt's tile beyond it). Until 2026-09-09 this scan had no way to see
/// it: the call site exempted a kept tile from the obstacle grid, so the exit
/// was *usable*, and then walked the perimeter North, East, South, West and
/// took the first free pair -- which on `run-1788936524-99544` was the plate
/// chest's NORTH side, with the kept east exit standing open one tile away.
/// The link's own belt and pole then sealed that exit into a pocket, and the
/// replan tunnelled out of it. A side kept for a run and not taken by it is
/// kept for nothing.
///
/// So a candidate whose inserter cell AND belt cell are both in `kept` is
/// tried first; every other candidate follows in the fixed order. **Both
/// cells, deliberately.** A reservation is two tiles in a line out of one
/// chest, and only that exact pair is this end's exit. A neighbouring chest's
/// exit can put one of its tiles on this perimeter as a lone inserter cell or
/// a lone belt cell, and preferring that would spend another cell's way out
/// on a run it was never kept for. With `kept` empty the scan is byte for
/// byte what it was.
///
/// `Err` names every occupied tile that stopped it, deduplicated and in
/// ascending cell order -- a fixed order rather than an artefact of the scan.
/// An off-grid candidate is skipped rather than named: it has no position
/// this window can state.
fn first_free_perimeter(
    blocked: &[bool],
    origin: (f64, f64),
    footprint: &Footprint,
    kept: &[(usize, usize)],
) -> Result<Endpoint, Vec<Position>> {
    let mut stopped: Vec<(usize, usize)> = Vec::new();
    let candidates = perimeter(footprint);
    let is_kept = |((x, y), (dx, dy)): &((i64, i64), (i64, i64))| {
        matches!(
            (in_grid(*x, *y), in_grid(x + dx, y + dy)),
            (Some(inserter), Some(belt)) if kept.contains(&inserter) && kept.contains(&belt)
        )
    };
    let ordered = candidates
        .iter()
        .filter(|candidate| is_kept(candidate))
        .chain(candidates.iter().filter(|candidate| !is_kept(candidate)));
    for &((x, y), (dx, dy)) in ordered {
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

/// A belt run that already stands between `from` and `to`, wanting only its
/// two arms -- the shape a replan meets after a batch was cut short.
///
/// # Why this exists
///
/// Every electric `inserter` of a link needs a circuit, and the circuit's
/// copper comes off the same cell's supply take; when that take fails, the
/// executor abandons its whole dependency cone and **every arm of the link
/// with it, while every belt stands** (`plan --replan 1 --fail "copper-plate
/// from the cell"` on seed 31337: 25 belts standing from the plate chest's
/// kept exit to the supply chest's door, and no arm at either end). To the
/// replan those belts were obstacles: the exit's belt tile was "taken", the
/// chest reported its next side as the one it had left, and a second,
/// duplicate run was laid from there -- or refused when no side was left.
///
/// # What counts as this run
///
/// Nothing here tracks who laid a belt; a belt is a belt. The criterion is
/// geometric and about BOTH ends: on some perimeter side of `from` the
/// arm's tile is free and the belt's tile holds a standing `transport-belt`;
/// following that belt's direction tile by tile through standing belts ends
/// on a tile that is the belt cell of a perimeter side of `to` whose arm
/// tile is free. A chain that leaves the door and goes anywhere else -- a
/// coal run passing the chest, a run to some other machine -- ends somewhere
/// that is not `to`'s door and is not matched. A chain with an underground
/// pair in it is followed only to the pair: pairing the halves is
/// `route_belt`'s business, and the surface search then runs as before.
///
/// The kept exit (`PlanState::reserve_ground`) needs no special case: the
/// belt standing on the exit's belt tile is exactly what this looks for.
///
/// Returns the two ends in `first_free_perimeter`'s own shape so the arms
/// are placed with the same facings a fresh run would give them.
fn standing_run(
    state: &PlanState,
    inserter: &str,
    origin: (f64, f64),
    from_footprint: &Footprint,
    to_footprint: &Footprint,
) -> Option<(Endpoint, Endpoint)> {
    use factorio_bot_core::num_traits::FromPrimitive;
    let belt_at = |cell: (usize, usize)| -> Option<FactorioEntity> {
        let at = enclosure::cell_to_position(origin, cell);
        state
            .entity_at(&at)
            .filter(|entity| entity.name == BELT && Pos::from(&entity.position) == Pos::from(&at))
    };
    let arm_free = |cell: (usize, usize)| -> bool {
        state.is_area_free(inserter, &enclosure::cell_to_position(origin, cell))
    };
    let doors_of = |footprint: &Footprint| -> Vec<Endpoint> {
        perimeter(footprint)
            .into_iter()
            .filter_map(|((x, y), (dx, dy))| {
                Some(Endpoint {
                    anchor: in_grid(x - dx, y - dy)?,
                    inserter: in_grid(x, y)?,
                    belt: in_grid(x + dx, y + dy)?,
                })
            })
            .collect()
    };
    let sinks = doors_of(to_footprint);
    for source in doors_of(from_footprint) {
        if !arm_free(source.inserter) {
            continue;
        }
        let Some(head) = belt_at(source.belt) else {
            continue;
        };
        // Follow the chain downstream. Bounded by the window: a cell is
        // visited once, and a step off the grid ends the chain.
        let mut visited = vec![false; GRID * GRID];
        let mut at = source.belt;
        let mut belt = head;
        loop {
            visited[enclosure::cell_index(at.0, at.1)] = true;
            let Some(facing) = Direction::from_u8(belt.direction) else {
                break;
            };
            let Some(step) = Position::new(0., -1.).turn(facing) else {
                break;
            };
            let Some(next) = in_grid(
                at.0 as i64 + step.x().round() as i64,
                at.1 as i64 + step.y().round() as i64,
            ) else {
                break;
            };
            if visited[enclosure::cell_index(next.0, next.1)] {
                break;
            }
            let Some(next_belt) = belt_at(next) else {
                break;
            };
            at = next;
            belt = next_belt;
        }
        let tail = at;
        if let Some(sink) = sinks
            .iter()
            .find(|sink| sink.belt == tail && arm_free(sink.inserter))
        {
            return Some((source, *sink));
        }
    }
    None
}

/// The steps that finish a [`standing_run`]: the bill for two arms and the
/// two `Place`s, with the facings a fresh run would give them. No belt, no
/// band, no underground.
#[allow(clippy::too_many_arguments)]
fn arms_only(
    ctx: &mut ExpansionCtx,
    from: &FactorioEntity,
    to: &FactorioEntity,
    item: &ItemId,
    inserter: &str,
    origin: (f64, f64),
    source: Endpoint,
    sink: Endpoint,
) -> Result<Vec<Step>, ConnectRefusal> {
    let source_anchor_pos = enclosure::cell_to_position(origin, source.anchor);
    let sink_anchor_pos = enclosure::cell_to_position(origin, sink.anchor);
    let belt_start_pos = enclosure::cell_to_position(origin, source.belt);
    let belt_end_pos = enclosure::cell_to_position(origin, sink.belt);
    let src_inserter_pos = enclosure::cell_to_position(origin, source.inserter);
    let dst_inserter_pos = enclosure::cell_to_position(origin, sink.inserter);
    let load_facing =
        inserter_facing(&source_anchor_pos, &belt_start_pos).ok_or(ConnectRefusal::NotCardinal)?;
    let unload_facing =
        inserter_facing(&belt_end_pos, &sink_anchor_pos).ok_or(ConnectRefusal::NotCardinal)?;
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
    let mut steps = Vec::with_capacity(3);
    steps.push(Step::Subgoal(Goal::Have {
        item: inserter.into(),
        count: 2,
        whose: Holder::Share(ctx.chain_actor),
        via: None,
    }));
    let load =
        FactorioEntity::new_named_inserter(inserter.to_string(), &src_inserter_pos, load_facing);
    let note = format!(
        "load {item} out of {} (the belt to {} stands)",
        from.name, to.name
    );
    steps.push(place_step(ctx, load, build, &note));
    let unload =
        FactorioEntity::new_named_inserter(inserter.to_string(), &dst_inserter_pos, unload_facing);
    let note = format!(
        "unload {item} into {} (the belt from {} stands)",
        to.name, from.name
    );
    steps.push(place_step(ctx, unload, build, &note));
    Ok(steps)
}

// ---------------------------------------------------------------------------
// The tap: a boxed-in source whose run already leaves it
// ---------------------------------------------------------------------------

/// One tile of a belt chain leaving the source, as [`outbound_chains`]
/// reads it off the state.
#[derive(Debug, Clone, Copy)]
struct ChainTile {
    cell: (usize, usize),
    facing: Direction,
}

/// Every belt chain that leaves `from` through an arm that picks up from
/// it, each as its tiles from the door's belt cell downstream, in chain
/// order. Belts only: a chain is followed to the first tile that is not a
/// `transport-belt`, exactly as [`standing_run`] follows one.
///
/// The door test is the arm's FACING: an inserter on a perimeter cell whose
/// `direction` is the one [`inserter_facing`] gives for picking up from the
/// machine and dropping on the belt cell beyond. An arm facing the other way
/// is a run INTO the machine, and tapping it would carry the wrong thing
/// backwards; it is not a door here. Nothing tracks who laid the arm or the
/// belts -- a run is a run -- and nothing here can say what the run carries:
/// it carries whatever that arm lifts out of `from`, which for the chests
/// and machines this module joins is the one thing they hold.
fn outbound_chains(
    state: &PlanState,
    origin: (f64, f64),
    from_footprint: &Footprint,
) -> Vec<Vec<ChainTile>> {
    use factorio_bot_core::num_traits::{FromPrimitive, ToPrimitive};
    let belt_at = |cell: (usize, usize)| -> Option<FactorioEntity> {
        let at = enclosure::cell_to_position(origin, cell);
        state
            .entity_at(&at)
            .filter(|entity| entity.name == BELT && Pos::from(&entity.position) == Pos::from(&at))
    };
    let mut chains = Vec::new();
    for ((x, y), (dx, dy)) in perimeter(from_footprint) {
        let (Some(anchor), Some(arm), Some(belt)) = (
            in_grid(x - dx, y - dy),
            in_grid(x, y),
            in_grid(x + dx, y + dy),
        ) else {
            continue;
        };
        let arm_pos = enclosure::cell_to_position(origin, arm);
        let Some(standing) = state
            .entity_at(&arm_pos)
            .filter(|entity| Pos::from(&entity.position) == Pos::from(&arm_pos))
        else {
            continue;
        };
        if standing.entity_type != "inserter" && !standing.name.ends_with("inserter") {
            continue;
        }
        let anchor_pos = enclosure::cell_to_position(origin, anchor);
        let belt_pos = enclosure::cell_to_position(origin, belt);
        let picks_up_from_machine = inserter_facing(&anchor_pos, &belt_pos)
            .and_then(|facing| facing.to_u8())
            .is_some_and(|facing| facing == standing.direction);
        if !picks_up_from_machine {
            continue;
        }
        let Some(head) = belt_at(belt) else {
            continue;
        };
        let mut chain = Vec::new();
        let mut visited = vec![false; GRID * GRID];
        let mut at = belt;
        let mut tile = head;
        loop {
            visited[enclosure::cell_index(at.0, at.1)] = true;
            let Some(facing) = Direction::from_u8(tile.direction) else {
                break;
            };
            chain.push(ChainTile { cell: at, facing });
            let Some(step) = Position::new(0., -1.).turn(facing) else {
                break;
            };
            let Some(next) = in_grid(
                at.0 as i64 + step.x().round() as i64,
                at.1 as i64 + step.y().round() as i64,
            ) else {
                break;
            };
            if visited[enclosure::cell_index(next.0, next.1)] {
                break;
            }
            let Some(next_tile) = belt_at(next) else {
                break;
            };
            at = next;
            tile = next_tile;
        }
        if !chain.is_empty() {
            chains.push(chain);
        }
    }
    chains
}

/// Where a splitter can be spliced into a standing chain, and where its new
/// branch leaves.
#[derive(Debug, Clone, Copy)]
struct Splice {
    /// The chain tile the splitter replaces: a belt with a same-facing belt
    /// before it and a same-facing belt after it.
    belt: (usize, usize),
    /// The direction of travel there, which the splitter faces.
    facing: Direction,
    /// The free tile beside `belt` the splitter's other half stands on.
    side: (usize, usize),
    /// The tile behind `side`: the splitter's second INPUT. Kept off the
    /// branch, so the branch can never curl round and feed itself back in.
    side_in: Option<(usize, usize)>,
    /// The tile in front of `side`: the splitter's second OUTPUT, where the
    /// branch starts, facing `facing`.
    side_out: (usize, usize),
}

impl Splice {
    /// The splitter's `position`: the midpoint of its two tiles.
    fn splitter_position(&self, origin: (f64, f64)) -> Position {
        let a = enclosure::cell_to_position(origin, self.belt);
        let b = enclosure::cell_to_position(origin, self.side);
        Position::new((a.x() + b.x()) / 2., (a.y() + b.y()) / 2.)
    }
}

/// The offset of one cell in `facing`, in grid steps.
fn step_of(facing: Direction) -> Option<(i64, i64)> {
    let step = Position::new(0., -1.).turn(facing)?;
    Some((step.x().round() as i64, step.y().round() as i64))
}

/// Every place on `chains` a splitter fits, nearest branch start to `sink`
/// first, then chain order, then the left side before the right.
///
/// # What "fits" means, and each rule's reason
///
/// - **Three same-facing belts in a row**, the middle one replaced. The
///   splitter's back edge must be fed straight (the tile before it), its
///   front edge must feed a belt facing away (the tile after it -- a belt
///   turning on the output tile would be side-loaded, one lane), and a
///   chain's first tile is its door's belt cell, which the arm drops on and
///   which no splitter may replace.
/// - **The side tile is free on the caller's grid** -- the placement grid
///   with every obstacle, reservation and tunnel already on it -- and so is
///   the tile in front of it, where the branch starts.
/// - **Nothing feeds the second input.** A belt-connectable entity behind
///   the side tile facing the splitter's way would merge onto the run; that
///   candidate is skipped rather than merged silently.
fn tap_candidates(
    state: &PlanState,
    origin: (f64, f64),
    blocked: &[bool],
    chains: &[Vec<ChainTile>],
    sink: (usize, usize),
) -> Vec<Splice> {
    use factorio_bot_core::num_traits::FromPrimitive;
    let feeds_into = |cell: (usize, usize), facing: Direction| -> bool {
        let at = enclosure::cell_to_position(origin, cell);
        state
            .entity_at(&at)
            .filter(|entity| Pos::from(&entity.position) == Pos::from(&at))
            .is_some_and(|entity| {
                matches!(
                    entity.entity_type.as_str(),
                    "transport-belt" | "underground-belt" | "splitter"
                ) && Direction::from_u8(entity.direction) == Some(facing)
            })
    };
    let mut out: Vec<(u32, usize, usize, Splice)> = Vec::new();
    for chain in chains {
        for index in 1..chain.len().saturating_sub(1) {
            let (before, here, after) = (chain[index - 1], chain[index], chain[index + 1]);
            let facing = here.facing;
            if before.facing != facing || after.facing != facing {
                continue;
            }
            let Some((fx, fy)) = step_of(facing) else {
                continue;
            };
            for (side_index, lateral) in [Position::new(-1., 0.), Position::new(1., 0.)]
                .into_iter()
                .enumerate()
            {
                let Some(lateral) = lateral.turn(facing) else {
                    continue;
                };
                let (sx, sy) = (lateral.x().round() as i64, lateral.y().round() as i64);
                let (bx, by) = (here.cell.0 as i64, here.cell.1 as i64);
                let (Some(side), Some(side_out)) = (
                    in_grid(bx + sx, by + sy),
                    in_grid(bx + sx + fx, by + sy + fy),
                ) else {
                    continue;
                };
                if blocked[enclosure::cell_index(side.0, side.1)]
                    || blocked[enclosure::cell_index(side_out.0, side_out.1)]
                {
                    continue;
                }
                let side_in = in_grid(bx + sx - fx, by + sy - fy);
                if side_in.is_some_and(|cell| feeds_into(cell, facing)) {
                    continue;
                }
                let distance = side_out.0.abs_diff(sink.0) + side_out.1.abs_diff(sink.1);
                out.push((
                    u32::try_from(distance).unwrap_or(u32::MAX),
                    index,
                    side_index,
                    Splice {
                        belt: here.cell,
                        facing,
                        side,
                        side_in,
                        side_out,
                    },
                ));
            }
        }
    }
    out.sort_by_key(|(distance, index, side, _)| (*distance, *index, *side));
    out.into_iter().map(|(_, _, _, splice)| splice).collect()
}

/// What [`connect_steps_reserving`] settled about the window before it chose
/// an end: the grid, the tunnels, and the two footprints. Bundled so the tap
/// takes the same view of the ground the plain run took, byte for byte.
struct Window<'a> {
    origin: (f64, f64),
    blocked: &'a [bool],
    tunnels: &'a [u8],
    reach: Option<u8>,
    sides: &'a [((usize, usize), Position)],
    threatened: Option<&'a [bool]>,
    from_footprint: Footprint,
    to_footprint: Footprint,
}

/// A belt tile of a route as the entity to place: a belt, or the half of
/// an underground pair `route_belt` says it is.
fn route_tile_entity(tile: &RouteTile) -> FactorioEntity {
    match underground_half_for_tile_kind(tile.kind) {
        None => FactorioEntity::new_transport_belt(&tile.position, tile.direction),
        // Both halves carry the tunnel's direction; `route_belt` keeps
        // the exit's step straight, so `tile.direction` is that for both.
        Some(half) => FactorioEntity::new_underground_belt(&tile.position, tile.direction, half),
    }
}

/// Tap a belt run that already leaves `from` with a splitter, and carry the
/// branch to `to`. **The fallback for a source with no free side**, and
/// only that -- see the call in [`connect_steps_reserving`].
///
/// # Why this exists
///
/// `run-1788941729-70024` and `run-1788946451-86723` (seed 31337, four
/// headless bots) both ended `stuck` on one shape: a `method::sustain`
/// plate chest whose four sides were all spent -- three by the cell's own
/// coal arms, the fourth by the arm carrying its plates onto the run the
/// plan had already laid -- and a second consumer wanting plates from that
/// same chest. `first_free_perimeter` refused, correctly: the perimeter
/// genuinely has no tile, and two arms cannot share one. The owner's ruling
/// (2026-09-09) was to **splice a splitter into the run the source already
/// has** rather than find a tile that is not there or build a second chest.
///
/// # What it does
///
/// 1. Reads every belt chain leaving `from` through an arm that picks up
///    from it ([`outbound_chains`]).
/// 2. Chooses the destination's end exactly as a fresh run would
///    (`first_free_perimeter` on `to`).
/// 3. Lists every splice that fits ([`tap_candidates`]) and, nearest first,
///    routes the branch from the splitter's second output to that end with
///    [`route_belt_launching`] -- the first branch tile continues the run's
///    direction, by construction -- on the same grids in the same order the
///    plain run uses: threats avoided first, surface before tunnel.
/// 4. Emits: the bill (one `splitter`, the branch's belts and pairs, ONE
///    inserter -- the load arm already stands), the branch, the unload arm,
///    then a `Chop` of the belt tile the splitter replaces and the
///    splitter's `Place`, linked chop-before-place. The branch and its arm
///    go down first so the tap is complete the moment it opens.
///
/// The belt is chopped rather than fast-replaced because the plan speaks in
/// `AreaFree` and `RemoveEntity`: the chop's effect is what makes the
/// splitter's precondition true in the overlay, and the belt comes back to
/// the bot's hands as one `transport-belt`. (The mod's `rcon_place_entity`
/// does pass `fast_replace`, so the game would accept the splitter over the
/// belt; the planner would not have, and a placement the planner cannot
/// state is not one it can order.)
///
/// # What it refuses, by name
///
/// `Err(why)` before anything is placed -- the promise every refusal in this
/// module makes -- when no run leaves the source, when `to` has no free
/// side, when no straight stretch of any run has a free side for the
/// splitter's other half, or when no branch routes from any splice. The
/// caller wraps it as [`ConnectRefusal::TapRefused`] beside the perimeter
/// refusal that sent it here.
///
/// # Scope, stated
///
/// Phase 1. The branch is emitted flat under the chain actor -- no bands --
/// and a splice needs three same-facing belts in a row; a run that turns
/// every other tile has no splice and refuses. Undergrounds in the standing
/// run end a chain (they end `standing_run`'s too). The splitter's
/// input/output priority and filter are left at the prototype's defaults,
/// which divide by availability -- the property the fallback rests on.
fn tap_standing_run(
    ctx: &mut ExpansionCtx,
    from: &FactorioEntity,
    to: &FactorioEntity,
    item: &ItemId,
    inserter: &str,
    window: &Window<'_>,
) -> Result<Vec<Step>, String> {
    let origin = window.origin;
    let chains = outbound_chains(&ctx.state, origin, &window.from_footprint);
    if chains.is_empty() {
        return Err(format!(
            "no belt run leaves the {} through an arm that picks up from it",
            from.name
        ));
    }
    let followed: usize = chains.iter().map(Vec::len).sum();
    let mut blocked = window.blocked.to_vec();
    let sink =
        first_free_perimeter(&blocked, origin, &window.to_footprint, &[]).map_err(|stopped| {
            let named: Vec<String> = stopped.iter().map(ToString::to_string).collect();
            format!(
                "the {} has no free side for the branch's arm, blocked by {}",
                to.name,
                named.join(" ")
            )
        })?;
    blocked[enclosure::cell_index(sink.inserter.0, sink.inserter.1)] = true;
    let candidates = tap_candidates(&ctx.state, origin, &blocked, &chains, sink.belt);
    if candidates.is_empty() {
        return Err(format!(
            "no straight stretch of the {} belt tile(s) leaving the {} has a free side \
             for a splitter's other half",
            followed, from.name
        ));
    }
    // SURFACE FROM ANY SPLICE BEFORE A TUNNEL FROM THE NEAREST. The plain
    // run's cascade -- surface first, then a pair -- is per endpoint; here
    // the endpoint is chosen from a list, and a jump out of the nearest
    // splice must not beat a plain belt out of the next one. Measured on
    // this module's own fixture: the nearest splice's launch tile faced
    // the destination's standing arm, and the branch went UNDER it rather
    // than leaving from the splice one tile back.
    let mut last: Option<String> = None;
    let passes: Vec<Option<u8>> = match window.reach {
        Some(reach) => vec![None, Some(reach)],
        None => vec![None],
    };
    for pass in passes {
        for splice in &candidates {
            let mut grid = blocked.clone();
            grid[enclosure::cell_index(splice.side.0, splice.side.1)] = true;
            if let Some(side_in) = splice.side_in {
                grid[enclosure::cell_index(side_in.0, side_in.1)] = true;
            }
            // A chest at either end closes its other sides to the branch,
            // as to any route of this module -- see the plain attempt.
            let chosen = [
                splice.belt,
                splice.side,
                splice.side_out,
                sink.inserter,
                sink.belt,
            ];
            for (cell, owner) in window.sides {
                let ours = same_tile(owner, &from.position) || same_tile(owner, &to.position);
                if ours && !chosen.contains(cell) {
                    grid[enclosure::cell_index(cell.0, cell.1)] = true;
                }
            }
            let avoiding = window.threatened.map(|threatened| {
                let mut avoiding = grid.clone();
                for (cell, is_threatened) in threatened.iter().enumerate() {
                    if *is_threatened {
                        avoiding[cell] = true;
                    }
                }
                avoiding[enclosure::cell_index(splice.side_out.0, splice.side_out.1)] = false;
                avoiding[enclosure::cell_index(sink.belt.0, sink.belt.1)] = false;
                avoiding
            });
            let search = |grid: &[bool]| {
                route_belt_launching(
                    grid,
                    window.tunnels,
                    origin,
                    splice.side_out,
                    sink.belt,
                    pass,
                    splice.facing,
                )
            };
            let found = avoiding
                .as_ref()
                .and_then(|grid| search(grid).ok())
                .map_or_else(|| search(&grid), Ok);
            match found {
                Ok(route) => {
                    return tap_steps(ctx, from, to, item, inserter, origin, splice, &sink, &route)
                        .map_err(|refusal| refusal.to_string());
                }
                Err(error) => {
                    let why = match error {
                        RouteError::NoPath { blocked } => ConnectRefusal::NoRoute { blocked },
                        RouteError::SpanTooLong { needed, max } => {
                            ConnectRefusal::SpanTooLong { needed, max }
                        }
                        RouteError::Cancelled => ConnectRefusal::NoRoute { blocked: vec![] },
                    };
                    last = Some(format!(
                        "from the splice at {}: {why}",
                        enclosure::cell_to_position(origin, splice.belt)
                    ));
                }
            }
        }
    }
    Err(format!(
        "a splitter fits at {} place(s) on the run leaving the {} and no branch routes from \
         any of them to the {}; the last: {}",
        candidates.len(),
        from.name,
        to.name,
        last.unwrap_or_default()
    ))
}

/// The steps of a tap once its splice and branch are chosen. See
/// [`tap_standing_run`] for the order and why.
#[allow(clippy::too_many_arguments)]
fn tap_steps(
    ctx: &mut ExpansionCtx,
    from: &FactorioEntity,
    to: &FactorioEntity,
    item: &ItemId,
    inserter: &str,
    origin: (f64, f64),
    splice: &Splice,
    sink: &Endpoint,
    route: &Route,
) -> Result<Vec<Step>, ConnectRefusal> {
    let sink_anchor_pos = enclosure::cell_to_position(origin, sink.anchor);
    let belt_end_pos = enclosure::cell_to_position(origin, sink.belt);
    let dst_inserter_pos = enclosure::cell_to_position(origin, sink.inserter);
    let unload_facing =
        inserter_facing(&belt_end_pos, &sink_anchor_pos).ok_or(ConnectRefusal::NotCardinal)?;
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
    let reach = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.reach_distance)
        .unwrap_or(10.0);

    // Nothing above this line has touched `ctx`. From here every step is
    // emitted and mirrored into the overlay in the same breath.
    let mut steps = Vec::with_capacity(route.tiles.len() + 7);
    let have = |item: &str, count: u32| {
        Step::Subgoal(Goal::Have {
            item: item.into(),
            count,
            whose: Holder::Share(ctx.chain_actor),
            via: None,
        })
    };
    steps.push(have(SPLITTER, 1));
    if belts > 0 {
        steps.push(have(BELT, belts));
    }
    if undergrounds > 0 {
        steps.push(have(UNDERGROUND, undergrounds));
    }
    steps.push(have(inserter, 1));

    let note = format!(
        "carry {item} from {} to {} (branched off its standing run by a splitter)",
        from.name, to.name
    );
    for tile in &route.tiles {
        steps.push(place_step(ctx, route_tile_entity(tile), build, &note));
    }
    let unload =
        FactorioEntity::new_named_inserter(inserter.to_string(), &dst_inserter_pos, unload_facing);
    let note = format!("unload {item} into {} (off the branch)", to.name);
    steps.push(place_step(ctx, unload, build, &note));

    // The belt the splitter replaces comes up first. Its `RemoveEntity` is
    // what makes the splitter's `AreaFree` true, in the overlay now and in
    // the network's ordering; the link below states the order outright
    // rather than leaving it to `infer_edges`, whose position match is on
    // the whole tile and the splitter's position is between two.
    let belt_pos = enclosure::cell_to_position(origin, splice.belt);
    let mut bill = mine_bill(&ctx.state, BELT);
    if bill.is_empty() {
        bill.insert(BELT.to_string(), 1);
    }
    let mut eff = vec![Effect::RemoveEntity {
        pos: belt_pos.clone(),
    }];
    for (yielded, count) in &bill {
        eff.push(Effect::GainItem {
            who: Actor::Role,
            item: yielded.as_str().into(),
            count: *count,
        });
    }
    let chop_id = ctx.ids.next();
    steps.push(Step::Act(Box::new(Action {
        id: chop_id,
        kind: ActionKind::Chop {
            pos: belt_pos.clone(),
            entity: BELT.to_string(),
            item: BELT.into(),
            count: 1,
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: belt_pos.clone(),
                radius: reach,
                min_radius: ctx.state.placement_clearance(BELT).unwrap_or(0.0),
            },
            // THE RUN IS NOT OPENED UNTIL THE SPLITTER IS IN HAND. Nothing
            // is spent here -- the `Place` below does that -- but the
            // condition is a real one: between the chop and the splitter
            // the run is cut and the source's arm feeds a stub. Without
            // this the scheduler was free to chop at tick 4,509 and place
            // at 37,140, on the far side of `research logistics` and the
            // craft (measured on `run-1788946451-86723`'s replan), which
            // starves the run's original destination for the whole gap.
            // Stated as a `HasItem`, the chop is linked to the splitter's
            // producer like any consumer and lands beside the placement.
            Condition::HasItem {
                who: Actor::Role,
                item: SPLITTER.into(),
                count: 1,
            },
        ],
        eff,
        duration: mining_ticks(&ctx.state, BELT),
        pinned: None,
        label: format!(
            "chop {BELT} at {belt_pos} -- open the run from {} for a splitter",
            from.name
        ),
    })));
    ctx.state.remove_entity(&belt_pos);

    let splitter = FactorioEntity::new_splitter(&splice.splitter_position(origin), splice.facing);
    let note = format!(
        "tap the run from {} for {item}, one output on to where it went, one to {}",
        from.name, to.name
    );
    let splitter_pos = splice.splitter_position(origin);
    let belt_pos = enclosure::cell_to_position(origin, splice.belt);
    eprintln!("TRACE_TAP: splitter=[{:.1},{:.1}] belt=[{:.1},{:.1}]",
        splitter_pos.x(), splitter_pos.y(), belt_pos.x(), belt_pos.y());
    ctx.state.reserve_ground(&[splitter_pos, belt_pos], "tap splitter");
    let place = place_step_position_free(ctx, splitter, build, &note);
    if let Step::Act(action) = &place {
        steps.push(Step::Link {
            from: chop_id,
            to: action.id,
            lag: 0,
        });
    }
    steps.push(place);
    Ok(steps)
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
    // Add PLACEMENT_HALF_BOX so entities whose expanded collision box reaches
    // into the window from just outside are still found.
    let plain = (area.width() / 2.).hypot(area.height() / 2.);
    let radius = plain + PLACEMENT_HALF_BOX;
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

/// Would a belt standing on `at` have a surface way out of the window
/// around it -- to the window's own edge, over free ground, on the same
/// placement grid [`connect_steps_reserving`] routes on?
///
/// `false` when `at` is itself occupied, off every grid, or sealed into a
/// pocket by what stands (base world and this plan's overlay alike). A
/// caller keeping a tile free for a run it cannot lay yet asks this, because
/// a free tile inside a ring of belts is kept for nothing: the run would have
/// to tunnel out, and a tunnel is a recipe the force may not have.
pub(crate) fn belt_reaches_open_ground(ctx: &ExpansionCtx, at: &Position) -> bool {
    let (area, origin) = enclosure::window(at);
    let blocked = enclosure::rasterize(
        ctx.state
            .base()
            .entity_graph
            .blocking_boxes_within(&area)
            .into_iter()
            .chain(overlay_boxes(ctx, &area)),
        origin,
        (PLACEMENT_HALF_BOX, PLACEMENT_HALF_BOX),
    );
    let Some(cell) = cell_of(origin, at) else {
        return false;
    };
    let index = enclosure::cell_index(cell.0, cell.1);
    let reach = factorio_bot_core::graph::enclosure::reachable_from_boundary(&blocked);
    !blocked[index] && reach[index]
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

// ---------------------------------------------------------------------------
// Banding a belt run across the roster
// ---------------------------------------------------------------------------

/// Cut a route into at most `bots` **contiguous** bands, in path order.
///
/// **Why this exists.** `run-1788920460-08860` planned 217 placements and bot
/// 1 made 200 of them, 156 of those one `transport-belt` run laid end to end
/// while three bots stood idle: fleet utilisation 25.1%, `steps/bot {1: 520,
/// 2: 136, 3: 119, 4: 106}`. Gathering divided four ways (mine 23/50/50/42);
/// construction did not divide at all (place 200/9/5/3). The capability to
/// split a build existed twice -- `method::blueprint::bands` and
/// `method::assemble`'s `deal_bundles` -- and this run used neither. See
/// `docs/superpowers/notes/2026-09-09-a-belt-run-is-one-bots-job.md`.
///
/// **A route is a line, and a band of it is a contiguous slice.** This is
/// `method::blueprint::bands`' shape -- a band is a *region*, so a bot never
/// crosses another's band -- with the axis question already answered: a
/// route's tiles are ordered by the path, and the path is the only axis a
/// belt run has. Banding by index stride instead (tile 0 to bot 1, tile 1 to
/// bot 2, ...) would interleave four bots along one corridor, which is
/// exactly the defect `bands` recorded on `MinerLine` and fixed on
/// 2026-09-05. Every tile lands in exactly one band and the bands abut, so
/// no boundary tile has two owners -- the belt-run cousin of the
/// self-crossing repair in `route_belt_with_tunnels` (`71f9227c`), which is
/// finished before this function ever sees the tiles.
///
/// **A band must be worth its walk.** Priced in the currency `deal_bundles`
/// prices a bundle in: a placement is [`PLACE_TICKS`] and the trip to it is
/// [`HANDOVER_WALK_TICKS`], so a stretch shorter than
/// `HANDOVER_WALK_TICKS / PLACE_TICKS` belts (ten, today) costs more to walk
/// to than to lay, and is not cut off. A nine-belt run is one band on any
/// roster; a 156-belt run is four bands on four bots. Without this rule a
/// six-belt run over four bots would send three bots walking to lay one or
/// two belts each, which is the contention this project has measured (eight
/// bots: a shorter plan and a longer run) with nothing bought for it.
///
/// **The remainder is spread, not dumped**, as in `bands`: 42 tiles over 4
/// bots is 11/11/10/10, the first `n % count` bands taking one extra.
///
/// **A cut never separates an underground pair.** `route_belt` emits the
/// entry and its exit as adjacent tiles (`tiles[i - 1]` / `tiles[i]`); a
/// boundary that would fall between them moves one tile on, so one bot lays
/// both halves and the tunnel is either standing or absent, never half
/// there with two bots each waiting on the other's inventory. The shift is
/// absorbed by the last band.
///
/// Deterministic: a function of the kinds and the count alone.
pub fn route_bands(kinds: &[TileKind], bots: usize) -> Vec<std::ops::Range<usize>> {
    let n = kinds.len();
    if n == 0 || bots == 0 {
        return Vec::new();
    }
    let worth_walking = usize::try_from(
        Ticks::try_from(n)
            .unwrap_or(Ticks::MAX)
            .saturating_mul(PLACE_TICKS)
            / HANDOVER_WALK_TICKS,
    )
    .unwrap_or(usize::MAX);
    let count = bots.min(worth_walking).max(1);
    let base = n / count;
    let remainder = n % count;
    let mut out = Vec::with_capacity(count);
    let mut start = 0;
    for band in 0..count {
        let size = base + usize::from(band < remainder);
        let mut end = (start + size).min(n);
        if end > 0 && end < n && kinds[end - 1] == TileKind::UndergroundEntry {
            end += 1;
        }
        if band + 1 == count {
            end = n;
        }
        if end > start {
            out.push(start..end);
        }
        start = end;
    }
    debug_assert_eq!(
        out.iter().map(|b| b.len()).sum::<usize>(),
        n,
        "every tile lands in exactly one band"
    );
    out
}

/// The bot that lays each band, one per entry of `bands`.
///
/// `deal_bundles`' rule, applied to bands instead of bundles: **heaviest
/// band first, each to whoever is lightest at that moment**, where a bot's
/// load is [`PlanState::planned_ticks`] -- what this expansion has already
/// committed it to -- plus every band it is dealt here. A band's price is
/// its belts from raw (`produce::craft_ticks`, the same from-raw pricing
/// `deal_bundles` measured its way to), its placements, and one
/// [`HANDOVER_WALK_TICKS`] for the trip. The tie-break is the lower
/// `BotId`, so the deal is a function of its inputs alone.
///
/// Not round-robin: a bot already carrying a cell's charge or a long mining
/// claim is lighter on paper only if nothing is counted, and
/// `planned_ticks` counts it. The taker (`ctx.chain_actor`) is a candidate
/// like anyone else, exactly as in `deal_bundles`.
fn deal_route_bands(
    state: &PlanState,
    kinds: &[TileKind],
    bands: &[std::ops::Range<usize>],
    builders: &[BotId],
) -> Vec<BotId> {
    let mut loads: BTreeMap<BotId, Ticks> = builders
        .iter()
        .map(|bot| (*bot, state.planned_ticks(*bot)))
        .collect();
    let price = |band: &std::ops::Range<usize>| -> Ticks {
        let tiles = &kinds[band.clone()];
        let undergrounds = u32::try_from(tiles.iter().filter(|k| **k != TileKind::Belt).count())
            .unwrap_or(u32::MAX);
        let belts = u32::try_from(tiles.len())
            .unwrap_or(u32::MAX)
            .saturating_sub(undergrounds);
        craft_ticks(state, BELT, belts, CRAFT_TICKS_MAX_DEPTH)
            .saturating_add(craft_ticks(
                state,
                UNDERGROUND,
                undergrounds,
                CRAFT_TICKS_MAX_DEPTH,
            ))
            .saturating_add(
                PLACE_TICKS.saturating_mul(Ticks::try_from(tiles.len()).unwrap_or(Ticks::MAX)),
            )
            .saturating_add(HANDOVER_WALK_TICKS)
    };
    let mut order: Vec<(Ticks, usize)> = bands.iter().map(price).zip(0..).collect();
    // Heaviest first; equal prices in band order, so the deal is stable.
    order.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut owners = vec![BotId(0); bands.len()];
    for (price, band) in order {
        let bot = loads
            .iter()
            .map(|(bot, load)| (*load, *bot))
            .min()
            .map(|(_, bot)| bot)
            .expect("builders is non-empty");
        let load = loads.entry(bot).or_default();
        *load = load.saturating_add(price);
        owners[band] = bot;
    }
    owners
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

/// One `Place` action with `PositionFree` instead of `AreaFree`.
///
/// Used by [`tap_steps`] for the splitter that replaces a belt on the same
/// tile: the chop action's `RemoveEntity` clears the exact position, so
/// a full `AreaFree` (which checks collision-box overlap with neighbours)
/// would incorrectly fail when an adjacent entity's box extends into this
/// tile. `PositionFree` checks only the 1Ã1 tile, which is sufficient here
/// because the tap always follows the chop on the same belt position.
fn place_step_position_free(ctx: &mut ExpansionCtx, entity: FactorioEntity, build: f64, note: &str) -> Step {
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
            Condition::PositionFree {
                pos: entity.position.clone(),
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
    connect_steps_reserving(ctx, from, to, item, inserter, &[])
}

/// [`connect_steps_with`], with tiles the caller has spoken for closed to
/// this route.
///
/// `reserved` names tile centres this run may not stand on -- not for an
/// inserter, not for a belt, not for a tunnel's mouth. They are marked on
/// the obstacle grid before either end is chosen, so a perimeter pair is
/// never picked on one of them and the route search never crosses one; a
/// run that cannot be made without them refuses exactly as it would for an
/// occupied tile, with the reserved tiles among the ones named, and places
/// nothing. A reserved tile outside this run's window costs nothing.
///
/// # Why the reservation is the CALLER's, and not a rule of this module
///
/// The four coal runs of a `method::sustain` cell leave from a 1x1 chest
/// with four sides, and between them they spend every side that chest has;
/// the run that would have failed for want of a fourth side is refused
/// before it is laid. That is the perimeter budget this module already
/// states. **A bystander chest -- one that is neither end of the run -- has
/// its sides spent the same way, by routes that merely pass it**, and this
/// module cannot know whether that matters: a coal chest whose every side a
/// later run of the same expansion is going to claim must stay open to
/// them, while a *plate* chest whose one exit a later *method* is going to
/// need must keep it. The two are the same prototype on the same grid.
///
/// A rule in here that kept one side of every bystander chest was tried
/// twice and reverted twice: it closed to the third coal run the side the
/// third coal run needed, and five `method::sustain` tests went red both
/// times. The information that separates the two chests is which runs are
/// still to come, and only the caller that is going to lay them has it. So
/// the budget is declared by the method that knows the future and enforced
/// here, rather than guessed here and argued with there.
///
/// `method::sustain` uses this for the product chest of every cell -- see
/// its `Offtake::exit`.
pub fn connect_steps_reserving(
    ctx: &mut ExpansionCtx,
    from: &FactorioEntity,
    to: &FactorioEntity,
    item: &ItemId,
    inserter: &str,
    reserved: &[Position],
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

    // THE CALLER'S RESERVATIONS, before either end is chosen: a reserved
    // tile is an obstacle to this route and nothing else, so it goes on the
    // grid with the rest of the obstacles and every search below sees it.
    for at in reserved {
        if let Some(cell) = cell_of(origin, at) {
            blocked[enclosure::cell_index(cell.0, cell.1)] = true;
        }
    }
    // AND THE PLAN'S, from `PlanState::reserve_ground` -- with one
    // exemption: a reserved tile on the perimeter of this run's own `from`
    // or `to` is this run's to use. The ground is kept FOR the run that
    // leaves the chest it borders, and that run must find its end there;
    // to every other route it is a wall like the caller's own. And the
    // `from` end does not merely MAY use it: the kept pair on its perimeter
    // is the pair it takes first (`first_free_perimeter`'s `kept`), because
    // a reservation is a product exit -- the arm's tile and the belt's tile
    // out of the chest -- and the run OUT of the chest is what it was kept
    // for. The `to` end keeps the plain order: a run INTO a chest is not the
    // run its exit was kept for, and the reservation carries no direction
    // of its own (`keeper` is prose), so the one purpose it has today is
    // read off which end the chest is.
    let ground_reserved: Vec<(usize, usize)> = ctx
        .state
        .reserved_ground()
        .iter()
        .filter_map(|(kept, _)| cell_of(origin, &kept.center()))
        .collect();

    let from_footprint = footprint_of(origin, from).ok_or_else(|| ConnectRefusal::NoRoute {
        blocked: vec![from.position.clone()],
    })?;
    let to_footprint = footprint_of(origin, to).ok_or_else(|| ConnectRefusal::NoRoute {
        blocked: vec![to.position.clone()],
    })?;

    // A RUN THAT ALREADY STANDS, MISSING ONLY ITS ARMS, IS FINISHED, NOT
    // ROUTED AROUND. See `standing_run`: a belt chain leaving `from`'s door
    // and ending at `to`'s door is this run, whoever laid it, and the
    // replan places the two arms and nothing else.
    if let Some((source, sink)) =
        standing_run(&ctx.state, inserter, origin, &from_footprint, &to_footprint)
    {
        return arms_only(ctx, from, to, item, inserter, origin, source, sink);
    }

    claim_footprint(&mut blocked, &from_footprint);
    claim_footprint(&mut blocked, &to_footprint);
    let own_perimeter = |cell: (usize, usize)| {
        [&from_footprint, &to_footprint]
            .into_iter()
            .any(|footprint| {
                perimeter(footprint).into_iter().any(|((x, y), (dx, dy))| {
                    in_grid(x, y) == Some(cell) || in_grid(x + dx, y + dy) == Some(cell)
                })
            })
    };
    let from_perimeter = |cell: (usize, usize)| {
        perimeter(&from_footprint)
            .into_iter()
            .any(|((x, y), (dx, dy))| {
                in_grid(x, y) == Some(cell) || in_grid(x + dx, y + dy) == Some(cell)
            })
    };
    let mut kept_exit: Vec<(usize, usize)> = Vec::new();
    for cell in ground_reserved {
        if from_perimeter(cell) {
            kept_exit.push(cell);
        } else {
            blocked[enclosure::cell_index(cell.0, cell.1)] = true;
        }
    }

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

    // What stands in the window is settled; choosing the two ends and
    // searching between them is one attempt over a COPY of the grid, so it
    // can be made twice -- see `attempt`'s doc and the call below it.
    let sides = container_sides(ctx, &area, origin);
    let threatened = threatened_cells(ctx, origin);

    // One attempt at the run: both ends chosen, then the search.
    //
    // Each end is claimed onto the grid as soon as it is chosen, so the
    // second search sees what the first took. Without this, both ends are
    // "the first free perimeter pair of X" against the same static grid and
    // neither knows what the other claimed -- in tight geometry the two could
    // pick the same tile, and this function would go on to emit two `Place`
    // actions for it. Claiming turns that collision into a refusal instead:
    // the tile is no longer free for whichever search asks next, so it keeps
    // looking, and if nothing is left it refuses via `first_free_perimeter`'s
    // `Err` exactly as an ordinary blocked tile would.
    //
    // `kept` is handed to the `from` end only -- see the reservation note
    // above -- and an attempt with it empty is the search this function has
    // always run.
    let attempt = |kept: &[(usize, usize)]| -> Result<(Endpoint, Endpoint, Route), ConnectRefusal> {
        let mut blocked = blocked.clone();
        let source = first_free_perimeter(&blocked, origin, &from_footprint, kept)
            .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
        blocked[enclosure::cell_index(source.inserter.0, source.inserter.1)] = true;
        blocked[enclosure::cell_index(source.belt.0, source.belt.1)] = true;

        let sink = first_free_perimeter(&blocked, origin, &to_footprint, &[])
            .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
        blocked[enclosure::cell_index(sink.inserter.0, sink.inserter.1)] = true;

        // A CHEST'S OTHER SIDES ARE RESERVED, not routed over. Measured
        // 2026-09-09 on seed 31337: the haul into a cell's coal chest ended
        // on its north side and ran its last belts down the chest's EAST
        // side on the way in, so the chest's fourth side -- the one the next
        // cell needed -- was spent by a belt that had no business there, and
        // the next run refused with all four neighbours named.
        // `method::sustain`'s doc had already said where the fix belonged:
        // "a real fix reserves the perimeter in `method::connect`". So once
        // the two ends are chosen, a chest at either end closes every other
        // side to this route.
        //
        // **Only this call's own two machines, and only if they are chests.**
        // The first version closed every chest in the window, and the
        // one-cell sustain arrangement in the fixtures -- three chests within
        // a few tiles -- lost every surface route and reached for a tunnel it
        // cannot craft. A bystander's sides are the bystander's own call's
        // business.
        for (cell, owner) in &sides {
            let ours = same_tile(owner, &from.position) || same_tile(owner, &to.position);
            let chosen = [source.inserter, source.belt, sink.inserter, sink.belt].contains(cell);
            if ours && !chosen {
                blocked[enclosure::cell_index(cell.0, cell.1)] = true;
            }
        }

        // THREATS: a belt is a standing structure, so the route prefers to
        // keep out of a charted enemy structure's reach -- but never at the
        // price of the route itself. `threatened_cells` is OR-ed onto a
        // *copy* of the grid and tried first; a refusal there falls through
        // to the plain grid, which is the search this function has always
        // run. So the guard can move a belt and can never delete one, the
        // same prefer-then-fall-back shape `method::util::free_area_near_where`
        // uses for siting, and for the same measured reason (a standing
        // thing cannot walk out of range).
        //
        // The endpoints are deliberately NOT part of it: they are fixed by
        // the machines, and blocking them would refuse every route on the
        // first attempt and make the whole pass a wasted search.
        //
        // ORDER: surface first, on both grids, and only then a tunnel. A
        // pair is a last resort -- it costs the iron of some sixteen belts,
        // needs a recipe that is disabled at t=0, and reserves the ground
        // beneath it -- so a route the surface can make, however long its
        // detour, is the route. This is what keeps every plan that routed
        // before undergrounds existed byte-for-byte the same plan.
        let avoiding = threatened.as_ref().map(|threatened| {
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
            .or_else(|| {
                reach.and_then(|_| avoiding.as_ref().and_then(|grid| search(grid, reach).ok()))
            })
            .map_or_else(|| search(&blocked, reach), Ok)
            .map_err(|error| match error {
                RouteError::NoPath { blocked } => ConnectRefusal::NoRoute { blocked },
                RouteError::SpanTooLong { needed, max } => {
                    ConnectRefusal::SpanTooLong { needed, max }
                }
                RouteError::Cancelled => ConnectRefusal::NoRoute { blocked: vec![] },
            })?;
        Ok((source, sink, route))
    };

    // THE KEPT EXIT FIRST, THE PLAIN ORDER IF IT REFUSES. A side kept for
    // this run is preferred, never imposed: the plan reserved it while the
    // cell was sited, and what has stood up since -- another method's
    // machine, a coal run, a tunnel -- can have shut the ground beyond it
    // without touching the pair itself. A refusal with the kept pair is then
    // a fact about that pair and not about the chest, so the run is tried
    // once more the way it always was, and only both refusing is a refusal.
    // The second attempt's refusal is the one reported: it names the tiles
    // the old search would have named. With nothing kept there is exactly
    // one attempt, as before.
    let (source, sink, route) = match attempt(&kept_exit) {
        Ok(found) => found,
        Err(refusal) => {
            let refusal = if kept_exit.is_empty() {
                Err(refusal)
            } else {
                attempt(&[])
            };
            match refusal {
                Ok(found) => found,
                // A SOURCE WITH NO SIDE LEFT IS TAPPED, NOT REFUSED -- when
                // a run already leaves it. The perimeter refusal is a fact
                // (two arms cannot share a tile), and the plain search has
                // just proved it twice; what a boxed-in source still has is
                // the belt its one arm feeds, and a splitter spliced into
                // that carries the same items on to a second destination.
                // Only the `from` end's perimeter sends a run here: a sink
                // with no side, or a route that found no path, is refused
                // as it always was. The tap's own refusal rides beside the
                // perimeter's, so the reader sees both.
                Err(refusal) => {
                    let from_has_no_side =
                        first_free_perimeter(&blocked, origin, &from_footprint, &[]).is_err();
                    let ConnectRefusal::NoRoute { blocked: stopped } = &refusal else {
                        return Err(refusal);
                    };
                    if !from_has_no_side {
                        return Err(refusal);
                    }
                    let window = Window {
                        origin,
                        blocked: &blocked,
                        tunnels: &tunnels,
                        reach,
                        sides: &sides,
                        threatened: threatened.as_deref(),
                        from_footprint,
                        to_footprint,
                    };
                    return match tap_standing_run(ctx, from, to, item, inserter, &window) {
                        Ok(steps) => Ok(steps),
                        Err(why) => Err(ConnectRefusal::TapRefused {
                            blocked: stopped.clone(),
                            why,
                        }),
                    };
                }
            }
        }
    };

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

    // THE RUN IS CUT INTO BANDS, one contiguous stretch per bot that takes
    // one, and each band is bound to its bot with `Step::Owned` exactly as
    // `method::blueprint` binds a block's bands. A run of one band -- a
    // one-bot roster, or a stretch too short to be worth a second walk --
    // is emitted flat under the chain actor, byte for byte the plan this
    // function made before bands existed. See `route_bands`.
    let kinds: Vec<TileKind> = route.tiles.iter().map(|t| t.kind).collect();
    let builders = participants_that_can_work(&ctx.state, ctx.state.bot_ids());
    let bands = route_bands(&kinds, builders.len().max(1));
    let owners: Vec<BotId> = if bands.len() > 1 {
        deal_route_bands(&ctx.state, &kinds, &bands, &builders)
    } else {
        vec![ctx.chain_actor; bands.len()]
    };

    let mut steps = Vec::with_capacity(route.tiles.len() + 5);
    if bands.len() <= 1 {
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

    let tile_entity = route_tile_entity;
    if bands.len() <= 1 {
        let note = format!("carry {item} from {} to {}", from.name, to.name);
        for tile in &route.tiles {
            steps.push(place_step(ctx, tile_entity(tile), build, &note));
        }
    } else {
        for (band, (range, bot)) in bands.iter().zip(&owners).enumerate() {
            let tiles = &route.tiles[range.clone()];
            let band_undergrounds =
                u32::try_from(tiles.iter().filter(|t| t.kind != TileKind::Belt).count())
                    .unwrap_or(u32::MAX);
            let band_belts = u32::try_from(tiles.len())
                .unwrap_or(u32::MAX)
                .saturating_sub(band_undergrounds);
            let build = ctx
                .state
                .bot(*bot)
                .map(|b| b.build_distance)
                .unwrap_or(10.0);
            // The band's own bill, stated for its own bot, so the shortfall
            // machinery sends THAT bot for its belts rather than the chain
            // actor for everyone's -- the same shape `blueprint.rs` gives a
            // block band.
            let mut block = Vec::with_capacity(tiles.len() + 2);
            if band_belts > 0 {
                block.push(Step::Subgoal(Goal::Have {
                    item: BELT.into(),
                    count: band_belts,
                    whose: Holder::Share(*bot),
                    via: None,
                }));
            }
            if band_undergrounds > 0 {
                block.push(Step::Subgoal(Goal::Have {
                    item: UNDERGROUND.into(),
                    count: band_undergrounds,
                    whose: Holder::Share(*bot),
                    via: None,
                }));
            }
            let note = format!(
                "carry {item} from {} to {} (belt band {band} of {})",
                from.name,
                to.name,
                bands.len()
            );
            for tile in tiles {
                block.push(place_step(ctx, tile_entity(tile), build, &note));
            }
            steps.push(Step::Owned {
                whose: Holder::Share(*bot),
                steps: block,
            });
        }
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

    /// Walks into `Step::Owned`, so a run cut into bands reads the same as
    /// one laid flat: the bands abut, so this is the route in path order.
    fn placements(steps: &[Step], name: &str) -> Vec<(Position, u8)> {
        let mut out = Vec::new();
        for step in steps {
            match step {
                Step::Act(action) => {
                    if let ActionKind::Place { entity } = &action.kind
                        && entity.name == name
                    {
                        out.push((entity.position.clone(), entity.direction));
                    }
                }
                Step::Owned { steps, .. } => out.extend(placements(steps, name)),
                _ => {}
            }
        }
        out
    }

    /// Each band of a run: its owner and its belts, in emission order.
    fn bands_of(steps: &[Step]) -> Vec<(BotId, Vec<(Position, u8)>)> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Owned {
                    whose: Holder::Share(bot) | Holder::Bot(bot),
                    steps,
                } => Some((*bot, placements(steps, BELT))),
                _ => None,
            })
            .collect()
    }

    fn belts(n: usize) -> Vec<TileKind> {
        vec![TileKind::Belt; n]
    }

    /// 42 tiles over four bots is 11/11/10/10 -- contiguous, abutting,
    /// remainder spread. A stride split (tile i to bot i % 4) fails here
    /// on contiguity, and `div_ceil` chunking (11/11/11/9) on the sizes.
    #[test]
    fn route_bands_are_contiguous_and_spread_the_remainder() {
        assert_eq!(
            route_bands(&belts(42), 4),
            vec![0..11, 11..22, 22..32, 32..42]
        );
        assert_eq!(route_bands(&belts(42), 4), route_bands(&belts(42), 4));
    }

    /// A band shorter than `HANDOVER_WALK_TICKS / PLACE_TICKS` belts costs
    /// more to walk to than to lay, so a short run is one band on any
    /// roster and a medium one fewer bands than bots. The 156-belt run that
    /// motivated this is four bands on four bots.
    #[test]
    fn a_band_must_be_worth_its_walk() {
        let min = usize::try_from(HANDOVER_WALK_TICKS / PLACE_TICKS).unwrap();
        assert_eq!(route_bands(&belts(min - 1), 4), vec![0..min - 1]);
        assert_eq!(route_bands(&belts(2 * min), 4), vec![0..min, min..2 * min]);
        assert_eq!(route_bands(&belts(156), 4).len(), 4);
        assert_eq!(route_bands(&belts(156), 1).len(), 1);
        assert!(route_bands(&belts(0), 4).is_empty());
    }

    /// `route_belt` emits an entry and its exit as adjacent tiles. A cut
    /// that would fall between them moves on by one, so one bot lays both
    /// halves; the last band absorbs the shift.
    #[test]
    fn a_cut_never_separates_an_underground_pair() {
        let mut kinds = belts(40);
        kinds[9] = TileKind::UndergroundEntry;
        kinds[10] = TileKind::UndergroundExit;
        let bands = route_bands(&kinds, 4);
        assert_eq!(
            bands[0],
            0..11,
            "the first cut would have split the pair at 10"
        );
        assert_eq!(bands.iter().map(|b| b.len()).sum::<usize>(), 40);
        for pair in bands.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "bands abut: {bands:?}");
        }
        for band in &bands {
            assert!(
                band.end == 40 || kinds[band.end - 1] != TileKind::UndergroundEntry,
                "band {band:?} ends on an entry whose exit belongs to the next bot"
            );
        }
    }

    /// `deal_bundles`' rule: heaviest first, each to whoever is lightest at
    /// that moment. Three bands of 4/3/3 over two idle bots go 1, 2, 2 --
    /// the third to bot 2, who is carrying three belts against bot 1's
    /// four. Round-robin would hand it to bot 1.
    #[test]
    fn bands_are_dealt_heaviest_first_to_the_lightest_bot() {
        let ctx = crate::test_world::connect_ctx_with_roster(vec![], &[BotId(1), BotId(2)]);
        let kinds = belts(10);
        let bands = vec![0..4, 4..7, 7..10];
        assert_eq!(
            deal_route_bands(&ctx.state, &kinds, &bands, &[BotId(1), BotId(2)]),
            vec![BotId(1), BotId(2), BotId(2)]
        );
    }

    /// The fixture's 2x2 furnace and a 3x3 lab twenty tiles apart: a run
    /// long enough for two bands on four bots. Each band is a `Step::Owned`
    /// naming a different bot, and laid end to end the bands are exactly
    /// the route a one-bot roster lays flat -- every tile in one band,
    /// bands abutting along the path, no bot inside another's stretch.
    #[test]
    fn a_long_run_is_laid_by_several_bots_in_contiguous_bands() {
        let furnace = FactorioEntity::new_stone_furnace(&Position::new(5.0, 5.0), Direction::North);
        let lab = crate::test_world::lab(&Position::new(25.5, 5.5));
        let roster = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut banded =
            crate::test_world::connect_ctx_with_roster(vec![furnace.clone(), lab.clone()], &roster);
        let mut solo = crate::test_world::connect_ctx_with_roster(
            vec![furnace.clone(), lab.clone()],
            &[BotId(1)],
        );
        let item: ItemId = "iron-plate".into();
        let steps = connect_steps(&mut banded, &furnace, &lab, &item).expect("open ground");
        let flat = connect_steps(&mut solo, &furnace, &lab, &item).expect("open ground");

        assert!(
            !flat.iter().any(|s| matches!(s, Step::Owned { .. })),
            "a one-bot roster lays the run flat, as before bands existed"
        );
        let route = placements(&flat, BELT);
        let min = usize::try_from(HANDOVER_WALK_TICKS / PLACE_TICKS).unwrap();
        assert!(
            route.len() >= 2 * min,
            "the fixture route is {} tiles",
            route.len()
        );

        let bands = bands_of(&steps);
        assert!(
            bands.len() >= 2,
            "a {}-tile run over four bots splits: {bands:?}",
            route.len()
        );
        let owners: std::collections::BTreeSet<BotId> = bands.iter().map(|(b, _)| *b).collect();
        assert_eq!(
            owners.len(),
            bands.len(),
            "each band has its own bot: {bands:?}"
        );
        let joined: Vec<(Position, u8)> = bands.iter().flat_map(|(_, b)| b.clone()).collect();
        assert_eq!(
            joined, route,
            "the bands laid end to end are the flat route, in path order"
        );
        for (_, band) in &bands {
            assert!(
                band.len() >= min,
                "no band is shorter than a walk is worth: {bands:?}"
            );
        }
        // No belt escapes the bands onto the chain actor's own list.
        let loose: Vec<_> = steps
            .iter()
            .filter(|s| matches!(s, Step::Act(a) if matches!(&a.kind, ActionKind::Place { entity } if entity.name == BELT)))
            .collect();
        assert!(loose.is_empty(), "every belt is in a band");
        // The inserters at each end are still the chain actor's.
        assert_eq!(placements(&steps, INSERTER).len(), 2);
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

    /// A run whose belts stand and whose arms do not -- the shape an
    /// abandoned batch leaves, every arm needing a circuit and every belt
    /// only iron -- is finished with its two arms on the tiles a fresh run
    /// would give them, and no belt is laid. Before `standing_run` the
    /// belts were obstacles, and the chest's next side got a second run.
    #[test]
    fn a_standing_run_missing_its_arms_is_finished_with_two_arms() {
        use factorio_bot_core::num_traits::FromPrimitive;
        let item: ItemId = "iron-plate".into();
        let (mut plain, source, sink) = crate::test_world::two_chests_on_open_ground();
        let fresh = connect_steps_with(&mut plain, &source, &sink, &item, INSERTER)
            .expect("the control: two chests on open ground");
        let (mut ctx, source, sink) = crate::test_world::two_chests_on_open_ground();
        for (at, facing) in placements(&fresh, BELT) {
            let facing = Direction::from_u8(facing).expect("a belt has a facing");
            ctx.state
                .create_entity(FactorioEntity::new_transport_belt(&at, facing));
        }
        let steps = connect_steps_with(&mut ctx, &source, &sink, &item, INSERTER)
            .expect("the standing belts are this run, not an obstacle");
        assert_eq!(
            placements(&steps, INSERTER),
            placements(&fresh, INSERTER),
            "exactly the two arms, on the tiles the fresh run gave them"
        );
        assert_eq!(
            placements(&steps, BELT),
            Vec::new(),
            "no belt is laid: they all stand"
        );
    }

    /// A chain that leaves the source's door and stops short of the sink's
    /// is NOT this run. The belts are obstacles as they always were, and the
    /// run is routed round them -- it lays belts -- or refuses; it is never
    /// answered with two arms and nothing between them.
    #[test]
    fn a_chain_that_ends_short_of_the_sink_is_not_taken_as_the_run() {
        use factorio_bot_core::num_traits::FromPrimitive;
        let item: ItemId = "iron-plate".into();
        let (mut plain, source, sink) = crate::test_world::two_chests_on_open_ground();
        let fresh = connect_steps_with(&mut plain, &source, &sink, &item, INSERTER)
            .expect("the control: two chests on open ground");
        let belts = placements(&fresh, BELT);
        assert!(belts.len() > 3, "fixture precondition: a run worth cutting");
        let (mut ctx, source, sink) = crate::test_world::two_chests_on_open_ground();
        for (at, facing) in &belts[..belts.len() - 2] {
            let facing = Direction::from_u8(*facing).expect("a belt has a facing");
            ctx.state
                .create_entity(FactorioEntity::new_transport_belt(at, facing));
        }
        if let Ok(steps) = connect_steps_with(&mut ctx, &source, &sink, &item, INSERTER) {
            assert!(
                !placements(&steps, BELT).is_empty(),
                "a chain two tiles short of the sink's door was taken as the whole run: {:?}",
                placements(&steps, INSERTER)
            );
        }
    }

    /// A tile the caller has reserved is closed to the route -- no belt, no
    /// arm -- and the run is still made round it. Then the sink's every
    /// side is reserved, and the run is refused **before anything lands in
    /// the overlay**, naming the reserved tiles among the blockers: the
    /// budget a caller declares is enforced with the same promise every
    /// other refusal here makes.
    #[test]
    fn a_reserved_tile_is_kept_off_and_a_reserved_perimeter_refuses_before_placing() {
        let (mut plain, source, sink) = crate::test_world::two_chests_on_open_ground();
        let straight =
            connect_steps_with(&mut plain, &source, &sink, &"iron-plate".into(), INSERTER)
                .expect("the control: two chests on open ground");
        let kept = Position::new(8.5, 3.5);
        assert!(
            placements(&straight, BELT)
                .iter()
                .any(|(at, _)| *at == kept),
            "fixture precondition: the straight run crosses {kept}: {:?}",
            placements(&straight, BELT)
        );

        let (mut ctx, source, sink) = crate::test_world::two_chests_on_open_ground();
        let steps = connect_steps_reserving(
            &mut ctx,
            &source,
            &sink,
            &"iron-plate".into(),
            INSERTER,
            std::slice::from_ref(&kept),
        )
        .expect("one reserved tile on open ground is a detour, not a wall");
        let laid: Vec<Position> = placements(&steps, BELT)
            .into_iter()
            .chain(placements(&steps, INSERTER))
            .chain(placements(&steps, UNDERGROUND))
            .map(|(at, _)| at)
            .collect();
        assert!(
            !laid.contains(&kept),
            "the run stands on the reserved tile {kept}: {laid:?}"
        );
        assert!(
            laid.len() > straight.len() - 2,
            "and it went round rather than through: {} placements against {} straight",
            laid.len(),
            placements(&straight, BELT).len() + 2
        );

        // Every side of the sink, arm tile and belt tile alike.
        let (mut boxed, source, sink) = crate::test_world::two_chests_on_open_ground();
        let before = boxed.state.entities_within(&sink.position, 30.).len();
        let mut perimeter = Vec::new();
        for (dx, dy) in [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)] {
            for reach in [1., 2.] {
                perimeter.push(Position::new(
                    sink.position.x() + dx * reach,
                    sink.position.y() + dy * reach,
                ));
            }
        }
        let refusal = connect_steps_reserving(
            &mut boxed,
            &source,
            &sink,
            &"iron-plate".into(),
            INSERTER,
            &perimeter,
        )
        .expect_err("a chest with every side reserved has no end to load into");
        match &refusal {
            ConnectRefusal::NoRoute { blocked } => {
                assert!(
                    blocked.iter().any(|tile| perimeter.contains(tile)),
                    "the refusal names a reserved tile, so the reader knows it is a \
                     budget and not an obstacle: {blocked:?}"
                );
            }
            other => panic!("refused for the wrong reason: {other}"),
        }
        assert_eq!(
            boxed.state.entities_within(&sink.position, 30.).len(),
            before,
            "a refusal must leave the overlay exactly as it found it"
        );
    }

    /// The load arm of a run out of `from`, by position.
    fn load_arm(steps: &[Step]) -> Position {
        placements(steps, INSERTER)
            .into_iter()
            .next()
            .map(|(at, _)| at)
            .expect("a run places its load arm first")
    }

    /// A pair the plan has KEPT for the run out of a chest is the pair the
    /// run takes -- not the first free side in the scan's fixed order.
    ///
    /// On open ground the scan's first free side of the source at
    /// `(4.5, 5.5)` is NORTH (arm `(4.5, 4.5)`), which is the control. With
    /// its EAST pair reserved in the state as a product exit, the run leaves
    /// by the east arm at `(5.5, 5.5)`; `run-1788936524-99544`'s first plan
    /// took the north side of exactly such a chest with the kept exit
    /// standing open beside it, and its own belt then sealed the exit.
    ///
    /// And the reservation is a way OUT: the sink's own reserved pair is
    /// not preferred for the run INTO it, because a run into a chest is not
    /// the run its exit was kept for. The unload arm stays where the plain
    /// order puts it.
    #[test]
    fn a_run_out_of_a_chest_leaves_by_the_pair_kept_for_it() {
        let (mut plain, source, sink) = crate::test_world::two_chests_on_open_ground();
        let control =
            connect_steps_with(&mut plain, &source, &sink, &"iron-plate".into(), INSERTER)
                .expect("the control: two chests on open ground");
        assert_eq!(
            load_arm(&control),
            Position::new(4.5, 4.5),
            "fixture precondition: unkept, the scan's first free side of the source is north"
        );

        let (mut kept, source, sink) = crate::test_world::two_chests_on_open_ground();
        let east_exit = [Position::new(5.5, 5.5), Position::new(6.5, 5.5)];
        kept.state
            .reserve_ground(&east_exit, "a cell's product exit");
        // The sink's west pair, reserved as if it were its exit: a run INTO
        // the sink must not take it.
        let sink_exit = [Position::new(11.5, 5.5), Position::new(10.5, 5.5)];
        kept.state
            .reserve_ground(&sink_exit, "a cell's product exit");
        let steps = connect_steps_with(&mut kept, &source, &sink, &"iron-plate".into(), INSERTER)
            .expect("a kept exit on open ground routes");
        let arms = placements(&steps, INSERTER);
        assert_eq!(
            arms[0].0,
            Position::new(5.5, 5.5),
            "the run leaves by the kept east exit: {arms:?}"
        );
        assert!(
            placements(&steps, BELT)
                .iter()
                .any(|(at, _)| *at == Position::new(6.5, 5.5)),
            "and its first belt stands on the kept belt tile: {:?}",
            placements(&steps, BELT)
        );
        assert_eq!(
            arms[1].0,
            Position::new(12.5, 4.5),
            "the run INTO the sink keeps the plain order, not the sink's kept exit: {arms:?}"
        );
    }

    /// Only a whole pair -- inserter cell AND belt cell -- is this chest's
    /// exit. A neighbour's reservation that puts one of its tiles on this
    /// perimeter is that neighbour's way out, and the run does not prefer
    /// it: with `(6.5, 5.5)` and `(7.5, 5.5)` kept (a pair out of a chest
    /// that would stand at `(5.5, 5.5)`), the source's east candidate has
    /// its belt cell kept and its arm cell not, and the run still leaves by
    /// the north.
    #[test]
    fn a_lone_kept_tile_on_the_perimeter_is_not_this_chests_exit() {
        let (mut ctx, source, sink) = crate::test_world::two_chests_on_open_ground();
        let neighbours_exit = [Position::new(6.5, 5.5), Position::new(7.5, 5.5)];
        ctx.state
            .reserve_ground(&neighbours_exit, "a cell's product exit");
        let steps = connect_steps_with(&mut ctx, &source, &sink, &"iron-plate".into(), INSERTER)
            .expect("a neighbour's reservation on open ground routes");
        assert_eq!(
            load_arm(&steps),
            Position::new(4.5, 4.5),
            "the run leaves by the plain first free side, north"
        );
    }

    /// A kept pair the run cannot leave by is not imposed. The source's east
    /// pair is kept and free, and the belt tile is sealed into a one-tile
    /// pocket: a block of wall seven columns wide east of it -- wider than
    /// the reach -- and the three tiles west of the chest walled too, because
    /// the first version of this fixture left them open and the run
    /// tunnelled WEST out of the pocket, under its own arm and the chest it
    /// was loading from, to `(1.5, 5.5)`. Legal, and not the case under
    /// test. With no landing there, a run from the kept pair refuses, the
    /// run falls back to the plain order and leaves by the north, and
    /// places everything it promised: a preference that turned a routable
    /// chest into a refusal would be worse than the scan it replaced.
    #[test]
    fn a_kept_pair_the_run_cannot_leave_by_falls_back_to_the_plain_order() {
        use crate::test_world::{connect_ctx_with_roster, iron_chest, stone_wall};
        let source = iron_chest(&Position::new(4.5, 5.5));
        let sink = iron_chest(&Position::new(4.5, 15.5));
        let mut entities = vec![source.clone(), sink.clone()];
        for x in 6..=12 {
            for y in -30..=30 {
                let at = Position::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                if at == Position::new(6.5, 5.5) {
                    continue;
                }
                entities.push(stone_wall(&at));
            }
        }
        for x in [1.5, 2.5, 3.5] {
            entities.push(stone_wall(&Position::new(x, 5.5)));
        }
        let mut ctx = connect_ctx_with_roster(entities, &[]);
        let east_exit = [Position::new(5.5, 5.5), Position::new(6.5, 5.5)];
        ctx.state
            .reserve_ground(&east_exit, "a cell's product exit");
        let steps = connect_steps_with(&mut ctx, &source, &sink, &"iron-plate".into(), INSERTER)
            .expect("a sealed kept pair falls back to the plain order rather than refusing");
        assert_eq!(
            load_arm(&steps),
            Position::new(4.5, 4.5),
            "the run leaves by the north, the plain order's first free side"
        );
        assert!(
            !placements(&steps, BELT)
                .iter()
                .any(|(at, _)| *at == Position::new(6.5, 5.5)),
            "and nothing stands in the pocket"
        );
    }

    // -----------------------------------------------------------------------
    // The tap
    // -----------------------------------------------------------------------

    /// Every `Chop` in `steps`, as `(position, entity)`.
    fn chops(steps: &[Step]) -> Vec<(Position, String)> {
        let mut out = Vec::new();
        for step in steps {
            match step {
                Step::Act(action) => {
                    if let ActionKind::Chop { pos, entity, .. } = &action.kind {
                        out.push((pos.clone(), entity.clone()));
                    }
                }
                Step::Owned { steps, .. } => out.extend(chops(steps)),
                _ => {}
            }
        }
        out
    }

    /// The source chest of `two_chests_on_open_ground`, its run to the
    /// second chest STANDING (belts and both arms, as the fresh run laid
    /// them), the source's three other sides taken by chests, and a third
    /// chest to the south-east that wants the same thing the run carries.
    ///
    /// The shape of `run-1788946451-86723`'s stuck replan in miniature: a
    /// 1x1 source with no free side and one run already leaving it.
    fn boxed_source_with_a_standing_run(
        box_east: bool,
    ) -> (ExpansionCtx, FactorioEntity, FactorioEntity, Vec<Position>) {
        use factorio_bot_core::num_traits::FromPrimitive;
        let item: ItemId = "copper-plate".into();
        let (mut plain, source, first) = crate::test_world::two_chests_on_open_ground();
        let fresh = connect_steps_with(&mut plain, &source, &first, &item, INSERTER)
            .expect("the control: two chests on open ground");
        let mut entities = vec![source.clone(), first.clone()];
        let mut standing_belts = Vec::new();
        for (at, facing) in placements(&fresh, BELT) {
            let facing = Direction::from_u8(facing).expect("a belt has a facing");
            entities.push(FactorioEntity::new_transport_belt(&at, facing));
            standing_belts.push(at);
        }
        for (at, facing) in placements(&fresh, INSERTER) {
            let facing = Direction::from_u8(facing).expect("an arm has a facing");
            entities.push(FactorioEntity::new_named_inserter(
                INSERTER.to_string(),
                &at,
                facing,
            ));
        }
        // The fresh run leaves by the NORTH side (`first_free_perimeter`'s
        // order); the other three are spent here.
        let mut blockers = vec![Position::new(4.5, 6.5), Position::new(3.5, 5.5)];
        if box_east {
            blockers.push(Position::new(5.5, 5.5));
        }
        for at in blockers {
            entities.push(crate::test_world::iron_chest(&at));
        }
        let second = crate::test_world::iron_chest(&Position::new(12.5, 10.5));
        entities.push(second.clone());
        let ctx = crate::test_world::connect_ctx_with_roster(entities, &[]);
        (ctx, source, second, standing_belts)
    }

    /// The owner's ruling of 2026-09-09 ("1 sounds good"): a source with no
    /// free side and a run already leaving it is tapped with a splitter
    /// spliced into that run, not refused. One splitter, facing the run's
    /// way, over one chopped belt tile and one free tile beside it; the
    /// branch starts in front of the free tile facing the same way; one
    /// arm, at the destination; and the chop is ordered before the
    /// splitter.
    #[test]
    fn a_source_with_no_free_side_is_tapped_where_its_run_already_leaves() {
        use factorio_bot_core::num_traits::FromPrimitive;
        let (mut ctx, source, second, standing) = boxed_source_with_a_standing_run(true);
        let item: ItemId = "copper-plate".into();
        let steps = connect_steps_with(&mut ctx, &source, &second, &item, INSERTER).unwrap_or_else(
            |refusal| panic!("a boxed-in source with a run out is tapped: {refusal}"),
        );

        let splitters = placements(&steps, SPLITTER);
        assert_eq!(splitters.len(), 1, "exactly one splitter: {splitters:?}");
        let (splitter_at, splitter_facing) = splitters[0].clone();
        assert_eq!(
            splitter_facing,
            dir(Direction::East),
            "the splitter faces the way the standing run runs"
        );

        let chopped = chops(&steps);
        assert_eq!(chopped.len(), 1, "exactly one belt comes up: {chopped:?}");
        let (chopped_at, chopped_entity) = chopped[0].clone();
        assert_eq!(chopped_entity, BELT);
        assert!(
            standing.contains(&chopped_at),
            "the chopped tile {chopped_at} is a belt of the standing run {standing:?}"
        );
        // The splitter straddles the chopped tile and the tile beside it:
        // its position is their midpoint, half a tile off the chopped one
        // across the direction of travel.
        assert_eq!(
            splitter_at.x(),
            chopped_at.x(),
            "same column as the chopped belt"
        );
        assert_eq!(
            (splitter_at.y() - chopped_at.y()).abs(),
            0.5,
            "half a tile off it across the run: splitter {splitter_at}, belt {chopped_at}"
        );
        let side_y = chopped_at.y() + 2. * (splitter_at.y() - chopped_at.y());
        assert!(
            !standing.contains(&Position::new(chopped_at.x(), side_y)),
            "the splitter's other half stands on free ground, not on the run"
        );

        // The branch: its first tile is in front of the splitter's free
        // half, facing the run's way, and none of it lies on the run. On
        // open ground it is belts only -- a surface route from the next
        // splice back beats a tunnel from the nearest one, and the nearest
        // one here launches straight at the first chest's standing arm.
        assert!(
            placements(&steps, UNDERGROUND).is_empty(),
            "no tunnel where a surface branch exists: {:?}",
            placements(&steps, UNDERGROUND)
        );
        let branch = placements(&steps, BELT);
        assert!(!branch.is_empty(), "the branch lays belts");
        let (first_at, first_facing) = branch[0].clone();
        assert_eq!(
            first_at,
            Position::new(chopped_at.x() + 1., side_y),
            "the branch starts on the splitter's second output tile: {branch:?}"
        );
        assert_eq!(
            Direction::from_u8(first_facing),
            Some(Direction::East),
            "and continues the splitter's direction rather than turning on its output"
        );
        for (at, _) in &branch {
            assert!(
                !standing.contains(at),
                "the branch is laid over the standing run at {at}"
            );
        }

        // One arm, at the destination, picking up from the branch.
        let arms = placements(&steps, INSERTER);
        assert_eq!(
            arms.len(),
            1,
            "the load arm stands; only the unload arm is placed: {arms:?}"
        );
        let (arm_at, _) = arms[0].clone();
        assert_eq!(
            (arm_at.x() - second.position.x()).abs() + (arm_at.y() - second.position.y()).abs(),
            1.0,
            "the arm is on the destination's perimeter"
        );

        // Ordering: the chop is emitted before the splitter's placement, and
        // a link states it.
        let index_of = |pred: &dyn Fn(&Action) -> bool| {
            steps
                .iter()
                .position(|s| matches!(s, Step::Act(a) if pred(a)))
        };
        let chop_index = index_of(&|a| matches!(a.kind, ActionKind::Chop { .. })).unwrap();
        let splitter_index = index_of(
            &|a| matches!(&a.kind, ActionKind::Place { entity } if entity.name == SPLITTER),
        )
        .unwrap();
        assert!(
            chop_index < splitter_index,
            "the belt comes up before the splitter goes down"
        );
        let chop_id = match &steps[chop_index] {
            Step::Act(a) => a.id,
            _ => unreachable!(),
        };
        let splitter_id = match &steps[splitter_index] {
            Step::Act(a) => a.id,
            _ => unreachable!(),
        };
        assert!(
            steps
                .iter()
                .any(|s| matches!(s, Step::Link { from, to, .. } if *from == chop_id && *to == splitter_id)),
            "a stated edge orders the chop before the splitter"
        );

        // The bill names the splitter and exactly one arm.
        let have = |name: &str| -> Option<u32> {
            steps.iter().find_map(|s| match s {
                Step::Subgoal(Goal::Have { item, count, .. }) if item.as_str() == name => {
                    Some(*count)
                }
                _ => None,
            })
        };
        assert_eq!(have(SPLITTER), Some(1));
        assert_eq!(have(INSERTER), Some(1));

        // And the overlay agrees with the steps: the chopped tile is gone,
        // the splitter stands.
        assert!(
            ctx.state
                .entity_at(&chopped_at)
                .is_none_or(|e| e.name != BELT),
            "the chopped belt has left the overlay"
        );
        assert!(
            ctx.state
                .entity_at(&splitter_at)
                .is_some_and(|e| e.name == SPLITTER),
            "the splitter is in the overlay"
        );
    }

    /// The tap is a FALLBACK. With one side of the source still free the
    /// plain run is laid from it, byte for byte as before, and no splitter
    /// or chop appears -- every plan that routed before this existed is the
    /// same plan.
    #[test]
    fn a_source_with_a_free_side_is_never_tapped() {
        let (mut ctx, source, second, _) = boxed_source_with_a_standing_run(false);
        let item: ItemId = "copper-plate".into();
        let steps = connect_steps_with(&mut ctx, &source, &second, &item, INSERTER)
            .expect("a source with its east side free is routed from it");
        assert!(placements(&steps, SPLITTER).is_empty(), "no splitter");
        assert!(chops(&steps).is_empty(), "no chop");
        assert_eq!(
            placements(&steps, INSERTER).len(),
            2,
            "a fresh run with an arm at each end"
        );
        assert_eq!(
            placements(&steps, INSERTER)[0].0,
            Position::new(5.5, 5.5),
            "from the source's free east side"
        );
    }

    /// A source with no free side and NO run leaving it refuses as it
    /// always did -- the four tiles named -- and says the tap was tried and
    /// why it could not be: silence about a fallback is how a fallback goes
    /// unmeasured.
    #[test]
    fn a_boxed_in_source_with_no_run_out_refuses_and_says_the_tap_was_tried() {
        let (source, _) = (crate::test_world::iron_chest(&Position::new(4.5, 5.5)), ());
        let mut entities = vec![source.clone()];
        for at in [
            Position::new(4.5, 4.5),
            Position::new(5.5, 5.5),
            Position::new(4.5, 6.5),
            Position::new(3.5, 5.5),
        ] {
            entities.push(crate::test_world::iron_chest(&at));
        }
        let sink = crate::test_world::iron_chest(&Position::new(12.5, 5.5));
        entities.push(sink.clone());
        let mut ctx = crate::test_world::connect_ctx_with_roster(entities, &[]);
        let before = ctx.state.entities_within(&source.position, 30.).len();
        let refusal = connect_steps_with(&mut ctx, &source, &sink, &"iron-plate".into(), INSERTER)
            .expect_err("a chest with no side and no run out cannot be connected");
        match &refusal {
            ConnectRefusal::TapRefused { blocked, why } => {
                assert_eq!(
                    blocked.len(),
                    4,
                    "the four neighbours, as before: {blocked:?}"
                );
                assert!(
                    why.contains("no belt run leaves"),
                    "the tap says why it could not be: {why}"
                );
            }
            other => panic!("expected TapRefused, got {other:?}"),
        }
        let text = refusal.to_string();
        assert!(
            text.starts_with("no belt route, blocked by 4 tile(s):"),
            "the refusal opens with the sentence it always had: {text}"
        );
        assert_eq!(
            ctx.state.entities_within(&source.position, 30.).len(),
            before,
            "and nothing landed in the overlay"
        );
    }
}
