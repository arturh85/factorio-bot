//! Joining two fluidboxes with pipe: the one primitive every fluid rung is
//! built out of.
//!
//! # Why this is a module and not a method
//!
//! It claims no [`crate::goal::Goal`]. A fluid ingredient is not a goal --
//! **it is a connectivity requirement** (owner ruling, 2026-09-07): the
//! machine's input fluidbox has to be joined to something holding the fluid,
//! and there is no quantity to satisfy, no inventory to count and no
//! `Goal::Stored`. *"You pipe crude to a refinery, you never carry it."* So
//! what a caller needs is a function that returns the tiles, refusing before
//! it emits, and that is what this is.
//!
//! Every line below except [`route_between`] and [`supplying_entities`] was
//! **moved unchanged from [`crate::method::gather`]**, which built the
//! wellhead's pumpjack-to-tank run first. `gather` now calls
//! [`route_between`]; the alternative was a second router agreeing with the
//! first until it did not.
//!
//! # Factorio 2.0 makes this small, and that is a measured fact about the game
//!
//! A connected run of pipes is **one fluid segment with one level**: no
//! per-pipe throughput, no length penalty, no pressure gradient (the runtime
//! API's `get_fluid_segment_id` / `get_fluid_segment_capacity` are the
//! evidence, and **none of it crosses our bridge** -- it is a runtime API, so
//! it cannot answer anything about a dump). **There is no flow model here and
//! there must not be one**: connectivity and direction are the whole problem,
//! and rate arithmetic over a pipe run would be a hard-coded model of a game
//! that does not work that way.
//!
//! # What a source is, and what it is not
//!
//! [`supplying_entities`] answers *"what standing thing could put this fluid
//! into a pipe"*, and it answers from `production_type` on the prototype's
//! own fluidboxes -- `output` or `input-output` -- **never from the name or
//! the entity type**. A storage tank, a pumpjack and a pump are all sources
//! by that test, which is the point: sulfur wants water, and water arrives
//! from an offshore pump rather than a tank.
//!
//! It cannot say what a standing tank *holds*: nothing in
//! [`crate::state::PlanState`] models fluid contents. That is the honest
//! limit of the connectivity rule and callers are told to say so.

use crate::error::PlannerError;

use crate::method::gather;
use crate::method::have::PLACE_TICKS;
use crate::method::{ExpansionCtx, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::graph::enclosure;
use factorio_bot_core::graph::route::{RouteError, TileKind, route_belt};
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{
    Direction, FactorioEntity, FactorioRecipe, FluidFilter, Position, Rect, TileFluid,
};

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use std::collections::BTreeSet;

/// `Direction::North` as the wire byte an emitted entity carries.
///
/// A storage tank is symmetric -- its four pipe connections sit in two
/// diagonally opposite pairs, so rotating one maps the set onto itself -- and
/// a pipe has a connection on all four sides. Neither has a facing that
/// changes what it does, so both are placed north and nothing here computes a
/// direction. That is a deliberate contrast with the pump this module refuses
/// to need.
const NORTH: u8 = 0;

/// Half the collision box of the largest thing this module *routes*: a pipe
/// is `0.578` tiles across, and this is the placement clearance
/// `enclosure::rasterize` grows every obstacle by.
///
/// The same reasoning as `method::connect`'s `PLACEMENT_HALF_BOX`, including
/// the defect it records: passing zero makes a cell count as blocked only when
/// an obstacle covers its exact centre, which is accidentally safe for
/// grid-aligned buildings and **misses trees and rocks entirely**, because
/// those sit at arbitrary sub-tile positions. Oil fields are exactly where
/// that matters -- they are unbuilt ground, so trees and rocks are all there
/// is to hit.
const PIPE_HALF_BOX: f64 = 0.3;

// ---------------------------------------------------------------------------
// Prototypes
// ---------------------------------------------------------------------------

/// The prototype that buffers a fluid: the `storage-tank`-typed entity this
/// world knows, first by name when there are several.
///
/// **Found by `entity_type`, never by the name `storage-tank`.** The name is
/// vanilla's; the type is the game's, and asking the world is what lets a
/// modded tank answer and a capture with no tank at all refuse by name.
pub(crate) fn buffer_prototype(
    state: &PlanState,
    fluid: &str,
    source: &str,
) -> Result<String, PlannerError> {
    prototype_of_type(state, "storage-tank").ok_or_else(|| PlannerError::NoFluidBuffer {
        fluid: fluid.to_string(),
        machine: source.to_string(),
        why: "no entity in this world has entity_type `storage-tank`".to_string(),
    })
}

/// The prototype that carries a fluid between two machines, chosen the same
/// way and refusing the same way.
pub(crate) fn pipe_prototype(
    state: &PlanState,
    fluid: &str,
    source: &str,
) -> Result<String, PlannerError> {
    prototype_of_type(state, "pipe").ok_or_else(|| PlannerError::NoFluidBuffer {
        fluid: fluid.to_string(),
        machine: source.to_string(),
        why: "no entity in this world has entity_type `pipe`".to_string(),
    })
}

/// The first prototype of `entity_type` by name. `entity_prototypes` is a
/// `BTreeMap`, so "first by name" is a stated order and not a hash accident.
fn prototype_of_type(state: &PlanState, entity_type: &str) -> Option<String> {
    // `entity_prototypes` is a `DashMap`, whose iteration order is a hash
    // accident -- so the *order* is thrown away and only `min` is kept. A
    // "first one found" here would pick a different tank between two runs of
    // the same binary, which is precisely the determinism this crate is
    // defined by.
    state
        .base()
        .globals
        .entity_prototypes
        .iter()
        .filter(|proto| proto.entity_type == entity_type)
        .map(|proto| proto.name.clone())
        .min()
}

// ---------------------------------------------------------------------------
// Fluid ports
// ---------------------------------------------------------------------------

/// One place a machine takes fluid in or out: the tiles a pipe may stand on to
/// meet it, and one tile that joins all of them.
///
/// # Two captures in this repo disagree about what `positions` MEANS
///
/// `fluidbox_prototypes[].pipe_connections[].positions` is a
/// `PipeConnectionDefinition`'s four positions, one per direction the entity
/// can face, north first. What each position *is* differs between the two
/// captures this repo holds, and the difference is not cosmetic -- read one as
/// the other and every pipe lands on the wrong tile:
///
/// | capture | pumpjack output, north | storage tank, north |
/// |---|---|---|
/// | `crates/core/tests/entity-prototype-fixtures.json` (1.x) | `(1,-2)` | `(-1,-2) (2,-1) (1,2) (-2,1)` |
/// | `crates/core/tests/live-2.1.17-world-snapshot.json`, and every live dump | `(1,-1)` | `(-1,-1) (1,1) (1,1) (-1,-1)` |
///
/// A pumpjack's footprint is 3x3, so `(1,-1)` is a tile **inside** it and
/// `(1,-2)` is the neighbour **outside** it. The old capture names the pipe
/// tile directly; the new one names the interior tile the connection sits on
/// and pairs it, in the game's own prototype, with a `direction` saying which
/// neighbour the pipe goes on -- **and the mod does not send that direction**.
///
/// So this is decided per connection, by whether the offset leaves the
/// footprint, and never by assuming a version. Where the offset is interior
/// and sits on a *corner*, two neighbours are outside and the pipe tile is
/// genuinely ambiguous from the data available. Guessing is the one thing this
/// must not do -- a pipe on the wrong side of a corner builds perfectly and
/// moves nothing -- so the port names **both** candidates plus the diagonal
/// tile that touches both, and the caller places all three. Whichever
/// candidate is the real connection is joined to the run either way, and the
/// other is an inert stub costing one iron plate.
///
/// **For the 2.x storage tank both candidates are real anyway.** Its four
/// connections carry only two distinct interior positions, `(-1,-1)` and
/// `(1,1)`, each appearing twice -- two corners with two directions each -- so
/// the pair of tiles at a corner is the shape of the machine rather than a
/// hedge. The 1.x capture describes a different tank, with one connection at
/// each of the four corners. **Which of those the shipped game has is not
/// settled here**, and this module does not need it settled: both are handled
/// on their own terms, and the answer would only ever save an iron plate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FluidPort {
    /// The tiles that might be the connection. One or two.
    pub candidates: Vec<Position>,
    /// A tile adjacent to every candidate: the corner between them when there
    /// are two, and the candidate itself when there is one. This is what a
    /// route starts from or ends at.
    pub junction: Position,
    /// Which fluidbox of the requested production type this port belongs to,
    /// counted from 0 in `fluidbox_prototypes` order.
    ///
    /// **This is what [`PipeEnd::port_index`] selects on, and it is not the
    /// same as the port's position in the returned list.** A fluidbox may
    /// declare several pipe connections -- 21 of the 56 boxes on a live
    /// 2.1.17 capture do, though no crafting machine among them -- and each
    /// becomes its own port, so "the second port" and "the second box" part
    /// company the moment one does. Selecting on the box is the question
    /// every caller is actually asking.
    pub box_ordinal: usize,
}

impl FluidPort {
    /// Every tile this port needs a pipe on, junction last.
    pub(crate) fn tiles(&self) -> Vec<Position> {
        let mut tiles = self.candidates.clone();
        if !tiles.iter().any(|tile| tile == &self.junction) {
            tiles.push(self.junction.clone());
        }
        tiles
    }
}

