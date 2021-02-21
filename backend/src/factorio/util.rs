use crate::factorio::entity_graph::QuadTreeRect;
use crate::types::{
    Direction, FactorioEntity, FactorioEntityPrototype, FactorioTile, Pos, Position, Rect,
};
use dashmap::DashMap;
use factorio_blueprint::BlueprintCodec;
use factorio_blueprint::Container::{Blueprint, BlueprintBook};
use human_sort::compare;
use itertools::Itertools;
use num_traits::ToPrimitive;
use pathfinding::prelude::astar;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

pub fn hashmap_to_lua(map: HashMap<String, String>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in map {
        parts.push(String::from(&format!("{}={}", k, v)));
    }
    format!("{{{}}}", parts.join(","))
}

pub fn value_to_lua(value: &Value) -> String {
    match value {
        Value::Null => "nil".into(),
        Value::Bool(bool) => bool.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(string) => str_to_lua(&string),
        Value::Array(vec) => format!(
            "{{ {} }}",
            vec.iter()
                .map(|value| value_to_lua(&value))
                .collect::<Vec<String>>()
                .join(", ")
        ),
        Value::Object(map) => {
            let mut parts: Vec<String> = Vec::new();
            for (k, v) in map {
                parts.push(String::from(&format!("{}={}", k, value_to_lua(v))));
            }
            format!("{{{}}}", parts.join(","))
        }
    }
}

pub fn position_to_lua(position: &Position) -> String {
    format!("{{{},{}}}", position.x, position.y)
}

pub fn rect_to_lua(rect: &Rect) -> String {
    format!(
        "{{{},{}}}",
        position_to_lua(&rect.left_top),
        position_to_lua(&rect.right_bottom)
    )
}

pub fn vec_to_lua(vec: Vec<String>) -> String {
    format!("{{ {} }}", vec.join(", "))
}

pub fn str_to_lua(str: &str) -> String {
    format!("'{}'", str)
}

pub fn calculate_distance(pos1: &Position, pos2: &Position) -> f64 {
    let x = pos1.x() - pos2.x();
    let y = pos1.y() - pos2.y();
    (x * x + y * y).sqrt()
}

pub fn move_position(pos: &Position, direction: Direction, offset: f64) -> Position {
    match direction {
        Direction::North => Position::new(pos.x(), pos.y() - offset),
        Direction::NorthWest => Position::new(pos.x() - offset, pos.y() - offset),
        Direction::NorthEast => Position::new(pos.x() + offset, pos.y() - offset),
        Direction::South => Position::new(pos.x(), pos.y() + offset),
        Direction::SouthWest => Position::new(pos.x() - offset, pos.y() + offset),
        Direction::SouthEast => Position::new(pos.x() + offset, pos.y() + offset),
        Direction::West => Position::new(pos.x() - offset, pos.y()),
        Direction::East => Position::new(pos.x() + offset, pos.y()),
    }
}

pub fn move_pos(pos: &Pos, direction: Direction, offset: i32) -> Pos {
    match direction {
        Direction::North => Pos(pos.0, pos.1 - offset),
        Direction::NorthWest => Pos(pos.0 - offset, pos.1 - offset),
        Direction::NorthEast => Pos(pos.0 + offset, pos.1 - offset),
        Direction::South => Pos(pos.0, pos.1 + offset),
        Direction::SouthWest => Pos(pos.0 - offset, pos.1 + offset),
        Direction::SouthEast => Pos(pos.0 + offset, pos.1 + offset),
        Direction::West => Pos(pos.0 - offset, pos.1),
        Direction::East => Pos(pos.0 + offset, pos.1),
    }
}

pub fn read_to_value(path: &PathBuf) -> anyhow::Result<Value> {
    let content = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(content.as_str())?)
}

pub fn write_value_to(value: &Value, path: &PathBuf) -> anyhow::Result<()> {
    let mut outfile = fs::File::create(&path)?;
    let bytes = serde_json::to_string(value).unwrap();
    outfile.write_all(bytes.as_ref())?;
    Ok(())
}

