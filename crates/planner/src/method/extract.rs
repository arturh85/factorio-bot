//! Extraction: mining a resource a hand cannot work, by a machine standing
//! on it.
//!
//! [`Goal::Extracted`] is what a Factorio 2.0 `mine-entity` trigger becomes
//! when the entity it names is one a character cannot dig -- `oil-processing`
//! names `crude-oil`, and `character.mine_entity(crude-oil)` answers false
//! (verified live 2026-09-04). The game satisfies that trigger when a
//! **pumpjack** extracts from a well.
//!
//! # What this method claims, and what it still refuses
//!
//! Since 2026-09-06 [`Extract`] **sites the extractor**: it picks a charted
//! tile of the resource, crafts the machine, carries electric supply to it
//! and places it there, hanging the technology's `Effect::Researched` on that
//! placement the same way [`crate::method::have::attach_unlock`] hangs one on
//! a craft or a mining action.
//!
//! **No fluid is modelled, on purpose.** `oil-processing`'s trigger is
//! `{type = "mine-entity", entities = {"crude-oil"}}` and the game fires it
//! natively when a pumpjack extracts -- it is one of the triggers
//! `docs/superpowers/notes/2026-09-05-research-triggers.md` measured firing
//! with no emulation at all, alongside a fuelled burner drill. So the goal is
//! satisfied by *a powered pumpjack standing on a well and running*, and where
//! the petroleum gas then goes is a separate problem with a separate owner.
//! A pumpjack with nothing connected fills its own output fluidbox and stops,
//! which is many extractions after the first.
//!
//! What it still answers [`crate::method::Method::refusal`] with is the *next
//! missing prerequisite*, in the order a reader can act on them:
//!
//! 1. the entity is not charted anywhere the plan can see --
//!    [`PlannerError::NotCharted`], with where charted ground ends;
//! 2. nothing in the prototype table mines it -- [`PlannerError::NoExtractor`],
//!    naming what the table could and could not say;
//! 3. the machine that mines it has a recipe this force has not unlocked --
//!    [`PlannerError::ExtractorLocked`], naming the technology;
//! 4. everything is in place and the cell is not modelled --
//!    [`PlannerError::ExtractionNotModelled`].
//!
//! The first two are facts about the world that no prerequisite research can
//! change, so [`crate::method::have::Researched`] asks them *before* it emits
//! a technology's prerequisites: `oil-processing` sits on `oil-gathering`,
//! 100 red-and-green packs, and a plan that gathered all of those before
//! discovering there is no well in sight would be refusing in the wrong
//! place. The last two are asked where the goal is reached, after those
//! prerequisites have been planned, so that a research already in the plan
//! is seen as done.
//!
//! **Tiers 1 to 3 still decide whether this method claims the goal at all**
//! ([`Extract::applicable`]), which is what keeps them being answered by
//! [`Method::refusal`] rather than by an expansion that has already started
//! spending. Tier 4 changed meaning rather than disappearing: it is now what
//! is said when the extractor is one this planner cannot **power** -- either
//! because [`crate::state::PlanState::consumer_draw_kw`] does not name its
//! draw, or because no run of poles this method can lay brings a generating
//! network to it. Both are honest readings of "the cell is not modelled", and
//! naming them keeps the variant reachable instead of leaving it as a
//! monument.
//!
//! # Power, and the one thing that is *not* here
//!
//! A pumpjack is 90 kW electric ([`crate::state::PlanState::consumer_draw_kw`]
//! carries the number). Power coverage is not power capacity, so this asks
//! [`crate::method::power::supply_for`] -- which adopts a standing network
//! with real headroom, finishes a half-built plant, or sites a new one at
//! water -- and then re-checks the *consumer's own* [`Condition::Powered`],
//! which is a headroom test and not a coverage test.
//!
//! Between that supply and the well there may be a gap, and on a real map
//! there usually is: a well charted 300 tiles out is nowhere near a lake, and
//! `method::power` sites its plant **at** the water. (Its own module doc
//! retracts the reason this line used to give -- "water is the one input that
//! cannot be moved". Water moves, through pipes; siting at the shore is what
//! that module does today, not something physics forces.) So this lays a
//! **straight run of small electric poles** from the supplying pole to a pole covering the extractor. It is
//! deliberately the dumbest router that can be right:
//!
//! * the spacing is [`power::POLE_STEP`](crate::method::power::POLE_STEP), comfortably inside a small pole's wire
//!   reach, so tile snapping cannot silently break a link;
//! * every pole stands on a tile [`PlanState::is_area_free`] accepts and the
//!   game has not already refused a build at;
//! * and the whole run is **verified by the game's own rule, not by this
//!   module's arithmetic**: the final check is `Condition::Powered` evaluated
//!   against a fork carrying every pole, which walks
//!   [`crate::state::PlanState`]'s union-find over the poles' real wire
//!   distances. If that says no, the run is refused rather than emitted.
//!
//! There is no medium or big pole, and no notion of what the wood costs. A run
//! that cannot get past a lake refuses; it does not tunnel. That is the same
//! scope line `method::connect` draws for underground belts, and for the same
//! reason -- a half-built power line is worse than none.
//!
//! # What bounds the reach -- and the ~64-tile ceiling that no longer does
//!
//! **The bill is the bound.** A run of more than [`power::MAX_POLE_RUN`](crate::method::power::MAX_POLE_RUN) poles -- 64,
//! about 380 tiles at [`power::POLE_STEP`](crate::method::power::POLE_STEP) -- is
//! [`PlannerError::ExtractionNotModelled`], and short of that a run refuses
//! only when the ground refuses a pole or when no supply can be reached at
//! all. Seed 31337's crude oil is charted at 256-384 tiles, which is inside
//! that.
//!
//! **This section used to say the opposite, and it was wrong for a day.** It
//! read *"a well more than ~64 tiles from generation refuses"*, from
//! `crate::state`'s `POWER_SEARCH_RADIUS` being 64 tiles and
//! being the *entire* extent of `PlanState::electric_supply_kw`'s search: a
//! longer pole run carried power the model could not see, the condition
//! answered "not powered", and this method refused. That was true when it was
//! written. `PlanState::electric_entities` now **follows the wire** -- one
//! `POWER_SEARCH_RADIUS` disc as a seed, then pole-by-pole expansion with no
//! hop limit -- so a generator any number of poles away is found, and the
//! radius bounds the seed disc rather than the reach. The constant's own doc
//! carries that history, including why *raising* it was the obvious fix and
//! the wrong one.
//!
//! `a_pole_chain_carries_however_long_it_is_but_a_broken_one_does_not`
//! (`power_reach_tests`, below) is the pin, against `PlanState` rather than
//! against this module: it asserts a long chain carries **and** that a broken
//! one does not, so a search that simply counted every generator on the map
//! would fail it.
//!
//! **The table that stood here was measured before that change and has not
//! been redone** -- offline on the seed-31337 dump with a crude-oil patch
//! charted into it (`docs/superpowers/notes/2026-09-06-a-pumpjack-on-a-well.md`),
//! and it read: ~52 tiles plans with 10 poles; ~121 tiles refuses
//! `ExtractionNotModelled`; ~281 tiles refuses `PowerPlantNeedsWater`. Rows two
//! and three are **history, not behaviour**: the first was the ceiling that is
//! gone, and the third predates `method::power`'s world-anchored retry, from
//! which the origin does see a lake. Redoing it costs one `tools/inject_oil.py`
//! dump and three `factorio-bot plan --world` runs; until someone does, quote
//! it as a measurement of the old build.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
#[cfg(test)]
use crate::ids::ActionId;
use crate::method::have::PLACE_TICKS;
use crate::method::power::ensure_powered;
#[cfg(test)]
use crate::method::power::{POLE, WIRE_REACH, pole_run};
use crate::method::util::{RecipeGate, recipe_for, recipe_gate, tile_alignment_facing};
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::{Direction, Position};

