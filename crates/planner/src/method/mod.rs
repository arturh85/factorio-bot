//! How to get what we want: hand-written decompositions, and the driver that
//! runs them until only actions remain.

pub mod have;
pub mod util;

use crate::action::Action;
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ActionIdGen, BotId, ChainId, ChainIdGen, ItemId, Ticks};
use crate::state::PlanState;

/// One element of a method's expansion.
#[derive(Clone, Debug)]
pub enum Step {
    /// Recurse: this goal is expanded by whatever method claims it.
    Subgoal(Goal),
    /// A primitive action. Boxed because `Action` is large.
    Act(Box<Action>),
    /// An explicit ordering edge with a minimum lag, for dependencies that
    /// inference cannot see — a furnace's smelting time, above all. The ids
    /// come from actions the same method allocated via `ExpansionCtx::ids`.
    Link {
        from: ActionId,
        to: ActionId,
        lag: Ticks,
    },
}

/// State threaded through one expansion.
///
/// `chain_actor` is used **only** to simulate effects while expanding, so that
/// a later sibling goal sees what an earlier one produced. It never reaches an
/// emitted action: methods emit `Actor::Role` with `pinned: None`, and the
/// scheduler decides who actually runs each action.
///
/// This rests on bots being interchangeable at the start of planning — chain
/// *j* is simulated against bot *j* on the assumption that any bot would
/// experience the same thing. That holds while `PlanState::from_world` gives
/// unknown bots identical defaults. **If bots ever start with materially
/// different inventories, this driver must be revisited.**
///
/// `chain` is the other half of the same story, and the half that outlives
/// expansion: every action emitted inside a chained subtree is stamped with it
/// in the network, so the scheduler can bind the whole chain to one bot instead
/// of choosing per action. A subtree is chained when a caller named a bot for
/// it (`Holder::Bot`), when the goal is a `Holder::Share` and so states that
/// its holding ends up in one inventory, or when the method claiming its root
/// `converges` — see `expand_goal_body`. It is `None` outside such a subtree,
/// which leaves an action freely assignable.
///
/// The first two also give the chain an **owner** — the named bot, in both
/// cases — so the scheduler runs it there rather than merely keeping it
/// together on whoever is cheapest. See the owner-binding comment in
/// `expand_goal_body` for why a `Holder::Share` earns this too, and at what
/// cost.
///
/// `top_level` records whether the goal being expanded is one the *caller*
/// asked for rather than one a method asked for. See `GoalSite`.
pub struct ExpansionCtx {
    pub state: PlanState,
    pub ids: ActionIdGen,
    /// Driver-owned. A method that allocated or overwrote a chain would break
    /// binding silently — the network would still schedule, just onto the wrong
    /// bots — so methods cannot reach these two at all.
    pub(crate) chains: ChainIdGen,
    pub chain_actor: BotId,
    /// The chain actions emitted right now belong to, if any. Driver-owned.
    pub(crate) chain: Option<ChainId>,
    /// True while expanding a goal the caller handed to `expand` (or a member
    /// of a top-level `Goal::All`), false beneath any method's subgoal.
    /// Driver-owned, for the same reason `chain` is.
    pub(crate) top_level: bool,
    pub depth: u32,
}

impl ExpansionCtx {
    pub fn new(state: PlanState, chain_actor: BotId) -> Self {
        ExpansionCtx {
            state,
            ids: ActionIdGen::new(),
            chains: ChainIdGen::new(),
            chain_actor,
            chain: None,
            top_level: true,
            depth: 0,
        }
    }
}

/// Where in an expansion a goal sits.
///
/// `applicable` answers "can I satisfy this goal at all", which depends only on
/// the goal and the world. Whether a method may *claim* a goal can also depend
/// on where the goal came from, and that is a fact only the driver holds. One
/// method needs it: `SplitAcrossBots` scatters a goal's shares across bots,
/// which is sound only when nothing downstream needs the results gathered in
/// one inventory — and every goal a method asked for has exactly such a
/// consumer waiting for it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GoalSite {
    /// True for a goal the caller handed to `expand`, and for a member of a
    /// top-level `Goal::All` — a bundle of independent goals is still a bundle
    /// of goals nothing downstream consumes. False for any subgoal a method
    /// asked for.
    pub top_level: bool,
    /// True when the goal is being expanded inside a per-bot chain, so its
    /// actions will all be welded to that chain's single runner.
    pub in_chain: bool,
}

impl GoalSite {
    /// The site of a goal the caller asked for directly.
    pub fn root() -> Self {
        GoalSite {
            top_level: true,
            in_chain: false,
        }
    }
}

/// One way to satisfy one kind of goal. Hand-written and readable by design:
/// there is no search here, and adding a method should never require
/// understanding the others.
pub trait Method {
    fn name(&self) -> &'static str;

    /// Can this method satisfy `goal` given `state`? Consulted in registration
    /// order, so a cheaper method registered earlier wins.
    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool;

    /// May this method claim a goal sitting *here*? Defaults to yes: almost
    /// every method is indifferent to where a goal came from. Override it only
    /// when a method's decomposition is unsound at some sites — see `GoalSite`.
    fn claims(&self, _site: GoalSite) -> bool {
        true
    }

    /// Does this method's decomposition require several *produced* items to
    /// meet in one inventory?
    ///
    /// If so the driver opens a chain over its subtree, welding the producers
    /// to the consumer that needs them together. Defaults to `false`, which is
    /// right for every method whose inputs arrive through separate actions —
    /// a furnace is loaded by one insert per ingredient, so three bots can each
    /// supply one and nothing has to converge.
    fn converges(&self, _goal: &Goal, _state: &PlanState) -> bool {
        false
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError>;
}

/// Methods in preference order. The first applicable one wins.
#[derive(Default)]
pub struct MethodRegistry {
    methods: Vec<Box<dyn Method>>,
}

impl MethodRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, method: Box<dyn Method>) -> Self {
        self.methods.push(method);
        self
    }

    pub fn find(&self, goal: &Goal, state: &PlanState, site: GoalSite) -> Option<&dyn Method> {
        self.methods
            .iter()
            .find(|m| m.claims(site) && m.applicable(goal, state))
            .map(|m| m.as_ref())
    }
}

use crate::network::ActionNetwork;

