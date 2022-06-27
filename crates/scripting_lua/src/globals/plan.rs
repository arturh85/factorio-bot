use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::graph::task_graph::{PositionRadius, TaskGraph};
use factorio_bot_core::parking_lot::RwLock;
use factorio_bot_core::plan::plan_builder::PlanBuilder;
use factorio_bot_core::rlua;
use factorio_bot_core::rlua::{Context, Table};
use factorio_bot_core::types::{FactorioEntity, PlayerId, Position};
use std::sync::Arc;

pub fn create_lua_plan_builder(
    ctx: Context,
    graph: Arc<RwLock<TaskGraph>>,
    world: Arc<FactorioWorld>,
) -> rlua::Result<Table> {
    let map_table = ctx.create_table()?;
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
        "__doc_fn_mine",
        String::from(
            r#"
--- adds a MINE node to graph
-- If required first a WALK node is inserted to walk near the item to mine 
-- @int player_id id of player
-- @param position x/y position table
-- @string name name of item to mine
-- @int count how many items to mine
function plan.mine(player_id, position, name, count)
end
"#,
        ),
    )?;
    map_table.set(
        "mine",
        ctx.create_function(
            move |_ctx, (player_id, position, name, count): (PlayerId, Table, String, u32)| {
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
        "__doc_fn_place",
        String::from(
            r#"
--- adds a PLACE node to graph
-- If required first a WALK node is inserted to walk near the item to place 
-- @int player_id id of player
-- @param position x/y position table 
-- @string name name of item to place
-- @return FactorioEntity table
function plan.place(player_id, position, name)
end
"#,
        ),
    )?;
    map_table.set(
        "place",
        ctx.create_function(
            move |_ctx, (player_id, position, name): (PlayerId, Table, String)| {
                let entity = FactorioEntity::from_prototype(
                    &name,
                    Position::new(position.get("x").unwrap(), position.get("y").unwrap()),
                    None,
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
        "__doc_fn_walk",
        String::from(
            r#"
--- adds a WALK node to graph
-- @int player_id id of player
-- @param position x/y position table 
-- @int radius how close to walk to
function plan.walk(player_id, position, radius)
end
"#,
        ),
    )?;
    let plan_builder = _plan_builder.clone();
    map_table.set(
        "walk",
        ctx.create_function(
            move |_ctx, (player_id, position, radius): (PlayerId, Table, f64)| {
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
        "__doc_fn_task_graph_graphviz",
        String::from(
            r#"
--- build graphviz from task graph
function plan.task_graph_graphviz()
end
"#,
        ),
    )?;
    map_table.set(
        "task_graph_graphviz",
        ctx.create_function(move |_ctx, ()| {
            let graph = graph.read();
            Ok(graph.graphviz_dot())
        })?,
    )?;
    let graph = _graph;
    map_table.set(
        "__doc_fn_task_graph_mermaid",
        String::from(
            r#"
--- build mermaid from task graph
function plan.task_graph_mermaid()
end
"#,
        ),
    )?;
    map_table.set(
        "task_graph_mermaid",
        ctx.create_function(move |_ctx, (bot_ids, title): (Vec<u8>, String)| {
            let graph = graph.read();
            Ok(graph.mermaid_gantt(bot_ids, &title))
        })?,
    )?;
    let plan_builder = _plan_builder.clone();
    map_table.set(
        "__doc_fn_group_start",
        String::from(
            r#"
--- adds a GROUP START node to graph
-- Groups are used to synchronize bots so their tasks are completed before the next group is started.
-- Should be closed ith groupEnd. 
-- @string label label of group
function plan.group_start(label)
end
"#,
        ),
    )?;
    map_table.set(
        "group_start",
        ctx.create_function(move |_ctx, label: String| {
            plan_builder.group_start(label.as_str());
            Ok(())
        })?,
    )?;
    let plan_builder = _plan_builder;
    map_table.set(
        "__doc_fn_group_end",
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
        ctx.create_function(move |_ctx, ()| {
            plan_builder.group_end();
            Ok(())
        })?,
    )?;
    Ok(map_table)
}