pub fn pad_rect(
    rect: &Rect,
    margin_left: f64,
    margin_top: f64,
    margin_right: f64,
    margin_bottom: f64,
) -> Rect {
    Rect {
        left_top: Position::new(
            rect.left_top.x() - margin_left,
            rect.left_top.y() - margin_top,
        ),
        right_bottom: Position::new(
            rect.right_bottom.x() + margin_right,
            rect.right_bottom.y() + margin_bottom,
        ),
    }
}

pub fn expand_rect_floor_ceil_div_2(rect: &Rect) -> Rect {
    Rect {
        left_top: Position::new(
            (rect.left_top.x() * 2.0).floor() / 2.0,
            (rect.left_top.y() * 2.0).floor() / 2.0,
        ),
        right_bottom: Position::new(
            (rect.right_bottom.x() * 2.0).ceil() / 2.0,
            (rect.right_bottom.y() * 2.0).ceil() / 2.0,
        ),
    }
}

pub fn rect_floor(rect: &Rect) -> Rect {
    Rect {
        left_top: Position::new((rect.left_top.x()).floor(), (rect.left_top.y()).floor()),
        right_bottom: Position::new(
            (rect.right_bottom.x()).floor(),
            (rect.right_bottom.y()).floor(),
        ),
    }
}

pub fn rect_floor_ceil(rect: &Rect) -> Rect {
    Rect {
        left_top: Position::new(rect.left_top.x().floor(), rect.left_top.y().floor()),
        right_bottom: Position::new(rect.right_bottom.x().ceil(), rect.right_bottom.y().ceil()),
    }
}
pub fn add_to_rect(rect: &Rect, position: &Position) -> Rect {
    Rect {
        left_top: Position::new(
            position.x() + rect.left_top.x(),
            position.y() + rect.left_top.y(),
        ),
        right_bottom: Position::new(
            position.x() + rect.right_bottom.x(),
            position.y() + rect.right_bottom.y(),
        ),
    }
}
pub fn add_to_rect_turned(rect: &Rect, position: &Position, direction: Direction) -> Rect {
    let rect = if direction == Direction::West || direction == Direction::East {
        rect.rotate_clockwise()
    } else {
        rect.clone()
    };
    add_to_rect(&rect, position)
}

pub fn expand_rect(total_rect: &mut Rect, rect: &Rect) {
    if rect.left_top.x() < total_rect.left_top.x() {
        total_rect.left_top = Position::new(rect.left_top.x(), total_rect.left_top.y());
    }
    if rect.left_top.y() < total_rect.left_top.y() {
        total_rect.left_top = Position::new(total_rect.left_top.x(), rect.left_top.y());
    }
    if rect.right_bottom.x() > total_rect.right_bottom.x() {
        total_rect.right_bottom = Position::new(rect.right_bottom.x(), total_rect.right_bottom.y());
    }
    if rect.right_bottom.y() > total_rect.right_bottom.y() {
        total_rect.right_bottom = Position::new(total_rect.right_bottom.x(), rect.right_bottom.y());
    }
}

pub fn blueprint_build_area(
    entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    blueprint: &str,
) -> Rect {
    let decoded = BlueprintCodec::decode_string(&blueprint).expect("failed to decode blueprint");
    let mut build_area = Rect::new(&Position::new(999.0, 999.0), &Position::new(-999.0, -999.0));
    match decoded {
        BlueprintBook(_blueprint_book) => {
            panic!("blueprint books are not supported!");
        }
        Blueprint(blueprint) => {
            for entity in blueprint.entities {
                let prototype = entity_prototypes.get(&entity.name);
                if let Some(prototype) = prototype {
                    let entity_position = Position::new(
                        entity.position.x.to_f64().unwrap(),
                        entity.position.y.to_f64().unwrap(),
                    );
                    let collision_box = expand_rect_floor_ceil_div_2(&prototype.collision_box);
                    let collision_rect = add_to_rect(&collision_box, &entity_position);
                    expand_rect(&mut build_area, &collision_rect)
                }
            }
        }
    };
    build_area
}

