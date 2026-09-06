use crate::globals::create_lua_globals;
use crate::globals::goal::create_lua_goal;
use crate::globals::rcon::create_lua_rcon;
use crate::globals::record::create_lua_record;
use crate::globals::world::create_lua_world;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::schemars::generate::SchemaSettings;
use factorio_bot_core::schemars::{JsonSchema, SchemaGenerator};
use factorio_bot_core::serde_json::Value;
use factorio_bot_core::types::{
    FactorioBlueprintInfo, FactorioEntity, FactorioPlayer, FactorioRecipe, InventoryResponse,
    Position, Rect,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

/// The four module tables the documentation is rendered from, paired with the
/// file each one is written to, built exactly as a run builds them.
///
/// Extracted from [`write_lua_docs`] so that [`crate::doc_guard`] holds the
/// `__doc_entry_*` strings to the bindings installed *beside* them on these
/// very tables. Rendering and checking must not each construct their own idea
/// of what a module contains: the whole defect class those guards close is a
/// doc string that says something the binding next to it does not do, and a
/// second construction site is one more place for the two to diverge.
///
/// `globals` is `lua.globals()` itself — `create_lua_globals` installs onto the
/// interpreter's global table rather than returning one — which is why it is
/// returned rather than looked up by the caller.
pub(crate) fn binding_tables(
    lua: &Lua,
    cwd: &std::path::Path,
) -> LuaResult<Vec<(&'static str, LuaTable)>> {
    let world = Arc::new(FactorioSurface::new());
    let rcon = Arc::new(FactorioRcon::new_empty());
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    let planner = Planner::new(world, None);
    let cwd = cwd.to_path_buf();
    let world_table = create_lua_world(
        lua,
        planner.plan_world.clone(),
        cwd.clone(),
        cwd.clone(),
        None,
    )?;
    let goal_table = create_lua_goal(
        lua,
        planner.plan_world.clone(),
        planner.real_world.clone(),
        None,
        vec![],
        planner.server,
    )?;
    let record_table = create_lua_record(
        lua,
        rcon.clone(),
        planner.real_world.clone(),
        cwd.clone(),
        vec![],
    )?;
    let rcon_table = create_lua_rcon(lua, rcon, planner.real_world)?;
    let code_by_path: HashMap<String, String> = HashMap::new();
    let code_by_path: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(code_by_path));
    create_lua_globals(
        lua,
        vec![],
        cwd.clone(),
        cwd,
        stdout,
        stderr,
        code_by_path,
        None,
    )?;
    Ok(vec![
        ("globals.lua", lua.globals()),
        ("world.lua", world_table),
        ("goal.lua", goal_table),
        ("rcon.lua", rcon_table),
        ("record.lua", record_table),
    ])
}