/// How far from the extractor a *standing* network is looked for before one is
/// built, in tiles.
///
/// The same number and the same reasoning as `method::have`'s
/// `LAB_SEARCH_RADIUS` and `method::assemble`'s `ANCHOR_SEARCH_RADIUS`: it is
/// only the cheap first tier of [`power::supply_for`](crate::method::power::supply_for), which widens to
/// [`crate::method::power::PLANT_ADOPT_RADIUS`] on its own before it will
/// build anything. Changing it moves work between tiers and changes no
/// answer that tier 2 would not have given.
pub const SUPPLY_SEARCH_RADIUS: f64 = 64.;

/// `Direction::North` as the wire byte a `FactorioEntity` carries.
///
/// A pumpjack's facing decides where its output fluidbox points and nothing
/// else about whether it extracts, so every placement here is north. Written
/// as the constant rather than converted, because `Direction`'s numeric
/// conversion is a `num_traits` method and importing that trait for one call
/// reads as though the value were computed.
const NORTH: u8 = 0;

/// The refusal for [`Goal::Extracted`], and -- since 2026-09-06 -- the method
/// that sites the extractor. See the module doc.
pub struct Extract;

impl Method for Extract {
    fn name(&self) -> &'static str {
        "extract"
    }

    /// Only when the world can answer tiers 1 to 3 of the ladder: the
    /// resource is charted, something mines it, and that machine's recipe is
    /// open or already being researched by this plan.
    ///
    /// **Deliberately not "when the whole thing would work."** Whether there
    /// is clear ground on the patch and whether power can reach it are
    /// findings this method has to *do the work* to discover, and a caller is
    /// better served by `expand`'s named error than by a bare
    /// `NoApplicableMethod` from the driver. The three tiers above are cheap
    /// and are the ones whose answer belongs in [`Method::refusal`], because
    /// they are the ones `Researched` asks before it plans a hundred science
    /// packs.
    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Extracted { entity, .. } = goal else {
            return false;
        };
        // `origin_of` needs a context; a refusal about charting is measured
        // from the actor, but *whether* the resource is charted at all is not,
        // and that is the only half applicability needs.
        if !state.has_resource_patches(entity) {
            return false;
        }
        let Ok(extractor) = extractor_for(state, entity) else {
            return false;
        };
        let Some(recipe) = recipe_for(state, &extractor) else {
            return false;
        };
        matches!(
            recipe_gate(state, &recipe),
            RecipeGate::Open | RecipeGate::PlannedResearch(_)
        )
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Extracted { entity, unlocks } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let origin = origin_of(ctx);
        // The ladder again, in full, and not only the half `applicable`
        // checked: a method's `expand` is reachable from tests and from a
        // future caller that never consulted `applicable`, and refusing here
        // by name is cheaper than refusing later by accident.
        if let Some(refusal) = world_refusal(&ctx.state, entity, &origin) {
            return Err(refusal);
        }
        let sited = site_extractor(&ctx.state, entity, &origin)?;
        let (steps, _place_id) = extractor_steps(ctx, entity, &sited, unlocks.as_deref(), &[])?;
        Ok(steps)
    }

    fn refusal(&self, goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
        let Goal::Extracted { entity, .. } = goal else {
            return None;
        };
        Some(refusal_for(&ctx.state, entity, &origin_of(ctx)))
    }
}

/// An extractor, decided but not yet emitted: which machine, what it draws,
/// which well tile it stands on, and the ground that tile costs.
///
/// The return of [`site_extractor`], which is **pure** -- it touches no
/// `ExpansionCtx` and reserves nothing -- so a caller that needs the site
/// before it commits to anything (`method::gather`, which routes pipe to it)
/// can ask without leaving a half-built plan behind if a later step refuses.
#[derive(Debug, Clone)]
pub(crate) struct SitedExtractor {
    /// The machine's prototype name.
    pub name: String,
    /// Its electric draw, in kW, from [`PlanState::consumer_draw_kw`].
    pub kw: f64,
    /// The well tile it is centred on.
    pub site: Position,
    /// Its collision footprint at that tile, facing [`NORTH`].
    pub area: factorio_bot_core::types::Rect,
}

/// Tiers 2 to 4 of the ladder, and then the siting search: which machine
/// mines `entity`, whether its recipe is reachable, whether this planner
/// knows its draw, and which charted well tile it can stand on.
///
/// **The order of the questions is the order of the ladder** and is not
/// arbitrary: an extractor whose recipe is locked must be reported as locked
/// even on a map where no well tile is free, because research is the thing
/// the caller can act on first.
pub(crate) fn site_extractor(
    state: &PlanState,
    entity: &str,
    origin: &Position,
) -> Result<SitedExtractor, PlannerError> {
    let extractor = extractor_for(state, entity)?;
    match recipe_for(state, &extractor).map(|r| recipe_gate(state, &r)) {
        Some(RecipeGate::Open) | Some(RecipeGate::PlannedResearch(_)) => {}
        Some(RecipeGate::NeedsResearch(technology)) => {
            return Err(PlannerError::ExtractorLocked {
                entity: entity.to_string(),
                extractor,
                technology,
            });
        }
        Some(RecipeGate::Unobtainable) => {
            return Err(PlannerError::NoExtractor {
                entity: entity.to_string(),
                why: format!(
                    "a {extractor} mines it, and its recipe is disabled with no technology \
                         to unlock it"
                ),
            });
        }
        None => {
            return Err(PlannerError::NoExtractor {
                entity: entity.to_string(),
                why: format!("a {extractor} mines it, and no recipe in this world makes one"),
            });
        }
    }
    // An extractor whose draw this planner does not know is one it cannot
    // decide is powered, and `Condition::Powered` would read the silence
    // as zero draw and pass. That is the one table in `crate::state` whose
    // unknown name errs towards permitting, so this refuses on its behalf.
    let Some(kw) = state.consumer_draw_kw(&extractor) else {
        return Err(PlannerError::ExtractionNotModelled {
            entity: entity.to_string(),
            extractor,
        });
    };

    let site = choose_site(state, entity, &extractor, origin)?;
    let Some(area) = state.collision_area(&extractor, &site) else {
        // Unreachable in practice: `choose_site` only returns a tile
        // `is_area_free_facing` accepted, which needs the same prototype.
        return Err(PlannerError::ExtractionNotModelled {
            entity: entity.to_string(),
            extractor,
        });
    };
    Ok(SitedExtractor {
        name: extractor,
        kw,
        site,
        area,
    })
}

