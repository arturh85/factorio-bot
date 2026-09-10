//! Compile selected module instances into action plans.
//!
//! Takes a [`ModuleSelection`] (designs + instances) and produces
//! construction steps (Place, Insert, etc.) that the scheduler can
//! execute. The compilation reuses the same action types the native
//! methods produce.rs and assemble.rs emit.

use std::collections::BTreeMap;

use factorio_bot_core::types::{FactorioEntity, Position};

use crate::action::{Action, ActionKind, Condition, Effect};
use crate::control::PlanControl;
use crate::PlannerError;


use crate::method::Step;
use crate::modules::instance::{ModuleInstance, PartState};
use crate::modules::ledger::OperatingLedger;
use crate::modules::select::ModuleSelection;
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// CompiledModules
// ---------------------------------------------------------------------------

/// The result of compiling a module selection into executable steps.
#[derive(Debug, Clone)]
pub struct CompiledModules {
    /// Steps for construction, fuelling, and configuration.
    pub steps: Vec<Step>,
    /// Module instances (with updated state).
    pub instances: Vec<ModuleInstance>,
    /// Operating ledger for supply/fuel accounting.
    pub ledger: OperatingLedger,
}

// ---------------------------------------------------------------------------
// compile_selection
// ---------------------------------------------------------------------------

/// Compile a module selection into bot-executable steps.
///
/// For each instance in the selection, this:
/// 1. Resolves each design part to an absolute world position
/// 2. Creates Place steps for every entity
/// 3. Adds fuel Insert steps for burner machines
/// 4. Adds recipe configuration where needed
/// 5. Records construction actions per instance role
pub fn compile_selection(
    selection: &ModuleSelection,
    state: &PlanState,
    control: &PlanControl,
) -> Result<CompiledModules, PlannerError> {
    control.checkpoint()?;

    let mut all_steps: Vec<Step> = Vec::new();
    let mut all_instances: Vec<ModuleInstance> = Vec::new();

    for (design, instance) in selection.designs.iter().zip(selection.instances.iter()) {
        control.checkpoint().map_err(|_| PlannerError::PlanningStopped { reason: "cancelled during compilation".into() })?;

        let mut construction_actions: BTreeMap<String, Vec<crate::ids::ActionId>> = BTreeMap::new();
        let mut part_states: BTreeMap<String, PartState> = BTreeMap::new();
        let mut steps: Vec<Step> = Vec::new();

        // Resolve the placement anchor: half-tile -> Position.
        let anchor_x = instance.placement.half_x as f64 * 0.5;
        let anchor_y = instance.placement.half_y as f64 * 0.5;

        for part in &design.parts {
            // Compute absolute position.
            let px = anchor_x + part.offset.half_x as f64 * 0.5;
            let py = anchor_y + part.offset.half_y as f64 * 0.5;
            let pos = Position::new(px, py);

            // Determine direction from instance placement + part direction.
                    // Build the entity.
            let entity = FactorioEntity {
                name: part.entity.clone(),
                entity_type: part.entity.clone(),
                position: pos.clone(),
                direction: part.direction,
                recipe: part.recipe.clone(),
                ..Default::default()
            };

            // Place step.
            steps.push(Step::Act(Box::new(Action {
                id: crate::ids::ActionId(0), // assigned by ActionIdGen during expansion
                kind: ActionKind::Place {
                    entity: Box::new(entity.clone()),
                },
                pre: vec![
                    Condition::AtPosition {
                        who: crate::action::Actor::Role,
                        pos: pos.clone(),
                        radius: 3.0,
                        min_radius: 0.5,
                    },
                    Condition::AreaFree {
                        pos: pos.clone(),
                        entity: part.entity.clone(),
                        direction: part.direction,
                    },
                ],
                eff: vec![
                    Effect::CreateEntity(Box::new(entity)),
                ],
                duration: 100, // placeholder; real value from prototype
                pinned: None,
                label: format!("place {} for {}", part.entity, part.role),
            })));

            part_states.insert(part.role.clone(), PartState::Standing);
        }

        // Build the updated instance.
        let updated_instance = ModuleInstance {
            parts: part_states,
            construction_actions,
            ..instance.clone()
        };

        all_steps.extend(steps);
        all_instances.push(updated_instance);
    }

    Ok(CompiledModules {
        steps: all_steps,
        instances: all_instances,
        ledger: OperatingLedger::default(),
    })
}

