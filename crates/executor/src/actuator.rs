use async_trait::async_trait;
use factorio_bot_core::blueprint::UndergroundHalf;
use factorio_bot_core::factorio::rcon::{DestinationFull, WalkStall};
pub use factorio_bot_core::factorio::ticks::ActionTicks;
use factorio_bot_core::record::map::Placement;
use factorio_bot_core::types::Position;
use factorio_bot_planner::{BotId, InventorySlot};

#[derive(Debug, thiserror::Error)]
pub enum ActuatorError {
    #[error("bot {0:?} has no mapped Factorio player")]
    UnknownBot(BotId),
    #[error("the game does not define inventory slot {0}")]
    UnknownInventorySlot(&'static str),
    #[error("game rejected the command: {0}")]
    Rejected(String),
    /// The command reached the game and no readable verdict ever came back.
    ///
    /// **Not a failure.** `Rejected` says the game judged the command and the
    /// judgement was no; this says there is no judgement to report — the
    /// `action_completed` carried a status nothing could read (see
    /// `crates/core`'s output parser, which deliberately records no completion
    /// for one), or the reply never arrived at all. The action may well have
    /// happened.
    ///
    /// It is separate from `Rejected` because the executor renders the two
    /// differently and must: a verdict of failure is recorded as
    /// [`crate::Status::Failed`] and counts towards recovery's escalation
    /// budget, while this is recorded as [`crate::Status::Lost`] and counts
    /// towards nothing, because nothing was learned.
    #[error("the game reported no readable outcome: {0}")]
    NoVerdict(String),
}

impl ActuatorError {
    /// Attaches the ticks the game had already stamped when this went wrong.
    ///
    /// The only way to build an [`ActuatorFailure`] that carries a measurement,
    /// and it reads at the call site as what it is: *this failure, at these
    /// observed ticks*. An implementation with nothing stamped uses `?` or
    /// `.into()` instead and gets [`ActionTicks::UNKNOWN`], which is the honest
    /// answer rather than a default.
    pub fn at(self, ticks: ActionTicks) -> ActuatorFailure {
        ActuatorFailure { error: self, ticks }
    }
}

/// A dispatch that did not succeed, **and whatever the game had already told us
/// about when**.
///
/// # Why the ticks live here and not only on the success path
///
/// Every method below returns [`ActionTicks`] on success, and used to return a
/// bare [`ActuatorError`] on failure — so a dispatch the game stamped and then
/// refused arrived at the log with no ticks at all. That `None` was absent by
/// *plumbing*: we were handed the measurement and dropped it on the way out.
/// Every other absent tick in this design is absent by *fact* — no clock, an
/// unparseable stamp, a command the game never saw — and a reader cannot tell a
/// dropped measurement from a missing one. One `None`, two meanings, and the
/// reader left carrying the difference.
///
/// So the failure path carries the same value the success path does. `ticks`
/// is [`ActionTicks::UNKNOWN`] when nothing was stamped, which is now a claim
/// about the world rather than about this type's shape.
///
/// # Nothing here may be invented
///
/// `ticks` holds only what the game reported. A failure that happened before
/// the game saw the command has `ActionTicks::UNKNOWN` and must keep it: the
/// `From<ActuatorError>` conversion — the one `?` uses — is deliberately the
/// one that cannot supply a number, so the easy path is the honest one and
/// attaching a measurement takes an explicit [`ActuatorError::at`].
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct ActuatorFailure {
    /// What went wrong.
    #[source]
    pub error: ActuatorError,
    /// What the game had stamped by the time it did. `ActionTicks::UNKNOWN`
    /// when the game said nothing — never a plan value, never a zero.
    pub ticks: ActionTicks,
}

/// The conversion `?` uses, and the reason a pre-dispatch failure stays absent
/// without anyone having to remember: an error on its own carries no
/// observation, so it gets none.
impl From<ActuatorError> for ActuatorFailure {
    fn from(error: ActuatorError) -> Self {
        ActuatorFailure {
            error,
            ticks: ActionTicks::UNKNOWN,
        }
    }
}

/// Everything the executor can tell a bot to do.
///
/// One method per `ActionKind` variant plus `walk`, which the schedule emits as
/// its own step. Argument order is normalized here; `FactorioRcon`'s own
/// signatures are inconsistent about where `world` goes (`move_player` takes it
/// first, `place_entity` last).
///
/// The trait exists mainly so the run loop can be tested without a running
/// game: `FactorioRcon`'s `automock` is gated on core's own `cfg(test)` and is
/// therefore invisible from this crate.
///
/// # Why every method returns [`ActionTicks`] rather than `()`
///
/// This is the executor's **only** game-clock source. Nothing in this crate
/// reads `game.tick`, and nothing may: a tick invented here would not be a
/// measurement. So the dispatch itself carries the ticks back — the round trip
/// was already being paid, and BotBridge stamps the game tick on every response
/// the executor dispatches (`mods/BotBridge/control.lua`, `stamp_tick`).
///
/// Two ticks, not one, because a dispatch genuinely has two observable moments:
/// when the game received the command and when it reported the outcome. For an
/// asynchronous action they differ by however long the bot took; for a
/// synchronous one they are the same tick, which is a fact about the action
/// rather than a gap being papered over.
///
/// An implementation with no clock returns [`ActionTicks::UNKNOWN`]. That is a
/// legitimate answer and every consumer must handle it — see
/// [`crate::log::Attempt`] on why an absent tick is never defaulted.
///
/// # The failure path carries ticks too
///
/// A dispatch that the game stamped and then refused has a real dispatch tick,
/// so the error type is [`ActuatorFailure`] — the error *and* whatever was
/// observed — rather than a bare [`ActuatorError`]. See its docs: this is what
/// keeps "we never measured it" and "we measured it and threw it away" from
/// looking identical downstream.
#[async_trait]
pub trait Actuator: Send + Sync {
    /// Go stand in the annulus `(min_radius, radius]` around `to`.
    ///
    /// `to` is the position the plan's `AtPosition` precondition names, which
    /// for a place, an insert or a remove is the **entity's own tile** — a
    /// tile the bot cannot occupy. The two radii are that condition's own
    /// bounds, and an implementation that ignores either is asking the game to
    /// path onto an obstacle: `radius` alone would accept the centre, and a
    /// positive `min_radius` is exactly the statement that the centre is
    /// forbidden ground. See
    /// [`factorio_bot_core::factorio::rcon::approach_annulus`] for what to ask
    /// the game for given both.
    async fn walk(
        &self,
        bot: BotId,
        to: Position,
        min_radius: f64,
        radius: f64,
    ) -> Result<ActionTicks, ActuatorFailure>;

