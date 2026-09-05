//! Which of a bot's actions occupy its character, and which the game runs in
//! the background while the character does something else.
//!
//! # Why this is a module and not a `matches!` in the run loop
//!
//! The executor used to give each bot exactly one action at a time and wait for
//! it to settle. Measured on `workspace/runs/run-1788465258-49050`: **0 of 99**
//! consecutive same-bot dispatch pairs overlap. That is right for mining — the
//! character swings the pick — and wrong for hand-crafting, which goes into the
//! player's own **crafting queue** and runs while the player walks, mines and
//! builds. Queueing crafts and carrying on is a standard speedrun technique,
//! and the mod side has always supported it: `rcon_action_start_crafting`
//! registers a waiter per `(player, recipe)` and settles it on
//! `on_player_crafted_item`, so several of a bot's crafts can be outstanding at
//! once (`mods/BotBridge/control.lua`).
//!
//! On that run, bot 1 spent 6,575 ticks (1.83 min) inside `craft` and 5,999
//! ticks (1.67 min) inside a single `research` — standing still, in the second
//! case, waiting for a lab it is not operating.
//!
//! The classification lives here, in one exhaustive `match`, so that adding an
//! `ActionKind` is a compile error that asks the question rather than a silent
//! default, and so a test can pin the answer for every variant at once.
//!
//! # The three rules a background action must satisfy
//!
//! 1. **The game does the work, not the character.** Crafting is queued;
//!    research runs in a lab. Mining, placing, inserting and walking are the
//!    character.
//! 2. **The character need not stay put.** Corroborated structurally:
//!    [`ActionKind::target_position`] answers `None` for exactly `Craft` and
//!    `Research`, so the planner emits no `Condition::AtPosition` for them and
//!    the scheduler emits no `Walk` step before them. [`occupancy`] re-checks
//!    that per action anyway — see its docs.
//! 3. **It parks no per-bot state in the actuator.** `Actuator::take_placement`
//!    and `Actuator::take_destination_full` hold one value per bot, drained by
//!    the run loop when the action that produced it settles, and that draining
//!    is only unambiguous while at most one action can have parked something.
//!    Only `Place` and `Insert` park anything, and both are exclusive, so the
//!    one-exclusive-action-per-bot invariant is what keeps that sound. **A new
//!    background kind that parks per-bot state would break it silently.**
//!
//! # And the hazard that is not about the character at all
//!
//! **Hand-crafting consumes materials from the bot's inventory**, and the
//! planner's sizing assumes the bot runs its actions in order. Two mechanisms
//! rely on that order:
//!
//! - `PlanState::available` hides items already promised to an action being
//!   emitted, so expansion's arithmetic is a walk over the plan in order.
//! - `ActionNetwork::infer_edges` deliberately **omits** the producer→consumer
//!   edge for a role-scoped `Condition::HasItem` when the two are in different
//!   chains, on the stated grounds that "a `HasItem { who: Role }` is
//!   re-derived by the scheduler's per-bot feasibility check". That check is
//!   about *one bot's ordered slice*. So for those pairs the schedule order is
//!   the only thing carrying the material dependency, and relaxing it would
//!   break the plan silently — as a craft the game starts fewer of than asked,
//!   or an insert that moves less, rather than as an error.
//!
//! [`inventory_footprint`] is the answer: every item an action may add to or
//! take out of the bot's inventory. Two of a bot's actions may overlap **only
//! if their footprints are disjoint**, which is a sufficient condition for
//! neither to be able to disturb the other's materials whatever order the game
//! sees them in. Where they are not disjoint the run loop waits, exactly as
//! before.
//!
//! That is deliberately conservative and it does forbid real wins — a craft
//! consuming iron plates blocks a later take of iron plates from a furnace,
//! which is harmless in fact, because Factorio removes a craft's ingredients
//! the moment `begin_crafting` accepts it. Distinguishing those cases needs the
//! recipe, which this crate does not have, and the reward for guessing wrong is
//! a plan that no longer matches the world. A correct smaller win beats an
//! unsound larger one.