/// The steps that stand `sited` up: its bill, whatever power it needs, and
/// the `Place` that creates it -- with `unlocks`' `Effect::Researched` riding
/// on that placement.
///
/// Returns the placement's id alongside the steps, because a caller that
/// attaches anything downstream of the machine (`method::gather`'s pipe run)
/// needs an edge to it and `infer_edges` cannot draw one: nothing an emitted
/// pipe requires is *satisfied* by the pumpjack existing.
///
/// `reserved` is ground a caller has already committed to but has not yet
/// emitted -- `method::gather`'s pipe run, computed before this is called so
/// that it can refuse without leaving anything behind. It is handed to
/// `ensure_powered` as occupants, because the pole run is sited here and
/// would otherwise take a tile the caller is about to place a pipe on: the
/// pipe's own `AreaFree` would then fail at execution, after the plan had
/// been called good. `Extract` passes an empty slice.
pub(crate) fn extractor_steps(
    ctx: &mut ExpansionCtx,
    entity: &str,
    sited: &SitedExtractor,
    unlocks: Option<&str>,
    reserved: &[factorio_bot_core::types::FactorioEntity],
) -> Result<(Vec<Step>, crate::ids::ActionId), PlannerError> {
    let SitedExtractor {
        name: extractor,
        kw,
        site,
        area,
    } = sited;
    let (extractor, kw, site, area) = (extractor.clone(), *kw, site.clone(), area.clone());
    {
        let mut steps: Vec<Step> = Vec::new();
        // The machine itself, first, for the same reason `power::plant_steps`
        // bills before it places: a shortfall refuses before any ground is
        // reserved.
        steps.push(Step::Subgoal(Goal::Have {
            item: extractor.clone(),
            count: 1,
            whose: Holder::Share(ctx.chain_actor),
        }));

        // The one place that decides power, shared with `method::assemble`
        // and `method::blueprint`. The extractor's own ground is handed over
        // as an occupant so a pole cannot be sited on the tile the machine is
        // about to stand on; it is not reserved in `ctx.state` until the
        // `Place` below, so a refusal between here and there leaves nothing
        // behind.
        //
        // `Ok(None)` is "no pole run of at most `MAX_POLE_RUN` carries power
        // here, or the model cannot see the finished run carrying it".
        // Carrying power that far is a thing this planner does not model,
        // which is what the refusal says.
        let mut occupants = vec![extractor_entity(&ctx.state, &extractor, &site)];
        occupants.extend_from_slice(reserved);
        let powering = ensure_powered(
            ctx,
            &extractor,
            &site,
            &area,
            kw,
            SUPPLY_SEARCH_RADIUS,
            &occupants,
        )?
        .ok_or_else(|| PlannerError::ExtractionNotModelled {
            entity: entity.to_string(),
            extractor: extractor.clone(),
        })?;
        steps.extend(powering.steps);
        let power_ids = powering.ids;
        let powered = powering.powered;

        let build = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.build_distance)
            .unwrap_or(10.0);
        let entity_to_place = extractor_entity(&ctx.state, &extractor, &site);
        let place_id = ctx.ids.next();
        let mut eff = vec![
            Effect::LoseItem {
                who: Actor::Role,
                item: extractor.clone(),
                count: 1,
            },
            Effect::CreateEntity(Box::new(entity_to_place.clone())),
        ];
        // The unlock rides on the placement, which is the action that makes
        // the machine exist and therefore the last one this plan performs
        // before the game's own trigger fires. `attach_unlock` cannot be used:
        // it finds its action by an `Effect::GainItem` for the goal's item,
        // and nothing here gains an item -- what comes out of a well is a
        // fluid no inventory can hold, which is why `Goal::Extracted` is not a
        // `Goal::Produced` in the first place.
        if let Some(tech) = unlocks {
            eff.push(Effect::Researched(tech.to_string()));
        }
        steps.push(Step::Act(Box::new(Action {
            id: place_id,
            kind: ActionKind::Place {
                entity: Box::new(entity_to_place.clone()),
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: site.clone(),
                    radius: build,
                    min_radius: ctx.state.placement_clearance(&extractor).unwrap_or(0.0),
                },
                Condition::AreaFree {
                    pos: site.clone(),
                    entity: extractor.clone(),
                    direction: NORTH,
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: extractor.clone(),
                    count: 1,
                },
                // Coverage is not capacity: this is the headroom test, and it
                // is what makes the plant or the pole run above load-bearing
                // rather than decorative.
                powered,
            ],
            eff,
            duration: PLACE_TICKS,
            pinned: None,
            label: format!("place {extractor} at {site}"),
        })));
        ctx.state.create_entity(entity_to_place);

        // Nothing satisfies `Condition::Powered`, so `infer_edges` draws no
        // edge from the plant or the poles to the placement that needs them --
        // the same gap `power::plant_steps` hands its ids back to close. The
        // method holds both ends, so the method states the edges.
        for id in power_ids {
            steps.push(Step::Link {
                from: id,
                to: place_id,
                lag: 0,
            });
        }
        Ok((steps, place_id))
    }
}

/// Where a refusal about the map is measured from: the chain actor.
pub(crate) fn origin_of(ctx: &ExpansionCtx) -> Position {
    ctx.state
        .bot(ctx.chain_actor)
        .map(|bot| bot.position.clone())
        .unwrap_or_default()
}

/// What mining `entity` yields, for a refusal that has to name an item: the
/// first product of the resource prototype, or the entity's own name when the
/// capture has none. Crude oil is both.
fn product_of(state: &PlanState, entity: &str) -> String {
    state
        .mine_products(entity)
        .into_iter()
        .next()
        .unwrap_or_else(|| entity.to_string())
}

/// The machine that mines `entity`: the `mining-drill` whose
/// `resource_categories` list the entity's `resource_category`, smallest
/// name first when several do (`electric-mining-drill` over `pumpjack` is
/// not a case that arises -- vanilla gives fluids their own category).
///
/// `Err` is [`PlannerError::NoExtractor`], and it says which of the three
/// things it needed was missing: a resource prototype, its category, or a
/// drill listing that category.
pub(crate) fn extractor_for(state: &PlanState, entity: &str) -> Result<String, PlannerError> {
    if !state.is_resource(entity) {
        return Err(PlannerError::NoExtractor {
            entity: entity.to_string(),
            why: "it is not a resource prototype this world knows".to_string(),
        });
    }
    let Some(category) = state.resource_category(entity) else {
        return Err(PlannerError::NoExtractor {
            entity: entity.to_string(),
            why: "the world does not report its resource category, so nothing can be matched \
                  against it (a capture from before the mod sent `resource_category`)"
                .to_string(),
        });
    };
    state
        .extractors_for(&category)
        .into_iter()
        .next()
        .ok_or_else(|| PlannerError::NoExtractor {
            entity: entity.to_string(),
            why: format!("no mining drill in this world lists its category, {category}"),
        })
}

/// The refusals that are facts about the world rather than about the plan:
/// the entity is not charted, or nothing mines it. `None` when both hold.
///
/// Asked by [`crate::method::have::Researched`] before it plans a
/// technology's prerequisites, because neither answer changes with research.
pub(crate) fn world_refusal(
    state: &PlanState,
    entity: &str,
    origin: &Position,
) -> Option<PlannerError> {
    if !state.has_resource_patches(entity) {
        return Some(not_charted(state, entity, origin));
    }
    extractor_for(state, entity).err()
}

