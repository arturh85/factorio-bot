//! Extraction: mining a resource a hand cannot work, by a machine standing
//! on it.
//!
//! [`Goal::Extracted`] is what a Factorio 2.0 `mine-entity` trigger becomes
//! when the entity it names is one a character cannot dig -- `oil-processing`
//! names `crude-oil`, and `character.mine_entity(crude-oil)` answers false
//! (verified live 2026-09-04). The game satisfies that trigger when a
//! **pumpjack** extracts from a well.
//!
//! **No method sites a pumpjack yet**, so [`Extract`] claims nothing. What it
//! does is answer [`crate::method::Method::refusal`] with the *next missing
//! prerequisite*, in the order a reader can act on them:
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

use crate::error::PlannerError;
use crate::goal::Goal;
use crate::method::util::{RecipeGate, recipe_for, recipe_gate};
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::types::Position;

/// The refusal for [`Goal::Extracted`]. Claims nothing; see the module doc.
pub struct Extract;

impl Method for Extract {
    fn name(&self) -> &'static str {
        "extract"
    }

    /// Never. Nothing here can stand an extractor up, and claiming the goal
    /// only to refuse in `expand` would keep the driver from asking every
    /// other method for a refusal first.
    fn applicable(&self, _goal: &Goal, _state: &PlanState) -> bool {
        false
    }

    fn expand(&self, goal: &Goal, _ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        Err(PlannerError::NoApplicableMethod {
            goal: goal.to_string(),
        })
    }

    fn refusal(&self, goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
        let Goal::Extracted { entity, .. } = goal else {
            return None;
        };
        Some(refusal_for(&ctx.state, entity, &origin_of(ctx)))
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
