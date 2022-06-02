use factorio_bot_core::plan::planner::Planner;
#[cfg(feature = "lua")]
use factorio_bot_scripting_lua::lua_runner::run_lua;
#[cfg(feature = "rhai")]
use factorio_bot_scripting_rhai::rhai_runner::run_rhai;

#[allow(dead_code)]
#[allow(unused_variables)]
pub fn run_script(
  planner: &mut Planner,
  language: &str,
  code: &str,
  bot_count: u32,
) -> miette::Result<(String, String)> {
  match language {
    #[cfg(feature = "lua")]
    "lua" => run_lua(planner, code, bot_count),
    #[cfg(feature = "rhai")]
    "rhai" => run_rhai(planner, code, bot_count),
    _ => Err(miette::Error::msg("unknown language")),
  }
}