    /// Take the stalls the walk just finished had to be retried through.
    ///
    /// Called once per walk, straight after [`Actuator::walk`], and it takes
    /// rather than reads so nothing carries over to the next walk.
    ///
    /// **`None` and `Some(vec![])` are different answers and must stay that
    /// way.** `None` is "this actuator does not observe stalls", which is what
    /// the default returns and what every stub in this workspace means;
    /// `Some(vec![])` is "I looked, and that walk did not stall". A mock that
    /// answered with an empty vector would archive a run with no
    /// instrumentation as a run with no trouble -- the exact substitution this
    /// field exists to prevent (see
    /// [`crate::log::WalkObservation::stalls`]).
    ///
    /// Defaulted for the reason [`Actuator::reach_corrections`] is: an
    /// actuator with no game underneath it has nothing to report.
    fn take_walk_stalls(&self, _bot: BotId) -> Option<Vec<WalkStall>> {
        None
    }

    /// How many walks have come to rest outside the action's reach and needed
    /// a corrective step, since this actuator was built.
    ///
    /// A walk stops on the *outer* ring of its annulus, holding back
    /// `factorio_bot_core::factorio::rcon::ARRIVAL_MARGIN` for the arrival
    /// itself. That margin is measured rather than proved, so this counter is
    /// how a run says whether the measurement held: **zero means the margin
    /// could be tightened, and a rising number means it is too thin.** It goes
    /// into `EventKind::BatchProgress`, beside the other counters that state
    /// facts and no verdict.
    ///
    /// Defaulted to zero rather than required, because an actuator with no
    /// game underneath it has nothing to report and should not have to say so:
    /// every stub in this workspace would otherwise grow a line about a
    /// mechanism it does not have.
    fn reach_corrections(&self) -> u64 {
        0
    }

