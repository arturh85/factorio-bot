//! How to get what we want: hand-written decompositions, and the driver that
//! runs them until only actions remain.

pub mod assemble;
pub mod have;
pub mod power;
pub mod produce;
pub mod util;

use crate::action::Action;
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ActionIdGen, BotId, ChainId, ChainIdGen, ItemId, Ticks};
use crate::state::{ClaimRunner, PlanState};
use std::collections::BTreeMap;

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
    /// A block of steps that belong to **another bot**.
    ///
    /// The driver opens a fresh chain owned by `whose`'s bot, rebinds
    /// `chain_actor` to it, runs `steps` inside it — so every `Step::Act` in
    /// there is stamped with that chain and every `Step::Subgoal` is sized
    /// against that bot's inventory — and then restores everything.
    ///
    /// This is the only way a method can emit an action it does not intend to
    /// run itself, and it exists because material convergence is exactly
    /// that: the insert belongs to the supplier and the take belongs to the
    /// consumer, and both are emitted by one method because only that method
    /// knows the `ActionId`s to link.
    ///
    /// `whose` must name a bot (`Holder::Bot` or `Holder::Share`).
    /// `Holder::Anyone` is refused with
    /// [`PlannerError::UnownedHandover`](crate::error::PlannerError::UnownedHandover)
    /// rather than silently treated as "keep the current chain": a handover
    /// with no named supplier is a method bug, and it should fail where it is
    /// written.
    Owned { whose: Holder, steps: Vec<Step> },
}

/// State threaded through one expansion.
///
/// `chain_actor` is used **only** to simulate effects while expanding, so that
/// a later sibling goal sees what an earlier one produced. It never reaches an
/// emitted action: methods emit `Actor::Role` with `pinned: None`, and the
/// scheduler decides who actually runs each action.
///
/// This no longer rests on bots being interchangeable. A goal that names a
/// bot (`Holder::Bot` or `Holder::Share`) rebinds `chain_actor` to that bot
/// before anything under it is simulated (`expand_goal`), and — since
/// 2026-09-02 — the chain such a goal opens is also *owned* by that same bot
/// (see the owner-binding comment in `expand_goal_body`). So a share's
/// sizing and who ends up running it are pinned to one real inventory by
/// construction, not by an assumption that any bot would experience the same
/// thing.
///
/// What is still a real choice, not a formality, is the `chain_actor` `expand`
/// is *called* with: it is what a goal naming no bot — including
/// `Researched`'s own trigger and pack bills, stated as
/// `Holder::Share(chain_actor)` — is simulated and (through the binding
/// above) run against. Picking a different bot here now sizes and executes
/// those subtrees against a different bot's actual stock, so callers must
/// pick it deterministically rather than arbitrarily. See
/// `crates/executor/src/recover.rs`'s tier 2 for where that choice is made
/// and why.
///
/// `chain` is the other half of the same story, and the half that outlives
/// expansion: every action emitted inside a chained subtree is stamped with it
/// in the network, so the scheduler can bind the whole chain to one bot instead
/// of choosing per action. A subtree is chained when a caller named a bot for
/// it (`Holder::Bot`), when the goal is a `Holder::Share` and so states that
/// its holding ends up in one inventory, or when the method claiming its root
/// `converges` — see `expand_goal_body` — and, since the material-convergence
/// work of 2026-09-02, when a method emits a [`Step::Owned`], which opens a
/// chain owned by the bot that step names. It is `None` outside such a
/// subtree, which leaves an action freely assignable.
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
    /// How many holders may pursue the goal being expanded *at the same time*,
    /// when some method named a limit; `None` when none did.
    ///
    /// Driver-owned like `chain` and `top_level`, and for a sharper reason
    /// than either: the answer is a fact about the whole registry, and a
    /// method computing it for itself would be asking only itself. The driver
    /// holds the registry, so the driver asks — see `expand_goal_body`, which
    /// fills this in at scatter sites and nowhere else.
    ///
    /// Read by exactly one method, `SplitAcrossBots`, which is the only method
    /// that hands one goal to several bots at once. It reads a *number* and
    /// never learns what produced it, which is what keeps a generic
    /// item-splitting method free of any notion of ore.
    pub(crate) concurrency: Option<u32>,
    /// True anywhere beneath a converging method's own expansion — a method
    /// that answered [`Method::split_probe`] and so is about to hand one goal
    /// to several bots.
    ///
    /// The termination argument, and driver-owned for the same reason `chain`
    /// is. A handover's supplier shares, and a buffer's own bill, are ordinary
    /// `Have` goals; without this flag a handover would converge its own
    /// inputs, and a buffer costing eight iron plates would want a handover to
    /// deliver those plates. One level of convergence per convergence point,
    /// deliberately.
    ///
    /// Carried *into* [`Step::Owned`] rather than cleared there: a supplier's
    /// own production must not itself converge either.
    pub(crate) converging: bool,
    pub depth: u32,
}