/// "Unexplored", measured from `origin` -- the same refusal `Mine` raises
/// for an ore nothing has charted, so a script acts on both the same way.
pub(crate) fn not_charted(state: &PlanState, entity: &str, origin: &Position) -> PlannerError {
    PlannerError::NotCharted {
        item: product_of(state, entity),
        resource: entity.to_string(),
        charting: Box::new(state.charting_summary(origin, crate::score::DEFAULT_SEARCH_RADIUS)),
    }
}

/// The whole ladder, as the module doc lists it.
pub(crate) fn refusal_for(state: &PlanState, entity: &str, origin: &Position) -> PlannerError {
    if let Some(refusal) = world_refusal(state, entity, origin) {
        return refusal;
    }
    let extractor = match extractor_for(state, entity) {
        Ok(extractor) => extractor,
        Err(refusal) => return refusal,
    };
    let Some(recipe) = recipe_for(state, &extractor) else {
        return PlannerError::NoExtractor {
            entity: entity.to_string(),
            why: format!("a {extractor} mines it, and no recipe in this world makes one"),
        };
    };
    match recipe_gate(state, &recipe) {
        RecipeGate::NeedsResearch(technology) => PlannerError::ExtractorLocked {
            entity: entity.to_string(),
            extractor,
            technology,
        },
        RecipeGate::Unobtainable => PlannerError::NoExtractor {
            entity: entity.to_string(),
            why: format!(
                "a {extractor} mines it, and its recipe is disabled with no technology to \
                 unlock it"
            ),
        },
        RecipeGate::Open | RecipeGate::PlannedResearch(_) => PlannerError::ExtractionNotModelled {
            entity: entity.to_string(),
            extractor,
        },
    }
}

// ---------------------------------------------------------------------------
// Siting
// ---------------------------------------------------------------------------

/// The `FactorioEntity` an extractor placement creates.
///
/// The same body as [`crate::method::power::entity_for`], and written out
/// rather than called for one reason: `PlantPart::name` is a `&'static str`
/// because a plant is made of six named constants, while an extractor's name
/// is read out of the world at expansion time and has no static lifetime.
/// `entity_for` is still used for the poles, which *are* a constant.
///
/// `entity_type` is read from the prototype rather than guessed -- a
/// pumpjack's is `mining-drill`, which is not its name, and `EntityGraph::add`
/// keys its whitelist on the pair.
fn extractor_entity(
    state: &PlanState,
    name: &str,
    position: &Position,
) -> factorio_bot_core::types::FactorioEntity {
    let entity_type = state
        .base()
        .entity_prototypes
        .get(name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| name.to_string());
    factorio_bot_core::types::FactorioEntity {
        name: name.to_string(),
        entity_type,
        position: position.clone(),
        direction: NORTH,
        ..Default::default()
    }
}

/// Which charted tile of `entity` the extractor stands on.
///
/// **Nearest to the actor first, and every charted tile is a candidate.**
/// There is no radius bound, for the reason `method::power`'s
/// `PLANT_WATER_SCAN_RADIUS` doc sets out at length: the walk is already
/// priced by [`crate::schedule`], so a distant well is a *worse plan* rather
/// than an impossible one, and a bound here would refuse a journey the run
/// makes routinely. The cost of looking is small -- crude oil is seven charted
/// tiles on seed 31337 after one exploration ring, against iron's 2,452.
///
/// # Centred on the well, not beside it
///
/// A `mining-drill` works the tiles inside its own `mining_drill_radius`
/// (`FactorioEntityPrototype::mining_drill_radius`, captured live since
/// `14c54bbf`), and a **pumpjack's is 0.49** -- one tile, the one it is centred
/// on, despite a 3x3 collision box. Reading the box as the working area places
/// the machine on eight tiles that yield nothing. Centring on the resource tile
/// is the choice that is right at *every* radius, including that one, so
/// nothing here reads the field: it is `None` on every dump written before
/// today (`workspace/scripts/map.json` included), and `None` means **unknown
/// reach, never zero reach**. A siting rule that needed the number would give
/// the right answer for a pumpjack by accident and the wrong one for
/// everything else.
///
/// # The half-tile
///
/// [`crate::state::PlanState::resource_patches`] hands back tile **centres**:
/// `EntityGraph`'s `resources` map is keyed by `Pos`, which floors, and
/// `resource_patches` puts the `.5` back before the positions leave the
/// process. A pumpjack's collision box is 2.4 tiles across, so its own
/// placement grid is tile centres too -- the two agree, and the candidate is
/// used exactly as it arrives. Rounding it would reintroduce the corner-for-
/// centre bug that once made mining fail for every ore on every map while
/// every test passed, because the test helper builds ore at integer positions,
/// the one input for which the lossy round trip is lossless.
/// [`the_sited_tile_is_a_resource_entitys_own_position`] is that claim as a
/// test, asserted against a resource entity's position rather than against
/// this function's own arithmetic.
///
/// Deterministic: `(distance, x, y)` with `total_cmp`, over patches whose own
/// elements `PlanState::resource_patches` has already sorted.
fn choose_site(
    state: &PlanState,
    entity: &str,
    extractor: &str,
    origin: &Position,
) -> Result<Position, PlannerError> {
    let mut tiles: Vec<(f64, Position)> = state
        .resource_patches(entity)
        .into_iter()
        .flat_map(|patch| patch.elements)
        .map(|position| (calculate_distance(&position, origin), position))
        .collect();
    tiles.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.1.y.total_cmp(&b.1.y))
    });
    let searched = tiles
        .last()
        .map(|(distance, _)| *distance)
        .unwrap_or_default();
    // A machine has to sit on its own build grid, and an even footprint's grid
    // is tile *boundaries* while a resource entity is always a tile *centre*.
    // The two coincide for a pumpjack (3x3, odd) and do not for a
    // `burner-mining-drill` (2x2, even), so a centred burner drill is an
    // illegal position rather than a tight fit. Refusing here is the honest
    // answer: nothing in this method knows how to work a well from beside it.
    let (grid_x, grid_y) = tile_alignment_facing(state, extractor, Direction::North);
    let on_grid = |p: &Position| {
        let fract = |v: f64, offset: f64| (v - offset).fract().abs() < 1. / 512.;
        fract(p.x(), grid_x) && fract(p.y(), grid_y)
    };
    let mut obstruction: Option<String> = None;
    for (_, tile) in &tiles {
        if !on_grid(tile) {
            obstruction.get_or_insert_with(|| {
                format!(
                    "a {extractor} does not stand on a tile centre, and a \
                     resource entity is always at one"
                )
            });
            continue;
        }
        if state.is_site_refused(extractor, tile) {
            obstruction.get_or_insert_with(|| {
                "a footprint the game already refused a build at".to_string()
            });
            continue;
        }
        if state.is_area_free_facing(extractor, tile, Direction::North) {
            return Ok(tile.clone());
        }
        if let Some(occupant) = state.placement_occupant(extractor, tile, Direction::North) {
            obstruction.get_or_insert_with(|| occupant.to_string());
        }
    }
    Err(PlannerError::NoSiteFound {
        entities: 1,
        seed: origin.to_string(),
        searched: searched.ceil() as i32,
        nearest_obstruction: obstruction
            .unwrap_or_else(|| format!("no {entity} tile is charted at all")),
    })
}

