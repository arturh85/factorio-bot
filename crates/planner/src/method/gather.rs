//! Gathering: a tank at the patch, and pipe from the wellhead into it.
//!
//! The rung above [`crate::method::extract`], and the smaller half of the
//! topology the owner described on 2026-09-06
//! (`docs/superpowers/notes/2026-09-06-how-oil-is-actually-played.md`):
//!
//! ```text
//! pumpjacks --pipes--> TANK ======= long trunk =======> TANK --> refinery
//! (one per well)     (at the patch)                  (at the base)
//! ```
//!
//! **This module builds the left-hand half only**: one pumpjack, one tank
//! sited for the field, and the pipe between them. The trunk is the next rung
//! and is deliberately not here -- see "What this does not do" below.
//!
//! # Why a tank is a precondition and not an optimisation
//!
//! No character inventory can hold a fluid ([`crate::substance`], and
//! `docs/superpowers/notes/2026-09-06-a-fluid-is-not-an-item.md`). A pumpjack
//! with nothing connected fills its own output fluidbox and stops, so before a
//! tank stands there is nowhere in the world for crude to *be* -- not for the
//! game, and not for this planner's model either. Everything above this rung
//! starts from a tank.
//!
//! # Three things this method is *not* doing, on purpose
//!
//! Each was a real risk of over-building, and each was ruled out by the note:
//!
//! * **No power plant at the well.** A wellhead needs a pumpjack and a tank.
//!   No refinery, therefore no water, therefore no plant. The refusal that
//!   cost this lane a day -- *"a power plant needs water, and the plan can see
//!   none within 128 tiles"* -- came from siting a plant at the consumer.
//!   Power crosses on poles, and [`crate::method::power::ensure_powered`] (via
//!   `extract`) is the one place that decides how.
//! * **No flow model.** 2.1's pipe bandwidth carries a patch down one trunk,
//!   so what is needed is connectivity and direction, not throughput. There is
//!   no rate arithmetic anywhere in this file.
//! * **No pump.** A pumpjack pushes into whatever is connected to its output;
//!   a pump would only be needed to lift fluid over distance or to enforce a
//!   direction, and neither applies between two machines a dozen tiles apart.
//!   That matters because **a pump's two pipe connections are its
//!   directionality**, and one placed backwards builds 100% correctly and
//!   moves nothing -- the silent class this repo has already paid for once
//!   with inserters. The cheapest way not to get that convention wrong is not
//!   to need it, which is the position this module takes.
//!
//! # The ladder, in the four-tier style `method::extract` uses
//!
//! Every refusal names the next missing thing, in the order a reader can act
//! on them, and **all four are returned before a single action is emitted**
//! -- the same promise `method::connect`'s `ConnectRefusal` makes, and for the
//! same reason: half a pipe run is worse than none, because the machine at the
//! near end fills up and stops with nothing to show for the iron.
//!
//! 1. everything [`crate::method::extract`] refuses -- the resource is not
//!    charted, nothing mines it, its extractor is locked, no well tile is
//!    free, or no power reaches one;
//! 2. no prototype in this world buffers a fluid, or none carries one --
//!    [`PlannerError::NoFluidBuffer`], found by `entity_type` rather than by
//!    the name `storage-tank`, so a modded tank answers and a capture that
//!    predates the field refuses instead of silently picking nothing;
//! 3. no clear footprint for the tank within [`TANK_SEARCH_RADIUS`] of the
//!    field centroid -- [`PlannerError::NoTankSite`], naming the centroid, how
//!    many wells it averaged and what stood in the way;
//! 4. no pipe route between the two -- [`PlannerError::NoPipeRoute`].
//!
//! A fifth, [`PlannerError::FluidPortUnknown`], is not a tier: it says the
//! prototype table cannot tell this module where a machine takes fluid in or
//! out, which is a fact about the *capture* rather than about the map.
//!
//! # Where the tank goes: the centroid of a FIELD, not of a patch
//!
//! The long distance is paid once per patch, not once per well, so the siting
//! question is "where does this field's tank go". The obvious implementation
//! -- average [`PlanState::resource_patches`] -- **is wrong for oil**, and
//! silently: `EntityGraph`'s patches come from a flood fill over *adjacent*
//! tiles, and crude-oil wells are never adjacent. On seed 31337's explored
//! dump the seven charted wells are seven patches of one tile each, so a
//! per-patch centroid is just the well itself and the whole idea collapses
//! into "put the tank next to the pumpjack".
//!
//! So this module defines its own neighbourhood: the **field** is every
//! charted tile of the resource within [`FIELD_RADIUS`] of the wellhead, and
//! the anchor is that set's centroid. [`the_field_is_not_the_graphs_patch`]
//! pins the distinction against the real dump's geometry.
//!
//! # What this does not do
//!
//! * **One pumpjack, not all of them.** The tank is sited for the field --
//!   that is the point of the centroid -- but only the nearest well is worked.
//!   Standing a pumpjack on every well is a loop over this same code with a
//!   pole run each, and it multiplies a 2,000-action plan; it is the obvious
//!   next increment and it is not in this one.
//! * **No trunk.** The second tank at the base, and the long run between them,
//!   is the other half of the topology. Its design question is not this one's:
//!   a run of hundreds of tiles cannot be routed on a single 48x48
//!   `enclosure::window`, so it needs either a coarser search or a chain of
//!   windows, and that is a decision worth taking on its own.
//! * **No idempotence for the pumpjack.** A tank already standing in the field
//!   is adopted (see [`expand`](Gather::expand)), so a replan does not stack
//!   tanks; the pumpjack is `method::extract`'s to decide and this module does
//!   not second-guess it.

use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::method::extract;
use crate::method::have::PLACE_TICKS;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::graph::enclosure;
use factorio_bot_core::graph::route::{RouteError, TileKind, route_belt};
use factorio_bot_core::types::{Direction, FactorioEntity, Position, Rect};

use crate::action::{Action, ActionKind, Actor, Condition, Effect};

