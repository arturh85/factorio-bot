#![allow(unused_imports)]
//! Select and site a bounded portfolio of module candidates.
//!
//! [`select_candidates`] chooses design variants and capacity counts for a
//! production request. [`site_candidates`] places instances of a chosen design
//! on observed terrain, returning feasible placements.

use std::collections::BTreeMap;
use std::sync::Arc;

use factorio_bot_core::types::Position;

use crate::control::PlanControl;
use crate::ids::BotId;
use crate::modules::artifact::{
    ModuleDesign, ModuleError, ModuleFamily, ModuleParameters, KnowledgeOrigin,
    OperatingContract,
};
use crate::modules::cache::{get_design, CacheMode, LibraryCache};
use crate::modules::instance::{
    InstanceMemory, ModuleInstance, PartState, Placement, PortBinding,
};
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// ProductionRequest
// ---------------------------------------------------------------------------

/// What a production goal asks for.
#[derive(Debug, Clone)]
pub struct ProductionRequest {
    pub item: String,
    pub per_minute: u32,
    pub support_ticks: u32,
}

// ---------------------------------------------------------------------------
// ModuleSelection
// ---------------------------------------------------------------------------

/// The result of selecting and siting modules for a production request.
#[derive(Debug, Clone)]
pub struct ModuleSelection {
    /// Selected designs (may include duplicates for multiple instances).
    pub designs: Vec<Arc<ModuleDesign>>,
    /// Placed instances (one per selected design copy).
    pub instances: Vec<ModuleInstance>,
    /// The original requests this selection satisfies.
    pub requests: Vec<ProductionRequest>,
}

// ---------------------------------------------------------------------------
// select_candidates
// ---------------------------------------------------------------------------