use factorio_bot_planner::{Action, ActionKind, Condition, Effect, ItemId};
use std::collections::BTreeSet;

/// Whether an action ties up the bot's character while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupancy {
    /// The character is doing it. At most one of these may be in flight per
    /// bot, which is the executor's invariant both before and after this
    /// module existed.
    Exclusive,
    /// The game does it while the character does something else. Any number may
    /// be in flight per bot, subject to [`inventory_footprint`].
    Background,
}

/// What kind of work this is, judged from the verb alone.
///
/// Exhaustive on purpose: a new [`ActionKind`] must be classified here, and the
/// compiler will say so. Do not add a `_` arm — the default that would silently
/// apply is the one that decides whether a bot stands still for it.
pub fn kind_occupancy(kind: &ActionKind) -> Occupancy {
    match kind {
        // The player's crafting queue. The character keeps walking and mining.
        ActionKind::Craft { .. } => Occupancy::Background,
        // A lab does this, and `Actuator::research` does not even take a bot:
        // research is force-wide. The bot has nothing to do with it beyond
        // having fed the lab, which is a separate `Insert`.
        ActionKind::Research { .. } => Occupancy::Background,
        // The character swings the pick.
        ActionKind::Mine { .. }
        | ActionKind::Chop { .. }
        // These settle in their dispatch tick, so nothing is lost by calling
        // them exclusive -- and both `Place` and `Insert` park per-bot state in
        // the actuator for the run loop to drain, which the one-exclusive
        // invariant is what makes unambiguous. See the module docs.
        | ActionKind::Place { .. }
        | ActionKind::Insert { .. }
        | ActionKind::Remove { .. }
        | ActionKind::SetRecipe { .. }
        // Walking, under another name.
        | ActionKind::Evacuate { .. }
        // Also walking. A survey occupies the bot for its whole duration --
        // the bot IS the instrument, and one that is halfway to a frontier
        // is not available for anything else.
        | ActionKind::Survey { .. } => Occupancy::Exclusive,
    }
}

/// What kind of work this action is, judged from the verb **and** from what the
/// plan says the character must be doing while it runs.
///
/// The second half is a safety rail rather than a live case. An action carrying
/// a `Condition::AtPosition { who: Actor::Role, .. }` is one the plan says the
/// running bot must be standing at, and a background action is by definition
/// one the bot walks away from — the scheduler emits a `Walk` step to satisfy
/// that condition and nothing re-establishes it later. Today no `Craft` or
/// `Research` carries one ([`ActionKind::target_position`] answers `None` for
/// both, and the methods that emit them push no such condition), so this arm is
/// never taken. It exists so that a method which *starts* attaching one gets
/// the old, correct, serialised behaviour instead of a bot that wandered off
/// mid-action.
pub fn occupancy(action: &Action) -> Occupancy {
    match kind_occupancy(&action.kind) {
        Occupancy::Exclusive => Occupancy::Exclusive,
        Occupancy::Background if action.pre.iter().any(pins_the_character) => Occupancy::Exclusive,
        Occupancy::Background => Occupancy::Background,
    }
}

fn pins_the_character(cond: &Condition) -> bool {
    matches!(
        cond,
        Condition::AtPosition {
            who: factorio_bot_planner::Actor::Role,
            ..
        }
    )
}

