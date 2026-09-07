//! The BotBridge mod is Lua that only ever runs inside Factorio, so nothing in
//! the Rust suite exercised it. That is how `serialize_product` kept asking the
//! game for a `probability` field that Factorio 2.1 does not have, and
//! `serialize_recipe` kept asking for `category`, which 2.1 replaced with
//! `categories`.
//!
//! These tests load the real `mods/BotBridge/types.lua` into a Lua 5.4 state
//! and feed it the table shapes `workspace/factorio-api-docs/runtime-api.json`
//! describes for 2.1.

use factorio_bot_core::blueprint::UndergroundHalf;
use factorio_bot_core::types::{FactorioEntity, FactorioRecipe};
use mlua::{Function, Lua, LuaSerdeExt, Table, Value};

/// `defines.inventory.crafter_input` as this Factorio publishes it.
///
/// The *number* is not what any test here asserts -- the stubs below are
/// written against this constant, so the tests prove that `types.lua` asks
/// `get_inventory` for the index it named in `defines`, whatever that index
/// is. What matters is that the name exists: 2.1.17 has `crafter_input` and
/// has no `furnace_source` or `assembling_machine_input` at all.
const CRAFTER_INPUT: i64 = 2;
/// `defines.inventory.lab_input`, on the same terms as [`CRAFTER_INPUT`].
const LAB_INPUT: i64 = 1;
/// `defines.entity_status.no_ingredients` -- a machine that is STOPPED and has
/// a name for why. The number is arbitrary here for the same reason
/// [`CRAFTER_INPUT`]'s is: what the tests assert is that the serialiser
/// resolves whatever the game's own table says, never a number written down.
const NO_INGREDIENTS: i64 = 22;
/// `defines.entity_status.working`.
const WORKING: i64 = 1;
/// `defines.entity_status.waiting_for_space_in_destination` -- the
/// back-pressure half of idleness, which nothing derivable from the flow graph
/// can see and which 194 drills of the world-record base were sitting in.
const WAITING_FOR_SPACE: i64 = 5;
/// A status value the stubbed `defines` cannot name, standing in for an
/// archive read against a Factorio whose enum has grown.
const UNMAPPED_STATUS: i64 = 9_001;

/// Loads the mod's serialisers. The path is the same live reference a debug
/// build uses for `workspace/mods`.
///
/// A `defines` stub is installed because `serialize_entity` needs one to find
/// an entity's INPUT inventory: `LuaEntity` has `get_output_inventory` and
/// `get_fuel_inventory` but no input counterpart, so the index has to be
/// named through `defines.inventory`. `types.lua` reaches it with
/// `rawget(_G, "defines")` precisely so that this state -- a plain Lua 5.4
/// interpreter that is not a game -- gets `nil` instead of an error; without
/// the stub the input inventory would be unreachable from every test here and
/// the field would be untested rather than tested.
fn botbridge_types() -> Lua {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/BotBridge/types.lua");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {path:?}: {err}"));
    // `clippy.toml` bans `Lua::new` because interpreters that run *user*
    // scripts must come from the sandbox in `factorio-bot-scripting-lua` (which
    // core cannot depend on: it depends on core). This state runs one trusted
    // file from this repository and no user input at all.
    #[allow(clippy::disallowed_methods)]
    let lua = Lua::new();
    let inventory = lua.create_table().expect("table");
    inventory.set("crafter_input", CRAFTER_INPUT).expect("set");
    inventory.set("lab_input", LAB_INPUT).expect("set");
    // `defines.transport_line`, which the serialiser INVERTS to name a lane.
    // Deliberately not in index order and deliberately not the whole enum: the
    // mapping under test is value -> name, so an order-dependent
    // implementation reading it as a list would produce the wrong names here
    // rather than accidentally the right ones.
    let transport_line = lua.create_table().expect("table");
    for (name, index) in [
        ("right_line", 2),
        ("left_line", 1),
        ("left_underground_line", 3),
        ("right_underground_line", 4),
    ] {
        transport_line.set(name, index).expect("set");
    }
    // `defines.entity_status`, which the serialiser INVERTS to name a status.
    // Four of the seventy-two 2.1.17 members and deliberately not in value
    // order, on the same terms as `transport_line` above: the mapping under
    // test is value -> name, so an implementation reading this as a list would
    // produce wrong names here rather than accidentally right ones. Leaving
    // most of the enum out is what makes [`UNMAPPED_STATUS`] a real case.
    let entity_status = lua.create_table().expect("table");
    for (name, value) in [
        ("no_ingredients", NO_INGREDIENTS),
        ("working", WORKING),
        ("waiting_for_space_in_destination", WAITING_FOR_SPACE),
        ("no_power", 12),
    ] {
        entity_status.set(name, value).expect("set");
    }
    let defines = lua.create_table().expect("table");
    defines.set("inventory", inventory).expect("set");
    defines.set("transport_line", transport_line).expect("set");
    defines.set("entity_status", entity_status).expect("set");
    lua.globals().set("defines", defines).expect("set");
    lua.load(&source)
        .set_name("types.lua")
        .exec()
        .expect("types.lua loads");
    lua
}

fn call(lua: &Lua, function: &str, argument: Table) -> Table {
    let function: Function = lua
        .globals()
        .get(function)
        .expect("the mod defines the function");
    function.call(argument).expect("the serialiser runs")
}

/// An `ItemProduct` as Factorio 2.1 reports it: no `probability` key at all.
fn product_2_1(lua: &Lua) -> Table {
    let shared = lua.create_table().expect("table");
    shared.set("min", 0.0).expect("set");
    shared.set("max", 1.0).expect("set");
    let product = lua.create_table().expect("table");
    product.set("name", "wooden-chest").expect("set");
    product.set("type", "item").expect("set");
    product.set("amount", 1).expect("set");
    product.set("independent_probability", 1.0).expect("set");
    product.set("shared_probability", shared).expect("set");
    product
}

#[test]
fn serialize_product_forwards_what_factorio_2_1_reports() {
    let lua = botbridge_types();
    let out = call(&lua, "serialize_product", product_2_1(&lua));

    assert_eq!(out.get::<String>("name").expect("name"), "wooden-chest");
    assert_eq!(out.get::<String>("product_type").expect("type"), "item");
    assert_eq!(out.get::<u32>("amount").expect("amount"), 1);
    assert!(
        matches!(
            out.get::<Value>("probability").expect("probability"),
            Value::Nil
        ),
        "2.1 has no probability field to send"
    );
    assert_eq!(
        out.get::<f64>("independent_probability")
            .expect("independent_probability must be forwarded"),
        1.0
    );
    let shared: Table = out
        .get("shared_probability")
        .expect("shared_probability must be forwarded");
    assert_eq!(shared.get::<f64>("max").expect("max"), 1.0);
}

/// Randomised outputs have no `amount` in 2.1, only a range.
#[test]
fn serialize_product_forwards_a_2_1_amount_range() {
    let lua = botbridge_types();
    let product = lua.create_table().expect("table");
    product.set("name", "uranium-235").expect("set");
    product.set("type", "item").expect("set");
    product.set("amount_min", 1).expect("set");
    product.set("amount_max", 3).expect("set");
    product.set("independent_probability", 0.007).expect("set");

    let out = call(&lua, "serialize_product", product);
    assert!(matches!(
        out.get::<Value>("amount").expect("amount"),
        Value::Nil
    ));
    assert_eq!(out.get::<u32>("amount_min").expect("amount_min"), 1);
    assert_eq!(out.get::<u32>("amount_max").expect("amount_max"), 3);
    assert_eq!(
        out.get::<f64>("independent_probability")
            .expect("independent_probability"),
        0.007
    );
}

fn recipe_table(lua: &Lua, category_key: &str, category: Value) -> Table {
    let group = lua.create_table().expect("table");
    group.set("name", "intermediate-products").expect("set");
    let subgroup = lua.create_table().expect("table");
    subgroup.set("name", "raw-material").expect("set");
    let ingredient = lua.create_table().expect("table");
    ingredient.set("name", "iron-ore").expect("set");
    ingredient.set("type", "item").expect("set");
    ingredient.set("amount", 1).expect("set");
    let ingredients = lua.create_table().expect("table");
    ingredients.set(1, ingredient).expect("set");
    let products = lua.create_table().expect("table");
    products.set(1, product_2_1(lua)).expect("set");

    let recipe = lua.create_table().expect("table");
    recipe.set("name", "iron-plate").expect("set");
    recipe.set("valid", true).expect("set");
    recipe.set("enabled", true).expect("set");
    recipe.set("hidden", false).expect("set");
    recipe.set("energy", 3.2).expect("set");
    recipe.set("order", "b[iron-plate]").expect("set");
    recipe.set(category_key, category).expect("set");
    recipe.set("ingredients", ingredients).expect("set");
    recipe.set("products", products).expect("set");
    recipe.set("group", group).expect("set");
    recipe.set("subgroup", subgroup).expect("set");
    recipe
}

/// 2.1 has `categories`, an array. The planner branches on the single category
/// (`crates/planner/src/method/have.rs`), so it has to keep arriving.
#[test]
fn serialize_recipe_takes_the_category_from_the_2_1_categories_array() {
    let lua = botbridge_types();
    let categories = lua.create_table().expect("table");
    categories.set(1, "smelting").expect("set");
    let recipe = recipe_table(&lua, "categories", Value::Table(categories));

    let out = call(&lua, "serialize_recipe", recipe);
    assert_eq!(out.get::<String>("category").expect("category"), "smelting");
}

/// A workspace that still holds a pre-2.1 game reports the old single string.
#[test]
fn serialize_recipe_still_reads_a_2_0_category() {
    let lua = botbridge_types();
    let category = lua.create_string("smelting").expect("string");
    let recipe = recipe_table(&lua, "category", Value::String(category));

    let out = call(&lua, "serialize_recipe", recipe);
    assert_eq!(out.get::<String>("category").expect("category"), "smelting");
}

