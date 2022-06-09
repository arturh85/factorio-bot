use crate::error::handle_rhai_err;
use crate::rhai_plan_builder::RhaiPlanBuilder;
use factorio_bot_core::plan::plan_builder::PlanBuilder;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_core::types::{PlayerId, Position};
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use miette::Result;
use rhai::{Engine, Scope};
use std::sync::Arc;

pub async fn run_rhai<'a>(
    planner: &'a mut Planner,
    code: &str,
    filename: Option<&str>,
    bot_count: u8,
    redirect: bool,
) -> Result<((String, String), Scope<'a>)> {
    let buffers = redirect_buffers(redirect);
    let all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let plan_builder = Arc::new(PlanBuilder::new(
        planner.graph.clone(),
        planner.plan_world.clone(),
    ));
    let mut engine = Engine::new();
    RhaiPlanBuilder::register(&mut engine);
    engine
        .register_type::<PlayerId>()
        .register_type::<Position>()
        .register_fn("pos", Position::new);

    let mut scope = Scope::new();
    scope
        .push("all_bots", all_bots)
        .push("world", planner.plan_world.clone())
        .push("plan", RhaiPlanBuilder::new(plan_builder));
    if let Some(rcon) = planner.rcon.as_ref() {
        scope.push("rcon", rcon.clone());
    }
    if let Err(err) = engine.run_with_scope(&mut scope, code) {
        handle_rhai_err(*err, code, filename)?;
    }
    Ok((buffers_to_string(buffers)?, scope))
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::test_utils::fixture_world;

    use super::*;

    #[tokio::test]
    async fn test_mining() {
        let bot_count = 2;
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (_, _) = run_rhai(
            &mut planner,
            r##"
    plan.group_start("Mine Stuff");
    for (playerId, idx) in all_bots {
        plan.mine(playerId, pos(idx.to_float() * 10.0, 43.0), "rock-huge", 1);
    }
    plan.group_end();
"##,
            None,
            bot_count,
            false,
        )
        .await
        .expect("run_rhai failed");
        let graph = planner.graph();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    2 [ label = "Start: Mine Stuff" ]
    3 [ label = "Walk to [0, 43]" ]
    4 [ label = "Mining rock-huge" ]
    5 [ label = "Walk to [10, 43]" ]
    6 [ label = "Mining rock-huge" ]
    7 [ label = "End" ]
    0 -> 2 [ label = "0" ]
    2 -> 3 [ label = "43" ]
    2 -> 5 [ label = "45" ]
    3 -> 4 [ label = "3" ]
    4 -> 7 [ label = "2" ]
    5 -> 6 [ label = "3" ]
    6 -> 7 [ label = "0" ]
    7 -> 1 [ label = "0" ]
}
"#,
        );
    }
}
