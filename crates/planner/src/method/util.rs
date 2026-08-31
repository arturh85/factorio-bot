//! Helpers shared by more than one method.

use crate::ids::Ticks;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::ToPrimitive;
use factorio_bot_core::types::{FactorioRecipe, FactorioTechnology, Position};

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

/// Ticks to mine one unit of `item`, from its entity prototype. Defaults to one
/// second when the prototype carries no mining time.
pub fn mining_ticks(state: &PlanState, item: &str) -> Ticks {
    let seconds = state
        .base()
        .entity_prototypes
        .get(item)
        .and_then(|p| p.mining_time)
        .unwrap_or(1.0);
    seconds_to_ticks(seconds)
}

/// The tile of `item` nearest `from` that still holds at least `need`.
///
/// Ties on distance are broken by `(x, y)`, so the result depends only on the
/// tile set and the origin — never on the order `resource_patches` happens to
/// return patches in, which is not stable across processes for patches of
/// equal size.
pub fn nearest_resource_tile(
    state: &PlanState,
    item: &str,
    from: &Position,
    need: u32,
) -> Option<Position> {
    let mut best: Option<(f64, Position)> = None;
    for patch in state.resource_patches(item) {
        for tile in patch.elements {
            if state.resource_available(&tile, item) < need {
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
/// from each. Empty when the patches cannot supply `need` in total.
///
/// Ties on distance break on `(x, y)`, like `nearest_resource_tile`, so the
/// result depends only on the tile set and the origin.
pub fn resource_tiles_for(
    state: &PlanState,
    item: &str,
    from: &Position,
    need: u32,
) -> Vec<(Position, u32)> {
    let mut candidates: Vec<(f64, Position, u32)> = Vec::new();
    for patch in state.resource_patches(item) {
        for tile in patch.elements {
            let available = state.resource_available(&tile, item);
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

    let mut out = Vec::new();
    let mut remaining = need;
    for (_, tile, available) in candidates {
        if remaining == 0 {
            break;
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

/// Can the map's remaining tiles of `item` supply `need` in total?
///
/// The same question `!resource_tiles_for(..).is_empty()` answers, without
/// building the answer: applicability asks only whether enough exists
/// anywhere, never which tiles are nearest, so there is nothing to collect,
/// nothing to sort and no origin to measure from. Stops at the first tile that
/// brings the running total up to `need`.
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
            total = total.saturating_add(state.resource_available(&tile, item));
            if total >= need {
                return true;
            }
        }
    }
    false
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
/// Candidates stay on the integer grid the tile search has always used, which
/// is where Factorio wants an even-sized entity like a furnace; the *test* is
/// `is_area_free`, which is exact.
pub fn free_area_near(state: &PlanState, from: &Position, entity: &str) -> Option<Position> {
    let base_x = from.x.floor() as i32;
    let base_y = from.y.floor() as i32;
    for radius in 0..=FREE_TILE_SEARCH_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // Only the ring at exactly this radius; inner ones were done.
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let candidate = Position::new((base_x + dx) as f64, (base_y + dy) as f64);
                if state.is_area_free(entity, &candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
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

/// How many of `item` one execution of `recipe` yields. Defaults to 1.
pub fn output_per_craft(recipe: &FactorioRecipe, item: &str) -> u32 {
    recipe
        .products
        .iter()
        .find(|p| p.name == item)
        .map(|p| p.amount.max(1))
        .unwrap_or(1)
}

/// A recipe's energy in ticks.
pub fn recipe_ticks(recipe: &FactorioRecipe) -> Ticks {
    seconds_to_ticks(recipe.energy.to_f64().unwrap_or(0.5))
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
    /// Craftable as things stand — either the force already has it, or the
    /// technology that unlocks it is researched (possibly by this very plan).
    Open,
    /// Craftable only after this technology is researched.
    NeedsResearch(String),
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
        // `is_researched` reads the overlay as well as the world, so a
        // technology an earlier sibling of this expansion already researched
        // leaves the recipe Open and costs no second subgoal. That is what
        // keeps the common case free: `Researched` emits its prerequisites
        // before its science packs, and the technology unlocking a pack's
        // recipe is normally one of those prerequisites.
        Some(tech) if state.is_researched(&tech) => RecipeGate::Open,
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
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
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
    fn mining_a_fixture_ore_takes_one_second() {
        let s = state();
        assert_eq!(mining_ticks(&s, "iron-ore"), 60);
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
        // would give 60 instead.
        assert_eq!(mining_ticks(&s, "stone-furnace"), 12);
        // An item with no prototype at all falls back to one second.
        assert_eq!(mining_ticks(&s, "not-a-real-entity"), 60);
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
}