/// The loop the live run broke on: what the mod emits for a 2.1 recipe has to
/// deserialise into `FactorioRecipe`.
#[test]
fn a_serialised_2_1_recipe_deserialises_into_factorio_recipe() {
    let lua = botbridge_types();
    let categories = lua.create_table().expect("table");
    categories.set(1, "smelting").expect("set");
    let recipe = recipe_table(&lua, "categories", Value::Table(categories));

    let out = call(&lua, "serialize_recipe", recipe);
    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the serialised recipe converts to json");
    let recipe: FactorioRecipe =
        serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"));

    assert_eq!(recipe.name, "iron-plate");
    assert_eq!(recipe.category, "smelting");
    assert_eq!(recipe.ingredients.expect("ingredients")[0].amount, 1);
    assert_eq!(recipe.products[0].amount, 1);
    assert_eq!(
        *recipe.products[0].probability,
        noisy_float::types::r64(1.0)
    );
}

/// Factorio 2.0 renamed every collision layer, dropping the `-layer` suffix.
/// `collides_with('player-layer')` does not return false on 2.1, it **raises**
/// ("Unknown collision-layer name"), which took the whole
/// `find_tiles_filtered` RCON call down and returned an error string in place
/// of JSON. Pinning the argument here is what keeps the name from drifting
/// back: the stub fails the call for anything but the 2.x spelling.
#[test]
fn serialize_tile_asks_for_the_2_1_collision_layer_name() {
    let lua = botbridge_types();
    let position = lua.create_table().expect("table");
    position.set("x", 3.5).expect("set");
    position.set("y", -4.5).expect("set");

    let tile = lua.create_table().expect("table");
    tile.set("name", "water").expect("set");
    tile.set("position", position).expect("set");
    // Stands in for `LuaTile::collides_with`, which only accepts a
    // `CollisionLayerID` the running game knows.
    let collides_with = lua
        .create_function(|_, layer: String| {
            if layer != "player" {
                return Err(mlua::Error::RuntimeError(format!(
                    "Unknown collision-layer name: {layer}"
                )));
            }
            Ok(true)
        })
        .expect("function");
    tile.set("collides_with", collides_with).expect("set");

    let out = call(&lua, "serialize_tile", tile);
    assert_eq!(out.get::<String>("name").expect("name"), "water");
    assert!(
        out.get::<bool>("player_collidable")
            .expect("player_collidable"),
        "water collides with the player layer"
    );
}

fn inserter_table(lua: &Lua) -> Table {
    let point = |x: f64, y: f64| {
        let table = lua.create_table().expect("table");
        table.set("x", x).expect("set");
        table.set("y", y).expect("set");
        table
    };
    let bounding_box = lua.create_table().expect("table");
    bounding_box
        .set("left_top", point(4.35, 2.35))
        .expect("set");
    bounding_box
        .set("right_bottom", point(4.65, 2.65))
        .expect("set");

    let entity = lua.create_table().expect("table");
    entity.set("name", "inserter").expect("set");
    entity.set("type", "inserter").expect("set");
    entity.set("direction", 4).expect("set");
    entity.set("position", point(4.5, 2.5)).expect("set");
    entity.set("drop_position", point(3.3, 2.5)).expect("set");
    entity.set("pickup_position", point(5.5, 2.5)).expect("set");
    entity.set("bounding_box", bounding_box).expect("set");
    // An inserter has neither inventory; the serialiser calls both.
    for getter in ["get_output_inventory", "get_fuel_inventory"] {
        let nothing = lua
            .create_function(|_, ()| Ok(Value::Nil))
            .expect("function");
        entity.set(getter, nothing).expect("set");
    }
    entity
}

/// `FactorioEntity` is `rename_all = "snake_case"` and its `pickup_position`
/// is an `Option`, so the `pickupPosition` this used to emit did not fail — it
/// silently arrived as `None` for every inserter in the world, and
/// `EntityGraph::connect` never linked one to what it picks up from.
#[test]
fn serialize_entity_sends_an_inserter_pickup_position_in_snake_case() {
    let lua = botbridge_types();
    let out = call(&lua, "serialize_entity", inserter_table(&lua));

    assert!(
        matches!(
            out.get::<Value>("pickupPosition").expect("pickupPosition"),
            Value::Nil
        ),
        "nothing in the tree reads the camelCase spelling"
    );
    let pickup: Table = out
        .get("pickup_position")
        .expect("pickup_position must be sent");
    assert_eq!(pickup.get::<f64>("x").expect("x"), 5.5);
    assert_eq!(pickup.get::<f64>("y").expect("y"), 2.5);
}

/// **This bug class has now bitten twice, and every other test in this file
/// is blind to it.** `pickup_position` first (comment above), and then
/// `underground_half`: a first attempt at reading an underground-belt's
/// input/output half back out of the game named the field
/// `belt_to_ground_type` (Factorio's own name for it) in the Lua table, which
/// matched nothing on `FactorioEntity` (whose field is `underground_half`) --
/// serde silently drops an unrecognised key, so the round trip came back
/// `None` with no error anywhere. Confirmed live: a raw `remote.call` showed
/// the mod's JSON carrying the right value, and every `rcon.find_entities_*`
/// read through the Rust struct still reported `nil`.
///
/// Every test above this one only checks the LUA TABLE `serialize_entity`
/// returns -- `serialize_entity_sends_an_inserter_pickup_position_in_snake_case`
/// asserts a key exists in the table, never that `FactorioEntity` actually
/// deserialises it. That is exactly the gap both bugs lived in: a key can be
/// present, correctly spelled even, and still not survive the trip into the
/// typed struct a caller actually reads (wrong name, wrong shape, a renamed
/// field). So these two tests go the whole way: real mod source, through
/// `serialize_entity`, through the same `LuaSerdeExt` + `serde_json`
/// round-trip `a_serialised_2_1_recipe_deserialises_into_factorio_recipe`
/// uses, into a real `FactorioEntity`, and assert the field survives THERE.
#[test]
fn a_serialised_inserter_deserialises_into_factorio_entity_with_pickup_position() {
    let lua = botbridge_types();
    let out = call(&lua, "serialize_entity", inserter_table(&lua));

    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the serialised entity converts to json");
    let entity: FactorioEntity =
        serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"));

    let pickup = entity
        .pickup_position
        .expect("pickup_position must survive into FactorioEntity, not just the Lua table");
    assert_eq!(pickup.x, 5.5);
    assert_eq!(pickup.y, 2.5);
}

fn underground_belt_table(lua: &Lua, half: &str) -> Table {
    let point = |x: f64, y: f64| {
        let table = lua.create_table().expect("table");
        table.set("x", x).expect("set");
        table.set("y", y).expect("set");
        table
    };
    let bounding_box = lua.create_table().expect("table");
    bounding_box
        .set("left_top", point(0.1015625, 10.1015625))
        .expect("set");
    bounding_box
        .set("right_bottom", point(0.8984375, 10.8984375))
        .expect("set");

    let entity = lua.create_table().expect("table");
    entity.set("name", "underground-belt").expect("set");
    entity.set("type", "underground-belt").expect("set");
    entity.set("direction", 4).expect("set");
    entity.set("position", point(0.5, 10.5)).expect("set");
    entity.set("bounding_box", bounding_box).expect("set");
    // `entity.belt_to_ground_type` -- Factorio's own field, confirmed live
    // against a real game to read "input"/"output" (never nil, even for an
    // underground-belt created with no `type` argument at all).
    entity.set("belt_to_ground_type", half).expect("set");
    for getter in ["get_output_inventory", "get_fuel_inventory"] {
        let nothing = lua
            .create_function(|_, ()| Ok(Value::Nil))
            .expect("function");
        entity.set(getter, nothing).expect("set");
    }
    entity
}

#[test]
fn a_serialised_underground_belt_deserialises_into_factorio_entity_with_its_half() {
    let lua = botbridge_types();

    let input_out = call(
        &lua,
        "serialize_entity",
        underground_belt_table(&lua, "input"),
    );
    let input_json: serde_json::Value = lua
        .from_value(Value::Table(input_out))
        .expect("the serialised entity converts to json");
    let input_entity: FactorioEntity = serde_json::from_value(input_json.clone())
        .unwrap_or_else(|err| panic!("{err} in {input_json}"));
    assert_eq!(
        input_entity.underground_half,
        Some(UndergroundHalf::Input),
        "underground_half must survive into FactorioEntity, not just Factorio's own \
         entity.belt_to_ground_type or the Lua table serialize_entity returns"
    );

    let output_out = call(
        &lua,
        "serialize_entity",
        underground_belt_table(&lua, "output"),
    );
    let output_json: serde_json::Value = lua
        .from_value(Value::Table(output_out))
        .expect("the serialised entity converts to json");
    let output_entity: FactorioEntity = serde_json::from_value(output_json.clone())
        .unwrap_or_else(|err| panic!("{err} in {output_json}"));
    assert_eq!(
        output_entity.underground_half,
        Some(UndergroundHalf::Output),
        "the two halves of a pair must not collapse to the same value"
    );
}

/// A `LuaTechnology` as `serialize_technology` reads it, with `trigger` as
/// the Lua source of `prototype.research_trigger`. Only the fields the
/// serialiser touches are present; `table_properties` pcalls the rest.
fn technology_with_trigger(lua: &Lua, trigger: &str) -> Table {
    lua.load(format!(
        r#"
        return {{
            name = "oil-processing", enabled = true, upgrade = false, order = "e-b",
            researched = false, level = 1, valid = true,
            research_unit_count = 1, research_unit_energy = 0,
            research_unit_ingredients = {{}},
            prerequisites = {{ {{ name = "oil-gathering" }} }},
            prototype = {{
                effects = {{ {{ type = "unlock-recipe", recipe = "oil-refinery" }} }},
                research_trigger = {trigger},
            }},
        }}
        "#
    ))
    .eval()
    .expect("the technology table builds")
}

fn trigger_of(lua: &Lua, trigger: &str) -> Table {
    call(
        lua,
        "serialize_technology",
        technology_with_trigger(lua, trigger),
    )
    .get("research_trigger")
    .expect("a technology with a trigger sends one")
}

fn names(table: &Table, key: &str) -> Vec<String> {
    table
        .get::<Table>(key)
        .unwrap_or_else(|e| panic!("{key} is a list: {e}"))
        .sequence_values::<String>()
        .map(|v| v.expect("a name"))
        .collect()
}

/// The shape `data/base/prototypes/technology.lua` writes for
/// `oil-processing`: `entities = {"crude-oil"}`, a list, and no `count`.
/// Sent as an entity list with the count defaulted to one.
#[test]
fn serialize_technology_sends_a_mine_entity_triggers_entity_list() {
    let lua = botbridge_types();
    let out = trigger_of(
        &lua,
        r#"{ type = "mine-entity", entities = { "crude-oil" } }"#,
    );
    assert_eq!(out.get::<String>("type").expect("type"), "mine-entity");
    assert_eq!(names(&out, "entities"), vec!["crude-oil"]);
    assert_eq!(out.get::<u32>("count").expect("count"), 1);
}

