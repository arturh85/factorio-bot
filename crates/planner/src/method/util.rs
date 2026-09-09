//! Helpers shared by more than one method.

use crate::action::{Action, ActionKind, Actor, Condition};
use crate::error::PlannerError;
use crate::ids::{ActionId, Ticks};
use crate::method::{ExpansionCtx, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::ToPrimitive;
use factorio_bot_core::types::{
    Direction, FactorioRecipe, FactorioTechnology, Position, Rect, ResearchTrigger, ResourcePatch,
};
use std::collections::BTreeMap;

const TICKS_PER_SECOND: f64 = 60.0;

/// How close to the chosen escape tile counts as "there", for an evacuation
/// action's own `AtPosition` precondition.
///
/// Kept equal to `crates/executor::run::EVACUATE_RADIUS` by convention, not by
/// a shared constant -- the two crates do not share code across the
/// planner/executor boundary for anything else in this enum either. Half a
/// tile: the escape target is a tile centre on the pathfinder's own grid
/// (`crate::enclosure::CELL` is one tile), so anywhere inside that tile is
/// the tile that was proven safe, and the game's own pathing does not promise
/// to land a character on an exact float.
pub(crate) const EVACUATION_RADIUS: f64 = 0.5;

/// One [`Step::Act`] that walks `evacuation.bot` clear of a footprint before
/// it is built, and the id it must precede every one of the footprint's own
/// placements.
///
/// Pinned to the bystander itself (`Actor::Role` plus `pinned: Some(bot)`),
/// never to `ctx.chain_actor`: the bystander is not the bot doing the
/// building, and binding this to the chain would hand it to whichever bot the
/// scheduler later gives the chain to, which reproduces exactly the bug
/// `PlanState`'s own `characters` field doc already warns against for the
/// acting bot.
///
/// `why` names what the bot is being walked clear of, straight into the
/// label -- the only place this reaches the run record. It dispatches
/// through the ordinary `record.actions()` path like any other action
/// (`ActionDispatched`/`ActionSettled`), and `action` is this label verbatim
/// (`crates/scripting_lua/src/globals/record.rs`), so a run that had to
/// evacuate a bot shows it in the same jsonl every other action already
/// writes to, findable by grepping "evacuate" without a second event schema
/// to learn.
pub(crate) fn evacuation_step(
    ctx: &mut ExpansionCtx,
    evacuation: &crate::enclosure::Evacuation,
    why: &str,
) -> (Step, ActionId) {
    let id = ctx.ids.next();
    let action = Action {
        id,
        kind: ActionKind::Evacuate {
            to: evacuation.to.clone(),
        },
        pre: vec![Condition::AtPosition {
            who: Actor::Role,
            pos: evacuation.to.clone(),
            radius: EVACUATION_RADIUS,
            min_radius: 0.0,
        }],
        eff: Vec::new(),
        duration: 0,
        pinned: Some(evacuation.bot),
        label: format!(
            "evacuate bot {} to {} clear of {why} -- would otherwise lose all \
             {:.2} sq tiles of reachable ground",
            evacuation.bot, evacuation.to, evacuation.pocket_tiles
        ),
    };
    (Step::Act(Box::new(action)), id)
}

/// How far out `free_area_near` will search before giving up, in tiles.
///
/// `pub(crate)` because it bounds how far a *sited* entity can wander from the
/// origin it was sited around, and `crate::method::power::PLANT_ADOPT_RADIUS`
/// has to reach past that: a plant's pole is placed by
/// [`free_area_near_where`] around its engine, so this is one of the three
/// terms in "the furthest a plant this planner builds could end up".
pub(crate) const FREE_TILE_SEARCH_RADIUS: i32 = 12;

/// Convert a recipe's or prototype's seconds into ticks, rounding up so that a
/// positive duration never becomes zero.
pub fn seconds_to_ticks(seconds: f64) -> Ticks {
    if seconds <= 0.0 {
        return 0;
    }
    (seconds * TICKS_PER_SECOND).ceil() as Ticks
}

/// The recipe **named** `item`, or `None`.
///
/// # This is a lookup by recipe name, and a product is not a recipe
///
/// The recipe table is keyed by recipe name. Passing a *product* name works
/// only where the two coincide, and answers `None` -- silently -- where they
/// do not. Measured on `crates/core/tests/live-2.1.17-world-snapshot.json`:
/// 394 of 662 recipes are not named after any of their own products, and **62
/// products have no same-named recipe at all**, including every raw resource
/// (`iron-ore`, `coal`, `stone`, `wood`), every early fluid (`crude-oil`,
/// `petroleum-gas`, `light-oil`, `heavy-oil`, `steam`, `water`) and items the
/// planner will want later (`solid-fuel`, `uranium-235`).
///
/// # Why every caller here is nonetheless correct today
///
/// Over the two categories this planner runs -- [`CRAFTING_CATEGORY`] and
/// [`SMELTING_CATEGORY`] -- that same capture says the product-to-recipe map
/// is **one-to-one, total, and name-preserving**: 194 products, none made by
/// two recipes, none lacking a same-named recipe, and no recipe with more than
/// one product (asserted in `tests/product_index_live_capture.rs`). Every
/// caller lands inside that set: `have`, `produce` and `assemble` gate the
/// result on one of the two categories, and `extract`'s lookup is for a
/// *machine* -- a drill, a pumpjack -- whose recipe is `crafting` and
/// self-named like any other.
///
/// So this function is exact over exactly the set the planner can reach, and
/// wrong immediately outside it -- which is why widening either category gate
/// must move to [`crate::products::ProductIndex`] in the same commit.
///
/// [`crate::products::ProductIndex::sole_recipe_producing`] is the honest
/// lookup: keyed by product, one-to-many in both directions, and it refuses by
/// name in three tiers instead of answering `None`.
pub fn recipe_for(state: &PlanState, item: &str) -> Option<FactorioRecipe> {
    state.base().globals.recipes.get(item).map(|r| r.clone())
}

/// A vanilla character's mining speed, used only when the world carries no
/// usable `character` prototype. See [`character_mining_speed`].
const VANILLA_CHARACTER_MINING_SPEED: f64 = 0.5;

/// How fast the acting character mines by hand, as the world reports it.
///
/// `LuaEntityPrototype::mining_speed` is documented as "the mining speed of
/// this mining drill/**character** prototype", and a vanilla `character` has
/// 0.5. It is read rather than hardcoded because it is prototype data a mod
/// can change; the constant is only the fallback for a world that has no
/// `character` prototype at all, which no real game produces but every
/// hand-built fixture can.
///
/// The acting force's `manual_mining_speed_modifier` multiplies that: "the
/// actual mining speed will be multiplied by `1 + manual_mining_speed_modifier`"
/// (`LuaForce::manual_mining_speed_modifier`). It defaults to 0, and vanilla's
/// `steel-axe` technology grants `character-mining-speed +1`, which doubles
/// hand mining — so this is not a constant either, and is likewise read.
///
/// # The gap this leaves, which meets the above on one technology
///
/// `steel-axe` is a **`research_trigger`** technology (craft 50 steel plates).
/// Both halves are now closed: `FactorioTechnology::research_trigger` carries
/// the trigger, and `trigger_requirement` below turns it into the `Goal::Have`
/// the technology really costs, so a plan that researches `steel-axe` gets the
/// resulting mining *rate* right and the research's own *cost* right too.
/// `electronics`, `steam-power` and `automation-science-pack` are the other
/// trigger technologies in the early tree; live 2.1.17 has 32 in all.
pub fn character_mining_speed(state: &PlanState) -> f64 {
    let base = state
        .base()
        .globals
        .entity_prototypes
        .get("character")
        .and_then(|p| p.mining_speed)
        .filter(|speed| *speed > 0.)
        .unwrap_or(VANILLA_CHARACTER_MINING_SPEED);
    // A modifier of -1 or below would zero or invert the speed. The game does
    // not produce one, but a mod could, and a zero divisor downstream is worse
    // than an unmodified rate.
    let modifier = state.manual_mining_speed_modifier().max(0.);
    base * (1. + modifier)
}

/// Ticks to mine one unit of `item` by hand.
///
/// Hand mining takes `mining_time / mining_speed` seconds — the prototype's
/// `mining_time` is the numerator of a division, not the answer. Dividing was
/// missing, which made every estimate here exactly `1 / 0.5 = 2x` too fast
/// against a vanilla character.
///
/// `mining_time` defaults to one second when the prototype carries none; the
/// divisor comes from [`character_mining_speed`].
pub fn mining_ticks(state: &PlanState, item: &str) -> Ticks {
    let seconds = state
        .base()
        .globals
        .entity_prototypes
        .get(item)
        .and_then(|p| p.mining_time)
        .unwrap_or(1.0);
    seconds_to_ticks(seconds / character_mining_speed(state))
}

/// The **whole** bill one swing at the standing entity `entity` yields, in
/// item order.
///
/// [`mining_ticks`]'s sibling, and read off the same prototype: `mine_result`
/// is the game's own answer to "what does mining this give you".
/// `EntityGraph::minables_yielding` answers the same question from the other
/// side — it is asked about an *item* and reports only that item's share — and
/// that half-answer is exactly what a caller must not act on for a rock. A
/// `huge-rock` yields `{coal, stone}`, and a method that credits only the item
/// it went there for leaves the other half of a real delivery out of the plan,
/// which then goes and fetches it again.
///
/// # The numbers here are the game's *minimum*, not its average
///
/// Vanilla rocks yield a **range**: `huge-rock` is 24–50 of each of coal and
/// stone, `big-sand-rock` 19–25 of stone. Nothing in this crate can sample a
/// range and stay deterministic, so the decision is made upstream and this is
/// where to read about it: `products_to_dict` (`mods/BotBridge/control.lua`)
/// takes `product.amount` when the prototype states one and
/// `product.amount_min` when it states a range, so what reaches
/// `FactorioEntityPrototype::mine_result` — and therefore this function, and
/// therefore every `Effect::GainItem` sized from it — is the **floor** of what
/// the game will actually hand over.
///
/// That is the safe direction and it was worth keeping. A plan sized on the
/// average would be right on average and short on roughly half of all swings,
/// and a short delivery is a goal that fails its own `HasItem` and forces a
/// replan; a plan sized on the floor is never short, and its cost is at most
/// one extra swing (three seconds on a `huge-rock`) that the run does not
/// need. Over-mining is a rounding error against a replan.
///
/// It does mean the numbers here are a *lower bound on reality* rather than a
/// prediction, so a bot that mined two `huge-rock`s for the 48 coal the plan
/// asked for may walk away with 100. Nothing downstream is harmed by arriving
/// with more than it planned for; every reader of an inventory reads the real
/// one.
///
/// An entity with no prototype, or a prototype with no `mine_result`, yields
/// nothing — which refuses the work rather than guessing a bill, exactly as
/// `minables_yielding` does.
pub fn mine_bill(state: &PlanState, entity: &str) -> BTreeMap<String, u32> {
    state
        .base()
        .globals
        .entity_prototypes
        .get(entity)
        .and_then(|proto| proto.mine_result.clone())
        .unwrap_or_default()
}

/// The tile of `item` nearest `from` that still holds at least `need` and has
/// not already been committed to a mining action by this plan.
///
/// Ties on distance are broken by `(x, y)`, so the result depends only on the
/// tile set and the origin — never on the order `resource_patches` happens to
/// return patches in, which is not stable across processes for patches of
/// equal size.
///
/// Reads [`PlanState::resource_unclaimed`], not `resource_available`: this is
/// tile *selection*, and every selector in this module has to see the same
/// commitments or two of them will pick the same tile. See the `claimed` field
/// on [`PlanState`] for why a commitment is whole-tile, and
/// [`PlanState::mining_tile_separation`] for why a commitment also excludes
/// the tiles *around* it — a bot mining one tile stands on the ones next to
/// it, and a tile a bot is standing on cannot be mined by anybody else.
/// Is `tile` inside a charted enemy structure's standoff?
///
/// The one place the four resource selectors below ask, so they cannot drift
/// apart -- and they must not, because `resource_supply_at_least` and
/// `resource_tiles_for` have a test asserting they agree about what is
/// available, and `resource_seats` sizes the split that
/// `resource_tiles_for` then has to fill. A filter applied in three of the
/// four would promise a split more seats than the ground safely holds, and the
/// fourth would refuse it -- the same disagreement the claim ledger exists to
/// prevent, arriving through a new door.
///
/// **Passed over, never refused, and that asymmetry is deliberate.** Ore comes
/// in fields of thousands of tiles; on the map this was measured against 15.3%
/// of charted `iron-ore` sits inside a nest's standoff and every other
/// resource is at 0%, so skipping the threatened ones costs a longer walk and
/// never a plan. A rock is the opposite -- a handful of discrete entities, any
/// of which may be the only one -- which is why `Chop` carries a named
/// refusal and this does not.
///
/// See [`crate::state::PlanState::threat_covering`] for what a `false` here
/// does and does not establish: it is the absence of a *charted* reason to
/// avoid the tile, not a guarantee about ground nobody has looked at.
fn threatened_tile(state: &PlanState, tile: &Position) -> bool {
    state.threat_covering(tile).is_some()
}

/// The threats that can reach any tile of one set of patches, resolved once
/// per query instead of once per tile.
///
/// [`threatened_tile`] is the right question for one tile and the wrong one
/// for a patch walk: `threat_covering` rebuilds and sorts the whole threat
/// list on every call, and the walks in this module ask it for every tile of
/// every patch of an item, once per goal, once per expansion pass. Measured
/// on 2026-09-08 (`gathered:crude-oil`, seed 31337, explored dump, 32
/// threats): 17.4 million `threats_from` calls in one plan, about a fifth of
/// its 199 s. The answer each of those calls was reduced to is a boolean --
/// *does any threat reach this tile* -- which needs no order, so the sort was
/// paying for a tie-break nobody read.
///
/// The set kept is exactly the threats that could cover *some* tile of the
/// patches: a threat further from the patches' bounding-box centre than its
/// standoff plus the box's half-diagonal cannot be within its standoff of any
/// tile inside the box (triangle inequality). For every tile inside the box,
/// [`ThreatField::covers`] therefore agrees with [`threatened_tile`] exactly;
/// what changed is the work, not the verdict. With no threat charted -- every
/// t=0 dump -- the field is empty and a tile costs nothing.
///
/// Distances are Euclidean, as `threats_from` measures them and as
/// `2bf76bd7` settled.
struct ThreatField {
    reaching: Vec<(Position, f64)>,
}

impl ThreatField {
    fn over(state: &PlanState, patches: &[ResourcePatch]) -> Self {
        let mut tiles = patches.iter().flat_map(|patch| patch.elements.iter());
        let Some(first) = tiles.next() else {
            return ThreatField {
                reaching: Vec::new(),
            };
        };
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x, first.y, first.x, first.y);
        for tile in tiles {
            min_x = min_x.min(tile.x);
            min_y = min_y.min(tile.y);
            max_x = max_x.max(tile.x);
            max_y = max_y.max(tile.y);
        }
        let centre = Position::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
        let half_diagonal = calculate_distance(&centre, &Position::new(max_x, max_y));
        let reaching = state
            .base()
            .entity_graph
            .threats_from(&centre)
            .into_iter()
            .filter_map(|(name, at, distance)| {
                let standoff = state.threat_standoff(&name).tiles;
                (distance < standoff + half_diagonal).then_some((at, standoff))
            })
            .collect();
        ThreatField { reaching }
    }

    fn covers(&self, tile: &Position) -> bool {
        self.reaching
            .iter()
            .any(|(at, standoff)| calculate_distance(tile, at) < *standoff)
    }
}

