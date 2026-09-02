use crate::error::PlannerError;
use crate::goal::Holder;
use crate::ids::{BotId, ItemId};
use factorio_bot_core::factorio::util::{add_to_rect, calculate_distance};
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::types::{
    FactorioEntity, FactorioTechnology, PlayerId, Pos, Position, Rect, ResourcePatch,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// How far two collision boxes may reach into each other before it counts.
///
/// Factorio lets boxes that merely touch along an edge coexist, and the
/// prototype numbers are binary fractions (a stone furnace is ±0.69921875),
/// so exact touching really happens. A 1/512 tile is finer than the 1/256 the
/// game stores positions at, so nothing this admits is a collision the game
/// would see; it only keeps float noise from reading as one.
const TOUCH_SLACK: f64 = 1. / 512.;

/// Half either side of a vanilla `character`'s collision box, used only when
/// the world's own `entity_prototypes` carries no `character` entry at all.
///
/// No real game omits its own player character from that table — this exists
/// for the hand-built fixtures that do, the same situation
/// `character_mining_speed`'s `VANILLA_CHARACTER_MINING_SPEED` fallback
/// covers. The number itself is not guessed: it is the `collision_box` of the
/// `character` entry in `crates/core/tests/entity-prototype-fixtures.json`,
/// which was captured off a live Factorio 2.1 game (`±0.19921875`, the same
/// source the `stone-furnace` and `assembling-machine-1` numbers quoted
/// elsewhere in this file come from).
const VANILLA_CHARACTER_COLLISION_HALF_SIDE: f64 = 0.19921875;

/// Half the side of the tile a resource entity occupies.
///
/// A tile spans one unit and the ore is reported at its *centre* — every real
/// resource sits at `(-40.5, -48.5)`, never `(-41, -49)` — so every point of
/// the tile is within this of the position the planner passes around. Used to
/// turn "a character must not stand on that tile" into a distance between two
/// tile centres; see [`PlanState::mining_tile_separation`].
const TILE_HALF_SIDE: f64 = 0.5;

/// A vanilla character's `resource_reach_distance`, used when no bot in the
/// roster reports a plausible one.
///
/// 2.7 is what live 2.1.17 reports (`crates/core/tests/live_2_1_payloads.rs`
/// pins it), and it is the same number `mine_step_aside_waypoint` in
/// `mods/BotBridge/control.lua` falls back to.
const VANILLA_RESOURCE_REACH: f64 = 2.7;

/// Above this, a reported `resource_reach_distance` is not a character's.
///
/// A *player* with no character reports `f64::MAX` here — the game's way of
/// saying "unbounded" — and feeding that into a separation would make every
/// tile on the map crowd every other one and refuse every mining plan. The
/// mod's `mine_step_aside_waypoint` guards the same value with the same
/// bound, for the same reason.
const MAX_PLAUSIBLE_RESOURCE_REACH: f64 = 1000.;

/// Half the `character` collision box on each axis, as `base` reports it.
///
/// Falls back to [`VANILLA_CHARACTER_COLLISION_HALF_SIDE`] on both axes when
/// the world carries no `character` prototype — see that constant for why a
/// world can lack one. Read rather than hardcoded because it is prototype data
/// a mod can change, exactly like `character_mining_speed`'s.
fn character_half_box(base: &FactorioWorld) -> (f64, f64) {
    base.entity_prototypes
        .get("character")
        .map(|p| {
            let b = &p.collision_box;
            (b.width() / 2., b.height() / 2.)
        })
        .unwrap_or((
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
        ))
}

/// The furthest a character's centre can be from a resource tile's centre
/// while still standing on that tile.
///
/// Two axis-aligned boxes overlap only while the gap on *both* axes is under
/// the sum of their half-sides, so the extreme separation at which they still
/// touch is corner to corner — that hypotenuse. It is the supremum of
/// [`PlanState::character_stands_on_tile`], and a unit test says so.
fn tile_occupancy_radius(base: &FactorioWorld) -> f64 {
    let (half_x, half_y) = character_half_box(base);
    (TILE_HALF_SIDE + half_x).hypot(TILE_HALF_SIDE + half_y)
}

/// Do two collision boxes share ground? Touching along an edge does not count.
fn boxes_overlap(a: &Rect, b: &Rect) -> bool {
    a.left_top.x() < b.right_bottom.x() - TOUCH_SLACK
        && b.left_top.x() < a.right_bottom.x() - TOUCH_SLACK
        && a.left_top.y() < b.right_bottom.y() - TOUCH_SLACK
        && b.left_top.y() < a.right_bottom.y() - TOUCH_SLACK
}

/// The one-tile box covering `tile`. Tile `(x, y)` spans `[x, x+1)`.
fn tile_area(tile: &Pos) -> Rect {
    Rect::new(
        &Position::new(tile.0 as f64, tile.1 as f64),
        &Position::new(tile.0 as f64 + 1., tile.1 as f64 + 1.),
    )
}

/// Every tile `area` reaches into, in a fixed order.
///
/// The slack keeps a box that stops exactly on a tile boundary from claiming
/// the tile beyond it, which is the same edge case `boxes_overlap` handles.
fn tiles_under(area: &Rect) -> Vec<Pos> {
    let x0 = (area.left_top.x() + TOUCH_SLACK).floor() as i32;
    let x1 = (area.right_bottom.x() - TOUCH_SLACK).floor() as i32;
    let y0 = (area.left_top.y() + TOUCH_SLACK).floor() as i32;
    let y1 = (area.right_bottom.y() - TOUCH_SLACK).floor() as i32;
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push(Pos(x, y));
        }
    }
    out
}

/// How much ore a tile is assumed to hold when **nobody has said**.
///
/// A fallback, no longer the answer. `EntityGraph::resource_amount` reports
/// what the game said is left in a tile -- the mod has always sent
/// `entity.amount` for a resource, and `FactorioEntity::amount` has always
/// carried it -- so this stands in only where that is `None`: a hand-built
/// fixture, whose `FactorioEntity::new_resource` sets no amount, or a resource
/// that reached the graph from a blueprint rather than from the game.
///
/// It used to be the answer for every tile on every map, and
/// `workspace/runs/run-1788334911-41961` is what that cost. Rung 6 asked one
/// bot for 50 iron ore from a tile the model said held 500; the tile held 14,
/// the bot mined it dry, and the mod reported `the target iron-ore was gone
/// before mining finished -- something else mined it first` -- which was true
/// about the disappearance and wrong about the cause, because there was no
/// something else. Five of the run's six iron mines died that way, on six
/// *different* tiles, against a roster the scheduler had put a single bot in.
///
/// The number itself is not a measurement and never was: it is large enough
/// that a fixture's ore never runs out mid-test, which is all a fixture needs.
/// A real tile near a twenty-run-old spawn holds single digits.
pub const DEFAULT_RESOURCE_PER_TILE: u32 = 500;

/// A bot's simulated state during planning.
#[derive(Clone, Debug)]
pub struct BotState {
    pub position: Position,
    pub inventory: BTreeMap<ItemId, u32>,
    pub build_distance: f64,
    pub reach_distance: f64,
    pub resource_reach_distance: f64,
}

impl Default for BotState {
    fn default() -> Self {
        BotState {
            position: Position::new(0., 0.),
            inventory: BTreeMap::new(),
            build_distance: 10.0,
            reach_distance: 10.0,
            resource_reach_distance: 3.0,
        }
    }
}