/// The shape `runtime-api.json` 2.1.17 documents for the same trigger: a
/// singular `entity` string. Nobody has captured which of the two the runtime
/// actually hands the mod, so both are read, and both come out as the list.
#[test]
fn serialize_technology_reads_the_documented_singular_entity_too() {
    let lua = botbridge_types();
    let out = trigger_of(&lua, r#"{ type = "mine-entity", entity = "uranium-ore" }"#);
    assert_eq!(names(&out, "entities"), vec!["uranium-ore"]);

    // An `EntityIDFilter` is a table with a `name`, and `build-entity` is
    // documented with one; a list of them is read the same way. `count`
    // is forwarded when present.
    let out = trigger_of(
        &lua,
        r#"{ type = "build-entity", entities = { { name = "radar", quality = "normal" }, "lab" }, count = 3 }"#,
    );
    assert_eq!(out.get::<String>("type").expect("type"), "build-entity");
    assert_eq!(names(&out, "entities"), vec!["radar", "lab"]);
    assert_eq!(out.get::<u32>("count").expect("count"), 3);
}

/// A trigger that names nothing is sent as its bare type, exactly as every
/// non-craft trigger was until 2026-09-05, so the planner can tell
/// "undescribed" from "unsupported" by the payload's absence alone.
#[test]
fn serialize_technology_sends_the_bare_type_for_a_trigger_naming_nothing() {
    let lua = botbridge_types();
    let out = trigger_of(&lua, r#"{ type = "mine-entity", entities = {} }"#);
    assert_eq!(out.get::<String>("type").expect("type"), "mine-entity");
    assert!(matches!(
        out.get::<Value>("entities").expect("entities"),
        Value::Nil
    ));
    assert!(matches!(
        out.get::<Value>("count").expect("count"),
        Value::Nil
    ));

    // The shipped `captivity` trigger: `{type = "capture-spawner"}`, any
    // spawner. Nothing to name, nothing sent but the type.
    let out = trigger_of(&lua, r#"{ type = "capture-spawner" }"#);
    assert_eq!(out.get::<String>("type").expect("type"), "capture-spawner");
    assert!(matches!(
        out.get::<Value>("entity").expect("entity"),
        Value::Nil
    ));
}

/// The `craft-item` shape is unchanged: `item` is an `ItemIDFilter` table
/// at runtime and a string in the prototype data, and both still come out as
/// the item's name with the count defaulted to one. Every archived dump was
/// read this way and the planner's early tree depends on it.
#[test]
fn serialize_technology_keeps_the_craft_item_shape() {
    let lua = botbridge_types();
    let out = trigger_of(&lua, r#"{ type = "craft-item", item = { name = "lab" } }"#);
    assert_eq!(out.get::<String>("type").expect("type"), "craft-item");
    assert_eq!(out.get::<String>("item").expect("item"), "lab");
    assert_eq!(out.get::<u32>("count").expect("count"), 1);
    assert!(matches!(
        out.get::<Value>("entities").expect("entities"),
        Value::Nil
    ));

    let out = trigger_of(
        &lua,
        r#"{ type = "craft-item", item = "steel-plate", count = 50 }"#,
    );
    assert_eq!(out.get::<String>("item").expect("item"), "steel-plate");
    assert_eq!(out.get::<u32>("count").expect("count"), 50);
}

/// The remaining payloads, per the runtime definition: `craft-fluid` carries
/// `fluid` and `amount`, `send-item-to-orbit` an `item`, `capture-spawner`
/// an optional `entity`.
#[test]
fn serialize_technology_sends_the_other_trigger_payloads() {
    let lua = botbridge_types();
    let out = trigger_of(
        &lua,
        r#"{ type = "craft-fluid", fluid = "steam", amount = 200 }"#,
    );
    assert_eq!(out.get::<String>("fluid").expect("fluid"), "steam");
    assert_eq!(out.get::<f64>("amount").expect("amount"), 200.0);

    let out = trigger_of(
        &lua,
        r#"{ type = "send-item-to-orbit", item = { name = "satellite" } }"#,
    );
    assert_eq!(out.get::<String>("item").expect("item"), "satellite");

    let out = trigger_of(
        &lua,
        r#"{ type = "capture-spawner", entity = "biter-spawner" }"#,
    );
    assert_eq!(
        out.get::<String>("entity").expect("entity"),
        "biter-spawner"
    );
}

/// What the planner reads: the serialised record round-trips through the
/// Rust type with the entity list intact.
#[test]
fn a_serialised_mine_entity_trigger_loads_as_the_rust_variant() {
    use factorio_bot_core::types::{FactorioTechnology, ResearchTrigger};
    let lua = botbridge_types();
    let record = call(
        &lua,
        "serialize_technology",
        technology_with_trigger(
            &lua,
            r#"{ type = "mine-entity", entities = { "crude-oil" } }"#,
        ),
    );
    let json: serde_json::Value = lua
        .from_value(Value::Table(record))
        .expect("the record is plain data");
    let tech: FactorioTechnology =
        serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"));
    assert_eq!(
        tech.research_trigger,
        Some(ResearchTrigger::MineEntity {
            entities: vec!["crude-oil".into()],
            count: 1,
        })
    );
}

// --------------------------------------------------------------------------
// The structural guard: every key `serialize_entity` emits, in every branch.
// --------------------------------------------------------------------------

/// A minimal `LuaEntity` table of the given `type`, with the two inventory
/// getters the serialiser unconditionally calls. `contents` decides whether
/// they answer with an inventory or with nothing, so the two inventory keys
/// are reachable from at least one branch.
fn entity_table(lua: &Lua, name: &str, entity_type: &str, with_inventories: bool) -> Table {
    let point = |x: f64, y: f64| {
        let table = lua.create_table().expect("table");
        table.set("x", x).expect("set");
        table.set("y", y).expect("set");
        table
    };
    let bounding_box = lua.create_table().expect("table");
    bounding_box.set("left_top", point(0.1, 0.1)).expect("set");
    bounding_box
        .set("right_bottom", point(0.9, 0.9))
        .expect("set");

    let entity = lua.create_table().expect("table");
    entity.set("name", name).expect("set");
    entity.set("type", entity_type).expect("set");
    entity.set("direction", 4).expect("set");
    entity.set("position", point(0.5, 0.5)).expect("set");
    entity.set("bounding_box", bounding_box).expect("set");
    for getter in ["get_output_inventory", "get_fuel_inventory"] {
        let getter_fn = if with_inventories {
            lua.create_function(|lua, ()| {
                // A `LuaInventory` as the serialiser uses it: only
                // `get_contents()`, whose 2.x shape is a list of
                // `{name, count, quality}`.
                let slot = lua.create_table()?;
                slot.set("name", "coal")?;
                slot.set("count", 3)?;
                slot.set("quality", "normal")?;
                let contents = lua.create_table()?;
                contents.set(1, slot)?;
                let inventory = lua.create_table()?;
                inventory.set(
                    "get_contents",
                    lua.create_function(move |_, ()| Ok(contents.clone()))?,
                )?;
                Ok(Value::Table(inventory))
            })
            .expect("function")
        } else {
            lua.create_function(|_, ()| Ok(Value::Nil))
                .expect("function")
        };
        entity.set(getter, getter_fn).expect("set");
    }
    // The INPUT inventory, which has no getter of its own: `serialize_entity`
    // resolves an index through `defines.inventory` and calls
    // `get_inventory(index)`. Answering `nil` for anything but the index the
    // mod named is what makes this a check rather than a rubber stamp -- a
    // serialiser that asked for the wrong inventory would get nothing and the
    // key would silently not appear.
    let get_inventory = if with_inventories {
        lua.create_function(|lua, index: i64| {
            if index != CRAFTER_INPUT && index != LAB_INPUT {
                return Ok(Value::Nil);
            }
            Ok(Value::Table(inventory_holding(lua, "iron-ore", 34)?))
        })
        .expect("function")
    } else {
        lua.create_function(|_, _index: i64| Ok(Value::Nil))
            .expect("function")
    };
    entity.set("get_inventory", get_inventory).expect("set");
    // Belt lanes, for the types that have them. `get_max_transport_line_index`
    // is declared for `TransportBeltConnectable` only, so a furnace and a tree
    // must not have the method at all -- giving every stub one would make the
    // serialiser's guard untestable and let a read that raises on a real
    // furnace pass here.
    if matches!(
        entity_type,
        "transport-belt" | "underground-belt" | "splitter" | "loader" | "loader-1x1"
    ) {
        let lanes = if entity_type == "underground-belt" {
            4
        } else {
            2
        };
        entity
            .set(
                "get_max_transport_line_index",
                lua.create_function(move |_, ()| Ok(lanes))
                    .expect("function"),
            )
            .expect("set");
        entity
            .set(
                "get_transport_line",
                lua.create_function(move |lua, index: i64| {
                    // Lane 1 carries something and the rest run empty, so a
                    // serialiser that reported only non-empty lanes, or only
                    // the first, fails rather than looking right.
                    let count = if index == 1 { 4 } else { 0 };
                    let line = lua.create_table()?;
                    let contents = inventory_holding(lua, "iron-ore", count)?
                        .get::<Function>("get_contents")?;
                    line.set("get_contents", contents)?;
                    Ok(line)
                })
                .expect("function"),
            )
            .expect("set");
    }
    entity
}

/// A `LuaInventory` as `serialize_entity` uses it: `get_contents()` only,
/// answering the 2.x list-of-`{name, count, quality}` shape. An empty
/// `contents` is a real and different answer -- an inventory that exists and
/// holds nothing -- so this takes a count rather than assuming one.
fn inventory_holding(lua: &Lua, item: &str, count: u32) -> mlua::Result<Table> {
    let contents = lua.create_table()?;
    if count > 0 {
        let slot = lua.create_table()?;
        slot.set("name", item)?;
        slot.set("count", count)?;
        slot.set("quality", "normal")?;
        contents.set(1, slot)?;
    }
    let inventory = lua.create_table()?;
    inventory.set(
        "get_contents",
        lua.create_function(move |_, ()| Ok(contents.clone()))?,
    )?;
    Ok(inventory)
}

/// Every branch of `serialize_entity`'s `elseif` chain, as a table the
/// serialiser can be handed. Adding a branch to the mod without adding a row
/// here leaves that branch unguarded -- which is the one thing this test
/// cannot check for itself, and the reason the table is written out by name
/// rather than derived.
fn every_serialize_entity_branch(lua: &Lua) -> Vec<(&'static str, Table)> {
    let resource = entity_table(lua, "iron-ore", "resource", false);
    resource.set("amount", 2500).expect("set");

    let inserter = inserter_table(lua);

    let ghost = entity_table(lua, "entity-ghost", "entity-ghost", false);
    ghost
        .set("ghost_name", "assembling-machine-1")
        .expect("set");
    ghost.set("ghost_type", "assembling-machine").expect("set");
    let recipe = lua.create_table().expect("table");
    recipe.set("name", "iron-gear-wheel").expect("set");
    ghost
        .set(
            "get_recipe",
            lua.create_function(move |_, ()| Ok(recipe.clone()))
                .expect("function"),
        )
        .expect("set");

    let assembler = entity_table(lua, "assembling-machine-1", "assembling-machine", true);
    // The one branch carrying a status, so `status` is among the keys this
    // guard sees. Every other stub leaves `entity.status` nil, which is the
    // ordinary case for a tree, a chest or a belt.
    assembler.set("status", WORKING).expect("set");
    let recipe = lua.create_table().expect("table");
    recipe.set("name", "iron-gear-wheel").expect("set");
    assembler
        .set(
            "get_recipe",
            lua.create_function(move |_, ()| Ok(recipe.clone()))
                .expect("function"),
        )
        .expect("set");

    let underground = underground_belt_table(lua, "input");

    // The final `else`: no branch of the chain applies at all.
    let plain = entity_table(lua, "transport-belt", "transport-belt", false);

    vec![
        ("resource", resource),
        ("inserter", inserter),
        ("entity-ghost", ghost),
        ("assembling-machine", assembler),
        ("underground-belt", underground),
        ("plain", plain),
    ]
}

/// **One test instead of one per field, because the tax was the problem.**
///
/// `serialize_entity` emits fourteen keys and `FactorioEntity` is plain
/// serde: a key whose name does not match a field is *dropped silently*,
/// with no error anywhere. That has now happened twice -- `pickupPosition`
/// for camelCase (every inserter arrived with `pickup_position: None` and
/// `EntityGraph::connect` linked nothing) and `belt_to_ground_type` for
/// Factorio's own spelling of `underground_half` (every live read came back
/// `nil` while a raw `remote.call` showed the value present). Each was
/// answered with a test about that one field, which guards that one field
/// and nothing else: the next mismatch is as invisible as the first two
/// were, and the next author pays a per-field tax to keep it that way.
///
/// So this drives EVERY branch of the serialiser's `elseif` chain and checks
/// each key it produces against `schema_for!(FactorioEntity)` -- the struct's
/// own declared shape, not a list written down here. A branch that emits a
/// key `FactorioEntity` has no field for fails, and the failure NAMES the
/// key. The one thing it cannot see is a new branch nobody added to
/// `every_serialize_entity_branch`, which is why that function says so.
///
/// **Deliberately not `deny_unknown_fields`**, which would answer the same
/// question at runtime: 24+ archived run records and a world dump were
/// written by older mods and must still deserialise, and a strict struct
/// would refuse every one of them.
#[test]
fn every_key_serialize_entity_emits_is_a_field_of_factorio_entity() {
    use factorio_bot_core::schemars::schema_for;

    let schema = serde_json::Value::from(schema_for!(FactorioEntity));
    let properties = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("FactorioEntity is an object schema with properties");
    let known: std::collections::BTreeSet<&str> = properties.keys().map(String::as_str).collect();

    let lua = botbridge_types();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (branch, table) in every_serialize_entity_branch(&lua) {
        let out = call(&lua, "serialize_entity", table);
        for pair in out.clone().pairs::<String, Value>() {
            let (key, _) = pair.expect("the serialised entity has string keys");
            assert!(
                known.contains(key.as_str()),
                "the `{branch}` branch of serialize_entity emits `{key}`, which is not a \
                 field of FactorioEntity -- serde will DROP it silently. Known fields: {known:?}"
            );
            seen.insert(key);
        }
        // And the whole record must survive the trip into the typed struct,
        // not merely name fields that exist.
        let json: serde_json::Value = lua
            .from_value(Value::Table(out))
            .expect("the serialised entity converts to json");
        let _: FactorioEntity = serde_json::from_value(json.clone())
            .unwrap_or_else(|err| panic!("{branch}: {err} in {json}"));
    }

    // The branches between them must reach every key the serialiser can
    // emit; a shrinking count would mean a branch stopped being exercised.
    assert_eq!(
        seen.len(),
        17,
        "serialize_entity emits seventeen distinct keys across its branches; saw {seen:?}"
    );
}

// --------------------------------------------------------------------------
// `volume`: how much a fluid box holds, which its connections cannot say.
// --------------------------------------------------------------------------

/// A `LuaFluidBoxPrototype` as `serialize_fluidbox_prototype` uses it.
/// `get_volume` is installed only when `volume` is `Some`, because the case
/// that matters is a prototype that does not have the METHOD at all -- an
/// older Factorio, or an attribute read against a game that only offers a
/// method. That case must leave the key absent and must not raise.
fn fluidbox_prototype(lua: &Lua, volume: Option<f64>) -> Table {
    let fluidbox = lua.create_table().expect("table");
    fluidbox
        .set("production_type", "input-output")
        .expect("set");
    fluidbox
        .set("pipe_connections", lua.create_table().expect("table"))
        .expect("set");
    if let Some(volume) = volume {
        fluidbox
            .set(
                "get_volume",
                lua.create_function(move |_, ()| Ok(volume))
                    .expect("function"),
            )
            .expect("set");
    }
    fluidbox
}

fn volume_of(lua: &Lua, volume: Option<f64>) -> Option<f64> {
    let out = call(
        lua,
        "serialize_fluidbox_prototype",
        fluidbox_prototype(lua, volume),
    );
    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the record is plain data");
    let prototype: factorio_bot_core::types::FactorioFluidBoxPrototype =
        serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"));
    prototype.volume
}

/// The capacity comes from CALLING `get_volume()`.
///
/// It is a method on `LuaFluidBoxPrototype` and there is no `volume` attribute
/// in 2.1.17 at all. Reading it as an attribute would hand back a function
/// rather than raise, `helpers.table_to_json` would drop it, and the field
/// would go missing with nothing anywhere to say it should not be -- which is
/// exactly how `crafting_speed` arrived nil for 1,028 prototypes.
#[test]
fn a_fluid_boxs_capacity_comes_from_calling_get_volume() {
    let lua = botbridge_types();
    assert_eq!(volume_of(&lua, Some(1000.0)), Some(1000.0));
}

/// And a prototype with no `get_volume` at all sends no key rather than a
/// zero: "this game cannot say" and "this box holds nothing" are different
/// claims, and a planner sizing storage must not read the first as the second.
#[test]
fn a_fluid_box_that_cannot_report_its_volume_says_nothing_rather_than_zero() {
    let lua = botbridge_types();
    assert_eq!(volume_of(&lua, None), None);
}

// --------------------------------------------------------------------------
// `status`: what the machine is DOING, which no inventory read can say.
// --------------------------------------------------------------------------

/// The record `serialize_entity` makes of an entity whose `status` attribute
/// reads `status`, as the typed struct.
///
/// `status` is an ATTRIBUTE on `LuaEntity` -- `optional: true`,
/// `subclasses: None` in 2.1.17's `runtime-api.json` -- so the stub carries a
/// plain value and not a function. Had it been a method, reading it as an
/// attribute would yield a function rather than raise, and the field would go
/// missing in silence; that is how `crafting_speed` arrived nil for 1,028
/// prototypes.
fn furnace_with_status(lua: &Lua, status: Option<i64>) -> FactorioEntity {
    let furnace = entity_table(lua, "stone-furnace", "furnace", true);
    if let Some(status) = status {
        furnace.set("status", status).expect("set");
    }
    let out = call(lua, "serialize_entity", furnace);
    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the serialised entity converts to json");
    serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"))
}

/// A stopped machine has a NAME for being stopped, and the name is what
/// crosses the wire.
///
/// The number never does. `defines.entity_status` is an enum whose numbering
/// is a Factorio implementation detail: an archived `22` would need that exact
/// version's table to be readable at all, and a version bump could silently
/// make it mean something else.
#[test]
fn a_stopped_machine_says_why_by_name() {
    let lua = botbridge_types();
    assert_eq!(
        furnace_with_status(&lua, Some(NO_INGREDIENTS)).status,
        Some("no_ingredients".to_owned()),
        "the name from the game's own defines.entity_status, never the number"
    );
    assert_eq!(
        furnace_with_status(&lua, Some(WAITING_FOR_SPACE)).status,
        Some("waiting_for_space_in_destination".to_owned()),
        "the back-pressure half of idleness must arrive distinguishable from \
         every other reason a machine is not running"
    );
    assert_eq!(
        furnace_with_status(&lua, Some(WORKING)).status,
        Some("working".to_owned())
    );
}

/// **Absent stays distinguishable from every named state**, which is the
/// whole point of the `Option`.
///
/// A tree, a chest and a belt have no status concept; the mod sends no key and
/// this reads as `None`, meaning *the sender did not say*. Every world dump
/// and run record written before the field existed reads the same way. A
/// machine that is merely *stopped* is a different answer and has a name for
/// itself -- so nothing may default the absent case to `working`, which would
/// invent a duty cycle out of an archive that never measured one.
#[test]
fn an_entity_with_no_status_says_nothing_rather_than_working() {
    let lua = botbridge_types();
    assert_eq!(
        furnace_with_status(&lua, None).status,
        None,
        "no status attribute must mean no key, not a fabricated `working`"
    );
    // And the same absence survives the trip a real archive takes: a record
    // written before the field existed carries no `status` at all.
    let archived: FactorioEntity = serde_json::from_value(serde_json::json!({
        "name": "stone-furnace",
        "entity_type": "furnace",
        "position": { "x": 0.5, "y": 0.5 },
        "bounding_box": {
            "left_top": { "x": 0.1, "y": 0.1 },
            "right_bottom": { "x": 0.9, "y": 0.9 },
        },
        "direction": 0,
    }))
    .expect("a record predating the field still deserialises");
    assert_eq!(archived.status, None);
}

/// A value this build's `defines` cannot name still reaches the record,
/// labelled as unresolved, rather than being dropped or written as a bare
/// integer nobody can decode later -- the rule `transport_line_name` follows.
#[test]
fn a_status_this_build_cannot_name_is_labelled_unmapped() {
    let lua = botbridge_types();
    assert_eq!(
        furnace_with_status(&lua, Some(UNMAPPED_STATUS)).status,
        Some(format!("unmapped_{UNMAPPED_STATUS}")),
    );
}

/// **The bulk path carries it, and that is not an oversight.**
///
/// `opts.omit_inventories` is the `writeout_entities` path -- every entity of
/// every chunk, item contents deliberately withheld -- and it is the *only*
/// path that fills the world model a dumped world is built from. A status
/// gated behind that flag would leave the duty cycle unmeasurable in exactly
/// the artefact the question is asked of. One short string is not an
/// item-by-item inventory.
#[test]
fn the_bulk_path_omits_inventories_and_still_carries_the_status() {
    let lua = botbridge_types();
    let furnace = entity_table(&lua, "stone-furnace", "furnace", true);
    furnace.set("status", NO_INGREDIENTS).expect("set");
    let opts = lua.create_table().expect("table");
    opts.set("omit_inventories", true).expect("set");
    let serialize: Function = lua
        .globals()
        .get("serialize_entity")
        .expect("the mod defines the function");
    let out: Table = serialize
        .call((furnace, opts))
        .expect("the serialiser runs");

    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the serialised entity converts to json");
    let entity: FactorioEntity =
        serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"));
    assert_eq!(
        entity.status,
        Some("no_ingredients".to_owned()),
        "the bulk writeout is what a dumped world is built from"
    );
    assert_eq!(
        entity.output_inventory, None,
        "and it must still withhold the contents it was asked to withhold"
    );
}

/// A technology whose prototype carries `effects` verbatim as Lua source.
fn technology_with_effects(lua: &Lua, effects: &str) -> Table {
    lua.load(format!(
        r#"
        return {{
            name = "steel-axe", enabled = true, upgrade = false, order = "c",
            researched = false, level = 1, valid = true,
            research_unit_count = 1, research_unit_energy = 0,
            research_unit_ingredients = {{}},
            prerequisites = {{ {{ name = "steel-processing" }} }},
            prototype = {{ effects = {effects} }},
        }}
        "#
    ))
    .eval()
    .expect("the technology table builds")
}

fn effects_of(lua: &Lua, effects: &str) -> Vec<(String, Option<f64>, Option<String>)> {
    call(
        lua,
        "serialize_technology",
        technology_with_effects(lua, effects),
    )
    .get::<Table>("effects")
    .expect("serialize_technology sends an effects list")
    .sequence_values::<Table>()
    .map(|entry| {
        let entry = entry.expect("an effect table");
        (
            entry
                .get::<String>("kind")
                .expect("every effect has a kind"),
            entry
                .get::<Option<f64>>("modifier")
                .expect("modifier reads"),
            entry.get::<Option<String>>("target").expect("target reads"),
        )
    })
    .collect()
}

/// The shape `data/base/prototypes/technology.lua` writes for `steel-axe`:
/// one `character-mining-speed` with `modifier = 1`.
///
/// **This is the datum the whole rate problem turned on.** The mod kept only
/// `unlock-recipe`, so nothing downstream could know that this technology
/// doubles hand mining, and the planner's inability to re-cost was a
/// consequence rather than the cause.
#[test]
fn serialize_technology_sends_a_character_mining_speed_effect() {
    let lua = botbridge_types();
    assert_eq!(
        effects_of(
            &lua,
            r#"{ { type = "character-mining-speed", modifier = 1 } }"#
        ),
        vec![("character-mining-speed".to_string(), Some(1.0), None)],
    );
}

/// `unlock-recipe` still reaches `unlocked_recipes`, and now also appears in
/// `effects` with its recipe as the target. Both keys are sent on purpose:
/// every existing consumer reads the narrow one.
#[test]
fn serialize_technology_keeps_unlocked_recipes_and_lists_the_same_effect() {
    let lua = botbridge_types();
    let out = call(
        &lua,
        "serialize_technology",
        technology_with_effects(
            &lua,
            r#"{ { type = "unlock-recipe", recipe = "steel-plate" },
                 { type = "character-mining-speed", modifier = 1 } }"#,
        ),
    );
    assert_eq!(
        names(&out, "unlocked_recipes"),
        vec!["steel-plate".to_string()],
        "the narrow key must not have moved",
    );
    let effects: Vec<String> = out
        .get::<Table>("effects")
        .expect("effects")
        .sequence_values::<Table>()
        .map(|e| e.expect("effect").get::<String>("kind").expect("kind"))
        .collect();
    assert_eq!(
        effects,
        vec![
            "unlock-recipe".to_string(),
            "character-mining-speed".to_string()
        ],
        "effects carries the whole list, unlock-recipe included",
    );
}

/// The two variants that do not spell their number `modifier`, and the
/// boolean ones. Checked against `TechnologyModifier`'s variant parameter
/// groups in `workspace/factorio-api-docs/runtime-api.json` at 2.1.17:
/// `change-recipe-productivity` is `{change, recipe}`, `give-item` is
/// `{count, item, quality}`, `mining-with-fluid` is `{modifier: boolean}`.
#[test]
fn serialize_technology_normalises_the_variants_that_spell_their_number_differently() {
    let lua = botbridge_types();
    assert_eq!(
        effects_of(
            &lua,
            r#"{ { type = "change-recipe-productivity", change = 0.25, recipe = "sulfur" },
                 { type = "give-item", count = 3, item = "iron-plate", quality = "normal" },
                 { type = "mining-with-fluid", modifier = true },
                 { type = "nothing", effect_description = "hello" } }"#
        ),
        vec![
            (
                "change-recipe-productivity".to_string(),
                Some(0.25),
                Some("sulfur".to_string())
            ),
            (
                "give-item".to_string(),
                Some(3.0),
                Some("iron-plate".to_string())
            ),
            ("mining-with-fluid".to_string(), Some(1.0), None),
            ("nothing".to_string(), None, None),
        ],
    );
}

/// The Lua the mod emits must load as the Rust type, which is the half a
/// serialiser test alone cannot check.
#[test]
fn a_serialised_technology_effect_loads_as_the_rust_type() {
    use factorio_bot_core::types::FactorioTechnology;
    let lua = botbridge_types();
    let out = call(
        &lua,
        "serialize_technology",
        technology_with_effects(
            &lua,
            r#"{ { type = "character-mining-speed", modifier = 1 },
                 { type = "unlock-recipe", recipe = "steel-plate" } }"#,
        ),
    );
    let technology: FactorioTechnology = lua.from_value(Value::Table(out)).expect("deserialises");
    assert_eq!(technology.effects.len(), 2);
    assert_eq!(technology.effects[0].kind, "character-mining-speed");
    assert_eq!(
        technology.effects[0]
            .modifier
            .as_deref()
            .copied()
            .map(f64::from),
        Some(1.0),
    );
    assert_eq!(technology.effects[1].target.as_deref(), Some("steel-plate"));
}

/// Every `LuaForce` rate bonus the mod now asks for reaches the record, and
/// the record deserialises into `FactorioForce`.
///
/// The names come from `runtime-api.json` 2.1.17, class `LuaForce`. This test
/// does not prove the game *has* them — nothing in a Rust suite can, and no
/// run has yet been made with this mod — it proves the serialiser asks for
/// each one by that name and forwards what it gets.
#[test]
fn serialize_force_sends_every_rate_bonus_it_asks_for() {
    use factorio_bot_core::types::FactorioForce;
    let lua = botbridge_types();
    let force: Table = lua
        .load(
            r#"
            return {
                name = "player", index = 1, research_progress = 0.5,
                current_research = nil,
                manual_mining_speed_modifier = 1,
                manual_crafting_speed_modifier = 0.25,
                character_running_speed_modifier = 0.5,
                laboratory_speed_modifier = 0.2,
                laboratory_productivity_bonus = 0.1,
                mining_drill_productivity_bonus = 0.3,
                inserter_stack_size_bonus = 2,
                bulk_inserter_capacity_bonus = 12,
                belt_stack_size_bonus = 4,
                worker_robots_speed_modifier = 0.35,
                technologies = {},
            }
            "#,
        )
        .eval()
        .expect("the force table builds");
    let out = call(&lua, "serialize_force", force);
    let force: FactorioForce = lua.from_value(Value::Table(out)).expect("deserialises");
    let read =
        |field: &Option<Box<noisy_float::types::R64>>| field.as_deref().copied().map(f64::from);
    assert_eq!(read(&force.manual_mining_speed_modifier), Some(1.0));
    assert_eq!(read(&force.manual_crafting_speed_modifier), Some(0.25));
    assert_eq!(read(&force.character_running_speed_modifier), Some(0.5));
    assert_eq!(read(&force.laboratory_speed_modifier), Some(0.2));
    assert_eq!(read(&force.laboratory_productivity_bonus), Some(0.1));
    assert_eq!(read(&force.mining_drill_productivity_bonus), Some(0.3));
    assert_eq!(read(&force.inserter_stack_size_bonus), Some(2.0));
    assert_eq!(read(&force.bulk_inserter_capacity_bonus), Some(12.0));
    assert_eq!(read(&force.belt_stack_size_bonus), Some(4.0));
    assert_eq!(read(&force.worker_robots_speed_modifier), Some(0.35));
}

/// A force payload captured before any of this existed — no `effects` key on
/// the technology and no bonus keys on the force — must still load. There are
/// 865 MB world dumps in this shape.
#[test]
fn a_payload_written_before_these_fields_existed_still_loads() {
    use factorio_bot_core::types::FactorioForce;
    let force: FactorioForce = serde_json::from_str(
        r#"{
          "name": "player", "force_id": 1,
          "current_research": null, "research_progress": null,
          "technologies": {
            "automation": {
              "name": "automation", "enabled": true, "upgrade": false,
              "researched": false, "prerequisites": null,
              "research_unit_ingredients": [], "research_unit_count": 10,
              "research_unit_energy": 30.0, "order": "a", "level": 1,
              "valid": true
            }
          }
        }"#,
    )
    .expect("an old payload must still parse");
    assert!(force.manual_crafting_speed_modifier.is_none());
    assert!(force.laboratory_speed_modifier.is_none());
    assert!(
        force.technologies["automation"].effects.is_empty(),
        "an absent effects key is an empty list, not a parse failure",
    );
}

