//! Deserialisation tests against payloads captured verbatim from a live
//! Factorio 2.1.17 game.
//!
//! Four shipped defects came from one mechanism: a Rust type modelled on an
//! older Factorio, invisible because no fixture could contradict it.
//! `FactorioProduct::probability` named a field 2.1 does not have,
//! `FactorioRecipe::category` was renamed to `categories[]`,
//! `FactorioPlayer::main_inventory` became an array, and three of that type's
//! distances are `double` while the struct declared `u64`.
//!
//! The three fixtures that already existed
//! (`{recipes,item-prototype,entity-prototype}-fixtures.json`) were captured
//! from Factorio **1.1 in 2022** and never refreshed, and the type whose three
//! fields were wrong — `FactorioPlayer` — had never had a fixture at all. A
//! type that is only ever fed hand-written JSON is only ever tested against
//! what the author already believed.
//!
//! Every fixture these tests read is a byte-for-byte RCON reply from Factorio
//! 2.1.17 with a player connected. Nothing here is rounded, tidied or
//! reformatted: `resource_reach_distance` really does arrive as
//! `2.70000000000000017763568394002504646778106689453125`, and a fixture that
//! rounded it would not have caught the defect that motivated the capture.
//!
//! See `crates/core/tests/README.md` for what each file is and how it was
//! captured.

use factorio_bot_core::factorio::snapshot::WorldSnapshot;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::test_utils::entity_graph_from;
use factorio_bot_core::types::{
    Direction, FactorioEntity, FactorioPlayer, FactorioTile, InventoryResponse,
};
use std::collections::BTreeSet;

const PLAYERS: &str = include_str!("live-2.1.17-players.json");
const WORLD_SNAPSHOT: &str = include_str!("live-2.1.17-world-snapshot.json");
const TILES: &str = include_str!("live-2.1.17-tiles.json");
const ENTITIES_SPAWN: &str = include_str!("live-2.1.17-entities-spawn.json");
const ENTITIES_RESOURCES: &str = include_str!("live-2.1.17-entities-resources.json");
const INVENTORY_CONTENTS_AT: &str = include_str!("live-2.1.17-inventory-contents-at.json");

// ---------------------------------------------------------------------------
// FactorioPlayer — `remote.call('botbridge', 'players')`
// ---------------------------------------------------------------------------

/// The reply that broke the executor's first contact with a real game.
///
/// `RconClient::connected_players` parses exactly this, and the connect-wait
/// loop parses it once a second, so a shape error here stops every run before
/// any planning happens.
#[test]
fn the_live_players_reply_deserialises_into_factorio_player() {
    let players: Vec<FactorioPlayer> =
        serde_json::from_str(PLAYERS).unwrap_or_else(|err| panic!("{err} in {PLAYERS}"));

    assert_eq!(players.len(), 1, "one connected player was captured");
    let player = &players[0];
    assert_eq!(player.player_id, 1);
    assert_eq!(player.position.x, 0.0);
    assert_eq!(player.position.y, 0.0);
}

/// The three `double` distances, at full precision.
///
/// These were declared `u64`. `2.7` is not an integer, so serde rejected the
/// whole document and *no* player ever parsed — the failure was total, not
/// partial. Asserting the exact `f64` is the point: a test that allowed
/// rounding would pass against the broken type as soon as someone "fixed" it
/// by truncating.
#[test]
fn the_live_players_reply_carries_fractional_reach_distances() {
    let players: Vec<FactorioPlayer> = serde_json::from_str(PLAYERS).expect("parses");
    let player = &players[0];

    assert_eq!(
        player.resource_reach_distance, 2.7,
        "a character's resource reach is 2.7, which no integer type can hold"
    );
    assert!(
        player.resource_reach_distance.fract() > 0.0,
        "the value has to stay fractional; rounding it here would let a u64 \
         field pass this test"
    );
    assert_eq!(player.item_pickup_distance, 1.0);
    assert_eq!(player.loot_pickup_distance, 2.0);

    // The three that really are uint32 in `LuaControl`.
    assert_eq!(player.build_distance, 10);
    assert_eq!(player.reach_distance, 10);
    assert_eq!(player.drop_item_distance, 10);
}

