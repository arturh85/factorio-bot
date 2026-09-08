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
//! * **Where the product ends up is a chest, and nothing empties that.** The
//!   offtake below moves the plates out of the furnace so the machine stops
//!   throttling on `full_output`; the chest it fills holds 32 stacks and is
//!   the new ceiling. That is far beyond a two-minute window at 15/min, but a
//!   *factory* would want the chest emptied too.
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
use crate::method::produce::{Cell, CellSpec, DRILL, FURNACE, cell_spec, cells_for};
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
/// So a cell's buffer is sited where the ground around it takes a belt: the
/// perimeter tiles, the cells beyond them, and the corners, because a route
/// leaving one side has to turn somewhere. Asked with a `transport-belt`'s own
/// footprint, which is what will actually stand there.
///
/// **9x9, raised from 7x7 when the offtake landed, and load-bearing.** The
/// three coal runs leave this chest one after another against a shared grid,
/// and each one's *route* -- not only its endpoint -- may hug the chest and eat
/// a side the next run needed. Adding an offtake near the furnace changes where
/// those routes bend, and at 7x7 the third run (the furnace's, the one that
/// matters most) refused with the chest's own four neighbours named. Reverting
/// this constant alone to 7 with everything else in place turns five of this
/// module's nine tests red, so it is not decoration.
///
/// It is still a *siting* heuristic and not a reservation: nothing stops a
/// later route from taking a side, which is why the number had to be measured
/// rather than reasoned. A real fix reserves the perimeter in
/// `method::connect`, and is not this rung.
///
/// It is a **siting** predicate and not a routing one: passing it does not
/// promise a route exists, only that the chest is not walled in before one is
/// looked for. `connect_steps` still refuses by name when the ground between
/// the two ends is blocked.
fn room_to_route(state: &PlanState, at: &Position) -> bool {
    const BELT: &str = "transport-belt";
    for dy in -4i32..=4 {
        for dx in -4i32..=4 {
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
///
/// # And an arm that stands is not an arm that delivers
///
/// The clause above is **structural**: it asks where an inserter stands and
/// which tile it drops on, and answers the same whether the belt behind it is
/// running or was never connected to anything. That is the wrong shape of
/// answer for a [`Goal::Sustain`], whose whole subject is what keeps turning
/// with no bot in the loop.
///
/// So the flow graph gets a veto, and only a veto. It is consulted through
/// [`flow_reaches`], which answers **`true` for anything it does not model** --
/// see that function for why the ignorance has to be read that way and what it
/// costs. The composition is deliberately one-directional: this predicate can
/// only ever become *stricter* than it was, never looser, so the worst a wrong
/// flow answer can do is make a replan build a feed that already exists, which
/// the belt primitive refuses by name. It cannot make this method skip a feed
/// it should have built.
fn fed_by_machine(state: &PlanState, at: &Position) -> bool {
    state
        .entities_within(at, FED_RADIUS)
        .into_iter()
        .any(|arm| {
            state.pickup_position(&arm).is_some()
                && state.delivers_into(&arm.position, at)
                && flow_reaches(state, &arm.position)
        })
}

/// Does anything actually arrive at the arm standing at `at`, as far as the
/// world's flow graph can tell?
///
/// The first question anything in this crate has ever asked
/// [`FlowGraph::throughput_at`], and the first time a flow-graph number has
/// affected a planning decision at all. Two things about the answer matter more
/// than the arithmetic.
///
/// # It is a question about the STANDING world, and cannot be otherwise
///
/// [`PlanState`] is an **overlay**: entities this expansion plans live in the
/// overlay and never reach `base().entity_graph`, which is what the flow graph
/// is built from. So an arm this plan is about to place is invisible here, and
/// asking about it would answer "nothing arrives" for the entirely wrong
/// reason.
///
/// That is why **an arm the flow graph has no node for reads as `true`**.
/// Ignorance is not evidence of a dead belt: the flow walk starts only from
/// offshore pumps and drills standing on ore, so an arm fed by hand, or one in
/// a corner of the world nothing has been walked from, has no node either. The
/// veto fires only where the graph positively models the arm **and** reports
/// nothing reaching it -- which is what a belt whose source has been mined out
/// or removed looks like.
///
/// # It cannot see back-pressure, and this predicate is not evidence of health
///
/// [`FlowGraph::throughput_at`] computes forwards from a source and models no
/// blocked sink and no buffer, so a full belt nobody unloads reports its rate
/// unchanged. A `true` here means "something is on its way", never "this arm is
/// working".
fn flow_reaches(state: &PlanState, at: &Position) -> bool {
    let flow = &state.base().flow_graph;
    if flow.node_at(at).is_none() {
        return true;
    }
    !flow.throughput_at(at).is_empty()
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
        .globals
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
    place_one(
        ctx,
        BUFFER,
        at,
        Direction::North,
        &format!("buffer the {FUEL} the belts carry"),
    )
}

/// One `Place`, in the shape every other placement in this crate takes, plus
/// the overlay update the next `AreaFree` has to see.
///
/// Field for field what `connect::place_step` emits — deliberately, because a
/// placement this method makes and one the belt primitive makes must be the
/// same kind of action to the executor. It is written out here rather than
/// borrowed because that function is private to `connect` and takes an entity
/// this module would have to build anyway.
fn place_one(
    ctx: &mut ExpansionCtx,
    name: &str,
    at: &Position,
    facing: Direction,
    note: &str,
) -> Option<Step> {
    let entity = sized(&ctx.state, name, at, facing)?;
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
    let min_radius = ctx.state.placement_clearance(name).unwrap_or(0.0);
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
                entity: name.into(),
                direction: entity.direction,
            },
            crate::action::Condition::HasItem {
                who: crate::action::Actor::Role,
                item: name.into(),
                count: 1,
            },
        ],
        eff: vec![
            crate::action::Effect::LoseItem {
                who: crate::action::Actor::Role,
                item: name.into(),
                count: 1,
            },
            crate::action::Effect::CreateEntity(Box::new(entity.clone())),
        ],
        duration: crate::method::have::PLACE_TICKS,
        pinned: None,
        label: format!("place {name} at {at} -- {note}"),
    }));
    ctx.state.create_entity(entity);
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