/// The ports of the machine `name` standing at `position`, facing north.
///
/// `production_type` filters: `Some("output")` for a pumpjack's single output,
/// `None` for a tank, whose fluidbox is `"none"` because a buffer neither
/// produces nor consumes.
///
/// Refuses with [`PlannerError::FluidPortUnknown`] rather than guessing, on
/// every shape it does not understand: no prototype, no fluidboxes, no
/// connections, a connection with no north position, a non-integer offset, or
/// an offset that is not on the footprint's edge.
pub(crate) fn fluid_ports(
    state: &PlanState,
    name: &str,
    position: &Position,
    production_type: Option<&str>,
) -> Result<Vec<FluidPort>, PlannerError> {
    let refuse = |why: &str| PlannerError::FluidPortUnknown {
        prototype: name.to_string(),
        why: why.to_string(),
    };
    let proto = state
        .base()
        .globals
        .entity_prototypes
        .get(name)
        .ok_or_else(|| refuse("this world has no prototype of that name"))?;
    let boxes = proto
        .fluidbox_prototypes
        .as_ref()
        .ok_or_else(|| refuse("its prototype carries no fluidbox_prototypes"))?;

    // The largest whole-tile offset still inside the footprint, per axis. A
    // 3x3 collision box (half-width 1.199) gives 1: the tiles at offset -1, 0
    // and 1 are the machine, and anything beyond is outside it.
    let span_x = (proto.collision_box.right_bottom.x() + 0.5).floor();
    let span_y = (proto.collision_box.right_bottom.y() + 0.5).floor();

    let mut ports: Vec<FluidPort> = Vec::new();
    let mut box_ordinal = 0usize;
    for fluidbox in boxes {
        if let Some(wanted) = production_type
            && fluidbox.production_type != wanted
        {
            continue;
        }
        let this_box = box_ordinal;
        box_ordinal += 1;
        let connections =
            fluidbox.pipe_connections.as_ref().as_ref().ok_or_else(|| {
                refuse("a fluidbox of its prototype declares no pipe_connections")
            })?;
        for connection in connections {
            // `positions` is one entry per entity direction, north first.
            let offset = connection
                .positions
                .first()
                .ok_or_else(|| refuse("a pipe connection carries no positions at all"))?;
            let (dx, dy) = (offset.x(), offset.y());
            if dx.fract() != 0. || dy.fract() != 0. {
                return Err(refuse(
                    "a pipe connection sits at a fractional offset, which this module cannot \
                     place a whole pipe tile against",
                ));
            }
            // **Which of the two conventions is this capture using?** See the
            // note above this function: a 2.x capture names the tile INSIDE
            // the machine and a 1.x one names the tile OUTSIDE it, and the
            // difference is decided per connection by whether the offset
            // leaves the footprint.
            let outside_x = dx.abs() > span_x;
            let outside_y = dy.abs() > span_y;
            let (candidates, junction): (Vec<(f64, f64)>, (f64, f64)) = match (outside_x, outside_y)
            {
                // Outside on both axes: a diagonal, which no pipe connection
                // is, under either convention.
                (true, true) => {
                    return Err(refuse(
                        "a pipe connection sits diagonally off the footprint's corner, which is \
                         neither an interior tile nor an orthogonal neighbour",
                    ));
                }
                // The 1.x convention: this IS the pipe tile, and there is
                // nothing to disambiguate.
                (true, false) | (false, true) => {
                    let adjacent = if outside_x {
                        dx.abs() == span_x + 1.
                    } else {
                        dy.abs() == span_y + 1.
                    };
                    if !adjacent {
                        return Err(refuse(
                            "a pipe connection sits more than one tile off the footprint, so it \
                             names neither an interior tile nor a neighbouring one",
                        ));
                    }
                    (vec![(dx, dy)], (dx, dy))
                }
                // The 2.x convention: an interior tile, whose connection
                // direction the mod does not send. Every neighbour outside
                // the footprint is a candidate, and for a corner there are
                // two -- joined by the diagonal between them.
                (false, false) => {
                    let mut candidates: Vec<(f64, f64)> = Vec::new();
                    if dx.abs() == span_x {
                        candidates.push((dx + dx.signum(), dy));
                    }
                    if dy.abs() == span_y {
                        candidates.push((dx, dy + dy.signum()));
                    }
                    if candidates.is_empty() {
                        return Err(refuse(
                            "a pipe connection sits away from the footprint's edge, so no \
                             neighbouring tile is outside the machine",
                        ));
                    }
                    let junction = if candidates.len() == 2 {
                        (dx + dx.signum(), dy + dy.signum())
                    } else {
                        candidates[0]
                    };
                    (candidates, junction)
                }
            };
            let at = |(ox, oy): (f64, f64)| Position::new(position.x() + ox, position.y() + oy);
            let port = FluidPort {
                candidates: candidates.iter().copied().map(at).collect(),
                junction: at(junction),
                box_ordinal: this_box,
            };
            // Deduplicated on the GROUND it claims, never on `box_ordinal`:
            // a storage tank names the same two interior corners four times
            // over, and that was one port before this field existed. Two
            // *different* boxes that resolve to the same tiles keep the
            // first, so an ordinal that loses the race simply has no port
            // and `select` refuses by name rather than handing back a
            // neighbour's.
            if !ports
                .iter()
                .any(|seen| seen.candidates == port.candidates && seen.junction == port.junction)
            {
                ports.push(port);
            }
        }
    }
    if ports.is_empty() {
        return Err(refuse(
            "no fluidbox of the requested production type has a pipe connection",
        ));
    }
    Ok(ports)
}

// ---------------------------------------------------------------------------
// What can supply a fluid
// ---------------------------------------------------------------------------

/// `production_type` values whose box can put fluid **into** a pipe.
///
/// **`"none"` is in here, and that is measured rather than assumed.** On the
/// live 2.1.17 capture a `storage-tank`'s single box and a `pipe`'s are both
/// `production_type: "none"` -- there is no `"input-output"` on any prototype
/// in the dump. A box that neither produces nor consumes is one fluid may
/// pass through in either direction, which is exactly what a buffer is, so
/// leaving `"none"` out excluded every tank from being a source. It did:
/// the first run of this code piped a refinery to a pumpjack because the tank
/// beside it was invisible. `"input-output"` is kept for a world that uses
/// it.
///
/// There is deliberately no `ACCEPTING` counterpart yet. It would have exactly
/// one plausible caller -- adopting a standing consumer as a fluid product's
/// sink instead of building a buffer -- and that caller does not exist, so it
/// would be a constant nothing reads. When it arrives it is
/// `["input", "input-output"]`, and the reason it is worth having is the
/// chemistry rung: a chemical plant consuming petroleum **is** a sink for a
/// refinery, one hop along from the buffer this crate builds today.
pub(crate) const SUPPLYING: [&str; 3] = ["output", "input-output", "none"];

/// Every standing entity whose prototype declares a fluidbox of one of
/// `production_types`, nearest to `from` first.
///
/// # Asked of the prototype, never of the name
///
/// The owner's correction of 2026-09-07, in one function: *"do not
/// special-case a tank as the only possible source"*. Sulfur wants water and
/// water comes from an offshore pump; crude comes from a tank or straight off
/// a pumpjack. `production_type` is the field that says which boxes can
/// supply, so it is the only thing consulted here -- a modded buffer answers
/// and a hard-coded `storage-tank` would not.
///
/// # What it cannot say
///
/// **Whether the thing is holding that particular fluid.**
/// [`crate::state::PlanState`] models no fluid contents at all, and no dump
/// this project holds carries any. So this answers *"could this put a fluid
/// into a pipe"* and nothing stronger; a caller that needs "and it is crude"
/// has to say so in its own words. That is the honest shape of the
/// connectivity rule, and pretending otherwise would be the silent class this
/// crate keeps paying for.
///
/// Deterministic: candidate prototype names come out of a `BTreeSet`, and the
/// result is sorted by `(distance, x, y)`.
pub(crate) fn fluidbox_entities(
    state: &PlanState,
    production_types: &[&str],
    from: &Position,
) -> Vec<FactorioEntity> {
    let names: BTreeSet<String> = state
        .base()
        .globals
        .entity_prototypes
        .iter()
        .filter(|proto| {
            proto.fluidbox_prototypes.as_ref().is_some_and(|boxes| {
                boxes
                    .iter()
                    .any(|b| production_types.contains(&b.production_type.as_str()))
            })
        })
        .map(|proto| proto.name.clone())
        .collect();
    let mut standing: Vec<FactorioEntity> = state
        .entities_named_any(&names)
        .into_values()
        .flatten()
        .collect();
    standing.sort_by(|a, b| {
        calculate_distance(&a.position, from)
            .total_cmp(&calculate_distance(&b.position, from))
            .then(a.position.x.total_cmp(&b.position.x))
            .then(a.position.y.total_cmp(&b.position.y))
            .then(a.name.cmp(&b.name))
    });
    standing
}

