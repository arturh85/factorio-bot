//! Holds each `__doc_entry_*` block to the binding installed beside it.
//!
//! [`crate::lua_docs::write_lua_docs`] renders `docs/lua/src/*.lua` *from* these
//! strings, so generation guarantees that the published file matches the
//! strings and says exactly nothing about whether the strings match the
//! functions. An audit of all 42 blocks found 12 wrong — a `@return` promising
//! `{types.FactorioEntity}` on a binding that answers a number, `@param` lists
//! copy-pasted from a different function, a summary describing a different
//! function entirely — none of it drift, all of it wrong from the first commit,
//! and green the whole time.
//!
//! Three of the four fields are checkable with a hand-written list on neither
//! side, and this module checks them. The fourth is not; see
//! [`the_return_field_is_not_guarded_here`] for what was rejected and why.
//!
//! Everything below reads one of two things: the live module tables, built by
//! [`crate::lua_docs::binding_tables`] exactly as a run builds them, or the
//! crate's own `src/globals` tree, walked at test time rather than named in an
//! `include_str!` list — so a new binding file is picked up without an edit
//! here.

use crate::lua_docs::binding_tables;
use factorio_bot_core::mlua::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// The live modules
// ---------------------------------------------------------------------------

/// One documented module: what the generator will write, and what a script
/// will actually be handed.
struct Module {
    /// The file the generator writes this module to, e.g. `world.lua`.
    file: &'static str,
    /// The name the header's `-- @module X` line declares, and therefore the
    /// namespace every `function X.name(..)` line in it must use.
    name: String,
    /// The `__doc_entry_*` blocks, keyed by the name after the prefix.
    entries: BTreeMap<String, String>,
    /// The keys the module installs a Lua *function* at.
    functions: BTreeSet<String>,
    /// The keys it installs anything else at (`globals.Direction` is a table,
    /// `globals.all_bots` a list), minus the documentation strings themselves.
    values: BTreeSet<String>,
    /// The subset of the two above that this crate is answerable for.
    ///
    /// The same as `functions ∪ values` for `world`, `goal` and `rcon`, whose
    /// tables are ours entirely. For `globals` — which *is* the interpreter's
    /// global table — it is that set minus the keys a bare sandbox already has,
    /// so `pairs` and `tostring` are not reported as undocumented bindings.
    ///
    /// Only the "installed but undocumented" direction uses it. The other
    /// direction must not: `globals.print` deliberately shadows the sandbox's
    /// own `print`, so subtracting the builtins there would make a documented,
    /// installed binding look absent — which is what it did, the first time
    /// this test ran.
    own: BTreeSet<String>,
}

/// The four modules, built the way the generator builds them.
///
/// The bare sandbox built alongside is what makes `own` derived rather than a
/// list of Lua builtins somebody would have to maintain.
fn modules() -> Vec<Module> {
    let dir = tempfile::tempdir().expect("tempdir");
    let bare = crate::sandbox::new_sandboxed_lua().expect("bare sandbox");
    let builtin: BTreeSet<String> = table_keys(&bare.globals())
        .into_iter()
        .map(|(key, _)| key)
        .collect();

    let lua = crate::sandbox::new_sandboxed_lua().expect("sandboxed lua");
    let tables = binding_tables(&lua, dir.path()).expect("binding tables");
    tables
        .into_iter()
        .map(|(file, table)| {
            let header: String = table.get("__doc__header").unwrap_or_default();
            let name = module_name(&header).unwrap_or_else(|| {
                panic!("{file}'s `__doc__header` carries no `-- @module <name>` line")
            });
            let mut entries = BTreeMap::new();
            let mut functions = BTreeSet::new();
            let mut values = BTreeSet::new();
            let mut own = BTreeSet::new();
            for (key, value) in table_keys(&table) {
                if let Some(entry) = key.strip_prefix("__doc_entry_") {
                    if let LuaValue::String(text) = &value {
                        entries.insert(entry.to_string(), text.to_string_lossy());
                    }
                    continue;
                }
                if key.starts_with("__doc__") {
                    continue;
                }
                if file != "globals.lua" || !builtin.contains(&key) {
                    own.insert(key.clone());
                }
                match value {
                    LuaValue::Function(_) => {
                        functions.insert(key);
                    }
                    _ => {
                        values.insert(key);
                    }
                }
            }
            Module {
                file,
                name,
                entries,
                functions,
                values,
                own,
            }
        })
        .collect()
}

