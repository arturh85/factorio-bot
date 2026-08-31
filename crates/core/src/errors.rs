// False positive warnings from thiserror/miette derive macros using struct fields in format strings
#![allow(unused_assignments)]

use crate::types::PlayerId;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[error("failed to find workspace!")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("correct settings.workspace_path to a valid directory")
)]
pub struct WorkspaceNotFound {}

#[derive(Error, Debug, Diagnostic)]
#[error("missing mods/ folder from working directory")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("correct settings.workspace_path to a valid directory")
)]
pub struct MissingModsFolder {}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to create factorio mods symlink")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("allow elevated access for symlink creating")
)]
pub struct ModSymlinkFailed {}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to extract mods content to workspace")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("allow elevated access for symlink creating")
)]
pub struct ModExtractFailed {}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to extract scripts to workspace")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("allow elevated access for symlink creating")
)]
pub struct PlansExtractFailed {}

#[derive(Error, Debug, Diagnostic)]
#[error("mod {mod_name} targets Factorio {mod_factorio_version} but the installed game is {game_version}")]
#[diagnostic(
    code(factorio::mods::incompatible_version),
    help("Factorio compares only major.minor, so it will refuse the mod with `Incompatible Factorio version (current: {game_major_minor}, required: {mod_major_minor})` and the level creation that follows fails with a misleading `failed to create factorio level`. Without {mod_name} there is no RCON bridge and nothing in this project works.\nFix: set \"factorio_version\": \"{game_major_minor}\" in {mod_info_path} (then delete the copy under the workspace mods directory so it is re-extracted), or install a Factorio {mod_major_minor}.x archive.\nThe installed version was read from {base_info_path}.")
)]
pub struct ModFactorioVersionMismatch {
    pub mod_name: String,
    pub mod_factorio_version: String,
    pub mod_major_minor: String,
    pub game_version: String,
    pub game_major_minor: String,
    pub mod_info_path: String,
    pub base_info_path: String,
}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to find factorio binary")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("delete factorio folder as it is broken")
)]
pub struct FactorioBinaryNotFound {}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to find instance")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct FactorioInstanceNotFound {}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to find factorio saves folder")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct FactorioSavesNotFound {}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to find factorio server settings")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct FactorioSettingsNotFound {}

#[derive(Error, Debug, Diagnostic)]
#[error("failed to create factorio level")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct FactorioLevelFailed {}

#[derive(Error, Debug, Diagnostic)]
#[error("factorio instance already running")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("stop running instance first")
)]
pub struct FactorioAlreadyStarted {}

#[derive(Error, Debug, Diagnostic)]
#[error("player not found (id {player_id})")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("provide correct player id")
)]
pub struct RconPlayerNotFound {
    pub player_id: PlayerId,
}

#[derive(Error, Debug, Diagnostic)]
#[error("player still blocks placement")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct RconPlayerBlockesPlacement {}

#[derive(Error, Debug, Diagnostic)]
#[error("player blocks placement in all directions")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct RconPlayerBlockesAllPlacement {}

#[derive(Error, Debug, Diagnostic)]
#[error("Unexpected Empty Response")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct RconUnexpectedEmptyResponse {}

#[derive(Error, Debug, Diagnostic)]
#[error("Unexpected Output: {output}")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct RconUnexpectedOutput {
    pub output: String,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Unexpected Response: {message}")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct RconError {
    pub message: String,
}

#[derive(Error, Debug, Diagnostic)]
#[error("no action result received in time")]
#[diagnostic(code(factorio::workspace::not_found), help("read logs"))]
pub struct RconTimeout {}

#[derive(Error, Debug, Diagnostic)]
#[error("max radius request {limit} exceeds limit of 3000")]
#[diagnostic(
    code(factorio::workspace::not_found),
    help("use lower value for radius")
)]
pub struct RconRadiusLimitReached {
    pub limit: u32,
}

#[derive(Error, Debug, Diagnostic)]
#[error("could not find water")]
#[diagnostic(code(factorio::workspace::not_found), help("build somewhere else"))]
pub struct RconNoWaterFound {}

#[derive(Error, Debug, Diagnostic)]
#[error("fromPosition is blocked")]
#[diagnostic(code(factorio::workspace::not_found), help("build somewhere else"))]
pub struct RconSourcePositionBlocked {}

#[derive(Error, Debug, Diagnostic)]
#[error("toPosition is blocked")]
#[diagnostic(code(factorio::workspace::not_found), help("build somewhere else"))]
pub struct RconTargetPositionBlocked {}

#[derive(Error, Debug, Diagnostic)]
#[error("no path found")]
#[diagnostic(code(factorio::workspace::not_found), help("build somewhere else"))]
pub struct RconNoPathFound {}

#[derive(Error, Debug, Diagnostic)]
#[error("invalid rect input: '{invalid_input}' (expected A,B;C,D like 1.2,3.4;5.6,7.8)")]
#[diagnostic(code(factorio::workspace::not_found), help("fix rect formatting"))]
pub struct RectInvalid {
    pub invalid_input: String,
}

/// A walk was asked to end at a position no path could reach.
///
/// Raised *before* any walk is dispatched, by `FactorioRcon::move_player_timed`,
/// when the best path the pathfinder produced ends further from the requested
/// goal than the requested radius allows. It exists because
/// `FactorioRcon::player_path` is a best-effort primitive: when the goal itself
/// is unreachable it retries against a synthesised goal offset away from the
/// real one, and the caller must not read the resulting path as evidence that
/// the bot will end up where it asked to be.
#[derive(Error, Debug, Diagnostic)]
#[error("no path to [{goal_x}, {goal_y}] — the best one found ends [{shortfall:.3}] tiles away at [{end_x}, {end_y}], outside the [{tolerance:.3}] tile arrival tolerance")]
#[diagnostic(
    code(factorio::rcon::walk_falls_short),
    help("the goal is very likely blocked — a tile the bot itself built on is the usual cause; pick a standing position beside the target instead of on it")
)]
pub struct RconWalkFallsShort {
    pub goal_x: f64,
    pub goal_y: f64,
    pub end_x: f64,
    pub end_y: f64,
    pub shortfall: f64,
    pub tolerance: f64,
}

/// A mine was about to be dispatched from outside the player's resource reach.
///
/// Raised *before* any mining command is sent, by
/// `FactorioRcon::player_mine_timed`, after the walk it makes first has
/// finished and left the bot still too far away. The game silently refuses to
/// mine a resource outside `resource_reach_distance` — no event, no error — so
/// dispatching anyway costs the whole action deadline in silence.
#[derive(Error, Debug, Diagnostic)]
#[error("still [{distance:.3}] tiles from [{target_x}, {target_y}] after walking, outside the [{reach:.3}] tile resource reach")]
#[diagnostic(
    code(factorio::rcon::out_of_resource_reach),
    help("the game refuses such a mine silently, so it is rejected here instead; retry, or plan a standing position closer to the resource")
)]
pub struct RconOutOfResourceReach {
    pub target_x: f64,
    pub target_y: f64,
    pub distance: f64,
    pub reach: f64,
}
