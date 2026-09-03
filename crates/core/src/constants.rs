pub const WORKSPACE_FOLDERNAME: &str = "workspace";
pub const MODS_FOLDERNAME: &str = "mods";
/// Factorio's own record of which mods it found and which are enabled,
/// written by the game into its mods directory on every start. A mod that is
/// present on disk but absent from an existing list is a *disabled* mod, so
/// this file can silently unload `BotBridge` -- see
/// `instance_setup::ensure_bridge_mod_enabled`.
pub const MOD_LIST_FILENAME: &str = "mod-list.json";
pub const SERVER_SETTINGS_FILENAME: &str = "server-settings.json";
pub const MAP_GEN_SETTINGS_FILENAME: &str = "map-gen-settings.json";
pub const MAP_SETTINGS_FILENAME: &str = "map-settings.json";

/// The force the bots play for.
///
/// **One fact about the mod, so one constant.** `mods/BotBridge/control.lua`
/// hardcodes `game.forces["player"]` in every place that speaks for the bots --
/// `collect_recipes`, `collect_player_force`, `start_research` -- so this is
/// the force whose technology and recipe tables describe them. The planner and
/// the executor each held their own copy of the string; they cannot
/// legitimately disagree, because a plan made for one force and executed
/// against another is the defect this constant exists to prevent, and it lives
/// here because `crates/core` is the only place both can see.
///
/// **It must not be picked by sorting `FactorioWorld::forces`.**
/// `writeout_forces` emits *all* of `game.forces`, so from the first
/// `on_research_finished` of a run the world also holds `enemy` and `neutral`,
/// whose technology tables describe nobody and are researched by nobody.
/// `PlanState::from_world` took `forces.keys().min()`, which is `enemy`, and
/// spent two milestones re-deriving technologies the player force already had.
/// See `docs/superpowers/notes/2026-09-02-recipe-not-enabled.md`.
pub const BOT_FORCE: &str = "player";