#[cfg(test)]
mod extract_siting_tests {
    use super::*;
    use crate::ids::BotId;
    use crate::test_world::{OilFixture, PumpjackRecipe, world_with_oil};
    use factorio_bot_core::types::{FactorioEntity, Rect};
    use std::sync::Arc;

    /// The world the oil ladder's tests already use, with the pumpjack recipe
    /// open so siting is actually reached.
    ///
    /// **This fixture was not written for this code.** `world_with_oil` and its
    /// twelve wells predate this method by a day and belong to the refusal
    /// ladder's own tests; the wells are placed where a resumed workspace
    /// (`run-1788538389-09170`) held them. That matters because
    /// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md` is
    /// about fixtures written by the author of the code they test. Two things
    /// here still are mine and are named rather than buried: the decoy entities
    /// in the occupied-well tests, and the two-pole world below.
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
        Goal::Extracted {
            entity: "crude-oil".into(),
            unlocks: Some("oil-processing".into()),
        }
    }

    fn places<'a>(steps: &'a [Step], name: &str) -> Vec<&'a Action> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) => Some(action.as_ref()),
                _ => None,
            })
            .filter(|action| {
                matches!(&action.kind, ActionKind::Place { entity } if entity.name == name)
            })
            .collect()
    }

    // -- the constant this module duplicates ---------------------------------

    /// [`WIRE_REACH`] is a copy of a private table in `crate::state`, so it is
    /// pinned against **that table's behaviour** rather than against itself:
    /// two poles exactly `WIRE_REACH` apart share a network, and two poles half
    /// a tile further do not.
    ///
    /// The oracle is `PlanState::nearest_supply_anchor`, which does the
    /// union-find over the poles' real reaches -- nothing in this test names
    /// 7.5 except the constant under test. A comparison against a second copy
    /// of the number would agree with the code that wrote it.
    #[test]
    fn two_poles_a_wire_reach_apart_are_one_network() {
        // A steam engine, a pole covering it, and a second pole at `gap`.
        // `nearest_supply_anchor` asked at the far pole answers only if the
        // two poles are wired, because the generation is all at the near one.
        let reachable = |gap: f64| {
            let mut state = oil_state(OPEN).fork();
            let engine = Position::new(-100.5, -100.5);
            state.create_entity(FactorioEntity {
                name: "steam-engine".into(),
                entity_type: "generator".into(),
                position: engine.clone(),
                ..Default::default()
            });
            let near = Position::new(engine.x() + 2.5, engine.y());
            state.create_entity(FactorioEntity {
                name: POLE.into(),
                entity_type: "electric-pole".into(),
                position: near.clone(),
                ..Default::default()
            });
            let far = Position::new(near.x() + gap, near.y());
            state.create_entity(FactorioEntity {
                name: POLE.into(),
                entity_type: "electric-pole".into(),
                position: far.clone(),
                ..Default::default()
            });
            state.nearest_supply_anchor(&far, 0.5, 90.).is_some()
        };
        assert!(
            reachable(0.),
            "control: one pole on the engine carries 900 kW, or this test \
             measures nothing"
        );
        assert!(
            reachable(WIRE_REACH),
            "two poles exactly WIRE_REACH apart must be wired, or every run \
             this module lays is one pole too sparse"
        );
        assert!(
            !reachable(WIRE_REACH + 0.5),
            "two poles further apart than WIRE_REACH must NOT be wired, or the \
             constant is smaller than the game's and this test proves nothing"
        );
    }

    // -- siting --------------------------------------------------------------

    /// The half-tile, asserted against a **resource entity's own position**
    /// rather than against this module's arithmetic.
    ///
    /// `EntityGraph` keys resources by `Pos`, which floors, and
    /// `resource_patches` restores the `.5`. A site that had been rounded would
    /// be an integer position, which `surface.find_entity` matches nothing at
    /// -- the defect that once made mining fail for every ore on every map.
    #[test]
    fn the_sited_tile_is_a_resource_entitys_own_position() {
        let state = oil_state(OPEN);
        let site = choose_site(&state, "crude-oil", "pumpjack", &Position::default())
            .expect("the fixture charts twelve wells");
        let wells: Vec<Position> = state
            .base()
            .entity_graph
            .resource_patches("crude-oil")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect();
        assert!(
            wells.iter().any(|well| well == &site),
            "the site {site} is not one of the charted wells {wells:?}"
        );
        assert!(
            (site.x().fract().abs() - 0.5).abs() < 1e-9
                && (site.y().fract().abs() - 0.5).abs() < 1e-9,
            "a resource entity sits at a tile CENTRE; {site} is not one"
        );
    }

    /// Nearest to the acting bot first, with `(distance, x, y)` breaking ties.
    #[test]
    fn the_nearest_charted_well_is_taken_first() {
        let state = oil_state(OPEN);
        let from = Position::new(200.5, 20.5);
        let site = choose_site(&state, "crude-oil", "pumpjack", &from).expect("a well is charted");
        let nearest = state
            .base()
            .entity_graph
            .resource_patches("crude-oil")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .min_by(|a, b| {
                calculate_distance(a, &from)
                    .total_cmp(&calculate_distance(b, &from))
                    .then(a.x.total_cmp(&b.x))
            })
            .expect("a well is charted");
        assert_eq!(site, nearest, "asked from {from}");
    }

    /// A well with something standing on it is passed over, not built into.
    #[test]
    fn an_occupied_well_is_passed_over_for_the_next_one() {
        let mut state = oil_state(OPEN).fork();
        let first = choose_site(&state, "crude-oil", "pumpjack", &Position::default())
            .expect("a well is charted");
        // A decoy of a name no part of this method places, so nothing can read
        // it as "the extractor is already there".
        state.create_entity(FactorioEntity {
            name: "wooden-chest".into(),
            entity_type: "container".into(),
            position: first.clone(),
            bounding_box: Rect::new(
                &Position::new(first.x() - 0.4, first.y() - 0.4),
                &Position::new(first.x() + 0.4, first.y() + 0.4),
            ),
            ..Default::default()
        });
        let second = choose_site(&state, "crude-oil", "pumpjack", &Position::default())
            .expect("eleven wells are left");
        assert_ne!(second, first, "the occupied well was chosen again");
    }

    /// Every well occupied is a refusal that names what is on the ground, not
    /// a silent choice of a tile the game would refuse.
    #[test]
    fn every_well_occupied_refuses_and_names_the_obstruction() {
        let mut state = oil_state(OPEN).fork();
        for well in state
            .base()
            .entity_graph
            .resource_patches("crude-oil")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect::<Vec<_>>()
        {
            state.create_entity(FactorioEntity {
                name: "wooden-chest".into(),
                entity_type: "container".into(),
                position: well.clone(),
                bounding_box: Rect::new(
                    &Position::new(well.x() - 0.4, well.y() - 0.4),
                    &Position::new(well.x() + 0.4, well.y() + 0.4),
                ),
                ..Default::default()
            });
        }
        let err = choose_site(&state, "crude-oil", "pumpjack", &Position::default())
            .expect_err("every well is taken");
        match &err {
            PlannerError::NoSiteFound {
                nearest_obstruction,
                ..
            } => assert!(
                nearest_obstruction.contains("wooden-chest"),
                "the refusal must name what is standing there, got \
                 {nearest_obstruction}"
            ),
            other => panic!("expected NoSiteFound, got {other}"),
        }
    }

    // -- the whole method ----------------------------------------------------

    /// The deliverable: a pumpjack stands on a charted well, and it is placed
    /// only where the plan can show it 90 kW of **headroom** -- coverage is not
    /// capacity, so the precondition is `Condition::Powered` and not an
    /// `EntityAt` for a pole.
    #[test]
    fn a_pumpjack_is_sited_on_a_well_and_its_placement_states_its_power() {
        let state = oil_state(OPEN);
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        let steps = Extract
            .expand(&goal(), &mut ctx)
            .expect("a charted well, an open recipe and a lake to power it from");
        let placed = places(&steps, "pumpjack");
        assert_eq!(placed.len(), 1, "exactly one pumpjack");
        let site = placed[0]
            .kind
            .target_position()
            .expect("a Place names a position");
        let wells: Vec<Position> = state
            .base()
            .entity_graph
            .resource_patches("crude-oil")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect();
        assert!(wells.contains(&site), "{site} is not a charted well");
        assert!(
            placed[0].pre.iter().any(|c| matches!(
                c,
                Condition::Powered { pos, entity, kw }
                    if pos == &site && entity == "pumpjack" && *kw == 90.
            )),
            "the placement must state its own 90 kW headroom: {:?}",
            placed[0].pre
        );
    }

    /// The unlock rides on the placement, because the placement is the last
    /// thing this plan does before the game's own `mine-entity` trigger fires.
    #[test]
    fn the_unlock_rides_on_the_extractors_placement() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let steps = Extract.expand(&goal(), &mut ctx).expect("it plans");
        let placed = places(&steps, "pumpjack");
        assert!(
            placed[0]
                .eff
                .contains(&Effect::Researched("oil-processing".into())),
            "no Effect::Researched on the pumpjack placement: {:?}",
            placed[0].eff
        );
        // And nowhere else: a second copy would let the technology be marked
        // by an action that does not make the machine exist.
        let carriers = steps
            .iter()
            .filter(|step| {
                matches!(step, Step::Act(a)
                if a.eff.contains(&Effect::Researched("oil-processing".into())))
            })
            .count();
        assert_eq!(carriers, 1, "exactly one action carries the unlock");
    }

    /// A goal with no `unlocks` -- `goal.extracted("crude-oil")` from a script
    /// -- plans the same cell and marks no technology.
    #[test]
    fn an_extraction_goal_with_no_unlock_marks_no_technology() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let steps = Extract
            .expand(
                &Goal::Extracted {
                    entity: "crude-oil".into(),
                    unlocks: None,
                },
                &mut ctx,
            )
            .expect("it plans");
        assert_eq!(places(&steps, "pumpjack").len(), 1);
        assert!(
            !steps.iter().any(|step| matches!(step, Step::Act(a)
                if a.eff.iter().any(|e| matches!(e, Effect::Researched(_))))),
            "nothing asked for a technology, so nothing may mark one"
        );
    }

    /// The machine is billed as a `Goal::Have`, so a roster that cannot make a
    /// pumpjack refuses through the ordinary shortfall machinery rather than
    /// planning a placement of something nobody holds.
    #[test]
    fn the_extractor_is_billed_before_it_is_placed() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let steps = Extract.expand(&goal(), &mut ctx).expect("it plans");
        let first_bill = steps.iter().position(
            |step| matches!(step, Step::Subgoal(Goal::Have { item, .. }) if item == "pumpjack"),
        );
        let placement = steps.iter().position(|step| {
            matches!(step, Step::Act(a)
                if matches!(&a.kind, ActionKind::Place { entity } if entity.name == "pumpjack"))
        });
        assert!(
            matches!((first_bill, placement), (Some(bill), Some(place)) if bill < place),
            "bill at {first_bill:?}, placement at {placement:?}"
        );
    }

    /// Power is carried to the well, and every consecutive pole in the run is
    /// inside a small pole's wire reach of the last.
    ///
    /// The run's own correctness is checked by `Condition::Powered` inside
    /// [`pole_run`]; this asserts the *shape* the emitted steps have, which is
    /// what a reader of the plan sees.
    #[test]
    fn the_poles_that_carry_power_to_the_well_are_within_reach_of_each_other() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let steps = Extract.expand(&goal(), &mut ctx).expect("it plans");
        let poles: Vec<Position> = places(&steps, POLE)
            .into_iter()
            .filter_map(|action| action.kind.target_position())
            .collect();
        assert!(
            poles.len() >= 2,
            "the fixture's lake is ~20 tiles from its nearest well, so a \
             single pole cannot span it; got {poles:?}"
        );
        for pair in poles.windows(2) {
            let gap = calculate_distance(&pair[0], &pair[1]);
            assert!(
                gap <= WIRE_REACH,
                "poles {} and {} are {gap} apart, past a small pole's \
                 {WIRE_REACH}",
                pair[0],
                pair[1]
            );
        }
        // Every pole is billed, or the plan places wood nobody chopped.
        let billed = steps
            .iter()
            .filter(|step| matches!(step, Step::Subgoal(Goal::Have { item, .. }) if item == POLE))
            .count();
        assert_eq!(billed, poles.len(), "one Have per pole placed");
    }

    /// Power ordering is **stated**, because nothing satisfies
    /// `Condition::Powered` and `infer_edges` can draw no edge to it. Without
    /// these links the scheduler may hand the pumpjack to a bot that places it
    /// before the pole run exists.
    #[test]
    fn every_power_placement_is_ordered_before_the_extractor() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let steps = Extract.expand(&goal(), &mut ctx).expect("it plans");
        let place_id = places(&steps, "pumpjack")[0].id;
        let linked: Vec<ActionId> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Link { from, to, .. } if *to == place_id => Some(*from),
                _ => None,
            })
            .collect();
        for pole in places(&steps, POLE) {
            assert!(
                linked.contains(&pole.id),
                "pole {} is not ordered before the pumpjack",
                pole.id.0
            );
        }
        assert!(
            !linked.is_empty(),
            "no power placement is ordered before the extractor at all"
        );
    }

    /// A pumpjack that is already standing and supplied is not built again.
    ///
    /// `holds` cannot answer a `Goal::Extracted`, so `AlreadySatisfied` never
    /// claims one and every replan reaches this method. Re-siting on the *next*
    /// well each time is the shape that built a second power plant in
    /// `run-1788408407-02764`; here the standing machine simply occupies its
    /// own tile, so this pins that a replan does not stack pumpjacks on the
    /// well it already used.
    #[test]
    fn a_replan_does_not_place_a_second_pumpjack_on_the_same_well() {
        let mut ctx = ExpansionCtx::new(oil_state(OPEN).fork(), BotId(1));
        let first = Extract.expand(&goal(), &mut ctx).expect("it plans");
        let first_site = places(&first, "pumpjack")[0]
            .kind
            .target_position()
            .expect("a site");
        // The same context, so `ctx.state` still carries what the first
        // expansion reserved -- which is what a second subtree of one plan
        // sees.
        let second = Extract.expand(&goal(), &mut ctx).expect("it plans again");
        let second_site = places(&second, "pumpjack")[0]
            .kind
            .target_position()
            .expect("a site");
        assert_ne!(
            first_site, second_site,
            "the reserved well was chosen twice"
        );
    }

    // -- the refusals that are preserved -------------------------------------

    /// Tier 3 is unchanged: a locked extractor recipe is refused by name and
    /// this method does not claim the goal, so `Method::refusal` is what a
    /// caller is told.
    #[test]
    fn a_locked_extractor_recipe_is_still_refused_and_unclaimed() {
        let state = oil_state(OilFixture {
            pumpjack: PumpjackRecipe::LockedBy { researched: false },
            ..OPEN
        });
        assert!(
            !Extract.applicable(&goal(), &state),
            "a locked recipe must leave the goal unclaimed, or the refusal \
             ladder is bypassed"
        );
        let ctx = ExpansionCtx::new(state.fork(), BotId(1));
        match Extract.refusal(&goal(), &ctx) {
            Some(PlannerError::ExtractorLocked { technology, .. }) => {
                assert_eq!(technology, "oil-gathering")
            }
            other => panic!("expected ExtractorLocked, got {other:?}"),
        }
    }

    /// Tier 1 is unchanged: an uncharted resource leaves the goal unclaimed
    /// and the refusal says where charted ground ends.
    #[test]
    fn an_uncharted_resource_is_still_unclaimed_and_refused_as_not_charted() {
        let state = oil_state(OilFixture {
            wells: false,
            ..OPEN
        });
        assert!(!Extract.applicable(&goal(), &state));
        let ctx = ExpansionCtx::new(state.fork(), BotId(1));
        assert!(matches!(
            Extract.refusal(&goal(), &ctx),
            Some(PlannerError::NotCharted { .. })
        ));
    }

    /// An extractor whose electrical draw `crate::state`'s table does not name
    /// cannot be shown to be powered -- `Condition::Powered` would read the
    /// silence as "draws nothing" and pass. Tier 4 says so by name instead.
    ///
    /// The case is real rather than contrived: a `burner-mining-drill` sorts
    /// before `electric-mining-drill`, so it is what `extractor_for` picks for
    /// any `basic-solid` resource, and it burns coal rather than drawing kW.
    #[test]
    fn an_extractor_with_no_modelled_draw_is_refused_as_not_modelled() {
        let world = world_with_oil(OPEN);
        world
            .entity_prototypes
            .get_mut("iron-ore")
            .expect("the fixture charts iron")
            .resource_category = Some("basic-solid".into());
        // `world_with_oil` only fills in the fluid pair and the character; the
        // solid drills are an older capture with no categories at all, and a
        // drill that lists none is never returned by `extractors_for`.
        world
            .entity_prototypes
            .get_mut("burner-mining-drill")
            .expect("the fixture has a burner drill")
            .resource_categories = Some(vec!["basic-solid".into()]);
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert_eq!(
            extractor_for(&state, "iron-ore").expect("a drill mines iron"),
            "burner-mining-drill",
            "control: the fixture must actually pick the burner drill, or this \
             test is about nothing"
        );
        assert!(
            state.consumer_draw_kw("burner-mining-drill").is_none(),
            "control: a burner drill must have no modelled electrical draw"
        );
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        let err = Extract
            .expand(
                &Goal::Extracted {
                    entity: "iron-ore".into(),
                    unlocks: None,
                },
                &mut ctx,
            )
            .expect_err("nothing here can say a burner drill is powered");
        assert!(
            matches!(&err, PlannerError::ExtractionNotModelled { extractor, .. }
                if extractor == "burner-mining-drill"),
            "got {err}"
        );
    }
}

