use crate::actuator::{Actuator, ActuatorError};
use async_trait::async_trait;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::{PlayerId, Position};
use factorio_bot_planner::{BotId, InventorySlot};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The game's own `defines.inventory` table, read once at construction.
///
/// Never hardcoded: these integers are entity-type dependent (see
/// `mods/BotBridge/control.lua`, `inventory_type_name(invtype, enttype)`) and
/// they move between Factorio versions.
#[derive(Debug, Clone, Default)]
pub struct InventoryDefines {
    by_name: BTreeMap<String, u32>,
}

impl InventoryDefines {
    pub fn from_json(s: &str) -> Result<Self, ActuatorError> {
        let by_name: BTreeMap<String, u32> = serde_json::from_str(s)
            .map_err(|e| ActuatorError::Rejected(format!("bad defines reply: {e}")))?;
        Ok(Self { by_name })
    }

    pub fn get(&self, slot: InventorySlot) -> Result<u32, ActuatorError> {
        let key = slot.defines_key();
        self.by_name
            .get(key)
            .copied()
            .ok_or(ActuatorError::UnknownInventorySlot(key))
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// The Lua the game runs to report its inventory defines.
///
/// `helpers.table_to_json`, not `game.table_to_json`: the latter was removed in
/// Factorio 2.0 and BotBridge itself uses `helpers` throughout
/// (`mods/BotBridge/control.lua`).
pub const DEFINES_QUERY: &str = "/silent-command \
local t={} for k,v in pairs(defines.inventory) do t[k]=v end \
rcon.print(helpers.table_to_json(t))";

/// Drives a real Factorio game over RCON.
pub struct RconActuator {
    rcon: Arc<FactorioRcon>,
    world: Arc<FactorioWorld>,
    defines: InventoryDefines,
    /// Planner bot ids to Factorio player ids. Bots are interchangeable to the
    /// planner; this is where they acquire an identity in the game.
    players: BTreeMap<BotId, PlayerId>,
}

impl RconActuator {
    /// Discovers its own bots: every connected player becomes a bot, numbered
    /// `BotId(0..n)` in ascending player-id order.
    ///
    /// Sorted, so the mapping is a function of who is connected and not of the
    /// order the game happened to list them. The planner treats bots as
    /// interchangeable, so which player gets which id does not affect the plan
    /// — but it must be stable across a re-plan within one run, or the executor
    /// would hand a chain to a different body midway.
    pub async fn new(
        rcon: Arc<FactorioRcon>,
        world: Arc<FactorioWorld>,
    ) -> Result<Self, ActuatorError> {
        let mut ids: Vec<PlayerId> = rcon
            .connected_players()
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))?
            .into_iter()
            .map(|p| p.player_id)
            .collect();
        ids.sort_unstable();
        let players = Self::bot_mapping(ids);
        if players.is_empty() {
            return Err(ActuatorError::Rejected("no connected players".to_string()));
        }
        // `FactorioRcon::send` is `async fn send(&self, command: &str)
        // -> Result<Option<Vec<String>>>` (crates/core/src/factorio/rcon.rs).
        // A silent-command reply arrives as one line; absence means the game
        // answered nothing, which is a hard error here.
        let reply = rcon
            .send(DEFINES_QUERY)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))?
            .and_then(|lines| lines.into_iter().next())
            .ok_or_else(|| ActuatorError::Rejected("no reply to defines query".to_string()))?;
        let defines = InventoryDefines::from_json(&reply)?;
        Ok(Self {
            rcon,
            world,
            defines,
            players,
        })
    }

    /// Ascending player ids become `BotId(0..n)`. Factored out of `new` so the
    /// numbering is testable without a game.
    fn bot_mapping(sorted_player_ids: Vec<PlayerId>) -> BTreeMap<BotId, PlayerId> {
        sorted_player_ids
            .into_iter()
            .enumerate()
            .map(|(i, pid)| (BotId(i as u8), pid))
            .collect()
    }

    fn player(&self, bot: BotId) -> Result<PlayerId, ActuatorError> {
        self.players
            .get(&bot)
            .copied()
            .ok_or(ActuatorError::UnknownBot(bot))
    }

    /// The bots this actuator discovered, lowest first.
    pub fn bots(&self) -> Vec<BotId> {
        self.players.keys().copied().collect()
    }

    pub fn defines(&self) -> &InventoryDefines {
        &self.defines
    }
}

