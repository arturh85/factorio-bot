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
/// Never hardcoded: the numbers move between Factorio versions, and so do the
/// names — 2.0 replaced `furnace_source` and `assembling_machine_input` with a
/// single `crafter_input`. The authority is the game's own table, published in
/// `runtime-api.json`; `mods/BotBridge/control.lua`'s `inventory_type_name` is
/// dead 1.1-era code with no callers and must not be used as a reference.
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
}

/// `defines.inventory` as the installed game publishes it, from
/// `workspace/factorio-api-docs/runtime-api.json` (Factorio 2.1.17).
///
/// A snapshot, so the check below still runs where the docs are not present:
/// `workspace/` is gitignored, so CI has no copy. When the docs *are* present,
/// `the_snapshot_still_matches_the_installed_games_defines` re-derives this
/// list and fails if the game has moved, which is how a game update surfaces
/// as a test failure rather than as `UnknownInventorySlot` mid-run.
#[cfg(test)]
const FACTORIO_2_1_INVENTORY_DEFINES: [&str; 56] = [
    "agricultural_tower_input",
    "agricultural_tower_modules",
    "agricultural_tower_output",
    "artillery_turret_ammo",
    "artillery_wagon_ammo",
    "assembling_machine_dump",
    "asteroid_collector_arm",
    "asteroid_collector_output",
    "beacon_modules",
    "burnt_result",
    "car_ammo",
    "car_trash",
    "car_trunk",
    "cargo_landing_pad_main",
    "cargo_landing_pad_trash",
    "cargo_unit",
    "cargo_wagon",
    "character_ammo",
    "character_armor",
    "character_corpse",
    "character_guns",
    "character_main",
    "character_trash",
    "character_vehicle",
    "chest",
    "crafter_input",
    "crafter_modules",
    "crafter_output",
    "crafter_trash",
    "editor_ammo",
    "editor_armor",
    "editor_guns",
    "editor_main",
    "fuel",
    "god_main",
    "hub_main",
    "hub_trash",
    "item_main",
    "lab_input",
    "lab_modules",
    "lab_trash",
    "linked_container_main",
    "logistic_container_trash",
    "mining_drill_modules",
    "proxy_main",
    "roboport_material",
    "roboport_robot",
    "robot_cargo",
    "robot_repair",
    "rocket_silo_attached_cargo_unit",
    "rocket_silo_rocket",
    "rocket_silo_trash",
    "spider_ammo",
    "spider_trash",
    "spider_trunk",
    "turret_ammo",
];

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
        let ids: Vec<PlayerId> = rcon
            .connected_players()
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))?
            .into_iter()
            .map(|p| p.player_id)
            .collect();
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

    /// Player ids become `BotId(0..n)` in ascending order.
    ///
    /// The sort lives here, with the function that promises the ordering, so
    /// the guarantee cannot be lost by a caller collecting ids in whatever
    /// order the game listed them. Factored out of `new` so the numbering is
    /// testable without a game.
    fn bot_mapping(mut player_ids: Vec<PlayerId>) -> BTreeMap<BotId, PlayerId> {
        player_ids.sort_unstable();
        player_ids
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
    use std::collections::BTreeSet;

    #[test]
    fn defines_are_parsed_from_the_games_reply() {
        let json = r#"{"chest":2,"crafter_input":50,"crafter_output":51,"fuel":0}"#;
        let d = InventoryDefines::from_json(json).expect("parses");
        assert_eq!(d.get(InventorySlot::FurnaceSource).unwrap(), 50);
        assert_eq!(d.get(InventorySlot::FurnaceResult).unwrap(), 51);
    }

    #[test]
    fn a_slot_the_game_does_not_define_is_an_error() {
        let d = InventoryDefines::from_json(r#"{"chest":2}"#).expect("parses");
        assert!(matches!(
            d.get(InventorySlot::LabInput),
            Err(ActuatorError::UnknownInventorySlot("lab_input"))
        ));
    }

    /// Reads `defines.inventory` out of the installed game's own API docs.
    /// `None` when they are not downloaded; `workspace/` is gitignored.
    fn defines_from_the_installed_game() -> Option<BTreeSet<String>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../workspace/factorio-api-docs/runtime-api.json");
        let raw = std::fs::read_to_string(path).ok()?;
        let doc: serde_json::Value = serde_json::from_str(&raw).expect("runtime-api.json parses");
        let inventory = doc["defines"]
            .as_array()
            .expect("defines is an array")
            .iter()
            .find(|d| d["name"] == "inventory")
            .expect("the game defines an inventory table");
        Some(
            inventory["values"]
                .as_array()
                .expect("inventory has values")
                .iter()
                .map(|v| v["name"].as_str().expect("a define has a name").to_string())
                .collect(),
        )
    }

    /// The check that would have caught the 1.1-era names: every slot the
    /// planner can emit must name an inventory the game actually has.
    #[test]
    fn every_slot_the_planner_can_emit_names_a_real_2_1_inventory() {
        let known: BTreeSet<&str> = FACTORIO_2_1_INVENTORY_DEFINES.into_iter().collect();
        for slot in InventorySlot::ALL {
            let key = slot.defines_key();
            assert!(
                known.contains(key),
                "{slot:?} asks for `{key}`, which Factorio 2.1 does not define"
            );
        }
    }

    #[test]
    fn the_snapshot_still_matches_the_installed_games_defines() {
        let Some(game) = defines_from_the_installed_game() else {
            // No docs downloaded; the snapshot test above still runs.
            return;
        };
        let snapshot: BTreeSet<String> = FACTORIO_2_1_INVENTORY_DEFINES
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(
            snapshot, game,
            "the game's defines.inventory has changed; re-derive the snapshot \
             and recheck every InventorySlot::defines_key"
        );
        for slot in InventorySlot::ALL {
            let key = slot.defines_key();
            assert!(
                game.contains(key),
                "{slot:?} asks for `{key}`, which the installed game does not define"
            );
        }
    }

    #[test]
    fn every_slot_resolves_once_the_game_reports_its_table() {
        // Numbers as `runtime-api.json` orders them for 2.1.17. They are only
        // ever read from the game at runtime; they appear here so the lookup
        // itself is exercised end to end.
        let json = r#"{
            "fuel": 0,
            "chest": 2,
            "lab_input": 20,
            "crafter_input": 50,
            "crafter_output": 51
        }"#;
        let d = InventoryDefines::from_json(json).expect("parses");
        for slot in InventorySlot::ALL {
            assert!(d.get(slot).is_ok(), "no mapping for {slot:?}");
        }
        // 2.0 unified furnaces and assemblers: these are now the same slot.
        assert_eq!(
            d.get(InventorySlot::FurnaceSource).unwrap(),
            d.get(InventorySlot::AssemblerInput).unwrap()
        );
        assert_eq!(
            d.get(InventorySlot::FurnaceResult).unwrap(),
            d.get(InventorySlot::AssemblerOutput).unwrap()
        );
    }

    #[test]
    fn a_reply_that_is_not_a_defines_table_is_rejected_not_silently_empty() {
        let err = InventoryDefines::from_json("nil").expect_err("must not parse");
        assert!(matches!(err, ActuatorError::Rejected(_)), "got {err:?}");
    }

    #[test]
    fn bots_are_numbered_from_zero_in_player_id_order() {
        // Deliberately unsorted: the ordering, not `enumerate`, is what this
        // guards. Bot identity must be a function of who is connected, not of
        // the order the game happened to list them, or a re-plan would hand a
        // chain to a different body midway.
        let ids: Vec<PlayerId> = vec![7, 2, 5];
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