/// A table's keys and values.
///
/// Read as `LuaValue` on both sides for the reason
/// [`crate::lua_docs::doc_entries`] spells out: these tables hold the bound
/// functions beside the documentation strings, and mlua's `TablePairs` ends
/// iteration — rather than skipping — at the first value that will not convert.
fn table_keys(table: &LuaTable) -> Vec<(String, LuaValue)> {
    let mut out = Vec::new();
    for pair in table.clone().pairs::<LuaValue, LuaValue>() {
        let Ok((key, value)) = pair else { continue };
        let Some(key) = key.as_string() else { continue };
        out.push((key.to_string_lossy(), value));
    }
    out
}

/// The namespace a module's header declares.
fn module_name(header: &str) -> Option<String> {
    header.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("-- @module ")?;
        let name = rest.trim();
        (!name.is_empty()).then(|| name.to_string())
    })
}

// ---------------------------------------------------------------------------
// The doc block
// ---------------------------------------------------------------------------

/// What kind of Lua value a parameter holds, at the resolution both sides of a
/// check can actually speak about.
///
/// [`Kind::Any`] is a wildcard on purpose, and it is what keeps the kind check
/// from crying wolf: several bindings deliberately take a `LuaValue` so they
/// can raise their own message instead of mlua's (`goal.have` documents
/// `@string item_name` over a `LuaValue` for exactly that reason), and an
/// ldoc `@param`/`@tparam` says nothing about primitiveness either. A
/// mismatch is only ever reported when both sides commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Str,
    Num,
    Bool,
    Any,
}

impl Kind {
    fn describe(self) -> &'static str {
        match self {
            Kind::Str => "a string",
            Kind::Num => "a number",
            Kind::Bool => "a boolean",
            Kind::Any => "any value",
        }
    }
}

/// One `@string name ...` / `@tparam[opt] table name ...` line.
#[derive(Debug)]
struct DocParam {
    name: String,
    kind: Kind,
    optional: bool,
}

/// The ldoc tags that describe a *parameter*, and the kind each commits to.
///
/// `@tparam` carries an explicit type token ahead of the name, which is why it
/// is flagged rather than merely listed.
fn param_tag(tag: &str) -> Option<(Kind, bool)> {
    Some(match tag {
        "string" => (Kind::Str, false),
        "number" | "int" => (Kind::Num, false),
        "bool" | "boolean" => (Kind::Bool, false),
        "param" | "func" => (Kind::Any, false),
        "tparam" => (Kind::Any, true),
        _ => return None,
    })
}

/// The parameters a doc block documents, in the order it documents them.
///
/// Only lines that *open* with a tag count. A tag's text wraps onto
/// continuation lines (`--   defaults to every bot in this run`), and those
/// carry no `@`, so they are skipped without having to know which tag they
/// belong to.
fn doc_params(block: &str) -> Vec<DocParam> {
    let mut out = Vec::new();
    for line in block.lines() {
        let Some(rest) = line.trim_start().strip_prefix("-- @") else {
            continue;
        };
        let tag = leading_ident(rest);
        let Some((kind, typed)) = param_tag(tag) else {
            continue;
        };
        let rest = &rest[tag.len()..];
        // `@string[opt]` / `@number[opt=1]` — ldoc's optional marker.
        let (optional, rest) = match rest.strip_prefix('[') {
            Some(bracketed) => match bracketed.find(']') {
                Some(close) => (
                    bracketed[..close].starts_with("opt"),
                    &bracketed[close + 1..],
                ),
                None => (false, rest),
            },
            None => (false, rest),
        };
        let mut tokens = rest.split_whitespace();
        if typed {
            tokens.next();
        }
        let Some(name) = tokens.next() else { continue };
        out.push(DocParam {
            name: name.to_string(),
            kind,
            optional,
        });
    }
    out
}

/// The `function ns.name(a, b)` line a doc block declares: the namespace, the
/// function name, and its parameters.
///
/// `["..."]` is a genuine Lua variadic, not a placeholder — `globals.print`
/// really does take one — and is reported as such rather than as one parameter
/// named `...`.
fn declared_signature(block: &str) -> Option<(String, String, Vec<String>)> {
    let line = block
        .lines()
        .find(|line| line.starts_with("function ") && line.trim_end().ends_with(')'))?;
    let rest = line.strip_prefix("function ")?;
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    let (path, args) = (&rest[..open], &rest[open + 1..close]);
    let (namespace, name) = path.split_once('.')?;
    let params = args
        .split(',')
        .map(str::trim)
        .filter(|arg| !arg.is_empty())
        .map(str::to_string)
        .collect();
    Some((namespace.to_string(), name.to_string(), params))
}