// --------------------------------------------------------------------------
// The input inventory: what the machine was GIVEN and has not consumed.
// --------------------------------------------------------------------------

/// A `furnace` whose input inventory holds `count` of `item`. `count = 0` is
/// an inventory that exists and is empty, which is a different answer from
/// having none at all and is asserted as such below.
fn furnace_holding(lua: &Lua, item: &'static str, count: u32) -> Table {
    let entity = entity_table(lua, "stone-furnace", "furnace", true);
    entity
        .set(
            "get_inventory",
            lua.create_function(move |lua, index: i64| {
                if index != CRAFTER_INPUT {
                    return Ok(Value::Nil);
                }
                Ok(Value::Table(inventory_holding(lua, item, count)?))
            })
            .expect("function"),
        )
        .expect("set");
    entity
}

fn entity_through_serde(lua: &Lua, entity: Table) -> FactorioEntity {
    let out = call(lua, "serialize_entity", entity);
    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the serialised entity converts to json");
    serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"))
}

/// **The whole point of the field.** A peer session measured a production
/// plateau, eliminated ore exhaustion, arm starvation and a full belt, and
/// still could not say where 29 of 46 mined ore went -- because a furnace
/// sitting on ore it was not smelting and a furnace no ore had ever reached
/// serialised identically. Both had an empty `output_inventory` and neither
/// said anything at all about its input.
///
/// So this asserts the *distinction*, not merely that a key arrives: the two
/// furnaces differ only in what is in the input inventory, and the two
/// `FactorioEntity`s must differ too. It goes the whole way through
/// `LuaSerdeExt` and `serde_json` into the struct a caller actually reads,
/// for the reason the `pickup_position` and `underground_half` tests above
/// give: a correctly spelled key can still fail to reach a field, silently.
#[test]
fn a_furnace_holding_ore_is_distinguishable_from_one_that_never_received_any() {
    let lua = botbridge_types();

    let holding = entity_through_serde(&lua, furnace_holding(&lua, "iron-ore", 34));
    let empty = entity_through_serde(&lua, furnace_holding(&lua, "iron-ore", 0));

    let held = holding
        .input_inventory
        .as_ref()
        .expect("a furnace has an input inventory, so this must be Some");
    assert_eq!(held.len(), 1, "one item kind in {held:?}");
    assert_eq!(held[0].name, "iron-ore");
    assert_eq!(held[0].count, 34);

    assert_eq!(
        empty.input_inventory,
        Some(Vec::new()),
        "an empty input inventory is Some(empty) -- the furnace HAS one and it \
         is empty, which is not the same claim as having none",
    );
    assert_ne!(
        holding.input_inventory, empty.input_inventory,
        "the two states this field exists to separate must not serialise alike",
    );
}

