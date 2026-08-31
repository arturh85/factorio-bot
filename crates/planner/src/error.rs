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
    ///
    /// `bound_to` is whichever committed the chain to a bot: its **owner**, if
    /// a caller named one, and otherwise the bot the scheduler bound it to when
    /// the chain opened. Ownership is checked first because it is the harder
    /// constraint — a caller's instruction, stated before scheduling begins,
    /// with no fallback tier — and because a chain with an owner is only ever
    /// bound to that owner, so the owner is the original cause.
    #[error(
        "action {action:?} is pinned to {pinned_to}, but its chain {chain:?} already belongs to {bound_to}"
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
        /// The action that could not be run, carried like every sibling
        /// variant carries one: tier-1 recovery has to know which action to
        /// re-plan around, and the chain alone does not say.
        action: ActionId,
        bot: BotId,
        condition: String,
    },

    /// Deliberately not a `NoApplicableMethod`. That variant means "this world
    /// offers no route to the thing you asked for", which sends a caller
    /// looking at prerequisites and resources. A technology no force in this
    /// world defines is not a routing problem at all — the name itself is
    /// wrong, or the world was never told about the technology — and no amount
    /// of mining will fix it.
    #[error("the force this plan acts for defines no technology named {technology}")]
    #[diagnostic(
        code(planner::unknown_technology),
        help("check the spelling, or whether the world's forces have been loaded yet")
    )]
    UnknownTechnology { technology: String },

    #[error("no method can satisfy goal: {goal}")]
    #[diagnostic(code(planner::no_applicable_method))]
    NoApplicableMethod { goal: String },

    /// A Factorio 2.0 `research_trigger` technology whose trigger this planner
    /// has no goal for.
    ///
    /// Deliberately an error rather than a zero cost. These technologies carry
    /// no science-pack bill and no research time at all, so a planner that
    /// reads only the pack fields plans them as *free* — the plan comes out
    /// correctly ordered and wrongly timed, and nothing in it says so. A
    /// makespan that is quietly missing several steps is worse than a refusal,
    /// because a caller cannot tell it happened. Refusing names the technology
    /// and the trigger kind, so a caller can see exactly what is not modelled.
    #[error(
        "{technology} is unlocked by a {trigger} trigger, which this planner cannot express as a goal"
    )]
    #[diagnostic(
        code(planner::unsupported_research_trigger),
        help(
            "only `craft-item` triggers can be planned; costing this one at zero would silently \
             under-report the plan's makespan"
        )
    )]
    UnsupportedResearchTrigger {
        technology: String,
        /// The trigger's `type` string, e.g. `mine-entity`.
        trigger: String,
    },

    /// A `craft-item` trigger asking for an item whose recipe only this same
    /// technology unlocks.
    ///
    /// Shipped 2.1.17 really contains six of these — `foundry` is triggered by
    /// crafting a foundry and is the only technology unlocking the foundry
    /// recipe, and `biochamber`, `big-mining-drill`, `cryogenic-plant`,
    /// `tungsten-carbide` and `electromagnetic-plant` are the same shape. The
    /// game resolves them by routes outside this planner's world model.
    ///
    /// Diagnosed here rather than left to recurse: expanding it naively goes
    /// `Researched(t)` -> `Have(item)` -> "that recipe needs `t`" ->
    /// `Researched(t)` until the driver's depth guard fires, and
    /// `ExpansionTooDeep` then reports "a method is probably expanding into
    /// itself" — which is true, and tells a caller nothing about which
    /// technology or why.
    #[error(
        "{technology} is triggered by crafting {item}, but only {technology} unlocks that recipe"
    )]
    #[diagnostic(
        code(planner::self_unlocking_research_trigger),
        help("this technology cannot be reached from the current world state")
    )]
    SelfUnlockingResearchTrigger { technology: String, item: ItemId },

    #[error(
        "expansion of {goal} exceeded {depth} levels; a method is probably expanding into itself"
    )]
    #[diagnostic(code(planner::expansion_too_deep))]
    ExpansionTooDeep { goal: String, depth: u32 },

    #[error(
        "{a} and {b} hold different amounts of {item}; expansion sizes each share against one bot's inventory and assumes any bot would do"
    )]
    #[diagnostic(
        code(planner::bots_not_interchangeable),
        help(
            "re-plan per bot, or extend the driver to size shares against the bot that will run them"
        )
    )]
    BotsNotInterchangeable { a: BotId, b: BotId, item: ItemId },
}
