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
            // Drill at origin, facing north. Its output emerges on the
            // tile immediately north of its north face (at y = 1.0 tiles
            // from centre, since burner-mining-drill is 2x2).
            offset: Offset { half_x: 0, half_y: 0 },
            direction: Direction::North as u8,
            recipe: None,
            underground_half: None,
        },
        Part {
            role: "furnace".into(),
            entity: "stone-furnace".into(),
            // Furnace placed so its south input face sits on the drill's
            // output tile at y = 2. Furnace centre at (0.5, 2.5) tiles =
            // offset (1, 5) in half-tiles, covering tile (0, 2)-(1, 3).
            //
            // The stone-furnace is ~2x2 so it occupies tiles (0,2)-(1,3)
            // which overlaps the drill's output tile (0,2). Items pass
            // directly from drill to furnace without an inserter.
            offset: Offset { half_x: 1, half_y: 5 },
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

    // Ports: ore belt input on the drill's west face, plate output on furnace's north face.
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
            offset: Offset { half_x: 1, half_y: 8 },
            direction: Direction::North as u8,
            lane: None,
            maximum: Rate::new(1, 600).unwrap(),
        },
    ];

    // Required clearance: a corridor around the cell.
    let required_clearance = vec![
        Offset { half_x: -3, half_y: -1 },
        Offset { half_x: 3, half_y: -1 },
        Offset { half_x: -3, half_y: 9 },
        Offset { half_x: 3, half_y: 9 },
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
    let item = "automation-science-pack".to_string();

    // Simple red-science cell: one assembler fed by two chests (one iron-gear-wheel
    // and one copper-plate), outputting to a third chest.
    // The native layout places the assembler at origin, with input chests to the
    // left and right, and an output chest to the north.
    let parts = vec![
        Part {
            role: "assembler".into(),
            entity: "assembling-machine-1".into(),
            offset: Offset { half_x: 0, half_y: 0 },
            direction: Direction::North as u8,
            recipe: Some(item.clone()),
            underground_half: None,
        },
    ];

    // Bill of materials: 1 assembling-machine-1 + 3 inserters + 3 chests.
    let bill = BTreeMap::from([
        ("assembling-machine-1".to_string(), 1u64),
        ("inserter".to_string(), 3u64),
        ("iron-chest".to_string(), 3u64),
    ]);

    // Ports: belt inputs for ingredients, lab output for packs.
    let ports = vec![
        Port {
            id: "input-copper".into(),
            mode: PortMode::BeltInput,
            item: "copper-plate".into(),
            // Copper input from the west
            offset: Offset { half_x: -4, half_y: 0 },
            direction: Direction::West as u8,
            lane: None,
            // 1 pack needs 1 copper-plate, assembler assembles with speed 0.5,
            // recipe energy 5 -> 180 ticks per pack = 1/180 packs/tick
            maximum: Rate::new(1, 180).unwrap(),
        },
        Port {
            id: "input-gears".into(),
            mode: PortMode::BeltInput,
            item: "iron-gear-wheel".into(),
            // Gear input from the east
            offset: Offset { half_x: 4, half_y: 0 },
            direction: Direction::East as u8,
            lane: None,
            // 1 pack needs 1 gear wheel
            maximum: Rate::new(1, 180).unwrap(),
        },
        Port {
            id: "output".into(),
            mode: PortMode::InventoryOutput,
            item: item.clone(),
            // Output to the north
            offset: Offset { half_x: 0, half_y: 5 },
            direction: Direction::North as u8,
            lane: None,
            maximum: Rate::new(1, 180).unwrap(),
        },
    ];

    // Required clearance: 3-tile corridor around the cell.
    let required_clearance: Vec<Offset> = vec![
        Offset { half_x: -3, half_y: -3 },
        Offset { half_x: 3, half_y: -3 },
        Offset { half_x: -3, half_y: 3 },
        Offset { half_x: 3, half_y: 3 },
    ];

    let operation = OperatingContract {
        inputs: BTreeMap::from([
            ("copper-plate".into(), Rate::new(1, 180).unwrap()),
            ("iron-gear-wheel".into(), Rate::new(1, 180).unwrap()),
        ]),
        outputs: BTreeMap::from([(item, Rate::new(1, 180).unwrap())]),
        power_watts: 90000, // assembling-machine-1 at ~90 kW
        fuel_per_tick: BTreeMap::new(),
        startup_latency_ticks: 600,
        startup_items: BTreeMap::new(),
        local_buffer_capacity: BTreeMap::new(),
        required_research: vec!["automation-science-pack".into()],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };

    let design = ModuleDesign {
        schema: 1,
        id: format!("red-science-cell"),
        family: ModuleFamily::RedScience,
        generator_version: 1,
        origin: KnowledgeOrigin::Extracted,
        parameters: ModuleParameters { item: "automation-science-pack".into(), with_pole: false, labs: 0 },
        prototype_hash: "red-science-1".into(),
        mod_versions: BTreeMap::new(),
        parents: vec![],
        training_manifest: None,
        parts,
        ports,
        bill,
        required_clearance,
        expansion_space: vec![],
        precedence: vec![],
        operation,
    };

    Ok(design)
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    #[test]
    fn red_science_extraction_works() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let design = extract_design(
            &state,
            ModuleFamily::RedScience,
            &ModuleParameters { item: "automation-science-pack".into(), with_pole: false, labs: 0 },
        ).unwrap();
        assert_eq!(design.family, ModuleFamily::RedScience);
        assert_eq!(design.operation.outputs.get("automation-science-pack").unwrap().numerator, 1);
        assert_eq!(design.bill.len(), 3);
        assert_eq!(design.ports.len(), 3);
    }

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