/// The other half of the distinction, and the one that is easy to get wrong
/// by being helpful: a belt has no input inventory, and that must arrive as
/// `None` rather than as an empty list. Collapsing the two would rebuild the
/// same ambiguity one layer up -- "this thing holds nothing" and "this thing
/// cannot hold anything" would read alike again.
#[test]
fn an_entity_with_no_input_inventory_serialises_none_and_not_an_empty_list() {
    let lua = botbridge_types();

    let belt = entity_through_serde(
        &lua,
        entity_table(&lua, "transport-belt", "transport-belt", false),
    );
    assert_eq!(
        belt.input_inventory, None,
        "a belt has no input inventory; None means the sender did not say, \
         Some(empty) would claim it has one and it is empty",
    );

    let inserter = entity_through_serde(&lua, inserter_table(&lua));
    assert_eq!(inserter.input_inventory, None);
}

/// The mod must ask `get_inventory` for the index it resolved from
/// `defines.inventory`, not for some other one. A furnace whose stub answers
/// only for `CRAFTER_INPUT` gets its ore; one that answers only for a
/// different index gets nothing -- which is exactly what a wrong define, or
/// the removed `furnace_source`, would produce, and it would be silent.
#[test]
fn the_input_read_uses_the_index_defines_names() {
    let lua = botbridge_types();

    let wrong_index = entity_table(&lua, "stone-furnace", "furnace", true);
    wrong_index
        .set(
            "get_inventory",
            lua.create_function(|lua, index: i64| {
                if index == CRAFTER_INPUT {
                    return Ok(Value::Nil);
                }
                Ok(Value::Table(inventory_holding(lua, "iron-ore", 34)?))
            })
            .expect("function"),
        )
        .expect("set");

    assert_eq!(
        entity_through_serde(&lua, wrong_index).input_inventory,
        None,
        "reading any index but defines.inventory.crafter_input must find \
         nothing, so that a wrong index cannot pass as an empty furnace",
    );
}

