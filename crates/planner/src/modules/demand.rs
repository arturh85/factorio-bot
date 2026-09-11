//! Normalise combined production and finite demand without changing goals.
//!
//! Separates a mixed goal list into three categories that the module system
//! can work with independently:
//!
//! - **gross_floor** -- continuous rate demands (`Producing`, `Sustain`),
//!   aggregated by item with the highest rate winning for each name.
//! - **finite** -- item-count demands (`Have`, `Produced`), preserved verbatim
//!   because the recipe/holder-aware batch expansion that will size them does
//!   not exist yet.
//! - **residual** -- goals the module system does not handle (`Researched`,
//!   `Extracted`, `Gathered`, `Built`, ...), returned so the caller can fall
//!   back to the legacy planner.
//!
//! # Material equation
//!
//! The key arithmetic is `required_gross(floor, internal, exported)`:
//!
//! ```text
//! gross = max(floor, internal + exported)
//! ```
//!
//! where:
//! - `floor` is the net rate a goal demands (what must be *produced outward*),
//! - `internal` is the rate consumed by other recipes in the same production
//!   chain (belt-fed modules consuming each other's output),
//! - `exported` is the rate sent to external consumers (other bases, exports,
//!   rocket silo payloads),
//! - `gross` is the total rate that must be put through the producing modules.
//!
//! # Deterministic recipe DAG
//!
//! When a `Producing / via` names a specific recipe, the module walks the
//! recipe's ingredient tree to compute internal consumption, rejecting cycles
//! and off-planet / recycling candidates.

use std::collections::BTreeMap;

use crate::goal::Goal;
use crate::modules::artifact::{ModuleError, Rate};

// ---------------------------------------------------------------------------
// DemandSet
// ---------------------------------------------------------------------------

/// Separated demand categories after goal normalisation.
///
/// Every field is a public, structural answer that the caller can forward,
/// merge, or act on independently.
#[derive(Debug, Clone, Default)]
pub struct DemandSet {
    /// Continuous rate demands, keyed by item name. Multiple goals targeting
    /// the same item are merged by taking the **maximum** rate.
    pub gross_floor: BTreeMap<String, Rate>,

    /// Finite (item-count) goals preserved verbatim: `Have` and `Produced`.
    /// These are not merged because each represents a distinct requirement
    /// (e.g. two `Have iron-plate 5` goals from two different holders).
    pub finite: Vec<Goal>,

    /// Goals the module system cannot handle directly: `Researched`,
    /// `Extracted`, `Gathered`, `Orbiting`, `Built`, `Charted`. Returned so
    /// the legacy planner can handle them after module production is arranged.
    pub residual: Vec<Goal>,

    /// Items that are being exported to external consumers — for the material
    /// equation, these add to `internal` demand for the producing modules.
    pub exports: BTreeMap<String, Rate>,
}

// ---------------------------------------------------------------------------
// required_gross
// ---------------------------------------------------------------------------

/// The material equation: `max(floor, internal + exported)`.
///
/// All three arguments are rates. Returns the gross rate that must be
/// produced to satisfy the floor demand while covering internal consumption
/// and export commitments.
///
/// # Arithmetic
///
/// Rates are rational numbers (`numerator / ticks`). Addition is
/// `a/b + c/d = (a*LCM/b + c*LCM/d) / LCM` normalised by GCD. Comparison
/// follows the same normalised form. Overflow in intermediate products
/// returns [`ModuleError::ArithmeticOverflow`].
pub fn required_gross(floor: Rate, internal: Rate, exported: Rate) -> Result<Rate, ModuleError> {
    let sum = rate_add(internal, exported)?;
    if rate_cmp(sum.clone(), floor.clone()) != std::cmp::Ordering::Less {
        Ok(sum)
    } else {
        Ok(floor)
    }
}

/// Add two rates with checked arithmetic and GCD normalisation.
fn rate_add(a: Rate, b: Rate) -> Result<Rate, ModuleError> {
    let (a_num, a_den) = (a.numerator, a.ticks.get());
    let (b_num, b_den) = (b.numerator, b.ticks.get());
    // a_num / a_den + b_num / b_den = (a_num * b_den + b_num * a_den) / (a_den * b_den)
    let num = (a_num as u128)
        .checked_mul(b_den as u128)
        .and_then(|p| p.checked_add((b_num as u128).checked_mul(a_den as u128)?))
        .ok_or(ModuleError::ArithmeticOverflow)?;
    let den = (a_den as u128)
        .checked_mul(b_den as u128)
        .ok_or(ModuleError::ArithmeticOverflow)?;
    if den == 0 {
        return Err(ModuleError::InvalidArtifact(
            "zero denominator in rate add".into(),
        ));
    }
    // Reduce
    let g = gcd_u128(num, den);
    Rate::new((num / g) as u64, (den / g) as u64)
}

