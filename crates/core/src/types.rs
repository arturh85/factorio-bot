use std::collections::{BTreeMap, HashMap};
use std::ops::Sub;
use std::str::FromStr;
use std::sync::Arc;

use dashmap::DashMap;
use euclid::{Point2D, Size2D};
use factorio_blueprint::objects::Entity;
use noisy_float::prelude::*;
use num_traits::ToPrimitive;
use serde_json::Value;
use typescript_definitions::TypeScriptify;

use crate::errors::RectInvalid;
use crate::factorio::util::{add_to_rect, add_to_rect_turned, calculate_distance, rect_floor_ceil};
use crate::graph::entity_graph::QuadTreeRect;
use crate::num_traits::FromPrimitive;
use miette::{IntoDiagnostic, Result};
use rlua::{Context, MultiValue};

pub type FactorioInventory = HashMap<String, u32>;

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioRecipe {
    pub name: String,
    pub valid: bool,
    pub enabled: bool,
    pub category: String,
    pub ingredients: Option<Vec<FactorioIngredient>>,
    pub products: Vec<FactorioProduct>,
    pub hidden: bool,
    pub energy: Box<R64>,
    pub order: String,
    pub group: String,
    pub subgroup: String,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FactorioBlueprintInfo {
    pub label: String,
    pub blueprint: String,
    pub width: u16,
    pub height: u16,
    pub rect: Rect,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioIngredient {
    pub name: String,
    pub ingredient_type: String,
    pub amount: u32,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioProduct {
    pub name: String,
    pub product_type: String,
    pub amount: u32,
    pub probability: Box<R64>,
}

pub type PlayerId = u8;
pub type ActionId = u32;

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioPlayer {
    pub player_id: PlayerId,
    pub position: Position,
    pub main_inventory: BTreeMap<String, u32>,
    pub build_distance: u32,          // for place_entity
    pub reach_distance: u32,          // for insert_to_inventory
    pub drop_item_distance: u32,      // remove_from_inventory
    pub item_pickup_distance: u64,    // not in use, for picking up items from the ground
    pub loot_pickup_distance: u64, // not in use, for picking up items from the ground automatically
    pub resource_reach_distance: u64, // for mine
}

impl Default for FactorioPlayer {
    fn default() -> Self {
        FactorioPlayer {
            player_id: 0,
            position: Position::default(),
            main_inventory: BTreeMap::new(),
            build_distance: 10,
            reach_distance: 10,
            drop_item_distance: 10,
            item_pickup_distance: 1,
            loot_pickup_distance: 2,
            resource_reach_distance: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequestEntity {
    pub name: String,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct InventoryResponse {
    pub name: String,
    pub position: Position,
    pub output_inventory: Box<Option<BTreeMap<String, u32>>>,
    pub fuel_inventory: Box<Option<BTreeMap<String, u32>>>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ChunkPosition {
    pub x: i32,
    pub y: i32,
}

impl From<&Pos> for ChunkPosition {
    fn from(pos: &Pos) -> ChunkPosition {
        ChunkPosition {
            x: pos.0 / 32,
            y: pos.1 / 32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub fn distance(&self, other: &Position) -> f64 {
        self.x.sub(other.x).abs() + self.y.sub(other.y).abs()
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "[{}, {}]",
            (self.x() * 100.).round() / 100.,
            (self.y() * 100.).round() / 100.
        ))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Pos(pub i32, pub i32);

impl Pos {
    pub fn distance(&self, other: &Pos) -> u32 {
        (self.0.abs_diff(other.0) + self.1.abs_diff(other.1)) as u32
    }
}

impl From<Point2D<f32, Rect>> for Position {
    fn from(point: Point2D<f32, Rect>) -> Position {
        Position::new(point.x as f64, point.y as f64)
    }
}

impl From<Position> for Point2D<f32, Rect> {
    fn from(pos: Position) -> Point2D<f32, Rect> {
        Point2D::new(pos.x() as f32, pos.y() as f32)
    }
}

impl From<&Position> for Pos {
    fn from(position: &Position) -> Pos {
        Pos(position.x().floor() as i32, position.y().floor() as i32)
    }
}

impl From<&Pos> for Position {
    fn from(pos: &Pos) -> Position {
        Position::new(pos.0 as f64, pos.1 as f64)
    }
}

#[derive(Primitive, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    North = 0,
    NorthEast = 1,
    East = 2,
    SouthEast = 3,
    South = 4,
    SouthWest = 5,
    West = 6,
    NorthWest = 7,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AreaFilter {
    Rect(Rect),
    PositionRadius((Position, Option<f64>)),
}

impl Direction {
    pub fn all() -> Vec<Direction> {
        (0..8).map(|n| Direction::from_u8(n).unwrap()).collect()
    }
    pub fn orthogonal() -> Vec<Direction> {
        (0..8)
            .filter(|n| n % 2 == 0)
            .map(|n| Direction::from_u8(n).unwrap())
            .collect()
    }
    pub fn opposite(&self) -> Direction {
        Direction::from_u8((Direction::to_u8(self).unwrap() + 4) % 8).unwrap()
    }
    pub fn clockwise(&self) -> Direction {
        Direction::from_u8((Direction::to_u8(self).unwrap() + 2) % 8).unwrap()
    }
}

impl Default for Direction {
    fn default() -> Self {
        Direction::North
    }
}

impl Position {
    pub fn new(x: f64, y: f64) -> Position {
        Position { x, y }
    }

    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn add(&self, position: &Position) -> Position {
        Position::new(self.x() + position.x(), self.y() + position.y())
    }

    pub fn turn(&self, direction: Direction) -> Position {
        match direction {
            Direction::North => self.clone(),
            Direction::East => self
                .rotate_clockwise()
                .rotate_clockwise()
                .rotate_clockwise(),
            Direction::South => self.rotate_clockwise().rotate_clockwise(),
            Direction::West => self.rotate_clockwise(),
            _ => panic!("diagonal turning not supported"),
        }
    }

    /*
    https://limnu.com/sketch-easy-90-degree-rotate-vectors/#:~:text=Normally%20rotating%20vectors%20involves%20matrix,swap%20X%20and%20Y%20values.
    Normally rotating vectors involves matrix math, but there’s a really simple trick for rotating a 2D vector by 90° clockwise:
    just multiply the X part of the vector by -1, and then swap X and Y values.
     */
    pub fn rotate_clockwise(&self) -> Position {
        Position::new(self.y(), self.x() * -1.0)
    }
}

impl Default for Position {
    fn default() -> Self {
        Position::new(0., 0.)
    }
}

impl FromStr for Position {
    type Err = miette::Report;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = str.split(',').collect();
        if parts.len() != 2 {
            return Err(RectInvalid {
                invalid_input: str.into(),
            }
            .into());
        }
        Ok(Position::new(
            parts[0].parse().into_diagnostic()?,
            parts[1].parse().into_diagnostic()?,
        ))
    }
}

#[derive(Debug, Clone, Default, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Rect {
    pub left_top: Position,
    pub right_bottom: Position,
}

impl Rect {
    pub fn new(left_top: &Position, right_bottom: &Position) -> Rect {
        Rect {
            left_top: left_top.clone(),
            right_bottom: right_bottom.clone(),
        }
    }
    pub fn contains(&self, position: &Position) -> bool {
        position.x() > self.left_top.x()
            && position.x() < self.right_bottom.x()
            && position.y() > self.left_top.y()
            && position.y() < self.right_bottom.y()
    }

    pub fn from_wh(width: f64, height: f64) -> Rect {
        Rect {
            left_top: Position::new(-width / 2., -height / 2.),
            right_bottom: Position::new(width / 2., height / 2.),
        }
    }
    pub fn width(&self) -> f64 {
        self.right_bottom.x() - self.left_top.x()
    }
    pub fn height(&self) -> f64 {
        self.right_bottom.y() - self.left_top.y()
    }
    pub fn center(&self) -> Position {
        Position::new(
            (self.left_top.x() + self.right_bottom.x()) / 2.,
            (self.left_top.y() + self.right_bottom.y()) / 2.,
        )
    }
    pub fn rotate_clockwise(&self) -> Rect {
        Rect::from_wh(self.height(), self.width())
    }
}

#[allow(clippy::from_over_into)]
impl Into<QuadTreeRect> for Rect {
    fn into(self) -> QuadTreeRect {
        QuadTreeRect::new(
            self.left_top.clone().into(),
            Size2D::new(self.width() as f32, self.height() as f32),
        )
    }
}

impl FromStr for Rect {
    type Err = miette::Report;
    fn from_str(str: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = str.split(';').collect();
        if parts.len() != 2 {
            return Err(RectInvalid {
                invalid_input: str.into(),
            }
            .into());
        }
        Ok(Rect {
            left_top: parts[0].parse()?,
            right_bottom: parts[1].parse()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioTile {
    pub name: String,
    pub player_collidable: bool,
    pub position: Position,
    pub color: Option<[u8; 4]>,
}

#[derive(Debug, Clone, Default, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FactorioChunk {
    pub entities: Vec<FactorioEntity>,
    // pub tiles: Vec<FactorioTile>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChunkObject {
    pub name: String,
    pub position: Position,
    pub direction: String,
    pub bounding_box: Rect,
    pub output_inventory: Box<Option<BTreeMap<String, u32>>>,
    pub fuel_inventory: Box<Option<BTreeMap<String, u32>>>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChunkResource {
    pub name: String,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioTechnology {
    pub name: String,
    pub enabled: bool,
    pub upgrade: bool,
    pub researched: bool,
    pub prerequisites: Option<Vec<String>>,
    pub research_unit_ingredients: Vec<FactorioIngredient>,
    pub research_unit_count: u64,
    pub research_unit_energy: Box<R64>,
    pub order: String,
    pub level: u32,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioForce {
    pub name: String,
    pub force_id: u32,
    // The current technology in research, or None if no research is currently ongoing.
    pub current_research: Option<String>,
    // Progress of current research, as a number in range [0, 1].
    pub research_progress: Option<Box<R64>>,
    pub technologies: Box<BTreeMap<String, FactorioTechnology>>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioGraphic {
    pub entity_name: String,
    pub image_path: String,
    pub width: u32,
    pub height: u32, // FIXME: add whatever this is, width&height are the first
                     // 1:1:0:0:0:0:1

                     //picspec.filename..":"..picspec.width..":"..picspec.height..":"..shiftx..":"..shifty..":"..xx..":"..yy..":"..scale
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioFluidBoxPrototype {
    pub pipe_connections: Box<Option<Vec<FactorioFluidBoxConnection>>>,
    pub production_type: String,
}

// #[derive(
//     EnumString,
//     Display,
//     Debug,
//     Clone,
//     TypeScriptify,
//     PartialEq,
//     Serialize,
//     Deserialize,
//     Hash,
//     Eq,
// )]
// #[strum(serialize_all = "kebab-case")]
// #[serde(rename_all = "kebab-case")]
// pub enum FactorioFluidBoxConnectionType {
//     Input,
//     Output,
//     InputOutput,
// }
// #[derive(
//     EnumString,
//     Display,
//     Debug,
//     Clone,
//     TypeScriptify,
//     PartialEq,
//     Serialize,
//     Deserialize,
//     Hash,
//     Eq,
// )]
// #[strum(serialize_all = "kebab-case")]
// #[serde(rename_all = "kebab-case")]
// pub enum FactorioFluidBoxProductionType {
//     Input,
//     Output,
//     InputOutput,
//     None,
// }

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioFluidBoxConnection {
    pub max_underground_distance: Option<u32>,
    pub connection_type: String,
    pub positions: Vec<Position>,
}
#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioEntityPrototype {
    pub name: String,
    pub entity_type: String,
    pub collision_mask: Option<Vec<String>>,
    pub collision_box: Rect,
    pub mine_result: Option<BTreeMap<String, u32>>,
    pub mining_time: Option<f64>,
    pub mining_speed: Option<f64>,
    pub crafting_speed: Option<f64>,
    pub max_underground_distance: Option<u8>,
    pub fluidbox_prototypes: Option<Vec<FactorioFluidBoxPrototype>>,
}

#[derive(Debug, Clone, Default, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioEntity {
    pub name: String,
    pub entity_type: String,
    pub position: Position,
    pub bounding_box: Rect,
    pub direction: u8,
    pub drop_position: Option<Position>,
    pub pickup_position: Option<Position>, // only type = inserter
    pub output_inventory: Option<BTreeMap<String, u32>>,
    pub fuel_inventory: Option<BTreeMap<String, u32>>,
    pub amount: Option<u32>,        // only type = resource
    pub recipe: Option<String>,     // only CraftingMachines
    pub ghost_name: Option<String>, // only type = entity-ghost
    pub ghost_type: Option<String>, // only type = entity-ghost
}

impl crate::aabb_quadtree::Spatial<Rect> for FactorioEntity {
    fn aabb(&self) -> QuadTreeRect {
        self.bounding_box.clone().into()
    }
}

impl FactorioEntity {
    pub fn from_blueprint_entity(
        entity: Entity,
        prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    ) -> Result<Self> {
        let position: Position = entity.position.into();
        let direction: Option<Direction> = entity
            .direction
            .map(|d| Direction::from_u8(d % 8).expect("should always work"));
        Self::from_prototype(
            &*entity.name,
            position,
            direction,
            entity.pickup_position.map(|p| p.into()),
            entity.drop_position.map(|p| p.into()),
            prototypes,
        )
    }

    pub fn from_prototype(
        name: &str,
        position: Position,
        direction: Option<Direction>,
        pickup_position: Option<Position>,
        drop_position: Option<Position>,
        prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    ) -> Result<Self> {
        let direction = direction.unwrap_or(Direction::North);
        if let Some(prototype) = prototypes.get(name) {
            Ok(FactorioEntity {
                bounding_box: add_to_rect_turned(&prototype.collision_box, &position, direction),
                position,
                direction: direction.to_u8().unwrap(),
                name: name.to_owned(),
                entity_type: prototype.entity_type.clone(),
                pickup_position,
                drop_position,
                ..Default::default()
            })
        } else {
            Ok(FactorioEntity {
                position,
                direction: direction.to_u8().unwrap(),
                name: name.to_owned(),
                pickup_position,
                drop_position,
                ..Default::default()
            })
        }
    }
    pub fn new_transport_belt(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::TransportBelt.to_string(),
            entity_type: EntityType::TransportBelt.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_splitter(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::Splitter.to_string(),
            entity_type: EntityType::Splitter.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.78, 0.78), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_inserter(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::Inserter.to_string(),
            entity_type: EntityType::Inserter.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.78, 0.78), position, direction),
            direction: direction.to_u8().unwrap(),
            drop_position: Some(position.add(&Position::new(0., 1.).turn(direction))),
            pickup_position: Some(position.add(&Position::new(0., -1.).turn(direction))),
            ..Default::default()
        }
    }
    pub fn new_burner_mining_drill(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::BurnerMiningDrill.to_string(),
            entity_type: EntityType::MiningDrill.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.8, 1.8), position, direction),
            direction: direction.to_u8().unwrap(),
            drop_position: Some(position.add(&Position::new(-0.5, -1.296875).turn(direction))),
            ..Default::default()
        }
    }
    pub fn new_electric_mining_drill(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::ElectricMiningDrill.to_string(),
            entity_type: EntityType::MiningDrill.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.8, 1.8), position, direction),
            direction: direction.to_u8().unwrap(),
            drop_position: Some(position.add(&Position::new(0., -2.).turn(direction))),
            ..Default::default()
        }
    }
    pub fn new_resource(position: &Position, direction: Direction, name: &str) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: EntityType::Resource.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_tree(position: &Position) -> FactorioEntity {
        FactorioEntity {
            name: "tree-42".into(),
            entity_type: EntityType::Tree.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect(&Rect::from_wh(0.8, 0.8), position),
            ..Default::default()
        }
    }
    pub fn new_rock(position: &Position, name: &str) -> FactorioEntity {
        FactorioEntity {
            name: name.to_owned(),
            entity_type: EntityType::SimpleEntity.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect(&Rect::from_wh(1.2, 1.2), position),
            ..Default::default()
        }
    }
    pub fn new_coal(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::Coal.to_string(),
            entity_type: EntityType::Resource.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(0.8, 0.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }
    pub fn new_stone_furnace(position: &Position, direction: Direction) -> FactorioEntity {
        FactorioEntity {
            name: EntityName::StoneFurnace.to_string(),
            entity_type: EntityType::Furnace.to_string(),
            position: position.clone(),
            bounding_box: add_to_rect_turned(&Rect::from_wh(1.8, 1.8), position, direction),
            direction: direction.to_u8().unwrap(),
            ..Default::default()
        }
    }

    pub fn is_minable(&self) -> bool {
        self.entity_type == EntityType::Tree.to_string()
            || self.entity_type == EntityType::SimpleEntity.to_string()
    }
}

impl From<factorio_blueprint::objects::Position> for Position {
    fn from(pos: factorio_blueprint::objects::Position) -> Self {
        Position {
            x: f64::from(pos.x),
            y: f64::from(pos.y),
        }
    }
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct FactorioItemPrototype {
    pub name: String,
    pub item_type: String,
    pub stack_size: u32,
    pub fuel_value: u64,
    pub place_result: String,
    pub group: String,
    pub subgroup: String,
}

#[derive(EnumString, Display, Debug)]
#[strum(serialize_all = "kebab-case")]
pub enum EntityName {
    // raw resources
    Water,
    Wood,
    Stone,
    Coal,
    IronOre,
    CopperOre,
    UraniumOre,
    CrudeOil,

    // processed
    StoneBrick,
    CopperPlate,
    IronPlate,
    Steel,

    // entities
    StoneFurnace,
    Inserter,
    BurnerMiningDrill,
    TransportBelt,
    Splitter,
    ElectricMiningDrill,
    Pumpjack,
}

#[derive(EnumString, Display, Debug, PartialEq, Clone, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
pub enum EntityType {
    AssemblingMachine,
    LogisticContainer,
    Boiler,
    Lab,
    Container,
    Resource,
    SimpleEntity,
    Tree,
    Inserter,
    MiningDrill,
    Furnace,
    TransportBelt,
    Splitter,
    UndergroundBelt,
    Pipe,
    PipeToGround,
    StorageTank,
    OffshorePump,
    FlyingText,
    StraightRail,
    CurvedRail,
    Fish,
}

impl EntityType {
    pub fn is_fluid_input(&self) -> bool {
        *self == EntityType::Pipe
            || *self == EntityType::StorageTank
            || *self == EntityType::PipeToGround
            || *self == EntityType::Boiler
    }
}

#[derive(Debug)]
pub struct ResourcePatch {
    pub name: String,
    pub id: u32,
    pub rect: Rect,
    pub elements: Vec<Position>,
}

impl ResourcePatch {
    pub fn contains(&self, pos: Pos) -> bool {
        self.elements
            .iter()
            .map(|e| e.into())
            .any(|x: Pos| x == pos)
    }
    pub fn find_free_rect(
        &self,
        width: u32,
        height: u32,
        near: &Position,
        // blocked: &ReadHandle<Pos, bool>,
    ) -> Option<Rect> {
        let mut elements = self.elements.clone();
        elements.sort_by(|a, b| {
            let da = r64(calculate_distance(a, near));
            let db = r64(calculate_distance(b, near));
            da.cmp(&db)
        });

        let mut element_map: HashMap<Pos, bool> = HashMap::new();
        for element in &elements {
            element_map.insert(element.into(), true);
        }
        for element in &elements {
            let mut invalid = false;
            for y in 0i32..height as i32 {
                for x in 0i32..width as i32 {
                    let pos = Pos(element.x() as i32 + x, element.y() as i32 + y);
                    // let blocked_pos = blocked.get_one(&pos);
                    // if blocked_pos.is_some() && !*blocked_pos.unwrap() {
                    //     invalid = true;
                    //     break;
                    // }
                    if element_map.get(&pos).is_none() {
                        invalid = true;
                        break;
                    }
                }
                if invalid {
                    break;
                }
            }
            if !invalid {
                return Some(rect_floor_ceil(&Rect {
                    left_top: element.clone(),
                    right_bottom: Position::new(
                        element.x() + width as f64,
                        element.y() + height as f64,
                    ),
                }));
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct InventoryItem {
    pub name: String,
    pub count: u32,
}

impl InventoryItem {
    pub fn new(name: &str, count: u32) -> InventoryItem {
        InventoryItem {
            name: name.into(),
            count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InventoryLocation {
    pub entity_name: String,
    pub position: Position,
    pub inventory_type: u32,
}

#[derive(Debug, Clone)]
pub struct EntityPlacement {
    pub item_name: String,
    pub position: Position,
    pub direction: Direction,
}

#[derive(Debug, Clone)]
pub struct PositionRadius {
    pub position: Position,
    pub radius: f64,
}

impl PositionRadius {
    pub fn new(x: f64, y: f64, radius: f64) -> PositionRadius {
        PositionRadius {
            position: Position::new(x, y),
            radius,
        }
    }
    pub fn from_position(pos: &Position, radius: f64) -> PositionRadius {
        PositionRadius {
            position: pos.clone(),
            radius,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MineTarget {
    pub position: Position,
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FactorioResult {
    pub success: bool,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlaceEntityResult {
    pub player: FactorioPlayer,
    pub entity: FactorioEntity,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct PlaceEntitiesResult {
    pub player: FactorioPlayer,
    pub entities: Vec<FactorioEntity>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PlayerChangedDistanceEvent {
    pub player_id: PlayerId,
    pub build_distance: u32,
    pub reach_distance: u32,
    pub drop_item_distance: u32,
    pub item_pickup_distance: u64,
    pub loot_pickup_distance: u64,
    pub resource_reach_distance: u64,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlayerChangedPositionEvent {
    pub player_id: PlayerId,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PlayerChangedMainInventoryEvent {
    pub player_id: PlayerId,
    pub main_inventory: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PlayerLeftEvent {
    pub player_id: PlayerId,
}

#[derive(Debug, Clone, PartialEq, TypeScriptify, Serialize, Deserialize, Hash, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PrimeVueTreeNode {
    pub key: String,
    pub label: String,
    pub leaf: bool,
    pub children: Vec<PrimeVueTreeNode>,
}

impl rlua::ToLua<'_> for InventoryResponse {
    fn to_lua(self, lua: Context) -> rlua::Result<rlua::Value> {
        rlua_serde::to_value(lua, self)
    }
}

impl<'lua> rlua::FromLuaMulti<'lua> for Position {
    fn from_lua_multi(values: MultiValue<'_>, _lua: Context<'_>) -> rlua::Result<Self> {
        let values: Vec<&rlua::Value> = values.iter().collect();
        if let rlua::Value::Number(x) = values[0] {
            if let rlua::Value::Number(y) = values[1] {
                Ok(Position::new(*x, *y))
            } else {
                Err(rlua::Error::RuntimeError("invalid position".into()))
            }
        } else {
            Err(rlua::Error::RuntimeError("invalid position".into()))
        }
    }
}

impl rlua::ToLua<'_> for Rect {
    fn to_lua(self, lua: Context) -> rlua::Result<rlua::Value> {
        rlua_serde::to_value(lua, self)
    }
}

//
// impl<'lua> rlua::FromLuaMulti<'lua> for Rect {
//     fn from_lua_multi(values: MultiValue<'_>, _lua: Context<'_>) -> rlua::Result<Self> {
//         let values: Vec<&rlua::Value> = values.iter().collect();
//         if let rlua::Value::Number(x) = values[0] {
//             if let rlua::Value::Number(y) = values[1] {
//                 Ok(Position::new(*x, *y))
//             } else {
//                 Err(rlua::Error::RuntimeError("invalid position".into()))
//             }
//         } else {
//             Err(rlua::Error::RuntimeError("invalid position".into()))
//         }
//     }
// }

impl rlua::ToLua<'_> for FactorioEntity {
    fn to_lua(self, lua: Context) -> rlua::Result<rlua::Value> {
        rlua_serde::to_value(lua, self)
    }
}