/// `LuaInventory::get_contents` returns an **array** of
/// `{name, quality, count}` in Factorio 2.0+, not the name→count map 1.1 sent.
#[test]
fn the_live_players_reply_carries_an_array_shaped_main_inventory() {
    let players: Vec<FactorioPlayer> = serde_json::from_str(PLAYERS).expect("parses");
    let inventory = &players[0].main_inventory;

    assert!(
        !inventory.is_empty(),
        "the capture was taken with a connected player holding starting items, \
         so an empty inventory here means the array shape was dropped"
    );
    assert_eq!(inventory.get("burner-mining-drill"), Some(&1));
    assert_eq!(inventory.get("wood"), Some(&1));

    // The raw text is the array form, not a map. If the mod ever reverts to a
    // map this fixture stops being the 2.x shape and the assertion above stops
    // meaning what it says.
    let raw: serde_json::Value = serde_json::from_str(PLAYERS).expect("parses as json");
    let main_inventory = &raw[0]["main_inventory"];
    assert!(
        main_inventory.is_array(),
        "2.0+ sends an array of item records, got {main_inventory}"
    );
    assert_eq!(main_inventory[0]["quality"], "normal");
}

// ---------------------------------------------------------------------------
// WorldSnapshot — `remote.call('botbridge', 'world_snapshot')`
// ---------------------------------------------------------------------------

/// The whole attach-mode payload: prototypes, recipes and the acting force in
/// one reply. Nothing exercised this before, and it is the only way a session
/// that attaches to a server it did not start learns what the world is made of.
#[test]
fn the_live_world_snapshot_deserialises() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT)
        .unwrap_or_else(|err| panic!("the live world_snapshot reply must parse: {err}"));

    assert!(
        snapshot.is_plannable(),
        "a real snapshot has to be plannable"
    );
    assert_eq!(snapshot.entity_prototypes.len(), 1028);
    assert_eq!(snapshot.item_prototypes.len(), 342);
    // 662, not the 23 this fixture carried when `collect_recipes` sent only
    // the recipes already enabled for the force. A planner asked to plan
    // toward a *future* state cannot be shown only the current one: the
    // automation science pack recipe is disabled on a fresh map, so
    // `goal.researched("automation")` had no recipe to craft its packs with
    // and failed outright. All 639 of the extra recipes are disabled.
    assert_eq!(snapshot.recipes.len(), 662);
    assert_eq!(
        snapshot.recipes.iter().filter(|r| r.enabled).count(),
        23,
        "the same 23 that used to be the whole payload"
    );
    assert_eq!(snapshot.forces.len(), 1, "only the player force is sent");
}

/// A disabled recipe arrives, and arrives flagged as disabled.
///
/// The pair matters more than either half: a recipe that is missing cannot be
/// planned with, and a recipe that is present but indistinguishable from an
/// enabled one gets planned with immediately, producing a craft the game
/// refuses. `automation-science-pack` is the exact recipe the live defect was
/// found on.
#[test]
fn the_live_world_snapshot_carries_disabled_recipes_flagged_as_disabled() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let pack = snapshot
        .recipes
        .iter()
        .find(|recipe| recipe.name == "automation-science-pack")
        .expect("a live 2.1 game has the automation science pack recipe");

    assert!(
        !pack.enabled,
        "it is locked behind its technology on a fresh map"
    );
    assert_eq!(
        pack.category, "crafting",
        "and is hand-craftable once unlocked"
    );
}

/// Proof this capture is 2.x and not another 1.1 fixture.
///
/// The 2022 fixtures are indistinguishable from current ones by inspection —
/// that is precisely why they went stale unnoticed. These four prototypes did
/// not exist before 2.0, so their presence dates the file from its contents
/// rather than from `git log`.
#[test]
fn the_live_world_snapshot_contains_factorio_2_x_prototypes() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let names: std::collections::HashSet<&str> = snapshot
        .entity_prototypes
        .iter()
        .map(|prototype| prototype.name.as_str())
        .collect();

    for introduced_in_2_x in [
        "agricultural-tower",
        "foundry",
        "biochamber",
        "asteroid-collector",
        "recycler",
    ] {
        assert!(
            names.contains(introduced_in_2_x),
            "{introduced_in_2_x} exists only in Factorio 2.x; this fixture is \
             not from a 2.x game"
        );
    }
}

