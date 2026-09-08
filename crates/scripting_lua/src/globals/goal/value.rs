//! `goal.*` values: Lua tables that describe a `Goal` without holding a
//! handle into the interpreter or the running game.
//!
//! Every constructor (`goal.have`, `goal.researched`, `goal.produced`,
//! `goal.producing`, `goal.sustain`, `goal.extracted`, `goal.gathered`,
//! `goal.built`, `goal.charted`, `goal.all`) validates
//! eagerly, so a mistake raises on the line that made it. [`goal_from_lua`]
//! validates again on the way back to a planner `Goal`, because a Lua table
//! is open: nothing stops a script from hand-building one that skips what a
//! constructor would have demanded. Both paths raise through the same
//! [`goal_error`], so a script sees one error shape regardless of which side
//! caught the mistake.

use super::goal_error;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::types::Position;
use factorio_bot_planner::{BotId, Goal, Holder, Site};

/// Installs `have`, `researched`, `produced`, `producing`, `sustain`,
/// `extracted`, `gathered`, `built`, `charted` and `all` on `table`.
///
/// They all share one metatable -- built once here and cloned (cheaply: a
/// Lua table is refcounted) onto every value they return -- so `tostring(g)`
/// renders the same way regardless of which of them built `g`.
///
/// Called by `create_lua_goal_with`: these *are* `goal.have`,
/// `goal.researched`, `goal.produced`, ... as a script sees them.
///
/// # A kind lives in more than one place here
///
/// Adding a goal kind is five edits in this file and two outside it, and
/// missing any one of them fails in exactly one place rather than everywhere
/// -- see [`KINDS`] for the incident that established this. The list, so the
/// next person adding a kind does not have to reconstruct it:
///
///   1. the constructor, here;
///   2. [`goal_from_lua`]'s arm, which is the only path to a planner `Goal`;
///   3. [`render_goal`]'s arm, which is `__tostring`;
///   4. [`KINDS`], which only `goal.all` consults;
///   5. a test that composes it **inside `goal.all`**, because 4 is the one
///      thing 1-3 cannot fail on;
///   6. a `__doc_entry_<kind>` string in `goal/mod.rs` --
///      `lua_docs::tests::the_goal_doc_entries_are_exactly_the_goal_surface`
///      fails without it, from both directions;
///   7. the expected set in `goal/mod.rs`'s
///      `the_goal_table_offers_exactly_the_new_surface`, which is a set
///      comparison and so fails on an *extra* function too.
pub(crate) fn install_goal_constructors(lua: &Lua, table: &LuaTable) -> LuaResult<()> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, t: LuaTable| render_goal(&t))?,
    )?;

    let mt = metatable.clone();
    table.set(
        "have",
        lua.create_function(
            move |lua, (item, count, opts): (LuaValue, LuaValue, Option<LuaTable>)| {
                let item = require_item(item)?;
                let count = require_count(count)?;
                let (bot, via) = match &opts {
                    Some(opts) => (
                        require_bot(opts.get("bot")?)?,
                        require_via(opts.get("via")?)?,
                    ),
                    None => (None, None),
                };
                let t = lua.create_table()?;
                t.set("kind", "have")?;
                t.set("item", item)?;
                t.set("count", count)?;
                if let Some(bot) = bot {
                    t.set("bot", bot)?;
                }
                // Set only when the caller named one. A goal table with no
                // `via` key is what every existing script produces, and
                // `goal_from_lua` reads its absence as `None` -- the caller
                // did not choose -- rather than as an empty choice.
                if let Some(via) = via {
                    t.set("via", via)?;
                }
                t.set_metatable(Some(mt.clone()))?;
                Ok(t)
            },
        )?,
    )?;

    let mt = metatable.clone();
    table.set(
        "researched",
        lua.create_function(move |lua, tech: LuaValue| {
            let tech = require_technology(tech)?;
            let t = lua.create_table()?;
            t.set("kind", "researched")?;
            t.set("technology", tech)?;
            t.set_metatable(Some(mt.clone()))?;
            Ok(t)
        })?,
    )?;

    let mt = metatable.clone();
    table.set(
        "producing",
        lua.create_function(move |lua, (item, per_minute): (LuaValue, LuaValue)| {
            let item = require_item(item)?;
            // The same `>= 1` integer rule as a count, and for a sharper
            // reason: the planner's own `Goal::Producing` carries a `u32`
            // rather than an `f64` precisely so that the machine count is
            // integer arithmetic end to end. A rate arriving as 15.0000001
            // from Lua would put that back.
            let per_minute = require_count(per_minute)?;
            let t = lua.create_table()?;
            t.set("kind", "producing")?;
            t.set("item", item)?;
            t.set("per_minute", per_minute)?;
            t.set_metatable(Some(mt.clone()))?;
            Ok(t)
        })?,
    )?;

    // `produced` is `have`'s sibling and deliberately shaped like it: same
    // item, same count, same `{ bot = , via = }` options. What differs is the
    // *question* -- `have` subtracts what a bot already holds, `produced`
    // never does, because a bot carrying six labs has not crafted one and a
    // Factorio 2.0 `craft-item` trigger fires on the act. The extra option is
    // `unlocks`; see [`require_unlocks`] for why it is a claim and not a
    // grant.
    let mt = metatable.clone();
    table.set(
        "produced",
        lua.create_function(
            move |lua, (item, count, opts): (LuaValue, LuaValue, Option<LuaTable>)| {
                let item = require_item(item)?;
                let count = require_count(count)?;
                let (bot, via, unlocks) = match &opts {
                    Some(opts) => (
                        require_bot(opts.get("bot")?)?,
                        require_via(opts.get("via")?)?,
                        require_unlocks(opts.get("unlocks")?)?,
                    ),
                    None => (None, None, None),
                };
                let t = lua.create_table()?;
                t.set("kind", "produced")?;
                t.set("item", item)?;
                t.set("count", count)?;
                if let Some(bot) = bot {
                    t.set("bot", bot)?;
                }
                // Absent is not a value, exactly as in `have`: a goal built
                // without `via` carries no `via` key, which `goal_from_lua`
                // reads as "the caller did not choose".
                if let Some(via) = via {
                    t.set("via", via)?;
                }
                if let Some(unlocks) = unlocks {
                    t.set("unlocks", unlocks)?;
                }
                t.set_metatable(Some(mt.clone()))?;
                Ok(t)
            },
        )?,
    )?;

    // `extracted` and `gathered` take an ENTITY, not an item -- `crude-oil`
    // here names the well in the ground. What comes out of one is a fluid,
    // and no character inventory can hold a fluid, so there is no count to
    // ask for and none is accepted. The two are one rung apart: `extracted`
    // is a machine working the well, `gathered` is that plus somewhere for
    // what it pumps to go. See `Goal::Extracted` / `Goal::Gathered`.
    let mt = metatable.clone();
    table.set(
        "extracted",
        lua.create_function(move |lua, (entity, opts): (LuaValue, Option<LuaTable>)| {
            let entity = require_entity(entity)?;
            let unlocks = match &opts {
                Some(opts) => require_unlocks(opts.get("unlocks")?)?,
                None => None,
            };
            let t = lua.create_table()?;
            t.set("kind", "extracted")?;
            t.set("entity", entity)?;
            if let Some(unlocks) = unlocks {
                t.set("unlocks", unlocks)?;
            }
            t.set_metatable(Some(mt.clone()))?;
            Ok(t)
        })?,
    )?;

    let mt = metatable.clone();
    table.set(
        "gathered",
        lua.create_function(move |lua, (entity, opts): (LuaValue, Option<LuaTable>)| {
            let entity = require_entity(entity)?;
            let unlocks = match &opts {
                Some(opts) => require_unlocks(opts.get("unlocks")?)?,
                None => None,
            };
            let t = lua.create_table()?;
            t.set("kind", "gathered")?;
            t.set("entity", entity)?;
            if let Some(unlocks) = unlocks {
                t.set("unlocks", unlocks)?;
            }
            t.set_metatable(Some(mt.clone()))?;
            Ok(t)
        })?,
    )?;

    let mt = metatable.clone();
    table.set(
        "sustain",
        lua.create_function(
            move |lua, (item, per_minute, window_ticks): (LuaValue, LuaValue, LuaValue)| {
                let item = require_item(item)?;
                let per_minute = require_count(per_minute)?;
                // **Required, with no default.** The same refusal as
                // `supervisor.witness`'s `within_ticks`: the window is the
                // number that decides what a failure means, and a library that
                // guessed it would hand back a verdict nobody derived. A
                // missing third argument raises here, on the line that made
                // the goal, rather than at `goal.plan` -- `require_count`'s
                // own message names the argument.
                let window_ticks = require_count(window_ticks)?;
                let t = lua.create_table()?;
                t.set("kind", "sustain")?;
                t.set("item", item)?;
                t.set("per_minute", per_minute)?;
                t.set("window_ticks", window_ticks)?;
                t.set_metatable(Some(mt.clone()))?;
                Ok(t)
            },
        )?,
    )?;

    let mt = metatable.clone();
    table.set(
        "built",
        lua.create_function(
            move |lua, (blueprint, opts): (LuaValue, Option<LuaValue>)| {
                let blueprint = require_blueprint(blueprint)?;
                let site = require_site(opts)?;
                let t = lua.create_table()?;
                t.set("kind", "built")?;
                t.set("blueprint", blueprint)?;
                // Written as flat `x`/`y` for `At` and a nested `near` table for
                // `Near`, so [`site_from_table`] can tell the three apart on the
                // way back -- see its own doc for why one function reads what
                // both this constructor and a hand-built table write.
                match site {
                    Site::At(pos) => {
                        t.set("x", pos.x())?;
                        t.set("y", pos.y())?;
                    }
                    Site::Near(pos) => {
                        let near = lua.create_table()?;
                        near.set("x", pos.x())?;
                        near.set("y", pos.y())?;
                        t.set("near", near)?;
                    }
                    Site::Beside { of, steps } => {
                        let beside = lua.create_table()?;
                        beside.set("x", of.x())?;
                        beside.set("y", of.y())?;
                        beside.set("dx", steps.0)?;
                        beside.set("dy", steps.1)?;
                        t.set("beside", beside)?;
                    }
                    Site::Anchored(pos) => {
                        let anchored = lua.create_table()?;
                        anchored.set("x", pos.x())?;
                        anchored.set("y", pos.y())?;
                        t.set("anchored", anchored)?;
                    }
                    Site::Anywhere => {}
                }
                t.set_metatable(Some(mt.clone()))?;
                Ok(t)
            },
        )?,
    )?;

    let mt = metatable.clone();
    table.set(
        "charted",
        lua.create_function(move |lua, (x, y, radius): (LuaValue, LuaValue, LuaValue)| {
            let t = lua.create_table()?;
            t.set("kind", "charted")?;
            t.set("x", require_coordinate(x, "x")?)?;
            t.set("y", require_coordinate(y, "y")?)?;
            // A radius is a coordinate as far as parsing goes -- a finite
            // number -- and a non-positive one is refused here rather than
            // reaching `PlanState::charting`, where every probe would land on
            // the origin and a one-tile disc would report the whole map as
            // seen.
            let radius = require_coordinate(radius, "radius")?;
            if radius <= 0.0 {
                return Err(goal_error("goal.charted needs a positive radius"));
            }
            t.set("radius", radius)?;
            t.set_metatable(Some(mt.clone()))?;
            Ok(t)
        })?,
    )?;

    table.set(
        "all",
        lua.create_function(move |lua, goals: LuaTable| {
            let len = goals.raw_len();
            if len == 0 {
                return Err(goal_error("goal.all needs at least one sub-goal"));
            }
            let sub_goals = lua.create_table()?;
            for i in 1..=len {
                let value: LuaValue = goals.get(i)?;
                sub_goals.set(i, require_goal_table(value)?)?;
            }
            let t = lua.create_table()?;
            t.set("kind", "all")?;
            t.set("goals", sub_goals)?;
            t.set_metatable(Some(metatable.clone()))?;
            Ok(t)
        })?,
    )?;

    Ok(())
}

