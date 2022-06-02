use factorio_bot_core::plan::planner::Planner;
use gag::BufferRedirect;
// use itertools::Itertools;
use miette::{IntoDiagnostic, Result};
use rhai::Engine;
use std::io::Read;
use std::sync::Arc;

pub fn run_rhai(
    planner: &mut Planner,
    rhai_code: &str,
    bot_count: u32,
) -> Result<(String, String)> {
    let mut stdout = BufferRedirect::stdout().into_diagnostic()?;
    let mut stderr = BufferRedirect::stderr().into_diagnostic()?;
    let _all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.plan_world = Arc::new((*planner.real_world).clone());

    // Create an 'Engine'
    let engine = Engine::new();

    // Run the script - prints "42"
    engine.run(rhai_code).unwrap();

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
print("hello world")
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
