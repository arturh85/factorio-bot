//! How to get what we want: hand-written decompositions, and the driver that
//! runs them until only actions remain.

pub mod assemble;
pub mod blueprint;
pub mod connect;
pub mod extract;
pub mod fabricate;
pub mod gather;
pub mod have;
pub mod machine;
pub mod pipe;
pub mod power;
pub mod produce;
pub mod scout;
pub mod sustain;
pub mod util;

use crate::action::{Action, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ActionIdGen, BotId, ChainId, ChainIdGen, ItemId, Ticks};
use crate::state::{ClaimRunner, PlanState};
use crate::substance::{FluidRefusal, FluidSource, SubstanceTable};
use std::cell::OnceCell;
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
    /// True inside the rehearsal [`expand`] runs to build its gathering
    /// forecast, false in the real pass and in every context built directly.
    ///
    /// Driver-owned like `converging`. Read by exactly one place,
    /// `smelt_steps`' furnace handover, which keeps every furnace with its
    /// taker while rehearsing: a handover moves a furnace's stone and coal
    /// onto a supplier's forecast on a price the forecast itself changes,
    /// and the real pass, which has the forecast, decides it once -- the
    /// handover block says what that cost when the rehearsal decided too.
    pub(crate) rehearsing: bool,
    pub depth: u32,
    /// Where each bot's items came from: for every `(bot, item)`, the
    /// actions that gained it for the role and how much of each gain is
    /// still unspent, oldest first. Written by `run_steps` as it simulates an
    /// action's effects, read there to **state** the supply edge from the
    /// producers a consumer draws on -- see `run_steps` for why that edge is
    /// stated rather than left to `ActionNetwork::infer_edges`.
    pub(crate) stock: BTreeMap<(BotId, ItemId), Vec<(ActionId, u32)>>,
    /// What this world calls each prototype name, built on first ask and then
    /// only read.
    ///
    /// Memoised rather than built per goal because building walks the whole
    /// recipe table and the whole item-prototype table -- 662 + 342 entries on
    /// a real capture, cloned out from behind two `DashMap`s -- and
    /// `expand_goal_body` asks once per goal, of which one plan has hundreds.
    /// [`crate::substance::SubstanceTable`]'s own doc states the rule this
    /// obeys: cheap enough to build per expansion, too expensive per lookup.
    ///
    /// Safe to cache for a whole context because it is derived from
    /// `PlanState::base()` alone -- recipes and prototypes are game data, and
    /// no plan overlay adds or removes one. `ctx.state` is never reassigned
    /// after construction, so the base cannot change underneath it.
    substances: OnceCell<SubstanceTable>,
}

