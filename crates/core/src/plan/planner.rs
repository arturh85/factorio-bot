#[cfg_attr(test, mockall_double::double)]
use crate::factorio::rcon::FactorioRcon;
use crate::factorio::world::FactorioWorld;
use crate::types::{EntityName, PlayerChangedMainInventoryEvent, Pos, Position, RequestEntity};
use miette::Result;
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
/// The entity names a plan may take materials back out of.
///
/// # Why a whitelist, and why it is here rather than in the planner
///
/// This is the answer to "what counts as a buffer", and it is the whole of it:
/// `crates/planner` believes whatever contents this world holds, so the set of
/// entities the game is ever *asked* about is the set the planner can ever
/// withdraw from. One list, in the code that issues the query.
///
/// It is not "every container in the world", which would be more useful and
/// would also invite the roster to empty a chest a person put there on
/// purpose. It is not "containers this plan filled" either -- that is
/// unimplementable across the boundary that matters, because a replan builds a
/// fresh `PlanState` with no memory of the plan before it, and the stranded
/// items this exists to recover are stranded by exactly that discontinuity.
///
/// What it is: **the entities this planner builds and unloads itself**. Two,
/// and each is here for a stated reason:
///
/// * `stone-furnace`, because `method::have::smelt_steps` places one and takes
///   items back out of it. A furnace's *result* slot is also the least
///   ambiguous inventory in the game to help yourself from: nobody stores
///   things there, so anything in it was smelted by whoever's plan put the ore
///   in.
/// * `wooden-chest`, because `method::have::Stockpile` places one per resource
///   patch and has the roster fill it. Without this line a replan is blind to
///   every chest the previous plan built, which is precisely the discontinuity
///   the paragraph above says this list exists to bridge -- the ore those
///   items came from is gone from the ground, so a plan that cannot see them
///   plans to mine it again.
///
/// **Not `iron-chest`**, which an earlier draft of this comment expected. The
/// chest handover landed on wooden chests instead: two wood off one dead tree
/// against eight iron plates, measured on the reference map at 372 planned
/// ticks against 2,965. That draft closed with "add it the day something
/// places one", and something now does -- so this is the answer to that,
/// written down so nobody has to derive it twice.
///
/// `method::assemble::plan_cell` places **three** iron chests per cell, and
/// **none of them is a store**. Read `Role`: `FeedChest(0..MAX_FEED)` "holds
/// one of the things the intermediate machine eats" and `SupplyChest` "holds
/// the ingredient nothing in the cell makes". All three are *inputs*, filled
/// by hand with `CELL_CHARGE_TICKS` worth of ingredients before the cell is
/// switched on; the cell's product never enters a chest at all, it sits in the
/// assembling machine's output slot, which `withdraw_slot` already reaches
/// under `assembling-machine`.
///
/// So adding the name here would not recover stranded items, it would let a
/// replan **drain a running cell** -- and quietly, because `plan_cell`'s own
/// doc records that nothing detects a cell running out ("After it runs out,
/// nothing detects it"), so the plan that emptied it would not put the charge
/// back. That is the whitelist's stated policy failing, not passing: the rule
/// is *entities this planner builds **and unloads itself***, and the planner
/// builds iron chests and never unloads one.
///
/// Making a cell's chests visible therefore needs something this list cannot
/// say. The list is keyed by entity name; the distinction that matters is
/// *what a particular chest is for*, which is a property of the tile and of
/// the plan that sited it. Withdrawing from a stage-2 chest is a separate
/// change with that distinction in it, not a third string here.
///
/// A wooden chest is more ambiguous than a furnace result slot -- a person
/// could put something in one -- and that is accepted for the same reason the
/// whitelist is narrow: these are chests *the bots built*, sited by
/// `free_area_near` beside an ore patch, not chests found lying about.
pub const BUFFER_ENTITIES: [&str; 2] = ["stone-furnace", "wooden-chest"];

pub struct Planner {
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

