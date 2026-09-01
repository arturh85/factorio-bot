use async_trait::async_trait;
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
    /// Go stand within `radius` of `to`.
    ///
    /// `to` is the position the plan's `AtPosition` precondition names, which
    /// for a place, an insert or a remove is the **entity's own tile** — a
    /// tile the bot cannot occupy. `radius` is the tolerance that makes the
    /// request satisfiable, and an implementation that ignores it is asking
    /// the game to path onto an obstacle. See
    /// [`factorio_bot_core::factorio::rcon::approach_radius`] for how far
    /// inside the tolerance to actually aim.
    async fn walk(
        &self,
        bot: BotId,
        to: Position,
        radius: f64,
    ) -> Result<ActionTicks, ActuatorFailure>;
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
    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
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
    async fn research(&self, tech: &str) -> Result<ActionTicks, ActuatorFailure>;

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
    /// **Stubbed at `1.0` deliberately.** A real answer needs an RCON round
    /// trip through `FactorioRcon` (`crates/core`), which this crate does not
    /// own and is not touching while another change is in flight there.
    /// `crates/core::factorio::rcon::FactorioRcon` needs a dedicated method
    /// to read `game.speed` — the same shape as the existing
    /// `RconActuator::new`'s `DEFINES_QUERY` silent-command round trip — and
    /// `RconActuator` (`crates/executor/src/rcon_actuator.rs`) should then
    /// override this default to call it. Until that lands every schedule
    /// keeps assuming normal speed, exactly as before this change, but the
    /// assumption now lives in one named, overridable place instead of a bare
    /// `60` a reader had to know to distrust.
    ///
    /// A bare [`ActuatorError`], unlike the dispatches above: this asks the
    /// game a question rather than telling a bot to do something, so there is
    /// no dispatch for a tick to belong to and nothing for an
    /// [`ActuatorFailure`] to carry.
    async fn game_speed(&self) -> Result<f64, ActuatorError> {
        Ok(1.0)
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

    #[test]
    fn the_trait_is_object_safe() {
        // Task 4 stores an actuator behind a trait object / generic bound; a
        // compile-time check here is cheaper than discovering it there.
        fn assert_object_safe(_: Option<&dyn Actuator>) {}
        assert_object_safe(None);
    }
}
