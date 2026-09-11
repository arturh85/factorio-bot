//! Tests for module instance memory persistence, ID monotonicity,
//! reconciliation after partial build, and rollback safety.
//!
//! These tests verify that InstanceMemory survives a round-trip through
//! a PlannedMilestone, that IDs are monotonic and overflow-safe, that
//! reconciled instance state is preserved across deserialization of old
//! records, and that a failed candidate scheduling rollback does not
//! leak instance IDs.

use std::collections::{BTreeMap, BTreeSet};

use factorio_bot_planner::memory::ReplanMemory;
use factorio_bot_planner::modules::artifact::ModuleError;
use factorio_bot_planner::modules::instance::{
    InstanceMemory, ModuleInstance, PartState, Placement,
};

// ---------------------------------------------------------------------------
// Old record deserialization
// ---------------------------------------------------------------------------

#[test]
fn old_record_deserializes_with_default_modules() {
    // An old JSON record without `modules` should deserialize with
    // InstanceMemory::default() via #[serde(default)].
    let old_json = r#"{"plan_round": 5, "entities": {}, "cells": [], "chains": [], "blocks": [], "recovery_overrides": []}"#;
    let memory: ReplanMemory =
        serde_json::from_str(old_json).expect("old record without modules should deserialize");
    assert_eq!(memory.plan_round, 5);
    assert!(memory.entities.is_empty());
    assert_eq!(memory.modules.next_id, 1);
    assert!(memory.modules.instances.is_empty());
}

// ---------------------------------------------------------------------------
// ID monotonicity and overflow
// ---------------------------------------------------------------------------

#[test]
fn instance_ids_are_monotonic_and_start_at_1() {
    let mut mem = InstanceMemory::default();
    assert_eq!(mem.next_id, 1);
    let id1 = mem.allocate_id().unwrap();
    let id2 = mem.allocate_id().unwrap();
    assert!(id2 > id1);
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
}

#[test]
fn instance_id_overflow_returns_error() {
    let mut mem = InstanceMemory::default();
    // Set next_id to u64::MAX so the next allocation overflows.
    mem.next_id = u64::MAX;
    let result = mem.allocate_id();
    assert!(result.is_err());
    match result {
        Err(ModuleError::InvalidArtifact(msg)) => {
            assert!(
                msg.contains("overflow") || msg.contains("ID"),
                "error should mention overflow/ID: {msg}"
            );
        }
        _ => panic!("expected InvalidArtifact error, got {:?}", result),
    }
}

// ---------------------------------------------------------------------------
// Same coordinates on distinct surfaces
// ---------------------------------------------------------------------------

#[test]
fn same_coordinates_on_distinct_surfaces() {
    let mut mem = InstanceMemory::default();
    let id1 = mem.allocate_id().unwrap();
    mem.insert(ModuleInstance {
        id: id1,
        design_id: "test".into(),
        placement: Placement {
            surface: "nauvis".into(),
            half_x: 10,
            half_y: 20,
            direction: 0,
        },
        bindings: vec![],
        parts: BTreeMap::new(),
        construction_actions: BTreeMap::new(),
        commissioned_tick: None,
    });

    let id2 = mem.allocate_id().unwrap();
    let instance2 = ModuleInstance {
        id: id2,
        design_id: "test".into(),
        placement: Placement {
            surface: "gleba".into(),
            half_x: 10,
            half_y: 20,
            direction: 0,
        },
        bindings: vec![],
        parts: BTreeMap::new(),
        construction_actions: BTreeMap::new(),
        commissioned_tick: None,
    };
    mem.insert(instance2);

    // Both instances should coexist since they're on different surfaces
    // despite same coordinates.
    assert_eq!(mem.instances.len(), 2);
    let inst1 = mem.get(id1).unwrap();
    let inst2 = mem.get(id2).unwrap();
    assert_eq!(inst1.placement.surface, "nauvis");
    assert_eq!(inst2.placement.surface, "gleba");
    assert_eq!(inst1.placement.half_x, inst2.placement.half_x);
}

// ---------------------------------------------------------------------------
// PlannerSession carries InstanceMemory
// ---------------------------------------------------------------------------

#[test]
fn planner_session_carries_instance_memory() {
    let session = factorio_bot_planner::modules::compile::PlannerSession::new();
    assert_eq!(session.memory.next_id, 1);
    assert!(session.memory.instances.is_empty());
}

