//! The method that claims [`Goal::Sustain`] — capacity **and** the supply
//! that keeps it running with no bot in the loop.
//!
//! Design note: `docs/superpowers/notes/2026-09-06-standing-goals.md`.
//! Result of the first live rung:
//! `docs/superpowers/notes/2026-09-06-a-cell-that-feeds-itself.md`.
//!
//! # What a standing goal decomposes to
//!
//! ```text
//! Sustain{item, rate, window}
//! ├── capacity   the cells `Producing` would build, sited near the FUEL
//! ├── supply     a drill on coal dropping into a buffer, and a belt run
//! │              from that buffer to every burner the cells contain --
//! │              including the coal drill's own fuel slot
//! ├── power      nothing: every machine here is a burner, deliberately
//! └── source     not modelled; see "What is still missing"
//! ```
//!
//! # Why the fuel is belted rather than the drills made electric
//!
//! Measured, not preferred, against seed 31337's own t=0 recipe table
//! (`workspace/scripts/map.json`, the `recipes` section):
//!
//! | recipe | `enabled` at t=0 | ingredients |
//! |---|---|---|
//! | `burner-inserter` | **true** | 1 iron-plate, 1 iron-gear-wheel |
//! | `transport-belt` | **true** | 1 iron-plate, 1 iron-gear-wheel |
//! | `inserter` | false | + 1 electronic-circuit |
//! | `electric-mining-drill` | false | + 3 electronic-circuit |
//! | `small-electric-pole` | false | wood, copper-cable |
//! | `offshore-pump` / `boiler` / `steam-engine` | false | — |
//! | `electric-furnace` | false | + **5 advanced-circuit** (oil) |
//!
//! So the electric route is not one rung but several: generation, poles and
//! the drill each need research before anything electric turns, and the
//! electric *furnace* is behind oil processing however much research is done,
//! which means the smelting half stays a burner whatever happens to the
//! mining half. Belting coal needs **no research at all** — both items it
//! costs are enabled from the start — and it is therefore the only mechanism
//! reachable at stage 1.
//!
//! The burner inserter carries one further property this arrangement rests
//! on: **it takes its own fuel out of the coal it is moving**, so the arms on
//! a coal belt need no supply of their own. That is a claim about the game and
//! nothing in this crate can check it; it is measured in the live run.
//!
//! # The arrangement, and why the buffer chest is in it
//!
//! A burner mining drill has no inventory an inserter can reach into — it
//! *pushes* to its drop target and is not a valid pickup for an arm. So the
//! coal cannot be taken straight off the drill: it is dropped into a
//! `wooden-chest` standing on the drill's own drop tile, and every belt run
//! starts from that chest, which an inserter **can** take from. One of those
//! runs goes back into the coal drill, which is what makes the source itself
//! standing rather than hand-charged.
//!
//! # What is still missing, said plainly
//!
//! * **Resource depletion is not modelled.** `Effect::ConsumeResource` is
//!   emitted by hand mining only, so nothing checks that the coal or the ore
//!   under the drills outlasts the window.
//! * **The furnace's output is not taken away.** A stone furnace holds a
//!   stack of its product, which is far more than a two-minute window at
//!   15/min needs, so this arrangement sustains a *window*, not a factory. A
//!   belt off the furnace is the next rung and is deliberately not here.
//! * **An ignition charge is still a bot's hands.** One coal reaches each
//!   burner by `insert`, because a machine with an empty fuel slot never
//!   turns over to receive the belt's first delivery. One coal is 1,600 ticks
//!   in a drill and 2,666 in a furnace, so any lead-in worth the name
//!   outlasts it — which is exactly the arithmetic the ten-minute default
//!   charge failed.

use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ItemId, Ticks};
use crate::method::connect::{ConnectRefusal, connect_steps_with};
use crate::method::produce::{CellSpec, DRILL, FURNACE, cell_spec, cells_for};
use crate::method::util::nearest_resource_tile;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::num_traits::{FromPrimitive, ToPrimitive};
use factorio_bot_core::types::{Direction, FactorioEntity, Pos, Position};

/// What a stage-1 burner cell burns. Not a parameter: `cell_spec` admits only
/// a stone furnace and a burner mining drill, and both burn this.
const FUEL: &str = "coal";

/// The inserter every arm in this arrangement is.
///
/// See the module header: the electric one is not craftable at t=0 and there
/// is no power to run it with either.
const ARM: &str = "burner-inserter";

/// What the coal drill drops into, and what every belt run starts from.
///
/// **`iron-chest`, and not the cheaper `wooden-chest`, for a planner reason
/// rather than a game one.** A wooden chest costs 2 wood, and no method in
/// this crate can obtain wood: `expand` answers
/// `NoApplicableMethod { goal: "have 2 wood" }` and the whole arrangement
/// refuses on its container. Trees are minable in the game and a roster starts
/// with one wood each, but neither fact reaches the planner. An iron chest
/// costs 8 iron plates, which is exactly what the rest of this plan is already
/// good at making.
///
/// Both are 1x1 and collide identically, so nothing about the geometry below
/// changes with the choice.
const BUFFER: &str = "iron-chest";