pub fn nearest_resource_tile(
    state: &PlanState,
    item: &str,
    from: &Position,
    need: u32,
) -> Option<Position> {
    let mut best: Option<(f64, Position)> = None;
    let patches = state.resource_patches(item);
    let threats = ThreatField::over(state, &patches);
    for patch in patches {
        for tile in patch.elements {
            if state.resource_unclaimed(&tile, item) < need {
                continue;
            }
            // Passed over, not refused: ore fields are thousands of tiles and
            // a threatened one always has a safe neighbour on our maps. See
            // `threatened_tile` for why all four selectors ask this.
            if threats.covers(&tile) {
                continue;
            }
            let distance = calculate_distance(from, &tile);
            let better = match &best {
                None => true,
                Some((best_distance, best_tile)) => matches!(
                    distance
                        .total_cmp(best_distance)
                        .then(tile.x.total_cmp(&best_tile.x))
                        .then(tile.y.total_cmp(&best_tile.y)),
                    std::cmp::Ordering::Less
                ),
            };
            if better {
                best = Some((distance, tile));
            }
        }
    }
    best.map(|(_, tile)| tile)
}

/// Tiles of `item` to draw `need` from, nearest first, with how much to take
/// from each. Empty when the *uncommitted* tiles cannot supply `need` in
/// total.
///
/// Ties on distance break on `(x, y)`, like `nearest_resource_tile`, so the
/// result depends only on the tile set and the origin.
///
/// **Each tile appears at most once, here and across the whole plan.** Within
/// one call that was always true; across calls it was not, and four bots each
/// asked for ten iron ore were all sent to the one nearest tile, because
/// `DEFAULT_RESOURCE_PER_TILE` left it looking like it had hundreds to spare.
/// Emitting a mining action claims its tile (`Effect::ConsumeResource` ->
/// `PlanState::consume_resource`), and [`PlanState::resource_unclaimed`] —
/// which this reads — then reports it as empty, so the next caller walks on to
/// the next-nearest tile.
///
/// A tile's take is still capped at what the tile holds, so one action never
/// over-commits one tile either; with exclusivity, that is the only
/// over-commitment left to prevent.
///
/// **Tiles are also spaced.** Exclusivity puts two bots on two tiles; it does
/// not stop the second bot *standing on* the first one's tile, which is how
/// run `run-1788313837-06402` lost six of thirteen mines to `another
/// character is standing on the iron-ore`. Every tile this returns is at least
/// [`PlanState::mining_tile_separation`] from every other tile the plan has
/// committed to (through `resource_unclaimed`) and from every other tile this
/// call itself picks (the check in the loop below).
pub fn resource_tiles_for(
    state: &PlanState,
    item: &str,
    from: &Position,
    need: u32,
) -> Vec<(Position, u32)> {
    let mut candidates: Vec<(f64, Position, u32)> = Vec::new();
    let patches = state.resource_patches(item);
    let threats = ThreatField::over(state, &patches);
    for patch in patches {
        for tile in patch.elements {
            let available = state.resource_unclaimed(&tile, item);
            if available == 0 {
                continue;
            }
            if threats.covers(&tile) {
                continue;
            }
            candidates.push((calculate_distance(from, &tile), tile, available));
        }
    }
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.1.y.total_cmp(&b.1.y))
    });

    let separation = state.mining_tile_separation();
    let mut out: Vec<(Position, u32)> = Vec::new();
    let mut remaining = need;
    for (_, tile, available) in candidates {
        if remaining == 0 {
            break;
        }
        // The same spacing `PlanState::resource_unclaimed` applies against
        // tiles claimed *earlier in the plan*, applied here against the tiles
        // this call has already picked. They are not claimed yet — the claim
        // lands when `run_steps` applies `Effect::ConsumeResource`, after
        // `expand` has returned all of them — so without this a single call
        // could still hand out two adjacent tiles.
        //
        // And dropped on exactly the condition that drops it for a claim: one
        // `Mine::expand` emits one action per tile into the *one* chain it is
        // expanding under, so when that chain names a runner
        // (`PlanState::claim_runner`) the actions are serial and no bot is
        // ever standing on another's tile. Keeping the spacing here while
        // relaxing it for claims would leave the two disagreeing about the
        // same fact — one call's tiles spaced, the next call's not — which is
        // the disagreement `resource_unclaimed` exists to prevent.
        if state.claim_runner().is_none()
            && out
                .iter()
                .any(|(picked, _)| calculate_distance(picked, &tile) < separation)
        {
            continue;
        }
        let take = available.min(remaining);
        remaining -= take;
        out.push((tile, take));
    }
    if remaining > 0 {
        return Vec::new();
    }
    out
}

/// Can the map's remaining *uncommitted* tiles of `item` supply `need` in
/// total?
///
/// The same question `!resource_tiles_for(..).is_empty()` answers, without
/// building the answer: applicability asks only whether enough exists
/// anywhere, never which tiles are nearest, so there is nothing to collect,
/// nothing to sort and no origin to measure from. Stops at the first tile that
/// brings the running total up to `need`.
///
/// # Where the two stop being identical, and why that is safe
///
/// Both read the same per-tile ledger, so they agree exactly on which tiles
/// are *available* — claimed, crowded, exhausted. They can only differ when a
/// single call needs **more than one** tile, because `resource_tiles_for` also
/// spaces its own picks from each other and this cannot: whether `k` spaced
/// tiles fit in a patch depends on which tile the walk starts from, and there
/// is no origin here. So this is an upper bound in that one case, and when it
/// over-reports, `Mine::expand` finds no tile set and returns the *same*
/// `NoApplicableMethod` a false answer here would have produced one frame
/// earlier. Nothing plans a mine it cannot execute either way. With a
/// `DEFAULT_RESOURCE_PER_TILE` of 500 the multi-tile case needs a single
/// share above 500 ore to arise at all.
///
/// The two must keep agreeing — there is a test that says so — so this reads
/// the same claim-aware ledger `resource_tiles_for` does. That is also what
/// makes a patch the plan has used up refuse the plan instead of
/// over-committing it: `Mine::applicable` goes false, no other method can
/// satisfy a raw ore goal, and expansion fails with `NoApplicableMethod`
/// naming that goal. Fewer bots on a smaller patch is a plan; two bots on one
/// tile is not.
///
/// `need == 0` is trivially satisfiable and returns `true` — where
/// `resource_tiles_for` returns an empty vector for it, because there is no
/// tile to draw nothing from. Callers asking about a shortfall check it is
/// non-zero first.
pub fn resource_supply_at_least(state: &PlanState, item: &str, need: u32) -> bool {
    let mut total: u32 = 0;
    if need == 0 {
        return true;
    }
    let patches = state.resource_patches(item);
    let threats = ThreatField::over(state, &patches);
    for patch in patches {
        for tile in patch.elements {
            if threats.covers(&tile) {
                continue;
            }
            total = total.saturating_add(state.resource_unclaimed(&tile, item));
            if total >= need {
                return true;
            }
        }
    }
    false
}

/// How many bots can mine `item` **at the same time**, counting no further
/// than `cap`.
///
/// A seat is an uncommitted tile that is at least
/// [`PlanState::mining_tile_separation`] from every other seat counted, which
/// is the same rule [`resource_tiles_for`] applies when it hands tiles out.
/// So this answers "how many miners fit on this item's patches", and it is the
/// only question `SplitAcrossBots` asks about resources — through
/// [`Method::concurrency`](crate::method::Method::concurrency), never
/// directly, so the generic splitting method never learns what ore is.
///
/// # It is a packing, not the maximum packing
///
/// Greedy from a fixed order, so what it returns is a set of seats that
/// genuinely exists — never an over-count — but not necessarily the largest
/// such set, which is a maximum-independent-set problem nobody needs solved.
/// Erring low is the safe direction: a split sized from this plans fewer
/// chains than the patch could theoretically hold, and every chain it does
/// plan has somewhere to stand.
///
/// The order is every patch's tiles flattened into one `(x, y)` sort, *not*
/// each patch walked in turn: `PlanState::resource_patches` partitions one
/// contiguous field into two or three patches differently from call to call
/// (see its doc), and a per-patch walk would count a different number of seats
/// each time. Flattening first makes the count depend only on the tile set.
///
/// # Why the cap
///
/// A real ore field is thousands of tiles and hundreds of seats, and the scan
/// is quadratic in the seats it finds. Nothing ever needs a number larger than
/// the roster — no split can open more chains than there are bots — so the
/// caller states its ceiling and the walk stops there.
pub fn resource_seats(state: &PlanState, item: &str, cap: u32) -> u32 {
    if cap == 0 {
        return 0;
    }
    let patches = state.resource_patches(item);
    let threats = ThreatField::over(state, &patches);
    let mut tiles: Vec<Position> = patches
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    tiles.dedup_by(|a, b| a.x.total_cmp(&b.x).is_eq() && a.y.total_cmp(&b.y).is_eq());

    let separation = state.mining_tile_separation();
    let mut seats: Vec<Position> = Vec::new();
    for tile in tiles {
        // `_for(.., None)`, never the current runner's view. A seat is a spot
        // for a bot that is *not yet in the plan*, working at the same time as
        // everyone already in it, so every existing claim conflicts with it
        // whoever holds it -- which is exactly what an unknown runner means.
        // Asking with the enclosing chain's runner would count that chain's
        // own tiles as free seats and promise a split more participants than
        // the ground can hold at once. See `PlanState::is_resource_crowded_for`.
        if state.resource_unclaimed_for(&tile, item, None) == 0 {
            continue;
        }
        if threats.covers(&tile) {
            continue;
        }
        if seats
            .iter()
            .any(|seat| calculate_distance(seat, &tile) < separation)
        {
            continue;
        }
        seats.push(tile);
        if seats.len() as u32 >= cap {
            break;
        }
    }
    seats.len() as u32
}

/// The nearest spot to `from` where an `entity` actually fits, searched in
/// rings so the result is close and reproducible.
///
/// Takes the entity because "free" is not a property of a tile: a stone
/// furnace is 1.398 tiles across, so a tile with nothing on it is still no
/// place for one if the neighbouring tile carries a furnace whose box reaches
/// over. Searching by tile and testing by tile is what sited two furnaces one
/// tile apart and had the game refuse the second.
///
/// Candidates sit on the grid [`tile_alignment`] gives this entity — the
/// integer grid for an even-sized one like a stone furnace, the half-tile grid
/// for an odd-sized one like a lab — and the *test* is `is_area_free`, which is
/// exact.
pub fn free_area_near(state: &PlanState, from: &Position, entity: &str) -> Option<Position> {
    free_area_near_where(state, from, entity, |_| true)
}

/// [`free_area_near`], with a second test the site must also pass.
///
/// Split out rather than duplicated so the ring order — and therefore which
/// site any given search settles on — is written once. `accept` is called only
/// for sites that already fit, so it never has to re-ask that question.
///
/// # Ore is refused HERE, and only here
///
/// A candidate whose footprint covers ore ([`PlanState::covers_any_resource`])
/// is skipped. This is the whole of "do not bury the patch you are about to
/// mine", and this is where it belongs: a **siting policy**, applied by the
/// search that chooses ground, not by
/// [`PlanState::is_area_free`], which answers what the game allows. The game
/// allows all of it — nothing buildable carries the `resource` collision layer
/// — and stating the policy as a collision rule is what refused belt routes
/// across patches, cell sites beside them and `MinerLine` a site at any
/// radius, all as `NoRoute` / `NoSiteFound`. See
/// `docs/superpowers/notes/2026-09-06-ore-does-not-block.md`.
///
/// **It is a refusal rather than a ranked preference, and that was measured.**
/// A first attempt walked the rings twice — once off the ore, then anywhere —
/// so a search could fall back onto the patch when the ground beside it ran
/// out. `seventy_five_packs_are_crafted_on_several_bots_and_each_delivers_to_a_lab`
/// then failed with `NoApplicableMethod { goal: "have 40 iron-ore" }`: the
/// smelt's own furnaces had taken the patch, and `PlanState::resource_tile_blocked`
/// correctly stopped the miners from selecting tiles with a furnace on them.
/// Running out of *ground* is recoverable — `smelt_steps` queues into a
/// furnace that already stands (`crates/planner/tests/furnace_ground.rs`) —
/// and running out of *ore* is not.
///
/// Nothing else in the planner asks this. `produce::fit` sites a cell's
/// furnace at a fixed offset from its drill and may put it on ore, which is
/// deliberate: the rim of a patch is its thin edge.
///
/// # A threat is PREFERRED AWAY FROM, not refused — the opposite of ore
///
/// The rings are walked twice: once skipping every candidate inside a charted
/// enemy structure's standoff ([`PlanState::threat_covering`]), and then, only
/// if that found nothing at all, again without that skip. So a site with any
/// unthreatened alternative within [`FREE_TILE_SEARCH_RADIUS`] moves out of
/// the worm's reach, and a site with none answers exactly what it answered
/// before this existed.
///
/// **The two-pass shape is the one the ore paragraph above says was measured
/// and rejected, and it is right here for the reason it was wrong there.**
/// Falling back onto ore consumes the patch a later goal needs — unrecoverable.
/// Falling back into a worm's reach is *today's behaviour*: it loses nothing
/// that was not already lost, whereas refusing outright would delete a plan
/// that exists. This repo's own rule, from the target-side guard that landed
/// the day before: a refusal where a plan used to exist is a defect, not
/// caution.
///
/// **Why placement gets a guard at all, when the walk to it does not.**
/// Measured 2026-09-08 against a live `small-worm-turret`
/// (`scripts/threat_pass_probe.sh`): a character *standing* 24 tiles away lost
/// 153 of 250 health in 300 ticks, while one *walking past* at the same 24
/// tiles lost nothing — it was inside the 25-tile attack range for only ~47
/// ticks, less than the worm takes to rear up and fire. A building does not
/// walk on. It is the standing exposure that this guard is sized for, and it
/// is the one this function decides.
///
/// # Cost
///
/// One extra [`PlanState::threat_covering`] per candidate that has already
/// passed every other test — normally the first one, because the search
/// returns on it. The second pass runs only when the first found nothing,
/// which on a world with no charted threat is never. See the note in
/// `docs/superpowers/notes/2026-09-08-a-threat-is-not-only-at-the-target.md`
/// for the measured planning cost of that.
pub fn free_area_near_where(
    state: &PlanState,
    from: &Position,
    entity: &str,
    accept: impl Fn(&Position) -> bool,
) -> Option<Position> {
    // Pass 1 avoids charted threats; pass 2 is the pre-guard search verbatim.
    // Written as a loop over the flag rather than two ring walks so the ring
    // ORDER stays written once -- the property this function's own doc opens
    // by promising, and the one a second copy would silently break.
    for avoid_threats in [true, false] {
        if let Some(found) = free_area_ring_walk(state, from, entity, &accept, avoid_threats) {
            return Some(found);
        }
    }
    None
}

/// One walk of [`free_area_near_where`]'s rings. See its doc; `avoid_threats`
/// selects the first pass from the second.
fn free_area_ring_walk(
    state: &PlanState,
    from: &Position,
    entity: &str,
    accept: &impl Fn(&Position) -> bool,
    avoid_threats: bool,
) -> Option<Position> {
    let (offset_x, offset_y) = tile_alignment(state, entity);
    let base_x = from.x.floor() as i32;
    let base_y = from.y.floor() as i32;
    for radius in 0..=FREE_TILE_SEARCH_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // Only the ring at exactly this radius; inner ones were done.
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let candidate = Position::new(
                    (base_x + dx) as f64 + offset_x,
                    (base_y + dy) as f64 + offset_y,
                );
                if !state.is_area_free(entity, &candidate) || !accept(&candidate) {
                    continue;
                }
                if state
                    .collision_area(entity, &candidate)
                    .is_some_and(|area| state.covers_any_resource(&area))
                {
                    continue;
                }
                // Last, because it is the only test here that is linear in
                // the threat table: everything cheaper has already had its
                // chance to reject this candidate.
                if avoid_threats && threatened_tile(state, &candidate) {
                    continue;
                }
                return Some(candidate);
            }
        }
    }
    None
}