// ---------------------------------------------------------------------------
// The offtake
// ---------------------------------------------------------------------------

/// Where a cell's product goes, so its furnace's output slot never fills.
///
/// One arm on the furnace's perimeter and a container on the tile beyond it —
/// **not** a `connect_steps_with` run, and the reason is a tile budget rather
/// than a preference. A belt run costs *two* arms, and every arm on this
/// offtake has to be fed coal by a belt run of its own (see [`Offtake`]'s
/// module note below), so a belted offtake would need two more runs leaving
/// the cell's 1x1 coal buffer, which has four perimeter tiles and already
/// spends three. Two arms do not fit; one does.
#[derive(Debug, Clone)]
struct Offtake {
    /// The arm that lifts the product out of the machine.
    arm: Position,
    /// Its facing — which names the side it PICKS UP from, i.e. the machine.
    facing: Direction,
    /// The container it drops into.
    sink: Position,
}

/// The tile centres a machine standing at `at` facing `facing` covers.
///
/// The same rule `PlanState`'s own `tiles_under` uses (that one is private):
/// a tile counts when the footprint covers it, and a 2x2 entity at an integer
/// position covers the four half-integer centres around it.
fn machine_tiles(
    state: &PlanState,
    name: &str,
    at: &Position,
    facing: Direction,
) -> Option<Vec<Position>> {
    let area = state.collision_area_facing(name, at, facing)?;
    let mut tiles = Vec::new();
    let (x0, x1) = (
        area.left_top.x().floor() as i64,
        (area.right_bottom.x() - f64::EPSILON).floor() as i64,
    );
    let (y0, y1) = (
        area.left_top.y().floor() as i64,
        (area.right_bottom.y() - f64::EPSILON).floor() as i64,
    );
    for y in y0..=y1 {
        for x in x0..=x1 {
            tiles.push(Position::new(x as f64 + 0.5, y as f64 + 0.5));
        }
    }
    (!tiles.is_empty()).then_some(tiles)
}

/// Is something already taking the product out of the machine at `at` and
/// putting it somewhere with room?
///
/// Narrow in the same way [`fed_by_machine`] is, and asked of the state's own
/// predicates rather than of the arithmetic that sited them: an arm counts
/// only when the machine covers **its pickup tile** (`delivers_into`'s pull
/// branch) and its drop lands in a [`BUFFER`]. A coal arm delivering *into*
/// the furnace fails the first half, and an arm dropping onto bare ground
/// fails the second.
fn standing_offtake(state: &PlanState, at: &Position) -> Option<Offtake> {
    let mut found: Option<Offtake> = None;
    for arm in state.entities_within(at, FED_RADIUS) {
        if state.pickup_position(&arm).is_none() {
            continue;
        }
        if !state.delivers_into(at, &arm.position) {
            continue;
        }
        let Some(facing) = Direction::from_u8(arm.direction) else {
            continue;
        };
        let Some(drop) = state.delivery_position(&arm) else {
            continue;
        };
        let Some(sink) = state.entity_at(&drop) else {
            continue;
        };
        if sink.name != BUFFER || !state.delivers_into(&arm.position, &sink.position) {
            continue;
        }
        // `entities_within` is sorted by (x, y), so "the first" is a fixed
        // answer rather than an artefact of iteration order.
        if found.is_none() {
            found = Some(Offtake {
                arm: arm.position.clone(),
                facing,
                sink: sink.position.clone(),
            });
        }
    }
    found
}

/// Has the arm at `at` a side its own coal can be belted in through?
///
/// **Measured, by the first thing this rung tried.** Siting the offtake on the
/// first perimeter tile that took an arm and a chest put it in a corner with
/// its remaining sides walled off, and the coal run to it refused —
/// `no belt route, blocked by 4 tile(s)`, naming the arm at `[-35.5, 32.5]`.
/// The arm was placeable and unfeedable, which is the same failure the cell's
/// own buffer had (see [`room_to_route`]) one entity along: **a site that
/// takes the entity is not the same as a site that takes the entity's
/// supply.**
///
/// So the question is `connect_steps`' own: is there a direction where the
/// neighbouring tile takes an arm *and* the tile beyond it takes a belt? That
/// is exactly what `first_free_perimeter` will look for, asked here while
/// there are still other sides to try. Passing it does not promise a route
/// exists — `feed` still refuses by name when the ground further out is
/// blocked — only that the arm is not walled in before one is looked for.
fn room_to_fuel(state: &PlanState, at: &Position, taken: &[Position]) -> bool {
    const BELT: &str = "transport-belt";
    let spoken_for = |p: &Position| taken.iter().any(|t| Pos::from(t) == Pos::from(p));
    [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)]
        .into_iter()
        .any(|(dx, dy)| {
            let neighbour = Position::new(at.x() + dx, at.y() + dy);
            let beyond = Position::new(at.x() + 2. * dx, at.y() + 2. * dy);
            // `taken` is the arrangement this call is *about* to place -- the
            // machine's own tiles and the container -- and none of it is in
            // the state yet, so `is_area_free` says those tiles are free.
            // **Reading that as room was the first version's defect**: it
            // accepted a site whose only fuel-side was the tile its own chest
            // would then stand on, and `connect` refused the coal run at the
            // arm with all four neighbours named.
            if spoken_for(&neighbour) || spoken_for(&beyond) {
                return false;
            }
            state.is_area_free(BELT, &neighbour) && state.is_area_free(BELT, &beyond)
        })
}