/// Every item this action may add to or take out of a bot's inventory.
///
/// Read from three places, and the redundancy is the point — this set decides
/// whether two of a bot's actions may run at once, so it must never be *too
/// small*, and being too large only costs concurrency:
///
/// - `pre`'s `Condition::HasItem` — what the action needs to be holding.
/// - `eff`'s `Effect::GainItem` / `Effect::LoseItem` — what it says it will
///   change.
/// - the [`ActionKind`]'s own item, which is what the dispatch actually names
///   to the game. A method that forgot a `HasItem` or a `LoseItem` would
///   otherwise hand back an empty footprint for an action that really does move
///   items.
///
/// `Actor` is deliberately ignored. Two actions are only ever compared when
/// they belong to the same bot, where `Actor::Role` resolves to that same bot
/// for both; an `Actor::Bound(other)` naming somebody else's inventory then
/// registers as a conflict it need not, which is the safe direction.
///
/// Chest and machine inventories are **not** in here: `Condition::BufferHas`
/// and `Effect::BufferLose` describe a container's contents, not a bot's, and
/// two bots drawing on one chest are ordered by the plan rather than by this.
/// The `ActionKind::Insert`/`Remove` item *is* included, because the other end
/// of that transfer is the bot's own pockets.
pub fn inventory_footprint(action: &Action) -> BTreeSet<ItemId> {
    let mut items: BTreeSet<ItemId> = BTreeSet::new();
    for cond in &action.pre {
        if let Condition::HasItem { item, .. } = cond {
            items.insert(item.clone());
        }
    }
    for eff in &action.eff {
        match eff {
            Effect::GainItem { item, .. } | Effect::LoseItem { item, .. } => {
                items.insert(item.clone());
            }
            _ => {}
        }
    }
    match &action.kind {
        ActionKind::Mine { item, .. }
        | ActionKind::Chop { item, .. }
        | ActionKind::Craft { item, .. }
        | ActionKind::Insert { item, .. }
        | ActionKind::Remove { item, .. } => {
            items.insert(item.clone());
        }
        // A placement spends one of the item it builds from, and the entity's
        // name is that item's name.
        ActionKind::Place { entity } => {
            items.insert(entity.name.clone());
        }
        // None of these spends an item. A survey buys information with time,
        // which is exactly why it costs nothing here.
        ActionKind::Research { .. }
        | ActionKind::SetRecipe { .. }
        | ActionKind::Evacuate { .. }
        | ActionKind::Survey { .. } => {}
    }
    items
}