    /// Ask the game what is standing in every buffer it knows about, and put
    /// the answers where the planner will read them.
    ///
    /// Answers how many entities were asked about, which is `0` when there is
    /// no RCON connection (the `--clients 0` planning mode) or when the world
    /// knows of no buffer yet.
    ///
    /// # Why this is a pull, and when to pull
    ///
    /// There is no event to subscribe to. Factorio raises none for "a chest's
    /// contents changed"; the mod's `on_some_entity_updated` fires only on
    /// `on_player_rotated_entity`, and `on_some_entity_created` describes a
    /// container at the instant it was built, which is an empty one. So
    /// contents are asked for, and the right moment to ask is **immediately
    /// before planning**: staleness is then bounded by the plan's own
    /// dispatch delay rather than by an event that may never come.
    ///
    /// One RCON round trip per call, naming only the entities in
    /// [`BUFFER_ENTITIES`] that the entity graph already knows about --
    /// typically a few dozen furnaces over a whole run.
    ///
    /// # Staleness is not eliminated, and is not pretended away
    ///
    /// A reading taken now is stale by the time a bot has walked there. The
    /// planner's overlay stops *this* plan double-counting a buffer
    /// (`PlanState::take_from_buffer`), and nothing can stop somebody else
    /// emptying it in the meantime. That case fails honestly, at the game and
    /// by an existing mechanism: `rcon_remove_from_inventory` complains when
    /// it moves fewer items than asked, `complain` writes into the RCON reply
    /// body, and `judge_transfer_reply` reads a non-empty body as a failed
    /// action. A short withdrawal is therefore a failed action with the
    /// numbers in it, not a silently short one.
    ///
    /// # An entity the game does not answer for keeps its old reading
    ///
    /// See [`FactorioWorld::observe_inventories`]. The mod skips a position
    /// where `surface.find_entity` finds nothing, so a query for five can come
    /// back with three, and the two missing ones mean "not found" rather than
    /// "empty". `PlanState::from_world` is what notices that the entity is
    /// gone, by checking the entity graph -- which is fed by
    /// `on_some_entity_deleted`, and which also clears the reading.
    ///
    /// # Failure
    ///
    /// The error is returned rather than swallowed, because a caller that
    /// wants to plan anyway can, and one that wants to stop can too. Planning
    /// against readings that failed to refresh is *not* unsafe -- it is the
    /// same staleness the paragraph above describes, one round trip further
    /// out -- so warn and carry on is a defensible choice; making it here for
    /// everybody is not.
    pub async fn refresh_buffers(&self) -> Result<usize> {
        let Some(rcon) = self.rcon.as_ref() else {
            return Ok(0);
        };
        // Deduplicated by tile and ordered by it. The entity graph is a
        // petgraph whose node order is an artefact of insertion, and while
        // nothing downstream reads the request order today, a request built in
        // a different order every call is the kind of thing that makes a
        // difference show up somewhere else later.
        let mut wanted: BTreeMap<Pos, RequestEntity> = BTreeMap::new();
        for node in self.real_world.entity_graph.inner_graph().node_weights() {
            if !BUFFER_ENTITIES.contains(&node.entity_name.as_str()) {
                continue;
            }
            wanted.insert(
                Pos::from(&node.position),
                RequestEntity {
                    name: node.entity_name.clone(),
                    position: node.position.clone(),
                },
            );
        }
        if wanted.is_empty() {
            return Ok(0);
        }
        let asked = wanted.len();
        let replies = rcon
            .inventory_contents_at(wanted.into_values().collect())
            .await?;
        self.real_world
            .observe_inventories(replies.into_iter().flatten().collect());
        Ok(asked)
    }

    /// Forget what was in whatever stood at `position`.
    ///
    /// For a caller that knows an entity is gone without the game having said
    /// so through `on_some_entity_deleted` -- a bot that mined it, above all.
    /// Kept beside [`Planner::refresh_buffers`] so that both halves of "what
    /// the planner believes about containers" are reachable from one place.
    pub fn forget_buffer(&self, position: &Position) -> bool {
        self.real_world.forget_inventory(position)
    }