#[cfg(test)]
mod power_reach_tests {
    //! What a pole run reaches, pinned against [`crate::state::PlanState`]
    //! rather than against `method::extract`.
    //!
    //! It is here because it is what decides whether a charted well can be
    //! powered at all, and because nothing else in the crate states it as a
    //! test -- `POWER_SEARCH_RADIUS`'s own doc argues for it as a cost bound
    //! and does not say what it costs.
    //!
    //! **This header used to answer that with "a pole run is useless past it,
    //! however many poles are in it", five lines above the test that says the
    //! opposite.** Since `PlanState::electric_entities` began following the
    //! wire, what it costs is a seed disc and nothing else: a chain carries as
    //! far as it is unbroken.

    use super::*;
    use crate::ids::BotId;
    use crate::test_world::{OilFixture, PumpjackRecipe, world_with_oil};
    use factorio_bot_core::types::FactorioEntity;
    use std::sync::Arc;

    /// A chain of poles carries power however long it is, and a chain that
    /// stops carries nothing past the break.
    ///
    /// **This test used to assert the opposite**, and its failure message said
    /// so: *"a 150-tile pole chain reads as powered, so `POWER_SEARCH_RADIUS`
    /// is no longer the ceiling on `method::extract`'s reach -- update the
    /// module doc's table and the note, this is good news."* It was written to
    /// pin a limitation and to instruct whoever removed it. That happened on
    /// 2026-09-06: `PlanState::electric_entities` follows the wire out from
    /// the consumer instead of searching one disc around it, so the ceiling is
    /// gone and this test is inverted rather than deleted.
    ///
    /// Keeping the shape matters. It still asserts near *and* far with
    /// identical poles, because a search that walks the wire is only correct
    /// if it also **refuses** — one that counted any generator on the map
    /// would pass a "far works" test on its own. The break case is what makes
    /// this a test of connectivity rather than of optimism.
    #[test]
    fn a_pole_chain_carries_however_long_it_is_but_a_broken_one_does_not() {
        let carries = |span: f64| {
            let world = world_with_oil(OilFixture {
                wells: true,
                categories: true,
                pumpjack: PumpjackRecipe::LockedBy { researched: true },
                prerequisite: false,
            });
            let mut state = PlanState::from_world(Arc::new(world), &[BotId(1)]).fork();
            let engine = Position::new(-200.5, 200.5);
            state.create_entity(FactorioEntity {
                name: "steam-engine".into(),
                entity_type: "generator".into(),
                position: engine.clone(),
                ..Default::default()
            });
            // Poles every 6 tiles, straight, from the engine to the
            // consumer. Starting 2.5 tiles clear of the engine rather than on
            // it: `PlanState`'s overlay is keyed by *tile*, so a pole created
            // on the engine's own tile replaces the engine and the network
            // silently has no generation at all. That is what made the first
            // version of this test fail its own control.
            let mut x = engine.x() + 2.5;
            while x <= engine.x() + span + 3. {
                state.create_entity(FactorioEntity {
                    name: POLE.into(),
                    entity_type: "electric-pole".into(),
                    position: Position::new(x, engine.y()),
                    ..Default::default()
                });
                x += 6.;
            }
            let consumer = Position::new(engine.x() + span, engine.y());
            Condition::Powered {
                pos: consumer,
                entity: "pumpjack".into(),
                kw: 90.,
            }
            .holds(&state, BotId(1))
        };
        assert!(
            carries(30.),
            "control: a short chain of poles must carry 900 kW, or this test \
             measures nothing at all"
        );
        assert!(
            carries(150.),
            "a 150-tile chain of wired poles must carry power: the search \
             follows the wire, not a radius"
        );
        assert!(
            carries(400.),
            "and 400 tiles too -- seed 31337's crude oil is 256-384 tiles out, \
             which is the distance this exists to reach"
        );
    }