pub fn vector_length(vector: &Position) -> f64 {
    (vector.x() * vector.x() + vector.y() * vector.y()).sqrt()
}

pub fn vector_normalize(vector: &Position) -> Position {
    let len = vector_length(vector);
    Position::new(vector.x() / len, vector.y() / len)
}

pub fn vector_substract(a: &Position, b: &Position) -> Position {
    Position::new(a.x() - b.x(), a.y() - b.y())
}

pub fn vector_add(a: &Position, b: &Position) -> Position {
    Position::new(a.x() + b.x(), a.y() + b.y())
}

pub fn vector_multiply(a: &Position, len: f64) -> Position {
    Position::new(a.x() * len, a.y() * len)
}

pub fn span_rect(a: &Position, b: &Position, margin: f64) -> Rect {
    Rect::new(
        &Position::new(
            if a.x() < b.x() { a.x() } else { b.x() } - margin,
            if a.y() < b.y() { a.y() } else { b.y() } - margin,
        ),
        &Position::new(
            if a.x() > b.x() { a.x() } else { b.x() } + margin,
            if a.y() > b.y() { a.y() } else { b.y() } + margin,
        ),
    )
}

#[allow(clippy::ptr_arg)]
pub fn bounding_box(elements: &Vec<Position>) -> Option<Rect> {
    let min_max_positions = elements.iter().fold(
        None as Option<(Position, Position)>,
        |min_max, position| match min_max {
            None => Some((position.clone(), position.clone())),
            Some((a, b)) => Some((
                Position::new(position.x().min(a.x()), position.y().min(a.y())),
                Position::new(position.x().max(b.x()), position.y().max(b.y())),
            )),
        },
    );
    min_max_positions.map(|min_max_positions| Rect {
        left_top: min_max_positions.0,
        right_bottom: min_max_positions.1,
    })
}

#[allow(clippy::ptr_arg)]
pub fn map_blocked_tiles(
    entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    block_entities: &Vec<&FactorioEntity>,
    block_tiles: &Vec<&FactorioTile>,
) -> HashMap<Pos, ()> {
    let silent = true;
    let mut blocked: HashMap<Pos, ()> = HashMap::new();
    for tile in block_tiles {
        if tile.player_collidable {
            blocked.insert((&tile.position).into(), ());
        }
    }
    for entity in block_entities {
        if entity.entity_type == "character" || entity.entity_type == "resource" {
            continue;
        }
        match entity_prototypes.get(&entity.name) {
            Some(entity_prototype) => {
                let collision_box = add_to_rect(&entity_prototype.collision_box, &entity.position);
                let rect = rect_floor(&collision_box);
                let w = rect.width();
                let h = rect.height();
                if !silent {
                    info!(
                        "'{}' @ {:?} rect {:?} -- w {} h {}",
                        &entity.name, &entity.position, &rect, w, h
                    );
                    info!("cbox {:?}", &collision_box);
                }
                // if w > 1.0 || h > 1.0 {
                //     rect = pad_rect(&rect_floor(&rect), 0., 0., 1., 1.);
                // } else {
                //     rect = rect_floor(&rect);
                // }
                for position in rect_fields(&rect) {
                    if !silent {
                        info!(
                            "> position {} {} => {:?}",
                            position.x(),
                            position.y(),
                            Pos::from(&position)
                        );
                    }
                    blocked.insert((&position).into(), ());
                }
            }
            None => {
                if !silent {
                    info!(
                        "point position {} {} => {:?}",
                        entity.position.x(),
                        entity.position.y(),
                        Pos::from(&entity.position)
                    );
                }
                blocked.insert((&entity.position).into(), ());
            }
        }
    }
    blocked
}