/// How long each burner is hand-charged for before the belt takes over.
///
/// One tick, which [`crate::method::produce::cell_steps_fuelled`] floors to
/// **one coal per machine** — 1,600 ticks in a drill, 2,666 in a furnace.
///
/// It is not zero because a burner machine with an empty fuel slot does not
/// run at all, and an inserter delivering into it is only reached once its
/// belt has been built and filled; the charge is what covers that gap. It is
/// not the ten-minute default because that default is precisely what let the
/// 2026-09-06 run return `SUSTAINED` off a single hand charge: 23 coal in a
/// drill is 36,800 ticks, five times the window it was measured over.
const IGNITION_TICKS: Ticks = 1;

/// How far from the fuel patch's nearest tile a drill site is looked for.
///
/// The same 12 `produce::CELL_SEARCH_RADIUS` uses and for the same reason: a
/// drill has to stand on the patch with a clear tile in front of it, so the
/// search has to be able to walk out of the middle of one to reach an edge.
const FUEL_SEARCH_RADIUS: i32 = 12;

/// How far from a machine something delivering into it may stand.
///
/// An inserter reaches exactly one tile, and the largest machine here is 2×2,
/// so anything feeding one is within two tiles of its centre. Used only to
/// decide whether a machine is **already** fed, which is what keeps a replan
/// from belting the same drill twice.
const FED_RADIUS: f64 = 2.5;

/// How far from a cell's **furnace** its own buffer chest may stand before
/// this method stops recognising it and sites a second one.
///
/// **Measured, after a live run halted on the guess.** The first version
/// searched from the *drill* with a radius of 6, reasoned from "a chest the
/// siting search chose is within a few tiles of the pair". It is not:
/// `free_area_near_where` searches from the **furnace** and the clearance test
/// pushes it out of the cell's own crowding, so on seed 31337 the chest landed
/// at `(-0.5, -30.5)` against a drill at `(-7, -27)` -- **7.38 tiles**, just
/// outside. The re-plan therefore did not recognise the chest it had just
/// built, sited a second one, and refused laying its belt over the first one's
/// (`run-1788679468-60128`: 288 of 288 actions succeeded, 81 entities stood,
/// and the milestone halted on `transport-belt fits at [7.5, -23.5] ... does
/// not hold there`).
///
/// So it is anchored where the siting is anchored and bounded by the same
/// `FREE_TILE_SEARCH_RADIUS + 1` the siting can reach, rather than by an
/// estimate of where the siting would land. A radius that cannot cover the
/// search it is meant to recognise is a bug however plausible the number
/// looks; the next cell is at least a whole cell away, so nothing else falls
/// inside it.
///
/// **The anchor is the load-bearing half, and the falsification says so.**
/// Reverting the radius alone to 6 leaves
/// `a_replan_over_the_arrangement_it_just_built_adds_nothing` green -- on the
/// compact test fixture the chest lands inside 6 tiles of the furnace anyway.
/// Reverting the *anchor* to the drill as well reproduces the live halt
/// exactly, with the same shape of message. A falsification that changes only
/// the plausible-looking number would have read as "this fix does nothing".
const LOCAL_BUFFER_RADIUS: f64 = crate::method::util::FREE_TILE_SEARCH_RADIUS as f64 + 1.;

/// Has the tile at `at` room for a chest that **three** belt runs leave from?
///
/// The nearest free tile is not good enough and that was measured, twice. A
/// chest is 1x1, so it has four perimeter tiles and each belt run claims one
/// of them plus the cell beyond it. `free_area_near` returns the nearest fit,
/// which is hard against the furnace, and `connect_steps` then refused the
/// third run with all four neighbours named:
/// `no belt route, blocked by 4 tile(s)`.
///
/// So a cell's buffer is sited where the 5x5 around it takes a belt: four
/// perimeter tiles and the four cells beyond them, with the corners thrown in
/// because a route leaving one side has to turn somewhere. Asked with a
/// `transport-belt`'s own footprint, which is what will actually stand there.
///
/// It is a **siting** predicate and not a routing one: passing it does not
/// promise a route exists, only that the chest is not walled in before one is
/// looked for. `connect_steps` still refuses by name when the ground between
/// the two ends is blocked.
fn room_to_route(state: &PlanState, at: &Position) -> bool {
    const BELT: &str = "transport-belt";
    for dy in -3i32..=3 {
        for dx in -3i32..=3 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let tile = Position::new(at.x() + f64::from(dx), at.y() + f64::from(dy));
            if !state.is_area_free(BELT, &tile) {
                return false;
            }
        }
    }
    true
}

/// Claims [`Goal::Sustain`].
pub struct Sustain;

