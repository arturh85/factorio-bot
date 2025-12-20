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

#[derive(Error, Debug, Diagnostic)]
#[error("player {player_id} does not have item '{item}' in inventory")]
#[diagnostic(code(factorio::workspace::not_found), help("fix logic"))]
pub struct PlayerMissingItem {
    pub player_id: PlayerId,
    pub item: String,
}