#[async_trait]
impl Actuator for RconActuator {
    async fn walk(&self, bot: BotId, to: Position) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        self.rcon
            .move_player(&self.world, p, &to, None)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn mine(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        count: u32,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        self.rcon
            .player_mine(&self.world, p, item, &at, count)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn craft(&self, bot: BotId, recipe: &str, count: u32) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        self.rcon
            .player_craft(&self.world, p, recipe, count)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        // `place_entity` returns the created FactorioEntity; the executor does
        // not need it, because the plan already knows what it placed and the
        // world snapshot is refreshed by the event stream, not by this reply.
        self.rcon
            .place_entity(p, item.to_string(), at, direction, &self.world)
            .await
            .map(|_entity| ())
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn insert(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        let inv = self.defines.get(slot)?;
        self.rcon
            .insert_to_inventory(
                p,
                entity.to_string(),
                at,
                inv,
                item.to_string(),
                count,
                &self.world,
            )
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    async fn remove(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<(), ActuatorError> {
        let p = self.player(bot)?;
        let inv = self.defines.get(slot)?;
        self.rcon
            .remove_from_inventory(
                p,
                entity.to_string(),
                at,
                inv,
                item.to_string(),
                count,
                &self.world,
            )
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }

    /// Research is server-wide: `add_research` takes no player id, so `bot`
    /// does not appear here. Two bots researching the same technology is
    /// idempotent in Factorio.
    async fn research(&self, tech: &str) -> Result<(), ActuatorError> {
        self.rcon
            .add_research(tech)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_are_parsed_from_the_games_reply() {
        let json = r#"{"chest":1,"furnace_source":2,"furnace_result":3,"fuel":1}"#;
        let d = InventoryDefines::from_json(json).expect("parses");
        assert_eq!(d.get(InventorySlot::FurnaceSource).unwrap(), 2);
        assert_eq!(d.get(InventorySlot::FurnaceResult).unwrap(), 3);
    }

    #[test]
    fn a_slot_the_game_does_not_define_is_an_error() {
        let d = InventoryDefines::from_json(r#"{"chest":1}"#).expect("parses");
        assert!(matches!(
            d.get(InventorySlot::LabInput),
            Err(ActuatorError::UnknownInventorySlot("lab_input"))
        ));
    }

    #[test]
    fn every_slot_the_planner_can_emit_resolves_against_a_full_defines_table() {
        // The keys are exactly those in `mods/BotBridge/control.lua`'s
        // `inventory_type_name`; the numbers are arbitrary, since the point is
        // that no planner slot is missing a lookup key.
        let json = r#"{
            "chest": 1,
            "fuel": 1,
            "furnace_source": 2,
            "furnace_result": 3,
            "assembling_machine_input": 2,
            "assembling_machine_output": 3,
            "lab_input": 2
        }"#;
        let d = InventoryDefines::from_json(json).expect("parses");
        for slot in [
            InventorySlot::Chest,
            InventorySlot::Fuel,
            InventorySlot::FurnaceSource,
            InventorySlot::FurnaceResult,
            InventorySlot::AssemblerInput,
            InventorySlot::AssemblerOutput,
            InventorySlot::LabInput,
        ] {
            assert!(d.get(slot).is_ok(), "no mapping for {slot:?}");
        }
    }

    #[test]
    fn a_reply_that_is_not_a_defines_table_is_rejected_not_silently_empty() {
        let err = InventoryDefines::from_json("nil").expect_err("must not parse");
        assert!(matches!(err, ActuatorError::Rejected(_)), "got {err:?}");
    }

    #[test]
    fn bots_are_numbered_from_zero_in_player_id_order() {
        let mut ids: Vec<PlayerId> = vec![7, 2, 5];
        ids.sort_unstable();
        let map = RconActuator::bot_mapping(ids);
        assert_eq!(map.get(&BotId(0)).copied(), Some(2));
        assert_eq!(map.get(&BotId(1)).copied(), Some(5));
        assert_eq!(map.get(&BotId(2)).copied(), Some(7));
    }

    #[test]
    fn no_connected_players_yields_no_bots() {
        assert!(RconActuator::bot_mapping(vec![]).is_empty());
    }

    #[test]
    fn the_defines_query_asks_the_game_and_not_a_hardcoded_table() {
        // Guards the two things that silently break only against a live game:
        // the 2.0 `helpers` namespace, and reading `defines.inventory` rather
        // than shipping our own numbers.
        assert!(DEFINES_QUERY.starts_with("/silent-command "));
        assert!(DEFINES_QUERY.contains("pairs(defines.inventory)"));
        assert!(DEFINES_QUERY.contains("helpers.table_to_json"));
        assert!(!DEFINES_QUERY.contains("game.table_to_json"));
        assert!(!DEFINES_QUERY.contains('\n'));
    }
}