/// A lab's input is its science packs, under a different `defines` index. The
/// per-type dispatch is a place a third type can be quietly forgotten, so the
/// second type it already handles is pinned.
#[test]
fn a_lab_sends_the_science_it_is_holding() {
    let lua = botbridge_types();
    let lab = entity_table(&lua, "lab", "lab", true);
    let entity = entity_through_serde(&lua, lab);
    let held = entity
        .input_inventory
        .expect("a lab has an input inventory");
    assert_eq!(held.len(), 1);
    assert_eq!(
        held[0].name, "iron-ore",
        "whatever the stub was told to hold"
    );
}

// --------------------------------------------------------------------------
// Beacon geometry: the numbers a block layout needs before it reserves ground.
// --------------------------------------------------------------------------

/// A `LuaEntityPrototype` for a beacon, shaped the way 2.1.17 really answers.
///
/// The two things this fixture is *about* are both easy to get wrong from
/// memory, so both are written the way `runtime-api.json` describes them
/// rather than the way the older API did:
///
///  - **`get_supply_area_distance()` is a method** and there is no
///    `supply_area_distance` attribute at all, so this table has only the
///    method. A serialiser reading the attribute gets nil, its `pcall`
///    swallows the nothing, and the field goes missing in silence -- which is
///    exactly what happened to `crafting_speed` on 1028 prototypes.
///  - **`profile` is an ARRAY**, one multiplier per beacon count.
fn beacon_prototype(lua: &Lua) -> Table {
    let point = |x: f64, y: f64| {
        let table = lua.create_table().expect("table");
        table.set("x", x).expect("set");
        table.set("y", y).expect("set");
        table
    };
    let collision_box = lua.create_table().expect("table");
    collision_box
        .set("left_top", point(-1.2, -1.2))
        .expect("set");
    collision_box
        .set("right_bottom", point(1.2, 1.2))
        .expect("set");

    let entity = lua.create_table().expect("table");
    entity.set("name", "beacon").expect("set");
    entity.set("type", "beacon").expect("set");
    entity.set("collision_box", collision_box).expect("set");
    entity
        .set(
            "get_supply_area_distance",
            lua.create_function(|_, ()| Ok(1.5)).expect("function"),
        )
        .expect("set");
    entity.set("distribution_effectivity", 1.5).expect("set");
    let profile = lua.create_table().expect("table");
    for (i, multiplier) in [1.0f64, 0.7, 0.55, 0.45].into_iter().enumerate() {
        profile.set(i + 1, multiplier).expect("set");
    }
    entity.set("profile", profile).expect("set");
    entity
}

fn prototype_through_serde(
    lua: &Lua,
    entity: Table,
) -> factorio_bot_core::types::FactorioEntityPrototype {
    let out = call(lua, "serialize_entity_prototype", entity);
    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the serialised prototype converts to json");
    serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"))
}

/// `FactorioEntityPrototype` carried nothing electrical, so a block layout had
/// no way to ask how far a beacon reaches or what it is worth --
/// `crates/planner/src/method/power.rs` writes `pole_supply_half_extent` out
/// as a hand-kept table of vanilla names for the same reason, and says in its
/// own doc that sending this is the follow-up that deletes it. Picking a
/// spacing from memory instead is the hard-coded-rate defect in another hat.
///
/// Goes the whole way into the struct rather than checking the Lua table,
/// because a correctly spelled key can still reach no field: serde drops an
/// unrecognised one in silence, and this file already records two live bugs of
/// exactly that shape (`pickupPosition`, `belt_to_ground_type`).
#[test]
fn a_serialised_beacon_prototype_carries_its_geometry() {
    let lua = botbridge_types();
    let beacon = prototype_through_serde(&lua, beacon_prototype(&lua));

    assert_eq!(
        beacon.supply_area_distance,
        Some(1.5),
        "half the side of the square the beacon reaches -- read through \
         get_supply_area_distance(), which is a method and has no attribute",
    );
    assert_eq!(beacon.distribution_effectivity, Some(1.5));
    assert_eq!(
        beacon.beacon_profile,
        Some(vec![1.0, 0.7, 0.55, 0.45]),
        "the profile is per BEACON COUNT: one number cannot express it, and a \
         caller given only distribution_effectivity would compute a value \
         right for exactly one beacon and silently wrong for the rest",
    );
}

/// The other half of the distinction. A stone furnace has no supply area, no
/// distribution effectivity and no profile, and all three must arrive `None` --
/// not `Some(0.0)`, which would claim a beacon that reaches nowhere, and not
/// an empty list, which would claim a profile with no entries.
#[test]
fn a_prototype_that_is_not_a_beacon_says_nothing_about_beacons() {
    let lua = botbridge_types();
    let point = |x: f64, y: f64| {
        let table = lua.create_table().expect("table");
        table.set("x", x).expect("set");
        table.set("y", y).expect("set");
        table
    };
    let collision_box = lua.create_table().expect("table");
    collision_box
        .set("left_top", point(-0.8, -0.8))
        .expect("set");
    collision_box
        .set("right_bottom", point(0.8, 0.8))
        .expect("set");
    let furnace = lua.create_table().expect("table");
    furnace.set("name", "stone-furnace").expect("set");
    furnace.set("type", "furnace").expect("set");
    furnace.set("collision_box", collision_box).expect("set");

    let prototype = prototype_through_serde(&lua, furnace);
    assert_eq!(prototype.supply_area_distance, None);
    assert_eq!(prototype.distribution_effectivity, None);
    assert_eq!(prototype.beacon_profile, None);
}