/// The standing entities that can be shown to supply `fluid`, nearest to
/// `from` first, and the ones that were considered and rejected.
///
/// # Why "has a supplying fluidbox" is NOT enough, measured rather than
/// argued
///
/// The first version of this rung took the nearest supplying fluidbox and
/// piped to it. On the very first offline run that reached the code -- a
/// `gathered:crude-oil` plan followed by `produced:petroleum-gas` -- it chose
/// a **boiler**, because `method::power` had sited a plant at the wellhead
/// and a boiler's steam box supplies. A refinery piped to a boiler builds
/// perfectly and makes nothing, which is this repo's standing silent class.
///
/// So a candidate must be *attributable* to the fluid. Two **observations**
/// answer first when they can -- the box's own [`FluidFilter`], which is what
/// now rejects that boiler, and the ground under a
/// [ground-drawing](drawn_from_ground) entity's source tile, which is what
/// answers water. Only when neither says anything do the three original
/// *inferences* run, each derived from the world rather than from a list of
/// names:
///
/// 1. **the plan told it what to make** -- `entity.recipe` names a recipe
///    whose products include the fluid. This is the test that will make the
///    chemistry rung compose: a refinery running `basic-oil-processing` is a
///    petroleum source for a chemical plant by exactly this rule, with no new
///    code;
/// 2. **it stands on the resource** -- an extractor's footprint covers a
///    charted tile of the fluid, i.e. a pumpjack on a crude well;
/// 3. **it is a buffer at that resource's field** -- a `storage-tank`-typed
///    entity within [`gather::FIELD_RADIUS`] of a charted tile of the fluid,
///    which is precisely the tank `method::gather` stands up.
///
/// # Water: closed on 2026-09-08, and the condition this doc twice named was
/// twice the wrong one
///
/// **Water used to have no answer here**, because water is not a charted
/// resource and an offshore pump carries no recipe, so all three inferences
/// failed on the one entity that produces it. This doc named its own
/// unblocking condition -- *"it becomes reachable the day the fluidbox's
/// accepted fluid crosses the bridge"* -- on the unstated assumption that an
/// offshore pump's output box is filtered to water. **The filter crossed and
/// that assumption was false.** Measured off a live seed-31337 dump, every one
/// of the 56 fluid boxes in this mod set answered, and:
///
/// ```text
/// offshore-pump   output any
/// boiler          input  only=water    output only=steam
/// heat-exchanger  input  only=water    output only=steam
/// chemical-plant  input any   input any   output any   output any
/// ```
///
/// A 2.0 offshore pump takes its fluid from the **tile it stands in front
/// of**, so the box is honestly unfiltered and [`FluidFilter::Any`] is the
/// correct answer; the only two `only=water` boxes in the entire mod set are
/// boiler and heat-exchanger *inputs*, and an input supplies nothing.
///
/// The successor condition was a lead too -- *"ask
/// `LuaEntity::get_fluid_source_fluid`"* -- and it is **also** not what
/// closed this. That call is a method with `subclasses: ["OffshorePump"]`, so
/// it can only be asked of a pump that already stands, which is precisely the
/// thing a planner choosing a site does not have. What closed it is the
/// *prototype* pair [`drawn_from_ground`] reads: `fluid_source_offset` (which
/// tile) and `LuaTilePrototype::fluid` (what that tile gives), neither
/// requiring an entity or a runtime call.
///
/// **So this doc named a sufficient condition twice and was wrong twice, in
/// the same way both times: it wrote down where the answer ought to live
/// instead of checking `runtime-api.json` for where it does.** The rule that
/// survives is the cheap one -- the shipped API description is one grep away,
/// and both wrong guesses cost more than the grep would have.
///
/// # What the filter did close
///
/// Four prototypes have supplying boxes that are *all* filtered: `boiler`
/// and `heat-exchanger` (steam), `fusion-generator` (fluoroketone-hot),
/// `fusion-reactor` (fusion-plasma). Both directions now answer on a fact for
/// those -- see [`attributable_to`]. **Steam had no rule at all before**, for
/// precisely water's reason, and has one now.
///
/// # What it still cannot do
///
/// **Nothing here reads what a tank contains**, because nothing in the model
/// does. Rule 3 attributes a tank by *where it stands*, which is an inference
/// from the map and not an observation of the fluid. The prototype filter
/// does not help there: a `storage-tank`'s box is
/// [`FluidFilter::Any`] -- correctly, since a tank will hold anything.
///
pub(crate) fn sources_of(
    state: &PlanState,
    fluid: &str,
    from: &Position,
) -> (Vec<FactorioEntity>, Vec<FactorioEntity>) {
    let field: Vec<Position> = state
        .resource_patches(fluid)
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    let mut attributable = Vec::new();
    let mut rejected = Vec::new();
    for entity in fluidbox_entities(state, &SUPPLYING, from) {
        if attributable_to(state, fluid, &field, &entity) {
            attributable.push(entity);
        } else {
            rejected.push(entity);
        }
    }
    // **A buffer outranks an extractor, which is the owner's topology and not
    // an optimisation**: *"the fluid tank the oil arrives in from far away
    // should be connected to the refineries"*. Piping a refinery straight off
    // a pumpjack works in the game and is the wrong shape -- it ties one
    // consumer to one well, and it is what this ranking existed to prevent
    // the first time the code chose by distance alone. `fluidbox_entities`
    // has already ordered by `(distance, x, y)`, and `sort_by_key` is stable,
    // so within a class that order survives.
    attributable.sort_by_key(|entity| u8::from(!is_buffer(state, &entity.name)));
    (attributable, rejected)
}

/// Can a pipe actually stand on every tile the chosen port needs?
///
/// # Why siting has to ask this
///
/// A machine's fluid port is a tile *outside* its own footprint, so a machine
/// sited flush against its source has its port **inside the source**. That is
/// not hypothetical: siting an `oil-refinery` beside the `storage-tank` it
/// draws from put the refinery's first input port at a tile the tank stands
/// on, and the run refused with "the tile at ... cannot hold a pipe" -- a
/// true statement about a site that should never have been chosen. So this is
/// the predicate `free_area_near_where` needs, and the alternative is a
/// method that sites first and discovers the impossibility afterwards.
///
/// A tile already holding this pipe prototype counts as placeable, for the
/// reason [`route_between`] gives: a standing pipe is a join.
pub(crate) fn port_is_placeable(
    state: &PlanState,
    name: &str,
    position: &Position,
    production_type: Option<&str>,
    port_index: Option<usize>,
    pipe: &str,
) -> bool {
    let Ok(ports) = fluid_ports(state, name, position, production_type) else {
        return false;
    };
    let chosen = select(ports, port_index);
    !chosen.is_empty()
        && chosen.iter().any(|port| {
            port.tiles().iter().all(|tile| {
                state.is_area_free(pipe, tile)
                    || state
                        .entity_at(tile)
                        .is_some_and(|standing| standing.name == pipe)
            })
        })
}

/// Every port of one fluidbox, by that box's ordinal, or all of them.
fn select(ports: Vec<FluidPort>, index: Option<usize>) -> Vec<FluidPort> {
    match index {
        Some(index) => ports
            .into_iter()
            .filter(|port| port.box_ordinal == index)
            .collect(),
        None => ports,
    }
}

/// Is `name` a fluid buffer -- found by `entity_type`, never by the name
/// `storage-tank`, the rule [`buffer_prototype`] states.
fn is_buffer(state: &PlanState, name: &str) -> bool {
    state
        .base()
        .globals
        .entity_prototypes
        .get(name)
        .is_some_and(|proto| proto.entity_type == "storage-tank")
}

// ---------------------------------------------------------------------------
// Which fluidbox takes which fluid
// ---------------------------------------------------------------------------

/// Why a recipe's fluid could not be tied to one of a machine's fluidboxes.
///
/// Carried rather than returned as a [`PlannerError`] because the caller
/// knows the goal and the recipe and this does not; [`crate::method::fabricate`]
/// turns it into the refusal a reader sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoxUndecidable {
    /// The fluid whose box could not be named.
    pub fluid: String,
    /// Its 1-based place among the recipe's fluids of that direction.
    pub ordinal: usize,
    /// How many fluids of that direction the recipe has.
    pub fluids: usize,
    /// How many boxes of that direction the machine declares, or `None` when
    /// this world has no prototype for the machine at all.
    pub boxes: Option<usize>,
}

/// Which fluidbox of `machine`, counted from 0 within `production_type`, each
/// of `recipe`'s fluids of that direction goes into.
///
/// Returns one ordinal per fluid, in recipe order, or the first fluid it
/// cannot decide.
///
/// # Three rules, each measured on a live 2.1.17 server rather than reasoned
///
/// This module used to assert, in [`PipeEnd::port_index`]'s own doc, that
/// *"the game assigns a recipe's fluid ingredients to the machine's input
/// boxes in order"*. **That is false, and the recipe it was written about is
/// the counter-example.** Placing each machine on a headless seed-31337
/// server, setting the recipe, and reading `LuaEntity::get_fluid_filter` back
/// off the standing entity (2026-09-08):
///
/// 1. **An explicit `fluidbox_index` wins, and it is 1-based within the
///    production type.** `basic-oil-processing` declares `2` for `crude-oil`
///    and `3` for `petroleum-gas`; the standing `oil-refinery` reports crude
///    on input box **2** and petroleum on output box **3**, and its *first*
///    input box, the one a positional rule picks, carries nothing at all.
/// 2. **Otherwise, when the counts match, positional is exact.** Verified on
///    all eleven of this mod set's multi-fluid recipes whose machine declares
///    as many input boxes as the recipe has fluid ingredients -- `sulfur`
///    (water then petroleum-gas into a `chemical-plant`'s two inputs),
///    `advanced-oil-processing`, `coal-liquefaction`,
///    `casting-low-density-structure`, `electromagnetic-science-pack` and the
///    rest -- in every case the *k*th fluid on the *k*th box.
/// 3. **Otherwise only the FIRST fluid has an answer, and it is box 0.** When
///    a machine declares more boxes of a direction than the recipe has fluids,
///    the game *merges* the surplus rather than leaving it idle, and the merge
///    is not "extras at the end": a `cryogenic-plant` running `fluoroketone`
///    (three input boxes, two fluids) merges boxes **1 and 2** for fluorine
///    and gives ammonia box **3**, so positional would pipe ammonia into the
///    fluorine box. Fluid #1 landed on box 0 in every case measured, merged or
///    not, so it is answered; everything after it is refused.
///
/// # `None` is "the recipe did not say", and every archived dump reads it
///
/// Nothing sent `fluidbox_index` before 2026-09-08, so on a dump taken before
/// that rule 1 never fires and `basic-oil-processing` falls to rule 3 -- box
/// 0, which is the box the game leaves empty. That is the pre-existing
/// behaviour and not a regression this introduces, but a plan made against an
/// old dump pipes that refinery's crude to a dead port. **Re-dump before
/// building an oil rig from an archived world.**
pub(crate) fn fluid_box_ordinals(
    state: &PlanState,
    machine: &str,
    recipe: &FactorioRecipe,
    production_type: &str,
) -> Result<Vec<(String, usize)>, BoxUndecidable> {
    let boxes = box_count(state, machine, production_type);
    let declared: Vec<(String, Option<u32>)> = if production_type == "output" {
        recipe
            .products
            .iter()
            .filter(|p| p.product_type == "fluid")
            .map(|p| (p.name.clone(), p.fluidbox_index))
            .collect()
    } else {
        recipe
            .ingredients
            .iter()
            .flatten()
            .filter(|i| i.ingredient_type == "fluid")
            .map(|i| (i.name.clone(), i.fluidbox_index))
            .collect()
    };
    let mut out = Vec::with_capacity(declared.len());
    for (ordinal, (fluid, declared_index)) in declared.iter().enumerate() {
        // Rule 1. `0` is not a legal `fluidbox_index` -- the field is 1-based
        // -- so it is treated as no answer rather than silently becoming box
        // 0, the `absent is not a value` rule this repo keeps relearning.
        if let Some(index) = declared_index
            && *index >= 1
        {
            out.push((fluid.clone(), (*index as usize) - 1));
            continue;
        }
        // Rule 2.
        if boxes == Some(declared.len()) {
            out.push((fluid.clone(), ordinal));
            continue;
        }
        // Rule 3.
        if ordinal == 0 {
            out.push((fluid.clone(), 0));
            continue;
        }
        return Err(BoxUndecidable {
            fluid: fluid.clone(),
            ordinal: ordinal + 1,
            fluids: declared.len(),
            boxes,
        });
    }
    Ok(out)
}

