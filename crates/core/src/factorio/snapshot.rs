//! Reading a world out of a Factorio server this process did not start.
//!
//! # Why this exists
//!
//! Until now every piece of *static* world data reached Rust the same way: the
//! BotBridge mod `print`ed it on the server's stdout at init (`writeout_recipes`
//! and friends) and [`crate::process::output_parser::OutputParser`] parsed that
//! stream. That works only for a child process this program spawned. Attaching
//! to a server the owner is already playing on gave an empty
//! [`FactorioWorld`]: no recipes, no prototypes, no entity graph, so
//! `PlanState::collision_area` returned `None` for every name and the planner
//! could not place anything.
//!
//! The mod already answered plenty over RCON — players, forces, inventories,
//! `find_entities_filtered`. The gap was only the bulk static data. So the mod
//! grew one more remote call, `world_snapshot`, which returns the *same records
//! the writeouts emit* (they share the `collect_*` functions in `control.lua`),
//! and this module turns that reply plus the RCON calls that already existed
//! into a populated world.
//!
//! # What a snapshot is not
//!
//! A snapshot is a point-in-time read. Owning the process additionally gives a
//! *stream*: `on_research_finished` re-emits recipes as technologies unlock,
//! `on_some_entity_created`/`_deleted` keep the entity graph current, and
//! player position and inventory changes arrive as they happen. None of that
//! reaches an attached session, because there is no stdout to read. See
//! [`attach_world`] for what the caller gets instead, and re-snapshot when the
//! world has moved on.

use std::sync::Arc;

#[cfg_attr(test, mockall_double::double)]
use crate::factorio::rcon::FactorioRcon;
use crate::factorio::world::FactorioWorld;
use crate::types::{
    AreaFilter, FactorioEntityPrototype, FactorioForce, FactorioItemPrototype, FactorioRecipe,
    Position, Rect,
};
use miette::Result;
use serde::{Deserialize, Serialize};

/// How far around the centre [`attach_world`] reads entities when the caller
/// does not say.
///
/// Entities are the one unbounded part of a world: prototypes and recipes are
/// a fixed few hundred kilobytes, but a long-played map has arbitrarily many
/// chunks and asking for all of them would be a request with no upper bound.
/// 200 tiles either side of the centre covers roughly the 400x400 starting area
/// a fresh save generates, which measured at ~1.9 MB and 0.7 s of RCON on a
/// Space Age freeplay map — enough ore and buildings to plan against, small
/// enough to be a bounded request.
pub const DEFAULT_ATTACH_RADIUS: f64 = 200.;

/// The static world data one `world_snapshot` RCON reply carries.
///
/// Field for field what `rcon_world_snapshot` in `mods/BotBridge/control.lua`
/// builds, and every element is produced by the same `serialize_*` /`collect_*`
/// function the stdout writeouts use, so the two transports cannot disagree
/// about the shape of a recipe or a prototype.
///
/// `forces` is a list holding exactly the force the bots act for (`player`).
/// The stdout path emits `enemy` and `neutral` as well; they cost ~120 kB of
/// technology tables each and describe nobody the planner plans for. Worse,
/// `PlanState::from_world` picks its acting force by taking the alphabetically
/// first name, so shipping them would make an attached session plan as
/// `enemy`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorldSnapshot {
    pub entity_prototypes: Vec<FactorioEntityPrototype>,
    pub item_prototypes: Vec<FactorioItemPrototype>,
    pub recipes: Vec<FactorioRecipe>,
    pub forces: Vec<FactorioForce>,
}

impl WorldSnapshot {
    /// Whether this snapshot carries enough to plan with.
    ///
    /// A world with no prototypes cannot answer `collision_area`, and one with
    /// no recipes cannot decompose a `have` goal — in both cases the planner
    /// fails with a message about the goal rather than about the world, which
    /// is exactly the confusion attaching used to produce. Checked at the point
    /// the data arrives so the error names the real cause.
    pub fn is_plannable(&self) -> bool {
        !self.entity_prototypes.is_empty() && !self.recipes.is_empty()
    }
}

/// The square of side `2 * radius` centred on `center`.
pub fn area_around(center: &Position, radius: f64) -> Rect {
    Rect::new(
        &Position::new(center.x() - radius, center.y() - radius),
        &Position::new(center.x() + radius, center.y() + radius),
    )
}

