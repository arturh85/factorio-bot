use crate::aabb_quadtree::{ItemId, QuadTree};
use crate::factorio::util::{
    add_to_rect, bounding_box, format_dotgraph, move_position, rect_fields, rect_floor,
};
use crate::num_traits::FromPrimitive;
use crate::types::{
    Direction, EntityName, EntityType, FactorioEntity, FactorioEntityPrototype, FactorioRecipe,
    FactorioTile, Pos, Position, Rect, ResourcePatch,
};
use dashmap::DashMap;
use euclid::{Point2D, Rect as EuclidRect, Size2D};
use factorio_blueprint::{BlueprintCodec, Container};
use miette::Result;
use paris::error;
use parking_lot::{RwLock, RwLockReadGuard};
use petgraph::dot::{Config, Dot};
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::stable_graph::StableGraph;
use petgraph::visit::{Bfs, EdgeRef};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

pub struct EntityGraph {
    entity_graph: RwLock<EntityGraphInner>,
    blocked_tree: RwLock<BlockedQuadTree>,
    entity_tree: RwLock<EntityQuadTree>,
    tile_tree: RwLock<TileQuadTree>,
    entity_nodes: DashMap<ItemId, NodeIndex>,
    entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
    recipes: Arc<DashMap<String, FactorioRecipe>>,
    resources: DashMap<String, Vec<Pos>>,
    resource_tree: RwLock<ResourceQuadTree>,
}