    /// The bots a run may actually plan for: the ids in `1..=bot_count` the
    /// world has a player for.
    ///
    /// # A bot that is not in the game is not a bot
    ///
    /// This used to be `initiate_missing_players_with_default_inventory`'s job,
    /// and that function *invents* a player for any id the world does not have
    /// -- position defaulted, which is `(0, 0)`. In a run whose fourth Factorio
    /// client failed to connect, the roster still contained bot 4, sitting at
    /// the origin with a starter inventory, and the planner sized and assigned
    /// real work to it. Those actions could never complete: `RconActuator`
    /// checks its own roster against `connected_players()` and refuses a player
    /// the game does not have, so every step that bot owned was dead on
    /// dispatch while the plan reported itself as covering four bots.
    ///
    /// # Omitted, and named while omitting it
    ///
    /// A missing bot is dropped rather than raised, because three working
    /// clients out of four are three working clients and a run that can still
    /// do most of the work should. But dropping it *silently* would be its own
    /// failure mode -- "why did bot 4 do nothing" has to be answerable -- so
    /// every absent id is named on the way out. The total failure is loud
    /// without any help from here: no client connected means an empty roster,
    /// and `crates/planner`'s `schedule` refuses one outright.
    ///
    /// # This is not the simulation path
    ///
    /// `--clients 0` plans for bots that were never meant to exist, and says so
    /// (`factorio-bot lua ... --clients 0 --bots 4`). That mode seeds its own
    /// players through
    /// [`Planner::initiate_missing_players_with_default_inventory`] before the
    /// script runs, so they are in the world by the time this reads it and the
    /// roster comes back whole. The discrimination is the caller's stated
    /// intent, not a guess made here from an empty player list -- which is the
    /// one signal that cannot tell "simulating" from "every client failed".
    pub fn roster(&self, bot_count: u8) -> Vec<u8> {
        let mut present: Vec<u8> = vec![];
        let mut absent: Vec<u8> = vec![];
        for player_id in 1u8..=bot_count {
            if self.real_world.players.contains_key(&player_id) {
                present.push(player_id);
            } else {
                absent.push(player_id);
            }
        }
        if !absent.is_empty() {
            warn!(
                "planning for {} of {} bot(s): the game has no player for {:?}, so nothing will be \
                 assigned to {}. A client that never connected is not a bot.",
                present.len(),
                bot_count,
                absent,
                if absent.len() == 1 { "it" } else { "them" }
            );
        }
        present
    }

    /// Invents a player for every id in `1..=bot_count` the world does not
    /// already have, at the default position and with a starter inventory, and
    /// answers with the whole `1..=bot_count`.
    ///
    /// **This is the simulation seam, not the roster.** Use
    /// [`Planner::roster`] for a run with a game behind it. Two callers are
    /// entitled to this one:
    ///
    /// * `--clients 0`, the planning-only mode, where no Factorio client is
    ///   started at all and the bots are avowedly hypothetical; and
    /// * fixtures, which build a world from nothing and need bots in it.
    ///
    /// Calling it on a live run is what put a phantom bot at `(0, 0)` into
    /// every roster whose client failed to connect -- see [`Planner::roster`].
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
    use crate::types::{InventoryResponse, PlayerChangedPositionEvent, Position};

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

    /// A run that asked for four clients and got three plans for three.
    ///
    /// The fourth client never connected, so the game has no player 4 and the
    /// roster must not contain one. It used to: the roster was built by
    /// *inventing* every missing id, and an invented player takes
    /// `FactorioPlayer::default()`'s position, which is the origin. The planner
    /// then sized and assigned work to a bot that did not exist, and every
    /// action it owned was refused on dispatch by an actuator that checks the
    /// game's own player list.
    #[test]
    fn a_client_that_never_connected_produces_no_bot() {
        let world = Arc::new(FactorioWorld::new());
        for id in [1u8, 2, 3] {
            world
                .player_changed_position(moved_to(id, -22.29, 35.34))
                .expect("seed a connected player");
        }
        let planner = Planner::new(world.clone(), None);

        assert_eq!(
            planner.roster(4),
            vec![1, 2, 3],
            "the roster must be who the game has, not who was asked for"
        );
        assert!(
            world.players.get(&4).is_none(),
            "asking for the roster invented a player the game does not have"
        );
    }