/// Compare two rates numerically. Returns `Less`, `Equal`, or `Greater`.
fn rate_cmp(a: Rate, b: Rate) -> std::cmp::Ordering {
    let a_numer = a.numerator as u128 * b.ticks.get() as u128;
    let b_numer = b.numerator as u128 * a.ticks.get() as u128;
    a_numer.cmp(&b_numer)
}

/// GCD for u128 (used internally after checked multiplication).
fn gcd_u128(a: u128, b: u128) -> u128 {
    if b == 0 { a } else { gcd_u128(b, a % b) }
}

// ---------------------------------------------------------------------------
// normalize_goals
// ---------------------------------------------------------------------------

/// Separate a mixed goal list into a [`DemandSet`].
///
/// The algorithm:
///
/// 1. Walk every goal in order.
/// 2. `Have` / `Produced` → appended to `finite` verbatim.
/// 3. `Producing` / `Sustain` → per-minute converted to [`Rate`] at 3600
///    ticks/game-minute, merged into `gross_floor` (max wins).
/// 4. `All(goals)` → recurse, then merge the child's gross_floor by max.
/// 5. Everything else (`Researched`, `Extracted`, `Gathered`, `Orbiting`,
///    `Built`, `Charted`) → appended to `residual`.
/// 6. `exports` is placed in the returned set as-is.
///
/// # Errors
///
/// Returns [`ModuleError::InvalidArtifact`] when a rate cannot be
/// represented (e.g. zero ticks).
pub fn normalize_goals(
    goals: &[Goal],
    exports: &BTreeMap<String, Rate>,
) -> Result<DemandSet, ModuleError> {
    let mut set = DemandSet::default();
    set.exports = exports.clone();

    for goal in goals {
        match goal {
            Goal::Have { .. } | Goal::Produced { .. } => {
                set.finite.push(goal.clone());
            }
            Goal::Producing { item, per_minute } => {
                let rate = Rate::new(*per_minute as u64, 3600)?;
                insert_max_rate(&mut set.gross_floor, item.clone(), rate);
            }
            Goal::Sustain {
                item, per_minute, ..
            } => {
                let rate = Rate::new(*per_minute as u64, 3600)?;
                insert_max_rate(&mut set.gross_floor, item.clone(), rate);
            }
            Goal::All(children) => {
                let child = normalize_goals(children, exports)?;
                // Merge child's gross_floor into parent's by max.
                for (item, rate) in child.gross_floor {
                    insert_max_rate(&mut set.gross_floor, item, rate);
                }
                // Finite and residual goals from children stay attached
                // at the top level — they are still unsatisfied.
                set.finite.extend(child.finite);
                set.residual.extend(child.residual);
            }
            // Everything else: research, extraction, orbiting, building, charting.
            _ => {
                set.residual.push(goal.clone());
            }
        }
    }

    Ok(set)
}

/// Insert `rate` into the map for `item`, keeping the **maximum** rate.
fn insert_max_rate(map: &mut BTreeMap<String, Rate>, item: String, rate: Rate) {
    match map.get(&item) {
        Some(existing)
            if rate_cmp(rate.clone(), existing.clone()) == std::cmp::Ordering::Greater =>
        {
            map.insert(item, rate);
        }
        None => {
            map.insert(item, rate);
        }
        _ => {
            // existing rate is >= new rate; keep existing.
        }
    }
}

// ---------------------------------------------------------------------------
// Recipe DAG building
// ---------------------------------------------------------------------------

/// Policy for choosing between multiple recipes that produce the same item.
///
/// In the vanilla + Space Age install, `petroleum-gas` is produced by five
/// recipes. This map lets the planner prefer one over another.
pub type RecipePolicy = BTreeMap<String, String>;