/// An axis-aligned box, rotated about its own origin into `direction`.
///
/// A collision box is written for a north-facing entity; a boiler is 3x2 tiles
/// facing north and **2x3 facing east**, and a placement checked against the
/// unrotated box is checking the wrong ground. Every `Place` this planner has
/// ever emitted carried direction 0, so nothing needed this until the power
/// plant, whose boiler and steam engine are turned by whichever way the
/// shoreline faces.
///
/// Rotating the two stated corners is enough: a 90-degree rotation maps a
/// rectangle onto a rectangle and takes opposite corners to opposite corners,
/// so the min/max of those two images spans the image of all four.
///
/// `None` for the eight half-diagonals — [`Position::turn`] names no rotation
/// for them, and a building never stands on one.
pub fn rotated_collision_box(box_: &Rect, direction: Direction) -> Option<Rect> {
    let a = box_.left_top.turn(direction)?;
    let b = box_.right_bottom.turn(direction)?;
    Some(Rect::new(
        &Position::new(a.x().min(b.x()), a.y().min(b.y())),
        &Position::new(a.x().max(b.x()), a.y().max(b.y())),
    ))
}

/// Entities whose build grid their collision box does not predict.
///
/// Factorio snaps a building by `tile_width`/`tile_height`, which *default* to
/// the collision box's extents rounded up — that default is exactly what
/// [`tile_alignment`] computes. A prototype may state them outright, and then
/// the collision box says nothing about the grid. **The mod sends neither
/// field**: `FactorioEntityPrototype` carries `collision_box`, mining and
/// crafting numbers and nothing about grids. So the exceptions are written
/// down here where they can be checked, the same discipline as
/// `crate::state`'s pole tables and [`COAL_BURN_TICKS`].
///
/// [`COAL_BURN_TICKS`]: crate::method::have::COAL_BURN_TICKS
///
/// There is exactly one in vanilla 2.1 among the entities this planner places.
/// `offshore-pump` declares `tile_width = 1, tile_height = 1`
/// (`base/prototypes/entity/entities.lua`, checked in this repo's
/// `workspace/data`) against a collision box of 1.195 x 1.344 tiles, which
/// would otherwise round up to 2 x 2. The difference is not cosmetic: an even
/// extent puts the entity's centre on a tile **corner**, and the pump's own
/// `tile_buildability_rules` — one ground tile beneath it, water ahead of it —
/// cannot be met from a corner by any shoreline that exists.
fn explicit_tile_extent(entity: &str) -> Option<(i64, i64)> {
    match entity {
        "offshore-pump" => Some((1, 1)),
        _ => None,
    }
}

/// Where on the tile grid `entity`'s centre belongs when it faces `direction`.
///
/// [`tile_alignment`] is this facing north, and every caller that places an
/// unrotated entity should keep using that.
pub fn tile_alignment_facing(state: &PlanState, entity: &str, direction: Direction) -> (f64, f64) {
    // `ceil` and not `round`: an entity 2.3984 tiles across occupies three
    // tiles, not two. The tiny epsilon keeps a box that is exactly `n` tiles
    // wide -- which the binary-fraction prototype numbers really can be --
    // from ceiling to `n + 1` on float noise.
    let tiles = |extent: f64| (extent - 1. / 512.).ceil() as i64;
    let parity = |tiles: i64| if tiles.rem_euclid(2) == 0 { 0. } else { 0.5 };
    let swapped = matches!(direction, Direction::East | Direction::West);

    if let Some((w, h)) = explicit_tile_extent(entity) {
        let (w, h) = if swapped { (h, w) } else { (w, h) };
        return (parity(w), parity(h));
    }
    let Some(prototype) = state.base().globals.entity_prototypes.get(entity) else {
        return (0., 0.);
    };
    let box_ = &prototype.collision_box;
    let (w, h) = (tiles(box_.width()), tiles(box_.height()));
    let (w, h) = if swapped { (h, w) } else { (w, h) };
    (parity(w), parity(h))
}

/// Where on the tile grid `entity`'s centre belongs, as an offset to add to an
/// integer tile coordinate: `0.0` or `0.5` on each axis.
///
/// Factorio aligns a building to the tile grid by its *footprint*, not by its
/// centre. An entity that covers an **even** number of tiles on an axis has its
/// centre on a tile boundary — a stone furnace is 1.3984 tiles across, covers
/// two, and sits at an integer. One that covers an **odd** number has its centre
/// at a tile *centre* — a lab is 2.3984 across, covers three, and sits at
/// `n + 0.5`. This is the same half-tile the resource positions carry, and
/// getting it wrong has cost this project a day once already.
///
/// Read from the prototype's own `collision_box`, so nothing here has to know
/// that a lab is 3x3. An entity the world has no prototype for keeps the
/// integer grid, which is what every caller did before this existed; its
/// placement is refused by `Condition::AreaFree` on the same missing prototype
/// anyway.
pub fn tile_alignment(state: &PlanState, entity: &str) -> (f64, f64) {
    tile_alignment_facing(state, entity, Direction::North)
}

/// Ingredients of `item`, or an empty vector when the recipe has none.
pub fn ingredients_of(recipe: &FactorioRecipe) -> Vec<(String, u32)> {
    recipe
        .ingredients
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|i| (i.name.clone(), i.amount))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// The recipe category the hand-craft method admits (`have.rs`, `Craft`).
///
/// Named rather than inlined so `tests/recipe_probability.rs` can assert over
/// the *actual* gate: a copy of the string in the test would keep passing when
/// the gate widened, which is the one moment the assertion exists for.
pub const CRAFTING_CATEGORY: &str = "crafting";

/// The recipe category the smelt method admits (`have.rs`, `Smelt`).
/// Named for the same reason as [`CRAFTING_CATEGORY`].
pub const SMELTING_CATEGORY: &str = "smelting";

/// How many of `item` one execution of `recipe` yields. Defaults to 1.
///
/// Deliberately does **not** divide by `FactorioProduct::probability`, and the
/// division would belong at the caller's `runs` rather than here in any case —
/// see the field's own note in `crates/core/src/types.rs` and
/// `tests/recipe_probability.rs` for why no reachable recipe needs it.
pub fn output_per_craft(recipe: &FactorioRecipe, item: &str) -> u32 {
    recipe
        .products
        .iter()
        .find(|p| p.name == item)
        .map(|p| p.amount.max(1))
        .unwrap_or(1)
}

/// A recipe's `energy` in seconds — the time one run takes in a machine of
/// crafting speed 1, which is what the field means.
fn recipe_seconds(recipe: &FactorioRecipe) -> f64 {
    recipe.energy.to_f64().unwrap_or(0.5)
}

/// A recipe's energy in ticks: how long one run takes at **crafting speed 1**.
///
/// This is the right answer for the hand-craft path, because a vanilla
/// character's crafting speed really is 1 — confirmed live against 2.1.17,
/// where `prototypes.entity["character"].get_crafting_speed()` returns 1.
///
/// It is deliberately *not* the right answer for a machine, which divides by
/// its own speed; use [`smelting_ticks`] there. It also stays the basis of the
/// coal bill in `Smelt::expand`, which is an energy quantity rather than a
/// duration — see [`crate::method::have::COAL_BURN_TICKS`].
pub fn recipe_ticks(recipe: &FactorioRecipe) -> Ticks {
    seconds_to_ticks(recipe_seconds(recipe))
}

/// A vanilla stone furnace's crafting speed, used only when the world reports
/// none for the acting machine. See [`machine_crafting_speed`].
const VANILLA_STONE_FURNACE_CRAFTING_SPEED: f64 = 1.0;

/// How fast `machine` runs a recipe, as the world reports it.
///
/// A crafting machine divides a recipe's time by its own crafting speed, and
/// the furnaces disagree: live 2.1.17 reports 1 for `stone-furnace` and **2**
/// for both `steel-furnace` and `electric-furnace` (and 0.5 for
/// `assembling-machine-1`). So this cannot be a constant, and — following
/// [`character_mining_speed`] — it is read from the world, with the constant
/// kept only as the fallback for a world that reports nothing.
///
/// # Why the fallback is a *stone* furnace and why that matters here
///
/// `Smelt::expand` places a `stone-furnace` and nothing else: the entity name
/// is a literal, the place action requires the bot to be *holding* one, and no
/// method in this crate ever adopts a furnace already standing in the world.
/// So today the divisor is 1 whichever branch is taken, and this division
/// changes no number a current plan produces.
///
/// It is written anyway because the alternative is a delayed fuse. The moment
/// anyone teaches `Smelt` to pick a better furnace, or to use one the save
/// already has — which a real save will have, and which may well be steel or
/// electric — every smelting estimate silently becomes 2x too slow, with no
/// test failing at the commit that breaks it. The divisor being present and
/// read means that change is correct for free.
///
/// # What this still does not model
///
/// Modules and beacons multiply a machine's effective speed, and this reads
/// only the prototype. That is not reachable today: this crate mentions
/// neither, a vanilla `stone-furnace` has no module slots at all, and nothing
/// places a beacon. It becomes reachable together with furnace adoption — an
/// existing furnace in a real save can carry modules and sit in a beacon's
/// range — so whoever adds adoption owns this too.
pub fn machine_crafting_speed(state: &PlanState, machine: &str) -> f64 {
    state
        .base()
        .globals
        .entity_prototypes
        .get(machine)
        .and_then(|p| p.crafting_speed)
        // A zero or negative speed is not a machine that crafts slowly, it is
        // a divide by zero. Fall back rather than emit an infinite duration.
        .filter(|speed| *speed > 0.)
        .unwrap_or(VANILLA_STONE_FURNACE_CRAFTING_SPEED)
}

/// Ticks for one run of `recipe` in `machine`.
///
/// A machine takes `recipe.energy / crafting_speed` seconds per run — the
/// recipe's `energy` is the numerator of a division, not the answer, exactly
/// as `mining_time` is in [`mining_ticks`]. The divisor was missing, which is
/// right for a stone furnace (speed 1) and 2x too slow for a steel or electric
/// one (speed 2).
///
/// Per run, then multiplied by the run count at the call site, so that the
/// rounding matches what the game does: a furnace rounds each craft, it does
/// not run one long fractional batch.
pub fn smelting_ticks(state: &PlanState, recipe: &FactorioRecipe, machine: &str) -> Ticks {
    seconds_to_ticks(recipe_seconds(recipe) / machine_crafting_speed(state, machine))
}

/// Whether a recipe may be used, and at what cost.
///
/// The world sends *every* recipe with an `enabled` flag, not just the ones the
/// force can currently craft, because a plan is a statement about the future:
/// `goal.researched("automation")` needs 10 automation science packs, and that
/// recipe is disabled until its own technology is researched. Hiding the recipe
/// made the goal unplannable; showing it without this gate would make it
/// *wrongly* plannable, emitting a craft the game would refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeGate {
    /// Craftable as things stand: either the recipe is enabled outright, or
    /// the force finished the technology that unlocks it before this plan
    /// began. Nothing in the plan has to run first, so nothing orders against
    /// it.
    Open,
    /// Craftable only after this technology is researched.
    NeedsResearch(String),
    /// Disabled in the world, but a *sibling of this expansion* has already
    /// undertaken the research that unlocks it.
    ///
    /// The distinction from [`RecipeGate::Open`] is the whole reason this
    /// variant exists, and it is worth stating plainly: the recipe is not open,
    /// it is *going to be* open, once an action this plan already contains has
    /// run. A caller must therefore emit no second research subgoal — the work
    /// is already in the network — but must still state
    /// `Condition::Researched`, because that condition is what
    /// `ActionNetwork::infer_edges` turns into the edge keeping the craft after
    /// the unlock.
    ///
    /// Reporting this as `Open` is what `run-1788338409-63794` died of: a
    /// `Have(automation-science-pack, 10)` split four ways, the first share
    /// expanded the unlock, and the other three read the recipe as open and
    /// came out with empty `deps` and `planned_start: 0`. The game answered
    /// `could not have player client2 craft 3 automation-science-pack (but only
    /// 0)` while client2 was holding six copper plates and five gear wheels.
    PlannedResearch(String),
    /// Disabled and no technology unlocks it, so nothing this planner can do
    /// will ever turn it on. Live 2.1.17 has eight of these — `loader`,
    /// `pistol`, `infinity-chest` and friends, which are editor or map-editor
    /// items. A method must decline rather than plan a craft that cannot run.
    Unobtainable,
}

/// The technology that unlocks `recipe`, if any.
///
/// Scans the acting force's technologies, which `PlanState` holds in a
/// `BTreeMap`, and takes the **lexicographically smallest** name among those
/// that unlock the recipe. A handful of recipes really do have several
/// unlockers (live 2.1.17 has seven, e.g. `roboport` from either
/// `construction-robotics` or `logistic-robotics`), and they are alternatives:
/// researching any one of them turns the recipe on. Picking one is therefore
/// sound, and picking the smallest name makes the choice depend only on the
/// data and not on iteration order — which is what keeps planning
/// deterministic. It is *not* claimed to be the cheapest of the alternatives;
/// costing them and choosing the cheapest is follow-up work.
///
/// One already-researched unlocker wins over any unresearched one regardless of
/// name, because a technology already done costs nothing and asking for a
/// different one would add work the world has already paid for.
pub fn unlocking_technology(state: &PlanState, recipe: &str) -> Option<String> {
    let mut candidate: Option<String> = None;
    for name in state.technology_names() {
        let Some(tech) = state.technology(&name) else {
            continue;
        };
        if !tech.unlocked_recipes.iter().any(|r| r == recipe) {
            continue;
        }
        if state.is_researched(&name) {
            return Some(name);
        }
        if candidate.is_none() {
            candidate = Some(name);
        }
    }
    candidate
}

/// Classify `recipe` for the acting force under the plan's overlay.
pub fn recipe_gate(state: &PlanState, recipe: &FactorioRecipe) -> RecipeGate {
    if recipe.enabled {
        return RecipeGate::Open;
    }
    match unlocking_technology(state, &recipe.name) {
        // Three states, not two. The world's own flag is the only one that
        // means "nothing has to happen first"; the overlay means "something in
        // this plan has to happen first, and it is already written down". Both
        // cost no second research subgoal — which is what keeps the common
        // case free, since `Researched` emits its prerequisites before its
        // science packs and the technology unlocking a pack's recipe is
        // normally one of those prerequisites — but only the first of them
        // needs no ordering edge. See `RecipeGate::PlannedResearch`.
        Some(tech) if state.is_world_researched(&tech) => RecipeGate::Open,
        Some(tech) if state.is_researched(&tech) => RecipeGate::PlannedResearch(tech),
        Some(tech) => RecipeGate::NeedsResearch(tech),
        None => RecipeGate::Unobtainable,
    }
}