// ---------------------------------------------------------------------------
// The Rust closure
// ---------------------------------------------------------------------------

/// The parameter list of the `lua.create_function` closure a binding is
/// installed with.
#[derive(Debug)]
struct RustParams {
    params: Vec<(Kind, bool)>,
    variadic: bool,
}

/// Every `.rs` file under `src/globals`, paired with the Lua module it
/// installs into.
///
/// Walked rather than `include_str!`-listed so that a new binding file joins
/// the check by existing. The module is read off the path: a file directly in
/// `globals/` installs the module named after it (`world.rs` -> `world`), and
/// anything in a subdirectory installs that subdirectory's module
/// (`goal/plan.rs`, `goal/run.rs` and `goal/value.rs` all -> `goal`), which is
/// how the `goal.*` bindings are found at all — they are registered by helper
/// installers in three files rather than beside their doc strings in
/// `goal/mod.rs`.
fn globals_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/globals");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs_files(&root, &mut files);
    files.sort();
    files
        .iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).ok()?;
            let module = match relative.parent().and_then(Path::file_name) {
                Some(dir) => dir.to_string_lossy().into_owned(),
                None => {
                    let stem = relative.file_stem()?.to_string_lossy().into_owned();
                    // `globals/mod.rs` holds shared helpers, not a module.
                    if stem == "mod" {
                        return None;
                    }
                    stem
                }
            };
            let source = std::fs::read_to_string(path).ok()?;
            Some((module, source))
        })
        .collect()
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The source up to its own `#[cfg(test)]`.
///
/// The scan below looks for markers this file's own tests also write down, so
/// a parser that read test code would invent bindings out of fixtures. (The
/// literal cannot match itself: in this file's bytes it is a backslash and an
/// `n`, not a newline.)
fn production_half(source: &str) -> &str {
    source.split("\n#[cfg(test)]").next().unwrap_or(source)
}

fn leading_ident(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

/// Every `<table>.set("name", lua.create_function(move |ctx, <params>| ..))` in
/// one source file, as the raw `<params>` text keyed by name.
///
/// A `Vec` per name rather than one entry: two registrations under one name in
/// one module would make the check ambiguous, and silently taking the first is
/// how a guard starts lying. The caller reports the ambiguity instead.
fn closure_params_by_name(source: &str) -> BTreeMap<String, Vec<String>> {
    let src = production_half(source);
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (idx, _) in src.match_indices(".set(") {
        let after = src[idx + ".set(".len()..].trim_start();
        let Some(rest) = after.strip_prefix('"') else {
            continue;
        };
        let key = leading_ident(rest);
        if key.is_empty() {
            continue;
        }
        let Some(rest) = rest[key.len()..].strip_prefix('"') else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix(',') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest
            .strip_prefix("lua.create_async_function(")
            .or_else(|| rest.strip_prefix("lua.create_function("))
        else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix("move")
            .map(str::trim_start)
            .unwrap_or(rest);
        let Some(rest) = rest.strip_prefix('|') else {
            continue;
        };
        // The first closure parameter is mlua's `&Lua`, spelled `lua`, `_lua`
        // or `_`; the documented parameters are what follows it.
        let rest = rest.trim_start();
        let context = leading_ident(rest);
        let Some(rest) = rest[context.len()..].trim_start().strip_prefix(',') else {
            continue;
        };
        out.entry(key.to_string())
            .or_default()
            .push(until_closing_bar(rest).trim().to_string());
    }
    out
}

/// The closure's parameter text, up to the `|` that closes its parameter list.
///
/// Depth-tracked over `(<[`, so `Option<String>` and `Vec<PlayerId>` do not end
/// it early. Nothing in a parameter position is a string literal, which is what
/// lets a delimiter counter be used here at all — the sibling guard in
/// `globals/rcon.rs` cannot count braces for exactly that reason.
fn until_closing_bar(text: &str) -> &str {
    let mut depth = 0i32;
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            '|' if depth == 0 => return &text[..idx],
            _ => {}
        }
    }
    text
}

/// Splits a comma-separated list at depth zero.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(text[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    out.push(text[start..].trim());
    out.retain(|part| !part.is_empty());
    out
}