    /// Ask the game to generate the ground around `around`, so a bot can walk
    /// there. Answers how many chunks actually appeared.
    ///
    /// **This is the one act in this trait a human player cannot perform**, and
    /// it exists because a bot cannot walk into ungenerated ground at all: the
    /// pathfinder returns no path past the edge of the generated world, so a
    /// survey aimed at unexplored ground is refused before it is dispatched.
    /// Measured live on seed 31337 -- x=200 reached, x=300 through x=600 all
    /// `failed to path find`. A player crosses that edge by walking, and the
    /// engine makes the ground as they go; a bot driven through `request_path`
    /// cannot, which is an artefact of how we steer a character rather than a
    /// rule of the game.
    ///
    /// `radius` is in **chunks and is clamped mod-side to 4**, the reveal a
    /// character standing there would have been given for free. The caller
    /// cannot widen it.
    ///
    /// It charts nothing and reveals nothing to the force; the world model
    /// still learns only through `on_chunk_generated`. What it does buy is
    /// ground appearing slightly *before* the bot arrives rather than as it
    /// does, which is why [`Actuator::ground_generated`] exists and why every
    /// run that uses it says so.
    ///
    /// Defaulted to "generated nothing" for the same reason
    /// [`Actuator::reach_corrections`] is defaulted: an actuator with no game
    /// underneath it has no ground to make. A stub answering zero is honest --
    /// it generated zero.
    async fn generate_chunks(
        &self,
        _around: &Position,
        _radius: u32,
    ) -> Result<u64, ActuatorFailure> {
        Ok(0)
    }

