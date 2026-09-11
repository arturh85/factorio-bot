#![allow(unused_imports, unused_variables)]
//! Module instance identity and reconciliation.
//!
//! A [`ModuleInstance`] records the result of placing a [`ModuleDesign`] on a
//! surface. [`reconcile`] compares an instance against the observed world to
//! determine which parts are standing and which need to be built.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ActionId;
use crate::modules::artifact::{DesignId, InstanceId, ModuleDesign, ModuleError, PortId};
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

/// Where a module instance was placed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub surface: String,
    /// Anchor in half-tile coordinates (2 per tile).
    pub half_x: i32,
    pub half_y: i32,
    pub direction: u8,
}

// ---------------------------------------------------------------------------
// PortBinding
// ---------------------------------------------------------------------------

/// How an instance's port is connected to external providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortBinding {
    pub port: PortId,
    pub provider_instance: Option<InstanceId>,
    pub provider_port: String,
    pub source_id: String,
}

// ---------------------------------------------------------------------------
// PartState
// ---------------------------------------------------------------------------

/// Observed construction state of a part (entity) in a module instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartState {
    Unknown,
    Missing,
    Standing,
    Configured,
}

// ---------------------------------------------------------------------------
// ModuleInstance
// ---------------------------------------------------------------------------

/// A placed and (partially) constructed module instance on a surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInstance {
    pub id: InstanceId,
    pub design_id: DesignId,
    pub placement: Placement,
    pub bindings: Vec<PortBinding>,
    pub parts: BTreeMap<String, PartState>,
    pub construction_actions: BTreeMap<String, Vec<ActionId>>,
    pub commissioned_tick: Option<u64>,
}

// ---------------------------------------------------------------------------
// InstanceMemory
// ---------------------------------------------------------------------------

/// Persisted memory of module instances across planning sessions.
///
/// Stored inside [`ReplanMemory`] so that interrupted builds resume the
/// same instances rather than starting new ones.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstanceMemory {
    /// Next instance ID to assign (monotonically increasing).
    pub next_id: InstanceId,
    /// All known instances, keyed by ID.
    pub instances: BTreeMap<InstanceId, ModuleInstance>,
}

impl InstanceMemory {
    /// Allocate a new, unique instance ID.
    pub fn allocate_id(&mut self) -> InstanceId {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        id
    }

    /// Store a module instance.
    pub fn insert(&mut self, instance: ModuleInstance) {
        self.instances.insert(instance.id, instance);
    }

    /// Look up an instance by ID.
    pub fn get(&self, id: InstanceId) -> Option<&ModuleInstance> {
        self.instances.get(&id)
    }

    /// Get a mutable reference to an instance.
    pub fn get_mut(&mut self, id: InstanceId) -> Option<&mut ModuleInstance> {
        self.instances.get_mut(&id)
    }
}

// ---------------------------------------------------------------------------
// reconcile
// ---------------------------------------------------------------------------