/// The two recipe defects, on a real recipe.
///
/// 2.1 has no `Product::probability` and no `LuaRecipe::category`; the mod
/// bridges `categories[1]` back to `category` because the planner branches on
/// it, and `FactorioProduct` folds `independent_probability` and the
/// `shared_probability` window into one number.
#[test]
fn the_live_world_snapshot_carries_a_2_1_smelting_recipe() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let iron_plate = snapshot
        .recipes
        .iter()
        .find(|recipe| recipe.name == "iron-plate")
        .expect("a live 2.1 game has an enabled iron-plate recipe");

    assert_eq!(
        iron_plate.category, "smelting",
        "the category has to survive the categories[] rename; the planner \
         branches on it"
    );
    assert_eq!(iron_plate.products.len(), 1);
    assert_eq!(iron_plate.products[0].name, "iron-plate");
    assert_eq!(iron_plate.products[0].amount, 1);
    assert_eq!(
        *iron_plate.products[0].probability,
        noisy_float::types::r64(1.0),
        "independent_probability 1 over the full shared window is certainty"
    );

    // The wire text has no `probability` key at all — that is the field 2.1
    // removed, and the reason a type that required it saw nothing.
    let raw: serde_json::Value = serde_json::from_str(WORLD_SNAPSHOT).expect("parses as json");
    let product = raw["recipes"]
        .as_array()
        .expect("recipes is an array")
        .iter()
        .find(|recipe| recipe["name"] == "iron-plate")
        .expect("iron-plate")["products"][0]
        .clone();
    assert!(
        product.get("probability").is_none(),
        "Factorio 2.1 sends no probability field, got {product}"
    );
    assert!(
        product.get("independent_probability").is_some(),
        "2.1 sends independent_probability instead, got {product}"
    );
}

/// `FactorioForce` and `FactorioTechnology`, neither of which had a fixture.
/// `serialize_force` is shared by the `player_force` RCON call, the `force`
/// stdout record and this snapshot, so one capture covers all three.
#[test]
fn the_live_world_snapshot_carries_the_player_force_and_its_technologies() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let force = &snapshot.forces[0];

    assert_eq!(force.name, "player");
    assert_eq!(
        force.current_research, None,
        "nothing was being researched when this was captured"
    );
    assert_eq!(force.technologies.len(), 277);

    let automation = force
        .technologies
        .get("automation")
        .expect("a vanilla game has the automation technology");
    assert!(!automation.researched);
    assert_eq!(automation.research_unit_count, 10);
    assert!(
        !automation.research_unit_ingredients.is_empty(),
        "automation costs science packs"
    );

    // The other half of planning through a locked recipe: which technology
    // turns one on. `effects` lives on `LuaTechnologyPrototype`, not on
    // `LuaTechnology`, so `serialize_technology` has to reach it through
    // `.prototype` — a capture is the only thing that says it really did.
    assert_eq!(
        automation.unlocked_recipes,
        vec![
            "assembling-machine-1".to_string(),
            "long-handed-inserter".to_string()
        ],
        "automation's own unlock-recipe effects"
    );

    // And the mapping that makes `goal.researched(\"automation\")` plannable:
    // the pack recipe it needs is unlocked by a *different* technology, which
    // is also one of automation's prerequisites.
    let pack_tech = force
        .technologies
        .get("automation-science-pack")
        .expect("the technology that unlocks the pack recipe");
    assert!(
        pack_tech
            .unlocked_recipes
            .contains(&"automation-science-pack".to_string())
    );
    assert!(
        automation
            .prerequisites
            .as_ref()
            .expect("automation lists prerequisites")
            .contains(&"automation-science-pack".to_string())
    );
}

/// Every disabled recipe should name a technology that unlocks it — with eight
/// real exceptions.
///
/// The exceptions are not a rounding error to be papered over: `loader`,
/// `pistol` and the `infinity-*` entities are editor items no technology ever
/// unlocks, so a planner that assumed "disabled implies unlockable" would plan
/// crafts that can never run. `RecipeGate::Unobtainable` exists for exactly
/// these, and this pins the number so that a future base game growing a ninth
/// is noticed rather than absorbed.
#[test]
fn almost_every_disabled_recipe_names_an_unlocking_technology() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let unlockable: BTreeSet<&str> = snapshot.forces[0]
        .technologies
        .values()
        .flat_map(|tech| tech.unlocked_recipes.iter().map(String::as_str))
        .collect();

    let orphans: BTreeSet<&str> = snapshot
        .recipes
        .iter()
        .filter(|recipe| !recipe.enabled)
        .map(|recipe| recipe.name.as_str())
        .filter(|name| !unlockable.contains(name))
        .collect();

    assert_eq!(
        orphans,
        BTreeSet::from([
            "express-loader",
            "fast-loader",
            "heat-interface",
            "infinity-chest",
            "infinity-pipe",
            "loader",
            "pistol",
            "turbo-loader",
        ])
    );
}

