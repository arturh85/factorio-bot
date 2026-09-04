use crate::actuator::{ActionTicks, Actuator, ActuatorError, ActuatorFailure};
use crate::pre_place::{PrePlace, STEP_ASIDE_RADIUS, judge_placement};
use crate::walk_memory::{note_walk_refusal, pathfinder_found_nothing};
use async_trait::async_trait;
use factorio_bot_core::constants::BOT_FORCE;
use factorio_bot_core::factorio::rcon::{
    ActionFailure, DestinationFull, Dispatch, FactorioRcon, approach_annulus,
};
use factorio_bot_core::factorio::world::{FactorioWorld, StepAside};
use factorio_bot_core::record::map::{EntitySnapshot, Placement, drift_between};
use factorio_bot_core::types::{PlayerId, Position};
use factorio_bot_planner::{BotId, InventorySlot};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

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

/// Whether `world` records `tech` as researched for the force the bots act for.
///
/// Three answers, and the third is the interesting one:
///
/// - `Some(true)` / `Some(false)` — the force is in the world and so is the
///   technology, so this is a report about the game.
/// - `None` — **nobody can say**. Either no force has been read yet (the world
///   is populated asynchronously from the mod's stdout) or the force has no
///   such technology. Neither is evidence that it is unresearched, and a caller
///   that waits for a `true` must not wait on this.
///
/// A free function rather than a method so it can be tested against a
/// hand-built [`FactorioWorld`]; [`RconActuator`] itself needs a live
/// [`FactorioRcon`] to exist.
pub fn technology_researched_in(world: &FactorioWorld, tech: &str) -> Option<bool> {
    let force = world.forces.get(BOT_FORCE)?;
    force.technologies.get(tech).map(|t| t.researched)
}

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
    /// The placement each bot most recently made, waiting to be claimed by
    /// [`Actuator::take_placement`].
    ///
    /// `Actuator::place` carries no `ActionId` — that belongs to the
    /// scheduler, not to this trait — so there is nothing here to attach a
    /// `Placement` to directly. Keyed by bot rather than a single slot so two
    /// bots placing concurrently cannot clobber each other's fact; the trait
    /// method that reads this removes the entry it returns, so `run.rs`'s
    /// settle path, which does have the `ActionId`, claims each placement
    /// exactly once rather than risking a stale one landing on a later,
    /// unrelated attempt.
    placements: Mutex<BTreeMap<PlayerId, Placement>>,
    /// The reason each bot's last `insert` succeeded without delivering
    /// everything, waiting to be claimed by
    /// [`Actuator::take_destination_full`].
    ///
    /// A second `placements`, keyed and drained the same way and for the same
    /// reason: this trait's `insert` has no `ActionId`, and the fact belongs to
    /// exactly one attempt. Overwritten rather than accumulated -- a bot runs
    /// its own steps strictly in order, so the previous entry has always been
    /// claimed by the time a second insert finishes.
    destinations_full: Mutex<BTreeMap<PlayerId, DestinationFull>>,
}