/// Converts a goal table -- built by `goal.have`/`goal.researched`/`goal.producing`/
/// `goal.all`, or hand-built by a script -- to the planner's own [`Goal`].
///
/// Never produces `Holder::Share`: that variant is the expansion's own
/// internal marker for one bot's slice of a split goal, not a shape a caller
/// can name.
///
/// `goal.plan` is the caller: it is the boundary where a goal value stops
/// being a script's table and becomes something the planner can expand.
pub(crate) fn goal_from_lua(value: &LuaTable) -> LuaResult<Goal> {
    match require_kind(value)?.as_str() {
        "have" => {
            let item = require_item(value.get("item")?)?;
            let count = require_count(value.get("count")?)?;
            let whose = match require_bot(value.get("bot")?)? {
                Some(bot) => Holder::Bot(BotId(bot)),
                None => Holder::Anyone,
            };
            let via = require_via(value.get("via")?)?;
            Ok(Goal::Have {
                item,
                count,
                whose,
                via,
            })
        }
        "researched" => Ok(Goal::Researched(require_technology(
            value.get("technology")?,
        )?)),
        "produced" => Ok(Goal::Produced {
            item: require_item(value.get("item")?)?,
            count: require_count(value.get("count")?)?,
            whose: match require_bot(value.get("bot")?)? {
                Some(bot) => Holder::Bot(BotId(bot)),
                None => Holder::Anyone,
            },
            unlocks: require_unlocks(value.get("unlocks")?)?,
            via: require_via(value.get("via")?)?,
        }),
        "producing" => Ok(Goal::Producing {
            item: require_item(value.get("item")?)?,
            per_minute: require_count(value.get("per_minute")?)?,
        }),
        "extracted" => Ok(Goal::Extracted {
            entity: require_entity(value.get("entity")?)?,
            unlocks: require_unlocks(value.get("unlocks")?)?,
        }),
        "gathered" => Ok(Goal::Gathered {
            entity: require_entity(value.get("entity")?)?,
            unlocks: require_unlocks(value.get("unlocks")?)?,
        }),
        // A hand-built table with no `window_ticks` is refused here exactly as
        // the constructor refuses a missing third argument: a standing goal
        // with no window is `Goal::Producing`, which is capacity and not
        // output, and guessing one would be answering a question nobody asked.
        "sustain" => Ok(Goal::Sustain {
            item: require_item(value.get("item")?)?,
            per_minute: require_count(value.get("per_minute")?)?,
            window_ticks: require_count(value.get("window_ticks")?)?,
        }),
        "built" => Ok(Goal::Built {
            blueprint: require_blueprint(value.get("blueprint")?)?,
            site: site_from_table(value)?,
        }),
        "charted" => Ok(Goal::Charted {
            around: Position::new(
                require_coordinate(value.get("x")?, "x")?,
                require_coordinate(value.get("y")?, "y")?,
            ),
            radius: require_coordinate(value.get("radius")?, "radius")?,
        }),
        "all" => {
            let goals = require_table_field(value.get("goals")?, "goals")?;
            let len = goals.raw_len();
            let mut out = Vec::with_capacity(len);
            for i in 1..=len {
                out.push(goal_from_lua(&goals.get::<LuaTable>(i)?)?);
            }
            Ok(Goal::All(out))
        }
        other => Err(unknown_kind(other)),
    }
}