impl ExpansionCtx {
    /// Whether the goal being expanded is one the caller asked for, as
    /// opposed to a subgoal of some method's own.
    ///
    /// A read of the driver-owned `top_level`, and only a read: the field
    /// stays the driver's to set. `Researched` asks it to decide whether the
    /// chain actor is free to craft a share of the packs -- at the top level
    /// the research is all the chain actor has, and beneath a cell it is one
    /// item on a timeline that is already the plan's makespan.
    pub fn is_top_level(&self) -> bool {
        self.top_level
    }

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
            stock: BTreeMap::new(),
            converging: false,
            rehearsing: false,
            depth: 0,
            substances: OnceCell::new(),
        }
    }

    /// What this world calls each prototype name -- an item a character can
    /// hold, or a fluid that no character can hold in any amount.
    ///
    /// Built on the first ask and reused for the rest of the expansion; see
    /// the field's own doc for why that is sound and why it matters.
    pub fn substances(&self) -> &SubstanceTable {
        self.substances
            .get_or_init(|| SubstanceTable::from_world(self.state.base()))
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

    /// Why this method, which did not claim `goal`, could not: a named
    /// refusal in place of the driver's `NoApplicableMethod`, or `None` --
    /// the default -- when the method has nothing to say.
    ///
    /// Asked only after every method has declined `goal` (see
    /// [`MethodRegistry::refusal`]), so it never changes which method runs;
    /// it changes what a caller is told when none does. A method answers here
    /// when it can tell a *world* reason from "not mine": `Mine` says an item
    /// comes out of the ground and a hand cannot dig it, or that no ground the
    /// plan can see has any. Both are facts a script can act on, where "no
    /// method can satisfy goal" is not.
    ///
    /// Takes the context rather than the state because a refusal about the
    /// map is asked *from somewhere* -- the chain actor's position is where
    /// charting is measured from -- and the state alone does not say who is
    /// asking.
    fn refusal(&self, _goal: &Goal, _ctx: &ExpansionCtx) -> Option<PlannerError> {
        None
    }
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

    /// The first named refusal any method offers for a goal none of them
    /// claimed, or `None` when the driver's `NoApplicableMethod` is all there
    /// is to say. See [`Method::refusal`].
    ///
    /// Every method is asked, in registration order and regardless of
    /// `claims`: a refusal is knowledge about the goal, not a bid to run it,
    /// and a method that would not have claimed the goal at this site can
    /// still know why nobody can.
    pub fn refusal(&self, goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
        self.methods.iter().find_map(|m| m.refusal(goal, ctx))
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

/// Which bot to hand [`expand`] as its `chain_actor`, given the order the
/// caller would otherwise have taken them in.
///
/// Returns the first bot in `preference` that is not walled in; the first bot
/// in `preference` if every one of them is; and `None` only for an empty
/// slice, which is the caller's own error to report — `expand` has no goal to
/// name and `schedule` would refuse such a roster anyway.
///
/// # Why the pick is not a formality
///
/// `chain_actor` is what a goal naming no holder is stated against, and
/// [`ExpansionCtx`]'s own doc spells out that the methods state those goals as
/// `Holder::Share(ctx.chain_actor)` — `Researched`'s trigger and pack bills,
/// `produce`'s and `assemble`'s cell bills. Two things follow from *one* value,
/// which is the whole point of choosing it here rather than downstream:
///
/// * the bill is **sized** against that bot's inventory (`state.available(&
///   Holder::Share(bot), ..)`), and
/// * the chain that expands from it is **owned** by that same bot
///   (`expand_goal_body`), which `crate::schedule` treats as a hard constraint
///   with no fallback tier.
///
/// So a top-level chain pinned to a bot that cannot walk anywhere is work no
/// other bot may ever take over and no replan can move — the same trap
/// `have::participants_that_can_work` argues at length for a gathering share,
/// arriving by the other road. That fix left this one open on purpose: it is
/// scoped to shares, and the top-level chain is not a share.
///
/// Sizing and binding stay in agreement *because* they are both read off this
/// one value. `run-1788405365-21697` is what disagreement costs — a chain sized
/// against bot 1's stock and bound to bot 2, which died with `precondition has
/// 3 iron-ore … does not hold for bot 2` — and that came from relaxing the
/// owner after expansion, not from the pick. Moving the pick moves both halves
/// together; nothing else here may move only one.
///
/// # Why it can never leave a plan with no actor
///
/// The same rule `participants_that_can_work` keeps, for the same reason: with
/// every bot walled in there is no better bot to move the work to, and a plan
/// that dispatches and fails leaves a record, a failed walk and a recovery
/// tier, where a plan that was never made leaves none of those. So the fallback
/// is the caller's own first choice, not `None`.
///
/// # Determinism
///
/// A function of the slice's order and a `BTreeMap` lookup, with no floats and
/// no iteration over an unordered collection. Callers that have no meaningful
/// preference of their own should pass a roster sorted by `BotId`, which is
/// what `crates/executor`'s tier 2 does; `crates/scripting_lua` passes the
/// roster the script wrote, because that is the order it already picked from.
/// A bot the state has never heard of is *not* skipped here — `expand` reports
/// it as `UnknownBot`, and silently planning around a caller's mistake would
/// hide it.
pub fn pick_chain_actor(state: &PlanState, preference: &[BotId]) -> Option<BotId> {
    let first = preference.first().copied()?;
    // The overwhelmingly common case, and the cheap one: no ledger, no
    // question. Stated rather than left to the `find` below so that a healthy
    // run provably takes the same path it always did.
    if !state.any_sidelined() {
        return Some(first);
    }
    // Walled in or benched -- `PlanState::may_own_work` is the one question,
    // and a benched bot (the game's own verdict that it cannot move) must not
    // own the chain any more than a walled-in one may.
    Some(
        preference
            .iter()
            .copied()
            .find(|bot| state.may_own_work(*bot))
            .unwrap_or(first),
    )
}

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
/// The expansion is run **twice**: a rehearsal that learns how much of each
/// raw item the plan gathers by hand, then the plan itself with that total
/// installed as [`PlanState::gathering_ahead`]'s forecast, so a per-fragment
/// decision -- today, whether a rock beats the tile -- can be taken over the
/// plan's whole demand. See the body for why nothing cheaper knows that
/// number, and `have::chop_beats_mining` for what it changed.
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
    // **Rehearse, then plan.** The expansion is run twice: once to learn how
    // much of each raw item the plan gathers by hand in total
    // (`PlanState::gathering_recorded`), and once for real with that total
    // installed as the forecast (`PlanState::set_gathering_forecast`), so a
    // decision taken per fragment can be made over the plan's demand
    // instead. One reader today, `have::chop_beats_mining`: a `Have { coal,
    // 1 }` for a furnace's fuel never pays for a 24-coal rock on its own,
    // and a plan made of a dozen such fragments hand-mined every one of them
    // beside the rocks it swung at for its cells -- measured on
    // `producing:logistic-science-pack:6`, 20 coal and 2,400 ticks.
    //
    // Nothing here knows the plan's demand for an item before the plan
    // exists: the fragments come from cells, hand-smelts and fuel visits
    // that are only stated as the expansion goes, so the demand is read off
    // a finished expansion and nothing else. Both passes are deterministic
    // -- the second differs from the first only through the forecast -- so
    // the plan is still a pure function of its inputs. A rehearsal that
    // fails leaves the forecast empty rather than failing the plan: the real
    // pass reports whatever it meets, exactly as before, and a forecast is
    // only ever advisory.
    //
    // The cost is one extra expansion. Scheduling, which is where a plan's
    // time goes, is not repeated.
    let forecast = {
        let mut rehearsal = ExpansionCtx::new(state.fork(), chain_actor);
        rehearsal.rehearsing = true;
        let mut scratch = ActionNetwork::new();
        let rehearsed = goals
            .iter()
            .try_for_each(|goal| expand_goal(goal, &mut rehearsal, &mut scratch, registry));
        match rehearsed {
            Ok(()) => rehearsal.state.gathering_recorded(),
            Err(_) => BTreeMap::new(),
        }
    };
    let mut forecast_state = state.fork();
    forecast_state.set_gathering_forecast(forecast);
    let mut ctx = ExpansionCtx::new(forecast_state, chain_actor);
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

/// The refusal for a [`Goal::Have`] whose item this world calls a fluid, or
/// `None` for every other goal and every item.
///
/// **Positive evidence only.** `SubstanceTable::is_fluid` answers `false` for
/// a name the world says nothing about, which is every name in this crate's
/// hand-built fixture worlds; a guard that refused on absence of evidence
/// would refuse most of the test suite and would be stating something nobody
/// established.
///
/// The count is carried into the message unchanged, including zero. A `Have
/// { count: 0 }` about a fluid is trivially "satisfied" in the arithmetic
/// sense and is still a statement about an inventory holding a fluid, so it is
/// refused for the same reason the others are -- the shape is wrong, not the
/// number. Nothing emits one today: a fluid can only enter a bill through a
/// recipe, and over the two categories this planner runs (`crafting`,
/// `smelting`) the live 2.1.17 capture has no recipe with a fluid ingredient
/// at all.
fn fluid_have_refusal(goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
    let Goal::Have { item, count, .. } = goal else {
        return None;
    };
    if !ctx.substances().is_fluid(item) {
        return None;
    }
    Some(PlannerError::FluidNotItem(FluidRefusal::NotCarryable {
        fluid: item.to_string(),
        count: *count,
        produced_by: FluidSource::of_world(ctx.state.base(), item),
    }))
}

/// The refusal for a goal that **named a recipe** which cannot answer it, or
/// `None` when no recipe was named or the named one is fine.
///
/// # Why this is here and not a `Method::refusal`
///
/// The same argument the fluid guard above makes, applied to a different
/// wrong shape. `Method::refusal` is consulted only when *nobody* claimed the
/// goal, and a goal like `have:iron-gear-wheel:5 via casting-iron-gear-wheel`
/// is claimed immediately -- by `HandCraft`, which would then plan the
/// ordinary gear recipe and hand back a schedule. The caller asked for one
/// recipe and got another with no diagnostic at all, which is exactly the
/// silent substitution `crate::products` exists to stop. So it is refused
/// **before any method is asked**, like a fluid `Have`, because a goal whose
/// qualifier its own methods cannot honour has not been stated, and nothing
/// should get the chance to satisfy it some other way.
///
/// # Absent is not a value
///
/// `via: None` returns `None` on the first line, before an index is built.
/// That is not an optimisation: it is the guarantee that a goal which named
/// no recipe travels the path it travelled before this function existed.
fn named_recipe_refusal(goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
    let via = match goal {
        Goal::Have { via, .. } | Goal::Produced { via, .. } => via.as_deref()?,
        _ => return None,
    };
    let item = match goal {
        Goal::Have { item, .. } | Goal::Produced { item, .. } => item,
        _ => return None,
    };
    let machines = crate::method::machine::MachineTable::from_state(&ctx.state);
    let index = crate::products::ProductIndex::from_state(&ctx.state);
    match index.recipe_producing(
        item,
        &crate::products::Categories::planner_runs(&machines),
        Some(via),
        &machines,
    ) {
        Ok(_) => None,
        Err(refusal) => Some(PlannerError::ProductNotMakeable(Box::new(refusal))),
    }
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

    // **Before any method is asked**, and deliberately not as a
    // `Method::refusal`.
    //
    // A `Goal::Have` about a fluid is not a goal no method happens to know how
    // to satisfy; it is a goal that cannot be *stated*, because `Have` means
    // "this is in an inventory" and no character inventory holds a fluid. If
    // the registry gets to answer first, `SplitAcrossBots` claims the goal and
    // divides 100 petroleum-gas into four shares of 25 -- and the refusal that
    // eventually surfaces reads `no method can satisfy goal: have 25
    // petroleum-gas (a share sized for bot 1)`, which describes a bot's share
    // of something no bot can hold any of. The share is an artefact of the
    // refusal path, not a fact about the request, and saying it at all is the
    // lie this guard exists to stop.
    //
    // `Goal::Produced` is deliberately **not** covered: "cause 100
    // petroleum-gas to come into existence" is a perfectly meaningful thing to
    // ask of a refinery, and `products::NoProducer` answers it by naming the
    // recipes and the category this planner has no machine for. Only `Have`
    // makes a claim about an inventory.
    if let Some(refusal) = fluid_have_refusal(goal, ctx) {
        return Err(refusal);
    }

    // Second guard, same reason, and deliberately **after** the fluid one: a
    // `Have` about a fluid is wrong whatever recipe was named, so the sharper
    // message wins. See `named_recipe_refusal`.
    if let Some(refusal) = named_recipe_refusal(goal, ctx) {
        return Err(refusal);
    }

    let chain_on_entry = ctx.chain;
    let site = GoalSite {
        top_level: ctx.top_level,
        in_chain: ctx.chain.is_some(),
        converging: ctx.converging,
    };
    // When nobody claims the goal, a method that knows *why* gets to say so
    // before the driver falls back to "no method can satisfy goal", which is
    // true and unactionable. See `Method::refusal`.
    let Some(method) = registry.find(goal, &ctx.state, site) else {
        return Err(registry.refusal(goal, ctx).unwrap_or_else(|| {
            PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            }
        }));
    };

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
    //   problem it removes. The parallelism *was* restored, on 2026-09-05,
    //   and not here: `Researched` now states each pack share, the trigger
    //   prerequisite and each lab as a different bot's `Holder::Share` inside
    //   a `Step::Owned` block, so they run on different bots because they
    //   are different bots' shares, each sized against and bound to its own
    //   runner by exactly this rule.
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

    // **The stock this goal is about to size itself against is claimed by this
    // goal, for as long as its own expansion runs.**
    //
    // `shortfall` credits what the holder already has towards a `Goal::Have`
    // and asks a method to make only the difference. That credit is a promise:
    // the items are counted as part of the `count` the goal will deliver. But
    // until 2026-09-04 nothing recorded the promise, so anything *inside* the
    // goal's own expansion could spend the very stock the shortfall had
    // already committed -- and the goal then delivered less than it said.
    //
    // `run-1788509918-33958` is why, and its numbers are the whole argument.
    // Green science asked for `Have { iron-plate, 150, Share(bot 1) }`.
    // `Withdraw` emptied seventeen furnaces into bot 1's hands and recursed;
    // the recursion sized itself at `150 - 17 = 133` and built a cell for the
    // difference. Building that cell needs a drill, the drill needs three
    // gears and three plates, and its `Have { iron-plate, 3 }` and
    // `Have { iron-plate, 6 }` subgoals looked at the seventeen plates sitting
    // unclaimed in bot 1's inventory, planned nothing, and spent nine of them.
    // The take delivered its 133 onto the eight that were left, the craft
    // demanded the 150 it had been promised, and `PlanState::lose` found 141.
    // The run died on the raise, having reached green science.
    //
    // Reserved rather than spent, like every other entry in this ledger:
    // `inventory_count` and `lose` still see the items, so this goal's own
    // arithmetic and its `Condition::HasItem`s are untouched, and only *other*
    // goals asking `available` are told the stock is spoken for.
    //
    // Measured **before** `method.expand`, because that is the number the
    // shortfall was computed from -- `registry.find` and the method's own
    // `demand` both read it here -- and applied **after**, because reserving
    // it first would hide the credit from the very sizing it describes and
    // make the method produce the whole `count` again.
    //
    // Only `Goal::Have` credits anything. `Goal::Produced` asks for the whole
    // count regardless of what is held (possession is not production), so it
    // has no credit to protect.
    let credited: Option<(Holder, ItemId, u32)> = match goal {
        Goal::Have {
            item, count, whose, ..
        } => {
            let held = (*count).min(ctx.state.available(whose, item));
            (held > 0).then(|| (whose.clone(), item.clone(), held))
        }
        _ => None,
    };

    let steps = method.expand(goal, ctx)?;

    if let Some((whose, item, count)) = &credited {
        ctx.state.reserve(whose, item, *count);
    }

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
    // Save, run, restore, on every exit path including the error one: a credit
    // held past its own goal would make every later goal size itself against
    // stock that is permanently invisible.
    if let Some((whose, item, count)) = &credited {
        ctx.state.release(whose, item, *count);
    }

    // What a chain made belongs to that chain.
    //
    // Expansion simulates every chain's effects into the *same* notional
    // inventory — `chain_actor`, one bot — while the scheduler is free to put
    // an *unowned* chain on any bot it likes. So the moment such a chain
    // closes, what it produced is stock some other runner may be carrying, and
    // a sibling that sizes itself against it plans work it cannot do.
    //
    // Which goals are told that, and which are not, is `owner`'s question and
    // is answered in `reserve_chain_produce` — it is the whole of this fix, so
    // read it there rather than assuming "hidden from everyone".
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
    reserve_chain_produce(ctx, produce_before, owner);
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
///
/// # Which holder the entry is filed under, and why it is not always the bot
///
/// `owner` is the chain's owner as the caller recorded it in the network:
/// `Some` for a chain a `Holder::Bot`, a `Holder::Share` or a [`Step::Owned`]
/// named a bot for, `None` for one a `converges`ing method opened, where
/// nothing named anybody and the scheduler will pick whoever is cheapest.
/// That distinction decides everything here.
///
/// **An unowned chain is hidden from everyone** — `Holder::Bot(chain_actor)`,
/// the original entry. Expansion simulated its effects into `chain_actor`'s
/// notional inventory, but nothing binds the chain to that bot, so at run time
/// the items are wherever the scheduler put the chain. No later goal, however
/// it is stated, may size itself against them.
///
/// **An owned chain is hidden only from `Holder::Anyone`.** The items really
/// are in the owner's hands: an owner is a hard, single-candidate constraint
/// in `schedule`, so the chain runs on that bot and nowhere else. A later goal
/// stated for *that same bot* — `Holder::Bot(b)` or `Holder::Share(b)`, which
/// opens a chain owned by `b` in its turn — is therefore asking about the one
/// inventory the items are provably in, and telling it otherwise makes it
/// re-mine and re-smelt stock its own runner is already carrying. A goal
/// stated for a *different* bot reads that bot's inventory and never saw these
/// items anyway; a `Holder::Anyone` goal reads the roster total and could be
/// run by anybody, so for it the stock stays spoken for — which is exactly
/// what an `Anyone` entry in this ledger means (see [`PlanState::available`]:
/// an `Anyone` reservation lowers the roster total and no individual bot's
/// figure).
///
/// The defect the ledger was added for is untouched by that narrowing, because
/// its premise is gone. `9a9cdd46` wrote it against "research `steam-power`
/// (craft 50 iron plates), then craft a lab, and the lab chain sees fifty
/// plates sitting there, smelts none of its own, **and the schedule then hands
/// the two chains to two bots**". The last clause stopped being possible on
/// 2026-09-02, when a `Holder::Share` began to *own* the chain it opens (see
/// the owner-binding comment in [`expand_goal_body`]): both chains state
/// `Holder::Share(chain_actor)`, so both are owned by that one bot and the
/// scheduler has no choice left to get wrong.
///
/// What the narrowing costs is measured, and it is the reason for it. That
/// exact shape — `world_with_trigger_prerequisite`, four bots — planned 157
/// steps and mined 91 iron-ore against the 113 steps and 41 iron-ore one bot
/// planned for the same goal, because every chain after the trigger's re-mined
/// what the trigger's had already made. See
/// `four_bots_do_not_re_mine_what_an_earlier_chain_of_theirs_produced`.
fn reserve_chain_produce(
    ctx: &mut ExpansionCtx,
    produce_before: Option<BTreeMap<ItemId, u32>>,
    owner: Option<BotId>,
) {
    let Some(before) = produce_before else {
        return;
    };
    let whose = match owner {
        Some(_) => Holder::Anyone,
        None => Holder::Bot(ctx.chain_actor),
    };
    for (item, count) in ctx.state.item_totals() {
        let gained = count.saturating_sub(before.get(&item).copied().unwrap_or(0));
        if gained > 0 {
            ctx.state.reserve(&whose, &item, gained);
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
                if let Goal::Have {
                    item, count, whose, ..
                } = &g
                {
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
                // Whether the action is anybody's in particular. An action
                // whose every condition and effect is about the world and
                // none about the runner (`Action::tied_to_runner`) is not
                // held to the chain below, and its ticks are not this bot's
                // load either: the scheduler gives it to whoever finishes it
                // soonest, and charging it to the chain actor here would make
                // that bot look busier to `furnace_suppliers` and the pack
                // deal than it will be.
                let tied = action.tied_to_runner();
                // The load ledger a method reads to rank bots
                // (`PlanState::planned_ticks`), fed here because this is the
                // one place every action passes with its chain's runner set.
                if tied {
                    ctx.state.note_planned_ticks(action.duration);
                }
                // **The supply edge is stated, not inferred.** `infer_edges`
                // pairs every producer of an item with every consumer of it
                // and drops whichever pairing would close a cycle -- so with
                // two cells standing, "gears crafted for the second cell's
                // drill" is paired with "craft the first cell's drill", and
                // the one edge that is real -- the take that supplies those
                // gears' plates -- arrives last and is the one dropped. The
                // scheduler then finds the chain owner without the plates:
                // `ChainOwnerInfeasible: has 6 iron-plate`, measured on
                // `producing:automation-science-pack:6` the moment a cell's
                // output could feed another cell's bill; and `has 42
                // iron-plate` on a ladder whose second rung spent the fifty
                // plates a `Produced` trigger had left as free stock. The
                // overlay knows what a bot holds; `ctx.stock` knows which
                // actions put it there, and a consumer is linked to the
                // producers it would spend, **newest first**. Stated edges
                // go in before inference and are never the ones inference
                // drops, and they cannot cycle: a producer is always in the
                // network before what it supplies.
                //
                // Newest first, since 2026-09-05; it was oldest first. A
                // subgoal's produce is emitted immediately before the action
                // that asked for it and reserved for that action (see the
                // doc above), so the newest stock is the stock that was made
                // for this consumer, and the oldest is what an *earlier*
                // sibling reserved for a *later* one. Oldest-first tied a
                // drill's six hand-smelted gear plates to the previous
                // cell's thirty-two-plate take, and the plan stood its cells
                // one per take: place, wait 8,400, place, wait 8,400 --
                // measured on `producing:logistic-science-pack:6`. Either
                // order is sound -- every consumer is linked to producers
                // covering its count and no unit is linked twice, so the
                // inventory at any consumer that satisfies its edges holds
                // what it needs -- and newest-first is the one that says
                // what the expansion meant.
                //
                // **Only together with `infer_edges` leaving same-chain
                // `HasItem` pairings to this edge.** Measured on the
                // reference dump, green's makespan: oldest-first with the
                // inference 142,092; newest-first with it 195,894 (the two
                // disagree and the scheduler honours both); oldest-first
                // without it 142,092 (the oldest producer *is* the inferred
                // edge); newest-first without it **97,232**. The stated edge
                // is the assignment, and it is exact only when it is the
                // only one.
                let suppliers: Vec<ActionId> = action
                    .pre
                    .iter()
                    .filter_map(|c| match c {
                        Condition::HasItem {
                            who: Actor::Role,
                            item,
                            count,
                        } => Some((item.clone(), *count)),
                        _ => None,
                    })
                    .flat_map(|(item, count)| {
                        let mut covered = 0u32;
                        ctx.stock
                            .get(&(binding, item))
                            .into_iter()
                            .flatten()
                            .rev()
                            .take_while(move |(_, left)| {
                                let short = covered < count;
                                covered = covered.saturating_add(*left);
                                short
                            })
                            .map(|(producer, _)| *producer)
                            .collect::<Vec<_>>()
                    })
                    .collect();
                let mut spends: Vec<(ItemId, u32)> = Vec::new();
                let mut gains: Vec<(ItemId, u32)> = Vec::new();
                for effect in &action.eff {
                    match effect {
                        Effect::LoseItem {
                            who: Actor::Role,
                            item,
                            count,
                        } => spends.push((item.clone(), *count)),
                        Effect::GainItem {
                            who: Actor::Role,
                            item,
                            count,
                        } => gains.push((item.clone(), *count)),
                        _ => {}
                    }
                }
                let id = net.add(*action);
                // Stamp it with the chain it was expanded under, so the
                // scheduler keeps the chain together. Outside a per-bot
                // subtree there is no chain and the action stays free.
                //
                // **Only an action that is somebody's.** A chain exists to
                // keep items and the hands that hold them together; an action
                // that names no hands -- `research automation`, whose labs,
                // packs and result are all world facts -- has nothing to keep
                // together, and stamping it anyway welds it to the chain
                // actor's queue. That is what put the green plan's tail on
                // one bot: `research automation` (6,000 ticks) sat behind
                // bot 1's cell build from tick 38,107 to 53,423 while bot 2
                // had been idle since 36,027, and `research
                // logistic-science-pack` (11,400) queued behind it. Left
                // unstamped, the scheduler offers it to the whole roster and
                // the bot that finishes it soonest takes it. Its ordering is
                // untouched: every edge to and from it is world-scoped
                // (`EntityAt`, `Researched`, the stated `Link`s) and
                // `infer_edges` keeps those whatever the chains.
                if let (Some(chain), true) = (ctx.chain, tied) {
                    net.set_chain(id, chain);
                }
                for producer in suppliers {
                    net.link(producer, id, 0);
                }
                for (item, count) in spends {
                    let mut left = count;
                    if let Some(stock) = ctx.stock.get_mut(&(binding, item)) {
                        while left > 0
                            && let Some((_, newest)) = stock.last_mut()
                        {
                            let spent = left.min(*newest);
                            *newest -= spent;
                            left -= spent;
                            if *newest == 0 {
                                stock.pop();
                            }
                        }
                    }
                }
                for (item, count) in gains {
                    if count > 0 {
                        ctx.stock
                            .entry((binding, item))
                            .or_default()
                            .push((id, count));
                    }
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
                // Always owned — `net.set_chain_owner(chain, bot)` above — so
                // the entry is filed under `Holder::Anyone`: the supplier's
                // output is provably in *this* bot's hands, hidden from any
                // goal that could run anywhere and visible to one stated for
                // this bot. See `reserve_chain_produce`.
                reserve_chain_produce(ctx, produce_before, Some(bot));

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

    /// **`expand` rehearses.** One bot is asked to produce one stone, four
    /// times over. Judged one at a time -- which is all a method sees --
    /// no single stone pays for a 360-tick swing at the fixture's
    /// `rock-huge` against 120 ticks of hand mining, so a single pass
    /// hand-mines four tiles. The rehearsal records that the bot gathers
    /// four stone; the plan itself then prices the first fragment over that
    /// four, and swings.
    ///
    /// `Produced` rather than `Have`, so that the four are four demands: a
    /// `Have` is a holding, and the second of four identical holdings is
    /// already satisfied by the first. `Produced` ignores inventory by
    /// design, which is also why only the first fragment swings here -- the
    /// three behind it are each judged over what is still ahead, three then
    /// two then one, and none of those pays. What this pins is the
    /// mechanism: a fragment that would not pay alone pays as the head of a
    /// demand.
    #[test]
    fn the_first_fragment_of_a_demand_swings_the_rock_the_whole_demand_pays_for() {
        let bots = [BotId(1)];
        let state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        let goals: Vec<Goal> = (0..4)
            .map(|_| Goal::Produced {
                item: "stone".into(),
                count: 1,
                whose: Holder::Bot(BotId(1)),
                unlocks: None,
                via: None,
            })
            .collect();
        let reg = crate::method::have::registry_for(&bots);
        let is_chop = |a: &Action| matches!(a.kind, crate::action::ActionKind::Chop { .. });
        let is_mine = |a: &Action| matches!(a.kind, crate::action::ActionKind::Mine { .. });

        // The rehearsal alone, which is what a plan used to be.
        let mut single = ExpansionCtx::new(state.fork(), BotId(1));
        let mut single_net = ActionNetwork::new();
        for goal in &goals {
            expand_goal(goal, &mut single, &mut single_net, &reg).expect("expands");
        }
        assert_eq!(
            single.state.gathering_recorded(),
            std::collections::BTreeMap::from([((BotId(1), "stone".to_string()), 4)]),
            "the rehearsal records the bot's whole demand"
        );
        assert!(
            !single_net.actions().any(is_chop),
            "control: judged fragment by fragment, nobody swings"
        );
        assert_eq!(single_net.actions().filter(|a| is_mine(a)).count(), 4);

        let net = expand(&goals, &state, &reg, BotId(1)).expect("expands");
        assert_eq!(
            net.actions().filter(|a| is_chop(a)).count(),
            1,
            "the head of the demand swings the rock"
        );
        assert_eq!(
            net.actions().filter(|a| is_mine(a)).count(),
            3,
            "and only that fragment left the patch alone"
        );

        // Two passes are still one function of the inputs.
        let again = expand(&goals, &state, &reg, BotId(1)).expect("expands");
        assert_eq!(
            format!("{net:?}"),
            format!("{again:?}"),
            "the rehearsal changes nothing about determinism"
        );
    }

    /// The supply edge is stated from stock provenance, not inferred: a
    /// consumer whose ingredients were already in the bot's hands -- put
    /// there by an earlier action of the same plan, with no subgoal of its
    /// own producing them -- is still ordered after that action. This is the
    /// ladder failure (`has 42 iron-plate`): a `Produced` trigger's fifty
    /// plates are free stock, the next goal's gears are crafted from them,
    /// and without the edge the scheduler crafted the gears before the take.
    #[test]
    fn a_consumer_is_ordered_after_the_actions_that_stocked_what_it_spends() {
        let bots = [BotId(1)];
        let state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        let net = expand(
            &[
                Goal::Produced {
                    item: "iron-plate".into(),
                    count: 50,
                    whose: Holder::Share(BotId(1)),
                    unlocks: None,
                    via: None,
                },
                Goal::Have {
                    item: "iron-gear-wheel".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                    via: None,
                },
            ],
            &state,
            &crate::method::have::registry_for(&bots),
            BotId(1),
        )
        .expect("both plan");
        let take = net
            .actions()
            .find(|a| a.label == "take 50 iron-plate from the cell")
            .expect("the trigger's fifty come out of a cell");
        let craft = net
            .actions()
            .find(|a| a.label == "craft 5 iron-gear-wheel")
            .expect("the gears are crafted");
        assert!(
            net.actions()
                .all(|a| !a.label.starts_with("take 10 iron-plate")),
            "the gears' ten plates are the trigger's, not a second production"
        );
        assert!(
            net.preds(craft.id).iter().any(|(from, _)| *from == take.id),
            "the craft is ordered after the take that stocked its plates: {:?}",
            net.preds(craft.id)
        );
    }

    /// The stated edge names the **newest** stock, not the oldest. Two
    /// producers of ten plates, then two consumers of ten: the first
    /// consumer is the one the second producer was made for, and it is
    /// linked to that producer alone. Oldest-first linked it to the first
    /// producer, which on a real chain was the previous cell's take with
    /// 8,400 ticks of lag on it (see the comment in `run_steps`). The goal
    /// names a bot so the subtree is a chain and `infer_edges` adds nothing
    /// of its own.
    #[test]
    fn a_consumer_is_linked_to_the_newest_stock_that_covers_it() {
        use crate::action::Condition;
        struct TwoThenTwo;
        impl Method for TwoThenTwo {
            fn name(&self) -> &'static str {
                "two-then-two"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "wood")
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                let spend = |ctx: &mut ExpansionCtx| Action {
                    id: ctx.ids.next(),
                    kind: ActionKind::Craft {
                        item: "iron-gear-wheel".into(),
                        count: 5,
                    },
                    pre: vec![Condition::HasItem {
                        who: Actor::Role,
                        item: "iron-plate".into(),
                        count: 10,
                    }],
                    eff: vec![Effect::LoseItem {
                        who: Actor::Role,
                        item: "iron-plate".into(),
                        count: 10,
                    }],
                    duration: 60,
                    pinned: None,
                    label: "spend 10 iron-plate".into(),
                };
                Ok(vec![
                    Step::Act(Box::new(gain_action(ctx, "iron-plate", 10))),
                    Step::Act(Box::new(gain_action(ctx, "iron-plate", 10))),
                    Step::Act(Box::new(spend(ctx))),
                    Step::Act(Box::new(spend(ctx))),
                    Step::Act(Box::new(gain_action(ctx, "wood", 1))),
                ])
            }
        }
        let bots = [BotId(1)];
        let state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        let net = expand(
            &[Goal::Have {
                item: "wood".into(),
                count: 1,
                whose: Holder::Bot(BotId(1)),
                via: None,
            }],
            &state,
            &MethodRegistry::new().with(Box::new(TwoThenTwo)),
            BotId(1),
        )
        .expect("it expands");
        let ids: Vec<ActionId> = net.actions().map(|a| a.id).collect();
        let (p1, p2, c1, c2) = (ids[0], ids[1], ids[2], ids[3]);
        assert_eq!(
            net.preds(c1),
            vec![(p2, 0)],
            "the first consumer spends the newest stock"
        );
        assert_eq!(
            net.preds(c2),
            vec![(p1, 0)],
            "the second spends what is left"
        );
    }

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
            via: None,
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
            via: None,
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
            via: None,
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
            via: None,
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
                    via: None,
                },
                Goal::Have {
                    item: "coal".into(),
                    count: 5,
                    whose: Holder::Anyone,
                    via: None,
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
                matches!(goal, Goal::Have { item, count, whose, .. }
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
                let Goal::Have {
                    item, count, whose, ..
                } = goal
                else {
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
                        via: None,
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
                        via: None,
                    }),
                    Step::Subgoal(Goal::Have {
                        item: "gadget".into(),
                        count: 1,
                        whose: Holder::Anyone,
                        via: None,
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
                via: None,
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
                matches!(goal, Goal::Have { item, count, whose, .. }
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
                        via: None,
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
                let Goal::Have {
                    item, count, whose, ..
                } = goal
                else {
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
                    via: None,
                },
                Goal::Have {
                    item: "cog".into(),
                    count: 2,
                    whose: Holder::Anyone,
                    via: None,
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
                        via: None,
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
            via: None,
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
            via: None,
        };
        assert!(matches!(
            expand(&[goal], &state, &reg, BotId(1)),
            Err(PlannerError::ExpansionTooDeep { .. })
        ));
    }

    /// **Rung 1, one bot against four.** Four bots must not mine twice what
    /// one bot mines for the same goal.
    ///
    /// `researched("automation")` over `world_with_trigger_prerequisite` --
    /// `steam-power` ("craft 50 iron plates") sitting under `automation`, the
    /// vanilla 2.0 shape -- with the freeplay starting inventory on every bot
    /// and no power standing, so the plan builds its own plant. That is the
    /// milestone `scripts/factory_stage2.lua` opens with, reduced to a
    /// fixture.
    ///
    /// **Why this shape and not a simpler one.** The duplication needs a chain
    /// that *closes with surplus in it* and a later chain of the same owner
    /// that could have spent the surplus. A plain pack bill has neither: every
    /// chain in it consumes what it makes. The trigger is what supplies the
    /// surplus -- fifty iron plates crafted to fire `steam-power`, still in
    /// hand when the lab bill starts -- which is why `world_with_technologies`
    /// reproduces nothing here however many bots it is given, and why this
    /// fixture is the one the regression is pinned against.
    ///
    /// Before the fix this fixture planned, at four bots, 157 steps and 91
    /// iron-ore, 70 stone and 35 coal, against one bot's 113 steps, 41
    /// iron-ore, 50 stone and 29 coal -- 2.2x the iron for the same ten
    /// science packs, because every chain after the trigger's re-made what the
    /// trigger's had already made. `workspace/runs/run-1788449752-46541`'s
    /// first rung-1 plan is the same defect at a real map's scale: 244 steps
    /// and 179 iron-ore, with the whole lab bill emitted twice (two
    /// `craft 1 lab`, twenty electronic circuits, one lab placed). See
    /// `reserve_chain_produce`.
    #[test]
    fn four_bots_do_not_re_mine_what_an_earlier_chain_of_theirs_produced() {
        /// Every raw unit the plan commits to gathering, by item.
        ///
        /// **`ActionKind::Chop` counts too, and its yield is read off its
        /// effects.** This looked at `Mine` alone until 2026-09-04, when
        /// `Chop` moved ahead of it and stone and coal started arriving off
        /// rocks. A helper blind to that would have read "the fleet digs less
        /// stone" off a plan that gathers exactly as much stone by a different
        /// verb, which is the claim this whole test exists to refuse. A chop's
        /// yield is never inferred from its `count`, which is a number of
        /// entities -- see `ActionKind::Chop`.
        fn mined(net: &ActionNetwork) -> BTreeMap<ItemId, u32> {
            let mut out: BTreeMap<ItemId, u32> = BTreeMap::new();
            for action in net.actions() {
                match &action.kind {
                    ActionKind::Mine { item, count, .. } => {
                        *out.entry(item.clone()).or_default() += count;
                    }
                    ActionKind::Chop { .. } => {
                        for effect in &action.eff {
                            if let crate::action::Effect::GainItem { item, count, .. } = effect {
                                *out.entry(item.clone()).or_default() += count;
                            }
                        }
                    }
                    // A take from a cell is ore a *drill* dug: the take
                    // carries `Effect::ConsumeResource` for exactly what it
                    // stands for (`produce::take_steps`), which is the same
                    // ledger a `Mine` writes. Counted since 2026-09-04, when
                    // the solo plan started drawing thirty-six of its
                    // forty-one iron ore out of the trigger's cell: the plan
                    // digs the same ore, by a machine.
                    ActionKind::Remove { .. } => {
                        for effect in &action.eff {
                            if let crate::action::Effect::ConsumeResource { item, count, .. } =
                                effect
                            {
                                *out.entry(item.clone()).or_default() += count;
                            }
                        }
                    }
                    _ => {}
                }
            }
            out
        }

        fn plan(bots: &[BotId]) -> ActionNetwork {
            let mut state = PlanState::from_world(
                Arc::new(crate::test_world::world_with_trigger_prerequisite()),
                bots,
            );
            for bot in bots {
                // `initiate_missing_players_with_default_inventory`, plus the
                // eight iron plates freeplay really starts a player with.
                state.gain(*bot, "wood", 1);
                state.gain(*bot, "stone-furnace", 1);
                state.gain(*bot, "burner-mining-drill", 1);
                state.gain(*bot, "iron-plate", 8);
            }
            expand(
                &[Goal::Researched("automation".into())],
                &state,
                &have::registry_for(bots),
                BotId(1),
            )
            .expect("rung 1 expands")
        }

        let solo = plan(&[BotId(1)]);
        let fleet = plan(&[BotId(1), BotId(2), BotId(3), BotId(4)]);

        // The single-bot path is the efficient one and must stay exactly where
        // it is: a "fix" that made four bots match one by making one worse
        // would pass the comparison below and be a regression.
        //
        // **Fifty stone -> fifteen**, which is seven furnaces this plan no
        // longer builds. With a roster of one, `have::patch_furnace_budget` is
        // one furnace per ore patch, so a smelt that finds the patch's furnace
        // busy queues behind the batch in it instead of mining for another. It
        // is pinned rather than bounded for the reason above: the number going
        // *down* is the result, and only an exact pin can tell that from four
        // bots being made to look good by making one bot worse.
        //
        // **Both raw figures went up on 2026-09-04 -- 15 stone -> 24 and 29
        // coal -> 33 -- and that is the change working rather than failing.**
        // These are units *gathered*, and a rock is indivisible: one swing at
        // a `rock-huge` hands over twenty-four coal and twenty-four stone
        // whether the plan needed all of it or a third of it. The plan
        // therefore ends up holding more raw material than before while
        // spending far less time on it -- 92 actions -> 86, and 41,835 ticks
        // -> 38,606 in `have::the_single_bot_rung_one_plan_is_untouched`,
        // which schedules this same expansion. Units are the wrong currency
        // for effort now; they are still the right one for *duplication*,
        // which is what this test is about.
        //
        // **Iron 41 -> 91, coal 33 -> 59, stone 24 -> 48, 86 actions -> 82,
        // later on 2026-09-04**, and the iron figure is the one to read
        // first: it did not go up, it became *visible*. The trigger's fifty
        // plates always came out of a burner cell, and `mined` never counted
        // what the drill dug. Now a take spends its ore off the drill's
        // tiles (`produce::take_steps`) and `mined` reads that, so the 91 is
        // the plan's whole iron: 5 dug by hand and 86 by the cell, where it
        // was 41 by hand and 50 the ledger could not see. The solo plan
        // draws its small fragments out of the cell it stood for the
        // trigger instead of digging for them -- 82 actions and a makespan
        // of 34,611 against 38,680 (`have::the_single_bot_rung_one_plan_is_untouched`)
        // -- and swings at a second rock for the coal that keeps the cell
        // running, which is the 59 and the 48.
        //
        // **Coal 59 -> 33, stone 48 -> 24, 82 actions -> 86 on 2026-09-05**,
        // when the drain cap that offered a backlogged cell to any fragment
        // was removed (`produce::Drain`): the small fragments are
        // hand-smelted again instead of waiting 12,000 ticks behind the
        // trigger's fifty, the second rock is not swung at, and the plan is
        // shorter for it (`have::the_single_bot_rung_one_plan_is_untouched`,
        // 31,482 -> 29,260). The iron figure does not move: the plates are
        // the same plates, dug by a hand instead of a drill.
        //
        // **Coal 33 -> 48, stone 24 -> 48, 86 actions -> 80 later on
        // 2026-09-05**, when `expand` started rehearsing: the first coal
        // fragment is priced over the plan's whole coal demand
        // (`have::chop_beats_mining`) and swings a second rock, so the nine
        // coal that were dug a tile at a time come out of that swing with
        // fifteen to spare and twenty-four stone beside them. Units gathered
        // up, actions and makespan down (`have::the_single_bot_rung_one_plan_is_untouched`,
        // 29,260 -> 26,770) -- the same shape as the rock change above, for
        // the same reason.
        assert_eq!(
            mined(&solo),
            BTreeMap::from([
                ("coal".to_string(), 48),
                ("copper-ore".to_string(), 29),
                ("iron-ore".to_string(), 91),
                ("stone".to_string(), 48),
            ]),
            "one bot's rung-1 bill"
        );
        assert_eq!(solo.len(), 80, "one bot's rung-1 step count");

        // The defect, stated as the property it breaks. Four bots dig no more
        // than one bot does -- they may split it differently and they may
        // schedule it differently, but there is no more ore in the ground to
        // dig for the same ten science packs.
        //
        // **Item by item, and no longer an equality.** R3 made the fleet's
        // bill genuinely *smaller* than the solo bot's: `smelt_steps` hands a
        // furnace it would have had to build to another bot, and every bot in
        // this fixture starts holding one (freeplay's seed), so three furnaces
        // now come out of pockets a single-bot plan can never reach. Fifteen
        // stone, exactly -- 50 for one bot, 35 for four. Loosening the
        // assertion to `<=` would let the original duplication back in on a
        // *different* item, so both halves are asserted: no item exceeds the
        // solo bill, and the whole fleet bill is pinned.
        //
        // **Stone is exempt, and the exemption is a policy and not a
        // loophole.** `have::patch_furnace_budget` is one stone furnace per bot
        // per ore patch, because a bot loads and unloads one furnace at a time
        // and independent smelts queueing behind each other is expensive —
        // four ten-plate smelts on one furnace measured 10,899 ticks against
        // 4,971 on four. So a fleet legitimately puts more furnaces on the
        // ground than a solo bot, and the stone under them is bought
        // deliberately. It is pinned exactly below, in both directions, which
        // is the assertion that would catch it growing again.
        for (item, count) in mined(&solo) {
            if item == "stone" {
                continue;
            }
            let fleet_count = mined(&fleet).get(&item).copied().unwrap_or(0);
            // Coal comes off the fixture's `rock-huge` twenty-four at a
            // swing, one swing per owned chain that fuels a cell of its own
            // (the fleet note below), so the fleet may exceed the solo bill
            // by one rock's coal and no more. 54 against 33 since
            // 2026-09-05: the solo plan stopped swinging at a second rock
            // when the drain cap went (`produce::Drain`) and its fragments
            // went back to hand-smelting; the fleet's trigger chain, which
            // is the lead's own, still fuels its cell off a rock of its own.
            //
            // Two rocks since later on 2026-09-05, 78 against 48: a bot with
            // no furnace of its own on the patch now stands one
            // (`have::patch_furnace_budget`), and a bot fuelling a furnace
            // of its own prices its coal over its own demand -- one swings a
            // rock for it, the others dig a few by hand. Gathered, not
            // duplicated: the plates are the same plates, smelted without
            // queueing behind bot 1.
            let slack = if item == "coal" { 48 } else { 0 };
            assert!(
                fleet_count <= count + slack,
                "four bots must not dig more {item} than one bot does for the \
                 same goal (two rocks of coal allowed): {fleet_count} against {count}"
            );
        }
        //
        // **Iron 91 -> 89, coal 33 -> 54 and stone 25 -> 48 on 2026-09-05**,
        // when `Researched` began handing a trigger's craft and the first
        // lab to a lead supplier as owned chains of that bot's, and dealing
        // the packs across the roster. Two fewer ore: the lead's lab comes
        // out of the trigger cell's fifty plates on the same bot, where the
        // chain actor used to dig for a plant and a lab both. Coal and stone
        // are a second rock: the trigger's cell is the lead's own chain now
        // and fuels itself off a `rock-huge` of its own, twenty-four coal
        // and twenty-four stone in one indivisible swing, exactly the solo
        // plan's second swing (59 and 48). Both are still at or under the
        // solo bill item by item, which is the half above, and the pin is
        // what would catch the leftover of that fifty coming to be dug
        // twice.
        //
        // **Coal 54 -> 78 and stone 48 -> 93 later on 2026-09-05**, when a
        // bot with no furnace of its own on the patch started standing one
        // rather than queueing behind bot 1's (`have::patch_furnace_budget`,
        // `tests/furnace_reuse.rs`). Three suppliers each want five stone
        // and a coal or two for a furnace of their own, and each prices that
        // over its own demand: one swings a `rock-huge` (24 coal, 24 stone),
        // one a `rock-big` (20 stone) and digs its coal by hand. Every extra
        // unit is a rock's surplus or a supplier's own fuel, not the solo
        // bill dug twice -- iron and copper, the items a duplication would
        // show on, do not move.
        //
        // **Stone 93 -> 97 on 2026-09-05**, when the rehearsal stopped
        // handing furnaces over (`ExpansionCtx::rehearsing`, and the
        // handover block in `have::smelt_steps` for why). Bot 1's stone
        // forecast now covers the furnaces it stands, so it swings a
        // `rock-huge` (24 stone, beside the coal it wanted anyway) instead
        // of digging one stone; bot 3 takes the `rock-big` bot 4 had, and
        // bot 4 digs its five by hand. Rock surplus, not the solo bill dug
        // twice: iron, copper and coal do not move.
        //
        // **Coal 78 -> 76, iron 89 -> 87 and stone 97 -> 72 later still on
        // 2026-09-05**, when the pack deal was priced in hand time
        // (`produce::hand_ticks`) and seeded with `planned_ticks`. The plant
        // is worth two packs in the hands, not twenty, so the packs go
        // 1 / 1 / 4 / 4 instead of piling onto the bots with no preload:
        // bots 3 and 4 craft theirs from the plates they start with and
        // never go to the patch, so nobody stands a furnace there for them
        // -- the `rock-big` and the hand-dug five are gone from the stone,
        // and with them the coal and the two ore that fed those furnaces.
        // Fewer, not different: iron and copper are still under the solo
        // bill.
        assert_eq!(
            mined(&fleet),
            BTreeMap::from([
                ("coal".to_string(), 76),
                ("copper-ore".to_string(), 29),
                // Four under the solo bill's 91, for the reasons above; the
                // 91 itself is the trigger's fifty, always drilled and now
                // counted, plus what the solo bot digs by hand.
                ("iron-ore".to_string(), 87),
                // The solo bill's own 48 since 2026-09-05 (it was 25, one
                // above the solo bill of the time): the second rock above
                // hands over twenty-four stone whether the plan wanted them
                // or not, so the fleet's extra furnaces come out of a
                // surplus that was already on the ground rather than out of
                // more digging. See the exemption above for why any excess
                // is bought rather than wasted.
                ("stone".to_string(), 72),
            ]),
            "four bots' rung-1 bill"
        );
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
            via: None,
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
                    via: None,
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
                via: None,
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
                via: None,
            },
        ];
        expand(&goals, &state, &reg, BotId(1)).unwrap();
        // Twice over, since `expand` rehearses: each pass rebinds and
        // restores on its own, and the second must see what the first saw.
        assert_eq!(
            *seen.borrow(),
            vec![BotId(2), BotId(1), BotId(2), BotId(1)],
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
                via: None,
            },
            Goal::Produced {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
                unlocks: None,
                via: None,
            },
        ];
        expand(&goals, &state, &reg, BotId(1)).unwrap();
        // Twice over -- `expand` rehearses; see the test above.
        assert_eq!(
            *seen.borrow(),
            vec![BotId(2), BotId(1), BotId(2), BotId(1)],
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
            via: None,
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
            via: None,
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
            via: None,
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
                via: None,
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Bot(BotId(2)),
                via: None,
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
                        via: None,
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
            via: None,
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
                via: None,
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
                via: None,
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
            via: None,
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
                via: None,
            },
            Goal::Have {
                item: "stone".into(),
                count: 1,
                whose: Holder::Anyone,
                via: None,
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
            via: None,
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
            via: None,
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
                via: None,
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
                via: None,
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

    /// How many cogs a plan for `goals` actually makes.
    fn cogs_made(net: &ActionNetwork) -> u32 {
        net.actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count } if item == "cog" => Some(*count),
                _ => None,
            })
            .sum()
    }

    /// A chain's produce **is** stock a later chain of the *same owner* may
    /// count on.
    ///
    /// Two shares of five cogs each, deliberately sized against the same bot.
    /// That is what makes this bite: the second share asks whether that bot
    /// can already count five towards it, and the five the first chain made
    /// are sitting right there in the simulated inventory. Two different bots
    /// would have proved nothing — the second would have looked at an empty
    /// inventory whatever the ledger said.
    ///
    /// Until 2026-09-03 this asserted the opposite, on the grounds that "the
    /// first chain's cogs may end up in hands the second chain never reaches".
    /// That was true when it was written and stopped being true on 2026-09-02,
    /// when a `Holder::Share` began to own the chain it opens: both goals here
    /// state `Holder::Share(BotId(1))`, so both chains are owned by bot 1, and
    /// an owner is a single-candidate constraint in `schedule` with no
    /// fallback tier. There are no two hands for the cogs to be split between.
    ///
    /// Believing otherwise is expensive rather than merely pedantic — it is
    /// what made a four-bot rung-1 plan mine 91 iron-ore where one bot mined
    /// 41, every chain after the first re-making what the first had made.
    #[test]
    fn what_one_owned_chain_made_is_offered_to_a_later_chain_of_the_same_owner() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(Produce));

        let share = |count| Goal::Have {
            item: "cog".into(),
            count,
            whose: Holder::Share(BotId(1)),
            via: None,
        };
        let net = expand(&[share(5), share(5)], &state, &reg, BotId(1)).expect("expands");
        assert_eq!(
            cogs_made(&net),
            5,
            "the second share is satisfied by the first chain's five, which its \
             own runner is provably carrying"
        );
    }

    /// …and it is still **not** stock a goal *any* bot could run may count on.
    ///
    /// The half of the ledger that the narrowing above leaves standing, and
    /// the reason the entry is filed under `Holder::Anyone` rather than
    /// dropped. The same first share, followed by a `Holder::Anyone` goal:
    /// nothing says which bot will run that one, so the five cogs sitting in
    /// bot 1's simulated inventory are not five it can count on, and it makes
    /// its own.
    #[test]
    fn what_one_chain_made_is_not_offered_to_a_goal_any_bot_could_run() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(Produce));

        let net = expand(
            &[
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                    via: None,
                },
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Anyone,
                    via: None,
                },
            ],
            &state,
            &reg,
            BotId(1),
        )
        .expect("expands");
        assert_eq!(
            cogs_made(&net),
            10,
            "an unaddressed goal may be run by a bot that never sees the first \
             chain's cogs, so it makes its own five"
        );
    }

    /// An **unowned** chain's produce is hidden from everyone, the same bot
    /// included.
    ///
    /// The other arm of `reserve_chain_produce`, and the one the narrowing
    /// must not touch. A chain a `converges`ing method opens has no owner —
    /// nothing named a bot for it, only the shape of the decomposition — so
    /// the scheduler will put it on whoever is cheapest and expansion's
    /// `chain_actor` is a simulation convenience, not a commitment. A later
    /// goal stated for that very bot therefore may **not** count on what it
    /// made.
    ///
    /// `Widget` is written for exactly this: it converges, so it opens a
    /// chain, and it takes no holder, so that chain gets no owner.
    #[test]
    fn what_an_unowned_chain_made_is_offered_to_nobody() {
        struct Widget;
        impl Method for Widget {
            fn name(&self) -> &'static str {
                "widget"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "widget")
            }
            fn converges(&self, _goal: &Goal, _state: &PlanState) -> bool {
                true
            }
            fn expand(
                &self,
                _goal: &Goal,
                _ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![Step::Subgoal(Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Anyone,
                    via: None,
                })])
            }
        }

        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(Widget))
            .with(Box::new(Produce));

        let net = expand(
            &[
                Goal::Have {
                    item: "widget".into(),
                    count: 1,
                    whose: Holder::Anyone,
                    via: None,
                },
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                    via: None,
                },
            ],
            &state,
            &reg,
            BotId(1),
        )
        .expect("expands");
        assert_eq!(
            cogs_made(&net),
            10,
            "the widget's chain names no runner, so its cogs are not bot 1's to \
             count even though bot 1 is who they were simulated onto"
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
                    via: None,
                },
                Goal::Have {
                    item: "cog".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                    via: None,
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
                        via: None,
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
                let Goal::Have {
                    item, count, whose, ..
                } = goal
                else {
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
                via: None,
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
            matches!(goal, Goal::Have { item, count, whose, .. }
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

        // `gains`, not `bare`: a chain holds actions that are somebody's,
        // and an action naming no bot at all is left unstamped -- see
        // `an_action_that_is_nobodys_goes_to_the_bot_that_finishes_it_soonest`.
        let mine = gains(&mut ctx, "the taker acts", "iron-gear-wheel", 1);
        let mine_id = mine.id;
        let theirs = gains(&mut ctx, "the supplier acts", "iron-gear-wheel", 1);
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

    /// **An action that is nobody's is not welded to the chain it was
    /// expanded under.** The mechanism behind the green plan's one-bot tail
    /// (`run-1788604520-39283`): `research automation` was emitted inside
    /// the cell's chain, so its only candidate was the chain's owner, bot 1,
    /// which ran it at tick 53,423 behind its whole build queue while its
    /// dependencies had been met at 38,107 and bot 2 had been idle since
    /// 36,027.
    ///
    /// The shape in miniature: one bot owns a chain of a short craft, a
    /// long craft, and between them a `Research` whose only condition is
    /// world-scoped (`Researched`) and whose only effect is too. Three other
    /// bots stand idle. The research may not start before the short craft
    /// (a stated link, as the method states it after the pack inserts) and
    /// takes 500 ticks; the long craft takes 1,000. Stamped, the research is
    /// bot 1's and the plan is 100 + 500 + 1,000; unstamped, an idle bot
    /// runs it alongside the long craft and the plan is 1,100.
    #[test]
    fn an_action_that_is_nobodys_goes_to_the_bot_that_finishes_it_soonest() {
        use crate::action::{Condition, Effect};
        use crate::schedule::schedule;
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        let mut ctx = ExpansionCtx::new(state.fork(), BotId(1));
        let mut net = crate::network::ActionNetwork::new();
        let reg = MethodRegistry::new();

        let mut short = gains(&mut ctx, "craft the lab", "lab", 1);
        short.duration = 100;
        let short_id = short.id;
        let mut research = bare(&mut ctx, "research automation");
        research.kind = crate::action::ActionKind::Research {
            tech: "automation".into(),
        };
        research.pre = vec![Condition::Researched("automation-science-pack".into())];
        research.eff = vec![Effect::Researched("automation".into())];
        research.duration = 500;
        let research_id = research.id;
        assert!(!research.tied_to_runner(), "control: nothing names a bot");
        assert!(short.tied_to_runner(), "control: the craft is its runner's");
        let mut long = gains(&mut ctx, "craft the cell", "inserter", 34);
        long.duration = 1000;
        let long_id = long.id;

        let mut unlock = bare(&mut ctx, "the trigger fires");
        unlock.eff = vec![Effect::Researched("automation-science-pack".into())];
        unlock.duration = 1;

        let steps = vec![
            Step::Act(unlock),
            Step::Owned {
                whose: Holder::Share(BotId(1)),
                steps: vec![
                    Step::Act(short),
                    Step::Act(research),
                    Step::Act(long),
                    Step::Link {
                        from: short_id,
                        to: research_id,
                        lag: 0,
                    },
                ],
            },
        ];
        let mut promised = Vec::new();
        run_steps(steps, &mut ctx, &mut net, &reg, &mut promised).expect("expands");
        net.infer_edges();

        let chain = net.chain_of(short_id).expect("the craft is chained");
        assert_eq!(net.chain_of(long_id), Some(chain));
        assert_eq!(net.owner_of(chain), Some(BotId(1)));
        assert_eq!(
            net.chain_of(research_id),
            None,
            "an action that names no bot is not held to the chain"
        );

        let plan = schedule(&net, &state, &bots).expect("schedules");
        assert_eq!(plan.assignment(short_id), Some(BotId(1)));
        assert_eq!(plan.assignment(long_id), Some(BotId(1)));
        let runner = plan
            .assignment(research_id)
            .expect("the research is scheduled");
        assert_ne!(
            runner,
            BotId(1),
            "the research goes to a bot that finishes it sooner than the chain's owner"
        );
        assert_eq!(
            plan.makespan, 1100,
            "the long craft and the research overlap: {:#?}",
            plan.steps
        );
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
                    via: None,
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
            "and it is spoken for: a goal any bot could run must not plan \
             against it"
        );
        assert_eq!(
            ctx.state.available(&Holder::Bot(BotId(1)), "iron-ore"),
            0,
            "the taker is a different bot and never saw the ore at all"
        );
        // The one figure that is deliberately *not* zero. A `Step::Owned`
        // always owns its chain, so the ore is provably in bot 2's hands and
        // stays there: a later goal stated for bot 2 — which opens a chain
        // owned by bot 2 in its turn, and so runs on bot 2 — is asking about
        // the one inventory the ore is in. Hiding it there is what made a
        // four-bot plan re-mine what it had already mined; see
        // `reserve_chain_produce`.
        assert_eq!(ctx.state.available(&Holder::Bot(BotId(2)), "iron-ore"), 5);
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

        let first = gains(&mut ctx, "one", "iron-gear-wheel", 1);
        let second = gains(&mut ctx, "two", "iron-gear-wheel", 1);
        let steps = vec![Step::Act(first), Step::Act(second)];
        let mut promised = Vec::new();
        run_steps(steps, &mut ctx, &mut net, &reg, &mut promised).expect("expands");

        let chains: BTreeSet<ChainId> = net.actions().filter_map(|a| net.chain_of(a.id)).collect();
        assert_eq!(chains, BTreeSet::from([outer]));
    }

    /// **A goal may not have the stock it was sized against spent underneath
    /// it.** The regression for `run-1788509918-33958`, which reached green
    /// science and then died on `bot 1 has 141 iron-plate, needs 150`.
    ///
    /// The shape, with the live run's numbers in brackets. A goal asks for ten
    /// plates [150] and the bot already holds four [17], so its method plans
    /// to make the six-plate difference [133]. Making them needs a tool [a
    /// drill], the tool costs three plates [nine], and the tool's own
    /// `Have { plate, 3 }` subgoal looks at the four plates sitting in the
    /// inventory, sees no shortfall, and plans nothing -- then spends three of
    /// the four the outer goal had already counted towards its ten. Six made
    /// plus one left is seven [141], the caller's craft demands ten [150], and
    /// `PlanState::lose` refuses.
    ///
    /// The distinction that makes this a defect rather than a world condition:
    /// nothing here is short of anything. Every plate is makeable and the
    /// method to make it is right there -- the plan simply spent the same
    /// stock twice. `refusal_for` classifies `InsufficientItems` as a fault
    /// for exactly this reason, and it is right to.
    #[test]
    fn stock_a_goal_was_sized_against_is_not_spent_underneath_it() {
        /// Nothing left to do: the holder already has what was asked for.
        struct Enough;
        impl Method for Enough {
            fn name(&self) -> &'static str {
                "enough"
            }
            fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, count, whose, .. }
                    if state.available(whose, item) >= *count)
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![])
            }
        }

        /// A big plate order needs a tool to fill it -- the cell in the live
        /// run, and the reason the outer goal has a subtree at all.
        struct Assemble;
        impl Method for Assemble {
            fn name(&self) -> &'static str {
                "assemble"
            }
            fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, count, whose, .. }
                    if item == "plate"
                        && *count >= 10
                        && state.available(whose, item) < *count)
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have {
                    item, count, whose, ..
                } = goal
                else {
                    unreachable!()
                };
                let short = count.saturating_sub(ctx.state.available(whose, item));
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "tool".into(),
                        count: 1,
                        whose: whose.clone(),
                        via: None,
                    }),
                    Step::Act(Box::new(gain_action(ctx, "plate", short))),
                ])
            }
        }

        /// A small plate order is made directly.
        struct Smelt;
        impl Method for Smelt {
            fn name(&self) -> &'static str {
                "smelt"
            }
            fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, count, whose, .. }
                    if item == "plate"
                        && *count < 10
                        && state.available(whose, item) < *count)
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have {
                    item, count, whose, ..
                } = goal
                else {
                    unreachable!()
                };
                let short = count.saturating_sub(ctx.state.available(whose, item));
                Ok(vec![Step::Act(Box::new(gain_action(ctx, "plate", short)))])
            }
        }

        /// The tool costs three plates -- the drill's nine.
        struct MakeTool;
        impl Method for MakeTool {
            fn name(&self) -> &'static str {
                "make-tool"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "tool")
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have { whose, .. } = goal else {
                    unreachable!()
                };
                let mut a = gain_action(ctx, "tool", 1);
                a.eff.push(Effect::LoseItem {
                    who: Actor::Role,
                    item: "plate".into(),
                    count: 3,
                });
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "plate".into(),
                        count: 3,
                        whose: whose.clone(),
                        via: None,
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }

        /// The caller: it asks for ten plates and then spends all ten.
        struct MakeGadget;
        impl Method for MakeGadget {
            fn name(&self) -> &'static str {
                "make-gadget"
            }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "gadget")
            }
            fn expand(
                &self,
                goal: &Goal,
                ctx: &mut ExpansionCtx,
            ) -> Result<Vec<Step>, PlannerError> {
                let Goal::Have { whose, .. } = goal else {
                    unreachable!()
                };
                let mut a = gain_action(ctx, "gadget", 1);
                a.eff.push(Effect::LoseItem {
                    who: Actor::Role,
                    item: "plate".into(),
                    count: 10,
                });
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "plate".into(),
                        count: 10,
                        whose: whose.clone(),
                        via: None,
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }

        let mut state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        // The four plates already in hand: the seventeen the live run had
        // withdrawn out of its furnaces before green's craft asked for 150.
        state.gain(BotId(1), "plate", 4);
        let reg = MethodRegistry::new()
            .with(Box::new(Enough))
            .with(Box::new(MakeGadget))
            .with(Box::new(MakeTool))
            .with(Box::new(Assemble))
            .with(Box::new(Smelt));

        let net = expand(
            &[Goal::Have {
                item: "gadget".into(),
                count: 1,
                whose: Holder::Bot(BotId(1)),
                via: None,
            }],
            &state,
            &reg,
            BotId(1),
        )
        .expect("ten plates is a reachable number of plates");

        // Six for the outer order and three more for the tool: the tool must
        // make its own, not help itself to stock the ten-plate goal has
        // already counted. Before the credit was reserved this summed to six
        // and the expansion died applying the gadget's `LoseItem`.
        let plates: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count } if item == "plate" => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(
            plates,
            9,
            "six for the order and three for the tool: {:?}",
            net.actions().map(|a| &a.label).collect::<Vec<_>>()
        );
    }
}