/// A standing fuel source: a drill on the fuel patch and the buffer it drops
/// into.
#[derive(Debug, Clone)]
struct FuelSource {
    drill: Position,
    facing: Direction,
    buffer: Position,
}

/// Is something already delivering into the machine at `at` that is not a
/// bot's hands?
///
/// The predicate that makes this method idempotent, and it is deliberately
/// **narrow**: only an inserter counts, and only one whose drop tile the
/// machine actually covers, which is [`PlanState::delivers_into`]'s own
/// answer rather than a restatement of it. A drill dropping into a furnace
/// makes the furnace fed for *ore* and says nothing about its coal, so the
/// caller asks this once per burner and not once per cell.
fn fed_by_machine(state: &PlanState, at: &Position) -> bool {
    state
        .entities_within(at, FED_RADIUS)
        .into_iter()
        .any(|arm| state.pickup_position(&arm).is_some() && state.delivers_into(&arm.position, at))
}

/// The entity a machine standing at `at` really is, sized from its prototype.
///
/// `method::produce::machine()` leaves `bounding_box` at its default, and
/// `connect_steps` reads that field to find a machine's footprint perimeter —
/// so handing it a plan-placed drill straight out of the overlay would size a
/// 2×2 machine as a single cell and run the belt through it. Reading the
/// prototype here is what stops that, and it is the same call
/// `PlanState::delivers_into` makes on the other side.
fn sized(
    state: &PlanState,
    name: &str,
    at: &Position,
    facing: Direction,
) -> Option<FactorioEntity> {
    let area = state.collision_area_facing(name, at, facing)?;
    let entity_type = state
        .base()
        .entity_prototypes
        .get(name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| name.to_string());
    Some(FactorioEntity {
        name: name.to_string(),
        entity_type,
        position: at.clone(),
        direction: facing.to_u8().unwrap_or(0),
        bounding_box: area,
        ..Default::default()
    })
}

/// Site a drill on the fuel patch with a buffer chest on its drop tile.
///
/// Reuses an arrangement that already stands before siting a new one, which is
/// the idempotence half: a replan over a world that already has the source
/// must find it, not build a second one beside it.
///
/// The check that the drill really delivers into the buffer is asked of a
/// **fork with both placed**, so it is the same `delivers_into` a
/// `Condition::Feeds` would be checked with rather than an arithmetic
/// restatement of the drop offset. That is deliberate: the drop offset is a
/// hand-written table in `state.rs`, and a fixture that recomputed it would
/// agree with the code by construction.
fn plan_fuel_source(state: &PlanState, from: &Position) -> Result<FuelSource, PlannerError> {
    let anchor = nearest_resource_tile(state, FUEL, from, 1).ok_or_else(|| {
        PlannerError::NoPatchForCell {
            item: FUEL.into(),
            ore: FUEL.into(),
        }
    })?;
    // Already standing? A drill on the fuel patch that delivers into a buffer
    // is this arrangement, whoever built it.
    for drill in state.entities_within(&anchor, f64::from(FUEL_SEARCH_RADIUS) + 4.) {
        if drill.name != DRILL {
            continue;
        }
        let Some(facing) = Direction::from_u8(drill.direction) else {
            continue;
        };
        let Some(area) = state.collision_area_facing(DRILL, &drill.position, facing) else {
            continue;
        };
        if !state.covers_resource(&area, FUEL) {
            continue;
        }
        if let Some(buffer) = state
            .entities_within(&drill.position, FED_RADIUS)
            .into_iter()
            .find(|e| e.name == BUFFER && state.delivers_into(&drill.position, &e.position))
        {
            return Ok(FuelSource {
                drill: drill.position.clone(),
                facing,
                buffer: buffer.position,
            });
        }
    }

    let base = Pos::from(&anchor);
    for radius in 0..=FUEL_SEARCH_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                for facing in Direction::orthogonal() {
                    let (offset_x, offset_y) =
                        crate::method::util::tile_alignment_facing(state, DRILL, facing);
                    let candidate = Position::new(
                        f64::from(base.0 + dx) + offset_x,
                        f64::from(base.1 + dy) + offset_y,
                    );
                    if let Some(source) = fits_fuel_source(state, &candidate, facing) {
                        return Ok(source);
                    }
                }
            }
        }
    }
    Err(PlannerError::SustainNoFuelSource {
        fuel: FUEL.into(),
        radius: FUEL_SEARCH_RADIUS,
    })
}

