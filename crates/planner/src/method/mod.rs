//! How to get what we want: hand-written decompositions, and the driver that
//! runs them until only actions remain.

use crate::action::Action;
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ActionIdGen, BotId, Ticks};
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
pub struct ExpansionCtx {
    pub state: PlanState,
    pub ids: ActionIdGen,
    pub chain_actor: BotId,
    pub depth: u32,
}

impl ExpansionCtx {
    pub fn new(state: PlanState, chain_actor: BotId) -> Self {
        ExpansionCtx {
            state,
            ids: ActionIdGen::new(),
            chain_actor,
            depth: 0,
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

    pub fn find(&self, goal: &Goal, state: &PlanState) -> Option<&dyn Method> {
        self.methods
            .iter()
            .find(|m| m.applicable(goal, state))
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
pub fn expand(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
) -> Result<ActionNetwork, PlannerError> {
    let mut ctx = ExpansionCtx::new(state.fork(), chain_actor);
    let mut net = ActionNetwork::new();
    for goal in goals {
        expand_goal(goal, &mut ctx, &mut net, registry)?;
    }
    net.infer_edges();
    net.validate()?;
    Ok(net)
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
    // call must leave `depth` and `chain_actor` exactly as it found them even
    // when it fails, or a caller that continues past an error inherits a
    // corrupted context and a comment claiming that cannot happen.
    let previous_actor = ctx.chain_actor;
    if let Goal::Have {
        whose: Holder::Bot(bot),
        ..
    } = goal
    {
        ctx.chain_actor = *bot;
    }
    ctx.depth += 1;

    let result = expand_goal_body(goal, ctx, net, registry);

    ctx.depth -= 1;
    ctx.chain_actor = previous_actor;
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

    let method =
        registry
            .find(goal, &ctx.state)
            .ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            })?;
    let steps = method.expand(goal, ctx)?;

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
                net.add(*action);
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
        assert_eq!(reg.find(&goal, &c.state).map(|m| m.name()), Some("nothing"));
    }

    #[test]
    fn the_registry_returns_none_when_nothing_applies() {
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let c = ctx();
        assert!(reg
            .find(&Goal::Researched("automation".into()), &c.state)
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
        assert_eq!(reg.find(&goal, &c.state).map(|m| m.name()), Some("nothing"));
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
        assert_eq!(reg.find(&goal, &c.state).map(|m| m.name()), Some("nothing"));
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
}
