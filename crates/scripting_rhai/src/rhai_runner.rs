use factorio_bot_core::plan::plan_builder::PlanBuilder;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use miette::Result;
use rhai::{Engine, Scope};
use std::sync::Arc;

#[derive(Clone)]
struct MyTest {}
impl MyTest {
    #[allow(dead_code)]
    pub fn my_func(&mut self) {
        println!("my_func called");
    }
}

pub fn run_rhai(
    planner: &mut Planner,
    rhai_code: &str,
    bot_count: u8,
    redirect: bool,
) -> Result<(String, String)> {
    let buffers = redirect_buffers(redirect);
    let all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let plan_builder = Arc::new(PlanBuilder::new(
        planner.graph.clone(),
        planner.plan_world.clone(),
    ));
    let mut engine = Engine::new();
    engine.register_type::<MyTest>();
    // engine.register_type_with_name::<PlanBuilder>("PlanBuilder");

    let mut scope = Scope::new();
    scope.set_value("all_bots", all_bots);
    scope.set_value("world", planner.plan_world.clone());
    scope.set_value("plan", plan_builder);
    scope.set_value("myTest", MyTest {});
    if let Some(rcon) = planner.rcon.as_ref() {
        scope.set_value("rcon", rcon.clone());
    }
    engine.run_with_scope(&mut scope, rhai_code).unwrap();
    buffers_to_string(buffers)
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::test_utils::fixture_world;

    use super::*;

    #[test]
    fn test_mining() {
        let bot_count = 2;
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        run_rhai(
            &mut planner,
            r##"
debug(myTest);
        "##,
            bot_count,
            false,
        )
        .unwrap();
        let graph = planner.graph();
        assert_eq!(
            graph.graphviz_dot(),
            r#"digraph {
    0 [ label = "Process Start" ]
    1 [ label = "Process End" ]
    0 -> 1 [ label = "0" ]
}
"#,
        );
    }
}
