//! Assignment of a bot-free action network to concrete bots over time.

use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, ChainId, Ticks};
use crate::network::ActionNetwork;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::Position;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Character walking speed in tiles per tick (roughly 9 tiles/second).
pub const WALK_TILES_PER_TICK: f64 = 0.15;

/// Ticks to get `from` into the annulus `(min_radius, radius]` around `to`.
/// Zero if already there.
///
/// Two ways to be outside it: too far, as ever (walk in, toward `to`, until
/// within `radius`); or, for a placement's annulus, too close — standing
/// inside the footprint the disc used to accept at distance zero. That case
/// walks the other way: away from `to`, until clear of `min_radius`. Every
/// comparison against a bound goes through `total_cmp`, per the planner's
/// determinism rule for float comparisons.
pub fn travel_ticks(from: &Position, to: &Position, min_radius: f64, radius: f64) -> Ticks {
    let distance = calculate_distance(from, to);
    if distance.total_cmp(&min_radius).is_ge() && distance.total_cmp(&radius).is_le() {
        return 0;
    }
    if distance.total_cmp(&min_radius).is_lt() {
        return ((min_radius - distance) / WALK_TILES_PER_TICK).ceil() as Ticks;
    }
    ((distance - radius) / WALK_TILES_PER_TICK).ceil() as Ticks
}