/// `collision_mask` carries collision *layer* names, not the names of the
/// fields of the table that holds them.
///
/// Factorio 2.0 turned `LuaEntityPrototype::collision_mask` from a flat set of
/// layer names into a `CollisionMask` table — `{layers = {name -> true}}` plus
/// three booleans. `serialize_entity_prototype` iterated the mask itself, so
/// every prototype reported that outer table's own keys: this fixture used to
/// read `["layers"]` on 796 of 1028 prototypes and `["colliding_with_tiles_only",
/// "layers"]` or similar on the rest — four distinct values across the whole
/// game, none of them a layer.
///
/// That failure mode is the reason this test asserts *contents* and not
/// presence. A plausible-looking non-empty list of strings is what a mistaken
/// read produced here, so "the field arrived" is exactly the check that could
/// not tell the two apart.
#[test]
fn the_live_world_snapshot_carries_real_collision_layer_names() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let layers: BTreeSet<&str> = snapshot
        .entity_prototypes
        .iter()
        .filter_map(|prototype| prototype.collision_mask.as_ref())
        .flat_map(|mask| mask.iter().map(String::as_str))
        .collect();

    // The four keys of the 2.0 `CollisionMask` table. None is a layer, and
    // seeing any of them means the mask itself is being iterated again.
    for not_a_layer in [
        "layers",
        "colliding_with_tiles_only",
        "consider_tile_transitions",
        "not_colliding_with_itself",
    ] {
        assert!(
            !layers.contains(not_a_layer),
            "{not_a_layer} is a field of the CollisionMask table, not a \
             collision layer — serialize_entity_prototype is iterating the \
             mask instead of its .layers"
        );
    }

    // Real layers, from `prototypes.collision_layer` in a 2.1 game.
    for layer in ["player", "object", "water_tile", "rail", "transport_belt"] {
        assert!(
            layers.contains(layer),
            "{layer} is a vanilla collision layer and must appear somewhere \
             in 1028 prototypes"
        );
    }

    // `stone-furnace` is a building, so it collides with the building set and
    // emphatically not with `doodad` or `rail`.
    let furnace = snapshot
        .entity_prototypes
        .iter()
        .find(|prototype| prototype.name == "stone-furnace")
        .expect("stone-furnace exists");
    let mask: BTreeSet<&str> = furnace
        .collision_mask
        .as_ref()
        .expect("a building collides with something")
        .iter()
        .map(String::as_str)
        .collect();
    assert!(mask.contains("object"), "got {mask:?}");
    assert!(mask.contains("player"), "got {mask:?}");
    assert!(!mask.contains("doodad"), "got {mask:?}");
}

/// A pipe connection reports which *kind* of connection it is.
///
/// `serialize_fluidbox_connection` asked for `type` and renamed it to
/// `connection_type`, which is how Factorio 1.1 spelled it.
/// `PipeConnectionDefinition` in 2.1.17 declares `connection_type` and no
/// `type` at all — and because it is a plain Lua table rather than a userdata,
/// the absent key did not raise: it returned nil, so the key was dropped with
/// no `pcall` involved and nothing to log. It arrived on **0 of 95** pipe
/// connections while `positions`, its neighbour on the same table, arrived on
/// all 95.
///
/// The `underground` connection is the discriminating case. It is the only one
/// of the 95 that is not `normal`, and it is the same connection that carries
/// `max_underground_distance` — so a read that returned a constant, or that
/// picked up `flow_direction` by mistake, could not produce this pairing.
#[test]
fn the_live_world_snapshot_carries_pipe_connection_types() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let connections: Vec<_> = snapshot
        .entity_prototypes
        .iter()
        .filter_map(|prototype| prototype.fluidbox_prototypes.as_ref())
        .flatten()
        .filter_map(|fluidbox| fluidbox.pipe_connections.as_ref().as_ref())
        .flatten()
        .collect();

    assert_eq!(connections.len(), 95, "vanilla 2.1.17 pipe connections");
    assert!(
        connections
            .iter()
            .all(|connection| connection.connection_type.is_some()),
        "every pipe connection has a connection_type"
    );

    let underground: Vec<_> = connections
        .iter()
        .filter(|connection| connection.connection_type.as_deref() == Some("underground"))
        .collect();
    assert_eq!(underground.len(), 1, "pipe-to-ground is the only one");
    assert_eq!(
        underground[0].max_underground_distance,
        Some(10),
        "the underground connection is the one with a reach, and no other \
         connection reports one"
    );
    assert_eq!(
        connections
            .iter()
            .filter(|connection| connection.max_underground_distance.is_some())
            .count(),
        1
    );
}

/// The force's own mining modifier arrives, and is a number rather than a
/// missing key read as a default.
///
/// `character_mining_speed` multiplies the character prototype's mining speed
/// by `1 + manual_mining_speed_modifier`, so this feeds every hand-mining
/// duration the planner emits. It is `#[serde(default)]` on `FactorioForce`,
/// which means a mod that stopped sending it would read as `None` and be
/// treated as the game's own default of `0` — the same "absent is a legal
/// outcome" shape that hid `crafting_speed` for four years. `Some(0.0)` and
/// `None` are indistinguishable downstream; only here can they be told apart.
#[test]
fn the_live_world_snapshot_carries_the_forces_manual_mining_speed_modifier() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");
    let modifier = snapshot.forces[0]
        .manual_mining_speed_modifier
        .as_deref()
        .copied()
        .expect("the mod sends the modifier; None means it stopped arriving");

    // A fresh game has not researched steel-axe, so the value itself is 0.
    // Asserting it is *present* is the point, not that it is zero.
    assert_eq!(modifier.raw(), 0.0);
}