#[allow(clippy::too_many_arguments)]
pub fn build_entity_path(
    entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    entity_name: &str,
    entity_type: &str,
    underground_entity_name: &str,
    underground_entity_type: &str,
    underground_max: u8,
    from_position: &Position,
    to_position: &Position,
    to_direction: Direction,
    block_entities: Vec<FactorioEntity>,
    block_tiles: Vec<FactorioTile>,
) -> anyhow::Result<Vec<FactorioEntity>> {
    let from_position: Pos = from_position.into();
    let to_position: Pos = to_position.into();
    let blocked = map_blocked_tiles(
        entity_prototypes,
        &block_entities.iter().collect(),
        &block_tiles.iter().collect(),
    );
    if blocked.contains_key(&from_position) {
        return Err(anyhow!("fromPosition is blocked",));
    }
    if blocked.contains_key(&to_position) {
        return Err(anyhow!("toPosition is blocked",));
    }
    // info!("start pathfinding");
    let path = astar(
        &(from_position.clone(), from_position.clone(), from_position),
        move |(last_last_pos, last_pos, current_pos)| {
            let mut options: Vec<((Pos, Pos, Pos), i32)> = vec![];
            let current_direction = relative_direction(last_pos, current_pos);
            let last_dist = last_pos.distance(last_last_pos);
            for direction in Direction::orthogonal() {
                // we cannot move in the opposite direction after we have moved
                if current_pos != last_pos && direction == current_direction.opposite() {
                    continue;
                }
                // after underground we need to go straight
                if last_dist > 1 && direction != current_direction {
                    continue;
                }
                for length in 1..(underground_max + 1) {
                    // after underground we cannot immediately underground again
                    if last_dist > 1 && length > 1 {
                        break;
                    }
                    let target: Pos = move_pos(current_pos, direction, length as i32);
                    if blocked.get(&target).is_none() {
                        options.push((
                            (last_pos.clone(), current_pos.clone(), target),
                            if length == 1 { 1 } else { length as i32 * 3 },
                        ));
                    }
                    if last_pos != current_pos {
                        // before underground we need a straight connection
                        if last_dist < 2 && direction != current_direction {
                            break;
                        }
                    }
                }
            }
            options
        },
        |(_, _, pos)| (pos.distance(&to_position) / 3) as i32,
        |(_, _, pos)| *pos == to_position,
    );
    // info!("finished pathfinding");
    let mirror_direction = underground_entity_type == "pipe-to-ground";
    match path {
        Some((path, _cost)) => {
            let mut result: Vec<FactorioEntity> = vec![];

            for i in 0..path.len() {
                let (_, last_pos, pos) = &path[i];
                let next: Option<&Pos> = if i + 1 < path.len() {
                    Some(&path[i + 1].2)
                } else {
                    None
                };

                let direction = if let Some(next) = next {
                    relative_direction(pos, next)
                } else {
                    to_direction
                };

                let distance = if last_pos != pos {
                    pos.distance(last_pos)
                } else {
                    1
                };

                if distance == 1 {
                    result.push(FactorioEntity {
                        name: entity_name.into(),
                        entity_type: entity_type.into(),
                        position: Position::new(pos.0 as f64, pos.1 as f64),
                        direction: if mirror_direction {
                            direction.opposite().to_u8().unwrap()
                        } else {
                            direction.to_u8().unwrap()
                        },
                        ..Default::default()
                    });
                } else {
                    result[i - 1].name = underground_entity_name.into();
                    result[i - 1].entity_type = underground_entity_type.into();
                    result.push(FactorioEntity {
                        name: underground_entity_name.into(),
                        entity_type: underground_entity_type.into(),
                        position: Position::new(pos.0 as f64, pos.1 as f64),
                        direction: if mirror_direction {
                            direction.to_u8().unwrap()
                        } else {
                            direction.opposite().to_u8().unwrap()
                        },
                        ..Default::default()
                    });
                }
            }
            Ok(result)
        }
        None => Err(anyhow!("no path found")),
    }
}

