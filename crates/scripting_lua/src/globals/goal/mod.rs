//! The `goal.*` Lua table: declarative goals planned by `factorio-bot-planner`
//! and executed by `factorio-bot-executor`.
//!
//! Everything a script holds here is a **value**, never a handle: a goal is a
//! plain Lua table (`goal/value.rs`), a plan is a [`plan::PlanValue`] and a
//! run is a [`run::RunValue`]. The integer-handle registries this module used
//! to keep — `Plans`, `Runs` and the `goal.schedule`/`goal.execute`/
//! `goal.progress`/`goal.wait` bindings that indexed them — are gone: a value
//! can be inspected, printed and passed around, and the rules a registry used
//! to police (a plan may be run once) now live on the value itself.
//!
//! Reachable from one line of user Lua, and this crate builds with
//! `panic = "abort"`, so every panic here is a remote kill of the whole server
//! process rather than a failed script. The lint keeps both argument parsing
//! and planner/executor failures on the `LuaError` path.
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod plan;
mod recovery;
mod run;
mod value;

use factorio_bot_core::factorio::rcon::{FactorioRcon, PlacementQuery, PlacementVerdict};
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::miette::miette;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::plan::planner::{Planner, ServerOwnership};
use factorio_bot_executor::walk_memory::reprobe_benched;
use factorio_bot_executor::{Actuator, RconActuator};
use factorio_bot_planner::{BotId, PlanState, PlannerError, holds};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

/// How a run gets its actuator.
///
/// A factory rather than an `Arc<dyn Actuator>` because building one talks to
/// the game: `RconActuator::new` asks who is connected and reads
/// `defines.inventory`. It is also the seam the tests need — everything above
/// this line needs a live Factorio server, everything below it is this module's
/// own logic, and without the seam `goal.start`/`goal.run` themselves can only
/// be tested by reaching around them into `run::spawn`, which pins the helper
/// and not the binding a script actually calls.
type ActuatorFactory = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<Arc<dyn Actuator>, String>> + Send>>
        + Send
        + Sync,
>;

/// How `goal.plan` asks the game whether the sites it just chose are legal.
///
/// # Why this is a separate seam from [`ActuatorFactory`]
///
/// The actuator belongs to a *run*: building one queries the game for the
/// connected roster and `defines.inventory`, and there is no run yet when a
/// plan is being expanded. This is a plain question about the map that needs
/// neither, and giving it its own seam is what lets `goal.plan` stay callable
/// with no run in sight.
///
/// # Why the planner's purity survives this
///
/// It is not called from expansion, and expansion cannot reach it. The
/// sequence is: expand (pure) -> schedule (pure) -> **ask** -> write what the
/// game said into the world's refusal ledger -> expand again (pure, and now
/// reading a world with one more fact in it). Every expansion is still a pure
/// function of a world snapshot and a roster, which is exactly what
/// `expansion_is_deterministic` pins; what changed between two of them is the
/// world, and a changing world is what `PlanState::from_world` is for.
///
/// `None` — no game, or a build with no RCON — means no pre-check, and a plan
/// is returned exactly as it always was. That default is deliberate: an
/// absent checker must never be able to look like a green answer.
pub(crate) type PlacementChecker = Arc<
    dyn Fn(
            Vec<PlacementQuery>,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<PlacementVerdict>, String>> + Send>>
        + Send
        + Sync,
>;

/// How `goal.plan` asks the game what is sitting in the buffers before it
/// plans, answering how many entities were asked about.
///
/// # Why this exists at all
///
/// A convergence hands materials over through a machine: one bot loads a
/// furnace, another unloads it. If the consumer never arrives and the plan is
/// remade, those plates are in that furnace and **the ore they came from is
/// gone from the ground** -- so a replan that cannot see them does not merely
/// forget them, it plans to mine ore that no longer exists, and stalls. The
/// planner half of that (`PlanState::buffers`, `Withdraw`) landed with
/// `docs/superpowers/notes/2026-09-03-buffers-are-visible.md`; this is the
/// call that puts anything in it. Without it every piece of that machinery is
/// correct, tested and inert.
///
/// # A separate seam from [`PlacementChecker`], for a different reason
///
/// The pre-check asks about ground the plan *has already chosen*, so it runs
/// after expansion and its answer feeds a re-expansion. This asks about the
/// world the plan will be built from, so it runs **once, before** any
/// expansion -- see `plan::plan_verified`. The two share a transport and
/// nothing else.
///
/// # Why the future returns a count
///
/// So that a reader can tell "asked, and the buffers were empty" from "never
/// asked". Those two produce identical plans and identical worlds, and only
/// one of them is a defect; the count is what separates them, and
/// `plan::narrate_buffer_refresh` is what prints it.
///
/// `None` -- no game, or a build with no RCON -- means no refresh, which is
/// exactly the behaviour that existed before buffers were visible at all.
pub(crate) type BufferRefresher =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<usize, String>> + Send>> + Send + Sync>;

/// What [`PlanningClockSeam`] is asked for. Every call answers with the
/// `game.tick` at which the game granted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClockCall {
    /// `game.tick_paused = true`.
    Stop,
    /// `game.tick_paused = false`.
    Restart,
    /// Read the clock and touch nothing.
    Read,
}

/// Whether a plan may stop the clock at all.
///
/// The pause is gated on the run **owning** the server
/// ([`ServerOwnership`]): a `--connect` or `--server <host>` run may be
/// attached to someone's live multiplayer game, and `game.tick_paused`
/// there freezes every human player in it. An attached run plans with the
/// clock running, reads the tick around the plan instead, and records the
/// reason on its `planning_timed` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClockPolicy {
    Stop,
    LeaveRunning { reason: &'static str },
}

/// The reason an attached run's `planning_timed` carries.
pub(crate) const ATTACHED_SERVER_REASON: &str = "attached server, clock left running";

/// How `goal.plan` reaches the game clock: stops it while it thinks and
/// restarts it after, or -- when [`ClockPolicy::LeaveRunning`] -- only reads
/// it, so the plan still knows the tick it was made at.
///
/// # Why planning is done against a stopped clock
///
/// Expansion and scheduling are wall-clock work -- seconds for automation,
/// tens of seconds for green -- and the game does not wait for them. A run at
/// `game.speed = s` is charged `60 * s` ticks per second of planning:
/// measured on 2026-09-05, automation cost 334 ticks of planning at 1x and
/// 1,837 at 10x, green 1,960 at 1x and 6,438 at 5x, and that difference was
/// the **entire** gap between the speeds' executed tick counts -- the lag
/// waits themselves overshoot by 2-5 ticks at every speed. Stopping the clock
/// makes the cost zero at every speed rather than proportional to it, and as
/// a side effect the world the plan was made from is the world it is
/// dispatched into.
///
/// `None` -- no game, or a build with no RCON -- means the clock runs on, as
/// it always did, and [`EventKind::PlanningTimed`] records `paused: false` so
/// the charge is visible rather than silent.
///
/// [`EventKind::PlanningTimed`]: factorio_bot_core::record::EventKind::PlanningTimed
pub(crate) type ClockCalls = Arc<
    dyn Fn(ClockCall) -> Pin<Box<dyn Future<Output = Result<u64, String>> + Send>> + Send + Sync,
>;

/// The clock and the rule for using it, handed to `goal.plan` together so
/// that a policy can never be paired with a transport it was not made for.
#[derive(Clone)]
pub(crate) struct PlanningClockSeam {
    pub(crate) calls: ClockCalls,
    pub(crate) policy: ClockPolicy,
}

/// A planner or executor failure is the script's problem, not the process's.
fn goal_error(err: impl std::fmt::Display) -> LuaError {
    LuaError::RuntimeError(format!("goal: {err}"))
}

/// A planner refusal: a **verdict about the world**, carried as an error value
/// a caller can recognise rather than a sentence a caller has to match on.
///
/// Raised, not returned, on purpose. `goal.plan` answers with a plan or it
/// does not answer at all, and a script that ignores a refusal must not
/// quietly receive an empty plan and read it as "nothing left to do" -- that
/// is the exact mistake `goal.holds` was added to stop the supervisor making.
/// So a refusal stays loud by default, and a caller that means to survive one
/// asks [`install_goal_refusal`] (`goal.refusal`) what it is holding.
///
/// `message` is the planner's own sentence, unprefixed, so a report line can
/// put its own word ("refused: ...") in front of it; `Display` adds the `goal:`
/// prefix every other error on this surface carries, so the raise reads exactly
/// as it always did.
#[derive(Debug)]
struct PlanRefusal {
    /// The planner's own miette code, e.g. `planner::research_needs_power`.
    code: String,
    message: String,
}

impl std::fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "goal: {}", self.message)
    }
}

impl std::error::Error for PlanRefusal {}