/// Does a drill at `candidate` facing `facing`, with a buffer on its drop
/// tile, stand and deliver?
fn fits_fuel_source(
    state: &PlanState,
    candidate: &Position,
    facing: Direction,
) -> Option<FuelSource> {
    let area = state.collision_area_facing(DRILL, candidate, facing)?;
    if !state.covers_resource(&area, FUEL) {
        return None;
    }
    // Ground the plan has already promised to a mining action is not ground a
    // drill may stand on -- `produce::fit`'s reasoning, and the bot arrives to
    // `expected coal, found burner-mining-drill` without it.
    if state.covers_claimed_resource(&area) {
        return None;
    }
    if !state.is_area_free_facing(DRILL, candidate, facing) {
        return None;
    }
    // The buffer goes on the tile the drop point lands in. A chest is 1x1, so
    // its centre is that tile's centre -- the half-integer every resource
    // position in this project already sits on.
    let drop = FactorioEntity::new_burner_mining_drill(candidate, facing).drop_position?;
    let buffer = Position::new(drop.x().floor() + 0.5, drop.y().floor() + 0.5);
    if !state.is_area_free(BUFFER, &buffer) {
        return None;
    }
    // A chest on ore is a chest on ground a drill wanted, exactly as
    // `produce::fit` refuses a furnace there.
    let buffer_area = state.collision_area(BUFFER, &buffer)?;
    if state.covers_any_resource(&buffer_area) {
        return None;
    }
    // And the delivery itself, asked of both entities standing.
    let mut trial = state.fork();
    trial.create_entity(sized(state, DRILL, candidate, facing)?);
    trial.create_entity(sized(state, BUFFER, &buffer, Direction::North)?);
    if !trial.delivers_into(candidate, &buffer) {
        return None;
    }
    Some(FuelSource {
        drill: candidate.clone(),
        facing,
        buffer,
    })
}

/// One `Place` for the buffer chest, in the shape every other placement in
/// this crate takes.
fn place_buffer(ctx: &mut ExpansionCtx, at: &Position) -> Option<Step> {
    let entity = sized(&ctx.state, BUFFER, at, Direction::North)?;
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
    let min_radius = ctx.state.placement_clearance(BUFFER).unwrap_or(0.0);
    let step = Step::Act(Box::new(crate::action::Action {
        id: ctx.ids.next(),
        kind: crate::action::ActionKind::Place {
            entity: Box::new(entity.clone()),
        },
        pre: vec![
            crate::action::Condition::AtPosition {
                who: crate::action::Actor::Role,
                pos: at.clone(),
                radius: build,
                min_radius,
            },
            crate::action::Condition::AreaFree {
                pos: at.clone(),
                entity: BUFFER.into(),
                direction: entity.direction,
            },
            crate::action::Condition::HasItem {
                who: crate::action::Actor::Role,
                item: BUFFER.into(),
                count: 1,
            },
        ],
        eff: vec![
            crate::action::Effect::LoseItem {
                who: crate::action::Actor::Role,
                item: BUFFER.into(),
                count: 1,
            },
            crate::action::Effect::CreateEntity(Box::new(entity)),
        ],
        duration: crate::method::have::PLACE_TICKS,
        pinned: None,
        label: format!("place {BUFFER} at {at} -- buffer the {FUEL} the belts carry"),
    }));
    ctx.state
        .create_entity(sized(&ctx.state.fork(), BUFFER, at, Direction::North)?);
    Some(step)
}

/// Belt `FUEL` from the buffer into one burner, unless something already
/// delivers into it.
///
/// A refusal is turned into [`PlannerError::SustainNoRouteForFuel`] carrying
/// the primitive's own sentence, because "no belt route, blocked by 3 tiles:
/// ..." says more about the map than any wording this method could invent.
fn feed(
    ctx: &mut ExpansionCtx,
    buffer: &FactorioEntity,
    machine: &FactorioEntity,
) -> Result<Vec<Step>, PlannerError> {
    if fed_by_machine(&ctx.state, &machine.position) {
        return Ok(Vec::new());
    }
    let fuel: ItemId = FUEL.into();
    connect_steps_with(ctx, buffer, machine, &fuel, ARM).map_err(|refusal| {
        PlannerError::SustainNoRouteForFuel {
            fuel: FUEL.into(),
            machine: machine.name.clone(),
            from: buffer.position.to_string(),
            to: machine.position.to_string(),
            why: match &refusal {
                ConnectRefusal::NoRoute { .. }
                | ConnectRefusal::SpanTooLong { .. }
                | ConnectRefusal::NotCardinal => refusal.to_string(),
            },
        }
    })
}