/// The world at a point in a hypothetical plan.
///
/// `base` is shared and never mutated; every difference lives in the overlay
/// fields, so `fork` costs the overlay rather than a world copy.
#[derive(Clone)]
pub struct PlanState {
    base: Arc<FactorioWorld>,
    bots: BTreeMap<BotId, BotState>,
    /// Bots `from_world` was asked for that `base` has no player for.
    ///
    /// Every bot in here got a `BotState::default()` instead of the world's
    /// real inventory and reach distances — an empty inventory and guessed
    /// reach limits, not "this bot has none of that yet". Today that never
    /// happens on the production path: `initiate_missing_players_with_default_inventory`
    /// (`crates/core/src/plan/planner.rs`) seeds every player id the run's
    /// roster names before `from_world` ever sees it, and every planner
    /// fixture that constructs a `PlanState` directly relies on the very
    /// fallback this records (`fixture_world()` never has players, so every
    /// existing test is "unknown" by this definition).
    ///
    /// That is exactly why `from_world` cannot turn this into a hard error
    /// without also rewriting every one of those fixtures, several of which
    /// (`tests/scheduling.rs`, `tests/red_science.rs`) are pinned byte-for-byte.
    /// So the substitution stays, but stops being silent: a caller that names
    /// a `BotId` the world does not have — the per-bot goal API this was
    /// flagged against — can check `unknown_bots()` and refuse to build a plan
    /// against invented reach distances, which `crates/scripting_lua` (out of
    /// this crate's scope) is where that refusal belongs.
    unknown_bots: BTreeSet<BotId>,
    /// Entities added by the plan, keyed by tile.
    added: BTreeMap<Pos, FactorioEntity>,
    /// Positions whose base-world entity the plan has removed.
    removed: BTreeSet<Pos>,
    /// Ore taken from a tile by the plan, subtracted from the base amount.
    consumed: BTreeMap<Pos, u32>,
    /// Tiles this plan has already committed to a mining action.
    ///
    /// `consumed` says *how much* the plan has taken from a tile; this says
    /// *that the tile is spoken for*, and the two answer different questions.
    /// Counting alone was not enough: [`DEFAULT_RESOURCE_PER_TILE`] is 500, so
    /// four bots each asked to mine ten iron ore all found the same nearest
    /// tile still holding hundreds and all four were sent to it. That is the
    /// defect this field exists for — the run that reached milestone 4 died on
    /// `the target stone was gone before mining finished, something else mined
    /// it first`, which is two bots on one tile seen from the game's side.
    ///
    /// So a tile is committed **whole**, not by the amount taken from it. The
    /// planner cannot know what a tile really holds (see
    /// [`DEFAULT_RESOURCE_PER_TILE`]), so "500 covers both shares" is a
    /// modelling assumption, not a fact, and it is exactly the assumption that
    /// failed. Whole-tile commitment needs no such assumption: one tile, one
    /// mining action, and the finite-amount question stops mattering between
    /// actions because there is only ever one.
    ///
    /// It costs almost nothing in locality. Candidate tiles are ordered by
    /// distance, so the second claimant takes the *next* nearest tile — one
    /// tile further on, inside the same patch — rather than a different patch.
    ///
    /// **A claim covers the ground around the tile, not only the tile.** Whole-
    /// tile exclusivity stopped two bots being sent to one ore and did nothing
    /// about the second bot *standing* on the first one's ore: run
    /// `run-1788313837-06402` spread four bots across four adjacent tiles and
    /// lost six of thirteen mines to `could not start mining for 301 ticks:
    /// another character is standing on the iron-ore`. So a claim also
    /// excludes every tile within [`PlanState::mining_tile_separation`] of it
    /// ([`PlanState::is_resource_crowded`]).
    ///
    /// Read through [`PlanState::resource_unclaimed`], never by the physical
    /// queries: [`PlanState::resource_available`] and
    /// `Condition::ResourceAvailable` still report what the ground holds,
    /// because a claim is a fact about *this plan*, not about the world the
    /// executor will meet.
    /// The tile centre is kept as the value, not rebuilt from the key.
    /// `Pos` floors, so `From<&Pos> for Position` hands back `(-41, -49)` for
    /// a tile whose ore really sits at `(-40.5, -48.5)`. That is harmless
    /// while a claim is only ever tested for equality, and is off by up to a
    /// tile the moment a *distance* is measured to it — which
    /// [`PlanState::is_resource_crowded`] does.
    claimed: BTreeMap<Pos, Position>,
    /// The one force this plan acts for, or `None` if `base` carries no forces.
    ///
    /// Chosen once, here, and read by everything that asks a question about
    /// technology — `is_researched` and `technology` both. That single
    /// selection site is the point. Before it there were two: `is_researched`
    /// answered "yes" if *any* force had the technology researched, while the
    /// cost of that same technology was read from whichever force sorted
    /// first. Each was individually defensible and together they disagreed
    /// about who we are planning for, so a world where `alpha` has not
    /// researched automation and `zeta` has made the planner silently emit
    /// nothing for `Researched("automation")` — refusing to research something
    /// the acting force does not have.
    ///
    /// The planner has no concept of acting for several forces at once, and
    /// inventing one to fix this would be speculative. So it acts for exactly
    /// one, and the type says so: there is nowhere else to make the choice and
    /// nothing else to disagree with.
    ///
    /// Which force: the alphabetically first, because `FactorioWorld::forces`
    /// is a `DashMap` whose iteration order moves with the hash seed and
    /// planning has to be reproducible. Every world this plans against has a
    /// single force, so the tie-break decides nothing in practice; it exists so
    /// that a world with several cannot make planning depend on the seed.
    force: Option<String>,
    /// Technologies the plan has completed.
    ///
    /// Keyed by bare technology name, which is unambiguous precisely because a
    /// `PlanState` acts for one force: the overlay cannot mean a different
    /// force's copy of the technology than `force` above names. If the planner
    /// ever learns to act for several, this has to become `(force, tech)` at
    /// the same time — the two are one decision, not two.
    researched: BTreeSet<String>,
    /// Holdings expansion has already promised to an action it is about to
    /// emit, keyed the way [`Holder`] keys a shortfall: per bot for
    /// `Holder::Bot`/`Holder::Share`, and once for the roster as a whole for
    /// `Holder::Anyone`.
    ///
    /// This is *not* inventory. Nothing here has been spent — the items are
    /// still in the bot's hands, and [`PlanState::inventory_count`] and
    /// [`PlanState::lose`] both still see them, which is what keeps the
    /// emitted plan's own arithmetic (and `Condition::HasItem`'s check of it)
    /// reading the real simulated inventory. A reservation only answers a
    /// different question: how much is left over for some *other* goal to
    /// count towards itself. See [`PlanState::available`].
    reserved: BTreeMap<BotId, BTreeMap<ItemId, u32>>,
    /// The `Holder::Anyone` half of `reserved`, which names no bot.
    reserved_by_anyone: BTreeMap<ItemId, u32>,
    /// Half the diagonal of the largest collision box among `base`'s known
    /// entity prototypes, or `0.` if it carries none.
    ///
    /// Computed once from `base` in [`PlanState::from_world`] rather than on
    /// every [`PlanState::is_area_clear`] call: `base` never changes after
    /// construction (see the struct doc), so the value cannot go stale, and
    /// this avoids rescanning the prototype table for every candidate
    /// placement a search considers. `entity_prototypes` is a `DashMap` with
    /// no defined iteration order, but a maximum over its values does not
    /// depend on that order, so this stays deterministic.
    max_prototype_half_diagonal: f64,
    /// How far apart two tiles must be before two *different* mining actions
    /// may claim them. See [`PlanState::mining_tile_separation`] for the
    /// derivation; computed once in [`PlanState::from_world`] because neither
    /// `base` nor the roster changes after construction.
    mining_tile_separation: f64,
    /// The collision box of every character standing on the surface, keyed by
    /// player id — roster bots included.
    ///
    /// A character is a physical obstacle: BotBridge builds with
    /// `build_check_type = manual` (`rcon_place_entity`,
    /// `mods/BotBridge/control.lua`), and that check collides with characters
    /// like any other entity. `EntityGraph` never sees one — `add` inserts a
    /// whitelist of *factory* entity types — so without this field the planner
    /// has no source for them at all and the ground under an idle bot reads as
    /// open.
    ///
    /// Run `run-1788319014-01846` is what that costs. Rung 4 left two bots
    /// parked where their last rung-2 mine had put them 14 000 ticks earlier;
    /// bot 2 sat at `(-18.2421875, 51.28125)`, the planner sited a stone
    /// furnace at `[-19, 51]` and then `[-18, 51]` five times over, and the
    /// game refused every one. The mod's own message says which character it
    /// was: it answers `§player_blocks_placement§` when the *acting* player is
    /// inside the footprint and the generic `can_place_entity said 'no'`
    /// otherwise, and the run got the generic one every time.
    ///
    /// # Why the roster is not excluded
    ///
    /// It was, for exactly one run. The first version of this field held only
    /// the characters the plan's roster did *not* name, on the reasoning that
    /// `base.players` is merely where a roster bot *started*, that the plan
    /// moves it, and that the acting bot is kept out of its own footprint by
    /// [`crate::action::Condition::AtPosition`]'s `min_radius` (see
    /// [`PlanState::placement_clearance`]) in any case.
    ///
    /// Run `run-1788322836-81715` falsified the middle step. Its roster was
    /// the whole connected game — `rcon.players()` returned `[2, 3, 4]` — so
    /// the filter emptied the field of every bot that mattered and left only
    /// the phantom at the origin. Rung 4 scheduled all 114 of its steps onto
    /// bot 2; bots 3 and 4 got none. Bot 3 was standing at
    /// `(-15.328125, -58.2890625)` where rung 2's copper had left it, the
    /// planner sited a stone furnace at `[-16, -58]` — whose box spans
    /// `[-16.8, -15.2] x [-58.8, -57.2]`, over bot 3 by a third of a tile —
    /// and the game refused it three times before the milestone gave up. A
    /// roster bot is not a bot the plan will move; it is a bot the plan *may*
    /// move, and which bots a plan actually moves is not known until
    /// `schedule` has assigned the work.
    ///
    /// Nor does the plan move one during expansion: `BotState::position` is
    /// seeded from `base.players` and never advances (`method::have`'s furnace
    /// siting says so in place, and is why it anchors on the ore instead of on
    /// the bot). So there is no second, fresher position to prefer — the
    /// observed one is the only position the planner has for any bot, roster
    /// or not, and it is a fact about the ground rather than a narrative.
    ///
    /// The acting bot is therefore blocked from the tile it currently stands
    /// on, which is over-conservative: `min_radius` will walk it clear before
    /// the placement runs. That costs one ring of [`crate::method::util::free_area_near`]'s
    /// outward search and picks a neighbouring tile. The other direction costs
    /// a milestone: a refused placement is replanned to the same site, refused
    /// again, and the run reports `stuck`. Exempting the actor is not even
    /// available as a middle course, because a site chosen while exempting the
    /// bot expansion had in hand is a site the *scheduler* may hand to a
    /// different bot, which reproduces the bug in a narrower form.
    ///
    /// Ordered by player id rather than collected straight off the `players`
    /// `DashMap`, whose iteration order moves with the hash seed.
    ///
    /// **Caveat, and it is not this field's to fix.**
    /// `Planner::initiate_missing_players_with_default_inventory`
    /// (`crates/core/src/plan/planner.rs`) invents a `FactorioPlayer` for
    /// every requested bot id the game has no player for, and a default one
    /// sits at `(0, 0)` with plausible-looking reach distances — nothing here
    /// can tell it from a real bot parked at the origin. Both runs above asked
    /// for four bots and got three clients, so player 1 was such a phantom,
    /// and it now shadows a ~0.4-tile box at the origin unconditionally rather
    /// than only when a roster filter happened to admit it. That is a small,
    /// bounded false refusal — `free_area_near` steps to the next ring — and
    /// it is the price of not silently believing a real parked bot is not
    /// there. The real fix belongs upstream, in not inventing the player, or
    /// in marking an invented one so consumers can tell.
    characters: BTreeMap<PlayerId, Rect>,
    /// Footprints the *game* has already refused a build at, this run.
    ///
    /// The sixth occupancy source, and the only one that is not a model of
    /// the ground: the five above are the planner's belief about what is
    /// there, while this is a verdict the game handed down about a specific
    /// box. It exists because the belief has been wrong four times in a row
    /// for four different reasons -- a forest, a non-roster character, a
    /// roster character, and a fourth still unidentified -- and because until
    /// now nothing carried the verdict from the run that earned it to the
    /// plan that came next. Run `run-1788323755-24892` re-chose one refused
    /// tile twice in a single milestone.
    ///
    /// # What is excluded, and why it is the box rather than the tile
    ///
    /// The whole footprint the game tested, not the tile at its centre. What
    /// the game said is "an entity of *this box* centred *here* cannot be
    /// built", so the sound inference is that something in that box blocks a
    /// build -- and nothing narrower is available, because the refusal names
    /// no cause and no coordinate. Excluding only the exact tile would let
    /// the next plan pick a neighbour whose box covers the same unknown
    /// blocker, and `free_area_near`'s rings would then walk into it one tile
    /// at a time, spending an iteration per step against a stall limit of
    /// three. Excluding the box steps clear in one move.
    ///
    /// It over-excludes when the blocker sits in a corner of the box. That is
    /// the same trade the `characters` field above documents, in the same
    /// direction: a few extra tiles of `free_area_near`'s 625-candidate
    /// window against a milestone.
    ///
    /// # It narrows the site, not the search
    ///
    /// Nothing here touches `FREE_TILE_SEARCH_RADIUS` or the ring order.
    /// A refusal makes specific candidates unavailable and lets the existing
    /// outward walk find the next one, so a refused site costs the *nearest*
    /// alternative and never a wider search than the one already run.
    ///
    /// # Believed for the whole run
    ///
    /// Read from `FactorioWorld::placement_refusals` on every
    /// [`PlanState::from_world`], and that ledger is never expired. Purity
    /// survives it: this is an input, read once at construction like every
    /// other field, and two `PlanState`s built from the same world and roster
    /// still expand to the same plan.
    ///
    /// Sorted by geometry rather than kept in arrival order, so the field
    /// does not depend on the sequence the game happened to refuse things in.
    refused: Vec<Rect>,
}

