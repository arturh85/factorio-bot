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