impl EntityGraph {
    #[allow(clippy::new_without_default)]
    pub fn new(
        entity_prototypes: Arc<DashMap<String, FactorioEntityPrototype>>,
        recipes: Arc<DashMap<String, FactorioRecipe>>,
    ) -> Self {
        let max_area = QuadTreeRect::new(Point2D::new(-5120., -5120.), Size2D::new(10240., 10240.));
        EntityGraph {
            entity_prototypes,
            recipes,
            entity_graph: RwLock::new(EntityGraphInner::new()),
            entity_tree: RwLock::new(QuadTree::new(max_area, false, 32, 128, 128, 8)),
            blocked_tree: RwLock::new(QuadTree::new(max_area, true, 8, 64, 1024, 8)),
            resource_tree: RwLock::new(QuadTree::new(max_area, true, 8, 64, 1024, 8)),
            tile_tree: RwLock::new(QuadTree::new(max_area, false, 32, 128, 128, 8)),
            entity_nodes: DashMap::new(),
            resources: DashMap::new(),
        }
    }
    pub fn inner_graph(&self) -> RwLockReadGuard<'_, EntityGraphInner> {
        self.entity_graph.read()
    }
    pub fn inner_tree(&self) -> RwLockReadGuard<'_, EntityQuadTree> {
        self.entity_tree.read()
    }
    pub fn tile_tree(&self) -> RwLockReadGuard<'_, TileQuadTree> {
        self.tile_tree.read()
    }
    pub fn blocked_tree(&self) -> RwLockReadGuard<'_, BlockedQuadTree> {
        self.blocked_tree.read()
    }
    pub fn resource_tree(&self) -> RwLockReadGuard<'_, ResourceQuadTree> {
        self.resource_tree.read()
    }
    pub fn entity_prototypes(&self) -> Arc<DashMap<String, FactorioEntityPrototype>> {
        self.entity_prototypes.clone()
    }
    pub fn recipes(&self) -> Arc<DashMap<String, FactorioRecipe>> {
        self.recipes.clone()
    }

    pub fn node_by_id(&self, id: &ItemId) -> Option<NodeIndex> {
        self.entity_nodes.get(id).map(|e| *e.value())
    }

    pub fn resource_contains(&self, resource_name: &str, pos: Pos) -> bool {
        let elements = self.resources.get(resource_name);
        if let Some(elements) = elements {
            elements.contains(&pos)
        } else {
            false
        }
    }

    pub fn find_entities_in_radius(
        &self,
        search_center: Position,
        radius: f64,
        search_name: Option<String>,
        search_type: Option<String>,
    ) -> Vec<FactorioEntity> {
        let tree = self.entity_tree.read();
        let rect = QuadTreeRect::new(
            Position::new(search_center.x - radius, search_center.y - radius).into(),
            Size2D::new(2. * radius as f32, 2. * radius as f32),
        );
        let mut entities = vec![];
        for (entity, _rect, _item_id) in tree.query(rect) {
            if let Some(search_name) = search_name.as_ref() {
                if entity.name != *search_name {
                    continue;
                }
            }
            if let Some(search_type) = search_type.as_ref() {
                if entity.entity_type != *search_type {
                    continue;
                }
            }
            if entity.position.distance(&search_center) > radius {
                continue;
            }
            entities.push(entity.clone())
        }
        entities
    }

    pub fn resource_patches(&self, resource_name: &str) -> Vec<ResourcePatch> {
        let mut patches: Vec<ResourcePatch> = vec![];
        let mut positions_by_id: HashMap<Pos, Option<u32>> = HashMap::new();
        let resource = self.resources.get(resource_name);
        if resource.is_none() {
            warn!("no resource patch found for '{}'", resource_name);
            warn!(
                "available resource paths '{:?}'",
                self.resources
                    .iter()
                    .map(|f| f.key().to_string())
                    .collect::<Vec<_>>()
            );
            return vec![];
        }
        for point in resource.unwrap().iter() {
            positions_by_id.insert(point.clone(), None);
        }
        let mut next_id: u32 = 0;
        while let Some((next_pos, _)) = positions_by_id.iter().find(|(_, value)| value.is_none()) {
            next_id += 1;
            let next_pos = next_pos.clone();
            let mut stack: Vec<Pos> = vec![next_pos.clone()];
            positions_by_id.insert(next_pos.clone(), Some(next_id));
            while let Some(pos) = stack.pop() {
                for direction in Direction::all() {
                    let other: Pos = (&move_position(&(&pos).into(), direction, 1.0)).into();
                    if let Some(p) = positions_by_id.get(&other) {
                        if p.is_none() {
                            positions_by_id.insert(pos.clone(), Some(next_id));
                            stack.push(other);
                        }
                    }
                }
            }
        }
        for id in 1..=next_id {
            let mut elements: Vec<Position> = vec![];
            for (k, v) in &positions_by_id {
                if v.unwrap() == id {
                    elements.push(k.into());
                }
            }
            patches.push(ResourcePatch {
                name: resource_name.into(),
                rect: bounding_box(&elements).unwrap(),
                elements,
                id,
            });
        }
        patches.sort_by(|a, b| b.elements.len().cmp(&a.elements.len()));
        patches
    }

    pub fn add_tiles(&self, tiles: Vec<FactorioTile>, _clear_rect: Option<Rect>) -> Result<()> {
        let mut tree = self.tile_tree.write();
        let mut blocked = self.blocked_tree.write();
        for tile in tiles {
            let rect: QuadTreeRect = add_to_rect(
                &Rect::from_wh(1., 1.),
                &Position::new(tile.position.x() + 0.5, tile.position.y() + 0.5),
            )
            .into();
            if tile.player_collidable {
                let minable = false; // player_collidable tiles like water are not minable
                blocked.insert_with_box(minable, rect);
            }
            tree.insert_with_box(tile, rect);
        }
        Ok(())
    }

    pub fn add_blueprint_entities(&self, str: &str) -> Result<()> {
        let decoded = BlueprintCodec::decode_string(str).expect("failed to parse blueprint");
        let mut entities: Vec<FactorioEntity> = vec![];
        match decoded {
            Container::Blueprint(blueprint) => {
                for ent in blueprint.entities {
                    entities.push(FactorioEntity::from_blueprint_entity(
                        ent,
                        self.entity_prototypes.clone(),
                    )?);
                }
            }
            _ => panic!("blueprint books not supported"),
        }
        self.add(entities, None)
    }

    pub fn add(&self, entities: Vec<FactorioEntity>, _clear_rect: Option<Rect>) -> Result<()> {
        let mut resource_tree = self.resource_tree.write();
        for entity in &entities {
            if entity.entity_type == EntityType::Resource.to_string() {
                match self.resources.get_mut(&entity.name) {
                    Some(mut positions) => {
                        positions.push((&entity.position).into());
                    }
                    None => {
                        self.resources
                            .insert(entity.name.clone(), vec![(&entity.position).into()]);
                    }
                }
                let rect: QuadTreeRect = add_to_rect(
                    &Rect::from_wh(1., 1.),
                    &Position::new(
                        entity.position.x().floor() + 0.5,
                        entity.position.y().floor() + 0.5,
                    ),
                )
                .into();
                resource_tree.insert_with_box(entity.name.clone(), rect);
            }
        }
        let mut blocked = self.blocked_tree.write();
        // println!("inserted {}", blocked.len());
        for mut entity in entities {
            if entity.entity_type == EntityType::FlyingText.to_string()
                || entity.entity_type == EntityType::Fish.to_string()
                || entity.bounding_box.width() == 0.
            {
                continue;
            }
            if entity.entity_type != EntityType::Resource.to_string()
                && entity.entity_type != EntityType::StraightRail.to_string()
                && entity.entity_type != EntityType::CurvedRail.to_string()
            {
                blocked.insert_with_box(entity.is_minable(), entity.bounding_box.clone().into());
            }
            if entity.name == EntityName::Pumpjack.to_string() {
                // for some reason pumpjacks report their drop position at their position so we fix it
                entity.drop_position = Some(entity.position.add(
                    &match Direction::from_u8(entity.direction).unwrap() {
                        Direction::North => Position::new(1., -2.),
                        Direction::East => Position::new(2., -1.),
                        Direction::South => Position::new(-1., 2.),
                        Direction::West => Position::new(-2., 1.),
                        _ => panic!("invalid pumpjack position"),
                    },
                ));
            }

            if let Ok(entity_type) = EntityType::from_str(&entity.entity_type) {
                match (entity.name.as_str(), &entity_type) {
                    (_, EntityType::Furnace)
                    | (_, EntityType::Inserter)
                    | (_, EntityType::Boiler)
                    | (_, EntityType::Lab)
                    | (_, EntityType::OffshorePump)
                    | (_, EntityType::MiningDrill)
                    | (_, EntityType::StorageTank)
                    | (_, EntityType::Container)
                    | (_, EntityType::Splitter)
                    | (_, EntityType::TransportBelt)
                    | (_, EntityType::UndergroundBelt)
                    | (_, EntityType::Pipe)
                    | (_, EntityType::PipeToGround)
                    | (_, EntityType::LogisticContainer)
                    | (_, EntityType::AssemblingMachine)
                    | ("rock-big", _)
                    | ("rock-huge", _) => {
                        if let Some(entity_id) = self.entity_at(&entity.position) {
                            let tree = self.entity_tree.read();
                            let block = tree.get(entity_id).unwrap();
                            warn!(
                                "failed to add {}@{} -> blocked by {}@{}",
                                entity.name, entity.position, block.name, block.position
                            );
                            continue;
                        }
                        if let Some(entity_id) = {
                            let mut tree = self.entity_tree.write();
                            tree.insert(entity.clone())
                        } {
                            let miner_ore = if entity_type == EntityType::MiningDrill {
                                let rect = rect_floor(&entity.bounding_box);
                                let mut miner_ore: Option<String> = None;
                                for resource in &[
                                    EntityName::IronOre,
                                    EntityName::CopperOre,
                                    EntityName::Coal,
                                    EntityName::Stone,
                                    EntityName::CrudeOil,
                                    EntityName::UraniumOre,
                                ] {
                                    let resource = resource.to_string();
                                    let resource_found = rect_fields(&rect).iter().any(|p| {
                                        self.resources
                                            .get(&resource)
                                            .and_then(|resources| {
                                                if resources.contains(&p.into()) {
                                                    Some(true)
                                                } else {
                                                    None
                                                }
                                            })
                                            .is_some()
                                    });
                                    if resource_found {
                                        miner_ore = Some(resource);
                                        break;
                                    }
                                }
                                if miner_ore.is_none() {
                                    warn!(
                                        "no ore found under miner {} @ {}",
                                        entity.name, entity.position
                                    );
                                }
                                miner_ore
                            } else {
                                None
                            };
                            let new_node = EntityNode::new(entity.clone(), miner_ore, entity_id);
                            let mut inner = self.entity_graph.write();
                            let new_node_index = inner.add_node(new_node);
                            self.entity_nodes.insert(entity_id, new_node_index);
                        } else {
                            warn!("failed to insert entity into quad tree");
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub fn condense(&self) -> EntityGraphInner {
        let _started = Instant::now();
        let mut graph = self.entity_graph.read().clone();
        let _starting_nodes = graph.node_indices().count();
        let mut roots: Vec<usize> = vec![];
        loop {
            let mut next_node: Option<NodeIndex> = None;
            for node_index in graph.externals(petgraph::Direction::Incoming) {
                if !roots.contains(&node_index.index()) {
                    roots.push(node_index.index());
                    next_node = Some(node_index);
                    break;
                }
            }
            if let Some(next_node) = next_node {
                let mut bfs = Bfs::new(&graph, next_node);
                while let Some(node_index) = bfs.next(&graph) {
                    let node = graph.node_weight(node_index).unwrap();
                    let incoming: Vec<String> = graph
                        .edges_directed(node_index, petgraph::Direction::Incoming)
                        .map(|edge| {
                            graph
                                .node_weight(edge.target())
                                .unwrap()
                                .entity_name
                                .clone()
                        })
                        .collect();
                    let outgoing: Vec<String> = graph
                        .edges_directed(node_index, petgraph::Direction::Outgoing)
                        .map(|edge| {
                            graph
                                .node_weight(edge.target())
                                .unwrap()
                                .entity_name
                                .clone()
                        })
                        .collect();
                    if incoming.len() == 1
                        && outgoing.len() == 1
                        && node.entity_name == incoming[0]
                        && incoming[0] == outgoing[0]
                    {
                        let incoming: NodeIndex = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| edge.source())
                            .find(|_| true)
                            .unwrap();
                        let outgoing = graph
                            .edges_directed(node_index, petgraph::Direction::Outgoing)
                            .map(|edge| edge.target())
                            .find(|_| true)
                            .unwrap();
                        let weight = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| *edge.weight())
                            .find(|_| true)
                            .unwrap()
                            + graph
                                .edges_directed(node_index, petgraph::Direction::Outgoing)
                                .map(|edge| *edge.weight())
                                .find(|_| true)
                                .unwrap();
                        graph.add_edge(incoming, outgoing, weight);
                        if let Some(edge) = graph.find_edge(incoming, node_index) {
                            graph.remove_edge(edge);
                        }
                        if let Some(edge) = graph.find_edge(node_index, outgoing) {
                            graph.remove_edge(edge);
                        }
                        graph.remove_node(node_index);
                    } else if incoming.len() == 2
                        && outgoing.len() == 2
                        && node.entity_name == incoming[0]
                        && incoming[0] == outgoing[0]
                    {
                        let incoming: Vec<NodeIndex> = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| edge.source())
                            .collect();
                        let weights: Vec<f64> = graph
                            .edges_directed(node_index, petgraph::Direction::Incoming)
                            .map(|edge| *edge.weight())
                            .collect();
                        let weight = weights[0] + weights[1];
                        graph.add_edge(incoming[0], incoming[1], weight);
                        graph.add_edge(incoming[1], incoming[0], weight);
                        for connected_index in incoming {
                            if let Some(edge) = graph.find_edge(connected_index, node_index) {
                                graph.remove_edge(edge);
                            }
                            if let Some(edge) = graph.find_edge(node_index, connected_index) {
                                graph.remove_edge(edge);
                            }
                        }
                        graph.remove_node(node_index);
                    }
                }
            } else {
                break;
            }
        }

        let mut orphans: Vec<NodeIndex> = vec![];
        for node_index in graph.node_indices() {
            if graph
                .edges_directed(node_index, petgraph::Direction::Incoming)
                .count()
                == 0
                && graph
                    .edges_directed(node_index, petgraph::Direction::Outgoing)
                    .count()
                    == 0
            {
                orphans.push(node_index);
            }
        }
        for orphan in orphans {
            graph.remove_node(orphan);
        }
        // info!(
        //     "condensing entity graph from {} to {} entities took {:?}",
        //     starting_nodes,
        //     graph.node_indices().count(),
        //     started.elapsed()
        // );
        graph
    }

    pub fn remove(&self, entity: &FactorioEntity) -> Result<()> {
        let mut nodes_to_remove: Vec<NodeIndex> = vec![];
        let mut edges_to_remove: Vec<EdgeIndex> = vec![];
        let mut entities_to_remove: Vec<ItemId> = vec![];

        if let Some(entity_id) = self.entity_at(&entity.position) {
            if let Some(node_index) = self.entity_nodes.get(&entity_id) {
                let inner = self.entity_graph.read();
                for edge in inner.edges_directed(*node_index, petgraph::Direction::Incoming) {
                    edges_to_remove.push(edge.id());
                }
                for edge in inner.edges_directed(*node_index, petgraph::Direction::Outgoing) {
                    edges_to_remove.push(edge.id());
                }
                nodes_to_remove.push(*node_index);
            }
            entities_to_remove.push(entity_id);
        }
        let mut inner = self.entity_graph.write();
        for edge in edges_to_remove {
            inner.remove_edge(edge);
        }
        for entity_id in entities_to_remove {
            self.entity_nodes.remove(&entity_id);
        }
        for node in nodes_to_remove {
            inner.remove_node(node);
        }

        let mut blocked_item_ids_to_remove: Vec<ItemId> = vec![];
        let blocked_tree = self.blocked_tree.read();
        for (_, _, item_id) in blocked_tree.query(entity.bounding_box.clone().into()) {
            blocked_item_ids_to_remove.push(item_id);
        }
        drop(blocked_tree);
        let mut blocked_tree = self.blocked_tree.write();
        for item_id in blocked_item_ids_to_remove {
            blocked_tree.remove(item_id);
        }
        drop(blocked_tree);
        let mut entity_item_ids_to_remove: Vec<ItemId> = vec![];
        let entity_tree = self.entity_tree.read();
        for (other_entity, _, item_id) in entity_tree.query(entity.bounding_box.clone().into()) {
            if entity.name == other_entity.name {
                entity_item_ids_to_remove.push(item_id);
            }
        }
        drop(entity_tree);
        let mut entity_tree = self.entity_tree.write();
        for item_id in entity_item_ids_to_remove {
            entity_tree.remove(item_id);
        }
        drop(entity_tree);

        if entity.entity_type == EntityType::Resource.to_string() {
            let mut resource_item_ids_to_remove: Vec<ItemId> = vec![];
            let resource_tree = self.resource_tree.read();
            for (_, _, item_id) in resource_tree.query(entity.bounding_box.clone().into()) {
                resource_item_ids_to_remove.push(item_id);
            }
            drop(resource_tree);
            let mut resource_tree = self.resource_tree.write();
            for item_id in resource_item_ids_to_remove {
                resource_tree.remove(item_id);
            }
            drop(resource_tree);
            if let Some(mut positions) = self.resources.get_mut(&entity.name) {
                let entity_pos: Pos = (&entity.position).into();
                if let Some(i) = positions.iter().position(|pos| *pos == entity_pos) {
                    positions.remove(i);
                }
            }
        }

        Ok(())
    }

    pub fn connect(&self) -> Result<()> {
        let _started = Instant::now();
        let tree = self.entity_tree.read();
        let mut edges_to_add: Vec<(NodeIndex, NodeIndex, f64)> = vec![];
        let nodes: Vec<NodeIndex> = self.entity_graph.read().node_indices().collect();
        println!("connecting {} nodes", nodes.len());
        for node_index in nodes {
            let inner = self.entity_graph.read();
            let node_index = node_index;
            if let Some(node) = inner.node_weight(node_index) {
                let node_entity = tree.get(node.entity_id.unwrap()).unwrap();
                if let Some(drop_position) = node_entity.drop_position.as_ref() {
                    // if node_entity.entity_type == "mining-drill" {
                    //     info!(
                    //         "drop position for {} -> {} @ {}",
                    //         node_entity.name, node_entity.position, drop_position
                    //     );
                    // }
                    match self.node_at(drop_position) {
                        Some(drop_index) => {
                            // if node_entity.name == "pumpjack" {
                            //     info!(
                            //         "found pipe?",
                            //     );
                            // }

                            if !inner.contains_edge(node_index, drop_index) {
                                edges_to_add.push((node_index, drop_index, 1.));
                            }
                        }
                        None => error!(
                            "connect entity graph could not find entity at Drop position {} for {} @ {}",
                            drop_position, node_entity.name, node_entity.position
                        ),
                    }
                }
                if let Some(pickup_position) = node_entity.pickup_position.as_ref() {
                    match self.node_at(pickup_position) {
                        Some(pickup_index) => {
                            if !inner.contains_edge(pickup_index, node_index) {
                                edges_to_add.push((pickup_index, node_index, 1.));
                            }
                        }
                        None => error!(
                            "connect entity graph could not find entity at Pickup position {} for {} @ {}",
                            pickup_position, node_entity.name, node_entity.position
                        ),
                    }
                }
                match node.entity_type {
                    EntityType::Splitter => {
                        let out1 = node
                            .position
                            .add(&Position::new(-0.5, -1.).turn(node.direction));
                        let out2 = node
                            .position
                            .add(&Position::new(0.5, -1.).turn(node.direction));
                        for pos in &[&out1, &out2] {
                            if let Some(next_index) = self.node_at(pos) {
                                let next = inner.node_weight(next_index).unwrap();
                                // info!(
                                //     "found splitter output: {} @ {}",
                                //     next.entity.name, next.entity.position
                                // );
                                if !inner.contains_edge(node_index, next_index)
                                    && self.is_entity_belt_connectable(node, next)
                                {
                                    edges_to_add.push((node_index, next_index, 1.));
                                }
                                // } else {
                                //     warn!(
                                //         "NOT found splitter output: for {} @ {} -> searched @ {}",
                                //         node.entity.name, node.entity.position, pos
                                //     );
                            }
                        }
                    }
                    EntityType::TransportBelt => {
                        if let Some(next_index) =
                            self.node_at(&move_position(&node.position, node.direction, 1.0))
                        {
                            let next = inner.node_weight(next_index).unwrap();
                            if !inner.contains_edge(node_index, next_index)
                                && self.is_entity_belt_connectable(node, next)
                            {
                                edges_to_add.push((node_index, next_index, 1.));
                                // } else {
                                //     warn!(
                                //         "2 not found transport belt connect from {} to {} ({:?})",
                                //         node.position,
                                //         move_position(&node.position, node.direction, 1.0),
                                //         node.direction
                                //     )
                            }
                            // } else {
                            //     warn!(
                            //         "1 not found transport belt connect from {} to {} ({:?})",
                            //         node.position,
                            //         move_position(&node.position, node.direction, 1.0),
                            //         node.direction
                            //     )
                        }
                    }
                    EntityType::OffshorePump => {
                        if let Some(next_index) =
                            self.node_at(&move_position(&node.position, node.direction, -1.))
                        {
                            let next = inner.node_weight(next_index).unwrap();
                            if next.entity_type.is_fluid_input()
                                && !inner.contains_edge(node_index, next_index)
                            {
                                edges_to_add.push((node_index, next_index, 1.));
                            }
                        }
                    }
                    EntityType::Pipe => {
                        for direction in Direction::orthogonal() {
                            if let Some(next_index) =
                                self.node_at(&move_position(&node.position, direction, 1.))
                            {
                                let next = inner.node_weight(next_index).unwrap();
                                if next.entity_type.is_fluid_input() {
                                    if !inner.contains_edge(node_index, next_index) {
                                        edges_to_add.push((node_index, next_index, 1.));
                                    }
                                    if !inner.contains_edge(next_index, node_index) {
                                        edges_to_add.push((next_index, node_index, 1.));
                                    }
                                }
                            }
                        }
                    }
                    EntityType::StorageTank => {
                        for position in &match node.direction {
                            Direction::North => [
                                node.position.add(&Position::new(-1., -2.)),
                                node.position.add(&Position::new(-2., -1.)),
                                node.position.add(&Position::new(2., 1.)),
                                node.position.add(&Position::new(1., 2.)),
                            ],
                            _ => [
                                node.position.add(&Position::new(2., -1.)),
                                node.position.add(&Position::new(1., -2.)),
                                node.position.add(&Position::new(-2., 1.)),
                                node.position.add(&Position::new(-1., 2.)),
                            ],
                        } {
                            if let Some(next_index) = self.node_at(position) {
                                let next = inner.node_weight(next_index).unwrap();
                                if next.entity_type.is_fluid_input() {
                                    if !inner.contains_edge(node_index, next_index) {
                                        edges_to_add.push((node_index, next_index, 1.));
                                    }
                                    if !inner.contains_edge(next_index, node_index) {
                                        edges_to_add.push((next_index, node_index, 1.));
                                    }
                                }
                            }
                        }
                    }
                    EntityType::UndergroundBelt => {
                        let mut found = false;
                        if let Some(prototype) = self.entity_prototypes.get(&node.entity_name) {
                            if let Some(max_distance) = prototype.max_underground_distance.as_ref()
                            {
                                for length in 1..=*max_distance {
                                    if let Some(next_index) = self.node_at(&move_position(
                                        &node.position,
                                        node.direction.opposite(),
                                        length as f64,
                                    )) {
                                        let next = inner.node_weight(next_index).unwrap();
                                        if next.entity_type == EntityType::UndergroundBelt
                                            && next.direction == node.direction
                                        {
                                            if !inner.contains_edge(next_index, node_index) {
                                                edges_to_add.push((
                                                    next_index,
                                                    node_index,
                                                    length as f64,
                                                ));
                                            }
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                            } else {
                                warn!("underground belt without max distance?!");
                            }
                        } else {
                            warn!("underground belt prototype not found");
                        }
                        if found {
                            if let Some(next_index) =
                                self.node_at(&move_position(&node.position, node.direction, 1.))
                            {
                                let next = inner.node_weight(next_index).unwrap();
                                if !inner.contains_edge(node_index, next_index)
                                    && self.is_entity_belt_connectable(node, next)
                                {
                                    edges_to_add.push((node_index, next_index, 1.));
                                }
                            }
                        }
                    }
                    EntityType::PipeToGround => {
                        let mut found = false;
                        if let Some(prototype) = self.entity_prototypes.get(&node.entity_name) {
                            if let Some(max_distance) = prototype.max_underground_distance.as_ref()
                            {
                                for length in 1..=*max_distance {
                                    if let Some(next_index) = self.node_at(&move_position(
                                        &node.position,
                                        node.direction,
                                        -(length as f64),
                                    )) {
                                        let next = inner.node_weight(next_index).unwrap();
                                        if next.entity_type == EntityType::PipeToGround
                                            && next.direction == node.direction.opposite()
                                        {
                                            if !inner.contains_edge(next_index, node_index) {
                                                edges_to_add.push((
                                                    next_index,
                                                    node_index,
                                                    length as f64,
                                                ));
                                            }
                                            if !inner.contains_edge(node_index, next_index) {
                                                edges_to_add.push((
                                                    node_index,
                                                    next_index,
                                                    length as f64,
                                                ));
                                            }
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                            } else {
                                warn!("underground pipe without max distance?!");
                            }
                        } else {
                            warn!("underground pipe prototype not found");
                        }
                        if found {
                            if let Some(next_index) =
                                self.node_at(&move_position(&node.position, node.direction, 1.))
                            {
                                let next = inner.node_weight(next_index).unwrap();
                                if next.entity_type.is_fluid_input()
                                    && !inner.contains_edge(node_index, next_index)
                                {
                                    edges_to_add.push((node_index, next_index, 1.));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut inner = self.entity_graph.write();
        for (a, b, w) in edges_to_add {
            if !inner.contains_edge(a, b) {
                inner.add_edge(a, b, w);
            }
        }
        // info!(
        //     "entity graph connecting {} entities took {:?}",
        //     inner.node_indices().count(),
        //     started.elapsed()
        // );
        Ok(())
    }
    pub fn entity_by_id(&self, id: ItemId) -> Option<FactorioEntity> {
        self.entity_tree.read().get(id).cloned()
    }

    pub fn node_at(&self, position: &Position) -> Option<NodeIndex> {
        self.entity_at(position)
            .and_then(|entity_id| self.entity_nodes.get(&entity_id).map(|e| *e))
    }

    pub fn entity_at(&self, position: &Position) -> Option<ItemId> {
        let tree = self.entity_tree.read();
        let results: Vec<ItemId> = tree
            .query(add_to_rect(&Rect::from_wh(0.1, 0.1), position).into())
            .iter()
            .map(|(_entity, _rect, item_id)| *item_id)
            .collect();

        if results.is_empty() {
            None
        } else if results.len() == 1 {
            Some(results[0])
        } else {
            warn!(
                "multiple entity quad tree results for {}: {:?}",
                position,
                tree.query(add_to_rect(&Rect::from_wh(0.1, 0.1), position).into())
            );
            Some(results[0])
        }
    }
    fn is_entity_belt_connectable(&self, node: &EntityNode, next: &EntityNode) -> bool {
        (next.entity_type == EntityType::TransportBelt
            || next.entity_type == EntityType::UndergroundBelt
            || next.entity_type == EntityType::Splitter)
            && next.direction != node.direction.opposite()
    }
    pub fn graphviz_dot(&self) -> String {
        format_dotgraph(
            Dot::with_config(&self.inner_graph().deref(), &[Config::GraphContentOnly]).to_string(),
        )
    }

    pub fn graphviz_dot_condensed(&self) -> String {
        let condensed = self.condense();
        format_dotgraph(Dot::with_config(&condensed, &[Config::GraphContentOnly]).to_string())
    }

    pub fn node_weight(&self, i: NodeIndex) -> Option<EntityNode> {
        self.entity_graph.read().node_weight(i).cloned()
    }

    pub fn edges_directed(&self, i: NodeIndex, dir: petgraph::Direction) -> Vec<NodeIndex> {
        self.entity_graph
            .read()
            .edges_directed(i, dir)
            .map(|e| e.target())
            .collect()
    }
}

impl Serialize for EntityGraph {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("EntityGraph", 9)?;
        state.serialize_field("entity_graph", &*self.entity_graph.read())?;
        state.serialize_field("blocked_tree", &*self.blocked_tree.read())?;
        state.serialize_field("entity_tree", &*self.entity_tree.read())?;
        state.serialize_field("tile_tree", &*self.tile_tree.read())?;
        state.serialize_field("entity_nodes", &self.entity_nodes)?;
        state.serialize_field("entity_prototypes", &*self.entity_prototypes)?;
        state.serialize_field("recipes", &*self.recipes)?;
        state.serialize_field("resources", &self.resources)?;
        state.serialize_field("resource_tree", &*self.resource_tree.read())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for EntityGraph {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            EntityGraph,
            BlockedTree,
            EntityTree,
            TileTree,
            EntityNodes,
            EntityPrototypes,
            Recipes,
            Resources,
            ResourceTree,
        }

        // This part could also be generated independently by:
        //
        //    #[derive(Deserialize)]
        //    #[serde(field_identifier, rename_all = "lowercase")]
        //    enum Field { Secs, Nanos }
        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("`secs` or `nanos`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "entity_graph" => Ok(Field::EntityGraph),
                            "blocked_tree" => Ok(Field::BlockedTree),
                            "entity_tree" => Ok(Field::EntityTree),
                            "tile_tree" => Ok(Field::TileTree),
                            "entity_nodes" => Ok(Field::EntityNodes),
                            "entity_prototypes" => Ok(Field::EntityPrototypes),
                            "recipes" => Ok(Field::Recipes),
                            "resources" => Ok(Field::Resources),
                            "resource_tree" => Ok(Field::ResourceTree),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct EntityGraphVisitor;

        impl<'de> Visitor<'de> for EntityGraphVisitor {
            type Value = EntityGraph;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct FactorioWorld")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut entity_graph = None;
                let mut blocked_tree = None;
                let mut entity_tree = None;
                let mut tile_tree = None;
                let mut entity_nodes = None;
                let mut entity_prototypes = None;
                let mut recipes = None;
                let mut resources = None;
                let mut resource_tree = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::EntityGraph => {
                            if entity_graph.is_some() {
                                return Err(de::Error::duplicate_field("entity_graph"));
                            }
                            entity_graph = Some(map.next_value()?);
                        }
                        Field::BlockedTree => {
                            if blocked_tree.is_some() {
                                return Err(de::Error::duplicate_field("blocked_tree"));
                            }
                            blocked_tree = Some(map.next_value()?);
                        }
                        Field::EntityTree => {
                            if entity_tree.is_some() {
                                return Err(de::Error::duplicate_field("entity_tree"));
                            }
                            entity_tree = Some(map.next_value()?);
                        }
                        Field::TileTree => {
                            if tile_tree.is_some() {
                                return Err(de::Error::duplicate_field("tile_tree"));
                            }
                            tile_tree = Some(map.next_value()?);
                        }
                        Field::EntityNodes => {
                            if entity_nodes.is_some() {
                                return Err(de::Error::duplicate_field("entity_nodes"));
                            }
                            entity_nodes = Some(map.next_value()?);
                        }
                        Field::EntityPrototypes => {
                            if entity_prototypes.is_some() {
                                return Err(de::Error::duplicate_field("entity_prototypes"));
                            }
                            entity_prototypes = Some(map.next_value()?);
                        }
                        Field::Recipes => {
                            if recipes.is_some() {
                                return Err(de::Error::duplicate_field("recipes"));
                            }
                            recipes = Some(map.next_value()?);
                        }
                        Field::Resources => {
                            if resources.is_some() {
                                return Err(de::Error::duplicate_field("resources"));
                            }
                            resources = Some(map.next_value()?);
                        }
                        Field::ResourceTree => {
                            if resource_tree.is_some() {
                                return Err(de::Error::duplicate_field("resource_tree"));
                            }
                            resource_tree = Some(map.next_value()?);
                        }
                    }
                }
                let entity_graph =
                    entity_graph.ok_or_else(|| de::Error::missing_field("entity_graph"))?;
                let blocked_tree =
                    blocked_tree.ok_or_else(|| de::Error::missing_field("blocked_tree"))?;
                let entity_tree =
                    entity_tree.ok_or_else(|| de::Error::missing_field("entity_tree"))?;
                let tile_tree = tile_tree.ok_or_else(|| de::Error::missing_field("tile_tree"))?;
                let entity_nodes =
                    entity_nodes.ok_or_else(|| de::Error::missing_field("entity_nodes"))?;
                let entity_prototypes = entity_prototypes
                    .ok_or_else(|| de::Error::missing_field("entity_prototypes"))?;
                let recipes = recipes.ok_or_else(|| de::Error::missing_field("recipes"))?;
                let resources = resources.ok_or_else(|| de::Error::missing_field("resources"))?;
                let resource_tree =
                    resource_tree.ok_or_else(|| de::Error::missing_field("resource_tree"))?;

                Ok(EntityGraph {
                    entity_graph: RwLock::new(entity_graph),
                    blocked_tree: RwLock::new(blocked_tree),
                    entity_tree: RwLock::new(entity_tree),
                    tile_tree: RwLock::new(tile_tree),
                    entity_nodes,
                    entity_prototypes: Arc::new(entity_prototypes),
                    recipes: Arc::new(recipes),
                    resources,
                    resource_tree: RwLock::new(resource_tree),
                })
            }
        }

        const FIELDS: &[&str] = &[
            "entity_graph",
            "blocked_tree",
            "entity_tree",
            "tile_tree",
            "entity_nodes",
            "entity_prototypes",
            "recipes",
            "resources",
            "resource_tree",
        ];
        deserializer.deserialize_struct("EntityGraph", FIELDS, EntityGraphVisitor)
    }
}

impl Clone for EntityGraph {
    fn clone(&self) -> Self {
        EntityGraph {
            entity_graph: RwLock::new(self.entity_graph.read().clone()),
            blocked_tree: RwLock::new(self.blocked_tree.read().clone()),
            entity_tree: RwLock::new(self.entity_tree.read().clone()),
            tile_tree: RwLock::new(self.tile_tree.read().clone()),
            entity_nodes: self.entity_nodes.clone(),
            entity_prototypes: Arc::new((*self.entity_prototypes).clone()),
            recipes: Arc::new((*self.recipes).clone()),
            resources: self.resources.clone(),
            resource_tree: RwLock::new(self.resource_tree.read().clone()),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.entity_graph = RwLock::new(source.entity_graph.read().clone());
        self.blocked_tree = RwLock::new(source.blocked_tree.read().clone());
        self.entity_tree = RwLock::new(source.entity_tree.read().clone());
        self.tile_tree = RwLock::new(source.tile_tree.read().clone());
        self.entity_nodes = source.entity_nodes.clone();
        self.entity_prototypes = Arc::new((*source.entity_prototypes).clone());
        self.recipes = Arc::new((*source.recipes).clone());
        self.resources = source.resources.clone();
        self.resource_tree = RwLock::new(source.resource_tree.read().clone());
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EntityNode {
    pub bounding_box: Rect,
    pub position: Position,
    pub direction: Direction,
    pub entity_name: String,
    pub entity_type: EntityType,
    pub entity_id: Option<ItemId>,
    pub miner_ore: Option<String>,
}

impl std::fmt::Display for EntityNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}{} at {}",
            if let Some(miner_ore) = &self.miner_ore {
                format!("{}: ", miner_ore)
            } else {
                String::new()
            },
            self.entity_type,
            self.position
        ))?;
        Ok(())
    }
}
impl std::fmt::Debug for EntityNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}{} at {}",
            if let Some(miner_ore) = &self.miner_ore {
                format!("{} ", miner_ore)
            } else {
                String::new()
            },
            self.entity_type,
            self.position
        ))?;
        Ok(())
    }
}

impl EntityNode {
    pub fn new(entity: FactorioEntity, miner_ore: Option<String>, entity_id: ItemId) -> EntityNode {
        let direction = Direction::from_u8(entity.direction).unwrap();
        let entity_type = EntityType::from_str(&entity.entity_type).unwrap();
        EntityNode {
            position: entity.position.clone(),
            bounding_box: entity.bounding_box.clone(),
            direction,
            miner_ore,
            entity_id: Some(entity_id),
            entity_name: entity.name,
            entity_type,
        }
    }
}

pub type EntityGraphInner = StableGraph<EntityNode, f64>;

pub type QuadTreeRect = EuclidRect<f32, Rect>;
pub type BlockedQuadTree = QuadTree<bool, Rect, [(ItemId, QuadTreeRect); 4]>;
pub type EntityQuadTree = QuadTree<FactorioEntity, Rect, [(ItemId, QuadTreeRect); 4]>;
pub type TileQuadTree = QuadTree<FactorioTile, Rect, [(ItemId, QuadTreeRect); 4]>;
pub type ResourceQuadTree = QuadTree<String, Rect, [(ItemId, QuadTreeRect); 4]>;

#[cfg(test)]
mod tests {
    use crate::test_utils::entity_graph_from;

    use super::*;

    #[test]
    fn test_splitters() {
        let graph = entity_graph_from(vec![
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 0.5), Direction::South),
            FactorioEntity::new_splitter(&Position::new(1., 1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(1.5, 2.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "transport-belt at [0.5, 0.5]" ]
    1 [ label = "transport-belt at [1.5, 0.5]" ]
    2 [ label = "splitter at [1, 1.5]" ]
    3 [ label = "transport-belt at [0.5, 2.5]" ]
    4 [ label = "transport-belt at [1.5, 2.5]" ]
    0 -> 2 [ label = "1" ]
    1 -> 2 [ label = "1" ]
    2 -> 3 [ label = "1" ]
    2 -> 4 [ label = "1" ]
}
"#,
        );
    }
    #[test]
    fn test_condense() {
        let graph = entity_graph_from(vec![
            FactorioEntity::new_transport_belt(&Position::new(0.5, 0.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 1.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 2.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 3.5), Direction::South),
            FactorioEntity::new_transport_belt(&Position::new(0.5, 4.5), Direction::South),
        ])
        .unwrap();
        assert_eq!(
            graph.graphviz_dot_condensed(),
            r#"digraph {
    0 [ label = "transport-belt at [0.5, 0.5]" ]
    4 [ label = "transport-belt at [0.5, 4.5]" ]
    0 -> 4 [ label = "4" ]
}
"#,
        );
    }

    #[test]
    fn test_splitters2() {
        let graph = entity_graph_from(vec![]).unwrap();
        graph.add_blueprint_entities("0eNqd0u+KwyAMAPB3yWd3TK/q5quM42i3MITWimbHleK7n64clK1lf74ZMb8kkhGa9oI+WEdgRrDH3kUwhxGiPbu6LXc0eAQDlrADBq7uShR9a4kwQGJg3Ql/wfDEHqZRqF30faBNgy3NkkX6YoCOLFmcGrgGw7e7dE0uY/iawcD3Maf1rlTN1EbxD8lgAKPzIZc42YDH6YEoPd7I4n6oBXP7b4rH4uczotytiGpBrJ6fXu7n0y9Y8h1L3P5ktSCrF2S9KquyCte1MbPlZPCDIU5fvuOVrvZaab5VUqX0B2ef55s=").expect("failed to read blueprint");
        graph.connect().unwrap();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "transport-belt at [-61.5, 71.5]" ]
    1 [ label = "splitter at [-60.5, 72]" ]
    2 [ label = "splitter at [-58.5, 72]" ]
    3 [ label = "transport-belt at [-59.5, 71.5]" ]
    4 [ label = "transport-belt at [-59.5, 72.5]" ]
    5 [ label = "transport-belt at [-57.5, 72.5]" ]
    0 -> 1 [ label = "1" ]
    1 -> 3 [ label = "1" ]
    1 -> 4 [ label = "1" ]
    2 -> 3 [ label = "1" ]
    2 -> 4 [ label = "1" ]
    5 -> 2 [ label = "1" ]
}
"#,
        );
    }
}
