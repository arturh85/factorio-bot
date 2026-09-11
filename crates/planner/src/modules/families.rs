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
        ModuleFamily::AssemblerCell => extract_assembler_cell(state, parameters),
        ModuleFamily::OilRefinery | ModuleFamily::ChemicalPlant => extract_fluid_manufacturing(state, family, parameters),
        ModuleFamily::RocketSilo => extract_rocket_silo(state, parameters),
        ModuleFamily::SpacePlatform => extract_space_platform(state, parameters),
        ModuleFamily::SmelterArray => extract_smelter_array(state, parameters),
        ModuleFamily::DrillArray => extract_drill_array(state, parameters),
    }
}

fn extract_ore_to_plate(
    state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;
    if item != "iron-plate" && item != "copper-plate" && item != "crude-oil" {
        return Err(ModuleError::Unsupported(format!(
            "ore-to-plate: unsupported item '{item}', expected iron-plate or copper-plate"
        )));
    }
    let ore = if item == "iron-plate" { "iron-ore" } else if item == "copper-plate" { "copper-ore" } else { "crude-oil" };

    // Build the static geometry of one ore-to-plate cell.
    // The native layout places a burner-mining-drill facing east, with a
    // stone-furnace directly north of its output.
    // Positions are relative to the drill's origin at (0, 0) half-tiles.
    let parts = vec![
        Part {
            role: "drill".into(),
            entity: "burner-mining-drill".into(),
            half_size: Some(state.entity_half_size("burner-mining-drill")),
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
            half_size: Some(state.entity_half_size("stone-furnace")),
            // Furnace placed so its south input face sits on the drill's
            // output tile at y = 2. Furnace centre at (0, 2) tiles =
            // offset (0, 4) in half-tiles, covering tile (-1, 1)-(0, 2).
            //
            // The stone-furnace is ~2x2 so it occupies tiles (0,2)-(1,3)
            // which overlaps the drill's output tile (0,2). Items pass
            // directly from drill to furnace without an inserter.
            offset: Offset { half_x: 0, half_y: 4 },
            direction: Direction::North as u8,
            recipe: Some(item.clone()),
            underground_half: None,
        },
    ];

    // Bill of materials: 1 drill + 1 furnace.
    let bill: BTreeMap<String, u64> = if item == "crude-oil" {
        BTreeMap::from([
            ("pumpjack".to_string(), 1u64),
            ("pipe".to_string(), 2u64),
        ])
    } else {
        BTreeMap::from([
            ("burner-mining-drill".to_string(), 1u64),
            ("stone-furnace".to_string(), 1u64),
        ])
    };

    // Ports: ore belt input on the drill's west face, plate output on furnace's north face.
    let ports: Vec<Port> = if item == "crude-oil" {
        vec![
            Port {
                id: "output".into(),
                mode: PortMode::InventoryOutput,
                item: "crude-oil".into(),
                offset: Offset { half_x: -1, half_y: 3 },
                direction: Direction::North as u8,
                lane: None,
                maximum: Rate::new(1, 120).unwrap(),
            },
        ]
    } else {
        vec![
            Port {
                id: "belt-input".into(),
                mode: PortMode::BeltInput,
                item: ore.into(),
                offset: Offset { half_x: -3, half_y: 0 },
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
        ]
    };

    // Required clearance: a corridor around the cell.
    let required_clearance: Vec<Offset> = if item == "crude-oil" {
        vec![
            Offset { half_x: -4, half_y: -4 },
            Offset { half_x: 4, half_y: -4 },
            Offset { half_x: -4, half_y: 4 },
            Offset { half_x: 4, half_y: 4 },
        ]
    } else {
        vec![
            Offset { half_x: -3, half_y: -1 },
            Offset { half_x: 3, half_y: -1 },
            Offset { half_x: -3, half_y: 9 },
            Offset { half_x: 3, half_y: 9 },
        ]
    };

    // Precedence: drill must be placed before furnace (furnace may go
    // on the drill's output).
    let precedence = vec![
        ("drill".into(), "furnace".into()),
    ];

    let operation: OperatingContract = if item == "crude-oil" {
        OperatingContract {
            inputs: BTreeMap::new(),
            outputs: BTreeMap::from([("crude-oil".into(), Rate::new(1, 120).unwrap())]),
            power_watts: 90000,
            fuel_per_tick: BTreeMap::new(),
            startup_latency_ticks: 120,
            startup_items: BTreeMap::from([("pipe".to_string(), 2u64)]),
            local_buffer_capacity: BTreeMap::new(),
            required_research: vec!["oil-processing".into()],
            required_surface: "nauvis".into(),
            unsupported_mechanisms: vec![],
        }
    } else {
        OperatingContract {
            inputs: BTreeMap::from([(ore.into(), Rate::new(1, 600).unwrap())]),
            outputs: BTreeMap::from([(item.clone(), Rate::new(1, 600).unwrap())]),
            power_watts: 0, // burner cell, no electric draw
            fuel_per_tick: BTreeMap::from([
                ("coal".into(), Rate::new(1, 4800).unwrap()),
            ]),
            startup_latency_ticks: 4800,
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
        }
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
    state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    if parameters.item != "automation-science-pack" {
        return Err(ModuleError::Unsupported(format!(
            "red-science: unsupported item '{}', expected automation-science-pack",
            parameters.item
        )));
    }
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
            half_size: Some(state.entity_half_size("assembling-machine-1")),
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

fn extract_assembler_cell(
    state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;

    // Count ingredients from the recipe prototype.
    let recipes = state.base().entity_graph.recipes();
    let recipe = recipes.get(item).ok_or_else(|| ModuleError::Unsupported(format!(
        "assembler-cell: no recipe for '{item}'"
    )))?;
    let ingredients = recipe.ingredients.as_ref().ok_or_else(|| ModuleError::Unsupported(format!(
        "assembler-cell: recipe for '{item}' has no ingredients"
    )))?;
    // Only handle crafting recipes (not oil-processing, chemistry, etc.)
    let category = recipe.category.as_str();
    match category {
        "crafting" | "crafting-with-fluid" | "advanced-crafting" => {}
        _ => return Err(ModuleError::Unsupported(format!(
            "assembler-cell: unsupported category '{category}' for '{item}'"
        ))),
    }
    let input_count = ingredients.len();
    if input_count > 3 {
        return Err(ModuleError::Unsupported(format!(
            "assembler-cell: {input_count}-input recipes not yet supported"
        )));
    }

    // Build geometry relative to the assembler centre at (0, 0).
    let mut parts: Vec<Part> = Vec::new();
    let mut ports: Vec<Port> = Vec::new();
    let mut bill: BTreeMap<String, u64> = BTreeMap::new();
    let mut required_clearance: Vec<Offset> = Vec::new();
    let mut precedence: Vec<(String, String)> = Vec::new();

    if input_count == 1 {
        // 1-input: belt at -6, inserter at -4, assembler at 0, out at 4.
        for hy in [-2i32, 0, 2] {
            parts.push(Part {
                role: "belt".into(),
                entity: "transport-belt".into(),
                half_size: Some(state.entity_half_size("transport-belt")),
                offset: Offset { half_x: -6, half_y: hy },
                direction: Direction::North as u8,
                recipe: None,
                underground_half: None,
            });
        }
        parts.push(Part {
            role: "inserter".into(),
            entity: "inserter".into(),
            half_size: Some(state.entity_half_size("inserter")),
            offset: Offset { half_x: -4, half_y: 0 },
            direction: Direction::West as u8,
            recipe: None,
            underground_half: None,
        });
        parts.push(Part {
            role: "power-pole".into(),
            entity: "small-electric-pole".into(),
            half_size: Some(state.entity_half_size("small-electric-pole")),
            offset: Offset { half_x: -4, half_y: -2 },
            direction: Direction::North as u8,
            recipe: None,
            underground_half: None,
        });
        ports.push(Port {
            id: "belt-input".into(),
            mode: PortMode::BeltInput,
            item: ingredients[0].name.clone(),
            offset: Offset { half_x: -7, half_y: -2 },
            direction: Direction::West as u8,
            lane: None,
            maximum: Rate::new(1, 60).unwrap(),
        });
        bill.entry("inserter".to_string()).or_insert(2);
        bill.entry("transport-belt".to_string()).or_insert(6);
        bill.entry("small-electric-pole".to_string()).or_insert(1);
        required_clearance = vec![
            Offset { half_x: -7, half_y: -2 },
            Offset { half_x: -7, half_y: 2 },
            Offset { half_x: 7, half_y: -2 },
            Offset { half_x: 7, half_y: 2 },
        ];
    } else if input_count == 2 {
        // 2-input: two belts at -8, -6; two inserters at (-4,-2) and (-4,2).
        for (i, ing) in ingredients.iter().enumerate() {
            let belt_x = -8 + i as i32 * 2;
            for hy in [-2i32, 0, 2] {
                parts.push(Part {
                    role: format!("belt-{}", i),
                    entity: "transport-belt".into(),
                    half_size: Some(state.entity_half_size("transport-belt")),
                    offset: Offset { half_x: belt_x, half_y: hy },
                    direction: Direction::North as u8,
                    recipe: None,
                    underground_half: None,
                });
            }
            let ins_x = -4i32;
            let ins_hy = -2 + i as i32 * 4;
            let is_long = i == 0; // far belt needs long-handed
            parts.push(Part {
                role: if i == 0 { "gear-inserter".into() } else { "copper-inserter".into() },
                entity: if is_long { "long-handed-inserter" } else { "inserter" }.into(),
                half_size: Some(state.entity_half_size(
                    if is_long { "long-handed-inserter" } else { "inserter" }
                )),
                offset: Offset { half_x: ins_x, half_y: ins_hy },
                direction: Direction::West as u8,
                recipe: None,
                underground_half: None,
            });
            ports.push(Port {
                id: format!("input-{i}"),
                mode: PortMode::BeltInput,
                item: ing.name.clone(),
                offset: Offset { half_x: belt_x - 1, half_y: -2 },
                direction: Direction::West as u8,
                lane: None,
                maximum: Rate::new(1, 60).unwrap(),
            });
            bill.entry(
                if is_long { "long-handed-inserter" } else { "inserter" }.to_string()
            ).and_modify(|c| *c += 1).or_insert(1);
        }
        parts.push(Part {
            role: "power-pole".into(),
            entity: "small-electric-pole".into(),
            half_size: Some(state.entity_half_size("small-electric-pole")),
            offset: Offset { half_x: -4, half_y: 0 },
            direction: Direction::North as u8,
            recipe: None,
            underground_half: None,
        });
        bill.entry("transport-belt".to_string()).or_insert(9);
        bill.entry("small-electric-pole".to_string()).or_insert(1);
        required_clearance = vec![
            Offset { half_x: -9, half_y: -2 },
            Offset { half_x: -9, half_y: 2 },
            Offset { half_x: 7, half_y: -2 },
            Offset { half_x: 7, half_y: 2 },
        ];
    } else {
        // 3-input belt-fed: belts at -10, -6, +6; inserters at -8, -4, +4
        for (i, ing) in ingredients.iter().enumerate() {
            let (belt_x, ins_x, ins_hy, is_long, dir): (i32, i32, i32, bool, u8) = match i {
                0 => (-10, -8, -2, true, Direction::West as u8),
                1 => (-6, -4, 2, false, Direction::West as u8),
                2 => (6, 4, 0, false, Direction::East as u8),
                _ => continue,
            };
            for hy in [0i32] {
                parts.push(Part {
                    role: format!("belt-{}", i),
                    entity: "transport-belt".into(),
                    half_size: Some(state.entity_half_size("transport-belt")),
                    offset: Offset { half_x: belt_x, half_y: hy },
                    direction: Direction::North as u8,
                    recipe: None,
                    underground_half: None,
                });
            }
            parts.push(Part {
                role: format!("inserter-{}", i),
                entity: if is_long { "long-handed-inserter" } else { "inserter" }.into(),
                half_size: Some(state.entity_half_size(
                    if is_long { "long-handed-inserter" } else { "inserter" }
                )),
                offset: Offset { half_x: ins_x, half_y: ins_hy },
                direction: dir,
                recipe: None,
                underground_half: None,
            });
            ports.push(Port {
                id: format!("input-{}", i),
                mode: PortMode::BeltInput,
                item: ing.name.clone(),
                offset: Offset { half_x: belt_x - 1, half_y: -2 },
                direction: Direction::West as u8,
                lane: None,
                maximum: Rate::new(1, 60).unwrap(),
            });
            bill.entry(
                if is_long { "long-handed-inserter" } else { "inserter" }.to_string()
            ).and_modify(|c| *c += 1).or_insert(1);
        }
        parts.push(Part {
            role: "power-pole".into(),
            entity: "small-electric-pole".into(),
            half_size: Some(state.entity_half_size("small-electric-pole")),
            offset: Offset { half_x: 0, half_y: -2 },
            direction: Direction::North as u8,
            recipe: None,
            underground_half: None,
        });
        bill.entry("transport-belt".to_string()).or_insert(3);
        bill.entry("small-electric-pole".to_string()).or_insert(1);
        required_clearance = vec![
            Offset { half_x: -6, half_y: -2 },
            Offset { half_x: -6, half_y: 3 },
            Offset { half_x: 6, half_y: -2 },
            Offset { half_x: 6, half_y: 3 },
        ];
    }

    // Common parts: assembler, output inserter, output belt
    parts.push(Part {
        role: "assembler".into(),
        entity: "assembling-machine-1".into(),
        half_size: Some(state.entity_half_size("assembling-machine-1")),
        offset: Offset { half_x: 0, half_y: 0 },
        direction: Direction::East as u8,
        recipe: Some(item.clone()),
        underground_half: None,
    });
    parts.push(Part {
        role: "out-inserter".into(),
        entity: "inserter".into(),
        half_size: Some(state.entity_half_size("inserter")),
        offset: Offset { half_x: 4, half_y: 0 },
        direction: Direction::West as u8,
        recipe: None,
        underground_half: None,
    });
    for hy in [-2i32, 0, 2] {
        parts.push(Part {
            role: "output-belt".into(),
            entity: "transport-belt".into(),
            half_size: Some(state.entity_half_size("transport-belt")),
            offset: Offset { half_x: 6, half_y: hy },
            direction: Direction::North as u8,
            recipe: None,
            underground_half: None,
        });
    }

    ports.push(Port {
        id: "output".into(),
        mode: PortMode::InventoryOutput,
        item: item.clone(),
        offset: Offset { half_x: 7, half_y: 2 },
        direction: Direction::North as u8,
        lane: None,
        maximum: Rate::new(1, 60).unwrap(),
    });

    bill.entry("assembling-machine-1".to_string()).or_insert(1);
    bill.entry("inserter".to_string())
        .and_modify(|c| *c += 1) // out-inserter
        .or_insert(1);

    let operation = OperatingContract {
        inputs: ingredients.iter().map(|ing| {
            (ing.name.clone(), Rate::new(1, 60).unwrap())
        }).collect(),
        outputs: BTreeMap::from([(item.clone(), Rate::new(1, 60).unwrap())]),
        power_watts: 90000,
        fuel_per_tick: BTreeMap::new(),
        startup_latency_ticks: 60,
        startup_items: BTreeMap::new(),
        local_buffer_capacity: BTreeMap::new(),
        required_research: vec!["automation".into()],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };

    let mut design = ModuleDesign {
        schema: 1,
        id: String::new(),
        family: ModuleFamily::AssemblerCell,
        generator_version: 1,
        origin: KnowledgeOrigin::Extracted,
        parameters: parameters.clone(),
        prototype_hash: "assembler-cell-v1".into(),
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

    design.id = design_id(&design)?;
    Ok(design)
}



fn extract_rocket_silo(
    state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;
    if item != "rocket-part" {
        return Err(ModuleError::Unsupported(format!(
            "rocket-silo: unsupported item '{item}'"
        )));
    }
    let bill = BTreeMap::from([
        ("rocket-silo".to_string(), 1u64),
        ("small-electric-pole".to_string(), 2u64),
    ]);
    let parts = vec![
        Part {
            role: "rocket-silo".into(),
            entity: "rocket-silo".into(),
            half_size: Some(Offset { half_x: 5, half_y: 5 }),
            offset: Offset { half_x: 0, half_y: 0 },
            direction: Direction::North as u8,
            recipe: Some("rocket-part".into()),
            underground_half: None,
        },
    ];
    let ports = vec![
        Port {
            id: "output".into(),
            mode: PortMode::InventoryOutput,
            item: "rocket-part".into(),
            offset: Offset { half_x: 5, half_y: 0 },
            direction: Direction::North as u8,
            lane: None,
            maximum: Rate::new(1, 180).unwrap(),
        },
    ];
    let required_clearance = vec![
        Offset { half_x: -6, half_y: -6 },
        Offset { half_x: 6, half_y: -6 },
        Offset { half_x: -6, half_y: 6 },
        Offset { half_x: 6, half_y: 6 },
    ];
    let operation = OperatingContract {
        inputs: BTreeMap::from([
            ("processing-unit".into(), Rate::new(1, 180).unwrap()),
            ("low-density-structure".into(), Rate::new(1, 180).unwrap()),
            ("rocket-fuel".into(), Rate::new(1, 180).unwrap()),
        ]),
        outputs: BTreeMap::from([("rocket-part".into(), Rate::new(1, 180).unwrap())]),
        power_watts: 1000000,
        fuel_per_tick: BTreeMap::new(),
        startup_latency_ticks: 60,
        startup_items: BTreeMap::new(),
        local_buffer_capacity: BTreeMap::new(),
        required_research: vec!["rocket-silo".into()],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };
    Ok(ModuleDesign {
        schema: 1,
        id: String::new(),
        family: ModuleFamily::RocketSilo,
        generator_version: 1,
        origin: KnowledgeOrigin::Extracted,
        parameters: parameters.clone(),
        prototype_hash: "rocket-silo-v1".into(),
        mod_versions: BTreeMap::new(), parents: vec![], training_manifest: None,
        parts, ports, required_clearance, expansion_space: vec![], bill,
        precedence: vec![], operation,
    })
}

fn extract_space_platform(
    state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;
    if item != "space-platform-starter-pack" {
        return Err(ModuleError::Unsupported(format!(
            "space-platform: unsupported item '{item}'"
        )));
    }
    let bill = BTreeMap::from([
        ("assembling-machine-3".to_string(), 1u64),
        ("small-electric-pole".to_string(), 1u64),
    ]);
    let parts = vec![
        Part {
            role: "assembler".into(),
            entity: "assembling-machine-3".into(),
            half_size: Some(Offset { half_x: 3, half_y: 3 }),
            offset: Offset { half_x: 0, half_y: 0 },
            direction: Direction::East as u8,
            recipe: Some("space-platform-starter-pack".into()),
            underground_half: None,
        },
    ];
    let ports = vec![
        Port {
            id: "output".into(),
            mode: PortMode::InventoryOutput,
            item: "space-platform-starter-pack".into(),
            offset: Offset { half_x: 4, half_y: 0 },
            direction: Direction::North as u8,
            lane: None,
            maximum: Rate::new(1, 600).unwrap(),
        },
    ];
    let required_clearance = vec![
        Offset { half_x: -3, half_y: -3 },
        Offset { half_x: 3, half_y: -3 },
        Offset { half_x: -3, half_y: 3 },
        Offset { half_x: 3, half_y: 3 },
    ];
    let operation = OperatingContract {
        inputs: BTreeMap::from([
            ("steel-plate".into(), Rate::new(1, 60).unwrap()),
            ("processing-unit".into(), Rate::new(1, 60).unwrap()),
            ("space-platform-foundation".into(), Rate::new(1, 60).unwrap()),
        ]),
        outputs: BTreeMap::from([("space-platform-starter-pack".into(), Rate::new(1, 600).unwrap())]),
        power_watts: 250000,
        fuel_per_tick: BTreeMap::new(),
        startup_latency_ticks: 120,
        startup_items: BTreeMap::new(),
        local_buffer_capacity: BTreeMap::from([("space-platform-starter-pack".into(), 2u64)]),
        required_research: vec!["space-platform".into()],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };
    Ok(ModuleDesign {
        schema: 1,
        id: String::new(),
        family: ModuleFamily::SpacePlatform,
        generator_version: 1,
        origin: KnowledgeOrigin::Extracted,
        parameters: parameters.clone(),
        prototype_hash: "space-platform-v1".into(),
        mod_versions: BTreeMap::new(), parents: vec![], training_manifest: None,
        parts, ports, required_clearance, expansion_space: vec![], bill,
        precedence: vec![], operation,
    })
}


fn extract_smelter_array(
    state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;
    // Support: iron-plate, copper-plate, steel-plate, stone-brick
    let supported = ["iron-plate", "copper-plate", "stone-brick", "steel-plate"];
    if !supported.contains(&item.as_str()) {
        return Err(ModuleError::Unsupported(format!(
            "smelter-array: unsupported item '{item}'"
        )));
    }
    // Determine ore and furnace count per belt
    // Yellow belt = 15/s = 900/min
    // Stone furnace = 0.3125/s = 18.75/min -> 48 per belt
    // Steel furnace = 0.625/s = 37.5/min -> 24 per belt
    // Electric furnace = 0.625/s = 37.5/min -> 24 per belt
    let furnace_entity = "steel-furnace";
    let furnaces_per_belt: u64 = 24;
    let belts = 1u64; // start with 1 belt
    let total = furnaces_per_belt * belts;

    let ore = match item.as_str() {
        "iron-plate" => "iron-ore",
        "copper-plate" => "copper-ore",
        "stone-brick" => "stone",
        "steel-plate" => "iron-plate",
        _ => return Err(ModuleError::Unsupported(format!("smelter-array: no ore for '{item}'"))),
    };

    // Two rows of furnaces: north row offset (0, -2), south row offset (0, +2)
    // Single belt down the middle at (0, 0)
    // Each row has total/2 furnaces
    let mut parts = Vec::new();
    let mut bill = BTreeMap::new();
    bill.insert(furnace_entity.to_string(), total);
    bill.insert("small-electric-pole".to_string(), total / 6 + 1);
    bill.insert("transport-belt".to_string(), 2);
    bill.insert("inserter".to_string(), total * 2);

    // For each furnace position
    for i in 0..total {
        let row = i % 2; // 0 = north, 1 = south
        let col = i / 2; // column index
        let half_x = (col as i32) * 4; // 2 tiles between furnace centres
        let half_y = if row == 0 { -4 } else { 4 }; // 2 tiles from belt

        // Furnace
        parts.push(Part {
            role: format!("furnace-{}", i).into(),
            entity: furnace_entity.into(),
            half_size: Some(state.entity_half_size(furnace_entity)),
            offset: Offset { half_x, half_y },
            direction: Direction::North as u8,
            recipe: Some(item.clone()),
            underground_half: None,
        });
        // Input inserter (from belt to furnace)
        let ins_hy = if row == 0 { -2 } else { 2 };
        parts.push(Part {
            role: format!("in-inserter-{}", i).into(),
            entity: "inserter".into(),
            half_size: Some(state.entity_half_size("inserter")),
            offset: Offset { half_x: half_x - 2, half_y: ins_hy },
            direction: if row == 0 { Direction::South as u8 } else { Direction::North as u8 },
            recipe: None,
            underground_half: None,
        });
        // Output inserter (from furnace to belt)
        let out_hy = if row == 0 { -2 } else { 2 };
        parts.push(Part {
            role: format!("out-inserter-{}", i).into(),
            entity: "inserter".into(),
            half_size: Some(state.entity_half_size("inserter")),
            offset: Offset { half_x: half_x + 2, half_y: out_hy },
            direction: if row == 0 { Direction::North as u8 } else { Direction::South as u8 },
            recipe: None,
            underground_half: None,
        });
    }

    // Belt columns
    for i in 0..(total / 2) {
        let hx = (i as i32) * 4;
        parts.push(Part {
            role: "belt".into(),
            entity: "transport-belt".into(),
            half_size: Some(state.entity_half_size("transport-belt")),
            offset: Offset { half_x: hx, half_y: 0 },
            direction: Direction::East as u8,
            recipe: None,
            underground_half: None,
        });
    }

    let ports = vec![
        Port {
            id: "input".into(),
            mode: PortMode::BeltInput,
            item: ore.into(),
            offset: Offset { half_x: -2, half_y: 0 },
            direction: Direction::West as u8,
            lane: None,
            maximum: Rate::new(1, 1).unwrap(),
        },
        Port {
            id: "output".into(),
            mode: PortMode::InventoryOutput,
            item: item.clone(),
            offset: Offset { half_x: (total as i32 / 2) * 4, half_y: 0 },
            direction: Direction::East as u8,
            lane: None,
            maximum: Rate::new(1, 1).unwrap(),
        },
    ];

    let left = -2i32;
    let right = (total as i32 / 2) * 4 + 2;
    let required_clearance = vec![
        Offset { half_x: left, half_y: -6 },
        Offset { half_x: right, half_y: -6 },
        Offset { half_x: left, half_y: 6 },
        Offset { half_x: right, half_y: 6 },
    ];

    let plates_per_tick = total as f64 * 0.625 / 60.0;
    let plates_per_tick_u64 = (plates_per_tick * 60.0) as u64;

    let operation = OperatingContract {
        inputs: BTreeMap::from([(ore.into(), Rate::new(plates_per_tick_u64, 60).unwrap())]),
        outputs: BTreeMap::from([(item.clone(), Rate::new(plates_per_tick_u64, 60).unwrap())]),
        power_watts: if furnace_entity == "electric-furnace" { total * 180000 } else { 0 },
        fuel_per_tick: BTreeMap::from([("coal".into(), Rate::new(total, 4800).unwrap())]),
        startup_latency_ticks: 600,
        startup_items: BTreeMap::from([
            ("coal".into(), total * 10),
            (ore.into(), total * 5),
        ]),
        local_buffer_capacity: BTreeMap::from([
            (ore.into(), total * 10),
            (item.clone(), total * 5),
        ]),
        required_research: vec![],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };

    Ok(ModuleDesign {
        schema: 1, id: String::new(), family: ModuleFamily::SmelterArray,
        generator_version: 1, origin: KnowledgeOrigin::Extracted,
        parameters: parameters.clone(),
        prototype_hash: "smelter-array-v1".into(),
        mod_versions: BTreeMap::new(), parents: vec![], training_manifest: None,
        parts, ports, required_clearance, expansion_space: vec![], bill,
        precedence: vec![], operation,
    })
}

fn extract_drill_array(
    state: &PlanState,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;
    let ore = match item.as_str() {
        "iron-ore" => "iron-ore",
        "copper-ore" => "copper-ore",
        "coal" => "coal",
        "stone" => "stone",
        _ => return Err(ModuleError::Unsupported(format!(
            "drill-array: unsupported item '{item}'"
        ))),
    };

    // Electric mining drill: 0.5 ore/s
    // Yellow belt: 15/s -> 30 drills per belt
    let drills_per_belt = 30u64;
    let total = drills_per_belt;

    let mut parts = Vec::new();
    let mut bill = BTreeMap::new();
    bill.insert("electric-mining-drill".to_string(), total);
    bill.insert("small-electric-pole".to_string(), total / 4 + 1);

    for i in 0..total {
        let col = (i as i32) / 2;
        let row = (i % 2) as i32; // 0 = west, 1 = east
        let half_x = col * 4;
        let half_y = if row == 0 { -2 } else { 2 };

        parts.push(Part {
            role: format!("drill-{}", i).into(),
            entity: "electric-mining-drill".into(),
            half_size: Some(state.entity_half_size("electric-mining-drill")),
            offset: Offset { half_x, half_y },
            direction: Direction::North as u8,
            recipe: None,
            underground_half: None,
        });
    }

    let ports = vec![
        Port {
            id: "output".into(),
            mode: PortMode::InventoryOutput,
            item: ore.into(),
            offset: Offset { half_x: 2, half_y: 0 },
            direction: Direction::East as u8,
            lane: None,
            maximum: Rate::new(1, 1).unwrap(),
        },
    ];

    let left = -2i32;
    let right = (total as i32 / 2) * 4;
    let required_clearance = vec![
        Offset { half_x: left, half_y: -4 },
        Offset { half_x: right, half_y: -4 },
        Offset { half_x: left, half_y: 4 },
        Offset { half_x: right, half_y: 4 },
    ];

    let ore_per_tick = total as f64 * 0.5 / 60.0;

    let operation = OperatingContract {
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([(ore.into(), Rate::new(1, 2).unwrap())]),
        power_watts: total * 90000,
        fuel_per_tick: BTreeMap::new(),
        startup_latency_ticks: 60,
        startup_items: BTreeMap::new(),
        local_buffer_capacity: BTreeMap::new(),
        required_research: vec!["electric-energy-distribution-1".into()],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };

    Ok(ModuleDesign {
        schema: 1, id: String::new(), family: ModuleFamily::DrillArray,
        generator_version: 1, origin: KnowledgeOrigin::Extracted,
        parameters: parameters.clone(),
        prototype_hash: "drill-array-v1".into(),
        mod_versions: BTreeMap::new(), parents: vec![], training_manifest: None,
        parts, ports, required_clearance, expansion_space: vec![], bill,
        precedence: vec![], operation,
    })
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



fn extract_fluid_manufacturing(
    state: &PlanState,
    family: ModuleFamily,
    parameters: &ModuleParameters,
) -> Result<ModuleDesign, ModuleError> {
    let item = &parameters.item;
    let recipes = state.base().entity_graph.recipes();
    let recipe = recipes.get(item).ok_or_else(|| ModuleError::Unsupported(format!(
        "fluid-mfg: no recipe for '{item}'"
    )))?;
    let category = recipe.category.as_str();
    let entity = match category {
        "oil-processing" => "oil-refinery",
        "chemistry" | "crafting-with-fluid" => "chemical-plant",
        _ => return Err(ModuleError::Unsupported(format!(
            "fluid-mfg: unsupported category '{category}' for '{item}'"
        ))),
    };

    let mut bill = BTreeMap::new();
    bill.insert(entity.to_string(), 1u64);
    bill.insert("pipe".to_string(), 4u64);
    bill.insert("small-electric-pole".to_string(), 1u64);
    if let Some(ref ing) = recipe.ingredients {
        for ingr in ing.iter() {
            if ingr.ingredient_type == "item" {
                *bill.entry(ingr.name.clone()).or_insert(0u64) += ingr.amount as u64;
            }
        }
    }

    let output = if let Some(p) = recipe.products.first() {
        p.name.clone()
    } else {
        item.clone()
    };

    let is_3x3 = entity == "oil-refinery";
    let half = if is_3x3 { 3 } else { 2 };

    let parts = vec![
        Part {
            role: entity.into(),
            entity: entity.into(),
            half_size: Some(Offset { half_x: half, half_y: half }),
            offset: Offset { half_x: 0, half_y: 0 },
            direction: Direction::North as u8,
            recipe: Some(item.clone()),
            underground_half: None,
        },
    ];

    let ports = vec![
        Port {
            id: "fluid-input".into(),
            mode: PortMode::BeltInput,
            item: "pipe".into(),
            offset: Offset { half_x: -2, half_y: -3 },
            direction: Direction::West as u8,
            lane: None,
            maximum: Rate::new(1, 60).unwrap(),
        },
        Port {
            id: "fluid-output".into(),
            mode: PortMode::InventoryOutput,
            item: output.clone(),
            offset: Offset { half_x: 2, half_y: 3 },
            direction: Direction::North as u8,
            lane: None,
            maximum: Rate::new(1, 60).unwrap(),
        },
    ];

    let required_clearance = vec![
        Offset { half_x: -3, half_y: -3 },
        Offset { half_x: 3, half_y: -3 },
        Offset { half_x: -3, half_y: 3 },
        Offset { half_x: 3, half_y: 3 },
    ];

    let power = if is_3x3 { 420000u64 } else { 210000u64 };
    let energy_ticks = (recipe.energy.raw() * 60.0) as u64;

    let mut inps = BTreeMap::new();
    if let Some(ref ing) = recipe.ingredients {
        for ingr in ing.iter() {
            inps.insert(ingr.name.clone(), Rate::new(ingr.amount as u64, energy_ticks).unwrap());
        }
    }
    let mut outs = BTreeMap::new();
    outs.insert(output.clone(), Rate::new(1, energy_ticks).unwrap());

    let mut buffer = BTreeMap::new();
    buffer.insert(output, 10u64);

    let operation = OperatingContract {
        inputs: inps,
        outputs: outs,
        power_watts: power,
        fuel_per_tick: BTreeMap::new(),
        startup_latency_ticks: energy_ticks,
        startup_items: BTreeMap::new(),
        local_buffer_capacity: buffer,
        required_research: vec![],
        required_surface: "nauvis".into(),
        unsupported_mechanisms: vec![],
    };

    Ok(ModuleDesign {
        schema: 1,
        id: String::new(),
        family,
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
        precedence: vec![],
        operation,
    })
}