/// **The trap this fixture exists to hold shut.** A serialiser that reads
/// `entity.supply_area_distance` -- the pre-2.0 attribute, which 2.1.17 does
/// not have -- finds nothing on a real beacon and reports `None`, and the
/// `pcall` around every prototype read means it does so without an error
/// anywhere. So a beacon whose ONLY route to the number is the method must
/// still answer; if this ever fails, the read went back to the attribute.
#[test]
fn the_supply_area_comes_from_the_method_and_not_the_attribute() {
    let lua = botbridge_types();
    let beacon = beacon_prototype(&lua);
    // A beacon exactly as the live API presents it: the attribute is absent
    // and only the method answers.
    assert!(
        matches!(
            beacon
                .get::<Value>("supply_area_distance")
                .expect("attribute"),
            Value::Nil
        ),
        "2.1.17 has no such attribute; a fixture that added one would make \
         the attribute read pass and the live game fail",
    );
    assert_eq!(
        prototype_through_serde(&lua, beacon).supply_area_distance,
        Some(1.5),
    );
}

/// A pole's wire reach crosses the bridge, and it comes from the **method**.
///
/// `crates/planner/src/state.rs` kept this as a hand-typed table of four
/// vanilla names, and it is the one that had **drifted**: `big-electric-pole`
/// read 30.0 against the game's 32, because Factorio 2.0 moved the value and a
/// hand-kept table of game data is only ever read by code that agrees with it.
///
/// **The trap this fixture holds shut is the spelling.** `maximum_wire_distance`
/// is the DATA-stage name — it is what `base/prototypes/entity/entities.lua`
/// writes, and it is what anybody reaching for this would try first — and there
/// is no such attribute on `LuaEntityPrototype` in 2.1.17. So the fixture
/// carries a *decoy* attribute with a different value: a serialiser that read
/// the attribute would pass a test asserting only "some number arrived" and
/// fail here, and on the live game it would raise, be swallowed by the mod's
/// `pcall`, and report `None` for every prototype in silence.
#[test]
fn a_serialised_pole_carries_the_wire_reach_from_the_method() {
    let lua = botbridge_types();
    let pole = machine_prototype(&lua, "big-electric-pole", "electric-pole");
    pole.set(
        "get_max_wire_distance",
        lua.create_function(|_, ()| Ok(32.0)).expect("function"),
    )
    .expect("set");
    // The data-stage spelling, which the live API does not have. Present here
    // with the WRONG number so that reading it cannot look like success.
    pole.set("maximum_wire_distance", 30.0).expect("set");

    let prototype = prototype_through_serde(&lua, pole);
    assert_eq!(
        prototype.maximum_wire_distance,
        Some(32.0),
        "read through get_max_wire_distance(); 30 is the Factorio 1.x value \
         sitting on the data-stage attribute name the live API lacks",
    );
}

/// **A wireless entity's zero is sent, and that is the whole point of sending
/// it.** `get_max_wire_distance()` carries no `subclasses` restriction and
/// answers 0 rather than raising, unlike `get_supply_area_distance()`, so
/// there is nothing for the mod's `pcall` to gate on and a collector that
/// wanted to drop the zeros would have to decide what a pole is in Lua.
///
/// Sending it keeps two different facts apart: `Some(0.0)` is *the game says
/// nothing connects to this*, `None` is *the sender did not say*, which is
/// every world dumped before 2026-09-07. `crates/planner/src/state.rs` falls
/// back to its vanilla table on exactly one of those.
///
/// **A tree, not a furnace, because the zero is rarer than it sounds.**
/// Measured over 1,028 live prototypes, a `stone-furnace` reports **9** — its
/// *circuit* wire distance — and so do chests and assembling machines; the
/// 930 that report 0 are trees, explosions and corpses. An earlier version of
/// this test used a furnace and asserted a zero the live game does not give.
#[test]
fn an_unconnectable_prototype_reports_zero_rather_than_nothing() {
    let lua = botbridge_types();
    let tree = machine_prototype(&lua, "tree-01", "tree");
    tree.set(
        "get_max_wire_distance",
        lua.create_function(|_, ()| Ok(0.0)).expect("function"),
    )
    .expect("set");
    assert_eq!(
        prototype_through_serde(&lua, tree).maximum_wire_distance,
        Some(0.0),
        "the game's own zero, not absence -- a reader must be able to tell \
         'nothing connects to this' from 'nobody asked'",
    );

    // And an entity the sender never asked about stays absent.
    let older = machine_prototype(&lua, "tree-01", "tree");
    assert_eq!(
        prototype_through_serde(&lua, older).maximum_wire_distance,
        None,
    );
}

/// A prototype table shaped like the live API presents a machine: a square
/// collision box, plus whatever `extra` the caller wants set on it.
fn machine_prototype(lua: &Lua, name: &str, entity_type: &str) -> Table {
    let point = |x: f64, y: f64| {
        let table = lua.create_table().expect("table");
        table.set("x", x).expect("set");
        table.set("y", y).expect("set");
        table
    };
    let collision_box = lua.create_table().expect("table");
    collision_box
        .set("left_top", point(-1.2, -1.2))
        .expect("set");
    collision_box
        .set("right_bottom", point(1.2, 1.2))
        .expect("set");
    let entity = lua.create_table().expect("table");
    entity.set("name", name).expect("set");
    entity.set("type", entity_type).expect("set");
    entity.set("collision_box", collision_box).expect("set");
    entity
}

/// The planner's `consumer_kw` and `generation_kw` were 14 hand-typed rows and
/// 2, and nothing electrical had ever crossed this bridge: the only `energy`
/// the mod sent anywhere was a recipe's crafting time. The milestone
/// arithmetic that says a second boiler is needed (24 electric furnaces at
/// 180 kW against a 1.8 MW plant) rested entirely on numbers no code had
/// checked against the game.
///
/// **Both values are joules per tick and are sent unconverted.** 3,000 J/tick
/// is an electric furnace's 180 kW; 15,000 is a steam engine's 900 kW.
/// `FactorioEntityPrototype::energy_usage_kw` does the x60/1000 in one place.
///
/// Goes the whole way into the struct, like the beacon test above, because a
/// correctly spelled key can still reach no field.
#[test]
fn a_serialised_electric_machine_carries_its_draw_and_output() {
    let lua = botbridge_types();

    let furnace = machine_prototype(&lua, "electric-furnace", "furnace");
    // Present, and its contents never read: the mod uses it only as the
    // is-this-electric gate.
    furnace
        .set(
            "electric_energy_source_prototype",
            lua.create_table().expect("table"),
        )
        .expect("set");
    furnace.set("energy_usage", 3000.0).expect("set");
    let prototype = prototype_through_serde(&lua, furnace);
    assert_eq!(
        prototype.electric_energy_usage,
        Some(3000.0),
        "joules per tick, the game's own unit, unconverted",
    );
    assert_eq!(
        prototype.energy_usage_kw(),
        Some(180.0),
        "3000 J/tick x 60 / 1000 is the 180 kW on the tooltip",
    );

    let engine = machine_prototype(&lua, "steam-engine", "generator");
    // A generator consumes steam and PRODUCES electricity, so it has no
    // electric energy source -- and must still report its output.
    engine
        .set(
            "get_max_energy_production",
            lua.create_function(|_, ()| Ok(15000.0)).expect("function"),
        )
        .expect("set");
    let prototype = prototype_through_serde(&lua, engine);
    assert_eq!(prototype.electric_energy_usage, None);
    assert_eq!(
        prototype.max_energy_production_kw(),
        Some(900.0),
        "15000 J/tick is the steam engine's 900 kW",
    );
}

/// **The gate that keeps coal out of the electricity budget.**
///
/// A `stone-furnace`'s `energy_usage` is 90 kW *of coal*. Charged against an
/// electric network it is a number in the wrong units that every test would
/// agree with, which is why `crates/planner/src/state.rs` leaves burner
/// machines out of `consumer_kw` rather than zeroing them. The gate lives in
/// the mod, where the energy source is visible, so the field must be **absent**
/// for a burner even though `energy_usage` reads fine on it.
#[test]
fn a_burner_machine_reports_no_electric_energy_usage() {
    let lua = botbridge_types();
    let furnace = machine_prototype(&lua, "stone-furnace", "furnace");
    // The attribute is there and answers; only the electric energy source is
    // missing. That is exactly a burner as the live API presents it.
    furnace.set("energy_usage", 1500.0).expect("set");

    let prototype = prototype_through_serde(&lua, furnace);
    assert_eq!(
        prototype.electric_energy_usage, None,
        "90 kW of coal is not 90 kW of electricity, and absent is the only \
         honest answer -- Some(0.0) would claim a machine that draws nothing",
    );
    assert_eq!(prototype.energy_usage_kw(), None);
}

/// **`get_max_energy_production` is a METHOD, and `energy_usage` is an
/// ATTRIBUTE.** Opposite shapes, both checked against this install's
/// `runtime-api.json` rather than recalled.
///
/// Reading a method as an attribute raises, the `pcall` swallows it, and the
/// field arrives `None` with nothing saying it should not have -- that is how
/// `crafting_speed` was nil for all 1,028 prototypes of a live game. So a
/// prototype whose ONLY route to the output is the method must still answer;
/// if this fails, the read went to an attribute that does not exist.
#[test]
fn the_max_energy_production_comes_from_the_method_and_not_the_attribute() {
    let lua = botbridge_types();
    let engine = machine_prototype(&lua, "steam-engine", "generator");
    engine
        .set(
            "get_max_energy_production",
            lua.create_function(|_, ()| Ok(15000.0)).expect("function"),
        )
        .expect("set");
    assert_eq!(
        prototype_through_serde(&lua, engine).max_energy_production,
        Some(15000.0),
    );
}

// --------------------------------------------------------------------------
// Transport lines: what is riding on the belt, lane by lane.
// --------------------------------------------------------------------------

fn lanes_of(entity: &FactorioEntity) -> Vec<(String, Vec<(String, u32)>)> {
    entity
        .transport_lines
        .as_ref()
        .expect("a belt-connectable entity reports its lanes")
        .iter()
        .map(|line| {
            (
                line.line.clone(),
                line.contents
                    .iter()
                    .map(|item| (item.name.clone(), item.count))
                    .collect(),
            )
        })
        .collect()
}