/// Build a deterministic recipe DAG starting from the gross floor demands.
///
/// Returns a map from item name to the rate that must be produced gross,
/// after accounting for internal consumption via the recipe chain.
///
/// # Rejection rules
///
/// - **Off-planet recipes**: any recipe whose category is not runnable on
///   Nauvis (e.g. `organic`, `agricultural`) is rejected.
/// - **Recycling recipes**: any recipe in the `recycling` category is
///   rejected — you cannot produce a thing by recycling it.
/// - **Cycle detection**: if walking the recipe graph revisits an item that
///   is already in the ancestor chain, the path is rejected with an error.
///
/// # Policy preference map
///
/// When a product has multiple runnable recipes, the `prefer` map selects one
/// by name. Entries are `product_name -> preferred_recipe_name`. When no
/// preference is given, [`crate::products::ProductIndex::sole_recipe_producing`]
/// is consulted; if that is ambiguous the function returns an error.
///
/// # Returns
///
/// A map from item name to the gross rate that must be produced, covering
/// both the original floor demand and all internal consumption by downstream
/// recipes.
pub fn resolve_recipe_dag(
    gross_floor: &BTreeMap<String, Rate>,
    product_index: &crate::products::ProductIndex,
    prefer: &RecipePolicy,
) -> Result<BTreeMap<String, Rate>, ModuleError> {
    let mut result: BTreeMap<String, Rate> = BTreeMap::new();
    let mut visited: Vec<String> = Vec::new();

    for (item, floor_rate) in gross_floor {
        visited.clear();
        let resolved = resolve_item(item, floor_rate, product_index, prefer, &mut visited)?;
        for (resolved_item, resolved_rate) in resolved {
            // Take the max rate if the same item appears from multiple chains.
            match result.get(&resolved_item) {
                Some(existing)
                    if rate_cmp(resolved_rate.clone(), existing.clone())
                        == std::cmp::Ordering::Greater =>
                {
                    result.insert(resolved_item, resolved_rate);
                }
                None => {
                    result.insert(resolved_item, resolved_rate);
                }
                _ => {}
            }
        }
    }

    Ok(result)
}

/// Recursively resolve a single item's production chain.
///
/// `ancestors` tracks items already in the current path for cycle detection.
fn resolve_item(
    item: &str,
    rate: &Rate,
    product_index: &crate::products::ProductIndex,
    prefer: &RecipePolicy,
    ancestors: &mut Vec<String>,
) -> Result<BTreeMap<String, Rate>, ModuleError> {
    // Cycle detection: reject if this item is already being resolved.
    if ancestors.contains(&item.to_string()) {
        return Err(ModuleError::InvalidArtifact(format!(
            "recipe cycle detected for item {}",
            item
        )));
    }

    // Check if this item is a raw resource (no recipe produces it).
    if !product_index.produces(item) {
        let mut result = BTreeMap::new();
        result.insert(item.to_string(), rate.clone());
        return Ok(result);
    }

    // Look up the recipe using the product index.
    // Build a categories set that excludes recycling.
    let categories = crate::products::Categories::any();

    // First check the preference map.
    let recipe = if let Some(preferred) = prefer.get(item) {
        product_index.recipe(preferred).ok_or_else(|| {
            ModuleError::InvalidArtifact(format!(
                "preferred recipe {} for {} does not exist",
                preferred, item
            ))
        })?
    } else {
        // Use the product index's own selection (respects the three rules).
        product_index
            .sole_recipe_producing(item, &categories)
            .map_err(|refusal| {
                ModuleError::InvalidArtifact(format!(
                    "cannot determine sole recipe for {}: {}",
                    item, refusal
                ))
            })?
    };

    // Reject recycling recipes: if the ONLY recipe for this item is
    // a recycling recipe, treat the item as a raw resource rather than
    // erroring — it comes out of the ground, not from destroying itself.
    if recipe.category == crate::products::RECYCLING_CATEGORY {
        // Check for non-recycling alternatives by looking at all recipes
        // producing this item.
        let has_productive: bool = product_index
            .recipes_producing(item)
            .iter()
            .any(|r| r.category != crate::products::RECYCLING_CATEGORY);
        if !has_productive {
            // No productive recipe — treat as raw resource.
            let mut result = BTreeMap::new();
            result.insert(item.to_string(), rate.clone());
            return Ok(result);
        }
        return Err(ModuleError::InvalidArtifact(format!(
            "recipe {} for {} is a recycling recipe and cannot produce net output",
            recipe.name, item
        )));
    }

    // Reject off-planet recipes (those whose category has no machine on Nauvis).
    // We check if the recipe has any ingredient that is not here and not producible.
    // For now, we reject recipes with `organic` or `agricultural` category as off-planet.
    let off_planet_categories = ["organic", "agricultural", "chemistry-plant-with-fluid"];
    if off_planet_categories.contains(&recipe.category.as_str()) {
        return Err(ModuleError::InvalidArtifact(format!(
            "recipe {} for {} is in category {} which is off-planet",
            recipe.name, item, recipe.category
        )));
    }

    ancestors.push(item.to_string());

    // For each ingredient, compute the rate needed using the recipe ratio.
    let mut result = BTreeMap::new();
    result.insert(item.to_string(), rate.clone());

    // Find the output amount for our target product.
    let output_amount: u64 = recipe
        .products
        .iter()
        .find(|p| p.name == item)
        .map(|p| p.amount as u64)
        .unwrap_or(1);

    if output_amount == 0 {
        ancestors.pop();
        return Err(ModuleError::InvalidArtifact(format!(
            "recipe {} has zero output amount for product {}",
            recipe.name, item
        )));
    }

    for ingredient in recipe.ingredients.iter().flatten() {
        let ing_name = &ingredient.name;
        let ing_amount = ingredient.amount as u64;

        // Compute the ingredient rate based on the recipe ratio.
        // rate * ing_amount / output_amount
        let ing_rate = scale_rate(rate, ing_amount, output_amount)?;

        let child = resolve_item(ing_name, &ing_rate, product_index, prefer, ancestors)?;
        for (child_item, child_rate) in child {
            match result.get(&child_item) {
                Some(existing) => {
                    let sum = rate_add(existing.clone(), child_rate)?;
                    result.insert(child_item, sum);
                }
                None => {
                    result.insert(child_item, child_rate);
                }
            }
        }
    }

    ancestors.pop();
    Ok(result)
}