impl RconActuator {
    /// Reads the roster the game actually has, so `player` can reject a bot
    /// that is not in it. It does **not** renumber: see [`BotId`].
    pub async fn new(
        rcon: Arc<FactorioRcon>,
        world: Arc<FactorioWorld>,
    ) -> Result<Self, ActuatorError> {
        let mut connected: BTreeSet<PlayerId> = rcon
            .connected_players()
            .await
            .map_err(|e| ActuatorError::Rejected(e.to_string()))?
            .into_iter()
            .map(|p| p.player_id)
            .collect();
        // Plus every player the world has heard from. `connected_players()`
        // is the mod's `rcon_players`, which lists only players **with a
        // character** -- so a bot that is dead at the moment a run starts is
        // absent from it, every one of its actions would fail here as
        // `UnknownBot` ("has no mapped Factorio player"), and the mod, the
        // one party that knows *why*, would never be asked. `world.players`
        // holds every player that has reported a position and loses one on
        // `on_player_left_game`, which is exactly "connected, with or without
        // a character". The membership check still refuses a bot the game
        // has never seen; a dead one goes through to `get_player`'s
        // `has no character` refusal and is classified as such.
        connected.extend(world.players.iter().map(|p| *p.key()));
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
            placements: Mutex::new(BTreeMap::new()),
            destinations_full: Mutex::new(BTreeMap::new()),
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

/// Translates a [`FactorioRcon`] dispatch failure into the executor's own.
///
/// # Why this is a `match` on a phase and not on an error kind
///
/// The question the log needs answered is "is there an action still out there
/// whose outcome this run will not learn" — [`crate::Status::Lost`]. The
/// tempting answer is "the error was a timeout", and it is wrong: a timeout is
/// also what a *path request* produces, and what the inner `move_player` a mine
/// may make first produces, and in both of those nothing was ever dispatched
/// for this action. Marking those `Lost` would report a bot as having lost
/// track of work it never started — a fabrication in the worst direction, and
/// the reason this crate reported every failure as `Rejected` until now.
///
/// So the classification is not made here at all. [`Dispatch`] is recorded in
/// `crates/core` at the statement that knows: `NoVerdict` is produced by
/// exactly one place, `sleep_for_action_result` running out of patience, which
/// is only ever entered *after* an `action_start_*` returned — i.e. after the
/// game itself answered the dispatch RPC. Everything weaker, including an RCON
/// round trip that failed after the bytes may already have gone out, is
/// `NotDispatched` and stays `Rejected`. That under-claims rather than over-,
/// which is the direction chosen deliberately.
///
/// `f.ticks` is passed through untouched, which is the other half: a dispatch
/// the game stamped and then refused arrives here with a real tick and keeps
/// it, while a failure raised before any stamp arrives with
/// [`ActionTicks::UNKNOWN`] and keeps that.
pub fn classify(f: ActionFailure) -> ActuatorFailure {
    let message = f.error.to_string();
    match f.dispatch {
        // The game took the command and never gave a readable verdict. Nothing
        // is known about the outcome, which is not the same as knowing it
        // failed.
        Dispatch::NoVerdict => ActuatorError::NoVerdict(message).at(f.ticks),
        // Either the game judged it and said no, or it never saw it. Both are
        // `Rejected`: neither leaves an action outstanding.
        Dispatch::Refused | Dispatch::NotDispatched => ActuatorError::Rejected(message).at(f.ticks),
    }
}

/// # What this implementation can and cannot report
///
/// Both facts the [`Actuator`] contract has room for are now produced here,
/// because `crates/core` carries them out of its `*_timed` methods on the error
/// path rather than dropping them: [`ActionFailure`] holds the ticks the game
/// stamped and the [`Dispatch`] phase the failure happened in, and [`classify`]
/// is the whole of the translation. See its docs for why the phase, not the
/// error kind, is what decides [`ActuatorError::NoVerdict`].
///
/// One thing is still deliberately not claimed. When the RCON round trip itself
/// fails — a broken pipe on a command that may already have been written — the
/// game may or may not have run it, and nothing distinguishes a failed connect
/// from a failed read. That comes back as [`Dispatch::NotDispatched`] and so as
/// `Rejected`, which under-claims: an action the game did run can be reported
/// as one it never saw. Fixing it needs evidence that does not exist at that
/// layer (an idempotency key the mod could echo back, or a post-hoc query of
/// the action's own state), and inventing a `Lost` there would put a bot's
/// outstanding work into the log on the strength of a guess.
#[async_trait]
impl Actuator for RconActuator {
    async fn walk(
        &self,
        bot: BotId,
        to: Position,
        min_radius: f64,
        radius: f64,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        // `None` here — Factorio's own default radius of 1 — is what turned
        // "stand within 10 tiles of the furnace" into "stand on the furnace",
        // and the pathfinder answered with a substituted goal 9.3 tiles from
        // where the plan believed the bot would be. The plan's *whole*
        // condition is passed instead: `approach_annulus` shrinks a plain disc
        // exactly as `approach_radius` always did, and turns an annulus into a
        // goal-and-radius that fits inside it, so a request can no longer be
        // satisfied by standing on ground the plan called forbidden.
        //
        // Where the bot is now is an input, not decoration: it decides which
        // side of the target to aim at, and it is a fact this layer has and
        // the planner does not.
        let here = self
            .world
            .players
            .get(&p)
            .map(|player| player.position.clone());
        let (goal, slack) = approach_annulus(&to, min_radius, radius, here.as_ref());
        self.rcon
            .move_player_timed(&self.world, p, &goal, Some(slack))
            .await
            .map_err(|failure| {
                // The one layer that holds both halves of what the game just
                // answered: the destination the plan named, and where the
                // character was standing when it asked. `run.rs` has the
                // first and not the second; `crates/core` has the second and
                // not the first. See `walk_memory` for what is remembered and
                // what is deliberately not.
                note_walk_refusal(&self.world, p, here.as_ref(), &to, &failure);
                // And the record gets both halves too. A pre-dispatch
                // refusal carries no position of its own -- the mod answers
                // `failed to path find` and nothing else -- so the run
                // record used to archive it as `no_path` from nowhere to
                // nowhere, which cannot be analysed: run
                // `run-1788552801-73005`'s three refusals from one spot
                // read as three unrelated failures. The wording is the one
                // `classify_walk_failure`'s `walk_endpoints` already reads
                // (`found no path from (x/y) to (x/y)`); `from` is the
                // world's reading of the character, `to` is the goal the
                // game was actually asked for, which `approach_annulus` has
                // moved off the plan's `to` by the annulus.
                let refused_from = here
                    .as_ref()
                    .filter(|_| pathfinder_found_nothing(&failure.error))
                    .cloned();
                let failure = classify(failure);
                match (refused_from, failure.error) {
                    (Some(from), ActuatorError::Rejected(message)) => ActuatorFailure {
                        error: ActuatorError::Rejected(format!(
                            "{message} -- found no path from ({}/{}) to ({}/{})",
                            from.x(),
                            from.y(),
                            goal.x(),
                            goal.y()
                        )),
                        ticks: failure.ticks,
                    },
                    (_, error) => ActuatorFailure {
                        error,
                        ticks: failure.ticks,
                    },
                }
            })
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
            .map_err(classify)
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
            .map_err(classify)
    }

    async fn place(
        &self,
        bot: BotId,
        item: &str,
        at: Position,
        direction: u8,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        // Before anything is built: would this footprint seal in the
        // character about to build it? See `pre_place` for the run that
        // answered yes, and for why this layer and no other can ask.
        match judge_placement(&self.world, p, item, &at, direction) {
            PrePlace::Proceed => {}
            PrePlace::StepAside {
                from,
                to,
                pocket_tiles,
            } => {
                let ticks = self
                    .rcon
                    .move_player_timed(&self.world, p, &to, Some(STEP_ASIDE_RADIUS))
                    .await
                    .map_err(|failure| {
                        let failure = classify(failure);
                        ActuatorFailure {
                            error: ActuatorError::Rejected(format!(
                                "stepping aside to {to} before placing {item} at {at} -- from \
                                 {from} the placement would wall the character into \
                                 {pocket_tiles} tiles -- and the walk failed: {}",
                                failure.error
                            )),
                            ticks: failure.ticks,
                        }
                    })?;
                self.world.record_step_aside(StepAside {
                    tick: ticks.replied,
                    player: p,
                    from,
                    to,
                    placing: item.to_string(),
                    site: at.clone(),
                    pocket_tiles,
                });
            }
            PrePlace::Refuse { from, pocket_tiles } => {
                return Err(ActuatorError::Rejected(format!(
                    "refused to place {item} at {at}: from {from} it would wall the \
                     placing character into {pocket_tiles} tiles, and no reachable tile \
                     within building reach stays open once it stands"
                ))
                .at(ActionTicks::UNKNOWN));
            }
        }
        // `place_entity` returns the FactorioEntity the game actually created,
        // which is the truth half of a placement; `item`/`at`/`direction` are
        // the intent half, captured before `at` is moved into the call below.
        // Recording both, and their drift, is why this call site exists: see
        // `Attempt::placed` (crates/executor/src/log.rs) and
        // `factorio_bot_core::record::map`.
        let intent = EntitySnapshot {
            name: item.to_string(),
            position: at.clone(),
            direction,
        };
        let (entity, ticks) = self
            .rcon
            .place_entity_timed(p, item.to_string(), at, direction, &self.world)
            .await
            .map_err(classify)?;
        let actual = EntitySnapshot {
            name: entity.name,
            position: entity.position,
            direction: entity.direction,
        };
        let drift = drift_between(&intent, &actual);
        self.placements
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                p,
                Placement {
                    intent,
                    actual,
                    drift,
                },
            );
        Ok(ticks)
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
        // `TransferOutcome::destination_full` is `Some` when the game accepted
        // fewer than were offered *because the destination had no room left*.
        // That is a success -- see `judge_transfer_reply` -- but it is not a
        // full delivery, and the difference is the only evidence a run record
        // carries that the plan asked for more than the world could hold. It
        // is parked here for `run.rs`'s settle path to claim, exactly as a
        // placement is, because this method has no `ActionId` to attach it to.
        let outcome = self
            .rcon
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
            .map_err(classify)?;
        if let Some(full) = outcome.destination_full {
            self.destinations_full
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(p, full);
        }
        Ok(outcome.ticks)
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
            .map_err(classify)
    }

    /// Research is server-wide: the mod's research entry takes no player id, so
    /// `bot` does not appear here. Two bots researching the same technology is
    /// idempotent in Factorio.
    ///
    /// **Durative.** This returns when the game raises `on_research_finished`,
    /// not when the technology is queued. It used to be the latter, which meant
    /// success was reported the instant the research was *requested* and every
    /// step that depended on the technology ran against a belief nothing
    /// established. See [`FactorioRcon::research_timed`].
    async fn research(
        &self,
        tech: &str,
        expected_ticks: u32,
    ) -> Result<ActionTicks, ActuatorFailure> {
        self.rcon
            .research_timed(&self.world, tech, expected_ticks)
            .await
            .map_err(classify)
    }

    async fn set_recipe(
        &self,
        bot: BotId,
        entity: &str,
        at: Position,
        recipe: &str,
    ) -> Result<ActionTicks, ActuatorFailure> {
        let p = self.player(bot)?;
        // No `InventoryDefines` lookup and no `Placement` to record: this
        // names a recipe, not a slot, and it builds nothing. The whole of the
        // work -- the reach check, the walk if the machine is out of reach,
        // and the judgement of the reply -- is `set_recipe_timed`'s, exactly
        // as `craft` and `research` delegate theirs.
        self.rcon
            .set_recipe_timed(p, entity.to_string(), at, recipe.to_string(), &self.world)
            .await
            .map_err(classify)
    }

    /// Read from the world, not asked over RCON.
    ///
    /// The mod already tells us: `on_research_finished` calls
    /// `writeout_recipes()` and `writeout_forces()` *before* it settles the
    /// action, so by the time a technology has landed, `OutputParser` has
    /// already written the new force into this world. A round trip would ask a
    /// question the answer to which is sitting in memory, and the caller
    /// (`run::await_research`) asks repeatedly.
    async fn technology_researched(&self, tech: &str) -> Result<Option<bool>, ActuatorError> {
        Ok(technology_researched_in(&self.world, tech))
    }

    /// The game's own clock, asked fresh.
    ///
    /// Not [`FactorioRcon::last_tick`]: that is the stamp off whatever was
    /// last sent, and the whole point of the caller
    /// (`run::wait_out_lag`) is that nothing is being sent while a furnace
    /// works. A stale tick there would report the wait finished the moment it
    /// began.
    ///
    /// A round trip that fails is reported as `Ok(None)` rather than as an
    /// error: "I could not read the clock" and "I have no clock" put the
    /// caller in exactly the same position, and the caller's answer to both —
    /// fall back to the wall-clock estimate — is right for both. Failing the
    /// wait outright over an unreadable clock would abandon a plan for a
    /// reason that has nothing to do with the plan.
    async fn game_tick(&self) -> Result<Option<u64>, ActuatorError> {
        Ok(self.rcon.game_tick().await.ok().flatten())
    }

    /// Claims the placement `bot` most recently made, if one is waiting.
    ///
    /// Removes it: a placement is a fact about one attempt, and leaving it in
    /// place would let a later, unrelated attempt read the same fact. See the
    /// `placements` field doc for why the cache exists; `run.rs`'s settle path
    /// is the caller, with the `ActionId` this trait cannot see, right after
    /// marking that action's `Attempt` done.
    fn take_placement(&self, bot: BotId) -> Option<Placement> {
        self.placements
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&bot.0)
    }

