use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::graph::task_graph::{PositionRadius, TaskGraph};
use factorio_bot_core::plan::plan_builder::PlanBuilder;
use factorio_bot_core::types::{PlayerId, Position};
use parking_lot::RwLock;
use rlua::{Context, Table};
use std::sync::Arc;

pub fn create_lua_plan_builder(
    ctx: Context,
    graph: Arc<RwLock<TaskGraph>>,
    world: Arc<FactorioWorld>,
) -> rlua::Result<Table> {
    let map_table = ctx.create_table()?;
    let _plan_builder = Arc::new(PlanBuilder::new(graph, world));

    let plan_builder = _plan_builder.clone();
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
    let plan_builder = _plan_builder.clone();
    map_table.set(
        "groupStart",
        ctx.create_function(move |_ctx, label: String| {
            plan_builder.group_start(label.as_str());
            Ok(())
        })?,
    )?;
    let plan_builder = _plan_builder;
    map_table.set(
        "groupEnd",
        ctx.create_function(move |_ctx, ()| {
            plan_builder.group_end();
            Ok(())
        })?,
    )?;
    Ok(map_table)
}