impl Method for Sustain {
    fn name(&self) -> &'static str {
        "sustain"
    }

    /// Claims [`Goal::Sustain`] and nothing else.
    ///
    /// Unconditionally — including for an item no cell can make and for a
    /// window of zero. A false answer here would produce `NoApplicableMethod`,
    /// which says only "nobody understood this"; the refusals this method
    /// raises instead say *which* half is missing, and the supervisor turns
    /// either into a `stuck` milestone carrying the planner's own code.
    fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
        matches!(goal, Goal::Sustain { .. })
    }

    /// One bot builds one arrangement: the drills, the furnaces, the chest,
    /// the belts, the arms and the ignition coal all have to meet in one
    /// inventory before any of them can be placed.
    fn converges(&self, _goal: &Goal, _state: &PlanState) -> bool {
        true
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Sustain {
            item,
            per_minute,
            window_ticks,
        } = goal
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let spec: CellSpec = cell_spec(&ctx.state, item)
            .ok_or_else(|| PlannerError::NoCellProduces { item: item.clone() })?;
        let needed = cells_for(*per_minute, spec.ticks_per_item)?;

        // Everything is sited from the FUEL, not from the roster. A cell a
        // belt cannot reach cannot be fed, and `enclosure::window` is 48 tiles
        // across, so the two patches have to be chosen against each other
        // rather than each against the bots. On seed 31337 the iron tile
        // nearest spawn is 31.4 tiles from the nearest coal and the two
        // patches' closest approach is 19.6 -- so siting from the bots refuses
        // and siting from the fuel does not.
        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        let source = plan_fuel_source(&ctx.state, &from)?;

        let mut steps: Vec<Step> = Vec::new();

        // The fuel source, if it is not there yet.
        let source_stands = ctx
            .state
            .entity_at(&source.drill)
            .is_some_and(|e| e.name == DRILL);
        if !source_stands {
            steps.push(Step::Subgoal(Goal::Have {
                item: DRILL.into(),
                count: 1,
                whose: Holder::Share(ctx.chain_actor),
            }));
            steps.push(Step::Subgoal(Goal::Have {
                item: BUFFER.into(),
                count: 1,
                whose: Holder::Share(ctx.chain_actor),
            }));
            // The drill alone: `cell_steps_fuelled` would place a furnace at
            // the second position, and a buffer is not a furnace. So the drill
            // is placed by the same helper the smelting cells use and the
            // chest by `place_buffer`.
            steps.extend(crate::method::produce::drill_only_steps(
                ctx,
                &source.drill,
                source.facing,
                IGNITION_TICKS,
                FUEL,
            ));
            if let Some(step) = place_buffer(ctx, &source.buffer) {
                steps.push(step);
            }
        }

        // The smelting cells, reusing whatever already stands.
        let mut cells = crate::method::produce::standing_cells(&ctx.state, &spec);
        cells.truncate(needed as usize);
        let build = needed.saturating_sub(cells.len() as u32);
        if build > 0 {
            let fresh = crate::method::produce::plan_cells(
                &ctx.state,
                &source.buffer,
                &spec,
                build,
                crate::method::produce::rate_cell_ore(&spec),
            )?;
            steps.extend(crate::method::produce::cell_steps_fuelled(
                ctx,
                &spec,
                &fresh,
                IGNITION_TICKS,
            ));
            cells.extend(fresh);
        }

        // And the belts.
        //
        // # Why there are two buffers and not one
        //
        // Measured on the first attempt, which had every run leaving the
        // source's own chest. A `wooden-chest` is 1x1 and therefore has
        // **four** perimeter tiles; the coal drill standing at its drop point
        // takes one, and `connect_steps` claims one more per run. Three runs
        // -- the drill's refuel and the cell's two burners -- do not fit, and
        // the third refused with all four neighbours named:
        // `no belt route, blocked by 4 tile(s)`. That refusal is the
        // measurement this shape comes from, not a guess about crowding.
        //
        // So the source chest carries at most two runs (its drill's refuel and
        // one haul) and each cell gets a chest of its own, sited on free
        // ground beside it, which the two burners are fed from. A roster
        // asking for more cells than the source chest has room to haul to
        // refuses by name at the run that does not fit, which is the honest
        // ceiling rather than a silent overlap.
        let buffer =
            sized(&ctx.state, BUFFER, &source.buffer, Direction::North).ok_or_else(|| {
                PlannerError::SustainNoFuelSource {
                    fuel: FUEL.into(),
                    radius: FUEL_SEARCH_RADIUS,
                }
            })?;
        // The coal drill's own fuel slot first: without it the source is
        // hand-charged and the whole arrangement is the thing this method
        // exists to stop.
        let coal_drill =
            sized(&ctx.state, DRILL, &source.drill, source.facing).ok_or_else(|| {
                PlannerError::SustainNoFuelSource {
                    fuel: FUEL.into(),
                    radius: FUEL_SEARCH_RADIUS,
                }
            })?;
        steps.extend(feed(ctx, &buffer, &coal_drill)?);
        for cell in &cells {
            // A chest beside the cell, hauled to from the source. Reused when
            // one already stands, for the replan.
            // The nearest chest that is not the SOURCE's own. Excluding it by
            // name is load-bearing and not defensive: on a map whose two
            // patches are close -- which is the only kind this arrangement
            // works on -- the source buffer falls inside this radius, and
            // without the exclusion the haul is planned from that chest to
            // itself and refuses with `from` and `to` the same position.
            // Found by the planner's own tests once the radius was widened.
            let mut candidates: Vec<FactorioEntity> = ctx
                .state
                .entities_within(&cell.furnace, LOCAL_BUFFER_RADIUS)
                .into_iter()
                .filter(|e| e.name == BUFFER && Pos::from(&e.position) != Pos::from(&source.buffer))
                .collect();
            // Nearest first, by an ordering that is total and float-free at
            // the comparison -- `entities_within` sorts by (x, y), which is
            // deterministic but is not "closest to the cell".
            candidates.sort_by(|a, b| {
                let d = |e: &FactorioEntity| {
                    (e.position.x() - cell.furnace.x()).hypot(e.position.y() - cell.furnace.y())
                };
                d(a).total_cmp(&d(b))
            });
            let local = match candidates.into_iter().next() {
                Some(existing) => existing.position,
                None => {
                    let at = crate::method::util::free_area_near_where(
                        &ctx.state,
                        &cell.furnace,
                        BUFFER,
                        |at| room_to_route(&ctx.state, at),
                    )
                    .ok_or_else(|| PlannerError::SustainNoFuelSource {
                        fuel: FUEL.into(),
                        radius: FUEL_SEARCH_RADIUS,
                    })?;
                    steps.push(Step::Subgoal(Goal::Have {
                        item: BUFFER.into(),
                        count: 1,
                        whose: Holder::Share(ctx.chain_actor),
                    }));
                    if let Some(step) = place_buffer(ctx, &at) {
                        steps.push(step);
                    }
                    at
                }
            };
            let local_entity =
                sized(&ctx.state, BUFFER, &local, Direction::North).ok_or_else(|| {
                    PlannerError::SustainNoFuelSource {
                        fuel: FUEL.into(),
                        radius: FUEL_SEARCH_RADIUS,
                    }
                })?;
            if !fed_by_machine(&ctx.state, &local) {
                steps.extend(feed(ctx, &buffer, &local_entity)?);
            }
            if let Some(drill) = sized(&ctx.state, DRILL, &cell.drill, cell.facing) {
                steps.extend(feed(ctx, &local_entity, &drill)?);
            }
            if let Some(furnace) = sized(&ctx.state, FURNACE, &cell.furnace, Direction::North) {
                steps.extend(feed(ctx, &local_entity, &furnace)?);
            }
        }

        if steps.is_empty() {
            // Everything stands. This is NOT satisfaction and must not be
            // reported as one: whether the rate held over `window_ticks` is a
            // fact about a window of history, which this crate has no clock to
            // read. `have::holds` answers `None` for exactly this reason, and
            // an empty network is this planner's word for "done".
            return Err(PlannerError::SustainSupplyNotStanding {
                item: item.clone(),
                per_minute: *per_minute,
                window_ticks: *window_ticks,
                inputs: format!(
                    "the whole arrangement stands -- the {} cell, its {FUEL} source, and a \
                     belted deliverer for every burner -- so there is nothing left to build",
                    spec.ore
                ),
            });
        }
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::{expand, holds, registry_for};
    use factorio_bot_core::factorio::util::add_to_rect;
    use factorio_bot_core::factorio::world::FactorioWorld;
    use factorio_bot_core::test_utils::{fixture_world, spawn_ore};
    use factorio_bot_core::types::Rect;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    /// The stock fixture, plus a coal patch **beside the iron one**.
    ///
    /// **This fixture is written by the same task as the code, and here is
    /// what it assumes.** `fixture_world` puts iron at `(-40, 40)` and coal at
    /// `(-60, 0)` — 44.7 tiles apart, which is further than one
    /// `enclosure::window` (48 tiles across, centred on the buffer) reaches, so
    /// on that world every belt run refuses and the success path cannot be
    /// exercised at all. This adds a second coal patch 14 tiles north of the
    /// iron, which is the same order of separation seed 31337 really has
    /// (19.6 tiles between the two patches' closest tiles, measured from
    /// `workspace/scripts/map.json`).
    ///
    /// It inherits `spawn_ore`'s known distortion: ore is spawned at **integer**
    /// positions, and a real resource entity sits at a tile centre. That is the
    /// one input for which `EntityGraph`'s flooring round-trip is lossless, so
    /// a geometry defect that depends on the half-tile offset would not show
    /// here. The offline plan against `map.json` is the check that does see it.
    fn world_with_coal_beside_the_iron() -> FactorioWorld {
        let world = fixture_world();
        let mut entities = Vec::new();
        spawn_ore(
            &mut entities,
            add_to_rect(&Rect::from_wh(8., 8.), &Position::new(-40., 26.)),
            FUEL,
        );
        world.update_chunk_entities(entities).unwrap();
        world
    }

    fn near_state() -> PlanState {
        PlanState::from_world(Arc::new(world_with_coal_beside_the_iron()), &[BotId(1)])
    }

    fn goal() -> Goal {
        Goal::Sustain {
            item: "iron-plate".into(),
            per_minute: 15,
            window_ticks: 7200,
        }
    }

    /// A sustain goal is never `already-satisfied`, so the method is always
    /// reached and always answers.
    #[test]
    fn the_method_is_reached_because_nothing_claims_the_goal_holds() {
        assert_eq!(holds(&goal(), &state()), None);
        assert!(Sustain.applicable(&goal(), &state()));
    }

    /// The fuel source stands **and delivers**, asked of both entities placed
    /// rather than of the drop-offset arithmetic that chose the site.
    #[test]
    fn the_fuel_drill_delivers_into_its_buffer() {
        let state = near_state();
        let source =
            plan_fuel_source(&state, &Position::new(0., 0.)).expect("the fixture world has coal");
        let mut trial = state.fork();
        trial.create_entity(
            sized(&state, DRILL, &source.drill, source.facing).expect("the drill is a prototype"),
        );
        trial.create_entity(
            sized(&state, BUFFER, &source.buffer, Direction::North).expect("so is the chest"),
        );
        assert!(
            trial.delivers_into(&source.drill, &source.buffer),
            "a drill that does not deliver into its buffer places perfectly and moves nothing"
        );

        // FALSIFICATION, and the substitution is asserted to have bitten: move
        // the buffer one tile further out and the delivery must stop holding.
        // A green here would mean `delivers_into` answers `true` for anything,
        // which would make the assertion above worthless.
        let away = Position::new(source.buffer.x(), source.buffer.y() - 3.);
        assert_ne!(
            Pos::from(&away),
            Pos::from(&source.buffer),
            "the substitution has to actually move the chest, or it proves nothing"
        );
        let mut moved = state.fork();
        moved.create_entity(
            sized(&state, DRILL, &source.drill, source.facing).expect("the drill is a prototype"),
        );
        moved.create_entity(sized(&state, BUFFER, &away, Direction::North).expect("chest"));
        assert!(
            !moved.delivers_into(&source.drill, &away),
            "a chest three tiles from a drill's drop point is not delivered into"
        );
    }

    /// **The evidence that decided the mechanism, as a test.** Every arm this
    /// method places is a `burner-inserter`, because the electric one is not
    /// craftable on a freeplay force at t=0 and there is no power to run it
    /// with either.
    #[test]
    fn every_arm_is_a_burner_inserter() {
        let roster = [BotId(1)];
        let net = expand(&[goal()], &near_state(), &registry_for(&roster), BotId(1))
            .expect("iron and coal are within one belt window of each other");
        let placed: Vec<String> = net
            .actions()
            .filter_map(|action| match &action.kind {
                crate::action::ActionKind::Place { entity } => Some(entity.name.clone()),
                _ => None,
            })
            .collect();
        let arms: Vec<&String> = placed.iter().filter(|n| n.ends_with("inserter")).collect();
        assert!(
            !arms.is_empty(),
            "a self-feeding cell that places no inserter has not belted anything: {placed:?}"
        );
        // The LITERAL, not `ARM`. Comparing against the constant the code
        // reads makes this test true whatever the constant says: it was
        // written that way first, a falsification set `ARM = "inserter"`, and
        // the test stayed green -- which is the "unexpected green under
        // substitution is a broken experiment" trap, found by running it.
        assert!(
            arms.iter().all(|n| n.as_str() == "burner-inserter"),
            "an `inserter` is not craftable at t=0 and needs power this stage has none of: {arms:?}"
        );
        assert!(
            placed.iter().any(|n| n == "transport-belt"),
            "and the arms have to be joined by a belt: {placed:?}"
        );
        assert!(
            placed.iter().any(|n| n == BUFFER),
            "the coal drill drops into a buffer an arm can take from: {placed:?}"
        );
    }

    /// The hand charge is an **ignition**, not a ten-minute load.
    ///
    /// The 2026-09-06 run returned `SUSTAINED` on 23 hand-delivered coal worth
    /// 36,800 ticks — five times the window it was measured over. A cell that
    /// feeds itself must not be able to pass that way, so the coal a bot
    /// carries is one per burner.
    #[test]
    fn no_burner_is_hand_charged_for_longer_than_it_takes_to_start() {
        let roster = [BotId(1)];
        let net = expand(&[goal()], &near_state(), &registry_for(&roster), BotId(1))
            .expect("iron and coal are within one belt window of each other");
        // Only the burners this arrangement BELTS are in scope. The plan also
        // hand-smelts the iron the drills and belts are made of, and those
        // furnaces are hand-fed on purpose -- they are one-shot crafting, not
        // the standing cell, and charging them is not what a `sustain` window
        // would be measured over. The belted burners are the ones a placed
        // `burner-inserter` drops into.
        let arms: Vec<Position> = net
            .actions()
            .filter_map(|action| match &action.kind {
                crate::action::ActionKind::Place { entity } if entity.name == ARM => {
                    entity.drop_position.clone()
                }
                _ => None,
            })
            .collect();
        assert!(
            !arms.is_empty(),
            "no belted burner at all means this test is measuring nothing"
        );
        let charges: Vec<u32> = net
            .actions()
            .filter_map(|action| match &action.kind {
                crate::action::ActionKind::Insert {
                    pos, item, count, ..
                } if item.as_str() == FUEL
                    // Inside the machine, not merely near it: an arm's drop
                    // point lands in one of the target's own tiles, so for the
                    // 2x2 burners here it is half a tile from the centre on
                    // each axis. A looser radius caught the hand-smelt
                    // furnaces standing beside the cell and made this test
                    // measure the wrong machines.
                    && arms.iter().any(|drop| {
                        (drop.x() - pos.x()).abs() <= 0.75 && (drop.y() - pos.y()).abs() <= 0.75
                    }) =>
                {
                    Some(*count)
                }
                _ => None,
            })
            .collect();
        assert!(
            !charges.is_empty(),
            "a burner with an empty fuel slot never turns over, so SOME ignition is expected"
        );
        // No BULK load. `cell_steps_fuelled(.., IGNITION_TICKS)` emits one
        // coal per burner; anything larger is a machine stocked to run
        // unattended, which is what a window would then be measured off.
        assert!(
            charges.iter().all(|&c| c <= 2),
            "the ten-minute default is 23 coal in a drill and 14 in a furnace: {charges:?}"
        );
        // And not much of it in total. **This is the number the run has to be
        // read against and it is NOT zero**: the `Goal::Have` chain that makes
        // the belts' own iron smelts by hand, and `smelt_steps` queues into a
        // furnace that already stands -- which, once the cell is up, is the
        // cell's furnace. So the cell is topped up a coal at a time while it
        // is being built, ~6 visits each on this fixture, and a lead-in has to
        // outlast that credit rather than the ignition alone.
        //
        // The bound is the OLD single-machine charge. A plan that hand-fed the
        // cell more than one machine's ten-minute load used to be the normal
        // case and is now the failure.
        let total: u32 = charges.iter().sum();
        assert!(
            total < 23,
            "hand-delivered coal at the belted burners is {total}, which is more than the \
             ten-minute drill charge this rung exists to get away from: {charges:?}"
        );
    }

    /// **The regression the first live run bought.**
    ///
    /// `run-1788679468-60128` dispatched all 288 of its actions, settled every
    /// one `success`, stood all 81 entities -- and then halted, because the
    /// **re-plan** did not recognise the chest it had just built and tried to
    /// lay a second belt run over the first. Nothing about the world was
    /// wrong; the method was not idempotent against its own output.
    ///
    /// So: expand, put everything the plan places into the world, expand
    /// again. The second answer must be the standing refusal and not another
    /// belt -- which is what `have::holds` answering `None` obliges, and what
    /// `supervisor.lua` turns into a satisfied milestone.
    #[test]
    fn a_replan_over_the_arrangement_it_just_built_adds_nothing() {
        let roster = [BotId(1)];
        let state = near_state();
        let net = expand(&[goal()], &state, &registry_for(&roster), BotId(1))
            .expect("iron and coal are within one belt window of each other");
        let mut built = state.fork();
        let mut placed = 0usize;
        for action in net.actions() {
            if let crate::action::ActionKind::Place { entity } = &action.kind {
                built.create_entity((**entity).clone());
                placed += 1;
            }
        }
        // The substitution this test rests on has to have happened: an empty
        // world would trivially "add nothing" for the wrong reason.
        assert!(
            placed > 30,
            "the first expansion has to have built the arrangement: {placed} placements"
        );
        let again = expand(&[goal()], &built, &registry_for(&roster), BotId(1));
        match again {
            Err(PlannerError::SustainSupplyNotStanding { .. }) => {}
            Err(other) => panic!(
                "a replan over the finished arrangement must refuse by the STANDING name, \
                 not by a fresh geometry failure: {other}"
            ),
            Ok(net) => {
                let names: Vec<String> = net
                    .actions()
                    .filter_map(|a| match &a.kind {
                        crate::action::ActionKind::Place { entity } => Some(entity.name.clone()),
                        _ => None,
                    })
                    .collect();
                panic!("a replan built a second arrangement beside the first: {names:?}");
            }
        }
    }

    /// And on a map whose fuel is further away than one belt window reaches,
    /// the answer is a **named refusal about the ground**, not a plan that
    /// would be measured `roster-fed`.
    ///
    /// The stock fixture is that map: iron at `(-40, 40)`, coal at `(-60, 0)`,
    /// 44.7 tiles apart against a 48-tile window centred on the buffer.
    #[test]
    fn fuel_out_of_belt_range_is_refused_by_name() {
        let roster = [BotId(1)];
        let err = expand(&[goal()], &state(), &registry_for(&roster), BotId(1))
            .expect_err("44.7 tiles is further than one belt window reaches");
        let msg = err.to_string();
        assert!(
            matches!(err, PlannerError::SustainNoRouteForFuel { .. }),
            "the refusal has to name the belt, not blame the cell: {msg}"
        );
        assert!(
            msg.contains(FUEL),
            "and it has to name what could not be carried: {msg}"
        );
    }
}
