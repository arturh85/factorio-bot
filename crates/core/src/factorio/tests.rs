use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;

use crate::draw::{draw_blocked_rects_mut, draw_resource_rects_mut};
use crate::factorio::entity_graph::EntityGraph;
use crate::factorio::util::{add_to_rect, rect_fields};
use crate::factorio::world::FactorioWorld;
use crate::types::{
    Direction, EntityName, FactorioItemPrototype, FactorioRecipe, FactorioTile, Position, Rect,
};
#[cfg(test)]
use crate::types::{FactorioEntity, FactorioEntityPrototype};
use image::RgbaImage;
use imageproc::drawing::draw_hollow_rect_mut;

pub fn entity_graph_from(entities: Vec<FactorioEntity>) -> anyhow::Result<EntityGraph> {
    let prototypes = fixture_entity_prototypes();
    let graph = EntityGraph::new(Arc::new(prototypes), Arc::new(DashMap::new()));
    graph.add(entities, None)?;
    graph.connect()?;
    Ok(graph)
}

pub fn fixture_entity_prototypes() -> DashMap<String, FactorioEntityPrototype> {
    let prototypes: HashMap<String, FactorioEntityPrototype> =
        serde_json::from_str(include_str!("../../tests/entity-prototype-fixtures.json"))
            .expect("failed to parse fixture");
    let dashmap: DashMap<String, FactorioEntityPrototype> = DashMap::new();
    for foo in prototypes {
        dashmap.insert(foo.0, foo.1);
    }
    dashmap
}

pub fn fixture_item_prototypes() -> DashMap<String, FactorioItemPrototype> {
    let prototypes: HashMap<String, FactorioItemPrototype> =
        serde_json::from_str(include_str!("../../tests/item-prototype-fixtures.json"))
            .expect("failed to parse fixture");
    let dashmap: DashMap<String, FactorioItemPrototype> = DashMap::new();
    for foo in prototypes {
        dashmap.insert(foo.0, foo.1);
    }
    dashmap
}
pub fn fixture_recipes() -> DashMap<String, FactorioRecipe> {
    let recipes: HashMap<String, FactorioRecipe> =
        serde_json::from_str(include_str!("../../tests/recipes-fixtures.json"))
            .expect("failed to parse fixture");
    let dashmap: DashMap<String, FactorioRecipe> = DashMap::new();
    for foo in recipes {
        dashmap.insert(foo.0, foo.1);
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

    spawn_trees(&mut entities, 100, Position::new(-20., -20.));
    spawn_ore(
        &mut entities,
        add_to_rect(&Rect::from_wh(10., 10.), &Position::new(0., 0.)),
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

pub fn draw_world(world: Arc<FactorioWorld>) {
    let image_width = 500.;
    let image_height = 500.;
    let bb_width = 200.;
    let bb_height = 200.;
    let mut buffer: RgbaImage = image::ImageBuffer::new(image_width as u32, image_height as u32);
    for (_x, _y, pixel) in buffer.enumerate_pixels_mut() {
        *pixel = image::Rgba([255, 255, 255, 255u8]);
    }
    let bounding_box = Rect::from_wh(bb_width, bb_height);
    let scaling_factor = image_width / bb_width;
    let resource_colors: HashMap<&str, image::Rgba<_>> = [
        ("iron-ore", image::Rgba([0u8, 140u8, 255u8, 255u8])),
        ("copper-ore", image::Rgba([255u8, 55u8, 0u8, 255u8])),
        ("coal", image::Rgba([0u8, 0u8, 0u8, 255u8])),
        ("stone", image::Rgba([150u8, 100u8, 80u8, 255u8])),
        ("uranium-ore", image::Rgba([100u8, 180u8, 0u8, 255u8])),
        ("crude-oil", image::Rgba([255u8, 0u8, 255u8, 255u8])),
    ]
    .iter()
    .cloned()
    .collect();
    draw_resource_rects_mut(
        &mut buffer,
        world.entity_graph.resource_tree(),
        &bounding_box,
        scaling_factor,
        resource_colors,
        image::Rgba([255u8, 0u8, 0u8, 255u8]),
    );
    draw_blocked_rects_mut(
        &mut buffer,
        world.entity_graph.blocked_tree(),
        &bounding_box,
        scaling_factor,
        image::Rgba([76u8, 175u8, 80u8, 255u8]),
        image::Rgba([255u8, 0u8, 0u8, 255u8]),
    );
    draw_hollow_rect_mut(
        &mut buffer,
        imageproc::rect::Rect::at((image_width / 2. - 1.) as i32, 0)
            .of_size(2, image_height as u32),
        image::Rgba([0u8, 0u8, 0u8, 255u8]),
    );
    draw_hollow_rect_mut(
        &mut buffer,
        imageproc::rect::Rect::at(0, (image_height / 2. - 1.) as i32)
            .of_size(image_width as u32, 2),
        image::Rgba([0u8, 0u8, 0u8, 255u8]),
    );
    buffer.save("tests/world.png").unwrap();
}