/// Builds a [`FactorioWorld`] by asking a running Factorio server over RCON.
///
/// The counterpart to [`crate::process::output_reader::read_output`] for a
/// server this process does not own. Four reads, all of them RCON:
///
/// 1. `world_snapshot` — prototypes, recipes and the player force.
/// 2. `players` — every connected player, with position and inventory.
/// 3. `find_entities_filtered` over `area` — the entity and resource graph.
/// 4. `entity_graph.connect()` / `flow_graph.update()`, the same wiring
///    `OutputParser::on_init` does once the stdout stream has gone quiet.
///
/// `area` bounds step 3. `None` means [`DEFAULT_ATTACH_RADIUS`] around the
/// first connected player, or around the origin when nobody is connected —
/// which is where a fresh map's starting resources are.
///
/// # What this gives up versus owning the process
///
/// * **No ongoing events.** Research finishing, entities being built or mined,
///   players walking: all of these reach an owned server through stdout and
///   none of them reach an attached one. The world is as of the moment of the
///   call. Call again to refresh.
/// * **Recipes are the enabled set at snapshot time.** The mod only emits
///   enabled recipes, and an owned server re-emits them on
///   `on_research_finished`. An attached one does not, so a technology finished
///   after attaching is invisible until the next snapshot.
/// * **Entities outside `area` do not exist** as far as the plan is concerned,
///   where an owned server streams every chunk it generates.
/// * **No tiles.** `update_chunk_tiles` feeds water into the blocked-tile tree
///   from the stdout `tiles` writeout (~2.1 MB for the starting area). Nothing
///   in the planner reads that tree, so paying for it here would be cost
///   without effect; `find_tiles_filtered` remains available to callers that
///   want it.
/// * **No graphics.** The sprite atlas the map renderer uses, ~100 kB, is not
///   part of planning.
pub async fn attach_world(rcon: &FactorioRcon, area: Option<Rect>) -> Result<Arc<FactorioWorld>> {
    let world = Arc::new(FactorioWorld::new());
    let snapshot = rcon.world_snapshot().await?;
    if !snapshot.is_plannable() {
        return Err(miette::miette!(
            "world snapshot carries {} entity prototypes and {} recipes; the BotBridge mod on that \
             server is too old to answer `world_snapshot` with usable data",
            snapshot.entity_prototypes.len(),
            snapshot.recipes.len()
        ));
    }
    world.apply_snapshot(snapshot)?;

    let players = rcon.connected_players().await?;
    let center = players
        .first()
        .map(|player| player.position.clone())
        .unwrap_or_else(|| Position::new(0., 0.));
    for player in players {
        world.players.insert(player.player_id, player);
    }

    let area = area.unwrap_or_else(|| area_around(&center, DEFAULT_ATTACH_RADIUS));
    let entities = rcon
        .find_entities_filtered(&AreaFilter::Rect(area), None, None)
        .await?;
    // `writeout_entities` skips characters, so the stdout path never puts a
    // player's own body into the graph. Doing so here would make every tile a
    // bot stands on read as occupied and refuse placements right where the bot
    // is — the one place it can reach.
    let entities = entities
        .into_iter()
        .filter(|entity| entity.entity_type != "character")
        .collect();
    world.update_chunk_entities(entities)?;

    world.entity_graph.connect()?;
    world.flow_graph.update()?;
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_around_is_centred_and_square() {
        let area = area_around(&Position::new(10., -4.), 2.);
        assert_eq!(area.left_top, Position::new(8., -6.));
        assert_eq!(area.right_bottom, Position::new(12., -2.));
    }

    /// The check exists to name the real cause when a server answers with
    /// nothing usable, so it has to distinguish "empty" from "populated" rather
    /// than always agreeing.
    #[test]
    fn plannability_needs_both_prototypes_and_recipes() {
        let mut snapshot = WorldSnapshot::default();
        assert!(!snapshot.is_plannable(), "an empty snapshot cannot plan");

        snapshot.entity_prototypes.push(FactorioEntityPrototype {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            collision_mask: None,
            collision_box: Rect::from_wh(1., 1.),
            mine_result: None,
            mining_time: None,
            mining_speed: None,
            crafting_speed: None,
            max_underground_distance: None,
            fluidbox_prototypes: None,
        });
        assert!(
            !snapshot.is_plannable(),
            "prototypes without recipes still cannot decompose a goal"
        );

        snapshot.recipes.push(FactorioRecipe {
            name: "iron-plate".into(),
            valid: true,
            enabled: true,
            category: "smelting".into(),
            ingredients: None,
            products: vec![],
            hidden: false,
            energy: Box::new(noisy_float::types::r64(3.2)),
            order: "a".into(),
            group: "g".into(),
            subgroup: "s".into(),
        });
        assert!(snapshot.is_plannable());
    }

    /// Every field of a snapshot has to reach the world. Dropping one is a
    /// silent failure -- the world simply lacks forces, or lacks item
    /// prototypes, and the symptom shows up much later as a plan that will not
    /// research or a REST route that answers empty.
    #[test]
    fn apply_snapshot_lands_every_field_in_the_world() {
        let mut snapshot = WorldSnapshot::default();
        snapshot.entity_prototypes.push(FactorioEntityPrototype {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            collision_mask: None,
            collision_box: Rect::from_wh(1.4, 1.4),
            mine_result: None,
            mining_time: None,
            mining_speed: None,
            crafting_speed: None,
            max_underground_distance: None,
            fluidbox_prototypes: None,
        });
        snapshot.item_prototypes.push(FactorioItemPrototype {
            name: "iron-plate".into(),
            item_type: "item".into(),
            stack_size: 100,
            fuel_value: 0,
            place_result: String::new(),
            group: "intermediate-products".into(),
            subgroup: "raw-material".into(),
        });
        snapshot.recipes.push(FactorioRecipe {
            name: "iron-plate".into(),
            valid: true,
            enabled: true,
            category: "smelting".into(),
            ingredients: None,
            products: vec![],
            hidden: false,
            energy: Box::new(noisy_float::types::r64(3.2)),
            order: "a".into(),
            group: "g".into(),
            subgroup: "s".into(),
        });
        snapshot.forces.push(FactorioForce {
            name: "player".into(),
            force_id: 1,
            current_research: None,
            research_progress: None,
            technologies: Box::default(),
        });

        let world = FactorioWorld::new();
        world.apply_snapshot(snapshot).expect("apply_snapshot");

        assert_eq!(
            world
                .entity_prototypes
                .get("stone-furnace")
                .map(|p| p.collision_box.width()),
            Some(1.4),
            "the prototype the planner reads collision boxes from is missing"
        );
        assert!(world.item_prototypes.contains_key("iron-plate"));
        assert!(world.recipes.contains_key("iron-plate"));
        assert!(
            world.forces.contains_key("player"),
            "without a force the planner has no technology table"
        );
    }
}