/// The position simulated as reached once a walk into `(min_radius, radius]`
/// around `to` completes.
///
/// **This is the plan's own bookkeeping, not a destination for the executor.**
/// It is what [`schedule`] advances the simulated bot to, and what any replay
/// of a schedule must advance it to, so that the two agree. It is deliberately
/// *not* what [`StepKind::Walk`] carries: naming a point commits to ground the
/// planner cannot see, and run 10's refusal is what that costs. Public so a
/// replay uses this implementation rather than an open-coded `to.x() +
/// min_radius`, which is precisely the sum the ulp correction below exists for.
///
/// For a plain disc (`min_radius == 0.`) this is `to` itself: the centre
/// trivially satisfies "within radius of `to`" for any non-negative radius,
/// which is why a disc's walk has always simply moved the bot onto the
/// target (see the call site in [`schedule`] this feeds — naming a point on
/// the *outer* ring instead would need to know which points are walkable,
/// knowledge the planner deliberately does not have).
///
/// An annulus's inner bound makes the centre the one point that can *never*
/// satisfy it, so this instead picks the point `min_radius` out along a fixed
/// direction. The direction is arbitrary — [`crate::action::Condition::holds`]
/// only measures distance, not bearing — so any direction that lands on the
/// correct distance is exactly as valid as any other; a fixed one keeps this
/// function deterministic without needing to know which directions are
/// actually walkable, the same limitation the disc case already accepts.
///
/// The offset is nudged *outward* when it has to be nudged at all.
/// `to.x() + min_radius` is a rounded sum, and the offset measured back out of
/// it -- which is what [`crate::action::Condition::holds`] and
/// [`travel_ticks`] both go on -- can come back a few ulps below `min_radius`
/// when the sum lands in a coarser binade than the radius itself. That is not
/// cosmetic: the inner bound is inclusive, so a point one ulp short of it
/// fails the very condition it was constructed to satisfy, and the scheduler
/// then rejects its own arrival point. It ended
/// `workspace/runs/run-1788325660-10154` at rung 4, where `-16.0 +
/// 1.2705824974445776` measured back as `1.2705824974445772` and no other
/// candidate bot existed because the chain had an owner.
///
/// The correction rounds away from `to`, never toward it, because the bound is
/// a *minimum*: a point rounded inward stands closer to the site than the
/// entity's own footprint allows, which is exactly the shortfall the annulus
/// was added to prevent, while a point rounded outward is merely a fraction of
/// a nanotile further away. One `next_up` normally suffices; the loop is there
/// so correctness does not rest on "normally".
pub fn arrival_point(to: &Position, min_radius: f64) -> Position {
    if min_radius.total_cmp(&0.0).is_le() {
        return to.clone();
    }
    // `x` starts strictly greater than `to.x()` (a positive `min_radius` was
    // just established), so `next_up` moves it further away and the measured
    // distance strictly increases: the loop terminates.
    let mut x = to.x() + min_radius;
    while calculate_distance(&Position::new(x, to.y()), to)
        .total_cmp(&min_radius)
        .is_lt()
    {
        x = x.next_up();
    }
    Position::new(x, to.y())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StepKind {
    Act {
        action: ActionId,
        label: String,
    },
    /// Go stand in the annulus `(min_radius, radius]` around `to`.
    ///
    /// **`to` is the thing to get near, not a tile to occupy.** All three
    /// fields are the `Condition::AtPosition` this walk exists to satisfy,
    /// copied verbatim, so `to` is routinely a position the bot can never
    /// stand on — the tile an insert's furnace sits on, the tile a place is
    /// about to build on, or the ore a mine consumes. Anywhere in the annulus
    /// satisfies the condition, and the actuator is expected to resolve it
    /// against the game, which is the only party that knows what is walkable.
    ///
    /// # Both bounds are here because both have been dropped before
    ///
    /// `radius` used to be dropped while `travel_ticks` went on using it,
    /// which made every such walk execute as "stand exactly on it". The
    /// pathfinder cannot route onto an occupied tile, so it silently
    /// substituted a goal of its own and the bot ended up wherever that
    /// happened to be — the 2026-08-30 live smelt run's walk `s4`, 9.3 tiles
    /// from where the plan believed it stood.
    ///
    /// `min_radius` was then dropped in the same way, and hidden better. This
    /// step used to carry a *substituted* `to` — the point [`arrival_point`]
    /// picks at exactly `min_radius` along `+x` — with `radius` forced to
    /// zero, so the inner bound was baked into a single named coordinate and
    /// the executor was handed no bound at all. Two things went wrong with
    /// that, both live:
    ///
    /// - The direction is arbitrary and the planner does not know what is
    ///   walkable, so the named point can be inside something else entirely.
    ///   Run 10's walk to `[-61.72941750255542, 11]` was refused before
    ///   dispatch because it "would end at `[-61.7265625, 11]`, inside a
    ///   collision box spanning `[-61.71, 10.54]` to `[-60.91, 11.34]`". The
    ///   guard was right; the destination should never have been chosen here.
    /// - A zero `radius` reaches `approach_radius` and comes back as its floor
    ///   of 0.5, so the pathfinder was free to end half a tile *inside*
    ///   `min_radius` — the exclusion zone the field exists to enforce.
    ///
    /// Handing both bounds over intact lets
    /// [`factorio_bot_core::factorio::rcon::approach_annulus`] ask the game
    /// for a disc that fits inside the annulus, and lets the game pick the
    /// point.
    Walk {
        to: Position,
        /// The annulus's inner bound: how close is *too* close. Zero for
        /// every walk that is not serving a `Place`.
        min_radius: f64,
        radius: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduledStep {
    pub what: StepKind,
    pub bot: BotId,
    pub start: Ticks,
    pub end: Ticks,
}

/// An immutable assignment of actions to bots over time.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub steps: Vec<ScheduledStep>,
    pub makespan: Ticks,
}

impl Schedule {
    pub fn steps_for(&self, bot: BotId) -> Vec<&ScheduledStep> {
        self.steps.iter().filter(|s| s.bot == bot).collect()
    }

    pub fn assignment(&self, action: ActionId) -> Option<BotId> {
        self.steps.iter().find_map(|s| match &s.what {
            StepKind::Act { action: id, .. } if *id == action => Some(s.bot),
            _ => None,
        })
    }
}

struct Candidate {
    action: ActionId,
    bot: BotId,
    /// The longest path from this action to the end of the network — see
    /// [`critical_path`]. The same for every bot offered the action.
    remaining: Ticks,
    /// When the action's dependencies allow it to start, before any walk.
    deps_ready: Ticks,
    /// Where the bot stands once this candidate's walk (if any) is done: the
    /// plan's `arrival_point`, or where it already is.
    arrival: Position,
    /// When the bot sets off. It walks as soon as it is free, even if the
    /// action's dependencies are not ready yet — see `schedule`.
    walk_start: Ticks,
    travel: Ticks,
    /// When the action itself begins: after the walk *and* after the
    /// dependencies, whichever is later.
    act_start: Ticks,
    end: Ticks,
    /// The position this action requires, verbatim from its condition, so the
    /// lookahead can price the walk from another candidate's arrival.
    walk_target: Option<(Position, f64, f64)>,
    /// The lower bound on this bot's finish if it runs this candidate next —
    /// see [`lookahead_bound`]. Filled in once every feasible candidate of the
    /// round is known; zero until then.
    bound: Ticks,
}

impl Candidate {
    /// The ranking key. Ascending, so the smallest wins: the candidate whose
    /// **bot would finish soonest, counting everything that bot still has
    /// ready**, then the one that itself finishes soonest, then the lowest
    /// action id, then the lowest bot id.
    ///
    /// `end` alone used to be the first key — "earliest finish first" — which
    /// defers exactly the work that most needs starting: a far trip finishes
    /// later than every near one, so it waits until nothing nearer is ready,
    /// and whatever lag it gates (a furnace's whole smelting time) starts that
    /// much later. See [`lookahead_bound`] for the rule and the measurements,
    /// including the one that rejected the obvious alternative.
    fn key(&self) -> (Ticks, Ticks, ActionId, BotId) {
        (self.bound, self.end, self.action, self.bot)
    }
}

/// The lower bound on a bot's finish if it runs candidate `next` now, given
/// the other candidates `others` the same bot could run this round.
///
/// ```text
/// bound = max( end(next) + after(next),
///              max over c in others of
///                  max(end(next) + walk(next -> c), deps_ready(c)) + remaining(c) )
/// ```
///
/// where `after(a) = remaining(a) - duration(a)` is the longest path hanging
/// off `a` once `a` is done, `remaining(c)` the longest path through `c`
/// itself, and `walk(next -> c)` the travel from where `next` leaves the bot
/// to where `c` needs it. Both are [`critical_path`] terms; the walk is the
/// one thing a static priority cannot know, because it depends on where the
/// bot is standing *now*.
///
/// # Why a lookahead and not a priority
///
/// The first attempt at fixing the deferred coal trip was the textbook
/// critical-path rule — rank ready actions by `remaining`, longest first,
/// with the old `(end, action, bot)` as tie-breaks. It fixed the fixture it
/// was aimed at (2,682 -> 2,464) and made every real plan **worse**:
/// `researched:automation` 28,023 -> 29,210, `producing:automation-science-
/// pack:6` 46,089 -> 47,515, `producing:logistic-science-pack:6` 217,749 ->
/// 219,274, all on the baseline map with bots 1-4. The listing said why: bot
/// 1 walked to the coal patch **three separate times** (~460 ticks each way),
/// because the three coal mines there feed three different furnaces and so
/// carry three different tails, and work with a longer tail elsewhere got
/// slotted between them. `end`-first had batched them by accident — once
/// standing at the coal, the next coal mine is the one that finishes soonest.
/// A priority computed on the network alone cannot see that the bot is
/// already there; this bound charges the walk it would waste.
///
/// So the rule is: among what this bot could do next, pick the choice under
/// which the bot's own critical path — its remaining ready work, each item
/// reached from where the choice leaves the bot — ends soonest. Starting the
/// long path early and not walking away from co-located work are then the
/// same objective, not two rules that have to be balanced.
///
/// `others` is every feasible candidate on the same bot this round, including
/// chain-opening actions that another bot may in the end take. That
/// overstates what this bot will really do, but it overstates every
/// alternative by the same items, and a bound that counts too much is still a
/// bound.
fn lookahead_bound(
    next: &Candidate,
    others: &[Candidate],
    durations: &BTreeMap<ActionId, Ticks>,
) -> Ticks {
    let own = next.end + next.remaining - durations[&next.action];
    others
        .iter()
        .filter(|c| c.bot == next.bot && c.action != next.action)
        .map(|c| {
            let walk = match &c.walk_target {
                Some((pos, min_radius, radius)) => {
                    travel_ticks(&next.arrival, pos, *min_radius, *radius)
                }
                None => 0,
            };
            (next.end + walk).max(c.deps_ready) + c.remaining
        })
        .fold(own, Ticks::max)
}

/// The longest path from each action to the end of the network: its own
/// duration, plus the largest of `lag + walk + path` over its successors,
/// where `walk` is the travel between the two actions' required positions
/// when both have one.
///
/// This is the critical-path length of list scheduling, computed once, before
/// any bot is chosen, over the network alone. An action on the longest
/// remaining path cannot be deferred without deferring the makespan. It is
/// not used as a priority on its own — see [`lookahead_bound`] for why that
/// was tried and rejected — but as the term every candidate's bound is built
/// from; the walk term is what makes a far trip count as *long*, not merely
/// *late*.
///
/// # Why a walk that belongs to no edge is counted on one
///
/// A network carries no walks — the scheduler emits them, per bot, from
/// wherever that bot happens to stand. But whichever bot runs `b` after `a`
/// has to reach `b`'s position, and if it is the bot that ran `a` (the common
/// case: a chain is one bot) it sets off from `a`'s. So the travel between the
/// two positions is the least walking that edge can cost, and a lower bound is
/// what a priority wants: it never claims work that need not happen.
///
/// # Why it exists
///
/// Measured on `red_science.rs`'s four-bot fixture at `19ac0cc0`, with the
/// ranking key `(end, action, bot)`: every bot placed both furnaces and mined
/// both ores before walking the ~240 ticks to the coal, because each of those
/// finished sooner than the coal trip. The fuel is what starts a furnace, so
/// each furnace's whole smelting lag began only after the last thing the bot
/// did, and the take at the end of the longer smelt waited 202 ticks with the
/// bot standing next to it. Ranked by the bound this feeds, the coal trip is
/// done on the way out, the iron furnace (the longer smelt) is served before
/// the copper one, and the fixture fell 2,682 -> 2,543. The rest of that
/// fixture's time is walking: bot 2 starts 30 tiles east and walks ~1,300 of
/// its 2,543 ticks, so the 2,063 the fixture measured before the fuel edge
/// carried the lag is not a target — that plan took plates from a furnace
/// that had not started.
///
/// Deterministic: a `BTreeMap` keyed by `ActionId`, filled in reverse
/// topological order (`ActionNetwork::topo_order`, itself deterministic), and
/// every term is an integer tick count.
pub fn critical_path(net: &ActionNetwork) -> Result<BTreeMap<ActionId, Ticks>, PlannerError> {
    // Successors, from the predecessor lists the network keeps.
    let mut succs: BTreeMap<ActionId, Vec<(ActionId, Ticks)>> = BTreeMap::new();
    for action in net.actions() {
        for (pred, lag) in net.preds(action.id) {
            succs.entry(pred).or_default().push((action.id, lag));
        }
    }
    let mut remaining: BTreeMap<ActionId, Ticks> = BTreeMap::new();
    for id in net.topo_order()?.into_iter().rev() {
        let action = net
            .action(id)
            .expect("topo_order names this network's actions");
        let from = action.required_position();
        let after = succs
            .get(&id)
            .into_iter()
            .flatten()
            .map(|(succ, lag)| {
                let walk = match (&from, net.action(*succ).and_then(|s| s.required_position())) {
                    (Some((a, _, _)), Some((b, min_radius, radius))) => {
                        travel_ticks(a, &b, min_radius, radius)
                    }
                    _ => 0,
                };
                lag + walk + remaining[succ]
            })
            .max()
            .unwrap_or(0);
        remaining.insert(id, action.duration + after);
    }
    Ok(remaining)
}

/// A pair rejected because one of the action's preconditions would not hold
/// for that bot. Kept only so the error names a plausible pair if *no* pair is
/// feasible.
struct Rejected {
    candidate: Candidate,
    condition: String,
    /// Set when the rejected candidate was the sole candidate of an owned
    /// chain, so the final error can name the owner-binding as the cause
    /// instead of reporting a bare precondition failure.
    owned_chain: Option<ChainId>,
}

/// Assign every action in `net` to one of `bots`, travel-aware and greedy.
///
/// A pure function of its three arguments. Ties break on `(ActionId, BotId)`
/// ascending, so the output is stable across runs.
///
/// A bot walks the moment it is free, not the moment its dependencies are:
///
/// ```text
/// walk_start = free_at[bot]
/// walk_end   = walk_start + travel
/// act_start  = max(walk_end, deps_ready)
/// end        = act_start + duration
/// ```
///
/// Walking across a lag is most of where multi-bot parallelism comes from —
/// heading for the next furnace while the current one smelts. Adding travel
/// *after* `max(free_at, deps_ready)` instead would idle the bot for the lag
/// and then walk, which is strictly worse and never better.
///
/// Assignment happens at **chain** granularity, not action granularity. The
/// first action of a chain (`ActionNetwork::chain_of`) is assigned freely, by
/// the rule above; every later action of that chain goes to the bot the chain
/// is already bound to. A chain's steps pass items to each other through one
/// inventory, and a branching chain — two ingredients that each need producing
/// — has two roots whose first actions carry no `HasItem` precondition to hold
/// them together, so without this they land on different bots and the action
/// consuming both has no feasible bot at all. A chain being opened prefers a
/// bot not already carrying one, so independent chains spread rather than pile
/// up on a bot that is merely nearby — a *preference*, offered as a first tier
/// and abandoned for the rest of the roster when no bot in it can feasibly run
/// the action.
///
/// Binding only ever *reorders and narrows the candidate set*: how a candidate
/// is ranked and judged feasible is unchanged, and no preference can leave an
/// action with no candidate at all. Actions belonging to no chain stay
/// individually assignable and take exactly the path they took before chains
/// existed.
pub fn schedule(
    net: &ActionNetwork,
    state: &PlanState,
    bots: &[BotId],
) -> Result<Schedule, PlannerError> {
    if bots.is_empty() {
        return Err(PlannerError::NoBots);
    }
    net.validate()?;
    let remaining = critical_path(net)?;
    let durations: BTreeMap<ActionId, Ticks> = net.actions().map(|a| (a.id, a.duration)).collect();

    let mut sim = state.fork();
    let mut free_at: BTreeMap<BotId, Ticks> = bots.iter().map(|b| (*b, 0)).collect();
    let mut finished: BTreeMap<ActionId, Ticks> = BTreeMap::new();
    let mut done: BTreeSet<ActionId> = BTreeSet::new();
    let mut steps: Vec<ScheduledStep> = Vec::new();
    let mut chain_binding: BTreeMap<ChainId, BotId> = BTreeMap::new();

    while done.len() < net.len() {
        let ready: Vec<&crate::action::Action> = net
            .actions()
            .filter(|a| !done.contains(&a.id))
            .filter(|a| net.preds(a.id).iter().all(|(p, _)| done.contains(p)))
            .collect();

        let stuck = net.actions().find(|a| !done.contains(&a.id)).map(|a| a.id);
        if ready.is_empty() {
            return Err(PlannerError::Deadlock {
                action: stuck.expect("loop condition guarantees an unfinished action"),
            });
        }

        let mut feasible: Vec<Candidate> = Vec::new();
        let mut best_rejected: Option<Rejected> = None;
        for action in &ready {
            let deps_ready = net
                .preds(action.id)
                .iter()
                .map(|(p, lag)| finished[p] + lag)
                .max()
                .unwrap_or(0);

            // The chain this action belongs to, the bot already running it,
            // and the bot a caller pinned it to, if any.
            let chain = net.chain_of(action.id);
            let bound = chain.and_then(|c| chain_binding.get(&c).copied());
            let owner = chain.and_then(|c| net.owner_of(c));

            // Candidate bots in preference tiers, tried in order. A later tier
            // is reached only when no bot in an earlier one can *feasibly* run
            // the action, so a preference can cost time but can never cost a
            // plan: ranking-then-checking is what the spec forbids, and a
            // preference with no fallback is that same mistake wearing a
            // narrower candidate set.
            let candidate_tiers: Vec<Vec<BotId>> = match action.pinned {
                Some(pinned) if !bots.contains(&pinned) => {
                    return Err(PlannerError::UnknownBot(pinned));
                }
                // Pinning takes precedence over a chain *binding*: the pin is an
                // explicit instruction, the binding an inference. But rather
                // than silently overriding it — which would hand a chain's
                // items to a bot that does not hold them, and fail later with a
                // confusing precondition error somewhere else — a contradiction
                // is reported here, where its cause is still visible. Nothing
                // sets `pinned` today, so this is defensive.
                //
                // A chain *owner* is checked first and on the same footing. An
                // owner is the same class of thing as a pin — a bot named
                // before scheduling begins, either by a caller (`Holder::Bot`)
                // or by the goal's own sizing (`Holder::Share`, since
                // 2026-09-02) — and the harder of the two: it gets no fallback
                // tier. Letting a pin quietly win over it would run the action
                // on a different bot than the rest of the chain, which is
                // precisely the silent override this arm exists to prevent.
                Some(pinned) => {
                    if let (Some(chain), Some(owner)) = (chain, owner)
                        && owner != pinned
                    {
                        return Err(PlannerError::ChainConflict {
                            chain,
                            action: action.id,
                            bound_to: owner,
                            pinned_to: pinned,
                        });
                    }
                    if let (Some(chain), Some(bound)) = (chain, bound)
                        && bound != pinned
                    {
                        return Err(PlannerError::ChainConflict {
                            chain,
                            action: action.id,
                            bound_to: bound,
                            pinned_to: pinned,
                        });
                    }
                    // A pin is an instruction, not a preference: there is no
                    // second tier to fall back to.
                    vec![vec![pinned]]
                }
                // A chain owner names a specific bot before scheduling begins,
                // so it is a hard constraint: no tier falls back past it. The
                // spread preference below is a preference precisely because
                // nothing named a bot for it.
                None if owner.is_some() => vec![vec![owner.expect("just checked")]],
                // An action already in a running chain follows it. Also not a
                // preference — the items are in that bot's inventory and
                // nowhere else.
                None if bound.is_some() => vec![vec![bound.expect("just checked")]],
                // A chain being *opened* prefers a bot that is not already
                // carrying a different chain, so independent chains spread
                // instead of piling onto whichever bot happens to be cheapest.
                // Chains never outnumber bots today, but nothing makes
                // `chain_binding` injective on its own: `free_at` is the only
                // thing pushing the next chain elsewhere, and a bot parked far
                // away can be dearer than a near one running work already —
                // exactly what `travel_cost_can_outweigh_an_idle_bot` asserts.
                // Two smelting chains on one bot then want four furnaces from a
                // stock of two, and since binding leaves that action a single
                // candidate there is no recovery from it.
                //
                // This can cost time: a chain may open on a distant idle bot
                // where a nearer busy one would have finished sooner. Failing
                // to schedule at all is worse than scheduling slowly.
                //
                // Bots already carrying a chain are the *second* tier, not
                // excluded: the opening action may need something only one of
                // them holds, and offering it nobody is how a preference turns
                // into a failed plan. An action in no chain at all is
                // unaffected — it has nothing to keep together, so it keeps the
                // whole roster in one tier and the path it took before chains
                // existed.
                None if chain.is_some() => {
                    let busy: BTreeSet<BotId> = chain_binding.values().copied().collect();
                    let (carrying, free): (Vec<BotId>, Vec<BotId>) =
                        bots.iter().copied().partition(|b| busy.contains(b));
                    [free, carrying]
                        .into_iter()
                        .filter(|tier| !tier.is_empty())
                        .collect()
                }
                None => vec![bots.to_vec()],
            };

            // Bots the game's pathfinder has already refused this destination
            // from where they stand go to the back of their own tier.
            //
            // # Why a tier and not an exclusion
            //
            // Excluding them would let one action's memory make a whole plan
            // impossible: with every bot refused, `best` stays `None` and
            // `schedule` returns `PreconditionUnsatisfied` for work it could
            // have dispatched. A plan that runs and fails leaves a record, a
            // failed walk and a recovery tier; a plan that was never made
            // leaves none of those, and the supervisor closes the milestone
            // `stuck` on the strength of a memory. So this is a preference,
            // exactly like the chain-spread preference above and subject to
            // the same rule: it may reorder and split a tier, never empty one.
            //
            // # Why here
            //
            // This is the only place in the crate that holds both halves of
            // the question the game answered -- *which bot*, and *from where*.
            // `expand` chooses destinations without knowing who will walk to
            // them, and the sites it chooses (`free_area_near`, the ore
            // selectors) are chosen for the plan, not for a bot. Filtering
            // there would either ban a site for everybody on one bot's
            // evidence, or need a roster it does not have.
            //
            // Run `run-1788432181-42528` is what the absence cost: bot 3 was
            // sent at `(-54.5, -12.5)` on five separate plans from the one spot
            // it had been standing at since tick 53 700, and refused before
            // dispatch every time, taking the rest of its chain down with it
            // (`abandon_rest`). The destination was fine -- bot 1 worked that
            // half of the map all run. The *pair* was not.
            let candidate_tiers: Vec<Vec<BotId>> = candidate_tiers
                .into_iter()
                .map(|tier| {
                    let (open, refused): (Vec<BotId>, Vec<BotId>) =
                        tier.into_iter().partition(|bot| {
                            match (sim.bot(*bot), action.required_position()) {
                                (Some(state), Some((pos, _, _))) => {
                                    !sim.is_walk_refused(*bot, &state.position, &pos)
                                }
                                // No position to walk to, or a bot this state
                                // does not know: nothing to remember, and the
                                // candidate loop below reports the unknown bot.
                                _ => true,
                            }
                        });
                    [open, refused]
                })
                .flat_map(|split| split.into_iter().filter(|tier| !tier.is_empty()))
                .collect();

            // Feasibility is judged per action and per tier: another action
            // finding a bot in tier one says nothing about this one.
            let mut feasible_in_an_earlier_tier = false;
            for tier in &candidate_tiers {
                if feasible_in_an_earlier_tier {
                    break;
                }
                for bot in tier.iter().copied() {
                    let from = &sim.bot(bot).ok_or(PlannerError::UnknownBot(bot))?.position;
                    let travel = match action.required_position() {
                        Some((ref pos, min_radius, radius)) => {
                            travel_ticks(from, pos, min_radius, radius)
                        }
                        None => 0,
                    };
                    let walk_start = free_at[&bot];
                    let act_start = (walk_start + travel).max(deps_ready);
                    let end = act_start + action.duration;
                    let walk_target = action.required_position();
                    // A benched bot gets no pairing that would make it walk.
                    //
                    // This is the one exclusion in a function whose every
                    // other memory is a preference, and it is an exclusion
                    // on purpose: the bench is the game's own answer that
                    // the character cannot leave its tile
                    // (`PlanState::benched`), and `run-1788614781-38058`
                    // is what treating that as a preference costs -- the
                    // refusal ledger put bot 6 at the back of its tier, the
                    // tier had one bot in it, and seven plans in a row sent
                    // it the walk the game had already refused four times.
                    // Where the bot already stands (`travel == 0`) it may
                    // still act. The rejection is recorded like any other
                    // infeasible pair, so a plan that has nobody else names
                    // the bench in its error rather than dispatching a walk
                    // it knows will fail.
                    if travel > 0 && sim.is_benched(bot) {
                        let at = sim
                            .benched()
                            .get(&bot)
                            .map(Position::to_string)
                            .unwrap_or_default();
                        let candidate = Candidate {
                            action: action.id,
                            bot,
                            remaining: remaining[&action.id],
                            deps_ready,
                            arrival: from.clone(),
                            walk_target,
                            walk_start,
                            travel,
                            act_start,
                            end,
                            bound: 0,
                        };
                        if best_rejected
                            .as_ref()
                            .is_none_or(|r| candidate.key() < r.candidate.key())
                        {
                            best_rejected = Some(Rejected {
                                condition: format!(
                                    "bot {bot} is benched at {at}: the game refused every \
                                     short hop from where it stands, so it cannot be sent \
                                     anywhere"
                                ),
                                owned_chain: owner.and(chain),
                                candidate,
                            });
                        }
                        continue;
                    }
                    let arrival = match (&walk_target, travel > 0) {
                        (Some((pos, min_radius, _)), true) => arrival_point(pos, *min_radius),
                        _ => from.clone(),
                    };
                    let candidate = Candidate {
                        action: action.id,
                        bot,
                        remaining: remaining[&action.id],
                        deps_ready,
                        arrival,
                        walk_target,
                        walk_start,
                        travel,
                        act_start,
                        end,
                        bound: 0,
                    };

                    // Feasibility is part of selection, not a check on the winner:
                    // a pair whose preconditions cannot hold is never offered, so
                    // another bot can take the action. A positional precondition is
                    // satisfied by the walk this very pair would emit, so it is
                    // tested against a fork with the bot already moved. Forking is
                    // an overlay clone — cheap enough to do per candidate, which is
                    // what the shared `Arc` base is for.
                    let mut trial = sim.fork();
                    if travel > 0 {
                        // The exact target is deliberately dropped *here* and
                        // only here, in favour of `arrival_point`. For a disc
                        // (`min_radius == 0.`) that is the centre, which
                        // satisfies the condition for every radius and so is
                        // the one point that cannot make a feasible pair look
                        // infeasible. Naming a concrete point on the ring
                        // instead would need to know which points are
                        // walkable — terrain, water, cliffs, other players —
                        // and `PlanState` knows only what entities occupy.
                        // A guess there would turn an optimistic estimate into
                        // a confidently wrong one, so the ring is resolved
                        // where the knowledge is: by the game's pathfinder,
                        // from the radius `StepKind::Walk` now carries. An
                        // annulus's inner bound makes the centre the one point
                        // that can *never* satisfy it, so `arrival_point`
                        // picks a point at exactly `min_radius` instead —
                        // still not a claim about where the bot will really
                        // end up, only the least committal point that could
                        // make this precondition true.
                        if let Some((pos, min_radius, _)) = action.required_position() {
                            trial.set_position(bot, arrival_point(&pos, min_radius));
                        }
                    }
                    let failing = action.pre.iter().find(|c| !c.holds(&trial, bot));

                    match failing {
                        None => {
                            // This tier can run the action, so no later tier is
                            // consulted for it — that is what makes the
                            // preference a preference and not a restriction.
                            feasible_in_an_earlier_tier = true;
                            feasible.push(candidate);
                        }
                        Some(condition) => {
                            if best_rejected
                                .as_ref()
                                .is_none_or(|r| candidate.key() < r.candidate.key())
                            {
                                best_rejected = Some(Rejected {
                                    condition: condition.to_string(),
                                    // `owner` names this action's chain owner
                                    // when it has one; pair it with `chain`
                                    // (guaranteed `Some` whenever `owner` is)
                                    // so the owner-binding is what the final
                                    // error blames, not a bare precondition.
                                    owned_chain: owner.and(chain),
                                    candidate,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Every feasible pair of the round is known, so each can be priced
        // against the rest of its bot's ready work.
        let bounds: Vec<Ticks> = feasible
            .iter()
            .map(|c| lookahead_bound(c, &feasible, &durations))
            .collect();
        for (candidate, bound) in feasible.iter_mut().zip(bounds) {
            candidate.bound = bound;
        }
        let best = feasible.into_iter().min_by_key(Candidate::key);

        // Only when no bot can run any ready action is the plan actually stuck.
        let chosen = match best {
            Some(candidate) => candidate,
            None => {
                let rejected = best_rejected.expect("ready and bots are both non-empty");
                return Err(match rejected.owned_chain {
                    // This bot is the chain's owner, so a precondition that
                    // fails for it is not "the world was not as planned" — it
                    // is the binding itself that cannot be met.
                    Some(chain) => PlannerError::ChainOwnerInfeasible {
                        chain,
                        action: rejected.candidate.action,
                        bot: rejected.candidate.bot,
                        condition: rejected.condition,
                    },
                    None => PlannerError::PreconditionUnsatisfied {
                        action: rejected.candidate.action,
                        bot: rejected.candidate.bot,
                        condition: rejected.condition,
                    },
                });
            }
        };
        let action = net
            .action(chosen.action)
            .expect("candidate came from this network");

        // Walk first, so the AtPosition precondition holds by `act_start`. The
        // walk may finish well before it, if the action waits on a lag.
        if chosen.travel > 0 {
            let (target, min_radius, radius) = action
                .required_position()
                .expect("travel is non-zero only when a position is required");
            // The condition, verbatim. The step says what has to become true
            // — "be in `(min_radius, radius]` around `target`" — and says
            // nothing about which point satisfies it, because that is a
            // question about walkable ground and `PlanState` knows only what
            // entities occupy. Resolving it here is what run 10 paid for: the
            // annulus used to be lowered to `arrival_point`'s single named
            // coordinate at zero tolerance, and the point it names along a
            // fixed `+x` landed inside a collision box the planner could not
            // see. See [`StepKind::Walk`].
            steps.push(ScheduledStep {
                what: StepKind::Walk {
                    to: target.clone(),
                    min_radius,
                    radius,
                },
                bot: chosen.bot,
                start: chosen.walk_start,
                end: chosen.walk_start + chosen.travel,
            });
            // The *simulated* arrival is still `arrival_point`, exactly as the
            // per-candidate feasibility fork above used it. It is the least
            // committal point that satisfies the condition, and it is a claim
            // about the plan's own bookkeeping rather than about the world —
            // which is precisely why it must not be what the executor is
            // handed. Keeping it unchanged keeps every downstream tick, and
            // therefore every makespan, identical.
            sim.set_position(chosen.bot, arrival_point(&target, min_radius));
        }

        // No precondition check here: selection already proved every one of them
        // holds for this pair against a fork that made exactly the move above.

        for effect in &action.eff {
            effect.apply(&mut sim, chosen.bot)?;
        }

        steps.push(ScheduledStep {
            what: StepKind::Act {
                action: action.id,
                label: action.label.clone(),
            },
            bot: chosen.bot,
            start: chosen.act_start,
            end: chosen.end,
        });

        // The chain now has a runner; every later action of it follows.
        if let Some(chain) = net.chain_of(chosen.action) {
            chain_binding.insert(chain, chosen.bot);
        }

        free_at.insert(chosen.bot, chosen.end);
        finished.insert(chosen.action, chosen.end);
        done.insert(chosen.action);
    }

    let makespan = steps.iter().map(|s| s.end).max().unwrap_or(0);
    Ok(Schedule { steps, makespan })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionKind, Actor, Condition, Effect};
    use crate::ids::ActionIdGen;
    use crate::network::ActionNetwork;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use std::sync::Arc;

    fn state(bots: &[BotId]) -> PlanState {
        let mut s = PlanState::from_world(Arc::new(fixture_world()), bots);
        for bot in bots {
            s.set_position(*bot, Position::new(0., 0.));
        }
        s
    }

    /// An action needing nothing, at the origin, taking `duration` ticks.
    fn free(id_gen: &mut ActionIdGen, label: &str, duration: Ticks) -> Action {
        Action {
            id: id_gen.next(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![],
            eff: vec![],
            duration,
            pinned: None,
            label: label.into(),
        }
    }

    /// An action requiring the bot to stand within `radius` of `pos` — a
    /// plain disc, `min_radius: 0.0`.
    fn at(id_gen: &mut ActionIdGen, label: &str, pos: Position, radius: f64) -> Action {
        at_annulus(id_gen, label, pos, 0.0, radius)
    }

    /// Like `at`, but with a caller-chosen duration.
    fn at_for(
        id_gen: &mut ActionIdGen,
        label: &str,
        pos: Position,
        radius: f64,
        duration: Ticks,
    ) -> Action {
        Action {
            id: id_gen.next(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius,
                min_radius: 0.0,
            }],
            eff: vec![],
            duration,
            pinned: None,
            label: label.into(),
        }
    }

    /// An action requiring the bot to stand in the annulus
    /// `(min_radius, radius]` around `pos` — the shape a `Place` action's own
    /// `AtPosition` now carries.
    fn at_annulus(
        id_gen: &mut ActionIdGen,
        label: &str,
        pos: Position,
        min_radius: f64,
        radius: f64,
    ) -> Action {
        Action {
            id: id_gen.next(),
            kind: ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            },
            pre: vec![Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius,
                min_radius,
            }],
            eff: vec![],
            duration: 60,
            pinned: None,
            label: label.into(),
        }
    }

    #[test]
    fn a_schedule_survives_a_json_round_trip() {
        // A schedule is the artefact the next increment serves over HTTP.
        use factorio_bot_core::serde_json;
        let plan = Schedule {
            steps: vec![
                ScheduledStep {
                    what: StepKind::Walk {
                        to: Position::new(10., 20.),
                        min_radius: 0.0,
                        radius: 3.0,
                    },
                    bot: BotId(1),
                    start: 0,
                    end: 60,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: ActionId(3),
                        label: "mine 5 iron-ore".into(),
                    },
                    bot: BotId(2),
                    start: 60,
                    end: 360,
                },
            ],
            makespan: 360,
        };
        let json = serde_json::to_string(&plan).expect("serialises");
        let back: Schedule = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, plan);
    }

    #[test]
    fn no_bots_is_an_error() {
        let net = ActionNetwork::new();
        let s = state(&[]);
        assert!(schedule(&net, &s, &[]).is_err());
    }

    #[test]
    fn an_empty_network_schedules_to_zero() {
        let net = ActionNetwork::new();
        let s = state(&[BotId(1)]);
        let result = schedule(&net, &s, &[BotId(1)]).unwrap();
        assert_eq!(result.makespan, 0);
        assert!(result.steps.is_empty());
    }

    #[test]
    fn independent_actions_run_in_parallel_on_two_bots() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut id_gen, "a", 100));
        net.add(free(&mut id_gen, "b", 100));
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 100, "two bots should overlap the work");
    }

    #[test]
    fn one_bot_serialises_the_same_actions() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(free(&mut id_gen, "a", 100));
        net.add(free(&mut id_gen, "b", 100));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.makespan, 200);
    }

    #[test]
    fn a_dependency_lag_delays_the_successor() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(free(&mut id_gen, "insert", 10));
        let b = net.add(free(&mut id_gen, "remove", 10));
        net.link(a, b, 200);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        // insert ends at 10, lag 200 -> remove starts no earlier than 210.
        assert_eq!(result.makespan, 220);
    }

    #[test]
    fn a_walk_is_emitted_when_the_bot_is_out_of_range() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut id_gen, "far", Position::new(30., 0.), 3.0));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert!(
            matches!(result.steps[0].what, StepKind::Walk { .. }),
            "first step should be the walk"
        );
        assert!(matches!(result.steps[1].what, StepKind::Act { .. }));
        // 30 tiles, radius 3: ceil(27 / 0.15) = 180 travel ticks, then 60 duration.
        assert_eq!(result.makespan, 240);
    }

    /// The tolerance a walk exists to satisfy travels with the walk.
    ///
    /// `Condition::AtPosition` carries a radius and `travel_ticks` has always
    /// used it, but the emitted step used to carry only `to` — so "stand within
    /// 10 tiles of the furnace" reached the executor as "stand on the furnace's
    /// tile", and the game's pathfinder was asked for a tile the plan had just
    /// built on.
    ///
    /// Two actions with **different** radii, not one, because a single walk
    /// cannot tell a radius read from its condition from any constant that
    /// happens to match.
    #[test]
    fn a_walk_carries_the_radius_of_the_condition_it_satisfies() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut id_gen, "wide", Position::new(30., 0.), 9.5));
        net.add(at(&mut id_gen, "tight", Position::new(-30., 0.), 2.25));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();

        let radii: Vec<f64> = result
            .steps
            .iter()
            .filter_map(|s| match &s.what {
                StepKind::Walk { radius, .. } => Some(*radius),
                StepKind::Act { .. } => None,
            })
            .collect();
        assert_eq!(radii.len(), 2, "one walk per action: {:?}", result.steps);
        let mut sorted = radii.clone();
        sorted.sort_by(f64::total_cmp);
        assert_eq!(
            sorted,
            vec![2.25, 9.5],
            "each walk carries its own action's radius, not a shared constant"
        );
    }

    #[test]
    fn no_walk_is_emitted_when_already_in_range() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(at(&mut id_gen, "near", Position::new(1., 1.), 5.0));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.makespan, 60);
    }

    /// The defect this annulus exists to fix, reproduced at the scheduler's
    /// own level: a bot standing exactly on a placement's target used to
    /// satisfy a disc precondition with zero travel, so no `Walk` step ever
    /// moved it clear of the footprint it was about to build on
    /// (milestone 4's `place stone-furnace at [38, 16]`, bot standing at
    /// `(38.30, 16.48)` — see `docs/superpowers/notes/2026-09-02-rcon-reply-fix.md`).
    /// With a positive `min_radius`, standing on the target now fails the
    /// precondition, and the scheduler must emit a real walk to satisfy it.
    #[test]
    fn a_walk_is_emitted_when_the_bot_stands_inside_the_annulus() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // The bot starts at the origin (see `state()`), and the target is the
        // origin too: distance zero, which a disc would accept outright.
        net.add(at_annulus(
            &mut id_gen,
            "place on my own feet",
            Position::new(0., 0.),
            1.5,
            10.0,
        ));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        match &result.steps[0].what {
            StepKind::Walk {
                to,
                min_radius,
                radius,
            } => {
                // The stored walk must carry the annulus itself -- not "within
                // 10 of the origin", which the bot already satisfied without
                // moving, and not a single coordinate on the inner edge, which
                // is a guess about walkable ground the planner cannot make.
                // Both bounds, unaltered, is the only claim it can honestly
                // hand the executor.
                assert_eq!(*to, Position::new(0., 0.));
                assert_eq!(*min_radius, 1.5);
                assert_eq!(*radius, 10.0);
            }
            other => panic!(
                "standing inside the inner bound must still produce a walk, got {:?}",
                other
            ),
        }
        assert!(matches!(result.steps[1].what, StepKind::Act { .. }));
        // ceil(1.5 / 0.15) = 10 travel ticks, then the 60-tick action.
        assert_eq!(result.makespan, 70);
    }

    /// The other half of the same fix: a bot standing at a distance the
    /// annulus was always going to accept must not be walked anywhere. An
    /// annulus that is satisfied everywhere the old disc was not sufficient
    /// evidence it does the right thing — it must also stay silent everywhere
    /// the old disc already was.
    #[test]
    fn no_walk_is_emitted_at_a_legitimate_annulus_distance() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Bot at the origin, target two tiles away: inside `(1.5, 10.0]`.
        net.add(at_annulus(
            &mut id_gen,
            "place at a sane distance",
            Position::new(2., 0.),
            1.5,
            10.0,
        ));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(
            result.steps.len(),
            1,
            "no walk should have been scheduled: {:?}",
            result.steps
        );
        assert_eq!(result.makespan, 60, "just the action, no travel");
    }

    /// `PlanState::placement_clearance("stone-furnace")` for a real Factorio
    /// 2.1 prototype set: half the diagonal of the furnace's collision box
    /// plus half the diagonal of the character's. Written out rather than
    /// computed so the reproduction below is pinned to the exact float the
    /// crashing run carried, not to whatever the fixture happens to hold.
    const STONE_FURNACE_CLEARANCE: f64 = 1.2705824974445776;

    /// The defect that ended `workspace/runs/run-1788325660-10154` at rung 4:
    ///
    /// ```text
    /// bot 1 owns chain ChainId(2) because its bill was sized against it,
    /// but between 1.2705824974445776 and 10 of [-16, 18] does not hold there
    /// ```
    ///
    /// `arrival_point` used to return `Position::new(to.x() + min_radius, ...)`
    /// outright, and that sum is *rounded*: measuring the offset back out of it
    /// can land a few ulps below `min_radius`. `-16.0 + 1.2705824974445776`
    /// crosses down into the `[8, 16)` binade, whose ulp is eight times coarser
    /// than the clearance's own, and the round-trip comes back as
    /// `1.2705824974445772` — short of the inclusive inner bound the point was
    /// constructed to sit on. The scheduler then rejected its own arrival
    /// point, and because the chain had an owner there was no second candidate.
    ///
    /// `-16` was the only x of the run's 44 furnace placements whose sum
    /// rounds down; every other one happened to round up and planned fine.
    #[test]
    fn a_walk_into_an_annulus_lands_where_the_condition_holds() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let target = Position::new(-16., 18.);
        net.add(at_annulus(
            &mut id_gen,
            "place stone-furnace at [-16, 18]",
            target.clone(),
            STONE_FURNACE_CLEARANCE,
            10.0,
        ));
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots)
            .expect("a bot 75 tiles away can always walk to a placement site");
        let condition = Condition::AtPosition {
            who: Actor::Role,
            pos: target.clone(),
            radius: 10.0,
            min_radius: STONE_FURNACE_CLEARANCE,
        };
        match &result.steps[0].what {
            StepKind::Walk {
                to,
                min_radius,
                radius,
            } => {
                // The step is the condition. It no longer names a point, so
                // there is no longer a coordinate here that could disagree
                // with the annulus it was derived from.
                assert_eq!(*to, target);
                assert_eq!(*min_radius, STONE_FURNACE_CLEARANCE);
                assert_eq!(*radius, 10.0);
            }
            other => panic!("a bot 75 tiles out must be walked in, got {:?}", other),
        }
        // The ulp regression itself: the point the scheduler *simulates* as
        // reached — which is what its own precondition check runs against, and
        // what rejected the plan in run-1788325660-10154 — must satisfy the
        // condition it was constructed for.
        let mut arrived = state(&bots);
        arrived.set_position(BotId(1), arrival_point(&target, STONE_FURNACE_CLEARANCE));
        assert!(
            condition.holds(&arrived, BotId(1)),
            "the arrival the plan simulates must satisfy the condition it was \
             emitted for: landed {} from the target, inner bound {}",
            calculate_distance(&arrival_point(&target, STONE_FURNACE_CLEARANCE), &target),
            STONE_FURNACE_CLEARANCE,
        );
    }

    /// The class, not just the instance. Whether `to.x() + min_radius` rounds
    /// up or down depends on which binade the sum lands in, so a single
    /// coordinate proves nothing — the run planned 43 furnaces at coordinates
    /// that happened to round the safe way before it hit the one that did not.
    #[test]
    fn an_arrival_point_never_rounds_inside_the_inner_bound() {
        let bots = [BotId(1)];
        let mut checked = 0u32;
        for x in -128i32..=128 {
            for min_radius in [
                STONE_FURNACE_CLEARANCE,
                0.5,
                1.5,
                2.0 / 3.0,
                std::f64::consts::SQRT_2,
            ] {
                let target = Position::new(f64::from(x), 18.);
                let landed = arrival_point(&target, min_radius);
                let condition = Condition::AtPosition {
                    who: Actor::Role,
                    pos: target.clone(),
                    radius: 10.0,
                    min_radius,
                };
                let mut arrived = state(&bots);
                arrived.set_position(BotId(1), landed.clone());
                assert!(
                    condition.holds(&arrived, BotId(1)),
                    "arrival point {:?} for target {:?} and inner bound {} \
                     measures {} -- inside the bound it was built to sit on",
                    landed,
                    target,
                    min_radius,
                    calculate_distance(&landed, &target),
                );
                // Outward rounding only: never more than one ulp of slack, so
                // this is a correction and not a margin.
                assert!(
                    calculate_distance(&landed, &target)
                        .total_cmp(&(min_radius * 1.000_000_1))
                        .is_le(),
                    "arrival point drifted well past the inner bound",
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 257 * 5);
    }

    #[test]
    fn travel_ticks_treats_both_annulus_edges_as_already_arrived() {
        let to = Position::new(0., 0.);
        // Exactly on the inner edge.
        assert_eq!(
            travel_ticks(&Position::new(1.5, 0.), &to, 1.5, 10.0),
            0,
            "the inner edge itself must count as arrived"
        );
        // Exactly on the outer edge.
        assert_eq!(
            travel_ticks(&Position::new(10.0, 0.), &to, 1.5, 10.0),
            0,
            "the outer edge itself must count as arrived"
        );
        // A hair inside the inner edge must still cost a walk out.
        assert_eq!(
            travel_ticks(&Position::new(1.0, 0.), &to, 1.5, 10.0),
            (0.5f64 / WALK_TILES_PER_TICK).ceil() as Ticks,
        );
    }

    #[test]
    fn a_pinned_action_goes_to_its_bot() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut id_gen, "pinned", 60);
        action.pinned = Some(BotId(2));
        let id = net.add(action);
        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.assignment(id), Some(BotId(2)));
    }

    #[test]
    fn an_unsatisfiable_precondition_is_an_error() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut action = free(&mut id_gen, "needs plates", 60);
        action.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        net.add(action);
        let bots = [BotId(1)];
        assert!(schedule(&net, &state(&bots), &bots).is_err());
    }

    #[test]
    fn effects_accumulate_so_a_later_action_sees_them() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut producer = free(&mut id_gen, "produce", 10);
        producer.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        let mut consumer = free(&mut id_gen, "consume", 10);
        consumer.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 5,
        }];
        let p = net.add(producer);
        let c = net.add(consumer);
        net.link(p, c, 0);
        // One bot only, so the consumer sees the producer's items.
        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(result.assignment(c), Some(BotId(1)));
    }

    #[test]
    fn a_bot_walks_across_a_dependency_lag() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Insert the ore, then remove the plate 200 ticks later, 30 tiles away.
        let insert = net.add(free(&mut id_gen, "insert", 10));
        let remove = net.add(at_for(
            &mut id_gen,
            "remove",
            Position::new(30., 0.),
            3.0,
            60,
        ));
        net.link(insert, remove, 200);

        let bots = [BotId(1)];
        let result = schedule(&net, &state(&bots), &bots).unwrap();

        // 30 tiles, radius 3: ceil(27 / 0.15) = 180 travel ticks. The bot is free
        // at 10 and the lag clears at 210, so it walks [10, 190] *during* the
        // lag, waits 20 ticks, and acts [210, 270]. Idling first and walking
        // afterwards would give 210 + 180 + 60 = 450.
        let walk = result
            .steps
            .iter()
            .find(|s| matches!(s.what, StepKind::Walk { .. }))
            .expect("the bot must walk");
        assert_eq!((walk.start, walk.end), (10, 190));
        let act = result
            .steps
            .iter()
            .find(|s| matches!(&s.what, StepKind::Act { action, .. } if *action == remove))
            .expect("the removal is scheduled");
        assert_eq!((act.start, act.end), (210, 270));
        assert_eq!(result.makespan, 270);
    }

    #[test]
    fn a_consumer_goes_to_the_bot_that_holds_the_items() {
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(100., 0.));

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        // Producing happens at the origin, where only bot 1 stands.
        let mut producer = at_for(&mut id_gen, "produce", Position::new(0., 0.), 3.0, 10);
        producer.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-ore".into(),
            count: 5,
        }];
        // Consuming happens where only bot 2 stands, but needs the producer's ore.
        let mut consumer = at_for(&mut id_gen, "consume", Position::new(100., 0.), 3.0, 10);
        consumer.pre.push(Condition::HasItem {
            who: Actor::Role,
            item: "iron-ore".into(),
            count: 5,
        });

        let p = net.add(producer);
        let c = net.add(consumer);
        net.link(p, c, 0);

        let result = schedule(&net, &s, &bots).expect("one bot can do both");

        assert_eq!(result.assignment(p), Some(BotId(1)));
        // Bot 2 is the cheapest by time — zero travel, so it would finish at 20
        // against bot 1's 667 — but it has no ore, so it is not a candidate at
        // all. Ranking without feasibility would bind it and fail the plan.
        assert_eq!(result.assignment(c), Some(BotId(1)));
        // Bot 1 walks 97 tiles: ceil(97 / 0.15) = 647, then acts for 10.
        assert_eq!(result.makespan, 667);
    }

    #[test]
    fn a_branching_chain_lands_wholly_on_one_bot() {
        use crate::ids::ChainIdGen;
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();

        // Two independent producers of *different* items — a branching chain
        // has two roots, and neither carries a `HasItem` precondition that
        // could keep it near the other. The consumer needs both, from one
        // inventory, because `Actor::Role` binds to the single bot that runs
        // it.
        let mut iron = free(&mut id_gen, "make iron", 10);
        iron.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 1,
        }];
        let mut copper = free(&mut id_gen, "make copper", 10);
        copper.eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: "copper-plate".into(),
            count: 1,
        }];
        let mut consumer = free(&mut id_gen, "craft the pack", 10);
        consumer.pre = vec![
            Condition::HasItem {
                who: Actor::Role,
                item: "iron-plate".into(),
                count: 1,
            },
            Condition::HasItem {
                who: Actor::Role,
                item: "copper-plate".into(),
                count: 1,
            },
        ];

        let i = net.add(iron);
        let c = net.add(copper);
        let pack = net.add(consumer);
        net.link(i, pack, 0);
        net.link(c, pack, 0);

        let mut chains = ChainIdGen::new();
        let chain = chains.next();
        for id in [i, c, pack] {
            net.set_chain(id, chain);
        }

        let bots = [BotId(1), BotId(2)];
        let result =
            schedule(&net, &state(&bots), &bots).expect("a chain bound to one bot is schedulable");

        // Both bots are idle at the origin, so without chain binding the two
        // roots are equally cheap and the greedy rule spreads them: the second
        // producer's idle bot finishes at 10 against the first bot's 20. The
        // consumer then finds one plate on each bot and no bot with both.
        let bot = result
            .assignment(i)
            .expect("the iron producer is scheduled");
        assert_eq!(result.assignment(c), Some(bot), "both roots on one bot");
        assert_eq!(result.assignment(pack), Some(bot), "and the consumer too");
    }

    #[test]
    fn a_new_chain_prefers_a_bot_that_is_not_carrying_one() {
        use crate::ids::ChainIdGen;
        // The shape of `travel_cost_can_outweigh_an_idle_bot`: bot 2 is parked
        // 200 tiles from the work, so greedy hands *both* actions to bot 1
        // (1200 beats 1914). Make them the roots of two different chains and
        // that piles two chains onto one bot — which for two smelting chains
        // means four furnaces from a stock of two, with no recovery, because
        // binding leaves each later action a single candidate.
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(200., 0.));

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(
            &mut id_gen,
            "first",
            Position::new(0., 0.),
            3.0,
            600,
        ));
        let second = net.add(at_for(
            &mut id_gen,
            "second",
            Position::new(0., 0.),
            3.0,
            600,
        ));

        let mut chains = ChainIdGen::new();
        net.set_chain(first, chains.next());
        net.set_chain(second, chains.next());

        let result = schedule(&net, &s, &bots).unwrap();

        assert_eq!(result.assignment(first), Some(BotId(1)));
        assert_eq!(
            result.assignment(second),
            Some(BotId(2)),
            "the second chain must open on the bot carrying none, dear though it is"
        );
        // The price of that: bot 2 walks 197 tiles (1314 ticks) and finishes at
        // 1914, where piling both onto bot 1 would have finished at 1200. The
        // chainless version of this network is `travel_cost_can_outweigh_an_idle_bot`,
        // which still asserts 1200 — the preference applies only to chains.
        assert_eq!(result.makespan, 1914);
    }

    #[test]
    fn a_new_chain_takes_a_carrying_bot_when_no_free_bot_can_run_it() {
        use crate::ids::ChainIdGen;
        // The preference must stay a preference. Bot 2 is free of chains and
        // idle at the same tile, so it wins the spread outright — but it does
        // not hold the furnace the second chain's opening action consumes.
        // Narrowing to the unbound bots and stopping there offers that action
        // nobody, and the whole plan fails on a precondition that bot 1 meets.
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        s.gain(BotId(1), "stone-furnace", 1);

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Pinned: with bot 1 the only bot that can open chain B, the
        // lookahead would otherwise hand chain A to the idle bot 2 (makespan
        // 10, not 20) and this test would stop exercising the fallback.
        let mut opens_a = free(&mut id_gen, "opens chain A", 10);
        opens_a.pinned = Some(BotId(1));
        let first = net.add(opens_a);
        let mut needs_furnace = free(&mut id_gen, "opens chain B", 10);
        needs_furnace.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "stone-furnace".into(),
            count: 1,
        }];
        needs_furnace.eff = vec![Effect::LoseItem {
            who: Actor::Role,
            item: "stone-furnace".into(),
            count: 1,
        }];
        let second = net.add(needs_furnace);

        let mut chains = ChainIdGen::new();
        net.set_chain(first, chains.next());
        net.set_chain(second, chains.next());

        let result = schedule(&net, &s, &bots).expect("bot 1 can run both chains");
        assert_eq!(result.assignment(first), Some(BotId(1)));
        assert_eq!(
            result.assignment(second),
            Some(BotId(1)),
            "the second chain falls back to the bot that holds the furnace"
        );
        // Bot 1 runs both, one after the other, at the origin.
        assert_eq!(result.makespan, 20);
    }

    #[test]
    fn a_chain_bound_elsewhere_refuses_a_contradicting_pin() {
        use crate::ids::ChainIdGen;
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(free(&mut id_gen, "opens the chain", 10));
        let mut pinned = free(&mut id_gen, "pinned elsewhere", 10);
        pinned.pinned = Some(BotId(2));
        let second = net.add(pinned);
        net.link(first, second, 0);

        let mut chains = ChainIdGen::new();
        let chain = chains.next();
        net.set_chain(first, chain);
        net.set_chain(second, chain);

        let bots = [BotId(1), BotId(2)];
        // The chain opens on bot 1 (ties break on the lower id), so the pin to
        // bot 2 contradicts it. That is a contradiction in the plan, not a
        // surprise about the world, so it must not arrive as a
        // `PreconditionUnsatisfied` — re-planning from observed state would
        // meet the very same conflict again, forever.
        match schedule(&net, &state(&bots), &bots) {
            Err(PlannerError::ChainConflict {
                chain: c,
                action,
                bound_to,
                pinned_to,
            }) => {
                assert_eq!(c, chain);
                assert_eq!(action, second);
                assert_eq!(bound_to, BotId(1));
                assert_eq!(pinned_to, BotId(2));
            }
            other => panic!(
                "expected a ChainConflict, got {:?}",
                other.map(|s| s.makespan)
            ),
        }
    }

    #[test]
    fn a_pin_that_contradicts_a_chain_owner_is_a_conflict() {
        // An owner is the same class of thing as a pin — a caller's
        // instruction — and the harder one, since it admits no fallback tier.
        // Before the owner was compared here the pin simply won: the action
        // ran on bot 1 while its chain belonged to bot 2, silently, with no
        // error anywhere.
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut pinned = free(&mut id_gen, "pinned away from its owner", 10);
        pinned.pinned = Some(BotId(1));
        let action = net.add(pinned);
        let chain = ChainId(0);
        net.set_chain(action, chain);
        net.set_chain_owner(chain, BotId(2));

        let bots = [BotId(1), BotId(2)];
        match schedule(&net, &state(&bots), &bots) {
            Err(PlannerError::ChainConflict {
                chain: c,
                action: a,
                bound_to,
                pinned_to,
            }) => {
                assert_eq!(c, chain);
                assert_eq!(a, action);
                assert_eq!(
                    bound_to,
                    BotId(2),
                    "the owner is what the pin contradicts, not a binding"
                );
                assert_eq!(pinned_to, BotId(1));
            }
            other => panic!(
                "expected a ChainConflict, got {:?}",
                other.map(|s| s.makespan)
            ),
        }
    }

    #[test]
    fn a_pin_that_agrees_with_the_chain_owner_is_allowed() {
        // The companion to the test above: the check rejects contradiction,
        // not the presence of a pin on an owned chain.
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut pinned = free(&mut id_gen, "pinned to its own owner", 10);
        pinned.pinned = Some(BotId(2));
        let action = net.add(pinned);
        let chain = ChainId(0);
        net.set_chain(action, chain);
        net.set_chain_owner(chain, BotId(2));

        let bots = [BotId(1), BotId(2)];
        let result = schedule(&net, &state(&bots), &bots).expect("pin and owner agree");
        assert_eq!(result.assignment(action), Some(BotId(2)));
    }

    #[test]
    fn an_owner_that_cannot_run_its_chain_blames_the_caller_not_the_world() {
        // `PreconditionUnsatisfied` means "the world was not as planned", which
        // the spec answers by re-planning from observed state. A caller naming
        // a bot that cannot meet the chain's own precondition is not that: the
        // instruction itself is unsatisfiable, and re-planning meets it again.
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut needs_ore = free(&mut id_gen, "the chain the caller asked for", 10);
        needs_ore.pre = vec![Condition::HasItem {
            who: Actor::Role,
            item: "uranium-ore".into(),
            count: 1,
        }];
        let action = net.add(needs_ore);
        let chain = ChainId(0);
        net.set_chain(action, chain);
        net.set_chain_owner(chain, BotId(2));

        // Neither bot holds any, so this is not about bot 2 being unlucky —
        // but the owner tier means bot 2 is the only bot ever offered it.
        let bots = [BotId(1), BotId(2)];
        let err = schedule(&net, &state(&bots), &bots).expect_err("nobody holds uranium ore");
        assert_eq!(
            err.to_string(),
            "bot 2 owns chain ChainId(0) because its bill was sized against it, \
             but has 1 uranium-ore does not hold there"
        );
        match err {
            PlannerError::ChainOwnerInfeasible {
                chain: c,
                action: a,
                bot,
                condition,
            } => {
                assert_eq!(c, chain);
                assert_eq!(
                    a, action,
                    "tier-1 recovery needs the action to re-plan around"
                );
                assert_eq!(bot, BotId(2));
                assert_eq!(condition, "has 1 uranium-ore");
            }
            other => panic!("expected a ChainOwnerInfeasible, got {:?}", other),
        }
    }

    #[test]
    fn scheduling_is_deterministic() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        for i in 0..6 {
            net.add(free(&mut id_gen, &format!("a{}", i), 40));
        }
        let bots = [BotId(1), BotId(2), BotId(3)];
        let first = schedule(&net, &state(&bots), &bots).unwrap();
        let second = schedule(&net, &state(&bots), &bots).unwrap();
        assert_eq!(first.steps, second.steps);
    }

    #[test]
    fn travel_cost_can_outweigh_an_idle_bot() {
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(200., 0.));

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(
            &mut id_gen,
            "first",
            Position::new(0., 0.),
            3.0,
            600,
        ));
        let second = net.add(at_for(
            &mut id_gen,
            "second",
            Position::new(0., 0.),
            3.0,
            600,
        ));

        let result = schedule(&net, &s, &bots).unwrap();

        // Round 1: bot 1 is on the spot, finishing at 600.
        assert_eq!(result.assignment(first), Some(BotId(1)));
        // Round 2: bot 1 queues and finishes at 1200. Bot 2 is idle but must walk
        // 197 tiles: ceil(197 / 0.15) = 1314 travel, so it would finish at 1914.
        // A travel-blind scheduler would hand this to the idle bot and claim 600.
        assert_eq!(result.assignment(second), Some(BotId(1)));
        assert_eq!(result.makespan, 1200);
    }

    #[test]
    fn an_idle_bot_wins_when_its_travel_is_affordable() {
        let bots = [BotId(1), BotId(2)];
        let mut s = PlanState::from_world(Arc::new(fixture_world()), &bots);
        s.set_position(BotId(1), Position::new(0., 0.));
        s.set_position(BotId(2), Position::new(30., 0.));

        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let first = net.add(at_for(
            &mut id_gen,
            "first",
            Position::new(0., 0.),
            3.0,
            600,
        ));
        let second = net.add(at_for(
            &mut id_gen,
            "second",
            Position::new(0., 0.),
            3.0,
            600,
        ));

        let result = schedule(&net, &s, &bots).unwrap();

        assert_eq!(result.assignment(first), Some(BotId(1)));
        // Bot 1 would queue to 1200. Bot 2 walks 27 tiles: ceil(27 / 0.15) = 180
        // travel, finishing at 780. The idle bot wins this time.
        assert_eq!(result.assignment(second), Some(BotId(2)));
        assert_eq!(result.makespan, 780);
    }

    #[test]
    fn ties_break_on_end_time_before_action_id() {
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        // Lower id, longer duration. Higher id, shorter duration. One bot.
        let long = net.add(free(&mut id_gen, "long", 100));
        let short = net.add(free(&mut id_gen, "short", 10));
        let bots = [BotId(1)];

        let result = schedule(&net, &state(&bots), &bots).unwrap();

        let order: Vec<_> = result
            .steps
            .iter()
            .map(|s| match &s.what {
                StepKind::Act { action, .. } => *action,
                StepKind::Walk { .. } => panic!("no walks in this scenario"),
            })
            .collect();
        // Both orders bound the bot's finish at 110, so the bound decides
        // nothing and `end` does: the shorter action is picked first despite
        // its higher id. With (action.id, end, bot) the order would be
        // reversed.
        assert_eq!(order, vec![short, long]);
        assert_eq!(result.makespan, 110);
    }

    /// A far trip that gates two lags must not wait behind near work.
    ///
    /// One bot at the origin, two "furnaces" on the spot with smelts of 600
    /// and 100 ticks, and the coal for both 60 tiles away (a 400-tick walk
    /// each way). Each furnace needs its ore mined (on the spot, 100 ticks),
    /// then fuel, then the ore inserted, then a take after the lag, then a
    /// craft. Under `(end, action, bot)` both ore mines finish sooner than the
    /// coal trip, so the bot mines them first and every lag starts after the
    /// bot's last errand; the plan then waits out the long smelt with nothing
    /// left to do: 1,760 ticks. Under the lookahead the coal goes first, the
    /// 600-tick furnace is loaded before the 100-tick one, and the short
    /// furnace's whole cycle happens under the long one's lag: 1,660. (Ranking
    /// by [`critical_path`] alone also puts the coal first but gives 1,710 —
    /// it loads the long furnace and then walks off to nothing better.)
    #[test]
    fn a_far_trip_gating_a_lag_goes_before_nearer_work() {
        let bots = [BotId(1)];
        let s = state(&bots);
        let here = Position::new(0., 0.);
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let coal = net.add(at_for(
            &mut id_gen,
            "coal",
            Position::new(60., 0.),
            0.0,
            100,
        ));
        let mut takes = Vec::new();
        for (name, lag) in [("long", 600), ("short", 100)] {
            let ore = net.add(at_for(
                &mut id_gen,
                &format!("ore {name}"),
                here.clone(),
                3.0,
                100,
            ));
            let fuel = net.add(at_for(
                &mut id_gen,
                &format!("fuel {name}"),
                here.clone(),
                3.0,
                10,
            ));
            let insert = net.add(at_for(
                &mut id_gen,
                &format!("insert {name}"),
                here.clone(),
                3.0,
                10,
            ));
            let take = net.add(at_for(
                &mut id_gen,
                &format!("take {name}"),
                here.clone(),
                3.0,
                10,
            ));
            let craft = net.add(free(&mut id_gen, &format!("craft {name}"), 50));
            net.link(coal, fuel, 0);
            net.link(ore, insert, 0);
            net.link(fuel, insert, 0);
            net.link(fuel, take, lag);
            net.link(insert, take, lag);
            net.link(take, craft, 0);
            takes.push(take);
        }

        let result = schedule(&net, &s, &bots).unwrap();

        let first_act = result
            .steps
            .iter()
            .find_map(|step| match &step.what {
                StepKind::Act { action, .. } => Some(*action),
                StepKind::Walk { .. } => None,
            })
            .expect("something was scheduled");
        assert_eq!(first_act, coal, "the far coal trip is the first thing done");
        let take_at = |take: ActionId| {
            result
                .steps
                .iter()
                .find(|step| matches!(&step.what, StepKind::Act { action, .. } if *action == take))
                .map(|step| step.start)
                .expect("every take is scheduled")
        };
        assert!(
            take_at(takes[1]) < take_at(takes[0]),
            "the short furnace's take ({}) happens under the long furnace's lag, \
             not after its take ({})",
            take_at(takes[1]),
            take_at(takes[0])
        );
        // 1,760 under `(end, action, bot)`: both ore mines, then the coal,
        // then everything else, and the long smelt waited out at the end.
        assert_eq!(result.makespan, 1660);
    }

    /// The obvious alternative — rank by [`critical_path`] alone — walks away
    /// from work the bot is standing on, and this is the shape that cost the
    /// baseline map 1,187 ticks (see [`lookahead_bound`]).
    ///
    /// Two coal mines on the same far tile, each feeding its own furnace at
    /// the origin; furnace A smelts for 700 ticks, B for 100. Once the first
    /// coal is mined, A's fuel load has the longest remaining path (720 =
    /// 10 + 700 + 10, against the second coal's 620 = 100 + 400 + 10 + 100 +
    /// 10), so a static priority sends the bot 400 ticks home to load it and 400
    /// back for the other coal: 1,890. Pricing the walk in the bound keeps the
    /// bot at the coal until both are mined: 1,700 — which is also what the
    /// old `(end, action, bot)` key gave, by accident: standing at the coal,
    /// the other coal is the action that finishes soonest.
    #[test]
    fn co_located_work_is_finished_before_walking_away() {
        let bots = [BotId(1)];
        let s = state(&bots);
        let here = Position::new(0., 0.);
        let far = Position::new(60., 0.);
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let mut coals = Vec::new();
        for (name, lag) in [("A", 700), ("B", 100)] {
            let coal = net.add(at_for(
                &mut id_gen,
                &format!("coal {name}"),
                far.clone(),
                0.0,
                100,
            ));
            let fuel = net.add(at_for(
                &mut id_gen,
                &format!("fuel {name}"),
                here.clone(),
                3.0,
                10,
            ));
            let take = net.add(at_for(
                &mut id_gen,
                &format!("take {name}"),
                here.clone(),
                3.0,
                10,
            ));
            net.link(coal, fuel, 0);
            net.link(fuel, take, lag);
            coals.push(coal);
        }

        let result = schedule(&net, &s, &bots).unwrap();

        let acts: Vec<ActionId> = result
            .steps
            .iter()
            .filter_map(|step| match &step.what {
                StepKind::Act { action, .. } => Some(*action),
                StepKind::Walk { .. } => None,
            })
            .collect();
        assert_eq!(&acts[..2], &coals[..], "both coals are mined in one trip");
        assert_eq!(result.makespan, 1700);
    }
}
