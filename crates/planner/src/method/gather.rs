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
//!    charted, nothing mines it, no well tile is free, or no power reaches
//!    one. **With one rung subtracted, since 2026-09-06: an extractor whose
//!    recipe is merely *not yet researched* is not refused here, it is
//!    billed.** See [`site_extractor_billing_the_unlock`], which says why the
//!    research subgoal is `have.rs`'s to emit and not this module's, and
//!    which rung stays a refusal because no research can clear it;
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
use crate::method::pipe::{
    PipeEnd, buffer_prototype, pipe_prototype, place_step, plain_entity, route_between,
};
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::{Direction, FactorioEntity, Position, Rect};

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
        let sited = site_extractor_billing_the_unlock(&ctx.state, entity, &origin)?;

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
        let run = route_between(
            &ctx.state,
            &PipeEnd {
                name: &sited.name,
                position: &sited.site,
                area: sited.area.clone(),
                production_type: Some("output"),
                port_index: None,
            },
            &PipeEnd {
                name: &tank,
                position: &tank_site,
                area: tank_area,
                production_type: None,
                port_index: None,
            },
            &pipe,
            &[],
        )?;

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
                via: None,
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
            via: None,
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

/// [`extract::site_extractor`], with a locked extractor recipe treated as a
/// **subgoal of this plan** rather than as a refusal.
///
/// # Why this exists
///
/// Every other goal kind in this planner bills the technology that unlocks
/// what it needs. `Goal::Have { pumpjack }` is the control: `have.rs`'s
/// `HandCraft` reads [`crate::method::util::RecipeGate::NeedsResearch`],
/// emits `Step::Subgoal(Goal::Researched(tech))` and states
/// `Condition::Researched(tech)` on the craft. Measured offline against
/// `map-31337-explored.json` on 2026-09-06, `have:pumpjack:1` plans and
/// `gathered:crude-oil` refused with
/// [`PlannerError::ExtractorLocked`](crate::error::PlannerError::ExtractorLocked)
/// -- naming `oil-gathering`, a technology that plans on that same dump in
/// 1,587 actions. So the fact was stated as an impossibility while the
/// planner could satisfy it.
///
/// # Why the *research* is not emitted here
///
/// **Because nothing about extraction needs it.** A pumpjack in a bot's
/// inventory can be placed whatever the force has researched; the recipe gate
/// binds the *craft*, and the craft is already asked for -- as
/// `Goal::Have { item: extractor }`, the first step
/// [`extract::extractor_steps`] emits. Billing the research a second time
/// here would be a second encoding of a rule `have.rs` already owns, and the
/// two would agree only until one of them changed. It would also be wrong in
/// the one case that matters: a roster already holding a pumpjack owes the
/// research nothing, and `Have` is the only thing that knows.
///
/// The ordering edge comes with it. `HandCraft` states
/// `Condition::Researched` on the craft, the craft's `Effect::GainItem`
/// satisfies the placement's `Condition::HasItem`, and `infer_edges` draws
/// both -- so the pumpjack cannot be placed before the technology it was
/// crafted under.
///
/// # What is still refused, and it is a different fact
///
/// Only the `NeedsResearch` rung is converted. Everything else
/// `site_extractor` says comes back untouched, including
/// `NoExtractor { why: "... disabled with no technology to unlock it" }` --
/// the [`RecipeGate::Unobtainable`](crate::method::util::RecipeGate::Unobtainable)
/// case, which is "nothing in this game unlocks this" and is not a research
/// this plan could ever do. A technology that exists but is itself
/// unreachable refuses too, from where that is known: expanding the
/// `Goal::Researched` the craft emits, in that goal's own words.
///
/// [`Gather::refusal`] is left alone deliberately: it delegates to
/// `extract::refusal_for`, which can still name `ExtractorLocked` -- but it
/// is only consulted when **no** method claims the goal, and
/// [`Gather::applicable`] does not consult the recipe gate, so for a
/// `Goal::Gathered` this method always claims and the locked rung is never
/// reached from there.
fn site_extractor_billing_the_unlock(
    state: &PlanState,
    entity: &str,
    origin: &Position,
) -> Result<extract::SitedExtractor, PlannerError> {
    match extract::site_extractor(state, entity, origin) {
        Err(PlannerError::ExtractorLocked { technology, .. }) => {
            // The overlay, on a fork, is how "an action in this plan will
            // have done it" is said -- `PlanState::is_world_researched` stays
            // false, so `recipe_gate` reads `PlannedResearch` rather than
            // `Open` and the distinction that `run-1788338409-63794` cost is
            // preserved. The fork is thrown away: `sited` is a tile, a
            // footprint and a draw, none of which research changes, and the
            // real `ctx.state` must still read `NeedsResearch` when
            // `extractor_steps`' `Goal::Have` reaches `HandCraft` -- that is
            // the thing that bills it.
            let mut planned = state.fork();
            planned.set_researched(&technology);
            extract::site_extractor(&planned, entity, origin)
        }
        other => other,
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

#[cfg(test)]
mod gather_tests {
    use super::*;
    use crate::action::ActionKind;
    use crate::ids::BotId;
    use crate::method::pipe::fluid_ports;
    use crate::method::util::{RecipeGate, recipe_for, recipe_gate};
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
            .globals
            .entity_prototypes
            .get_mut("pumpjack")
            .expect("the fixture has a pumpjack")
            .fluidbox_prototypes = Some(vec![FactorioFluidBoxPrototype {
            production_type: "output".into(),
            // A pumpjack fixture that is about pipe GEOMETRY: the capacity
            // is real on the live prototype and irrelevant here.
            volume: None,
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
            .globals
            .entity_prototypes
            .get_mut("pumpjack")
            .expect("the fixture has a pumpjack")
            .fluidbox_prototypes = Some(vec![FactorioFluidBoxPrototype {
            production_type: "output".into(),
            // A pumpjack fixture that is about pipe GEOMETRY: the capacity
            // is real on the live prototype and irrelevant here.
            volume: None,
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
            .globals
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

    // -- the unlock ----------------------------------------------------------

    /// The same fixture with the pumpjack recipe **locked**, which is the
    /// world seed 31337 actually presents at t=0.
    const LOCKED: OilFixture = OilFixture {
        pumpjack: PumpjackRecipe::LockedBy { researched: false },
        ..OPEN
    };

    /// The gate `method::extract` reads, asked of a state directly.
    fn pumpjack_gate(state: &PlanState) -> RecipeGate {
        let recipe = recipe_for(state, "pumpjack").expect("the fixture has a pumpjack recipe");
        recipe_gate(state, &recipe)
    }

    /// **The gap this module had until 2026-09-06.** A technology that has not
    /// been researched yet is a subgoal everywhere else in this planner --
    /// `have:pumpjack:1` plans and bills `oil-gathering` -- and `gathered:`
    /// alone stated it as a refusal.
    ///
    /// The fixture's own precondition is asserted first, so that a later
    /// change making `LOCKED` no longer locked fails here loudly instead of
    /// leaving this test passing about nothing.
    #[test]
    fn a_locked_extractor_recipe_is_billed_rather_than_refused() {
        let state = oil_state(LOCKED);
        assert_eq!(
            pumpjack_gate(&state),
            RecipeGate::NeedsResearch("oil-gathering".into()),
            "the fixture must present a recipe this force has not unlocked"
        );

        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        let steps = Gather
            .expand(&goal(), &mut ctx)
            .expect("the research is a subgoal, not a refusal");

        // Absolute, not a relation: one pumpjack is placed, and the machine
        // whose recipe is locked is asked for by name. That `Have` is the
        // whole billing mechanism -- see `site_extractor_billing_the_unlock`.
        assert_eq!(
            placed(&steps, "pumpjack").len(),
            1,
            "the plan stands exactly one pumpjack"
        );
        assert!(
            steps.iter().any(|step| matches!(
                step,
                Step::Subgoal(Goal::Have { item, count, .. })
                    if item == "pumpjack" && *count == 1
            )),
            "the extractor is billed as `Have`, which is what carries the unlock"
        );

        // And the plan is the one the open world produces: unlocking is the
        // only difference, so nothing about the siting, the tank or the pipe
        // run may move.
        let mut open_ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let open = Gather
            .expand(&goal(), &mut open_ctx)
            .expect("the open fixture plans");
        assert_eq!(
            steps.len(),
            open.len(),
            "a locked recipe changes what is billed, not what is built"
        );
        assert_eq!(
            placed(&steps, "storage-tank"),
            placed(&open, "storage-tank"),
            "the tank is sited identically"
        );
        assert_eq!(
            placed_labelled(&steps, "pipe", RUN_NOTE),
            placed_labelled(&open, "pipe", RUN_NOTE),
            "and so is the pipe run"
        );
    }

    /// **The research must still be owed when the `Have` subgoal is
    /// expanded.** `site_extractor_billing_the_unlock` gets past the gate on a
    /// **fork**; were it to mark `ctx.state` instead, `recipe_gate` would
    /// answer `PlannedResearch` for `have.rs`'s `HandCraft`, which emits no
    /// research subgoal for that -- and nothing in the plan would ever
    /// research `oil-gathering`. The plan would look bigger and be
    /// unexecutable.
    #[test]
    fn siting_past_the_lock_does_not_mark_the_research_as_planned() {
        let state = oil_state(LOCKED);
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        Gather
            .expand(&goal(), &mut ctx)
            .expect("the locked fixture plans");
        assert_eq!(
            pumpjack_gate(&ctx.state),
            RecipeGate::NeedsResearch("oil-gathering".into()),
            "the plan still owes the research after siting, so `Have` will bill it"
        );
        assert!(
            !ctx.state.is_researched("oil-gathering"),
            "and nothing in this plan has claimed to have done it"
        );
    }

    /// The other half of the distinction, and it is a different fact: a recipe
    /// **no technology unlocks** is not "not yet researched", it is
    /// unreachable, and no subgoal can change that. Refused by name, with the
    /// reason in the message.
    #[test]
    fn a_recipe_no_technology_unlocks_is_still_refused_by_name() {
        let world = world_with_oil(LOCKED);
        // Strip the unlock from `oil-gathering`, leaving the recipe disabled
        // and nothing in the tree able to turn it on.
        let mut force = world
            .globals
            .forces
            .get("player")
            .expect("the oil fixture has a player force")
            .clone();
        force
            .technologies
            .get_mut("oil-gathering")
            .expect("the locked fixture carries oil-gathering")
            .unlocked_recipes
            .clear();
        world.update_force(force).expect("the force is well-formed");

        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert_eq!(
            pumpjack_gate(&state),
            RecipeGate::Unobtainable,
            "the fixture must present a recipe nothing unlocks"
        );

        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let refusal = Gather
            .expand(&goal(), &mut ctx)
            .expect_err("nothing in this world can make a pumpjack");
        match &refusal {
            PlannerError::NoExtractor { entity, why } => {
                assert_eq!(entity, "crude-oil");
                assert!(
                    why.contains("no technology to unlock it"),
                    "the message must say why it is unreachable, got {why}"
                );
            }
            other => panic!("expected NoExtractor, got {other:?}"),
        }
    }
}
