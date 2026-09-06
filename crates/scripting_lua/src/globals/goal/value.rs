//! `goal.*` values: Lua tables that describe a `Goal` without holding a
//! handle into the interpreter or the running game.
//!
//! Every constructor (`goal.have`, `goal.researched`, `goal.producing`,
//! `goal.built`, `goal.all`) validates
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

/// Installs `have`, `researched`, `producing`, `built` and `all` on `table`.
///
/// All five share one metatable -- built once here and cloned (cheaply: a
/// Lua table is refcounted) onto every value the five functions return -- so
/// `tostring(g)` renders the same way regardless of which of them built `g`.
///
/// Called by `create_lua_goal_with`: these five *are* `goal.have`,
/// `goal.researched`, `goal.producing`, `goal.built` and `goal.all` as a
/// script sees them.
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
                    Site::Anywhere => {}
                }
                t.set_metatable(Some(mt.clone()))?;
                Ok(t)
            },
        )?,
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
            Ok(Goal::Have { item, count, whose })
        }
        "researched" => Ok(Goal::Researched(require_technology(
            value.get("technology")?,
        )?)),
        "producing" => Ok(Goal::Producing {
            item: require_item(value.get("item")?)?,
            per_minute: require_count(value.get("per_minute")?)?,
        }),
        "built" => Ok(Goal::Built {
            blueprint: require_blueprint(value.get("blueprint")?)?,
            site: site_from_table(value)?,
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
            Ok(format!("have {count} {item}"))
        }
        "researched" => Ok(format!(
            "researched {}",
            require_technology(t.get("technology")?)?
        )),
        "producing" => Ok(format!(
            "producing {} {}/min",
            require_count(t.get("per_minute")?)?,
            require_item(t.get("item")?)?
        )),
        "built" => {
            let blueprint = require_blueprint(t.get("blueprint")?)?;
            let where_ = match site_from_table(t)? {
                Site::At(pos) => format!("at {pos}"),
                Site::Near(pos) => format!("near {pos}"),
                Site::Anywhere => "anywhere".to_string(),
            };
            Ok(format!("build {}-byte block {}", blueprint.len(), where_))
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

/// The five kinds a goal table may name. Fixed by this module -- no world is
/// consulted to decide whether a `kind` is one of them, which is why an
/// unknown one is a shape error rather than a semantic one.
const KINDS: &[&str] = &["have", "researched", "producing", "built", "all"];

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
    match (has_anchor, has_near) {
        (true, true) => Err(goal_error(
            "goal.built: an anchor (x/y) and a near hint (near) are mutually exclusive",
        )),
        (true, false) => Ok(Site::At(Position::new(
            require_coordinate(t.get("x")?, "x")?,
            require_coordinate(t.get("y")?, "y")?,
        ))),
        (false, true) => {
            let near = require_table_field(t.get("near")?, "near")?;
            Ok(Site::Near(Position::new(
                require_coordinate(near.get("x")?, "x")?,
                require_coordinate(near.get("y")?, "y")?,
            )))
        }
        (false, false) => Ok(Site::Anywhere),
    }
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
                    whose: Holder::Anyone
                },
                Goal::Have {
                    item: "coal".into(),
                    count: 2,
                    whose: Holder::Bot(BotId(3))
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
