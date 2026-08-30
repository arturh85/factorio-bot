//! Helpers shared by more than one method.

use crate::ids::Ticks;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::ToPrimitive;
use factorio_bot_core::types::{FactorioRecipe, Position};

const TICKS_PER_SECOND: f64 = 60.0;

/// How far out `free_tile_near` will search before giving up, in tiles.
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

/// The nearest unoccupied tile to `from`, searched in rings so the result is
/// close and reproducible.
pub fn free_tile_near(state: &PlanState, from: &Position) -> Option<Position> {
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
                if state.is_position_free(&candidate) {
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
        // The iron field sits around x -45..-35, y 35..45.
        assert!(
            tile.x <= -35.0 && tile.x >= -45.0,
            "unexpected x: {}",
            tile.x
        );
        assert!(tile.y >= 35.0 && tile.y <= 45.0, "unexpected y: {}", tile.y);
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
    fn a_missing_resource_has_no_tile() {
        let s = state();
        assert!(nearest_resource_tile(&s, "uranium-ore", &Position::new(0., 0.), 1).is_none());
    }

    #[test]
    fn a_free_tile_is_found_and_is_actually_free() {
        let s = state();
        let pos = free_tile_near(&s, &Position::new(0., 0.)).expect("origin area is open");
        assert!(s.is_position_free(&pos));
    }

    #[test]
    fn a_free_tile_avoids_an_occupied_one() {
        let mut s = state();
        let origin = Position::new(0., 0.);
        let first = free_tile_near(&s, &origin).unwrap();
        let furnace = factorio_bot_core::types::FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: first.clone(),
            ..Default::default()
        };
        s.create_entity(furnace);
        let second = free_tile_near(&s, &origin).unwrap();
        assert_ne!(first, second);
        assert!(s.is_position_free(&second));
    }

    #[test]
    fn mining_a_fixture_ore_takes_one_second() {
        let s = state();
        assert_eq!(mining_ticks(&s, "iron-ore"), 60);
    }

    #[test]
    fn an_exact_distance_tie_breaks_on_the_lower_position() {
        let s = state();
        // Exactly halfway between the ore tiles at x = -41 and x = -40 on row
        // y = 40. Both are equidistant, so the lower (x, y) must win regardless of
        // which patch was visited first.
        let origin = Position::new(-40.5, 40.0);
        let tile = nearest_resource_tile(&s, "iron-ore", &origin, 1).expect("iron ore");
        assert_eq!(tile, Position::new(-41.0, 40.0));
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
        let found = free_tile_near(&s, &origin).expect("ring 2 is open");
        assert!(s.is_position_free(&found));
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