/// A Rust parameter type, as the kind a Lua caller must pass and whether it may
/// be omitted.
///
/// The number list is the set of Rust scalars a `create_function` closure can
/// actually name here; a type it does not know is [`Kind::Any`], which is
/// permissive and never reports. That is the right default: a guard that
/// guesses is a guard that gets worked around.
fn kind_of(rust_type: &str) -> (Kind, bool, bool) {
    let mut ty = rust_type.trim();
    let mut optional = false;
    let mut variadic = false;
    if let Some(inner) = ty.strip_prefix("Option<").and_then(|t| t.strip_suffix('>')) {
        optional = true;
        ty = inner.trim();
    }
    if let Some(inner) = ty
        .strip_prefix("LuaVariadic<")
        .and_then(|t| t.strip_suffix('>'))
    {
        variadic = true;
        ty = inner.trim();
    }
    let kind = match ty {
        "String" | "&str" => Kind::Str,
        "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" | "f32"
        | "f64" | "PlayerId" | "BotId" => Kind::Num,
        "bool" => Kind::Bool,
        _ => Kind::Any,
    };
    (kind, optional, variadic)
}

/// Reads `(a, b): (String, u32)`, `name: String`, `(): ()` or `()` into the
/// parameters a Lua caller passes.
///
/// The *type* side is what is read; the Rust binding names are deliberately
/// ignored, because they legitimately differ from the documented ones
/// (`goal.plan(goal, opts)` is installed as `|_lua, (g, opts)|`, `goal` being a
/// module name in that file).
fn rust_params(text: &str) -> Option<RustParams> {
    let text = text.trim();
    if text == "()" {
        return Some(RustParams {
            params: Vec::new(),
            variadic: false,
        });
    }
    let mut depth = 0i32;
    let colon = text.char_indices().find_map(|(idx, ch)| {
        match ch {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ':' if depth == 0 => return Some(idx),
            _ => {}
        }
        None
    })?;
    let types = text[colon + 1..].trim();
    let listed = match types.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(inner) => split_top_level(inner),
        None => vec![types],
    };
    let mut params = Vec::new();
    let mut variadic = false;
    for one in listed {
        let (kind, optional, is_variadic) = kind_of(one);
        variadic |= is_variadic;
        params.push((kind, optional));
    }
    Some(RustParams { params, variadic })
}

// ---------------------------------------------------------------------------
// The guards
// ---------------------------------------------------------------------------