/// Every crafting-capable prototype reports a crafting speed, and nothing else
/// does.
///
/// The `get_crafting_speed()` fix has a sharper check available than "some
/// prototypes have it": the set that does must be exactly the set that can
/// craft. 2.0 moved the attribute to a method, so the read raised and the
/// `pcall` reported absence on all 1028 — and a repair that worked for
/// furnaces but raised for, say, `rocket-silo` would look just as fixed
/// against a furnace-only assertion.
#[test]
fn crafting_speed_arrives_for_exactly_the_prototypes_that_can_craft() {
    let snapshot: WorldSnapshot = serde_json::from_str(WORLD_SNAPSHOT).expect("parses");

    let with_speed: BTreeSet<&str> = snapshot
        .entity_prototypes
        .iter()
        .filter(|prototype| prototype.crafting_speed.is_some())
        .map(|prototype| prototype.name.as_str())
        .collect();
    let can_craft: BTreeSet<&str> = snapshot
        .entity_prototypes
        .iter()
        .filter(|prototype| {
            matches!(
                prototype.entity_type.as_str(),
                "assembling-machine" | "furnace" | "rocket-silo" | "character"
            )
        })
        .map(|prototype| prototype.name.as_str())
        .collect();

    assert_eq!(
        with_speed, can_craft,
        "crafting_speed must arrive for every crafting machine and for no \
         other prototype"
    );
    assert_eq!(with_speed.len(), 18);

    // The three furnaces the smelting divisor actually chooses between.
    let speed_of = |name: &str| {
        snapshot
            .entity_prototypes
            .iter()
            .find(|prototype| prototype.name == name)
            .and_then(|prototype| prototype.crafting_speed)
            .unwrap_or_else(|| panic!("{name} has a crafting speed"))
    };
    assert_eq!(speed_of("stone-furnace"), 1.0);
    assert_eq!(speed_of("steel-furnace"), 2.0);
    assert_eq!(speed_of("electric-furnace"), 2.0);
    assert_eq!(speed_of("character"), 1.0, "hand crafting is speed 1");
}

// ---------------------------------------------------------------------------
// FactorioTile — `remote.call('botbridge', 'find_tiles_filtered')`
// ---------------------------------------------------------------------------

/// This call returned an error string rather than JSON on every 2.x game until
/// the capture work found it: `serialize_tile` asked for the collision layer
/// `player-layer`, which Factorio 2.0 renamed to `player`. The old name does
/// not silently miss, it raises, so `find_tiles_filtered` was dead. Nothing
/// noticed because `FactorioTile` had never had a fixture.
#[test]
fn the_live_tiles_reply_deserialises_into_factorio_tile() {
    let tiles: Vec<FactorioTile> =
        serde_json::from_str(TILES).unwrap_or_else(|err| panic!("{err} in {TILES}"));

    assert_eq!(tiles.len(), 64, "an 8x8 area is 64 tiles");
    assert!(
        tiles.iter().any(|tile| tile.name == "water"),
        "the captured area straddles a shoreline"
    );
    assert!(tiles.iter().any(|tile| tile.name == "grass-1"));
}

/// `player_collidable` is the only field the mod computes rather than copies,
/// and it is the one the layer rename broke. A fixture of all-water or
/// all-land tiles could not tell a working `collides_with` from one that
/// returned a constant, so the captured area deliberately contains both.
#[test]
fn the_live_tiles_reply_distinguishes_collidable_from_walkable_tiles() {
    let tiles: Vec<FactorioTile> = serde_json::from_str(TILES).expect("parses");

    let collidable = tiles.iter().filter(|tile| tile.player_collidable).count();
    let walkable = tiles.iter().filter(|tile| !tile.player_collidable).count();
    assert_eq!(collidable, 62, "48 water + 14 deepwater");
    assert_eq!(walkable, 2, "2 grass tiles");

    for tile in &tiles {
        let expected = tile.name == "water" || tile.name == "deepwater";
        assert_eq!(
            tile.player_collidable,
            expected,
            "{} should{} collide with the player layer",
            tile.name,
            if expected { "" } else { " not" }
        );
    }
}

// ---------------------------------------------------------------------------
// FactorioEntity — `remote.call('botbridge', 'find_entities_filtered')`
// ---------------------------------------------------------------------------