/// `Direction::North` as the wire byte an emitted entity carries.
///
/// A storage tank is symmetric -- its four pipe connections sit in two
/// diagonally opposite pairs, so rotating one maps the set onto itself -- and
/// a pipe has a connection on all four sides. Neither has a facing that
/// changes what it does, so both are placed north and nothing here computes a
/// direction. That is a deliberate contrast with the pump this module refuses
/// to need.
const NORTH: u8 = 0;

/// How far from the wellhead a tile counts as the same **field**.
///
/// Chosen against the routing window rather than against geology, and the
/// constraint is worth stating because it is the only thing that bounds it: a
/// pipe run is searched on one [`enclosure::window`], which reaches
/// `enclosure::SEARCH_RADIUS` (24) tiles from its origin. The centroid of a
/// set of tiles within `R` of the wellhead is itself within `R` of it, so
/// `R` must leave room for the tank's own footprint and for a route that is
/// not a straight line. **A nearer tank is a worse plan; an unroutable one is
/// no plan at all**, so this errs low.
///
/// On seed 31337's explored dump this takes 5 of the 7 charted wells into the
/// field and leaves the two eastmost out — which is the honest answer for a
/// module that only pipes one wellhead anyway.
pub const FIELD_RADIUS: f64 = 20.0;

/// How far from the field centroid a tank footprint is looked for.
///
/// Small on purpose. The centroid is the *right* place for the tank and every
/// tile away from it is a longer pipe for every future pumpjack on the field,
/// so a search that wandered far would be quietly answering a different
/// question. Refusing by name and letting a caller move the goal is the better
/// failure.
pub const TANK_SEARCH_RADIUS: f64 = 12.0;

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

/// The method that claims [`Goal::Gathered`]. See the module doc.
pub struct Gather;

impl Method for Gather {
    fn name(&self) -> &'static str {
        "gather"
    }

    /// The same cheap half of the ladder [`extract::Extract::applicable`]
    /// answers, and for the same reason: whether the ground is clear, whether
    /// power reaches and whether a pipe routes are findings this method has to
    /// do the work to discover, and a caller is better served by `expand`'s
    /// named error than by a bare `NoApplicableMethod`.
    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Gathered { entity, .. } = goal else {
            return false;
        };
        state.has_resource_patches(entity) && extract::extractor_for(state, entity).is_ok()
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Gathered { entity, unlocks } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let origin = extract::origin_of(ctx);
        // Tier 1, in full, exactly as `Extract::expand` asks it: reachable
        // from a test or a caller that never consulted `applicable`.
        if let Some(refusal) = extract::world_refusal(&ctx.state, entity, &origin) {
            return Err(refusal);
        }
        let sited = extract::site_extractor(&ctx.state, entity, &origin)?;

        // Tier 2.
        let tank = buffer_prototype(&ctx.state, entity, &sited.name)?;
        let pipe = pipe_prototype(&ctx.state, entity, &sited.name)?;

        // Tier 3. The field, then the tank on it. An existing tank in the
        // field is adopted rather than duplicated, so a replan over a
        // half-built wellhead does not stack tanks -- the same shape
        // `Goal::Built` takes, where expanding means "the entities not yet
        // standing".
        let field = field_of(&ctx.state, entity, &sited.site);
        let centroid = centroid_of(&field);
        let standing = standing_buffer(&ctx.state, &tank, &centroid);
        let tank_site = match &standing {
            Some(position) => position.clone(),
            None => choose_tank_site(&ctx.state, &tank, entity, &centroid, &field, &sited)?,
        };
        let Some(tank_area) = ctx.state.collision_area(&tank, &tank_site) else {
            return Err(PlannerError::FluidPortUnknown {
                prototype: tank.clone(),
                why: "the world has no collision box for it, so its footprint cannot be reserved"
                    .to_string(),
            });
        };

        // Tier 4. Every refusal above and below this line happens before
        // anything is emitted or reserved.
        let run = pipe_run(&ctx.state, &sited, &tank, &tank_site, &tank_area, &pipe)?;

        // -- nothing refuses past here ---------------------------------------

        // The pipe tiles are handed over as ground already spoken for: the
        // pole run is sited inside `extractor_steps`, sees no pipe (none is
        // emitted yet), and would otherwise be free to stand a pole on a tile
        // this run needs -- a plan that reads as good and whose pipe's own
        // `AreaFree` fails when it is executed.
        let reserved: Vec<FactorioEntity> = run
            .iter()
            .map(|position| plain_entity(&ctx.state, &pipe, position))
            .collect();
        let (mut steps, _place_id) =
            extract::extractor_steps(ctx, entity, &sited, unlocks.as_deref(), &reserved)?;

        if standing.is_none() {
            steps.push(Step::Subgoal(Goal::Have {
                item: tank.clone(),
                count: 1,
                whose: Holder::Share(ctx.chain_actor),
            }));
            let entity_to_place = plain_entity(&ctx.state, &tank, &tank_site);
            steps.push(place_step(
                ctx,
                entity_to_place,
                &format!("buffer {entity} at the {} field", entity),
            ));
        }

        let count = u32::try_from(run.len()).unwrap_or(u32::MAX);
        steps.push(Step::Subgoal(Goal::Have {
            item: pipe.clone(),
            count,
            whose: Holder::Share(ctx.chain_actor),
        }));
        for position in &run {
            let entity_to_place = plain_entity(&ctx.state, &pipe, position);
            steps.push(place_step(
                ctx,
                entity_to_place,
                &format!("carry {entity} from the {} into the {tank}", sited.name),
            ));
        }
        Ok(steps)
    }

    fn refusal(&self, goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
        let Goal::Gathered { entity, .. } = goal else {
            return None;
        };
        Some(extract::refusal_for(
            &ctx.state,
            entity,
            &extract::origin_of(ctx),
        ))
    }
}

// ---------------------------------------------------------------------------
// The field
// ---------------------------------------------------------------------------

