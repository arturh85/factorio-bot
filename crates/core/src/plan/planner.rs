#[cfg_attr(test, mockall_double::double)]
use crate::factorio::rcon::FactorioRcon;
use crate::factorio::world::FactorioWorld;
use crate::types::{EntityName, PlayerChangedMainInventoryEvent};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The Lua runtime's context holder: rcon, and the world scripts query.
///
/// Not a planner. The task-graph planner it was named for was deleted and goal
/// decomposition now lives in `crates/planner`, which is pure and simulates
/// forward through its own `PlanState` overlay. The name survives because
/// `run_lua` takes one.
///
/// # There is one world
///
/// `real_world` and `plan_world` were once two: `plan_world` was a **deep
/// copy** of `real_world` that the old task-graph planner mutated as it
/// simulated, so that a hypothetical never leaked into reality. When that
/// planner went, its writes went with it -- and nothing replaced them. What
/// was left was a copy that no code ever wrote to, taken once per run, and
/// handed to every `world.*` Lua binding for the run's whole duration.
///
/// That is not staleness, it is a freeze. `OutputParser` keeps `real_world`
/// current through the run; the copy never moved. Measured live: a script read
/// its bot at `(0, 0)` while the game had it at `(-22.29, 35.34)`, 41.8 tiles
/// and some 6,200 ticks out, and the ten iron plates the run had just smelted
/// were invisible -- which made a goal that had genuinely succeeded look like
/// it had produced nothing.
///
/// So the copy is gone: `plan_world` is now the same `Arc` as `real_world`.
/// Collapsing a simulation surface into a truth surface would be a real
/// mistake, but there was no simulation surface left to collapse -- the search
/// for a writer of `plan_world` found none outside this file. The field and
/// [`Planner::update_plan_world`] are kept only because `lua_runner` still
/// names them (see the tests below, which pin the aliasing so the deep copy
/// cannot creep back); they are a rename away from being retired.
pub struct Planner {
    #[allow(dead_code)]
    pub rcon: Option<Arc<FactorioRcon>>,
    pub real_world: Arc<FactorioWorld>,
    /// The same world as [`Planner::real_world`], not a copy of it. See the
    /// type's documentation for why this is a second name and not a second
    /// world.
    pub plan_world: Arc<FactorioWorld>,
}

impl Planner {
    pub fn new(world: Arc<FactorioWorld>, rcon: Option<Arc<FactorioRcon>>) -> Planner {
        Planner {
            rcon,
            plan_world: world.clone(),
            real_world: world,
        }
    }

    /// Kept for `lua_runner`, which calls it after seeding the run's players.
    ///
    /// It is a no-op now: there is nothing to refresh into. It stays so the
    /// caller keeps compiling, and is documented as pointless rather than
    /// quietly removed, because the shape it had -- "replace the field with a
    /// fresh copy" -- is what made the freeze survivable in the first place.
    /// A refresh that replaces a field cannot reach bindings that already
    /// closed over the old handle.
    pub fn update_plan_world(&mut self) {
        self.plan_world = self.real_world.clone();
    }

    /// The world to query. Live, and the only one.
    pub fn world(&self) -> Arc<FactorioWorld> {
        self.real_world.clone()
    }

    pub fn initiate_missing_players_with_default_inventory(&mut self, bot_count: u8) -> Vec<u8> {
        let mut player_ids: Vec<u8> = vec![];
        for player_id in 1u8..=bot_count {
            player_ids.push(player_id);
            // initialize missing players with default inventory
            if self.real_world.players.get(&player_id).is_none() {
                let mut main_inventory: BTreeMap<String, u32> = BTreeMap::new();
                main_inventory.insert(EntityName::Wood.to_string(), 1);
                main_inventory.insert(EntityName::StoneFurnace.to_string(), 1);
                main_inventory.insert(EntityName::BurnerMiningDrill.to_string(), 1);
                self.real_world
                    .player_changed_main_inventory(PlayerChangedMainInventoryEvent::from_btreemap(
                        player_id,
                        main_inventory,
                    ))
                    .expect("failed to set player inventory");
            }
        }
        player_ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PlayerChangedPositionEvent, Position};

    fn moved_to(player_id: u8, x: f64, y: f64) -> PlayerChangedPositionEvent {
        PlayerChangedPositionEvent {
            player_id,
            position: Position::new(x, y),
        }
    }

    /// The world the Lua bindings are handed must be the running game, not a
    /// photograph of it.
    ///
    /// `lua_runner` clones `planner.plan_world` **once**, before the chunk
    /// runs, and every `world.*` binding closes over that clone for the whole
    /// script. While `plan_world` was a deep copy, the output parser's writes
    /// landed in `real_world` and the script kept reading the start state --
    /// live, a bot at `(-22.29, 35.34)` still read as `(0, 0)`, 41.8 tiles
    /// out, and the ten iron plates it was holding were invisible.
    ///
    /// The two reads below bracket the mutation deliberately: the first pins
    /// the start state, so a binding that simply always answered the *new*
    /// value would fail here rather than sail through the second assertion.
    #[test]
    fn the_world_handed_to_the_bindings_tracks_the_real_world() {
        let world = Arc::new(FactorioWorld::new());
        world
            .player_changed_position(moved_to(1, 0., 0.))
            .expect("seed the player");

        let mut planner = Planner::new(world.clone(), None);
        planner.initiate_missing_players_with_default_inventory(1);
        // Exactly what `lua_runner` does: refresh, then take one handle that
        // outlives every binding built from it.
        planner.update_plan_world();
        let bound = planner.plan_world.clone();

        assert_eq!(
            bound.players.get(&1).expect("seeded player").position,
            Position::new(0., 0.),
            "precondition: the handle starts out agreeing with the world"
        );

        // The game moves the bot and hands it plates; `OutputParser` writes
        // both into `real_world` while the script is still running.
        world
            .player_changed_position(moved_to(1, -22.29, 35.34))
            .expect("move the player");
        let mut inventory: BTreeMap<String, u32> = BTreeMap::new();
        inventory.insert("iron-plate".to_owned(), 10);
        world
            .player_changed_main_inventory(PlayerChangedMainInventoryEvent::from_btreemap(
                1, inventory,
            ))
            .expect("give the player plates");

        let player = bound.players.get(&1).expect("player still present");
        assert_eq!(
            player.position,
            Position::new(-22.29, 35.34),
            "the bindings' world is frozen: it still reports the pre-run position"
        );
        assert_eq!(
            player.main_inventory.get("iron-plate").copied(),
            Some(10),
            "the bindings' world is frozen: the plates the run produced are invisible"
        );
    }

    /// `update_plan_world` must not hand out a *new* handle that the already
    /// built bindings would not be holding.
    ///
    /// This is the second half of the same trap: even when the deep copy was
    /// refreshed, the refresh replaced the field, while the bindings still
    /// closed over the Arc taken before it. Anything the planner exposes has
    /// to be the one world, so that a refresh is a no-op rather than a
    /// divergence.
    #[test]
    fn refreshing_does_not_orphan_a_handle_taken_earlier() {
        let world = Arc::new(FactorioWorld::new());
        let mut planner = Planner::new(world.clone(), None);
        let taken_early = planner.world();

        planner.update_plan_world();
        world
            .player_changed_position(moved_to(1, 7., 9.))
            .expect("seed the player");

        assert_eq!(
            taken_early.players.get(&1).expect("player").position,
            Position::new(7., 9.),
            "a handle taken before the refresh was orphaned by it"
        );
        assert!(
            Arc::ptr_eq(&taken_early, &planner.world()),
            "the planner is handing out more than one world"
        );
    }
}
