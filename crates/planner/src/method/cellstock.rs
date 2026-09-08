//! Draw an item out of the output chest of a **standing assembly cell**,
//! instead of crafting it in a hand.
//!
//! # The edge this is
//!
//! [`crate::method::assemble::BuildAssemblyCell`] has been able to build a
//! cell that makes red science since stage 2, and
//! [`crate::method::have::HandCraft`] has claimed every crafting
//! [`Goal::Have`] since long before that. Those two facts never met: nothing
//! in the tree emitted [`Goal::Producing`], so the only way a cell was ever
//! built was a human typing `producing:automation-science-pack:6` at the CLI,
//! and every science pack this project has ever put into a lab was made in a
//! bot's hands.
//!
//! This method is the join. [`crate::method::have::Researched`] asks for a
//! cell (see its `expand`, and `machine_made_packs`), and this claims the
//! resulting `Goal::Have` for the pack **once the cell stands**, turning it
//! into a walk to the cell's output chest and one `Remove`.
//!
//! # Registration: ahead of `Withdraw`, and therefore ahead of `HandCraft`
//!
//! Order is the decision site. Behind `HandCraft` this method would never be
//! consulted at all, because `HandCraft` claims every `Goal::Have` whose item
//! has a crafting recipe. Ahead of it, this one is asked first and declines
//! unless a cell for the item is **already standing in the plan overlay** —
//! which only happens because something above emitted a `Goal::Producing` and
//! it was expanded first. So the bootstrap cannot deadlock: with no cell
//! built, nothing here claims anything, and the ten red packs that pay for
//! `automation` are hand-crafted exactly as they are today.
//!
//! Ahead of `Withdraw` as well, and that one was **measured rather than
//! reasoned about**. The output chest is a buffer like any other now that
//! `BuildAssemblyCell` records the charge there, so `Withdraw` claimed the
//! goal first and drew the packs with a plain transfer duration — in the
//! `researched:logistic-science-pack` plan, ten ticks after the supply chest
//! was charged, against the 9,000 ticks the machines need to make what was
//! taken. A plan that walks a bot to an empty chest is worse than one that
//! hand-crafts, so this method is asked first and prices the wait.
//!
//! # What is drawn, and why it is bounded
//!
//! A cell is charged once, with [`crate::method::assemble::CELL_CHARGE_TICKS`]
//! worth of ingredients, and **nothing refills it**. So a cell yields
//! `AssemblySpec::charge_products` items and then stops, and that number is
//! what `BuildAssemblyCell` now records as an
//! [`Effect::BufferGain`](crate::action::Effect::BufferGain) on the output
//! chest at charge time. This method spends against exactly that ledger: it
//! draws `min(need, buffered)` and emits the residual as the same goal again,
//! which — the chest now empty in the overlay — falls through to `HandCraft`.
//!
//! **That ledger is the honest bound and it is deliberately not widened.** A
//! technology whose bill is larger than one charge (green science costs 75 red
//! packs against a charge of 15) is partly machine-made and partly
//! hand-crafted, and says so in the plan. Covering the whole bill means either
//! more cells or a charge sized from demand rather than from a fixed tick
//! budget; both are real rungs and neither is smuggled in here.
//!
//! # The timing claim
//!
//! The packs are not in the chest when the charge lands — the machines have to
//! run. The `Remove` this method emits therefore carries the cell's own tempo
//! in its `duration`: `ticks_per_item` per item drawn. It is the same
//! device the crate uses everywhere a bot waits on a machine, and it is a
//! claim about *this cell*, derived from its recipe and its machine's
//! `crafting_speed`, rather than a constant.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::Goal;
use crate::ids::Ticks;
use crate::method::assemble::{AssemblySpec, CHEST, assembly_spec, cell_output_chests};
use crate::method::have::demand;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::types::Position;

/// Ticks one chest-to-hand transfer costs, as everywhere else in the crate.
const TRANSFER_TICKS: Ticks = 10;

