//! Helpers shared by more than one method.

use crate::error::PlannerError;
use crate::ids::Ticks;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::ToPrimitive;
use factorio_bot_core::types::{FactorioRecipe, FactorioTechnology, Position, ResearchTrigger};

const TICKS_PER_SECOND: f64 = 60.0;

/// How far out `free_area_near` will search before giving up, in tiles.
const FREE_TILE_SEARCH_RADIUS: i32 = 12;

/// Convert a recipe's or prototype's seconds into ticks, rounding up so that a
/// positive duration never becomes zero.
pub fn seconds_to_ticks(seconds: f64) -> Ticks {
    if seconds <= 0.0 {
        return 0;
    }
    (seconds * TICKS_PER_SECOND).ceil() as Ticks
}

pub fn recipe_for(state: &PlanState, item: &str) -> Option<FactorioRecipe> {
    state.base().recipes.get(item).map(|r| r.clone())
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
        .entity_prototypes
        .get(item)
        .and_then(|p| p.mining_time)
        .unwrap_or(1.0);
    seconds_to_ticks(seconds / character_mining_speed(state))
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
pub fn nearest_resource_tile(
    state: &PlanState,
    item: &str,
    from: &Position,
    need: u32,
) -> Option<Position> {
    let mut best: Option<(f64, Position)> = None;
    for patch in state.resource_patches(item) {
        for tile in patch.elements {
            if state.resource_unclaimed(&tile, item) < need {
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
    for patch in state.resource_patches(item) {
        for tile in patch.elements {
            let available = state.resource_unclaimed(&tile, item);
            if available == 0 {
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
    for patch in state.resource_patches(item) {
        for tile in patch.elements {
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
    let mut tiles: Vec<Position> = state
        .resource_patches(item)
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
pub fn free_area_near_where(
    state: &PlanState,
    from: &Position,
    entity: &str,
    accept: impl Fn(&Position) -> bool,
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
                if state.is_area_free(entity, &candidate) && accept(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
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
    let Some(prototype) = state.base().entity_prototypes.get(entity) else {
        return (0., 0.);
    };
    let axis = |extent: f64| {
        // `ceil` and not `round`: an entity 2.3984 tiles across occupies three
        // tiles, not two. The tiny epsilon keeps a box that is exactly `n`
        // tiles wide -- which the binary-fraction prototype numbers really can
        // be -- from ceiling to `n + 1` on float noise.
        let tiles = (extent - 1. / 512.).ceil() as i64;
        if tiles.rem_euclid(2) == 0 { 0. } else { 0.5 }
    };
    let box_ = &prototype.collision_box;
    (axis(box_.width()), axis(box_.height()))
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

/// What a `research_trigger` technology actually costs, as the items its
/// trigger requires be crafted.
///
/// `Ok(None)` is the ordinary pack-researched technology, whose bill
/// `research_ingredients` already describes. `Ok(Some((item, count)))` is a
/// trigger this planner can express: the technology completes when `count` of
/// `item` have been crafted, which is an ordinary `Goal::Have`.
///
/// Everything else is an error, deliberately. A trigger technology has an empty
/// pack bill and zero research time, so the alternative to refusing is planning
/// it as free — which is precisely the defect this function exists to fix, and
/// which is invisible in the resulting plan. Both error variants name the
/// technology, so a caller learns which step is not modelled rather than
/// receiving a makespan that is quietly too small.
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
) -> Result<Option<(String, u32)>, PlannerError> {
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
            Ok(Some((item.clone(), *count)))
        }
        other => Err(PlannerError::UnsupportedResearchTrigger {
            technology: tech.name.clone(),
            trigger: other.kind().to_string(),
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
    let ticks = tech.research_unit_energy.to_f64().unwrap_or(0.0)
        * tech.research_unit_count.to_f64().unwrap_or(0.0);
    if ticks <= 0.0 {
        return 0;
    }
    if ticks >= f64::from(Ticks::MAX) {
        return Ticks::MAX;
    }
    ticks.ceil() as Ticks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::{ClaimRunner, DEFAULT_RESOURCE_PER_TILE, PlanState};
    use factorio_bot_core::serde_json;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{Direction, FactorioEntity, FactorioForce, Position};
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
                    .entity_prototypes
                    .get("character")
                    .expect("the fixture ships a character prototype")
                    .clone();
                character.mining_speed = Some(speed);
                world
                    .entity_prototypes
                    .insert("character".into(), character);
            }
            None => {
                world.entity_prototypes.remove("character");
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

    #[test]
    fn a_free_tile_near_ore_is_not_on_the_ore() {
        // Siting a furnace by an ore patch starts the ring search on the ore
        // tile itself. A tile carrying ore is not placeable in the game, so the
        // search has to step off the patch rather than return where it started.
        let s = state();
        let ore = nearest_resource_tile(&s, "iron-ore", &Position::new(0., 0.), 1)
            .expect("fixture has iron ore");
        let tile =
            free_area_near(&s, &ore, "stone-furnace").expect("open ground next to the patch");
        assert_ne!(tile, ore, "the furnace was sited on the ore tile itself");
        assert_eq!(
            s.resource_available(&tile, "iron-ore"),
            0,
            "the chosen tile {:?} still holds ore",
            tile
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
                    .entity_prototypes
                    .get(machine)
                    .unwrap_or_else(|| panic!("the fixture ships a {machine} prototype"))
                    .clone();
                prototype.crafting_speed = Some(speed);
                world.entity_prototypes.insert(machine.into(), prototype);
            }
            None => {
                world.entity_prototypes.remove(machine);
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
}
