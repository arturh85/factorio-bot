//! Building a designed block: a blueprint, an anchor, and one band per bot.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder, Site};
use crate::ids::ActionId;
use crate::method::have::PLACE_TICKS;
use crate::method::util::nearest_resource_tile;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::blueprint::{Blueprint, BlueprintEntity, UndergroundHalf, decode};
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{Direction, FactorioEntity, Pos, Position, Rect};
use std::collections::{BTreeMap, BTreeSet};

/// Which of a block's two axes the bands are cut across.
///
/// Not a preference: a band is only a *region* if the cut runs across the
/// block's short side, and the promise a band makes -- "a bot never crosses
/// another's band, which is the structural reason two of them cannot trap
/// each other" -- is a promise about regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    /// Vertical slabs: cut across x, right for a block wider than it is tall.
    X,
    /// Horizontal slabs: cut across y.
    Y,
}

/// The axis `bands` will cut across for these entities: the block's LONGER
/// one, so the slabs are cut across its short side.
///
/// Ties (a square block) go to x, which is the axis this function always
/// used; nothing about a square makes either choice better and a fixed
/// tie-break keeps the split deterministic.
fn split_axis(entities: &[BlueprintEntity]) -> SplitAxis {
    let mut min = (f64::INFINITY, f64::INFINITY);
    let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for e in entities {
        min = (min.0.min(e.offset.x()), min.1.min(e.offset.y()));
        max = (max.0.max(e.offset.x()), max.1.max(e.offset.y()));
    }
    let width = max.0 - min.0;
    let height = max.1 - min.1;
    if height > width {
        SplitAxis::Y
    } else {
        SplitAxis::X
    }
}

/// Split a block into one band per bot, **balanced by entity count and cut
/// across the block's longer axis**.
///
/// Sorted along the dominant axis, then chunked so each band holds as near an
/// equal number of entities as divides. Balancing by *extent* instead would
/// hand one bot a dense corner and another an empty margin, which is why the
/// chunking counts entities; choosing the axis by extent is a different
/// question and is answered by [`split_axis`].
///
/// **This sorted by x unconditionally until 2026-09-05, and the spec's
/// spatial claim was false on a fixture this crate ships.** `MinerLine` is 4
/// tiles wide and 21 tall: over its 37 entities, bands 0, 1 and 2 all
/// occupied x = 3.5, and band 0 spanned the whole 20-tile height that bands 1
/// and 2 were segments of -- three bots interleaved in a one-tile corridor,
/// which is the opposite of the disjointness the band exists to provide. The
/// synthetic test that passed was correct for its own case (a wide block) and
/// is exactly what let this through; `bands_over_the_real_miner_line_are_
/// disjoint_along_the_split_axis` is the one that would not have.
///
/// **The remainder is spread, not dumped.** `div_ceil` chunking gave six
/// entities across four bots as 2/2/2/0 -- a whole idle bot -- where the even
/// split is 2/2/1/1. The first `n % bots` bands take one extra each.
///
/// Deterministic: the sort is by `total_cmp` on the chosen axis with the
/// entity's index as the tie-break, so equal-coordinate entities always fall
/// the same way.
pub fn bands(entities: &[BlueprintEntity], bots: usize) -> Vec<Vec<usize>> {
    if bots == 0 {
        return Vec::new();
    }
    let axis = split_axis(entities);
    let key = |i: usize| match axis {
        SplitAxis::X => entities[i].offset.x(),
        SplitAxis::Y => entities[i].offset.y(),
    };
    let mut order: Vec<usize> = (0..entities.len()).collect();
    order.sort_by(|a, b| key(*a).total_cmp(&key(*b)).then(a.cmp(b)));

    // Sizes first, then fill: `n / bots` each, and the first `n % bots` bands
    // take one extra. Bands that want nothing (more bots than entities) stay
    // empty rather than being handed a stray entity.
    let base = entities.len() / bots;
    let remainder = entities.len() % bots;
    let mut out = vec![Vec::new(); bots];
    let mut rest = order.as_slice();
    for (band, slot) in out.iter_mut().enumerate() {
        let size = base + usize::from(band < remainder);
        let (mine, tail) = rest.split_at(size);
        slot.extend_from_slice(mine);
        rest = tail;
    }
    debug_assert!(rest.is_empty(), "every entity lands in exactly one band");
    out
}

/// The `FactorioEntity` one blueprint entity places, at its world position.
///
/// Follows `power.rs::entity_for` / `assemble.rs::entity_for`: `entity_type`
/// is read from the world's own prototype table rather than guessed, and the
/// `bounding_box` is left for `PlanState::create_entity` to fill in from the
/// same table (see its own doc for why that overlay half exists).
///
/// `e.underground_half` is carried straight onto the entity -- `Some` only
/// for one half of an underground-belt pair (`BlueprintEntity::underground_half`),
/// `None` for everything else -- which is what makes the two halves of a pair
/// distinguishable all the way to `rcon_place_entity`.
fn entity_for(state: &PlanState, e: &BlueprintEntity, position: &Position) -> FactorioEntity {
    let entity_type = state
        .base()
        .entity_prototypes
        .get(e.name.as_str())
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| e.name.clone());
    FactorioEntity {
        name: e.name.clone(),
        entity_type,
        position: position.clone(),
        direction: e.direction,
        underground_half: e.underground_half,
        ..Default::default()
    }
}

/// Copied from `crates/planner/src/method/connect.rs`'s `place_step`: the
/// same preconditions, the same effects, and the same overlay call so the
/// next entity's `AreaFree` sees what this one took.
fn place_step(ctx: &mut ExpansionCtx, entity: FactorioEntity, build: f64, note: &str) -> Step {
    let min_radius = ctx.state.placement_clearance(&entity.name).unwrap_or(0.0);
    let step = Step::Act(Box::new(Action {
        id: ctx.ids.next(),
        kind: ActionKind::Place {
            entity: Box::new(entity.clone()),
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: entity.position.clone(),
                radius: build,
                min_radius,
            },
            Condition::AreaFree {
                pos: entity.position.clone(),
                entity: entity.name.as_str().into(),
                direction: entity.direction,
            },
            Condition::HasItem {
                who: Actor::Role,
                item: entity.name.as_str().into(),
                count: 1,
            },
        ],
        eff: vec![
            Effect::LoseItem {
                who: Actor::Role,
                item: entity.name.as_str().into(),
                count: 1,
            },
            Effect::CreateEntity(Box::new(entity.clone())),
        ],
        duration: PLACE_TICKS,
        pinned: None,
        label: format!("place {} at {} -- {note}", entity.name, entity.position),
    }));
    // The overlay half, exactly as `power.rs` does it: the next tile's
    // `AreaFree` must see what this one took.
    ctx.state.create_entity(entity);
    step
}

/// `"input"` / `"output"` / `"neither"`, for a refusal message.
fn half_name(half: Option<UndergroundHalf>) -> &'static str {
    match half {
        Some(UndergroundHalf::Input) => "input",
        Some(UndergroundHalf::Output) => "output",
        None => "neither",
    }
}

/// What is standing where a blueprint entity wants to be.
///
/// Three answers, and the middle one is the whole point of this type.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Standing {
    /// Nothing of this name is centred on that tile.
    Nothing,
    /// The blueprint's entity, exactly as designed -- same name, same tile,
    /// same facing, same underground half. Nothing to do.
    AsDesigned,
    /// Something of the right name on the right tile, **facing the wrong way
    /// or the wrong half of an underground pair**. Carries what stands and
    /// what was wanted, so a refusal can say both.
    Differently {
        direction: (u8, u8),
        half: (Option<UndergroundHalf>, Option<UndergroundHalf>),
    },
}

/// Is `e` already standing, as designed, centred at `world`?
///
/// The same pattern `assemble.rs::standing_parts` and `power.rs::finish` use:
/// `entity_at` answers for anything covering the point, so the name and the
/// **tile-centred** position both have to match, or a machine one tile off
/// the layout would be read as this one.
///
/// **It compared name and tile only until 2026-09-05, and that is the worst
/// shape of bug this branch can have.** A belt standing on the right tile
/// facing the wrong way, or an underground half placed as `input` where
/// `output` was wanted, read as *already built*. It cannot produce a bad
/// build from a clean start -- but it permanently freezes one in, because
/// replanning is exactly what would otherwise correct it, and replanning is
/// the mechanism this whole method is built on ("re-derived against the world
/// on every expansion rather than remembered"). Direction and
/// `underground_half` are the two fields whose whole reason for existing on
/// this path is that placing correctly and functioning are separate concerns.
fn already_stands(state: &PlanState, e: &BlueprintEntity, world: &Position) -> Standing {
    let Some(entity) = state.entity_at(world) else {
        return Standing::Nothing;
    };
    if entity.name != e.name || Pos::from(&entity.position) != Pos::from(world) {
        return Standing::Nothing;
    }
    if entity.direction == e.direction && entity.underground_half == e.underground_half {
        return Standing::AsDesigned;
    }
    Standing::Differently {
        direction: (entity.direction, e.direction),
        half: (entity.underground_half, e.underground_half),
    }
}

/// Recover the anchor from GHOSTS -- one match is enough, unlike the vote
/// path below.
///
/// Answered live against Factorio 2.1.17 before this was written (see
/// `docs/superpowers/plans/2026-09-05-block-siting.md`'s Task 8, Step 1):
/// ghosts do not expire, and a real placement consumes the ghost beneath it
/// cleanly, so a ghost standing at a blueprint offset is exactly as reliable
/// a witness as the real entity it will become.
///
/// **Why no `satisfied >= 2` floor, where [`recover_anchor`]'s vote path
/// needs one.** That floor exists to refuse a coincidence: an unrelated
/// entity of the same name, built for some other purpose, standing at one of
/// this block's own offsets by chance. A ghost cannot be that coincidence --
/// [`ActionKind::StampGhosts`] is the only thing in this project that ever
/// creates one, and it always stamps the WHOLE block at once, so a single
/// surviving ghost of one of this block's entities means this exact block
/// was already sited here. There is nothing to outvote.
///
/// Matched on name (via [`FactorioEntity::ghost_name`], never `.name` --
/// see [`PlanState::ghosts_named_any`]'s own doc), tile, direction and
/// underground half, the same fields [`already_stands`] compares for a real
/// entity: a ghost standing at the right tile but facing the wrong way is
/// not evidence of anything this block designed, any more than a
/// wrongly-facing real entity is.
///
/// Deterministic: `bp.entities` is walked in blueprint order, and
/// `ghosts_named_any`'s buckets are sorted by `Pos` -- the first blueprint
/// entity (in list order) with a matching-orientation ghost anywhere wins,
/// which is a fixed answer regardless of iteration order anywhere else.
fn recover_anchor_from_ghosts(state: &PlanState, bp: &Blueprint) -> Option<Position> {
    let names: BTreeSet<String> = bp.entities.iter().map(|e| e.name.clone()).collect();
    let ghosts_by_name = state.ghosts_named_any(&names);
    if ghosts_by_name.is_empty() {
        return None;
    }
    for e in &bp.entities {
        let Some(candidates) = ghosts_by_name.get(&e.name) else {
            continue;
        };
        for ghost in candidates {
            if ghost.direction != e.direction || ghost.underground_half != e.underground_half {
                continue;
            }
            return Some(Position::new(
                ghost.position.x() - e.offset.x(),
                ghost.position.y() - e.offset.y(),
            ));
        }
    }
    None
}

/// The anchor this block is ALREADY sited at, read back off the ground.
///
/// Siting must not be recomputed on a replan. `Goal::Built`'s whole shape
/// assumes an anchor is stable -- "building it twice is a no-op rather than
/// a second factory" -- and a search that re-runs against a world we have
/// since built into can answer differently than it did last time. A block
/// half-built at site A would then restart at site B: two half-factories, no
/// error, and a production curve that still rises.
///
/// So the site is chosen exactly once, when the first entity goes down, and
/// every later expansion rediscovers it from the entities themselves. This
/// needs no new state and nothing to keep in sync, because `already_stands`
/// answers the question backwards: each standing entity that matches a
/// blueprint entity implies `standing.position - blueprint.offset`.
///
/// Scored by how many of the block's entities that candidate satisfies, so an
/// unrelated entity of the same name cannot outvote the block itself.
///
/// **A single match is refused outright, never merely outvoted.** A fresh
/// build has NOTHING standing at its real anchor, so a lone entity elsewhere
/// that happens to share one name -- a power pole built for an unrelated
/// purpose, say -- would otherwise be the only candidate in `votes` and win
/// by default, with nothing to outvote it. That is not hypothetical: a power
/// rig planted purely to unlock research (`test_world::with_steam_power`,
/// a `small-electric-pole` and a `steam-engine` with no relation to any
/// block) was read as one-sixth of `StarterSteamEngineBoiler` on a
/// perfectly empty site, and the plan silently placed five of its six
/// entities as if the sixth already stood -- while genuinely standing at
/// nowhere near the requested anchor. Requiring at least two corroborating
/// entities before trusting a recovery is what a single coincidence cannot
/// pass; it is also **the honest limit this now has**: a one-entity
/// blueprint can never be recovered (there is only ever one thing to match),
/// and a two-or-more block whose FIRST entity alone has been built is read
/// as nothing standing rather than as a one-entity partial build. Both are
/// the conservative wrong answer -- re-siting a block that is genuinely one
/// entity into its own build -- not the dangerous one this replaces.
///
/// Ties on the score are broken toward the LARGER key, not an arbitrary one:
/// a block whose own entities are evenly spaced (offsets `0, 3, 6, 9`)
/// standing only partly built (two of four, spaced by the same `3`) is
/// satisfied equally by several candidate anchors -- shifting the guess by
/// any multiple of that spacing re-lines-up the same two standing entities
/// against a different pair of offsets. The larger key is the one that
/// assigns the standing entities to the block's *earliest* offsets rather
/// than a later, coincidentally-matching pair, which is the answer a caller
/// who placed entities in blueprint order actually wants.
///
/// Votes are keyed by half-tile fixed point (`(x, y) * 2, rounded`), not by
/// `Pos`: `Pos::from` floors to `(i32, i32)`, which is lossy for a fact this
/// exact -- every legal Factorio entity centre is a multiple of 0.5, so an
/// anchor at 10.0 and one at 10.5 would collapse into the same bucket. This
/// is the identical shape of round-trip that once made mining fail for every
/// ore on every map while every test passed, because `Pos` floors resource
/// positions too.
fn recover_anchor(state: &PlanState, bp: &Blueprint) -> Option<Position> {
    // Ghosts first, and unconditionally -- see `recover_anchor_from_ghosts`'s
    // own doc for why one match is enough here where the vote path below
    // needs two.
    if let Some(anchor) = recover_anchor_from_ghosts(state, bp) {
        return Some(anchor);
    }
    // One traversal of the world for however many distinct names this block
    // has -- 7 for `FurnaceLine`'s 179 entities -- instead of one whole-world
    // scan per blueprint ENTITY (`entities_named` called 179 times, each a
    // full scan of `inner_tree()`). See `entities_named_any`'s own doc: this
    // is the fix for the measured ~95s `recover_anchor` cost on a
    // fixed-anchor `Site::At` far from anything, where nothing of the block
    // stands and the entire cost was 179 scans of a world with plenty in it.
    let names: BTreeSet<String> = bp.entities.iter().map(|e| e.name.clone()).collect();
    let by_name = state.entities_named_any(&names);
    // Early out: nothing of this block stands anywhere, which is the normal
    // case (a fresh build, or any replan before the first entity goes down).
    // `by_name` holds no empty buckets (see its own doc), so an empty map
    // here means every name came back with nothing -- cheap to check, and it
    // skips the candidate/satisfied sweep below entirely rather than running
    // it over zero candidates for the same answer.
    if by_name.is_empty() {
        return None;
    }
    let mut votes: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for e in &bp.entities {
        let Some(candidates) = by_name.get(&e.name) else {
            continue;
        };
        for candidate in candidates {
            // Candidate anchor: `e` is standing where `candidate` actually
            // is, so the block's anchor -- if this is really it -- is offset
            // back by `e.offset`.
            let anchor = Position::new(
                candidate.position.x() - e.offset.x(),
                candidate.position.y() - e.offset.y(),
            );
            let satisfied = bp
                .entities
                .iter()
                .filter(|b| {
                    matches!(
                        already_stands(state, b, &anchor.add(&b.offset)),
                        Standing::AsDesigned
                    )
                })
                .count();
            // >= 2, not > 0: see the doc above -- a single matching entity
            // is exactly the shape of coincidence this must refuse, not
            // merely risk losing a tie-break to.
            if satisfied >= 2 {
                let key = (
                    (anchor.x() * 2.0).round() as i64,
                    (anchor.y() * 2.0).round() as i64,
                );
                votes.insert(key, satisfied);
            }
        }
    }
    votes
        // Most entities satisfied wins; on a tie the larger key wins (see
        // the doc above) -- deterministic either way, so the answer does not
        // depend on iteration order.
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
        .map(|(key, _)| Position::new(key.0 as f64 / 2.0, key.1 as f64 / 2.0))
}