pub fn write_lua_docs(target_path: PathBuf) -> LuaResult<()> {
    let lua = crate::sandbox::new_sandboxed_lua()?;
    // Doc generation never executes a script, so the sandbox root only has to
    // be a real directory; the bindings are introspected, not called.
    let cwd = target_path.parent().unwrap_or(&target_path).to_path_buf();
    let bindings: Vec<(&'static str, String)> = binding_tables(&lua, &cwd)?
        .iter()
        .map(|(file, table)| (*file, render_lua_doc(table)))
        .collect();

    // `types.lua` is rendered from the Rust types, and then held to what the
    // binding documentation just said a script would be handed. Both halves
    // are derived, so the only way they can disagree is a real change, and the
    // build says so rather than publishing a broken link.
    let documented = collect_documented_types();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for (_, body) in &bindings {
        referenced.extend(referenced_type_names(body));
    }
    reconcile_documented_types(&documented.roots, &referenced).map_err(LuaError::runtime)?;

    for (file, body) in &bindings {
        fs::write(target_path.join(file), body).expect("failed to write");
    }
    fs::write(
        target_path.join("types.lua"),
        render_types_doc(&documented.all),
    )
    .expect("failed to write");
    Ok(())
}

/// The types a Lua script is actually handed, as JSON Schemas derived from the
/// Rust structs themselves.
///
/// This is a list of *roots*, not of documented types: `schema_for!` carries
/// every type reachable from a root along in `definitions`, so
/// `FactorioIngredient` and `InventoryItemWithQuality` are described here
/// without being named here, and a new field of a new type joins the docs the
/// moment it compiles -- `JsonSchema` is required transitively, so it cannot
/// be added without one. `Rect` was such a type until
/// `world.find_free_resource_rect` started naming it: a binding that hands one
/// back directly makes it a root, and the reconciliation below then requires
/// it here.
///
/// Nothing in the list is spelled as a string: the name under which each type
/// is documented is `schemars`' own `title`, which is the Rust type name. A
/// renamed struct renames its section with no edit here.
///
/// The list itself is the one hand-written thing left, and it is not trusted:
/// [`reconcile_documented_types`] requires it to match, exactly, the set of
/// `` `types.X` `` references in the four generated binding files -- which are
/// themselves generated from the `__doc_entry_*` strings. Adding a root that
/// no binding hands out fails; documenting a return type without a root fails.
fn documented_type_schemas() -> Vec<Value> {
    vec![
        serialize_schema::<FactorioBlueprintInfo>(),
        serialize_schema::<FactorioEntity>(),
        serialize_schema::<FactorioPlayer>(),
        serialize_schema::<FactorioRecipe>(),
        serialize_schema::<InventoryResponse>(),
        serialize_schema::<Position>(),
        serialize_schema::<Rect>(),
    ]
}

/// One type's schema, described as it is **serialised**.
///
/// The contract is chosen explicitly rather than taken from `schema_for!`,
/// whose default describes deserialisation. The two differ wherever serde is
/// told to convert: `FactorioProduct` is `#[serde(from = "RawFactorioProduct")]`,
/// so its deserialize contract is the mod's wire shape and its serialize
/// contract is its own fields. Lua is *handed* these values, so the serialize
/// contract is the true one, and the deserialize contract would document
/// fields (`shared_probability`, `independent_probability`) that a script can
/// never see.
///
/// schemars 0.8 had no contracts and always described the struct's own
/// fields, which made it accidentally right here. 1.x makes it a choice, and
/// taking the default would have silently rewritten two of these types into
/// shapes no Lua caller receives.
fn serialize_schema<T: JsonSchema>() -> Value {
    let generator = SchemaGenerator::new(SchemaSettings::default().for_serialize());
    let schema = generator.into_root_schema_for::<T>();
    Value::from(schema)
}

/// Everything `types.lua` describes.
struct DocumentedTypes {
    /// The names of the roots -- the types a binding hands back directly, and
    /// so the ones its `@return` is expected to name. Read off `schemars`'
    /// `title`, never written down.
    roots: BTreeSet<String>,
    /// The roots and everything reachable from them, keyed by name. A reader
    /// who is handed a `FactorioRecipe` needs `FactorioIngredient` described
    /// too, even though no `@return` names it.
    all: BTreeMap<String, Value>,
}

/// Walks [`documented_type_schemas`] into the roots and their closure.
fn collect_documented_types() -> DocumentedTypes {
    let mut roots: BTreeSet<String> = BTreeSet::new();
    let mut all: BTreeMap<String, Value> = BTreeMap::new();
    for mut root in documented_type_schemas() {
        // The reachable closure travels with the root under `$defs` (0.8 called
        // it `definitions`); lift it out so every named type is a peer, and drop
        // the key so a root does not carry a copy of its own dependencies.
        if let Some(object) = root.as_object_mut()
            && let Some(Value::Object(defs)) = object.remove("$defs")
        {
            for (name, schema) in defs {
                all.insert(name, schema);
            }
        }
        let name = root
            .get("title")
            .and_then(Value::as_str)
            .expect("schemars titles every root schema with its type name")
            .to_owned();
        roots.insert(name.clone());
        all.insert(name, root);
    }
    DocumentedTypes { roots, all }
}

/// The `` `types.X` `` names a generated binding file points readers at.
///
/// Backticked deliberately: the doc strings talk about `world.player` and
/// `entity_prototypes.clone()` in prose, and a looser scan would read those as
/// type references. Only what a reader sees rendered as a type link counts.
fn referenced_type_names(body: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for candidate in body.split("`types.").skip(1) {
        let Some(name) = candidate.split('`').next() else {
            continue;
        };
        if !name.is_empty()
            && name.starts_with(|c: char| c.is_ascii_uppercase())
            && name.chars().all(|c| c.is_ascii_alphanumeric())
        {
            names.insert(name.to_string());
        }
    }
    names
}

/// The roots `types.lua` describes and the types the binding docs promise must
/// be the same set.
///
/// Roots, not the whole closure: `FactorioIngredient` is described because
/// `FactorioRecipe` contains one, and no `@return` will ever name it. What must
/// match is the set of types a binding hands back *directly*.
///
/// Returns the disagreement as text rather than asserting, so that the build
/// itself fails on it: [`write_lua_docs`] runs from `app/src-tauri/build.rs`,
/// which means a `types.X` reference nobody rendered stops the build instead
/// of shipping a link to a description that is not there.
fn reconcile_documented_types(
    roots: &BTreeSet<String>,
    referenced: &BTreeSet<String>,
) -> Result<(), String> {
    let undescribed: Vec<&String> = referenced.difference(roots).collect();
    let unreferenced: Vec<&String> = roots.difference(referenced).collect();
    if undescribed.is_empty() && unreferenced.is_empty() {
        return Ok(());
    }
    let mut message = String::from("types.lua disagrees with the binding documentation:");
    if !undescribed.is_empty() {
        message += &format!(
            "\n  the binding docs promise {undescribed:?}, which no schema describes -- add \
             `schema_for!(..)` for it to `documented_type_schemas()`"
        );
    }
    if !unreferenced.is_empty() {
        message += &format!(
            "\n  {unreferenced:?} is described but no binding hands it to a script -- drop it \
             from `documented_type_schemas()`, or say so in the `__doc_entry_*` that returns it"
        );
    }
    Err(message)
}

/// Collects a module table's `__doc_entry_*` strings, sorted by key.
///
/// The obvious spelling — `pairs::<String, String>().flatten()` — silently
/// truncated every module, because these tables hold the *bound functions*
/// beside the documentation strings. mlua's `TablePairs::next` opens with
/// `self.key.take()` and restores the cursor only on the success arm
/// (`mlua-0.12.0/src/table.rs:1369-1402`); a conversion error is yielded once
/// and every later call falls into `else { None }`. So the first function
/// value *ended* iteration rather than being skipped, and `.flatten()` turned
/// that into silence — a stopped iterator and an exhausted one look the same.
/// The published docs had been a prefix of the real API for as long as this
/// existed, with a green build the whole time.
///
/// Reading both halves as `LuaValue` is infallible, so nothing can terminate
/// the walk early; a non-string value is simply not documentation.
///
/// Sorting is not cosmetic. Lua 5.4 seeds its string hashes per interpreter
/// state, so `pairs` visits a table in a different order in every `Lua`
/// instance — two generations inside one process disagreed. That made the
/// generated files irreproducible and, worse, made the truncation look like
/// churn rather than a bug.
fn doc_entries(doc_table: &LuaTable) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for pair in doc_table.clone().pairs::<LuaValue, LuaValue>() {
        let Ok((key, value)) = pair else { continue };
        let (Some(key), Some(value)) = (key.as_string(), value.as_string()) else {
            continue;
        };
        let key = key.to_string_lossy();
        if key.starts_with("__doc_entry_") {
            entries.push((key, value.to_string_lossy()));
        }
    }
    entries.sort();
    entries
}

