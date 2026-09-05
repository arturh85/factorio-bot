//! The BotBridge mod is Lua that only ever runs inside Factorio, so nothing in
//! the Rust suite exercised it. That is how `serialize_product` kept asking the
//! game for a `probability` field that Factorio 2.1 does not have, and
//! `serialize_recipe` kept asking for `category`, which 2.1 replaced with
//! `categories`.
//!
//! These tests load the real `mods/BotBridge/types.lua` into a Lua 5.4 state
//! and feed it the table shapes `workspace/factorio-api-docs/runtime-api.json`
//! describes for 2.1.

use factorio_bot_core::types::FactorioRecipe;
use mlua::{Function, Lua, LuaSerdeExt, Table, Value};

/// Loads the mod's serialisers. The path is the same live reference a debug
/// build uses for `workspace/mods`.
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