/// How many fluidboxes of one production type `machine` declares, or `None`
/// when this world has no prototype for it or the prototype carries no
/// fluidboxes.
fn box_count(state: &PlanState, machine: &str, production_type: &str) -> Option<usize> {
    Some(
        state
            .base()
            .globals
            .entity_prototypes
            .get(machine)?
            .fluidbox_prototypes
            .as_ref()?
            .iter()
            .filter(|b| b.production_type == production_type)
            .count(),
    )
}

/// The filters on every box of `name` that could supply a fluid, in
/// `fluidbox_prototypes` order.
///
/// Empty when the prototype is unknown to the model or declares no supplying
/// box at all -- which is a different thing from a box that answered
/// [`FluidFilter::Unknown`], and callers must not treat the two alike.
fn supplying_filters(state: &PlanState, name: &str) -> Vec<FluidFilter> {
    state
        .base()
        .globals
        .entity_prototypes
        .get(name)
        .and_then(|proto| proto.fluidbox_prototypes.clone())
        .map(|boxes| {
            boxes
                .into_iter()
                .filter(|b| SUPPLYING.contains(&b.production_type.as_str()))
                .map(|b| b.filter)
                .collect()
        })
        .unwrap_or_default()
}

/// The three tests [`sources_of`] documents, in that order, with the box's
/// own filter consulted first when it has one.
///
/// # The box gets to answer before anything is inferred
///
/// The three original tests are inferences from the map -- what the plan told
/// a machine to craft, what it stands on, what it stands near. Since the
/// prototype's [`FluidFilter`] crosses the bridge there is a fourth test that
/// is not an inference at all: **the box says so**. It runs first because a
/// definite answer should not be reached through three guesses, and it does
/// two things nothing here could do before:
///
/// - **it gives steam a rule, where none of the three could reach.** A boiler
///   carries no recipe, steam is not a charted resource and a boiler is not a
///   buffer, so every inference fails on it and always would have -- the same
///   shape as the water hole [`sources_of`] documents, which this does *not*
///   close (read that doc: the pump's box turned out to be unfiltered);
/// - **it refuses the boiler by name.** The bug that motivated the three
///   tests was a refinery piped to a boiler, because a boiler's steam box
///   supplies and distance chose it. A box filtered `steam` definitely does
///   not supply petroleum gas, so it is now rejected on a fact rather than
///   surviving to be rejected on the absence of one.
///
/// # The ground answers too, and it is the only thing that can answer water
///
/// A [ground-drawing](drawn_from_ground) entity -- one whose prototype has a
/// `fluid_source_offset`, which in this mod set is the offshore pump and
/// nothing else -- produces whatever the tile at that offset yields, because
/// its output box carries no filter to say otherwise. So the second test is
/// **the ground**, and unlike the three inferences below it can refuse as
/// well as attribute: a pump on dry land, or on a lava tile when petroleum
/// gas was asked for, is definitely not a source and the inferences must not
/// be allowed to talk it back up. `TileFluid::Unknown` -- unexplored ground,
/// an archived dump, a mod predating the field -- does neither, for the
/// reason the next section gives.
///
/// # What [`FluidFilter::Unknown`] does, and why
///
/// **Nothing.** It neither attributes nor excludes, so a prototype that never
/// said falls through to exactly the three inferences that ran before this
/// field existed. That is the only choice with no regression in it: erring
/// towards permitting would make every unreadable box a phantom source, and
/// erring towards refusing would make every archived dump -- all of which
/// predate the field, so every box in them is `Unknown` -- unplannable.
/// [`FluidFilter::Any`] behaves the same way here for a different reason: an
/// unfiltered box genuinely does accept the fluid, and *which* recipe fills
/// it is what the three inferences are for.
fn attributable_to(
    state: &PlanState,
    fluid: &str,
    field: &[Position],
    entity: &FactorioEntity,
) -> bool {
    let filters = supplying_filters(state, &entity.name);
    if filters.iter().any(|f| f.is_only(fluid)) {
        return true;
    }
    // THE GROUND ANSWERS BEFORE ANYTHING IS INFERRED, exactly as the box
    // filter does, and for the same reason: it is an observation, not a guess.
    match drawn_from_ground(state, entity) {
        Some(ground) if ground.yields(fluid) => return true,
        // Definitely dry, or definitely a different fluid. A ground-drawing
        // entity has *no other* source -- its box is unfiltered and it carries
        // no recipe -- so this is a refusal on a fact, and the three
        // inferences below must not be allowed to talk it back up.
        Some(ground) if ground.is_dry() || ground.named().is_some() => return false,
        // `TileFluid::Unknown`, or an entity that does not draw from the
        // ground at all. Falls through, inert.
        _ => {}
    }
    // Every supplying box names a *different* fluid, so no recipe, no
    // footprint and no neighbourhood can make this entity a source. Guarded
    // on non-empty: an entity with no supplying box at all reaches here only
    // if `fluidbox_entities` let it through, and "no boxes" is not a claim
    // that they all exclude the fluid.
    if !filters.is_empty() && filters.iter().all(|f| f.excludes(fluid)) {
        return false;
    }
    if let Some(recipe) = &entity.recipe
        && state
            .base()
            .globals
            .recipes
            .get(recipe.as_str())
            .is_some_and(|r| r.products.iter().any(|p| p.name == fluid))
    {
        return true;
    }
    if field.is_empty() {
        return false;
    }
    if let Some(area) = state.collision_area(&entity.name, &entity.position)
        && field.iter().any(|tile| {
            tile.x() > area.left_top.x()
                && tile.x() < area.right_bottom.x()
                && tile.y() > area.left_top.y()
                && tile.y() < area.right_bottom.y()
        })
    {
        return true;
    }
    is_buffer(state, &entity.name)
        && field
            .iter()
            .any(|tile| calculate_distance(tile, &entity.position) <= gather::FIELD_RADIUS)
}

/// What the ground under `entity`'s fluid source tile yields, or `None` when
/// this entity does not draw from the ground at all.
///
/// # Why this is two prototype facts and not a name
///
/// A Factorio 2.0 offshore pump takes its fluid from the **tile it stands in
/// front of**, so nothing about the pump itself says what it produces. That
/// was measured, not assumed: on 2026-09-08 every one of the 56 fluid boxes
/// in this mod set answered its filter, and `offshore-pump`'s *output* box is
/// [`factorio_bot_core::types::FluidFilter::Any`] -- the only `only=water`
/// boxes anywhere are the boiler's and heat-exchanger's inputs, which supply
/// nothing. So [`sources_of`]'s three inferences could never reach water: it
/// is not a charted resource, a pump carries no recipe, and a pump is not a
/// buffer.
///
/// The two halves that do answer are both prototype data, so neither this
/// function nor its caller names `offshore-pump`, `water`, or `{0, -1}`:
///
/// - [`FactorioEntityPrototype::fluid_source_offset`] -- which tile, as an
///   offset in the north frame. The runtime API marks it
///   `subclasses: ["OffshorePump"]`, so **its absence is the discriminator**:
///   a boiler standing on a shoreline has none and is declined here without
///   anybody having to exclude boilers by name;
/// - [`factorio_bot_core::types::TileFluid`] on the tile that offset lands on
///   -- from `LuaTilePrototype::fluid`, *"The fluid offshore pump produces on
///   this tile, if any"*.
///
/// **The runtime call `LuaEntity::get_fluid_source_fluid` is NOT what
/// crosses**, and could not be: it is a method with the same `OffshorePump`
/// subclass restriction, so it can only be asked of a pump that already
/// stands. A planner deciding where to *put* one has no such entity. The
/// prototype pair answers the same question with no entity at all, which is
/// strictly more than the runtime call could have given.
///
/// # Three answers out, and `Unknown` is inert
///
/// [`Some(TileFluid::Yields)`](factorio_bot_core::types::TileFluid::Yields)
/// and [`Some(TileFluid::Dry)`](factorio_bot_core::types::TileFluid::Dry) are
/// both claims about the map.
/// [`Some(TileFluid::Unknown)`](factorio_bot_core::types::TileFluid::Unknown)
/// is not: it is unexplored ground, an archived dump, or a mod predating the
/// field, and it falls through to exactly the inferences that ran before this
/// existed. `None` -- no `fluid_source_offset` -- is a different absence
/// again, and also falls through, since every other supplying entity in the
/// game is judged by those inferences and must go on being.
///
/// A non-cardinal `direction` yields `None` rather than an answer: `turn`
/// only names the four rotations, and a pump at a diagonal is not something
/// this can site or read.
fn drawn_from_ground(state: &PlanState, entity: &FactorioEntity) -> Option<TileFluid> {
    let offset = state
        .base()
        .globals
        .entity_prototypes
        .get(entity.name.as_str())?
        .fluid_source_offset
        .clone()?;
    let turned = offset.turn(Direction::from_u8(entity.direction)?)?;
    let tile = Position::new(
        entity.position.x() + turned.x(),
        entity.position.y() + turned.y(),
    );
    Some(state.base().entity_graph.fluid_at(&tile))
}