/// Resource entities carry `amount`, which nothing had ever parsed from a real
/// reply. This area is a plain ore patch: no inventories, so it exercises the
/// path that works today.
#[test]
fn the_live_resource_entities_reply_deserialises_into_factorio_entity() {
    let entities: Vec<FactorioEntity> = serde_json::from_str(ENTITIES_RESOURCES)
        .unwrap_or_else(|err| panic!("live resource entities must parse: {err}"));

    assert_eq!(entities.len(), 40);
    let ore: Vec<&FactorioEntity> = entities
        .iter()
        .filter(|entity| entity.entity_type == "resource")
        .collect();
    assert_eq!(ore.len(), 39);
    assert!(
        ore.iter().all(|entity| entity.amount.is_some()),
        "every resource entity reports an amount"
    );
    assert_eq!(ore[0].name, "iron-ore");
    assert_eq!(ore[0].amount, Some(13));
    // A resource has a real bounding box, not the default.
    assert!(ore[0].bounding_box.right_bottom.x > ore[0].bounding_box.left_top.x);
}

/// Every position `resource_patches` reports must be a position a real ore
/// entity actually occupies.
///
/// Real Factorio resource entities sit at **tile centres** — the capture in
/// `live-2.1.17-entities-resources.json` is all `x.5`/`y.5`. `EntityGraph`
/// stores them in `resources: DashMap<String, Vec<Pos>>`, and `Pos` is
/// `(i32, i32)`: `From<&Position> for Pos` floors, `From<&Pos> for Position`
/// widens the integer straight back. So an ore at `(-46.5, -52.5)` went in as
/// `Pos(-47, -53)` and came back out as `(-47.0, -53.0)` — a tile *corner*, a
/// position no entity is ever at.
///
/// That position is what the planner puts in a `Mine` action and what the
/// executor sends to `action_start_mining`, whose
/// `surface.find_entity(name, position)` matches the entity position
/// **exactly**. It therefore found nothing, every time, for any ore: the live
/// run reported `Error: no entity to mine` while standing 0.83 tiles from 17
/// reachable iron-ore entities.
///
/// No existing test could see this, because `test_utils::spawn_ore` builds ore
/// from `rect_fields`, which emits *integer* positions — the one input for
/// which the lossy round-trip happens to be lossless.
#[test]
fn resource_patch_elements_are_positions_real_ore_entities_occupy() {
    let entities: Vec<FactorioEntity> = serde_json::from_str(ENTITIES_RESOURCES)
        .unwrap_or_else(|err| panic!("live resource entities must parse: {err}"));

    let occupied: BTreeSet<(String, String)> = entities
        .iter()
        .filter(|entity| entity.entity_type == "resource")
        .map(|entity| {
            (
                entity.name.clone(),
                format!("{},{}", entity.position.x(), entity.position.y()),
            )
        })
        .collect();
    assert!(
        occupied.iter().any(|(_, pos)| pos.contains(".5")),
        "the capture must contain half-tile positions, or this proves nothing"
    );

    let graph = entity_graph_from(entities).expect("the live entities build a graph");
    for name in ["iron-ore", "stone"] {
        for patch in graph.resource_patches(name) {
            for element in &patch.elements {
                let key = (name.to_string(), format!("{},{}", element.x(), element.y()));
                assert!(
                    occupied.contains(&key),
                    "resource_patches reported {name} at ({}, {}), where the live \
                     game has no {name} entity; find_entity matches exactly, so \
                     action_start_mining can never resolve this position",
                    element.x(),
                    element.y(),
                );
            }
        }
    }
}

/// The inserter branch of `serialize_entity`.
///
/// It used to send `pickupPosition` in camelCase, which `FactorioEntity`
/// (`rename_all = "snake_case"`) never read — every inserter arrived with
/// `pickup_position: None`, so `EntityGraph::connect` silently never linked an
/// inserter to what it picks up from. That is an `Option`, so it failed
/// quietly; only a capture from a real game could show it.
#[test]
fn the_live_spawn_entities_reply_carries_an_inserter_pickup_position() {
    let raw: serde_json::Value = serde_json::from_str(ENTITIES_SPAWN).expect("parses as json");
    let inserter = raw
        .as_array()
        .expect("an array of entities")
        .iter()
        .find(|entity| entity["entity_type"] == "inserter")
        .expect("an inserter was placed before this capture");

    assert!(
        inserter.get("pickupPosition").is_none(),
        "the camelCase spelling nothing reads must not come back: {inserter}"
    );
    let pickup = inserter
        .get("pickup_position")
        .expect("snake_case is what FactorioEntity reads");
    assert_eq!(pickup["x"], 5.5);
    assert_eq!(pickup["y"], 2.5);
    assert!(
        inserter.get("drop_position").is_some(),
        "an inserter also reports where it drops"
    );
}