    /// The other direction, and the one that stops the traversal being a
    /// licence to count any generator on the map.
    ///
    /// Same engine, same consumer, but the pole chain stops a third of the way
    /// there. Nothing bridges the gap, so nothing carries power.
    #[test]
    fn a_chain_that_stops_short_carries_nothing() {
        let world = world_with_oil(OilFixture {
            wells: true,
            categories: true,
            pumpjack: PumpjackRecipe::LockedBy { researched: true },
            prerequisite: false,
        });
        let mut state = PlanState::from_world(Arc::new(world), &[BotId(1)]).fork();
        let engine = Position::new(-200.5, 200.5);
        state.create_entity(FactorioEntity {
            name: "steam-engine".into(),
            entity_type: "generator".into(),
            position: engine.clone(),
            ..Default::default()
        });
        // Poles for the first 50 tiles only; the consumer is at 150.
        let mut x = engine.x() + 2.5;
        while x <= engine.x() + 50. {
            state.create_entity(FactorioEntity {
                name: POLE.into(),
                entity_type: "electric-pole".into(),
                position: Position::new(x, engine.y()),
                ..Default::default()
            });
            x += 6.;
        }
        let consumer = Position::new(engine.x() + 150., engine.y());
        assert!(
            !Condition::Powered {
                pos: consumer,
                entity: "pumpjack".into(),
                kw: 90.,
            }
            .holds(&state, BotId(1)),
            "the wire stops 100 tiles short, so the consumer is not on the network"
        );
    }
}