/// Renders one module's documentation. Returns the body rather than writing
/// it, so that the `` `types.X` `` links inside it can be reconciled against
/// what `types.lua` describes before anything reaches disk.
fn render_lua_doc(doc_table: &LuaTable) -> String {
    let mut body = doc_table
        .get::<String>("__doc__header")
        .unwrap_or_default()
        .trim()
        .to_string();
    body += "\n\n";

    for (_, value) in doc_entries(doc_table) {
        body += value.trim();
        body += "\n\n"
    }
    body += doc_table
        .get::<String>("__doc__footer")
        .unwrap_or_default()
        .trim();
    body += "\n";
    body
}

/// Renders `types.lua` from the schemas, in ldoc's shape.
///
/// The file used to be hand-written and tracked, which is why it could say
/// `FactorioEntity.output_inventory` was `{[string]=int,...}` for as long as
/// it did while serde had been emitting a list of `InventoryItemWithQuality`.
/// Everything below is read off the schema, so the only spelling choices left
/// are how a JSON type is written for a Lua reader.
fn render_types_doc(documented: &BTreeMap<String, Value>) -> String {
    let mut body = String::from(
        "--- Types\n\
         --\n\
         -- The shapes the other modules hand back. These are not tables a script can\n\
         -- reference by name -- there is no `types` global -- they are what a value\n\
         -- returned by `world.*` or `rcon.*` looks like once it arrives in Lua.\n\
         --\n\
         -- GENERATED from the Rust structs in `crates/core/src/types.rs` via their\n\
         -- `JsonSchema` derive, which reads the same serde attributes that decide the\n\
         -- wire shape. Do not edit: the build overwrites it, and a type named by a\n\
         -- `@return` that nothing here describes fails that build.\n\
         -- @module types\n",
    );
    for (name, schema) in documented {
        body += "\n";
        body += &format!("--- {name}\n");
        if let Some(description) = schema.get("description").and_then(Value::as_str) {
            body += &comment_lines(description, "-- ");
        }
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            body += &format!("{name} = nil -- {}\n", lua_field(schema).1);
            continue;
        };
        body += &format!("{name} = {{\n");
        for (field, field_schema) in properties {
            if let Some(description) = field_schema.get("description").and_then(Value::as_str) {
                body += &comment_lines(description, "    -- ");
            }
            let (placeholder, note) = lua_field(field_schema);
            body += &format!("    {field} = {placeholder}, -- {note}\n");
        }
        body += "}\n";
    }
    body
}