/// Rings outward from `seed`, first clear footprint wins.
///
/// Deterministic by construction: rings ascend, and within a ring tiles are
/// visited in `(x, y)` order. No RNG, no float comparison, no hash iteration —
/// the planner is pure, and a site that varied between two plans of the same
/// world would make every offline comparison meaningless.
///
/// The candidate test is `first_obstruction`, built on
/// [`PlanState::siting_occupant`] — a NARROWER predicate than the
/// `placement_occupant` `expand`'s own footprint pre-check uses, and
/// deliberately so: see `siting_occupant`'s own doc for why a character must
/// not be one of the things a search result depends on. The two are meant to
/// agree on everything durable; where they differ, it is this one difference
/// on purpose, not two copies drifting apart.
///
/// **`seed` must be replan-stable**, and so must every OTHER input the
/// search's outcome can depend on — a fact this doc used to get half right.
/// `recover_anchor` only trusts an anchor once two of the block's entities
/// stand (see its own doc), so a block with exactly one entity built recovers
/// nothing and falls back to this search. The seed itself: callers pass a
/// fixed reference — the world origin for `Site::Anywhere`, the caller's own
/// point for `Site::Near` — never anything that tracks where bots have
/// walked to, because a seed that moves between expansions (a roster
/// centroid, say) can re-order the rings and site the block a second time.
/// The obstacle set the seed is searched against: entities are added by this
/// plan and by the game, never removed by anything this method does, so
/// `first_obstruction` (which skips this block's own entities standing as
/// designed) sees every earlier ring still blocked on every later call.
/// **Characters are the one source that is NOT append-only** — a bystander
/// or one of this plan's own bots can stand in a candidate ring on one
/// expansion and be gone (or a different one arrived) on the next, entirely
/// outside this plan's control. `siting_occupant` is what keeps that from
/// reaching the result: by excluding characters from the candidate test
/// altogether, the only things the search can trip over are the sources that
/// truly are monotonic, and the guarantee above holds without needing
/// anything about where a person or a bot happens to be standing.
///
/// `pub`, matching `method::connect::connect_steps`: called from
/// `resolve_site` below, and exercised directly by this module's own tests.
pub fn search_site(
    state: &PlanState,
    bp: &Blueprint,
    seed: &Position,
    max_radius: i32,
) -> Result<Position, PlannerError> {
    let mut nearest: Option<String> = None;
    for radius in 0..=max_radius {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // Ring, not disc: skip what an inner radius already tried.
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let anchor = Position::new(seed.x() + dx as f64, seed.y() + dy as f64);
                match first_obstruction(state, bp, &anchor) {
                    None => return Ok(anchor),
                    Some(what) => {
                        if nearest.is_none() {
                            nearest = Some(what);
                        }
                    }
                }
            }
        }
    }
    Err(PlannerError::NoSiteFound {
        entities: bp.entities.len(),
        seed: format!("{seed}"),
        searched: max_radius,
        nearest_obstruction: nearest
            .unwrap_or_else(|| "nothing (the search bound was reached first)".to_string()),
    })
}

/// The first thing standing in this block's way at `anchor`, if any.
///
/// An entity already standing AS DESIGNED is not an obstruction — it is this
/// block, already partly built, which is exactly the case `recover_anchor`
/// hands here.
///
/// Asks [`PlanState::siting_occupant`], not `placement_occupant`: a
/// character is not durable ground, and this function's whole job is
/// choosing an anchor that stays chosen (see `search_site`'s doc).
/// `expand`'s own footprint pre-check uses the SAME predicate for exactly
/// this anchor, for exactly this reason -- a character must not be able to
/// veto ground this search already chose one line earlier. Where the anchor
/// instead came from the caller (`Site::At`) or from recovery (the block is
/// already partly built there, a fact about the world rather than a search
/// result), the pre-check still uses `placement_occupant` and still refuses
/// a character standing on it, by name: see `resolve_site`'s doc for which
/// is which.
fn first_obstruction(state: &PlanState, bp: &Blueprint, anchor: &Position) -> Option<String> {
    for e in &bp.entities {
        let world = anchor.add(&e.offset);
        if matches!(already_stands(state, e, &world), Standing::AsDesigned) {
            continue;
        }
        let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
        if let Some(occupant) = state.siting_occupant(&e.name, &world, facing) {
            return Some(occupant.to_string());
        }
    }
    if let Some(why) = drills_are_fed(state, bp, anchor) {
        return Some(why);
    }
    None
}

/// Does `area` cover a tile of some resource `drill` can actually extract?
///
/// Inverts the game's own rule ([`PlanState::extractors_for`]): a resource is
/// mined by the machines whose `resource_categories` list its own
/// `resource_category`. So for every resource name this world knows
/// ([`PlanState::resource_names`]), this checks whether `drill` is one of
/// that resource's extractors and, only then, whether `area` actually covers
/// a tile of it -- `covers_resource` walks every tile under the box, and
/// skipping it whenever the cheap category test alone already says no keeps
/// this affordable over a world with many resource kinds.
///
/// A resource whose capture predates `resource_category` reads `None` there
/// ([`PlanState::resource_category`]'s own doc), and `None` can never satisfy
/// this: there is nothing to match a drill's `resource_categories` against,
/// and treating an unresolved category as a match would be exactly the false
/// acceptance this whole check exists to avoid.
fn covers_resource_extractable_by(state: &PlanState, drill: &str, area: &Rect) -> bool {
    state.resource_names().iter().any(|resource| {
        state
            .resource_category(resource)
            .is_some_and(|category| state.extractors_for(&category).iter().any(|d| d == drill))
            && state.covers_resource(area, resource)
    })
}

/// The rectangle a mining drill named `name`, standing at `position`, can
/// actually pull ore from -- as opposed to
/// [`collision_area_facing`](PlanState::collision_area_facing), which is
/// only the ground it occupies.
///
/// Reads `FactorioEntityPrototype::mining_drill_radius` directly off the
/// world's own prototype table (`state.base().entity_prototypes`, not
/// `state.collision_area_facing`, which only ever answers about the
/// collision box). When the field is present, the area is a square
/// centred on `position`, **`direction` does not rotate it** -- a mining
/// drill's reach is the same distance on every side regardless of which way
/// it faces, unlike its collision box -- and sized by ceiling the *doubled*
/// radius to a whole number of tiles: `(2 * radius).ceil()`. Doubling
/// before ceiling, not ceiling the radius and doubling that, is the
/// difference between the right answer and a whole tile too generous, and
/// ceiling at all (rather than comparing the raw float to a footprint half
/// -width) is what keeps a burner drill's shaved-under-2 collision box
/// from reading as reach beyond itself -- see the doc on
/// `FactorioEntityPrototype::mining_drill_radius` for the two measurements
/// this was checked against (burner: 0.99 -> 2x2, same as its own
/// footprint; electric: 2.49 -> 5x5, one tile beyond its 3x3 footprint).
///
/// `None` on the prototype -- every capture before `mining_drill_radius`
/// existed, and any capture since that simply never asked -- means
/// *unknown reach*, never *zero reach*: this falls back to
/// [`collision_area_facing`](PlanState::collision_area_facing) exactly as
/// this function did before the field existed, rather than shrinking every
/// old dump's drills to a reach of nothing.
fn mining_area(
    state: &PlanState,
    name: &str,
    position: &Position,
    direction: Direction,
) -> Option<Rect> {
    let radius = state
        .base()
        .entity_prototypes
        .get(name)
        .and_then(|proto| proto.mining_drill_radius);
    match radius {
        Some(radius) => {
            let side = (radius * 2.0).ceil();
            let half = side / 2.0;
            Some(Rect::new(
                &Position::new(position.x() - half, position.y() - half),
                &Position::new(position.x() + half, position.y() + half),
            ))
        }
        None => state.collision_area_facing(name, position, direction),
    }
}

/// Does every mining drill in this block have ore it can actually extract,
/// standing at `anchor`?
///
/// Returns the reason it does not, or `None` when they all do.
///
/// Asks [`mining_area`] rather than the drill's own footprint, so a drill
/// whose `mining_drill_radius` reaches past its collision box (every
/// electric mining drill, one tile on every side) is fed by ore beside it
/// and not only ore under it. A drill with no known radius still gets the
/// old, conservative answer -- see [`mining_area`]'s own doc for why
/// *unknown* and *zero* must not be conflated.
fn drills_are_fed(state: &PlanState, bp: &Blueprint, anchor: &Position) -> Option<String> {
    for e in &bp.entities {
        if !state.stands_on_resources(&e.name) {
            continue;
        }
        let world = anchor.add(&e.offset);
        let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
        let Some(area) = mining_area(state, &e.name, &world, facing) else {
            continue;
        };
        if !covers_resource_extractable_by(state, &e.name, &area) {
            return Some(format!(
                "the {} at ({}, {}) would stand on no ore it can mine",
                e.name,
                world.x(),
                world.y()
            ));
        }
    }
    None
}

/// How far siting looks before refusing, in tiles.
///
/// 48 covers the whole starting area of a fresh map without making a failed
/// search scan 10,000 candidate anchors: the cost is O(radius^2) footprint
/// scans, and each scan is O(entities).
const SEARCH_RADIUS: i32 = 48;

/// The nearest tile of ore this block's own mining drills can extract,
/// measured from the world origin -- `None` when the block has no drill at
/// all, or when no drill in it can reach any resource this world knows about.
///
/// **Why the origin, and not a roster position.** A bot's position moves
/// between expansions; a resource patch and the origin do not. Measuring
/// from anywhere else would make this seed re-order between replans exactly
/// as a roster centroid would (see `search_site`'s own doc on why that is
/// unacceptable) -- the origin is the only reference point this crate has
/// that is guaranteed stable and does not require picking one drill's offset
/// over another's.
///
/// **Why nearest ore matters, not merely "some ore exists".** On the
/// benchmark map the nearest copper is 55 tiles from spawn while
/// [`SEARCH_RADIUS`] is 48: a copper-drill block seeded at the origin could
/// never reach it, scanning the whole bound and refusing with a message that
/// blames occupied ground rather than a search radius that stopped seven
/// tiles short. Seeding here instead collapses that search to a handful of
/// rings around the patch itself.
///
/// Ties across resource kinds are broken the same way
/// [`crate::method::util::nearest_resource_tile`] breaks ties within one
/// kind -- distance, then `(x, y)` -- so the answer depends only on the
/// world's resource layout, never on `resource_names`' iteration order.
fn nearest_ore_seed(state: &PlanState, bp: &Blueprint) -> Option<Position> {
    let origin = Position::new(0.0, 0.0);
    let mut drills: BTreeSet<&str> = BTreeSet::new();
    for e in &bp.entities {
        if state.stands_on_resources(&e.name) {
            drills.insert(e.name.as_str());
        }
    }
    if drills.is_empty() {
        return None;
    }
    let mut best: Option<Position> = None;
    for resource in state.resource_names() {
        let extractable = state.resource_category(&resource).is_some_and(|category| {
            state
                .extractors_for(&category)
                .iter()
                .any(|d| drills.contains(d.as_str()))
        });
        if !extractable {
            continue;
        }
        let Some(tile) = nearest_resource_tile(state, &resource, &origin, 1) else {
            continue;
        };
        best = Some(match best {
            None => tile,
            Some(current) => {
                let ordering = calculate_distance(&origin, &tile)
                    .total_cmp(&calculate_distance(&origin, &current))
                    .then(tile.x().total_cmp(&current.x()))
                    .then(tile.y().total_cmp(&current.y()));
                if ordering == std::cmp::Ordering::Less {
                    tile
                } else {
                    current
                }
            }
        });
    }
    best
}