/// Every `@param`-family tag must name a parameter the block's own
/// `function ns.name(a, b)` line declares, in that order.
///
/// Self-consistency: both halves are inside one doc string, so nothing outside
/// it has to be parsed and nothing outside it can make this fire spuriously.
/// It is the cheapest of the four candidate checks and the one that catches the
/// audit's `@param` findings — `world.parse_blueprint` documented
/// `@string blueprint` and `@string label` under a `function
/// world.parse_blueprint(...)` line, and `rcon.*` blocks carried `@number
/// player_id` tags copy-pasted onto signatures that spell the parameter
/// differently.
///
/// A declared `(...)` is exempt *here* and only here: documenting a variadic
/// with one representative tag is ldoc's own convention, which
/// `globals.print`/`print_err`/`print_warn` follow correctly. What a `(...)`
/// cannot get away with is not being variadic, and
/// [`every_doc_block_matches_the_closure_its_binding_is_installed_with`] is
/// what says so — which is how `parse_blueprint`'s `(...)` would be caught
/// today.
#[test]
fn every_doc_block_documents_the_parameters_its_signature_declares() {
    let mut examined = 0usize;
    let mut problems: Vec<String> = Vec::new();
    for module in modules() {
        for (name, block) in &module.entries {
            let Some((_, _, params)) = declared_signature(block) else {
                continue;
            };
            // Counted before the variadic exemption and before the comparison:
            // a floor that only counts blocks which *passed* is reduced by the
            // very defect it stands guard over, and then fires "the parse
            // broke" instead of naming the wrong block.
            examined += 1;
            if params == ["..."] {
                continue;
            }
            let documented: Vec<String> = doc_params(block)
                .into_iter()
                .map(|param| param.name)
                .collect();
            if documented != params {
                problems.push(format!(
                    "  `{}.{name}` declares `({})` but documents {:?} -- add, remove or \
                     rename the `@param`/`@string`/`@number`/`@bool`/`@tparam` lines so \
                     they name the declared parameters in order, or fix the `function \
                     {}.{name}(..)` line",
                    module.name,
                    params.join(", "),
                    documented,
                    module.name,
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} doc block(s) document parameters their own signature does not declare:\n{}",
        problems.len(),
        problems.join("\n")
    );
    assert!(
        examined >= 35,
        "only {examined} doc blocks carried a `function ns.name(..)` line at all; the \
         parse broke and this test would then assert nothing"
    );
}

/// A `__doc_entry_X` and a binding at key `X` must come in pairs, and the block
/// must declare `function <module>.X(..)` under the module's own `@module`
/// name.
///
/// Both directions, and against the live tables rather than a list: a doc entry
/// with no binding is documentation for a function a script cannot call, and a
/// binding with no doc entry is a function no reader can find. The generalised
/// form of `lua_docs`' `the_goal_doc_entries_are_exactly_the_goal_surface`,
/// which held only `goal`.
///
/// The namespace half is what catches a block copy-pasted between modules:
/// `world.find_entities_in_radius` and `rcon.find_entities_in_radius` are
/// different functions with near-identical documentation, and a block that
/// wandered from one file to the other would otherwise read as correct.
#[test]
fn every_doc_entry_names_a_binding_and_every_binding_a_doc_entry() {
    let mut examined = 0usize;
    let mut problems: Vec<String> = Vec::new();
    for module in modules() {
        let expected_module = module.file.trim_end_matches(".lua");
        if module.name != expected_module {
            problems.push(format!(
                "  {} declares `-- @module {}` but is written to `{}` -- the generated \
                 file name and the namespace a reader types must agree",
                module.file, module.name, module.file
            ));
        }
        for (name, block) in &module.entries {
            examined += 1;
            let is_function = module.functions.contains(name);
            if !is_function && !module.values.contains(name) {
                problems.push(format!(
                    "  `__doc_entry_{name}` documents `{}.{name}`, but the module installs \
                     nothing at `{name}` -- add the binding, or drop the documentation",
                    module.name
                ));
                continue;
            }
            if !is_function {
                // `globals.Direction` is a table and `globals.all_bots` a list;
                // ldoc documents those as an assignment, not a signature.
                if !block
                    .lines()
                    .any(|line| line.starts_with(&format!("{}.{name}", module.name)))
                {
                    problems.push(format!(
                        "  `{}.{name}` is installed as a value, but its doc block never \
                         opens a line with `{}.{name}` -- show the reader the name they \
                         type",
                        module.name, module.name
                    ));
                }
                continue;
            }
            match declared_signature(block) {
                None => problems.push(format!(
                    "  `{}.{name}` is installed as a function, but its doc block declares no \
                     `function {}.{name}(..)` line -- add one, so the reader is shown a \
                     signature and the parameter guard has something to check",
                    module.name, module.name
                )),
                Some((namespace, declared, _))
                    if (&namespace, &declared) != (&module.name, name) =>
                {
                    problems.push(format!(
                        "  `__doc_entry_{name}` on module `{}` declares `function \
                         {namespace}.{declared}(..)` -- rename the declaration to \
                         `{}.{name}`, or move the block to the module it belongs to",
                        module.name, module.name
                    ));
                }
                Some(_) => {}
            }
        }
        let documented: BTreeSet<String> = module.entries.keys().cloned().collect();
        for name in module
            .own
            .intersection(&module.functions)
            .filter(|name| !documented.contains(*name))
        {
            problems.push(format!(
                "  `{}.{name}` is installed but nothing documents it -- add a \
                 `__doc_entry_{name}` beside the binding, or stop installing it",
                module.name
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} doc entry/binding pair(s) disagree:\n{}",
        problems.len(),
        problems.join("\n")
    );
    // Reachable, and covering this crate's actual historical failure: a
    // truncated `pairs` walk finds neither doc entries nor bindings, so
    // `problems` stays empty and only the floor is left to notice. It is
    // asserted second so that a real defect -- which does fill `problems` --
    // names itself rather than being reported as a broken parse.
    assert!(
        examined >= 40,
        "only {examined} `__doc_entry_*` blocks were found across the four modules; the \
         parse broke and this test would then assert nothing"
    );
}

/// A doc block's parameter list must match the Rust closure the binding is
/// actually installed with: same arity, same optionality, and no primitive
/// contradiction.
///
/// This is the half that reaches outside the doc string, and the one that found
/// defects the audit missed: `rcon.insert_to_inventory` and
/// `rcon.remove_from_inventory` documented `@string inventory_type` over a
/// closure taking `u32` — a `defines.inventory` index — so a script that
/// followed the documentation and passed `"main"` got mlua's conversion error
/// instead of an insert.
///
/// Names are *not* compared: the Rust names legitimately differ (`goal.plan`'s
/// closure spells its first parameter `g`, `goal` being a module name in that
/// file). Kinds are compared only where both sides commit — see [`Kind::Any`].
/// Optionality is compared exactly, `Option<T>` against ldoc's `[opt]`, which
/// is the audit's "optional params documented as required" finding.
#[test]
fn every_doc_block_matches_the_closure_its_binding_is_installed_with() {
    let sources = globals_sources();
    let mut by_module: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    let mut signatures_found = 0usize;
    for (module, source) in &sources {
        let found = closure_params_by_name(source);
        let into = by_module.entry(module.clone()).or_default();
        for (name, params) in found {
            signatures_found += params.len();
            into.entry(name).or_default().extend(params);
        }
    }

    let mut examined = 0usize;
    let mut problems: Vec<String> = Vec::new();
    for module in modules() {
        let empty = BTreeMap::new();
        let found = by_module.get(&module.name).unwrap_or(&empty);
        for (name, block) in &module.entries {
            if !module.functions.contains(name) {
                continue;
            }
            // Counted here, before anything can go wrong, for the reason
            // spelled out in the sibling test: the floor must count what was
            // examined, never what passed.
            examined += 1;
            let Some((_, _, declared)) = declared_signature(block) else {
                // Owned by `every_doc_entry_names_a_binding_and_every_binding_a_doc_entry`.
                continue;
            };
            let installed = match found.get(name).map(Vec::as_slice) {
                Some([one]) => one,
                Some(many) => {
                    problems.push(format!(
                        "  `{}.{name}` is installed by {} different \
                         `.set(\"{name}\", lua.create_function(..))` sites under \
                         `src/globals/`, so which closure documents it is ambiguous -- \
                         install it once, or this guard has to be taught how to tell them \
                         apart",
                        module.name,
                        many.len()
                    ));
                    continue;
                }
                None => {
                    problems.push(format!(
                        "  `{}.{name}` is installed at runtime, but no \
                         `.set(\"{name}\", lua.create_function(move |lua, ..|))` was found \
                         under `src/globals/{}` -- register it that way, or teach this \
                         guard the new shape in `closure_params_by_name`",
                        module.name, module.name
                    ));
                    continue;
                }
            };
            let Some(rust) = rust_params(installed) else {
                problems.push(format!(
                    "  `{}.{name}`'s closure parameters `{installed}` did not parse -- \
                     spell them `|lua, name: T|` or `|lua, (a, b): (T, U)|`, or teach \
                     `rust_params` the new shape",
                    module.name
                ));
                continue;
            };

            if rust.variadic != (declared == ["..."]) {
                problems.push(format!(
                    "  `{}.{name}` declares `({})` but its closure takes {} -- a Lua \
                     variadic and a `LuaVariadic<T>` parameter go together, and neither \
                     stands alone",
                    module.name,
                    declared.join(", "),
                    if rust.variadic {
                        "`LuaVariadic<..>`".to_string()
                    } else {
                        format!("{} fixed parameter(s)", rust.params.len())
                    }
                ));
                continue;
            }
            if rust.variadic {
                continue;
            }
            if rust.params.len() != declared.len() {
                problems.push(format!(
                    "  `{}.{name}` declares {} parameter(s) `({})` but its closure takes \
                     {} -- fix the `function {}.{name}(..)` line, or the closure",
                    module.name,
                    declared.len(),
                    declared.join(", "),
                    rust.params.len(),
                    module.name
                ));
                continue;
            }
            let documented = doc_params(block);
            if documented.len() != rust.params.len() {
                // The tag/signature disagreement itself belongs to the sibling
                // test; here it only means there is nothing to compare against.
                continue;
            }
            for (index, ((kind, optional), tag)) in
                rust.params.iter().zip(documented.iter()).enumerate()
            {
                let position = index + 1;
                if *kind != Kind::Any && tag.kind != Kind::Any && *kind != tag.kind {
                    problems.push(format!(
                        "  `{}.{name}` documents parameter {position} `{}` as {}, but its \
                         closure takes {} -- change the tag to match the closure, or the \
                         closure to match the documented type",
                        module.name,
                        tag.name,
                        tag.kind.describe(),
                        kind.describe()
                    ));
                }
                if *optional != tag.optional {
                    problems.push(format!(
                        "  `{}.{name}` documents parameter {position} `{}` as {}, but its \
                         closure takes {} -- {}",
                        module.name,
                        tag.name,
                        if tag.optional { "optional" } else { "required" },
                        if *optional {
                            "an `Option<..>`"
                        } else {
                            "a required value"
                        },
                        if *optional {
                            "mark the tag `[opt]`, or drop the `Option<..>`"
                        } else {
                            "drop the `[opt]` marker, or make the closure parameter an `Option<..>`"
                        }
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} doc block(s) disagree with the closure their binding is installed with:\n{}",
        problems.len(),
        problems.join("\n")
    );
    assert!(
        examined >= 35,
        "only {examined} documented functions were compared against a closure; the parse \
         broke and this test would then assert nothing"
    );
    assert!(
        signatures_found >= 35,
        "only {signatures_found} `lua.create_function` registrations were found under \
         `src/globals/`; the source scan broke and this test would then assert nothing"
    );
}

/// **`@return` is not guarded, and this records why rather than pretending.**
///
/// It was the worst of the audit's findings — `world.inventory` promised
/// `{types.FactorioEntity}` and answers a number, contradicting its own
/// summary; `world.find_free_resource_rect` promised `types.FactorioPlayer` and
/// answers a `Rect`; `place_blueprint`, `cheat_blueprint` and `revive_ghost`
/// promised nothing at all and hand back entities — so it is also the field
/// most worth checking. It is not checkable here, for a reason that is about
/// the code and not about effort:
///
/// * **There is no return type to read.** A binding is a closure passed to
///   `lua.create_function`, whose return type is inferred: nowhere in
///   `src/globals/**` is the type of any binding's result written down. The
///   three checks above work because the Rust *parameter* types are annotated —
///   mlua requires it — and the return type never is.
///
/// * **The value is built, not named.** The tail is `lua.to_value(&rect)`,
///   `Ok(*count)`, `Ok(())`, or a helper's result several calls away, often on
///   one arm of a `match` inside an `async move` block. Deciding which of those
///   a `@return` describes is type inference, and a textual approximation of
///   type inference is precisely the guard that rots.
///
/// * **Slicing a closure body cannot even be done reliably here.** The sibling
///   guard in `globals/rcon.rs` had to abandon brace counting because the RCON
///   client is full of `format!("{..}")`; the binding bodies have the same
///   problem, and the `.set(` boundaries used above slice a *registration*, not
///   a closure body.
///
/// What *is* guarded is the part that names a type:
/// `lua_docs::reconcile_documented_types` requires every `` `types.X` `` a
/// `@return` mentions to have a `schema_for!` root, in both directions. That
/// leaves the returns which name no type at all — `@return {[string]=number,...}`
/// is how `rcon.inventory_contents_at` stayed wrong — outside any check.
///
/// Closing it would mean annotating each binding's return type in Rust so that
/// rustc, not a parser, holds the two together. That is a change to production
/// code across four files and was out of scope here; it is the honest next
/// step, and it is the only one that would not be a mirror.
#[test]
fn the_return_field_is_not_guarded_here() {
    // Asserting the one thing that *is* true of `@return` today, so this is a
    // test and not a comment: every type a `@return` names is reconciled
    // against the rendered schemas, and at least one block names one.
    let named: usize = modules()
        .iter()
        .flat_map(|module| module.entries.values())
        .filter(|block| block.contains("@return `types.") || block.contains("@return {`types."))
        .count();
    assert!(
        named >= 5,
        "only {named} doc blocks name a `types.X` in their `@return`; the part of the \
         return field that *is* checked, by `lua_docs::reconcile_documented_types`, has \
         stopped covering anything"
    );
}

// ---------------------------------------------------------------------------
// The parsers, on inputs these tests control
// ---------------------------------------------------------------------------

/// The tag reader, shown discriminating on each shape the doc strings use.
#[test]
fn doc_params_reads_every_tag_shape_the_blocks_use() {
    let block = "\
--- summary
-- prose mentioning @string in the middle of a line, which is not a tag
-- @string ore_name name of the resource
-- @number[opt=1] count how many
-- @bool force_build forces the build
-- @param near `types.Position` to search from
-- @tparam[opt] table opts `{ bot = id }`
-- @tparam {int} helper_player_ids array of player ids
--   a continuation line, which carries no tag
-- @return `types.Rect` the free rectangle
-- @raise if the item name is empty
function world.demo(ore_name, count, force_build, near, opts, helper_player_ids)
end";
    let params = doc_params(block);
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "ore_name",
            "count",
            "force_build",
            "near",
            "opts",
            "helper_player_ids"
        ],
        "`@return`, `@raise` and continuation lines must not read as parameters"
    );
    assert_eq!(
        params.iter().map(|p| p.kind).collect::<Vec<_>>(),
        [
            Kind::Str,
            Kind::Num,
            Kind::Bool,
            Kind::Any,
            Kind::Any,
            Kind::Any
        ]
    );
    assert_eq!(
        params.iter().map(|p| p.optional).collect::<Vec<_>>(),
        [false, true, false, false, true, false],
        "`[opt]` and `[opt=1]` mark a parameter optional; a bare tag does not"
    );
    assert_eq!(
        declared_signature(block).map(|(ns, name, args)| (ns, name, args.len())),
        Some(("world".to_string(), "demo".to_string(), 6))
    );
}

/// The closure reader, shown on each registration shape `src/globals/**` uses.
#[test]
fn rust_params_reads_every_closure_shape_the_bindings_use() {
    /// One case: the closure's parameter text, the (kind, optional) pairs it
    /// must read as, and whether it is variadic.
    type Case = (&'static str, Vec<(Kind, bool)>, bool);
    let cases: [Case; 7] = [
        ("()", vec![], false),
        ("(): ()", vec![], false),
        ("name: String", vec![(Kind::Str, false)], false),
        (
            "strings: LuaVariadic<String>",
            vec![(Kind::Str, false)],
            true,
        ),
        (
            "(player_id, item_name): (PlayerId, String)",
            vec![(Kind::Num, false), (Kind::Str, false)],
            false,
        ),
        (
            "(a, b, c, d): ( LuaTable, f64, Option<String>, Option<String>, )",
            vec![
                (Kind::Any, false),
                (Kind::Num, false),
                (Kind::Str, true),
                (Kind::Str, true),
            ],
            false,
        ),
        (
            "(player_id, blueprint, position, direction, force_build, only_ghosts, ids): \
             (PlayerId, String, LuaTable, u8, bool, bool, Vec<PlayerId>)",
            vec![
                (Kind::Num, false),
                (Kind::Str, false),
                (Kind::Any, false),
                (Kind::Num, false),
                (Kind::Bool, false),
                (Kind::Bool, false),
                (Kind::Any, false),
            ],
            false,
        ),
    ];
    for (text, expected, variadic) in cases {
        let parsed = rust_params(text).unwrap_or_else(|| panic!("`{text}` did not parse"));
        assert_eq!(parsed.params, expected, "parsing `{text}`");
        assert_eq!(parsed.variadic, variadic, "parsing `{text}`");
    }
}

/// The registration scan, shown finding both `create_function` and
/// `create_async_function`, both with and without `move`, and shown *not*
/// finding the `t.set("x", value)` calls that fill result tables.
#[test]
fn closure_params_by_name_finds_registrations_and_only_registrations() {
    let source = "\
    map_table.set(
        \"inventory\",
        lua.create_function(move |_lua, (player_id, item_name): (PlayerId, String)| {
            t.set(\"x\", 1)?;
        })?,
    )?;
    table.set(\"start\", lua.create_async_function(move |lua, plan: LuaUserDataRef<PlanValue>| {})?)?;
    metatable.set(\"__tostring\", lua.create_function(|_, t: LuaTable| render(&t))?)?;
    t.set(\"kind\", \"walk\")?;
";
    let found = closure_params_by_name(source);
    assert_eq!(
        found.keys().collect::<Vec<_>>(),
        ["__tostring", "inventory", "start"],
        "`t.set(\"x\", 1)` and `t.set(\"kind\", \"walk\")` are not registrations"
    );
    assert_eq!(
        found["inventory"],
        ["(player_id, item_name): (PlayerId, String)"]
    );
    assert_eq!(found["start"], ["plan: LuaUserDataRef<PlanValue>"]);
}

/// Every module the generator writes must be found by the source scan under a
/// matching directory name, or the closure check silently examines nothing for
/// it.
///
/// The floor in
/// [`every_doc_block_matches_the_closure_its_binding_is_installed_with`] is a
/// total across modules, so one module going missing could hide under the other
/// three; this says each of the four was reached.
#[test]
fn the_source_scan_reaches_every_documented_module() {
    let found: BTreeSet<String> = globals_sources()
        .into_iter()
        .map(|(module, _)| module)
        .collect();
    for module in modules() {
        assert!(
            found.contains(&module.name),
            "no `src/globals/` source was attributed to module `{}`; the closure check \
             examines nothing for it. Sources are attributed by path -- `world.rs` to \
             `world`, `goal/*.rs` to `goal` -- so a module whose bindings moved needs its \
             file or directory named after it",
            module.name
        );
    }
}
