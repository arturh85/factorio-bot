use async_trait::async_trait;
pub use factorio_bot_core::factorio::ticks::ActionTicks;
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
#[async_trait]
pub trait Actuator: Send + Sync {
    async fn walk(&self, bot: BotId, to: Position) -> Result<ActionTicks, ActuatorError>;
    async fn mine(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        count: u32,
    ) -> Result<ActionTicks, ActuatorError>;
    async fn craft(
        &self,
        bot: BotId,
        recipe: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorError>;
    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
    ) -> Result<ActionTicks, ActuatorError>;
    #[allow(clippy::too_many_arguments)]
    async fn insert(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorError>;
    #[allow(clippy::too_many_arguments)]
    async fn remove(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorError>;
    /// Research is server-wide in Factorio: it takes no player id.
    async fn research(&self, tech: &str) -> Result<ActionTicks, ActuatorError>;

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
    async fn game_speed(&self) -> Result<f64, ActuatorError> {
        Ok(1.0)
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
    fn the_trait_is_object_safe() {
        // Task 4 stores an actuator behind a trait object / generic bound; a
        // compile-time check here is cheaper than discovering it there.
        fn assert_object_safe(_: Option<&dyn Actuator>) {}
        assert_object_safe(None);
    }
}