/// How many nested goals one expansion may reach before it is treated as a
/// runaway. Exists to turn a method that expands into itself into an error
/// rather than a hang.
///
/// Two independent things spend this budget, and they add up:
///
/// * **Recipe nesting.** Science pack to gear to plate to ore is four levels,
///   plus one for a per-bot split.
/// * **Research prerequisites.** `method::have::Researched` emits a
///   `Researched` subgoal per prerequisite, so a technology's prerequisite
///   chain costs one level per link, on top of the recipe nesting under
///   whichever link's science packs run deepest.
///
/// Measured against the fixtures, not derived: a prerequisite chain of
/// technologies that cost no science packs expands to 32 links and fails at
/// 33; one whose every link costs an automation science pack expands to 27 and
/// fails at 28, the difference being the pack's own recipe nesting. Both are
/// pinned by tests in `method::have` that state a chain length rather than
/// arithmetic on this constant, so changing it here cannot make them pass by
/// definition.
///
/// **Not raised for research.** Factorio's own technology tree runs to roughly
/// eighteen links at its deepest, which fits inside 27 with room to spare, and
/// the bound is only useful while it is low enough to catch a runaway quickly.
/// Raise it when a real tree is shown not to fit, and say which one.
pub const MAX_EXPANSION_DEPTH: u32 = 32;

/// Expand `goals` into a schedulable network.
///
/// Each goal is expanded by the first applicable method, recursively, until
/// only actions remain. The context's state is advanced as actions are emitted,
/// so a later sibling sees what an earlier one produced — that progression is
/// what lets hand-written methods compose without knowing about each other.
///
/// Ordering edges are inferred at the end, on top of whatever explicit `Link`
/// steps the methods emitted for dependencies inference cannot see.
///
/// Three rosters meet here and nothing else reconciles them: the one
/// `registry_for` was built with, `chain_actor`, and the bots `state` was built
/// with. A bot the state does not know has no position and no inventory, and
/// the methods would quietly plan for a default one standing at the origin, so
/// every bot this expansion simulates against must be a bot the state knows —
/// `chain_actor` here, and each `Holder::Bot` the expansion meets below.
pub fn expand(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
) -> Result<ActionNetwork, PlannerError> {
    if state.bot(chain_actor).is_none() {
        return Err(PlannerError::UnknownBot(chain_actor));
    }
    let mut ctx = ExpansionCtx::new(state.fork(), chain_actor);
    let mut net = ActionNetwork::new();
    for goal in goals {
        expand_goal(goal, &mut ctx, &mut net, registry)?;
    }
    net.infer_edges();
    net.validate()?;
    Ok(net)
}

/// Whose inventory a goal is stated against, if it names one.
///
/// [`Goal::Have`] and [`Goal::Produced`] carry the *same* `whose` with the
/// *same* meaning — `Produced`'s own doc says so — and every place the driver
/// reads it has to treat them alike. Reading it off `Have` alone is how a
/// trigger technology's production came to be planned for one bot's inventory
/// and welded to nobody's: `Researched` asks for a trigger's item as
/// `Produced { whose: Holder::Share(chain_actor) }` "for the same reason the
/// pack bill uses it: one action reading one bot's inventory", and that claim
/// was simply not enforced. The smelt's `insert 50 iron-ore` stayed freely
/// assignable while the `Have { iron-ore, 50, Share }` under it opened a chain
/// of its own, so the scheduler could — and on a four-bot run did — mine onto
/// one bot and ask another to load the furnace.
///
/// One function so the three reads below cannot drift apart.
fn stated_holder(goal: &Goal) -> Option<&Holder> {
    match goal {
        Goal::Have { whose, .. } | Goal::Produced { whose, .. } => Some(whose),
        _ => None,
    }
}

fn expand_goal(
    goal: &Goal,
    ctx: &mut ExpansionCtx,
    net: &mut ActionNetwork,
    registry: &MethodRegistry,
) -> Result<(), PlannerError> {
    if ctx.depth >= MAX_EXPANSION_DEPTH {
        return Err(PlannerError::ExpansionTooDeep {
            goal: goal.to_string(),
            depth: MAX_EXPANSION_DEPTH,
        });
    }

    // Save, run, restore — on every exit path, errors included. A completed
    // call must leave `depth`, `chain_actor`, `chain` and `top_level` exactly
    // as it found them even when it fails, or a caller that continues past an
    // error inherits a corrupted context and a comment claiming that cannot
    // happen.
    let previous_actor = ctx.chain_actor;
    let previous_chain = ctx.chain;
    let previous_top_level = ctx.top_level;
    if let Some(Holder::Bot(bot) | Holder::Share(bot)) = stated_holder(goal) {
        // The same reconciliation `expand` does for `chain_actor`, applied to
        // the roster a method decomposes with: `SplitAcrossBots` addresses the
        // bots the registry was built with, and nothing has checked those
        // against the state until here.
        if ctx.state.bot(*bot).is_none() {
            return Err(PlannerError::UnknownBot(*bot));
        }
        // `whose` is propagated verbatim into subgoals — a per-bot science
        // pack asks for per-bot plates — so simulated effects must land in
        // the same inventory the shortfall checks read for the whole subtree
        // this goal sits above, not just for this goal alone. Chain opening
        // itself happens in `expand_goal_body`, once the method is known.
        ctx.chain_actor = *bot;
    }
    ctx.depth += 1;

    let result = expand_goal_body(goal, ctx, net, registry);

    ctx.depth -= 1;
    ctx.chain_actor = previous_actor;
    ctx.chain = previous_chain;
    ctx.top_level = previous_top_level;
    result
}

