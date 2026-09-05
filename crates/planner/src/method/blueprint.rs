//! Building a designed block: a blueprint, an anchor, and one band per bot.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder, Site};
use crate::method::have::PLACE_TICKS;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::blueprint::{Blueprint, BlueprintEntity, UndergroundHalf, decode};
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{Direction, FactorioEntity, Pos, Position, Rect};
use std::collections::BTreeMap;

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
    let mut votes: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for e in &bp.entities {
        for candidate in state.entities_named(&e.name) {
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
/// The candidate test is `placement_occupant`, which is the SAME predicate
/// `expand` already uses to refuse an anchor. Two predicates meant to agree,
/// written twice, eventually disagree — and here a disagreement would site a
/// block on ground the very next check refuses.
///
/// **`seed` must be replan-stable.** `recover_anchor` only trusts an anchor
/// once two of the block's entities stand (see its own doc), so a block with
/// exactly one entity built recovers nothing and falls back to this search.
/// Placements only ever ADD obstacles and `first_obstruction` skips this
/// block's own entities standing as designed, so a search from the SAME seed
/// always finds every earlier ring still blocked and returns the same
/// anchor. A seed that moves between expansions (a roster centroid, say)
/// breaks that: it can re-order the rings and site the block a second time,
/// with no error and a production curve that still rises. Callers pass a
/// fixed reference — the world origin for `Site::Anywhere`, the caller's own
/// point for `Site::Near` — never anything that tracks where bots have
/// walked to.
///
/// `pub`, matching `method::connect::connect_steps`: as of this task nothing
/// in the tree calls it yet (the seed policy above is wiring for the task
/// that does), and a private, uncalled function would be flagged dead code
/// rather than read as work in progress.
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
fn first_obstruction(state: &PlanState, bp: &Blueprint, anchor: &Position) -> Option<String> {
    for e in &bp.entities {
        let world = anchor.add(&e.offset);
        if matches!(already_stands(state, e, &world), Standing::AsDesigned) {
            continue;
        }
        let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
        if let Some(occupant) = state.placement_occupant(&e.name, &world, facing) {
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

/// Does every mining drill in this block have ore under it at `anchor`?
///
/// Returns the reason it does not, or `None` when they all do.
///
/// **Conservative on purpose.** A real electric mining drill mines a 5x5 area
/// while its collision box is 3x3, and no mining radius reaches us -- nothing
/// on the prototypes carries it. So this asks whether ore lies under the
/// drill's own FOOTPRINT, which can reject a site where the drill would in
/// fact reach ore just outside it. That direction is the safe one: a false
/// refusal is a site not taken, a false acceptance is a drill that places
/// perfectly and produces nothing, which is the failure this project has paid
/// for repeatedly. Recorded so the next author knows it is a floor, not a
/// measurement.
fn drills_are_fed(state: &PlanState, bp: &Blueprint, anchor: &Position) -> Option<String> {
    for e in &bp.entities {
        if !state.stands_on_resources(&e.name) {
            continue;
        }
        let world = anchor.add(&e.offset);
        let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
        let Some(area) = state.collision_area_facing(&e.name, &world, facing) else {
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
        // `recover_anchor`'s own doc. Task 3 (the search) is what fills in
        // `Near`/`Anywhere` when nothing is standing yet; until then those
        // two still refuse rather than guess.
        let anchor = match recover_anchor(&ctx.state, &bp) {
            Some(recovered) => recovered,
            None => match site {
                Site::At(p) => p.clone(),
                Site::Near(_) | Site::Anywhere => {
                    return Err(PlannerError::BlueprintRefused {
                        reason: "siting is not implemented yet; pass an explicit anchor"
                            .to_string(),
                    });
                }
            },
        };

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
        for e in &wanted {
            let world = anchor.add(&e.offset);
            let facing = Direction::from_u8(e.direction).unwrap_or(Direction::North);
            if let Some(occupant) = ctx.state.placement_occupant(&e.name, &world, facing) {
                return Err(PlannerError::BlockGroundOccupied {
                    entity: e.name.clone(),
                    tile: format!("({}, {})", world.x(), world.y()),
                    occupant: occupant.to_string(),
                });
            }
        }

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
                block.push(place_step(ctx, entity, build, &note));
            }
            steps.push(Step::Owned {
                whose: Holder::Share(bot),
                steps: block,
            });
        }
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::blueprint::{BlueprintEntity, UndergroundHalf};
    use factorio_bot_core::types::Position;

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