/// `LuaTransportLine::get_contents()` had blocked four separate questions
/// here, the fourth a diagnosis: a run mined 46 ore, made 17 plates and
/// stranded 29, and a full belt could be neither ruled in nor out because the
/// belt's contents never left the game.
///
/// **The lanes are the point.** A `transport-belt` has two and which one an
/// item is on decides whether an arm can take it -- an inserter drops on the
/// far lane and a side-load arrives on the near one, and this project has
/// measured a block where getting that backwards put ore and coal on a single
/// lane and produced one plate. A single aggregated number over the whole belt
/// would have been unable to say that.
#[test]
fn a_belts_lanes_are_reported_by_name_with_what_is_on_them() {
    let lua = botbridge_types();
    let belt = entity_through_serde(
        &lua,
        entity_table(&lua, "transport-belt", "transport-belt", false),
    );

    assert_eq!(
        lanes_of(&belt),
        vec![
            ("left_line".to_string(), vec![("iron-ore".to_string(), 4)]),
            ("right_line".to_string(), vec![]),
        ],
        "both lanes, named from defines.transport_line and in index order -- \
         an empty lane is reported as an empty lane, not omitted, or a belt \
         with one loaded lane would read the same as a belt with two",
    );
}

/// The names come from inverting the game's own `defines.transport_line`, not
/// from a list of strings written down beside the code. Index 3 is
/// `left_underground_line` on an underground belt and a different lane on a
/// splitter, so a caller handed a bare number would have to rebuild the
/// mapping from the entity type -- which is inventing it.
#[test]
fn an_underground_belts_extra_lanes_are_named_too() {
    let lua = botbridge_types();
    let underground = entity_through_serde(
        &lua,
        entity_table(&lua, "underground-belt", "underground-belt", false),
    );

    let names: Vec<String> = lanes_of(&underground)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        vec![
            "left_line",
            "right_line",
            "left_underground_line",
            "right_underground_line"
        ],
        "four lanes, each under the game's own name for its index",
    );
}

/// The other half of the distinction, and the reason the mod checks for the
/// method rather than calling it: `get_max_transport_line_index` is declared
/// for `TransportBeltConnectable` only, so reading it off a furnace raises --
/// the same shape as the `crafting_progress` read that once took a live run
/// down from inside a sampler.
///
/// `None` here, never an empty list: "this belt is running empty" and "this is
/// not a belt" are precisely the two answers a belt diagnosis has to separate.
#[test]
fn an_entity_that_is_not_belt_connectable_reports_no_lanes() {
    let lua = botbridge_types();

    let furnace = entity_through_serde(&lua, furnace_holding(&lua, "iron-ore", 34));
    assert_eq!(furnace.transport_lines, None);

    let inserter = entity_through_serde(&lua, inserter_table(&lua));
    assert_eq!(inserter.transport_lines, None);
}

/// A lane whose contents arrive as Lua's empty table must still be a lane.
///
/// `helpers.table_to_json({})` renders an empty Lua table as `{}` and not
/// `[]`, so an empty lane reaches serde as an empty *map* where a sequence is
/// declared. That is a hard error without
/// `deserialize_helpers::vec_or_empty_map`, and it would surface only on a
/// belt that happens to be running empty -- which is most belts, most of the
/// time, and exactly the case somebody is diagnosing.
#[test]
fn an_empty_lane_survives_the_empty_table_json_renders_as_an_object() {
    let entity: FactorioEntity = serde_json::from_str(
        r#"{
          "name": "transport-belt", "entity_type": "transport-belt",
          "position": {"x": 0.5, "y": 0.5},
          "bounding_box": {"left_top": {"x": 0.1, "y": 0.1},
                           "right_bottom": {"x": 0.9, "y": 0.9}},
          "direction": 0,
          "transport_lines": [
            {"line": "left_line", "contents": {}},
            {"line": "right_line",
             "contents": [{"name": "coal", "quality": "normal", "count": 2}]}
          ]
        }"#,
    )
    .expect("an empty lane rendered as {} must still parse");

    let lanes = entity.transport_lines.expect("lanes");
    assert_eq!(lanes[0].line, "left_line");
    assert!(
        lanes[0].contents.is_empty(),
        "an empty object is an empty lane, not a parse failure and not a \
         missing lane",
    );
    assert_eq!(lanes[1].contents.len(), 1);
}

/// A solar panel's nameplate is its **noon** output, so the fields that turn
/// it into an average have to cross this bridge or a solar base is planned
/// dead. Two of the three are on the entity prototype
/// (`solar_panel_performance_at_day` / `_at_night`, the curve's endpoints);
/// the third — the accumulator's buffer — is on a *sub*-prototype nothing here
/// had ever read through.
///
/// Goes the whole way into the struct, for the reason
/// `a_serialised_beacon_prototype_carries_its_geometry` gives: a correctly
/// spelled Lua key that matches no serde field is dropped in silence, and this
/// file records two live bugs of exactly that shape.
#[test]
fn a_serialised_solar_panel_carries_both_ends_of_its_curve() {
    let lua = botbridge_types();
    let panel = machine_prototype(&lua, "solar-panel", "solar-panel");
    panel
        .set(
            "get_max_energy_production",
            lua.create_function(|_, ()| Ok(1000.0)).expect("function"),
        )
        .expect("set");
    panel
        .set("solar_panel_performance_at_day", 1.0)
        .expect("set");
    panel
        .set("solar_panel_performance_at_night", 0.0)
        .expect("set");
    let prototype = prototype_through_serde(&lua, panel);

    assert_eq!(
        prototype.max_energy_production_kw(),
        Some(60.0),
        "1000 J/tick x 60 / 1000 is the 60 kW a vanilla panel makes at noon",
    );
    assert_eq!(prototype.solar_panel_performance_at_day, Some(1.0));
    assert_eq!(
        prototype.solar_panel_performance_at_night,
        Some(0.0),
        "a real zero, and it must survive as Some(0.0): a vanilla panel \
         genuinely makes nothing at midnight, which is not the same fact as a \
         sender that did not say",
    );
}

/// The accumulator's real number is not on `LuaEntityPrototype` at all:
/// `get_max_energy_production()` answers its 300 kW *discharge limit*, and
/// what sizing needs is the 5 MJ it holds, which lives on
/// `LuaElectricEnergySourcePrototype`. Reading through a sub-prototype is new
/// here, so it gets its own test.
#[test]
fn a_serialised_accumulator_carries_its_buffer_through_the_sub_prototype() {
    let lua = botbridge_types();
    let accumulator = machine_prototype(&lua, "accumulator", "accumulator");
    let source = lua.create_table().expect("table");
    source.set("buffer_capacity", 5_000_000.0).expect("set");
    accumulator
        .set("electric_energy_source_prototype", source)
        .expect("set");
    let prototype = prototype_through_serde(&lua, accumulator);

    assert_eq!(
        prototype.electric_buffer_capacity,
        Some(5_000_000.0),
        "joules, unconverted, read off the sub-prototype",
    );
}

/// An entity with no electric energy source has no buffer to report, and the
/// absence must stay absence: a burner machine that read as a zero-joule
/// battery would be indistinguishable from an accumulator somebody drained.
#[test]
fn a_burner_machine_reports_no_electric_buffer() {
    let lua = botbridge_types();
    let furnace = machine_prototype(&lua, "stone-furnace", "furnace");
    let prototype = prototype_through_serde(&lua, furnace);

    assert_eq!(prototype.electric_buffer_capacity, None);
    assert_eq!(
        prototype.solar_panel_performance_at_day, None,
        "the endpoints carry `subclasses: [\"SolarPanel\"]`, so their presence \
         is what says a prototype is a panel",
    );
}

/// The daylight curve is **surface state**, and this is the first record in
/// this project that is about a surface rather than about a prototype, an
/// entity or a force. Every field is an attribute on `LuaSurface` in 2.1.17 —
/// there is no `get_dawn()` — and the whole point of going the whole way into
/// the struct is that a method read as an attribute raises, the mod's `pcall`
/// swallows it, and the field arrives missing with nothing saying it should
/// not have.
#[test]
fn a_serialised_surface_carries_its_daylight_curve() {
    let lua = botbridge_types();
    let daylight = daylight_through_serde(&lua, vanilla_nauvis_surface(&lua));

    assert_eq!(
        daylight.surface,
        Some(factorio_bot_core::types::SurfaceId::nauvis()),
        "a curve is only about the surface it was read from",
    );
    assert_eq!(daylight.ticks_per_day, Some(25_000));
    assert_eq!(daylight.dusk, Some(0.25));
    assert_eq!(daylight.evening, Some(0.45));
    assert_eq!(daylight.morning, Some(0.55));
    assert_eq!(daylight.dawn, Some(0.75));
    assert_eq!(daylight.solar_power_multiplier, Some(1.0));
    assert_eq!(daylight.always_day, Some(false));
    assert_eq!(daylight.freeze_daytime, Some(false));
    assert_eq!(
        daylight.daytime,
        Some(0.0),
        "kept even though the average must not depend on it: a frozen clock \
         makes it the only thing that matters",
    );
}

/// Vanilla Nauvis as `LuaSurface` reports it, for the daylight tests.
fn vanilla_nauvis_surface(lua: &Lua) -> Table {
    let surface = lua.create_table().expect("table");
    surface.set("name", "nauvis").expect("set");
    surface.set("ticks_per_day", 25_000).expect("set");
    surface.set("dusk", 0.25).expect("set");
    surface.set("evening", 0.45).expect("set");
    surface.set("morning", 0.55).expect("set");
    surface.set("dawn", 0.75).expect("set");
    surface.set("daytime", 0.0).expect("set");
    surface.set("solar_power_multiplier", 1.0).expect("set");
    surface.set("always_day", false).expect("set");
    surface.set("freeze_daytime", false).expect("set");
    surface
}

fn daylight_through_serde(lua: &Lua, surface: Table) -> factorio_bot_core::types::SurfaceDaylight {
    let out = call(lua, "serialize_surface_daylight", surface);
    let json: serde_json::Value = lua
        .from_value(Value::Table(out))
        .expect("the serialised daylight converts to json");
    serde_json::from_value(json.clone()).unwrap_or_else(|err| panic!("{err} in {json}"))
}