#[cfg(test)]
mod extractor_grid_tests {
    //! An extractor has to be **centred on the well**, and not every
    //! extractor's build grid allows that.

    use super::*;
    use crate::ids::BotId;
    use crate::test_world::{OilFixture, PumpjackRecipe, world_with_oil};
    use factorio_bot_core::types::FactorioEntity;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(
            Arc::new(world_with_oil(OilFixture {
                wells: true,
                categories: true,
                pumpjack: PumpjackRecipe::LockedBy { researched: true },
                prerequisite: false,
            })),
            &[BotId(1)],
        )
    }

    /// A pumpjack works exactly the tile it stands on, so this method's whole
    /// siting rule depends on its 3x3 footprint being **odd** -- an odd
    /// footprint's build grid is tile centres, which is where every resource
    /// entity sits.
    ///
    /// Compared in **tiles**, not in collision-box extents: Factorio sizes a
    /// 2x2 box slightly under 2 tiles so neighbours do not touch, and a
    /// predicate written against that shaved number measures the shaving.
    #[test]
    fn a_pumpjacks_build_grid_is_the_tile_centre_a_well_sits_on() {
        let s = state();
        assert_eq!(
            crate::method::util::tile_alignment_facing(&s, "pumpjack", Direction::North),
            (0.5, 0.5),
            "a pumpjack must stand on a tile centre, or it cannot be centred \
             on a well at all"
        );
        assert_eq!(
            crate::method::util::tile_alignment_facing(&s, "burner-mining-drill", Direction::North),
            (0., 0.),
            "control: an even footprint's grid is tile boundaries, which is \
             the case the guard below exists for"
        );
    }

    /// The run's last check refuses a chain the *model* cannot see carrying
    /// power, and this exercises it directly because no fixture can reach it
    /// through `expand`.
    ///
    /// **Distance is no longer the reason, and that is the point of the
    /// rewrite.** This test used to put the generator 150 tiles away and rely
    /// on `POWER_SEARCH_RADIUS` being unable to see it. Since
    /// `PlanState::electric_entities` follows the wire, a 150-tile run of
    /// poles the plan itself laid *does* carry power, so that construction
    /// stopped testing anything — it asserted a refusal that had become wrong.
    ///
    /// The check it guards is still real, so the cause was replaced rather
    /// than the test deleted: here the anchor pole is **not wired to the
    /// generator at all**, 40 tiles from it against a small pole's 7.5 wire
    /// reach. `pole_run` will still happily lay a legal chain from that anchor
    /// to the site — the route search only asks whether each tile is free —
    /// and the scheduler checks the same `Condition::Powered` later, so
    /// emitting the run would put a pumpjack in the plan that nothing could
    /// ever schedule. Refusing here turns that into a named refusal at
    /// expansion time.
    #[test]
    fn a_run_whose_power_the_model_cannot_see_is_refused_rather_than_emitted() {
        let mut trial = state().fork();
        // A real, generating network -- but far away.
        let engine = Position::new(-150.5, 20.5);
        trial.create_entity(FactorioEntity {
            name: "steam-engine".into(),
            entity_type: "generator".into(),
            position: engine.clone(),
            ..Default::default()
        });
        // 40 tiles clear of the engine: far beyond a small pole's 7.5-tile
        // wire reach, so this anchor is on no network at all.
        let anchor = Position::new(engine.x() + 40., engine.y());
        trial.create_entity(FactorioEntity {
            name: POLE.into(),
            entity_type: "electric-pole".into(),
            position: anchor.clone(),
            ..Default::default()
        });
        let site = choose_site(&trial, "crude-oil", "pumpjack", &Position::default())
            .expect("a well is charted");
        let area = trial
            .collision_area("pumpjack", &site)
            .expect("a pumpjack has a footprint");
        trial.create_entity(extractor_entity(&trial, "pumpjack", &site));
        let powered = Condition::Powered {
            pos: site.clone(),
            entity: "pumpjack".into(),
            kw: 90.,
        };
        assert!(
            calculate_distance(&anchor, &engine) > 7.5,
            "control: the anchor must be off the generator's network, or this \
             test is about nothing; it is {} tiles from the engine",
            calculate_distance(&anchor, &engine)
        );
        assert!(
            !Condition::Powered {
                pos: anchor.clone(),
                entity: "pumpjack".into(),
                kw: 90.,
            }
            .holds(&trial, BotId(1)),
            "control: the anchor itself must be unpowered, or the run would be \
             right to succeed"
        );
        assert_eq!(
            pole_run(&mut trial, &anchor, &site, &area, &powered, BotId(1))
                .expect("no shortfall is possible here"),
            None,
            "a run the model cannot see carrying power must be refused"
        );
    }

    /// An extractor whose grid is tile boundaries cannot be centred on a well,
    /// and is refused by name rather than placed half a tile off where the
    /// game would reject it.
    #[test]
    fn an_even_footprint_extractor_cannot_be_centred_on_a_well() {
        let s = state();
        let err = choose_site(&s, "crude-oil", "burner-mining-drill", &Position::default())
            .expect_err("a 2x2 drill cannot stand on a tile centre");
        match &err {
            PlannerError::NoSiteFound {
                nearest_obstruction,
                ..
            } => assert!(
                nearest_obstruction.contains("tile centre"),
                "the refusal must say why, got {nearest_obstruction}"
            ),
            other => panic!("expected NoSiteFound, got {other}"),
        }
        // Control: the same call for the pumpjack succeeds, so the refusal is
        // about the grid and not about the wells.
        assert!(choose_site(&s, "crude-oil", "pumpjack", &Position::default()).is_ok());
    }
}