    /// A gap in the middle is a gap, not a shift.
    ///
    /// `BotId` is the Factorio player id and nothing renumbers it anywhere in
    /// the stack, so a roster missing player 2 must come back as `[1, 3]` --
    /// never as `[1, 2]` with the survivors packed down, which would drive the
    /// wrong player for every step bot 3 owns.
    #[test]
    fn a_gap_in_the_middle_of_the_roster_stays_a_gap() {
        let world = Arc::new(FactorioWorld::new());
        for id in [1u8, 3] {
            world
                .player_changed_position(moved_to(id, 0., 0.))
                .expect("seed a connected player");
        }
        let planner = Planner::new(world.clone(), None);

        assert_eq!(planner.roster(3), vec![1, 3]);
    }

    /// No client connected at all: the roster is empty and stays empty.
    ///
    /// This is the case a caller must be allowed to see. `crates/planner`'s
    /// `schedule` refuses an empty roster outright, so the run fails with a
    /// sentence about having no bots rather than quietly planning for four
    /// ghosts and reporting every action lost.
    #[test]
    fn a_run_whose_clients_all_failed_gets_no_bots_at_all() {
        let planner = Planner::new(Arc::new(FactorioWorld::new()), None);
        assert!(planner.roster(4).is_empty());
    }

    /// The simulation seam still simulates.
    ///
    /// `--clients 0` has no game to defer to and says so, and this is the call
    /// that gives it bots. Pinned because the fix for the phantom bot is
    /// precisely that the *roster* stopped doing this -- if this function ever
    /// stops inventing, planning-only runs silently plan for nobody.
    #[test]
    fn the_simulation_seam_still_invents_the_bots_it_is_asked_for() {
        let world = Arc::new(FactorioWorld::new());
        let mut planner = Planner::new(world.clone(), None);

        assert_eq!(
            planner.initiate_missing_players_with_default_inventory(4),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            planner.roster(4),
            vec![1, 2, 3, 4],
            "once seeded, the world has them and the roster comes back whole"
        );
        assert_eq!(
            world
                .players
                .get(&4)
                .expect("seeded")
                .main_inventory
                .get("stone-furnace")
                .copied(),
            Some(1)
        );
    }

    fn furnace_at(x: f64, y: f64) -> crate::types::FactorioEntity {
        crate::types::FactorioEntity::new_stone_furnace(
            &Position::new(x, y),
            crate::types::Direction::North,
        )
    }

    fn reply(name: &str, position: Position, item: &str, count: u32) -> InventoryResponse {
        InventoryResponse {
            name: name.into(),
            position,
            output_inventory: Box::new(Some(vec![crate::types::InventoryItemWithQuality {
                name: item.into(),
                quality: "normal".into(),
                count,
            }])),
            fuel_inventory: Box::new(None),
        }
    }