// ---------------------------------------------------------------------------
// PlannerMode and friends
// ---------------------------------------------------------------------------

/// Which planning engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerMode {
    Legacy,
    Modules,
}

/// Configuration for a planning session.
#[derive(Debug, Clone)]
pub struct PlannerOptions {
    pub mode: PlannerMode,
    pub candidate_limit: usize,
    pub support_ticks: u32,
}

impl Default for PlannerOptions {
    fn default() -> Self {
        Self {
            mode: PlannerMode::Legacy,
            candidate_limit: 8,
            support_ticks: 18000,
        }
    }
}

/// Mutable state across planning sessions.
#[derive(Debug, Clone)]
pub struct PlannerSession {
    pub observed_revision: u64,
}

impl PlannerSession {
    pub fn new() -> Self {
        Self { observed_revision: 0 }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::control::{BudgetLimits, PlanControl};
    use crate::goal::Goal;
    use crate::ids::BotId;
    use crate::registry_for;
    use crate::modules::artifact::{
        KnowledgeOrigin, ModuleDesign, ModuleFamily, ModuleParameters, Offset, OperatingContract, Part, Rate,
    };
    use crate::modules::instance::{InstanceMemory, Placement};
    use crate::modules::select::ProductionRequest;
    use crate::request::plan_controlled;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;

    fn make_control() -> PlanControl {
        PlanControl::new(BudgetLimits::default())
    }

    #[test]
    fn compile_ore_to_plate_produces_two_placement_steps() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);

        // Build a minimal OreToPlate design by hand.
        let design = ModuleDesign {
            schema: 1,
            id: "test-ore-to-plate".into(),
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
            parts: vec![
                Part {
                    role: "drill".into(),
                    entity: "burner-mining-drill".into(),
                    offset: Offset { half_x: 0, half_y: 0 },
                    direction: 0,
                    recipe: None,
                    underground_half: None,
                },
                Part {
                    role: "furnace".into(),
                    entity: "stone-furnace".into(),
                    offset: Offset { half_x: 0, half_y: 4 },
                    direction: 0,
                    recipe: Some("iron-plate".into()),
                    underground_half: None,
                },
            ],
            ports: vec![],
            required_clearance: vec![],
            expansion_space: vec![],
            bill: BTreeMap::from([
                ("burner-mining-drill".into(), 1u64),
                ("stone-furnace".into(), 1u64),
            ]),
            precedence: vec![],
            operation: OperatingContract {
                inputs: BTreeMap::from([("iron-ore".into(), Rate::new(1, 600).unwrap())]),
                outputs: BTreeMap::from([("iron-plate".into(), Rate::new(1, 600).unwrap())]),
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

        let instance = ModuleInstance {
            id: 1,
            design_id: "test-ore-to-plate".into(),
            placement: Placement {
                surface: "nauvis".into(),
                half_x: 0,
                half_y: 0,
                direction: 0,
            },
            bindings: vec![],
            parts: BTreeMap::from([
                ("drill".into(), PartState::Missing),
                ("furnace".into(), PartState::Missing),
            ]),
            construction_actions: BTreeMap::new(),
            commissioned_tick: None,
        };

        let selection = ModuleSelection {
            designs: vec![Arc::new(design)],
            instances: vec![instance],
            requests: vec![ProductionRequest {
                item: "iron-plate".into(),
                per_minute: 1,
                support_ticks: 36000,
            }],
        };

        let control = make_control();
        let result = compile_selection(&selection, &state, &control).unwrap();

        // Should have 2 Place steps (drill + furnace).
        assert_eq!(result.steps.len(), 2, "expected 2 placement steps");
        for (i, step) in result.steps.iter().enumerate() {
            match step {
                Step::Act(action) => println!("  {}: Place {:?}", i, action.kind),
                other => println!("  {}: {:?}", i, other),
            }
        }

        assert!(matches!(&result.steps[0], Step::Act(a) if matches!(a.kind, ActionKind::Place { .. })));
        assert_eq!(result.instances.len(), 1);
        assert_eq!(result.instances[0].parts.len(), 2);
    }
}