/// Site an arm on the machine's perimeter with a container on the tile beyond
/// it.
///
/// Three collinear tile centres — a footprint tile the arm reaches into, the
/// arm's own tile, the container's — scanned North, East, South, West and in
/// footprint order along each side, so the same layout always yields the same
/// site. The facing comes from [`crate::method::connect::inserter_facing`],
/// which is the one place new planner code is supposed to derive it, and the
/// two deliveries are then asked of a **fork with all three standing**, so
/// this is `delivers_into`'s answer and not a restatement of the offset table
/// that produced it.
fn plan_offtake(
    state: &PlanState,
    machine: &str,
    at: &Position,
    facing: Direction,
) -> Result<Offtake, String> {
    let Some(tiles) = machine_tiles(state, machine, at, facing) else {
        return Err(format!("{machine} is not a prototype this world describes"));
    };
    let covers = |p: &Position| tiles.iter().any(|t| Pos::from(t) == Pos::from(p));
    // Every tile that was tried and the word for why it was not taken.
    // **A refusal that does not name the ground sends the next reader
    // guessing**, which is exactly what the first live refusal of this
    // method did -- `no tile on its perimeter takes an arm` is true and says
    // nothing about which of four checks did it.
    let mut rejected: Vec<String> = Vec::new();
    for (dx, dy) in [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)] {
        for anchor in &tiles {
            let arm = Position::new(anchor.x() + dx, anchor.y() + dy);
            let sink = Position::new(anchor.x() + 2. * dx, anchor.y() + 2. * dy);
            if covers(&arm) || covers(&sink) {
                continue;
            }
            let mut note = |why: &str| rejected.push(format!("{arm}->{sink} {why}"));
            if !state.is_area_free(ARM, &arm) {
                note("arm tile occupied");
                continue;
            }
            if !state.is_area_free(BUFFER, &sink) {
                note("container tile occupied");
                continue;
            }
            // Neither may stand on ore, for `produce::fit`'s reason: ground a
            // drill wants is ground this plan must not build on, and a plan
            // that eats its own patch is not recoverable.
            let (Some(arm_area), Some(sink_area)) = (
                state.collision_area(ARM, &arm),
                state.collision_area(BUFFER, &sink),
            ) else {
                note("no collision box");
                continue;
            };
            if state.covers_any_resource(&arm_area) {
                note("arm tile is ore");
                continue;
            }
            if state.covers_any_resource(&sink_area) {
                note("container tile is ore");
                continue;
            }
            let mut taken: Vec<Position> = tiles.clone();
            taken.push(sink.clone());
            if !room_to_fuel(state, &arm, &taken) {
                note("no side left to belt the arm its coal");
                continue;
            }
            let Some(arm_facing) = crate::method::connect::inserter_facing(anchor, &sink) else {
                note("not cardinal");
                continue;
            };
            let (Some(arm_entity), Some(sink_entity)) = (
                sized(state, ARM, &arm, arm_facing),
                sized(state, BUFFER, &sink, Direction::North),
            ) else {
                note("no prototype");
                continue;
            };
            let mut trial = state.fork();
            trial.create_entity(arm_entity);
            trial.create_entity(sink_entity);
            if !trial.delivers_into(at, &arm) {
                note("the machine does not reach the arm");
                continue;
            }
            if !trial.delivers_into(&arm, &sink) {
                note("the arm does not reach the container");
                continue;
            }
            return Ok(Offtake {
                arm,
                facing: arm_facing,
                sink,
            });
        }
    }
    Err(format!(
        "no tile on its perimeter takes an arm with a container beyond it -- tried {}: {}",
        rejected.len(),
        rejected.join("; ")
    ))
}

/// What the world's flow graph says is arriving at each standing cell's
/// furnace, in items per minute, as one sentence for a refusal to carry.
///
/// # Why a number appears in an error string rather than in a decision
///
/// Because this is the only place in this method where it can be honest.
/// Everything above is being *planned*, and a planned entity lives in
/// [`PlanState`]'s overlay where the flow graph cannot see it (see
/// [`flow_reaches`]). Here, and only here, every entity the arrangement
/// consists of is known to stand -- that is what `steps.is_empty()` means -- so
/// the flow graph is being asked about exactly the world it was built from.
///
/// It is deliberately **not** turned into a verdict. A `Sustain` goal is a
/// claim about a window of history, this crate has no clock to read one with,
/// and [`FlowGraph::throughput_at`] models neither back-pressure nor buffers --
/// so a modelled rate is an upper bound under ideal distribution and cannot
/// settle whether the arrangement sustains anything. Reporting it beside the
/// refusal is what lets a human compare the model against the run, which is the
/// only way the model gets validated at all.
fn modelled_delivery(state: &PlanState, cells: &[Cell], ore: &str) -> String {
    let flow = &state.base().flow_graph;
    let mut per_minute = 0.;
    let mut modelled = 0usize;
    for cell in cells {
        if flow.node_at(&cell.furnace).is_none() {
            continue;
        }
        modelled += 1;
        per_minute += flow
            .throughput_at(&cell.furnace)
            .into_iter()
            .filter(|(item, _)| item == ore)
            .map(|(_, rate)| rate * 60.)
            .sum::<f64>();
    }
    if modelled == 0 {
        return format!(
            "the flow graph models none of the {} furnace(s), so it has nothing to say about \
             what reaches them",
            cells.len()
        );
    }
    format!(
        "the flow graph models {modelled} of {} furnace(s) and puts {per_minute:.1} {ore}/min \
         into them -- an upper bound under ideal distribution, blind to back-pressure and to \
         buffers, and no evidence that anything moved",
        cells.len()
    )
}

/// The belt nearest `at` among the placements in `steps`.
///
/// Deterministic without a float tie-break reaching the ordering: distances are
/// compared with `total_cmp` and equal ones fall back to the position, which is
/// the same rule the buffer search two functions along uses.
fn nearest_belt_of(steps: &[Step], at: &Position) -> Option<FactorioEntity> {
    const BELT: &str = "transport-belt";
    let mut best: Option<(f64, FactorioEntity)> = None;
    for step in steps {
        let Step::Act(action) = step else { continue };
        let crate::action::ActionKind::Place { entity } = &action.kind else {
            continue;
        };
        if entity.name != BELT {
            continue;
        }
        let d = (entity.position.x() - at.x()).hypot(entity.position.y() - at.y());
        let better = match &best {
            None => true,
            Some((bd, be)) => match d.total_cmp(bd) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Equal => Pos::from(&entity.position) < Pos::from(&be.position),
                std::cmp::Ordering::Greater => false,
            },
        };
        if better {
            best = Some((d, (**entity).clone()));
        }
    }
    best.map(|(_, entity)| entity)
}