    /// The refresh asks the game about the buffers the world knows, and puts
    /// the answers where the planner reads them.
    #[tokio::test]
    async fn refreshing_buffers_asks_about_every_known_furnace_and_stores_the_reply() {
        let world = Arc::new(FactorioWorld::new());
        world
            .on_some_entity_created(furnace_at(10., 10.))
            .expect("a furnace");
        world
            .on_some_entity_created(furnace_at(20., 20.))
            .expect("another furnace");

        let mut rcon = FactorioRcon::default();
        rcon.expect_inventory_contents_at()
            .times(1)
            .returning(|entities| {
                // Ordered by tile, deduplicated, and carrying exactly the two
                // furnaces the graph knows about.
                assert_eq!(
                    entities
                        .iter()
                        .map(|e| (e.name.clone(), e.position.x, e.position.y))
                        .collect::<Vec<_>>(),
                    vec![
                        ("stone-furnace".to_string(), 10., 10.),
                        ("stone-furnace".to_string(), 20., 20.),
                    ]
                );
                Ok(vec![Some(reply(
                    "stone-furnace",
                    Position::new(10., 10.),
                    "iron-plate",
                    7,
                ))])
            });

        let planner = Planner::new(world.clone(), Some(Arc::new(rcon)));
        assert_eq!(planner.refresh_buffers().await.expect("the query runs"), 2);

        let observed = world.observed_inventories();
        assert_eq!(observed.len(), 1, "only the entity that answered is stored");
        assert_eq!(observed[0].1.output.get("iron-plate").copied(), Some(7));
        // The furnace that did not answer is *not* recorded as empty: a
        // missing reply is a failed lookup, not an observation of nothing.
        assert!(world.inventories.get(&Pos(20, 20)).is_none());
    }

    /// A run with no game behind it asks nothing and reports so.
    ///
    /// `--clients 0` plans against invented players and has no RCON. Returning
    /// an error there would make the planning-only mode unusable; returning
    /// zero says truthfully that nothing was refreshed.
    #[tokio::test]
    async fn refreshing_buffers_without_rcon_asks_nothing() {
        let world = Arc::new(FactorioWorld::new());
        world
            .on_some_entity_created(furnace_at(10., 10.))
            .expect("a furnace");
        let planner = Planner::new(world, None);
        assert_eq!(
            planner.refresh_buffers().await.expect("no rcon, no error"),
            0
        );
    }

    /// Only [`BUFFER_ENTITIES`] are asked about.
    ///
    /// This is the whole of the "what counts as a buffer" decision, so it is
    /// pinned rather than left to the constant's doc comment: a container the
    /// bots never place is never asked about, so the planner can never
    /// withdraw from it.
    ///
    /// The example is an **iron** chest. It used to be a wooden one, until the
    /// stockpile handover started placing wooden chests and they joined the
    /// whitelist -- an example has to be something the roster genuinely never
    /// builds, or the test passes for the wrong reason.
    #[tokio::test]
    async fn a_container_that_is_not_a_buffer_entity_is_never_asked_about() {
        let world = Arc::new(FactorioWorld::new());
        world
            .on_some_entity_created(crate::types::FactorioEntity {
                name: "iron-chest".into(),
                entity_type: "container".into(),
                position: Position::new(5., 5.),
                bounding_box: crate::factorio::util::add_to_rect(
                    &crate::types::Rect::from_wh(0.7, 0.7),
                    &Position::new(5., 5.),
                ),
                ..Default::default()
            })
            .expect("somebody put a chest down");
        // With RCON present, so that the whitelist is really what stops the
        // query rather than the absence of a connection. `times(0)` is the
        // assertion: a chest a person put down is never even asked about.
        let mut rcon = FactorioRcon::default();
        rcon.expect_inventory_contents_at().times(0);
        let planner = Planner::new(world, Some(Arc::new(rcon)));
        assert_eq!(
            planner.refresh_buffers().await.expect("nothing to ask"),
            0,
            "an iron chest is not a buffer, so there is nothing to ask about"
        );

        // The discriminating half: with a furnace present the same world
        // *would* have something to ask about, so the zero above is really
        // about the chest and not about the graph being empty.
        let world = Arc::new(FactorioWorld::new());
        world
            .on_some_entity_created(furnace_at(5., 5.))
            .expect("a furnace");
        let mut rcon = FactorioRcon::default();
        rcon.expect_inventory_contents_at()
            .times(1)
            .returning(|_| Ok(vec![]));
        let planner = Planner::new(world, Some(Arc::new(rcon)));
        assert_eq!(planner.refresh_buffers().await.expect("the query runs"), 1);
    }
}