pub fn floor_position(position: &Position) -> Position {
    Position::new(position.x().floor(), position.y().floor())
}

pub fn position_equal(a: &Position, b: &Position) -> bool {
    (a.x().floor() - b.x().floor()).abs() < std::f64::EPSILON
        && (a.y().floor() - b.y().floor()).abs() < std::f64::EPSILON
}

pub fn rect_fields(rect: &Rect) -> Vec<Position> {
    let mut res = vec![];
    for y in rect.left_top.y().floor() as i32..=rect.right_bottom.y().floor() as i32 {
        for x in rect.left_top.x().floor() as i32..=rect.right_bottom.x().floor() as i32 {
            res.push(Position::new(x as f64, y as f64));
        }
    }
    res
}

// if "to" is west of "from" then returns Direction::West
#[allow(clippy::comparison_chain)]
pub fn relative_direction(from: &Pos, to: &Pos) -> Direction {
    if from.0 < to.0 {
        if from.1 > to.1 {
            Direction::NorthEast
        } else if from.1 < to.1 {
            Direction::SouthEast
        } else {
            Direction::East
        }
    } else if from.0 > to.0 {
        if from.1 > to.1 {
            Direction::NorthWest
        } else if from.1 < to.1 {
            Direction::SouthWest
        } else {
            Direction::West
        }
    } else if from.1 > to.1 {
        Direction::North
    } else if from.1 < to.1 {
        Direction::South
    } else {
        // if both positions are equal just return north
        Direction::North
    }
}

#[cfg(test)]
mod tests {
    use crate::types::Pos;

    use super::*;

    #[test]
    fn test_relative_direction() {
        let center = Pos(0, 0);
        let left = Pos(-1, 0);
        let right = Pos(1, 0);
        let top = Pos(0, -1);
        let bottom = Pos(0, 1);
        assert_eq!(relative_direction(&center, &right), Direction::East);
        assert_eq!(relative_direction(&center, &left), Direction::West);
        assert_eq!(relative_direction(&center, &top), Direction::North);
        assert_eq!(relative_direction(&center, &bottom), Direction::South);
        let left_top = Pos(-1, -1);
        assert_eq!(relative_direction(&center, &left_top), Direction::NorthWest);
        let right_top = Pos(1, -1);
        assert_eq!(
            relative_direction(&center, &right_top),
            Direction::NorthEast
        );
        let left_bottom = Pos(-1, 1);
        assert_eq!(
            relative_direction(&center, &left_bottom),
            Direction::SouthWest
        );
        let right_bottom = Pos(1, 1);
        assert_eq!(
            relative_direction(&center, &right_bottom),
            Direction::SouthEast
        );
    }
}

pub fn format_dotgraph(str: String) -> String {
    format!(
        "digraph {{\n{}\n}}\n",
        str.lines()
            .sorted_by(|a, b| {
                let a_is_edge = a.contains(" -> ");
                let b_is_edge = b.contains(" -> ");
                let ordering = a_is_edge.cmp(&b_is_edge);
                if ordering == Ordering::Equal {
                    compare(a, b)
                } else {
                    ordering
                }
            })
            .join("\n")
    )
}

pub fn scaled_draw_rect(
    bounding_box: &Rect,
    rect: QuadTreeRect,
    scaling_factor: f64,
) -> Option<imageproc::rect::Rect> {
    let width = (rect.size.width as f64 * scaling_factor).round() as u32;
    let height = (rect.size.height as f64 * scaling_factor).round() as u32;
    if width > 0 && height > 0 {
        let base_x = bounding_box.left_top.x();
        let base_y = bounding_box.left_top.y();
        Some(
            imageproc::rect::Rect::at(
                ((rect.origin.x as f64 - base_x) * scaling_factor).round() as i32,
                ((rect.origin.y as f64 - base_y) * scaling_factor).round() as i32,
            )
            .of_size(width, height),
        )
    } else {
        None
    }
}