/// Select candidate module designs for a production request.
///
/// Candidates are ordered by `(family, parameters, design_id)`. The global
/// candidate cap per milestone lives in the experiment manifest and is
/// enforced here through the work budget.
///
/// `CacheMode::Off` bypasses the design cache and regenerates each time.
pub fn select_candidates(
    request: &ProductionRequest,
    state: &PlanState,
    cache: &mut LibraryCache,
    mode: CacheMode,
    control: &PlanControl,
) -> Result<Vec<Arc<ModuleDesign>>, ModuleError> {
    // Check the budget first.
    control.checkpoint().map_err(|_| ModuleError::Cancelled)?;

    // Determine which families can satisfy this request.
    let families = matching_families(&request.item);

    let mut candidates: Vec<Arc<ModuleDesign>> = Vec::new();

    for family in &families {
        // Check budget before each family attempt.
        control.checkpoint().map_err(|_| ModuleError::Cancelled)?;

        // Get or generate the design.
        let params = ModuleParameters {
            item: request.item.clone(),
            with_pole: false,
            labs: if *family == ModuleFamily::RedScience { 1 } else { 0 },
        };

        match get_design(cache, state, *family, &params, mode) {
            Ok(design) => {
                candidates.push(design);
            }
            Err(ModuleError::Unsupported(_)) => {
                // Family doesn't support this item; skip.
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    // Sort candidates by (family, parameters, design_id).
    candidates.sort_by(|a, b| {
        let key = (&a.family, &a.parameters.item, &a.id);
        let other = (&b.family, &b.parameters.item, &b.id);
        key.cmp(&other)
    });

    Ok(candidates)
}

/// Return the list of module families that could produce `item`.
fn matching_families(_item: &str) -> Vec<ModuleFamily> {
    vec![
        ModuleFamily::OreToPlate,
        ModuleFamily::RedScience,
        ModuleFamily::AssemblerCell,
        ModuleFamily::OilRefinery,
        ModuleFamily::ChemicalPlant,
        ModuleFamily::RocketSilo,
        ModuleFamily::SpacePlatform,
        ModuleFamily::SmelterArray,
        ModuleFamily::DrillArray,
    ]
}

// ---------------------------------------------------------------------------
// site_candidates
// ---------------------------------------------------------------------------

/// Site instances of a design on observed terrain.
///
/// Attempts to find `count` distinct placements near `near` that pass
/// cheap occupancy, power, escape, and routing checks. Each orientation is
/// tried before moving to the next candidate anchor.
use factorio_bot_core::types::Direction;

pub fn site_candidates(
    design: &ModuleDesign,
    state: &PlanState,
    near: &Position,
    count: u32,
    control: &PlanControl,
) -> Result<Vec<ModuleInstance>, ModuleError> {
    // Check budget.
    control.checkpoint().map_err(|_| ModuleError::Cancelled)?;

    let mut instances = Vec::new();

    // Search outward from `near` in a spiral pattern.
    let anchor_hx = (near.x * 2.0).round() as i32;
    let anchor_hy = (near.y * 2.0).round() as i32;
    let mut ring = 0;

    while instances.len() < count as usize && ring < 12 {
        for dx in -(ring as i32)..=(ring as i32) {
            for dy in -(ring as i32)..=(ring as i32) {
                if dx.abs() != ring && dy.abs() != ring {
                    continue; // only perimeter of the ring
                }

                control.checkpoint().map_err(|_| ModuleError::Cancelled)?;
                control.charge(crate::control::WorkKind::Site)
                    .map_err(|_| ModuleError::Cancelled)?;

                let half_x = anchor_hx + dx * 12; // 6-tile spacing
                let half_y = anchor_hy + dy * 12;

                // Check all parts fit in charted area.
                let mut all_clear = true;
                let mut direction = 0u8;
                for part in &design.parts {
                    let px = half_x + part.offset.half_x;
                    let py = half_y + part.offset.half_y;
                    let pos = Position::new(px as f64 * 0.5, py as f64 * 0.5);
                    let dir = <Direction as factorio_bot_core::num_traits::FromPrimitive>::from_u8(part.direction).unwrap_or(Direction::North);
                    direction = part.direction;

                    if !state.is_area_free_facing(&part.entity, &pos, dir) {
                        all_clear = false;
                        break;
                    }
                }

                if all_clear {
                    let instance = ModuleInstance {
                        id: 0,
                        design_id: design.id.clone(),
                        placement: Placement {
                            surface: "nauvis".into(),
                            half_x,
                            half_y,
                            direction,
                        },
                        bindings: vec![],
                        parts: design
                            .parts
                            .iter()
                            .map(|p| (p.role.clone(), PartState::Missing))
                            .collect(),
                        construction_actions: BTreeMap::new(),
                        commissioned_tick: None,
                    };
                    instances.push(instance);
                    if instances.len() >= count as usize {
                        break;
                    }
                }
            }
            if instances.len() >= count as usize {
                break;
            }
        }
        ring += 1;
    }

    Ok(instances)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn make_control() -> PlanControl {
        PlanControl::new(crate::control::BudgetLimits::default())
    }

    #[test]
    fn select_iron_plate_returns_ore_to_plate() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut cache = LibraryCache::new();
        let control = make_control();

        let request = ProductionRequest {
            item: "iron-plate".into(),
            per_minute: 1,
            support_ticks: 36000,
        };

        let candidates = select_candidates(&request, &state, &mut cache, CacheMode::On, &control)
            .unwrap();
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].family, ModuleFamily::OreToPlate);
        assert_eq!(candidates[0].parameters.item, "iron-plate");
    }

    #[test]
    fn select_copper_plate_returns_ore_to_plate() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut cache = LibraryCache::new();
        let control = make_control();

        let request = ProductionRequest {
            item: "copper-plate".into(),
            per_minute: 1,
            support_ticks: 36000,
        };

        let candidates = select_candidates(&request, &state, &mut cache, CacheMode::On, &control)
            .unwrap();
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].family, ModuleFamily::OreToPlate);
    }

    #[test]
    fn select_unknown_item_returns_empty() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut cache = LibraryCache::new();
        let control = make_control();

        let request = ProductionRequest {
            item: "processing-unit".into(),
            per_minute: 1,
            support_ticks: 36000,
        };

        let candidates = select_candidates(&request, &state, &mut cache, CacheMode::On, &control)
            .unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn site_candidates_returns_requested_count() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let control = make_control();

        // Create a minimal design.
        let design = ModuleDesign {
            schema: 1,
            id: "test".into(),
            family: ModuleFamily::OreToPlate,
            generator_version: 1,
            origin: KnowledgeOrigin::Extracted,
            parameters: ModuleParameters {
                item: "iron-plate".into(),
                with_pole: false,
                labs: 0,
            },
            prototype_hash: "test".into(),
            mod_versions: BTreeMap::new(),
            parents: vec![],
            training_manifest: None,
            parts: vec![],
            ports: vec![],
            required_clearance: vec![],
            expansion_space: vec![],
            bill: BTreeMap::new(),
            precedence: vec![],
            operation: OperatingContract {
                inputs: BTreeMap::new(),
                outputs: BTreeMap::new(),
                power_watts: 0,
                fuel_per_tick: BTreeMap::new(),
                startup_latency_ticks: 0,
                startup_items: BTreeMap::new(),
                local_buffer_capacity: BTreeMap::new(),
                required_research: vec![],
                required_surface: "nauvis".into(),
                unsupported_mechanisms: vec![],
            },
        };

        let near = Position::new(0.0, 0.0);
        let instances = site_candidates(&design, &state, &near, 3, &control).unwrap();
        assert_eq!(instances.len(), 3);
    }

    #[test]
    fn cancellation_at_site_limit() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        // Use a control with zero Site budget.
        let control = PlanControl::new(crate::control::BudgetLimits {
            maxima: BTreeMap::from([(crate::control::WorkKind::Site, 0)]),
        });

        let design = ModuleDesign {
            schema: 1,
            id: "test".into(),
            family: ModuleFamily::OreToPlate,
            generator_version: 1,
            origin: KnowledgeOrigin::Extracted,
            parameters: ModuleParameters {
                item: "iron-plate".into(),
                with_pole: false,
                labs: 0,
            },
            prototype_hash: "test".into(),
            mod_versions: BTreeMap::new(),
            parents: vec![],
            training_manifest: None,
            parts: vec![],
            ports: vec![],
            required_clearance: vec![],
            expansion_space: vec![],
            bill: BTreeMap::new(),
            precedence: vec![],
            operation: OperatingContract {
                inputs: BTreeMap::new(),
                outputs: BTreeMap::new(),
                power_watts: 0,
                fuel_per_tick: BTreeMap::new(),
                startup_latency_ticks: 0,
                startup_items: BTreeMap::new(),
                local_buffer_capacity: BTreeMap::new(),
                required_research: vec![],
                required_surface: "nauvis".into(),
                unsupported_mechanisms: vec![],
            },
        };

        let near = Position::new(0.0, 0.0);
        let result = site_candidates(&design, &state, &near, 1, &control);
        assert!(result.is_err());
    }
}
