//! Building a designed block: a blueprint, an anchor, and one band per bot.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::Goal;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::blueprint::{Blueprint, BlueprintEntity, decode};
use factorio_bot_core::types::{FactorioEntity, Pos, Position};

/// How long one placement is modelled to take. Same figure `connect.rs` uses.
const PLACE_TICKS: u32 = 30;

/// Split a block into one band per bot, **balanced by entity count**.
///
/// Sorted by x, then chunked so each band holds as near an equal number of
/// entities as divides. Balancing by width instead would hand one bot a dense
/// corner and another an empty margin.
///
/// Deterministic: the sort is by `total_cmp` on x with the entity's index as
/// the tie-break, so equal-x entities always fall the same way.
pub fn bands(entities: &[BlueprintEntity], bots: usize) -> Vec<Vec<usize>> {
    if bots == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..entities.len()).collect();
    order.sort_by(|a, b| {
        entities[*a]
            .offset
            .x()
            .total_cmp(&entities[*b].offset.x())
            .then(a.cmp(b))
    });
    let mut out = vec![Vec::new(); bots];
    let per = entities.len().div_ceil(bots).max(1);
    for (slot, idx) in order.into_iter().enumerate() {
        out[(slot / per).min(bots - 1)].push(idx);
    }
    out
}

/// The `FactorioEntity` one blueprint entity places, at its world position.
///
/// Follows `power.rs::entity_for` / `assemble.rs::entity_for`: `entity_type`
/// is read from the world's own prototype table rather than guessed, and the
/// `bounding_box` is left for `PlanState::create_entity` to fill in from the
/// same table (see its own doc for why that overlay half exists).
fn entity_for(state: &PlanState, name: &str, position: &Position, direction: u8) -> FactorioEntity {
    let entity_type = state
        .base()
        .entity_prototypes
        .get(name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| name.to_string());
    FactorioEntity {
        name: name.to_string(),
        entity_type,
        position: position.clone(),
        direction,
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

/// Is a blueprint entity of `name` already standing centred at `world`?
///
/// The same pattern `assemble.rs::standing_parts` and `power.rs::finish` use:
/// `entity_at` answers for anything covering the point, so the name and the
/// **tile-centred** position both have to match, or a machine one tile off
/// the layout would be read as this one.
fn already_stands(state: &PlanState, name: &str, world: &Position) -> bool {
    match state.entity_at(world) {
        Some(entity) => entity.name == name && Pos::from(&entity.position) == Pos::from(world),
        None => false,
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
        let Goal::Built { blueprint, anchor } = goal else {
            return Ok(Vec::new());
        };
        let bp: Blueprint = decode(blueprint).map_err(|e| PlannerError::BlueprintRefused {
            reason: format!("{e:?}"),
        })?;

        // Only what is NOT already standing. This is what makes the goal
        // re-checkable on a replan and idempotent when built twice.
        let mut wanted: Vec<&BlueprintEntity> = Vec::new();
        for e in &bp.entities {
            let world = anchor.add(&e.offset);
            if !already_stands(&ctx.state, &e.name, &world) {
                wanted.push(e);
            }
        }
        if wanted.is_empty() {
            return Ok(Vec::new());
        }

        let owned: Vec<BlueprintEntity> = wanted.iter().map(|e| (*e).clone()).collect();
        let roster = ctx.state.bot_ids();
        let split = bands(&owned, roster.len().max(1));

        let build = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.build_distance)
            .unwrap_or(10.0);

        let mut steps = Vec::new();
        for (band, indices) in split.iter().enumerate() {
            for idx in indices {
                let e = &owned[*idx];
                let world = anchor.add(&e.offset);
                let entity = entity_for(&ctx.state, &e.name, &world, e.direction);
                let note = format!("block band {band}");
                steps.push(place_step(ctx, entity, build, &note));
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

    fn at(x: f64) -> BlueprintEntity {
        BlueprintEntity {
            name: "transport-belt".into(),
            offset: Position::new(x, 0.0),
            direction: 4,
            underground_half: None::<UndergroundHalf>,
        }
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
}