/// What a whole research costs, as (item, total count) pairs in the order the
/// technology lists them.
///
/// `research_unit_ingredients` is the cost of **one** unit and
/// `research_unit_count` is how many units the technology takes, so the bill is
/// the product. The multiply is done in `u64` and clamped, because `amount` is
/// a `u32` and a modded technology with a large unit count could otherwise wrap
/// a plan's cost down to something cheap.
pub fn research_ingredients(tech: &FactorioTechnology) -> Vec<(String, u32)> {
    tech.research_unit_ingredients
        .iter()
        .map(|ingredient| {
            let total = u64::from(ingredient.amount).saturating_mul(tech.research_unit_count);
            (
                ingredient.name.clone(),
                u32::try_from(total).unwrap_or(u32::MAX),
            )
        })
        .collect()
}

/// What a `research_trigger` technology asks of the plan, as a goal shape.
///
/// Three of the eight trigger kinds are planned, and they are planned
/// differently: a `craft-item` is satisfied by *producing* the item, which
/// any of the producing methods can do and hang the unlock on; a
/// `mine-entity` is satisfied by mining a *named entity*, which is a hand's
/// work when the character can dig it and a machine's when it cannot --
/// `oil-processing` names `crude-oil`, and a character cannot mine a well.
/// a `create-space-platform` is satisfied by a rocket silo, a starter pack
/// and one force-level call, which is [`crate::method::orbit`]'s whole
/// subject.
/// The choice between those is [`crate::method::have::Researched`]'s, made
/// against the world; this only says what the trigger wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerRequirement {
    /// `count` of `item` must be produced.
    Craft { item: String, count: u32 },
    /// `count` of any one of `entities` must be mined, by whatever can mine
    /// it. Never empty: an empty list refuses in [`trigger_requirement`].
    Mine { entities: Vec<String>, count: u32 },
    /// A space platform must be created, which needs a rocket silo, a starter
    /// pack and one force-level call. See [`crate::method::orbit`].
    ///
    /// The third planned kind, added 2026-09-09. Unlike the other two it
    /// carries nothing from the trigger: `ResearchTrigger::CreateSpacePlatform`
    /// is a bare variant with no fields at all, so the planet and the pack are
    /// this planner's choice rather than the game's statement, and they are
    /// named where the goal is built rather than invented here.
    CreatePlatform,
}

/// What a `research_trigger` technology actually costs, as the work its
/// trigger requires.
///
/// `Ok(None)` is the ordinary pack-researched technology, whose bill
/// `research_ingredients` already describes. `Ok(Some(_))` is a trigger this
/// planner can express -- see [`TriggerRequirement`].
///
/// Everything else is an error, deliberately. A trigger technology has an empty
/// pack bill and zero research time, so the alternative to refusing is planning
/// it as free — which is precisely the defect this function exists to fix, and
/// which is invisible in the resulting plan. Every error variant names the
/// technology, so a caller learns which step is not modelled rather than
/// receiving a makespan that is quietly too small. Three are told apart:
///
/// * [`PlannerError::UnsupportedResearchTrigger`] -- a kind with no goal
///   (`craft-fluid`, `build-entity`, orbit, spawner, scripted, or one this
///   build has never heard of), named with the act it wants. **`space-platform`
///   left this list on 2026-09-09**, when `ActionKind::CreatePlatform` gave the
///   planner an action that performs its act; the two that remain
///   (`space-science-pack`, `biter-egg-handling`) still want acts nothing here
///   does -- the mod's `build-entity` emulation is honest only because a bot
///   has actually placed the entity, and no bot can place an asteroid
///   collector;
/// * [`PlannerError::UndescribedResearchTrigger`] -- a `mine-entity` with no
///   entity named, which is what every dump written before 2026-09-05 holds,
///   because the mod sent the bare type. A new dump fixes it; nothing in the
///   planner can;
/// * [`PlannerError::SelfUnlockingResearchTrigger`] -- below.
///
/// # The self-unlocking case
///
/// A `craft-item` trigger may ask for an item whose recipe *only this same
/// technology* unlocks. That is not hypothetical: shipped 2.1.17 has six,
/// starting with `foundry`, which is triggered by crafting a foundry and is the
/// only technology unlocking the foundry recipe. Emitting the subgoal anyway
/// would send the expansion round `Researched(t)` -> `Have(item)` ->
/// `NeedsResearch(t)` -> `Researched(t)` until the driver's depth guard fired,
/// reporting `ExpansionTooDeep` — a true statement that names neither the
/// technology nor the reason. The cycle is understood here, so it is diagnosed
/// here.
///
/// The check reuses `recipe_gate`, so it asks the question the crafting methods
/// would actually ask, and it reads the overlay: a technology an earlier step
/// already researched leaves the recipe open and the guard does not fire.
pub fn trigger_requirement(
    state: &PlanState,
    tech: &FactorioTechnology,
) -> Result<Option<TriggerRequirement>, PlannerError> {
    let Some(trigger) = &tech.research_trigger else {
        return Ok(None);
    };
    match trigger {
        ResearchTrigger::CraftItem { item, count } => {
            if let Some(recipe) = recipe_for(state, item)
                && let RecipeGate::NeedsResearch(unlocker) = recipe_gate(state, &recipe)
                && unlocker == tech.name
            {
                return Err(PlannerError::SelfUnlockingResearchTrigger {
                    technology: tech.name.clone(),
                    item: item.clone(),
                });
            }
            Ok(Some(TriggerRequirement::Craft {
                item: item.clone(),
                count: *count,
            }))
        }
        ResearchTrigger::MineEntity { entities, count } => {
            if entities.is_empty() {
                return Err(PlannerError::UndescribedResearchTrigger {
                    technology: tech.name.clone(),
                    trigger: trigger.kind().to_string(),
                });
            }
            Ok(Some(TriggerRequirement::Mine {
                entities: entities.clone(),
                count: (*count).max(1),
            }))
        }
        // Planned since 2026-09-09, and the trigger the game states most
        // sparely: it names no entity, no item and no count, because there is
        // exactly one act that satisfies it.
        ResearchTrigger::CreateSpacePlatform => Ok(Some(TriggerRequirement::CreatePlatform)),
        other => Err(PlannerError::UnsupportedResearchTrigger {
            technology: tech.name.clone(),
            trigger: other.kind().to_string(),
            act: other.to_string(),
        }),
    }
}

/// How long a whole research takes, in ticks.
///
/// **`research_unit_energy` is in ticks, not seconds** — unlike
/// `FactorioRecipe::energy`, which `recipe_ticks` above converts from seconds.
/// The asymmetry is Factorio's, not ours: the runtime API multiplies a
/// technology prototype's `unit.time` by 60 before handing it out, so
/// automation's `time = 10` arrives here as `600`. `mods/BotBridge/types.lua`
/// copies the field through untouched, so what lands in `FactorioTechnology` is
/// whatever the runtime API said. If a future Factorio changes that, this is
/// the one line to change, and the error is a factor of sixty in a *time
/// estimate* — it moves makespans, it does not make a plan wrong.
pub fn research_ticks(tech: &FactorioTechnology) -> Ticks {
    research_ticks_in_labs(tech, 1)
}

/// How long a whole research takes when `labs` labs share it, in ticks.
///
/// Labs research **units**, one at a time each, and the game hands every
/// lab holding packs its own unit, so `labs` labs finish `count` units in
/// `ceil(count / labs)` rounds of `unit_time`. That is the model, and it is
/// exact for labs of equal speed that all hold packs — which is what
/// `Researched` builds, since it feeds every lab it places. A lab that is
/// standing empty contributes nothing, and it is the inserts, not this
/// arithmetic, that decide whether a lab is empty.
///
/// `labs == 0` is treated as one: a research in no lab is not a faster
/// research, and a caller that has counted no labs has counted wrong.
///
/// **`research_unit_energy` is in ticks, not seconds** — see
/// [`research_ticks`], whose single-lab figure this generalises and to
/// which it is identical at `labs == 1`.
pub fn research_ticks_in_labs(tech: &FactorioTechnology, labs: u32) -> Ticks {
    let labs = f64::from(labs.max(1));
    let units = tech.research_unit_count.to_f64().unwrap_or(0.0);
    let ticks = tech.research_unit_energy.to_f64().unwrap_or(0.0) * (units / labs).ceil();
    if ticks <= 0.0 {
        return 0;
    }
    if ticks >= f64::from(Ticks::MAX) {
        return Ticks::MAX;
    }
    ticks.ceil() as Ticks
}

/// The vanilla beacon's prototype name.
///
/// A name, not a rate: naming a prototype is how every method in this crate
/// asks the world about one (`assemble::MACHINE`, `assemble::CHEST`), and a
/// world that does not carry it answers `None` everywhere below rather than
/// being given a made-up beacon.
pub const BEACON: &str = "beacon";

/// How much ground a beacon needs, and how far it reaches, **derived from the
/// beacon's own prototype rather than from a remembered 9x9.**
///
/// # Why this type exists before anything places a beacon
///
/// Modules and beacons multiply a machine's effective speed (see
/// [`machine_crafting_speed`]), and retrofitting a beacon lane into a base
/// that is already built and belted means tearing the base down. Reserving
/// the ground costs nothing but ground *now*, so the reservation is worth
/// making before the beacon is, which is the whole reason this is here with
/// no beacon in any plan yet.
///
/// # The geometry, and why the two halves are not equally available
///
/// A beacon supplies its effect to every machine whose bounding box meets its
/// **supply area**, which is the beacon's own footprint grown by
/// `supply_area_distance` tiles on all four sides. That is *not* the electric
/// pole convention and the difference is a trap: the prototype docs say a
/// pole's `supply_area_distance` "corresponds to **half** of the supply area"
/// (2.5 gives 5x5, centred on a 1x1 pole), while a beacon's is "the maximum
/// distance that this beacon can supply its neighbors" — 3 on a 3x3 footprint
/// gives 9x9, which is footprint + 2d and not 2d. Reading one convention off
/// the other is how a beacon lane ends up three tiles too narrow.
///
/// Write `b` for the footprint and `d` for `supply_area_distance`, both in
/// tiles, and put a beacon in a lane between two machine rows:
///
/// ```text
///        row A          lane           row B
///   ...####|<-- gA -->|#####|<-- gB -->|####...
///                       b
/// ```
///
/// The supply area reaches `d` tiles past each edge of `b`, so **row A is
/// reached iff `gA < d`**, and a beacon is worth its ground only when it
/// reaches *both* rows, i.e. `gA < d` **and** `gB < d`. Centring the beacon
/// makes the two gaps equal, so the two rows may sit at most
/// `S = b + 2d` apart, edge to edge, and no less than `b` apart or the beacon
/// does not fit. That is [`Self::lane_tiles`] and
/// [`Self::max_row_separation_tiles`].
///
/// # Both `b` and `d` come from the world now, and the cheap answer still
/// needs only `b`
///
/// `b` comes from `collision_box`. `d` comes from
/// `FactorioEntityPrototype::supply_area_distance`, which landed on
/// 2026-09-06; [`beacon_supply_area_distance`] reads it.
///
/// **An earlier version of this doc said `d` "does not exist in our data at
/// all", and that it is `None` "for every beacon in every world this planner
/// has ever seen".** It was true when written and stopped being true the same
/// night. What survives of it: a world whose sender predates the field —
/// every dump archived before that date — still answers `None`, and `None`
/// here means *unknown*, never a zero supply area and never a guess.
///
/// The saving grace was always that **the reservation decision does not need
/// `d`.** Set `gA = gB = 0` — machines flush against the beacon's footprint
/// — and the reach condition becomes `0 < d`, which holds for every beacon
/// that supplies anything at all. So the *narrowest* lane that can ever work
/// is exactly `b` wide, and it is also the cheapest ground to reserve. `d`
/// bounds only how much *wider* a lane may usefully be.
///
/// # What is still missing: the WORTH of a beacon, not its reach
///
/// The *benefit* needs `distribution_effectivity` (the multiplier applied to a
/// module's effect when shared) and `beacon_profile`. Both now ride on
/// `FactorioEntityPrototype` too, and this type still models neither, for a
/// reason that is not laziness: **`beacon_profile` is an ARRAY, indexed by how
/// many beacons reach one receiver**, so a receiver's share is
/// `distribution_effectivity * beacon_profile[n]` and there is no scalar
/// answer. Vanilla's array is 100 entries beginning `1, 0.7071, 0.5773, 0.5`
/// — the second beacon on a machine is worth 71% of what the first was, and
/// anything treating `distribution_effectivity` as the whole answer is right
/// for exactly `n = 1` and silently wrong everywhere else. Pricing a beacon
/// needs a caller that knows `n`, which is a siting decision nothing here
/// makes yet. This type answers where a beacon may stand, not what it is
/// worth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeaconGeometry {
    footprint_tiles: f64,
    supply_area_distance: Option<f64>,
}

impl BeaconGeometry {
    /// A beacon of `footprint_tiles` across reaching `supply_area_distance`
    /// tiles beyond itself.
    ///
    /// Public so that the arithmetic above can be exercised with a `d` fed to
    /// it directly. That was the *only* way to reach it until 2026-09-06,
    /// when the prototype field landed; it stays public because a fixture
    /// world is still the cheapest way to pin the `b + 2d` convention against
    /// the pole's half-the-side one.
    #[must_use]
    pub fn new(footprint_tiles: f64, supply_area_distance: Option<f64>) -> Self {
        Self {
            footprint_tiles,
            supply_area_distance,
        }
    }

    /// The beacon's footprint in whole tiles, from its `collision_box`.
    ///
    /// Factorio sizes an n-tile box at slightly under n (a 3x3 beacon's box is
    /// `+/-1.19921875`, i.e. 2.3984375 across) so that two of them may abut,
    /// which is why this rounds **up** rather than to nearest: 2.3984375 is a
    /// three-tile building, not a two-tile one.
    #[must_use]
    pub fn footprint_tiles(&self) -> f64 {
        self.footprint_tiles
    }

    /// `supply_area_distance`, in tiles beyond the footprint — `None` for a
    /// world whose sender predates the field. See the type doc.
    #[must_use]
    pub fn supply_area_distance(&self) -> Option<f64> {
        self.supply_area_distance
    }

    /// The narrowest lane a beacon can stand in and still reach a machine row
    /// flush against each side: its own footprint, and **no function of
    /// `supply_area_distance` at all**.
    #[must_use]
    pub fn lane_tiles(&self) -> f64 {
        self.footprint_tiles
    }

    /// The widest two machine rows may be apart, edge to edge, for one beacon
    /// centred between them to reach both: `b + 2d`.
    ///
    /// `None` while `d` is absent — a caller that needs this number cannot
    /// have it yet, and is told so rather than handed `b` and a shrug.
    #[must_use]
    pub fn max_row_separation_tiles(&self) -> Option<f64> {
        self.supply_area_distance
            .map(|d| self.footprint_tiles + 2.0 * d)
    }

    /// Whether a machine row whose near face is `gap` tiles from the beacon's
    /// footprint is inside the supply area: `gap < d`.
    ///
    /// `None` while `d` is absent.
    #[must_use]
    pub fn reaches_gap(&self, gap: f64) -> Option<bool> {
        self.supply_area_distance.map(|d| gap < d)
    }
}

/// [`BeaconGeometry`] for `beacon`, or `None` for a world that does not carry
/// that prototype.
///
/// A refusal rather than a default, following [`machine_crafting_speed`]'s
/// neighbours in `crate::state`: an unknown name contributes nothing, which
/// under-credits instead of over-crediting.
#[must_use]
pub fn beacon_geometry(state: &PlanState, beacon: &str) -> Option<BeaconGeometry> {
    let prototype = state.base().globals.entity_prototypes.get(beacon)?;
    let box_ = &prototype.collision_box;
    let footprint_tiles = box_.width().max(box_.height()).ceil();
    if footprint_tiles <= 0.0 {
        return None;
    }
    Some(BeaconGeometry {
        footprint_tiles,
        supply_area_distance: beacon_supply_area_distance(state, beacon),
    })
}