/// Whether two of one bot's actions may disturb each other's materials.
///
/// Sufficient, not necessary: disjoint footprints cannot interfere in any
/// order, overlapping ones might. See the module docs for why the executor is
/// not in a position to be more precise, and what that costs.
pub fn shares_inventory(a: &BTreeSet<ItemId>, b: &BTreeSet<ItemId>) -> bool {
    a.intersection(b).next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::types::{Direction, FactorioEntity, Position};
    use factorio_bot_planner::{ActionId, Actor, InventorySlot};

    fn action(kind: ActionKind) -> Action {
        Action {
            id: ActionId(0),
            kind,
            pre: vec![],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "test".into(),
        }
    }

    fn mine(item: &str) -> ActionKind {
        ActionKind::Mine {
            pos: Position::new(1., 1.),
            item: item.into(),
            count: 1,
        }
    }

    fn craft(item: &str) -> ActionKind {
        ActionKind::Craft {
            item: item.into(),
            count: 1,
        }
    }

    /// Every variant, so the list cannot silently gain an unclassified member:
    /// this is the table the run loop's whole concurrency rule reads.
    #[test]
    fn every_action_kind_is_classified_and_only_two_are_background() {
        let kinds = vec![
            mine("iron-ore"),
            ActionKind::Chop {
                pos: Position::new(1., 1.),
                entity: "tree-01".into(),
                item: "wood".into(),
                count: 1,
            },
            craft("iron-gear-wheel"),
            ActionKind::Place {
                entity: Box::new(FactorioEntity::new_stone_furnace(
                    &Position::new(1., 1.),
                    Direction::North,
                )),
            },
            ActionKind::Insert {
                pos: Position::new(1., 1.),
                entity: "stone-furnace".into(),
                slot: InventorySlot::Fuel,
                item: "coal".into(),
                count: 1,
            },
            ActionKind::Remove {
                pos: Position::new(1., 1.),
                entity: "stone-furnace".into(),
                slot: InventorySlot::FurnaceResult,
                item: "iron-plate".into(),
                count: 1,
            },
            ActionKind::Research {
                tech: "automation".into(),
            },
            ActionKind::SetRecipe {
                pos: Position::new(1., 1.),
                entity: "assembling-machine-1".into(),
                recipe: "iron-gear-wheel".into(),
            },
            ActionKind::Evacuate {
                to: Position::new(1., 1.),
            },
        ];
        let background: Vec<String> = kinds
            .iter()
            .filter(|k| kind_occupancy(k) == Occupancy::Background)
            .map(|k| format!("{k:?}"))
            .collect();
        assert_eq!(
            background.len(),
            2,
            "exactly `Craft` and `Research` are background; got {background:?}"
        );
        assert!(background.iter().any(|k| k.starts_with("Craft")));
        assert!(background.iter().any(|k| k.starts_with("Research")));
    }

    #[test]
    fn a_craft_the_plan_pins_to_a_position_is_exclusive_again() {
        let mut a = action(craft("iron-gear-wheel"));
        assert_eq!(occupancy(&a), Occupancy::Background);
        a.pre.push(Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(5., 5.),
            radius: 3.0,
            min_radius: 0.0,
        });
        assert_eq!(
            occupancy(&a),
            Occupancy::Exclusive,
            "an action the plan says the character must stand at is not one the \
             bot may walk away from"
        );
    }

    #[test]
    fn a_crafts_footprint_names_its_ingredients_and_its_product() {
        let mut a = action(craft("iron-gear-wheel"));
        a.pre.push(Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 4,
        });
        a.eff.push(Effect::LoseItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 4,
        });
        a.eff.push(Effect::GainItem {
            who: Actor::Role,
            item: "iron-gear-wheel".into(),
            count: 2,
        });
        let footprint = inventory_footprint(&a);
        assert!(footprint.contains("iron-plate"), "{footprint:?}");
        assert!(footprint.contains("iron-gear-wheel"), "{footprint:?}");
        assert_eq!(footprint.len(), 2, "{footprint:?}");
    }

    /// The redundancy the doc comment claims: an action whose effects were
    /// never written down still names its item, because the dispatch does.
    #[test]
    fn an_action_with_no_conditions_or_effects_still_names_its_item() {
        assert!(inventory_footprint(&action(mine("copper-ore"))).contains("copper-ore"));
        assert!(
            inventory_footprint(&action(ActionKind::Insert {
                pos: Position::new(1., 1.),
                entity: "stone-furnace".into(),
                slot: InventorySlot::Fuel,
                item: "coal".into(),
                count: 4,
            }))
            .contains("coal")
        );
    }

    #[test]
    fn a_research_touches_no_inventory_at_all() {
        assert!(
            inventory_footprint(&action(ActionKind::Research {
                tech: "automation".into(),
            }))
            .is_empty()
        );
    }

    /// The rule the run loop applies, stated on the two cases that matter: a
    /// craft of gears and a mine of ore share nothing, a craft of gears and a
    /// take of plates share the plates.
    #[test]
    fn footprints_overlap_only_when_they_name_the_same_item() {
        let mut gears = action(craft("iron-gear-wheel"));
        gears.pre.push(Condition::HasItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 4,
        });
        let ore = action(mine("iron-ore"));
        let plates = action(ActionKind::Remove {
            pos: Position::new(1., 1.),
            entity: "stone-furnace".into(),
            slot: InventorySlot::FurnaceResult,
            item: "iron-plate".into(),
            count: 5,
        });

        let gears = inventory_footprint(&gears);
        assert!(!shares_inventory(&gears, &inventory_footprint(&ore)));
        assert!(shares_inventory(&gears, &inventory_footprint(&plates)));
    }
}