/// Where this block goes, resolved in one fixed order -- and whether that
/// anchor was CHOSEN BY THE SEARCH (`true`) or is a fact the caller or the
/// world already settled (`false`).
///
/// That bit is what `expand`'s footprint pre-check uses to decide which
/// occupant predicate a character is checked against. `siting_occupant`
/// (used only when this returns `true`) is deliberately blind to characters
/// -- see `search_site`'s doc -- because a bystander standing in a candidate
/// ring must not veto a site the search would otherwise pick, and must not
/// make the search's answer depend on where bots happen to be standing. A
/// caller-chosen or recovered anchor carries no such promise: nothing chose
/// it FOR its ground being clear of bots, so a character actually standing
/// there is exactly the fact `placement_occupant` exists to name. Refusing
/// on `siting_occupant` too, for those two cases, would be the over-broad
/// fix -- it would make `Site::At` silently build a block on top of a bot's
/// own model. Getting this bit wrong in either direction reintroduces one of
/// the two failures this module exists to keep apart: refusing ground the
/// search itself just picked, or approving ground the caller (or the game)
/// needed to be told was occupied.
///
/// Recovery comes FIRST and unconditionally, even for `Site::At`: if the
/// block is already partly built, the ground outranks anything the caller
/// says, because the alternative is two half-blocks and no error. A
/// recovered anchor is always `false` here -- it is a fact about the world,
/// not a choice this expansion made.
///
/// `Site::Near(p)` searches from the caller's own point, which is stable by
/// construction -- it came in with the goal, not off a bot's current
/// position. `Site::Anywhere` seeds at the nearest ore patch this block's own
/// drills can extract, falling back to the world origin for a block with no
/// drill at all; both are stable across replans (see `nearest_ore_seed`'s own
/// doc), which a roster centroid is not. Both are `true`: the search chose
/// the anchor in this call.
fn resolve_site(
    state: &PlanState,
    bp: &Blueprint,
    site: &Site,
) -> Result<(Position, bool), PlannerError> {
    if let Some(recovered) = recover_anchor(state, bp) {
        return Ok((recovered, false));
    }
    match site {
        Site::At(p) => Ok((p.clone(), false)),
        Site::Near(p) => Ok((search_site(state, bp, p, SEARCH_RADIUS)?, true)),
        Site::Anywhere => {
            let seed = nearest_ore_seed(state, bp).unwrap_or_else(|| Position::new(0.0, 0.0));
            Ok((search_site(state, bp, &seed, SEARCH_RADIUS)?, true))
        }
    }
}

/// Build a designed block by hand, one band per bot.
pub struct BuildBlock;