impl ExpansionCtx {
    pub fn new(state: PlanState, chain_actor: BotId) -> Self {
        // Outside any chain nothing is known about who runs what, and the
        // state may have been forked from one that was mid-expansion. Stated
        // rather than assumed, so an expansion always begins with mining
        // claims answering to nobody.
        let mut state = state;
        state.set_claim_runner(None);
        ExpansionCtx {
            state,
            ids: ActionIdGen::new(),
            chains: ChainIdGen::new(),
            chain_actor,
            chain: None,
            top_level: true,
            concurrency: None,
            converging: false,
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
    /// True anywhere beneath a converging method's own expansion.
    ///
    /// The termination guard: a converging method's own inputs — its supplier
    /// shares, and any buffer it has to build — must not converge again. One
    /// level of convergence per convergence point.
    pub converging: bool,
}

impl GoalSite {
    /// The site of a goal the caller asked for directly.
    pub fn root() -> Self {
        GoalSite {
            top_level: true,
            in_chain: false,
            converging: false,
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

    /// How many holders can pursue `goal` **at the same time**, if this method
    /// is what would satisfy it? `None` — the default — means this method
    /// names no limit.
    ///
    /// This exists so that a method which splits a goal across bots can size
    /// the split against what the world can actually accommodate, without
    /// learning what any particular method's obstacle *is*. `SplitAcrossBots`
    /// is a generic item-splitting method; seats on an ore patch are a
    /// resource-mining concept; the limit is the one thing they have to agree
    /// on, so the limit is what crosses the boundary and nothing else does.
    /// A method that wants to cap concurrency answers here in its own terms,
    /// and every method that has nothing to say keeps the default and is
    /// unaffected.
    ///
    /// **Answered whether or not the method is applicable.** A patch this plan
    /// has already committed makes `Mine::applicable` false, and that is
    /// exactly the state whose limit a caller most needs to hear about: an
    /// applicability-gated question would go quiet at zero and report "no
    /// limit". So `None` here must mean "this method has nothing to say about
    /// this goal at all" — not "this method cannot help right now".
    ///
    /// `cap` is the largest answer the caller can use, so a method whose count
    /// is expensive may stop there. Returning more than `cap` is allowed and
    /// harmless; returning less than the true limit is a narrower plan, never
    /// a wrong one, which is the direction to err in.
    fn concurrency(&self, _goal: &Goal, _state: &PlanState, _cap: u32) -> Option<u32> {
        None
    }

    /// The goal whose concurrency limit this method needs, if it is going to
    /// hand one goal to several bots.
    ///
    /// `None` — the default — for every method that scatters nothing. A
    /// converging method returns the goal it will split, which is not always
    /// the goal it was asked about: `SharedSmelt` is asked for iron *plate*
    /// and splits iron *ore*, and the seats that bound the split are the ore
    /// patch's. Returning the goal keeps the method from learning what a seat
    /// is ([`Method::concurrency`]'s whole point) while still getting the
    /// number.
    ///
    /// Answering also tells the driver that this method converges, so the
    /// whole subtree beneath it is marked [`GoalSite::converging`] and cannot
    /// converge again. The two are one answer because they are one decision:
    /// a method that splits a goal is a convergence point.
    fn split_probe(&self, _goal: &Goal, _state: &PlanState) -> Option<Goal> {
        None
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

    /// How many holders may pursue `goal` at once: the tightest limit any
    /// method names, or `None` when none of them names one.
    ///
    /// **Every method is asked, not the one `find` would pick.** The goal a
    /// splitter is looking at is the *shared* form; the goals its shares will
    /// become carry a different count and a different holder, so which method
    /// claims them is not settled yet. The minimum over everyone who has
    /// something to say is the conservative reading of that uncertainty, and
    /// it is exact in practice because an item is either mined or crafted and
    /// never both — only one method has anything to say about any given item.
    ///
    /// See [`Method::concurrency`] for why applicability is deliberately not
    /// consulted.
    pub fn concurrency(&self, goal: &Goal, state: &PlanState, cap: u32) -> Option<u32> {
        self.methods
            .iter()
            .filter_map(|m| m.concurrency(goal, state, cap))
            .min()
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
    // call must leave `depth`, `chain_actor`, `chain`, `top_level`,
    // `concurrency` and `converging` exactly as it found them even when it
    // fails, or a caller that continues past an error inherits a corrupted
    // context and a comment claiming that cannot happen.
    let previous_actor = ctx.chain_actor;
    let previous_chain = ctx.chain;
    let previous_top_level = ctx.top_level;
    let previous_concurrency = ctx.concurrency;
    let previous_converging = ctx.converging;
    // Whose timeline a mining claim would sit on. Kept in the state and
    // nowhere else — every tile selector reads it from there, so a second copy
    // in the context could only ever drift from the one that is consulted.
    // `set_claim_runner` hands back what it replaced, which makes the state
    // itself the save slot.
    let previous_claim_runner = ctx.state.claim_runner();
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
    ctx.concurrency = previous_concurrency;
    ctx.converging = previous_converging;
    // A frame that did not restore this would leave the tile selectors
    // answering for a bot the driver has already stopped expanding for — a
    // sibling goal picking its ore under a nephew's crowding rule.
    ctx.state.set_claim_runner(previous_claim_runner);
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
        converging: ctx.converging,
    };
    let method =
        registry
            .find(goal, &ctx.state, site)
            .ok_or_else(|| PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            })?;

    // How many holders may pursue this goal at once, if any method names a
    // limit. See `Method::concurrency` for why the question exists and
    // `MethodRegistry::concurrency` for why every method is asked.
    //
    // Computed at a **scatter site** and nowhere else. A scatter site is
    // exactly where one goal can be handed to several bots at once — the same
    // condition `SplitAcrossBots::claims` tests — so it is the only place an
    // answer can change a plan, and asking anywhere else would buy a walk of
    // an ore field per subgoal for a number nobody reads. The site is the
    // driver's own concept (see `GoalSite`), not a special case for one
    // method: any future method that scatters a goal is claimed at the same
    // sites and served by the same value.
    //
    // Assigned unconditionally rather than left alone, so a nested expansion
    // can never read an ancestor's answer about a different goal.
    //
    // The second arm is the one exception the original objection allows for.
    // A converging method — one that answered `split_probe` — is going to hand
    // one goal to several bots from *inside* a chain, so it needs the same
    // number at a site the first arm does not cover; and it names the goal it
    // will actually split, which need not be the goal it was asked about (a
    // shared smelt is asked for plate and splits ore). It fires only for a
    // method that has already decided it is going to read the answer, so the
    // walk of an ore field is still not bought for a number nobody reads.
    //
    // No more holders can ever be wanted than the state has bots, so that is
    // the ceiling a counting method may stop at — and it is a true ceiling,
    // not merely a plausible one, because `SplitAcrossBots` refuses a roster
    // naming a bot the state does not know before it sizes anything. Were that
    // not so, this would silently narrow a split to the state's roster and
    // hide the caller's mistake.
    let cap = ctx.state.bot_ids().len() as u32;
    let probe = method.split_probe(goal, &ctx.state);
    ctx.concurrency = if site.top_level && !site.in_chain {
        registry.concurrency(goal, &ctx.state, cap)
    } else if let Some(probe) = &probe {
        // A wider ceiling than a scatter site's, because a converging method
        // does not only ask "how many may work at once" — it also has to know
        // whether the answer is *comfortable*. Its split claims one working
        // spot per supplier where the solo version claims one in total, and a
        // claim is never released during an expansion, so a method that
        // converges on a barely-sufficient count starves whatever the plan
        // wants to produce next. The largest number it can use is its own
        // width plus the roster, and its width is at most the roster.
        registry.concurrency(probe, &ctx.state, cap.saturating_mul(3))
    } else {
        None
    };

    // A method that will scatter this goal is a convergence point, and nothing
    // beneath it may be one again — see `GoalSite::converging`. Sticky rather
    // than assigned, so a converging method nested under another (which the
    // guard is there to prevent in the first place) cannot clear the flag.
    // Restored by `expand_goal`, like every other driver-owned field.
    ctx.converging = ctx.converging || probe.is_some();

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
    let owner = match stated_holder(goal) {
        Some(Holder::Bot(bot) | Holder::Share(bot)) => Some(*bot),
        _ => None,
    };
    if ctx.chain.is_none() {
        let one_inventory = matches!(stated_holder(goal), Some(Holder::Bot(_) | Holder::Share(_)));
        if one_inventory || method.converges(goal, &ctx.state) {
            let chain = ctx.chains.next();
            ctx.chain = Some(chain);
            if let Some(bot) = owner {
                net.set_chain_owner(chain, bot);
            }
            // Opening a chain is also the answer to "when": everything
            // emitted under it runs on one bot, one action at a time, so its
            // mining claims cannot collide with each other. An owner names
            // that bot, and so covers every *other* chain it owns too; an
            // unowned chain names only itself, which is the weaker but still
            // sound reading — see `crate::state::ClaimRunner`.
            //
            // Set on the same lines as the owner rather than derived later, so
            // the two can never disagree about which chains are owned.
            // `expand_goal` restores both halves.
            let runner = Some(match owner {
                Some(bot) => ClaimRunner::Bot(bot),
                None => ClaimRunner::Chain(chain),
            });
            ctx.state.set_claim_runner(runner);
        }
    } else if let (Some(chain), Some(bot)) = (ctx.chain, owner)
        && net.owner_of(chain).is_none()
    {
        // **A chain that inherits a bot-sized goal is owned by that bot.**
        //
        // The other half of the rule above, and the half that was missing.
        // Opening a chain is not the only way a `Holder::Share` reaches one:
        // a method that `converges` opens a chain *unowned* — nothing named
        // a bot for it, only the shape of the decomposition — and everything
        // it decomposes into then lands inside that chain, where the branch
        // above cannot see it. `AssembleCell::converges` is `true` and
        // `Goal::Producing` names no holder, so the whole red-science bill —
        // `Goal::Have { whose: Holder::Share(ctx.chain_actor) }`, every
        // machine, inserter, chest, plate and coal of it — was expanded into
        // one ownerless chain.
        //
        // `expand_goal` still rebinds `chain_actor`, so the *sizing* was done
        // against that bot's inventory: a bot already carrying 48 iron-ore is
        // asked to mine none. But the scheduler was free to bind the chain to
        // whoever could run its first action cheapest, and on
        // `run-1788405365-21697` — four bots, three of them parked on the ore
        // patch — that was bot 2. Every later action followed the binding, and
        // the first insert sized against bot 1's stock refused with
        // `precondition has 3 iron-ore of action ActionId(41) does not hold for
        // bot 2`. It is the same defect `stated_holder` records, one level
        // further in: sizing against a bot that nothing then commits to.
        //
        // First holder wins, and only an ownerless chain is claimed: a chain
        // that already names an owner is not up for reinterpretation, and
        // `Step::Owned` — the one place a *different* bot's share is
        // deliberately handed out inside a chain — opens a chain of its own
        // and so never reaches here.
        //
        // The claim runner is deliberately left as it is. `ClaimRunner::Chain`
        // is the weaker but sound reading of the same fact (see the comment
        // above), so keeping it costs a little crowding and asserts nothing
        // untrue, where re-pointing it mid-expansion would move mining tiles
        // that sibling goals have already been sited against.
        net.set_chain_owner(chain, bot);
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
    reserve_chain_produce(ctx, produce_before);
    result
}

/// Book everything a chain just produced to the bot that ran it.
///
/// Extracted from `expand_goal_body` so that [`Step::Owned`] — which opens a
/// chain the body's `chain_on_entry.is_none()` test can never see — pays the
/// same ledger entry. Without the call there, a supplier chain's output would
/// look like spare stock to the *taker's* subsequent shortfall arithmetic:
/// the exact defect the ledger was added for, reintroduced through the new
/// door.
///
/// `produce_before` is `None` when the caller did not open a chain (or when
/// the roster is one bot, which is exempt — see the caller), and the whole
/// thing is then a no-op. Reserved against `ctx.chain_actor`, so it must be
/// called *before* a caller restores that field.
fn reserve_chain_produce(ctx: &mut ExpansionCtx, produce_before: Option<BTreeMap<ItemId, u32>>) {
    let Some(before) = produce_before else {
        return;
    };
    for (item, count) in ctx.state.item_totals() {
        let gained = count.saturating_sub(before.get(&item).copied().unwrap_or(0));
        if gained > 0 {
            ctx.state
                .reserve(&Holder::Bot(ctx.chain_actor), &item, gained);
        }
    }
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
            Step::Owned {
                whose,
                steps: inner,
            } => {
                // A handover names a supplier. `Holder::Anyone` names nobody,
                // and treating it as "keep the current chain" would silently
                // turn a convergence back into the welding it exists to undo.
                let (Holder::Bot(bot) | Holder::Share(bot)) = &whose else {
                    return Err(PlannerError::UnownedHandover {
                        holder: whose.to_string(),
                    });
                };
                let bot = *bot;
                // The same reconciliation `expand` and `expand_goal` make: a
                // bot the state does not know has no inventory and no
                // position, and would be planned for as a default one standing
                // at the origin.
                if ctx.state.bot(bot).is_none() {
                    return Err(PlannerError::UnknownBot(bot));
                }

                // Save, run, restore — on every exit path, errors included,
                // exactly as `expand_goal` does and for the same reason.
                let previous_actor = ctx.chain_actor;
                let previous_chain = ctx.chain;
                let previous_top_level = ctx.top_level;

                let chain = ctx.chains.next();
                // Always owned. A `Step::Owned` names a bot; that is the whole
                // point, and it puts the supplier's chain on the same footing
                // as a `Holder::Bot` or a `Holder::Share` in the scheduler,
                // where an owner is a hard constraint with no fallback tier.
                net.set_chain_owner(chain, bot);
                ctx.chain_actor = bot;
                ctx.chain = Some(chain);
                ctx.top_level = false;
                // Always owned, so the supplier's mining claims always know
                // whose timeline they sit on — the one place in the crate
                // where that is true by construction rather than by what the
                // caller happened to state.
                let previous_claim_runner = ctx.state.set_claim_runner(Some(ClaimRunner::Bot(bot)));
                // `ctx.converging` is deliberately *not* cleared: a supplier's
                // own production must not converge again either.

                // The same exemption the enclosing body makes, for the same
                // reason: with one bot there is no second runner for the
                // ledger to protect against.
                let produce_before =
                    (ctx.state.bot_ids().len() > 1).then(|| ctx.state.item_totals());

                let mut inner_promised: Vec<(Holder, ItemId, u32)> = Vec::new();
                let result = run_steps(inner, ctx, net, registry, &mut inner_promised);
                for (whose, item, count) in &inner_promised {
                    ctx.state.release(whose, item, *count);
                }
                reserve_chain_produce(ctx, produce_before);

                ctx.chain_actor = previous_actor;
                ctx.chain = previous_chain;
                ctx.top_level = previous_top_level;
                ctx.state.set_claim_runner(previous_claim_runner);
                result?;
            }
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
    fn producing_needs_a_registry_that_carries_build_cell() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let goal = Goal::Producing {
            item: "automation-science-pack".into(),
            per_minute: 150,
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

    // ---------------------------------------------------------------------
    // `Step::Owned`: a method emitting actions into another bot's chain.
    // ---------------------------------------------------------------------

    fn two_bot_ctx() -> ExpansionCtx {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        ExpansionCtx::new(state, BotId(1))
    }

    /// A bare action: no preconditions, no effects, nobody pinned.
    fn bare(ctx: &mut ExpansionCtx, label: &str) -> Box<crate::action::Action> {
        Box::new(crate::action::Action {
            id: ctx.ids.next(),
            kind: crate::action::ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![],
            eff: vec![],
            duration: 10,
            pinned: None,
            label: label.into(),
        })
    }

    /// An action that puts `count` of `item` into whoever runs it.
    fn gains(
        ctx: &mut ExpansionCtx,
        label: &str,
        item: &str,
        count: u32,
    ) -> Box<crate::action::Action> {
        let mut action = bare(ctx, label);
        action.eff = vec![crate::action::Effect::GainItem {
            who: crate::action::Actor::Role,
            item: item.into(),
            count,
        }];
        action
    }

    #[test]
    fn an_owned_block_puts_its_actions_in_a_chain_owned_by_the_named_bot() {
        let mut ctx = two_bot_ctx();
        let mut net = crate::network::ActionNetwork::new();
        let reg = MethodRegistry::new();

        // The taker's own chain, exactly as `expand_goal_body` would open it.
        let taker_chain = ctx.chains.next();
        ctx.chain = Some(taker_chain);
        net.set_chain_owner(taker_chain, BotId(1));

        let mine = bare(&mut ctx, "the taker acts");
        let mine_id = mine.id;
        let theirs = bare(&mut ctx, "the supplier acts");
        let theirs_id = theirs.id;
        let steps = vec![
            Step::Act(mine),
            Step::Owned {
                whose: Holder::Share(BotId(2)),
                steps: vec![Step::Act(theirs)],
            },
        ];
        let mut promised = Vec::new();
        run_steps(steps, &mut ctx, &mut net, &reg, &mut promised).expect("a handover expands");

        let supplier_chain = net
            .chain_of(theirs_id)
            .expect("the supplier's action is chained");
        assert_eq!(net.chain_of(mine_id), Some(taker_chain));
        assert_ne!(
            supplier_chain, taker_chain,
            "a handover must open a chain of its own, or the scheduler welds it back onto the taker"
        );
        assert_eq!(net.owner_of(supplier_chain), Some(BotId(2)));
        assert_eq!(net.owner_of(taker_chain), Some(BotId(1)));
    }

    #[test]
    fn an_owned_block_restores_the_context_however_it_exits() {
        for failing in [false, true] {
            let mut ctx = two_bot_ctx();
            let mut net = crate::network::ActionNetwork::new();
            // An empty registry claims nothing, so a subgoal is an error.
            let reg = MethodRegistry::new();
            let outer_chain = ctx.chains.next();
            ctx.chain = Some(outer_chain);
            ctx.top_level = true;
            ctx.converging = true;

            let inner = if failing {
                vec![Step::Subgoal(Goal::Have {
                    item: "coal".into(),
                    count: 1,
                    whose: Holder::Anyone,
                })]
            } else {
                vec![]
            };
            let steps = vec![Step::Owned {
                whose: Holder::Share(BotId(2)),
                steps: inner,
            }];
            let mut promised = Vec::new();
            let result = run_steps(steps, &mut ctx, &mut net, &reg, &mut promised);
            assert_eq!(result.is_err(), failing, "failing={failing}");

            assert_eq!(ctx.chain_actor, BotId(1), "failing={failing}");
            assert_eq!(ctx.chain, Some(outer_chain), "failing={failing}");
            assert!(ctx.top_level, "failing={failing}");
            // Deliberately *not* restored, and deliberately not cleared: a
            // supplier's own production must not converge either.
            assert!(ctx.converging, "failing={failing}");
        }
    }

    #[test]
    fn a_handover_that_names_nobody_is_refused_where_it_was_written() {
        let mut ctx = two_bot_ctx();
        let mut net = crate::network::ActionNetwork::new();
        let reg = MethodRegistry::new();
        let act = bare(&mut ctx, "whose?");
        let steps = vec![Step::Owned {
            whose: Holder::Anyone,
            steps: vec![Step::Act(act)],
        }];
        let mut promised = Vec::new();
        let err = run_steps(steps, &mut ctx, &mut net, &reg, &mut promised)
            .expect_err("a handover to nobody is a method bug");
        assert!(
            matches!(err, PlannerError::UnownedHandover { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn a_handover_to_a_bot_the_state_does_not_know_is_refused() {
        let mut ctx = two_bot_ctx();
        let mut net = crate::network::ActionNetwork::new();
        let reg = MethodRegistry::new();
        let act = bare(&mut ctx, "for a stranger");
        let steps = vec![Step::Owned {
            whose: Holder::Share(BotId(9)),
            steps: vec![Step::Act(act)],
        }];
        let mut promised = Vec::new();
        let err = run_steps(steps, &mut ctx, &mut net, &reg, &mut promised)
            .expect_err("a bot with no inventory and no position cannot be planned for");
        assert!(
            matches!(err, PlannerError::UnknownBot(BotId(9))),
            "got {err:?}"
        );
    }

    /// What a supplier's chain made belongs to that supplier's chain.
    ///
    /// The produce ledger fires in `expand_goal_body` only when *that frame*
    /// opened the chain, which a nested one never does. Without the extracted
    /// `reserve_chain_produce` call in the `Step::Owned` arm, a supplier's
    /// output would read as spare stock to the taker's own shortfall
    /// arithmetic — the exact defect the ledger exists for, coming back through
    /// the new door.
    #[test]
    fn a_supplier_chains_output_is_not_spare_stock_for_the_taker() {
        let mut ctx = two_bot_ctx();
        let mut net = crate::network::ActionNetwork::new();
        let reg = MethodRegistry::new();
        ctx.chain = Some(ctx.chains.next());

        let act = gains(&mut ctx, "the supplier mines", "iron-ore", 5);
        let steps = vec![Step::Owned {
            whose: Holder::Share(BotId(2)),
            steps: vec![Step::Act(act)],
        }];
        let mut promised = Vec::new();
        run_steps(steps, &mut ctx, &mut net, &reg, &mut promised).expect("expands");

        assert_eq!(
            ctx.state.inventory_count(BotId(2), "iron-ore"),
            5,
            "the ore really is in the supplier's hands"
        );
        assert_eq!(
            ctx.state.available(&Holder::Anyone, "iron-ore"),
            0,
            "and it is spoken for: a later goal must not plan against it"
        );
        assert_eq!(ctx.state.available(&Holder::Bot(BotId(2)), "iron-ore"), 0);
    }

    /// The inertness wedge, stated as a test rather than as an argument.
    ///
    /// Every nested stated holder this crate emits names `ctx.chain_actor`, so
    /// until a method emits a `Step::Owned` the driver change cannot move a
    /// plan. A method that emits none must therefore produce exactly one chain
    /// for a `Holder::Bot` goal, as it always has.
    #[test]
    fn a_method_that_emits_no_owned_step_opens_no_extra_chain() {
        let mut ctx = two_bot_ctx();
        let mut net = crate::network::ActionNetwork::new();
        let reg = MethodRegistry::new();
        let outer = ctx.chains.next();
        ctx.chain = Some(outer);

        let first = bare(&mut ctx, "one");
        let second = bare(&mut ctx, "two");
        let steps = vec![Step::Act(first), Step::Act(second)];
        let mut promised = Vec::new();
        run_steps(steps, &mut ctx, &mut net, &reg, &mut promised).expect("expands");

        let chains: BTreeSet<ChainId> = net.actions().filter_map(|a| net.chain_of(a.id)).collect();
        assert_eq!(chains, BTreeSet::from([outer]));
    }
}