/// The 2.0 inventory array shape, on entities rather than on a player.
#[test]
fn the_live_spawn_entities_reply_carries_array_shaped_container_inventories() {
    let raw: serde_json::Value = serde_json::from_str(ENTITIES_SPAWN).expect("parses as json");
    let spaceship = raw
        .as_array()
        .expect("an array of entities")
        .iter()
        .find(|entity| entity["name"] == "crash-site-spaceship")
        .expect("the crash site is at every vanilla spawn");

    let inventory = &spaceship["output_inventory"];
    assert!(
        inventory.is_array(),
        "a non-empty inventory arrives as an array in 2.0+, got {inventory}"
    );
    assert_eq!(inventory[0]["name"], "firearm-magazine");
    assert_eq!(inventory[0]["count"], 8);
    assert_eq!(inventory[0]["quality"], "normal");
}

/// **Formerly a known defect; this is the fixture that caught it.**
///
/// Lua cannot hold a `nil` in a table, so an *empty* inventory reaches Rust as
/// `{}` — a map — while a non-empty one is an array. `FactorioEntity` used to
/// declare both inventories as plain `Option<Vec<..>>` with no tolerance for
/// the empty map, so the first entity anyone places crashed the run:
/// `OutputParser` unwrapped this parse and panicked the whole process, which
/// is what a live `place_entity` of a stone furnace did during this capture.
///
/// Fixed by `deserialize_helpers::option_vec_or_empty_map` in
/// `crates/core/src/types.rs`, applied to `FactorioEntity::{output_inventory,
/// fuel_inventory}` (and to the same two fields on `InventoryResponse` and
/// `ChunkObject`).
#[test]
fn an_empty_entity_inventory_from_the_live_game_deserialises() {
    let raw: serde_json::Value = serde_json::from_str(ENTITIES_SPAWN).expect("parses as json");
    let furnace = raw
        .as_array()
        .expect("an array of entities")
        .iter()
        .find(|entity| entity["name"] == "stone-furnace")
        .expect("a stone furnace was placed before this capture");
    assert_eq!(
        furnace["output_inventory"],
        serde_json::json!({}),
        "an empty inventory arrives as an empty map, not an empty array"
    );
    assert_eq!(furnace["fuel_inventory"], serde_json::json!({}));

    let entities: Vec<FactorioEntity> = serde_json::from_str(ENTITIES_SPAWN)
        .unwrap_or_else(|err| panic!("{err} in {ENTITIES_SPAWN}"));
    let furnace = entities
        .iter()
        .find(|entity| entity.name == "stone-furnace")
        .expect("a stone furnace was placed before this capture");
    assert_eq!(
        furnace.output_inventory,
        Some(Vec::new()),
        "an empty map must deserialise to an empty (not absent) inventory"
    );
    assert_eq!(furnace.fuel_inventory, Some(Vec::new()));
}

// ---------------------------------------------------------------------------
// InventoryResponse — `remote.call('botbridge', 'inventory_contents_at')`
// ---------------------------------------------------------------------------

/// The mod used to answer this call with `outputInventory` / `fuelInventory`
/// in camelCase, which `InventoryResponse` (`rename_all = "snake_case"`) never
/// read. Both fields are optional, so this did not fail loudly: every reply
/// deserialised happily with *both inventories `None`*, and the caller was
/// told an empty chest and a full one look the same. Fixed in
/// `mods/BotBridge/control.lua`; this asserts the names that reach the wire
/// now.
#[test]
fn the_live_inventory_contents_at_reply_uses_snake_case_field_names() {
    let raw: serde_json::Value =
        serde_json::from_str(INVENTORY_CONTENTS_AT).expect("parses as json");
    let record = &raw.as_array().expect("an array of records")[0];

    assert!(
        record.get("outputInventory").is_none(),
        "the camelCase spelling nothing reads must not come back: {record}"
    );
    assert_eq!(record["name"], "crash-site-spaceship");
    let inventory = record
        .get("output_inventory")
        .expect("snake_case is what InventoryResponse reads");
    assert_eq!(inventory[0]["name"], "firearm-magazine");
    assert_eq!(inventory[0]["count"], 8);
}