fn expand_goal_body(
    goal: &Goal,
    ctx: &mut ExpansionCtx,
    net: &mut ActionNetwork,
    registry: &MethodRegistry,
) -> Result<(), PlannerError> {
    // A goal addressed to one bot rebinds the chain actor for its whole
    // subtree, so that simulated effects land in the same inventory the
    // shortfall checks read. Without this the driver would credit a chain's
    // mining to one bot while asking whether a different one was satisfied.
    // (The rebind itself happens in `expand_goal`, which also restores it.)
    if let Goal::All(inner) = goal {
        for g in inner {
            expand_goal(g, ctx, net, registry)?;
        }
        return Ok(());
    }

    let chain_on_entry = ctx.chain;
    let site = GoalSite {
        top_level: ctx.top_level,
        in_chain: ctx.chain.is_some(),
    };
    let method =
        registry
            .find(goal, &ctx.state, site)
            .ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            })?;

    // A chain welds actions to one runner. Three things ask for that.
    //
    // * **A caller naming a bot** (`Holder::Bot`), which additionally records
    //   that bot as the chain's owner.
    // * **A `Holder::Share`**, which states that the holding has to end up in
    //   *one* inventory, sized against that bot's starting inventory. It
    //   *also* records that bot as the chain's owner, for the same reason a
    //   `Holder::Bot` does: `SplitAcrossBots` hands a bot a share to mine,
    //   smelt and craft on its own, and `Researched` asks for packs — and a
    //   trigger's whole production bill — by share "because the research is
    //   one action reading one bot's inventory". Both claims are true only if
    //   the bot the bill was sized against is the bot that runs it. Before
    //   2026-09-02 the chain got no owner here, on the theory that bots are
    //   interchangeable so it does not matter who runs a chain sized against
    //   one of them; that theory broke on a live four-bot run once gathering
    //   milestones had left the roster unequal (8 / 8 / 4 iron-ore) — the
    //   scheduler bound a share's chain to whichever bot was cheapest, not
    //   the one its bill was sized against, mined the shortfall onto that
    //   bot, and a downstream action needing the full count failed for a
    //   *different* bot, naming first bot 2 then bot 3 across two crashes of
    //   the same run. See `docs/superpowers/notes/2026-09-02-rung-3-4-findings.md`,
    //   "The rest of the story", for the three options weighed and why this
    //   one (option 1) was chosen: it makes the sizing true by construction
    //   rather than true by assumption, at a real, measured cost — the
    //   recorded run's trigger subtree and pack subtree ran concurrently on
    //   two different bots for 22,072 ticks; binding both to their shared
    //   `chain_actor` serialises them onto one. A slower correct run beats a
    //   faster crashing one, so that cost is accepted, not incidental — do
    //   not "restore parallelism" here without also re-solving the sizing
    //   problem it removes.
    // * **A method that `converges`**, whose decomposition makes several
    //   produced items meet in one inventory. This gets no owner: nothing
    //   named a bot for it, only the shape of the decomposition, so who runs
    //   it stays the scheduler's decision.
    //
    // Nothing else. A goal that merely sits inside a chain needs no second
    // one, and welding what nobody has to gather serialises work that could
    // have run in parallel.
    //
    // The holder is read through `stated_holder`, so a `Goal::Produced` says
    // all this exactly as a `Goal::Have` does. Reading it off `Have` alone is
    // how a trigger technology's fifty iron plates came to be smelted by
    // whoever, out of ore mined by someone else — see `stated_holder`.
    if ctx.chain.is_none() {
        let owner = match stated_holder(goal) {
            Some(Holder::Bot(bot) | Holder::Share(bot)) => Some(*bot),
            _ => None,
        };
        let one_inventory = matches!(stated_holder(goal), Some(Holder::Bot(_) | Holder::Share(_)));
        if one_inventory || method.converges(goal, &ctx.state) {
            let chain = ctx.chains.next();
            ctx.chain = Some(chain);
            if let Some(bot) = owner {
                net.set_chain_owner(chain, bot);
            }
        }
    }

    // Whether *this* frame opened the chain, and so owes the ledger entry at
    // the end of the body. Methods cannot reach `ctx.chain`, so nothing
    // between here and there can change the answer.
    //
    // A roster of one is exempt, and not as an optimisation: the entry below
    // exists because two chains may be handed to two bots, and with one bot
    // they cannot be. What one chain made really is in the next chain's hands,
    // so hiding it would make a single bot mine and smelt a second time for
    // stock it is already carrying — `researched("automation")` at one bot
    // goes from 58 steps to 101 with no defect to show for it.
    let opened_chain =
        ctx.chain.is_some() && chain_on_entry.is_none() && ctx.state.bot_ids().len() > 1;
    let produce_before = opened_chain.then(|| ctx.state.item_totals());

    let steps = method.expand(goal, ctx)?;

    // Everything below this line was asked for by a method, not by the caller,
    // so nothing in the subtree is top level. `expand_goal` restores the flag
    // for our own caller. A `Goal::All` never reaches here — it returns above,
    // so its members keep the site the bundle itself had.
    ctx.top_level = false;

    // Save, run, restore, exactly as `expand_goal` does for the chain: a
    // reservation this method made must be given back however the body exits,
    // or a caller that continues past an error inherits a state that thinks
    // items are spoken for by an action that was never emitted.
    let mut promised: Vec<(Holder, ItemId, u32)> = Vec::new();
    let result = run_steps(steps, ctx, net, registry, &mut promised);
    for (whose, item, count) in &promised {
        ctx.state.release(whose, item, *count);
    }

    // What a chain made belongs to that chain.
    //
    // Expansion simulates every chain's effects into the *same* notional
    // inventory — `chain_actor`, one bot — while the scheduler is free to put
    // two sibling chains on two different bots. So the moment a chain closes,
    // what it produced is stock some *other* runner may be carrying, and a
    // sibling that sizes itself against it plans work it cannot do: research
    // `steam-power` (whose trigger is "craft 50 iron plates"), then craft a
    // lab, and the lab chain sees fifty plates sitting there, smelts none of
    // its own, and the schedule then hands the two chains to two bots.
    //
    // Reserved, not spent, for the same reason as every other entry in this
    // ledger: `inventory_count` and `lose` still see the items, so the chain's
    // own arithmetic and its `Condition::HasItem`s are untouched, and only
    // *other* goals asking `available` are told the stock is spoken for.
    //
    // Held for the rest of the expansion rather than released with the
    // enclosing method, which is the one asymmetry with `run_steps`'
    // reservations: those exist for one pending action and end with it, this
    // one records where the items physically are and that does not stop being
    // true. The caller that asked for the chain is already served by its own
    // `Have` reservation above and by reading the inventory, not `available`.
    //
    // Inert on its own — a plan with no chains has nothing to reserve — so
    // this is the other half of chaining a share rather than a change in its
    // own right.
    if let Some(before) = produce_before {
        for (item, count) in ctx.state.item_totals() {
            let gained = count.saturating_sub(before.get(&item).copied().unwrap_or(0));
            if gained > 0 {
                ctx.state
                    .reserve(&Holder::Bot(ctx.chain_actor), &item, gained);
            }
        }
    }
    result
}