/// A Rust doc comment, re-wrapped as Lua line comments at the given prefix.
fn comment_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{prefix}{}\n", line.trim_end()))
        .collect()
}

/// How one field's schema reads to a Lua author: the placeholder to show it
/// with, and the note describing what it holds.
fn lua_field(schema: &Value) -> (&'static str, String) {
    let Some(object) = schema.as_object() else {
        return ("nil", "any".to_string());
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        return ("nil", format!("`{name}`"));
    }
    // `Option<T>` over a named type is `anyOf: [T, null]`, not a nullable
    // `type`; unwrap to the one real variant and mark it optional.
    if let Some(variants) = ["anyOf", "oneOf", "allOf"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_array))
    {
        let real: Vec<&Value> = variants.iter().filter(|v| !is_null_schema(v)).collect();
        let optional = real.len() < variants.len();
        if let [only] = real[..] {
            let (placeholder, note) = lua_field(only);
            return if optional {
                ("nil", format!("{note}, or nil"))
            } else {
                (placeholder, note)
            };
        }
    }
    let types: Vec<&str> = match object.get("type") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => return ("nil", "any".to_string()),
    };
    let optional = types.contains(&"null");
    let Some(primary) = types.iter().find(|t| **t != "null") else {
        return ("nil", "nil".to_string());
    };
    let (placeholder, note) = match *primary {
        "string" => ("''", "string".to_string()),
        "integer" | "number" => ("0", "number".to_string()),
        "boolean" => ("false", "boolean".to_string()),
        "array" => {
            let item = object
                .get("items")
                .map(|items| lua_field(items).1)
                .unwrap_or_else(|| "any".to_string());
            ("nil", format!("{{{item}}}"))
        }
        "object" => {
            let value = object
                .get("additionalProperties")
                .map(|value| lua_field(value).1)
                .unwrap_or_else(|| "any".to_string());
            ("nil", format!("{{[string]={value},...}}"))
        }
        _ => ("nil", "any".to_string()),
    };
    if optional {
        ("nil", format!("{note}, or nil"))
    } else {
        (placeholder, note)
    }
}

