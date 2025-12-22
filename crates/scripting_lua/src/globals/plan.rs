use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::graph::task_graph::TaskGraph;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::parking_lot::RwLock;
use factorio_bot_core::plan::plan_builder::PlanBuilder;
use factorio_bot_core::types::{Direction, FactorioEntity, PlayerId, Position, PositionRadius};
use std::sync::Arc;

pub fn create_lua_plan_builder(
    lua: &Lua,
    graph: Arc<RwLock<TaskGraph>>,
    world: Arc<FactorioWorld>,
) -> LuaResult<LuaTable> {
    let map_table = lua.create_table()?;
    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Plan Builder
-- Internally holds a graph of Task Nodes which can be grown by using the methods.  
-- @module plan

local plan = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return plan"#))?;

    let _graph = graph.clone();
    let _world = world.clone();
    let _plan_builder = Arc::new(PlanBuilder::new(graph, world));

    let plan_builder = _plan_builder.clone();
    map_table.set(
        "__doc_entry_mine",
        String::from(
            r#"
--- adds a MINE node to graph
-- If required first a WALK node is inserted to walk near the item to mine 
-- @number player_id id of player
-- @param position `types.Position`
-- @string name name of item to mine
-- @number count how many items to mine
function plan.mine(player_id, position, name, count)
end
"#,
        ),
    )?;
    map_table.set(
        "mine",
        lua.create_function(
            move |_lua, (player_id, position, name, count): (PlayerId, LuaTable, String, u32)| {
                plan_builder
                    .mine(
                        player_id,
                        Position::new(position.get("x").unwrap(), position.get("y").unwrap()),
                        name.as_str(),
                        count,
                    )
                    .unwrap();
                Ok(())
            },
        )?,
    )?;
    let plan_builder = _plan_builder.clone();
    // let world = _world.clone();
    let world = _world;
    map_table.set(
        "__doc_entry_place",
        String::from(
            r#"
--- adds a PLACE node to graph
-- If required first a WALK node is inserted to walk near the item to place 
-- @number player_id id of player
-- @string entity_name name of item to place
-- @param position `types.Position` 
-- @param[opt] direction `types.Direction` 
-- @return `types.FactorioEntity`
function plan.place(player_id, entity_name, position, direction)
end
"#,
        ),
    )?;
    map_table.set(
        "place",
        lua.create_function(
            move |_lua,
                  (player_id, entity_name, position, direction): (
                PlayerId,
                String,
                LuaTable,
                Option<u8>,
            )| {
                let entity = FactorioEntity::from_prototype(
                    &entity_name,
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap()),
                    direction.map(|d| Direction::from_u8(d).expect("invalid direction")),
                    None,
                    None,
                    world.entity_prototypes.clone(),
                )
                .expect("failed to build entity");
                let entity = plan_builder.add_place(player_id, entity).unwrap();
                Ok(entity)
            },
        )?,
    )?;
    map_table.set(
        "__doc_entry_walk",
        String::from(
            r#"
--- adds a WALK node to graph
-- @number player_id id of player
-- @param position `types.Position` 
-- @number radius how close to walk to
function plan.walk(player_id, position, radius)
end
"#,
        ),
    )?;
    let plan_builder = _plan_builder.clone();
    map_table.set(
        "walk",
        lua.create_function(
            move |_lua, (player_id, position, radius): (PlayerId, LuaTable, f64)| {
                plan_builder
                    .add_walk(
                        player_id,
                        PositionRadius::new(
                            position.get("x").unwrap(),
                            position.get("y").unwrap(),
                            radius,
                        ),
                    )
                    .unwrap();
                Ok(())
            },
        )?,
    )?;
    let graph = _graph.clone();
    map_table.set(
        "__doc_entry_task_graph_graphviz",
        String::from(
            r#"
--- build graphviz from task graph
--@return string graphviz string
function plan.task_graph_graphviz()
end
"#,
        ),
    )?;
    map_table.set(
        "task_graph_graphviz",
        lua.create_function(move |_lua, ()| {
            let graph = graph.read();
            Ok(graph.graphviz_dot())
        })?,
    )?;
    let graph = _graph;
    map_table.set(
        "__doc_entry_task_graph_mermaid_gantt",
        String::from(
            r#"
--- build mermaid gantt from task graph
--@return string mermaid string
function plan.task_graph_mermaid_gantt()
end
"#,
        ),
    )?;
    map_table.set(
        "task_graph_mermaid_gantt",
        lua.create_function(move |_lua, (bot_ids, title): (Vec<u8>, String)| {
            let graph = graph.read();
            Ok(graph.mermaid_gantt(bot_ids, &title))
        })?,
    )?;
    let plan_builder = _plan_builder.clone();
    map_table.set(
        "__doc_entry_group_start",
        String::from(
            r#"
--- adds a GROUP START node to graph
-- Groups are used to synchronize bots so their tasks are completed before the next group is started.
-- Should be closed with `group_end`. 
-- @string label label of group
function plan.group_start(label)
end
"#,
        ),
    )?;
    map_table.set(
        "group_start",
        lua.create_function(move |_lua, label: String| {
            plan_builder.group_start(label.as_str());
            Ok(())
        })?,
    )?;
    let plan_builder = _plan_builder;
    map_table.set(
        "__doc_entry_group_end",
        String::from(
            r#"
--- adds a GROUP END node to graph
function plan.group_end()
end
"#,
        ),
    )?;
    map_table.set(
        "group_end",
        lua.create_function(move |_lua, ()| {
            plan_builder.group_end();
            Ok(())
        })?,
    )?;
    Ok(map_table)
}