/// Every charted tile of `entity` within [`FIELD_RADIUS`] of `wellhead`,
/// including the wellhead itself.
///
/// **Not [`PlanState::resource_patches`]**, and the module doc says why at
/// length: the graph's patches are a flood fill over adjacent tiles, and oil
/// wells are never adjacent, so every well is its own patch and a per-patch
/// centroid says nothing.
///
/// Deterministic: `resource_patches` has already sorted each patch's elements,
/// and this preserves that order.
pub(crate) fn field_of(state: &PlanState, entity: &str, wellhead: &Position) -> Vec<Position> {
    state
        .resource_patches(entity)
        .into_iter()
        .flat_map(|patch| patch.elements)
        .filter(|tile| calculate_distance(tile, wellhead) <= FIELD_RADIUS)
        .collect()
}

/// The arithmetic mean of `field`, snapped to the nearest tile centre.
///
/// Snapped because a 3x3 footprint stands on a tile centre and an unsnapped
/// mean is almost never one; doing it here rather than inside the search keeps
/// the anchor a single stated number that a refusal can quote.
///
/// An empty field answers `(0.5, 0.5)`, which no caller can reach: `field_of`
/// always contains the wellhead it was measured from.
pub(crate) fn centroid_of(field: &[Position]) -> Position {
    if field.is_empty() {
        return Position::new(0.5, 0.5);
    }
    let n = field.len() as f64;
    let x: f64 = field.iter().map(|p| p.x()).sum::<f64>() / n;
    let y: f64 = field.iter().map(|p| p.y()).sum::<f64>() / n;
    Position::new(x.floor() + 0.5, y.floor() + 0.5)
}

// ---------------------------------------------------------------------------
// Prototypes
// ---------------------------------------------------------------------------

/// The prototype that buffers a fluid: the `storage-tank`-typed entity this
/// world knows, first by name when there are several.
///
/// **Found by `entity_type`, never by the name `storage-tank`.** The name is
/// vanilla's; the type is the game's, and asking the world is what lets a
/// modded tank answer and a capture with no tank at all refuse by name.
fn buffer_prototype(state: &PlanState, fluid: &str, source: &str) -> Result<String, PlannerError> {
    prototype_of_type(state, "storage-tank").ok_or_else(|| PlannerError::NoFluidBuffer {
        fluid: fluid.to_string(),
        machine: source.to_string(),
        why: "no entity in this world has entity_type `storage-tank`".to_string(),
    })
}

/// The prototype that carries a fluid between two machines, chosen the same
/// way and refusing the same way.
fn pipe_prototype(state: &PlanState, fluid: &str, source: &str) -> Result<String, PlannerError> {
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
        .entity_prototypes
        .iter()
        .filter(|proto| proto.entity_type == entity_type)
        .map(|proto| proto.name.clone())
        .min()
}

/// A tank of `name` already standing within [`FIELD_RADIUS`] of the field
/// centroid, nearest first, or `None`.
///
/// The idempotence half: expanding this goal twice against a world where the
/// first expansion has been *executed* must not build a second tank.
fn standing_buffer(state: &PlanState, name: &str, centroid: &Position) -> Option<Position> {
    let mut standing: Vec<(f64, Position)> = state
        .entities_named(name)
        .into_iter()
        .map(|entity| {
            (
                calculate_distance(&entity.position, centroid),
                entity.position,
            )
        })
        .filter(|(distance, _)| *distance <= FIELD_RADIUS)
        .collect();
    standing.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.1.y.total_cmp(&b.1.y))
    });
    standing.into_iter().next().map(|(_, position)| position)
}

// ---------------------------------------------------------------------------
// Siting the tank
// ---------------------------------------------------------------------------

/// Where the field's tank goes: the nearest tile centre to `centroid` whose
/// footprint is clear, does not cover a well, and does not touch the
/// pumpjack's own ground.
///
/// **A well is treated as occupied even though the game would let a tank stand
/// on one.** Resources do not collide with buildings, so `is_area_free` says
/// yes -- and a tank on a well permanently destroys the only thing that makes
/// that tile worth anything. Refusing it here is a decision about the game,
/// not about collision.
///
/// Deterministic: candidates are generated in a fixed `(distance, x, y)` order
/// and the first acceptable one wins.
fn choose_tank_site(
    state: &PlanState,
    tank: &str,
    entity: &str,
    centroid: &Position,
    field: &[Position],
    sited: &extract::SitedExtractor,
) -> Result<Position, PlannerError> {
    let radius = TANK_SEARCH_RADIUS as i64;
    let mut candidates: Vec<(f64, Position)> = Vec::new();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let position = Position::new(centroid.x() + dx as f64, centroid.y() + dy as f64);
            let distance = calculate_distance(&position, centroid);
            if distance <= TANK_SEARCH_RADIUS {
                candidates.push((distance, position));
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.1.y.total_cmp(&b.1.y))
    });

    let mut obstruction: Option<String> = None;
    for (_, position) in &candidates {
        let Some(area) = state.collision_area(tank, position) else {
            break;
        };
        if boxes_overlap(&area, &sited.area) {
            obstruction.get_or_insert_with(|| format!("the {} this plan is siting", sited.name));
            continue;
        }
        if field.iter().any(|well| covers(&area, well)) {
            obstruction
                .get_or_insert_with(|| format!("a {entity} well, which a tank must not bury"));
            continue;
        }
        if state.is_site_refused(tank, position) {
            obstruction.get_or_insert_with(|| {
                "a footprint the game already refused a build at".to_string()
            });
            continue;
        }
        if state.is_area_free(tank, position) {
            return Ok(position.clone());
        }
        if let Some(occupant) = state.placement_occupant(tank, position, Direction::North) {
            obstruction.get_or_insert_with(|| occupant.to_string());
        }
    }
    Err(PlannerError::NoTankSite {
        tank: tank.to_string(),
        entity: entity.to_string(),
        wells: field.len(),
        centroid: centroid.to_string(),
        searched: TANK_SEARCH_RADIUS as i32,
        nearest_obstruction: obstruction
            .unwrap_or_else(|| "nothing -- no candidate tile was even generated".to_string()),
    })
}