// ---------------------------------------------------------------------------
// The pipe run
// ---------------------------------------------------------------------------

/// One end of a pipe run: a machine, where it stands, the ground it takes up,
/// and which of its fluidboxes may be joined.
///
/// **A machine and not a position**, for the reason `method::connect`'s doc
/// gives about belts: the size and the parity of a machine are the whole
/// problem. A fluidbox is an offset from a footprint, so a bare coordinate
/// cannot say where a pipe may meet it.
pub(crate) struct PipeEnd<'a> {
    pub name: &'a str,
    pub position: &'a Position,
    /// The footprint to keep the route out of. Passed in rather than derived
    /// because the caller usually has it already and, for a machine this plan
    /// has not placed yet, it is not on any grid this function can consult.
    pub area: Rect,
    /// `Some("output")`, `Some("input")`, or `None` for every box -- a
    /// storage tank's is `"none"`, since a buffer neither produces nor
    /// consumes.
    pub production_type: Option<&'a str>,
    /// Which fluidbox of the selected direction may be joined, counted from
    /// 0 in `fluidbox_prototypes` order, or `None` for "any of them, nearest
    /// first".
    ///
    /// # Do not derive this here -- ask [`fluid_box_ordinals`]
    ///
    /// **This doc used to say the game assigns a recipe's fluid ingredients
    /// to the input boxes in order, and named `basic-oil-processing` as the
    /// example. That recipe is the counter-example.** It declares
    /// `fluidbox_index = 2` for crude oil, and a standing `oil-refinery` set
    /// to it reports crude on input box 2 with box 1 -- the one this rule
    /// picked -- carrying nothing at all. Measured live on 2026-09-08; see
    /// [`fluid_box_ordinals`], which states all three rules and which of them
    /// an archived dump can still reach.
    ///
    /// Ranking by distance instead would pick whichever box the pipe reaches
    /// first, which builds 100% correctly and moves nothing whenever it
    /// guesses wrong -- the class this crate has already paid for with
    /// inserters and pumps. `None` is for an end where every box is
    /// interchangeable, i.e. a buffer.
    pub port_index: Option<usize>,
}

/// Every tile a run from `from` to `to` needs a pipe on: both ends' port
/// tiles and the route between their junctions, in placement order and
/// without duplicates.
///
/// Touches no [`ExpansionCtx`] and reserves nothing, so **every refusal it
/// makes leaves the plan exactly as it found it** -- the promise
/// `method::connect` states and the reason it is worth stating: a pipe run
/// that stops halfway is worse than no pipe run, because the machine at the
/// near end fills up and stops with nothing to show for the iron.
///
/// `reserved` is ground some other run of the same expansion has already
/// claimed. It exists because two runs into one machine -- fluid in, fluid
/// out -- are routed one after the other against a world where neither is
/// emitted yet, so the second would happily cross the first.
pub(crate) fn route_between(
    state: &PlanState,
    from: &PipeEnd<'_>,
    to: &PipeEnd<'_>,
    pipe: &str,
    reserved: &[Rect],
) -> Result<Vec<Position>, PlannerError> {
    let refuse = |why: String| PlannerError::NoPipeRoute {
        from: format!("the {} at {}", from.name, from.position),
        to: format!("the {} at {}", to.name, to.position),
        why,
    };

    let sources = select(
        fluid_ports(state, from.name, from.position, from.production_type)?,
        from.port_index,
    );
    let sinks = select(
        fluid_ports(state, to.name, to.position, to.production_type)?,
        to.port_index,
    );
    if sources.is_empty() || sinks.is_empty() {
        return Err(refuse(format!(
            "the recipe's fluid is the #{} of its direction and the {} declares fewer boxes \
             than that",
            from.port_index.or(to.port_index).unwrap_or(0) + 1,
            if sources.is_empty() {
                from.name
            } else {
                to.name
            }
        )));
    }

    let (area, origin) = enclosure::window(from.position);
    let mut blocked = enclosure::rasterize(
        state
            .base()
            .entity_graph
            .blocking_boxes_within(&area)
            .into_iter()
            .chain(overlay_boxes(state, &area))
            // Neither machine need be in the overlay -- this function is
            // asked before anything is emitted -- so both footprints are
            // claimed by hand, or the route would happily run through the
            // machine it is aiming at.
            .chain([from.area.clone(), to.area.clone()])
            .chain(reserved.iter().cloned()),
        origin,
        (PIPE_HALF_BOX, PIPE_HALF_BOX),
    );
    // **A pipe already standing is a join, not an obstacle.** Nothing else
    // in this crate needed that: `method::gather` routes on virgin ground at
    // a wellhead. The moment a second run starts from the tank the first one
    // filled, the first run's own pipes are in the way -- and they are the
    // one kind of occupant a pipe run may end on, since a tile that already
    // holds this prototype needs no placement and carries fluid either way.
    // Without this the second run refuses with "the tile at ... cannot hold a
    // pipe", which reads as blocked ground and is the opposite of the truth.
    let standing_pipes: Vec<Position> = state
        .entities_named(pipe)
        .into_iter()
        .map(|entity| entity.position)
        .filter(|position| {
            position.x() >= area.left_top.x()
                && position.x() <= area.right_bottom.x()
                && position.y() >= area.left_top.y()
                && position.y() <= area.right_bottom.y()
        })
        .collect();
    for tile in &standing_pipes {
        if let Some(cell) = cell_of(origin, tile) {
            blocked[enclosure::cell_index(cell.0, cell.1)] = false;
        }
    }

    // A port's own tiles are where pipes go, so they must not read as
    // obstacles to the search that has to end on one.
    let port_tiles: Vec<Position> = sources
        .iter()
        .chain(sinks.iter())
        .flat_map(|port| port.tiles())
        .collect();
    for tile in &port_tiles {
        if let Some(cell) = cell_of(origin, tile) {
            blocked[enclosure::cell_index(cell.0, cell.1)] = false;
        }
    }

    // Both ends are ranked by straight-line distance, so a corner the route
    // cannot reach does not refuse the whole run. With one source port -- a
    // pumpjack's, which is what `method::gather` has -- the outer loop runs
    // once and this is exactly the search that module made before the code
    // moved here.
    let mut ranked_sources: Vec<&FluidPort> = sources.iter().collect();
    ranked_sources.sort_by(|a, b| {
        calculate_distance(&a.junction, to.position)
            .total_cmp(&calculate_distance(&b.junction, to.position))
            .then(a.junction.x.total_cmp(&b.junction.x))
            .then(a.junction.y.total_cmp(&b.junction.y))
    });

    let mut last: Option<String> = None;
    for source in ranked_sources {
        let Some(start) = cell_of(origin, &source.junction) else {
            last = Some(format!(
                "the {}'s connection at {} is outside the searched window",
                from.name, source.junction
            ));
            continue;
        };
        let mut ranked: Vec<&FluidPort> = sinks.iter().collect();
        ranked.sort_by(|a, b| {
            calculate_distance(&a.junction, &source.junction)
                .total_cmp(&calculate_distance(&b.junction, &source.junction))
                .then(a.junction.x.total_cmp(&b.junction.x))
                .then(a.junction.y.total_cmp(&b.junction.y))
        });
        for sink in ranked {
            let Some(end) = cell_of(origin, &sink.junction) else {
                last = Some(format!(
                    "the {}'s connection at {} is outside the searched window",
                    to.name, sink.junction
                ));
                continue;
            };
            // `max_underground: None`: an underground pipe pair has an input
            // half and an output half, and neither `FactorioEntity` nor the
            // mod's `rcon_place_entity` can say which -- the identical
            // constraint `method::connect` states for underground belts. A
            // route that would need to tunnel refuses.
            match route_belt(&blocked, origin, start, end, None) {
                Ok(route) => {
                    let mut tiles: Vec<Position> = Vec::new();
                    let mut push = |position: &Position| {
                        if !tiles.contains(position) {
                            tiles.push(position.clone());
                        }
                    };
                    for tile in source.tiles() {
                        push(&tile);
                    }
                    for tile in &route.tiles {
                        debug_assert!(
                            matches!(tile.kind, TileKind::Belt),
                            "route_belt with max_underground: None can only produce surface tiles"
                        );
                        push(&tile.position);
                    }
                    for tile in sink.tiles() {
                        push(&tile);
                    }
                    // The grid says a pipe fits; `is_area_free` is what the
                    // game will be asked. It knows about water, which the
                    // entity rasterisation does not, and an oil field beside
                    // a lake is exactly where that differs.
                    // A tile this run needs and a pipe already occupies is
                    // dropped rather than placed again: the run is complete
                    // without it, and re-placing would fail its own
                    // `AreaFree`. That is also what makes a replan over a
                    // half-laid run a no-op rather than a refusal.
                    tiles.retain(|tile| !standing_pipes.contains(tile));
                    if let Some(bad) = tiles.iter().find(|tile| !state.is_area_free(pipe, tile)) {
                        return Err(refuse(format!("the tile at {bad} cannot hold a {pipe}",)));
                    }
                    return Ok(tiles);
                }
                Err(RouteError::NoPath { blocked }) => {
                    last = Some(match blocked.first() {
                        Some(first) => format!(
                            "no route to the {}'s connection at {}, blocked by {} tile(s) from \
                             {first}",
                            to.name,
                            sink.junction,
                            blocked.len()
                        ),
                        None => format!(
                            "no route to the {}'s connection at {}, and nothing on the searched \
                             grid blocked it: the two are further apart than one window reaches",
                            to.name, sink.junction
                        ),
                    });
                }
                Err(RouteError::SpanTooLong { needed, max }) => {
                    last = Some(format!(
                        "the obstacle needs an underground span of {needed} tiles and the pipe \
                         allows {max}"
                    ));
                }
            }
        }
    }
    Err(refuse(last.unwrap_or_else(|| {
        format!("the {} declares no pipe connection to aim at", to.name)
    })))
}