/// Is this planner failure a verdict about the world, or a fault?
///
/// **A verdict is a statement about the world**: the goal cannot be reached
/// from here, and the planner says exactly why. Nothing is broken, nothing is
/// retryable, and the honest thing for a caller that spans several goals to do
/// is record the reason and carry on -- which is why these carry a
/// [`PlanRefusal`]. **A fault is a defect**: a method contradicting itself, a
/// network it could not have built correctly, a roster or a name that never
/// made sense. Those must end the run loudly, because a run that keeps going
/// past one reports a world condition for what is really a bug.
///
/// The match is exhaustive with no wildcard arm, deliberately. A new
/// `PlannerError` variant must fail this build rather than default into either
/// bucket: defaulting to "verdict" would let a new bug be recorded as a world
/// condition, and defaulting to "fault" would put the next `ResearchNeedsPower`
/// back on the path that destroyed run 31's own record of itself.
///
/// Per-variant reasoning, which is the whole of the decision:
///
/// - [`NoApplicableMethod`](PlannerError::NoApplicableMethod),
///   [`NoRoomToWork`](PlannerError::NoRoomToWork) -- "this world offers no
///   route to that", and "this world has nowhere to stand while doing it".
///   Both are readings of the map.
/// - [`ResearchNeedsPower`](PlannerError::ResearchNeedsPower),
///   [`UnsupportedResearchTrigger`](PlannerError::UnsupportedResearchTrigger),
///   [`SelfUnlockingResearchTrigger`](PlannerError::SelfUnlockingResearchTrigger)
///   -- three ways of saying "that technology cannot be reached from the state
///   this world is in", each naming what is missing. Their own doc comments in
///   `crates/planner/src/error.rs` argue for stating the gap rather than
///   costing it at zero; a caller that dies on the statement gets less than one
///   that records it.
/// - [`PreconditionUnsatisfied`](PlannerError::PreconditionUnsatisfied) --
///   the crate's own doc calls this "the world was not as planned", and the
///   planner's `two_bots_cannot_mine_the_same_exhausted_tile` shows what it
///   carries: an exhausted ore tile, named by the condition that failed.
/// - [`ChainOwnerInfeasible`](PlannerError::ChainOwnerInfeasible) -- the same
///   rejection as the line above with the blame placed on the chain's owner
///   instead of on the world; the `condition` it names is still a fact about
///   the world that does not hold. Splitting the pair would be arbitrary.
///
/// And the faults:
///
/// - [`InsufficientItems`](PlannerError::InsufficientItems) -- only reachable
///   by *applying* an effect, which both `schedule` and the method driver do
///   only after judging the same action feasible. It therefore means the
///   feasibility check and the effect disagree. A world that is genuinely
///   short reports `NoApplicableMethod` or `PreconditionUnsatisfied` instead.
/// - [`UnknownBot`](PlannerError::UnknownBot),
///   [`NoBots`](PlannerError::NoBots) -- the roster and the state disagree, or
///   there is no roster. `goal.plan` refuses both before the planner is
///   reached (`refuse_unknown_bots`), so arriving here at all is a contract
///   violation and not a shortage of anything.
/// - [`CyclicNetwork`](PlannerError::CyclicNetwork),
///   [`Deadlock`](PlannerError::Deadlock) -- a network whose edges cannot be
///   run. No world makes that true or false; a method built it wrong.
/// - [`ChainConflict`](PlannerError::ChainConflict),
///   [`UnownedHandover`](PlannerError::UnownedHandover) -- both say so in
///   their own doc comments: a pin contradicting a binding is "not about the
///   world at all", and a handover naming nobody "is a mistake in the method
///   that wrote the step".
/// - [`ExpansionTooDeep`](PlannerError::ExpansionTooDeep) -- the depth guard,
///   which fires on a method expanding into itself.
/// - [`BufferShort`](PlannerError::BufferShort) -- a plan that tried to take
///   more out of a container than the plan itself believes is in there. That
///   is two parts of one expansion counting the same items, which is
///   `InsufficientItems`' defect in the buffer ledger; it says nothing about
///   the world. Somebody *else* emptying the container between planning and
///   dispatch is a different and real failure, and it surfaces as a refused
///   `Remove` at the game, not here.
/// - [`TooManyCells`](PlannerError::TooManyCells) -- the caller asked for a
///   rate needing more machines than one plan may build. The bound is a
///   constant of this planner, so **no state of the world makes the goal
///   meaningful**: a retry on a different map meets exactly the same number.
///   That is `UnknownTechnology`'s test, and it puts this on the fault side
///   even though its three siblings below are verdicts.
/// - [`UnknownTechnology`](PlannerError::UnknownTechnology) -- the borderline
///   one, and a fault. The name refers to nothing, so no state of the world
///   makes the goal meaningful: either the script has a typo or the world was
///   never told about the technology, and both are broken inputs rather than
///   verdicts. `supervisor.lua` has named "an unknown item or technology" a
///   construction error since it was written, and this keeps that promise.
/// - [`BlueprintRefused`](PlannerError::BlueprintRefused) -- a fault for the
///   same reason as `UnknownTechnology`: it names a defect in the blueprint
///   *string* the script handed over (bad base64, bad zlib, bad JSON, or
///   content outside the decoder's allowlist), not a fact about the world.
///   No map, no research and no amount of exploring changes whether a given
///   string decodes.
/// - [`BlockGroundOccupied`](PlannerError::BlockGroundOccupied) -- its
///   sibling, and the opposite answer: a **verdict**. It names a tile and
///   what is on it, which is precisely a fact about the world, and every one
///   of its cases is something a script can act on -- move the anchor, clear
///   the tree, or walk the bot standing on the footprint off it. Grouping it
///   with `BlueprintRefused` because both mention a blueprint would tell a
///   supervisor that a clear anchor two tiles away was a construction error.
/// - [`NoSiteFound`](PlannerError::NoSiteFound) -- `BlockGroundOccupied`'s
///   sibling and the same verdict: the ground defeated the placement, this
///   time after the planner searched rather than at one caller-chosen
///   anchor. It names how far the search looked and what it hit, which is
///   exactly what a script needs to widen the search, try a different seed,
///   or clear ground before asking again.
/// - The three cell variants -- [`NoCellProduces`](PlannerError::NoCellProduces),
///   [`NoPatchForCell`](PlannerError::NoPatchForCell) and
///   [`NoRoomForCell`](PlannerError::NoRoomForCell) and
///   [`NoRoomForCellNearPower`](PlannerError::NoRoomForCellNearPower) -- verdicts, the first for
///   `NoApplicableMethod`'s reason and the other two for the `PowerPlant*`
///   ones'. "No machine I can build makes this", "this map has no such ore"
///   and "this patch has no edge with room" are all facts a different world
///   makes false, and a script can act on each: hand-craft it instead, chart
///   more map, or aim at another patch.
/// - [`ResearchNeedsRoom`](PlannerError::ResearchNeedsRoom) -- a verdict for
///   the same reason `ResearchNeedsPower` is one, and a fact a script can act
///   on: the lab has nowhere powered to stand, so the answer is to clear
///   ground or to bring supply, not to ask again.
/// - The two `PowerPlant*` variants -- verdicts for the same reason
///   `ResearchNeedsPower` is one, and now the ones a caller actually sees:
///   since the plant landed, an unpowered world is answered by *building* a
///   power plant, so what reaches a script is "there is no water in reach" or
///   "no shoreline here has room for one". Both are facts about the map, and
///   a different map makes them false. There were three until
///   `PowerPlantTooFarFromWater` was removed: it refused water the planner
///   could see, on a borrowed 64-tile bound guarding a walk the schedule
///   already prices, and a distant plant is now a slower plan rather than a
///   refusal.
/// - [`NotHandMinable`](PlannerError::NotHandMinable) -- a verdict about the
///   *prototypes* rather than the map: a character cannot hand-mine crude
///   oil, and no amount of exploring changes that. A script acts on it by
///   not asking a bot for the item at all, so it is a verdict rather than a
///   fault: the goal is meaningful, it is the hand that is wrong.
/// - [`NotCharted`](PlannerError::NotCharted) -- the verdict the first piece
///   of the exploration design
///   (`docs/superpowers/specs/2026-09-04-exploration-design.md`) asked for:
///   the item's resource is nowhere in the model, and the error says where
///   charted ground ends. That is the fact a supervisor script branches on
///   to walk a bot to the frontier and re-plan.
/// - The four extraction rungs a `mine-entity` trigger can stop at --
///   [`UndescribedResearchTrigger`](PlannerError::UndescribedResearchTrigger),
///   [`NoExtractor`](PlannerError::NoExtractor),
///   [`ExtractorLocked`](PlannerError::ExtractorLocked) and
///   [`ExtractionNotModelled`](PlannerError::ExtractionNotModelled) -- all
///   verdicts, each about a different thing a script can act on: the world
///   was captured by a mod that did not describe the trigger (dump again),
///   the prototypes name no machine for the resource, the machine's recipe
///   wants a research the script can plan first, and the planner's own
///   modelling ends there. `UnsupportedResearchTrigger` is their sibling and
///   was a verdict already.
/// - [`SustainSupplyNotStanding`](PlannerError::SustainSupplyNotStanding) --
///   the capacity for a standing rate stands and nothing delivers its inputs
///   without a bot. A verdict about **this planner's modelling**, like
///   `ExtractionNotModelled`: the goal is meaningful and the world may well
///   allow it, but `method::connect` has no caller, so no plan can belt coal
///   into a burner cell. A script acts on it by measuring what it has
///   (`supervisor.sustain`) rather than by asking for a plan that cannot
///   exist. Since the belted-fuel rung it means the narrower thing its own
///   message says: the whole arrangement stands and there is nothing left to
///   build, which is still not satisfaction -- only the record can say
///   whether the rate held.
/// - [`SustainNoOfftake`](PlannerError::SustainNoOfftake) -- the cell can be
///   built and fed and its product cannot be taken away, so its furnace fills
///   its output slot and throttles. A verdict about **the ground** like the
///   two below it, and measured rather than anticipated: the first belted cell
///   ran 27,249 ticks unattended at 100% of nominal tick rate and still came
///   back `SHORT`, with `full_output` tied for the dominant non-working status.
/// - [`SustainNoFuelSource`](PlannerError::SustainNoFuelSource) and
///   [`SustainNoRouteForFuel`](PlannerError::SustainNoRouteForFuel) -- the two
///   ways a self-feeding cell fails to be placeable on a given map: no site on
///   the fuel patch takes a drill with a buffer in front of it, and no belt run
///   joins that buffer to a machine that has to burn it. Both are verdicts
///   about **the ground**, and both name it: the second carries
///   `method::connect`'s own sentence, including the blocked tiles.
/// - [`ProductNotMakeable`](PlannerError::ProductNotMakeable) -- a verdict
///   about the **prototypes**, the sibling of `NotHandMinable`: every recipe
///   that produces the item is in a category this planner has no machine for
///   (`oil-processing`, `chemistry`, `metallurgy`), or no recipe produces it
///   at all. A script acts on it by not asking for the item, exactly as with
///   `NotHandMinable`; no amount of exploring or researching changes it,
///   because it is the shape of the request that is wrong. The message names
///   every producing recipe and its category, which is the diagnosis.
/// - [`FluidNotItem`](PlannerError::FluidNotItem) -- a verdict about the
///   **shape of the request**, and the only one of these that is not about the
///   world at all. `have:<fluid>:N` asks a character to hold something no
///   character inventory accepts in any amount, so nothing a script can do
///   changes the answer. A script acts on it by asking for the fluid where a
///   fluid can be: `gathered:<fluid>` stands a pumpjack and a storage tank on
///   the field. The message names every recipe in this world that produces the
///   fluid and its category, so a reader can see what machine is missing.
/// - [`NoFluidBuffer`](PlannerError::NoFluidBuffer),
///   [`NoTankSite`](PlannerError::NoTankSite),
///   [`NoPipeRoute`](PlannerError::NoPipeRoute) and
///   [`FluidPortUnknown`](PlannerError::FluidPortUnknown) -- `method::gather`'s
///   ladder, and three different kinds of verdict in a row. The first is about
///   the **prototypes** (nothing in this world buffers or carries a fluid), the
///   middle two about the **ground** (no clear footprint at the field centroid;
///   no pipe route to it, with the blocking tiles named), and the last about
///   the **capture** -- the mod sent no `fluidbox_prototypes`, so where a
///   machine takes fluid in or out cannot be read. A script acts on the middle
///   two by clearing ground or choosing another field, and on the outer two by
///   not asking, or by dumping the world again from a mod that describes
///   fluidboxes.
/// - [`BlockDrillUnfed`](PlannerError::BlockDrillUnfed) -- a verdict about
///   **the ground**, and the sibling of `BlockGroundOccupied`: that one says
///   something is on the tile, this one says something is missing under it.
///   A mining drill would stand where there is nothing it can mine, at an
///   anchor the planner's own siting search never screened -- a caller's
///   fixed `Site::At`, or a recovered anchor, which cannot be moved. A script
///   acts on the first by naming a different anchor and on the second by
///   clearing the half-built block, and the message says which it is. Left
///   unrefused it becomes a placement the GAME rejects mid-build, and because
///   nothing is on the tile that rejection names no blocker, so the footprint
///   is remembered as refused and every later replan blames terrain that was
///   never there.
/// - [`SolarBankShort`](PlannerError::SolarBankShort) and
///   [`SolarBankNotSizable`](PlannerError::SolarBankNotSizable) -- verdicts
///   about **what stands on the network**, and a refusal the caller can act on
///   by building accumulators. A solar array with no bank does not run slowly
///   at night, it stops, so this is a refusal a script sees at plan time
///   instead of a run that stalls at a tick nothing explains. The second says
///   the sizing itself is unanswerable -- most often a world dumped before it
///   reported its surface daylight -- and a script acts on it by re-dumping,
///   not by asking for less.
fn refusal_for(err: &PlannerError) -> Option<PlanRefusal> {
    use miette::Diagnostic;

    let verdict = match err {
        PlannerError::NoApplicableMethod { .. }
        | PlannerError::NoRoomToWork { .. }
        | PlannerError::ResearchNeedsPower { .. }
        | PlannerError::ResearchNeedsRoom { .. }
        | PlannerError::PowerPlantNeedsWater { .. }
        | PlannerError::PowerPlantNeedsShore { .. }
        | PlannerError::PowerPlantTooSmall { .. }
        | PlannerError::PowerHeadroomShort { .. }
        | PlannerError::UnsupportedResearchTrigger { .. }
        | PlannerError::SelfUnlockingResearchTrigger { .. }
        | PlannerError::PreconditionUnsatisfied { .. }
        | PlannerError::ChainOwnerInfeasible { .. }
        | PlannerError::NoCellProduces { .. }
        | PlannerError::NoPatchForCell { .. }
        | PlannerError::NoRoomForCell { .. }
        | PlannerError::NoRoomForCellNearPower { .. }
        | PlannerError::NotHandMinable { .. }
        | PlannerError::NotCharted { .. }
        | PlannerError::UndescribedResearchTrigger { .. }
        | PlannerError::NoExtractor { .. }
        | PlannerError::ExtractorLocked { .. }
        | PlannerError::ExtractionNotModelled { .. }
        | PlannerError::NoFluidBuffer { .. }
        | PlannerError::NoTankSite { .. }
        | PlannerError::NoPipeRoute { .. }
        | PlannerError::FluidPortUnknown { .. }
        | PlannerError::BlockGroundOccupied { .. }
        | PlannerError::BlockDrillUnfed { .. }
        | PlannerError::NoSiteFound { .. }
        | PlannerError::SustainSupplyNotStanding { .. }
        | PlannerError::SustainNoFuelSource { .. }
        | PlannerError::SustainNoRouteForFuel { .. }
        | PlannerError::SustainNoOfftake { .. }
        | PlannerError::SolarBankShort { .. }
        | PlannerError::SolarBankNotSizable { .. }
        | PlannerError::ProductNotMakeable(_)
        | PlannerError::FluidNotItem(_) => true,

        PlannerError::InsufficientItems { .. }
        | PlannerError::UnknownBot(_)
        | PlannerError::NoBots
        | PlannerError::CyclicNetwork(_)
        | PlannerError::Deadlock { .. }
        | PlannerError::ChainConflict { .. }
        | PlannerError::UnownedHandover { .. }
        | PlannerError::ExpansionTooDeep { .. }
        | PlannerError::BufferShort { .. }
        | PlannerError::UnknownTechnology { .. }
        | PlannerError::NoMachineForRecipe { .. }
        | PlannerError::TooManyCells { .. }
        | PlannerError::BlueprintRefused { .. } => false,
    };
    verdict.then(|| PlanRefusal {
        // Every variant carries a `#[diagnostic(code(...))]` today. The
        // fallback exists because the trait allows `None`, and it names no
        // variant precisely so it can never be mistaken for one.
        code: err
            .code()
            .map_or_else(|| "planner".to_string(), |code| code.to_string()),
        message: err.to_string(),
    })
}

/// A planner failure as a Lua error, with a verdict marked as one.
///
/// The single seam between `crates/planner`'s error type and every script:
/// both places that call the planner (`expand_goal` here, `schedule` in
/// `plan.rs`) go through it, so a refusal cannot reach Lua by one path
/// unmarked and by the other marked.
fn planner_error(err: PlannerError) -> LuaError {
    match refusal_for(&err) {
        Some(refusal) => LuaError::external(refusal),
        None => goal_error(err),
    }
}

/// Installs `goal.refusal`: what kind of failure is this error?
///
/// The classifier a caller needs to act on the distinction [`refusal_for`]
/// draws. It answers `nil` for everything that is not a planner verdict --
/// including a plain string error, an error from any other part of this
/// surface, and a fault from the planner itself -- because the one answer that
/// must never be given by accident is "this was only a verdict, carry on".
fn install_goal_refusal(lua: &Lua, table: &LuaTable) -> LuaResult<()> {
    table.set(
        "refusal",
        lua.create_function(|lua, value: LuaValue| {
            let LuaValue::Error(err) = &value else {
                return Ok(LuaValue::Nil);
            };
            let Some(refusal) = err.downcast_ref::<PlanRefusal>() else {
                return Ok(LuaValue::Nil);
            };
            let out = lua.create_table()?;
            out.set("code", refusal.code.as_str())?;
            out.set("message", refusal.message.as_str())?;
            Ok(LuaValue::Table(out))
        })?,
    )?;
    Ok(())
}