/// Renders a goal table for `__tostring`. Recurses into `all`'s sub-goals the
/// same way [`goal_from_lua`] recurses into `Goal::All`, so the two never
/// disagree about shape.
fn render_goal(t: &LuaTable) -> LuaResult<String> {
    match require_kind(t)?.as_str() {
        "have" => {
            let item = require_item(t.get("item")?)?;
            let count = require_count(t.get("count")?)?;
            match require_via(t.get("via")?)? {
                Some(recipe) => Ok(format!("have {count} {item} via {recipe}")),
                None => Ok(format!("have {count} {item}")),
            }
        }
        "researched" => Ok(format!(
            "researched {}",
            require_technology(t.get("technology")?)?
        )),
        // Rendered exactly as `Goal::Display` renders it -- see
        // `the_new_kinds_render_the_way_the_planner_renders_them`. The two
        // arms above predate that rule and are deliberately left alone;
        // nothing new should add a third dialect.
        "produced" => {
            let item = require_item(t.get("item")?)?;
            let count = require_count(t.get("count")?)?;
            let head = match require_unlocks(t.get("unlocks")?)? {
                Some(tech) => format!("produce {count} {item} to unlock {tech}"),
                None => format!("produce {count} {item}"),
            };
            match require_via(t.get("via")?)? {
                Some(recipe) => Ok(format!("{head} via {recipe}")),
                None => Ok(head),
            }
        }
        "producing" => Ok(format!(
            "producing {} {}/min",
            require_count(t.get("per_minute")?)?,
            require_item(t.get("item")?)?
        )),
        "extracted" => {
            let entity = require_entity(t.get("entity")?)?;
            Ok(match require_unlocks(t.get("unlocks")?)? {
                Some(tech) => format!("extract from {entity} to unlock {tech}"),
                None => format!("extract from {entity}"),
            })
        }
        "gathered" => {
            let entity = require_entity(t.get("entity")?)?;
            Ok(match require_unlocks(t.get("unlocks")?)? {
                Some(tech) => format!("gather {entity} into a tank to unlock {tech}"),
                None => format!("gather {entity} into a tank"),
            })
        }
        // Rendered exactly as `Goal::Display` renders it, unlike the two arms
        // above -- see `sustain_render_goal_agrees_with_the_planner_goals_own_display`.
        "sustain" => Ok(format!(
            "sustain {} {}/min over {} ticks",
            require_count(t.get("per_minute")?)?,
            require_item(t.get("item")?)?,
            require_count(t.get("window_ticks")?)?
        )),
        "built" => {
            let blueprint = require_blueprint(t.get("blueprint")?)?;
            let where_ = match site_from_table(t)? {
                Site::At(pos) => format!("at {pos}"),
                Site::Near(pos) => format!("near {pos}"),
                Site::Anywhere => "anywhere".to_string(),
                Site::Anchored(pos) => format!("at its recorded anchor {pos}"),
                Site::Beside { of, steps } => format!(
                    "({}, {}) pitch(es) from the block at {of}",
                    steps.0, steps.1
                ),
            };
            Ok(format!("build {}-byte block {}", blueprint.len(), where_))
        }
        "charted" => {
            let around = Position::new(
                require_coordinate(t.get("x")?, "x")?,
                require_coordinate(t.get("y")?, "y")?,
            );
            Ok(format!(
                "chart within {:.0} tiles of {around}",
                require_coordinate(t.get("radius")?, "radius")?
            ))
        }
        "all" => {
            let goals = require_table_field(t.get("goals")?, "goals")?;
            let len = goals.raw_len();
            let mut parts = Vec::with_capacity(len);
            for i in 1..=len {
                parts.push(render_goal(&goals.get::<LuaTable>(i)?)?);
            }
            Ok(format!("all {{ {} }}", parts.join(", ")))
        }
        other => Err(unknown_kind(other)),
    }
}

/// The `kind` field every goal table carries, as a plain `String`.
fn require_kind(t: &LuaTable) -> LuaResult<String> {
    match t.get("kind")? {
        LuaValue::String(s) => Ok(s.to_string_lossy()),
        _ => Err(goal_error("goal table is missing a string \"kind\" field")),
    }
}

/// The kinds a goal table may name. Fixed by this module -- no world is
/// consulted to decide whether a `kind` is one of them, which is why an
/// unknown one is a shape error rather than a semantic one.
///
/// **`charted` was missing from this list and it is not any more** -- the
/// paragraph that stood here calling it "a pre-existing defect ... left alone"
/// described the state before `charted` was added, and outlived it. The
/// incident itself is worth keeping: the parser and the renderer both handled
/// `charted`, so `goal.charted(...)` worked everywhere EXCEPT inside
/// `goal.all`, where [`require_known_kind`] rejected it as unknown. **A kind
/// added to the parser and not to this list fails in exactly one place, which
/// is the hardest kind of gap to find** -- so every kind here has a test that
/// composes it *inside* `goal.all`, which is the only construct that reads
/// this list.
const KINDS: &[&str] = &[
    "have",
    "researched",
    "produced",
    "producing",
    "sustain",
    "extracted",
    "gathered",
    "built",
    "charted",
    "all",
];

/// [`require_kind`], plus the check that it names a kind that exists.
fn require_known_kind(t: &LuaTable) -> LuaResult<String> {
    let kind = require_kind(t)?;
    if !KINDS.contains(&kind.as_str()) {
        return Err(unknown_kind(&kind));
    }
    Ok(kind)
}

fn unknown_kind(kind: &str) -> LuaError {
    goal_error(format!(
        "unknown goal kind \"{kind}\"; expected one of {}",
        KINDS.join(", ")
    ))
}

/// A `goal.all` element: must be a table naming a known `kind`, i.e.
/// something a constructor (or an equally careful hand-built table) produced.
///
/// The kind is checked here, at construction, not deferred to `goal.plan`:
/// `goal.all` is the one constructor that takes a caller-built table, and
/// which kinds exist is this module's own fact, needing no world.
fn require_goal_table(value: LuaValue) -> LuaResult<LuaTable> {
    match value {
        LuaValue::Table(t) => {
            require_known_kind(&t)?;
            Ok(t)
        }
        other => Err(goal_error(format!(
            "goal.all: every element must be a goal, got a {}",
            other.type_name()
        ))),
    }
}

fn require_table_field(value: LuaValue, field: &str) -> LuaResult<LuaTable> {
    match value {
        LuaValue::Table(t) => Ok(t),
        _ => Err(goal_error(format!(
            "goal table's \"{field}\" field must be a table"
        ))),
    }
}

fn require_item(value: LuaValue) -> LuaResult<String> {
    require_nonempty_string(value, "item")
}