impl Method for BuildBlock {
    fn name(&self) -> &'static str {
        "BuildBlock"
    }

    fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
        matches!(goal, Goal::Built { .. })
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Built { blueprint, site } = goal else {
            return Ok(Vec::new());
        };
        let bp: Blueprint = decode(blueprint).map_err(|e| PlannerError::BlueprintRefused {
            reason: format!("{e:?}"),
        })?;

        // A recovered anchor wins over anything the caller says: standing
        // entities are a fact about the world, and a `Site` is only ever a
        // hint about where to start looking. This is what stops a replan
        // from re-siting a block that is already partly built -- see
        // `recover_anchor`'s own doc. `resolve_site` fills in `Near`/
        // `Anywhere` from a stable seed when nothing is standing yet.
        //
        // `is_fresh_site` is checked separately (a second, identical call --
        // `recover_anchor` is pure and cheap, see its own doc) because it
        // answers a different question than `resolve_site` does: not "where
        // is this block", but "has anything -- ghost or real -- ever
        // confirmed a site for it before". That is exactly the one-time
        // window `ActionKind::StampGhosts` is emitted in: once a ghost
        // stands, `recover_anchor`'s ghost pass finds it on every later
        // expansion and this is never true again for this block.
        let is_fresh_site = recover_anchor(&ctx.state, &bp).is_none();
        let (anchor, sited_by_search) = resolve_site(&ctx.state, &bp, site)?;

        // This used to refuse the whole goal, by name, whenever it contained
        // an underground belt: neither `FactorioEntity` nor the mod's
        // `rcon_place_entity` could say which half of a pair was being
        // built, so the generic placement path below would have emitted the
        // SAME entity twice -- a run that places 100% correctly and connects
        // nothing, the failure this project has already paid for twice. Both
        // now carry `underground_half` (task 5), via `entity_for` below, so
        // the two halves place as the distinct entities they are and this
        // method no longer needs to know underground belts exist at all.

        // Only what is NOT already standing. This is what makes the goal
        // re-checkable on a replan and idempotent when built twice.
        //
        // An entity standing on the right tile facing the WRONG way is
        // neither: it is not built, and this method has no action that
        // rotates or removes it (`ActionKind` has `Place`, and its `Remove`
        // is an inventory slot, not an entity). That is the weaker of two
        // reasons this is refused rather than re-emitted. The one that
        // actually holds: a `Place` step's own `Condition::AreaFree` is
        // evaluated by the exact same predicate this pre-check's
        // `placement_occupant` uses (`is_area_free_facing` reduces to
        // `occupant_of(..).is_none()`, and `placement_occupant` calls
        // `occupant_of` directly) -- so a re-placement emitted over the
        // standing entity could never be scheduled, and would fail at
        // `schedule()` with exactly the opaque `PlannerError::ChainOwnerInfeasible`
        // this ground pre-check exists to replace with a named tile. Whether
        // the GAME itself would refuse a same-name re-placement was never
        // established, and is not the reason for this refusal: the one
        // checkable fact points the other way -- `rcon_place_entity`
        // (mods/BotBridge/control.lua:3522) passes `fast_replace = true`
        // with `build_check_type.manual`, so a live re-placement over a
        // wrong-facing entity may well succeed. So it is refused here, by
        // name, saying both facings. What must never happen again is the
        // third option: reading it as done.
        //
        // Wiring up the re-placement instead is not the two-line change it
        // looks like (a `Remove` step here plus a carve-out in the footprint
        // scan below for the entity being replaced): `Condition::AreaFree`
        // would still see the standing entity and refuse the `Place` at
        // schedule time, so the carve-out would have to reach `AreaFree`
        // too, or scheduling refuses it anyway.
        //
        // One more thing this refusal does not distinguish: a wrong-facing
        // entity and an unrelated obstacle (a tree, water, a footprint the
        // game already refused) both surface as
        // `PlannerError::BlockGroundOccupied` -- "there is a tree in the
        // way" and "the block is built wrong and nothing here can fix it"
        // share an error code. The message text says which; the variant
        // does not.
        let mut wanted: Vec<&BlueprintEntity> = Vec::new();
        for e in &bp.entities {
            let world = anchor.add(&e.offset);
            match already_stands(&ctx.state, e, &world) {
                Standing::AsDesigned => {}
                Standing::Nothing => wanted.push(e),
                Standing::Differently { direction, half } => {
                    return Err(PlannerError::BlockGroundOccupied {
                        entity: e.name.clone(),
                        tile: format!("({}, {})", world.x(), world.y()),
                        occupant: format!(
                            "a {} already stands there facing {} where the blueprint wants {}{}; \
                             this planner has no action that rotates or removes a standing \
                             entity, so it cannot be corrected from here",
                            e.name,
                            direction.0,
                            direction.1,
                            match half {
                                (standing, wanted) if standing != wanted => format!(
                                    ", and it is the {} half where the {} half was wanted",
                                    half_name(standing),
                                    half_name(wanted)
                                ),
                                _ => String::new(),
                            }
                        ),
                    });
                }
            }
        }
        if wanted.is_empty() {
            return Ok(Vec::new());
        }

        // **The fourth refusal: the ground itself.** The spec named it and it
        // was never built, so occupancy reached the caller as
        // `PlannerError::ChainOwnerInfeasible` out of `schedule()` -- an
        // internal scheduling verdict standing in for a fact about a tile.
        // Four runs across three anchors were spent distinguishing hypotheses
        // this answers in one line, and the note recording them still ends
        // unresolved. Scanned over the whole footprint BEFORE a single step
        // is emitted, so nothing half-plans; and only over `wanted`, since an
        // entity already standing as designed occupies its own tile.
        //
        // **The character predicate matches whoever chose this anchor.** When
        // `resolve_site` picked the ground itself (`sited_by_search`,
        // `Site::Near`/`Site::Anywhere` with nothing to recover), this uses
        // `siting_occupant` -- the same predicate the search already screened
        // every candidate ring with -- so a character cannot veto ground the
        // search chose one line earlier: the defect this module exists to
        // fix (see `resolve_site`'s own doc). For a caller-chosen `Site::At`
        // or a recovered anchor, this still uses `placement_occupant` and
        // still names a standing character, because in both of those cases
        // the ground was never screened for characters at all -- the caller
        // needs telling.
        for e in &wanted {
            let world = anchor.add(&e.offset);
            let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
            let occupant = if sited_by_search {
                ctx.state.siting_occupant(&e.name, &world, facing)
            } else {
                ctx.state.placement_occupant(&e.name, &world, facing)
            };
            if let Some(occupant) = occupant {
                return Err(PlannerError::BlockGroundOccupied {
                    entity: e.name.clone(),
                    tile: format!("({}, {})", world.x(), world.y()),
                    occupant: occupant.to_string(),
                });
            }
        }

        // Would building this block seal one of the bots into a pocket?
        //
        // **`method::blueprint` had no enclosure guard at all until now**,
        // while `method::assemble` has had one for its cells -- so the method
        // that builds the LARGEST blocks in this project (179 entities, for
        // `FurnaceLine`) was the one with no check, and the small cells were
        // guarded. A 27-entity block sited from the roster's own seed found
        // that gap live: the executor reported `the character is already
        // walled in here ... pocket_tiles=1.0`, a walk ended inside a
        // furnace's collision box, and the build stopped with 13 of 29 steps
        // never dispatched while still reporting `done=true`.
        //
        // The executor's own `pre_place` cannot cover this. It judges only the
        // character *doing* the placing (`world.players.get(&player)`), so one
        // bot walling in another is invisible to it -- and once a bot is
        // enclosed, every later placement reads as "already walled in, this
        // placement does not change that" and is allowed. Bystanders are the
        // planner's job, which is exactly what `enclosure::check` is for.
        //
        // Deliberately NOT fixed by making the search avoid characters: the
        // anchor must not depend on where a bot happens to stand, or a replan
        // moves the block every time somebody walks. That invariant has its
        // own test (`a_bystander_in_the_search_path_does_not_move_the_sited_
        // anchor`) and this fix preserves it -- the anchor is unchanged and
        // the bots walk, which is what `Site::At`'s refusal text has always
        // promised ("cleared by walking, not by moving the block").
        let evacuations = {
            let mut trial = ctx.state.fork();
            for e in &wanted {
                let world = anchor.add(&e.offset);
                let entity = entity_for(&ctx.state, e, &world);
                trial.create_entity(entity);
            }
            match crate::enclosure::check(&ctx.state, &trial, &anchor) {
                crate::enclosure::EnclosurePrevention::Clear => Vec::new(),
                crate::enclosure::EnclosurePrevention::Evacuate(evacuations) => evacuations,
                // Every way out of the pocket runs through the block itself,
                // so no walk can fix it and only not building here can. A
                // named refusal beats a build that reports done with a third
                // of its steps never dispatched.
                crate::enclosure::EnclosurePrevention::Refuse => {
                    return Err(PlannerError::BlockGroundOccupied {
                        entity: "the block".to_string(),
                        tile: format!("({}, {})", anchor.x(), anchor.y()),
                        occupant: "would seal a bot into a pocket with no way out that                                    does not run through the block itself"
                            .to_string(),
                    });
                }
            }
        };

        let owned: Vec<BlueprintEntity> = wanted.iter().map(|e| (*e).clone()).collect();
        // Sorted (`bot_ids` reads a `BTreeMap`'s keys), so band `i` naming
        // `roster[i]` is a deterministic, replan-stable assignment.
        let roster = ctx.state.bot_ids();
        let split = bands(&owned, roster.len().max(1));

        // Each band is bound to its own bot with `Step::Owned`, the same
        // machinery `assemble.rs`'s cell-charging does for a bot's own
        // materials. Unbound, every placement is `Actor::Role` with no chain
        // of its own, and the scheduler assigns greedily -- nothing then
        // stops two bots working the same corner, which is the whole
        // structural reason a band exists in the first place.
        let mut steps = Vec::with_capacity(split.iter().map(Vec::len).sum());

        // Every bystander this block would seal in walks clear before any of
        // its own entities go down -- the same shape `method::power` and
        // `method::assemble` use, ordered by the `Step::Link`s below rather
        // than by position in this vector.
        let evacuation_ids: Vec<ActionId> = evacuations
            .iter()
            .map(|evacuation| {
                let (step, id) = crate::method::util::evacuation_step(
                    ctx,
                    evacuation,
                    &format!("the block at {anchor}"),
                );
                steps.push(step);
                id
            })
            .collect();

        // The block's ghosts, stamped once, before any `Place` -- see
        // `ActionKind::StampGhosts`'s own doc. Not tied to any band's chain
        // (no `Actor::Role` condition or effect), so nothing here orders it
        // ahead of the placements by itself; the `Step::Link`s below do that
        // explicitly, the same way `method::power` orders an evacuation
        // ahead of every part of a plant.
        let stamp_id = is_fresh_site.then(|| {
            let id = ctx.ids.next();
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::StampGhosts {
                    blueprint: blueprint.clone(),
                    anchor: anchor.clone(),
                },
                pre: vec![],
                eff: vec![],
                duration: PLACE_TICKS,
                pinned: None,
                label: format!("stamp ghosts for the block at {anchor}"),
            })));
            id
        });
        let mut place_ids: Vec<ActionId> = Vec::new();

        for (band, indices) in split.iter().enumerate() {
            if indices.is_empty() {
                continue;
            }
            let bot = roster.get(band).copied().unwrap_or(ctx.chain_actor);
            let build = ctx.state.bot(bot).map(|b| b.build_distance).unwrap_or(10.0);

            // The bill, stated as `Goal::Have` subgoals -- the same pattern
            // `connect.rs` uses for its belt and inserter counts -- so the
            // existing shortfall machinery goes and gets what this band is
            // short of before the first placement, rather than refusing with
            // a bare `HasItem` precondition failure. Counted from `owned`,
            // which is already only what is NOT standing, so replanning a
            // block that is partly built bills only the remainder and never
            // double-counts what a previous expansion (or the live world)
            // already placed. A `BTreeMap` keeps the emission order -- and
            // so the `ActionId` allocation the tie-break in `schedule`
            // depends on -- alphabetical and deterministic rather than
            // hash-order.
            let mut bill: BTreeMap<String, u32> = BTreeMap::new();
            for idx in indices {
                *bill.entry(owned[*idx].name.clone()).or_insert(0) += 1;
            }
            let mut block = Vec::with_capacity(bill.len() + indices.len());
            for (item, count) in bill {
                block.push(Step::Subgoal(Goal::Have {
                    item,
                    count,
                    whose: Holder::Share(bot),
                }));
            }
            for idx in indices {
                let e = &owned[*idx];
                let world = anchor.add(&e.offset);
                let entity = entity_for(&ctx.state, e, &world);
                let note = format!("block band {band}");
                let step = place_step(ctx, entity, build, &note);
                if let Step::Act(action) = &step {
                    place_ids.push(action.id);
                }
                block.push(step);
            }
            steps.push(Step::Owned {
                whose: Holder::Share(bot),
                steps: block,
            });
        }
        if let Some(stamp_id) = stamp_id {
            for place_id in &place_ids {
                steps.push(Step::Link {
                    from: stamp_id,
                    to: *place_id,
                    lag: 0,
                });
            }
        }
        // An evacuation precedes every placement, not just its own band's:
        // the bot is being walked clear of the whole footprint, and any
        // entity of it could be the wall that traps them.
        for evacuation_id in &evacuation_ids {
            for place_id in &place_ids {
                steps.push(Step::Link {
                    from: *evacuation_id,
                    to: *place_id,
                    lag: 0,
                });
            }
            if let Some(stamp_id) = stamp_id {
                steps.push(Step::Link {
                    from: *evacuation_id,
                    to: stamp_id,
                    lag: 0,
                });
            }
        }
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::blueprint::{BlueprintEntity, UndergroundHalf};
    use factorio_bot_core::types::Position;

    /// Builds a real blueprint string (version byte, base64, zlib, JSON)
    /// around a tiny hand-written entity list -- the same shape
    /// `crates/core/tests/blueprint_decode.rs`'s own `encode_blueprint`
    /// helper builds. Needed here (rather than reusing one of the fixture
    /// `.txt` blueprints) because `Goal::Built` takes blueprint TEXT, not a
    /// `Blueprint` struct, and the siting tests below need a block with NO
    /// mining drill, so `Site::Anywhere` seeds the search at the world
    /// origin rather than an ore patch -- see `nearest_ore_seed`'s doc.
    fn encode_test_blueprint(entities: &[(&str, f64, f64)]) -> String {
        use base64::Engine;
        use std::io::Write;
        let entity_json: Vec<String> = entities
            .iter()
            .enumerate()
            .map(|(i, (name, x, y))| {
                format!(
                    r#"{{"entity_number":{},"name":"{name}","position":{{"x":{x},"y":{y}}},"direction":0}}"#,
                    i + 1
                )
            })
            .collect();
        let envelope = format!(
            r#"{{"blueprint":{{"version":1,"entities":[{}]}}}}"#,
            entity_json.join(",")
        );
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(envelope.as_bytes())
            .expect("in-memory zlib write cannot fail");
        let compressed = encoder.finish().expect("in-memory zlib finish cannot fail");
        let encoded = base64::engine::general_purpose::STANDARD.encode(compressed);
        format!("0{encoded}")
    }

    fn at(x: f64) -> BlueprintEntity {
        BlueprintEntity {
            name: "transport-belt".into(),
            offset: Position::new(x, 0.0),
            direction: 4,
            underground_half: None::<UndergroundHalf>,
        }
    }

    /// A fresh, empty world with one bot -- the state `recover_anchor`'s own
    /// tests build on, before any entity is stood on it.
    fn test_state() -> PlanState {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    fn at_named(x: f64, y: f64, name: &str) -> BlueprintEntity {
        BlueprintEntity {
            name: name.to_string(),
            offset: Position::new(x, y),
            direction: 0,
            underground_half: None::<UndergroundHalf>,
        }
    }

    fn stone_furnace_at(x: f64, y: f64) -> FactorioEntity {
        FactorioEntity::new_stone_furnace(&Position::new(x, y), Direction::North)
    }

    /// A stone furnace that can never be read as `Standing::AsDesigned`
    /// against `at_named`'s default direction (0, i.e. `Direction::North`).
    ///
    /// **Why not `stone_furnace_at`.** `first_obstruction` treats an entity
    /// standing exactly as a blueprint entity designs it (same name, same
    /// tile, same facing) as friendly ground, not an obstruction -- that is
    /// the whole point of the stability guarantee (Ruling A / the search
    /// tests below). `stone_furnace_at` places its furnace facing
    /// `Direction::North`, which is also `at_named`'s default `direction:
    /// 0`, so a single-entity blueprint's own designed entity is
    /// indistinguishable from that decoy: `search_site` would read it as
    /// "this block, already built here" and stop instantly, never stepping
    /// outward -- which silently defeats a test whose entire point is
    /// forcing the search past a blocked seed. Facing a different way makes
    /// it a genuine, unrelated obstacle instead.
    fn blocking_stone_furnace_at(x: f64, y: f64) -> FactorioEntity {
        FactorioEntity::new_stone_furnace(&Position::new(x, y), Direction::South)
    }

    /// A ghost of `name` at `(x, y)`, facing north (direction 0) -- the shape
    /// `ActionKind::StampGhosts`'s live dispatch produces and
    /// `recover_anchor_from_ghosts` reads back.
    ///
    /// **`name` is `"entity-ghost"`, never the real name** -- that separation
    /// (the real name lives in `ghost_name`) is what stops a ghost being read
    /// as a built entity by `already_stands`, and it is load-bearing, not
    /// incidental: faking a ghost as a same-named real entity would test
    /// nothing about the code path this exists to exercise.
    fn ghost_of(name: &str, x: f64, y: f64) -> FactorioEntity {
        FactorioEntity {
            name: crate::state::GHOST_ENTITY_NAME.to_string(),
            entity_type: crate::state::GHOST_ENTITY_NAME.to_string(),
            position: Position::new(x, y),
            ghost_name: Some(name.to_string()),
            ..Default::default()
        }
    }

    /// **Recovery, not re-siting.** Two of a four-furnace block stand at an
    /// anchor the caller never names again -- a replan must find them, not
    /// choose somewhere new. This is the failure `Goal::Built` exists to make
    /// unreachable: a block half-built at site A restarting at site B, with
    /// no error and a production curve that still rises.
    #[test]
    fn a_partly_built_block_recovers_its_own_anchor_and_does_not_move() {
        let bp = Blueprint {
            entities: vec![
                at_named(0.0, 0.0, "stone-furnace"),
                at_named(3.0, 0.0, "stone-furnace"),
                at_named(6.0, 0.0, "stone-furnace"),
                at_named(9.0, 0.0, "stone-furnace"),
            ],
            version: 0,
        };
        let mut state = test_state();
        // The block was sited at (20.5, 20.5) on a previous plan and two of
        // its furnaces got built before the replan.
        state.create_entity(stone_furnace_at(20.5, 20.5));
        state.create_entity(stone_furnace_at(23.5, 20.5));

        let recovered = recover_anchor(&state, &bp).expect("two standing furnaces imply an anchor");
        assert_eq!(Pos::from(&recovered), Pos::from(&Position::new(20.5, 20.5)));
    }

    /// **Task 8, Step 2's test, verbatim.** The whole point: the anchor is
    /// known before ANY real entity exists, which is exactly the window
    /// vote-based recovery cannot cover -- `recover_anchor`'s own vote path
    /// needs two standing entities of this block's name and finds none here,
    /// so a pass against the OLD code alone could never make this pass. Only
    /// a genuine ghost-aware pass can.
    #[test]
    fn a_ghost_recovers_the_anchor_with_no_real_entity_standing() {
        let bp = Blueprint {
            entities: vec![
                at_named(0.0, 0.0, "stone-furnace"),
                at_named(3.0, 0.0, "stone-furnace"),
            ],
            version: 0,
        };
        let mut state = test_state();
        state.create_entity(ghost_of("stone-furnace", 20.5, 20.5));

        let recovered = recover_anchor(&state, &bp).expect("a ghost is a site marker");
        assert_eq!(Pos::from(&recovered), Pos::from(&Position::new(20.5, 20.5)));
    }

    /// The negative half of the test above: a ghost of the WRONG name, or one
    /// facing the wrong way, is not evidence of anything this block designed
    /// -- exactly as a wrongly-facing real entity is not, in `already_stands`.
    #[test]
    fn a_ghost_of_the_wrong_name_or_facing_does_not_recover_an_anchor() {
        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "stone-furnace")],
            version: 0,
        };

        let mut wrong_name = test_state();
        wrong_name.create_entity(ghost_of("wooden-chest", 20.5, 20.5));
        assert!(
            recover_anchor(&wrong_name, &bp).is_none(),
            "a ghost of an unrelated entity must not recover this block's anchor"
        );

        let mut wrong_facing = test_state();
        let mut furnace_ghost = ghost_of("stone-furnace", 20.5, 20.5);
        furnace_ghost.direction = 8; // south; the blueprint entity above defaults to 0 (north)
        wrong_facing.create_entity(furnace_ghost);
        assert!(
            recover_anchor(&wrong_facing, &bp).is_none(),
            "a ghost facing the wrong way is not this block, already sited"
        );
    }

    /// **`already_stands` must never read a ghost as the real thing** -- the
    /// separation `ghost_name` exists to preserve. A ghost standing exactly
    /// where a blueprint entity wants to be must still be reported as
    /// nothing built, or a replan would think this block finished without a
    /// single real entity on the ground.
    #[test]
    fn a_ghost_is_not_read_as_an_already_standing_entity() {
        let mut state = test_state();
        state.create_entity(ghost_of("stone-furnace", 0.5, 0.5));
        let e = at_named(0.0, 0.0, "stone-furnace");
        assert_eq!(
            already_stands(&state, &e, &Position::new(0.5, 0.5)),
            Standing::Nothing,
            "a ghost is a marker, not a built entity"
        );
    }

    /// **`BuildBlock::expand` stamps ghosts exactly once per block**: the
    /// first expansion against a fresh site emits `ActionKind::StampGhosts`
    /// ahead of every `Place`, linked to each of them; a replan against the
    /// SAME block -- now recoverable by the ghost this expansion just
    /// stamped -- must not emit a second one, or every replan would re-stamp
    /// (and, live, re-dispatch `rcon_place_blueprint` over ground this block
    /// already occupies).
    #[test]
    fn the_stamp_is_emitted_once_and_linked_ahead_of_every_place() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint_text = include_str!("../../../core/tests/blueprints/furnace_line.txt")
            .trim()
            .to_string();
        let bp: Blueprint = decode(&blueprint_text).expect("the fixture blueprint decodes");

        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );
        let goal = Goal::Built {
            blueprint: blueprint_text.clone(),
            site: Site::At(Position::new(100.5, 100.5)),
        };
        let first = BuildBlock
            .expand(&goal, &mut ctx)
            .expect("a fresh block plans");

        let stamps: Vec<&Action> = first
            .iter()
            .filter_map(|s| match s {
                Step::Act(a) if matches!(a.kind, ActionKind::StampGhosts { .. }) => {
                    Some(a.as_ref())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            stamps.len(),
            1,
            "exactly one stamp for a fresh block, not one per entity or per band"
        );
        let stamp_id = stamps[0].id;

        let place_ids: BTreeSet<crate::ids::ActionId> = first
            .iter()
            .flat_map(|s| match s {
                Step::Owned { steps, .. } => steps.as_slice(),
                _ => &[],
            })
            .filter_map(|s| match s {
                Step::Act(a) if matches!(a.kind, ActionKind::Place { .. }) => Some(a.id),
                _ => None,
            })
            .collect();
        assert!(!place_ids.is_empty(), "the fixture places real entities");

        let links: BTreeSet<crate::ids::ActionId> = first
            .iter()
            .filter_map(|s| match s {
                Step::Link { from, to, .. } if *from == stamp_id => Some(*to),
                _ => None,
            })
            .collect();
        assert_eq!(
            links, place_ids,
            "the stamp must be linked ahead of every place this expansion emits, not merely some"
        );

        // The stamp's own blueprint records what `entity_for` will place --
        // decoding it back must recover the same entity count as the fixture.
        let ActionKind::StampGhosts {
            blueprint: stamped_text,
            anchor: stamped_anchor,
        } = &stamps[0].kind
        else {
            panic!("filtered on StampGhosts above");
        };
        let restamped: Blueprint = decode(stamped_text).expect("the stamped text still decodes");
        assert_eq!(restamped.entities.len(), bp.entities.len());
        assert_eq!(
            Pos::from(stamped_anchor),
            Pos::from(&Position::new(100.5, 100.5))
        );

        // Now apply the stamp's own effect on a fresh state -- ghosts of
        // every entity, standing -- and replan. The second expansion must
        // recover the SAME anchor via the ghost pass and must NOT emit a
        // second stamp.
        let mut resumed = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        for e in &bp.entities {
            let world = Position::new(100.5 + e.offset.x(), 100.5 + e.offset.y());
            resumed.create_entity(ghost_of(&e.name, world.x(), world.y()));
        }
        let mut ctx2 = ExpansionCtx::new(resumed, BotId(1));
        let second = BuildBlock
            .expand(&goal, &mut ctx2)
            .expect("a replan still plans");
        let second_stamps = second
            .iter()
            .filter(
                |s| matches!(s, Step::Act(a) if matches!(a.kind, ActionKind::StampGhosts { .. })),
            )
            .count();
        assert_eq!(
            second_stamps, 0,
            "a block the ghost pass can already recover must not be re-stamped"
        );
    }

    /// **Ruling B: recovery outranks even an explicit `Site::At`.** A caller
    /// naming an anchor is a hint about where to start looking, not a fact --
    /// standing entities are the fact. A goal replanned with the SAME
    /// explicit anchor it was first built with must still resolve to where
    /// the block actually stands if that has drifted from the caller's own
    /// number (a stale goal, a hand-edited script), or the run ends up
    /// building two half-blocks in two different places with no error.
    #[test]
    fn resolve_site_prefers_the_recovered_anchor_over_an_explicit_site_at() {
        let bp = Blueprint {
            entities: vec![
                at_named(0.0, 0.0, "stone-furnace"),
                at_named(3.0, 0.0, "stone-furnace"),
                at_named(6.0, 0.0, "stone-furnace"),
                at_named(9.0, 0.0, "stone-furnace"),
            ],
            version: 0,
        };
        let mut state = test_state();
        // Actually standing at (20.5, 20.5) ...
        state.create_entity(stone_furnace_at(20.5, 20.5));
        state.create_entity(stone_furnace_at(23.5, 20.5));

        // ... but the caller names a different anchor entirely.
        let site = Site::At(Position::new(0.5, 0.5));

        let (resolved, sited_by_search) =
            resolve_site(&state, &bp, &site).expect("recovery answers even for At");
        assert_eq!(
            Pos::from(&resolved),
            Pos::from(&Position::new(20.5, 20.5)),
            "the ground outranks the caller's explicit anchor: building at \
             (0.5, 0.5) here would start a second, unrelated furnace line"
        );
        assert!(
            !sited_by_search,
            "a recovered anchor is a fact about the world, not something \
             this call's search chose"
        );
    }

    #[test]
    fn a_block_with_nothing_standing_recovers_no_anchor() {
        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "stone-furnace")],
            version: 0,
        };
        let state = test_state();
        assert!(recover_anchor(&state, &bp).is_none());
    }

    /// A decoy furnace unrelated to the block stands alone; the block's own
    /// two furnaces stand together. The pair must outvote the single.
    #[test]
    fn the_anchor_satisfying_the_most_entities_wins() {
        let bp = Blueprint {
            entities: vec![
                at_named(0.0, 0.0, "stone-furnace"),
                at_named(3.0, 0.0, "stone-furnace"),
            ],
            version: 0,
        };
        let mut state = test_state();
        state.create_entity(stone_furnace_at(-40.5, -40.5)); // decoy
        state.create_entity(stone_furnace_at(10.5, 10.5));
        state.create_entity(stone_furnace_at(13.5, 10.5));

        let recovered = recover_anchor(&state, &bp).expect("the pair implies an anchor");
        assert_eq!(Pos::from(&recovered), Pos::from(&Position::new(10.5, 10.5)));
    }

    /// The ring search itself: blocked at the seed tile, it must step
    /// outward to the first clear footprint, and answer the same way twice.
    #[test]
    fn a_block_is_sited_on_the_first_clear_ring_and_is_deterministic() {
        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "stone-furnace")],
            version: 0,
        };
        let mut state = test_state();
        // Block the seed tile itself, so the search must step outward. Faced
        // away from the blueprint's own (default) direction -- see
        // `blocking_stone_furnace_at`'s doc for why a same-facing furnace
        // would not do.
        state.create_entity(blocking_stone_furnace_at(0.5, 0.5));

        let first =
            search_site(&state, &bp, &Position::new(0.5, 0.5), 20).expect("open ground exists");
        let again = search_site(&state, &bp, &Position::new(0.5, 0.5), 20).expect("same answer");
        assert_eq!(
            Pos::from(&first),
            Pos::from(&again),
            "siting must be deterministic"
        );
        assert_ne!(Pos::from(&first), Pos::from(&Position::new(0.5, 0.5)));
    }

    /// A search bounded and refused must say how far it looked and what was
    /// in the way -- "cannot site" and "looked one tile" must not read alike.
    #[test]
    fn a_search_that_finds_nothing_says_how_far_it_looked() {
        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "stone-furnace")],
            version: 0,
        };
        let mut state = test_state();
        // Wall off every tile within the search bound, faced away from the
        // blueprint's own direction so none of them read as this block
        // already standing (see `blocking_stone_furnace_at`'s doc).
        for x in -3..=3 {
            for y in -3..=3 {
                state.create_entity(blocking_stone_furnace_at(x as f64 + 0.5, y as f64 + 0.5));
            }
        }
        let err = search_site(&state, &bp, &Position::new(0.5, 0.5), 2).unwrap_err();
        let text = format!("{err}");
        assert!(
            text.contains('2'),
            "the refusal must say how far it searched: {text}"
        );
        assert!(
            text.contains("stone-furnace"),
            "the refusal must name what is in the way: {text}"
        );
    }

    /// **A character in the search path must not move the sited anchor.**
    ///
    /// `occupant_of`'s six sources include live characters, and until this
    /// fix `first_obstruction` (via `placement_occupant`) saw them like any
    /// other obstacle. A character is the one source among those six that
    /// moves with no plan action behind it at all -- a bystander (or one of
    /// this plan's own bots) can stand in a candidate ring on one expansion
    /// and be gone on the next, entirely outside what this plan controls.
    /// That reaches the same two-half-factories failure `search_site`'s doc
    /// already worried about for a moving SEED, but through a moving
    /// OBSTACLE instead: an unrelated bot blocks the nearest ring, the
    /// search steps outward, the bot walks off, and a later expansion (a
    /// fresh `PlanState` off a later world snapshot) finds the near ring
    /// clear and sites the block a second time.
    ///
    /// This proves the fix two ways: the anchor a bystander-free search picks
    /// is unchanged once a bystander is standing exactly on it, AND the
    /// ordinary placement predicate (what `expand`'s own footprint pre-check
    /// uses once a block is actually being built) still sees that same
    /// bystander as a real occupant -- so this is not "characters became
    /// invisible everywhere", only "siting stopped depending on them".
    #[test]
    fn a_bystander_in_the_search_path_does_not_move_the_sited_anchor() {
        use crate::ids::BotId;
        use crate::state::Occupant;
        use factorio_bot_core::test_utils::fixture_world;
        use factorio_bot_core::types::FactorioPlayer;
        use std::sync::Arc;

        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "stone-furnace")],
            version: 0,
        };
        let seed = Position::new(0.5, 0.5);

        // Baseline: nobody standing anywhere.
        let empty = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let baseline = search_site(&empty, &bp, &seed, 10).expect("open ground exists");

        // A bystander -- NOT this plan's own bot -- stands exactly on the
        // tile the baseline search chose, as if it had walked there between
        // one expansion and the next (modelled here as a second, later world
        // snapshot, which is how two real expansions actually differ).
        let occupied_world = fixture_world();
        occupied_world.players.insert(
            99,
            FactorioPlayer {
                player_id: 99,
                position: baseline.clone(),
                build_distance: 10,
                reach_distance: 10,
                resource_reach_distance: 4.0,
                ..Default::default()
            },
        );
        let occupied = PlanState::from_world(Arc::new(occupied_world), &[BotId(1)]);

        // The ordinary placement predicate still sees the bystander -- this
        // is a genuine occupant, not an empty test.
        assert!(
            matches!(
                occupied.placement_occupant("stone-furnace", &baseline, Direction::North),
                Some(Occupant::Character {
                    on_roster: false,
                    ..
                })
            ),
            "the bystander must be a real occupant by the placement predicate \
             `expand` uses, or this test proves nothing"
        );

        let with_bystander = search_site(&occupied, &bp, &seed, 10)
            .expect("a bystander does not make siting fail, only irrelevant");
        assert_eq!(
            Pos::from(&baseline),
            Pos::from(&with_bystander),
            "a bystander standing in the search path must not move the sited \
             anchor: a character walks away with no plan action behind it, \
             so it cannot be part of what a stable search depends on"
        );
    }

    /// A world with `iron-ore`/`electric-mining-drill` prototypes that know
    /// their `resource_category`/`resource_categories` -- the shared
    /// `fixture_world` predates both fields entirely
    /// (`crates/core/tests/entity-prototype-fixtures.json` has no
    /// `resource_categor` anywhere in it), so this sets them the same way
    /// `test_world::world_with_oil`'s `categories` fixture and
    /// `have.rs`'s `a_resource_category_the_character_does_not_mine_refuses_by_category`
    /// already do for the same gap.
    ///
    /// Used, unmodified, as the base for BOTH the bare-ground and
    /// ore-covered cases below, so the only difference between them is
    /// whether ore actually sits on the ground -- not whether the category
    /// data exists to judge it by.
    fn drill_world() -> factorio_bot_core::factorio::world::FactorioWorld {
        use factorio_bot_core::test_utils::fixture_world;

        let world = fixture_world();
        world
            .entity_prototypes
            .get_mut("iron-ore")
            .expect("the fixture has an iron-ore prototype")
            .resource_category = Some("basic-solid".to_string());
        world
            .entity_prototypes
            .get_mut("electric-mining-drill")
            .expect("the fixture has an electric-mining-drill prototype")
            .resource_categories = Some(vec!["basic-solid".to_string()]);
        world
    }

    /// **A drill over bare ground places perfectly and mines nothing.**
    /// `MinerLine` is 13 `electric-mining-drill`s, so refusing a site with no
    /// ore under any of them is not caution, it is the whole point of this
    /// task.
    ///
    /// **Ore goes into the BASE world via `update_chunk_entities`, never
    /// into the plan overlay via `PlanState::create_entity`.**
    /// `covers_resource`/`resource_available` read `self.base.entity_graph`
    /// only; `create_entity` writes solely into the overlay's `added` map,
    /// which those two never consult (it exists for entities THIS plan
    /// places, not for resources the map already has). An ore entity handed
    /// to `create_entity` would be invisible to every check this test
    /// exists to exercise, and the "ored" case below would refuse for
    /// exactly the same reason as "bare" -- the opposite of a test that
    /// would catch a wrong answer.
    ///
    /// **Ore at tile CENTRES**, as every real resource entity is
    /// (`(-40.5, -48.5)`, never `(-41, -49)`) -- `FactorioEntity::new_resource`
    /// is used rather than a bare `FactorioEntity { .. Default::default() }`
    /// literal, because the latter leaves `entity_type` empty and
    /// `EntityGraph::add` only routes an entity into the resource tree when
    /// `entity_type == "resource"`; a default-typed entity would silently
    /// land in the ordinary obstacle tree instead of being seen as ore at
    /// all.
    #[test]
    fn a_drill_block_is_refused_on_bare_ground_and_accepted_over_ore() {
        use crate::ids::BotId;
        use std::sync::Arc;

        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "electric-mining-drill")],
            version: 0,
        };

        let bare = PlanState::from_world(Arc::new(drill_world()), &[BotId(1)]);
        assert!(
            search_site(&bare, &bp, &Position::new(0.5, 0.5), 3).is_err(),
            "a drill over no ore at all must be refused, not sited"
        );

        let ored_world = drill_world();
        let mut ore = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                ore.push(FactorioEntity::new_resource(
                    &Position::new(2.5 + dx as f64, 2.5 + dy as f64),
                    Direction::North,
                    "iron-ore",
                ));
            }
        }
        ored_world
            .update_chunk_entities(ore)
            .expect("a fixture world accepts its own ore");
        let ored = PlanState::from_world(Arc::new(ored_world), &[BotId(1)]);

        let sited = search_site(&ored, &bp, &Position::new(0.5, 0.5), 6)
            .expect("a drill must be sited onto the ore patch");
        let area = ored
            .collision_area_facing("electric-mining-drill", &sited, Direction::North)
            .expect("the drill has a collision box");
        assert!(
            ored.covers_resource(&area, "iron-ore"),
            "the chosen site {sited} does not cover ore"
        );
    }

    /// Places one iron-ore tile at `ore_offset` tiles from `drill_pos`
    /// (both in whole tiles) and returns a world where an
    /// `electric-mining-drill` or `burner-mining-drill` at `drill_pos`,
    /// with `radius` as its `mining_drill_radius`, can be asked whether it
    /// is fed.
    ///
    /// **Ore at a tile CENTRE** (`drill_pos + ore_offset`, itself built from
    /// whole-tile inputs so the centre lands on a half-integer) -- never an
    /// integer position. `EntityGraph` keys resources by a flooring `Pos`,
    /// so an integer position is the one input for which that round-trip is
    /// lossless and would prove nothing about a real map; this is the exact
    /// mistake the task brief calls out.
    fn drill_reach_world(
        drill_name: &str,
        drill_pos: Position,
        radius: Option<f64>,
        ore_tile_offset: (i32, i32),
    ) -> (factorio_bot_core::factorio::world::FactorioWorld, Position) {
        let world = drill_world();
        {
            let mut proto = world
                .entity_prototypes
                .get_mut(drill_name)
                .unwrap_or_else(|| panic!("the fixture has a {drill_name} prototype"));
            proto.resource_categories = Some(vec!["basic-solid".to_string()]);
            proto.mining_drill_radius = radius;
        }
        let ore_center = Position::new(
            (drill_pos.x().floor() as i32 + ore_tile_offset.0) as f64 + 0.5,
            (drill_pos.y().floor() as i32 + ore_tile_offset.1) as f64 + 0.5,
        );
        world
            .update_chunk_entities(vec![FactorioEntity::new_resource(
                &ore_center,
                Direction::North,
                "iron-ore",
            )])
            .expect("a fixture world accepts its own ore");
        (world, drill_pos)
    }

    /// **The behaviour change this task exists to make.** An electric
    /// mining drill's `mining_drill_radius` (2.49, measured live) reaches a
    /// full tile past its 3x3 collision box, so ore one tile beyond the
    /// footprint -- covered by neither `collision_area_facing` nor the old,
    /// footprint-only `drills_are_fed` -- must now read as fed.
    ///
    /// The drill sits at (0.5, 0.5), a tile CENTRE, which is the correct
    /// alignment for its odd (3x3) footprint (`method::util::tile_alignment`
    /// -- an even footprint sits on a tile boundary, an odd one on a tile
    /// centre). Its footprint then spans tiles x,y in {-1, 0, 1}; ore at
    /// tile (2, 0) -- one tile outside that box on the east side -- is
    /// inside the 5x5 mining area (tiles {-2..=2}) but not the footprint.
    ///
    /// **This must fail without the fix**: reverting `drills_are_fed` to
    /// ask `collision_area_facing` instead of `mining_area` refuses this
    /// site, because ore at tile (2, 0) is outside the 3x3 footprint.
    #[test]
    fn an_electric_drill_is_fed_by_ore_adjacent_but_not_underneath() {
        let (world, drill_pos) = drill_reach_world(
            "electric-mining-drill",
            Position::new(0.5, 0.5),
            Some(2.49),
            (2, 0),
        );
        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "electric-mining-drill")],
            version: 0,
        };
        let state = PlanState::from_world(std::sync::Arc::new(world), &[crate::ids::BotId(1)]);
        assert_eq!(
            drills_are_fed(&state, &bp, &drill_pos),
            None,
            "an electric drill's mining area reaches one tile past its own \
             footprint and must be fed by ore sitting there"
        );
    }

    /// **The burner drill's own reach does not widen anything.** Its
    /// `mining_drill_radius` (0.99, measured live) is, doubled and ceiled,
    /// exactly its own 2x2 footprint -- see the table in this function's
    /// module-level doc and in `FactorioEntityPrototype::mining_drill_radius`.
    /// Ore placed one tile beyond a burner drill the same way the electric
    /// drill's test places it must still refuse.
    ///
    /// The drill sits at (2.0, 2.0), an integer position, which is the
    /// correct alignment for its even (2x2) footprint. Its footprint (and,
    /// since the radius does not widen it, its mining area) spans tiles
    /// x, y in {1, 2}; ore at tile (3, 2) -- one tile past the east edge --
    /// is outside both.
    #[test]
    fn a_burner_drill_is_not_fed_by_ore_adjacent_but_not_underneath() {
        let (world, drill_pos) = drill_reach_world(
            "burner-mining-drill",
            Position::new(2.0, 2.0),
            Some(0.99),
            (1, 0),
        );
        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "burner-mining-drill")],
            version: 0,
        };
        let state = PlanState::from_world(std::sync::Arc::new(world), &[crate::ids::BotId(1)]);
        assert!(
            drills_are_fed(&state, &bp, &drill_pos).is_some(),
            "a burner drill's mining area is its own footprint in tiles, so \
             ore one tile beyond it must still be refused"
        );
    }

    /// **`None` means unknown reach, never zero reach.** Every dump written
    /// before `mining_drill_radius` existed carries `null` for the field;
    /// treating that as zero would make this change silently MORE
    /// conservative than the code it replaces on every one of them. With no
    /// radius the check must fall back to exactly the old footprint
    /// behaviour: ore under the footprint feeds the drill, and this must
    /// hold even at the same off-footprint offset the radius-aware test
    /// above uses (proving the fallback does not fabricate a radius from
    /// thin air either).
    #[test]
    fn a_drill_with_unknown_radius_falls_back_to_its_footprint_not_zero_reach() {
        let (world, drill_pos) = drill_reach_world(
            "electric-mining-drill",
            Position::new(0.5, 0.5),
            None,
            (0, 0),
        );
        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "electric-mining-drill")],
            version: 0,
        };
        let state = PlanState::from_world(std::sync::Arc::new(world), &[crate::ids::BotId(1)]);
        assert_eq!(
            drills_are_fed(&state, &bp, &drill_pos),
            None,
            "ore under the footprint must still feed a drill with unknown \
             mining_drill_radius -- None is not the same as a real reach of \
             zero, but it is also not a licence to treat the footprint as \
             unfed"
        );

        let (world_adjacent, drill_pos) = drill_reach_world(
            "electric-mining-drill",
            Position::new(0.5, 0.5),
            None,
            (2, 0),
        );
        let state_adjacent =
            PlanState::from_world(std::sync::Arc::new(world_adjacent), &[crate::ids::BotId(1)]);
        assert!(
            drills_are_fed(&state_adjacent, &bp, &drill_pos).is_some(),
            "with no known radius, ore one tile past the footprint must \
             still be refused -- unknown reach is handled conservatively, \
             it is not silently widened"
        );
    }

    /// **Ruling A's reason for existing, made concrete.** On the benchmark
    /// map the nearest copper is 55 tiles from spawn while [`SEARCH_RADIUS`]
    /// is 48: a drill block seeded at the world origin can never reach it --
    /// the search exhausts its whole bound without the anchor ever landing
    /// on ore. Ore placed here more than `SEARCH_RADIUS` from the origin
    /// reproduces exactly that shape, so `resolve_site` under `Site::Anywhere`
    /// only succeeds at all if it seeds `search_site` at the ore patch
    /// itself, not at the origin.
    #[test]
    fn a_drill_block_anywhere_is_seeded_at_ore_the_origin_could_never_reach() {
        use crate::ids::BotId;
        use std::sync::Arc;

        let bp = Blueprint {
            entities: vec![at_named(0.0, 0.0, "electric-mining-drill")],
            version: 0,
        };

        let world = drill_world();
        // Centred well past SEARCH_RADIUS (48) from the origin in x alone --
        // a search seeded at (0, 0) could not place even one ring on it.
        let mut ore = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                ore.push(FactorioEntity::new_resource(
                    &Position::new(60.5 + dx as f64, 0.5 + dy as f64),
                    Direction::North,
                    "iron-ore",
                ));
            }
        }
        world
            .update_chunk_entities(ore)
            .expect("a fixture world accepts its own ore");
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        let seed = nearest_ore_seed(&state, &bp).expect("the drill block has ore to seed from");
        assert!(
            calculate_distance(&Position::new(0.0, 0.0), &seed) > SEARCH_RADIUS as f64,
            "the seed must be far enough from the origin that an origin-seeded \
             search could never have reached it: {seed}"
        );

        let (sited, _) = resolve_site(&state, &bp, &Site::Anywhere)
            .expect("seeding at the ore patch puts the far-away ore within reach");
        let area = state
            .collision_area_facing("electric-mining-drill", &sited, Direction::North)
            .expect("the drill has a collision box");
        assert!(
            state.covers_resource(&area, "iron-ore"),
            "the chosen site {sited} does not cover ore"
        );
    }

    /// **Guardrail for `recover_anchor`'s `satisfied >= 2` floor.** A single
    /// standing entity that happens to sit at one of this block's own offsets
    /// must NOT be read as an anchor -- that is exactly the coincidence the
    /// floor exists to refuse (see the doc on `recover_anchor` and
    /// `the_anchor_satisfying_the_most_entities_wins` above, where an
    /// unrelated single entity nearly won by default).
    ///
    /// If this test ever starts failing because recovery got demonstrably
    /// BETTER -- some new, provably safe way to trust a single match -- that
    /// is fine. If it fails because the threshold was simply lowered without
    /// re-deriving the safety argument, the bug it prevents comes back: a
    /// block with exactly one entity built would recover an anchor from a
    /// stray match, and a genuinely one-entity block would get "confirmed"
    /// rather than searched.
    #[test]
    fn one_standing_entity_is_not_enough_to_recover_an_anchor() {
        let bp = Blueprint {
            entities: vec![
                at_named(0.0, 0.0, "stone-furnace"),
                at_named(3.0, 0.0, "stone-furnace"),
                at_named(6.0, 0.0, "stone-furnace"),
                at_named(9.0, 0.0, "stone-furnace"),
            ],
            version: 0,
        };
        let mut state = test_state();
        // Exactly ONE genuine entity of the block, standing at a real block
        // offset (its first) -- not a decoy elsewhere, and nothing else on
        // the ground.
        state.create_entity(stone_furnace_at(20.5, 20.5));

        assert!(
            recover_anchor(&state, &bp).is_none(),
            "one standing entity must not be trusted as an anchor: a lone \
             match is exactly the shape of coincidence `satisfied >= 2` is \
             meant to refuse, not merely risk losing a tie-break to"
        );
    }

    /// **The regression test for Ruling A: siting is stable across a partial
    /// build, from a seed that does not move.**
    ///
    /// `recover_anchor` cannot trust a block with exactly one entity built
    /// (see the guardrail above), so that block falls straight back into
    /// `search_site` on every replan. The search only answers the same way
    /// twice if the seed it is handed is the same both times -- a roster
    /// centroid moves as bots walk, which can re-order the rings and site the
    /// SAME block a second time, silently, with no error and a production
    /// curve that still rises. This resolves a site from a fixed seed, builds
    /// ONE of the block's entities at it, and re-searches from the identical
    /// seed: the anchor must not change, because placements only ever ADD
    /// obstacles and `first_obstruction` treats this block's own
    /// as-designed entities as clear ground rather than as something in its
    /// own way.
    #[test]
    fn the_search_is_stable_across_a_partial_build() {
        let bp = Blueprint {
            entities: vec![
                at_named(0.0, 0.0, "stone-furnace"),
                at_named(3.0, 0.0, "stone-furnace"),
                at_named(6.0, 0.0, "stone-furnace"),
                at_named(9.0, 0.0, "stone-furnace"),
            ],
            version: 0,
        };
        // The stable seed Ruling A mandates for `Site::Anywhere`: the world
        // origin, never the roster centroid.
        let seed = Position::new(0.0, 0.0);
        let mut state = test_state();

        let first = search_site(&state, &bp, &seed, 30).expect("open ground exists");

        // Build only the block's FIRST entity at the resolved anchor -- the
        // exact one-entity window the guardrail above shows `recover_anchor`
        // refuses to trust.
        let e = &bp.entities[0];
        let world = first.add(&e.offset);
        state.create_entity(entity_for(&state, e, &world));
        assert!(
            recover_anchor(&state, &bp).is_none(),
            "this test must exercise the SEARCH, not recovery -- one \
             standing entity is still not enough to recover an anchor"
        );

        let second = search_site(&state, &bp, &seed, 30).expect("still sites the same block");
        assert_eq!(
            Pos::from(&first),
            Pos::from(&second),
            "a partial build must not move the site: a stable seed plus a \
             monotonic obstacle set means the same anchor wins every time"
        );
    }

    /// **The regression test for this entire sub-project.** `resolve_site` is
    /// what `expand` actually calls, end to end -- recovery first, then the
    /// `Site` match -- for a goal whose caller asked for `Site::Anywhere` and
    /// gave no anchor at all. A block with exactly one entity built is
    /// exactly the window `recover_anchor` refuses to trust (see the
    /// guardrail above), so this exercises the real risk: if the seed
    /// `resolve_site` hands to `search_site` moved between the two calls (a
    /// roster centroid, say, which walks as bots do), the second call could
    /// re-order the search rings and site the SAME block a second time, with
    /// no error and a production curve that still rises.
    #[test]
    fn a_replan_after_partial_construction_keeps_the_same_site() {
        let bp = Blueprint {
            entities: vec![
                at_named(0.0, 0.0, "stone-furnace"),
                at_named(3.0, 0.0, "stone-furnace"),
                at_named(6.0, 0.0, "stone-furnace"),
                at_named(9.0, 0.0, "stone-furnace"),
            ],
            version: 0,
        };
        let mut state = test_state();
        let site = Site::Anywhere;

        let (first, _) = resolve_site(&state, &bp, &site).expect("a first site exists");

        // Build one entity of the block, as a real run would, then replan.
        let e = &bp.entities[0];
        let world = first.add(&e.offset);
        state.create_entity(entity_for(&state, e, &world));
        assert!(
            recover_anchor(&state, &bp).is_none(),
            "this must exercise resolve_site's SEARCH path on the second \
             call, not recovery -- one standing entity is still not enough \
             to recover an anchor"
        );

        // The roster walks, between the two expansions, to somewhere far
        // from where it started. This is the whole point of the test: a
        // `roster_centroid` seed would move with the bot and could re-order
        // the search rings on the second call. `test_state` seats exactly
        // `BotId(1)`, so moving it is moving the whole roster.
        use crate::ids::BotId;
        state.set_position(BotId(1), Position::new(500.0, 500.0));

        let (second, _) = resolve_site(&state, &bp, &site).expect("a second site exists");

        assert_eq!(
            Pos::from(&first),
            Pos::from(&second),
            "a partly-built block must not be re-sited: that builds it twice, in two places"
        );
    }

    /// Bands are balanced by ENTITY COUNT, not by area: a block whose entities
    /// bunch at one end must still divide into equal work, or one bot builds
    /// while three watch.
    #[test]
    fn bands_split_by_count_not_by_width() {
        // Twelve entities: nine crowded in x 0..3, three spread to x 40..42.
        let mut ents: Vec<BlueprintEntity> = Vec::new();
        for i in 0..9 {
            ents.push(at((i % 3) as f64));
        }
        for i in 0..3 {
            ents.push(at(40.0 + i as f64));
        }
        let bands = bands(&ents, 3);
        assert_eq!(bands.len(), 3);
        for b in &bands {
            assert_eq!(b.len(), 4, "each of 3 bots takes 4 of 12: {bands:?}");
        }
    }

    #[test]
    fn every_entity_lands_in_exactly_one_band() {
        let ents: Vec<BlueprintEntity> = (0..10).map(|i| at(i as f64)).collect();
        let bands = bands(&ents, 4);
        let mut seen: Vec<usize> = bands.iter().flatten().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn bands_are_deterministic() {
        let ents: Vec<BlueprintEntity> = (0..17).map(|i| at((i % 5) as f64)).collect();
        assert_eq!(bands(&ents, 4), bands(&ents, 4));
    }

    /// The `FurnaceLine` fixture (`crates/core/tests/blueprints/furnace_line.txt`)
    /// carries one underground-belt pair -- `an_underground_belt_carries_which_half_it_is`
    /// in `crates/core/tests/blueprint_decode.rs` pins that the decoder reports
    /// both halves. `expand()` used to refuse the WHOLE goal by name rather
    /// than place either half through the generic path, because neither
    /// `FactorioEntity` nor the mod's `rcon_place_entity` could say which half
    /// was being built. Both now can (task 5's `FactorioEntity::underground_half`
    /// and `entity_for` above), so this pins the refusal's replacement: the
    /// blueprint plans, and the two placements it emits for `underground-belt`
    /// carry the two different halves, not the same one twice -- the exact
    /// failure ("places 100% correctly and connects nothing") this whole task
    /// exists to make unreachable.
    #[test]
    fn a_blueprint_with_an_underground_belt_pair_now_plans() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint = include_str!("../../../core/tests/blueprints/furnace_line.txt")
            .trim()
            .to_string();
        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );
        let goal = Goal::Built {
            blueprint,
            site: Site::At(Position::new(0.0, 0.0)),
        };

        let steps = BuildBlock
            .expand(&goal, &mut ctx)
            .expect("a blueprint with an underground-belt pair now plans");

        let halves = underground_belt_halves(&steps);
        assert_eq!(
            halves.len(),
            2,
            "FurnaceLine's one underground-belt pair is two placements: {halves:?}"
        );
        assert!(
            halves.contains(&Some(UndergroundHalf::Input)),
            "one half must be the input: {halves:?}"
        );
        assert!(
            halves.contains(&Some(UndergroundHalf::Output)),
            "one half must be the output: {halves:?}"
        );
    }

    /// Every `underground_half` carried by an `underground-belt` `Place`
    /// action anywhere in `steps`, in emission order. Walks `Step::Owned`
    /// the same way `have_bills` (below) does, since a real plan spreads a
    /// block's placements over one `Step::Owned` per band.
    fn underground_belt_halves(
        steps: &[Step],
    ) -> Vec<Option<factorio_bot_core::blueprint::UndergroundHalf>> {
        let mut out = Vec::new();
        for step in steps {
            match step {
                Step::Act(action) => {
                    if let ActionKind::Place { entity } = &action.kind
                        && entity.name == "underground-belt"
                    {
                        out.push(entity.underground_half);
                    }
                }
                Step::Owned { steps, .. } => out.extend(underground_belt_halves(steps)),
                _ => {}
            }
        }
        out
    }

    /// Every `Goal::Have` stated anywhere in `steps`, summed by item across
    /// however many bands (and therefore `Step::Owned` blocks) it is spread
    /// over. A one-bot roster puts the whole bill in one band, but this
    /// stays correct for a multi-bot split too, which is what the coming
    /// live task actually runs.
    fn have_bills(steps: &[Step]) -> BTreeMap<String, u32> {
        let mut out = BTreeMap::new();
        for step in steps {
            match step {
                Step::Subgoal(Goal::Have { item, count, .. }) => {
                    *out.entry(item.clone()).or_insert(0) += *count;
                }
                Step::Owned { steps, .. } => {
                    for (item, count) in have_bills(steps) {
                        *out.entry(item).or_insert(0) += count;
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The brief's own requirement: the bill must be stated as `Goal::Have`
    /// subgoals, or a roster starting with nothing but a freeplay inventory
    /// (the coming live task's four bots) never gets off the ground --
    /// `HasItem` preconditions alone only plan a block bots already happen
    /// to be carrying in full.
    ///
    /// `MinerLine` (`crates/core/tests/blueprints/miner_line.txt`) is the
    /// fixture `the_miner_line_decodes_to_its_37_entities` in
    /// `crates/core/tests/blueprint_decode.rs` already pins at 13
    /// `electric-mining-drill`, 21 `transport-belt` and 3
    /// `small-electric-pole` -- the counts asserted here are not invented,
    /// they are that same fixture's own numbers.
    #[test]
    fn a_plan_for_miner_line_on_an_empty_world_bills_its_materials() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint = include_str!("../../../core/tests/blueprints/miner_line.txt")
            .trim()
            .to_string();
        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );
        let goal = Goal::Built {
            blueprint,
            site: Site::At(Position::new(0.0, 0.0)),
        };

        let steps = BuildBlock
            .expand(&goal, &mut ctx)
            .expect("an empty world plans a fresh block");

        let bill = have_bills(&steps);
        assert_eq!(
            bill,
            BTreeMap::from([
                ("electric-mining-drill".to_string(), 13),
                ("small-electric-pole".to_string(), 3),
                ("transport-belt".to_string(), 21),
            ]),
            "the bill states exactly what MinerLine's 37 entities need, and \
             nothing else: {bill:?}"
        );
    }

    /// **The band's spatial promise, checked against a fixture this crate
    /// ships, which is where the promise was false.**
    ///
    /// The spec claims "a bot never crosses another's band, which is the
    /// structural reason two of them cannot trap each other". `bands` sorted
    /// by x unconditionally, so over `MinerLine` -- 4 tiles wide, 20 tall --
    /// bands 0, 1 and 2 all occupied x = 3.5 (21 of its 37 entities sit on
    /// that one column) and band 0 spanned the whole height that bands 1 and
    /// 2 were segments of: three bots interleaved in a one-tile corridor.
    ///
    /// `bands_split_by_count_not_by_width` above is correct and passed
    /// throughout, because its synthetic block is wide. That is exactly how
    /// this got through, and it is why this test uses the real fixture.
    #[test]
    fn bands_over_the_real_miner_line_are_disjoint_along_the_split_axis() {
        let bp = decode(include_str!("../../../core/tests/blueprints/miner_line.txt").trim())
            .expect("fixture decodes");

        assert_eq!(
            split_axis(&bp.entities),
            SplitAxis::Y,
            "MinerLine spans x 1.5..=5.5 and y 0.5..=20.5, so the cut runs across y"
        );

        let split = bands(&bp.entities, 4);
        let interval = |band: &Vec<usize>| {
            band.iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, i| {
                    let y = bp.entities[*i].offset.y();
                    (acc.0.min(y), acc.1.max(y))
                })
        };
        let mut reached = f64::NEG_INFINITY;
        for (n, band) in split.iter().enumerate() {
            assert!(
                !band.is_empty(),
                "37 across 4 leaves no band empty: {split:?}"
            );
            let (lo, hi) = interval(band);
            assert!(
                lo >= reached,
                "band {n} starts at y={lo} but band {} already reached y={reached}: the bands \
                 interleave, which is the failure this test exists for",
                n.saturating_sub(1)
            );
            reached = hi;
        }

        // And the demonstration that x was the wrong axis for this block:
        // every band covers essentially the whole 4-tile width, so no cut
        // across x could have separated them into regions at all.
        for (n, band) in split.iter().enumerate() {
            let (lo, hi) = band
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, i| {
                    let x = bp.entities[*i].offset.x();
                    (acc.0.min(x), acc.1.max(x))
                });
            assert!(
                hi - lo >= 2.0,
                "band {n} spans x {lo}..={hi}; MinerLine's bands all span its width, which is \
                 why the split cannot be made along x"
            );
        }
    }

    /// **The remainder is spread, not dumped on the last band.**
    /// `div_ceil` chunking gave six entities across four bots as 2/2/2/0 --
    /// a whole idle bot on a small block -- where the even split is 2/2/1/1.
    #[test]
    fn a_remainder_is_spread_across_the_bands_not_dumped() {
        let ents: Vec<BlueprintEntity> = (0..6).map(|i| at(i as f64)).collect();
        let lengths: Vec<usize> = bands(&ents, 4).iter().map(Vec::len).collect();
        assert_eq!(lengths, vec![2, 2, 1, 1], "six across four is 2/2/1/1");

        // And nothing is lost or duplicated by the spreading.
        let ents: Vec<BlueprintEntity> = (0..37).map(|i| at((i % 7) as f64)).collect();
        let split = bands(&ents, 4);
        let lengths: Vec<usize> = split.iter().map(Vec::len).collect();
        assert_eq!(lengths, vec![10, 9, 9, 9], "37 across four is 10/9/9/9");
        let mut seen: Vec<usize> = split.iter().flatten().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..37).collect::<Vec<_>>());
    }

    /// A block with fewer entities than bots leaves the extra bands empty
    /// rather than handing one of them a stray entity.
    #[test]
    fn more_bots_than_entities_leaves_the_extra_bands_empty() {
        let ents: Vec<BlueprintEntity> = (0..2).map(|i| at(i as f64)).collect();
        let lengths: Vec<usize> = bands(&ents, 4).iter().map(Vec::len).collect();
        assert_eq!(lengths, vec![1, 1, 0, 0]);
    }

    /// **The worst shape of bug this method can have, and it was live.**
    ///
    /// `already_stands` compared name and tile only, so a belt standing on
    /// the right tile facing the WRONG way read as already built. It cannot
    /// produce a bad build from a clean start; it *freezes one in*, because
    /// replanning -- the mechanism this whole method rests on -- is what
    /// would otherwise correct it, and this is the one path the branch exists
    /// to protect.
    ///
    /// The correction is not a silent placement. `ActionKind` has no action
    /// that rotates or removes a standing entity -- but even if it did, a
    /// `Place` emitted over the wrong-facing belt carries the same
    /// `Condition::AreaFree` every other placement does, evaluated by the
    /// same predicate as this method's ground pre-check, so it could never
    /// be scheduled: the run would end on the opaque
    /// `PlannerError::ChainOwnerInfeasible` the pre-check exists to replace.
    /// (Whether the GAME would refuse the re-placement at dispatch was never
    /// established -- `rcon_place_entity` passes `fast_replace = true`, so
    /// the one checkable fact points the other way.) So it is refused here,
    /// by name, saying both facings -- what must never happen again is
    /// reading it as done.
    #[test]
    fn an_entity_facing_the_wrong_way_is_not_read_as_already_built() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint = include_str!("../../../core/tests/blueprints/miner_line.txt")
            .trim()
            .to_string();
        let bp = decode(&blueprint).expect("fixture decodes");
        let anchor = Position::new(0.0, 0.0);

        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );
        // Stand the whole block as designed, then turn ONE belt.
        let turned = bp
            .entities
            .iter()
            .position(|e| e.name == "transport-belt")
            .expect("MinerLine has belts");
        for (n, e) in bp.entities.iter().enumerate() {
            let world = anchor.add(&e.offset);
            let mut entity = entity_for(&ctx.state, e, &world);
            if n == turned {
                // 4 is east, 12 is west: the same tile, the opposite way, and
                // a belt run that carries nothing.
                entity.direction = if e.direction == 4 { 12 } else { 4 };
            }
            ctx.state.create_entity(entity);
        }

        // The direct fact first: the standing entity is NOT "as designed".
        let e = &bp.entities[turned];
        let world = anchor.add(&e.offset);
        assert!(
            matches!(
                already_stands(&ctx.state, e, &world),
                Standing::Differently { .. }
            ),
            "a belt facing the wrong way is neither absent nor as designed"
        );

        // And the goal it belongs to no longer plans as if the block were
        // finished. Before this fix `expand` returned Ok(vec![]) here -- the
        // exact silent freeze.
        let goal = Goal::Built {
            blueprint,
            site: Site::At(anchor),
        };
        let err = BuildBlock
            .expand(&goal, &mut ctx)
            .expect_err("a wrong-facing entity is not silently accepted");
        let message = err.to_string();
        assert!(
            message.contains("transport-belt")
                && message.contains(&format!("({}, {})", world.x(), world.y())),
            "the refusal names the entity and the tile: {message}"
        );
        assert!(
            message.contains("facing"),
            "the refusal says which way it faces and which way was wanted: {message}"
        );
    }

    /// The underground half is the other field whose whole reason for
    /// existing is that placing correctly and functioning are separate
    /// concerns: an `input` half where an `output` was wanted stands on the
    /// right tile, faces the right way, and connects nothing.
    #[test]
    fn an_underground_belt_on_the_wrong_half_is_not_read_as_already_built() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint = include_str!("../../../core/tests/blueprints/furnace_line.txt")
            .trim()
            .to_string();
        let bp = decode(&blueprint).expect("fixture decodes");
        let anchor = Position::new(0.0, 0.0);
        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );

        let e = bp
            .entities
            .iter()
            .find(|e| e.underground_half == Some(UndergroundHalf::Input))
            .expect("FurnaceLine has an input half");
        let world = anchor.add(&e.offset);
        let mut standing = entity_for(&ctx.state, e, &world);
        standing.underground_half = Some(UndergroundHalf::Output);
        ctx.state.create_entity(standing);

        match already_stands(&ctx.state, e, &world) {
            Standing::Differently { half, .. } => assert_eq!(
                half,
                (Some(UndergroundHalf::Output), Some(UndergroundHalf::Input)),
                "the refusal has to know which half stands and which was wanted"
            ),
            other => panic!("the wrong half must not read as built: {other:?}"),
        }
    }

    /// **The spec's fourth refusal, which was never built.**
    ///
    /// `expand` used to emit every placement regardless of what was on the
    /// ground, and occupancy surfaced from `schedule()` as
    /// `ChainOwnerInfeasible` -- an internal scheduling verdict standing in
    /// for a fact about a tile. Four runs across three anchors were spent
    /// distinguishing hypotheses this answers in one line.
    #[test]
    fn a_block_whose_ground_is_occupied_is_refused_naming_the_tile() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint = include_str!("../../../core/tests/blueprints/miner_line.txt")
            .trim()
            .to_string();
        let bp = decode(&blueprint).expect("fixture decodes");
        let anchor = Position::new(0.0, 0.0);
        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );

        // A stone furnace squarely on the tile the block's first entity wants.
        let blocked = anchor.add(&bp.entities[0].offset);
        ctx.state.create_entity(FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: blocked.clone(),
            ..Default::default()
        });

        let goal = Goal::Built {
            blueprint,
            site: Site::At(anchor),
        };
        let err = BuildBlock
            .expand(&goal, &mut ctx)
            .expect_err("occupied ground is refused before anything is emitted");
        let message = err.to_string();
        assert!(
            message.contains(&format!("({}, {})", blocked.x(), blocked.y()))
                && message.contains("stone-furnace"),
            "the refusal names the tile and what is on it: {message}"
        );
    }

    /// **A roster bot's own body is the refusal a researcher hits first.**
    ///
    /// The block is placed at a fixed offset, and a character blocks a
    /// placement exactly as a rock does -- but it is cleared by walking, not
    /// by moving the block, so the message has to say which of the two it is.
    #[test]
    fn a_roster_bot_standing_on_the_footprint_is_named_as_such() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use factorio_bot_core::types::FactorioPlayer;
        use std::sync::Arc;

        let blueprint = include_str!("../../../core/tests/blueprints/miner_line.txt")
            .trim()
            .to_string();
        let bp = decode(&blueprint).expect("fixture decodes");
        let anchor = Position::new(0.0, 0.0);
        let on_top = anchor.add(&bp.entities[0].offset);

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                position: on_top.clone(),
                build_distance: 10,
                reach_distance: 10,
                resource_reach_distance: 4.0,
                ..Default::default()
            },
        );
        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(world), &[BotId(1)]),
            BotId(1),
        );

        let goal = Goal::Built {
            blueprint,
            site: Site::At(anchor),
        };
        let err = BuildBlock
            .expand(&goal, &mut ctx)
            .expect_err("a bot standing on the footprint refuses the block");
        let message = err.to_string();
        assert!(
            message.contains("character 1") && message.contains("own bots"),
            "the refusal distinguishes a roster bot's body from a rock: {message}"
        );
    }

    /// **The regression test for the defect this module exists to fix.**
    ///
    /// `Site::Anywhere` seeds the search at the world origin when the block
    /// has no mining drill of its own (`nearest_ore_seed` returns `None` --
    /// see its own doc), and that is exactly where a character bot spawns.
    /// Siting rightly ignores the bot when choosing ground --
    /// `search_site`'s whole stability argument depends on that -- but the
    /// pre-check used to re-check the SAME anchor with `placement_occupant`,
    /// which names a character, and refused the very ground siting had just
    /// picked one line earlier. Measured live in `run-1788685081-91006`.
    /// This fails on master with `BlockGroundOccupied` naming "character 1".
    #[test]
    fn a_character_at_the_origin_does_not_block_site_anywhere() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use factorio_bot_core::types::FactorioPlayer;
        use std::sync::Arc;

        let blueprint = encode_test_blueprint(&[("stone-furnace", 0.0, 0.0)]);

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                position: Position::new(0.0, 0.0),
                build_distance: 10,
                reach_distance: 10,
                resource_reach_distance: 4.0,
                ..Default::default()
            },
        );
        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(world), &[BotId(1)]),
            BotId(1),
        );

        let goal = Goal::Built {
            blueprint,
            site: Site::Anywhere,
        };
        let steps = BuildBlock.expand(&goal, &mut ctx).expect(
            "a character standing on ground the search itself chose must not \
             refuse the block -- that is exactly the defect this module fixes",
        );
        assert!(
            !steps.is_empty(),
            "a fresh block on open ground must actually place something"
        );
    }

    /// **Guardrail: the sited path still refuses real ground, not just
    /// characters.**
    ///
    /// Only a character is exempt from the sited-path ground check --
    /// everything else `siting_occupant` recognises (a real entity, water, a
    /// refused footprint) must still stop the search cold. A single entity
    /// whose footprint is deliberately wider than the whole search radius
    /// stands in for "there is nowhere left to go": if a fix mistakenly
    /// disabled the ground check altogether whenever the anchor came from
    /// search, rather than switching which predicate it used, this would
    /// plan straight through a real obstruction instead of refusing.
    #[test]
    fn a_real_entity_still_blocks_the_search_sited_path() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint = encode_test_blueprint(&[("stone-furnace", 0.0, 0.0)]);

        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );
        // Wider than `SEARCH_RADIUS` in every direction -- nowhere within
        // the search bound is left clear, so this stands in for a real,
        // comprehensive obstruction rather than a single blocked tile the
        // search could just step around.
        ctx.state.create_entity(FactorioEntity {
            name: "an-enormous-obstruction".into(),
            entity_type: "simple-entity".into(),
            position: Position::new(0.0, 0.0),
            bounding_box: Rect::new(
                &Position::new(-1000.0, -1000.0),
                &Position::new(1000.0, 1000.0),
            ),
            ..Default::default()
        });

        let goal = Goal::Built {
            blueprint,
            site: Site::Anywhere,
        };
        let err = BuildBlock
            .expand(&goal, &mut ctx)
            .expect_err("a real obstruction covering the whole search radius must still refuse");
        assert!(
            matches!(err, PlannerError::NoSiteFound { .. }),
            "the sited path must still fail closed on real ground, not plan \
             through it: {err:?}"
        );
    }

    /// **Must not double-count.** A block already standing has nothing left
    /// to place, so it must ask for nothing either -- billing the full 37
    /// items for a block that is already there would send bots gathering
    /// materials for a build with no work left to do.
    #[test]
    fn a_block_already_standing_bills_nothing() {
        use crate::ids::BotId;
        use factorio_bot_core::test_utils::fixture_world;
        use std::sync::Arc;

        let blueprint = include_str!("../../../core/tests/blueprints/miner_line.txt")
            .trim()
            .to_string();
        let bp = decode(&blueprint).expect("fixture decodes");
        let anchor = Position::new(0.0, 0.0);

        let mut ctx = ExpansionCtx::new(
            PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]),
            BotId(1),
        );
        // Stand every entity the blueprint names, exactly where `expand`
        // would look for it, so `already_stands` finds all 37 already there.
        for e in &bp.entities {
            let world = anchor.add(&e.offset);
            let entity = entity_for(&ctx.state, e, &world);
            ctx.state.create_entity(entity);
        }

        let goal = Goal::Built {
            blueprint,
            site: Site::At(anchor),
        };
        let steps = BuildBlock
            .expand(&goal, &mut ctx)
            .expect("a fully-standing block plans cleanly");

        assert!(
            steps.is_empty(),
            "nothing to place means nothing to bill either: {steps:?}"
        );
    }

    /// **The bill was verified by tests that could not have failed.**
    ///
    /// `fixture_world` marks every recipe `enabled: true`
    /// (`crates/core/tests/recipes-fixtures.json`), so
    /// `a_plan_for_miner_line_on_an_empty_world_bills_its_materials` above
    /// proves the SHAPE of the bill -- one `Goal::Have` per item still
    /// missing -- but cannot prove the bill is ever actually CONSULTED. A
    /// `Place` action whose `HasItem` precondition has no `Goal::Have`
    /// behind it at all reads exactly the same against that fixture as one
    /// that does, because nothing in an all-enabled world is ever short of
    /// anything: `BuildBlock::expand()` called directly, in isolation,
    /// cannot tell "the bill works" from "there is no bill". That gap is
    /// not hypothetical -- a live offline check against the real seed-31337
    /// dump was once misread as exactly this defect (a stale binary, not a
    /// real one, but the class of failure it described was real: a bill
    /// with the right shape that the planner never actually gathers).
    ///
    /// So this drives `Goal::Built` through the REAL top-level driver
    /// (`crate::method::expand`, with `registry_for` -- the registry
    /// `goal.plan`, the `plan` CLI and `score-map` all actually build from,
    /// not the dead `default_registry` the brief pointed at) against a
    /// world where `steam-engine`'s recipe is genuinely locked
    /// (`crate::test_world::world_with_locked_recipe`), unlocked by a
    /// `steam-power` technology exactly as it is in the real game -- not
    /// simply absent from a fixture that never modelled locks at all. If
    /// the bill were ever silently dropped, this is the test that would
    /// catch it: `expand()` would return `PlannerError::InsufficientItems`
    /// for a bot holding zero steam engines with nothing gathering any,
    /// rather than a network with a research action, several craft actions
    /// and six place actions, correctly ordered.
    #[test]
    fn a_locked_recipe_is_actually_researched_and_crafted_not_merely_billed() {
        use crate::ids::BotId;
        use crate::method::expand;
        use crate::method::have::registry_for;
        use crate::schedule::schedule;
        use std::sync::Arc;

        // `scripts/rcontest.lua`'s `StarterSteamEngineBoiler` -- six
        // entities: two `steam-engine`, two `small-electric-pole`, one
        // `boiler`, one `pipe`. Chosen over `MinerLine` for this test
        // because it is the cheaper of the two blueprints the same live
        // check named, and cheap is what a fixture-bounded ore patch wants:
        // the point here is the research/craft PATH, not another pass at
        // the quantities `a_plan_for_miner_line_on_an_empty_world_bills_its_materials`
        // already covers.
        let blueprint = "0eNqdkdEKwjAMRf8lz504nRv0V0Rkm0ECbVrWThxj/242RQXrgz6VhHtPLr0jNKZH3xFH0CNQ6ziA3o8Q6My1mXdx8AgaKKIFBVzbeQoRa5shn4kRJgXEJ7yCzqeDAuRIkfDOWYbhyL1tsBNBmqDAuyAmx/NFAWUiHOQppkl9QDYviK2NydBgGztqM+9MgvVAlSnU9rc8eYpR/BMnSdo9SY0jI5tvOYrVLuUvn35P/utpsUpLS5/6rX4FF+zCIq6qbZ5X1brcyP/fAHsdtKc=".to_string();

        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        // `world_with_locked_recipe` builds on `fixture_world`, whose own
        // hundred trees are `tree-42` -- a name the prototype fixture gives
        // no `mine_result`, so they yield nothing (see
        // `have.rs::wood_is_still_refused_in_a_world_whose_trees_have_no_prototype`).
        // A pole needs wood, so a real, minable tree has to be added the
        // same way `have.rs::wooded_state` does, clear of both the
        // blueprint's anchor and `with_steam_power`'s fixtures below.
        let world = crate::test_world::with_trees(
            crate::test_world::world_with_locked_recipe("steam-engine", &["steam-power"]),
            &[Position::new(5.0, 5.0), Position::new(6.0, 5.0)],
        );
        let mut state = PlanState::from_world(Arc::new(world), &bots);
        // A locked recipe's unlocker may need researching, and research
        // needs somewhere powered to put a lab -- the same reason
        // `have.rs::locked_state` supplies it. Without this the test would
        // fail on `ResearchNeedsPower` rather than on the question it
        // actually asks.
        crate::test_world::with_steam_power(&mut state);

        let goal = Goal::Built {
            blueprint,
            site: Site::At(Position::new(30.0, 30.0)),
        };

        let net = expand(&[goal], &state, &registry_for(&bots), BotId(1))
            .expect("a genuinely locked recipe is researched and crafted, not refused");

        // Proof that the bill was CONSULTED, not merely stated: a research
        // action for the unlocking technology and every one of the six
        // placements are actually present in the expanded network. A world
        // where the `Have` subgoal did nothing (the exact defect this test
        // answers to) would have failed inside `expand` above with
        // `PlannerError::InsufficientItems` on the first `Place`'s
        // precondition -- it would never have reached this line at all.
        assert!(
            net.actions()
                .any(|a| matches!(&a.kind, ActionKind::Research { tech } if tech == "steam-power")),
            "steam-power is locked in this fixture, so six standing \
             entities must go through a research action: {:?}",
            net.actions().map(|a| &a.label).collect::<Vec<_>>()
        );
        // Filtered to THIS blueprint's own placements, not every `Place` in
        // the network: a correct plan also builds scaffolding of its own --
        // stone furnaces to smelt the plates, a lab to run the research --
        // and those are `ActionKind::Place` too. `place_step`'s label always
        // carries "block band N" (see `expand`, above), which nothing else
        // in the plan emits, so it is what tells the six placements this
        // test is actually about apart from the plan's own infrastructure.
        let placements = net
            .actions()
            .filter(|a| {
                matches!(a.kind, ActionKind::Place { .. }) && a.label.contains("block band")
            })
            .count();
        assert_eq!(
            placements,
            6,
            "all six of StarterSteamEngineBoiler's entities are placed, not \
             just billed: {:?}",
            net.actions().map(|a| &a.label).collect::<Vec<_>>()
        );

        // The round trip a live caller actually takes: `expand` alone proves
        // the network is buildable, `schedule` proves it is also runnable.
        schedule(&net, &state, &bots).expect("the plan schedules");
    }
}

