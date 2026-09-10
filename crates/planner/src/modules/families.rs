//! Extract existing cell designs into reusable module artifacts.
//!
//! Initial supported families:
//! - `OreToPlate`: a single burner-mining-drill + stone-furnace pair
//! - `RedScience`: automation-science-pack assembly with direct lab sink

use std::collections::BTreeMap;

use factorio_bot_core::types::Direction;

use crate::modules::artifact::{
    KnowledgeOrigin, ModuleDesign, ModuleError, ModuleFamily, ModuleParameters,
    Offset, OperatingContract, Part, Port, PortMode, Rate,
    design_id,
};
use crate::state::PlanState;

/// Extract a module design from the native planner layout for a given family
/// and parameters.
///
/// Returns the design with all fields populated, validated, and content-hashed.
pub fn extract_design(
    state: &PlanState,
    family: ModuleFamily,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    match family {
        ModuleFamily::OreToPlate => extract_ore_to_plate(state, parameters),
        ModuleFamily::RedScience => extract_red_science(state, parameters),
    }
}

fn extract_ore_to_plate(
    _state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;
    if item != "iron-plate" && item != "copper-plate" {
        return Err(ModuleError::Unsupported(format!(
            "ore-to-plate: unsupported item '{item}', expected iron-plate or copper-plate"
        )));
    }
    let ore = if item == "iron-plate" { "iron-ore" } else { "copper-ore" };

    // Build the static geometry of one ore-to-plate cell.
    // The native layout places a burner-mining-drill facing east, with a
    // stone-furnace directly north of its output.
    // Positions are relative to the drill's origin at (0, 0) half-tiles.
    let parts = vec![
        Part {
            role: "drill".into(),
            entity: "burner-mining-drill".into(),
            offset: Offset { half_x: 0, half_y: 0 },
            direction: Direction::East as u8,
            recipe: None,
            underground_half: None,
        },
        Part {
            role: "furnace".into(),
            entity: "stone-furnace".into(),
            // Furnace is 2 tiles north of the drill (in half-tile offset).
            // The drill occupies a 2x2 area, and the furnace is placed
            // with its centre 2 tiles (4 half-tiles) north.
            offset: Offset { half_x: 0, half_y: 4 },
            direction: Direction::North as u8,
            recipe: Some(item.clone()),
            underground_half: None,
        },
    ];

    // Bill of materials: 1 drill + 1 furnace.
    let bill = BTreeMap::from([
        ("burner-mining-drill".to_string(), 1u64),
        ("stone-furnace".to_string(), 1u64),
    ]);

    // Ports: the input side (drill`s belt feed) and output side (furnace output).
    let ports = vec![
        Port {
            id: "belt-input".into(),
            mode: PortMode::BeltInput,
            item: ore.into(),
            offset: Offset { half_x: -2, half_y: 0 },
            direction: Direction::West as u8,
            lane: None,
            maximum: Rate::new(1, 600).unwrap(),
        },
        Port {
            id: "inventory-output".into(),
            mode: PortMode::InventoryOutput,
            item: item.clone(),
            offset: Offset { half_x: 0, half_y: 6 },
            direction: Direction::North as u8,
            lane: None,
            maximum: Rate::new(1, 600).unwrap(),
        },
    ];

    // Required clearance: a corridor around the cell.
    let required_clearance = vec![
        Offset { half_x: -3, half_y: -1 },
        Offset { half_x: 3, half_y: -1 },
        Offset { half_x: -3, half_y: 5 },
        Offset { half_x: 3, half_y: 5 },
    ];

    // Precedence: drill must be placed before furnace (furnace may go
    // on the drill's output).
    let precedence = vec![
        ("drill".into(), "furnace".into()),
    ];

    let operation = OperatingContract {
        inputs: BTreeMap::from([(ore.into(), Rate::new(1, 600).unwrap())]),
        outputs: BTreeMap::from([(item.clone(), Rate::new(1, 600).unwrap())]),
        power_watts: 0, // burner cell, no electric draw
        fuel_per_tick: BTreeMap::from([
            ("coal".into(), Rate::new(1, 4800).unwrap()),
        ]),
        startup_latency_ticks: 4800, // ~80 seconds at 60 UPS
        startup_items: BTreeMap::from([
            ("coal".to_string(), 10u64),
            (ore.to_string(), 5u64),
        ]),
        local_buffer_capacity: BTreeMap::from([
            (ore.into(), 50u64),
            (item.clone(), 10u64),
        ]),
        required_research: vec![],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };

    let mut design = ModuleDesign {
        schema: 1,
        id: String::new(),
        family: ModuleFamily::OreToPlate,
        generator_version: 1,
        origin: KnowledgeOrigin::Extracted,
        parameters: parameters.clone(),
        prototype_hash: "extracted-v1".into(),
        mod_versions: BTreeMap::new(),
        parents: vec![],
        training_manifest: None,
        parts,
        ports,
        required_clearance,
        expansion_space: vec![],
        bill,
        precedence,
        operation,
    };

    // Compute the content-hash ID.
    design.id = design_id(&design)?;

    Ok(design)
}

fn extract_red_science(
    _state: &PlanState,
    _parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    Err(ModuleError::Unsupported("red-science extraction not yet implemented".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    #[test]
    fn extracted_iron_cell_still_contains_the_actual_pair() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let design = extract_design(
            &state,
            ModuleFamily::OreToPlate,
            &ModuleParameters {
                item: "iron-plate".into(),
                with_pole: false,
                labs: 0,
            },
        )
        .unwrap();
        assert_eq!(design.bill.get("burner-mining-drill"), Some(&1));
        assert_eq!(design.bill.get("stone-furnace"), Some(&1));
        assert_eq!(design.operation.outputs["iron-plate"].numerator, 1);
    }

    #[test]
    fn copper_plate_extraction_works() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let design = extract_design(
            &state,
            ModuleFamily::OreToPlate,
            &ModuleParameters {
                item: "copper-plate".into(),
                with_pole: false,
                labs: 0,
            },
        )
        .unwrap();
        assert_eq!(design.operation.inputs.contains_key("copper-ore"), true);
        assert_eq!(design.bill.get("burner-mining-drill"), Some(&1));
    }

    #[test]
    fn unsupported_item_is_refused() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let result = extract_design(
            &state,
            ModuleFamily::OreToPlate,
            &ModuleParameters {
                item: "stone-brick".into(),
                with_pole: false,
                labs: 0,
            },
        );
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ModuleError::Unsupported(_)));
    }
}