/// A beacon's `supply_area_distance`, read off its own prototype.
///
/// The one seam between [`BeaconGeometry`] and the world, and it now answers:
/// the mod sends `get_supply_area_distance()` (a method in 2.0, not an
/// attribute) as `FactorioEntityPrototype::supply_area_distance`.
///
/// Three refusals, each of which would otherwise be a silent wrong number:
///
/// * **a prototype the world does not carry** — `None`, following
///   [`machine_crafting_speed`]'s neighbours in `crate::state`;
/// * **a prototype that is not a beacon** — `None`, because the same field on
///   an `electric-pole` means *half the side of a supply square* and not
///   *distance beyond the footprint*. Believing a pole's `2.5` here would
///   report a small pole as reaching 2.5 tiles past a footprint it does not
///   have. `crate::state`'s `pole_supply_half_extent` gates on the same
///   discriminator from the other side, and the two deliberately share no
///   helper: one number, two conventions, and a shared helper would quietly
///   mean whichever the caller assumed.
/// * **a beacon whose prototype does not declare it** — `None`, i.e.
///   *unknown*. This is every world dumped before 2026-09-06. There is
///   deliberately **no vanilla fallback table here**, unlike the pole side:
///   the poles needed one because deleting theirs would blind the planner's
///   power model on the archived maps, whereas nothing consumes a beacon's
///   `d` yet, so `None` costs nothing and a guess would cost the honesty.
///
/// Still absent from this answer, on purpose: `distribution_effectivity` and
/// `beacon_profile` also ride on the prototype now, and pricing a beacon needs
/// both plus the beacon *count* — see the note under [`BeaconGeometry`]. Module
/// effects come from the *item* side, `LuaItemPrototype::module_effects`, which
/// `FactorioItemPrototype` does not carry at all.
#[must_use]
pub fn beacon_supply_area_distance(state: &PlanState, beacon: &str) -> Option<f64> {
    let prototype = state.base().globals.entity_prototypes.get(beacon)?;
    if prototype.entity_type != "beacon" {
        return None;
    }
    prototype.supply_area_distance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::{ClaimRunner, DEFAULT_RESOURCE_PER_TILE, PlanState};
    use factorio_bot_core::factorio::util::add_to_rect;
    use factorio_bot_core::factorio::world::FactorioSurface;
    use factorio_bot_core::serde_json;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{Direction, FactorioEntity, FactorioForce, Position, Rect};
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    /// `fixture_world()` with the `character` prototype's mining speed set to
    /// `speed`, or the prototype removed entirely when `speed` is `None`.
    ///
    /// The fixture ships a real `character` at 0.5, so overwriting it in place
    /// is what lets a test say "the divisor came from the world" rather than
    /// "the divisor happens to equal the constant".
    fn state_with_character_mining_speed(speed: Option<f64>) -> PlanState {
        let world = fixture_world();
        match speed {
            Some(speed) => {
                let mut character = world
                    .globals
                    .entity_prototypes
                    .get("character")
                    .expect("the fixture ships a character prototype")
                    .clone();
                character.mining_speed = Some(speed);
                world
                    .globals
                    .entity_prototypes
                    .insert("character".into(), character);
            }
            None => {
                world.globals.entity_prototypes.remove("character");
            }
        }
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// `fixture_world()` plus one `player` force carrying `modifier` verbatim
    /// as JSON.
    ///
    /// `fixture_world()` deliberately carries no forces (see `test_world.rs`),
    /// so anything force-scoped has to bolt one on. Built by deserialising
    /// rather than by struct literal so that these tests also pin the wire
    /// shape `mods/BotBridge`'s `serialize_force` sends — including that a
    /// payload with no `manual_mining_speed_modifier` key at all still parses,
    /// which is what every fixture captured before the field existed looks
    /// like.
    fn state_with_force_json(modifier: &str) -> PlanState {
        let force: FactorioForce = serde_json::from_str(&format!(
            r#"{{
              "name": "player",
              "force_id": 1,
              "current_research": null,
              "research_progress": null,
              {modifier}
              "technologies": {{}}
            }}"#
        ))
        .expect("the force fixture must parse");
        let world = fixture_world();
        world.update_force(force).expect("update_force");
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    fn state_with_mining_speed_modifier(modifier: f64) -> PlanState {
        state_with_force_json(&format!(r#""manual_mining_speed_modifier": {modifier},"#))
    }

    #[test]
    fn seconds_convert_to_ticks_and_round_up() {
        assert_eq!(seconds_to_ticks(1.0), 60);
        assert_eq!(seconds_to_ticks(3.2), 192);
        assert_eq!(seconds_to_ticks(0.5), 30);
        // Never round a positive duration down to nothing.
        assert_eq!(seconds_to_ticks(0.001), 1);
        assert_eq!(seconds_to_ticks(0.0), 0);
    }

    #[test]
    fn recipes_are_found_by_name() {
        let s = state();
        let r = recipe_for(&s, "iron-gear-wheel").expect("fixture has iron-gear-wheel");
        assert_eq!(r.category, "crafting");
        assert!(recipe_for(&s, "nonexistent-thing").is_none());
    }

    #[test]
    fn smelting_and_crafting_recipes_are_distinguishable() {
        let s = state();
        assert_eq!(recipe_for(&s, "iron-plate").unwrap().category, "smelting");
        assert_eq!(
            recipe_for(&s, "automation-science-pack").unwrap().category,
            "crafting"
        );
    }

    /// A worm whose reach **clips** the iron field must push tile selection
    /// onto the tiles beyond it -- and the four selectors must agree about
    /// that, because `resource_seats` sizes a split `resource_tiles_for` then
    /// has to fill, and a seat counted here but refused there is exactly the
    /// disagreement the claim ledger exists to prevent.
    ///
    /// **The geometry is chosen so the guard has something to refuse AND
    /// something to fall back to, and both are asserted as preconditions.**
    /// `test_utils::fixture_world` holds one 11x11 iron field, x -44.5..-34.5
    /// and y 35.5..45.5, whose diagonal is about 14 tiles. A small worm
    /// reaches 25, so a worm standing *in* the field covers all 121 tiles and
    /// there is no safe tile at all. The worm is therefore parked 20 tiles off
    /// the near corner along the field's own diagonal, where the 25-tile
    /// boundary cuts through the middle of the patch.
    ///
    /// Two earlier versions of this test failed for that reason and the
    /// failure is the finding: **on a patch smaller than a worm's reach there
    /// is no safe tile**, and ore then falls through to `NoApplicableMethod`
    /// rather than to a refusal naming the worm -- unlike `Chop`, which does
    /// name it. That asymmetry is deliberate for the maps measured here (0% of
    /// coal, copper, stone, uranium and crude oil charted on seed 31337 is
    /// threatened, and 15.3% of iron) and is written down rather than fixed.
    #[test]
    fn a_worm_clipping_the_iron_field_moves_selection_past_its_reach() {
        let origin = Position::new(0., 0.);
        let unguarded = state();
        let first = nearest_resource_tile(&unguarded, "iron-ore", &origin, 1)
            .expect("the fixture has iron ore");

        let world = fixture_world();
        let mut worm = factorio_bot_core::types::FactorioEntity::new_stone_furnace(
            &Position::new(-20.5, 21.5),
            Direction::North,
        );
        worm.name = "small-worm-turret".to_owned();
        worm.entity_type = "turret".to_owned();
        world
            .update_chunk_entities(vec![worm])
            .expect("a fixture world accepts an enemy structure");
        let guarded = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        let tiles: Vec<Position> = guarded
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect();
        let covered = tiles
            .iter()
            .filter(|t| guarded.threat_covering(t).is_some())
            .count();
        assert!(
            covered > 0 && covered < tiles.len(),
            "fixture precondition: the worm must clip the field, not miss it and not swallow it              -- {covered} of {} tiles covered",
            tiles.len()
        );
        assert!(
            guarded.threat_covering(&first).is_some(),
            "fixture precondition: the worm covers the tile the unguarded code picked ({first})"
        );

        let picked = nearest_resource_tile(&guarded, "iron-ore", &origin, 1)
            .expect("tiles beyond the worm's reach remain");
        assert_ne!(
            picked, first,
            "selection must move off a tile inside a worm's reach"
        );
        assert!(
            guarded.threat_covering(&picked).is_none(),
            "and the tile it moved to must itself be outside every standoff, got {picked}"
        );

        for (tile, _take) in resource_tiles_for(&guarded, "iron-ore", &origin, 20) {
            assert!(
                guarded.threat_covering(&tile).is_none(),
                "resource_tiles_for handed out a threatened tile: {tile}"
            );
        }
        assert!(
            resource_supply_at_least(&guarded, "iron-ore", 20),
            "the far tiles still supply; the guard must not empty the map"
        );
        assert!(
            resource_seats(&guarded, "iron-ore", 4) > 0,
            "and they still seat miners"
        );
    }

    /// `fixture_world()` with a `small-worm-turret` standing at `at`.
    ///
    /// Built from a stone furnace and renamed, the way
    /// `a_worm_clipping_the_iron_field_moves_selection_past_its_reach` does:
    /// `update_chunk_entities` is the only door into `EntityGraph::threats`
    /// and it decides by `entity_type`, so the shape of the entity it is
    /// handed does not matter and the name and type do.
    fn state_with_worm(at: Position) -> PlanState {
        let world = fixture_world();
        let mut worm = FactorioEntity::new_stone_furnace(&at, Direction::North);
        worm.name = "small-worm-turret".to_owned();
        worm.entity_type = "turret".to_owned();
        world
            .update_chunk_entities(vec![worm])
            .expect("a fixture world accepts an enemy structure");
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// A **building** is sited out of a worm's reach when there is anywhere
    /// else to put it -- the placement half of the owner's "threats for all
    /// actions", and the half the 2026-09-08 probe says matters most: a
    /// character standing 24 tiles from a live small worm lost 153 of 250
    /// health in 300 ticks, while one walking past at the same distance lost
    /// nothing.
    ///
    /// The geometry is the same trick the ore test above needs, for the same
    /// reason. `free_area_near_where` reaches 12 rings and a small worm reaches
    /// 25, so a worm covering the origin covers most of the search window: it
    /// is parked 20 tiles west, where the origin is inside the standoff and
    /// the eastern rings are outside it. Both halves are asserted as
    /// preconditions, because a worm that missed the origin or swallowed the
    /// whole window would make this test pass without the guard existing.
    #[test]
    fn a_furnace_is_sited_out_of_a_worms_reach_when_the_rings_offer_one() {
        let origin = Position::new(0., 0.);
        let unguarded = state();
        let before =
            free_area_near(&unguarded, &origin, "stone-furnace").expect("open ground at spawn");

        let guarded = state_with_worm(Position::new(-20.5, 0.5));
        assert!(
            guarded.threat_covering(&before).is_some(),
            "fixture precondition: the worm must cover the site the unguarded search picked \
             ({before})"
        );

        let after = free_area_near(&guarded, &origin, "stone-furnace")
            .expect("ground beyond the worm's reach is inside the search radius");
        assert_ne!(
            after, before,
            "a building must not be sited inside a charted worm's attack range"
        );
        assert!(
            guarded.threat_covering(&after).is_none(),
            "and the site it moved to must itself be outside every standoff, got {after}"
        );
    }

    /// **And it falls back rather than refusing.** This is the whole
    /// difference between the threat guard here and the ore guard beside it:
    /// running out of unthreatened ground is recoverable -- the building
    /// stands where it always did -- and deleting a plan that exists is not.
    ///
    /// The worm sits 10 tiles west, near enough that all 625 candidates in all
    /// 12 rings are inside its 25-tile reach -- the furthest corner is 24.4
    /// tiles from it -- and far enough that its own collision box stands on
    /// none of them. **That second condition is not decoration**: a first
    /// version parked the worm on the origin and the test failed with the site
    /// moving to (-1, -1), because the worm's own footprint made (0, 0)
    /// unbuildable. The guard was not involved at all, and a fixture that
    /// blocks the ground it is measuring cannot tell the two apart.
    #[test]
    fn a_worm_over_the_whole_search_window_still_yields_the_unguarded_site() {
        let origin = Position::new(0., 0.);
        let before = free_area_near(&state(), &origin, "stone-furnace").expect("open ground");

        let guarded = state_with_worm(Position::new(-9.5, 0.5));
        assert!(
            guarded.is_area_free("stone-furnace", &before),
            "fixture precondition: the worm must not stand on the site the unguarded search \
             picked ({before}), or this measures its footprint and not the guard"
        );
        let corner = Position::new(
            origin.x + FREE_TILE_SEARCH_RADIUS as f64,
            origin.y + FREE_TILE_SEARCH_RADIUS as f64,
        );
        assert!(
            guarded.threat_covering(&corner).is_some(),
            "fixture precondition: the worm must cover even the furthest ring ({corner}), or the \
             first pass would succeed and this would not be testing the fallback"
        );

        let after = free_area_near(&guarded, &origin, "stone-furnace")
            .expect("a threat must not delete a site that exists");
        assert_eq!(
            after, before,
            "with no safe candidate anywhere, the search must answer exactly what it answered \
             before the guard existed"
        );
    }

    #[test]
    fn the_nearest_resource_tile_is_in_the_patch_and_holds_enough() {
        let s = state();
        let origin = Position::new(0., 0.);
        let tile = nearest_resource_tile(&s, "iron-ore", &origin, 5).expect("fixture has iron ore");
        assert!(s.resource_available(&tile, "iron-ore") >= 5);
        // The iron field covers the tiles x -45..=-35, y 35..=45. Tiles are
        // reported at their *centres* -- where the ore entity actually is, and
        // the only position `find_entity` will match -- so the positions run
        // x -44.5..=-34.5, y 35.5..=45.5.
        assert!(
            tile.x <= -34.5 && tile.x >= -44.5,
            "unexpected x: {}",
            tile.x
        );
        assert!(tile.y >= 35.5 && tile.y <= 45.5, "unexpected y: {}", tile.y);
        assert_eq!(
            tile.x.fract().abs(),
            0.5,
            "a resource position is a tile centre, not a corner: {tile:?}"
        );
    }

    #[test]
    fn the_nearest_resource_tile_is_deterministic() {
        let s = state();
        let origin = Position::new(0., 0.);
        let a = nearest_resource_tile(&s, "iron-ore", &origin, 1).unwrap();
        let b = nearest_resource_tile(&s, "iron-ore", &origin, 1).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_supply_test_agrees_with_the_tiles_it_replaces() {
        // `resource_supply_at_least` exists so `Mine::applicable` need not
        // build a sorted union of every tile just to ask whether enough
        // exists. It must answer exactly what that emptiness test answered.
        let s = state();
        let origin = Position::new(0., 0.);
        for (item, need) in [
            ("iron-ore", 1u32),
            ("iron-ore", 500),
            ("iron-ore", 100_000),
            ("iron-ore", u32::MAX),
            ("copper-ore", 1200),
            ("uranium-ore", 1),
        ] {
            assert_eq!(
                resource_supply_at_least(&s, item, need),
                !resource_tiles_for(&s, item, &origin, need).is_empty(),
                "disagreed on {} {}",
                need,
                item
            );
        }
    }

    #[test]
    fn a_supply_test_follows_what_has_been_consumed() {
        let mut s = state();
        let tile = nearest_resource_tile(&s, "iron-ore", &Position::new(0., 0.), 1).unwrap();
        let available = s.resource_available(&tile, "iron-ore");
        assert!(resource_supply_at_least(&s, "iron-ore", available));
        s.consume_resource(&tile, "iron-ore", available).unwrap();
        assert_eq!(s.resource_available(&tile, "iron-ore"), 0);
        // The rest of the field still holds plenty, so the emptied tile must
        // not be counted and must not stop the walk either.
        assert!(resource_supply_at_least(&s, "iron-ore", available));
    }

    /// A tile committed to a mining action is gone from *selection* while
    /// still holding what it holds. This is the difference between the two
    /// ledgers, at the level of one call.
    #[test]
    fn a_claimed_tile_is_not_offered_to_the_next_caller() {
        let mut s = state();
        let origin = Position::new(0., 0.);
        let first = nearest_resource_tile(&s, "iron-ore", &origin, 5).expect("iron ore");
        s.claim_resource(&first);

        let second = nearest_resource_tile(&s, "iron-ore", &origin, 5).expect("the patch is big");
        assert_ne!(first, second, "two callers must not get the same tile");

        let tiles = resource_tiles_for(&s, "iron-ore", &origin, 5);
        assert_eq!(tiles.len(), 1);
        assert_ne!(tiles[0].0, first);

        // The claim is a fact about the plan, not about the ground: the tile
        // still holds a full 500, which is what `Condition::ResourceAvailable`
        // has to see when the bot that claimed it actually swings.
        assert_eq!(s.resource_available(&first, "iron-ore"), 500);
        assert_eq!(s.resource_unclaimed(&first, "iron-ore"), 0);
    }

    /// Emitting a mining action is what claims its tile, and the two ledgers
    /// move together: `consume_resource` is the only thing `Effect::Mine`
    /// applies.
    #[test]
    fn consuming_from_a_tile_also_commits_it() {
        let mut s = state();
        let origin = Position::new(0., 0.);
        let tile = nearest_resource_tile(&s, "iron-ore", &origin, 1).expect("iron ore");
        s.consume_resource(&tile, "iron-ore", 1).unwrap();
        assert_eq!(s.resource_available(&tile, "iron-ore"), 499);
        assert_eq!(s.resource_unclaimed(&tile, "iron-ore"), 0);
        assert_ne!(
            nearest_resource_tile(&s, "iron-ore", &origin, 1).expect("iron ore"),
            tile,
            "the 499 left over must not attract a second bot"
        );
    }

    /// The supply test and the tile walk have to agree on claims too, or
    /// `Mine::applicable` would claim a goal `Mine::expand` cannot satisfy.
    #[test]
    fn a_supply_test_follows_what_has_been_claimed() {
        let mut s = state();
        let origin = Position::new(0., 0.);
        let mut tiles: Vec<Position> = s
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect();
        tiles.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        tiles.dedup_by(|a, b| a.x.total_cmp(&b.x).is_eq() && a.y.total_cmp(&b.y).is_eq());
        for tile in &tiles {
            s.claim_resource(tile);
        }
        assert!(
            !resource_supply_at_least(&s, "iron-ore", 1),
            "a fully committed patch supplies nothing more"
        );
        assert_eq!(
            resource_supply_at_least(&s, "iron-ore", 1),
            !resource_tiles_for(&s, "iron-ore", &origin, 1).is_empty(),
            "the two must agree on claims, not only on consumption"
        );
    }

    /// The fixture's iron field is 121 tiles and seats nine bots.
    ///
    /// Derived, not observed: the field is 11 tiles on each axis and the
    /// fixture separation is 3.989, so a greedy walk in `(x, y)` order takes
    /// tiles 0, 4 and 8 along each axis and nothing between them — three
    /// columns of three. A test that read the number back off the function
    /// would pass against any number the function happened to produce.
    #[test]
    fn a_patch_seats_far_fewer_bots_than_it_has_tiles() {
        let s = state();
        let tiles: usize = s
            .resource_patches("iron-ore")
            .into_iter()
            .map(|patch| patch.elements.len())
            .sum();
        assert_eq!(tiles, 121, "the fixture's iron field is 11 by 11");
        assert_eq!(resource_seats(&s, "iron-ore", 100), 9);
    }

    /// A committed patch seats nobody — which is the answer that turns an
    /// unreadable `NoApplicableMethod` into `PlannerError::NoRoomToWork`, so
    /// it has to be zero rather than merely small.
    #[test]
    fn a_fully_committed_patch_seats_nobody() {
        let mut s = state();
        let tiles: Vec<Position> = s
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect();
        for tile in &tiles {
            s.claim_resource(tile);
        }
        assert_eq!(resource_seats(&s, "iron-ore", 100), 0);
        // The copper field next door is untouched, so this is a fact about
        // the patch and not about the state having stopped answering.
        assert!(resource_seats(&s, "copper-ore", 100) > 0);
    }

    /// The cap is a ceiling on the count, not on the patch. It exists only so
    /// that a real ore field — thousands of tiles, hundreds of seats — is not
    /// walked further than any caller can use.
    #[test]
    fn counting_seats_stops_at_the_cap() {
        let s = state();
        assert_eq!(resource_seats(&s, "iron-ore", 4), 4);
        assert_eq!(resource_seats(&s, "iron-ore", 1), 1);
        assert_eq!(resource_seats(&s, "iron-ore", 0), 0);
    }

    /// **The ceiling this work lifts, measured on the fixture it capped.**
    ///
    /// `fixture_world`'s iron patch is 121 tiles and seats nine miners *at
    /// once* at a separation of 3.99. Before a claim carried a runner that was
    /// also the number of mining actions the whole plan could ever emit
    /// against that patch, because a claim was held for the length of the
    /// expansion and crowded everybody out of its neighbourhood — the
    /// un-converged four-bot unlock plan already used eight of the nine, which
    /// is why `worth_converging`'s G6 has to decline there and why nine
    /// pre-existing tests went red the first time convergence fired.
    ///
    /// One bot's own claims are serial, so they need no separation from each
    /// other, and one runner can now work the whole patch tile by tile. The
    /// seat count is unchanged and must be: nine is still the right answer to
    /// "how many bots at once".
    #[test]
    fn one_runner_may_work_the_whole_patch_the_roster_can_only_seat_nine_of() {
        let seats = resource_seats(&state(), "iron-ore", u32::MAX);
        assert_eq!(
            seats, 9,
            "the fixture patch is what the convergence work measured it to be"
        );

        let mut s = state();
        s.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        let origin = Position::new(0., 0.);
        let mut taken = 0u32;
        while let Some(tile) = nearest_resource_tile(&s, "iron-ore", &origin, 1) {
            s.claim_resource(&tile);
            taken += 1;
            assert!(taken <= 1000, "the walk must terminate on a finite patch");
        }
        assert_eq!(
            taken, 121,
            "one runner works its own patch tile by tile; seats are for other bots"
        );

        // And the patch really is used up afterwards, so nothing here has made
        // a tile reusable — only reachable by the one bot whose timeline the
        // claims sit on.
        assert_eq!(resource_seats(&s, "iron-ore", u32::MAX), 0);
    }

    /// A seat is a spot for a bot that is **not yet in the plan**, so it is
    /// counted against every claim whoever holds it. Sharing a runner with the
    /// claims must not make them invisible to the count, or a split would be
    /// promised more participants than the ground can hold at once — which is
    /// exactly the over-commitment `resource_seats` exists to prevent.
    #[test]
    fn seats_are_counted_blind_to_whose_claims_they_are() {
        let mut blind = state();
        let mut owned = state();
        owned.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));

        let tile =
            nearest_resource_tile(&blind, "iron-ore", &Position::new(0., 0.), 1).expect("iron ore");
        blind.claim_resource(&tile);
        owned.claim_resource(&tile);

        assert_eq!(
            resource_seats(&owned, "iron-ore", u32::MAX),
            resource_seats(&blind, "iron-ore", u32::MAX),
            "a claim costs a seat whoever made it"
        );
        assert!(
            resource_seats(&owned, "iron-ore", u32::MAX) < 9,
            "and it really does cost one, or the comparison above is vacuous"
        );
    }

    /// The in-call spacing follows the same rule as the claim, because it is
    /// the same fact: one `Mine::expand` emits one action per tile into one
    /// chain, and a chain is serial. A share big enough to need two tiles gets
    /// the two *nearest* ones when the runner is known, and spaced ones when
    /// it is not.
    #[test]
    fn one_calls_own_tiles_are_spaced_only_when_the_runner_is_unknown() {
        let origin = Position::new(0., 0.);
        let need = DEFAULT_RESOURCE_PER_TILE + 1;

        let blind = state();
        let spaced = resource_tiles_for(&blind, "iron-ore", &origin, need);
        assert_eq!(spaced.len(), 2, "one tile cannot cover more than it holds");
        assert!(
            calculate_distance(&spaced[0].0, &spaced[1].0) >= blind.mining_tile_separation(),
            "with no runner named, two actions may be two bots at once: {spaced:?}"
        );

        let mut owned = state();
        owned.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        let packed = resource_tiles_for(&owned, "iron-ore", &origin, need);
        assert_eq!(packed.len(), 2);
        assert!(
            calculate_distance(&packed[0].0, &packed[1].0) < owned.mining_tile_separation(),
            "one bot's two swings are serial, so it takes the nearer tile: {packed:?}"
        );
    }

    /// An item with no patches seats nobody, and says so without panicking on
    /// an empty walk. `Mine::concurrency` is what turns this into "no limit"
    /// rather than "a limit of zero"; the count itself has no opinion.
    #[test]
    fn an_item_that_is_not_a_resource_has_no_seats() {
        let s = state();
        assert_eq!(resource_seats(&s, "iron-plate", 100), 0);
    }

    /// Tile centres for three isolated `uranium-ore` tiles, ten tiles apart --
    /// far more than [`PlanState::mining_tile_separation`] -- so each is
    /// trivially its own seat and excluding one never perturbs the others.
    /// `fixture_world()` ships no uranium at all (see
    /// `a_missing_resource_has_no_tile`), so these are the only tiles of this
    /// item in play and every assertion below can be exact.
    const URANIUM_A: (f64, f64) = (100.5, 100.5);
    const URANIUM_B: (f64, f64) = (110.5, 100.5);
    const URANIUM_C: (f64, f64) = (120.5, 100.5);

    /// `fixture_world()` plus the three uranium tiles above and, when `block`
    /// is `true`, a `simple-entity` covering `URANIUM_A` built exactly like
    /// `FactorioEntity::new_rock` -- the same shape crash-site wreckage takes
    /// in the real graph (`EntityGraph::add`'s whitelist routes anything
    /// named other than `rock-big`/`rock-huge` into `blocked_tree` only, never
    /// `entity_tree`), so this reproduces the obstruction without needing a
    /// live game's wreck prototype.
    fn state_with_uranium(block: bool) -> PlanState {
        let world = fixture_world();
        let mut entities = vec![
            FactorioEntity::new_resource(
                &Position::new(URANIUM_A.0, URANIUM_A.1),
                Direction::North,
                "uranium-ore",
            ),
            FactorioEntity::new_resource(
                &Position::new(URANIUM_B.0, URANIUM_B.1),
                Direction::North,
                "uranium-ore",
            ),
            FactorioEntity::new_resource(
                &Position::new(URANIUM_C.0, URANIUM_C.1),
                Direction::North,
                "uranium-ore",
            ),
        ];
        if block {
            entities.push(FactorioEntity::new_rock(
                &Position::new(URANIUM_A.0, URANIUM_A.1),
                "crash-site-spaceship-wreck-medium-3",
            ));
        }
        world.update_chunk_entities(entities).unwrap();
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// The failure this fix targets: a resource tile covered by debris must
    /// not be handed to a bot. `nearest_resource_tile` has to walk past it to
    /// the next-nearest tile, exactly as `is_area_free` already walks past a
    /// tree for placement (`docs/superpowers/notes/2026-09-02-placement-refusal.md`).
    #[test]
    fn a_resource_tile_under_debris_is_skipped_by_the_nearest_tile_search() {
        let s = state_with_uranium(true);
        let a = Position::new(URANIUM_A.0, URANIUM_A.1);
        let b = Position::new(URANIUM_B.0, URANIUM_B.1);
        let from = Position::new(95., URANIUM_A.1);

        let tile = nearest_resource_tile(&s, "uranium-ore", &from, 1)
            .expect("two unblocked uranium tiles remain");
        assert_eq!(
            tile, b,
            "the nearest tile is covered by debris and must be skipped"
        );

        // The ore is covered, not gone: `EntityGraph::add` never removes a
        // resource entity because something else was placed over it, so the
        // physical reading `Condition::ResourceAvailable` relies on must stay
        // truthful even though the tile is unusable for a *new* assignment.
        assert!(
            s.resource_available(&a, "uranium-ore") > 0,
            "the covered tile still physically holds ore"
        );
        assert_eq!(
            s.resource_unclaimed(&a, "uranium-ore"),
            0,
            "but it must not be offered to a new mining action"
        );

        // resource_tiles_for reads the same ledger and must agree: asking for
        // everything two tiles hold (`need` is an amount of ore, not a count
        // of tiles -- each tile supplies at most `DEFAULT_RESOURCE_PER_TILE`)
        // draws from exactly the two unblocked ones.
        let tiles = resource_tiles_for(&s, "uranium-ore", &from, 2 * DEFAULT_RESOURCE_PER_TILE);
        assert_eq!(tiles.len(), 2);
        assert!(
            tiles.iter().all(|(t, _)| *t != a),
            "the blocked tile must never appear in a selection"
        );
    }

    /// Negative control: a patch nothing obstructs is unaffected by a block
    /// elsewhere, and only the covered tile -- not its whole patch -- is
    /// excluded. Without this, a selector that excluded far more than the one
    /// obstructed tile could still pass the test above.
    #[test]
    fn an_unobstructed_patch_is_unchanged_by_a_block_elsewhere() {
        let unblocked = state_with_uranium(false);
        let blocked = state_with_uranium(true);
        let origin = Position::new(0., 0.);

        // The fixture's copper-ore field is nowhere near the uranium tiles
        // above, so blocking one of the latter must not perturb it at all.
        assert_eq!(
            nearest_resource_tile(&unblocked, "copper-ore", &origin, 1),
            nearest_resource_tile(&blocked, "copper-ore", &origin, 1),
        );
        assert_eq!(
            resource_seats(&unblocked, "copper-ore", 100),
            resource_seats(&blocked, "copper-ore", 100),
        );

        // Nor does it touch the *other* uranium tiles: all three are pickable
        // when nothing is blocked, and exactly two remain once one is. `need`
        // is an amount of ore, not a tile count, so asking for all three
        // tiles' worth requires `3 * DEFAULT_RESOURCE_PER_TILE`.
        let from = Position::new(95., URANIUM_A.1);
        assert_eq!(
            resource_tiles_for(
                &unblocked,
                "uranium-ore",
                &from,
                3 * DEFAULT_RESOURCE_PER_TILE
            )
            .len(),
            3
        );
        assert_eq!(
            resource_tiles_for(
                &blocked,
                "uranium-ore",
                &from,
                3 * DEFAULT_RESOURCE_PER_TILE
            )
            .len(),
            0,
            "asking for one more tile's worth than the two unblocked tiles hold must fail \
             closed, not silently draw from the covered one"
        );
        assert_eq!(
            resource_tiles_for(
                &blocked,
                "uranium-ore",
                &from,
                2 * DEFAULT_RESOURCE_PER_TILE
            )
            .len(),
            2,
            "exactly the covered tile is missing, nothing more"
        );
    }

    /// `resource_seats` must never promise a bot a tile `resource_tiles_for`
    /// then refuses to hand out -- they read the same ledger by design
    /// (`resource_unclaimed`), and this pins that they keep agreeing once a
    /// tile is obstructed rather than only when one is claimed or crowded.
    #[test]
    fn seats_and_selection_agree_about_an_obstructed_patch() {
        let s = state_with_uranium(true);
        let from = Position::new(95., URANIUM_A.1);

        let seats = resource_seats(&s, "uranium-ore", 100);
        assert_eq!(seats, 2, "one of the three tiles is covered by debris");

        // `resource_tiles_for`'s `need` is an amount of ore, not a tile
        // count, so the capacity `seats` tiles promise is `seats *
        // DEFAULT_RESOURCE_PER_TILE`. Selection must be able to fulfil
        // exactly that -- no more, since a third tile does not exist to draw
        // from, and no less, since that would mean a seat existed selection
        // could not actually place a bot on.
        let tiles = resource_tiles_for(&s, "uranium-ore", &from, seats * DEFAULT_RESOURCE_PER_TILE);
        assert_eq!(
            tiles.len() as u32,
            seats,
            "every seat resource_seats counts must be a tile resource_tiles_for can hand out"
        );
    }

    #[test]
    fn a_missing_resource_cannot_supply_anything() {
        let s = state();
        assert!(!resource_supply_at_least(&s, "uranium-ore", 1));
        // Nothing is always available: the zero case is trivially satisfiable,
        // which is why callers check the shortfall is non-zero first.
        assert!(resource_supply_at_least(&s, "uranium-ore", 0));
    }

    #[test]
    fn a_missing_resource_has_no_tile() {
        let s = state();
        assert!(nearest_resource_tile(&s, "uranium-ore", &Position::new(0., 0.), 1).is_none());
    }

    #[test]
    fn a_free_tile_is_found_and_is_actually_free() {
        let s = state();
        let pos = free_area_near(&s, &Position::new(0., 0.), "stone-furnace")
            .expect("origin area is open");
        assert!(s.is_area_free("stone-furnace", &pos));
    }

    #[test]
    fn a_free_tile_avoids_an_occupied_one() {
        let mut s = state();
        let origin = Position::new(0., 0.);
        let first = free_area_near(&s, &origin, "stone-furnace").unwrap();
        let furnace = factorio_bot_core::types::FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: first.clone(),
            ..Default::default()
        };
        s.create_entity(furnace);
        let second = free_area_near(&s, &origin, "stone-furnace").unwrap();
        assert_ne!(first, second);
        assert!(s.is_area_free("stone-furnace", &second));
    }

    /// Siting keeps off the ore -- and that is the siting search's own rule,
    /// not a fact about the ground.
    ///
    /// The ring search starts *on* the anchor, which is an ore tile, and
    /// `PlanState::is_area_free` would take it: the game builds over a patch,
    /// and since `ore-does-not-block` so does this planner. What steps off it
    /// is `free_area_near_where`'s own filter, which exists so a plan does not
    /// bury the patch it is about to mine -- measured, not assumed: without it
    /// `seventy_five_packs_are_crafted_on_several_bots_and_each_delivers_to_a_lab`
    /// fails with `NoApplicableMethod` on `have 40 iron-ore`.
    #[test]
    fn siting_steps_off_the_ore_that_no_longer_blocks_it() {
        let s = state();
        let ore = nearest_resource_tile(&s, "iron-ore", &Position::new(0., 0.), 1)
            .expect("fixture has iron ore");
        assert!(
            s.resource_available(&ore, "iron-ore") > 0,
            "the anchor {ore:?} is an ore tile"
        );
        assert!(
            s.is_area_free("stone-furnace", &ore),
            "the ground itself takes a furnace -- ore is not an obstacle"
        );
        let tile =
            free_area_near(&s, &ore, "stone-furnace").expect("open ground next to the patch");
        let area = s
            .collision_area("stone-furnace", &tile)
            .expect("the fixture knows a stone furnace");
        assert!(
            !s.covers_any_resource(&area),
            "the chosen site {tile:?} covers ore"
        );
    }

    #[test]
    fn mining_a_fixture_ore_takes_two_seconds_not_one() {
        // `iron-ore`'s `mining_time` is 1.0 s, but that is the numerator of a
        // division, not an answer: a character mines at 0.5, so one ore takes
        // two seconds. Reading `mining_time` straight out of the prototype gave
        // 60 and made every hand-mining estimate exactly 2x too fast — measured
        // live, 180 planned against 362 observed for three ore.
        let s = state();
        assert_eq!(mining_ticks(&s, "iron-ore"), 120);
    }

    #[test]
    fn the_character_mining_speed_comes_from_the_world_not_a_constant() {
        // The divisor is a prototype value a mod can change, so it has to be
        // read rather than baked in. Same ore, three different characters.
        for (speed, expected) in [(1.0, 60), (0.5, 120), (0.25, 240)] {
            let s = state_with_character_mining_speed(Some(speed));
            assert_eq!(
                mining_ticks(&s, "iron-ore"),
                expected,
                "a character mining at {speed} should take {expected} ticks per ore"
            );
        }
    }

    #[test]
    fn the_forces_manual_mining_speed_modifier_speeds_mining_up() {
        // Vanilla's `steel-axe` research grants `character-mining-speed +1`,
        // which the game applies as `speed * (1 + modifier)`. A force that has
        // it mines an ore in one second, not two — so a planner that ignored
        // the modifier would be 2x too *slow* for a mid-game force, exactly
        // the mirror of the defect the divisor fixed.
        for (modifier, expected) in [(0.0, 120), (1.0, 60), (3.0, 30)] {
            let s = state_with_mining_speed_modifier(modifier);
            assert_eq!(
                mining_ticks(&s, "iron-ore"),
                expected,
                "a force with manual_mining_speed_modifier {modifier} \
                 should take {expected} ticks per ore"
            );
        }
    }

    #[test]
    fn a_force_that_reports_no_modifier_mines_at_the_unmodified_rate() {
        // `None` is "the world did not tell us", which is the game's own
        // default of 0 — not an error and not a reason to skip the divisor.
        let absent = state_with_force_json("");
        assert_eq!(mining_ticks(&absent, "iron-ore"), 120);
        let null = state_with_force_json(r#""manual_mining_speed_modifier": null,"#);
        assert_eq!(mining_ticks(&null, "iron-ore"), 120);
    }

    #[test]
    fn a_world_with_no_character_prototype_falls_back_to_the_vanilla_speed() {
        // Better a documented vanilla default than a silent divide by nothing:
        // an absent or nonsensical prototype must not turn into 0 or infinity.
        for missing in [None, Some(0.0)] {
            let s = state_with_character_mining_speed(missing);
            assert_eq!(mining_ticks(&s, "iron-ore"), 120);
        }
    }

    #[test]
    fn an_exact_distance_tie_breaks_on_the_lower_position() {
        let s = state();
        // Exactly halfway between the ore tiles at x = -41 and x = -40, whose
        // centres are -40.5 and -39.5, on the row whose centre is y = 39.5.
        // Both are equidistant, so the lower (x, y) must win regardless of
        // which patch was visited first.
        let origin = Position::new(-40.0, 39.5);
        let tile = nearest_resource_tile(&s, "iron-ore", &origin, 1).expect("iron ore");
        assert_eq!(tile, Position::new(-40.5, 39.5));
    }

    #[test]
    fn mining_time_comes_from_the_prototype_not_a_constant() {
        let s = state();
        // stone-furnace's prototype says 0.2 s; a hardcoded one-second default
        // would give 120 instead. Divided by the character's 0.5, 0.4 s.
        assert_eq!(mining_ticks(&s, "stone-furnace"), 24);
        // An item with no prototype at all falls back to one second of mining
        // time, which is still two seconds of mining.
        assert_eq!(mining_ticks(&s, "not-a-real-entity"), 120);
    }

    #[test]
    fn the_free_tile_search_moves_outward_through_rings() {
        let mut s = state();
        let origin = Position::new(0., 0.);
        // Block the origin and the whole first ring.
        for dx in -1..=1 {
            for dy in -1..=1 {
                s.create_entity(factorio_bot_core::types::FactorioEntity {
                    name: "stone-furnace".into(),
                    entity_type: "furnace".into(),
                    position: Position::new(dx as f64, dy as f64),
                    ..Default::default()
                });
            }
        }
        let found = free_area_near(&s, &origin, "stone-furnace").expect("ring 2 is open");
        assert!(s.is_area_free("stone-furnace", &found));
        assert!(
            found.x.abs() >= 2.0 || found.y.abs() >= 2.0,
            "must have moved past the blocked 3x3, got {}",
            found
        );
    }

    /// The end-to-end shape of the run-8b failure: the site chosen for a
    /// furnace next to an ore patch has to be one the game would actually
    /// accept, and "no factory entity here" is not the same question.
    ///
    /// `fixture_world`'s forest sits around `(-20, -20)`; the search starts
    /// inside it. Before `is_area_clear` read the blocked tree the first
    /// candidate — the forest floor itself — came back as free, which is the
    /// position the executor then had refused with
    /// `can_place_entity said 'no'`.
    #[test]
    fn the_free_tile_search_walks_out_of_a_forest() {
        let s = state();
        let in_the_trees = Position::new(-20., -20.);
        assert!(
            !s.is_area_free("stone-furnace", &in_the_trees),
            "the search has to start somewhere a furnace does not fit, or \
             this test proves nothing"
        );
        let found =
            free_area_near(&s, &in_the_trees, "stone-furnace").expect("the forest is not endless");
        assert_ne!(found, in_the_trees);
        assert!(
            s.is_area_free("stone-furnace", &found),
            "the site it returns must be one a furnace fits on, got {found}"
        );
    }

    #[test]
    fn one_tile_is_enough_for_a_small_request() {
        let s = state();
        let tiles = resource_tiles_for(&s, "iron-ore", &Position::new(0., 0.), 5);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].1, 5);
    }

    #[test]
    fn a_large_request_spans_tiles_nearest_first() {
        let s = state();
        // 500 per tile, so 1200 needs three: 500 + 500 + 200.
        let tiles = resource_tiles_for(&s, "iron-ore", &Position::new(0., 0.), 1200);
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles.iter().map(|(_, n)| *n).sum::<u32>(), 1200);
        assert_eq!(tiles[0].1, 500);
        assert_eq!(tiles[1].1, 500);
        assert_eq!(tiles[2].1, 200);
        // Nearest first: distances must be non-decreasing.
        let origin = Position::new(0., 0.);
        for pair in tiles.windows(2) {
            let a = calculate_distance(&origin, &pair[0].0);
            let b = calculate_distance(&origin, &pair[1].0);
            assert!(a <= b, "tiles must come nearest-first: {} then {}", a, b);
        }
    }

    #[test]
    fn a_request_larger_than_the_patch_yields_nothing() {
        let s = state();
        let tiles = resource_tiles_for(&s, "iron-ore", &Position::new(0., 0.), 10_000_000);
        assert!(tiles.is_empty());
    }

    #[test]
    fn an_absent_resource_yields_nothing() {
        let s = state();
        assert!(resource_tiles_for(&s, "uranium-ore", &Position::new(0., 0.), 1).is_empty());
    }

    #[test]
    fn recipe_helpers_read_ingredients_products_and_energy() {
        let s = state();
        let asp = recipe_for(&s, "automation-science-pack").unwrap();
        let mut ingredients = ingredients_of(&asp);
        ingredients.sort();
        assert_eq!(
            ingredients,
            vec![
                ("copper-plate".to_string(), 1),
                ("iron-gear-wheel".to_string(), 1)
            ]
        );
        assert_eq!(output_per_craft(&asp, "automation-science-pack"), 1);
        assert_eq!(recipe_ticks(&asp), 300, "5 s");

        let gear = recipe_for(&s, "iron-gear-wheel").unwrap();
        assert_eq!(ingredients_of(&gear), vec![("iron-plate".to_string(), 2)]);
        assert_eq!(recipe_ticks(&gear), 30, "0.5 s");

        // An item this recipe does not produce defaults to one per craft.
        assert_eq!(output_per_craft(&gear, "something-else"), 1);
    }

    /// `fixture_world()` with `machine`'s crafting speed set to `speed`, or the
    /// prototype removed entirely when `speed` is `None`.
    ///
    /// The fixture ships real furnaces carrying real speeds — 1.0, 2.0, 2.0 —
    /// so overriding one in place is what lets a test distinguish "the divisor
    /// came from the world" from "the divisor happens to equal the constant".
    fn state_with_crafting_speed(machine: &str, speed: Option<f64>) -> PlanState {
        let world = fixture_world();
        match speed {
            Some(speed) => {
                let mut prototype = world
                    .globals
                    .entity_prototypes
                    .get(machine)
                    .unwrap_or_else(|| panic!("the fixture ships a {machine} prototype"))
                    .clone();
                prototype.crafting_speed = Some(speed);
                world
                    .globals
                    .entity_prototypes
                    .insert(machine.into(), prototype);
            }
            None => {
                world.globals.entity_prototypes.remove(machine);
            }
        }
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    #[test]
    fn the_furnaces_report_the_speeds_the_live_game_reports() {
        // The premise of the whole division, asserted rather than assumed:
        // the furnaces do NOT all run at the same speed, so a smelting
        // duration that ignores the machine is only right for one of them.
        //
        // These three values were read out of a live 2.1.17 game via
        // `prototypes.entity[n].get_crafting_speed()`: 1, 2, 2. The fixture
        // agrees, which is what makes it usable as a stand-in here.
        let s = state();
        assert_eq!(machine_crafting_speed(&s, "stone-furnace"), 1.0);
        assert_eq!(machine_crafting_speed(&s, "steel-furnace"), 2.0);
        assert_eq!(machine_crafting_speed(&s, "electric-furnace"), 2.0);
    }

    #[test]
    fn smelting_in_a_faster_furnace_takes_proportionally_less_time() {
        // iron-plate's `energy` is 3.2 s. That is the time at crafting speed
        // 1 and the numerator of a division, not the answer: a steel or
        // electric furnace runs at 2, so it smelts a plate in 1.6 s.
        //
        // Reading `energy` straight out of the recipe gave 192 for all three,
        // which is right for the stone furnace and exactly 2x too slow for
        // the other two.
        let s = state();
        let iron_plate = recipe_for(&s, "iron-plate").expect("the fixture has iron-plate");
        assert_eq!(recipe_ticks(&iron_plate), 192, "3.2 s at speed 1");

        assert_eq!(smelting_ticks(&s, &iron_plate, "stone-furnace"), 192);
        assert_eq!(smelting_ticks(&s, &iron_plate, "steel-furnace"), 96);
        assert_eq!(smelting_ticks(&s, &iron_plate, "electric-furnace"), 96);
    }

    #[test]
    fn the_crafting_speed_comes_from_the_world_not_a_constant() {
        // The divisor is prototype data a mod can change, and Factorio's own
        // furnaces already disagree, so it has to be read rather than baked
        // in. Same recipe, same machine name, four different worlds.
        let iron_plate = recipe_for(&state(), "iron-plate").expect("the fixture has iron-plate");
        for (speed, expected) in [(0.5, 384), (1.0, 192), (2.0, 96), (4.0, 48)] {
            let s = state_with_crafting_speed("stone-furnace", Some(speed));
            assert_eq!(
                smelting_ticks(&s, &iron_plate, "stone-furnace"),
                expected,
                "a furnace crafting at {speed} should take {expected} ticks per plate"
            );
        }
    }

    #[test]
    fn a_machine_the_world_cannot_speak_for_falls_back_to_the_vanilla_speed() {
        // Better a documented vanilla stone furnace than a silent divide by
        // nothing: an absent, zero or negative speed must not turn a duration
        // into zero, infinity or a negative number.
        let iron_plate = recipe_for(&state(), "iron-plate").expect("the fixture has iron-plate");
        for absent in [None, Some(0.0), Some(-1.0)] {
            let s = state_with_crafting_speed("stone-furnace", absent);
            assert_eq!(
                smelting_ticks(&s, &iron_plate, "stone-furnace"),
                192,
                "crafting_speed {absent:?} must fall back, not divide by it"
            );
        }
        // A machine with no prototype at all is the same case.
        let s = state();
        assert_eq!(smelting_ticks(&s, &iron_plate, "not-a-real-machine"), 192);
    }

    #[test]
    fn the_hand_craft_duration_is_left_at_crafting_speed_one() {
        // `recipe_ticks` is not a leftover: it is the character's own answer.
        // A live 2.1.17 `character` prototype reports
        // `get_crafting_speed() == 1`, so dividing the hand-craft path by it
        // would change nothing, and this pins that reading rather than
        // leaving the two paths looking accidentally inconsistent.
        // The fixture's `character` predates the field and carries no speed,
        // so it is given the 1 the live game reports — otherwise this would
        // assert the fallback rather than a reading.
        let s = state_with_crafting_speed("character", Some(1.0));
        let gear = recipe_for(&s, "iron-gear-wheel").expect("the fixture has iron-gear-wheel");
        assert_eq!(recipe_ticks(&gear), 30, "0.5 s at the character's speed 1");
        assert_eq!(
            smelting_ticks(&s, &gear, "character"),
            30,
            "and the divisor, read from the world, agrees — which is why the \
             hand-craft path is left alone"
        );
    }

    /// The fixture's iron patch is `rect_fields` over (-45, 35)..(-35, 45) --
    /// integer positions, which key to tiles whose centres are a half-tile
    /// east and south -- so the tile nearest a bot at the origin is this one.
    const NEAREST_IRON: (f64, f64) = (-34.5, 35.5);

    /// A machine an earlier plan left standing on the iron patch, reaching
    /// the world through the same door the mod's `on_some_entity_created`
    /// uses. `at` is the entity position; the box is the prototype's own.
    fn world_with_standing(name: &str, entity_type: &str, at: Position) -> Arc<FactorioSurface> {
        let world = fixture_world();
        let collision = world
            .globals
            .entity_prototypes
            .get(name)
            .map(|proto| proto.collision_box.clone())
            .unwrap_or_else(|| panic!("the fixture has no prototype for {name}"));
        world
            .on_some_entity_created(FactorioEntity {
                name: name.into(),
                entity_type: entity_type.into(),
                bounding_box: add_to_rect(&collision, &at),
                position: at,
                ..Default::default()
            })
            .expect("the machine stands");
        Arc::new(world)
    }

    /// A burner drill at (-35, 36) covers the four iron tiles nearest the
    /// origin, `NEAREST_IRON` among them.
    fn standing_drill() -> (Arc<FactorioSurface>, Position) {
        let at = Position::new(-35., 36.);
        (
            world_with_standing("burner-mining-drill", "mining-drill", at.clone()),
            at,
        )
    }

    /// Every tile of `item` a hand-mine selector would offer from the origin
    /// must be outside `covered`, and the nearest offered must not be the
    /// covered nearest.
    fn assert_selection_walks_past(s: &PlanState, covered: &Rect, covered_nearest: &Position) {
        let origin = Position::new(0., 0.);
        let tile = nearest_resource_tile(s, "iron-ore", &origin, 1).expect("iron remains");
        assert_ne!(&tile, covered_nearest, "the covered tile must be skipped");
        assert!(
            !covered.contains(&tile),
            "the nearest tile offered, {tile}, is under the machine at {covered:?}"
        );
        assert!(
            s.resource_available(covered_nearest, "iron-ore") > 0,
            "the ore under the machine is covered, not gone"
        );
        assert_eq!(
            s.resource_unclaimed(covered_nearest, "iron-ore"),
            0,
            "but it is not offered to a new mining action"
        );
        let tiles = resource_tiles_for(s, "iron-ore", &origin, 6 * DEFAULT_RESOURCE_PER_TILE);
        assert!(!tiles.is_empty());
        assert!(
            tiles.iter().all(|(t, _)| !covered.contains(t)),
            "no covered tile appears in a selection: {tiles:?}"
        );
    }

    /// The tile a character would be sent to is the one the game selects at
    /// that position, and over a standing machine that is the machine --
    /// `expected iron-ore at (-7.5/-29.5), found burner-mining-drill`
    /// (`run-1788559688-08406`, plan 2). So a tile under a machine an
    /// *earlier plan* built is not hand-minable, whatever the machine is.
    #[test]
    fn a_resource_tile_under_a_standing_machine_is_skipped_by_hand_mining() {
        let nearest = Position::new(NEAREST_IRON.0, NEAREST_IRON.1);
        for (name, entity_type, at) in [
            (
                "burner-mining-drill",
                "mining-drill",
                Position::new(-35., 36.),
            ),
            ("stone-furnace", "furnace", Position::new(-35., 36.)),
            ("wooden-chest", "container", nearest.clone()),
        ] {
            let world = world_with_standing(name, entity_type, at.clone());
            let s = PlanState::from_world(world, &[BotId(1)]);
            let covered = s
                .collision_area(name, &at)
                .expect("the fixture has the prototype");
            assert!(
                covered.contains(&nearest),
                "{name} at {at} must cover the nearest tile"
            );
            assert_selection_walks_past(&s, &covered, &nearest);
        }
    }

    /// The shape the run actually had: the drill had been standing long
    /// enough to empty one of its four tiles, and the mod's report of that
    /// tile's deletion is what made the model forget the drill's ground.
    /// The drill must still keep hand mining off its remaining three tiles
    /// -- and still count them as its own for the cell ledger, since a drill
    /// mines the tiles under itself.
    #[test]
    fn a_drill_that_emptied_a_tile_under_itself_still_covers_the_rest() {
        let (world, at) = standing_drill();
        let per_tile = {
            let s = PlanState::from_world(world.clone(), &[BotId(1)]);
            s.resource_available(&Position::new(-40.5, 40.5), "iron-ore")
        };
        assert!(per_tile > 0);
        {
            let s = PlanState::from_world(world.clone(), &[BotId(1)]);
            assert_eq!(
                crate::method::produce::cell_yield(&s, &at, Direction::North, "iron-ore"),
                4 * per_tile,
                "the drill's own view counts all four tiles under it"
            );
        }

        // The mod's `on_resource_depleted` -> `on_some_entity_deleted`: the
        // tile at (-34.5, 36.5), under the drill, with nothing left on it.
        let mut emptied =
            FactorioEntity::new_resource(&Position::new(-34.5, 36.5), Direction::North, "iron-ore");
        emptied.amount = Some(0);
        world
            .on_some_entity_deleted(emptied)
            .expect("the report lands");

        let s = PlanState::from_world(world, &[BotId(1)]);
        let covered = s
            .collision_area("burner-mining-drill", &at)
            .expect("the fixture has the prototype");
        let nearest = Position::new(NEAREST_IRON.0, NEAREST_IRON.1);
        assert_selection_walks_past(&s, &covered, &nearest);
        assert_eq!(
            crate::method::produce::cell_yield(&s, &at, Direction::North, "iron-ore"),
            3 * per_tile,
            "the drill still owns the three tiles it has not emptied"
        );
    }

    /// Labs share a research by units: `ceil(units / labs)` rounds of the
    /// unit time, identical to `research_ticks` at one lab, and a lab count
    /// of zero is read as one rather than as a division.
    #[test]
    fn labs_share_a_research_by_units() {
        let s = PlanState::from_world(
            Arc::new(crate::test_world::world_with_long_research(75, 300.0)),
            &[BotId(1)],
        );
        let tech = s.technology("long-research").expect("fixture technology");
        assert_eq!(research_ticks(&tech), 22_500);
        assert_eq!(research_ticks_in_labs(&tech, 1), research_ticks(&tech));
        assert_eq!(research_ticks_in_labs(&tech, 2), 38 * 300);
        assert_eq!(research_ticks_in_labs(&tech, 3), 25 * 300);
        assert_eq!(research_ticks_in_labs(&tech, 75), 300);
        assert_eq!(
            research_ticks_in_labs(&tech, 76),
            300,
            "a lab with no unit is idle, not negative"
        );
        assert_eq!(research_ticks_in_labs(&tech, 0), research_ticks(&tech));
    }

    /// The lane is the beacon's own footprint, read off `collision_box`, and
    /// the fixture's beacon is the live 2.1.17 one (+/-1.19921875, i.e.
    /// 2.3984375 across -- a *three*-tile building rounded up, not a two-tile
    /// one rounded down).
    #[test]
    fn a_beacons_lane_is_its_own_footprint_from_the_prototype() {
        let state = state();
        let geometry = beacon_geometry(&state, BEACON).expect("the fixture ships a beacon");
        assert!(
            (geometry.footprint_tiles() - 3.0).abs() < f64::EPSILON,
            "a 2.3984375-wide collision box is a 3x3 building, got {}",
            geometry.footprint_tiles()
        );
        assert!((geometry.lane_tiles() - 3.0).abs() < f64::EPSILON);
    }

    /// The falsifier for the whole approach: the lane must follow the
    /// prototype, so a beacon a mod made 5x5 must get a five-tile lane. A
    /// hard-coded three passes the test above and fails this one.
    #[test]
    fn the_lane_follows_the_prototype_and_not_the_number_three() {
        let world = fixture_world();
        let mut beacon = world
            .globals
            .entity_prototypes
            .get(BEACON)
            .expect("the fixture ships a beacon")
            .clone();
        beacon.collision_box = Rect::new(
            &Position::new(-2.19921875, -2.19921875),
            &Position::new(2.19921875, 2.19921875),
        );
        world
            .globals
            .entity_prototypes
            .insert(BEACON.into(), beacon);
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        let geometry = beacon_geometry(&state, BEACON).expect("the beacon is still there");
        assert!(
            (geometry.lane_tiles() - 5.0).abs() < f64::EPSILON,
            "a 4.3984375-wide beacon needs a five-tile lane, got {}",
            geometry.lane_tiles()
        );
    }

    /// `supply_area_distance` is absent from our data, so every question that
    /// needs it answers `None`. This is what makes the gap visible instead of
    /// letting a default stand in for a measurement.
    ///
    /// The fixture's beacon still declares nothing, so this is now the
    /// *pre-field dump* case rather than the only case there is — see
    /// `a_beacons_supply_area_distance_comes_from_its_own_prototype`.
    #[test]
    fn an_absent_supply_area_distance_answers_none_rather_than_a_default() {
        let state = state();
        let geometry = beacon_geometry(&state, BEACON).expect("the fixture ships a beacon");
        assert_eq!(geometry.supply_area_distance(), None);
        assert_eq!(geometry.max_row_separation_tiles(), None);
        assert_eq!(geometry.reaches_gap(0.0), None);
        assert_eq!(beacon_supply_area_distance(&state, BEACON), None);
    }

    /// The rule the field feeds: a row is reached iff its gap is **strictly
    /// under** `d`, and two rows may sit `b + 2d` apart.
    ///
    /// Fed by hand through [`BeaconGeometry::new`], which pins the arithmetic
    /// with no world in the way. Vanilla's 3x3 beacon at `d = 3` gives the 9x9
    /// supply area the game shows, which is `b + 2d` and emphatically not
    /// `2d`.
    #[test]
    fn a_fed_supply_area_distance_reaches_a_row_inside_it_and_not_one_at_it() {
        let geometry = BeaconGeometry::new(3.0, Some(3.0));
        assert_eq!(geometry.reaches_gap(0.0), Some(true), "flush is reached");
        assert_eq!(geometry.reaches_gap(2.9), Some(true));
        assert_eq!(
            geometry.reaches_gap(3.0),
            Some(false),
            "the supply area ends at d, so a row exactly d away is outside it"
        );
        assert_eq!(geometry.max_row_separation_tiles(), Some(9.0));
    }

    /// `fixture_world()` with `name`'s `supply_area_distance` set.
    ///
    /// The fixture ships every pole and the beacon with the field **absent**,
    /// which is what a pre-2026-09-06 dump looks like, so overwriting it in
    /// place is what lets a test say "the distance came from the world".
    fn state_with_supply_area(name: &str, distance: Option<f64>) -> PlanState {
        let world = fixture_world();
        let mut prototype = world
            .globals
            .entity_prototypes
            .get(name)
            .expect("the fixture ships this prototype")
            .clone();
        prototype.supply_area_distance = distance;
        world
            .globals
            .entity_prototypes
            .insert(name.into(), prototype);
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// The seam reads the prototype, and the whole geometry follows it.
    ///
    /// Vanilla's beacon is `d = 3` on a 3x3, so the two rows it can serve sit
    /// `b + 2d = 9` apart — the 9x9 the game draws. A reader that had copied
    /// the *pole* convention would answer `2d = 6` here, which is the
    /// three-tile error this pairing exists to catch.
    #[test]
    fn a_beacons_supply_area_distance_comes_from_its_own_prototype() {
        let state = state_with_supply_area(BEACON, Some(3.0));
        assert_eq!(beacon_supply_area_distance(&state, BEACON), Some(3.0));

        let geometry = beacon_geometry(&state, BEACON).expect("the fixture ships a beacon");
        assert_eq!(geometry.supply_area_distance(), Some(3.0));
        assert_eq!(
            geometry.max_row_separation_tiles(),
            Some(9.0),
            "b + 2d, not 2d"
        );
        assert_eq!(geometry.reaches_gap(2.9), Some(true));
        assert_eq!(geometry.reaches_gap(3.0), Some(false));
    }

    /// **A pole carries the same field under a different convention, and this
    /// must not read it.**
    ///
    /// `small-electric-pole`'s 2.5 is half the side of a 5x5 square centred on
    /// the pole; a beacon's is distance beyond its own footprint. Answering
    /// `Some(2.5)` for a pole would hand [`BeaconGeometry`] a `b + 2d` it has
    /// no business computing, and nothing downstream could tell.
    #[test]
    fn a_poles_supply_area_distance_is_not_a_beacons() {
        let state = state_with_supply_area("small-electric-pole", Some(2.5));
        assert_eq!(
            beacon_supply_area_distance(&state, "small-electric-pole"),
            None,
            "an electric-pole's number means half a side, not distance beyond a footprint"
        );
    }

    /// A beacon that declares nothing answers **unknown**, and there is
    /// deliberately no vanilla beacon table to fall back on.
    ///
    /// The pole side keeps one because the archived maps' power model would go
    /// blind without it; nothing consumes a beacon's `d` yet, so honesty is
    /// free here and is taken.
    #[test]
    fn a_beacon_prototype_without_the_field_stays_unknown() {
        let state = state_with_supply_area(BEACON, None);
        assert_eq!(beacon_supply_area_distance(&state, BEACON), None);
        assert_eq!(
            beacon_geometry(&state, BEACON)
                .expect("the fixture ships a beacon")
                .max_row_separation_tiles(),
            None,
            "unknown must not degrade into a vanilla 9"
        );
    }

    /// A world with no beacon prototype gets no geometry, rather than a
    /// vanilla one invented for it.
    #[test]
    fn a_world_with_no_beacon_prototype_refuses() {
        let world = fixture_world();
        world.globals.entity_prototypes.remove(BEACON);
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(beacon_geometry(&state, BEACON).is_none());
    }
}