/// The typed parse, and what an absent inventory means.
///
/// Lua drops nil values from a table, so a container — which has no fuel
/// inventory — sends no `fuel_inventory` key at all. Both fields tolerate that
/// and come back `None`, which is exactly why the camelCase spelling above was
/// silent for so long: "absent" and "misspelled" are indistinguishable to the
/// caller. Asserting the *present* inventory is non-`None` is what makes this
/// test able to fail if the names ever drift again.
#[test]
fn the_live_inventory_contents_at_reply_deserialises_into_inventory_response() {
    let records: Vec<InventoryResponse> = serde_json::from_str(INVENTORY_CONTENTS_AT)
        .unwrap_or_else(|err| panic!("{err} in {INVENTORY_CONTENTS_AT}"));

    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.name, "crash-site-spaceship");
    assert_eq!(record.position.x, -5.0);
    assert_eq!(record.position.y, -6.0);

    let output = record
        .output_inventory
        .as_ref()
        .as_ref()
        .expect("the crash site container holds firearm magazines");
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].name, "firearm-magazine");
    assert_eq!(output[0].count, 8);
    assert_eq!(output[0].quality, "normal");

    assert!(
        record.fuel_inventory.is_none(),
        "a container has no fuel inventory, so the mod sends no key"
    );
}

// ---------------------------------------------------------------------------
// Direction — `defines.direction` on the 2.x scale
// ---------------------------------------------------------------------------

/// The live round trip, taken from the game rather than from a test's own
/// assumptions.
///
/// The starting-area inserter arrives with `direction: 4`, and the *geometry*
/// the same reply carries says which direction that is. It sits at
/// `(4.5, 2.5)`, picks up from `(5.5, 2.5)` — one tile **east** — and drops at
/// `(3.3, 2.5)`, to the **west**. This codebase's convention (see
/// `FactorioEntity::new_inserter`) is that an inserter picks up from the
/// direction it faces, so the game's `4` is **east**.
///
/// That is exactly what direction equality alone cannot show. Before the
/// widening, `Direction::from_u8(4)` was `South`, which would put the pickup at
/// `(4.5, 3.5)` — and a test that only checked `from_u8(4).to_u8() == 4` would
/// have passed against that too, because the error was symmetric on the way in
/// and the way out. The pickup and drop positions break the symmetry: they come
/// from Factorio, not from us.
///
/// This is the claim the whole widening rests on, and it needs no running game
/// to re-check — the reply is captured byte for byte in
/// `live-2.1.17-entities-spawn.json`.
#[test]
fn the_live_inserters_direction_and_geometry_agree_that_four_is_east() {
    let entities: Vec<FactorioEntity> =
        serde_json::from_str(ENTITIES_SPAWN).expect("parses as entities");
    let inserter = entities
        .iter()
        .find(|entity| entity.name == "inserter")
        .expect("the starting area has one inserter");

    assert_eq!(inserter.direction, 4, "the game sent 4");
    assert_eq!(
        Direction::from_u8(inserter.direction),
        Some(Direction::East),
        "4 is east in Factorio 2.x defines.direction; it read back as South before the widening"
    );

    let position = &inserter.position;
    let pickup = inserter.pickup_position.as_ref().expect("has a pickup");
    let drop = inserter.drop_position.as_ref().expect("has a drop");

    assert!(
        pickup.x() > position.x() && (pickup.y() - position.y()).abs() < f64::EPSILON,
        "the game put the pickup due east of the inserter: {position} -> {pickup}"
    );
    assert!(
        drop.x() < position.x() && (drop.y() - position.y()).abs() < f64::EPSILON,
        "the game put the drop due west of the inserter: {position} -> {drop}"
    );

    // And the reconstruction agrees: building the same inserter facing East
    // lands the pickup on the tile the game reports.
    let ours = FactorioEntity::new_inserter(position, Direction::East);
    let our_pickup = ours.pickup_position.expect("we compute a pickup");
    assert_eq!(
        (our_pickup.x(), our_pickup.y()),
        (pickup.x(), pickup.y()),
        "our East inserter must pick up from the tile the live game named"
    );
}

/// Nothing in a byte-for-byte 2.1.17 capture carries a direction the enum
/// cannot read, now that it covers all sixteen.
///
/// Under the old eight-value enum this file contained a value (`4`) that was
/// *readable but wrong*, which is why a range check alone never caught it.
#[test]
fn every_direction_in_the_live_captures_is_readable() {
    for (name, json) in [
        ("entities-spawn", ENTITIES_SPAWN),
        ("entities-resources", ENTITIES_RESOURCES),
    ] {
        let entities: Vec<FactorioEntity> =
            serde_json::from_str(json).unwrap_or_else(|err| panic!("{name}: {err}"));
        for entity in entities {
            assert!(
                Direction::from_u8(entity.direction).is_some(),
                "{name}: {} at {} reports direction {}, outside defines.direction",
                entity.name,
                entity.position,
                entity.direction
            );
        }
    }
}
