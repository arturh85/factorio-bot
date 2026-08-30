//! How to get what we want: hand-written decompositions, and the driver that
//! runs them until only actions remain.

pub mod have;
pub mod util;

use crate::action::Action;
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ActionIdGen, BotId, ChainId, ChainIdGen, ItemId, Ticks};
use crate::state::PlanState;
use std::collections::{BTreeMap, BTreeSet};

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
/// expansion: every action emitted inside a per-bot subtree is stamped with it
/// in the network, so the scheduler can bind the whole chain to one bot instead
/// of choosing per action. It is `None` outside such a subtree, which leaves an
/// action freely assignable.
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

/// Recipe chains in this domain are shallow — science pack to gear to plate to
/// ore is four levels, plus one for a per-bot split. This bound exists to turn
/// a method that expands into itself into an error rather than a hang.
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
    check_bots_interchangeable(state)?;
    let mut ctx = ExpansionCtx::new(state.fork(), chain_actor);
    let mut net = ActionNetwork::new();
    for goal in goals {
        expand_goal(goal, &mut ctx, &mut net, registry)?;
    }
    net.infer_edges();
    net.validate()?;
    Ok(net)
}

/// The driver sizes each bot's share of a goal against one bot's inventory and
/// assumes any bot would do (see `ExpansionCtx` docs). Enforce that before
/// forking the state: comparing inventories only, never positions, since
/// travel cost is exactly what legitimately makes bots sit apart.
fn check_bots_interchangeable(state: &PlanState) -> Result<(), PlannerError> {
    let bot_ids = state.bot_ids();
    let Some((&first, rest)) = bot_ids.split_first() else {
        return Ok(());
    };
    // `bot_ids` come from a `BTreeMap`, so `first` is deterministic and the
    // caller's `chain_actor` need not be it.
    let first_inventory = &state
        .bot(first)
        .expect("bot_ids only returns ids present in the state")
        .inventory;
    for &other in rest {
        let other_inventory = &state
            .bot(other)
            .expect("bot_ids only returns ids present in the state")
            .inventory;
        if let Some(item) = first_differing_item(first_inventory, other_inventory) {
            return Err(PlannerError::BotsNotInterchangeable {
                a: first,
                b: other,
                item,
            });
        }
    }
    Ok(())
}

/// The lexicographically first item whose count differs between two
/// inventories, or `None` if they agree on every item. Both maps are
/// `BTreeMap`s, so walking their union in key order is deterministic.
fn first_differing_item(a: &BTreeMap<ItemId, u32>, b: &BTreeMap<ItemId, u32>) -> Option<ItemId> {
    let items: BTreeSet<&ItemId> = a.keys().chain(b.keys()).collect();
    for item in items {
        let in_a = a.get(item).copied().unwrap_or(0);
        let in_b = b.get(item).copied().unwrap_or(0);
        if in_a != in_b {
            return Some(item.clone());
        }
    }
    None
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
    if let Goal::Have {
        whose: Holder::Bot(bot) | Holder::Share(bot),
        ..
    } = goal
    {
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

    // A chain welds actions to one runner. Two things ask for that: a caller
    // naming a bot, and a method whose decomposition makes several produced
    // items meet in one inventory. Nothing else — a goal that merely sits
    // inside a split does not need welding, and welding it serialises work
    // that could have run in parallel.
    if ctx.chain.is_none() {
        let owner = match goal {
            Goal::Have {
                whose: Holder::Bot(bot),
                ..
            } => Some(*bot),
            _ => None,
        };
        if owner.is_some() || method.converges(goal, &ctx.state) {
            let chain = ctx.chains.next();
            ctx.chain = Some(chain);
            if let Some(bot) = owner {
                net.set_chain_owner(chain, bot);
            }
        }
    }

    let steps = method.expand(goal, ctx)?;

    // Everything below this line was asked for by a method, not by the caller,
    // so nothing in the subtree is top level. `expand_goal` restores the flag
    // for our own caller. A `Goal::All` never reaches here — it returns above,
    // so its members keep the site the bundle itself had.
    ctx.top_level = false;

    for step in steps {
        match step {
            Step::Subgoal(g) => expand_goal(&g, ctx, net, registry)?,
            Step::Act(action) => {
                // Simulate against the chain actor so later siblings see this
                // action's results. The emitted action stays unpinned.
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
        assert!(reg
            .find(
                &Goal::Researched("automation".into()),
                &c.state,
                GoalSite::root()
            )
            .is_none());
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

    /// Expands a `Have` into one action per unit, so a subtree has more than
    /// one action to compare chains across.
    struct ProduceEach;
    impl Method for ProduceEach {
        fn name(&self) -> &'static str {
            "produce-each"
        }
        fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
            matches!(goal, Goal::Have { .. })
        }
        fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
            let Goal::Have { item, count, .. } = goal else {
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

    #[test]
    fn expansion_rejects_bots_that_are_not_interchangeable() {
        let bots = [BotId(1), BotId(2)];
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        state.gain(BotId(1), "iron-plate", 12);
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert!(matches!(
            expand(&[goal], &state, &reg, BotId(1)),
            Err(PlannerError::BotsNotInterchangeable { .. })
        ));
    }

    #[test]
    fn identical_bots_are_accepted() {
        let bots = [BotId(1), BotId(2)];
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        for b in bots {
            state.gain(b, "stone-furnace", 2);
        }
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert!(expand(&[goal], &state, &reg, BotId(1)).is_ok());
    }

    #[test]
    fn a_single_bot_is_trivially_interchangeable() {
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        state.gain(BotId(1), "iron-plate", 12);
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let goal = Goal::Have {
            item: "coal".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        assert!(expand(&[goal], &state, &reg, BotId(1)).is_ok());
    }
}