/// `goal.have`'s optional `via`: the **recipe** that is to make the item.
///
/// # Absent is not a value, and this is where the two could be confused
///
/// A Lua table has no null, so a missing key and an explicit `nil` arrive
/// identically, and both mean *the caller did not choose* -- exactly what
/// `Goal::Have::via = None` means. What is refused here is an empty string
/// and a non-string: `{via = ""}` is a caller who meant to name a recipe and
/// named nothing, and letting it through as `Some("")` would reach the
/// planner as a recipe that does not exist and be refused a rung later with
/// a message about the world instead of about the script.
///
/// Raises on the line that built the goal, like every other `require_*` here.
fn require_via(value: LuaValue) -> LuaResult<Option<String>> {
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::String(s) => {
            let s = s.to_string_lossy();
            if s.is_empty() {
                Err(goal_error(
                    "goal via must be a non-empty recipe name; omit it entirely to let the \
                     planner choose",
                ))
            } else {
                Ok(Some(s))
            }
        }
        other => Err(goal_error(format!(
            "goal via must be a recipe name string, got a {}",
            other.type_name()
        ))),
    }
}

/// The **entity** `goal.extracted` / `goal.gathered` name: a resource in the
/// ground, like `crude-oil`.
///
/// Kept separate from [`require_item`] even though both are non-empty strings,
/// for the reason `RecipeName` is kept separate from `ItemId` in the planner:
/// an entity name and an item name coincide often enough to be believed and
/// differ exactly where it matters -- `crude-oil` is a well nothing can hold,
/// not a stack.
fn require_entity(value: LuaValue) -> LuaResult<String> {
    require_nonempty_string(value, "entity")
}

/// The optional `unlocks`: a technology this production or extraction fires
/// the trigger for.
///
/// # It is a claim about the game, not a grant
///
/// The name rides on the goal so the resulting `Effect::Researched` can land
/// on whichever action ends up doing the deed -- craft, smelt or mine -- which
/// only the producing method knows. Nothing here checks that the technology
/// exists, or that the act named actually triggers it: a wrong name makes the
/// **plan** believe a technology is finished, and the game will disagree. Say
/// it only where the trigger is real (`docs/superpowers/notes/
/// 2026-09-05-research-triggers.md` enumerates all 32), and omit it otherwise
/// -- omitting it is the shape every existing goal has.
///
/// Absent is not a value, as with [`require_via`]: a missing key and an
/// explicit `nil` both mean *the caller did not name one*, and an empty string
/// is refused rather than passed on as `Some("")`.
fn require_unlocks(value: LuaValue) -> LuaResult<Option<String>> {
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::String(s) => {
            let s = s.to_string_lossy();
            if s.is_empty() {
                Err(goal_error(
                    "goal unlocks must be a non-empty technology name; omit it entirely when the \
                     goal triggers no research",
                ))
            } else {
                Ok(Some(s))
            }
        }
        other => Err(goal_error(format!(
            "goal unlocks must be a technology name string, got a {}",
            other.type_name()
        ))),
    }
}

fn require_technology(value: LuaValue) -> LuaResult<String> {
    require_nonempty_string(value, "technology")
}

fn require_blueprint(value: LuaValue) -> LuaResult<String> {
    require_nonempty_string(value, "blueprint")
}

/// One coordinate of an anchor, read back off a goal table's flat `x`/`y`
/// fields, or off the nested `near` table's own `x`/`y`.
fn require_coordinate(value: LuaValue, field: &str) -> LuaResult<f64> {
    match value {
        LuaValue::Integer(n) => Ok(n as f64),
        LuaValue::Number(n) => Ok(n),
        _ => Err(goal_error(format!("anchor {field} must be a number"))),
    }
}

/// `goal.built`'s optional second argument: `{x=, y=}` for [`Site::At`],
/// `{near={x=, y=}}` for [`Site::Near`], or nothing at all for
/// [`Site::Anywhere`].
///
/// Delegates the table shape to [`site_from_table`] once the argument is
/// known to be a table (or absent), so the constructor and a hand-built
/// goal's own `goal_from_lua` conversion read the same two fields the same
/// way -- this is the one place the *argument*, rather than the table
/// `goal.built` goes on to build, is validated.
fn require_site(opts: Option<LuaValue>) -> LuaResult<Site> {
    match opts {
        None => Ok(Site::Anywhere),
        Some(LuaValue::Table(t)) => site_from_table(&t),
        Some(other) => Err(goal_error(format!(
            "goal.built's second argument must be a table ({{x=, y=}} or {{near={{x=, y=}}}}), \
             got a {}",
            other.type_name()
        ))),
    }
}

/// Reads a goal table's site fields back into a planner [`Site`]: a flat
/// `x`/`y` means [`Site::At`], a nested `near` table means [`Site::Near`],
/// and neither means [`Site::Anywhere`].
///
/// Shared by [`require_site`] (the constructor's own argument, already known
/// to be a table), [`goal_from_lua`]'s `"built"` arm (a table built by the
/// constructor above, or hand-built by a script) and `render_goal`'s
/// `"built"` arm -- one function, so a constructed table, a hand-built one
/// and its `tostring` can never disagree about which fields mean what. That
/// agreement is exactly what a Task 1 review caught missing: `render_goal`
/// used to be a second, hand-maintained copy that assumed every `"built"`
/// table was `Site::At` and hardcoded "at" into the string, which would have
/// rendered a sited block as though it were anchored.
fn site_from_table(t: &LuaTable) -> LuaResult<Site> {
    // Either coordinate commits to the anchor branch, not just `x`: a table
    // with only `y` set is anchor-shaped and malformed, not an unrelated
    // shape that happens to fall through to `Anywhere`. Checking `x` alone
    // let `{y = 1}` slip past both branches into `(false, false)` --
    // `Site::Anywhere`, with the stray `y` silently discarded and no
    // diagnostic at all, which is the worst outcome this function can
    // produce: a malformed request answered with a different, valid goal
    // instead of an error.
    let has_anchor = !matches!(t.get("x")?, LuaValue::Nil) || !matches!(t.get("y")?, LuaValue::Nil);
    let has_near = !matches!(t.get("near")?, LuaValue::Nil);
    // `anchored` is a RECORDED anchor -- one siting already resolved and the
    // caller pinned -- and it is authoritative where `x`/`y` is only a hint.
    // See `Site::Anchored`'s own doc for why the two cannot be the same field:
    // merging them would either make every stale caller anchor override the
    // ground, or leave persistence inexpressible.
    let has_anchored = !matches!(t.get("anchored")?, LuaValue::Nil);
    // `beside` is "a NEW block, one pitch along from that one" -- the question
    // that had no expression until 2026-09-08. Exclusive with the rest for the
    // same reason they are exclusive with each other: they are four different
    // claims about where the block goes.
    let has_beside = !matches!(t.get("beside")?, LuaValue::Nil);

    // Counted rather than matched as a tuple. The 2x2 match this replaces was
    // exhaustive over two flags; a third makes eight cases of which six are the
    // same error, and writing them out invites exactly the (false, false)
    // fall-through its own comment warns about -- a malformed request answered
    // with a different, valid goal.
    let named = usize::from(has_anchor)
        + usize::from(has_near)
        + usize::from(has_anchored)
        + usize::from(has_beside);
    if named > 1 {
        return Err(goal_error(
            "goal.built: an anchor (x/y), a near hint (near), a recorded anchor \
             (anchored) and a neighbour (beside) are mutually exclusive",
        ));
    }
    if has_beside {
        let b = require_table_field(t.get("beside")?, "beside")?;
        return Ok(Site::Beside {
            of: Position::new(
                require_coordinate(b.get("x")?, "x")?,
                require_coordinate(b.get("y")?, "y")?,
            ),
            // Whole pitches, so these are counts and not coordinates.
            steps: (
                require_coordinate(b.get("dx")?, "dx")? as i32,
                require_coordinate(b.get("dy")?, "dy")? as i32,
            ),
        });
    }
    if has_anchored {
        let a = require_table_field(t.get("anchored")?, "anchored")?;
        return Ok(Site::Anchored(Position::new(
            require_coordinate(a.get("x")?, "x")?,
            require_coordinate(a.get("y")?, "y")?,
        )));
    }
    if has_near {
        let near = require_table_field(t.get("near")?, "near")?;
        return Ok(Site::Near(Position::new(
            require_coordinate(near.get("x")?, "x")?,
            require_coordinate(near.get("y")?, "y")?,
        )));
    }
    if has_anchor {
        return Ok(Site::At(Position::new(
            require_coordinate(t.get("x")?, "x")?,
            require_coordinate(t.get("y")?, "y")?,
        )));
    }
    Ok(Site::Anywhere)
}

