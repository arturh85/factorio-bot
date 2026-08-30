use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;

use crate::factorio::util::{add_to_rect, rect_fields};
use crate::factorio::world::FactorioWorld;
use crate::graph::entity_graph::EntityGraph;
use crate::types::{
    Direction, EntityName, FactorioItemPrototype, FactorioRecipe, FactorioTile, Position, Rect,
};
use crate::types::{FactorioEntity, FactorioEntityPrototype};
use miette::Result;

pub fn entity_graph_from(entities: Vec<FactorioEntity>) -> Result<EntityGraph> {
    let prototypes = fixture_entity_prototypes();
    let graph = EntityGraph::new(Arc::new(prototypes), Arc::new(DashMap::new()));
    graph.add(entities, None)?;
    graph.connect()?;
    Ok(graph)
}

pub fn fixture_entity_prototypes() -> DashMap<String, FactorioEntityPrototype> {
    let prototypes: DashMap<String, FactorioEntityPrototype> =
        serde_json::from_str(include_str!("../tests/entity-prototype-fixtures.json"))
            .expect("failed to parse fixture");
    prototypes
}

pub fn fixture_item_prototypes() -> DashMap<String, FactorioItemPrototype> {
    let prototypes: HashMap<String, FactorioItemPrototype> =
        serde_json::from_str(include_str!("../tests/item-prototype-fixtures.json"))
            .expect("failed to parse fixture");
    let dashmap: DashMap<String, FactorioItemPrototype> = DashMap::new();
    for prototype in prototypes {
        dashmap.insert(prototype.0, prototype.1);
    }
    dashmap
}
pub fn fixture_recipes() -> DashMap<String, FactorioRecipe> {
    let recipes: HashMap<String, FactorioRecipe> =
        serde_json::from_str(include_str!("../tests/recipes-fixtures.json"))
            .expect("failed to parse fixture");
    let dashmap: DashMap<String, FactorioRecipe> = DashMap::new();
    for prototype in recipes {
        dashmap.insert(prototype.0, prototype.1);
    }
    dashmap
}

pub fn spawn_trees(entities: &mut Vec<FactorioEntity>, count: u32, around: Position) {
    let a_x = around.x() as i32;
    let a_y = around.y() as i32;
    let mut x = 0;
    let mut y = 0;
    let mut t;
    let mut dx = 0;
    let mut dy = -1;
    for _ in 0..count * 100 {
        entities.push(FactorioEntity::new_tree(&Position::new(
            (a_x + x) as f64,
            (a_y + y) as f64,
        )));
        if entities.len() >= count as usize {
            break;
        }
        if (x == y) || ((x < 0) && (x == -y)) || ((x > 0) && (x == 1 - y)) {
            t = dx;
            dx = -dy;
            dy = t;
        }
        x += dx;
        y += dy;
    }
}
pub fn spawn_rocks(entities: &mut Vec<FactorioEntity>, count: u32, around: Position, name: &str) {
    let a_x = around.x() as i32;
    let a_y = around.y() as i32;
    let mut x = 0;
    let mut y = 0;
    let mut t;
    let mut dx = 0;
    let mut dy = -1;
    for _ in 0..count * 100 {
        entities.push(FactorioEntity::new_rock(
            &Position::new((a_x + x) as f64, (a_y + y) as f64),
            name,
        ));
        if entities.len() >= count as usize {
            break;
        }
        if (x == y) || ((x < 0) && (x == -y)) || ((x > 0) && (x == 1 - y)) {
            t = dx;
            dx = -dy;
            dy = t;
        }
        x += dx;
        y += dy;
    }
}

pub fn spawn_ore(entities: &mut Vec<FactorioEntity>, rect: Rect, resource_name: &str) {
    for pos in rect_fields(&rect) {
        entities.push(FactorioEntity::new_resource(
            &pos,
            Direction::North,
            resource_name,
        ));
    }
}

pub fn spawn_water(tiles: &mut Vec<FactorioTile>, rect: Rect) {
    for pos in rect_fields(&rect) {
        tiles.push(FactorioTile {
            position: pos,
            name: EntityName::Water.to_string(),
            player_collidable: true,
            color: None,
        });
    }
}

pub fn fixture_world() -> FactorioWorld {
    let world = FactorioWorld::new();
    let entity_prototypes: Vec<FactorioEntityPrototype> = fixture_entity_prototypes()
        .iter()
        .map(|v| v.clone())
        .collect();
    let item_prototypes: Vec<FactorioItemPrototype> = fixture_item_prototypes()
        .iter()
        .map(|v| v.clone())
        .collect();
    let recipes: Vec<FactorioRecipe> = fixture_recipes().iter().map(|v| v.clone()).collect();
    world.update_entity_prototypes(entity_prototypes).unwrap();
    world.update_item_prototypes(item_prototypes).unwrap();
    world.update_recipes(recipes).unwrap();

    let mut entities: Vec<FactorioEntity> = vec![];
    let mut tiles: Vec<FactorioTile> = vec![];

    spawn_rocks(&mut entities, 3, Position::new(20., 20.), "rock-huge");
    spawn_rocks(&mut entities, 2, Position::new(40., 30.), "rock-big");
    spawn_trees(&mut entities, 100, Position::new(-20., -20.));
    spawn_ore(
        &mut entities,
        add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-40., 40.)),
        &EntityName::IronOre.to_string(),
    );
    spawn_ore(
        &mut entities,
        add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-40., 0.)),
        &EntityName::CopperOre.to_string(),
    );
    spawn_ore(
        &mut entities,
        add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-60., 0.)),
        &EntityName::Coal.to_string(),
    );
    spawn_ore(
        &mut entities,
        add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-80., 0.)),
        &EntityName::Stone.to_string(),
    );

    spawn_water(
        &mut tiles,
        add_to_rect(&Rect::from_wh(4., 4.), &Position::new(40., 40.)),
    );
    world.update_chunk_tiles(tiles).unwrap();
    world.update_chunk_entities(entities).unwrap();
    world
}