/// Poisoning is not a reason to abort: the only thing a panicking holder of one
/// of these locks can leave behind is a partially-updated log, and every reader
/// below re-checks what it needs anyway.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Builds the `goal` table.
///
/// `plan_world` is what goals are planned against; `real_world` is what the
/// executor drives. They are now the same world -- see [`Planner`]'s docs: the
/// hypothetical `plan_world` once held was a deep copy that no surviving code
/// wrote to, and that froze every `world.*` read at the run's start state. The
/// two parameters remain only so the split can be reintroduced deliberately if
/// a simulation surface is ever wanted again.
///
/// [`Planner`]: factorio_bot_core::plan::planner::Planner
/// `bots` is the run's roster, as player ids.
pub fn create_lua_goal(
    lua: &Lua,
    plan_world: Arc<FactorioSurface>,
    real_world: Arc<FactorioSurface>,
    rcon: Option<Arc<FactorioRcon>>,
    bots: Vec<u8>,
    server: ServerOwnership,
) -> LuaResult<LuaTable> {
    // Cloned before the actuator factory takes ownership of `rcon`: the
    // pre-check, the buffer refresh and the actuator all need it and none of
    // them owns the others.
    let probe_rcon = rcon.clone();
    let refresh_rcon = rcon.clone();
    let pause_rcon = rcon.clone();
    let actuator: ActuatorFactory = Arc::new(move || {
        let rcon = rcon.clone();
        let world = real_world.clone();
        Box::pin(async move {
            let rcon = rcon
                .ok_or_else(|| "no rcon connection; goal.run needs a running game".to_string())?;
            RconActuator::new(rcon, world)
                .await
                .map(|a| Arc::new(a) as Arc<dyn Actuator>)
                .map_err(|err| err.to_string())
        })
    });
    // Captures `plan_world`, not `real_world`: what this writes is read back
    // by `PlanState::from_world`, which is given the planning world. They are
    // the same object today (see this function's own doc comment) and the
    // pre-check would still work if they were separated -- but it would be
    // writing an observation about the ground into a world nobody plans
    // against, which is the one way this could go quietly useless.
    let checker: Option<PlacementChecker> = probe_rcon.map(|rcon| {
        let world = plan_world.clone();
        Arc::new(move |queries: Vec<PlacementQuery>| {
            let rcon = rcon.clone();
            let world = world.clone();
            Box::pin(async move {
                rcon.can_place_entities(&world, &queries)
                    .await
                    .map_err(|err| err.to_string())
            })
                as Pin<Box<dyn Future<Output = Result<Vec<PlacementVerdict>, String>> + Send>>
        }) as PlacementChecker
    });
    // Captures `plan_world` for exactly the reason the pre-check above does:
    // what this writes is read back by `PlanState::from_world`, which is given
    // the planning world. The two are the same object today, and if they were
    // ever separated again, refreshing the wrong one would leave the planner
    // reading an empty buffer map while believing it had asked -- which is the
    // failure this whole seam exists to make impossible.
    //
    // A `Planner` is built per call rather than held: it is a context holder
    // (`rcon` plus the world), `Planner::new` is two `Arc` clones, and the two
    // things it holds are exactly the two things captured here. Keeping one
    // would be caching a struct that is cheaper to rebuild than to reason
    // about the lifetime of.
    let refresher: Option<BufferRefresher> = refresh_rcon.map(|rcon| {
        let world = plan_world.clone();
        Arc::new(move || {
            let planner = Planner::new(world.clone(), Some(rcon.clone()));
            let rcon = rcon.clone();
            let world = world.clone();
            Box::pin(async move {
                // The same moment, for the same reason: this runs at the top
                // of every plan, and a bench is the one ledger only the game
                // can lift. A benched bot that can move again and is not
                // re-asked stays benched for the whole plan, idle beside
                // work it could do. The probe is four short path requests
                // per benched bot and none at all for a healthy roster.
                let released = reprobe_benched(&rcon, &world).await;
                if !released.is_empty() {
                    factorio_bot_core::paris::info!(
                        "released from the bench: bot(s) <bright-blue>{}</> -- the game will \
                         path them again",
                        released
                            .iter()
                            .map(|p| p.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                planner
                    .refresh_buffers()
                    .await
                    .map_err(|err| err.to_string())
            }) as Pin<Box<dyn Future<Output = Result<usize, String>> + Send>>
        }) as BufferRefresher
    });
    let clock: Option<PlanningClockSeam> = pause_rcon.map(|rcon| PlanningClockSeam {
        calls: Arc::new(move |call: ClockCall| {
            let rcon = rcon.clone();
            Box::pin(async move {
                match call {
                    ClockCall::Stop => rcon.set_tick_paused(true).await,
                    ClockCall::Restart => rcon.set_tick_paused(false).await,
                    ClockCall::Read => rcon
                        .game_tick()
                        .await
                        .and_then(|t| t.ok_or_else(|| miette!("the game answered without a tick"))),
                }
                .map_err(|err| err.to_string())
            }) as Pin<Box<dyn Future<Output = Result<u64, String>> + Send>>
        }),
        policy: match server {
            ServerOwnership::Owned => ClockPolicy::Stop,
            ServerOwnership::Attached => ClockPolicy::LeaveRunning {
                reason: ATTACHED_SERVER_REASON,
            },
        },
    });
    create_lua_goal_with(lua, plan_world, actuator, bots, checker, refresher, clock)
}

/// [`create_lua_goal`] with the actuator supplied rather than built from RCON.
///
/// The only caller in production is `create_lua_goal`; the tests use it to drive
/// the real bindings — `goal.start`/`goal.run` included — against a stub.
pub(crate) fn create_lua_goal_with(
    lua: &Lua,
    plan_world: Arc<FactorioSurface>,
    actuator: ActuatorFactory,
    bots: Vec<u8>,
    placement_checker: Option<PlacementChecker>,
    buffer_refresher: Option<BufferRefresher>,
    planning_clock: Option<PlanningClockSeam>,
) -> LuaResult<LuaTable> {
    let map_table = lua.create_table()?;
    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Goals
-- Declarative goals: say what you want, not how to get it. A goal is expanded
-- into an action network by the planner, assigned to bots by the scheduler,
-- and then executed against the running game.
--
-- Nothing here is a handle. `goal.have`, `goal.researched`, `goal.producing`,
-- `goal.built`, `goal.charted` and `goal.all` build **goal values**: ordinary Lua tables you can read (`g.item`,
-- `g.count`), print and pass around. `goal.plan` turns one into a
-- **PlanValue**, which carries the schedule it was given and answers questions
-- about it (`plan.makespan`, `plan.bots`, `plan.steps`, `plan.tick` -- the
-- game tick the plan was made at, `nil` with no game to ask --
-- `plan:count{...}`, `plan:find{...}`, `plan:for_bot(id)`, `plan:graphviz()`,
-- `plan:gantt(title)`). `goal.start` and `goal.run` execute a plan and hand
-- back a **RunValue** / an observation table rather than a number to look up
-- later. A plan may be executed at most once.
--
-- A run that did not finish its plan is not the end of it: `obs:recover()`
-- proposes the next plan to run -- or `nil`, when the work is done or nothing
-- mechanical is left -- so a script can write its own retry loop, with its own
-- budget, out of the same `goal.run` it already uses. See `goal.run`.
--
-- @module goal

local goal = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return goal"#))?;

    let roster: Vec<BotId> = bots.into_iter().map(BotId).collect();

    // `goal.have` / `goal.researched` / `goal.producing` / `goal.built` / `goal.all`: the goal-value
    // constructors. Pure — they touch neither the world nor the planner, so
    // an unknown item is not an error here; it is one at `goal.plan`, which
    // is the first call that has a world to check it against.
    map_table.set(
        "__doc_entry_have",
        String::from(
            r#"
--- builds a goal value: the bots end up holding items
-- Pure: nothing is planned, expanded or checked against the world here, so an
-- item name no recipe produces raises at `goal.plan`, not at this call. The
-- table it returns is an ordinary Lua table (`{ kind = "have", item = ...,
-- count = ..., bot = ... }`) that you can inspect and `tostring`.
--
-- Any bot may contribute by default: the count is satisfied by the sum across
-- the whole roster, which is what lets the planner split the work. Pass
-- `{ bot = id }` to demand one named bot hold them instead.
-- @string item_name name of the item, e.g. "iron-plate"
-- @number count how many are wanted; an integer >= 1
-- @tparam[opt] table opts `{ bot = id }` to pin the goal to one bot
-- @treturn table a goal value
-- @raise if the item name is empty, or the count is not an integer >= 1
function goal.have(item_name, count, opts)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_researched",
        String::from(
            r#"
--- builds a goal value: a technology is researched
--
-- Pure, like `goal.have`: a technology no force defines raises at
-- `goal.plan`, which is the first call with a world to check it against.
--
-- Planning it decomposes into the technology's prerequisites, researched
-- first and recursively, then the science packs it costs -- its per-unit
-- ingredients times its unit count, produced by the same methods `goal.have`
-- uses -- then the research itself. A technology the force has already
-- researched, or that an earlier goal in the same plan already researched,
-- costs nothing and adds no actions.
-- @string technology_name name of the technology, e.g. "automation"
-- @treturn table a goal value
-- @raise if the technology name is empty
function goal.researched(technology_name)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_producing",
        String::from(
            r#"
--- builds a goal value: a factory that makes an item at a rate
--
-- Pure, like `goal.have`. This is the goal that means *build a machine*: the
-- other three are satisfied by hand-mining and hand-crafting, and this one is
-- satisfied only by machines that stand, face the right way and deliver into
-- one another.
--
-- Planning it builds one or more cells, each a burner mining drill standing on
-- the ore and dropping straight into a stone furnace, and fuels both with
-- coal. Cells that already stand count towards the rate, so replanning a
-- half-built factory finishes it rather than doubling it. Only the plates that
-- smelt from a single ore can be produced this way today; anything else raises
-- at `goal.plan`, which is the first call with a world to check it against.
--
-- **What it does not claim.** `goal.holds` answers this from *structure* -- the
-- machines are there and connected -- and never from observation. A drill whose
-- fuel has run out and a furnace nobody empties both still read as standing.
-- Follow a production milestone with one that dispatches nothing and watches
-- the furnace's output rise; only that is evidence about production.
-- @string item_name name of the item, e.g. "iron-plate"
-- @number per_minute how many a minute are wanted; an integer >= 1
-- @treturn table a goal value
-- @raise if the item name is empty, or the rate is not an integer >= 1
function goal.producing(item_name, per_minute)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_sustain",
        String::from(
            r#"
--- builds a goal value: an item keeps coming out of machines at a rate
--
-- The one goal in this vocabulary that means **keep this true**. Every other
-- kind is one-shot or structural, which is why every measured run's production
-- stops at exactly the bill its plan was written for.
--
-- Read it as: `item_name` comes out of *machines* at `per_minute` or better,
-- continuously, for `window_ticks`, with nothing a bot carried able to explain
-- it.
--
-- **`goal.holds` answers `nil` for it, always.** Not `true` and not `false`:
-- satisfaction of a standing rate is a fact about a *window of history*, and
-- the planner has no clock and no I/O to read one with. A bundle containing
-- one is therefore never reported satisfied either.
--
-- **The planner only ever builds the arrangement.** Planning this plans the
-- same capacity `goal.producing` does -- the cells that stand. When that
-- capacity already stands, planning refuses by name
-- (`planner::sustain_supply_not_standing`) rather than returning an empty
-- plan, because the input that has no standing deliverer is a bot's hands: a
-- burner cell's ore and coal arrive as `insert` actions, and belting them in
-- needs a primitive nothing calls yet. One coal burns for ~1,600 ticks in a
-- drill and ~2,666 in a furnace, so such a cell can sustain no window longer
-- than a single fuel load.
--
-- **What decides it is the run's record, not this call.** Pair it with
-- `supervisor.sustain`, a rung that dispatches nothing for
-- `lead_in_ticks + window_ticks`, and read the answer with
-- `tools/run_analysis.py --sustain <item>:<rate>:<window>:<lead-in>`: machine
-- counters over the trailing window, plus zero feeding-verb dispatches in it
-- and in the lead-in before it. The lead-in is not optional padding -- a stone
-- furnace's input slot holds one stack, 9,600 ticks of hand-fed running, so a
-- window with every bot idle is equally consistent with a factory that was
-- charged by hand before it opened.
--
-- `window_ticks` has **no default**, deliberately: it is the number that
-- decides what a failure means, and a library that guessed it would hand back
-- a verdict nobody derived.
-- @string item_name name of the item, e.g. "iron-plate"
-- @number per_minute how many a minute are wanted; an integer >= 1
-- @number window_ticks how long the rate must hold, in game ticks; an integer >= 1
-- @treturn table a goal value
-- @raise if the item name is empty, or either number is not an integer >= 1
function goal.sustain(item_name, per_minute, window_ticks)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_built",
        String::from(
            r#"
--- builds a goal value: a designed blueprint stands somewhere
--
-- Pure, like `goal.have`: the blueprint string is decoded only at
-- `goal.plan`, which is the first call with a world to check it against.
--
-- **Underground belts are supported**, and were the headline result of the
-- work that added this goal. They used to be refused by name -- neither the
-- entity type nor the mod's placement RPC could say which half of a pair was
-- being built, so the two halves would have been placed as the same entity: a
-- block that stands 100% correctly and moves nothing. Both halves now carry
-- their own `input`/`output`, verified off a live surface, so a blueprint
-- containing them plans and builds like any other.
--
-- Planning it means *the entities the blueprint names that are not yet
-- standing*, re-derived against the world on every expansion rather than
-- remembered -- so replanning after a partial build finishes it rather than
-- doubling it, and building an already-standing blueprint plans nothing at
-- all. "Already standing" means the same name on the same tile facing the
-- same way, and the same underground half: an entity turned the wrong way is
-- *not* built, and is reported rather than skipped.
--
-- **Three ways to say where.** The second argument picks one:
--
--   * `goal.built(bp, { x = ..., y = ... })` -- this exact anchor, or refuse.
--     The original behaviour: the block's own origin lands there and nowhere
--     else.
--   * `goal.built(bp, { near = { x = ..., y = ... } })` -- search outward from
--     that point for a clear footprint.
--   * `goal.built(bp)` -- no second argument at all: search outward from the
--     roster's own centroid.
--
-- An anchor and a near hint are mutually exclusive; passing both raises. Only
-- the first form can refuse for the ground being occupied at the one tile you
-- named -- the other two search, and refuse only when nothing in range is
-- clear (see `goal.plan`'s `NoSiteFound`, surfaced through `goal.refusal`).
--
-- Four other things refuse it, each by name at `goal.plan`: a string that does
-- not decode; content the decoder does not understand (a tile, a recipe, a
-- module request, a circuit wire); an entity already standing on the right
-- tile the wrong way round, which nothing here can rotate or remove; and, for
-- an explicit anchor, **ground that is not clear** -- which names the tile and
-- what is on it, including the case of one of your own bots standing on the
-- footprint, cleared by walking rather than by moving the block.
--
-- The entities are split into bands across the roster, one band per bot, cut
-- across the block's longer axis so each band is a region a bot can work
-- without crossing another's.
-- @string blueprint_string the blueprint, in Factorio's exported string form
-- @tparam[opt] table site `{ x = ..., y = ... }` to demand this exact anchor,
--   `{ near = { x = ..., y = ... } }` to search outward from a hint, or
--   omitted entirely to search outward from the roster
-- @treturn table a goal value
-- @raise if the blueprint string is empty, or `site` is neither of the two
--   shapes above, or names both an anchor and a near hint
function goal.built(blueprint_string, site)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_charted",
        String::from(
            r#"
--- builds a goal value: the ground within a radius has been looked at
--
-- Pure, like `goal.have`. The exploration primitive: every other goal names
-- something to end up *with*, and this one names ground to end up having
-- *seen*.
--
-- It exists because `goal.plan` can refuse with `planner::not_charted` -- the
-- item's resource is nowhere in the world model, and the refusal says where
-- charted ground ends. No amount of crafting, research or building clears
-- that refusal; somebody has to go and look. This is how a script asks for
-- that, and the refusal's own message names the frontier to aim at.
--
-- Planning it emits one `survey` step per blind probe: seventeen points are
-- checked -- the centre, then the eight compass directions at half the radius
-- and at the full radius -- and a bot is walked to each one the model has no
-- ground for. Charting is the *engine's* response to a character standing
-- somewhere new, so a survey asks the game for nothing beyond the walk.
--
-- **A disc that is already charted plans nothing**, so a supervisor loop can
-- re-issue this every round without paying for it twice, and `goal.holds`
-- answers it directly.
--
-- **What it does not claim.** That anything is *there*. A survey that walks
-- the whole disc and finds bare grass has succeeded -- and that is the useful
-- outcome, because it turns "unexplored, so unknown" into "looked, and it is
-- not there", which are genuinely different answers.
-- @number x centre of the disc
-- @number y centre of the disc
-- @number radius how far out to look, in tiles; must be > 0
-- @treturn table a goal value
-- @raise if any argument is not a finite number, or the radius is not positive
function goal.charted(x, y, radius)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_all",
        String::from(
            r#"
--- builds a goal value: every sub-goal, in one plan
-- The sub-goals are planned together rather than one after another, so work
-- shared between them is done once.
-- @tparam table goals a list of goal values
-- @treturn table a goal value
-- @raise if the list is empty, or holds anything that is not a goal value
function goal.all(goals)
end
"#,
        ),
    )?;
    value::install_goal_constructors(lua, &map_table)?;

    // `goal.plan`
    map_table.set(
        "__doc_entry_plan",
        String::from(
            r#"
--- expands a goal value and schedules it against one roster, in one call
-- Consumes a goal value: a table with a `kind` field, such as
-- `{ kind = "have", item = "iron-plate", count = 8 }`, built by `goal.have`,
-- `goal.researched`, `goal.producing`, `goal.built` or `goal.all`. Expansion and scheduling share the
-- same roster -- `SplitAcrossBots` sizes each bot's share against that bot's
-- own holdings, so a network expanded for four bots only ever makes sense
-- scheduled on those same four; this call is what makes the mismatch
-- unrepresentable.
-- @tparam table goal a goal value
-- @tparam[opt] table opts `{ bots = { ... } }` -- bot ids, not a count;
--   defaults to every bot in this run
--
-- Every entry of `plan.steps` carries `bot`, `start`, `finish` and `kind`.
-- `finish`, never `end`: `end` is a Lua keyword, so `step.end` does not parse.
-- A `kind == "walk"` step also carries `to` -- a `types.Position` -- and the
-- two bounds that walk's own precondition asked for. `radius` is what tells
-- "stand on this tile" from "stand near it", which for a place, insert or
-- remove is the difference between a reachable request and the entity's own
-- tile; a walk that only has to get close carries a non-zero one.
-- `min_radius` is the other end: how close is *too* close. It is non-zero
-- only for a walk serving a placement, where `to` is the ground the building
-- will stand on and the acting bot must be clear of it.
-- Both radii and the other scalar fields are accepted by `plan:count{...}`
-- and `plan:find{...}`; `to` and `pos` are not, being tables rather than
-- comparable values.
--
-- A `kind == "place"` step carries `entity`, `pos`, and -- since 2026-09-05
-- -- `direction` (Factorio 2.x's 16-point `defines.direction`, already
-- migrated off a pre-2.0 blueprint's eight-point scale) plus
-- `underground_half`, which is `"input"` or `"output"` for one half of an
-- underground-belt pair and absent for everything else. Both exist because
-- placing correctly and functioning are separate concerns: a belt on the
-- right tile facing the wrong way, and two underground halves that are the
-- same half, each stand perfectly and carry nothing. Without these a caller
-- verifying a build had to re-derive the direction migration itself.
-- Every step also carries `label`, which for a `goal.built` placement always
-- contains "block band N" -- that is what tells the block's own placements
-- apart from the scaffolding a plan builds for itself (furnaces to smelt the
-- plates, a lab to run a research), which are `kind == "place"` too.
--
-- A `kind == "chop"` step is a swing at a standing tree or rock rather than at
-- an ore tile, and it is its own kind because its `entity` and its `item` are
-- different names -- `tree-01` yields `wood` -- where a `kind == "mine"`
-- step's one `item` is both. Its `count` counts entities, not items; what
-- arrives is the entity's own fixed yield.
--
-- **This asks the game before it hands the plan back.** Once the sites are
-- chosen, one RCON call puts every placement in the plan to
-- `surface.can_place_entity`; any the game would refuse is remembered for the
-- rest of the run (`record.refusals()` writes them out) and the goal is
-- expanded again around them, up to twice. So a plan that would have died on
-- `can_place_entity said 'no'` mid-run is re-sited before anything is
-- dispatched, at a cost of one round trip for a plan with nothing wrong with
-- it and none at all for a plan that places nothing.
--
-- A green answer is **not a guarantee**. It is what the game said at the tick
-- it was asked, and the plan runs afterwards: a bot can walk into the
-- footprint before the build happens. A build can still be refused at
-- dispatch, exactly as before.
-- @treturn PlanValue the expanded, scheduled plan
-- @raise if the goal names an unknown item or technology, if any bot in
--   `opts.bots` is not a connected player, or if `opts.bots` is empty
function goal.plan(goal, opts)
end
"#,
        ),
    )?;
    plan::install_goal_plan(
        lua,
        &map_table,
        plan_world.clone(),
        roster.clone(),
        placement_checker,
        buffer_refresher,
        planning_clock,
    )?;

    // `goal.holds`
    map_table.set(
        "__doc_entry_holds",
        String::from(
            r#"
--- answers whether a goal already holds, without planning it
-- The question satisfaction really is. A loop that runs a goal to completion
-- has to decide when it is done, and until this existed the only signal was
-- "the plan came back empty" -- which is a fact about the planner, not about
-- the goal. The two agree today, and `goal.plan` is still what you call to
-- find out *how* to get there; this is how you find out whether you already
-- are.
--
-- Reads the same world snapshot `goal.plan` does, for the same roster, so a
-- `have` goal with no `bot` is satisfied by the sum across the roster and one
-- with a `bot` is satisfied only by that bot's own inventory.
--
-- Three answers, not two. `nil` means the goal names something possession
-- cannot settle -- a production, which is an event and not a state -- so a
-- caller must not read it as "no": treating `nil` as unfinished re-runs work
-- that may be done, and treating it as finished is the lie this call exists
-- to prevent. No constructor on this table builds such a goal today, so today
-- every answer is a boolean; the third case is what a caller must not have
-- assumed away by the time one does.
-- @tparam table goal a goal value
-- @tparam[opt] table opts `{ bots = { ... } }` -- bot ids, not a count;
--   defaults to every bot in this run
-- @treturn boolean|nil true, false, or nil when the goal cannot be answered
--   by looking at the world
-- @raise if any bot in `opts.bots` is not a connected player, or if
--   `opts.bots` is empty
function goal.holds(goal, opts)
end
"#,
        ),
    )?;
    install_goal_holds(lua, &map_table, plan_world.clone(), roster.clone())?;

    // `goal.refusal`: the classifier that makes a refusal survivable.
    map_table.set(
        "__doc_entry_refusal",
        String::from(
            r#"
--- tells a planner verdict from a planner fault
-- `goal.plan` raises when it cannot plan, and the two reasons it can have are
-- not the same kind of thing. A **verdict** is a statement about the world:
-- the goal cannot be reached from here, and the message says why -- no route
-- to the item, nowhere to stand while mining it, a research whose lab would
-- have no power. A **fault** is a defect: a method that contradicted itself,
-- a technology name that refers to nothing, a roster naming a bot the world
-- has never heard of.
--
-- Pass the error a `pcall` caught. A verdict answers with
-- `{ code = "planner::research_needs_power", message = "automation needs a
-- lab with ..." }` -- the planner's own diagnostic code, so a caller never
-- has to match on message text -- and everything else answers `nil`:
-- a fault, an error from any other call, a plain string. `nil` is the safe
-- answer, because the one thing that must never happen by accident is a real
-- bug being read as "only a verdict, carry on".
--
-- A loop spanning several goals uses this to record the milestone it could
-- not plan and finish normally, instead of dying and taking its own record
-- with it; see `scripts/supervisor.lua`.
-- @param err the error value a `pcall` around `goal.plan` caught
-- @treturn table|nil `{ code = ..., message = ... }` for a verdict, `nil` for
--   anything else
function goal.refusal(err)
end
"#,
        ),
    )?;
    install_goal_refusal(lua, &map_table)?;

    // `goal.start` / `goal.run`
    map_table.set(
        "__doc_entry_start",
        String::from(
            r#"
--- starts executing a plan value and returns immediately
-- Consumes the plan: a `PlanValue` may only be taken for a run once, and a
-- second `goal.start`/`goal.run` on the same plan raises rather than
-- dispatching its actions again. The bots keep working while the script does
-- something else; poll with `run:progress()` or block with `run:wait()`.
--
-- "Returns immediately" is about this call, not about the script as a whole:
-- a run that is still going when the script ends is not abandoned. The
-- script's *own* end blocks until every run it started has finished, so a
-- script that never waits still has its bots run to completion -- it just
-- finds out how they went one call later than a script that waited would.
--
-- Either observation a `RunValue` gives back -- `run:progress()` mid-run,
-- `run:wait()` at the end -- carries `:recover()`, but only a finished one
-- will answer it; see `goal.run` for what it proposes.
-- @tparam PlanValue plan a plan returned by `goal.plan`
-- @treturn RunValue the run, immediately -- before it has finished
-- @raise if the plan was already taken for a run, or no game is connected
function goal.start(plan)
end
"#,
        ),
    )?;
    map_table.set(
        "__doc_entry_run",
        String::from(
            r#"
--- starts executing a plan value and blocks until it finishes
-- Exactly `goal.start(plan):wait()`.
-- @tparam PlanValue plan a plan returned by `goal.plan`
-- @treturn table an observation: `{ done, pending, running, success, failed,
--   lost, walks_failed, walks_lost, first_error, actions, walks, failures,
--   recover }` -- see `RunValue`'s own `:wait()` for the shape.
--
--   `walks_failed` counts walks the game refused and `walks_lost` counts walks
--   that never answered. A walk has no action id, so neither is in any of the
--   counts above; when one does not succeed the rest of that bot's slice is
--   abandoned, so a run whose walking went wrong reports `failed = 0` with
--   everything `pending`. `first_error` falls back to the first such walk's
--   error when no action failed, so such a run is never silent.
--
--   `lost` counts what was dispatched and never accounted for: the game
--   answered with no readable outcome, or the run ended still waiting. Those
--   actions carry `status == "lost"`, and they are deliberately neither
--   `running` (nothing is in flight) nor `failed` (no verdict was ever
--   given), so a display that draws `running` as a busy bot does not draw one
--   for work nobody is watching. `obs:failures()` does not list them.
--
--   `obs:recover()` answers what to do about a run that did not finish its
--   plan, as `next_plan, why`. `why` is one of `"rescheduled"` (the plan
--   still fits the world, so here it is again minus what already succeeded),
--   `"reexpanded"` (the world no longer affords that approach, so the same
--   goal was planned afresh), `"complete"` (every action succeeded; there is
--   nothing to run) or `"surfaced"` (nothing mechanical is left to try). The
--   first two come with a plan to hand straight back to `goal.run`; the last
--   two come with `nil`, which is what a retry loop stops on:
--
--       local obs, tries = goal.run(plan), 0
--       while obs.failed > 0 and tries < 3 do
--         local next_plan = obs:recover()
--         if next_plan == nil then break end
--         obs = goal.run(next_plan)
--         tries = tries + 1
--       end
--
--   Nothing is retried unless a script asks, and the budget above is the
--   script's on purpose: a re-expansion is proposed from the world alone, so
--   a world that has not moved gets the same proposal again, and a loop with
--   no bound of its own can spend a very long time on a goal that cannot be
--   reached. The proposal is a plan like any other -- inspect it
--   (`plan.steps`, `plan:count{...}`) or drop it.
--
--   A recovered plan already carries the history it must be run against, so
--   there is no log to pass and none to get wrong: `obs.actions[id].attempts`
--   keeps counting across a `"rescheduled"` retry, and starts again at 1
--   after a `"reexpanded"` one, whose ids number a different plan from zero.
--   Recovering a run that has not finished raises instead of answering: a
--   proposal made mid-flight would re-dispatch whatever the bots are doing
--   right now, so wait (`run:wait()`, or `goal.run`, which waits) first.
--
--   `walks` is an array of `{ bot, step_index, to, status,
--   planned_start, planned_end, dispatched_tick, replied_tick }`, one per walk
--   step the run dispatched, ordered by bot and then step index; a walk has no
--   action id, so `(bot, step_index)` is what names it. As on an action,
--   `dispatched_tick`/`replied_tick` are `game.tick` as the game reported it
--   and are `nil` -- never zero, never the planned value -- when it did not.
-- @raise on the same conditions as `goal.start`, and if the run was refused
--   before it started at all (a schedule implying a circular wait): nothing
--   ran, so there is no observation to report
function goal.run(plan)
end
"#,
        ),
    )?;
    run::install_goal_run(lua, &map_table, actuator)?;

    Ok(map_table)
}

/// Refuses to plan or schedule against a bot `PlanState::from_world` had to
/// fabricate.
///
/// `PlanState::from_world` hands any roster id the world has no player for a
/// `BotState::default()`: an empty inventory and guessed reach distances
/// (`build_distance`/`reach_distance` 10.0, `resource_reach_distance` 3.0).
/// Planning against that is not harmless — it schedules cleanly and then
/// fails at execution against limits that were never real. Naming the bot
/// here, before either the planner or the scheduler ever sees the fabricated
/// state, turns that into an error a script can act on immediately.
///
/// This cannot fire on the ordinary path: `run_lua` always calls
/// `Planner::initiate_missing_players_with_default_inventory` for every id in
/// the run's roster before a `PlanState` is ever built from that world (see
/// `lua_runner.rs`), so every id `goal.plan` passes to `PlanState::from_world`
/// already has a real player and `unknown_bots()` comes back empty. It only
/// fires when a roster names a player the world has never heard of at all,
/// which is exactly the fabrication this closes.
fn refuse_unknown_bots(state: &PlanState) -> LuaResult<()> {
    let unknown = state.unknown_bots();
    if unknown.is_empty() {
        return Ok(());
    }
    let names = unknown
        .iter()
        .map(|bot| bot.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(goal_error(format!(
        "bot(s) {names} are not connected players in this world; refusing to plan \
         against a fabricated inventory and guessed reach distances"
    )))
}

/// Installs `goal.holds` on `table`.
///
/// The one call on this surface that answers a question *about the world*
/// rather than producing a plan, and it exists because nothing else could.
/// `scripts/supervisor.lua` treats an empty plan as satisfaction and says so in
/// a paragraph explaining that it has no way to check: within today's method
/// registry an empty plan really does mean the goal held, but that is an
/// internal invariant of `crates/planner`, not a contract this surface
/// exposed, and asserting satisfaction on it was a guess. It is now checkable —
/// and the planner pins the agreement itself (`an_empty_expansion_and_a_held_
/// goal_agree`), so the Lua side may rely on it rather than assume it.
///
/// No scheduling and no expansion: this is a read of the same world snapshot
/// `goal.plan` reads, through the same roster resolution and the same refusal
/// of bots the world has never heard of.
fn install_goal_holds(
    lua: &Lua,
    table: &LuaTable,
    world: Arc<FactorioSurface>,
    default_roster: Vec<BotId>,
) -> LuaResult<()> {
    table.set(
        "holds",
        lua.create_function(move |_lua, (g, opts): (LuaTable, Option<LuaTable>)| {
            let goal = value::goal_from_lua(&g)?;
            let roster = plan::resolve_roster(opts.as_ref(), &default_roster)?;
            let state = PlanState::from_world(world.clone(), &roster);
            refuse_unknown_bots(&state)?;
            // `Option<bool>` reaches Lua as a boolean or `nil` -- the three
            // answers the planner gives, unflattened. Collapsing the third
            // into `false` here would put the guess back one layer down.
            Ok(holds(&goal, &state))
        })?,
    )?;
    Ok(())
}

/// Expands one goal against a roster.
///
/// `SplitAcrossBots` needs to know who exists before it can split anything, so
/// the roster is not optional. That is not a separate decision from
/// assignment: the split sizes each share against the holdings of the bot it
/// names, so the network it produces only makes sense scheduled on that same
/// roster. `goal.plan` passes one roster to this and to `schedule`, which is
/// what makes the old expand-here-assign-there mismatch unrepresentable.
///
/// The `chain_actor` is the roster's first bot **that can walk somewhere**, not
/// simply its first bot. A goal naming no holder — `Researched`, `BuildCell`,
/// `Producing` — is both sized against the chain actor's inventory and, through
/// the `Holder::Share(chain_actor)` bills the methods state, run by it; pinning
/// that to a walled-in bot is work no other bot may take over and no replan can
/// move. `pick_chain_actor` makes the choice and argues it; the state has to be
/// built before it can be asked, which is the only reason the two lines below
/// swapped order.
///
/// # Test-only since 2026-09-05
///
/// `plan_rounds` used to call this and then `schedule`; it now calls
/// [`factorio_bot_planner::plan_best`], which builds the plan under each
/// drain policy and keeps the shorter schedule -- a choice that cannot be
/// made from a network alone. Every property this function's doc argues is
/// still the production path's: `plan_rounds` builds the same `PlanState`,
/// calls the same `refuse_unknown_bots` and the same `pick_chain_actor`, and
/// hands `plan_best` the actor it returns. This survives as the lower-level
/// probe its own tests use.
// These reach the planner directly and are test-only since `plan_rounds`
// moved to `plan_best`. They sit here, below every `.set("name", ...)`
// registration in this file, and not with the other imports at the top,
// because `doc_guard::production_half` cuts a source file at its FIRST
// column-zero `#[cfg(test)]` -- a test-gated import above the bindings hides
// every binding under it, and the guard then reports `goal.holds` and
// `goal.refusal` as installed at runtime but never registered. Measured, not
// guessed: that is exactly what it said.
#[cfg(test)]
use factorio_bot_planner::{ActionNetwork, Goal, expand, pick_chain_actor, registry_for};

#[cfg(test)]
fn expand_goal(
    goal: Goal,
    world: &Arc<FactorioSurface>,
    bots: &[BotId],
) -> LuaResult<ActionNetwork> {
    let state = PlanState::from_world(world.clone(), bots);
    refuse_unknown_bots(&state)?;
    let chain_actor = pick_chain_actor(&state, bots)
        .ok_or_else(|| goal_error("no bots in this run; goals need at least one"))?;
    // No rewriting of the planner's own errors. There used to be one here,
    // because `registry_for` held no method for `Goal::Researched` and every
    // research goal came back as `NoApplicableMethod` — "no method can satisfy
    // goal: research automation" — which reads as "that technology is
    // unreachable in this world" for what was really an unbuilt feature. The
    // feature is built (`method::have::Researched`), and the planner now
    // distinguishes the cases itself: a technology no force defines comes back
    // as `UnknownTechnology`, naming it.
    expand(
        std::slice::from_ref(&goal),
        &state,
        &registry_for(bots),
        chain_actor,
    )
    .map_err(planner_error)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::tokio::sync::{mpsc, watch};
    use factorio_bot_core::types::Position;
    use factorio_bot_executor::{ActionTicks, ActuatorError, ActuatorFailure};
    use factorio_bot_planner::{Holder, InventorySlot, Schedule, schedule};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    /// When the stub refuses an action.
    ///
    /// `pub(crate)`: `goal::plan`'s and `goal::run`'s own tests reuse this and
    /// `StubActuator` rather than duplicating a second stub actuator.
    pub(crate) enum Failure {
        Never,
        Always,
        /// Only the first action dispatched anywhere. That is what produces an
        /// *abandoned* tail: one bot stops at its first step while the others
        /// finish theirs, so the run ends with actions that were never
        /// dispatched and so never reached the log at all.
        First(AtomicBool),
        /// Every action comes back with no readable verdict — the game
        /// answered and the answer said nothing usable. Not a refusal: it is
        /// what an unreadable `action_completed` looks like from here, and it
        /// is the only way to reach `Status::Lost` from a script.
        WithoutVerdict,
    }

    /// An actuator that never touches a game.
    ///
    /// `walk` succeeds by default and is never gated: a bot whose *walk* fails
    /// has the rest of its slice abandoned before a single action is
    /// dispatched, so everything would stay `Pending` and the counting most of
    /// these tests exist to exercise would never run. `with_failing_walks` is
    /// for the tests that want exactly that shape — it is what a run whose
    /// pathfinder refuses looks like, and it is the shape that reached
    /// `stuck_silent` with no error to show for it.
    pub(crate) struct StubActuator {
        pub(crate) delay: Duration,
        pub(crate) fails: Failure,
        /// Every walk is refused, before any action is dispatched.
        pub(crate) fail_walks: bool,
        /// Signalled as each action is dispatched, so a test can observe a run
        /// mid-flight without sleeping and hoping.
        pub(crate) entered: Option<mpsc::UnboundedSender<()>>,
        /// Actions block here until the test sets it to `true`. A gate that is
        /// never opened is how "did `goal.start` return without waiting?"
        /// becomes a question with a definite answer.
        pub(crate) gate: Option<watch::Receiver<bool>>,
        /// A stand-in game clock, or `None` for an actuator that has none.
        ///
        /// `None` is the default because it is the honest default: a stub is
        /// not a game and has nothing to observe, so it reports
        /// [`ActionTicks::UNKNOWN`] and every test built on it exercises the
        /// absent-tick path all the way out to Lua. `with_clock` is for the
        /// tests that need to prove the ticks a run reports came from the
        /// actuator and not from the schedule.
        pub(crate) clock: Option<Arc<AtomicU64>>,
    }

    /// Where [`StubActuator::with_clock`] starts counting.
    ///
    /// Far beyond any tick these fixtures schedule, so "is this number the
    /// plan's or the actuator's?" has an answer that does not depend on
    /// knowing the schedule.
    pub(crate) const STUB_CLOCK_BASE: u64 = 500_000;

    impl StubActuator {
        pub(crate) fn new(fails: Failure) -> Self {
            StubActuator {
                delay: Duration::ZERO,
                fails,
                fail_walks: false,
                entered: None,
                gate: None,
                clock: None,
            }
        }

        /// Every walk comes back not-succeeding, so the run dispatches nothing
        /// at all. Two runs hit this: `run-1788341905-92036` with a refusal (33
        /// steps planned, none dispatched, `failed = 0` for every action, and
        /// the pathfinder's refusal reaching no record), and
        /// `run-1788344167-58471` with the teleport spin, where each walk ran
        /// until the executor's deadline and came back `Lost` instead. Which of
        /// the two this produces follows `fails`.
        pub(crate) fn with_failing_walks(mut self) -> Self {
            self.fail_walks = true;
            self
        }

        /// Gives the stub a monotonic clock, so its dispatches report ticks
        /// that advance the way a real game's would.
        pub(crate) fn with_clock(mut self) -> Self {
            self.clock = Some(Arc::new(AtomicU64::new(STUB_CLOCK_BASE)));
            self
        }

        /// Two ticks per dispatch, the second strictly after the first, and
        /// every dispatch after the one before it — the shape a real run
        /// produces. `None` when the stub has no clock.
        fn tick(&self) -> ActionTicks {
            match &self.clock {
                None => ActionTicks::UNKNOWN,
                Some(clock) => {
                    let dispatched = clock.fetch_add(7, Ordering::SeqCst);
                    let replied = clock.fetch_add(3, Ordering::SeqCst);
                    ActionTicks::new(Some(dispatched), Some(replied))
                }
            }
        }

        async fn act(&self) -> Result<ActionTicks, ActuatorFailure> {
            if let Some(entered) = &self.entered {
                let _ = entered.send(());
            }
            if let Some(gate) = &self.gate {
                let mut gate = gate.clone();
                while !*gate.borrow_and_update() {
                    if gate.changed().await.is_err() {
                        break;
                    }
                }
            }
            if !self.delay.is_zero() {
                factorio_bot_core::tokio::time::sleep(self.delay).await;
            }
            if let Failure::WithoutVerdict = &self.fails {
                return Err(ActuatorError::NoVerdict(
                    "stub answers with a status nothing can read".to_string(),
                )
                .into());
            }
            let refuse = match &self.fails {
                Failure::Never | Failure::WithoutVerdict => false,
                Failure::Always => true,
                Failure::First(spent) => !spent.swap(true, Ordering::SeqCst),
            };
            if refuse {
                return Err(ActuatorError::Rejected("stub refuses".to_string()).into());
            }
            Ok(self.tick())
        }
    }

    #[async_trait]
    impl Actuator for StubActuator {
        async fn walk(
            &self,
            _bot: BotId,
            _to: Position,
            _min_radius: f64,
            _radius: f64,
        ) -> Result<ActionTicks, ActuatorFailure> {
            if self.fail_walks {
                // Which *kind* of not-succeeding follows `fails`, so one flag
                // covers both walk outcomes a run actually produces: a refusal
                // the game gave a verdict on, and a walk that ran until the
                // executor stopped waiting. The second is what a teleport spin
                // looks like from here, and it is `Lost`, not `Failed`.
                if let Failure::WithoutVerdict = &self.fails {
                    return Err(ActuatorError::NoVerdict(
                        "no action result received in time".to_string(),
                    )
                    .into());
                }
                // The live wording, so a test asserting the error text is
                // asserting something a run could actually produce.
                return Err(ActuatorError::Rejected(
                    "the game's pathfinder returned no path".to_string(),
                )
                .into());
            }
            Ok(self.tick())
        }
        async fn mine(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn craft(
            &self,
            _bot: BotId,
            _recipe: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn place(
            &self,
            _bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
            _underground_half: Option<factorio_bot_core::blueprint::UndergroundHalf>,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn insert(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn remove(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn research(&self, _tech: &str, _: u32) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
        async fn set_recipe(
            &self,
            _bot: BotId,
            _entity: &str,
            _at: Position,
            _recipe: &str,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.act().await
        }
    }

    /// Wraps a stub as the factory `create_lua_goal_with` takes.
    pub(crate) fn factory(stub: Arc<dyn Actuator>) -> ActuatorFactory {
        Arc::new(move || {
            let stub = stub.clone();
            Box::pin(async move { Ok(stub) })
        })
    }

    /// Two bots mining iron ore: one action each, so no bot has a tail to
    /// abandon. Small on purpose, for the tests that only need *an* action.
    ///
    /// `pub(crate)` for `goal::run`'s tests, which drive `run::spawn` against
    /// it directly — the counting they assert on is below the Lua seam, and
    /// building the same network through `goal.plan` would put the planner's
    /// choices between the test and the thing it is testing.
    pub(crate) fn mining_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
        let bots = [BotId(1), BotId(2)];
        let state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-ore".into(),
                count: 20,
                whose: Holder::Anyone,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("the fixture world can be mined");
        assert!(
            net.len() > 1,
            "the tests below want more than one action, got {}",
            net.len()
        );
        let scheduled = schedule(&net, &state, &bots).expect("schedulable");
        (Arc::new(net), Arc::new(scheduled))
    }

    /// Ten red science across four bots: the plan from
    /// `planner/tests/red_science.rs`, and long enough per bot that a bot which
    /// fails its first action leaves a genuine abandoned tail behind it.
    ///
    /// `mining_plan` cannot do this. One action per bot means the failure *is*
    /// the whole slice, nothing is left to abandon, and every action still ends
    /// up with a status — which is exactly why `done` derived from the counts
    /// survived against it.
    pub(crate) fn science_plan() -> (Arc<ActionNetwork>, Arc<Schedule>) {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        for bot in bots {
            state.gain(bot, "stone-furnace", 2);
        }
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 10,
                whose: Holder::Anyone,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("ten red science expands");
        assert!(
            net.len() > 10,
            "this plan must be long enough for a bot to have a tail to abandon, got {}",
            net.len()
        );
        let scheduled = schedule(&net, &state, &bots).expect("schedulable");
        (Arc::new(net), Arc::new(scheduled))
    }

    // ------------------------------------------------------------- the bindings

    /// A world whose players are seeded exactly the way a run seeds them, for
    /// an arbitrary roster of player ids.
    ///
    /// `Planner::initiate_missing_players_with_default_inventory` (used by
    /// `seeded_world` below) only ever seeds `1..=bot_count`, so it cannot
    /// stand in for a roster that deliberately names ids outside that range
    /// (`every_bot_the_bindings_dispatch_to_is_a_player_the_actuator_can_drive`'s
    /// `[3, 4]` case exists precisely to do that). This gives each id in
    /// `roster` the same inventory production seeding gives, directly through
    /// the same `FactorioSurface` event the real seeding uses, so
    /// `PlanState::unknown_bots()` comes back empty for it — the property
    /// `goal.plan` now requires of every bot it is asked to plan for. A bare
    /// `fixture_world()` never seeds any player at all, so every bot named
    /// against it is "unknown" by that same definition; tests that exercise
    /// the bindings above the refusal need this instead.
    pub(crate) fn seeded_world_for(roster: &[u8]) -> Arc<FactorioSurface> {
        let world = fixture_world();
        seed_players(&world, roster);
        Arc::new(world)
    }

    /// Gives each id in `roster` the same inventory production seeding
    /// gives, directly on an already-built `world` -- the piece
    /// `seeded_world_for` cannot offer on its own when a test also needs to
    /// set up something else on the same world first (e.g. a research
    /// force), since `fixture_world()` cannot be seeded twice into two
    /// different `FactorioSurface` values and then merged.
    fn seed_players(world: &FactorioSurface, roster: &[u8]) {
        use factorio_bot_core::types::{EntityName, PlayerChangedMainInventoryEvent};

        for &player_id in roster {
            let mut main_inventory: BTreeMap<String, u32> = BTreeMap::new();
            main_inventory.insert(EntityName::Wood.to_string(), 1);
            main_inventory.insert(EntityName::StoneFurnace.to_string(), 1);
            main_inventory.insert(EntityName::BurnerMiningDrill.to_string(), 1);
            world
                .player_changed_main_inventory(PlayerChangedMainInventoryEvent::from_btreemap(
                    player_id,
                    main_inventory,
                ))
                .expect("seed player");
        }
    }

    /// A [`PlanOrigin`] for a plan that no goal produced.
    ///
    /// The tests that need one build their network and schedule by hand, so
    /// there is no goal they came from and no recovery they could ask for: an
    /// origin is on a `PlanValue` for the benefit of `obs:recover()`, which
    /// none of them reach. The goal below is a stand-in and says so; the
    /// roster is the part that has to be right, since `plan.bots` reports it.
    pub(crate) fn test_origin(roster: &[BotId]) -> Arc<plan::PlanOrigin> {
        let ids: Vec<u8> = roster.iter().map(|bot| bot.0).collect();
        Arc::new(plan::PlanOrigin {
            goal: Goal::Have {
                item: "iron-ore".into(),
                count: 1,
                whose: Holder::Anyone,
            },
            world: seeded_world_for(&ids),
            roster: roster.to_vec(),
        })
    }

    /// Installs the real `goal` table, backed by `stub`, into a sandboxed
    /// interpreter — the same one user scripts get.
    ///
    /// Installs a `PendingWork`, as `run_lua` always does in production,
    /// so that `goal.start` is free to run: most of the tests below are
    /// about what happens *after* a run starts, not about the app-data
    /// check itself. The one test for that check
    /// (`goal::run`'s `start_without_pending_work_installed_fails_loudly...`)
    /// builds its own `Lua` without this helper so it can leave `PendingWork`
    /// out on purpose.
    ///
    /// Seeded via `seeded_world_for`, not a bare `fixture_world()`: these
    /// tests are about plan/run mechanics, not about the unknown-bot refusal,
    /// so bots 1 and 2 need to be real players or `goal.plan` refuses before
    /// any of that mechanics is ever reached.
    pub(crate) fn lua_with_goal(stub: Arc<dyn Actuator>) -> Lua {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1, 2]),
            factory(stub),
            vec![1, 2],
            None,
            None,
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");
        lua
    }

    /// How long a script gets before it is called a hang.
    pub(crate) const EXEC_BOUND: Duration = Duration::from_secs(10);

    /// A wall-clock bound that survives a hang which never yields.
    ///
    /// This exists because `tokio::time::timeout` **cannot** bound the hang
    /// these tests are most likely to hit. Every test here is a
    /// `#[tokio::test]`, i.e. a *current-thread* runtime, and a timer on a
    /// current-thread runtime is only polled when the wrapped future yields.
    /// `goal.plan` and `goal.have` are synchronous `create_function`s whose
    /// bodies (`expand` + `schedule`) are pure CPU work with no await point,
    /// so a hang inside one never returns control to the runtime and the
    /// timer never runs. That is not a theory: a probe of
    /// `while true do i = i + 1 end` under the old guard sat at 99.7% CPU with
    /// the test thread in state `R` and the 10s timeout never fired —
    /// the same signature as the run that wedged for 8h52m at ~665% CPU.
    ///
    /// So the bound is enforced from a *separate OS thread*, which the
    /// spinning one cannot starve, and which reports by name.
    ///
    /// **Why it exits the process rather than failing one test.** The obvious
    /// nicer design — run the script on a thread the test abandons on timeout
    /// — is not available: `mlua::Lua` is `!Send` without mlua's `send`
    /// feature (`Lua` holds an `XRc<ReentrantMutex<RawLua>>`, and
    /// `unsafe impl Send for RawLua` is `#[cfg(feature = "send")]`), so
    /// neither the `Lua` nor a `&Lua` can cross a thread boundary at all.
    /// Enabling `send` would force `Send` bounds on every production
    /// `create_function` closure and app-data value, which is a change to
    /// shipped code in service of a test helper. Since the wedged thread can
    /// be neither unwound nor abandoned, the watchdog ends the process
    /// instead. Under `cargo nextest` — the runner this workspace's baseline
    /// uses — each test is its own process, so that *is* a single named test
    /// failure with no collateral. Under plain `cargo test` it aborts the run
    /// with the message below, which is still a bounded, named, loud failure
    /// rather than a suite that hangs until a human notices.
    pub(crate) struct Watchdog {
        /// `true` once the guarded work finished. Paired with a `Condvar` so
        /// disarming wakes the watchdog immediately instead of leaving one
        /// sleeping thread per guarded call.
        finished: Arc<(Mutex<bool>, std::sync::Condvar)>,
    }

    impl Watchdog {
        /// Arms a bound of `limit` over whatever runs before this is dropped.
        fn arm(what: &str, limit: Duration) -> Self {
            let finished = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            let watched = Arc::clone(&finished);
            let what = what.trim().to_owned();
            let thread = std::thread::current();
            let test = thread.name().unwrap_or("<unnamed>").to_owned();
            std::thread::Builder::new()
                .name("exec-bounded-watchdog".to_owned())
                .spawn(move || {
                    let (done, wake) = &*watched;
                    let (done, timeout) = wake
                        .wait_timeout_while(lock(done), limit, |done| !*done)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if *done || !timeout.timed_out() {
                        return;
                    }
                    drop(done);
                    // Not a `panic!`: this thread is not the failing one, and
                    // a panic here would only unwind the watchdog. And not
                    // `eprintln!` either -- that routes through libtest's
                    // per-thread output capture, which `std::thread::spawn`
                    // inherits, and the captured buffer is never printed
                    // because the process ends before libtest reports. Writing
                    // to the `Stderr` handle bypasses the capture, so the
                    // message survives with or without `--nocapture`.
                    use std::io::Write;
                    let message = format!(
                        "\nexec_bounded: the script did not finish within {limit:?}.\n  \
                         test:   {test}\n  \
                         script: {what}\n  \
                         This is a hang that never yields, so tokio::time::timeout could not \
                         fire on it and the test thread can be neither unwound nor abandoned \
                         (mlua::Lua is !Send). Exiting the test process with 101 so this reads \
                         as a named failure rather than a wedged suite.\n"
                    );
                    let mut err = std::io::stderr();
                    let _ = err.write_all(message.as_bytes());
                    let _ = err.flush();
                    std::process::exit(101);
                })
                .expect("spawn the exec_bounded watchdog");
            Self { finished }
        }
    }

    impl Drop for Watchdog {
        fn drop(&mut self) {
            let (done, wake) = &*self.finished;
            *lock(done) = true;
            wake.notify_all();
        }
    }

    /// Runs `work` under both bounds, returning `None` if it timed out.
    ///
    /// Two bounds, because they catch different failures. The `tokio` timeout
    /// is the one that can report an *awaiting* hang as an ordinary panic,
    /// leaving the rest of the suite to run, so it is given the shorter
    /// deadline and wins whenever it can fire at all. The watchdog is the
    /// backstop for the case it structurally cannot cover.
    pub(crate) async fn bounded<F: std::future::Future>(
        limit: Duration,
        what: &str,
        work: F,
    ) -> Option<F::Output> {
        let _watchdog = Watchdog::arm(what, limit * 2);
        factorio_bot_core::tokio::time::timeout(limit, work)
            .await
            .ok()
    }

    /// Runs `code`, failing rather than hanging if it does not finish.
    ///
    /// The bound is the point. A `goal.start` that waited for its run would
    /// block here forever behind the shut gate, and a test that hangs on
    /// regression is not a guard — it reads as a slow suite. This turns it into
    /// a named assertion failure. See [`Watchdog`] for the half of that which
    /// a timeout alone cannot do.
    pub(crate) async fn exec_bounded(lua: &Lua, code: &str) {
        exec_bounded_within(EXEC_BOUND, lua, code).await;
    }

    /// Runs `code`, failing rather than hanging if it does not finish, and
    /// returning the error `code` raised.
    ///
    /// [`exec_bounded`] is the success-path counterpart; this is its mirror for
    /// the tests that assert a script must fail -- `exec_bounded` itself
    /// `.expect`s success, so it cannot be used for them. It shares
    /// [`bounded`], so it shares the watchdog: the bound here also covers a
    /// hang that never yields. See [`Watchdog`].
    pub(crate) async fn exec_bounded_err(lua: &Lua, code: &str) -> String {
        match bounded(EXEC_BOUND, code, lua.load(code).exec_async()).await {
            None => panic!("the script did not finish within {EXEC_BOUND:?}"),
            Some(Ok(())) => panic!("the script was expected to fail but succeeded"),
            Some(Err(err)) => err.to_string(),
        }
    }

    /// [`exec_bounded`] with the deadline named, for the tests *about* the
    /// bound — they must not wait [`EXEC_BOUND`] to observe it.
    pub(crate) async fn exec_bounded_within(limit: Duration, lua: &Lua, code: &str) {
        match bounded(limit, code, lua.load(code).exec_async()).await {
            None => panic!("the script did not finish within {limit:?}: goal.start blocked"),
            Some(result) => result.expect("the script failed"),
        }
    }

    // -------------------------------------------------- the bound's own proof

    /// The bound a script gets in [`the_bound_fires_on_a_script_that_never_yields`].
    ///
    /// Short on purpose: the guard's real deadline is [`EXEC_BOUND`], and a
    /// test of the guard must not cost that. The watchdog fires at twice this.
    const PROBE_BOUND: Duration = Duration::from_millis(400);

    /// The probe's full path in this test binary, as libtest's `--exact`
    /// wants it. A rename that does not update this makes the parent test
    /// fail (no test matched), not silently pass.
    const PROBE: &str = "globals::goal::tests::the_watchdog_probe_that_never_yields";

    /// Wedges on purpose, and is meant to.
    ///
    /// `#[ignore]`, so no ordinary run selects it;
    /// [`the_bound_fires_on_a_script_that_never_yields`] runs it as a child
    /// process and asserts on how it died. The loop is Lua rather than a
    /// `sleep`: an awaiting hang is the case the old guard already handled,
    /// and testing only that is precisely why this defect survived. This one
    /// never returns to the runtime at all, so nothing but the watchdog can
    /// end it.
    #[tokio::test]
    #[ignore = "wedges on purpose; driven as a child process by the_bound_fires_on_a_script_that_never_yields"]
    async fn the_watchdog_probe_that_never_yields() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded_within(PROBE_BOUND, &lua, "local i = 0 while true do i = i + 1 end").await;
        unreachable!("the watchdog must have ended this process");
    }

    /// The guard bounds a hang that never yields — proven, not asserted.
    ///
    /// Necessarily a child process: the whole point is that the guard cannot
    /// hand control back to a wedged test, so there is no in-process
    /// observation to make. What is checked is the contract callers rely on —
    /// it ends, it ends *within the bound*, it ends unsuccessfully, and it
    /// says which script and which test.
    ///
    /// This test bounds its own wait externally (`try_wait` against a
    /// deadline, then `kill`) rather than through the mechanism under test,
    /// so a regression in the watchdog shows up here as a failure and never
    /// as a second hang.
    #[test]
    fn the_bound_fires_on_a_script_that_never_yields() {
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::time::Instant;

        let exe = std::env::current_exe().expect("this test binary's own path");
        let mut child = Command::new(exe)
            // No `--test-threads=1`: at concurrency 1 libtest runs the test on
            // `main` rather than on a thread named after it, and the name is
            // what puts the test in the watchdog's message.
            .args([PROBE, "--exact", "--ignored", "--nocapture"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the probe");

        // Generous next to the watchdog's 800ms, tight next to "forever":
        // this is the assertion that the bound exists at all.
        let deadline = Instant::now() + Duration::from_secs(20);
        let started = Instant::now();
        let status = loop {
            match child.try_wait().expect("poll the probe") {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "the probe was still spinning after 20s: the bound did not fire. \
                         This is the pre-fix behaviour -- tokio::time::timeout cannot \
                         preempt a future that never yields."
                    );
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        let elapsed = started.elapsed();

        let mut stderr = String::new();
        child
            .stderr
            .take()
            .expect("piped stderr")
            .read_to_string(&mut stderr)
            .expect("read the probe's stderr");

        assert!(
            !status.success(),
            "a wedged script must fail the probe, not pass it: {status}\n{stderr}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "the bound must fire near {:?}, not merely eventually: took {elapsed:?}",
            PROBE_BOUND * 2
        );
        for expected in [
            "exec_bounded: the script did not finish within",
            "while true do i = i + 1 end",
            PROBE,
        ] {
            assert!(
                stderr.contains(expected),
                "the failure must name {expected:?} so the next occurrence is diagnosable, \
                 not just loud. Got:\n{stderr}"
            );
        }
    }

    // ------------------------------------------------------------- the surface

    /// The whole surface, as a **set**.
    ///
    /// Deliberately not a loop over an expected-name list: that shape is a
    /// mirror of the code — it stays green when a seventh function appears,
    /// which is exactly the drift it is supposed to catch. Comparing the set
    /// of callables both ways fails on a missing function *and* on an extra
    /// one.
    #[test]
    fn the_goal_table_offers_exactly_the_new_surface() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        lua.load(
            r#"
            local expected = { have=true, researched=true, producing=true,
                               sustain=true,
                               built=true, charted=true, all=true, plan=true, run=true,
                               start=true, holds=true, refusal=true }
            local actual = {}
            for k, v in pairs(goal) do
                -- the __doc__ keys are strings consumed by the doc generator
                if type(v) == "function" then actual[k] = true end
            end
            for name in pairs(expected) do
                assert(actual[name], "missing from the goal table: " .. name)
            end
            for name in pairs(actual) do
                assert(expected[name], "unexpected function on the goal table: " .. name)
            end
            -- Named explicitly as well, so the failure message says *which* old
            -- name survived rather than only that the set differs.
            for _, gone in ipairs{"schedule","graphviz","gantt","execute","progress","wait"} do
                assert(goal[gone] == nil, gone .. " must be gone, not merely deprecated")
            end
        "#,
        )
        .exec()
        .expect("script");
    }

    /// `goal.holds` answers about the world, and answers per holder.
    ///
    /// `lua_with_goal`'s roster is bots 1 and 2, each seeded the way a run
    /// seeds them: one furnace, one drill, one wood. So two furnaces exist
    /// between them and one exists on each -- the case that makes "whose"
    /// load-bearing rather than decorative, and the same shape as the live
    /// milestone that closed satisfied on a starting inventory nobody had
    /// smelted.
    #[test]
    fn holds_answers_satisfaction_without_planning() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        lua.load(
            r#"
            assert(goal.holds(goal.have("stone-furnace", 2)) == true,
                   "two furnaces exist across the roster")
            assert(goal.holds(goal.have("stone-furnace", 3)) == false,
                   "a third does not")
            assert(goal.holds(goal.have("stone-furnace", 2, { bot = 1 })) == false,
                   "and bot 1 holds only one of them, which is the whole point")
            assert(goal.holds(goal.have("stone-furnace", 1, { bot = 1 })) == true)
            assert(goal.holds(goal.all { goal.have("wood", 2),
                                         goal.have("stone-furnace", 3) }) == false,
                   "a bundle holds only when every member does")
            assert(goal.holds(goal.have("stone-furnace", 2), { bots = { 1 } }) == false,
                   "and the roster asked about is the roster answered for")
        "#,
        )
        .exec()
        .expect("script");
    }

    /// The agreement `supervisor.lua` used to assume: where the plan is empty,
    /// `goal.holds` says the goal is met, and where it is not, it does not.
    /// Pinned through the bindings as well as inside the planner, because it
    /// is the Lua side that acts on it.
    #[test]
    fn an_empty_plan_and_a_held_goal_agree_through_the_bindings() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        lua.load(
            r#"
            for _, n in ipairs{1, 2, 3, 8} do
                local g = goal.have("stone-furnace", n)
                local empty = #goal.plan(g).steps == 0
                assert(empty == (goal.holds(g) == true),
                       "plan emptiness and satisfaction disagree at " .. n)
            end
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn holds_refuses_a_roster_the_world_does_not_have() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        let err = lua
            .load(r#"goal.holds(goal.have("wood", 1), { bots = { 9 } })"#)
            .exec()
            .expect_err("bot 9 is not a connected player");
        assert!(
            err.to_string().contains("not connected players"),
            "the refusal must name the cause, got {err}"
        );
    }

    /// The whole surface, composed, through the table a script really gets.
    ///
    /// Every test the value-based surface grew before this one built its own
    /// table and installed the constructors onto it by hand, because the
    /// production table still carried the handle-based `have`/`researched`.
    /// So each part was proven against a parallel wiring and the composition
    /// against none. This is the one test that holds `create_lua_goal_with`'s
    /// own table to the whole chain: goal value -> plan -> run -> the refusal
    /// of a spent plan.
    #[tokio::test]
    async fn the_production_goal_table_composes_end_to_end() {
        let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
        exec_bounded(
            &lua,
            r#"
            local g    = goal.all { goal.have("iron-ore", 2), goal.have("coal", 1) }
            local p    = goal.plan(g, { bots = { 1 } })
            local obs  = goal.run(p)
            assert(#p.steps > 0, "the composed surface produced a plan")
            assert(p:count { kind = "mine" } > 0, "and it contains real work")
            assert(obs.done and obs.failed == 0, "and the run completed cleanly")
            -- `tostring` because a Rust-raised error arrives in Lua as an
            -- error *object* (mlua userdata carrying the `LuaError`), not as
            -- a string: `:find` on it raises "attempt to index a userdata
            -- value" and would report a spent-plan refusal as a surface
            -- defect. Every other pcall assertion in this crate spells it the
            -- same way.
            local ok, err = pcall(goal.run, p)
            assert(not ok, "a spent plan must refuse a second run")
            assert(tostring(err):find("already"), "and say so: " .. tostring(err))
        "#,
        )
        .await;
    }

    // --------------------------------------------- the scheduler/executor seam

    /// Records the bot named by every command it is asked to perform.
    #[derive(Default)]
    struct RecordingActuator {
        bots: std::sync::Mutex<Vec<BotId>>,
    }

    impl RecordingActuator {
        /// No game clock: this actuator records who was dispatched to, not
        /// when, so it reports [`ActionTicks::UNKNOWN`].
        fn note(&self, bot: BotId) -> Result<ActionTicks, ActuatorFailure> {
            #[allow(clippy::unwrap_used)]
            self.bots.lock().unwrap().push(bot);
            Ok(ActionTicks::UNKNOWN)
        }
        fn recorded(&self) -> Vec<BotId> {
            #[allow(clippy::unwrap_used)]
            self.bots.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Actuator for RecordingActuator {
        async fn walk(
            &self,
            bot: BotId,
            _to: Position,
            _min_radius: f64,
            _radius: f64,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn mine(
            &self,
            bot: BotId,
            _item: &str,
            _at: Position,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn craft(
            &self,
            bot: BotId,
            _recipe: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn place(
            &self,
            bot: BotId,
            _item: &str,
            _at: Position,
            _direction: u8,
            _underground_half: Option<factorio_bot_core::blueprint::UndergroundHalf>,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn insert(
            &self,
            bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn remove(
            &self,
            bot: BotId,
            _entity: &str,
            _at: Position,
            _slot: InventorySlot,
            _item: &str,
            _count: u32,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
        async fn research(&self, _tech: &str, _: u32) -> Result<ActionTicks, ActuatorFailure> {
            Ok(ActionTicks::UNKNOWN)
        }
        async fn set_recipe(
            &self,
            bot: BotId,
            _entity: &str,
            _at: Position,
            _recipe: &str,
        ) -> Result<ActionTicks, ActuatorFailure> {
            self.note(bot)
        }
    }

    /// The seam between the half of the stack that *assigns* work and the half
    /// that *performs* it.
    ///
    /// When this was broken both halves were internally consistent: the
    /// schedule numbered bots `1..=n` and the actuator numbered them `0..n-1`,
    /// and each had a passing test against its own convention. So this test
    /// states neither. It gives the bindings a roster of player ids — the same
    /// thing `Planner::initiate_missing_players_with_default_inventory` returns
    /// to `run_lua` — drives the real `goal.*` bindings all the way through
    /// `goal.run`, and then asks the *executor's own* resolution function
    /// whether each bot it was handed names a player that is in the game.
    ///
    /// The `[3, 4]` roster is the discriminating one, and it fails against
    /// either side's old convention: a schedule built from `1..=bot_count`
    /// dispatches to players 1 and 2, who are not in the game, and an actuator
    /// that renumbers the connected players `0..n-1` has no bot 3 or 4.
    #[tokio::test]
    async fn every_bot_the_bindings_dispatch_to_is_a_player_the_actuator_can_drive() {
        use factorio_bot_core::types::PlayerId;
        use std::collections::BTreeSet;

        for roster in [vec![1u8], vec![1, 2], vec![3, 4], vec![1, 2, 3, 4]] {
            let rec = Arc::new(RecordingActuator::default());
            let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
            lua.set_app_data(crate::lua_runner::PendingWork::default());
            // Seeded via `seeded_world_for`, not a bare `fixture_world()`:
            // `[3, 4]` is exactly the roster `seeded_world` (which only ever
            // seeds `1..=bot_count`) cannot produce, and this test's whole
            // point is that roster, so every id in it needs to be a real
            // player or `goal.plan` refuses before the seam below is ever
            // exercised.
            let table = create_lua_goal_with(
                &lua,
                seeded_world_for(&roster),
                factory(rec.clone()),
                roster.clone(),
                None,
                None,
                None,
            )
            .expect("goal table");
            lua.globals().set("goal", table).expect("install");

            exec_bounded(
                &lua,
                r#"
                goal.run(goal.plan(goal.have("iron-ore", 20)))
                "#,
            )
            .await;

            let dispatched = rec.recorded();
            assert!(
                !dispatched.is_empty(),
                "roster {roster:?}: nothing was dispatched, so the loop below would \
                 hold for any numbering at all"
            );
            // The game has exactly the run's players in it — that is what the
            // roster means. `RconActuator::new` builds this same set from
            // `connected_players()`.
            let connected: BTreeSet<PlayerId> = roster.iter().copied().collect();
            for bot in &dispatched {
                let player = RconActuator::resolve_player(&connected, *bot)
                    .unwrap_or_else(|err| panic!("roster {roster:?}: {err}"));
                assert!(
                    roster.contains(&player),
                    "roster {roster:?}: {bot} drove player {player}, who is not in this run"
                );
            }
        }
    }

    /// A world whose players are seeded exactly the way a run seeds them.
    ///
    /// `Planner::initiate_missing_players_with_default_inventory` gives each
    /// bot **one** stone furnace, one burner mining drill and one piece of
    /// wood, and `lua_runner` calls it and then `update_plan_world` before the
    /// bindings ever see the world. Handing every bot everything the plan needs
    /// is what let this bug live under 400-odd green tests, so the seeding is
    /// taken from the production call rather than written out here, and the
    /// assertion below states the property the test depends on.
    fn seeded_world(bot_count: u8) -> Arc<FactorioSurface> {
        use factorio_bot_core::plan::planner::Planner;

        let mut planner = Planner::new(Arc::new(fixture_world()), None);
        let roster = planner.initiate_missing_players_with_default_inventory(bot_count);
        planner.update_plan_world();
        let world = planner.world();
        for id in roster {
            let player = world
                .globals
                .players
                .get(&id)
                .expect("the run seeded this player");
            assert_eq!(
                player.main_inventory.get("stone-furnace").copied(),
                Some(1),
                "bot {id} must hold exactly one furnace, or this test cannot reach the bug"
            );
        }
        world
    }

    /// A plan for a run of several bots, made for one of them.
    ///
    /// Under the old handle surface this was the split that could not be
    /// expressed safely: `goal.have` expanded against every bot the run had,
    /// `SplitAcrossBots` gave each of them a chain spending *its own* starting
    /// stone furnace, and `goal.schedule(p, 1)` then asked one bot to run all
    /// of them — asking that bot for as many furnaces as the roster had
    /// between them. Live, with `scripts/goal_smoke.lua`, that was:
    ///
    /// ```text
    /// goal: precondition has 1 stone-furnace of action ActionId(0) does not hold for bot 1
    /// ```
    ///
    /// identically for `--bots 2` and `--bots 4`, while `--bots 1` — the only
    /// roster where expansion and assignment agree — succeeded. `goal.plan`
    /// takes one roster for both, so the mismatch cannot be written any more;
    /// this test is what proves the surviving call really does expand for the
    /// bots it schedules for, against the *production* seeding rather than a
    /// hand-built world.
    ///
    /// Rosters of 2 and 4 are both here because the split's arithmetic differs
    /// between them: a shortfall of five over two bots is 3 + 2, over four bots
    /// 2 + 1 + 1 + 1, and only the second lets a share of one reach the chain
    /// that a share of one is supposed to open.
    #[tokio::test]
    async fn a_plan_made_for_one_bot_of_a_larger_run_is_schedulable() {
        for bot_count in [1u8, 2, 4] {
            let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
            lua.set_app_data(crate::lua_runner::PendingWork::default());
            let table = create_lua_goal_with(
                &lua,
                seeded_world(bot_count),
                factory(Arc::new(StubActuator::new(Failure::Never))),
                (1..=bot_count).collect(),
                None,
                None,
                None,
            )
            .expect("goal table");
            lua.globals().set("goal", table).expect("install");

            // The smoke script's own shape: plan for one bot out of the run,
            // then render what was scheduled.
            exec_bounded(
                &lua,
                r#"
                local p = goal.plan(goal.have("iron-plate", 5), { bots = { 1 } })
                assert(p.makespan > 0, "a real plan takes a positive number of ticks")
                for _, s in ipairs(p.steps) do assert(s.bot == 1, "every step on bot 1") end
                assert(#p:gantt("smoke") > 0, "the gantt must describe what was scheduled")
                "#,
            )
            .await;
        }
    }

    /// One force with one technology, added on top of the shared fixture
    /// world.
    ///
    /// `fixture_world` carries no forces, and technologies live on forces, so
    /// `goal.researched` has nothing to plan against it. The fixture is shared
    /// with the planner's own pinned makespan tests and must not grow a force
    /// of its own, so this adds one here, for this test only. The numbers are
    /// the game's own for `automation`: 10 units of one automation science
    /// pack each, 600 ticks per unit.
    const RESEARCH_FORCE_JSON: &str = r#"
    {
      "name": "player",
      "force_id": 1,
      "current_research": null,
      "research_progress": null,
      "technologies": {
        "automation": {
          "name": "automation",
          "enabled": true,
          "upgrade": false,
          "researched": false,
          "prerequisites": [],
          "research_unit_ingredients": [
            { "name": "automation-science-pack", "ingredient_type": "item", "amount": 1 }
          ],
          "research_unit_count": 10,
          "research_unit_energy": 600.0,
          "order": "a-a",
          "level": 1,
          "valid": true
        }
      }
    }
    "#;

    /// Runs `tests/goal_script.lua` through the real interpreter and the real
    /// `run_lua` harness, which is the only thing that proves the table is
    /// installed as a global under a live sandbox.
    ///
    /// The sandbox root is a temp directory: the script touches no files, and
    /// rooting it in the repository would put it beside the write-only
    /// artifacts `test_script` regenerates.
    #[tokio::test]
    async fn the_goal_script_fixture_runs() {
        use factorio_bot_core::plan::planner::Planner;
        use factorio_bot_core::types::FactorioForce;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let world = fixture_world();
        let force: FactorioForce = factorio_bot_core::serde_json::from_str(RESEARCH_FORCE_JSON)
            .expect("the research force fixture must parse");
        world.update_force(force).expect("update_force");
        let mut planner = Planner::new(Arc::new(world), None);
        // The fixture has no Factorio behind it, so the world has no players
        // and `Planner::roster` would hand the script nobody -- correctly, for
        // a live run. Seed them the way the planning-only mode does
        // (`--clients 0`, in `cli/lua.rs`), which is the production caller this
        // fixture stands in for.
        planner.initiate_missing_players_with_default_inventory(4);
        crate::lua_runner::run_lua(
            &mut planner,
            include_str!("../../../tests/goal_script.lua"),
            None,
            &root,
            4,
            None,
        )
        .await
        .expect("goal_script.lua failed");
    }

    // ---------------------------------------------- the unknown-bot refusal

    /// `goal.plan` refuses the run's *default* roster when it names a bot the
    /// world has no player for, rather than silently planning it with
    /// `BotState::default()`'s empty inventory and guessed reach distances.
    ///
    /// The default roster, specifically: `plan.rs`'s
    /// `an_unknown_bot_raises_and_names_it` covers an explicit `opts.bots`,
    /// and a check that only looked at what the caller passed would leave
    /// this path — the one every ordinary script takes — unguarded.
    ///
    /// Bot 1 is seeded and bot 2 is not, so this also discriminates from a
    /// check that only ever looks at "is the roster empty" or similar: a
    /// roster with one real bot in it must still be refused for the other,
    /// unnamed one, and the error must name it.
    #[tokio::test]
    async fn goal_plan_refuses_a_default_roster_naming_a_bot_the_world_does_not_have() {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1]),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
            None,
            None,
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err = lua
            .load(r#"return goal.plan(goal.have("iron-ore", 20))"#)
            .eval_async::<LuaValue>()
            .await
            .expect_err("bot 2 is not a player in this world");
        let message = err.to_string();
        // Not just "does the call error": a fabricated bot's empty inventory
        // also trips unrelated planner errors (e.g. a mismatched starting
        // inventory across bots) that happen to name "bot 2" too, so the
        // assertion has to be on this refusal's own wording, not merely on
        // the bot id appearing somewhere in the message.
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the unknown bot: {message}"
        );
    }

    /// A `researched` goal goes through the same `expand_goal` path, so the
    /// same refusal must fire for it, against a world that actually has a
    /// technology to research (otherwise the planner's own
    /// `UnknownTechnology` error would fire first and this would prove
    /// nothing about the unknown-bot check specifically).
    #[tokio::test]
    async fn goal_plan_refuses_an_unknown_bot_for_a_research_goal_too() {
        use factorio_bot_core::types::FactorioForce;

        let world = fixture_world();
        let force: FactorioForce = factorio_bot_core::serde_json::from_str(RESEARCH_FORCE_JSON)
            .expect("the research force fixture must parse");
        world.update_force(force).expect("update_force");
        // Bot 1 only; the roster below also names bot 2, which this world
        // never seeds.
        seed_players(&world, &[1]);
        let world = Arc::new(world);

        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            world,
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
            None,
            None,
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        let err = lua
            .load(r#"return goal.plan(goal.researched("automation"))"#)
            .eval_async::<LuaValue>()
            .await
            .expect_err("bot 2 is not a player in this world");
        let message = err.to_string();
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the unknown bot: {message}"
        );
    }

    /// **The top-level chain does not pin to a bot that cannot walk.**
    ///
    /// `expand_goal` used to hand `expand` the roster's first bot outright, so
    /// a walled-in bot 1 took the whole of a `Researched` or `Producing` goal
    /// with it: the methods state those bills as `Holder::Share(chain_actor)`,
    /// which both sizes the bill against that bot and welds the chain to it,
    /// and `schedule` treats a chain owner as a hard constraint with no
    /// fallback tier. `crates/planner`'s `pick_chain_actor` decides it now, and
    /// this is the seam that proves this caller asks.
    ///
    /// The geometry and the argument are
    /// `crates/planner/tests/unreachable_memory.rs`'; here only the wiring is
    /// at stake, so the goal is the cheapest one that states a
    /// `Holder::Share(chain_actor)` bill.
    #[test]
    fn expand_goal_does_not_pin_the_chain_to_a_walled_in_first_bot() {
        use factorio_bot_core::factorio::world::Enclosure;
        use factorio_bot_core::types::{FactorioEntity, FactorioPlayer};
        use factorio_bot_planner::ActionKind;

        let boxed_in = Position::new(-56.2421875, 14.28125);
        let outside = Position::new(-23.1875, -37.90625);
        let world = fixture_world();
        // A closed square ring of trees, spaced half a tile so nothing a
        // character-sized box could slip through is left between two of them --
        // the same construction the planner's own walled-in tests use, and for
        // the reason they argue: the obstacles that actually box a bot in are
        // structurally absent from a fixture and have to be built.
        let mut ring = Vec::new();
        let mut offset = -3.0_f64;
        while offset <= 3.0 {
            for pos in [
                Position::new(boxed_in.x() + offset, boxed_in.y() - 3.),
                Position::new(boxed_in.x() + offset, boxed_in.y() + 3.),
                Position::new(boxed_in.x() - 3., boxed_in.y() + offset),
                Position::new(boxed_in.x() + 3., boxed_in.y() + offset),
            ] {
                ring.push(FactorioEntity::new_tree(&pos));
            }
            offset += 0.5;
        }
        world.update_chunk_entities(ring).expect("the ring loads");
        for (id, position) in [(1u8, boxed_in.clone()), (2, outside)] {
            world.globals.players.insert(
                id,
                FactorioPlayer {
                    player_id: id,
                    position,
                    ..Default::default()
                },
            );
        }
        // The first witness: the game's own pathfinder refused a route from
        // where bot 1 stands. Without it nothing is inferred at all.
        world.record_enclosure(Enclosure {
            tick: None,
            player: 1,
            at: boxed_in,
            pocket_tiles: 24.,
            searched_tiles: 24.,
        });
        let world = Arc::new(world);

        let bots = [BotId(1), BotId(2)];
        let state = PlanState::from_world(world.clone(), &bots);
        assert!(
            state.is_walled_in(BotId(1)) && !state.is_walled_in(BotId(2)),
            "the premise: bot 1 is sealed in and bot 2 is not; got {:?}",
            state.walled_in()
        );

        let net = expand_goal(
            Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 10,
            },
            &world,
            &bots,
        )
        .expect("a burner cell needs no power and the fixture has ore");
        let cell_chain = net
            .actions()
            .find(|a| {
                matches!(&a.kind, ActionKind::Place { entity } if entity.name == "stone-furnace")
            })
            .and_then(|a| net.chain_of(a.id))
            .expect("the furnace is placed inside a chain");
        assert_eq!(
            net.owner_of(cell_chain),
            Some(BotId(2)),
            "bot 1 cannot walk anywhere, so the cell may neither be sized against its \
             inventory nor welded to it"
        );
    }

    /// The refusal reads the world as it is **now**, not as it was when the
    /// table was built.
    ///
    /// A goal value is pure and can be built long before it is planned, so
    /// the check cannot be hoisted to `create_lua_goal_with` or memoised on
    /// the roster: both bots are real players when the goal value is made,
    /// and bot 2 is only removed from the world afterwards -- from the same
    /// `FactorioSurface` the run's `goal` table already closed over, so
    /// `goal.plan`'s `PlanState::from_world` call is the thing that has to
    /// notice.
    #[tokio::test]
    async fn goal_plan_refuses_a_bot_removed_after_the_goal_value_was_built() {
        let world = seeded_world_for(&[1, 2]);
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            world.clone(),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1, 2],
            None,
            None,
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        exec_bounded(&lua, r#"g = goal.have("iron-ore", 20)"#).await;
        // Proof the goal value itself is fine while both bots are real: this
        // same call is what must fail after the removal below.
        exec_bounded(&lua, "goal.plan(g)").await;

        world.remove_player(2).expect("remove_player");

        let err = lua
            .load("return goal.plan(g)")
            .eval_async::<LuaValue>()
            .await
            .expect_err("bot 2 was just removed from the world");
        let message = err.to_string();
        assert!(
            message.contains("bot 2") && message.contains("not connected players"),
            "the error must be this refusal, naming the now-unknown bot: {message}"
        );
    }

    // ------------------------------------------- refusing a second run

    /// The regression this closes: running the same plan twice used to spawn
    /// a second, independent run against the same schedule, dispatching every
    /// action again -- against a live game that means placing an entity
    /// twice, inserting twice, mining an already-mined tile.
    ///
    /// `goal::run`'s `running_one_plan_twice_raises` covers the error text.
    /// This covers the part an error message cannot: `rec` records every
    /// command any run performs, so a refused second `goal.run` that had
    /// nevertheless spawned its run would show up as a higher count after it
    /// than before.
    #[tokio::test]
    async fn a_second_run_on_the_same_plan_dispatches_nothing_again() {
        let rec = Arc::new(RecordingActuator::default());
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(&[1, 2]),
            factory(rec.clone()),
            vec![1, 2],
            None,
            None,
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        exec_bounded(
            &lua,
            r#"
            p = goal.plan(goal.have("iron-ore", 20))
            goal.run(p)
            "#,
        )
        .await;
        let after_first = rec.recorded().len();
        assert!(
            after_first > 0,
            "the first run must have dispatched something"
        );

        exec_bounded(
            &lua,
            r#"
            local ok, err = pcall(goal.run, p)
            result = { ok = ok, err = tostring(err) }
            "#,
        )
        .await;
        let result: LuaTable = lua.globals().get("result").expect("result");
        let ok: bool = result.get("ok").expect("ok");
        let err: String = result.get("err").expect("err");
        assert!(!ok, "a second run on the same plan must be refused");
        assert!(
            err.contains("already"),
            "the error should say the plan already ran: {err}"
        );
        assert_eq!(
            rec.recorded().len(),
            after_first,
            "the refused second run must not have dispatched anything"
        );
    }

    // ------------------------------- a verdict about the world is not a fault

    /// Run 31's own failure, reproduced through the real bindings.
    ///
    /// `workspace/runs/run-1788372605-35170/` satisfied six milestones in
    /// twelve minutes and then asked for `researched("automation")` in a world
    /// whose plan could show no electric supply at all. The planner refused,
    /// correctly, and the refusal took the run down with it -- the summary
    /// printed milestones 1-6 and no `milestone 7` line whatsoever.
    ///
    /// `goal.plan` still *raises* it: a caller who ignores a refusal must not
    /// quietly receive an empty plan instead. What is new is that the raise is
    /// recognisable -- it carries the planner's own diagnostic code, so a loop
    /// spanning milestones can tell "this world cannot do that" from "this
    /// planner is broken" without matching on message text.
    ///
    /// **The refusal this reproduces has moved, and that is the point.** Since
    /// the planner learned to build a power plant, an unpowered world is
    /// answered by building one; what a script can still be told is that the
    /// *water* is out of reach. So the bot is put 80 tiles from the fixture's
    /// lake -- far enough to fail the 64-tile siting bound, near enough that
    /// the refusal can name the distance -- and the code checked is
    /// `power_plant_too_far_from_water`. The classification seam this test
    /// exists for is unchanged.
    #[tokio::test]
    async fn a_research_that_cannot_reach_the_water_raises_a_recognisable_refusal() {
        use factorio_bot_core::types::{FactorioForce, PlayerChangedPositionEvent, Position};

        // **A world with no water at all**, not the shared fixture with the
        // bot walked away from its lake.
        //
        // This test used to stand the bot at (-200, 40), 240 tiles from
        // `fixture_world`'s lake and past every scan. That stopped producing a
        // refusal on 2026-09-06, when `method::power::supply_for` learned to
        // fall back to a **world-anchored** search: the lake is 57 tiles from
        // the origin whatever the bot did, and siting off the roster's walk
        // history was the defect being fixed (a charted dump whose bots parked
        // 355 tiles out refused every power-needing goal while `score-map`
        // reported water at 48 tiles).
        //
        // So the unreachable case is now exactly one thing -- a map with no
        // water on it -- and that is what this builds. The classification seam
        // the test exists for is unchanged.
        let world = factorio_bot_core::test_utils::fixture_world_without_water();
        let force: FactorioForce = factorio_bot_core::serde_json::from_str(RESEARCH_FORCE_JSON)
            .expect("the research force fixture must parse");
        world.update_force(force).expect("update_force");
        seed_players(&world, &[1]);
        world
            .player_changed_position(PlayerChangedPositionEvent {
                player_id: 1,
                position: Position::new(-200., 40.),
            })
            .expect("moving a seeded player cannot fail");

        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            Arc::new(world),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1],
            None,
            None,
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        exec_bounded(
            &lua,
            r#"
            local ok, err = pcall(goal.plan, goal.researched("automation"))
            local refusal = goal.refusal(err)
            result = { ok = ok, text = tostring(err),
                       code = refusal and refusal.code,
                       message = refusal and refusal.message }
            "#,
        )
        .await;

        let result: LuaTable = lua.globals().get("result").expect("result");
        assert!(
            !result.get::<bool>("ok").expect("ok"),
            "a plant that cannot be sited cannot be planned; the call must not succeed"
        );
        let text: String = result.get("text").expect("text");
        assert!(
            text.contains("needs water") && text.contains("can see none"),
            "the planner's own sentence must survive to the script: {text}"
        );
        assert_eq!(
            result
                .get::<Option<String>>("code")
                .expect("code")
                .as_deref(),
            Some("planner::power_plant_needs_water"),
            "the refusal must be recognisable by the planner's own code, \
             not by matching the message: {text}"
        );
        let message: String = result.get("message").expect("message");
        assert!(
            message.starts_with("a power plant needs water"),
            "the carried message is the planner's sentence, unprefixed, so a \
             report line can put its own word in front of it: {message}"
        );
    }

    /// The negative control, and the reason this is not a catch-all: a
    /// technology no force defines is a **fault**, so it carries no refusal
    /// and stays a raise that ends the run.
    ///
    /// The name refers to nothing. No amount of mining, building or waiting
    /// makes `researched("no-such-technology")` mean something, so there is no
    /// verdict about the world to record -- either the script has a typo or
    /// the world was never told about the technology, and both are broken
    /// inputs. `supervisor.lua` has said so since it was written; this pins
    /// that the classification agrees with it.
    #[tokio::test]
    async fn a_technology_no_force_defines_is_a_fault_carrying_no_refusal() {
        use factorio_bot_core::types::FactorioForce;

        let world = fixture_world();
        let force: FactorioForce = factorio_bot_core::serde_json::from_str(RESEARCH_FORCE_JSON)
            .expect("the research force fixture must parse");
        world.update_force(force).expect("update_force");
        seed_players(&world, &[1]);

        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            Arc::new(world),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            vec![1],
            None,
            None,
            None,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");

        exec_bounded(
            &lua,
            r#"
            local ok, err = pcall(goal.plan, goal.researched("no-such-technology"))
            result = { ok = ok, text = tostring(err), refusal = goal.refusal(err) }
            "#,
        )
        .await;

        let result: LuaTable = lua.globals().get("result").expect("result");
        assert!(!result.get::<bool>("ok").expect("ok"));
        let text: String = result.get("text").expect("text");
        assert!(
            text.contains("defines no technology"),
            "the planner's own diagnosis must survive: {text}"
        );
        assert_eq!(
            result.get::<LuaValue>("refusal").expect("refusal"),
            LuaValue::Nil,
            "a name that refers to nothing is not a verdict about the world; \
             calling it one would let a typo end a run quietly: {text}"
        );
    }
}