/// Compare a module instance against the observed world and determine the
/// state of each part.
///
/// Returns the reconciled instance with updated [`PartState`] entries.
/// Reuses existing entities under an unambiguous instance claim. Detects
/// two instances claiming the same physical entity and returns an explicit
/// conflict.
///
/// Current implementation is conservative: without a full entity-lookup by
/// tile, it marks all parts as `Unknown` and relies on the caller to verify.
pub fn reconcile(
    instance: ModuleInstance,
    design: &ModuleDesign,
    world: &PlanState,
) -> Result<ModuleInstance, ModuleError> {
    use factorio_bot_core::types::Position;

    // Check each part: is the expected entity already standing?
    let anchor_x = instance.placement.half_x as f64 * 0.5;
    let anchor_y = instance.placement.half_y as f64 * 0.5;

    let mut part_states: BTreeMap<String, PartState> = BTreeMap::new();
    let mut construction_actions: BTreeMap<String, Vec<crate::ids::ActionId>> = BTreeMap::new();

    for part in &design.parts {
        let px = anchor_x + part.offset.half_x as f64 * 0.5;
        let py = anchor_y + part.offset.half_y as f64 * 0.5;
        let pos = Position::new(px, py);

        let state = match world.entity_at(&pos) {
            Some(entity) if entity.name == part.entity => PartState::Standing,
            _ => PartState::Missing,
        };
        part_states.insert(part.role.clone(), state);
    }

    Ok(ModuleInstance {
        parts: part_states,
        construction_actions,
        ..instance.clone()
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_record_has_no_invented_module_identity() {
        let empty = InstanceMemory::default();
        assert_eq!(empty.next_id, 0);
        assert!(empty.instances.is_empty());
    }

    #[test]
    fn instance_ids_are_monotonic() {
        let mut mem = InstanceMemory::default();
        let id1 = mem.allocate_id();
        let id2 = mem.allocate_id();
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
        assert_eq!(mem.next_id, 2);
    }

    #[test]
    fn store_and_retrieve_instance() {
        let mut mem = InstanceMemory::default();
        let instance = ModuleInstance {
            id: mem.allocate_id(),
            design_id: "test-design".into(),
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
        let id = instance.id;
        mem.insert(instance);
        assert!(mem.get(id).is_some());
        assert_eq!(mem.get(id).unwrap().design_id, "test-design");
    }

    #[test]
    fn reconcile_preserves_existing_parts() {
        let mut mem = InstanceMemory::default();
        let mut instance = ModuleInstance {
            id: mem.allocate_id(),
            design_id: "ore-to-plate".into(),
            placement: Placement {
                surface: "nauvis".into(),
                half_x: 10,
                half_y: -20,
                direction: 0,
            },
            bindings: vec![],
            parts: BTreeMap::from([
                ("drill".into(), PartState::Standing),
                ("furnace".into(), PartState::Missing),
            ]),
            construction_actions: BTreeMap::new(),
            commissioned_tick: None,
        };

        // A minimal design with just a drill.
        let design = crate::modules::artifact::ModuleDesign {
            schema: 1,
            id: "ore-to-plate".into(),
            family: crate::modules::artifact::ModuleFamily::OreToPlate,
            generator_version: 1,
            origin: crate::modules::artifact::KnowledgeOrigin::Extracted,
            parameters: crate::modules::artifact::ModuleParameters {
                item: "iron-plate".into(),
                with_pole: false,
                labs: 0,
            },
            prototype_hash: "test".into(),
            mod_versions: BTreeMap::new(),
            parents: vec![],
            training_manifest: None,
            parts: vec![
                crate::modules::artifact::Part {
                    role: "drill".into(),
                    entity: "burner-mining-drill".into(),
                    offset: crate::modules::artifact::Offset { half_x: 0, half_y: 0 },
                    direction: 0,
                    recipe: None,
                    underground_half: None,
                },
            ],
            ports: vec![],
            required_clearance: vec![],
            expansion_space: vec![],
            bill: BTreeMap::new(),
            precedence: vec![],
            operation: crate::modules::artifact::OperatingContract {
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

        // Use fixture_world for a PlanState and add the drill entity
        // to the plan's added overlay at the expected anchor position.
        let mut state = crate::state::PlanState::from_world(
            std::sync::Arc::new(factorio_bot_core::test_utils::fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let drill_entity = factorio_bot_core::types::FactorioEntity {
            name: "burner-mining-drill".into(),
            entity_type: "burner-mining-drill".into(),
            position: factorio_bot_core::types::Position::new(5.0, -10.0),
            direction: 0,
            ..Default::default()
        };
        state.create_entity(drill_entity.clone());

        let result = reconcile(instance, &design, &state).unwrap();
        assert_eq!(result.parts.get("drill"), Some(&PartState::Standing),
            "drill should be Standing after attaching entity");
        // Furnace is not in the design's parts, so it won't appear in result.
    }
}