/// Site exactly one more cell against the world as it stands *now*, and push
/// its placements onto `steps`.
///
/// The whole point is the "now": [`ExpansionCtx::state`] is an overlay that
/// every `place_one` and every `connect_steps_with` above has already written
/// into, so a cell sited through this helper routes around the belts its
/// predecessors laid. `plan_cells` called once for `n` cells cannot do that --
/// it forks the state before any belt exists.
///
/// One cell per call and not `n`, because the belts of cell `k` only exist
/// after cell `k` has been *belted*, which happens in the caller's loop body
/// and not here.
fn site_one_cell(
    ctx: &mut ExpansionCtx,
    spec: &CellSpec,
    from: &Position,
    steps: &mut Vec<Step>,
) -> Result<Vec<Cell>, PlannerError> {
    let fresh = crate::method::produce::plan_cells(
        &ctx.state,
        from,
        spec,
        1,
        crate::method::produce::rate_cell_ore(spec),
    )?;
    steps.extend(crate::method::produce::cell_steps_fuelled(
        ctx,
        spec,
        &fresh,
        IGNITION_TICKS,
    ));
    Ok(fresh)
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
                via: None,
            }));
            steps.push(Step::Subgoal(Goal::Have {
                item: BUFFER.into(),
                count: 1,
                whose: Holder::Share(ctx.chain_actor),
                via: None,
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
        // # One cell at a time, and the rest sited AFTER their predecessors'
        // belts exist
        //
        // `plan_cells` sites every cell against a fork holding the previous
        // cells' **parts** -- a drill and a furnace each -- and nothing else.
        // The belts are not in that fork because this method has not laid them
        // yet, so cell 2 was packed as tightly against cell 1 as two drills
        // allow and then had to fit ~65 belts of coal run through the gap.
        //
        // Measured offline against seed 31337, with the two defects above
        // fixed: cell 2's offtake arm at `[-7.5, -32.5]` refused its coal
        // branch with `no belt route, blocked by 10 tile(s)`, and every one of
        // the ten was a belt **cell 1 had just laid**. Siting was blind to the
        // only thing that was ever going to be in the way.
        //
        // So the first cell is sited here, where nothing exists to route
        // around, and each later one is sited at the bottom of the loop --
        // against a `ctx.state` that holds every entity its predecessors put
        // in the overlay, belts included. That ordering also keeps a
        // single-cell plan byte-identical to what this method produced before,
        // which is the control that says the change is about the second cell
        // and not about the first.
        let mut to_build = needed.saturating_sub(cells.len() as u32);
        if to_build > 0 {
            cells.extend(site_one_cell(ctx, &spec, &source.buffer, &mut steps)?);
            to_build -= 1;
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
        // # Every cell's PLATE chest, not only this one's
        //
        // The local-buffer search below takes "the nearest [`BUFFER`] that is
        // not spoken for", and an offtake sink **is** a [`BUFFER`] — the same
        // `iron-chest`, one tile off a furnace's face. Excluding only the
        // cell's own sink is what the first version did, and it is enough for
        // exactly one cell. With two, **cell 2 adopts cell 1's plate chest as
        // its coal buffer**: measured offline against seed 31337, cell 2's
        // furnace at `[-7, -31]` chose `local = [-5.5, -29.5]`, which is
        // cell 1's `hold the iron-plate the cell makes`.
        //
        // Two failures follow from that one adoption, and the *second* is what
        // the refusal named:
        //
        // 1. coal would be belted into the chest the plates come out of, and
        // 2. `fed_by_machine` reads that chest as already fed — cell 1's
        //    offtake arm delivers into it — so no coal run is laid at all, the
        //    tap search over this cell's own (empty) slice of `steps` finds no
        //    belt, and the arrangement refuses with
        //    `the cell's coal runs laid no belt to branch the offtake arm's
        //    own fuel off`.
        //
        // That refusal is true and points at the wrong entity: the arm was
        // fine, the chest three tiles away was not. So the exclusion is
        // accumulated across cells rather than reset per cell, and it is
        // seeded with the sinks that already **stand**, which is the replan
        // half of the same fact.
        let mut plate_chests: Vec<Position> = cells
            .iter()
            .filter_map(|cell| standing_offtake(&ctx.state, &cell.furnace))
            .map(|offtake| offtake.sink)
            .collect();
        let mut index = 0usize;
        while index < cells.len() {
            let cell = cells[index].clone();
            let cell = &cell;
            // # The offtake is sited and placed FIRST, and both halves of that
            // were measured rather than chosen
            //
            // **First, because the furnace's perimeter is the scarce thing.**
            // Sited last -- after the three coal runs had wrapped around the
            // cell -- the offline plan against seed 31337 refused with all
            // eight of the furnace's perimeter tiles reported `occupied`, by
            // this plan's own belts and arms. The offtake needs two collinear
            // tiles off one face; a coal run needs any one face plus a route,
            // and has six left after the offtake has taken its two.
            //
            // **And its chest has to be excluded from the coal buffer's search
            // by position, not merely when it was already standing.** The
            // offtake's sink and the cell's coal buffer are both a [`BUFFER`],
            // and the search below takes "the nearest chest that is not the
            // source's" -- which, one tile off the furnace's face, is the plate
            // chest this block has just put in the overlay. The first version
            // of the exclusion only skipped a chest found *standing*, which is
            // the replan case; on a fresh plan the cell then adopted its own
            // plate chest as its coal buffer and belted coal into it, and the
            // run to the furnace refused with that chest's four neighbours
            // named. Same shape as the source-buffer exclusion further down,
            // and found the same way: by asking what the next step sees.
            let standing = standing_offtake(&ctx.state, &cell.furnace);

            // # The offtake, and why it is a rung of its own
            //
            // **Measured, in `run-1788679826-02267`.** The cell above ran for
            // 27,249 ticks with no bot in the loop, at 100% of nominal tick
            // rate, and still came back `SHORT`: its furnace read
            // `working 80, no_ingredients 19, no_fuel 15, full_output 15` and
            // its iron-plate production decayed 166 -> 72 -> 8 across the run
            // while coal and ore held flat at ~16/min. Nothing took the plates
            // away, so the output slot filled and the machine throttled itself.
            // The arrangement sustained a *window*, not a rate.
            let offtake = match standing {
                Some(existing) => existing,
                None => {
                    let planned =
                        plan_offtake(&ctx.state, FURNACE, &cell.furnace, Direction::North)
                            .map_err(|why| PlannerError::SustainNoOfftake {
                                item: spec.item.clone(),
                                machine: FURNACE.into(),
                                at: cell.furnace.to_string(),
                                why,
                            })?;
                    steps.push(Step::Subgoal(Goal::Have {
                        item: ARM.into(),
                        count: 1,
                        whose: Holder::Share(ctx.chain_actor),
                        via: None,
                    }));
                    steps.push(Step::Subgoal(Goal::Have {
                        item: BUFFER.into(),
                        count: 1,
                        whose: Holder::Share(ctx.chain_actor),
                        via: None,
                    }));
                    let note = format!("take {} out of the {FURNACE}", spec.item);
                    if let Some(step) = place_one(ctx, ARM, &planned.arm, planned.facing, &note) {
                        steps.push(step);
                    }
                    let note = format!("hold the {} the cell makes", spec.item);
                    if let Some(step) =
                        place_one(ctx, BUFFER, &planned.sink, Direction::North, &note)
                    {
                        steps.push(step);
                    }
                    planned
                }
            };
            // Standing or planned, this cell's sink joins the set the next
            // cell's coal buffer must not be.
            if !plate_chests
                .iter()
                .any(|sink| Pos::from(sink) == Pos::from(&offtake.sink))
            {
                plate_chests.push(offtake.sink.clone());
            }

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
                .filter(|e| {
                    e.name == BUFFER
                        && Pos::from(&e.position) != Pos::from(&source.buffer)
                        && !plate_chests
                            .iter()
                            .any(|sink| Pos::from(sink) == Pos::from(&e.position))
                })
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
                        via: None,
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
            // # And the offtake arm needs coal, which it cannot get for itself
            //
            // The property the rest of this arrangement rests on -- **a burner
            // inserter takes its own fuel out of the coal it is moving** -- is
            // a property of the *cargo*, not of the arm. Every other arm here
            // is on a coal run and so refuels itself; this one moves iron
            // plates and can never take a plate as fuel. Left with only an
            // ignition charge it would stop as soon as that coal burned
            // through, and the cell would go quietly back to filling its
            // output slot -- the failure this whole block exists to remove,
            // returning by the back door.
            //
            // It needs no ignition charge, because **an arm being filled does
            // not have to swing to receive**: `run-1788679826-02267` placed
            // eight arms with no charge at all and they all started.
            //
            // ## Why the coal is branched off a BELT and not run from the chest
            //
            // Measured, offline, against seed 31337 and against the crate's own
            // fixture. A fourth run leaving the cell's coal buffer is what this
            // first tried, and **a 1x1 chest cannot carry four**: it has four
            // perimeter tiles, each run claims one plus the cell beyond it, and
            // the belts of the earlier runs curl around the rest. Three runs
            // (haul in, drill, furnace) is the proven budget -- the fourth
            // refused at the *furnace*, with the chest's own four neighbours
            // named, so the cost fell on the run that matters most.
            //
            // A branch off a belt costs no chest perimeter at all. An inserter
            // takes items off a belt exactly as it takes them out of a chest,
            // and the arm doing it is on coal, so it self-fuels like every
            // other arm here.
            //
            // **Every belt this method places carries coal**, which is what
            // makes "the nearest belt" a safe source. That is a property of
            // this method and not of the map: a belt somebody else built inside
            // the cell would be picked up by the same search, and nothing here
            // could tell.
            //
            // ## And the branch is laid HERE, before the drill and furnace runs
            //
            // Measured both ways against seed 31337's dump, which is the only
            // reason this sits between the haul and the two cell runs rather
            // than at the end where it reads better. Laid last, the arm's
            // remaining sides have been taken by the drill's and the furnace's
            // belts and the branch refuses -- on the real map, with the tap belt
            // diagonal to the arm and all four of its neighbours named. Laid
            // here, only the haul's belts exist, the arm still has two open
            // sides, and the two cell runs route around what this leaves. The
            // fixture accepts either order; the real map accepts only this one,
            // which is the direction that decides.
            let arm_entity =
                sized(&ctx.state, ARM, &offtake.arm, offtake.facing).ok_or_else(|| {
                    PlannerError::SustainNoOfftake {
                        item: spec.item.clone(),
                        machine: ARM.into(),
                        at: offtake.arm.to_string(),
                        why: format!("{ARM} is not a prototype in this world"),
                    }
                })?;
            if !fed_by_machine(&ctx.state, &offtake.arm) {
                // # Every coal belt this plan knows about, not only this
                // cell's own slice
                //
                // `coal_runs_from` scoped the search to the runs *this* cell
                // laid, which is right for the first cell and wrong for every
                // one after it. Cells 2..n share the first cell's coal buffer
                // -- correctly, that is what stops a second haul from the
                // source -- so `feed` returns no steps and the slice is
                // **empty**. Measured offline against seed 31337: with the
                // plate-chest exclusion above in place, cell 2 chose
                // `local = [0.5, -31.5]`, cell 1's coal chest, read it as
                // already fed, laid nothing, and refused with
                // `the cell's coal runs laid no belt to branch the offtake
                // arm's own fuel off`. The belts existed; this call could not
                // see them.
                //
                // Widening the *source* set cannot change the ordering the
                // comment below is about -- the branch is still laid here,
                // before the drill and furnace runs -- and it only ever adds
                // candidates to a nearest-wins search. Every belt this method
                // places carries coal, which is what makes any of them a legal
                // tap; that caveat is unchanged and is stated below.
                let tap = nearest_belt_of(&steps, &offtake.arm).ok_or_else(|| {
                    PlannerError::SustainNoOfftake {
                        item: spec.item.clone(),
                        machine: ARM.into(),
                        at: offtake.arm.to_string(),
                        why: "the cell's coal runs laid no belt to branch the offtake arm's own \
                              fuel off"
                            .into(),
                    }
                })?;
                steps.extend(feed(ctx, &tap, &arm_entity)?);
            }
            if let Some(drill) = sized(&ctx.state, DRILL, &cell.drill, cell.facing) {
                steps.extend(feed(ctx, &local_entity, &drill)?);
            }
            if let Some(furnace) = sized(&ctx.state, FURNACE, &cell.furnace, Direction::North) {
                steps.extend(feed(ctx, &local_entity, &furnace)?);
            }
            index += 1;
            // The next cell, sited now that this one's belts stand in the
            // overlay. See the note above the first cell for why the siting
            // is staggered rather than done in one call.
            if index == cells.len() && to_build > 0 {
                cells.extend(site_one_cell(ctx, &spec, &source.buffer, &mut steps)?);
                to_build -= 1;
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
                     belted deliverer for every burner -- so there is nothing left to build. \
                     {}",
                    spec.ore,
                    modelled_delivery(&ctx.state, &cells, &spec.ore)
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
    use factorio_bot_core::factorio::world::FactorioSurface;
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
    /// exercised at all. This adds a second coal patch north of the iron.
    ///
    /// **Its distance was wrong and the offtake is what exposed it.** The patch
    /// sat at `(-40, 26)`: 14 tiles centre to centre, but both patches are 8x8,
    /// so their closest tiles were **6 tiles apart** against the 19.6 that seed
    /// 31337 really has (measured from `workspace/scripts/map.json`). That is
    /// three times tighter than the map it stands for, and the whole cell --
    /// drill, furnace, coal buffer, three belt runs and now an offtake -- had
    /// to fit in the gap. Every belt run refused there while the same plan
    /// succeeded on the real map, which is the wrong way round for a fixture:
    /// it was refusing arrangements the target map accepts.
    ///
    /// `(-40, 18)` puts the closest tiles **14 apart**, still tighter than
    /// 31337 and so still the harder case, but no longer harder than anything
    /// this method is meant to build on. **Said plainly because it is the trap
    /// this file already warns about**: the fixture was changed while the code
    /// under it was failing, and a fixture moved until the code passes proves
    /// nothing. What justifies it is a number measured off the real map and
    /// independent of this change -- and the real-map plan, which succeeded at
    /// both distances.
    ///
    /// It inherits `spawn_ore`'s known distortion: ore is spawned at **integer**
    /// positions, and a real resource entity sits at a tile centre. That is the
    /// one input for which `EntityGraph`'s flooring round-trip is lossless, so
    /// a geometry defect that depends on the half-tile offset would not show
    /// here. The offline plan against `map.json` is the check that does see it.
    fn world_with_coal_beside_the_iron() -> FactorioSurface {
        let world = fixture_world();
        let mut entities = Vec::new();
        spawn_ore(
            &mut entities,
            add_to_rect(&Rect::from_wh(8., 8.), &Position::new(-40., 18.)),
            FUEL,
        );
        world.update_chunk_entities(entities).unwrap();
        world
    }

    fn near_state() -> PlanState {
        PlanState::from_world(Arc::new(world_with_coal_beside_the_iron()), &[BotId(1)])
    }

    /// The ignorance rule, asserted on its own because everything else in this
    /// module depends on it and it is the half that could quietly break the
    /// method.
    ///
    /// A planner's world is an **overlay**: every entity this expansion is
    /// about to place lives there and never reaches the flow graph, which is
    /// built from `base().entity_graph`. If ignorance read as "nothing arrives"
    /// the veto would fire on every arm this method plans, and the arrangement
    /// would never converge.
    ///
    /// The falsification for this is the interesting one: flipping the `true`
    /// to `false` turns the whole module red, which is what says the flow graph
    /// is genuinely wired into `fed_by_machine` rather than merely referenced
    /// there.
    #[test]
    fn an_arm_the_flow_graph_knows_nothing_about_is_not_vetoed() {
        let state = near_state();
        assert!(
            state
                .base()
                .flow_graph
                .node_at(&Position::new(0., 0.))
                .is_none(),
            "the fixture world has no source root, so its flow graph is empty"
        );
        assert!(
            flow_reaches(&state, &Position::new(0., 0.)),
            "a position the flow graph has never heard of must not be read as a dead belt"
        );
    }

    /// The other half of the veto: a modelled arm that nothing reaches.
    ///
    /// A furnace fed **only fuel** smelts nothing, so the arm on its output is
    /// a node the flow graph positively knows about and reports an empty rate
    /// for. That is the one condition `flow_reaches` refuses on, and without a
    /// fixture holding it the refusal branch is never executed by any test in
    /// this crate -- which is exactly what removing the call from
    /// `fed_by_machine` demonstrated: the whole module stayed green.
    ///
    /// **This task wrote both the code and this fixture.** What it assumes: a
    /// stone furnace whose only input is coal produces nothing (true of the
    /// game, and of the recipe table, where coal is an ingredient of no
    /// smelting recipe), and `EntityGraph::connect` links drill -> belt ->
    /// inserter -> furnace -> inserter by drop and pickup positions. The
    /// preconditions are asserted below rather than assumed, so the test fails
    /// loudly instead of vacuously if either stops holding.
    #[test]
    fn an_arm_on_a_furnace_that_only_gets_fuel_is_refused() {
        use factorio_bot_core::types::{Direction as Dir, FactorioEntity as E};
        let world = fixture_world();
        let mut entities = Vec::new();
        spawn_ore(
            &mut entities,
            add_to_rect(&Rect::from_wh(4., 4.), &Position::new(0., -2.)),
            FUEL,
        );
        entities.extend(vec![
            E::new_electric_mining_drill(&Position::new(0.5, -1.5), Dir::South),
            E::new_transport_belt(&Position::new(0.5, 0.5), Dir::South),
            E::new_inserter(&Position::new(0.5, 1.5), Dir::North),
            E::new_stone_furnace(&Position::new(1., 3.), Dir::South),
            E::new_inserter(&Position::new(0.5, 4.5), Dir::North),
        ]);
        world.update_chunk_entities(entities).unwrap();
        // Explicitly, because `update_chunk_entities` adds NODES and does not
        // connect them -- `EntityGraph::connect` has exactly two callers in the
        // whole workspace, `OutputParser::on_init` and `factorio::snapshot`,
        // and both are one-shot at world initialisation. Without this the flow
        // graph is empty and the preconditions below fail rather than the
        // assertion, which is the point of asserting them.
        world.entity_graph.connect().unwrap();
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        let furnace = Position::new(1., 3.);
        let arm = Position::new(0.5, 4.5);
        assert!(
            !state.base().flow_graph.throughput_at(&furnace).is_empty(),
            "precondition: the chain is connected and coal reaches the furnace"
        );
        assert!(
            state.base().flow_graph.node_at(&arm).is_some(),
            "precondition: the output arm is a node the flow graph models"
        );

        assert!(
            !flow_reaches(&state, &arm),
            "a furnace with fuel and no ore smelts nothing, so nothing reaches the arm \
             taking from it -- and this is the case the veto exists for"
        );
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

    /// The world the plan leaves behind: everything it places, standing.
    fn built_world(net: &crate::network::ActionNetwork, state: &PlanState) -> (PlanState, usize) {
        let mut built = state.fork();
        let mut placed = 0usize;
        for action in net.actions() {
            if let crate::action::ActionKind::Place { entity } = &action.kind {
                built.create_entity((**entity).clone());
                placed += 1;
            }
        }
        (built, placed)
    }

    /// **THE RUNG.** Something takes the plates out of the furnace.
    ///
    /// `run-1788679826-02267` is the measurement this exists for: a belted
    /// cell that ran 27,249 ticks with no bot in the loop, at 100% of nominal
    /// tick rate, and still came back `SHORT` because its furnace read
    /// `full_output` in 15 of 125 samples and its plate production decayed
    /// 166 -> 72 -> 8 while coal and ore held flat.
    ///
    /// Asked of `standing_offtake`, which is `delivers_into`'s answer in both
    /// directions rather than a restatement of the offsets that sited it.
    #[test]
    fn the_furnaces_output_is_taken_into_a_chest() {
        let roster = [BotId(1)];
        let state = near_state();
        let net = expand(&[goal()], &state, &registry_for(&roster), BotId(1))
            .expect("iron and coal are within one belt window of each other");
        let (built, placed) = built_world(&net, &state);
        assert!(
            placed > 30,
            "the expansion has to have built the arrangement: {placed} placements"
        );

        let spec = cell_spec(&built, "iron-plate").expect("a stone furnace smelts iron");
        let cells = crate::method::produce::standing_cells(&built, &spec);
        assert!(
            !cells.is_empty(),
            "no cell stands, so this test measures nothing"
        );

        for cell in &cells {
            let offtake = standing_offtake(&built, &cell.furnace).unwrap_or_else(|| {
                panic!(
                    "the {FURNACE} at {} has nothing taking its {} away, which is the \
                     `full_output` throttle this rung exists to remove",
                    cell.furnace, spec.item
                )
            });
            assert!(
                built.delivers_into(&cell.furnace, &offtake.arm),
                "the arm at {} does not pick up from the furnace at {}",
                offtake.arm,
                cell.furnace
            );
            assert!(
                built.delivers_into(&offtake.arm, &offtake.sink),
                "the arm at {} does not drop into the chest at {}",
                offtake.arm,
                offtake.sink
            );
            assert_eq!(
                built.entity_at(&offtake.sink).map(|e| e.name),
                Some(BUFFER.to_string()),
                "the plates have to land somewhere with capacity"
            );

            // FALSIFICATION, with the substitution asserted to have bitten.
            // Turn the offtake arm around -- it then picks up from the chest
            // and drops into the furnace, which is a layout that places
            // perfectly and moves nothing. `standing_offtake` must stop
            // recognising it. A green here would mean the predicate answers
            // `true` for any arm near a furnace, which would make everything
            // above worthless.
            let arm = built.entity_at(&offtake.arm).expect("the arm stands");
            let facing = Direction::from_u8(arm.direction).expect("a cardinal facing");
            let flipped = opposite(facing);
            assert_ne!(
                flipped, facing,
                "the substitution has to actually turn the arm, or it proves nothing"
            );
            let mut reversed = built.fork();
            reversed.create_entity(
                sized(&built, ARM, &offtake.arm, flipped).expect("the arm is a prototype"),
            );
            assert_ne!(
                reversed.entity_at(&offtake.arm).map(|e| e.direction),
                Some(arm.direction),
                "and the substitution has to have reached the state"
            );
            assert!(
                standing_offtake(&reversed, &cell.furnace).is_none(),
                "an arm pointing the other way feeds the furnace from the chest; it is not \
                 an offtake, and reading it as one is this project's defining failure"
            );
        }
    }

    /// **An arm carrying plates can never fuel itself, so the offtake is
    /// belted like every other burner here.**
    ///
    /// The property the rest of the arrangement rests on -- a burner inserter
    /// takes its own fuel out of the coal it is moving -- is a property of the
    /// cargo. This one moves iron plates. Left with an ignition charge it
    /// would stop when that coal burned through and the cell would go quietly
    /// back to filling its output slot.
    ///
    /// So: a machine delivers into it, and no bot's hands do.
    #[test]
    fn the_offtake_arm_is_belted_its_own_coal() {
        let roster = [BotId(1)];
        let state = near_state();
        let net = expand(&[goal()], &state, &registry_for(&roster), BotId(1))
            .expect("iron and coal are within one belt window of each other");
        let (built, _) = built_world(&net, &state);
        let spec = cell_spec(&built, "iron-plate").expect("a stone furnace smelts iron");
        let cells = crate::method::produce::standing_cells(&built, &spec);
        let cell = cells.first().expect("a cell stands");
        let offtake = standing_offtake(&built, &cell.furnace).expect("the furnace has an offtake");

        assert!(
            fed_by_machine(&built, &offtake.arm),
            "nothing delivers coal into the offtake arm at {}, so it stops as soon as any \
             charge it was given burns through",
            offtake.arm
        );
        let by_hand = net
            .actions()
            .filter(|action| match &action.kind {
                crate::action::ActionKind::Insert { pos, item, .. } => {
                    item.as_str() == FUEL && Pos::from(pos) == Pos::from(&offtake.arm)
                }
                _ => false,
            })
            .count();
        assert_eq!(
            by_hand, 0,
            "the offtake arm is hand-charged {by_hand} time(s); a burner a bot has to revisit \
             is the thing a standing goal exists to get away from"
        );

        // FALSIFICATION, with the substitution asserted to have bitten: build
        // the same world WITHOUT the arms that deliver into the offtake arm,
        // and `fed_by_machine` must go false. A green would mean the predicate
        // is satisfied by the offtake arm's own presence.
        let mut starved = state.fork();
        let mut dropped = 0usize;
        for action in net.actions() {
            if let crate::action::ActionKind::Place { entity } = &action.kind {
                let feeds_it = entity.name == ARM
                    && Pos::from(&entity.position) != Pos::from(&offtake.arm)
                    && built
                        .delivery_position(entity)
                        .is_some_and(|drop| Pos::from(&drop) == Pos::from(&offtake.arm));
                if feeds_it {
                    dropped += 1;
                    continue;
                }
                starved.create_entity((**entity).clone());
            }
        }
        assert!(
            dropped > 0,
            "the substitution matched nothing -- no placed arm delivers into the offtake arm, \
             so this experiment is broken and its result says nothing about the test"
        );
        assert!(
            !fed_by_machine(&starved, &offtake.arm),
            "with its {dropped} deliverer(s) removed the arm still reads as fed, so \
             `fed_by_machine` is not measuring what this test claims"
        );
    }

    /// A site that takes the arrangement is not a site that takes its supply.
    ///
    /// **The defect this pins was live for one test run.** `room_to_fuel`
    /// asked `is_area_free` about the arm's four sides -- but the offtake's
    /// own chest is not in the state yet when it is asked, so the tile the
    /// chest was about to occupy read as free. The site was accepted on that
    /// side alone, and `connect` then refused the coal run at the arm with all
    /// four of its neighbours named:
    /// `no belt route, blocked by 4 tile(s): [-36.5, 32.5] [-35.5, 31.5]
    /// [-35.5, 34.5] [-34.5, 32.5]`.
    #[test]
    fn a_side_the_offtake_itself_will_occupy_is_not_room_for_its_coal() {
        let state = near_state();
        // Open ground well clear of both patches and of anything the fixture
        // spawns -- so the only thing that can make the answer `false` is the
        // `taken` list this test is about.
        let arm = Position::new(0.5, 0.5);
        assert!(
            room_to_fuel(&state, &arm, &[]),
            "open ground has four free sides; if this fails the fixture moved and the \
             substitution below would prove nothing"
        );
        let sides: Vec<Position> = [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)]
            .into_iter()
            .map(|(dx, dy)| Position::new(arm.x() + dx, arm.y() + dy))
            .collect();
        assert_eq!(sides.len(), 4, "an arm has four sides");
        assert!(
            !room_to_fuel(&state, &arm, &sides),
            "a side the arrangement has already spoken for is not room, however free the \
             ground under it still reads"
        );
        // And one side spoken for is not four: the predicate must not be a
        // constant `false` once anything is passed.
        assert!(
            room_to_fuel(&state, &arm, &sides[..3]),
            "three sides taken still leaves one, and refusing there would refuse every \
             real site"
        );
    }

    /// `Direction`'s half-turn, written out because the type carries no
    /// `opposite()` this crate can call.
    /// A goal wanting two cells refuses about the GROUND, not about a belt
    /// that is standing right there.
    ///
    /// # The defect, and why the message was the whole of it
    ///
    /// A second cell used to refuse with
    ///
    /// ```text
    /// the burner-inserter at [-7.5, -32.5] makes iron-plate and nothing
    /// within reach can take it away: the cell's coal runs laid no belt to
    /// branch the offtake arm's own fuel off
    /// ```
    ///
    /// which reads as the fuel-physics ceiling this whole arrangement is
    /// supposed to have -- *an arm that moves plates cannot fuel itself* -- and
    /// is not one. Measured offline against seed 31337 by printing what the
    /// second cell actually chose:
    ///
    /// ```text
    /// furnace=[-5, -27] arm=[-5.5, -28.5] local=[0.5, -31.5]  local_fed=false
    /// furnace=[-7, -31] arm=[-7.5, -32.5] local=[-5.5, -29.5] local_fed=true
    /// ```
    ///
    /// `[-5.5, -29.5]` is cell 1's **plate** chest -- `hold the iron-plate the
    /// cell makes`, one tile off its furnace's face and an [`BUFFER`] like any
    /// other. Cell 2 adopted it as its coal buffer, read it as already fed
    /// (cell 1's offtake arm delivers into it), laid no coal run at all, and
    /// then had no belt to tap. Two failures, and the refusal named the
    /// second.
    ///
    /// So this test asserts the *kind* of refusal. Once the exclusion is
    /// cumulative the second cell shares cell 1's **coal** chest, which is
    /// correct, and fails where a 1x1 chest has to fail -- on its fourth
    /// perimeter tile, a fact about ground that
    /// [`PlannerError::SustainNoRouteForFuel`] carries with the tiles named.
    /// `SustainNoOfftake` here means the plate chest has been adopted again.
    #[test]
    fn a_second_cell_refuses_about_ground_and_not_about_a_belt_that_stands() {
        let roster = [BotId(1)];
        let state = near_state();
        let two_cells = Goal::Sustain {
            item: "iron-plate".into(),
            per_minute: 30,
            window_ticks: 7200,
        };
        let err = expand(&[two_cells], &state, &registry_for(&roster), BotId(1))
            .expect_err("two burner cells do not fit around one 1x1 coal chest");
        assert!(
            matches!(err, PlannerError::SustainNoRouteForFuel { .. }),
            "the second cell refused with {err:?}; a `SustainNoOfftake` here means it adopted \
             an earlier cell's plate chest as its coal buffer, found that chest already fed, and \
             laid no coal run to tap -- the refusal then names the arm and the defect is three \
             tiles away"
        );
    }

    fn opposite(d: Direction) -> Direction {
        match d {
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::East => Direction::West,
            _ => Direction::East,
        }
    }
}
