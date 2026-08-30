use crate::ids::{ActionId, BotId, ChainId, ItemId};
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum PlannerError {
    #[error("{bot} has {available} {item}, needs {required}")]
    #[diagnostic(code(planner::insufficient_items))]
    InsufficientItems {
        bot: BotId,
        item: ItemId,
        required: u32,
        available: u32,
    },

    #[error("no bot {0:?} in this plan state")]
    #[diagnostic(code(planner::unknown_bot))]
    UnknownBot(BotId),

    #[error("action network contains a cycle involving {0:?}")]
    #[diagnostic(code(planner::cyclic_network))]
    CyclicNetwork(ActionId),

    #[error("action {action:?} is unreachable: its predecessors can never all complete")]
    #[diagnostic(code(planner::deadlock))]
    Deadlock { action: ActionId },

    #[error("precondition {condition} of action {action:?} does not hold for {bot}")]
    #[diagnostic(code(planner::precondition_unsatisfied))]
    PreconditionUnsatisfied {
        action: ActionId,
        bot: BotId,
        condition: String,
    },

    /// Deliberately not a `PreconditionUnsatisfied`: that variant means "the
    /// world was not as planned", which the spec answers by re-planning from
    /// observed state. A pin that contradicts a chain binding is not about the
    /// world at all — re-planning would produce the same contradiction forever.
    #[error(
        "action {action:?} is pinned to {pinned_to}, but its chain {chain:?} is already bound to {bound_to}"
    )]
    #[diagnostic(code(planner::chain_conflict))]
    ChainConflict {
        chain: ChainId,
        action: ActionId,
        bound_to: BotId,
        pinned_to: BotId,
    },

    #[error("no bots supplied to the scheduler")]
    #[diagnostic(code(planner::no_bots))]
    NoBots,

    #[error(
        "{bot} owns chain {chain:?} because a caller named it, but {condition} does not hold there"
    )]
    #[diagnostic(code(planner::chain_owner_infeasible))]
    ChainOwnerInfeasible {
        chain: ChainId,
        bot: BotId,
        condition: String,
    },

    #[error("no method can satisfy goal: {goal}")]
    #[diagnostic(code(planner::no_applicable_method))]
    NoApplicableMethod { goal: String },

    #[error(
        "expansion of {goal} exceeded {depth} levels; a method is probably expanding into itself"
    )]
    #[diagnostic(code(planner::expansion_too_deep))]
    ExpansionTooDeep { goal: String, depth: u32 },

    #[error("{a} and {b} hold different amounts of {item}; expansion sizes each share against one bot's inventory and assumes any bot would do")]
    #[diagnostic(
        code(planner::bots_not_interchangeable),
        help("re-plan per bot, or extend the driver to size shares against the bot that will run them")
    )]
    BotsNotInterchangeable { a: BotId, b: BotId, item: ItemId },
}