fn require_nonempty_string(value: LuaValue, what: &str) -> LuaResult<String> {
    match value {
        LuaValue::String(s) => {
            let s = s.to_string_lossy();
            if s.is_empty() {
                Err(goal_error(format!("{what} must be a non-empty string")))
            } else {
                Ok(s)
            }
        }
        _ => Err(goal_error(format!("{what} must be a non-empty string"))),
    }
}

fn require_count(value: LuaValue) -> LuaResult<u32> {
    match value {
        LuaValue::Integer(n) if n >= 1 => {
            u32::try_from(n).map_err(|_| goal_error(format!("count {n} does not fit a u32")))
        }
        LuaValue::Number(n) if n >= 1.0 && n.fract() == 0.0 => Ok(n as u32),
        _ => Err(goal_error("count must be an integer >= 1")),
    }
}

/// A `{ bot = id }` option, as the [`BotId`] the planner uses.
///
/// Narrowed to `u8` here rather than at `goal.plan`, because that is what a
/// bot id *is*: `BotId` wraps a `u8` and the number is the Factorio player
/// id, so 300 is not a player this world happens to lack -- it is not a
/// player id at all. That makes it a shape error, and the spec puts shape
/// errors on the line that contains them.
fn require_bot(value: LuaValue) -> LuaResult<Option<u8>> {
    let n = match value {
        LuaValue::Nil => return Ok(None),
        LuaValue::Integer(n) if n >= 1 => n,
        LuaValue::Number(n) if n >= 1.0 && n.fract() == 0.0 => n as i64,
        _ => return Err(goal_error("bot must be an integer >= 1")),
    };
    u8::try_from(n)
        .map(Some)
        .map_err(|_| goal_error(format!("bot {n} is not a player id (1-255)")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn lua_with_goal() -> Lua {
        // `clippy.toml` bans `Lua::new` because interpreters that run *user*
        // scripts must come from `sandbox::new_sandboxed_lua`. This state runs
        // nothing but this module's own fixed test snippets, no user input,
        // and it is testing the goal-value constructors in isolation from the
        // sandbox.
        #[allow(clippy::disallowed_methods)]
        let lua = Lua::new();
        let table = lua.create_table().unwrap();
        install_goal_constructors(&lua, &table).unwrap();
        lua.globals().set("goal", table).unwrap();
        lua
    }

    #[test]
    fn have_builds_an_inspectable_table() {
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.have("iron-plate", 5)
            assert(g.kind == "have", "kind")
            assert(g.item == "iron-plate", "item")
            assert(g.count == 5, "count")
            assert(g.bot == nil, "no bot means anyone")
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn have_targets_a_named_bot() {
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.have("coal", 2, { bot = 3 })
            assert(g.bot == 3, "bot")
        "#,
        )
        .exec()
        .expect("script");
    }

    /// `{ via = "recipe" }` names the recipe, and **absence is not a value**:
    /// a goal built without it has no `via` key at all, and converts to
    /// `via: None` -- the shape every script written before this option
    /// existed produces.
    #[test]
    fn have_can_name_the_recipe_that_makes_it() {
        let lua = lua_with_goal();
        let named: LuaTable = lua
            .load(r#"return goal.have("petroleum-gas", 100, { via = "basic-oil-processing" })"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&named).expect("converts"),
            Goal::Have {
                item: "petroleum-gas".into(),
                count: 100,
                whose: Holder::Anyone,
                via: Some("basic-oil-processing".into()),
            }
        );
        assert!(
            named.get::<LuaValue>("via").expect("via").is_string(),
            "the key is set on the table a script can inspect"
        );

        let plain: LuaTable = lua
            .load(r#"return goal.have("iron-plate", 5)"#)
            .eval()
            .expect("script");
        assert!(
            matches!(plain.get::<LuaValue>("via").expect("via"), LuaValue::Nil),
            "no key at all, not an empty one"
        );
        assert_eq!(
            goal_from_lua(&plain).expect("converts"),
            Goal::Have {
                item: "iron-plate".into(),
                count: 5,
                whose: Holder::Anyone,
                via: None,
            }
        );
    }

    /// `tostring` shows the recipe, and shows it the way the planner's own
    /// `Display` does -- `render_goal` is a second, hand-maintained copy and
    /// has already drifted once for this very arm.
    #[test]
    fn a_named_recipe_renders_the_way_the_planner_renders_it() {
        let lua = lua_with_goal();
        let t: LuaTable = lua
            .load(r#"return goal.have("petroleum-gas", 100, { via = "basic-oil-processing" })"#)
            .eval()
            .expect("script");
        assert_eq!(
            render_goal(&t).expect("render_goal"),
            "have 100 petroleum-gas via basic-oil-processing"
        );
        // And the unqualified rendering is untouched, which is what every
        // existing script's `tostring` prints.
        let plain: LuaTable = lua
            .load(r#"return goal.have("iron-plate", 5)"#)
            .eval()
            .expect("script");
        assert_eq!(
            render_goal(&plain).expect("render_goal"),
            "have 5 iron-plate"
        );
    }

    /// An empty `via` is a caller who meant to name a recipe and named
    /// nothing. Refused here, on the line that built the goal, rather than
    /// travelling to the planner as a recipe no world has.
    #[test]
    fn an_empty_via_is_refused_rather_than_treated_as_absent() {
        let lua = lua_with_goal();
        let err = lua
            .load(r#"return goal.have("iron-plate", 5, { via = "" })"#)
            .eval::<LuaTable>()
            .expect_err("an empty recipe name raises");
        assert!(err.to_string().contains("non-empty recipe name"), "{err}");

        let err = lua
            .load(r#"return goal.have("iron-plate", 5, { via = 7 })"#)
            .eval::<LuaTable>()
            .expect_err("a non-string recipe raises");
        assert!(err.to_string().contains("recipe name string"), "{err}");
    }

    #[test]
    fn shape_errors_raise_at_construction() {
        let lua = lua_with_goal();
        for (src, want) in [
            (r#"goal.have("iron-plate", 0)"#, "count"),
            (r#"goal.have("iron-plate", -1)"#, "count"),
            (r#"goal.have("", 1)"#, "item"),
            (r#"goal.researched("")"#, "technology"),
            (r#"goal.producing("", 15)"#, "item"),
            // The integer rule, from the Lua side: the planner's own
            // `Goal::Producing` carries a `u32` so that the machine count is
            // integer arithmetic end to end, and a rate arriving as 15.5
            // would put a float back on that path.
            (r#"goal.producing("iron-plate", 0)"#, "count"),
            (r#"goal.producing("iron-plate", 15.5)"#, "count"),
            (r#"goal.built("", {x = 1, y = 1})"#, "blueprint"),
            // `nil` and an omitted argument are the same call from Lua's
            // side, so both mean `Site::Anywhere` -- this is not a shape
            // error, unlike every other case in this list.
            (r#"goal.built("0eNq...", {x = 1})"#, "y"),
            (r#"goal.built("0eNq...", 5)"#, "table"),
            (r#"goal.built("0eNq...", {near = 5})"#, "near"),
            (
                r#"goal.built("0eNq...", {x = 1, y = 1, near = {x = 2, y = 2}})"#,
                "mutually exclusive",
            ),
            // The third site kind is exclusive with BOTH others, and the
            // check counts rather than enumerating pairs -- a tuple match
            // over three flags is eight cases of which six are this error,
            // and the one that gets forgotten falls through to `Anywhere`,
            // answering a malformed request with a different valid goal.
            (
                r#"goal.built("0eNq...", {x = 1, y = 1, anchored = {x = 2, y = 2}})"#,
                "mutually exclusive",
            ),
            (
                r#"goal.built("0eNq...", {near = {x = 1, y = 1}, anchored = {x = 2, y = 2}})"#,
                "mutually exclusive",
            ),
            (r#"goal.built("0eNq...", {anchored = 5})"#, "anchored"),
            (r#"goal.built("0eNq...", {anchored = {x = 1}})"#, "y"),
            (r#"goal.all({})"#, "at least one"),
            (r#"goal.all({ 42 })"#, "goal"),
            (r#"goal.have("iron-plate", 1, { bot = 0 })"#, "bot"),
            // `BotId` is a `u8` and *is* the Factorio player id, so a bot
            // outside 1..=255 is a shape error, not a "no such player"
            // semantic one -- there is no world in which it could be right.
            (r#"goal.have("iron-plate", 1, { bot = 300 })"#, "player id"),
            // An unrecognised `kind` is a shape error too: `goal.all` is the
            // one constructor that accepts a caller-built table, and the set
            // of kinds is fixed by this module, not by the world.
            (r#"goal.all({ { kind = "bogus" } })"#, "bogus"),
        ] {
            let err = lua.load(src).exec().expect_err(src).to_string();
            assert!(err.contains(want), "{src}: {err} lacks {want}");
        }
    }

    /// **A recorded anchor survives the Lua round trip and is not confused
    /// with a caller's hint.**
    ///
    /// `goal.built` writes the site back into the table it returns, and
    /// `site_from_table` reads that same table — so the constructor and the
    /// reader must agree on which field means which claim. `x`/`y` is a hint
    /// the ground may override; `anchored` is an anchor siting already
    /// resolved, which the ground must NOT override. Writing a recorded
    /// anchor into `x`/`y` would silently demote it to a hint and restore the
    /// crosstalk defect it exists to fix, with nothing failing.
    #[test]
    fn a_recorded_anchor_round_trips_and_stays_distinct_from_a_hint() {
        let lua = lua_with_goal();
        // Rendered through `tostring`, which is the metatable's own
        // `__tostring` and therefore the same path a script or a plan log
        // takes -- not a second formatter that could agree with the
        // constructor while the real one does not.
        let describe = |src: &str| -> String {
            lua.load(format!("return tostring({src})"))
                .eval::<String>()
                .expect(src)
        };

        // Round trip: the constructor's own output re-read by `describe`.
        let anchored = describe(r#"goal.built("0eNq...", {anchored = {x = 3, y = 4}})"#);
        assert!(
            anchored.contains("recorded anchor"),
            "a recorded anchor must render as one: {anchored}"
        );
        assert!(
            anchored.contains('3') && anchored.contains('4'),
            "and must carry its coordinates: {anchored}"
        );

        // The same coordinates as a plain hint must render differently, or
        // the two claims are indistinguishable to anyone reading a plan.
        let hinted = describe(r#"goal.built("0eNq...", {x = 3, y = 4})"#);
        assert!(
            !hinted.contains("recorded"),
            "a caller hint is not a recorded anchor: {hinted}"
        );
        assert_ne!(
            anchored, hinted,
            "a recorded anchor and a hint at the same tile must not describe \
             identically -- they carry different authority over the ground"
        );
    }

    #[test]
    fn goals_nest_and_render() {
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.all { goal.have("iron-plate", 5), goal.researched("automation") }
            assert(g.kind == "all", "kind")
            assert(#g.goals == 2, "two sub-goals")
            assert(tostring(g):find("automation"), "tostring mentions the technology")
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn conversion_maps_every_shape_to_the_planner() {
        let lua = lua_with_goal();
        let g: LuaTable = lua
            .load(
                r#"
            return goal.all {
                goal.have("iron-plate", 5),
                goal.have("coal", 2, { bot = 3 }),
                goal.researched("automation"),
                goal.producing("iron-plate", 15),
            }
        "#,
            )
            .eval()
            .expect("script");
        let converted = goal_from_lua(&g).expect("converts");
        assert_eq!(
            converted,
            Goal::All(vec![
                Goal::Have {
                    item: "iron-plate".into(),
                    count: 5,
                    whose: Holder::Anyone,
                    via: None,
                },
                Goal::Have {
                    item: "coal".into(),
                    count: 2,
                    whose: Holder::Bot(BotId(3)),
                    via: None,
                },
                Goal::Researched("automation".into()),
                Goal::Producing {
                    item: "iron-plate".into(),
                    per_minute: 15,
                },
            ])
        );
    }

    #[test]
    fn producing_builds_an_inspectable_table_and_renders_its_rate() {
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.producing("iron-plate", 15)
            assert(g.kind == "producing", "kind")
            assert(g.item == "iron-plate", "item")
            assert(g.per_minute == 15, "per_minute, not rate")
            assert(tostring(g) == "producing 15 iron-plate/min", tostring(g))
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn built_builds_an_inspectable_table_and_renders_its_anchor() {
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.built("0eNq...", {x = 10, y = 10})
            assert(g.kind == "built", "kind")
            assert(g.blueprint == "0eNq...", "blueprint")
            assert(g.x == 10, "x")
            assert(g.y == 10, "y")
            assert(tostring(g):find("10"), "tostring mentions the anchor")
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn built_converts_to_the_planner_goal() {
        let lua = lua_with_goal();
        let g: LuaTable = lua
            .load(r#"return goal.built("0eNq...", {x = 10, y = 10})"#)
            .eval()
            .expect("script");
        let converted = goal_from_lua(&g).expect("converts");
        assert_eq!(
            converted,
            Goal::Built {
                blueprint: "0eNq...".into(),
                site: Site::At(Position::new(10.0, 10.0)),
            }
        );
    }

    #[test]
    fn built_accepts_a_position_a_near_hint_or_neither() {
        // The three forms must be *distinguished*, not just each individually
        // producing a plausible-looking result: a constructor that answered
        // `Site::Anywhere` for every input would still pass three separate
        // "does this look right" assertions, so each case here asserts the
        // exact variant a wrong implementation could not produce for all
        // three at once.
        let lua = lua_with_goal();

        let at: LuaTable = lua
            .load(r#"return goal.built("0eNq...", {x = 3, y = 4})"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&at).expect("converts"),
            Goal::Built {
                blueprint: "0eNq...".into(),
                site: Site::At(Position::new(3.0, 4.0)),
            },
            "an explicit x/y must produce Site::At"
        );

        let near: LuaTable = lua
            .load(r#"return goal.built("0eNq...", {near = {x = 3, y = 4}})"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&near).expect("converts"),
            Goal::Built {
                blueprint: "0eNq...".into(),
                site: Site::Near(Position::new(3.0, 4.0)),
            },
            "a near hint must produce Site::Near"
        );

        let anywhere: LuaTable = lua
            .load(r#"return goal.built("0eNq...")"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&anywhere).expect("converts"),
            Goal::Built {
                blueprint: "0eNq...".into(),
                site: Site::Anywhere,
            },
            "no second argument at all must produce Site::Anywhere"
        );

        // `tostring` must agree with what was actually built -- this is the
        // check that would have caught `render_goal` staying a hand-written
        // copy that assumed every "built" table was `Site::At`.
        let tostring: LuaFunction = lua.globals().get("tostring").expect("tostring exists");
        assert!(
            tostring
                .call::<String>(at)
                .expect("tostring")
                .contains("at "),
            "an anchored goal renders \"at\""
        );
        assert!(
            tostring
                .call::<String>(near)
                .expect("tostring")
                .contains("near "),
            "a near-sited goal renders \"near\", not \"at\""
        );
        assert!(
            tostring
                .call::<String>(anywhere)
                .expect("tostring")
                .contains("anywhere"),
            "an unsited goal renders \"anywhere\", not \"at\""
        );
    }

    #[test]
    fn built_refuses_an_anchor_missing_either_coordinate() {
        // A table naming only one of `x`/`y` is anchor-shaped, and the whole
        // point of committing to the anchor branch on *either* key is that
        // `{y = 1}` must not fall through the gap between "has an anchor"
        // and "has a near hint" into `Site::Anywhere` -- silently discarding
        // the stray `y` and answering with a different, valid goal instead
        // of an error. Both directions are checked, and the message is
        // checked, not just that one was raised: a caller needs to be told
        // which coordinate is missing.
        let lua = lua_with_goal();

        let err = lua
            .load(r#"goal.built("0eNq...", {x = 1})"#)
            .exec()
            .expect_err("missing y")
            .to_string();
        assert!(
            err.contains("anchor y must be a number"),
            "{err} does not name the missing y"
        );

        let err = lua
            .load(r#"goal.built("0eNq...", {y = 1})"#)
            .exec()
            .expect_err("missing x")
            .to_string();
        assert!(
            err.contains("anchor x must be a number"),
            "{err} does not name the missing x"
        );
    }

    #[test]
    fn built_render_goal_agrees_with_the_planner_goals_own_display() {
        // Scoped to `built` only. `render_goal` is a second, hand-maintained
        // copy of `Goal::Display`, and the two have already drifted once for
        // a different kind: `render_goal`'s `"have"` arm omits the
        // `Holder`/`whose` field that `Goal::Display` includes, so
        // `tostring` tells a script author something different from what
        // the planner's own error messages say. This pins agreement for the
        // `"built"` arm this task touched, and only that arm -- widening it
        // to `"have"` would fail immediately on a defect nobody on this
        // branch owns.
        let lua = lua_with_goal();
        for src in [
            r#"return goal.built("0eNq...", {x = 3, y = 4})"#,
            r#"return goal.built("0eNq...", {near = {x = 3, y = 4}})"#,
            r#"return goal.built("0eNq...")"#,
        ] {
            let t: LuaTable = lua.load(src).eval().expect(src);
            let rendered = render_goal(&t).expect("render_goal");
            let goal = goal_from_lua(&t).expect("goal_from_lua");
            assert_eq!(
                rendered,
                goal.to_string(),
                "{src}: render_goal and Goal::Display disagree"
            );
        }
    }

    /// The standing goal's Lua shape, and the window it refuses to guess.
    #[test]
    fn sustain_builds_an_inspectable_table_and_names_its_window() {
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.sustain("iron-plate", 15, 7200)
            assert(g.kind == "sustain", "kind")
            assert(g.item == "iron-plate", "item")
            assert(g.per_minute == 15, "per_minute")
            assert(g.window_ticks == 7200, "window_ticks, in ticks")
            assert(tostring(g) == "sustain 15 iron-plate/min over 7200 ticks", tostring(g))
        "#,
        )
        .exec()
        .expect("script");
    }

    /// No default window, on either side of the boundary.
    ///
    /// `supervisor.witness`'s `within_ticks` refuses one for the same reason:
    /// the window decides what a failure means. A constructor that filled one
    /// in would hand back a verdict nobody derived.
    #[test]
    fn sustain_refuses_a_goal_with_no_window() {
        let lua = lua_with_goal();
        assert!(
            lua.load(r#"goal.sustain("iron-plate", 15)"#)
                .exec()
                .is_err(),
            "a window is required at construction"
        );
        let t: LuaTable = lua
            .load(r#"return { kind = "sustain", item = "iron-plate", per_minute = 15 }"#)
            .eval()
            .expect("table");
        assert!(
            goal_from_lua(&t).is_err(),
            "and again on the way back, because a Lua table is open"
        );
    }

    #[test]
    fn sustain_converts_to_the_planner_goal() {
        let lua = lua_with_goal();
        let g: LuaTable = lua
            .load(r#"return goal.sustain("iron-plate", 15, 7200)"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&g).expect("converts"),
            Goal::Sustain {
                item: "iron-plate".into(),
                per_minute: 15,
                window_ticks: 7200,
            }
        );
    }

    /// `render_goal` is a second, hand-maintained copy of `Goal::Display`, and
    /// the two have already drifted for `have` and `producing`. This arm is
    /// written to agree and is pinned so, so a diagnostic a script prints and
    /// one the planner prints name the same goal.
    #[test]
    fn sustain_render_goal_agrees_with_the_planner_goals_own_display() {
        let lua = lua_with_goal();
        let t: LuaTable = lua
            .load(r#"return goal.sustain("iron-plate", 15, 7200)"#)
            .eval()
            .expect("script");
        assert_eq!(
            render_goal(&t).expect("render_goal"),
            goal_from_lua(&t).expect("goal_from_lua").to_string()
        );
    }

    /// And it is in `KINDS`, so it may sit inside a bundle.
    #[test]
    fn a_sustain_goal_may_sit_inside_goal_all() {
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.all { goal.sustain("iron-plate", 15, 7200) }
            assert(#g.goals == 1, "one sub-goal")
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn a_charted_goal_may_sit_inside_goal_all() {
        // Regression: `charted` was handled by the parser and the renderer but
        // missing from `KINDS`, so this was the ONE place a charted goal was
        // rejected — `goal.charted(...)` worked standalone and failed only
        // when nested. Found while adding `sustain`, not by using it.
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.all { goal.charted(0, 0, 256) }
            assert(#g.goals == 1, "one sub-goal")
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn a_producing_goal_may_sit_inside_goal_all() {
        // `goal.all` is the one constructor that takes a caller-built table
        // and checks the `kind` itself, so a new kind that was not added to
        // `KINDS` would be rejected here and nowhere else.
        let lua = lua_with_goal();
        lua.load(
            r#"
            local g = goal.all { goal.producing("iron-plate", 15) }
            assert(#g.goals == 1, "one sub-goal")
        "#,
        )
        .exec()
        .expect("script");
    }

    /// `goal.produced` is not `goal.have`: it never subtracts what a bot
    /// already holds, and it carries the recipe the owner ruled the goal
    /// should name.
    #[test]
    fn produced_names_its_recipe_and_converts() {
        let lua = lua_with_goal();
        let t: LuaTable = lua
            .load(r#"return goal.produced("petroleum-gas", 45, { via = "basic-oil-processing" })"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&t).expect("converts"),
            Goal::Produced {
                item: "petroleum-gas".into(),
                count: 45,
                whose: Holder::Anyone,
                unlocks: None,
                via: Some("basic-oil-processing".into()),
            }
        );
    }

    /// Absence is not a value: no options at all is the shape every goal had
    /// before `via`/`unlocks` existed, and it must convert to `None` for both
    /// rather than to an empty choice.
    #[test]
    fn produced_without_options_chooses_nothing() {
        let lua = lua_with_goal();
        let t: LuaTable = lua
            .load(r#"return goal.produced("iron-plate", 5)"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&t).expect("converts"),
            Goal::Produced {
                item: "iron-plate".into(),
                count: 5,
                whose: Holder::Anyone,
                unlocks: None,
                via: None,
            }
        );
    }

    #[test]
    fn produced_targets_a_named_bot_and_an_unlock() {
        let lua = lua_with_goal();
        let t: LuaTable = lua
            .load(r#"return goal.produced("lab", 1, { bot = 2, unlocks = "automation-science-pack" })"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&t).expect("converts"),
            Goal::Produced {
                item: "lab".into(),
                count: 1,
                whose: Holder::Bot(BotId(2)),
                unlocks: Some("automation-science-pack".into()),
                via: None,
            }
        );
    }

    /// An `unlocks` a caller *meant* to name and left empty is a mistake, and
    /// `Some("")` would reach the planner as a technology that does not exist.
    #[test]
    fn an_empty_unlocks_is_refused_on_the_line_that_wrote_it() {
        let lua = lua_with_goal();
        let err = lua
            .load(r#"return goal.gathered("crude-oil", { unlocks = "" })"#)
            .eval::<LuaTable>()
            .expect_err("empty unlocks")
            .to_string();
        assert!(err.contains("unlocks"), "{err}");
    }

    /// The two oil rungs take an **entity**, and neither takes a count: what
    /// comes out of a well is a fluid no inventory can hold.
    #[test]
    fn gathered_and_extracted_carry_an_entity() {
        let lua = lua_with_goal();
        let gathered: LuaTable = lua
            .load(r#"return goal.gathered("crude-oil")"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&gathered).expect("converts"),
            Goal::Gathered {
                entity: "crude-oil".into(),
                unlocks: None,
            }
        );
        let extracted: LuaTable = lua
            .load(r#"return goal.extracted("crude-oil", { unlocks = "oil-processing" })"#)
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&extracted).expect("converts"),
            Goal::Extracted {
                entity: "crude-oil".into(),
                unlocks: Some("oil-processing".into()),
            }
        );
    }

    #[test]
    fn an_empty_entity_name_is_refused() {
        let lua = lua_with_goal();
        let err = lua
            .load(r#"return goal.extracted("")"#)
            .eval::<LuaTable>()
            .expect_err("empty entity")
            .to_string();
        assert!(err.contains("entity"), "{err}");
    }

    /// `__tostring` and `Goal::Display` must agree for every kind added since
    /// the rule was written -- `have` and `producing` predate it and are
    /// deliberately not covered.
    #[test]
    fn the_new_kinds_render_the_way_the_planner_renders_them() {
        let lua = lua_with_goal();
        for src in [
            r#"return goal.produced("petroleum-gas", 45, { via = "basic-oil-processing" })"#,
            r#"return goal.produced("lab", 1, { unlocks = "automation-science-pack" })"#,
            r#"return goal.gathered("crude-oil")"#,
            r#"return goal.extracted("crude-oil", { unlocks = "oil-processing" })"#,
        ] {
            let t: LuaTable = lua.load(src).eval().expect("script");
            assert_eq!(
                render_goal(&t).expect("render_goal"),
                goal_from_lua(&t).expect("goal_from_lua").to_string(),
                "{src}"
            );
        }
    }

    /// **The seam that broke last time.** `goal.all` is the only construct
    /// that consults `KINDS`, so a kind wired into the constructor, the parser
    /// and the renderer but not into that list works standalone and fails
    /// here and nowhere else. One case per new kind, and the bundle is
    /// converted rather than only built, so the parser's own `all` arm is
    /// crossed too.
    #[test]
    fn every_new_kind_composes_inside_goal_all() {
        let lua = lua_with_goal();
        let bundle: LuaTable = lua
            .load(
                r#"
                return goal.all {
                    goal.gathered("crude-oil"),
                    goal.extracted("crude-oil"),
                    goal.produced("petroleum-gas", 45, { via = "basic-oil-processing" }),
                }
            "#,
            )
            .eval()
            .expect("script");
        assert_eq!(
            goal_from_lua(&bundle).expect("converts"),
            Goal::All(vec![
                Goal::Gathered {
                    entity: "crude-oil".into(),
                    unlocks: None,
                },
                Goal::Extracted {
                    entity: "crude-oil".into(),
                    unlocks: None,
                },
                Goal::Produced {
                    item: "petroleum-gas".into(),
                    count: 45,
                    whose: Holder::Anyone,
                    unlocks: None,
                    via: Some("basic-oil-processing".into()),
                },
            ])
        );
    }

    /// The oil milestone's own composition, as `scripts/oil_milestone.lua`
    /// states it. A goal a script cannot say is a milestone that cannot be
    /// reached from the only path that can satisfy it.
    #[test]
    fn the_oil_milestone_goal_is_expressible_from_lua() {
        let lua = lua_with_goal();
        let g: LuaTable = lua
            .load(
                r#"
                return goal.all {
                    goal.gathered("crude-oil"),
                    goal.produced("petroleum-gas", 45, { via = "basic-oil-processing" }),
                }
            "#,
            )
            .eval()
            .expect("script");
        // Asserted as an exact `Goal`, not as a shape, because this value has
        // a counterpart outside the process: it is byte-for-byte the goal
        // `factorio-bot plan --goal-json '{"All":[{"Gathered":{"entity":
        // "crude-oil"}},{"Produced":{"item":"petroleum-gas","count":45,
        // "whose":"Anyone","via":"basic-oil-processing"}}]}'` carries, which
        // is how the composition was confirmed to reach the planner against a
        // real world dump. A looser assertion here would let the two drift.
        assert_eq!(
            goal_from_lua(&g).expect("converts"),
            Goal::All(vec![
                Goal::Gathered {
                    entity: "crude-oil".into(),
                    unlocks: None,
                },
                Goal::Produced {
                    item: "petroleum-gas".into(),
                    count: 45,
                    whose: Holder::Anyone,
                    unlocks: None,
                    via: Some("basic-oil-processing".into()),
                },
            ])
        );
    }

    #[test]
    fn conversion_refuses_a_hand_built_table() {
        let lua = lua_with_goal();
        let t: LuaTable = lua
            .load(r#"return { kind = "have", item = "iron-plate" }"#)
            .eval()
            .expect("script");
        let err = goal_from_lua(&t).expect_err("no count").to_string();
        assert!(err.contains("count"), "{err}");
    }
}
