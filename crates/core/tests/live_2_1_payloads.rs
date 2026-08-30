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
use factorio_bot_core::types::{FactorioEntity, FactorioPlayer, FactorioTile, InventoryResponse};

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
    assert_eq!(snapshot.recipes.len(), 23);
    assert_eq!(snapshot.forces.len(), 1, "only the player force is sent");
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

/// **Known defect, and the reason this fixture exists.**
///
/// Lua cannot hold a `nil` in a table, so an *empty* inventory reaches Rust as
/// `{}` — a map — while a non-empty one is an array. `FactorioEntity` declares
/// both inventories as plain `Option<Vec<..>>` with no tolerance for the empty
/// map, so the first entity anyone places crashes the run: `OutputParser`
/// unwraps this parse and panics the whole process, which is what a live
/// `place_entity` of a stone furnace did during this capture.
///
/// `crates/core/src/types.rs` already has the fix in it —
/// `deserialize_helpers::vec_or_empty_map`, used by
/// `FactorioTechnology::research_unit_ingredients` and
/// `PlayerChangedMainInventoryEvent::main_inventory` — it is simply not applied
/// to `FactorioEntity`'s two inventory fields or to `InventoryResponse`'s.
/// That file was being edited concurrently and is deliberately untouched here.
///
/// When it is applied, this test will fail. That is the intent: replace it with
/// the positive assertion that the furnace parses with two empty inventories.
#[test]
fn an_empty_entity_inventory_from_the_live_game_does_not_yet_deserialise() {
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

    let err = serde_json::from_str::<Vec<FactorioEntity>>(ENTITIES_SPAWN)
        .expect_err("see this test's doc comment: vec_or_empty_map is not applied yet");
    assert!(
        err.to_string().contains("invalid type: map"),
        "the failure must still be the empty-map inventory, got: {err}"
    );
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