impl PlanState {
    pub fn from_world(base: Arc<FactorioWorld>, bots: &[BotId]) -> PlanState {
        let mut map = BTreeMap::new();
        let mut unknown_bots = BTreeSet::new();
        for id in bots {
            let state = match base.players.get(&id.0) {
                Some(player) => BotState {
                    position: player.position.clone(),
                    inventory: player.main_inventory.clone(),
                    build_distance: player.build_distance as f64,
                    reach_distance: player.reach_distance as f64,
                    resource_reach_distance: player.resource_reach_distance,
                },
                None => {
                    unknown_bots.insert(*id);
                    BotState::default()
                }
            };
            map.insert(*id, state);
        }
        let max_prototype_half_diagonal = base
            .entity_prototypes
            .iter()
            .map(|entry| {
                let b = &entry.value().collision_box;
                (b.width() / 2.).hypot(b.height() / 2.)
            })
            .fold(
                0.0_f64,
                |acc, d| if d.total_cmp(&acc).is_gt() { d } else { acc },
            );
        let force = base.forces.iter().map(|entry| entry.key().clone()).min();
        // The roster's worst case, not each bot's own: a tile is claimed by
        // one action and has to keep *every* other bot off it, so the bound
        // that matters is the largest reach anybody in the roster swings from.
        // `f64::MAX` — what a player with no character reports — is not a
        // character's reach and is dropped rather than propagated.
        let reach = map
            .values()
            .map(|bot| bot.resource_reach_distance)
            .filter(|reach| *reach > 0. && *reach <= MAX_PLAUSIBLE_RESOURCE_REACH)
            .fold(None, |acc: Option<f64>, reach| {
                Some(match acc {
                    Some(best) if best.total_cmp(&reach).is_ge() => best,
                    _ => reach,
                })
            })
            .unwrap_or(VANILLA_RESOURCE_REACH);
        let mining_tile_separation = reach + tile_occupancy_radius(&base);
        // Every character on the surface, roster or not. The roster used to be
        // filtered out here; see the `characters` field doc for the run that
        // showed a roster bot is not a bot the plan is going to move.
        let (half_x, half_y) = character_half_box(&base);
        let characters: BTreeMap<PlayerId, Rect> = base
            .players
            .iter()
            .map(|player| {
                let p = &player.value().position;
                (
                    *player.key(),
                    Rect::new(
                        &Position::new(p.x() - half_x, p.y() - half_y),
                        &Position::new(p.x() + half_x, p.y() + half_y),
                    ),
                )
            })
            .collect();
        // Sites the game refused, turned into the boxes it refused them at.
        // The prototype lookup is the same one `collision_area` makes, under
        // the same name the planner used when it chose the site, so the box
        // excluded here is exactly the box that was offered and turned down.
        let mut refused: Vec<Rect> = base
            .placement_refusals()
            .iter()
            .map(|refusal| {
                base.entity_prototypes
                    .get(&refusal.entity)
                    .map(|proto| add_to_rect(&proto.collision_box, &refusal.position))
                    // No prototype: the one thing that can be said without
                    // inventing a size is that the game refused a build
                    // centred here. A unit box around that centre is the
                    // floor, not a guess at the entity -- it under-excludes,
                    // which is the safe direction for a fallback that should
                    // never fire (the planner only ever places names the
                    // world has prototypes for).
                    .unwrap_or_else(|| {
                        Rect::new(
                            &Position::new(refusal.position.x - 0.5, refusal.position.y - 0.5),
                            &Position::new(refusal.position.x + 0.5, refusal.position.y + 0.5),
                        )
                    })
            })
            .collect();
        refused.sort_by(|a, b| {
            a.left_top
                .x
                .total_cmp(&b.left_top.x)
                .then(a.left_top.y.total_cmp(&b.left_top.y))
                .then(a.right_bottom.x.total_cmp(&b.right_bottom.x))
                .then(a.right_bottom.y.total_cmp(&b.right_bottom.y))
        });
        PlanState {
            base,
            bots: map,
            unknown_bots,
            added: Default::default(),
            removed: Default::default(),
            consumed: Default::default(),
            claimed: Default::default(),
            force,
            researched: Default::default(),
            reserved: Default::default(),
            reserved_by_anyone: Default::default(),
            max_prototype_half_diagonal,
            mining_tile_separation,
            characters,
            refused,
        }
    }

    pub fn fork(&self) -> PlanState {
        self.clone()
    }

    pub fn base(&self) -> &Arc<FactorioWorld> {
        &self.base
    }

    /// Bots passed to [`PlanState::from_world`] that `base` had no player
    /// for, and so were seeded with `BotState::default()` — an empty
    /// inventory and guessed reach distances — instead of the world's own
    /// data. Empty whenever every requested bot was a real player, which is
    /// every production call today; see the field doc for why this is a
    /// detectable flag rather than a hard error.
    pub fn unknown_bots(&self) -> &BTreeSet<BotId> {
        &self.unknown_bots
    }

    /// Technology `name` as the acting force defines it.
    ///
    /// The only way to reach a `FactorioTechnology` from a `PlanState`, so
    /// that a caller cannot read one force's cost while `is_researched`
    /// answers about another's.
    pub fn technology(&self, name: &str) -> Option<FactorioTechnology> {
        let force = self.force.as_deref()?;
        self.base
            .forces
            .get(force)
            .and_then(|entry| entry.technologies.get(name).cloned())
    }

    /// The acting force's `manual_mining_speed_modifier`, or `0.` when the
    /// world does not report one.
    ///
    /// Read through the acting force for the same reason `technology` is: the
    /// planner acts for exactly one force, and a mining rate taken from a
    /// different force than the research questions are answered against would
    /// be the same silent disagreement that field doc describes.
    ///
    /// `0.` is the game's own default for an unmodified force, so a world
    /// with no forces at all (every planner fixture) reads as "no bonus"
    /// rather than as an error.
    pub fn manual_mining_speed_modifier(&self) -> f64 {
        let Some(force) = self.force.as_deref() else {
            return 0.;
        };
        self.base
            .forces
            .get(force)
            .and_then(|entry| entry.manual_mining_speed_modifier.as_deref().copied())
            .map(f64::from)
            .unwrap_or(0.)
    }