#[cfg(test)]
mod enclosure_guard_tests {
    use super::*;
    use crate::ids::BotId;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::FactorioPlayer;
    use std::sync::Arc;

    /// A block that closes a ring around one of this plan's own bots must walk
    /// it clear before the walls go up — or refuse, if there is no way out.
    ///
    /// **`method::blueprint` had no enclosure guard at all** until this test's
    /// fix, while `method::assemble` has had one for its cells. So the method
    /// that builds the largest blocks in the project was the unguarded one. A
    /// 27-entity block found it live: `the character is already walled in here
    /// ... pocket_tiles=1.0`, and the build reported `done=true` with 13 of 29
    /// steps never dispatched.
    ///
    /// The executor's `pre_place` cannot cover this — it judges only the
    /// character *doing* the placing, so one bot walling in another is
    /// invisible to it, and once a bot is enclosed every later placement reads
    /// as "already walled in, not this placement's doing" and is allowed.
    ///
    /// The fixture is a closed ring of eight 2x2 furnaces whose interior is the
    /// 2x2 tile square at the origin. Consecutive furnaces touch, so there is
    /// no gap to walk through; a bot at (0.5, 0.5) is inside it.
    #[test]
    fn a_block_that_rings_a_bot_evacuates_it_or_refuses() {
        let blueprint_text = include_str!("../../../core/tests/blueprints/ring_block.txt")
            .trim()
            .to_string();

        let world = fixture_world();
        // Bot 1 stands in the ring's interior. This is a bot ON THE ROSTER, not
        // a bystander: the case a researcher building a block near their own
        // bots actually hits.
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                position: Position::new(0.5, 0.5),
                build_distance: 10,
                reach_distance: 10,
                resource_reach_distance: 4.0,
                ..Default::default()
            },
        );
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        // The bot really is where the test says, or nothing below means
        // anything — the same guard the bystander test uses.
        assert!(
            state
                .characters_near(&Position::new(0.0, 0.0), 5.0)
                .iter()
                .any(|(player, _)| *player == 1),
            "bot 1 must be standing at the ring's centre for this to be the \
             enclosure case at all"
        );

        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let goal = Goal::Built {
            blueprint: blueprint_text,
            site: Site::At(Position::new(0.0, 0.0)),
        };

        match BuildBlock.expand(&goal, &mut ctx) {
            Ok(steps) => {
                let evacuations: Vec<&Action> = steps
                    .iter()
                    .filter_map(|s| match s {
                        Step::Act(a) if matches!(a.kind, ActionKind::Evacuate { .. }) => {
                            Some(a.as_ref())
                        }
                        _ => None,
                    })
                    .collect();
                assert!(
                    !evacuations.is_empty(),
                    "a block that rings bot 1 must emit an Evacuate before its \
                     placements; got {} steps and none of them evacuate anyone",
                    steps.len()
                );
                // Pinned to the bot being rescued, not to whoever builds — the
                // whole point of `evacuation_step`'s `pinned` field.
                assert_eq!(
                    evacuations[0].pinned,
                    Some(BotId(1)),
                    "the evacuation must be pinned to the bot it rescues"
                );
                // And it must precede every placement, or the walls can go up
                // first and the walk becomes impossible.
                let place_ids: Vec<ActionId> = steps
                    .iter()
                    .flat_map(|s| match s {
                        Step::Owned { steps, .. } => steps.clone(),
                        other => vec![other.clone()],
                    })
                    .filter_map(|s| match s {
                        Step::Act(a) if matches!(a.kind, ActionKind::Place { .. }) => Some(a.id),
                        _ => None,
                    })
                    .collect();
                assert!(!place_ids.is_empty(), "the block must place something");
                for place_id in &place_ids {
                    assert!(
                        steps.iter().any(|s| matches!(
                            s,
                            Step::Link { from, to, .. }
                                if *from == evacuations[0].id && to == place_id
                        )),
                        "every placement must be linked after the evacuation; \
                         {place_id:?} is not"
                    );
                }
            }
            Err(PlannerError::BlockGroundOccupied { occupant, .. }) => {
                // The other legal outcome: every way out runs through the block
                // itself, so no walk can help. A named refusal is the correct
                // answer and is what the fix returns for that case.
                assert!(
                    occupant.contains("seal a bot into a pocket"),
                    "a refusal here must name the enclosure, not something \
                     else; got {occupant}"
                );
            }
            Err(other) => panic!("expected an evacuation or an enclosure refusal, got {other:?}"),
        }
    }
}