/// The cell `at` falls in, or `None` when it lies outside the window.
///
/// The same body as `method::connect`'s, which is private to that module.
fn cell_of(origin: (f64, f64), at: &Position) -> Option<(usize, usize)> {
    let x = (at.x() - origin.0) / enclosure::CELL;
    let y = (at.y() - origin.1) / enclosure::CELL;
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    (x < enclosure::GRID && y < enclosure::GRID).then_some((x, y))
}

/// The footprints of everything this plan has already put on the ground
/// inside `area` -- the overlay half of the obstacle grid, which
/// `EntityGraph::blocking_boxes_within` cannot see.
fn overlay_boxes(state: &PlanState, area: &Rect) -> Vec<Rect> {
    let centre = Position::new(
        (area.left_top.x() + area.right_bottom.x()) / 2.,
        (area.left_top.y() + area.right_bottom.y()) / 2.,
    );
    let radius = (area.width() / 2.).hypot(area.height() / 2.);
    state
        .entities_within(&centre, radius)
        .into_iter()
        .filter_map(|entity| {
            <Direction as factorio_bot_core::num_traits::FromPrimitive>::from_u8(entity.direction)
                .and_then(|facing| {
                    state.collision_area_facing(&entity.name, &entity.position, facing)
                })
                .or_else(|| {
                    let box_ = &entity.bounding_box;
                    (box_.width() > 0. && box_.height() > 0.).then(|| box_.clone())
                })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// The `FactorioEntity` a placement of `name` at `position` creates, facing
/// north.
///
/// `entity_type` is read from the prototype rather than guessed, for the
/// reason `extract::extractor_entity` gives: `EntityGraph::add` keys its
/// whitelist on the `(name, type)` pair, and a tank's type is `storage-tank`
/// while a pumpjack's is `mining-drill`.
pub(crate) fn plain_entity(state: &PlanState, name: &str, position: &Position) -> FactorioEntity {
    let entity_type = state
        .base()
        .globals
        .entity_prototypes
        .get(name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| name.to_string());
    FactorioEntity {
        name: name.to_string(),
        entity_type,
        position: position.clone(),
        direction: NORTH,
        ..Default::default()
    }
}

/// One `Place`, with the preconditions and effects every other method's
/// placements carry, and the overlay update that makes the next one see it.
///
/// The same body as `method::connect`'s `place_step`, which is private there.
pub(crate) fn place_step(ctx: &mut ExpansionCtx, entity: FactorioEntity, note: &str) -> Step {
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
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
    ctx.state.create_entity(entity);
    step
}

#[cfg(test)]
mod pipe_tests {
    use super::*;
    use crate::ids::BotId;
    use crate::test_world::{OilFixture, PumpjackRecipe, world_with_oil};
    use std::sync::Arc;

    const OIL: OilFixture = OilFixture {
        wells: true,
        categories: true,
        pumpjack: PumpjackRecipe::LockedBy { researched: true },
        prerequisite: false,
    };

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(world_with_oil(OIL)), &[BotId(1)])
    }

    /// Overwrite the filter on every supplying box of `name`, the way the mod
    /// now reports it. The fixture capture predates the field, so everything
    /// in it reads [`FluidFilter::Unknown`] -- which is exactly the control
    /// these tests need.
    fn filter_supplying(state: &PlanState, name: &str, filter: &FluidFilter) {
        let mut proto = state
            .base()
            .globals
            .entity_prototypes
            .get_mut(name)
            .expect("the fixture has this prototype");
        for b in proto
            .fluidbox_prototypes
            .as_mut()
            .expect("the fixture prototype has fluid boxes")
            .iter_mut()
            .filter(|b| SUPPLYING.contains(&b.production_type.as_str()))
        {
            b.filter = filter.clone();
        }
    }

    fn standing(state: &mut PlanState, name: &str, at: Position) {
        state.create_entity(FactorioEntity {
            name: name.into(),
            entity_type: name.into(),
            position: at,
            direction: 0,
            ..Default::default()
        });
    }

    fn is_source(state: &PlanState, fluid: &str, name: &str) -> bool {
        let (attributable, _rejected) = sources_of(state, fluid, &Position::new(0., 0.));
        attributable.iter().any(|e| e.name == name)
    }

    /// **A box filtered to the fluid is a source with no inference at all.**
    ///
    /// Steam is the live case, verified against a seed-31337 dump: a boiler
    /// carries no recipe, stands on no charted resource -- steam is not one --
    /// and is not a buffer, so all three inference rules fail on it and always
    /// would have. Its output box really is `only=steam`, so this test asserts
    /// a fact about the shipped mod set and not a hypothetical.
    ///
    /// **The exemplar used to be an offshore pump and water, and that was
    /// wrong** -- a 2.0 pump takes its fluid from the tile in front of it, so
    /// its box is unfiltered and the filter never rescued water. The mechanism
    /// this test pins is unaffected; the claim about which fluid it rescued
    /// was not. Water is answered by [`drawn_from_ground`] instead.
    ///
    /// The control is the same boiler carrying [`FluidFilter::Unknown`], the
    /// filter the fixture capture actually holds, which must still be refused:
    /// without it this test would pass for a version that had simply stopped
    /// attributing anything.
    #[test]
    fn a_box_filtered_to_the_fluid_is_a_source_where_no_inference_reaches() {
        let mut state = state();
        // Well away from the crude field, so the footprint inference cannot
        // fire for any fluid.
        standing(&mut state, "boiler", Position::new(60.5, 60.5));

        assert!(
            !is_source(&state, "steam", "boiler"),
            "an unread filter must attribute nothing -- the three inferences \
             cannot see steam, and water used to sit in the same gap"
        );

        filter_supplying(
            &state,
            "boiler",
            &FluidFilter::Only {
                fluid: "steam".to_owned(),
            },
        );
        assert!(
            is_source(&state, "steam", "boiler"),
            "a box that names steam IS a steam source"
        );
    }

    /// **A box filtered to another fluid is refused even where an inference
    /// would have accepted it.**
    ///
    /// This is the boiler that motivated the three tests in the first place:
    /// a refinery piped to a boiler builds perfectly and makes nothing. Here
    /// the boiler stands on the crude field, so inference 2 -- "its footprint
    /// covers a charted tile of the fluid" -- accepts it, and the control
    /// half of this test proves that it does. A box that names `steam`
    /// definitely does not supply crude, and that fact now outranks the
    /// inference rather than being unavailable to it.
    #[test]
    fn a_box_filtered_to_another_fluid_outranks_an_inference_that_would_accept() {
        let mut state = state();
        standing(&mut state, "boiler", Position::new(20.5, 20.5));

        assert!(
            is_source(&state, "crude-oil", "boiler"),
            "control: standing on the field, the footprint inference accepts \
             this boiler -- which is the defect, not the desired answer"
        );

        filter_supplying(
            &state,
            "boiler",
            &FluidFilter::Only {
                fluid: "steam".to_owned(),
            },
        );
        assert!(
            !is_source(&state, "crude-oil", "boiler"),
            "a box that names steam is not a crude source, whatever it stands on"
        );
    }

    /// **[`FluidFilter::Any`] is not [`FluidFilter::Only`] and not a veto.**
    ///
    /// An unfiltered box -- every crafting machine, and a storage tank -- is a
    /// definite answer that says nothing about *which* fluid, so the three
    /// inferences must still decide. Pinned separately from `Unknown` so that
    /// collapsing the two states would fail something: the two agree here on
    /// purpose, and a reader should see that the agreement was chosen.
    #[test]
    fn an_unfiltered_box_neither_attributes_nor_vetoes() {
        let mut on_field = state();
        standing(&mut on_field, "boiler", Position::new(20.5, 20.5));
        filter_supplying(&on_field, "boiler", &FluidFilter::Any);
        assert!(
            is_source(&on_field, "crude-oil", "boiler"),
            "`any` leaves the inferences in charge"
        );

        // And the live shape of the thing that made the old exemplar wrong:
        // an offshore pump's output box really is `any`, and `any` attributes
        // nothing by itself. Water is answered now, but by the GROUND -- see
        // `a_pump_draws_the_fluid_the_tile_in_front_of_it_yields`; this pump
        // has no `fluid_source_offset` and stands on nothing charted, so the
        // filter alone still says nothing about it.
        let mut elsewhere = state();
        standing(&mut elsewhere, "offshore-pump", Position::new(60.5, 60.5));
        filter_supplying(&elsewhere, "offshore-pump", &FluidFilter::Any);
        assert!(
            !is_source(&elsewhere, "water", "offshore-pump"),
            "`any` attributes nothing by itself"
        );
    }

    /// Give `name` a `fluid_source_offset`, the way the mod now reports it for
    /// an offshore pump. The fixture capture predates the field, so every
    /// prototype in it reads `None` -- which is the control these tests need.
    fn draws_from(state: &PlanState, name: &str, offset: Position) {
        state
            .base()
            .globals
            .entity_prototypes
            .get_mut(name)
            .expect("the fixture has this prototype")
            .fluid_source_offset = Some(offset);
    }

    /// Chart one tile, at its corner, with the fluid it yields.
    fn ground(state: &PlanState, x: f64, y: f64, fluid: TileFluid) {
        state
            .base()
            .entity_graph
            .add_tiles(
                vec![factorio_bot_core::types::FactorioTile {
                    name: match &fluid {
                        TileFluid::Yields { .. } => "water",
                        _ => "grass-1",
                    }
                    .to_owned(),
                    player_collidable: matches!(fluid, TileFluid::Yields { .. }),
                    position: Position::new(x, y),
                    color: None,
                    surface: None,
                    fluid,
                }],
                None,
            )
            .expect("charting one tile cannot fail");
    }

    fn water() -> TileFluid {
        TileFluid::Yields {
            fluid: "water".to_owned(),
        }
    }

    /// **The ground answers water, where the box filter and all three
    /// inferences cannot.**
    ///
    /// This is the hole [`sources_of`] carried in its own doc for two
    /// sessions. Water is not a charted resource, an offshore pump carries no
    /// recipe and is not a buffer, and its output box is honestly
    /// [`FluidFilter::Any`] -- so every other rule in `attributable_to` fails
    /// on it and always would have.
    ///
    /// **Three states, asserted in one test on purpose**, because the whole
    /// design is that they are three and not two: an uncharted source tile
    /// attributes nothing, a charted dry one attributes nothing, and only a
    /// tile that names the fluid does. Splitting them would let a version that
    /// collapsed `Unknown` into `Dry` pass both halves.
    #[test]
    fn a_pump_draws_the_fluid_the_tile_in_front_of_it_yields() {
        let mut state = state();
        // Well away from the crude field, so no inference can fire.
        standing(&mut state, "offshore-pump", Position::new(60.5, 60.5));
        filter_supplying(&state, "offshore-pump", &FluidFilter::Any);

        assert!(
            !is_source(&state, "water", "offshore-pump"),
            "control: with no `fluid_source_offset` on the prototype this is              the pre-2026-09-08 world, and water has no answer at all"
        );

        draws_from(&state, "offshore-pump", Position::new(0., -1.));
        assert!(
            !is_source(&state, "water", "offshore-pump"),
            "the offset alone says WHICH tile, never what is in it -- and the              tile is uncharted, so `TileFluid::Unknown` must attribute nothing"
        );

        // The tile the offset names: north of a pump at (60.5, 60.5), by its
        // corner, is (60, 59).
        ground(&state, 60., 59., water());
        assert!(
            is_source(&state, "water", "offshore-pump"),
            "a pump in front of water IS a water source"
        );
        assert!(
            !is_source(&state, "steam", "offshore-pump"),
            "and it is a source of that fluid only"
        );
    }

    /// **Charted dry ground REFUSES a pump an inference would have accepted.**
    ///
    /// The ground is an observation, not a guess, so it outranks the three
    /// inferences below it exactly as the box filter does -- and a
    /// ground-drawing entity has no other source, since its box is unfiltered
    /// and it carries no recipe.
    ///
    /// The control is the same pump with the same footprint over the same
    /// crude field and no `fluid_source_offset`: inference 2 accepts it, which
    /// is the wrong answer this rule exists to outrank. Without that half the
    /// test would pass for a version that had simply stopped attributing
    /// anything.
    #[test]
    fn dry_ground_outranks_the_footprint_inference() {
        let mut state = state();
        standing(&mut state, "offshore-pump", Position::new(20.5, 20.5));
        filter_supplying(&state, "offshore-pump", &FluidFilter::Any);
        assert!(
            is_source(&state, "crude-oil", "offshore-pump"),
            "control: standing on the crude field, the footprint inference              accepts this pump -- which is the defect, not the desired answer"
        );

        draws_from(&state, "offshore-pump", Position::new(0., -1.));
        ground(&state, 20., 19., TileFluid::Dry);
        assert!(
            !is_source(&state, "crude-oil", "offshore-pump"),
            "a pump facing dry land draws nothing, whatever it stands on"
        );
    }

    /// **The offset is in the pump's own frame, so its DIRECTION decides which
    /// tile is read.**
    ///
    /// Two pumps, one lake, one facing it and one not. Without the rotation a
    /// north-frame `{0, -1}` would read the tile above both of them and this
    /// test's two halves would return the same answer.
    #[test]
    fn the_source_tile_turns_with_the_pump() {
        // Facing east (Factorio 2.x direction 4): the source tile is the one
        // to the pump's east, at (61, 60).
        let mut facing_the_lake = state();
        facing_the_lake.create_entity(FactorioEntity {
            name: "offshore-pump".into(),
            entity_type: "offshore-pump".into(),
            position: Position::new(60.5, 60.5),
            direction: 4,
            ..Default::default()
        });
        filter_supplying(&facing_the_lake, "offshore-pump", &FluidFilter::Any);
        draws_from(&facing_the_lake, "offshore-pump", Position::new(0., -1.));
        ground(&facing_the_lake, 61., 60., water());
        assert!(
            is_source(&facing_the_lake, "water", "offshore-pump"),
            "facing east, the pump reads the tile to its east"
        );

        // The same lake tile, the same offset, a pump facing north instead:
        // it reads (60, 59), which nobody charted.
        let mut facing_away = state();
        standing(&mut facing_away, "offshore-pump", Position::new(60.5, 60.5));
        filter_supplying(&facing_away, "offshore-pump", &FluidFilter::Any);
        draws_from(&facing_away, "offshore-pump", Position::new(0., -1.));
        ground(&facing_away, 61., 60., water());
        assert!(
            !is_source(&facing_away, "water", "offshore-pump"),
            "facing north, the tile to its east is not what it draws from"
        );
    }

    /// **An entity with no `fluid_source_offset` is not judged by the ground
    /// at all**, and that absence is the discriminator -- not a name.
    ///
    /// A boiler standing beside the same lake must go on being judged by the
    /// three inferences, because every other supplying entity in the game is.
    /// The runtime API marks `fluid_source_offset`
    /// `subclasses: ["OffshorePump"]`, so the boiler genuinely has none.
    #[test]
    fn an_entity_that_does_not_draw_from_the_ground_is_unaffected_by_it() {
        let mut state = state();
        standing(&mut state, "boiler", Position::new(20.5, 20.5));
        ground(&state, 20., 19., TileFluid::Dry);
        assert!(
            is_source(&state, "crude-oil", "boiler"),
            "the boiler has no fluid source offset, so dry ground in front of              it says nothing -- the footprint inference still decides"
        );
    }

    // -----------------------------------------------------------------------
    // Which box takes which fluid
    // -----------------------------------------------------------------------

    /// A recipe with the named fluid ingredients, each with an optional
    /// explicit `fluidbox_index`, and no products.
    fn recipe_taking(fluids: &[(&str, Option<u32>)]) -> FactorioRecipe {
        FactorioRecipe {
            name: "probe".into(),
            valid: true,
            enabled: true,
            category: "chemistry".into(),
            ingredients: Some(
                fluids
                    .iter()
                    .map(
                        |(name, index)| factorio_bot_core::types::FactorioIngredient {
                            name: (*name).into(),
                            ingredient_type: "fluid".into(),
                            amount: 30,
                            fluidbox_index: *index,
                        },
                    )
                    .collect(),
            ),
            products: vec![],
            hidden: false,
            energy: Box::new(
                factorio_bot_core::num_traits::cast::FromPrimitive::from_f64(1.0).expect("finite"),
            ),
            order: "a".into(),
            group: "g".into(),
            subgroup: "s".into(),
        }
    }

    /// How many input boxes the fixture's `oil-refinery` declares -- asserted
    /// rather than assumed, because every rule below is stated in terms of it.
    #[test]
    fn the_fixture_refinery_declares_two_input_boxes() {
        let state = state();
        assert_eq!(
            box_count(&state, "oil-refinery", "input"),
            Some(2),
            "the rules below are about a machine with two input boxes"
        );
    }

    /// **Rule 2, and the sulfur case.** Two fluids into a machine declaring
    /// two input boxes is the *k*th fluid on the *k*th box -- measured on a
    /// live 2.1.17 server, where a standing `chemical-plant` set to `sulfur`
    /// reported `water` on box 1 and `petroleum-gas` on box 2.
    ///
    /// The order is asserted, not just the set: a resolver that handed back
    /// `[1, 0]` would satisfy "each box used once" and pipe both fluids to
    /// the wrong port.
    #[test]
    fn matching_counts_put_the_kth_fluid_on_the_kth_box() {
        let state = state();
        let assigned = fluid_box_ordinals(
            &state,
            "oil-refinery",
            &recipe_taking(&[("water", None), ("petroleum-gas", None)]),
            "input",
        )
        .expect("two fluids into two boxes is decidable");
        assert_eq!(
            assigned,
            vec![("water".to_string(), 0), ("petroleum-gas".to_string(), 1)]
        );
    }

    /// **Rule 1, and the counter-example that killed the positional claim.**
    /// `basic-oil-processing` declares `fluidbox_index = 2` for crude oil,
    /// and a live refinery set to it puts crude on input box 2 while box 1 --
    /// the box positional picks -- carries nothing at all.
    ///
    /// The control is the same recipe with the field absent, which is what
    /// every dump taken before 2026-09-08 reads: it answers 0, the wrong box,
    /// and that is stated here so the difference the field makes is visible
    /// rather than assumed.
    #[test]
    fn an_explicit_fluidbox_index_outranks_position() {
        let state = state();
        let declared = fluid_box_ordinals(
            &state,
            "oil-refinery",
            &recipe_taking(&[("crude-oil", Some(2))]),
            "input",
        )
        .expect("an explicit index is always decidable");
        assert_eq!(declared, vec![("crude-oil".to_string(), 1)]);

        let silent = fluid_box_ordinals(
            &state,
            "oil-refinery",
            &recipe_taking(&[("crude-oil", None)]),
            "input",
        )
        .expect("one fluid always has an answer");
        assert_eq!(
            silent,
            vec![("crude-oil".to_string(), 0)],
            "without the field the old, wrong box is what comes back"
        );
    }

    /// `fluidbox_index` is 1-based, so `0` is not an index -- it is a sender
    /// that said nothing usable, and it must not become box 0 by arithmetic.
    /// The `absent is not a value` rule, in the one place a nonsense value
    /// could quietly become a plausible one.
    #[test]
    fn a_zero_fluidbox_index_is_not_box_zero_by_accident() {
        let state = state();
        let assigned = fluid_box_ordinals(
            &state,
            "oil-refinery",
            &recipe_taking(&[("water", None), ("petroleum-gas", Some(0))]),
            "input",
        )
        .expect("the counts still match, so rule 2 answers");
        assert_eq!(
            assigned,
            vec![("water".to_string(), 0), ("petroleum-gas".to_string(), 1)],
            "a 0 falls through to the positional rule rather than naming box 0"
        );
    }

    /// **Rule 3, and the case that still refuses.** A machine declaring more
    /// input boxes than the recipe has fluids merges the surplus, and the
    /// merge is not positional: a live `cryogenic-plant` running
    /// `fluoroketone` (three boxes, two fluids) merged boxes 1 and 2 for
    /// fluorine and gave ammonia box 3.
    ///
    /// So the first fluid is answered -- it landed on box 0 in every case
    /// measured -- and the second refuses by name. Both halves are asserted,
    /// because a resolver that refused *everything* under this rule would
    /// pass a test that only checked the refusal.
    #[test]
    fn a_surplus_of_boxes_answers_only_the_first_fluid() {
        let state = state();
        let one = fluid_box_ordinals(
            &state,
            "oil-refinery",
            &recipe_taking(&[("crude-oil", None)]),
            "input",
        )
        .expect("one fluid into two boxes is answered");
        assert_eq!(one, vec![("crude-oil".to_string(), 0)]);

        // Three fluids into two boxes is the other side of "the counts
        // differ", and the second one has no answer either.
        let undecidable = fluid_box_ordinals(
            &state,
            "oil-refinery",
            &recipe_taking(&[("water", None), ("steam", None), ("crude-oil", None)]),
            "input",
        )
        .expect_err("three fluids into two boxes is not decidable");
        assert_eq!(undecidable.fluid, "steam");
        assert_eq!(undecidable.ordinal, 2);
        assert_eq!(undecidable.fluids, 3);
        assert_eq!(undecidable.boxes, Some(2));
    }

    /// A machine this world has never heard of has no box count, and that is
    /// `None` rather than zero -- so the first fluid still answers by rule 3
    /// and the second refuses, saying it does not know the count.
    #[test]
    fn an_unknown_machine_reports_an_unknown_box_count() {
        let state = state();
        let undecidable = fluid_box_ordinals(
            &state,
            "no-such-machine",
            &recipe_taking(&[("water", None), ("petroleum-gas", None)]),
            "input",
        )
        .expect_err("nothing is known about this machine's boxes");
        assert_eq!(undecidable.boxes, None, "unknown, never zero");
        assert_eq!(undecidable.fluid, "petroleum-gas");
    }

    /// `port_index` selects a *fluidbox*, not a position in the returned
    /// list, and the two are different the moment one box declares several
    /// pipe connections. A `storage-tank` is exactly that case in the live
    /// capture: one box, four connections.
    #[test]
    fn every_port_of_a_multi_connection_box_carries_that_boxs_ordinal() {
        let state = state();
        let ports = fluid_ports(&state, "storage-tank", &Position::new(24.5, 26.5), None)
            .expect("a tank has ports");
        assert!(!ports.is_empty(), "the fixture tank declares ports");
        assert!(
            ports.iter().all(|port| port.box_ordinal == 0),
            "a tank has one fluidbox, so every port belongs to box 0: {ports:?}"
        );
        assert!(
            select(ports.clone(), Some(1)).is_empty(),
            "there is no second box, so selecting one hands back nothing rather than a \
             neighbour's port"
        );
        assert_eq!(
            select(ports.clone(), Some(0)).len(),
            ports.len(),
            "selecting box 0 keeps every port of that box, not just the first"
        );
    }

    fn tank(state: &mut PlanState, at: Position) {
        state.create_entity(FactorioEntity {
            name: "storage-tank".into(),
            entity_type: "storage-tank".into(),
            position: at,
            direction: 0,
            ..Default::default()
        });
    }

    /// **A pipe already standing is a join, not an obstacle.** Measured, not
    /// argued: the second run out of a tank the first run filled refused with
    /// *"the tile at ... cannot hold a pipe"*, which reads as blocked ground
    /// and is the opposite of the truth.
    ///
    /// The control is the same route with a *stone furnace* on that tile,
    /// which must still refuse -- otherwise this test would pass for a
    /// version that ignored obstacles altogether.
    #[test]
    fn a_standing_pipe_is_a_join_and_a_furnace_is_not() {
        let from = Position::new(24.5, 26.5);
        let to = Position::new(24.5, 36.5);
        let ends = |state: &PlanState| {
            let a = state.collision_area("storage-tank", &from).expect("a box");
            let b = state.collision_area("storage-tank", &to).expect("a box");
            (a, b)
        };

        let mut plain = state();
        tank(&mut plain, from.clone());
        tank(&mut plain, to.clone());
        let (a, b) = ends(&plain);
        let clean = route_between(
            &plain,
            &PipeEnd {
                name: "storage-tank",
                position: &from,
                area: a.clone(),
                production_type: None,
                port_index: None,
            },
            &PipeEnd {
                name: "storage-tank",
                position: &to,
                area: b.clone(),
                production_type: None,
                port_index: None,
            },
            "pipe",
            &[],
        )
        .expect("open ground routes");
        assert!(!clean.is_empty(), "the run has tiles");

        // A pipe on one of those very tiles: the route still answers, and the
        // occupied tile is no longer emitted -- it is already there.
        let occupied = clean[clean.len() / 2].clone();
        let mut with_pipe = state();
        tank(&mut with_pipe, from.clone());
        tank(&mut with_pipe, to.clone());
        with_pipe.create_entity(FactorioEntity {
            name: "pipe".into(),
            entity_type: "pipe".into(),
            position: occupied.clone(),
            direction: 0,
            ..Default::default()
        });
        let joined = route_between(
            &with_pipe,
            &PipeEnd {
                name: "storage-tank",
                position: &from,
                area: a.clone(),
                production_type: None,
                port_index: None,
            },
            &PipeEnd {
                name: "storage-tank",
                position: &to,
                area: b.clone(),
                production_type: None,
                port_index: None,
            },
            "pipe",
            &[],
        )
        .expect("a standing pipe does not block a pipe run");
        assert!(
            !joined.contains(&occupied),
            "the tile that already holds a pipe is not placed again"
        );

        // The control: a furnace on the same tile is a real obstacle.
        let mut with_furnace = state();
        tank(&mut with_furnace, from.clone());
        tank(&mut with_furnace, to.clone());
        with_furnace.create_entity(FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: occupied.clone(),
            direction: 0,
            ..Default::default()
        });
        let blocked = route_between(
            &with_furnace,
            &PipeEnd {
                name: "storage-tank",
                position: &from,
                area: a,
                production_type: None,
                port_index: None,
            },
            &PipeEnd {
                name: "storage-tank",
                position: &to,
                area: b,
                production_type: None,
                port_index: None,
            },
            "pipe",
            &[],
        );
        if let Ok(tiles) = blocked {
            assert!(
                !tiles.contains(&occupied),
                "a furnace's tile is never piped over"
            );
        }
    }

    /// `port_index` picks the *n*th box of a direction, which is how a recipe's
    /// fluids map to a machine's fluidboxes. An `oil-refinery` declares two
    /// input boxes and three output boxes, so "any of them, nearest first"
    /// would be a coin flip.
    #[test]
    fn a_port_index_selects_one_box_and_none_selects_all() {
        let state = state();
        let at = Position::new(24.5, 36.5);
        let all = fluid_ports(&state, "oil-refinery", &at, Some("input")).expect("ports");
        assert_eq!(all.len(), 2, "the fixture refinery has two input boxes");
        let outputs = fluid_ports(&state, "oil-refinery", &at, Some("output")).expect("ports");
        assert_eq!(outputs.len(), 3, "and three output boxes");
        assert_eq!(select(all.clone(), Some(0)), vec![all[0].clone()]);
        assert_eq!(select(all.clone(), Some(1)), vec![all[1].clone()]);
        assert!(
            select(all.clone(), Some(2)).is_empty(),
            "an index past the last box selects nothing, and route_between refuses"
        );
        assert_eq!(select(all.clone(), None), all);
    }

    /// A machine sited flush against its source has its port **inside** the
    /// source. Measured: a refinery beside its tank put its first input port
    /// on a tile the tank stands on.
    #[test]
    fn a_port_inside_another_machine_is_not_placeable() {
        let mut state = state();
        let tank_at = Position::new(24.5, 26.5);
        tank(&mut state, tank_at.clone());
        // The refinery's first input port is at (-1, +3) from its centre, so
        // this site puts that tile inside the tank's 3x3 footprint.
        let flush = Position::new(25.5, 23.5);
        assert!(
            !port_is_placeable(
                &state,
                "oil-refinery",
                &flush,
                Some("input"),
                Some(0),
                "pipe"
            ),
            "the port lands inside the tank"
        );
        let clear = Position::new(35.5, 45.5);
        assert!(
            port_is_placeable(
                &state,
                "oil-refinery",
                &clear,
                Some("input"),
                Some(0),
                "pipe"
            ),
            "open ground is placeable, so the assertion above is about the tank"
        );
    }
}