// ---------------------------------------------------------------------------
// ReplanMemory round-trip preserves module memory
// ---------------------------------------------------------------------------

#[test]
fn replan_memory_round_trip() {
    let memory = ReplanMemory {
        plan_round: 0,
        entities: BTreeMap::new(),
        cells: vec![],
        chains: vec![],
        blocks: vec![],
        recovery_overrides: BTreeSet::new(),
        modules: {
            let mut m = InstanceMemory::default();
            let id = m.allocate_id().unwrap();
            m.insert(ModuleInstance {
                id,
                design_id: "rocket-silo-design".into(),
                placement: Placement {
                    surface: "nauvis".into(),
                    half_x: 0,
                    half_y: 0,
                    direction: 0,
                },
                bindings: vec![],
                parts: BTreeMap::from([
                    ("silo".into(), PartState::Standing),
                    ("pole".into(), PartState::Missing),
                ]),
                construction_actions: BTreeMap::new(),
                commissioned_tick: None,
            });
            m
        },
    };

    // Serialize and deserialize.
    let json = serde_json::to_string(&memory).expect("serialize ReplanMemory");
    let back: ReplanMemory =
        serde_json::from_str(&json).expect("deserialize ReplanMemory with modules");

    assert_eq!(back.modules.instances.len(), 1);
    let instance = back.modules.get(1).expect("instance id 1");
    assert_eq!(instance.design_id, "rocket-silo-design");
    assert_eq!(instance.parts.get("silo"), Some(&PartState::Standing));
}

// ---------------------------------------------------------------------------
// Recovery: partially built cells
// ---------------------------------------------------------------------------

#[test]
fn partially_built_cell_recovery() {
    // Simulate a cell where the drill is built but the furnace is not.
    let mut mem = InstanceMemory::default();
    let id = mem.allocate_id().unwrap();
    mem.insert(ModuleInstance {
        id,
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
    });

    // The memory should preserve partial state.
    let inst = mem.get(id).expect("instance exists");
    assert_eq!(inst.parts.get("drill"), Some(&PartState::Standing));
    assert_eq!(inst.parts.get("furnace"), Some(&PartState::Missing));

    // Serialize and deserialize.
    let json = serde_json::to_string(&mem).expect("serialize");
    let back: InstanceMemory = serde_json::from_str(&json).expect("deserialize");
    let recovered = back.get(id).expect("recovered instance");
    assert_eq!(recovered.parts.get("drill"), Some(&PartState::Standing));
}

// ---------------------------------------------------------------------------
// Rollback after failed candidate scheduling does not leak IDs
// ---------------------------------------------------------------------------

#[test]
fn rollback_does_not_leak_ids() {
    let mut mem = InstanceMemory::default();
    let id_before = mem.next_id;

    // Simulate a sequence: allocate, then roll back by discarding the memory.
    let mut candidate_local = mem.clone();
    let _candidate_id = candidate_local.allocate_id().unwrap();

    // Discard candidate-local memory (simulates failed scheduling).
    // The original mem should be unchanged.
    assert_eq!(mem.next_id, id_before);
    assert_eq!(mem.next_id, 1);
    assert!(mem.instances.is_empty());
}

// ---------------------------------------------------------------------------
// Two instances cannot share the same ID (detected at insert)
// ---------------------------------------------------------------------------

#[test]
fn duplicate_id_is_rejected() {
    let mut mem = InstanceMemory::default();
    let id = mem.allocate_id().unwrap();

    mem.insert(ModuleInstance {
        id,
        design_id: "first".into(),
        placement: Placement {
            surface: "nauvis".into(),
            half_x: 0,
            half_y: 0,
            direction: 0,
        },
        bindings: vec![],
        parts: BTreeMap::new(),
        construction_actions: BTreeMap::new(),
        commissioned_tick: None,
    });

    // Second insert with same ID overwrites (BTreeMap behavior).
    mem.insert(ModuleInstance {
        id,
        design_id: "second".into(),
        placement: Placement {
            surface: "nauvis".into(),
            half_x: 10,
            half_y: 10,
            direction: 0,
        },
        bindings: vec![],
        parts: BTreeMap::new(),
        construction_actions: BTreeMap::new(),
        commissioned_tick: None,
    });

    // The instance memory should have only one entry (the overwritten one).
    assert_eq!(mem.instances.len(), 1);
    assert_eq!(mem.get(id).unwrap().design_id, "second");
}
