use crate::actuator::{ActionTicks, Actuator, ActuatorError, ActuatorFailure};
use async_trait::async_trait;
use factorio_bot_core::factorio::rcon::FactorioRcon;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::{PlayerId, Position};
use factorio_bot_planner::{BotId, InventorySlot};
use std::collections::{BTreeMap, BTreeSet};
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
    /// The players the game reported as connected when this actuator was built.
    ///
    /// Not a mapping: a `BotId` *is* a player id (see [`BotId`]). This set is
    /// only the membership test, so a schedule naming a player who is not in
    /// the game fails as `UnknownBot` rather than being silently renumbered
    /// onto whoever happens to be present.
    connected: BTreeSet<PlayerId>,
}

impl RconActuator {
    /// Reads the roster the game actually has, so `player` can reject a bot
    /// that is not in it. It does **not** renumber: see [`BotId`].
    pub async fn new(
        rcon: Arc<FactorioRcon>,
        world: Arc<FactorioWorld>,
    ) -> Result<Self, ActuatorError> {
        let connected: BTreeSet<PlayerId> = rcon
            .connected_players()
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))?
            .into_iter()
            .map(|p| p.player_id)
            .collect();
        if connected.is_empty() {
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
            connected,
        })
    }

    /// The Factorio player a `BotId` names: itself.
    ///
    /// The identity is the whole point — see [`BotId`]. All this adds is the
    /// membership check, and it is a free function over the roster so the seam
    /// between "who the scheduler assigns work to" and "who the executor
    /// drives" can be exercised without a running game.
    pub fn resolve_player(
        connected: &BTreeSet<PlayerId>,
        bot: BotId,
    ) -> Result<PlayerId, ActuatorError> {
        let player: PlayerId = bot.0;
        if connected.contains(&player) {
            Ok(player)
        } else {
            Err(ActuatorError::UnknownBot(bot))
        }
    }

    fn player(&self, bot: BotId) -> Result<PlayerId, ActuatorError> {
        Self::resolve_player(&self.connected, bot)
    }
}

/// # What this implementation cannot yet report, and why
///
/// Two facts the [`Actuator`] contract now has room for are **not** available
/// here, and both are lost inside `crates/core` before this crate is reached.
/// They are stated rather than approximated, because guessing either one would
/// be exactly the fabrication the contract exists to prevent.
///
/// - **A dispatch tick on the failure path.** `FactorioRcon`'s `*_timed`
///   methods are shaped `let dispatched = action_start_…().await?; let replied
///   = sleep_for_action_result(…).await?;`, so when the *second* call fails the
///   first call's tick — a real stamp the game produced — is dropped by the
///   `?`. Every failure therefore arrives here with nothing attached and is
///   reported as [`ActionTicks::UNKNOWN`]. Making it available needs those
///   methods to carry the dispatch tick out on their error path; until then
///   `UNKNOWN` is the honest answer this crate can give, and
///   [`ActuatorFailure`] is where the tick will go the moment core surfaces it.
/// - **[`ActuatorError::NoVerdict`].** A `sleep_for_action_result` timeout is
///   the executor-visible shape of an `action_completed` whose status the
///   parser could not read, and it is genuinely "no verdict". But the same
///   `RconTimeout` also comes back from a *path request* and from the inner
///   `move_player` a mine may make first — failures where nothing was
///   dispatched for this action at all. Classifying by error type alone would
///   report an action as `Lost` ("it may have happened") when the game never
///   saw it, which overclaims in the direction that matters most. So every
///   failure here stays `Rejected` until core distinguishes the two at the
///   point where it knows the difference.
#[async_trait]
impl Actuator for RconActuator {
    async fn walk(&self, bot: BotId, to: Position) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        self.rcon
            .move_player_timed(&self.world, p, &to, None)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()).into())
    }

    async fn mine(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        self.rcon
            .player_mine_timed(&self.world, p, item, &at, count)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()).into())
    }

    async fn craft(
        &self,
        bot: BotId,
        recipe: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        self.rcon
            .player_craft_timed(&self.world, p, recipe, count)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()).into())
    }

    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        // `place_entity` returns the created FactorioEntity; the executor does
        // not need it, because the plan already knows what it placed and the
        // world snapshot is refreshed by the event stream, not by this reply.
        // The ticks it also returns are the point of the `_timed` variant.
        self.rcon
            .place_entity_timed(p, item.to_string(), at, direction, &self.world)
            .await
            .map(|(_entity, ticks)| ticks)
            .map_err(|e| ActuatorError::Rejected(e.to_string()).into())
    }

    async fn insert(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        let inv = self.defines.get(slot)?;
        self.rcon
            .insert_to_inventory_timed(
                p,
                entity.to_string(),
                at,
                inv,
                item.to_string(),
                count,
                &self.world,
            )
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()).into())
    }

    async fn remove(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        let inv = self.defines.get(slot)?;
        self.rcon
            .remove_from_inventory_timed(
                p,
                entity.to_string(),
                at,
                inv,
                item.to_string(),
                count,
                &self.world,
            )
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()).into())
    }

    /// Research is server-wide: `add_research` takes no player id, so `bot`
    /// does not appear here. Two bots researching the same technology is
    /// idempotent in Factorio.
    async fn research(&self, tech: &str) -> Result<ActionTicks, ActuatorFailure> {
        self.rcon
            .add_research_timed(tech)
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()).into())
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
    fn a_bot_id_is_the_player_id_and_is_never_renumbered() {
        // A gappy roster is the case that tells identity apart from any
        // position-based numbering: under `enumerate` these would come out as
        // players 2, 5 and 7 for bots 0, 1 and 2, and bot 7 would not exist.
        let connected: BTreeSet<PlayerId> = [2, 5, 7].into_iter().collect();
        for id in [2u8, 5, 7] {
            assert_eq!(
                RconActuator::resolve_player(&connected, BotId(id)).unwrap(),
                id,
                "bot {id} must drive player {id}"
            );
        }
    }

    #[test]
    fn a_bot_whose_player_is_not_in_the_game_is_rejected_not_substituted() {
        let connected: BTreeSet<PlayerId> = [2, 5, 7].into_iter().collect();
        for absent in [0u8, 1, 3, 8] {
            assert!(
                matches!(
                    RconActuator::resolve_player(&connected, BotId(absent)),
                    Err(ActuatorError::UnknownBot(BotId(b))) if b == absent
                ),
                "player {absent} is not connected, so bot {absent} must be unknown"
            );
        }
    }

    #[test]
    fn no_connected_players_means_every_bot_is_unknown() {
        let connected: BTreeSet<PlayerId> = BTreeSet::new();
        assert!(RconActuator::resolve_player(&connected, BotId(1)).is_err());
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