    /// Every technology the acting force defines, in name order.
    ///
    /// Ordered because `recipe -> technology` lookups scan this and must pick
    /// the same answer on every run; the underlying map is a `BTreeMap`, so the
    /// order is the data's, not the hash seed's.
    pub fn technology_names(&self) -> Vec<String> {
        let Some(force) = self.force.as_deref() else {
            return Vec::new();
        };
        self.base
            .forces
            .get(force)
            .map(|entry| entry.technologies.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn bot_ids(&self) -> Vec<BotId> {
        self.bots.keys().copied().collect()
    }

    pub fn bot(&self, id: BotId) -> Option<&BotState> {
        self.bots.get(&id)
    }

    pub fn inventory_count(&self, id: BotId, item: &str) -> u32 {
        self.bots
            .get(&id)
            .and_then(|b| b.inventory.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Every item any bot holds, with the roster-wide total for each.
    pub fn item_totals(&self) -> BTreeMap<ItemId, u32> {
        let mut out: BTreeMap<ItemId, u32> = BTreeMap::new();
        for bot in self.bots.values() {
            for (item, count) in &bot.inventory {
                *out.entry(item.clone()).or_insert(0) += *count;
            }
        }
        out
    }

    /// Sum across every bot — the meaning of `Holder::Anyone`.
    pub fn total_count(&self, item: &str) -> u32 {
        self.bots
            .values()
            .map(|b| b.inventory.get(item).copied().unwrap_or(0))
            .sum()
    }

    /// How much of `item` is left for a *new* goal to count towards itself:
    /// what `whose` holds, less what expansion has already promised to an
    /// action it is about to emit.
    ///
    /// This — not `inventory_count`/`total_count` — is the question
    /// "is this goal already satisfied" has to ask. Asking the raw holding
    /// double-counts an intermediate two sub-goals of one recipe both draw on:
    /// a lab's own ten gears are visible to the sub-goal that produces its
    /// transport belts, which then spends two of them and leaves the lab craft
    /// short. Reserving is what stops a holding being claimed twice.
    ///
    /// A per-bot reservation lowers the roster total too, because the items it
    /// names are a known bot's and so genuinely spoken for. An `Anyone`
    /// reservation does *not* lower any one bot's figure, because it names no
    /// bot: nothing says which of them holds the promised items, and guessing
    /// would refuse work a bot can really do.
    pub fn available(&self, whose: &Holder, item: &str) -> u32 {
        match whose {
            Holder::Anyone => {
                let promised: u32 = self
                    .reserved
                    .values()
                    .map(|held| held.get(item).copied().unwrap_or(0))
                    .sum::<u32>()
                    .saturating_add(self.reserved_by_anyone.get(item).copied().unwrap_or(0));
                self.total_count(item).saturating_sub(promised)
            }
            Holder::Bot(id) | Holder::Share(id) => self
                .inventory_count(*id, item)
                .saturating_sub(self.reserved_for(*id, item)),
        }
    }

    fn reserved_for(&self, id: BotId, item: &str) -> u32 {
        self.reserved
            .get(&id)
            .and_then(|held| held.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Promise `count` of `item` to `whose`, hiding it from [`available`] until
    /// [`PlanState::release`] gives it back. Reservations add up, so two
    /// promises of the same item hide both.
    ///
    /// [`available`]: PlanState::available
    pub fn reserve(&mut self, whose: &Holder, item: &str, count: u32) {
        let ledger = match whose {
            Holder::Anyone => &mut self.reserved_by_anyone,
            Holder::Bot(id) | Holder::Share(id) => self.reserved.entry(*id).or_default(),
        };
        let entry = ledger.entry(item.to_string()).or_insert(0);
        *entry = entry.saturating_add(count);
    }

    /// Undo one [`PlanState::reserve`]. Saturating rather than fallible: a
    /// release is always paired with a reserve by the caller that made it, so
    /// there is no failure for a caller to handle, and clamping at zero cannot
    /// invent stock the way wrapping would.
    pub fn release(&mut self, whose: &Holder, item: &str, count: u32) {
        let ledger = match whose {
            Holder::Anyone => &mut self.reserved_by_anyone,
            Holder::Bot(id) | Holder::Share(id) => self.reserved.entry(*id).or_default(),
        };
        let Some(entry) = ledger.get_mut(item) else {
            return;
        };
        *entry = entry.saturating_sub(count);
        if *entry == 0 {
            ledger.remove(item);
        }
    }

    pub fn gain(&mut self, id: BotId, item: &str, count: u32) {
        let bot = self.bots.entry(id).or_default();
        *bot.inventory.entry(item.to_string()).or_insert(0) += count;
    }

    pub fn lose(&mut self, id: BotId, item: &str, count: u32) -> Result<(), PlannerError> {
        let available = self.inventory_count(id, item);
        if available < count {
            return Err(PlannerError::InsufficientItems {
                bot: id,
                item: item.to_string(),
                required: count,
                available,
            });
        }
        let bot = self.bots.get_mut(&id).ok_or(PlannerError::UnknownBot(id))?;
        let entry = bot.inventory.entry(item.to_string()).or_insert(0);
        *entry -= count;
        if *entry == 0 {
            bot.inventory.remove(item);
        }
        Ok(())
    }

    pub fn set_position(&mut self, id: BotId, position: Position) {
        self.bots.entry(id).or_default().position = position;
    }

    pub fn entity_at(&self, position: &Position) -> Option<FactorioEntity> {
        let key = Pos::from(position);
        if let Some(entity) = self.added.get(&key) {
            return Some(entity.clone());
        }
        if self.removed.contains(&key) {
            return None;
        }
        self.base
            .entity_graph
            .entity_at(position)
            .and_then(|id| self.base.entity_graph.entity_by_id(id))
    }

    /// The ground an entity of `name` would cover with its centre at `position`,
    /// in world coordinates.
    ///
    /// Read from the game's own `collision_box` prototype — never assumed and
    /// never per-entity constants. A stone furnace is ±0.69921875 and an
    /// assembling machine ±1.19921875; the planner has no business knowing
    /// either number.
    ///
    /// `None` when the world has no prototype under that name. Callers treat
    /// that as *not placeable* rather than guessing a size: a guess that is too
    /// small is exactly the defect this function exists to remove, and the only
    /// names the planner ever places come from the world's own prototypes and
    /// recipes, so a miss means the world carries no prototype data at all.
    /// Failing to plan is recoverable; planning a placement the game refuses,
    /// which costs the bot its whole remaining slice, is not.
    pub fn collision_area(&self, name: &str, position: &Position) -> Option<Rect> {
        self.base
            .entity_prototypes
            .get(name)
            .map(|proto| add_to_rect(&proto.collision_box, position))
    }

    /// The ground an entity already in the plan covers.
    ///
    /// `create_entity` fills the entity's `bounding_box` from its prototype, so
    /// this is normally that box. The fallback — the single tile the entity
    /// stands on — is for entities put into the state directly with neither a
    /// bounding box nor a known prototype, and it under-reserves; it is the
    /// least this can claim without inventing a size.
    fn footprint_of(&self, entity: &FactorioEntity) -> Rect {
        if entity.bounding_box.width() > 0. && entity.bounding_box.height() > 0. {
            return entity.bounding_box.clone();
        }
        self.collision_area(&entity.name, &entity.position)
            .unwrap_or_else(|| tile_area(&Pos::from(&entity.position)))
    }

    /// Whether an entity of `name` could be built with its centre at `position`.
    ///
    /// The question `is_position_free` could not ask. A stone furnace is 1.398
    /// tiles across, so two of them one tile apart overlap while each one's
    /// *tile* is free — which is how the red-science plan came to site furnaces
    /// at `[-34, -1]` and `[-34, 0]` and have the game refuse the second.
    ///
    /// Entities are compared box against box rather than tile against tile, so
    /// a placement is refused when it would actually collide and not merely
    /// when it shares a tile.
    pub fn is_area_free(&self, name: &str, position: &Position) -> bool {
        match self.collision_area(name, position) {
            Some(area) => self.is_area_clear(&area),
            None => false,
        }
    }

    /// How far a placement's own footprint pushes the acting character's
    /// stand-point away from the entity's centre: half the diagonal of the
    /// entity's collision box, plus half the diagonal of the character's.
    ///
    /// This is the annulus's inner radius, not a margin on top of one — at
    /// exactly this distance the two boxes can, in the worst-case orientation,
    /// touch at a corner (which `boxes_overlap`'s own `TOUCH_SLACK` already
    /// treats as clear); any closer and they are guaranteed to overlap.
    ///
    /// The guarantee: no point of a box is farther from that box's own centre
    /// than the box's half-diagonal (that is what a half-diagonal *is* — the
    /// distance from centre to corner). So if a point `p` is common to both
    /// boxes, the triangle inequality gives `|entity_centre - character_centre|
    /// <= |entity_centre - p| + |p - character_centre| <= entity_half_diag +
    /// character_half_diag`. Contrapositive: centres farther apart than that
    /// sum cannot share a point. This is the same reasoning
    /// [`PlanState::is_area_clear`] already uses to widen its own search
    /// radius, applied here to bound a minimum instead of a maximum.
    ///
    /// `None` when the world has no prototype for `name` — mirrors
    /// [`PlanState::collision_area`]: an unknown size is not something this
    /// can bound, not a guessed zero. In practice this is moot for any `Place`
    /// action that could ever run: its own [`crate::action::Condition::AreaFree`]
    /// needs the same prototype and refuses the action first.
    ///
    /// The character's own box falls back to
    /// [`VANILLA_CHARACTER_COLLISION_HALF_SIDE`] when the world carries no
    /// `character` prototype; see that constant's doc for where it comes from.
    pub fn placement_clearance(&self, name: &str) -> Option<f64> {
        let entity = self.base.entity_prototypes.get(name)?;
        let entity_half_diag = {
            let b = &entity.collision_box;
            (b.width() / 2.).hypot(b.height() / 2.)
        };
        let character = character_half_box(&self.base);
        let character_half_diag = character.0.hypot(character.1);
        Some(entity_half_diag + character_half_diag)
    }

    /// Whether anything the plan can see occupies `area`.
    ///
    /// Six sources, because no single one of them sees everything: entities
    /// this plan has placed, entities the base world already had, the terrain
    /// nobody built, the characters standing on it, ore, and the footprints
    /// the game itself has already refused. The middle three
    /// are the odd ones — `EntityGraph::add` only ever inserts a whitelist of
    /// *factory* entity types into the entity tree, so trees, cliffs, small
    /// rocks, units and water tiles have to come out of
    /// `blocking_boxes_within`; characters are not in any tree at all and come
    /// from `base.players` via
    /// [`characters`](PlanState#structfield.characters); and
    /// `add` routes resource entities into `resources`/`resource_tree` only,
    /// so ore has to be asked for by tile.
    ///
    /// Leaving the terrain source out is what made the planner site a stone
    /// furnace on a forest tile and the game answer `can_place_entity said
    /// 'no'` (run `run-1788309767-54739`, first dispatched action). The data
    /// was never missing — `EntityGraph` is fed `surface.find_entities(area)`,
    /// which is ~79% trees — it was only in a tree this function did not read.
    /// Leaving the character source out produced the *same message* from the
    /// same call for an entirely different reason two runs later, and again
    /// three runs after that when the character source was there but excluded
    /// the roster; see the `characters` field doc for both. The sixth source
    /// is the admission that this list will keep being incomplete: it is not
    /// a model of anything, it is the game's own refusals played back, and it
    /// is what stops the next unmodelled obstacle costing a whole milestone
    /// instead of one placement. See
    /// [`refused`](PlanState#structfield.refused).
    fn is_area_clear(&self, area: &Rect) -> bool {
        for entity in self.added.values() {
            if boxes_overlap(&self.footprint_of(entity), area) {
                return false;
            }
        }
        // `find_entities_in_radius` keeps only entities whose *centre* point
        // falls within `radius` of `search_center` (see
        // `EntityGraph::find_entities_in_radius`,
        // `core/src/graph/entity_graph.rs:132`) — it does not know the
        // entity's own footprint, so a neighbour whose box overlaps `area`
        // while its centre sits outside `radius` would silently be skipped.
        // By the triangle inequality for the Euclidean norm, an entity whose
        // box overlaps `area` has its centre no further from `area`'s centre
        // than `area`'s own half-diagonal plus that entity's half-diagonal.
        // Widening the radius by the largest half-diagonal among the world's
        // known prototypes therefore cannot miss a genuine overlap, as long
        // as no unknown prototype is wider than every known one. The exact
        // test is the `boxes_overlap` below; this only narrows the search.
        let area_half_diagonal = (area.width() / 2.).hypot(area.height() / 2.);
        let radius = area_half_diagonal + self.max_prototype_half_diagonal + TOUCH_SLACK;
        for entity in
            self.base
                .entity_graph
                .find_entities_in_radius(area.center(), radius, None, None)
        {
            if self.removed.contains(&Pos::from(&entity.position)) {
                continue;
            }
            if boxes_overlap(&entity.bounding_box, area) {
                return false;
            }
        }
        // Everything the entity tree structurally cannot hold: trees, cliffs,
        // small rocks, units, and `player_collidable` tiles (water). These
        // arrive as bare rectangles — `blocked_tree` keeps only an
        // `is_minable` flag, no name and no position — so a plan that removed
        // a base entity is matched by the tile its box is centred on, which is
        // the same key `remove_entity` stores and the same one the entity loop
        // above compares. A minable obstacle is *not* treated as clear: no
        // method emits an action to mine one out of the way, so believing a
        // tree will move is the same wrong answer as not seeing it at all.
        for blocked in self.base.entity_graph.blocking_boxes_within(area) {
            if self.removed.contains(&Pos::from(&blocked.center())) {
                continue;
            }
            if boxes_overlap(&blocked, area) {
                return false;
            }
        }
        // Characters. Nothing above can see one: they are in no tree, and
        // `removed` cannot free them either — the plan has no action that
        // makes a character move out of the way, and believing one will is the
        // same wrong answer as not seeing it at all. Roster bots included:
        // being on the roster is not a promise that this plan will move you.
        for character in self.characters.values() {
            if boxes_overlap(character, area) {
                return false;
            }
        }
        // Footprints the game has already refused a build at. Not a model of
        // an obstacle -- a verdict about one. `removed` cannot clear these
        // either: nothing the plan does is known to change the answer, since
        // the refusal never said what the answer was about.
        for refused in &self.refused {
            if boxes_overlap(refused, area) {
                return false;
            }
        }
        !tiles_under(area)
            .iter()
            .any(|tile| self.base.entity_graph.any_resource_at(tile))
    }

    /// The footprints [`is_area_clear`](PlanState::is_area_clear) refuses
    /// because the game refused them first, in the deterministic order
    /// [`PlanState::from_world`] sorted them into.
    ///
    /// Exposed so a caller can say *which* sites the planner is avoiding --
    /// a plan that quietly prefers distant tiles and cannot say why is the
    /// failure mode this memory would otherwise introduce.
    pub fn refused_footprints(&self) -> &[Rect] {
        &self.refused
    }

    /// Whether the game has already refused this exact placement.
    ///
    /// Narrower than [`PlanState::is_area_free`], which answers "is anything
    /// in the way" from six sources at once. This asks only about the one
    /// source that is a verdict rather than a model, so a caller can tell
    /// "the plan no longer fits the world" from "the game said no to this",
    /// and treat the second as durable. `crates/executor`'s `recover` uses it
    /// to refuse a tier-1 retry of a placement that is guaranteed to be
    /// refused again.
    ///
    /// `false` for an entity the world has no prototype for: an unknown size
    /// is not a footprint that can be compared, and the refusals themselves
    /// fall back to a unit box in that case, which is not a shape to test
    /// somebody else's placement against.
    pub fn is_site_refused(&self, name: &str, position: &Position) -> bool {
        let Some(area) = self.collision_area(name, position) else {
            return false;
        };
        self.refused
            .iter()
            .any(|refused| boxes_overlap(refused, &area))
    }

    /// Whether this tile is clear.
    ///
    /// A tile-granularity question, kept because that is what
    /// `Condition::PositionFree` asks. It says nothing about whether a
    /// *particular* entity fits — a placement wants [`is_area_free`], which
    /// knows how big the thing being placed is.
    ///
    /// Presence, not quantity: a tile whose ore this plan has drained to zero is
    /// still an ore tile in the ground, so it stays occupied. `remove_entity`
    /// frees a placed entity's tile, but it does not clear ore.
    pub fn is_position_free(&self, position: &Position) -> bool {
        self.is_area_clear(&tile_area(&Pos::from(position)))
    }

    /// Records a placed entity, giving it the footprint its prototype says it
    /// has if it arrived without one.
    ///
    /// The fill-in happens here, once, rather than at each of the places that
    /// ask about occupancy: methods build a `FactorioEntity` from a name and a
    /// position and leave `bounding_box` at its zero default, and a zero box
    /// collides with nothing.
    pub fn create_entity(&mut self, mut entity: FactorioEntity) {
        let key = Pos::from(&entity.position);
        if (entity.bounding_box.width() == 0. || entity.bounding_box.height() == 0.)
            && let Some(area) = self.collision_area(&entity.name, &entity.position)
        {
            entity.bounding_box = area;
        }
        self.removed.remove(&key);
        self.added.insert(key, entity);
    }

    pub fn remove_entity(&mut self, position: &Position) {
        let key = Pos::from(position);
        self.added.remove(&key);
        self.removed.insert(key);
    }

    /// Ore remaining at a tile: the tile's capacity less what this plan has taken.
    ///
    /// Presence comes from `resource_contains`, not `entity_at`: `EntityGraph::add`
    /// routes resource entities into `resources`/`resource_tree` only, so they are
    /// absent from the entity tree that `entity_at` queries.
    pub fn resource_available(&self, position: &Position, item: &str) -> u32 {
        let key = Pos::from(position);
        if !self.base.entity_graph.resource_contains(item, key.clone()) {
            return 0;
        }
        // What the game said, and only if it said nothing,
        // `DEFAULT_RESOURCE_PER_TILE`. The substitution happens exactly here,
        // once, so every selector that reads this ledger -- and the four in
        // `method::util` all do, through `resource_unclaimed` -- sees the same
        // capacity for a tile. A second substitution site is how the seats
        // count and the tile walk would start disagreeing about how many bots
        // a patch can hold.
        let capacity = self
            .base
            .entity_graph
            .resource_amount(item, &key)
            .unwrap_or(DEFAULT_RESOURCE_PER_TILE);
        capacity.saturating_sub(self.consumed.get(&key).copied().unwrap_or(0))
    }

    /// Has this plan already committed `position` to a mining action?
    ///
    /// See the [`claimed`](PlanState#structfield.claimed) field for why a
    /// commitment is whole-tile rather than by amount.
    pub fn is_resource_claimed(&self, position: &Position) -> bool {
        self.claimed.contains_key(&Pos::from(position))
    }

    /// How far apart the tiles of two *different* mining actions must be.
    ///
    /// # Why exclusivity alone was not enough
    ///
    /// Whole-tile claims stop two bots being sent to the same ore. They say
    /// nothing about where a bot *stands* to mine, and a bot has to stand
    /// somewhere: run `run-1788313837-06402` spread four bots across four
    /// adjacent tiles and then lost six of thirteen mines to
    /// `could not start mining for 301 ticks: another character is standing on
    /// the iron-ore`. Before reservation the four raced for one tile and the
    /// losers failed cleanly; after it they were neatly spaced one tile apart
    /// and blocked each other physically.
    ///
    /// # The derivation, term by term
    ///
    /// * **`resource_reach_distance`** — the radius of the disc a miner may
    ///   rest in. This is not an estimate of where the walk lands: it is the
    ///   bound `FactorioRcon::player_mine_timed` *enforces*
    ///   (`within_resource_reach`, `crates/core/src/factorio/rcon.rs`), and
    ///   the same bound the mod re-checks before setting `mining_state`. A bot
    ///   further out than this does not mine at all, so every bot that does
    ///   mine tile `T` is somewhere in `disc(T, reach)`. The executor actually
    ///   aims at `approach_radius(reach)` — half of it — so this is the worst
    ///   case rather than the expected one, which is the direction a
    ///   separation has to err in. The roster's largest plausible reach is
    ///   used; see [`MAX_PLAUSIBLE_RESOURCE_REACH`].
    /// * **`(0.5 + character_x).hypot(0.5 + character_y)`** — the furthest a
    ///   character's *centre* can be from a tile's centre while its collision
    ///   box still overlaps that tile. Two axis-aligned boxes overlap only
    ///   while both axis gaps are under the sum of their half-sides, so the
    ///   extreme is the corner-to-corner case, which is that hypotenuse.
    ///   [`TILE_HALF_SIDE`] is the tile's half-side; the character's comes
    ///   from the world's own `character` prototype (`character_half_box`).
    ///
    /// Their sum is the triangle inequality applied once: if
    /// `d(A, B) >= reach + overlap`, then a bot anywhere within `reach` of `B`
    /// is more than `overlap` from `A`, and so cannot be standing on `A`.
    /// With live numbers (2.7, ±0.19921875) that is **3.690** tiles.
    ///
    /// # What it deliberately is not
    ///
    /// It is not the mod's step-aside distance and does not try to be. The
    /// step-aside (`mine_step_aside_waypoint`) is the recovery for a bot that
    /// is *already* blocked; this is what stops the plan creating the block in
    /// the first place. Both are wanted — a re-plan cannot know where the
    /// previous plan's bots are still standing, so the step-aside remains the
    /// backstop across plans — but only one of them belongs in the planner.
    pub fn mining_tile_separation(&self) -> f64 {
        self.mining_tile_separation
    }

    /// Would a character standing at `stand` be standing on the resource tile
    /// whose ore sits at `tile`?
    ///
    /// The character's collision box against the tile's own square. Both are
    /// axis-aligned, so they share ground exactly while the gap on both axes
    /// is under the sum of their half-sides. `tile` is a tile *centre* — a
    /// real resource sits at `(-40.5, -48.5)` — so [`TILE_HALF_SIDE`] reaches
    /// its edges without any offset being restored first.
    ///
    /// This is the thing the mod reports as `another character is standing on
    /// the <ore>`, stated in geometry the planner can check, and
    /// [`PlanState::mining_tile_separation`] is derived from it.
    pub fn character_stands_on_tile(&self, stand: &Position, tile: &Position) -> bool {
        let (half_x, half_y) = character_half_box(&self.base);
        (stand.x() - tile.x()).abs() < TILE_HALF_SIDE + half_x
            && (stand.y() - tile.y()).abs() < TILE_HALF_SIDE + half_y
    }

    /// Is `position` too close to a tile some *other* mining action has
    /// already claimed?
    ///
    /// Distances are measured between tile *centres*, taken from the claim's
    /// stored `Position` rather than rebuilt from its flooring `Pos` key —
    /// see the [`claimed`](PlanState#structfield.claimed) field. Rebuilding
    /// would shift every claim by half a tile on each axis and make a
    /// separation that reads exact wrong by up to 0.71 tiles.
    ///
    /// The tile's own claim never counts: "claimed" and "crowded" are separate
    /// questions and a caller can ask either. [`PlanState::resource_unclaimed`]
    /// asks both.
    pub fn is_resource_crowded(&self, position: &Position) -> bool {
        let key = Pos::from(position);
        self.claimed.iter().any(|(claimed_key, claimed_centre)| {
            *claimed_key != key
                && calculate_distance(position, claimed_centre) < self.mining_tile_separation
        })
    }

    /// Ore at a tile that is still *available to plan against*: what
    /// [`PlanState::resource_available`] reports, or zero once the tile has
    /// been committed to a mining action, or covered by something else.
    ///
    /// This — not `resource_available` — is what tile *selection* must ask.
    /// The physical reading answers "will the ore be there when the bot
    /// swings", which is what `Condition::ResourceAvailable` needs and which a
    /// claim must not distort; this one answers "may I send another bot here",
    /// and the answer is no — whether because the tile itself is spoken for,
    /// because a bot mining a nearby claim will be standing on it, because a
    /// character is standing on it *now*
    /// ([`Self::resource_tile_occupied`]), or because something else occupies
    /// it ([`Self::resource_tile_blocked`]). All four, in the order they are
    /// cheap to test.
    pub fn resource_unclaimed(&self, position: &Position, item: &str) -> u32 {
        if self.is_resource_claimed(position) || self.is_resource_crowded(position) {
            return 0;
        }
        if self.resource_tile_occupied(position) {
            return 0;
        }
        if self.resource_tile_blocked(&Pos::from(position)) {
            return 0;
        }
        self.resource_available(position, item)
    }

    /// Is a character standing on the resource tile whose ore sits at
    /// `position`, right now, in the world this plan was built from?
    ///
    /// The crowding rule ([`PlanState::is_resource_crowded`]) keeps two tiles
    /// of *one plan* far enough apart that neither bot can stand on the
    /// other's. It says nothing about a character that is already there and
    /// that this plan is not moving, because a claim is per-`PlanState` and
    /// every plan starts with an empty one. Run
    /// `workspace/runs/run-1788329146-40305` is what that costs: rungs 1-3
    /// left all four bots parked where their rung-2 copper mine had put them,
    /// rung 4 then scheduled all of its steps onto bot 1 and none onto bots 2,
    /// 3 and 4, and the tiles it sent bot 1 to were the tiles the other three
    /// were still standing on. Bots 2, 3 and 4 did not move once between tick
    /// 6000 and the end of the run (`samples.jsonl`); bot 4 sat at
    /// `(23.305, 53.77)` and bot 1 was sent to `(23.5, 53.5)`, bot 2 sat at
    /// `(22.203, 48.785)` and bot 1 was sent to `(22.5, 48.5)`. Both mines
    /// died on `could not start mining for 301 ticks: another character is
    /// standing on the copper-ore`, and the milestone re-planned the same
    /// tiles eight times before reporting `stuck`.
    ///
    /// # Every character, the eventual miner included
    ///
    /// A bot standing on the tile it is itself about to mine is fine — the mod
    /// only refuses when `player.selected` resolves to a character that is not
    /// the miner — so an exemption for "the assignee" would be the tighter
    /// rule if it could be stated. It cannot, for two independent reasons.
    ///
    /// * **Three of the four selectors have no bot to exempt.**
    ///   [`crate::method::util::resource_supply_at_least`] and
    ///   [`crate::method::util::resource_seats`] are reached from
    ///   `Method::applicable` and `Method::concurrency`, which are handed a
    ///   `&PlanState` and nothing else; `resource_seats` is what
    ///   `SplitAcrossBots` sizes a split from. An exemption available only to
    ///   [`crate::method::util::resource_tiles_for`] would make seats promise
    ///   bots that selection then refuses to place, which is precisely the
    ///   agreement this ledger exists to keep.
    /// * **`Mine::expand`'s `ctx.chain_actor` is not the assignee.** Methods
    ///   emit `Actor::Role` with `pinned: None`; the runner is decided by
    ///   `schedule`, and a chain opened because its method `converges` gets no
    ///   owner at all ("who runs it stays the scheduler's decision", see
    ///   `expand_goal_body`). A tile chosen while exempting the bot expansion
    ///   had in hand is a tile the scheduler may hand to a different bot,
    ///   which reproduces this exact failure in a narrower form. That is the
    ///   same argument the `characters` field records for placement, and the
    ///   same one `run-1788322836-81715` settled there.
    ///
    /// So the occupant's own presence is not an exception — and it does not
    /// need to be one. The cost is that a bot parked on ore is sent to the
    /// next tile of the same patch instead of the one under its feet: one step
    /// of `resource_tiles_for`'s already-sorted walk, on a patch of thousands.
    /// The cost of the other direction is the milestone. Only the conservative
    /// direction is self-correcting.
    ///
    /// The predicate is [`PlanState::character_stands_on_tile`] — the mod's
    /// `another character is standing on the <ore>` stated as geometry, and the
    /// same one [`PlanState::mining_tile_separation`] is derived from — so the
    /// two rules cannot disagree about what "standing on" means. `position` is
    /// a tile *centre*, which is what `EntityGraph::resource_patches` hands
    /// out, so no half-tile offset has to be restored first; the character's
    /// centre is recovered from the box
    /// [`characters`](PlanState#structfield.characters) stores, which was
    /// built symmetrically around it.
    fn resource_tile_occupied(&self, position: &Position) -> bool {
        self.characters
            .values()
            .any(|character| self.character_stands_on_tile(&character.center(), position))
    }

    /// Does a blocking entity — debris, a tree, a rock, water — sit over the
    /// resource tile centred at `tile`?
    ///
    /// The counterpart of the fourth source in [`PlanState::is_area_clear`],
    /// aimed at the opposite mistake. There, the base world can hold an
    /// obstacle that reads as open ground because `entity_tree` never sees it;
    /// here, the base world can hold *ore* that reads as minable when the
    /// entity the game will actually select at that position is something
    /// else. `EntityGraph::add` routes crash-site wreckage — a
    /// `simple-entity`, exactly like a small rock — into `blocked_tree` only,
    /// never `entity_tree` (see that function's whitelist), and the mod's
    /// `player.update_selected_entity` does not filter by name: whichever
    /// entity is selectable at the position wins. When that is the wreck
    /// instead of the ore, mining stalls with `expected coal ..., found
    /// crash-site-spaceship-wreck-...` until the mod's own timeout fires
    /// (`workspace/runs/run-1788317597-64759`, rung 4).
    ///
    /// The ore itself is not gone — `add` never removes a resource entity
    /// because something else was placed over it, and this does not touch
    /// [`PlanState::resource_available`], which still reports the tile's full
    /// physical count. Only *new selection* is refused: a tile this returns
    /// `true` for must be skipped in favour of the next one, not reported as
    /// exhausted.
    fn resource_tile_blocked(&self, tile: &Pos) -> bool {
        let area = tile_area(tile);
        self.base
            .entity_graph
            .blocking_boxes_within(&area)
            .into_iter()
            .filter(|blocked| !self.removed.contains(&Pos::from(&blocked.center())))
            .any(|blocked| boxes_overlap(&blocked, &area))
    }

    /// Commit `position` to a mining action without taking anything from it.
    ///
    /// Separate from [`PlanState::consume_resource`] so a caller can say which
    /// of the two it means; `consume_resource` calls this, because taking ore
    /// out of a tile in a plan is also the plan committing to that tile.
    pub fn claim_resource(&mut self, position: &Position) {
        self.claimed.insert(Pos::from(position), position.clone());
    }

    /// Take `count` of `item` out of a tile, and commit the tile to the action
    /// that took it.
    ///
    /// The bound checked is the *physical* one, so a tile can still be drawn
    /// from twice by anything that deliberately does so (the schedule's replay
    /// re-applies each effect once, and the executor's recovery re-plans from
    /// a fresh state). Selection is what claims exclude, via
    /// [`PlanState::resource_unclaimed`].
    pub fn consume_resource(
        &mut self,
        position: &Position,
        item: &str,
        count: u32,
    ) -> Result<(), PlannerError> {
        let available = self.resource_available(position, item);
        if available < count {
            return Err(PlannerError::InsufficientItems {
                bot: BotId(0),
                item: item.to_string(),
                required: count,
                available,
            });
        }
        *self.consumed.entry(Pos::from(position)).or_insert(0) += count;
        self.claim_resource(position);
        Ok(())
    }

    /// Has the acting force finished `tech`, either already or under this plan?
    ///
    /// The union of the plan's overlay and what the acting force reports. The
    /// two cannot contradict each other on the *time* axis, because research
    /// is monotone: nothing in the game or in this planner un-researches a
    /// technology, so the overlay can only add. They could once contradict
    /// each other on the *force* axis, which is what `force` above fixes —
    /// this asks the same force `technology` reads costs from, and no other.
    pub fn is_researched(&self, tech: &str) -> bool {
        if self.researched.contains(tech) {
            return true;
        }
        matches!(self.technology(tech), Some(t) if t.researched)
    }

    /// Had the acting force finished `tech` *before this plan started*?
    ///
    /// [`Self::is_researched`]'s world half, without the overlay. The two
    /// answer different questions and the difference is load-bearing: a
    /// technology the overlay knows about is one some action in this same plan
    /// still has to run, so anything depending on it needs an ordering edge to
    /// that action. A technology the *world* reports is already done, so
    /// nothing orders against it and nothing has to.
    ///
    /// Collapsing the two is what `run-1788338409-63794` cost: the first share
    /// of a split goal researched `automation-science-pack` and applied
    /// `Effect::Researched` to the expansion state, so every later share read
    /// the recipe as plainly open, emitted no `Condition::Researched`, got no
    /// inferred edge, and was dispatched at tick zero. Three of four bots were
    /// told to craft a recipe the force had not unlocked yet.
    pub fn is_world_researched(&self, tech: &str) -> bool {
        matches!(self.technology(tech), Some(t) if t.researched)
    }

    pub fn set_researched(&mut self, tech: &str) {
        self.researched.insert(tech.to_string());
    }

    /// The base world's resource patches, with each patch's tiles in a stable
    /// order.
    ///
    /// `EntityGraph::resource_patches` builds `elements` by iterating a
    /// `HashMap` (`core/src/graph/entity_graph.rs:164-170`), so tile order
    /// varies with the randomised hash seed. Planning is required to be
    /// deterministic — the same world and the same goal must give the same
    /// schedule — and a method picking "the first tile of the patch" would
    /// otherwise pick a different one on every run. Sorting here keeps the fix
    /// inside the planner rather than perturbing `core`.
    ///
    /// **This makes tile order deterministic, not the patches themselves.** The
    /// flood fill at `entity_graph.rs:152-162` marks `pos` where it means
    /// `other`, so a tile pushed onto the stack is only ever labelled if it
    /// still has an unlabelled neighbour when it is popped; one that does not
    /// stays unlabelled and seeds a fresh patch. The fixture's contiguous
    /// 11x11 ore square therefore comes back as two or three patches, and which
    /// tiles are orphaned changes between calls *within one process*. Sorting
    /// cannot repair a partition that is wrong before it arrives. Fixing it is
    /// a one-word change in `core` and belongs with a test there.
    pub fn resource_patches(&self, item: &str) -> Vec<ResourcePatch> {
        let mut patches = self.base.entity_graph.resource_patches(item);
        for patch in &mut patches {
            patch
                .elements
                .sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        }
        patches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::{fixture_entity_prototypes, fixture_world};
    use factorio_bot_core::types::{FactorioEntity, Position};

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)])
    }

    fn entity_at_pos(name: &str, x: f64, y: f64) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: "furnace".into(),
            position: Position::new(x, y),
            ..Default::default()
        }
    }

    #[test]
    fn a_second_furnace_one_tile_from_the_first_does_not_fit() {
        // The defect, in miniature. The neighbouring tile is empty by the tile
        // test -- the first furnace's *centre* is not on it -- but a stone
        // furnace is 1.398 tiles across, so the boxes overlap and the game
        // refuses the second placement. This is what sited the red-science
        // furnaces at [-34, -1] and [-34, 0].
        let mut s = state();
        let first = Position::new(0., -1.);
        assert!(s.is_area_free("stone-furnace", &first), "open ground");
        s.create_entity(entity_at_pos("stone-furnace", first.x, first.y));

        assert!(
            !s.is_area_free("stone-furnace", &Position::new(0., 0.)),
            "a furnace one tile from another overlaps it"
        );
        // Two tiles apart it does fit -- otherwise this test would also pass on
        // an `is_area_free` that simply always says no.
        assert!(
            s.is_area_free("stone-furnace", &Position::new(0., 1.)),
            "two tiles of clearance is enough for a 1.398-wide entity"
        );
    }

    #[test]
    fn a_wide_base_world_neighbour_outside_the_old_radius_still_blocks() {
        // Reviewer's case. `find_entities_in_radius`
        // (`core/src/graph/entity_graph.rs:132`) filters candidates on each
        // entity's *centre* point, not its footprint. A real `rock-huge` is
        // 3 tiles across (half-extent 1.5, per its prototype's collision
        // box), so a copy of it centred 1.75 tiles from the query centre
        // geometrically overlaps a stone furnace placed there -- but the old
        // radius, half the query box's own span plus a flat `1.`, topped out
        // at ~1.699 and so never asked about a neighbour that far out. This
        // builds the neighbour with its *real* collision box rather than
        // going through `spawn_rocks`/`new_rock`, which hard-codes every rock
        // to 1.2 wide regardless of name and would not reproduce the defect.
        let world = fixture_world();
        let neighbour_pos = Position::new(1.75, 0.);
        let collision_box = fixture_entity_prototypes()
            .get("rock-huge")
            .expect("fixture has rock-huge")
            .collision_box
            .clone();
        world
            .update_chunk_entities(vec![FactorioEntity {
                name: "rock-huge".into(),
                entity_type: "simple-entity".into(),
                position: neighbour_pos.clone(),
                bounding_box: add_to_rect(&collision_box, &neighbour_pos),
                ..Default::default()
            }])
            .unwrap();
        let s = PlanState::from_world(Arc::new(world), &[]);

        assert!(
            !s.is_area_free("stone-furnace", &Position::new(0., 0.)),
            "a stone furnace at the origin overlaps the rock-huge's real \
             footprint, even though the rock's centre is 1.75 away and the \
             old query radius topped out at ~1.70"
        );
    }

    #[test]
    fn how_much_room_is_needed_comes_from_the_prototype() {
        // Identical geometry, two tiles apart, and the answer differs by
        // prototype: 1.398 across fits, 2.398 across does not. Neither number
        // appears here, and a fixed size -- whatever it was -- would have to
        // give these two the same answer.
        let mut small = state();
        small.create_entity(entity_at_pos("stone-furnace", 0., -1.));
        assert!(
            small.is_area_free("stone-furnace", &Position::new(0., 1.)),
            "a stone furnace fits two tiles from another"
        );

        let mut large = state();
        large.create_entity(entity_at_pos("assembling-machine-1", 0., -1.));
        assert!(
            !large.is_area_free("assembling-machine-1", &Position::new(0., 1.)),
            "an assembling machine does not fit in the same gap"
        );
    }

    /// The defect that stuck run `run-1788309767-54739` on its first action.
    ///
    /// `fixture_world` spawns 100 trees around `(-20, -20)` and a 4x4 patch of
    /// `player_collidable` water around `(40, 40)` — the same two obstacle
    /// classes a real map is full of, and the same two `EntityGraph::add`
    /// keeps out of `entity_tree`. Before `is_area_clear` read
    /// `blocking_boxes_within`, both of these read as open ground and the
    /// planner sited furnaces on them; the game answered
    /// `can_place_entity said 'no'` and the run had no way forward.
    #[test]
    fn a_furnace_does_not_fit_on_a_tree_or_in_water() {
        let s = state();
        assert!(
            !s.is_area_free("stone-furnace", &Position::new(-20., -20.)),
            "the fixture's trees are centred here; a furnace cannot go on them"
        );
        assert!(
            !s.is_area_free("stone-furnace", &Position::new(40., 40.)),
            "the fixture's water is here; a furnace cannot go on it"
        );
        // Open ground a few tiles from either, so this cannot pass on an
        // `is_area_free` that simply always says no.
        assert!(
            s.is_area_free("stone-furnace", &Position::new(0., -1.)),
            "open ground still takes a furnace"
        );
    }

    /// A tree the plan has already mined out of the way must stop blocking.
    ///
    /// `blocked_tree` carries no name and no position, only a rectangle, so
    /// the `removed` ledger is matched against the tile the rectangle is
    /// centred on. Getting that key wrong would leave every removal invisible
    /// on this path, which is silent: the placement would simply never be
    /// planned.
    #[test]
    fn removing_a_tree_frees_the_ground_under_it() {
        let mut s = state();
        let tree = Position::new(-20., -20.);
        assert!(!s.is_area_free("stone-furnace", &tree));
        // The fixture's trees are one per tile and 0.8 across, so a tree
        // centred a whole tile away still reaches into a furnace's 1.398-wide
        // box: the 3x3 around the site, not the 2x2 under it, is what has to
        // go.
        for x in -21..=-19 {
            for y in -21..=-19 {
                s.remove_entity(&Position::new(x as f64, y as f64));
            }
        }
        assert!(
            s.is_area_free("stone-furnace", &tree),
            "with the trees mined the ground is buildable again"
        );
    }

    #[test]
    fn an_entity_with_no_prototype_is_refused_rather_than_guessed_at() {
        let s = state();
        assert!(
            s.collision_area("not-a-real-entity", &Position::new(0., 0.))
                .is_none()
        );
        assert!(
            !s.is_area_free("not-a-real-entity", &Position::new(0., 0.)),
            "an unknown size must fail to plan, not be assumed small"
        );
    }

    /// Drives the same `boxes_overlap` the real box-against-box check uses,
    /// with an actual character-sized box standing at the computed distance.
    ///
    /// Not a ghost check: ghosts do not collide (`only_ghosts = true`
    /// validates nothing, per `CLAUDE.md`'s note on the trap), so a test that
    /// only asked `is_area_free`/a ghost placement to succeed would prove
    /// nothing about whether the clearance is actually big enough. This asks
    /// the geometry question directly, against the real fixture collision
    /// boxes.
    ///
    /// The diagonal, not an axis, because two axis-aligned squares first
    /// touch along their diagonal at exactly the sum of their half-diagonals —
    /// on an axis the true threshold is the (smaller) sum of half-*widths*,
    /// so an axis-aligned probe would pass even for a clearance far too small
    /// to be safe in the worst-case orientation.
    #[test]
    fn placement_clearance_keeps_the_characters_own_box_off_the_footprint() {
        let s = state();
        let clearance = s
            .placement_clearance("stone-furnace")
            .expect("fixture has a stone-furnace prototype");

        let furnace_box = s
            .collision_area("stone-furnace", &Position::new(0., 0.))
            .expect("fixture has a stone-furnace prototype");
        let character_box = fixture_entity_prototypes()
            .get("character")
            .expect("fixture has a character prototype")
            .collision_box
            .clone();

        let diag = std::f64::consts::FRAC_1_SQRT_2;
        let stand_at_clearance = Position::new(clearance * diag, clearance * diag);
        assert!(
            !boxes_overlap(
                &furnace_box,
                &add_to_rect(&character_box, &stand_at_clearance)
            ),
            "a character standing at the computed clearance, on the diagonal, \
             must clear the furnace's own footprint"
        );

        // A few centimetres inside it, same direction, must overlap -- or the
        // clearance is generous enough to pass regardless of what it actually
        // computed.
        let too_close = clearance - 0.05;
        let stand_too_close = Position::new(too_close * diag, too_close * diag);
        assert!(
            boxes_overlap(&furnace_box, &add_to_rect(&character_box, &stand_too_close)),
            "0.05 tiles inside the computed clearance must still collide on \
             the diagonal, or this test proves nothing"
        );
    }

    #[test]
    fn placement_clearance_grows_with_the_entity_and_is_none_for_an_unknown_one() {
        let s = state();
        let furnace = s
            .placement_clearance("stone-furnace")
            .expect("fixture has a stone-furnace prototype");
        let assembler = s
            .placement_clearance("assembling-machine-1")
            .expect("fixture has an assembling-machine-1 prototype");
        assert!(
            assembler > furnace,
            "a bigger entity needs more clearance: furnace {furnace}, assembler {assembler}"
        );
        assert!(
            s.placement_clearance("not-a-real-entity").is_none(),
            "an unknown size must not be guessed at, same as `collision_area`"
        );
    }

    #[test]
    fn a_placed_entity_carries_the_footprint_its_prototype_gives_it() {
        // Methods build a `FactorioEntity` from a name and a position and leave
        // `bounding_box` at zero; a zero box collides with nothing, so the fill
        // in `create_entity` is what makes the overlap test above possible.
        let mut s = state();
        assert_eq!(
            entity_at_pos("stone-furnace", 0., -1.).bounding_box.width(),
            0.
        );
        s.create_entity(entity_at_pos("stone-furnace", 0., -1.));
        let stored = s.entity_at(&Position::new(0., -1.)).expect("placed");
        assert!(
            stored.bounding_box.width() > 1.,
            "expected the prototype's 1.398, got {}",
            stored.bounding_box.width()
        );
    }

    #[test]
    fn fork_shares_the_base_world() {
        let a = state();
        let b = a.fork();
        assert!(Arc::ptr_eq(a.base(), b.base()));
    }

    #[test]
    fn fork_isolates_inventory_changes() {
        let a = state();
        let mut b = a.fork();
        b.gain(BotId(1), "iron-ore", 5);
        assert_eq!(b.inventory_count(BotId(1), "iron-ore"), 5);
        assert_eq!(a.inventory_count(BotId(1), "iron-ore"), 0);
    }

    /// `item_totals` is the roster's inventory, not one bot's.
    ///
    /// It exists so the driver can diff a chain's produce, and a diff that saw
    /// only the bot it happened to start from would under-count every chain
    /// whose actor is anyone else — silently, since a smaller diff just
    /// reserves less.
    #[test]
    fn item_totals_reports_every_bots_holdings() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 3);
        a.gain(BotId(2), "iron-plate", 4);
        a.gain(BotId(2), "coal", 1);
        let totals = a.item_totals();
        assert_eq!(totals.get("iron-plate"), Some(&7));
        assert_eq!(totals.get("coal"), Some(&1));
        assert_eq!(totals.get("wood"), None, "nothing is invented");
    }

    #[test]
    fn total_count_sums_across_bots() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 3);
        a.gain(BotId(2), "iron-plate", 4);
        assert_eq!(a.total_count("iron-plate"), 7);
    }

    #[test]
    fn lose_more_than_held_is_an_error() {
        let mut a = state();
        a.gain(BotId(1), "coal", 2);
        assert!(a.lose(BotId(1), "coal", 3).is_err());
        assert_eq!(a.inventory_count(BotId(1), "coal"), 2);
    }

    #[test]
    fn a_bot_takes_its_position_inventory_and_reach_from_its_player() {
        // The only path a real world takes into the planner: `fixture_world()`
        // has no players, so every other test exercises the `None` arm and the
        // `Some` arm's field reads run nowhere else.
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                position: Position::new(12.5, -7.5),
                main_inventory: BTreeMap::from([("iron-plate".to_string(), 42u32)]),
                build_distance: 12,
                reach_distance: 8,
                resource_reach_distance: 4.0,
                ..Default::default()
            },
        );

        let s = PlanState::from_world(Arc::new(world), &[BotId(1), BotId(2)]);
        let bot = s.bot(BotId(1)).expect("bot 1 exists");
        assert_eq!(bot.position, Position::new(12.5, -7.5));
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 42);
        assert_eq!(bot.build_distance, 12.0);
        assert_eq!(bot.reach_distance, 8.0);
        assert_eq!(bot.resource_reach_distance, 4.0);

        // Bot 2 has no player and still falls back to the defaults.
        let other = s.bot(BotId(2)).expect("bot 2 exists");
        assert_eq!(other.position, Position::new(0., 0.));
        assert_eq!(other.build_distance, 10.0);

        // The fallback is recorded: bot 2 got invented data, bot 1 did not.
        assert_eq!(s.unknown_bots(), &BTreeSet::from([BotId(2)]));
    }

    #[test]
    fn a_bot_with_a_real_player_is_not_unknown() {
        // Guards the flag in isolation from the fallback-data test above: a
        // `PlanState` built entirely from real players must report no unknown
        // bots at all, not merely "fewer than requested".
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                ..Default::default()
            },
        );

        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            s.unknown_bots().is_empty(),
            "bot 1 has a real player and must not be flagged unknown"
        );
    }

    #[test]
    fn missing_bots_get_default_reach_distances() {
        let a = state();
        let bot = a.bot(BotId(1)).expect("bot 1 exists");
        assert_eq!(bot.build_distance, 10.0);
        assert_eq!(bot.reach_distance, 10.0);
        assert_eq!(bot.resource_reach_distance, 3.0);
    }

    fn iron_ore_tile(state: &PlanState) -> Position {
        state
            .resource_patches("iron-ore")
            .first()
            .expect("fixture_world has an iron-ore patch")
            .elements
            .first()
            .expect("patch has tiles")
            .clone()
    }

    #[test]
    fn patch_tiles_come_back_sorted() {
        let a = state();
        for patch in a.resource_patches("iron-ore") {
            let mut sorted = patch.elements.clone();
            sorted.sort_by(|l, r| l.x.total_cmp(&r.x).then(l.y.total_cmp(&r.y)));
            assert_eq!(
                patch.elements, sorted,
                "every patch's tiles must come back sorted by (x, y)"
            );
        }
        // Deliberately not asserted: that two calls return the same patches.
        // They do not — see `resource_patches` for why, and why the planner
        // cannot fix it.
    }

    #[test]
    fn consuming_ore_reduces_what_is_available() {
        let mut a = state();
        let pos = iron_ore_tile(&a);
        let before = a.resource_available(&pos, "iron-ore");
        assert!(before > 0, "fixture ore tile should hold ore");
        a.consume_resource(&pos, "iron-ore", 1).unwrap();
        assert_eq!(a.resource_available(&pos, "iron-ore"), before - 1);
    }

    #[test]
    fn consuming_more_ore_than_present_is_an_error() {
        let mut a = state();
        let pos = iron_ore_tile(&a);
        let all = a.resource_available(&pos, "iron-ore");
        assert!(a.consume_resource(&pos, "iron-ore", all + 1).is_err());
        assert_eq!(a.resource_available(&pos, "iron-ore"), all);
    }

    #[test]
    fn consuming_ore_does_not_affect_the_fork_parent() {
        let a = state();
        let pos = iron_ore_tile(&a);
        let before = a.resource_available(&pos, "iron-ore");
        let mut b = a.fork();
        b.consume_resource(&pos, "iron-ore", 1).unwrap();
        assert_eq!(a.resource_available(&pos, "iron-ore"), before);
    }

    #[test]
    fn created_entities_occupy_their_position() {
        let mut a = state();
        let pos = Position::new(3., 3.);
        assert!(a.is_position_free(&pos));
        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };
        a.create_entity(furnace);
        assert!(!a.is_position_free(&pos));
        assert_eq!(a.entity_at(&pos).unwrap().name, "stone-furnace");
    }

    #[test]
    fn removed_entities_free_their_position() {
        let mut a = state();
        let pos = Position::new(3., 3.);
        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };
        a.create_entity(furnace);
        a.remove_entity(&pos);
        assert!(a.is_position_free(&pos));
        assert!(a.entity_at(&pos).is_none());
    }

    #[test]
    fn a_tile_holding_ore_is_not_free() {
        let a = state();
        let ore = a
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|p| p.elements)
            .next()
            .expect("fixture has iron ore");
        assert!(a.resource_available(&ore, "iron-ore") > 0);
        // Resources never reach the entity tree, so `entity_at` is blind to
        // them; occupancy must still see them.
        assert!(a.entity_at(&ore).is_none());
        assert!(
            !a.is_position_free(&ore),
            "ore tile {:?} was reported free",
            ore
        );
    }

    #[test]
    fn research_defaults_to_unresearched_and_can_be_set() {
        let mut a = state();
        assert!(!a.is_researched("automation"));
        a.set_researched("automation");
        assert!(a.is_researched("automation"));
    }

    /// A reservation is a claim on stock, not a withdrawal of it. Both halves
    /// are asserted: `available` must fall, and the inventory the emitted plan
    /// is checked against must not move at all — a reservation that debited
    /// the inventory would make `Effect::LoseItem` fail on items the bot
    /// really holds.
    #[test]
    fn a_reservation_hides_stock_from_available_without_spending_it() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 10);
        let whose = Holder::Share(BotId(1));

        assert_eq!(a.available(&whose, "iron-plate"), 10);
        a.reserve(&whose, "iron-plate", 4);
        assert_eq!(a.available(&whose, "iron-plate"), 6, "four are spoken for");
        assert_eq!(
            a.inventory_count(BotId(1), "iron-plate"),
            10,
            "but none have been spent"
        );
        a.lose(BotId(1), "iron-plate", 10)
            .expect("a reservation must not block spending what is really held");

        a.release(&whose, "iron-plate", 4);
        a.gain(BotId(1), "iron-plate", 10);
        assert_eq!(a.available(&whose, "iron-plate"), 10, "and released again");
    }

    /// Reservations add up rather than overwrite: two claims on the same item
    /// hide both, which is the whole point when one recipe's ingredients each
    /// reduce to the same intermediate.
    #[test]
    fn two_reservations_on_one_item_hide_both() {
        let mut a = state();
        a.gain(BotId(1), "iron-gear-wheel", 10);
        let whose = Holder::Share(BotId(1));
        a.reserve(&whose, "iron-gear-wheel", 6);
        a.reserve(&whose, "iron-gear-wheel", 3);
        assert_eq!(a.available(&whose, "iron-gear-wheel"), 1);
        a.release(&whose, "iron-gear-wheel", 6);
        assert_eq!(a.available(&whose, "iron-gear-wheel"), 7);
    }

    /// The two ledgers, and the deliberate asymmetry between them. A bot's
    /// reservation names a bot, so it lowers the roster total too. An
    /// `Anyone` reservation names none, so it lowers the total and nobody's
    /// individual figure.
    #[test]
    fn a_bot_reservation_lowers_the_roster_total_but_an_anyone_reservation_lowers_no_bot() {
        let mut a = state();
        a.gain(BotId(1), "coal", 4);
        a.gain(BotId(2), "coal", 6);
        assert_eq!(a.available(&Holder::Anyone, "coal"), 10);

        a.reserve(&Holder::Share(BotId(1)), "coal", 3);
        assert_eq!(a.available(&Holder::Share(BotId(1)), "coal"), 1);
        assert_eq!(
            a.available(&Holder::Share(BotId(2)), "coal"),
            6,
            "one bot's claim says nothing about another's stock"
        );
        assert_eq!(
            a.available(&Holder::Anyone, "coal"),
            7,
            "but it is spoken for as far as the roster is concerned"
        );

        a.reserve(&Holder::Anyone, "coal", 2);
        assert_eq!(a.available(&Holder::Anyone, "coal"), 5);
        assert_eq!(
            a.available(&Holder::Share(BotId(2)), "coal"),
            6,
            "an Anyone claim names no bot, so it cannot be charged to one"
        );
    }

    /// Releasing more than was reserved clamps at zero rather than wrapping,
    /// which would otherwise hand a caller `u32::MAX` items of headroom.
    #[test]
    fn releasing_more_than_was_reserved_clamps_at_nothing_reserved() {
        let mut a = state();
        a.gain(BotId(1), "stone", 5);
        let whose = Holder::Share(BotId(1));
        a.reserve(&whose, "stone", 2);
        a.release(&whose, "stone", 9);
        assert_eq!(a.available(&whose, "stone"), 5);
    }

    /// The separation is a *derived* number, not a tuned one: it is the reach
    /// the game enforces plus the radius at which a character stops standing
    /// on a tile. Both halves are pinned here so a change to either shows up
    /// as a change to this, rather than silently.
    #[test]
    fn the_mining_separation_is_reach_plus_the_occupancy_radius() {
        let a = state();
        let reach = a.bot(BotId(1)).expect("bot 1").resource_reach_distance;
        let occupancy = a.mining_tile_separation() - reach;
        // The fixture world's `character` box, or the vanilla fallback: either
        // way, half a tile plus half a character on each axis, corner to
        // corner.
        let (half_x, half_y) = character_half_box(a.base());
        assert!(
            (occupancy - (TILE_HALF_SIDE + half_x).hypot(TILE_HALF_SIDE + half_y)).abs() < 1e-12,
            "separation {} is not reach {} plus the occupancy radius",
            a.mining_tile_separation(),
            reach
        );
    }

    /// The occupancy radius has to be the *supremum* of
    /// `character_stands_on_tile`, or the separation built on it is either
    /// unsafe (too small) or wasteful (too large). Walked in a fine ring
    /// rather than argued.
    #[test]
    fn the_occupancy_radius_is_exactly_where_a_character_stops_standing_on_a_tile() {
        let a = state();
        let tile = Position::new(-40.5, 40.5);
        let radius =
            a.mining_tile_separation() - a.bot(BotId(1)).expect("bot 1").resource_reach_distance;
        for step in 0..720 {
            let angle = std::f64::consts::TAU * f64::from(step) / 720.;
            let (sin, cos) = angle.sin_cos();
            let outside = Position::new(
                tile.x() + cos * (radius + 1e-6),
                tile.y() + sin * (radius + 1e-6),
            );
            assert!(
                !a.character_stands_on_tile(&outside, &tile),
                "a character {radius} out at {angle} rad still stands on the tile"
            );
        }
        // And it is not slack: the corner direction still overlaps just inside.
        let diagonal = std::f64::consts::FRAC_1_SQRT_2 * (radius - 1e-6);
        assert!(a.character_stands_on_tile(
            &Position::new(tile.x() + diagonal, tile.y() + diagonal),
            &tile
        ));
    }

    /// A player with no character reports `f64::MAX` here. Propagating that
    /// would crowd every tile on the map out of every plan.
    #[test]
    fn an_unbounded_reach_does_not_become_an_unbounded_separation() {
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                resource_reach_distance: f64::MAX,
                ..Default::default()
            },
        );
        let a = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            a.mining_tile_separation() < 5.,
            "separation ran away: {}",
            a.mining_tile_separation()
        );
    }

    /// The whole point of storing the claim's `Position` beside its `Pos` key:
    /// a tile centre must not be rounded down before a distance is measured
    /// to it.
    #[test]
    fn crowding_is_measured_from_tile_centres_not_from_floored_keys() {
        let mut a = state();
        let tile = Position::new(-40.5, 40.5);
        a.claim_resource(&tile);

        let separation = a.mining_tile_separation();
        // A tile just outside the separation measured from the *centre*, and
        // just inside it if the claim had been floored to (-41, 40).
        let outside = Position::new(-40.5 + separation + 0.01, 40.5);
        assert!(
            !a.is_resource_crowded(&outside),
            "measured from the floored key, this would read as crowded"
        );
        let inside = Position::new(-40.5 + separation - 0.01, 40.5);
        assert!(a.is_resource_crowded(&inside));
    }

    /// Claimed and crowded are separate questions, and `resource_unclaimed`
    /// asks both. A tile is never crowded by its own claim.
    #[test]
    fn a_tile_next_to_a_claim_is_crowded_without_being_claimed() {
        let mut a = state();
        let tile = Position::new(-40.5, 40.5);
        let neighbour = Position::new(-39.5, 40.5);
        a.claim_resource(&tile);

        assert!(a.is_resource_claimed(&tile));
        assert!(
            !a.is_resource_crowded(&tile),
            "a tile must not crowd itself, or the two predicates cannot be read apart"
        );
        assert!(!a.is_resource_claimed(&neighbour));
        assert!(a.is_resource_crowded(&neighbour));

        assert_eq!(a.resource_unclaimed(&neighbour, "iron-ore"), 0);
        assert_eq!(
            a.resource_available(&neighbour, "iron-ore"),
            DEFAULT_RESOURCE_PER_TILE,
            "crowding is a fact about the plan, not about the ground"
        );
    }
}
