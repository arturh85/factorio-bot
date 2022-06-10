use crate::rune_plan_builder::RunePlanBuilder;
use factorio_bot_core::miette::{miette, Result};
use factorio_bot_core::paris::info;
use factorio_bot_core::plan::plan_builder::PlanBuilder;
use factorio_bot_core::plan::planner::Planner;
use factorio_bot_scripting::{buffers_to_string, redirect_buffers};
use rune::runtime::VmError;
use rune::termcolor::{ColorChoice, StandardStream};
use rune::{Context, ContextError, Diagnostics, Source, Sources, Vm};
use std::sync::Arc;

pub async fn run_rune(
    planner: &mut Planner,
    code: &str,
    filename: Option<&str>,
    bot_count: u8,
    redirect: bool,
) -> Result<(String, String)> {
    info!("rune");
    let buffers = redirect_buffers(redirect);
    let _all_bots = planner.initiate_missing_players_with_default_inventory(bot_count);
    planner.update_plan_world();
    let plan_builder = Arc::new(PlanBuilder::new(
        planner.graph.clone(),
        planner.plan_world.clone(),
    ));
    let to_miette = |err: ContextError| miette!(err);

    let mut context = Context::with_default_modules().map_err(to_miette)?;
    context
        .install(&RunePlanBuilder::module(plan_builder).map_err(to_miette)?)
        .map_err(to_miette)?;

    let runtime = Arc::new(context.runtime());

    let mut sources = Sources::new();
    sources.insert(Source::new(filename.unwrap_or("unknown"), code));

    let mut diagnostics = Diagnostics::new();

    let result = rune::prepare(&mut sources)
        .with_context(&context)
        .with_diagnostics(&mut diagnostics)
        .build();

    if !diagnostics.is_empty() {
        let mut writer = StandardStream::stderr(ColorChoice::Always);
        diagnostics
            .emit(&mut writer, &sources)
            .map_err(|err| miette!(err))?;
    }

    let unit = result.map_err(|err| miette!(err))?;

    let to_miette = |err: VmError| miette!(err);
    let mut vm = Vm::new(runtime, Arc::new(unit));

    let _output = vm
        .execute(&["main"], ())
        .map_err(to_miette)?
        .complete()
        .map_err(to_miette)?;
    // let output = vm
    //     .call(&["add"], (10i64, 20i64))
    //     .map_err(|err| miette!(err))?;
    // let output = i64::from_value(output).map_err(|err| miette!(err))?;
    // engine.on_print(|text| info!("{}", text));
    // engine.on_debug(|text, source, pos| {
    //     if let Some(source) = source {
    //         info!("{} @ {:?} | {}", source, pos, text);
    //     } else if pos.is_none() {
    //         info!("{}", text);
    //     } else {
    //         info!("{:?} | {}", pos, text);
    //     }
    // });
    // RhaiPlanBuilder::register(&mut engine);
    // RhaiRcon::register(&mut engine);
    // engine
    //     .register_type::<PlayerId>()
    //     .register_type::<Position>()
    //     .register_fn("pos", Position::new);
    //
    // let mut scope = Scope::new();
    // scope
    //     .push("all_bots", all_bots)
    //     .push("world", planner.plan_world.clone())
    //     .push("plan", RhaiPlanBuilder::new(plan_builder));
    // if let Some(rcon) = planner.rcon.as_ref() {
    //     scope.push("rcon", RhaiRcon::new(rcon.clone()));
    // }
    // if let Err(err) = engine.run_with_scope(&mut scope, code) {
    // handle_rhai_err(*err, code, filename)?;
    // }
    // Ok((buffers_to_string(buffers)?, scope))
    buffers_to_string(buffers)
}

#[cfg(test)]
mod tests {
    use factorio_bot_core::test_utils::fixture_world;

    use super::*;

    #[tokio::test]
    async fn test_mining() -> Result<()> {
        let bot_count = 2;
        let world = Arc::new(fixture_world());
        let mut planner = Planner::new(world, None);
        let (_, _) = run_rune(
            &mut planner,
            r##"pub fn main() {
    /*
    plan.foo.group_start("Mine Stuff");
    for (playerId, idx) in all_bots {
        plan.foo.mine(playerId, pos(idx.to_float() * 10.0, 43.0), "rock-huge", 1);
    }
    plan.foo.group_end();
    */
}"##,
            None,
            bot_count,
            false,
        )
        .await?;
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
        Ok(())
    }
}