/// `Option<T>` renders its absent arm as a schema whose only type is `null`.
fn is_null_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every binding that carries a `__doc_entry_*` string, spelled the way it
    /// appears in the rendered file, **in the order the generator must emit
    /// them** — sorted by `__doc_entry_*` key, which is why `Direction` leads
    /// `all_bots` (ASCII `D` < `a`) and `direction_opposite` leads
    /// `directions_all` (`_` < `s`).
    ///
    /// This list is deliberately exhaustive rather than a spot check. The
    /// generator's failure mode was *truncation* — it emitted a prefix of each
    /// module and stopped — so a test that asserted one entry per file would
    /// have passed against the broken generator whenever that entry happened
    /// to land first. Requiring all of them means a truncated file always
    /// names something it is missing.
    const EXPECTED: &[(&str, &[&str])] = &[
        (
            "globals.lua",
            &[
                "globals.Direction = {",
                "globals.all_bots",
                "function globals.direction_clockwise(",
                "function globals.direction_opposite(",
                "function globals.directions_all(",
                "function globals.directions_compass(",
                "function globals.directions_orthogonal(",
                "function globals.file_read(",
                "function globals.file_write(",
                "function globals.include(",
                "function globals.print(",
                "function globals.print_err(",
                "function globals.print_warn(",
            ],
        ),
        (
            "world.lua",
            &[
                "function world.draw(",
                "function world.find_entities_in_radius(",
                "function world.find_free_resource_rect(",
                "function world.inventory(",
                "function world.parse_blueprint(",
                "function world.player(",
                "function world.recipe(",
            ],
        ),
        (
            "rcon.lua",
            &[
                "function rcon.add_research(",
                "function rcon.cheat_all_technologies(",
                "function rcon.cheat_blueprint(",
                "function rcon.cheat_item(",
                "function rcon.cheat_technology(",
                "function rcon.craft(",
                "function rcon.find_entities_in_radius(",
                "function rcon.game_tick(",
                "function rcon.insert_to_inventory(",
                "function rcon.inventory_contents_at(",
                "function rcon.mine(",
                "function rcon.move(",
                "function rcon.place_blueprint(",
                "function rcon.place_entity(",
                "function rcon.print(",
                "function rcon.remove_from_inventory(",
                "function rcon.revive_ghost(",
            ],
        ),
        (
            "goal.lua",
            &[
                "function goal.all(",
                "function goal.have(",
                "function goal.plan(",
                "function goal.producing(",
                "function goal.researched(",
                "function goal.run(",
                "function goal.start(",
            ],
        ),
    ];

    /// The functions `create_lua_goal` really installs on the `goal` table.
    ///
    /// Read off the live table rather than listed here, which is the whole
    /// point of [`the_goal_doc_entries_are_exactly_the_goal_surface`]: a list
    /// compared against a list is a mirror in both directions, and a mirror
    /// cannot notice a binding nobody documented.
    ///
    /// Reading both halves of each pair as `LuaValue` for the same reason
    /// [`doc_entries`] does: the table holds bound functions beside
    /// documentation strings, and a typed `pairs` stops the walk at the first
    /// value that will not convert.
    fn goal_table_functions() -> std::collections::BTreeSet<String> {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandboxed lua");
        let planner = Planner::new(Arc::new(FactorioSurface::new()), None);
        let goal_table = create_lua_goal(
            &lua,
            planner.plan_world.clone(),
            planner.real_world.clone(),
            None,
            vec![],
            planner.server,
        )
        .expect("goal table");
        let mut names = std::collections::BTreeSet::new();
        for pair in goal_table.pairs::<LuaValue, LuaValue>() {
            let Ok((key, value)) = pair else { continue };
            let (Some(key), LuaValue::Function(_)) = (key.as_string(), &value) else {
                continue;
            };
            names.insert(key.to_string_lossy());
        }
        names
    }

    /// `goal.lua`, in **both** directions, against the **real** goal table.
    ///
    /// [`every_documented_binding_reaches_the_generated_file`] asserts only
    /// that each listed entry is present, so it catches a function that is
    /// *removed* and stays silent on one that is *added* — the list is then a
    /// mirror of the code rather than a check on it, and it stays green when
    /// a seventh function appears.
    ///
    /// The first fix for that compared the emitted set against a *hardcoded*
    /// set, which is still a mirror: a binding added to the goal table with
    /// no `__doc_entry_*` beside it changes neither side. So the expected set
    /// is taken from [`goal_table_functions`] — the functions
    /// `create_lua_goal` actually installs. An undocumented binding and an
    /// unimplemented doc entry now fail from opposite directions.
    ///
    /// Only `goal.lua`: it is the file this change churns, and the other
    /// three carry longer lists that nothing here is renaming.
    #[test]
    fn the_goal_doc_entries_are_exactly_the_goal_surface() {
        use std::collections::BTreeSet;

        let (_dir, target) = generate();
        let body = fs::read_to_string(target.join("goal.lua")).expect("goal.lua");
        let emitted: BTreeSet<String> = body
            .lines()
            .filter_map(|line| line.strip_prefix("function goal."))
            .filter_map(|rest| rest.split('(').next())
            .map(str::to_string)
            .collect();
        let installed = goal_table_functions();
        assert!(
            !installed.is_empty(),
            "the goal table installed no functions at all; this test would then \
             assert nothing"
        );
        assert_eq!(
            emitted, installed,
            "goal.lua's `__doc_entry_*` set and the functions `create_lua_goal` \
             installs must be the same set: an entry missing from the left is an \
             undocumented binding, one missing from the right is documentation \
             for a function that does not exist"
        );
    }

    /// The type names the *binding* documentation actually points a reader at,
    /// read out of the generated files rather than listed here.
    ///
    /// This is the whole reason `types.lua` can no longer drift quietly. The
    /// four binding files are generated from the `__doc_entry_*` strings, so
    /// the set of `` `types.X` `` references in them is a function of the Rust
    /// source; comparing it against the schemas [`documented_type_schemas`]
    /// actually renders gives a check with a hand-written expectation on
    /// neither side.
    fn referenced_in_generated_docs(target: &std::path::Path) -> BTreeSet<String> {
        let bodies: Vec<String> = ["globals.lua", "world.lua", "rcon.lua", "goal.lua"]
            .iter()
            .map(|file| fs::read_to_string(target.join(file)).expect("generated file"))
            .collect();
        let mut names = BTreeSet::new();
        for body in &bodies {
            names.extend(referenced_type_names(body));
        }
        names
    }

    /// The headings `types.lua` actually emits.
    fn documented_in_types_lua(target: &std::path::Path) -> BTreeSet<String> {
        let body = fs::read_to_string(target.join("types.lua")).expect("types.lua");
        body.lines()
            .filter_map(|line| line.strip_prefix("--- "))
            .map(str::to_string)
            .collect()
    }

    /// Every type a binding's documentation names must be described.
    ///
    /// `types.lua` used to be hand-written and tracked, and nothing read it, so
    /// it disagreed with `crates/core/src/types.rs` in ways a green build never
    /// mentioned: it said `FactorioEntity.output_inventory` was
    /// `{[string]=int,...}` when serde emits a list of
    /// `InventoryItemWithQuality`, and it described `EntityPlacement`,
    /// `InventoryLocation` and `PositionRadius`, none of which derive
    /// `Serialize` and none of which can therefore ever reach a script.
    #[test]
    fn types_lua_documents_every_type_the_binding_docs_reference() {
        let (_dir, target) = generate();
        let referenced = referenced_in_generated_docs(&target);
        assert!(
            !referenced.is_empty(),
            "no binding documentation names a `types.X` at all; this test would \
             then assert nothing"
        );
        let documented = documented_in_types_lua(&target);
        let missing: Vec<&String> = referenced.difference(&documented).collect();
        assert!(
            missing.is_empty(),
            "types.lua does not describe {missing:?}, which the binding \
             documentation tells readers they will be handed"
        );
    }

    /// `types.lua` lists exactly the fields serde emits, for every type.
    ///
    /// Not tautological, given this file's history: the failure that shipped
    /// here was a *renderer* that stopped early and emitted a prefix, with the
    /// right input all along. Reading the rendered text back and comparing it
    /// against the schema it was rendered from is what contradicts that.
    #[test]
    fn types_lua_lists_every_field_the_rust_schema_has() {
        let (_dir, target) = generate();
        let body = fs::read_to_string(target.join("types.lua")).expect("types.lua");
        let mut checked = 0usize;
        for (name, schema) in collect_documented_types().all {
            let expected: Vec<String> = match schema.get("properties").and_then(Value::as_object) {
                Some(properties) => properties.keys().cloned().collect(),
                None => continue,
            };
            if expected.is_empty() {
                continue;
            }
            let start = body
                .find(&format!("\n{name} = {{\n"))
                .unwrap_or_else(|| panic!("types.lua has no `{name} = {{` block"));
            let rest = &body[start + 1..];
            let end = rest
                .find("\n}")
                .unwrap_or_else(|| panic!("{name} block never closes"));
            let block = &rest[..end];
            let emitted: Vec<String> = block
                .lines()
                .skip(1)
                .filter_map(|line| line.trim().split(" =").next())
                .filter(|field| !field.is_empty() && !field.starts_with("--"))
                .map(str::to_string)
                .collect();
            assert_eq!(
                emitted, expected,
                "types.lua's `{name}` block and the fields serde emits for it must \
                 be the same list, in the same order"
            );
            checked += 1;
        }
        assert!(
            checked >= 5,
            "only {checked} types carried fields to compare; the schema walk found \
             nothing to check and this test would assert nothing"
        );
    }

    /// The reconciliation fires in **both** directions, shown on inputs this
    /// test controls rather than on the live sets.
    ///
    /// A type the binding docs promise but nothing renders is a reader sent to
    /// a description that is not there. A type rendered that no binding hands
    /// out is documentation of something a script cannot obtain -- which is
    /// exactly the state the hand-written file was in.
    #[test]
    fn reconcile_rejects_a_gap_in_either_direction() {
        let both: BTreeSet<String> = ["Position".to_string(), "Rect".to_string()]
            .into_iter()
            .collect();
        assert!(
            reconcile_documented_types(&both, &both).is_ok(),
            "identical sets must reconcile"
        );

        let only_referenced: BTreeSet<String> = ["Position".to_string()].into_iter().collect();
        let err = reconcile_documented_types(&both, &only_referenced)
            .expect_err("a rendered type nothing references must be rejected");
        assert!(
            err.contains("Rect"),
            "the error must name the unreferenced type, said: {err}"
        );

        let err = reconcile_documented_types(&only_referenced, &both)
            .expect_err("a referenced type nothing renders must be rejected");
        assert!(
            err.contains("Rect"),
            "the error must name the undescribed type, said: {err}"
        );
    }

    fn generate() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("lua");
        fs::create_dir_all(&target).expect("create target");
        write_lua_docs(target.clone()).expect("write_lua_docs");
        (dir, target)
    }

    /// The generator walks tables that hold the bound *functions* beside the
    /// documentation strings. Nothing in the build reads these files, so a
    /// generator that drops entries produces a green build and empty docs;
    /// this is the only thing that contradicts it.
    #[test]
    fn every_documented_binding_reaches_the_generated_file() {
        let (_dir, target) = generate();
        let mut missing: Vec<String> = Vec::new();
        for (file, entries) in EXPECTED {
            let path = target.join(file);
            let body =
                fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            for entry in *entries {
                if !body.contains(entry) {
                    missing.push(format!("{file} is missing `{entry}`"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "{} documentation entries never reached the generated files:\n{}",
            missing.len(),
            missing.join("\n")
        );
    }

    /// The generated files must be a function of the source, not of Lua's
    /// hash seed — so the entries appear in sorted key order, always.
    ///
    /// This asserts the ordering *directly*, by position in the file, because
    /// the obvious test for it cannot be trusted. See
    /// [`generated_files_are_ordered_deterministically`] for what went wrong
    /// with the obvious one; the short version is that comparing two
    /// generations only discriminates when the two interpreters happen to draw
    /// different seeds, which is a property of the allocator and the clock
    /// rather than of anything the test does. Reading positions out of one
    /// file draws no entropy at all: if `entries.sort()` goes, the emitted
    /// order becomes Lua's hash order, and hash order matching sorted order
    /// for twelve keys is 1 in 12! — repeated independently across four files.
    #[test]
    fn entries_are_emitted_in_sorted_key_order() {
        let (_dir, target) = generate();
        for (file, entries) in EXPECTED {
            let body = fs::read_to_string(target.join(file)).expect("read generated file");
            let mut previous: Option<(&str, usize)> = None;
            for entry in *entries {
                let at = body
                    .find(entry)
                    .unwrap_or_else(|| panic!("{file} is missing `{entry}`"));
                if let Some((earlier, earlier_at)) = previous {
                    assert!(
                        earlier_at < at,
                        "{file} emits `{entry}` before `{earlier}`; entries must be \
                         sorted by their `__doc_entry_*` key, and are not when the \
                         generator leaks Lua's per-state hash order into the output"
                    );
                }
                previous = Some((entry, at));
            }
        }
    }

    /// Two generations in one process agree.
    ///
    /// **This test guards weakly and cannot be made to guard strongly.** It is
    /// kept because it states the user-visible property — build twice, get the
    /// same file — and removed reliance on it is exactly what
    /// [`entries_are_emitted_in_sorted_key_order`] is for.
    ///
    /// The trap, which review caught: it only discriminates when the two
    /// interpreters draw *different* seeds. PUC Lua's `luai_makeseed` mixes the
    /// new `lua_State`'s heap address, a stack address and `time(NULL)`. Two
    /// back-to-back generations can agree on all three — a quiet allocator
    /// hands the second state the block the first just freed, and no clock
    /// second is crossed — and then the test passes with the sort deleted. On
    /// the reviewer's machine it did so 30 times out of 30 in isolation while
    /// failing 15 out of 15 under the full crate suite, where neighbouring
    /// tests perturb the allocator; on mine it failed 30 out of 30 in
    /// isolation. Same test, same mutation, opposite verdicts: its
    /// discriminating power was coming from the environment, not from itself.
    ///
    /// Holding the intervening states *alive* is what this can do about it:
    /// the first generation's freed block is occupied when the second
    /// generation allocates, so the addresses cannot coincide. That is a
    /// guarantee about the allocator, not about the clock, which is why it is
    /// the backup assertion and not the primary one.
    #[test]
    fn generated_files_are_ordered_deterministically() {
        let (_a, first) = generate();
        // Kept alive across the second `generate()` on purpose: dropping these
        // would hand the block straight back and restore the coincidence.
        let _occupy_the_freed_states: Vec<Lua> = (0..8)
            .map(|_| crate::sandbox::new_sandboxed_lua().expect("sandboxed lua"))
            .collect();
        let (_b, second) = generate();
        for (file, _) in EXPECTED {
            assert_eq!(
                fs::read_to_string(first.join(file)).expect("first"),
                fs::read_to_string(second.join(file)).expect("second"),
                "{file} differs between two generations"
            );
        }
    }

    /// The sandbox notes that Task 1 bounded are the part of these strings a
    /// reader has to see; assert they are actually in the shipped text rather
    /// than only in the source.
    #[test]
    fn sandbox_notes_are_documented() {
        let (_dir, target) = generate();
        let globals = fs::read_to_string(target.join("globals.lua")).expect("globals.lua");
        let world = fs::read_to_string(target.join("world.lua")).expect("world.lua");
        // Backticked, because the bare names are substrings of ordinary words
        // in this file — `direction` contains "io", `position` contains "os" —
        // and an assertion that passes on `direction` is worse than no
        // assertion, since it reads as coverage.
        for missing in [
            "`io`",
            "`os`",
            "`package`",
            "`require`",
            "`dofile`",
            "`loadfile`",
        ] {
            assert!(
                globals.contains(missing),
                "globals.lua does not mention that {missing} is unavailable"
            );
        }
        for body in [&globals, &world] {
            assert!(
                body.contains("scripts directory"),
                "a bounded binding does not say paths are relative to the scripts directory"
            );
        }
    }
}