    /// How much ground this run has asked the game to create: the number of
    /// [`Actuator::generate_chunks`] calls, the chunks they actually made, and
    /// how many of those calls failed.
    ///
    /// **The disclosure counter.** A run that generated ground is not
    /// comparable to one that did not, and the project's rule is that anything
    /// a player could not do is recorded rather than argued about -- the way
    /// `research_trigger_emulated` is. These two ride in
    /// `EventKind::BatchProgress`, which already states facts and no verdict
    /// and beats every 30 seconds, so the disclosure survives a killed run.
    ///
    /// The third number is what keeps a broken verb from reading as an
    /// uneventful run: this crate has no logger, so a `generate_chunks` that
    /// the server refuses -- an older BotBridge with no such function, say --
    /// would otherwise be indistinguishable from ground that already existed.
    /// `calls > 0, chunks == 0, failures == 0` is "the ground was already
    /// there"; `failures == calls` is "this verb is not working".
    ///
    /// `(0, 0, 0)` means this run never asked, which is the honest reading for
    /// every actuator that has no game under it.
    fn ground_generated(&self) -> (u64, u64, u64) {
        (0, 0, 0)
    }
    async fn mine(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure>;
    async fn craft(
        &self,
        bot: BotId,
        recipe: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure>;
    /// `underground_half` is `Some` only when `item` is one half of an
    /// underground-belt pair (`FactorioEntity::underground_half`); every
    /// other `Place` action passes `None`. Forwarded all the way to
    /// `rcon_place_entity` (`mods/BotBridge/control.lua`), which sends it to
    /// `surface.create_entity` as `type` -- the reason the two halves of a
    /// pair can now be told apart at all. See
    /// `FactorioEntity::new_underground_belt`.
    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
        underground_half: Option<UndergroundHalf>,
    ) -> Result<ActionTicks, ActuatorFailure>;
    #[allow(clippy::too_many_arguments)]
    async fn insert(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure>;
    #[allow(clippy::too_many_arguments)]
    async fn remove(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure>;
    /// Research is server-wide in Factorio: it takes no player id.
    /// `expected_ticks` is the plan's nominal duration for the research -- the
    /// one action kind whose real length is set by the factory (lab count,
    /// pack supply) rather than by a bot, and the one most able to outlast a
    /// flat wall-clock deadline honestly. The actuator sizes its wait from it.
    async fn research(
        &self,
        tech: &str,
        expected_ticks: u32,
    ) -> Result<ActionTicks, ActuatorFailure>;

    /// Ask the force to create a space platform in orbit of `planet`, to be
    /// delivered by a rocket carrying `starter_pack`.
    ///
    /// **No `BotId`, for the same reason [`Actuator::research`] takes none**:
    /// `LuaForce.create_space_platform` is force-level. It names no
    /// `LuaEntity` and no tile either, which is why the planner cannot express
    /// it as a `place` or an `insert` -- see `ActionKind::CreatePlatform`.
    ///
    /// **A required method, not a default**, on [`Actuator::set_recipe`]'s
    /// grounds: a default returning `Ok` would let an actuator report a
    /// platform created by a game that was never asked.
    ///
    /// It creates a *pending* platform, waiting for its starter pack. What
    /// makes it real is the ordinary `insert` that follows, and then the silo
    /// launching itself -- there is deliberately no launch verb on this trait.
    async fn create_platform(
        &self,
        name: &str,
        planet: &str,
        starter_pack: &str,
    ) -> Result<ActionTicks, ActuatorFailure>;

    /// Put `recipe` on the crafting machine named `entity` at `at`.
    ///
    /// `entity` as well as `at` for the same reason [`Actuator::insert`] takes
    /// both: the mod addresses an existing building with
    /// `surface.find_entity(name, position)`.
    ///
    /// **A required method, not a default.** Every other dispatch here is
    /// required, and a default that returned `Ok` would let an actuator report
    /// a recipe set on a machine it never touched -- the placed-but-dead
    /// machine this verb exists to prevent, arriving from the one direction
    /// nothing downstream can check.
    ///
    /// # The one idempotent dispatch on this trait
    ///
    /// `insert` moves items, `remove` moves them back, `place` builds; running
    /// any of them twice is not running it once. This one assigns, so a
    /// re-dispatch of a recipe already set is a no-op in the game as well as
    /// in the plan -- which is what makes it safe under `recover.rs`'s tier 1,
    /// where its neighbours are the counter-examples.
    async fn set_recipe(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        recipe: &str,
    ) -> Result<ActionTicks, ActuatorFailure>;

    /// Stamp ghosts of `blueprint`'s entities at `anchor` --
    /// `rcon_place_blueprint(..., only_ghosts = true)`, the same call
    /// `Actuator::place` would make with `only_ghosts = false`.
    ///
    /// [`ActionKind::StampGhosts`]'s dispatch, and it exists to be **skippable
    /// without consequence for correctness**: a ghost is a recovery marker and
    /// a viewer cue, not a building. Nothing downstream of this needs it to
    /// have run -- `method::blueprint::BuildBlock`'s real `Place` steps carry
    /// their own preconditions and place correctly with or without a ghost
    /// ever having stood there. That is what justifies defaulting this to a
    /// no-op success rather than requiring every test actuator to grow a
    /// stub for it, the same reasoning [`Actuator::generate_chunks`] already
    /// rests on: an actuator with no game underneath has nothing to mark, and
    /// [`RconActuator`](crate::rcon_actuator::RconActuator) is the one
    /// implementation that overrides it to actually ask the game.
    ///
    /// `ActionTicks::UNKNOWN` on the default path is honest, not lazy: no
    /// actuator without a game clock can say when a mark it never made would
    /// have landed.
    async fn stamp_ghosts(
        &self,
        _bot: BotId,
        _blueprint: &str,
        _anchor: Position,
    ) -> Result<ActionTicks, ActuatorFailure> {
        Ok(ActionTicks::UNKNOWN)
    }

    /// The game's simulation speed multiplier — Factorio's own `game.speed`,
    /// where `1.0` is normal (60 ticks/second) and the game accepts anything
    /// from `0.01` up.
    ///
    /// `run::ticks_to_wall_clock` uses this to convert a lag edge (machine
    /// time — a furnace keeps smelting after the bot walks away) into a sleep
    /// duration. Getting it wrong makes every such wait wrong: at half speed
    /// a wait computed for `1.0` fires while the furnace still has plates
    /// left to make, and the executor collects from a furnace too early.
    ///
    /// **The default is `1.0`**, which is what every mock wants.
    /// `RconActuator` (`crates/executor/src/rcon_actuator.rs`) overrides it
    /// by asking the game through `FactorioRcon::game_speed`, so a run started
    /// with `--game-speed` sizes its lag waits from the speed the world is
    /// actually running at.
    ///
    /// A bare [`ActuatorError`], unlike the dispatches above: this asks the
    /// game a question rather than telling a bot to do something, so there is
    /// no dispatch for a tick to belong to and nothing for an
    /// [`ActuatorFailure`] to carry.
    async fn game_speed(&self) -> Result<f64, ActuatorError> {
        Ok(1.0)
    }

    /// What tick the game is on **now**, or `None` from an actuator with no
    /// clock.
    ///
    /// # Why a lag edge cannot be waited out on the wall clock
    ///
    /// A lag edge is a count of *game* ticks — how long the furnace needs, not
    /// how long we should stand around. Converting it to seconds through
    /// [`Actuator::game_speed`] assumes the server actually delivers
    /// `60 * speed` ticks a second, and a server that is behind delivers
    /// fewer. `game.speed` cannot report that: it is the rate the game is
    /// *asked* to run at, and a headless server sharing a machine with four
    /// graphical clients and screenshotting six cameras every 300 ticks misses
    /// it by around a tenth.
    ///
    /// A tenth is enough. Run `run-1788320177-77989` waited a modelled 4032
    /// ticks for 20 iron plates, ~3599 ticks passed, the furnace had made 18,
    /// and the rung died there. The loss scales with the batch, which is why
    /// the small smelts earlier in the same run came back clean — nothing was
    /// wrong with the model, only with the clock it was measured against.
    ///
    /// So the wait reads this instead, and the wall clock survives only as the
    /// estimate of how long to sleep between readings. Returning `None` says
    /// "I have no clock" and puts the caller back on the old wall-clock wait,
    /// which is what every mock does and why none of them needed changing.
    ///
    /// A bare [`ActuatorError`] for the same reason as
    /// [`Actuator::game_speed`]: this asks the game a question rather than
    /// telling a bot to do something, so there is no dispatch for a tick to
    /// belong to.
    async fn game_tick(&self) -> Result<Option<u64>, ActuatorError> {
        Ok(None)
    }

    /// Has the force the bots act for finished `tech`?
    ///
    /// `Ok(None)` means **this actuator cannot answer** — no world to read, or
    /// a technology it has no record of. It is not "no": a caller that treats
    /// it as one waits out its whole budget for an answer that is never
    /// coming, so the two are kept apart here the same way [`ActuatorError::
    /// NoVerdict`] is kept apart from [`ActuatorError::Rejected`].
    ///
    /// # Why an action needs to ask at all
    ///
    /// A plan states `Condition::Researched(tech)` on a craft whose recipe a
    /// technology unlocks, and `ActionNetwork::infer_edges` turns that into an
    /// edge from whichever action carries the matching `Effect::Researched`.
    /// The edge orders the *actions* correctly and the executor honours it —
    /// but the game does not apply a 2.0 `craft-item` trigger technology in
    /// the tick the craft completes.
    ///
    /// `run-1788365280-15443` measured the gap. `workspace/server-log.txt`:
    /// `§53469§action_completed§ok 44` is the lab craft settling, and
    /// `§53485§on_research_finished§` is the technology it triggers landing —
    /// **16 ticks later**. The dependent craft went out inside that window and
    /// the mod refused it with "recipe automation-science-pack is not enabled
    /// for this force", which abandoned the milestone's whole iteration and
    /// cost 27,462 ticks of redone work. Nothing in the plan can express this:
    /// the edge already says everything the planner knows.
    ///
    /// So the executor checks the precondition it was given rather than
    /// assuming the predecessor's success delivered it. A default of `Ok(None)`
    /// keeps every actuator that cannot look — including every mock — behaving
    /// exactly as it did before this existed.
    ///
    /// A bare [`ActuatorError`] for the same reason as [`Actuator::game_speed`]
    /// and [`Actuator::game_tick`]: this asks the game a question rather than
    /// telling a bot to do something, so there is no dispatch for a tick to
    /// belong to.
    async fn technology_researched(&self, tech: &str) -> Result<Option<bool>, ActuatorError> {
        let _ = tech;
        Ok(None)
    }

    /// Claims the placement `bot` most recently made, if this actuator is
    /// tracking one.
    ///
    /// A default rather than a required method, and deliberately not async:
    /// giving `place` an `ActionId` to attach a `Placement` to directly would
    /// mean threading one through this whole trait and every implementation
    /// of it, including the two `mockall` mocks in `run.rs`'s and
    /// `recover.rs`'s tests, for a value most of them have no use for. Instead
    /// `run.rs`'s settle path calls this once the *scheduler's* id for the
    /// just-completed action is back in hand, and attaches whatever comes
    /// back to that action's `Attempt`.
    ///
    /// The default returns `None`: an actuator that tracks no placements has
    /// none to give, which is the honest answer for every implementation that
    /// does not override this — in particular every mock, which inherits it
    /// unmodified and so needs no change to keep compiling or passing.
    fn take_placement(&self, bot: BotId) -> Option<Placement> {
        let _ = bot;
        None
    }

    /// Claims the reason `bot`'s last `insert` succeeded without delivering
    /// everything, if that is what happened.
    ///
    /// The same side channel as [`Actuator::take_placement`], for the same
    /// reason and drained in the same breath: an `insert` that filled its
    /// destination is a **success** (see `judge_transfer_reply` in
    /// `factorio_bot_core::factorio::rcon`), so the fact has no failure to ride
    /// out on, and this trait has no `ActionId` to attach it to.
    ///
    /// Without it a run in which every boiler top-up moved 3 of 17 would be
    /// indistinguishable from one in which they all moved 17 -- and that
    /// difference is the whole diagnosis: it says the planner is sizing from
    /// demand against a fuel level nothing reports. The short-source failure it
    /// used to be confused with is still a failure and still arrives as one, so
    /// the record shows the two as different rows and not as one.
    ///
    /// The default returns `None`, exactly as `take_placement`'s does: an
    /// actuator that tracks no transfers has nothing to give, and every mock
    /// inherits it unchanged.
    fn take_destination_full(&self, bot: BotId) -> Option<DestinationFull> {
        let _ = bot;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_bot_names_the_bot() {
        let err = ActuatorError::UnknownBot(BotId(3));
        assert_eq!(
            err.to_string(),
            "bot BotId(3) has no mapped Factorio player"
        );
    }

    #[test]
    fn unknown_inventory_slot_names_the_defines_key() {
        let err = ActuatorError::UnknownInventorySlot(InventorySlot::LabInput.defines_key());
        assert_eq!(
            err.to_string(),
            "the game does not define inventory slot lab_input"
        );
    }

    #[test]
    fn a_failure_that_was_never_stamped_carries_no_ticks() {
        // The `?` path. An error on its own is not an observation, so the
        // conversion cannot supply one — which is what makes a pre-dispatch
        // failure stay absent without anybody having to remember.
        let f: ActuatorFailure = ActuatorError::UnknownBot(BotId(3)).into();
        assert_eq!(f.ticks, ActionTicks::UNKNOWN);
        assert_eq!(f.ticks.dispatched, None);
        assert_ne!(f.ticks.dispatched, Some(0), "absent is not tick zero");
        assert_eq!(f.to_string(), "bot BotId(3) has no mapped Factorio player");
    }

    #[test]
    fn a_failure_the_game_stamped_keeps_both_the_tick_and_the_message() {
        let f = ActuatorError::Rejected("no ore".to_string())
            .at(ActionTicks::new(Some(4_211), Some(4_270)));
        assert_eq!(f.ticks.dispatched, Some(4_211));
        assert_eq!(f.ticks.replied, Some(4_270));
        assert_eq!(f.to_string(), "game rejected the command: no ore");
        assert!(matches!(f.error, ActuatorError::Rejected(_)));
    }

    #[test]
    fn a_verdict_of_failure_and_no_verdict_at_all_are_different_errors() {
        // They are recorded as different states, so they must be distinguishable
        // here: `Rejected` says the game judged the command, `NoVerdict` says
        // there is nothing to report.
        let rejected = ActuatorError::Rejected("no ore".to_string());
        let silent = ActuatorError::NoVerdict("unreadable status".to_string());
        assert!(matches!(rejected, ActuatorError::Rejected(_)));
        assert!(!matches!(silent, ActuatorError::Rejected(_)));
        assert_eq!(
            silent.to_string(),
            "the game reported no readable outcome: unreadable status"
        );
    }

    #[tokio::test]
    async fn an_actuator_with_no_world_says_it_cannot_answer_rather_than_no() {
        // The default has to be `None`, not `Some(false)`. `false` is a claim
        // about the game — "this technology is not researched" — and a caller
        // that believes it waits out its budget every single time.
        struct Blind;
        #[async_trait]
        impl Actuator for Blind {
            async fn walk(
                &self,
                _: BotId,
                _: Position,
                _: f64,
                _: f64,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn mine(
                &self,
                _: BotId,
                _: &str,
                _: Position,
                _: u32,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn craft(
                &self,
                _: BotId,
                _: &str,
                _: u32,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn place(
                &self,
                _: BotId,
                _: &str,
                _: Position,
                _: u8,
                _: Option<UndergroundHalf>,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn insert(
                &self,
                _: BotId,
                _: &str,
                _: Position,
                _: InventorySlot,
                _: &str,
                _: u32,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn remove(
                &self,
                _: BotId,
                _: &str,
                _: Position,
                _: InventorySlot,
                _: &str,
                _: u32,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn research(&self, _: &str, _: u32) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn create_platform(
                &self,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
            async fn set_recipe(
                &self,
                _: BotId,
                _: &str,
                _: Position,
                _: &str,
            ) -> Result<ActionTicks, ActuatorFailure> {
                unreachable!()
            }
        }

        let answer = Blind
            .technology_researched("automation-science-pack")
            .await
            .expect("the default cannot fail");
        assert_eq!(answer, None, "an actuator with no world cannot answer");
        assert_ne!(
            answer,
            Some(false),
            "cannot answer is not the same claim as not researched"
        );
    }

    #[test]
    fn the_trait_is_object_safe() {
        // Task 4 stores an actuator behind a trait object / generic bound; a
        // compile-time check here is cheaper than discovering it there.
        fn assert_object_safe(_: Option<&dyn Actuator>) {}
        assert_object_safe(None);
    }
}
