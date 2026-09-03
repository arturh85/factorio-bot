pub mod action;
pub mod enclosure;
pub mod error;
pub mod goal;
pub mod ids;
pub mod method;
pub mod network;
pub mod render;
pub mod report;
pub mod schedule;
pub mod state;

/// Test-only worlds. Not part of the crate's API: research needs a world with
/// a force, and the shared `fixture_world` has none.
#[cfg(test)]
mod test_world;

pub use action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
pub use error::PlannerError;
pub use goal::{Goal, Holder};
pub use ids::{ActionId, ActionIdGen, BotId, ChainId, ChainIdGen, ItemId, Ticks};
pub use method::have::default_registry;
pub use method::have::holds;
pub use method::have::registry_for;
pub use method::{ExpansionCtx, GoalSite, Method, MethodRegistry, Step};
pub use method::{MAX_EXPANSION_DEPTH, expand, pick_chain_actor};
pub use network::{ActionNetwork, Edge};
pub use render::{graphviz, mermaid_gantt, ticks_to_timestamp};
pub use report::{BotReport, PlanReport};
pub use schedule::{
    Schedule, ScheduledStep, StepKind, WALK_TILES_PER_TICK, schedule, travel_ticks,
};
pub use state::{BotState, Buffer, PlanState};