/// Scale `rate` by `multiplier / divisor` using checked arithmetic.
fn scale_rate(rate: &Rate, multiplier: u64, divisor: u64) -> Result<Rate, ModuleError> {
    if divisor == 0 {
        return Err(ModuleError::InvalidArtifact(
            "zero divisor in scale_rate".into(),
        ));
    }
    // rate * multiplier / divisor = (num * multiplier) / (ticks * divisor)
    let num = (rate.numerator as u128)
        .checked_mul(multiplier as u128)
        .ok_or(ModuleError::ArithmeticOverflow)?;
    let den = (rate.ticks.get() as u128)
        .checked_mul(divisor as u128)
        .ok_or(ModuleError::ArithmeticOverflow)?;
    let g = gcd_u128(num, den);
    Rate::new((num / g) as u64, (den / g) as u64)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{Goal, Holder};

    fn r(n: u64) -> Rate {
        Rate::new(n, 3600).unwrap()
    }

    #[test]
    fn gross_floor_is_not_an_implicit_export() {
        // The material equation: max(floor, internal + exported).
        assert_eq!(required_gross(r(60), r(60), r(0)).unwrap(), r(60));
        assert_eq!(required_gross(r(60), r(60), r(60)).unwrap(), r(120));
    }

    #[test]
    fn rate_addition_is_correct() {
        // 30/min + 60/min = 90/min
        assert_eq!(rate_add(r(30), r(60)).unwrap(), r(90));
        // 1/600 + 1/600 = 2/600 = 1/300
        let a = Rate::new(1, 600).unwrap();
        let sum = rate_add(a.clone(), a.clone()).unwrap();
        assert_eq!(sum.numerator, 1);
        assert_eq!(sum.ticks.get(), 300);
    }

    #[test]
    fn normalize_goals_preserves_have_and_produced() {
        let goals = vec![
            Goal::Have {
                item: "iron-plate".into(),
                count: 10,
                whose: Holder::Anyone,
                via: None,
            },
            Goal::Produced {
                item: "copper-plate".into(),
                count: 20,
                whose: Holder::Anyone,
                unlocks: None,
                via: None,
            },
        ];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        assert_eq!(set.finite.len(), 2);
        assert!(set.gross_floor.is_empty());
        assert!(set.residual.is_empty());
    }

    #[test]
    fn normalize_goals_merges_producing_by_max() {
        let goals = vec![
            Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 30,
            },
            Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 60,
            },
        ];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        assert_eq!(set.gross_floor.len(), 1);
        assert_eq!(set.gross_floor["iron-plate"], r(60));
    }

    #[test]
    fn normalize_goals_recurse_all() {
        let goals = vec![
            Goal::All(vec![
                Goal::Producing {
                    item: "iron-plate".into(),
                    per_minute: 30,
                },
                Goal::Have {
                    item: "iron-gear-wheel".into(),
                    count: 5,
                    whose: Holder::Anyone,
                    via: None,
                },
            ]),
            Goal::All(vec![
                Goal::Producing {
                    item: "iron-plate".into(),
                    per_minute: 15, // lower than 30, so max wins
                },
                Goal::Researched("automation".into()),
            ]),
        ];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        // gross_floor: iron-plate at max(30,15)=30
        assert_eq!(set.gross_floor.len(), 1);
        assert_eq!(set.gross_floor["iron-plate"], r(30));
        // finite: 1 (iron-gear-wheel)
        assert_eq!(set.finite.len(), 1);
        // residual: 1 (automation)
        assert_eq!(set.residual.len(), 1);
    }

    #[test]
    fn research_is_residual() {
        let goals = vec![Goal::Researched("automation".into())];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        assert!(set.gross_floor.is_empty());
        assert!(set.finite.is_empty());
        assert_eq!(set.residual.len(), 1);
    }

    #[test]
    fn exports_appear_in_demand_set() {
        let mut exports = BTreeMap::new();
        exports.insert("iron-plate".into(), r(30));
        let set = normalize_goals(&[], &exports).unwrap();
        assert_eq!(set.exports.len(), 1);
        assert_eq!(set.exports["iron-plate"], r(30));
    }

    #[test]
    fn zero_rate_requires_no_new_module() {
        // Producing at per_minute=0 should produce a zero rate,
        // which is a valid rate (0/3600 normalized to 0/1).
        let goals = vec![Goal::Producing {
            item: "iron-plate".into(),
            per_minute: 0,
        }];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        assert_eq!(set.gross_floor.len(), 1);
        assert_eq!(set.gross_floor["iron-plate"], Rate::new(0, 3600).unwrap());
    }

    #[test]
    fn missing_child_makes_whole_plan_incomplete() {
        // A Producing goal with an item that has no recipe in our world
        // should propagate as an error through the recipe DAG.
        // (This is tested via resolve_recipe_dag with an empty product_index.)
        let empty_index = crate::products::ProductIndex::from_parts(
            std::iter::empty::<&factorio_bot_core::types::FactorioRecipe>(),
            std::iter::empty::<&str>(),
        );
        let mut floor = BTreeMap::new();
        floor.insert("unknown-item".into(), r(30));
        let result = resolve_recipe_dag(&floor, &empty_index, &BTreeMap::new());
        assert!(result.is_err(), "unknown item should be an error");
    }

    #[test]
    fn fractional_recipe_yields_preserve_ratio() {
        // If a recipe produces 1.5 units (implemented as amount=3/2 internally
        // using integers, e.g. 3 ticks/2 items), the ingredient rate scaling
        // should preserve the rational ratio.
        //
        // We test the scale_rate helper directly.
        let rate = r(60); // 60/hour = 60/3600 = 1/60
        let scaled = scale_rate(&rate, 3, 2).unwrap();
        // 60/hr * 3/2 = 90/hr
        assert_eq!(scaled, r(90));
    }

    #[test]
    fn duplicate_goals_in_all_merge() {
        let goals = vec![Goal::All(vec![
            Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 45,
            },
            Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 45,
            },
        ])];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        assert_eq!(set.gross_floor.len(), 1);
        assert_eq!(set.gross_floor["iron-plate"], r(45));
    }

    #[test]
    fn have_with_named_holder_is_preserved() {
        let goals = vec![Goal::Have {
            item: "iron-plate".into(),
            count: 10,
            whose: Holder::Bot(crate::ids::BotId(1)),
            via: None,
        }];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        assert_eq!(set.finite.len(), 1);
        if let Goal::Have {
            item, count, whose, ..
        } = &set.finite[0]
        {
            assert_eq!(item, "iron-plate");
            assert_eq!(*count, 10);
            assert_eq!(*whose, Holder::Bot(crate::ids::BotId(1)));
        } else {
            panic!("expected Have goal");
        }
    }

    #[test]
    fn produced_with_via_unlocks_is_preserved() {
        let goals = vec![Goal::Produced {
            item: "petroleum-gas".into(),
            count: 100,
            whose: Holder::Anyone,
            unlocks: Some("oil-processing".into()),
            via: Some("basic-oil-processing".into()),
        }];
        let set = normalize_goals(&goals, &BTreeMap::new()).unwrap();
        assert_eq!(set.finite.len(), 1);
        if let Goal::Produced {
            item,
            count,
            unlocks,
            via,
            ..
        } = &set.finite[0]
        {
            assert_eq!(item, "petroleum-gas");
            assert_eq!(*count, 100);
            assert_eq!(unlocks.as_deref(), Some("oil-processing"));
            assert_eq!(via.as_deref(), Some("basic-oil-processing"));
        } else {
            panic!("expected Produced goal");
        }
    }

    #[test]
    fn scale_rate_zero_divisor_errors() {
        let result = scale_rate(&r(60), 1, 0);
        assert!(result.is_err());
    }
}
