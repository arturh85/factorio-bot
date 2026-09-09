//! Phase 0 infrastructure: direct-insertion burner-mining-drill + stone-furnace
//! pairs.
//!
//! Runs BEFORE any goal is expanded. Checks how many drills and furnaces already
//! stand on each smeltable item's ore patch, and if the plan needs plates but
//! lacks enough pairs, builds them.
//!
//! Each pair is a burner-mining-drill on ore with a stone-furnace two tiles
//! behind it -- the same stage-1 cell [`crate::method::produce`] builds, placed
//! outside of any rate goal so that every later method sees finished pairs.

use crate::error::PlannerError;
use crate::goal::Goal;
use crate::ids::{ItemId, Ticks};
use crate::method::produce;
use crate::method::{ExpansionCtx, Step};
use std::collections::BTreeMap;

/// How many pairs to build per item below the threshold.
const MIN_PAIRS: u32 = 8;

/// How many ticks to fuel Phase 0 pairs (10 minutes, matching
/// [`produce::CELL_FUELLED_TICKS`] which is private to that module).
const PHASE0_FUELLED_TICKS: Ticks = 72_000;

/// Items a Phase 0 cell can produce.
const SMELTABLE_ITEMS: &[&str] = &["iron-plate", "copper-plate", "stone-brick"];

/// Minimum plate demand needed to trigger Phase 0 (in plates).
/// Phase 0 only fires for substantial plate counts (>500).
/// Below this threshold, hand-mining and hand-smelting is more efficient.
const MIN_DEMAND_THRESHOLD: u32 = 200;

/// Survey aggregate plate demand and build direct-insertion pairs if worth it.
///
/// Runs BEFORE any goal is expanded. Checks:
/// - How many iron/copper/stone plates the whole plan needs
/// - How many drills+furnaces already stand or are planned
/// - If deficit > threshold, place N pairs on the nearest ore patch
pub fn ensure_infrastructure(
    goals: &[Goal],
    ctx: &mut ExpansionCtx,
) -> Result<Vec<Step>, PlannerError> {
    let demand = aggregate_plate_demand(goals);
    if demand.is_empty() {
        return Ok(Vec::new());
    }

    // Where the chain actor stands, for siting cells near it.
    let from = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.position.clone())
        .unwrap_or_default();

    let mut all_steps: Vec<Step> = Vec::new();

    for &item in SMELTABLE_ITEMS {
        if !demand.contains_key(item) {
            continue;
        }
        // Skip if the bots already hold enough of this item to meet demand
        let needed = *demand.get(item).unwrap_or(&0);
        let held = ctx.state.total_count(&item);
        if held >= needed {
            continue;
        }
        // Also skip if any infrastructure already stands
        if !ctx.state.entities_named("burner-mining-drill").is_empty() {
            continue;
        }

        // Can a cell make this item? (needs a smelting recipe from 1 ore)
        let spec = match produce::cell_spec(&ctx.state, item) {
            Some(s) => s,
            None => continue,
        };

        // How many of those cells already stand?
        let standing = produce::cells_standing(&ctx.state, &spec);
        if standing >= MIN_PAIRS {
            continue;
        }

        let build = MIN_PAIRS.saturating_sub(standing);

        // Try to site the cells. If the patch has no room or no patch exists,
        // skip Phase 0 for this item -- the normal goal machinery will still
        // hand-mine and hand-smelt.
        let want = produce::rate_cell_ore(&spec);
        let cells = match produce::plan_cells(&ctx.state, &from, &spec, build, want) {
            Ok(c) => c,
            Err(PlannerError::NoRoomForCell { .. } | PlannerError::NoPatchForCell { .. }) => {
                continue;
            }
            Err(e) => return Err(e),
        };

        // Generate the placement and fueling steps for these cells.
        let steps = produce::cell_steps_fuelled(ctx, &spec, &cells, PHASE0_FUELLED_TICKS);
        all_steps.extend(steps);
    }

    Ok(all_steps)
}

/// Collect total plate-like demand from a goal tree.
///
/// Only [`Goal::Have`] and [`Goal::Produced`] with significant counts trigger
/// Phase 0. [`Goal::Producing`] and [`Goal::Sustain`] already have their own
/// cell-building methods (\`BuildCell\`, \`Sustain\`), so Phase 0 avoids
/// duplicating their work.
fn aggregate_plate_demand(goals: &[Goal]) -> BTreeMap<ItemId, u32> {
    let mut demand: BTreeMap<ItemId, u32> = BTreeMap::new();
    for goal in goals {
        match goal {
            Goal::Have { item, count, .. } | Goal::Produced { item, count, .. } => {
                if SMELTABLE_ITEMS.contains(&item.as_str()) && *count >= MIN_DEMAND_THRESHOLD {
                    *demand.entry(item.clone()).or_insert(0) += count;
                }
            }
            Goal::All(inner) => {
                let inner_demand = aggregate_plate_demand(inner);
                for (k, v) in inner_demand {
                    *demand.entry(k).or_insert(0) += v;
                }
            }
            Goal::Researched(_)
            | Goal::Producing { .. }
            | Goal::Sustain { .. }
            | Goal::Extracted { .. }
            | Goal::Gathered { .. }
            | Goal::Orbiting { .. }
            | Goal::Built { .. }
            | Goal::Charted { .. } => {}
        }
    }
    demand
}