/// What this method can draw for `goal`, if anything: the spec of the cell
/// that makes the item, and the chests holding some of it.
///
/// One helper for `applicable` and `expand`, for [`demand`]'s own reason: two
/// answers to the same question is how a method comes to claim a goal it then
/// refuses.
fn drawable(goal: &Goal, state: &PlanState) -> Option<(AssemblySpec, Vec<(Position, u32)>)> {
    let want = demand(goal, state)?;
    if want.need == 0 {
        return None;
    }
    // A caller that named a recipe named it for a reason this method cannot
    // honour: the cell runs the recipe it was built with, and nothing here
    // can re-recipe a standing machine.
    if want.via.is_some() {
        return None;
    }
    let spec = assembly_spec(state, want.item)?;
    let chests: Vec<(Position, u32)> = cell_output_chests(state, &spec)
        .into_iter()
        .map(|pos| {
            let held = state.buffered(&pos, want.item);
            (pos, held)
        })
        .filter(|(_, held)| *held > 0)
        .collect();
    if chests.is_empty() {
        return None;
    }
    Some((spec, chests))
}

/// Take what a standing cell has made, and leave the rest to the hands.
pub struct DrawFromCell;

impl Method for DrawFromCell {
    fn name(&self) -> &'static str {
        "draw from cell"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        drawable(goal, state).is_some()
    }

    // Neither `converges` nor `hands_over` is overridden, and both defaults
    // (`false`) are the true answers: this decomposition is **one action**.
    // Nothing is produced here to converge with anything, and nothing passes
    // from hand to hand -- the item goes from a chest into the hand the goal
    // is about, and that is the end of it.

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some((spec, chests)) = drawable(goal, &ctx.state) else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let want = demand(goal, &ctx.state).ok_or_else(|| PlannerError::NoApplicableMethod {
            goal: goal.to_string(),
        })?;
        let item = want.item.clone();
        let mut left = want.need;
        let reach = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.reach_distance)
            .unwrap_or(10.0);
        let mut steps: Vec<Step> = Vec::new();
        let mut drawn = 0u32;
        for (pos, held) in chests {
            if left == 0 {
                break;
            }
            let take = left.min(held);
            if take == 0 {
                continue;
            }
            left -= take;
            drawn = drawn.saturating_add(take);
            steps.push(Step::Act(Box::new(Action {
                id: ctx.ids.next(),
                kind: ActionKind::Remove {
                    pos: pos.clone(),
                    entity: CHEST.into(),
                    slot: InventorySlot::Chest,
                    item: item.clone(),
                    count: take,
                },
                pre: vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: pos.clone(),
                        radius: reach,
                        min_radius: 0.0,
                    },
                    Condition::EntityAt {
                        pos: pos.clone(),
                        name: CHEST.into(),
                    },
                    Condition::BufferHas {
                        pos: pos.clone(),
                        item: item.clone(),
                        count: take,
                    },
                ],
                eff: vec![
                    Effect::BufferLose {
                        pos: pos.clone(),
                        item: item.clone(),
                        count: take,
                    },
                    Effect::GainItem {
                        who: Actor::Role,
                        item: item.clone(),
                        count: take,
                    },
                ],
                // The machines have to run before the chest holds this. The
                // cell's own tempo, per item drawn -- see this module's doc.
                duration: TRANSFER_TICKS.saturating_add(spec.ticks_per_item.saturating_mul(take)),
                pinned: None,
                label: format!("take {} {} from the cell's output chest", take, item),
            })));
        }
        if drawn == 0 {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        }
        // Whatever a charge could not cover is ordinary work, stated with the
        // goal's own count for `Withdraw`'s reason: `run_steps` has already
        // simulated the take into the hand, so `shortfall` recomputes the
        // difference itself and a pre-subtracted number would subtract twice.
        // The chest is empty in the overlay now, so this comes back to
        // `HandCraft` rather than here.
        if left > 0 {
            steps.push(Step::Subgoal(goal.clone()));
        }
        Ok(steps)
    }
}
