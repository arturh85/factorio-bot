use factorio_bot_core::plan::planner::Planner;
use gag::BufferRedirect;
// use itertools::Itertools;
use factorio_bot_core::plan::plan_builder::PlanBuilder;
use miette::{IntoDiagnostic, Result};
use rhai::{Engine, Scope};
use std::io::Read;
use std::sync::Arc;

#[derive(Clone)]
struct MyTest {}
impl MyTest {
    #[allow(dead_code)]
    pub fn my_func(&mut self) {
        println!("my_func called");
    }
}

pub fn run_rhai(planner: &mut Planner, rhai_code: &str, bot_count: u8) -> Result<(String, String)> {
    let mut stdout = BufferRedirect::stdout().into_diagnostic()?;
    let mut stderr = BufferRedirect::stderr().into_diagnostic()?;
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
    let mut stdout_str = String::new();
    let mut stderr_str = String::new();
    stdout.read_to_string(&mut stdout_str).into_diagnostic()?;
    stderr.read_to_string(&mut stderr_str).into_diagnostic()?;
    Ok((stdout_str, stderr_str))
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