    /// Claims the full-destination fact `bot`'s last insert produced, if one is
    /// waiting. Removes it, for the same reason `take_placement` does: it is a
    /// fact about one attempt, and leaving it would let a later, unrelated
    /// insert be recorded as having filled something.
    fn take_destination_full(&self, bot: BotId) -> Option<DestinationFull> {
        self.destinations_full
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&bot.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::errors::{RconError, RconPlayerNotFound, RconTimeout};
    use std::collections::BTreeSet;

    /// A force carrying exactly the technologies named, with the researched
    /// flag each pair gives.
    fn force_with(name: &str, techs: &[(&str, bool)]) -> factorio_bot_core::types::FactorioForce {
        use factorio_bot_core::types::{FactorioForce, FactorioTechnology};
        let technologies = techs
            .iter()
            .map(|(tech, researched)| {
                (
                    (*tech).to_string(),
                    FactorioTechnology {
                        name: (*tech).to_string(),
                        enabled: true,
                        upgrade: false,
                        researched: *researched,
                        prerequisites: None,
                        research_unit_ingredients: Vec::new(),
                        research_unit_count: 0,
                        research_unit_energy: Default::default(),
                        order: String::new(),
                        level: 1,
                        valid: true,
                        unlocked_recipes: Vec::new(),
                        research_trigger: None,
                    },
                )
            })
            .collect();
        FactorioForce {
            name: name.to_string(),
            force_id: 1,
            current_research: None,
            research_progress: None,
            manual_mining_speed_modifier: None,
            technologies: Box::new(technologies),
        }
    }

    #[test]
    fn the_bots_force_answers_for_a_technology_it_has_finished() {
        let world = FactorioWorld::new();
        world
            .update_force(force_with("player", &[("automation-science-pack", true)]))
            .expect("force accepted");
        assert_eq!(
            technology_researched_in(&world, "automation-science-pack"),
            Some(true)
        );
    }

    #[test]
    fn the_other_forces_in_the_world_do_not_get_a_vote() {
        // `writeout_forces` emits all of `game.forces`, so from the first
        // `on_research_finished` of a run the world holds `enemy` and
        // `neutral` too -- and neither ever researches anything. Sorting the
        // map and taking the first name (which is what
        // `PlanState::from_world` does) picks `enemy` and answers "not
        // researched" forever. This is the test that would have caught it.
        let world = FactorioWorld::new();
        world
            .update_force(force_with("player", &[("automation-science-pack", true)]))
            .expect("force accepted");
        world
            .update_force(force_with("enemy", &[("automation-science-pack", false)]))
            .expect("force accepted");
        world
            .update_force(force_with("neutral", &[("automation-science-pack", false)]))
            .expect("force accepted");
        assert_eq!(
            technology_researched_in(&world, "automation-science-pack"),
            Some(true),
            "the bots act for `player`; `enemy` sorts first and must not decide"
        );
    }

    #[test]
    fn a_world_that_has_read_no_force_yet_cannot_answer() {
        let world = FactorioWorld::new();
        assert_eq!(
            technology_researched_in(&world, "automation-science-pack"),
            None
        );
    }

    #[test]
    fn a_technology_the_force_does_not_list_is_unknown_rather_than_unresearched() {
        // `Some(false)` here would make a caller wait out its whole budget for
        // a technology the game has never heard of.
        let world = FactorioWorld::new();
        world
            .update_force(force_with("player", &[("electronics", true)]))
            .expect("force accepted");
        assert_eq!(technology_researched_in(&world, "no-such-technology"), None);
    }

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

    /// Fact 2, the direction that needs the state: the game acknowledged the
    /// command and never answered, so the outcome is unknown rather than bad.
    /// `NoVerdict` is what `run.rs` turns into [`crate::Status::Lost`].
    #[test]
    fn a_dispatch_the_game_never_answered_is_reported_as_no_verdict() {
        let f = classify(ActionFailure::no_verdict(
            RconTimeout {}.into(),
            ActionTicks::new(Some(7_777), None),
        ));
        assert!(
            matches!(f.error, ActuatorError::NoVerdict(_)),
            "a dispatched action with no answer is Lost, not Failed; got {:?}",
            f.error
        );
        assert_eq!(
            f.ticks.dispatched,
            Some(7_777),
            "the dispatch stamp is the evidence that there is an action to have lost"
        );
    }

    /// Fact 2, the direction that would overclaim — and the whole reason the
    /// phase is carried instead of being guessed from the error. This is the
    /// *same* `RconTimeout` as above; only the phase differs, and it must come
    /// out `Rejected`, because reporting an action the game never saw as `Lost`
    /// puts work into the log that no bot ever started.
    #[test]
    fn a_timeout_before_any_dispatch_is_not_reported_as_no_verdict() {
        let f = classify(ActionFailure::not_dispatched(RconTimeout {}.into()));
        assert!(
            matches!(f.error, ActuatorError::Rejected(_)),
            "nothing was dispatched, so nothing can be lost; got {:?}",
            f.error
        );
        assert!(
            !matches!(f.error, ActuatorError::NoVerdict(_)),
            "a path request that timed out must never be rendered as an outstanding action"
        );
    }

    /// Fact 1, the direction that used to drop a measurement.
    #[test]
    fn a_refused_dispatch_arrives_with_the_tick_the_game_stamped() {
        let f = classify(ActionFailure::refused(
            RconError {
                message: "out of reach".to_string(),
            }
            .into(),
            ActionTicks::new(Some(4_211), Some(4_270)),
        ));
        assert!(matches!(f.error, ActuatorError::Rejected(_)));
        assert_eq!(f.ticks, ActionTicks::new(Some(4_211), Some(4_270)));
        assert!(
            f.to_string().contains("out of reach"),
            "the game's own message must survive; got {f}"
        );
    }

    /// Fact 1, the direction that would fabricate: nothing stamped it, so
    /// nothing may be reported — and absent is not tick zero.
    #[test]
    fn a_failure_before_any_stamp_still_reports_no_tick() {
        let f = classify(ActionFailure::not_dispatched(
            RconPlayerNotFound { player_id: 3 }.into(),
        ));
        assert_eq!(f.ticks, ActionTicks::UNKNOWN);
        assert_eq!(f.ticks.dispatched, None);
        assert_ne!(f.ticks.dispatched, Some(0), "absent is not tick zero");
    }

    /// The production trait impl, not just the free function: a real
    /// [`RconActuator`] whose rcon has no connection fails inside
    /// `player_path`, which is the pre-dispatch phase, and the answer must come
    /// back `Rejected` with nothing attached.
    ///
    /// Built field-by-field rather than through [`RconActuator::new`], which
    /// needs a live game to read the roster and the defines table.
    #[test]
    fn the_walk_dispatch_reports_a_pre_dispatch_failure_as_rejected_and_untimed() {
        let actuator = RconActuator {
            rcon: Arc::new(FactorioRcon::new_empty()),
            world: Arc::new(FactorioWorld::new()),
            defines: InventoryDefines::default(),
            connected: [1u8].into_iter().collect(),
            placements: Mutex::new(BTreeMap::new()),
            destinations_full: Mutex::new(BTreeMap::new()),
        };
        let f = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(actuator.walk(BotId(1), Position::new(10.0, 10.0), 0.0, 3.0))
            .expect_err("a disconnected rcon cannot request a path");
        assert!(
            matches!(f.error, ActuatorError::Rejected(_)),
            "the game never saw this walk, so it is not Lost; got {:?}",
            f.error
        );
        assert_eq!(
            f.ticks,
            ActionTicks::UNKNOWN,
            "nothing stamped this, so nothing may be reported"
        );
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