/// Does `area` cover the tile centred at `point`?
fn covers(area: &Rect, point: &Position) -> bool {
    point.x() > area.left_top.x()
        && point.x() < area.right_bottom.x()
        && point.y() > area.left_top.y()
        && point.y() < area.right_bottom.y()
}

/// Do two footprints share any ground? Touching edges do not count -- two
/// machines may stand flush against each other.
fn boxes_overlap(a: &Rect, b: &Rect) -> bool {
    a.left_top.x() < b.right_bottom.x()
        && b.left_top.x() < a.right_bottom.x()
        && a.left_top.y() < b.right_bottom.y()
        && b.left_top.y() < a.right_bottom.y()
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
}

impl FluidPort {
    /// Every tile this port needs a pipe on, junction last.
    fn tiles(&self) -> Vec<Position> {
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
    for fluidbox in boxes {
        if let Some(wanted) = production_type
            && fluidbox.production_type != wanted
        {
            continue;
        }
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
            };
            if !ports.contains(&port) {
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
// The pipe run
// ---------------------------------------------------------------------------

/// Every tile the run needs a pipe on: both ports' tiles and the route
/// between their junctions, in placement order and without duplicates.
///
/// Touches no `ExpansionCtx` and reserves nothing, so every refusal it makes
/// leaves the plan exactly as it found it.
fn pipe_run(
    state: &PlanState,
    sited: &extract::SitedExtractor,
    tank: &str,
    tank_site: &Position,
    tank_area: &Rect,
    pipe: &str,
) -> Result<Vec<Position>, PlannerError> {
    let refuse = |why: String| PlannerError::NoPipeRoute {
        from: format!("the {} at {}", sited.name, sited.site),
        to: format!("the {tank} at {tank_site}"),
        why,
    };

    let source = fluid_ports(state, &sited.name, &sited.site, Some("output"))?;
    let source = source.first().expect("fluid_ports refuses an empty answer");
    let sinks = fluid_ports(state, tank, tank_site, None)?;

    let (area, origin) = enclosure::window(&sited.site);
    let mut blocked = enclosure::rasterize(
        state
            .base()
            .entity_graph
            .blocking_boxes_within(&area)
            .into_iter()
            .chain(overlay_boxes(state, &area))
            // Neither machine is in the overlay yet -- this function is asked
            // before anything is emitted -- so both footprints are claimed by
            // hand, or the route would happily run through the tank it is
            // aiming at.
            .chain([sited.area.clone(), tank_area.clone()]),
        origin,
        (PIPE_HALF_BOX, PIPE_HALF_BOX),
    );
    // A port's own tiles are where pipes go, so they must not read as
    // obstacles to the search that has to end on one.
    let port_tiles: Vec<Position> = source
        .tiles()
        .into_iter()
        .chain(sinks.iter().flat_map(|port| port.tiles()))
        .collect();
    for tile in &port_tiles {
        if let Some(cell) = cell_of(origin, tile) {
            blocked[enclosure::cell_index(cell.0, cell.1)] = false;
        }
    }

    let from = cell_of(origin, &source.junction).ok_or_else(|| {
        refuse(format!(
            "the {}'s own output tile {} is outside the searched window",
            sited.name, source.junction
        ))
    })?;

    // The tank's nearest port by straight-line distance, then the next, so a
    // corner the route cannot reach does not refuse the whole run.
    let mut ranked: Vec<&FluidPort> = sinks.iter().collect();
    ranked.sort_by(|a, b| {
        calculate_distance(&a.junction, &source.junction)
            .total_cmp(&calculate_distance(&b.junction, &source.junction))
            .then(a.junction.x.total_cmp(&b.junction.x))
            .then(a.junction.y.total_cmp(&b.junction.y))
    });

    let mut last: Option<String> = None;
    for sink in ranked {
        let Some(to) = cell_of(origin, &sink.junction) else {
            last = Some(format!(
                "the {tank}'s connection at {} is outside the searched window",
                sink.junction
            ));
            continue;
        };
        // `max_underground: None`: an underground pipe pair has an input half
        // and an output half, and neither `FactorioEntity` nor the mod's
        // `rcon_place_entity` can say which -- the identical constraint
        // `method::connect` states for underground belts. A route that would
        // need to tunnel refuses.
        match route_belt(&blocked, origin, from, to, None) {
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
                // The grid says a pipe fits; `is_area_free` is what the game
                // will be asked. It knows about water, which the entity
                // rasterisation does not, and an oil field beside a lake is
                // exactly where that differs.
                if let Some(bad) = tiles.iter().find(|tile| !state.is_area_free(pipe, tile)) {
                    return Err(refuse(format!("the tile at {bad} cannot hold a {pipe}",)));
                }
                return Ok(tiles);
            }
            Err(RouteError::NoPath { blocked }) => {
                last = Some(match blocked.first() {
                    Some(first) => format!(
                        "no route to the {tank}'s connection at {}, blocked by {} tile(s) from \
                         {first}",
                        sink.junction,
                        blocked.len()
                    ),
                    None => format!(
                        "no route to the {tank}'s connection at {}, and nothing on the searched \
                         grid blocked it: the two are further apart than one window reaches",
                        sink.junction
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
    Err(refuse(last.unwrap_or_else(|| {
        format!("the {tank} declares no pipe connection to aim at")
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
fn plain_entity(state: &PlanState, name: &str, position: &Position) -> FactorioEntity {
    let entity_type = state
        .base()
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
fn place_step(ctx: &mut ExpansionCtx, entity: FactorioEntity, note: &str) -> Step {
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
mod gather_tests {
    use super::*;
    use crate::ids::BotId;
    use crate::test_world::{OilFixture, PumpjackRecipe, world_with_oil};
    use factorio_bot_core::factorio::world::FactorioSurface;
    use factorio_bot_core::types::{FactorioFluidBoxConnection, FactorioFluidBoxPrototype};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    /// The oil ladder's own fixture with the pumpjack recipe open, exactly as
    /// `method::extract`'s siting tests use it.
    ///
    /// **Not written for this code**, which is the point: `world_with_oil` and
    /// its twelve wells predate this module by a day, and the wells sit where
    /// a resumed workspace held them. See
    /// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`
    /// for why that matters.
    const OPEN: OilFixture = OilFixture {
        wells: true,
        categories: true,
        pumpjack: PumpjackRecipe::LockedBy { researched: true },
        prerequisite: false,
    };

    fn oil_state(fixture: OilFixture) -> PlanState {
        PlanState::from_world(Arc::new(world_with_oil(fixture)), &[BotId(1)])
    }

    fn goal() -> Goal {
        Goal::Gathered {
            entity: "crude-oil".into(),
            unlocks: None,
        }
    }

    /// Every position a `Place` of `name` names, in emission order.
    fn placed(steps: &[Step], name: &str) -> Vec<Position> {
        placed_labelled(steps, name, "")
    }

    /// [`placed`], restricted to placements whose label contains `note`.
    ///
    /// **A plan holds pipes this module did not lay.** `ensure_powered` sites
    /// a power plant when there is none, and a plant is an offshore pump, a
    /// boiler, a steam engine and the *pipe* between them -- three more
    /// `Place { pipe }` actions in the same `Vec<Step>`. A connectivity test
    /// over every pipe in the plan therefore measures two unrelated runs at
    /// once and fails for the wrong reason, which is exactly what it did
    /// before this existed.
    fn placed_labelled(steps: &[Step], name: &str, note: &str) -> Vec<Position> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) => match &action.kind {
                    ActionKind::Place { entity }
                        if entity.name == name && action.label.contains(note) =>
                    {
                        Some(entity.position.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// The label every pipe of a gather run carries, and nothing else does.
    const RUN_NOTE: &str = "carry crude-oil from the pumpjack into the storage-tank";

    fn key(p: &Position) -> (i64, i64) {
        ((p.x() * 2.) as i64, (p.y() * 2.) as i64)
    }

    // -- the two captures ----------------------------------------------------

    /// **The claim this module rests on, checked against a SECOND capture.**
    ///
    /// `crates/core/tests/entity-prototype-fixtures.json` (which
    /// `fixture_world` loads) says a north-facing pumpjack's output connection
    /// is at offset `(1, -2)` -- a tile outside its 3x3 footprint, so it names
    /// the pipe tile directly. Every live dump instead says `(1, -1)`, a tile
    /// inside the footprint, and does not send the direction that would say
    /// which neighbour the pipe goes on.
    ///
    /// The two describe the same machine, so **the 1.x capture's single answer
    /// must be one of the candidates the 2.x reading produces**, and the
    /// junction must be adjacent to it. That is an oracle from data this
    /// module did not write, which is exactly what a geometry test needs: an
    /// assertion against the module's own arithmetic would agree with whatever
    /// the code did.
    #[test]
    fn the_two_captures_agree_on_where_a_pumpjacks_pipe_goes() {
        let at = Position::new(20.5, 20.5);
        let old = oil_state(OPEN);
        let old_ports = fluid_ports(&old, "pumpjack", &at, Some("output"))
            .expect("the 1.x fixture describes the pumpjack's output");
        assert_eq!(old_ports.len(), 1, "a pumpjack has one output connection");
        assert_eq!(
            old_ports[0].candidates,
            vec![Position::new(21.5, 18.5)],
            "the 1.x capture names the pipe tile outright, so there is nothing \
             to disambiguate"
        );

        let new = live_capture_state();
        let new_ports = fluid_ports(&new, "pumpjack", &at, Some("output"))
            .expect("the 2.x capture describes the pumpjack's output");
        assert_eq!(new_ports.len(), 1, "still one output connection");
        let port = &new_ports[0];
        assert_eq!(
            port.candidates.len(),
            2,
            "an interior corner tile has two neighbours outside the footprint: {port:?}"
        );
        assert!(
            port.candidates.contains(&Position::new(21.5, 18.5)),
            "the 2.x reading must offer the tile the 1.x capture names \
             outright; it offered {:?}",
            port.candidates
        );
        for candidate in &port.candidates {
            let dx = (candidate.x() - port.junction.x()).abs();
            let dy = (candidate.y() - port.junction.y()).abs();
            assert!(
                (dx - 1.).abs() < 1e-9 && dy < 1e-9 || dx < 1e-9 && (dy - 1.).abs() < 1e-9,
                "the junction {} must touch every candidate; {candidate} is not adjacent to it",
                port.junction
            );
        }
    }

    /// The fixture world with the pumpjack's fluidbox rewritten to the numbers
    /// **the live 2.1.17 capture and every world dump carry**: the interior
    /// tile `(1, -1)`, rotated per direction, with no direction sent.
    ///
    /// Hand-built rather than loaded, because the live snapshot is a
    /// prototype-only capture with no entity graph and this module needs a
    /// world; the four numbers come from
    /// `crates/core/tests/live-2.1.17-world-snapshot.json` and from
    /// `workspace/scripts/map-31337-explored.json`, which agree.
    fn live_capture_state() -> PlanState {
        let world: FactorioSurface = world_with_oil(OPEN);
        world
            .entity_prototypes
            .get_mut("pumpjack")
            .expect("the fixture has a pumpjack")
            .fluidbox_prototypes = Some(vec![FactorioFluidBoxPrototype {
            production_type: "output".into(),
            pipe_connections: Box::new(Some(vec![FactorioFluidBoxConnection {
                max_underground_distance: None,
                connection_type: Some("normal".into()),
                positions: vec![
                    Position::new(1., -1.),
                    Position::new(1., 1.),
                    Position::new(-1., 1.),
                    Position::new(-1., -1.),
                ],
            }])),
        }]);
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// A connection offset that is neither an interior tile nor an orthogonal
    /// neighbour is refused by name, never rounded to the nearest thing that
    /// looks like one.
    #[test]
    fn a_connection_off_the_footprints_corner_is_refused() {
        let world: FactorioSurface = world_with_oil(OPEN);
        world
            .entity_prototypes
            .get_mut("pumpjack")
            .expect("the fixture has a pumpjack")
            .fluidbox_prototypes = Some(vec![FactorioFluidBoxPrototype {
            production_type: "output".into(),
            pipe_connections: Box::new(Some(vec![FactorioFluidBoxConnection {
                max_underground_distance: None,
                connection_type: Some("normal".into()),
                // Off the corner on both axes at once.
                positions: vec![Position::new(2., -2.); 4],
            }])),
        }]);
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        let refusal = fluid_ports(
            &state,
            "pumpjack",
            &Position::new(20.5, 20.5),
            Some("output"),
        )
        .expect_err("a diagonal connection cannot be placed against");
        assert!(
            matches!(refusal, PlannerError::FluidPortUnknown { .. }),
            "expected FluidPortUnknown, got {refusal:?}"
        );
    }

    // -- the field -----------------------------------------------------------

    /// **The reason this module does not use `PlanState::resource_patches`.**
    ///
    /// The graph's patches come from a flood fill over adjacent tiles, and oil
    /// wells are never adjacent -- so on the fixture's twelve wells the graph
    /// reports twelve patches of one tile each, and a "patch centroid" is just
    /// the well itself. The field is the module's own neighbourhood and is
    /// bigger than one tile.
    #[test]
    fn the_field_is_not_the_graphs_patch() {
        let state = oil_state(OPEN);
        let patches = state.resource_patches("crude-oil");
        assert_eq!(patches.len(), 12, "twelve wells, none adjacent to another");
        assert!(
            patches.iter().all(|patch| patch.elements.len() == 1),
            "every graph patch here is a single tile: {:?}",
            patches.iter().map(|p| p.elements.len()).collect::<Vec<_>>()
        );

        let wellhead = Position::new(20.5, 20.5);
        let field = field_of(&state, "crude-oil", &wellhead);
        assert!(
            field.len() > 1,
            "the field must gather several wells, not one: {field:?}"
        );
        assert!(
            field
                .iter()
                .all(|tile| calculate_distance(tile, &wellhead) <= FIELD_RADIUS),
            "every field tile is within FIELD_RADIUS of the wellhead"
        );
        assert!(
            field.len() < patches.len(),
            "and it is a neighbourhood, not the whole map: {} of {}",
            field.len(),
            patches.len()
        );
        let centroid = centroid_of(&field);
        assert_ne!(
            centroid, wellhead,
            "a centroid over several wells is not the wellhead itself"
        );
        assert!(
            (centroid.x().fract().abs() - 0.5).abs() < 1e-9
                && (centroid.y().fract().abs() - 0.5).abs() < 1e-9,
            "the anchor is snapped to a tile centre, where a 3x3 footprint stands: {centroid}"
        );
    }

    /// A tank is never sited on a well, even though the game would allow it:
    /// resources do not collide with buildings, so `is_area_free` says yes and
    /// the tile's only value is destroyed for good.
    #[test]
    fn a_tank_is_never_sited_on_a_well() {
        let state = oil_state(OPEN);
        let wells: Vec<Position> = state
            .resource_patches("crude-oil")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect();
        // Anchored on a well, so the nearest candidate is the well itself and
        // nothing but the rule under test can move it.
        let centroid = wells[0].clone();
        let sited = extract::SitedExtractor {
            name: "pumpjack".into(),
            kw: 90.,
            site: Position::new(200.5, 200.5),
            area: Rect::new(&Position::new(199., 199.), &Position::new(202., 202.)),
        };
        let site = choose_tank_site(
            &state,
            "storage-tank",
            "crude-oil",
            &centroid,
            &wells,
            &sited,
        )
        .expect("the fixture has open ground beside its wells");
        let area = state
            .collision_area("storage-tank", &site)
            .expect("the fixture has a storage-tank prototype");
        for well in &wells {
            assert!(
                !covers(&area, well),
                "the tank at {site} buries the well at {well}"
            );
        }
        assert!(
            state.is_area_free("storage-tank", &site),
            "and it stands on ground the game would accept"
        );
    }

    /// The tank does not stand on the pumpjack's own ground either -- the
    /// pumpjack is not in the overlay when the tank is sited, so nothing but
    /// this check keeps them apart.
    #[test]
    fn a_tank_is_not_sited_on_the_pumpjack_this_plan_is_placing() {
        let state = oil_state(OPEN);
        let wellhead = Position::new(20.5, 20.5);
        let sited = extract::site_extractor(&state, "crude-oil", &wellhead)
            .expect("the fixture charts wells and opens the recipe");
        let field = field_of(&state, "crude-oil", &sited.site);
        // Anchor the search on the pumpjack itself, so the first candidate
        // tried is the one that overlaps it.
        let site = choose_tank_site(
            &state,
            "storage-tank",
            "crude-oil",
            &sited.site,
            &field,
            &sited,
        )
        .expect("there is ground beside the pumpjack");
        let area = state
            .collision_area("storage-tank", &site)
            .expect("a storage-tank prototype");
        assert!(
            !boxes_overlap(&area, &sited.area),
            "the tank at {site} overlaps the pumpjack at {}",
            sited.site
        );
    }

    // -- the run -------------------------------------------------------------

    /// **The test this module exists for.** A pipe run that places perfectly
    /// and connects nothing is the silent failure this repo has already paid
    /// for once, so the emitted tiles are checked as a *graph*: one connected
    /// component under four-adjacency, touching a real connection of the
    /// pumpjack at one end and of the tank at the other.
    #[test]
    fn the_pipe_run_joins_the_pumpjack_to_the_tank() {
        let state = oil_state(OPEN);
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        let steps = Gather
            .expand(&goal(), &mut ctx)
            .expect("a charted well, an open recipe and a lake to power it from");

        let pumpjacks = placed(&steps, "pumpjack");
        let tanks = placed(&steps, "storage-tank");
        let pipes = placed_labelled(&steps, "pipe", RUN_NOTE);
        assert!(
            pipes.len() < placed(&steps, "pipe").len(),
            "the plan must also hold the power plant's own pipes, or this test's \
             filter is measuring nothing"
        );
        assert_eq!(pumpjacks.len(), 1, "one pumpjack");
        assert_eq!(tanks.len(), 1, "one tank");
        assert!(pipes.len() >= 2, "a run is more than one tile: {pipes:?}");

        let cells: BTreeSet<(i64, i64)> = pipes.iter().map(key).collect();
        assert_eq!(cells.len(), pipes.len(), "no tile is placed twice");

        // One connected component, walked from the first tile.
        let mut seen: BTreeSet<(i64, i64)> = BTreeSet::new();
        let mut frontier = vec![key(&pipes[0])];
        while let Some(cell) = frontier.pop() {
            if !seen.insert(cell) {
                continue;
            }
            for (dx, dy) in [(2, 0), (-2, 0), (0, 2), (0, -2)] {
                let next = (cell.0 + dx, cell.1 + dy);
                if cells.contains(&next) && !seen.contains(&next) {
                    frontier.push(next);
                }
            }
        }
        assert_eq!(
            seen.len(),
            cells.len(),
            "the run is in {} pieces; a pipe that touches nothing moves nothing",
            cells.len() - seen.len() + 1
        );

        let source = fluid_ports(&ctx.state, "pumpjack", &pumpjacks[0], Some("output"))
            .expect("a pumpjack has an output");
        assert!(
            source[0].candidates.iter().any(|c| cells.contains(&key(c))),
            "no pipe stands on any candidate for the pumpjack's output {:?}",
            source[0].candidates
        );
        let sinks = fluid_ports(&ctx.state, "storage-tank", &tanks[0], None)
            .expect("a tank has connections");
        assert!(
            sinks
                .iter()
                .any(|port| port.candidates.iter().any(|c| cells.contains(&key(c)))),
            "no pipe stands on any of the tank's connections"
        );
    }

    /// **The branch every live dump takes, end to end.** The fixture world
    /// describes the pumpjack's output in the 1.x convention, where a port is
    /// one unambiguous tile -- so every other test in this module exercises
    /// the *easy* half of [`fluid_ports`] and the two-candidate half that
    /// production actually uses is reached by nothing.
    ///
    /// Found by falsification: dropping the source port's tiles from the run
    /// broke no test at all, because in the 1.x reading that tile is also the
    /// route's own start and `route_belt` emits it anyway.
    #[test]
    fn a_run_on_the_live_capture_places_both_candidates_and_their_junction() {
        let mut ctx = ExpansionCtx::new(live_capture_state().fork(), BotId(1));
        let steps = Gather
            .expand(&goal(), &mut ctx)
            .expect("the same fixture, with the pumpjack's fluidbox as a dump carries it");
        let pumpjack = placed(&steps, "pumpjack")[0].clone();
        let pipes: BTreeSet<(i64, i64)> = placed_labelled(&steps, "pipe", RUN_NOTE)
            .iter()
            .map(key)
            .collect();
        let port = fluid_ports(&ctx.state, "pumpjack", &pumpjack, Some("output"))
            .expect("an output")
            .remove(0);
        assert_eq!(
            port.candidates.len(),
            2,
            "this test is pointless unless the port is the ambiguous kind"
        );
        for candidate in &port.candidates {
            assert!(
                pipes.contains(&key(candidate)),
                "the run must cover BOTH candidates -- the mod does not send which one is \
                 real -- and {candidate} has no pipe"
            );
        }
        assert!(
            pipes.contains(&key(&port.junction)),
            "and the junction {} that joins them",
            port.junction
        );
    }

    /// The obstacle grid is what makes the route go *around* something. A
    /// wall with a gap in it is planned through the gap; a search that ignored
    /// the grid would head straight at the wall and be refused by the
    /// per-tile check afterwards, so this fails as a **refusal** rather than
    /// as a bad layout.
    ///
    /// Also found by falsification: unblocking every cell of the grid broke
    /// no test, because `an_unroutable_tank_refuses_and_leaves_nothing_behind`
    /// is answered by that per-tile check and never needed the grid at all.
    #[test]
    fn a_route_goes_around_an_obstacle_rather_than_through_it() {
        let state = oil_state(OPEN);
        let wellhead = Position::new(20.5, 20.5);
        let sited = extract::site_extractor(&state, "crude-oil", &wellhead)
            .expect("the fixture charts wells");
        // A wall two tiles east of the pumpjack, spanning the corridor a
        // straight route would take. It is open at both ends, so a route
        // exists and has to leave the wall's own span to find it.
        let mut walled = state.fork();
        let mut wall = Vec::new();
        for dy in -4..=3i64 {
            let at = Position::new(sited.site.x() + 2., sited.site.y() + dy as f64);
            wall.push(at.clone());
            walled.create_entity(FactorioEntity {
                name: "stone-wall".into(),
                entity_type: "wall".into(),
                position: at.clone(),
                bounding_box: Rect::new(
                    &Position::new(at.x() - 0.49, at.y() - 0.49),
                    &Position::new(at.x() + 0.49, at.y() + 0.49),
                ),
                ..Default::default()
            });
        }
        let mut ctx = ExpansionCtx::new(walled, BotId(1));
        let steps = Gather
            .expand(&goal(), &mut ctx)
            .expect("the wall has an open end, so a route exists around it");
        let pipes = placed_labelled(&steps, "pipe", RUN_NOTE);
        let on_wall: Vec<&Position> = pipes
            .iter()
            .filter(|pipe| wall.iter().any(|w| key(w) == key(pipe)))
            .collect();
        assert!(
            on_wall.is_empty(),
            "the route ran through the wall at {on_wall:?}"
        );
        // The wall spans `site.y - 4 ..= site.y + 3`; a route that ignored it
        // would stay inside that span, because both ports do.
        let escaped = pipes
            .iter()
            .any(|pipe| pipe.y() < sited.site.y() - 4. || pipe.y() > sited.site.y() + 3.);
        assert!(
            escaped,
            "the route never left the wall's own span, so it did not go around \
             anything: {pipes:?}"
        );
    }

    /// The materials are stated as `Goal::Have` before the ground is claimed,
    /// so a shortfall refuses rather than half-building.
    #[test]
    fn the_tank_and_the_pipe_are_billed_before_they_are_placed() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let steps = Gather.expand(&goal(), &mut ctx).expect("it plans");
        let pipes = placed_labelled(&steps, "pipe", RUN_NOTE).len() as u32;
        let bill = |item: &str| {
            steps.iter().position(
                |step| matches!(step, Step::Subgoal(Goal::Have { item: i, .. }) if i == item),
            )
        };
        let place = |name: &str, note: &str| {
            steps.iter().position(|step| {
                matches!(step, Step::Act(a)
                    if a.label.contains(note)
                    && matches!(&a.kind, ActionKind::Place { entity } if entity.name == name))
            })
        };
        assert!(
            bill("storage-tank") < place("storage-tank", ""),
            "the tank is billed before it is placed"
        );
        assert!(bill("pipe") < place("pipe", RUN_NOTE), "so is the pipe");
        // **Not the sum**: `ensure_powered` bills its own pipes for the plant
        // it sites, in the same plan and under the same item name. What is
        // asserted is that one `Have` states exactly this run's length -- a
        // sum would pass on a bill that was wrong by the plant's three.
        let billed: Vec<u32> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Subgoal(Goal::Have { item, count, .. }) if item == "pipe" => Some(*count),
                _ => None,
            })
            .collect();
        assert!(
            billed.contains(&pipes),
            "no pipe bill states the run's own length {pipes}; the bills are {billed:?}"
        );
    }

    /// **Refuses before placing anything.** A tank the route cannot reach
    /// leaves no action and no entity behind -- the promise
    /// `method::connect`'s `ConnectRefusal` makes, restated here because a
    /// half-built pipe run fills the pumpjack and stops.
    #[test]
    fn an_unroutable_tank_refuses_and_leaves_nothing_behind() {
        let state = oil_state(OPEN);
        let wellhead = Position::new(20.5, 20.5);
        let sited = extract::site_extractor(&state, "crude-oil", &wellhead)
            .expect("the fixture charts wells");
        // A wall all the way round the pumpjack, one tile outside its own
        // footprint, so every route off it is blocked and nothing else about
        // the plan changes.
        let mut walled = state.fork();
        for dx in -3i64..=3 {
            for dy in -3i64..=3 {
                if dx.abs() != 3 && dy.abs() != 3 {
                    continue;
                }
                let at = Position::new(sited.site.x() + dx as f64, sited.site.y() + dy as f64);
                walled.create_entity(FactorioEntity {
                    name: "stone-wall".into(),
                    entity_type: "wall".into(),
                    position: at.clone(),
                    bounding_box: Rect::new(
                        &Position::new(at.x() - 0.49, at.y() - 0.49),
                        &Position::new(at.x() + 0.49, at.y() + 0.49),
                    ),
                    ..Default::default()
                });
            }
        }
        let before = walled.entities_within(&sited.site, 64.).len();
        let mut ctx = ExpansionCtx::new(walled, BotId(1));
        let refusal = Gather
            .expand(&goal(), &mut ctx)
            .expect_err("no route leaves a walled-in pumpjack");
        assert!(
            matches!(refusal, PlannerError::NoPipeRoute { .. }),
            "expected NoPipeRoute, got {refusal:?}"
        );
        assert_eq!(
            ctx.state.entities_within(&sited.site, 64.).len(),
            before,
            "a refusal must leave the overlay exactly as it found it"
        );
    }

    /// A tank already standing in the field is adopted, so a replan does not
    /// stack tanks on top of each other.
    #[test]
    fn a_standing_tank_is_adopted_rather_than_duplicated() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let first = Gather.expand(&goal(), &mut ctx).expect("it plans");
        assert_eq!(placed(&first, "storage-tank").len(), 1, "the first tank");
        let second = Gather
            .expand(&goal(), &mut ctx)
            .expect("it plans again against the world the first plan built");
        assert!(
            placed(&second, "storage-tank").is_empty(),
            "the standing tank must be adopted: {:?}",
            placed(&second, "storage-tank")
        );
    }

    /// A world with no tank prototype refuses by name at tier 2, rather than
    /// planning a pumpjack whose output has nowhere to go.
    #[test]
    fn a_world_with_nothing_to_buffer_a_fluid_refuses_by_name() {
        let world: FactorioSurface = world_with_oil(OPEN);
        world
            .entity_prototypes
            .get_mut("storage-tank")
            .expect("the fixture has a storage-tank")
            .entity_type = "container".into();
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let refusal = Gather
            .expand(&goal(), &mut ctx)
            .expect_err("nothing left in this world is a fluid buffer");
        assert!(
            matches!(refusal, PlannerError::NoFluidBuffer { .. }),
            "expected NoFluidBuffer, got {refusal:?}"
        );
    }

    /// The refusals `method::extract` owns are still the ones a caller sees:
    /// an unexplored map is `NotCharted`, not a tank problem.
    #[test]
    fn an_uncharted_resource_refuses_before_any_tank_question() {
        let state = oil_state(OilFixture {
            wells: false,
            ..OPEN
        });
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let refusal = Gather
            .expand(&goal(), &mut ctx)
            .expect_err("no well is charted");
        assert!(
            matches!(refusal, PlannerError::NotCharted { .. }),
            "expected NotCharted, got {refusal:?}"
        );
        assert!(
            !Gather.applicable(&goal(), &ctx.state),
            "and the method does not claim the goal at all"
        );
    }
}