/// Walk one method's steps, expanding subgoals, emitting actions and holding
/// each satisfied subgoal's produce for the action that asked for it.
///
/// **Why the reservations.** A method's `Have` subgoal exists because an
/// action further down its own step list needs that holding — `HandCraft`
/// asks for every ingredient of the craft it is about to emit, `Smelt` for the
/// ore, the coal and the furnace it is about to load. But the action does not
/// *spend* any of it until the `Step::Act` at the end, so between the subgoal
/// being satisfied and the action being emitted the items sit in the simulated
/// inventory looking spare — and the next sibling subgoal, which asks
/// `shortfall` whether it is already supplied, helps itself to them.
///
/// That is the shared-intermediate defect. A lab needs ten iron gear wheels
/// and four transport belts; the belts are made of gears; the belt sub-goal
/// finds the lab's own ten gears sitting there, plans no gears of its own, and
/// spends two — so the lab craft comes to eight and the expansion fails
/// arithmetic it should have got right. Reserving each subgoal's stated count
/// as soon as it is satisfied is what makes the sibling see the two gears it
/// really has to make.
///
/// The count reserved is the subgoal's own `count`, not its shortfall: the
/// action's `Condition::HasItem` demands the whole holding, so the whole
/// holding is spoken for, whether it was produced here or was already in hand.
///
/// A `Goal::All` reserves nothing — it never reaches here, returning from
/// `expand_goal_body` above. That is deliberate: its members are independent
/// goals with no action of the caller's waiting to consume them together, and
/// a `Have` states a holding rather than a delivery, so two members asking for
/// five gears each still describe one bot holding five.
fn run_steps(
    steps: Vec<Step>,
    ctx: &mut ExpansionCtx,
    net: &mut ActionNetwork,
    registry: &MethodRegistry,
    promised: &mut Vec<(Holder, ItemId, u32)>,
) -> Result<(), PlannerError> {
    for step in steps {
        match step {
            Step::Subgoal(g) => {
                expand_goal(&g, ctx, net, registry)?;
                if let Goal::Have { item, count, whose } = &g {
                    ctx.state.reserve(whose, item, *count);
                    promised.push((whose.clone(), item.clone(), *count));
                }
            }
            Step::Act(action) => {
                // Simulate against the chain actor so later siblings see this
                // action's results. The emitted action stays unpinned.
                //
                // The reservations above are still held here, and must be:
                // `Effect::LoseItem` debits the *inventory*, which reservations
                // do not touch, so the two ledgers cannot disagree — the items
                // leave the inventory for real and the promise is given back
                // when this method's body ends.
                let binding = ctx.chain_actor;
                for effect in &action.eff {
                    effect.apply(&mut ctx.state, binding)?;
                }
                let id = net.add(*action);
                // Stamp it with the chain it was expanded under, so the
                // scheduler keeps the chain together. Outside a per-bot
                // subtree there is no chain and the action stays free.
                if let Some(chain) = ctx.chain {
                    net.set_chain(id, chain);
                }
            }
            Step::Link { from, to, lag } => net.link(from, to, lag),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{Goal, Holder};
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn ctx() -> ExpansionCtx {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        ExpansionCtx::new(state, BotId(1))
    }

    /// A method that claims every `Have` goal and expands to nothing.
    struct Nothing;
    impl Method for Nothing {
        fn name(&self) -> &'static str {
            "nothing"
        }
        fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
            matches!(goal, Goal::Have { .. })
        }
        fn expand(&self, _goal: &Goal, _ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
            Ok(vec![])
        }
    }

    #[test]
    fn the_registry_finds_an_applicable_method() {
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let c = ctx();
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert_eq!(
            reg.find(&goal, &c.state, GoalSite::root())
                .map(|m| m.name()),
            Some("nothing")
        );
    }

    #[test]
    fn the_registry_returns_none_when_nothing_applies() {
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let c = ctx();
        assert!(
            reg.find(
                &Goal::Researched("automation".into()),
                &c.state,
                GoalSite::root()
            )
            .is_none()
        );
    }

    #[test]
    fn the_registry_prefers_the_first_registered_applicable_method() {
        struct Other;
        impl Method for Other {
            fn name(&self) -> &'static str {
                "other"
            }
            fn applicable(&self, _goal: &Goal, _state: &PlanState) -> bool {
                true
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![])
            }
        }
        let reg = MethodRegistry::new()
            .with(Box::new(Nothing))
            .with(Box::new(Other));
        let c = ctx();
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert_eq!(
            reg.find(&goal, &c.state, GoalSite::root())
                .map(|m| m.name()),
            Some("nothing")
        );
    }

    #[test]
    fn the_context_allocates_ascending_action_ids() {
        let mut c = ctx();
        let first = c.ids.next();
        let second = c.ids.next();
        assert!(first < second);
    }

    #[test]
    fn the_registry_skips_an_inapplicable_method_and_falls_through() {
        /// Declines everything, so the registry must keep looking.
        struct Declines;
        impl Method for Declines {
            fn name(&self) -> &'static str {
                "declines"
            }
            fn applicable(&self, _goal: &Goal, _state: &PlanState) -> bool {
                false
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                unreachable!("an inapplicable method must never be expanded")
            }
        }
        let reg = MethodRegistry::new()
            .with(Box::new(Declines))
            .with(Box::new(Nothing));
        let c = ctx();
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert_eq!(
            reg.find(&goal, &c.state, GoalSite::root())
                .map(|m| m.name()),
            Some("nothing")
        );
    }

    use crate::action::{Action, ActionKind, Actor, Effect};

    fn gain_action(ctx: &mut ExpansionCtx, item: &str, count: u32) -> Action {
        Action {
            id: ctx.ids.next(),
            kind: ActionKind::Craft {
                item: item.into(),
                count,
            },
            pre: vec![],
            eff: vec![Effect::GainItem {
                who: Actor::Role,
                item: item.into(),
                count,
            }],
            duration: 60,
            pinned: None,
            label: format!("make {} {}", count, item),
        }
    }

    /// Expands `Have` into one action that produces the requested count.
    struct Produce;
    impl Method for Produce {
        fn name(&self) -> &'static str {
            "produce"
        }
        fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
            matches!(goal, Goal::Have { .. })
        }
        fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
            let Goal::Have { item, count, .. } = goal else {
                unreachable!()
            };
            let a = gain_action(ctx, item, *count);
            Ok(vec![Step::Act(Box::new(a))])
        }
    }

    #[test]
    fn the_driver_turns_a_goal_into_a_network() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let goal = Goal::Have {
            item: "coal".into(),
            count: 3,
            whose: Holder::Anyone,
        };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 1);
    }

    #[test]
    fn the_driver_advances_its_state_so_siblings_see_earlier_effects() {
        // The two goals ask for different counts, so an implementation that
        // merely deduplicated equal `Goal` values (without any state
        // simulation) would also produce two actions here — that shortcut is
        // ruled out by checking the second action's count below, which is
        // only correct if the driver's state genuinely carries the first
        // goal's 3 coal forward before `Satisfied` is asked about the second.
        struct Satisfied;
        impl Method for Satisfied {
            fn name(&self) -> &'static str {
                "satisfied"
            }
            fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
                match goal {
                    Goal::Have { item, count, .. } => state.total_count(item) >= *count,
                    _ => false,
                }
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![])
            }
        }
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Satisfied))
            .with(Box::new(Produce));
        let net = expand(
            &[
                Goal::Have {
                    item: "coal".into(),
                    count: 3,
                    whose: Holder::Anyone,
                },
                Goal::Have {
                    item: "coal".into(),
                    count: 5,
                    whose: Holder::Anyone,
                },
            ],
            &state,
            &reg,
            BotId(1),
        )
        .unwrap();
        // The first goal produces 3. The second is not a duplicate and is not
        // satisfied, so it produces too — but only because the driver's state
        // actually carries the first goal's 3 coal forward.
        assert_eq!(net.len(), 2);
    }

    /// The driver holds a satisfied subgoal's produce for the action that
    /// asked for it, so a *sibling* subgoal cannot count the same items
    /// towards itself.
    ///
    /// Written with invented items and hand-written methods rather than a
    /// recipe, because the rule is the driver's and not any method's: a widget
    /// needs four cogs and a gadget, and a gadget is itself a cog. Without the
    /// reservation the gadget's own cog subgoal looks at the four cogs the
    /// widget just had made, declares itself supplied, and spends one — and
    /// the widget is left with three of the four it was promised.
    #[test]
    fn a_sibling_subgoal_cannot_spend_what_an_earlier_one_was_asked_to_supply() {
        /// Satisfied when the holder can still count `count` towards this
        /// goal — the reservation-aware question, which is the one a method
        /// has to ask.
        struct Enough;
        impl Method for Enough {
            fn name(&self) -> &'static str {
                "enough"
            }
            fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, count, whose }
                    if state.available(whose, item) >= *count)
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![])
            }
        }

        /// Makes exactly the shortfall, out of nothing.
        struct MakeCog;
        impl Method for MakeCog {
            fn name(&self) -> &'static str {
                "make-cog"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "cog")
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have { item, count, whose } = goal else {
                    unreachable!()
                };
                let short = count.saturating_sub(ctx.state.available(whose, item));
                let a = gain_action(ctx, "cog", short);
                Ok(vec![Step::Act(Box::new(a))])
            }
        }

        /// Turns one cog into one gadget, spending the cog.
        struct MakeGadget;
        impl Method for MakeGadget {
            fn name(&self) -> &'static str {
                "make-gadget"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "gadget")
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                let mut a = gain_action(ctx, "gadget", 1);
                a.eff.push(Effect::LoseItem {
                    who: Actor::Role,
                    item: "cog".into(),
                    count: 1,
                });
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "cog".into(),
                        count: 1,
                        whose: Holder::Anyone,
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }

        /// Four cogs *and* a gadget, both spent.
        struct MakeWidget;
        impl Method for MakeWidget {
            fn name(&self) -> &'static str {
                "make-widget"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "widget")
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                let mut a = gain_action(ctx, "widget", 1);
                a.eff.push(Effect::LoseItem {
                    who: Actor::Role,
                    item: "cog".into(),
                    count: 4,
                });
                a.eff.push(Effect::LoseItem {
                    who: Actor::Role,
                    item: "gadget".into(),
                    count: 1,
                });
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "cog".into(),
                        count: 4,
                        whose: Holder::Anyone,
                    }),
                    Step::Subgoal(Goal::Have {
                        item: "gadget".into(),
                        count: 1,
                        whose: Holder::Anyone,
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }

        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(MakeWidget))
            .with(Box::new(MakeGadget))
            .with(Box::new(MakeCog));
        let net = expand(
            &[Goal::Have {
                item: "widget".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &state,
            &reg,
            BotId(1),
        )
        .expect("five cogs is a reachable amount of cogs");

        let cogs: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count } if item == "cog" => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(
            cogs,
            5,
            "four cogs for the widget and one more for its gadget: {:?}",
            net.actions().map(|a| &a.label).collect::<Vec<_>>()
        );
    }

    /// A reservation lasts exactly as long as the method that made it. Once
    /// its action has been emitted — and has spent the items for real — the
    /// promise is given back, or every later goal in the plan would be sized
    /// against stock that is permanently invisible and the plan would grow
    /// without bound.
    ///
    /// Two independent top-level goals are the shortest way to say it: the
    /// second is asked for after the first's method has finished, and finds
    /// exactly the two cogs the first left over.
    #[test]
    fn a_reservation_ends_with_the_method_that_made_it() {
        struct Enough;
        impl Method for Enough {
            fn name(&self) -> &'static str {
                "enough"
            }
            fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, count, whose }
                    if state.available(whose, item) >= *count)
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![])
            }
        }

        /// Asks for six cogs and spends four of them, leaving two.
        struct Spend;
        impl Method for Spend {
            fn name(&self) -> &'static str {
                "spend"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "widget")
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                let mut a = gain_action(ctx, "widget", 1);
                a.eff.push(Effect::LoseItem {
                    who: Actor::Role,
                    item: "cog".into(),
                    count: 4,
                });
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "cog".into(),
                        count: 6,
                        whose: Holder::Anyone,
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }

        struct MakeCog;
        impl Method for MakeCog {
            fn name(&self) -> &'static str {
                "make-cog"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "cog")
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have { item, count, whose } = goal else {
                    unreachable!()
                };
                let short = count.saturating_sub(ctx.state.available(whose, item));
                let a = gain_action(ctx, "cog", short);
                Ok(vec![Step::Act(Box::new(a))])
            }
        }

        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(Spend))
            .with(Box::new(MakeCog));
        let net = expand(
            &[
                Goal::Have {
                    item: "widget".into(),
                    count: 1,
                    whose: Holder::Anyone,
                },
                Goal::Have {
                    item: "cog".into(),
                    count: 2,
                    whose: Holder::Anyone,
                },
            ],
            &state,
            &reg,
            BotId(1),
        )
        .expect("two leftover cogs satisfy a goal asking for two");

        let cogs: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count } if item == "cog" => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(
            cogs,
            6,
            "the widget's six cogs, with the two it did not spend left free              for the second goal rather than promised forever: {:?}",
            net.actions().map(|a| &a.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_driver_recurses_through_subgoals() {
        struct ViaSubgoal;
        impl Method for ViaSubgoal {
            fn name(&self) -> &'static str {
                "via-subgoal"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "iron-gear-wheel")
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                let a = gain_action(ctx, "iron-gear-wheel", 1);
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "iron-plate".into(),
                        count: 2,
                        whose: Holder::Anyone,
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new()
            .with(Box::new(ViaSubgoal))
            .with(Box::new(Produce));
        let goal = Goal::Have {
            item: "iron-gear-wheel".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 2, "the subgoal's action and the gear itself");
    }

    #[test]
    fn an_unclaimed_goal_is_an_error() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let result = expand(
            &[Goal::Researched("automation".into())],
            &state,
            &reg,
            BotId(1),
        );
        assert!(matches!(
            result,
            Err(PlannerError::NoApplicableMethod { .. })
        ));
    }

    #[test]
    fn producing_has_no_method_in_this_increment() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let goal = Goal::Producing {
            item: "automation-science-pack".into(),
            rate: 150.0,
        };
        assert!(matches!(
            expand(&[goal], &state, &reg, BotId(1)),
            Err(PlannerError::NoApplicableMethod { .. })
        ));
    }

    #[test]
    fn runaway_recursion_is_an_error_not_a_hang() {
        struct Forever;
        impl Method for Forever {
            fn name(&self) -> &'static str {
                "forever"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { .. })
            }
            fn expand(
                &self,
                goal: &Goal,
                _c: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![Step::Subgoal(goal.clone())])
            }
        }
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Forever));
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert!(matches!(
            expand(&[goal], &state, &reg, BotId(1)),
            Err(PlannerError::ExpansionTooDeep { .. })
        ));
    }

    #[test]
    fn expanding_against_a_bot_the_state_does_not_know_is_an_error() {
        // `registry_for(bots)`, `expand(.., chain_actor)` and `schedule(.., bots)`
        // each take a roster and nothing reconciles them. A bot the state does
        // not know has no position and no inventory, so the methods would plan
        // for a default one standing at the origin — a plan sited nowhere near
        // the bot that has to run it. A Lua caller will make this mistake.
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        assert!(matches!(
            expand(std::slice::from_ref(&goal), &state, &reg, BotId(9)),
            Err(PlannerError::UnknownBot(BotId(9)))
        ));
        // And the same for a roster a method decomposes with: the state knows
        // only bot 1, so bot 2's share cannot be planned.
        let bots = [BotId(1), BotId(2)];
        assert!(matches!(
            expand(
                &[Goal::Have {
                    item: "iron-ore".into(),
                    count: 4,
                    whose: Holder::Anyone,
                }],
                &state,
                &crate::method::have::registry_for(&bots),
                BotId(1),
            ),
            Err(PlannerError::UnknownBot(BotId(2)))
        ));
        // The bot it does know is still fine.
        assert!(expand(&[goal], &state, &reg, BotId(1)).is_ok());
    }

    #[test]
    fn a_bot_addressed_goal_rebinds_the_chain_actor() {
        // `Record` reports which actor the driver was simulating against.
        use std::cell::RefCell;
        use std::rc::Rc;
        struct Record(Rc<RefCell<Vec<BotId>>>);
        impl Method for Record {
            fn name(&self) -> &'static str {
                "record"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { .. })
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                self.0.borrow_mut().push(ctx.chain_actor);
                Ok(vec![])
            }
        }
        let seen = Rc::new(RefCell::new(Vec::new()));
        let reg = MethodRegistry::new().with(Box::new(Record(seen.clone())));
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let goals = vec![
            Goal::Have {
                item: "coal".into(),
                count: 1,
                whose: Holder::Bot(BotId(2)),
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
            },
        ];
        expand(&goals, &state, &reg, BotId(1)).unwrap();
        assert_eq!(
            *seen.borrow(),
            vec![BotId(2), BotId(1)],
            "a Bot-addressed goal rebinds, and the binding is restored afterwards"
        );
    }

    /// The same, for the goal kind the driver used to read `whose` off nothing
    /// but `Have`. A trigger technology's work is a `Produced`, and it carries
    /// the identical `whose` with the identical meaning.
    #[test]
    fn a_production_goal_rebinds_the_chain_actor_too() {
        use std::cell::RefCell;
        use std::rc::Rc;
        struct Record(Rc<RefCell<Vec<BotId>>>);
        impl Method for Record {
            fn name(&self) -> &'static str {
                "record"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Produced { .. })
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                self.0.borrow_mut().push(ctx.chain_actor);
                Ok(vec![])
            }
        }
        let seen = Rc::new(RefCell::new(Vec::new()));
        let reg = MethodRegistry::new().with(Box::new(Record(seen.clone())));
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let goals = vec![
            Goal::Produced {
                item: "coal".into(),
                count: 1,
                whose: Holder::Share(BotId(2)),
                unlocks: None,
            },
            Goal::Produced {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
                unlocks: None,
            },
        ];
        expand(&goals, &state, &reg, BotId(1)).unwrap();
        assert_eq!(
            *seen.borrow(),
            vec![BotId(2), BotId(1)],
            "a Share-addressed production rebinds, and the binding is restored"
        );
    }

    /// Expands a `Have` into one action per unit, so a subtree has more than
    /// one action to compare chains across.
    struct ProduceEach;
    impl Method for ProduceEach {
        fn name(&self) -> &'static str {
            "produce-each"
        }
        fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
            matches!(goal, Goal::Have { .. } | Goal::Produced { .. })
        }
        fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
            let (Goal::Have { item, count, .. } | Goal::Produced { item, count, .. }) = goal else {
                unreachable!()
            };
            Ok((0..*count)
                .map(|_| Step::Act(Box::new(gain_action(ctx, item, 1))))
                .collect())
        }
    }

    #[test]
    fn a_bot_addressed_goals_actions_all_share_one_chain() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new().with(Box::new(ProduceEach));
        let goal = Goal::Have {
            item: "coal".into(),
            count: 3,
            whose: Holder::Bot(BotId(2)),
        };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 3);
        let chains: Vec<Option<_>> = net.actions().map(|a| net.chain_of(a.id)).collect();
        assert!(chains[0].is_some(), "a per-bot subtree opens a chain");
        assert!(
            chains.iter().all(|c| *c == chains[0]),
            "one chain for the whole subtree, got {:?}",
            chains
        );
    }

    /// **The live-run defect, at the level it was introduced.**
    ///
    /// `Researched` asks for a `craft-item` trigger's work as
    /// `Produced { whose: Holder::Share(chain_actor) }`, saying in its own
    /// comment that it does so "for the same reason the pack bill uses it: one
    /// action reading one bot's inventory". The driver read `whose` off
    /// `Goal::Have` alone, so that claim bought nothing: the production's
    /// actions were left freely assignable while the `Have` subgoals beneath
    /// them opened chains of their own. On a four-bot run the scheduler duly
    /// mined 42 iron ore onto one bot and offered `insert 50 iron-ore` to
    /// another, and the plan died on `precondition has 50 iron-ore ... does not
    /// hold for bot 2`.
    #[test]
    fn a_share_addressed_production_goals_actions_all_share_one_chain() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new().with(Box::new(ProduceEach));
        let goal = Goal::Produced {
            item: "coal".into(),
            count: 3,
            whose: Holder::Share(BotId(2)),
            unlocks: None,
        };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 3);
        let chains: Vec<Option<_>> = net.actions().map(|a| net.chain_of(a.id)).collect();
        assert!(
            chains[0].is_some(),
            "a production stated against one inventory opens a chain"
        );
        assert!(
            chains.iter().all(|c| *c == chains[0]),
            "one chain for the whole subtree, got {:?}",
            chains
        );
    }

    /// The control for the test above: `Holder::Anyone` states nothing about
    /// whose inventory, so it must still weld nothing. Without this, welding
    /// every `Produced` would pass the test above just as happily.
    #[test]
    fn a_production_anyone_can_satisfy_opens_no_chain() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new().with(Box::new(ProduceEach));
        let goal = Goal::Produced {
            item: "coal".into(),
            count: 3,
            whose: Holder::Anyone,
            unlocks: None,
        };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert!(
            net.actions().all(|a| net.chain_of(a.id).is_none()),
            "nothing has to gather, so nothing is welded"
        );
    }

    #[test]
    fn two_bot_addressed_goals_get_different_chains() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new().with(Box::new(ProduceEach));
        let goals = vec![
            Goal::Have {
                item: "coal".into(),
                count: 1,
                whose: Holder::Bot(BotId(1)),
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Bot(BotId(2)),
            },
        ];
        let net = expand(&goals, &state, &reg, BotId(1)).unwrap();
        let chains: Vec<Option<_>> = net.actions().map(|a| net.chain_of(a.id)).collect();
        assert_eq!(chains.len(), 2);
        assert!(chains.iter().all(|c| c.is_some()));
        assert_ne!(
            chains[0], chains[1],
            "independent per-bot goals must not share a chain"
        );
    }

    #[test]
    fn a_nested_bot_addressed_subgoal_stays_in_its_parents_chain() {
        // `whose` is propagated into subgoals, so the ingredient goal is
        // `Holder::Bot` too. It must extend the chain, not start a new one:
        // splitting here is exactly how a branching recipe ends up scattered.
        struct ViaSubgoal;
        impl Method for ViaSubgoal {
            fn name(&self) -> &'static str {
                "via-subgoal"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "iron-gear-wheel")
            }
            fn expand(&self, g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have { whose, .. } = g else {
                    unreachable!()
                };
                let a = gain_action(ctx, "iron-gear-wheel", 1);
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "iron-plate".into(),
                        count: 2,
                        whose: whose.clone(),
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new()
            .with(Box::new(ViaSubgoal))
            .with(Box::new(ProduceEach));
        let goal = Goal::Have {
            item: "iron-gear-wheel".into(),
            count: 1,
            whose: Holder::Bot(BotId(2)),
        };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        let chains: Vec<Option<_>> = net.actions().map(|a| net.chain_of(a.id)).collect();
        assert!(chains[0].is_some());
        assert!(
            chains.iter().all(|c| *c == chains[0]),
            "the ingredient and its consumer share a chain, got {:?}",
            chains
        );
    }

    #[test]
    fn the_chain_is_restored_after_a_bot_addressed_subtree() {
        // The mirror of `a_bot_addressed_goal_rebinds_the_chain_actor`: the
        // second goal is not addressed to anyone, so it must come back out of
        // the chain the first one opened.
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new().with(Box::new(ProduceEach));
        let goals = vec![
            Goal::Have {
                item: "coal".into(),
                count: 1,
                whose: Holder::Bot(BotId(2)),
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
            },
        ];
        let net = expand(&goals, &state, &reg, BotId(1)).unwrap();
        let chains: Vec<Option<_>> = net.actions().map(|a| net.chain_of(a.id)).collect();
        assert!(chains[0].is_some(), "the per-bot goal is in a chain");
        assert_eq!(
            chains[1], None,
            "and the goal after it is freely assignable again"
        );
    }

    #[test]
    fn a_failed_expansion_restores_the_chain() {
        struct Fails;
        impl Method for Fails {
            fn name(&self) -> &'static str {
                "fails"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { .. })
            }
            fn expand(&self, g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Err(PlannerError::NoApplicableMethod {
                    goal: g.to_string(),
                })
            }
        }
        let reg = MethodRegistry::new().with(Box::new(Fails));
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let mut net = ActionNetwork::new();
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Bot(BotId(2)),
        };
        assert!(expand_goal(&goal, &mut ctx, &mut net, &reg).is_err());
        assert_eq!(ctx.chain, None, "the chain must survive an error");
    }

    #[test]
    fn all_expands_each_of_its_goals() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let goal = Goal::All(vec![
            Goal::Have {
                item: "coal".into(),
                count: 1,
                whose: Holder::Anyone,
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
            },
        ]);
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 2);
    }

    #[test]
    fn a_failed_expansion_restores_the_context() {
        /// Always claims the goal, always fails.
        struct Fails;
        impl Method for Fails {
            fn name(&self) -> &'static str {
                "fails"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { .. })
            }
            fn expand(
                &self,
                goal: &Goal,
                _c: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                Err(PlannerError::NoApplicableMethod {
                    goal: goal.to_string(),
                })
            }
        }
        let reg = MethodRegistry::new().with(Box::new(Fails));
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let mut net = ActionNetwork::new();
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Bot(BotId(2)),
        };

        assert!(expand_goal(&goal, &mut ctx, &mut net, &reg).is_err());
        assert_eq!(
            ctx.chain_actor,
            BotId(1),
            "the binding must survive an error"
        );
        assert_eq!(ctx.depth, 0, "the depth must survive an error");
    }

    /// T4 -- asymmetric end to end, through the public `expand()`.
    ///
    /// Four bots, only bot 1 holding a freeplay-like starting inventory (8
    /// iron plates, a furnace, a drill, a wood), goal `Have{iron-plate, 40,
    /// Anyone}`. Bots differ in the goal's own item, which is exactly the
    /// shape `check_bots_interchangeable` used to refuse -- the guard's own
    /// five tests described it as the one shape that was not safe to let
    /// through. This is the permanent replacement for that guard: not just
    /// that `expand()` succeeds, but that the resulting network still
    /// schedules to a plan that actually uses the roster's spare capacity,
    /// which is the property the guard's removal put at risk (see
    /// `docs/superpowers/specs/2026-09-01-per-bot-share-sizing-design.md` §4).
    ///
    /// A lower-level probe of the same scenario -- calling `expand_goal`
    /// directly to reach expansion underneath the guard while it still
    /// existed -- passed before this deletion; see the "pin that an
    /// asymmetric roster survives expand and schedule" commit. That probe is
    /// superseded by this test once the guard is gone: `expand()` itself now
    /// takes the asymmetric roster, so there is nothing left to bypass.
    #[test]
    fn a_freeplay_roster_plans_and_schedules() {
        use crate::schedule::schedule;

        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        state.gain(BotId(1), "iron-plate", 8);
        state.gain(BotId(1), "stone-furnace", 1);
        state.gain(BotId(1), "burner-mining-drill", 1);
        state.gain(BotId(1), "wood", 1);

        let goal = Goal::Have {
            item: "iron-plate".into(),
            count: 40,
            whose: Holder::Anyone,
        };
        let registry = have::registry_for(&bots);
        let net =
            expand(&[goal], &state, &registry, BotId(1)).expect("an asymmetric roster expands");
        let plan = schedule(&net, &state, &bots).expect("an asymmetric roster schedules");
        assert!(plan.makespan > 0, "a real plan takes real time");

        // A real assertion about the plan's shape, not just that scheduling
        // returned `Ok`: the roster's spare capacity actually gets used
        // rather than every share landing on bot 1 regardless of its head
        // start.
        let participating = bots
            .iter()
            .filter(|b| !plan.steps_for(**b).is_empty())
            .count();
        assert!(
            participating > 1,
            "an asymmetric roster's plan should spread work across more than \
             one bot, got steps: {:?}",
            plan.steps
        );
    }

    /// A `Holder::Share` states that the holding has to end up in one
    /// inventory, and the driver makes that true by opening a chain over its
    /// subtree — and, since 2026-09-02, by binding that chain's ownership to
    /// the bot the share was sized against.
    ///
    /// Written with hand-made methods and invented items so the rule is the
    /// driver's and not any recipe's: a widget is made from a cog and a spring
    /// by three separate actions, none of which needs two of them together, so
    /// no method here `converges`. Without the chain the three actions are
    /// individually assignable and the scheduler is free to put the cog on one
    /// bot and the spring on another — which is exactly `Smelt`'s shape, where
    /// the ore, the coal and the furnace each feed an action of their own.
    #[test]
    fn a_share_welds_its_whole_subtree_to_one_chain() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));

        let shared = expand(
            &[Goal::Have {
                item: "cog".into(),
                count: 3,
                whose: Holder::Share(BotId(1)),
            }],
            &state,
            &reg,
            BotId(1),
        )
        .expect("expands");
        let chains: BTreeSet<Option<ChainId>> =
            shared.actions().map(|a| shared.chain_of(a.id)).collect();
        assert_eq!(chains.len(), 1, "one share, one chain, got {chains:?}");
        assert!(
            chains.iter().all(Option::is_some),
            "a share's actions must carry a chain, got {chains:?}"
        );
        let chain = shared
            .chain_of(shared.actions().next().expect("an action").id)
            .expect("just asserted");
        assert_eq!(
            shared.owner_of(chain),
            Some(BotId(1)),
            "a share sizes against a bot, and since 2026-09-02 also runs on \
             it -- see the owner-binding comment in expand_goal_body"
        );

        // The contrast: `Holder::Anyone` says the roster may hold it between
        // them, and opens nothing.
        let scattered = expand(
            &[Goal::Have {
                item: "cog".into(),
                count: 3,
                whose: Holder::Anyone,
            }],
            &state,
            &reg,
            BotId(1),
        )
        .expect("expands");
        assert!(
            scattered
                .actions()
                .all(|a| scattered.chain_of(a.id).is_none()),
            "nothing has to be gathered, so nothing is welded"
        );
    }

    /// A chain's produce is not stock a *sibling* chain may count on.
    ///
    /// Expansion simulates every chain into the same notional inventory while
    /// the scheduler may put two chains on two bots, so a sibling that sizes
    /// itself against what another chain made plans work it cannot do. This is
    /// the ledger half of chaining a share; `a_share_welds_…` is the other.
    #[test]
    fn what_one_chain_made_is_not_offered_to_the_next() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(Produce));

        // Two shares of five cogs each, deliberately sized against the *same*
        // bot. That is what makes this bite: the second share asks whether
        // that bot can already count five towards it, and the five the first
        // chain made are sitting right there in the simulated inventory. Two
        // different bots would have proved nothing — the second would have
        // looked at an empty inventory whatever the ledger said.
        let net = expand(
            &[
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                },
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                },
            ],
            &state,
            &reg,
            BotId(1),
        )
        .expect("expands");
        let made: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count } if item == "cog" => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(
            made, 10,
            "each share makes its own five: the first chain's cogs may end up in \
             hands the second chain never reaches"
        );
    }

    /// With one bot there is nothing to guard against, so nothing is guarded.
    ///
    /// The ledger above exists because two chains may be handed to two
    /// runners. A roster of one cannot do that: what the first chain made is
    /// in the only pair of hands there is, and hiding it would send that bot
    /// out to make a second five for no reason. Same two goals as
    /// `what_one_chain_made_…`, one bot instead of two, opposite answer.
    #[test]
    fn one_bot_may_count_what_its_own_earlier_chain_made() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(Produce));
        let net = expand(
            &[
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                },
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                },
            ],
            &state,
            &reg,
            BotId(1),
        )
        .expect("expands");
        let made: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count } if item == "cog" => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(
            made, 5,
            "one bot, one pair of hands: the second share is already holding what \
             it asked for"
        );
    }

    /// Surplus a subgoal produced is still the *chain's* to spend, so a later
    /// subgoal of the same chain may count it.
    ///
    /// The chain ledger is written once, when the chain closes — not at every
    /// frame beneath it. Writing it per frame would hide a subgoal's overshoot
    /// from its own siblings and make the chain buy the same thing twice, and
    /// that is invisible unless something over-produces: `ProducePairs` makes
    /// cogs two at a time, so asking for one leaves one spare.
    #[test]
    fn a_chains_own_surplus_is_still_spendable_inside_that_chain() {
        /// One widget out of two separate one-cog subgoals.
        struct Widget;
        impl Method for Widget {
            fn name(&self) -> &'static str {
                "widget"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "widget")
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have { whose, .. } = goal else {
                    unreachable!()
                };
                let cog = |whose: &Holder| {
                    Step::Subgoal(Goal::Have {
                        item: "cog".into(),
                        count: 1,
                        whose: whose.clone(),
                    })
                };
                let a = gain_action(ctx, "widget", 1);
                Ok(vec![cog(whose), cog(whose), Step::Act(Box::new(a))])
            }
        }

        /// Cogs come two to a run, so a shortfall of one leaves one spare.
        struct ProducePairs;
        impl Method for ProducePairs {
            fn name(&self) -> &'static str {
                "produce-pairs"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "cog")
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have { item, count, whose } = goal else {
                    unreachable!()
                };
                let short = count.saturating_sub(ctx.state.available(whose, item));
                let runs = short.div_ceil(2);
                let a = gain_action(ctx, "cog", runs * 2);
                Ok(vec![Step::Act(Box::new(a))])
            }
        }

        // Two bots, because the chain ledger is only written for a roster
        // that can put two chains on two runners.
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(Widget))
            .with(Box::new(ProducePairs));
        let net = expand(
            &[Goal::Have {
                item: "widget".into(),
                count: 1,
                whose: Holder::Share(BotId(1)),
            }],
            &state,
            &reg,
            BotId(1),
        )
        .expect("expands");
        let cogs: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count } if item == "cog" => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(
            cogs, 2,
            "one run of two cogs covers both subgoals: the spare from the first \
             is still the chain's, and only stops being available to *other* \
             chains once this one closes"
        );
    }

    /// Satisfied when the holder can still count `count` towards this goal.
    struct Enough;
    impl Method for Enough {
        fn name(&self) -> &'static str {
            "enough"
        }
        fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
            matches!(goal, Goal::Have { item, count, whose }
                if state.available(whose, item) >= *count)
        }
        fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
            Ok(vec![])
        }
    }
}
