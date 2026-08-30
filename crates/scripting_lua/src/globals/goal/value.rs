//! `goal.*` values: Lua tables that describe a `Goal` without holding a
//! handle into the interpreter or the running game.
//!
//! Every constructor (`goal.have`, `goal.researched`, `goal.all`) validates
//! eagerly, so a mistake raises on the line that made it. [`goal_from_lua`]
//! validates again on the way back to a planner `Goal`, because a Lua table
//! is open: nothing stops a script from hand-building one that skips what a
//! constructor would have demanded. Both paths raise through the same
//! [`goal_error`], so a script sees one error shape regardless of which side
//! caught the mistake.

use super::goal_error;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_planner::{BotId, Goal, Holder};

/// Installs `have`, `researched` and `all` on `table`.
///
/// All three share one metatable -- built once here and cloned (cheaply: a
/// Lua table is refcounted) onto every value the three functions return -- so
/// `tostring(g)` renders the same way regardless of which of them built `g`.
///
/// Called by `create_lua_goal_with`: these three *are* `goal.have`,
/// `goal.researched` and `goal.all` as a script sees them.
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
                let bot = match &opts {
                    Some(opts) => require_bot(opts.get("bot")?)?,
                    None => None,
                };
                let t = lua.create_table()?;
                t.set("kind", "have")?;
                t.set("item", item)?;
                t.set("count", count)?;
                if let Some(bot) = bot {
                    t.set("bot", bot)?;
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

/// Converts a goal table -- built by `goal.have`/`goal.researched`/
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
                Some(bot) => Holder::Bot(BotId(u8::try_from(bot).map_err(|_| {
                    goal_error(format!("bot {bot} does not fit a player id (0-255)"))
                })?)),
                None => Holder::Anyone,
            };
            Ok(Goal::Have { item, count, whose })
        }
        "researched" => Ok(Goal::Researched(require_technology(
            value.get("technology")?,
        )?)),
        "all" => {
            let goals = require_table_field(value.get("goals")?, "goals")?;
            let len = goals.raw_len();
            let mut out = Vec::with_capacity(len);
            for i in 1..=len {
                out.push(goal_from_lua(&goals.get::<LuaTable>(i)?)?);
            }
            Ok(Goal::All(out))
        }
        other => Err(goal_error(format!("unknown goal kind \"{other}\""))),
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
            Ok(format!("have {count} {item}"))
        }
        "researched" => Ok(format!(
            "researched {}",
            require_technology(t.get("technology")?)?
        )),
        "all" => {
            let goals = require_table_field(t.get("goals")?, "goals")?;
            let len = goals.raw_len();
            let mut parts = Vec::with_capacity(len);
            for i in 1..=len {
                parts.push(render_goal(&goals.get::<LuaTable>(i)?)?);
            }
            Ok(format!("all {{ {} }}", parts.join(", ")))
        }
        other => Err(goal_error(format!("unknown goal kind \"{other}\""))),
    }
}

/// The `kind` field every goal table carries, as a plain `String`.
fn require_kind(t: &LuaTable) -> LuaResult<String> {
    match t.get("kind")? {
        LuaValue::String(s) => Ok(s.to_string_lossy()),
        _ => Err(goal_error("goal table is missing a string \"kind\" field")),
    }
}

/// A `goal.all` element: must be a table carrying a `kind`, i.e. something a
/// constructor (or an equally careful hand-built table) produced.
fn require_goal_table(value: LuaValue) -> LuaResult<LuaTable> {
    match value {
        LuaValue::Table(t) => {
            require_kind(&t)?;
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

fn require_technology(value: LuaValue) -> LuaResult<String> {
    require_nonempty_string(value, "technology")
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

fn require_bot(value: LuaValue) -> LuaResult<Option<i64>> {
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::Integer(n) if n >= 1 => Ok(Some(n)),
        LuaValue::Number(n) if n >= 1.0 && n.fract() == 0.0 => Ok(Some(n as i64)),
        _ => Err(goal_error("bot must be an integer >= 1")),
    }
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

    #[test]
    fn shape_errors_raise_at_construction() {
        let lua = lua_with_goal();
        for (src, want) in [
            (r#"goal.have("iron-plate", 0)"#, "count"),
            (r#"goal.have("iron-plate", -1)"#, "count"),
            (r#"goal.have("", 1)"#, "item"),
            (r#"goal.researched("")"#, "technology"),
            (r#"goal.all({})"#, "at least one"),
            (r#"goal.all({ 42 })"#, "goal"),
            (r#"goal.have("iron-plate", 1, { bot = 0 })"#, "bot"),
        ] {
            let err = lua.load(src).exec().expect_err(src).to_string();
            assert!(err.contains(want), "{src}: {err} lacks {want}");
        }
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
                    whose: Holder::Anyone
                },
                Goal::Have {
                    item: "coal".into(),
                    count: 2,
                    whose: Holder::Bot(BotId(3))
                },
                Goal::Researched("automation".into()),
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
