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
//! A cell yields what its sources deliver -- the
//! [`crate::method::assemble::SupplyHorizon`], the fewest products any belted
//! input's source will make before its ore is dug out, bounded by the hand
//! charge ([`crate::method::assemble::CELL_CHARGE_TICKS`]) where a chest
//! remains -- and that number is what `BuildAssemblyCell` records as an
//! [`Effect::BufferGain`](crate::action::Effect::BufferGain) on the output
//! chest when the cell is complete. This method spends against exactly that
//! ledger: it draws `min(need, buffered)` and emits the residual as the same
//! goal again, which — the chest now empty in the overlay — falls through to
//! `HandCraft`.
//!
//! **That ledger is the honest bound and it is deliberately not widened.**
//! Until 2026-09-09 it was one charge -- fifteen red packs, whatever stood --
//! so a technology costing 75 was mostly hand-crafted; a belted cell's ledger
//! is its sources' ore, which on a real patch is hundreds of packs. What is
//! still not modelled: a source slower than the cell's tempo, whose wait the
//! lag below under-states (the tempo is the lower bound).
//!
//! # The timing claim
//!
//! The packs are not in the chest when the charge lands — the machines have to
//! run, `ticks_per_item` per pack. That wait is stated as a
//! [`Step::Link`](crate::method::Step::Link) from the action that charged the
//! cell to the `Remove` that draws from it, with a `lag` of
//! `ticks_per_item * take`, and the `Remove` itself costs one transfer.
//!
//! **It used to be the `Remove`'s own `duration`, and that stood a bot at the
//! chest for the whole of it** — fifteen packs at 600 ticks is 9,000 ticks of
//! a bot doing nothing but waiting, which is an artificial serialisation and
//! not a fact about the cell. A lag edge is the device this crate already uses
//! for machine time everywhere else (`method::have`'s smelts, `method::power`'s
//! boilers), and the executor models it directly.
//!
//! **The lag is the cell's TEMPO times the count, and deliberately not
//! `CELL_CHARGE_TICKS`.** `ticks_per_item * take` says "N items cost N of
//! this machine's cycles", which is a fact about the recipe and the machine's
//! `crafting_speed` and stays true whatever feeds the machines. Belting the
//! cell (2026-09-09) revisited the *ledger* above -- it is the sources' ore
//! now, not `charge_products` -- and left this edge alone, as this paragraph
//! said it would.
//!
//! **The id crosses a method boundary, which is the part that needed
//! machinery.** The charge is emitted by `method::assemble` and the draw by
//! this module, so nothing here allocated the `ActionId` it has to name.
//! `ExpansionCtx::buffer_stock` carries it: `run_steps` records, for every
//! `Effect::BufferGain` it simulates, which action produced it — the buffer
//! analogue of the `stock` map that states a hand's supply edges, and the same
//! shape as `PlanState::machine_queue`'s `release`.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::Goal;
use crate::ids::Ticks;
use crate::method::assemble::{AssemblySpec, CHEST, INSERTER, assembly_spec, cell_output_chests};
use crate::method::have::demand;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::types::Position;

/// Ticks one chest-to-hand transfer costs, as everywhere else in the crate.
const TRANSFER_TICKS: Ticks = 10;

/// Can a cell's output be spent, or would spending it pay for the cell?
///
/// # The question the `is_science_pack` allowlist was standing in for
///
/// Until 2026-09-09 this was a name check: a cell's output was spendable only
/// if some technology ate it. That was bought by a measured regression --
/// `producing:transport-belt:6`, a 307-action plan, became
///
/// ```text
/// bot 1 owns chain ChainId(22) because its bill was sized against it, but
/// 4 transport-belt in the buffer at [33.5, -12.5] does not hold there
/// ```
///
/// because [`crate::method::sustain`] belts the cell's fuel, so the belts a
/// belt cell had yet to make were drawn to pay for building it. A science pack
/// cannot close that loop -- no machine, chest, inserter, belt or pole has one
/// in its bill -- so the allowlist closed the hole by construction.
///
/// **It closed it for the wrong reason.** `steel-plate` is not a science pack
/// and is also not in any cell's bill, so the name check refuses it for no
/// reason at all; and an allowlist that grows an item at a time is a list of
/// things somebody remembered. This asks the question directly: is
/// `spec.item` in the **transitive bill of this cell's own construction**?
///
/// # What is seeded, and what is deliberately not
///
/// The seed is what the plan spends on one cell: the buildings
/// ([`bill`](super::assemble)), the power plant
/// [`crate::method::power::ensure_powered`] may raise for it, the belt run
/// [`crate::method::sustain`] may lay to fuel it, and the charge that goes
/// into its chests. A charge is in the seed on purpose -- drawing the output
/// to fill the cell's own supply chest is the same loop by a shorter route.
///
/// **The subject is not seeded.** `crate::products::reachable_here` once
/// seeded its closure with the product it was choosing for, which for `water`
/// eliminated every genuine producer; the same shape here would report every
/// cell as looping. `spec.item` enters the closure only if a recipe reachable
/// from a seed genuinely lists it.
///
/// Recycling is excluded, for the reason [`crate::products`] and
/// [`crate::method::machine::obtain_costs`] exclude it: `X-recycling` produces
/// `X` from `X`, so a walk that followed it would find every item in its own
/// bill. Cycles are otherwise safe because the walk carries a visited set.
///
/// # The direction it errs in
///
/// The closure is the **union** over every non-recycling recipe producing a
/// seed, not the cheapest one `obtain_costs` would pick. That over-approximates
/// -- it can name an item the plan would never actually route through -- and
/// over-approximating means *refusing to write a ledger entry*, which is
/// exactly the behaviour every non-pack item had before this existed. The
/// other direction is the regression above.
pub(crate) fn cell_output_loop(state: &PlanState, spec: &AssemblySpec) -> Option<CellLoop> {
    let mut seed: Vec<String> = vec![
        spec.machine.to_string(),
        INSERTER.to_string(),
        CHEST.to_string(),
        // The plant `method::power` raises when a cell needs kilowatts, and
        // the pole that reaches it.
        crate::method::power::POLE.to_string(),
        crate::method::power::PUMP.to_string(),
        crate::method::power::PIPE.to_string(),
        crate::method::power::BOILER.to_string(),
        crate::method::power::ENGINE.to_string(),
    ];
    // What `method::sustain` lays to keep a burner cell fed. Named here rather
    // than imported because they are function-local constants over there.
    //
    // **Nothing holds the two lists together, and this comment used to claim
    // something did.** It named a test about a belt cell looping on the fuel
    // run — a test that exists nowhere. `doclint`'s `cited_names_resolve`
    // caught it, and a citation to a guard that does not exist is worse than
    // no citation, because it stops the next reader looking.
    //
    // (The dead name is deliberately not written in backticks here: the lint
    // reads any backticked identifier as a citation, so quoting it to explain
    // it re-creates the failure. That is the lint working correctly.)
    //
    // What actually holds it today is an **offline measurement**:
    // `producing:transport-belt:6` planning at 307 actions rather than
    // refusing, which is only true while a belt stays undrawable. That is a
    // real check and it is not a test — nothing runs it in CI, and if
    // `sustain` or `power` ever lays a prototype absent from this seed the
    // loop check **silently under-refuses**.
    //
    // The honest fix is to derive the seed from `sustain`/`power` rather than
    // restate it, which needs those constants to stop being function-local.
    // Until then this is a list, and it is the one place this otherwise
    // derived check still is one.
    seed.extend(
        ["transport-belt", "burner-inserter", "coal"]
            .iter()
            .map(|s| (*s).to_string()),
    );
    // And what the cell eats: `supplied` and the intermediate's ingredients,
    // whether they arrive in a chest a bot fills or on a belt from a standing
    // cell. Either way they are what the cell is built to consume, and
    // drawing the output to make them is the same loop by a shorter route.
    seed.push(spec.supplied.0.clone());
    if let Some(made) = spec.intermediate.as_ref() {
        seed.extend(made.ingredients.iter().map(|(item, _)| item.clone()));
    }

    let index = crate::products::ProductIndex::from_state(state);
    // `from` records, for each item reached, the seed-side item whose recipe
    // listed it -- enough to reconstruct the path back out for the message.
    let mut from: std::collections::BTreeMap<String, String> = Default::default();
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    let mut queue: std::collections::VecDeque<String> = Default::default();
    for name in seed {
        if seen.insert(name.clone()) {
            queue.push_back(name);
        }
    }
    while let Some(item) = queue.pop_front() {
        if item == spec.item {
            let mut path = vec![item.clone()];
            let mut cursor = item;
            while let Some(parent) = from.get(&cursor) {
                path.push(parent.clone());
                cursor = parent.clone();
            }
            path.reverse();
            return Some(CellLoop {
                item: spec.item.clone(),
                path,
            });
        }
        // **The ground is a base case, exactly as it is in
        // [`crate::method::machine::obtain_costs`]**, and it is what keeps
        // this from following a recipe the plan would never run. Without it
        // the check reported, on the real seed-31337 dump:
        //
        // ```text
        // steel-plate is in its own cell's construction bill
        //   (coal -> water -> water-barrel -> barrel -> steel-plate)
        // ```
        //
        // Every step of which is a real recipe and none of which the plan
        // would ever take: a bot mines coal, and coal is charted here. Water
        // is a ground fluid, and the barrel pair is the same cycle
        // `crate::products` documents at length -- `fill-water-barrel` makes
        // a `water-barrel` out of water and `empty-water-barrel` makes water
        // out of a `water-barrel`, so any walk that enters it reaches
        // `barrel`, which is a steel plate.
        //
        // **Empty means unknown here too**: a world that charted nothing
        // supplies nothing, and the walk falls back to the loose union --
        // more refusals, which is the safe direction.
        if index.ground_supplies(&item) {
            continue;
        }
        for recipe in index.recipes_producing(&item) {
            if recipe.category == crate::products::RECYCLING_CATEGORY {
                continue;
            }
            for ingredient in recipe.ingredients.iter().flatten() {
                if seen.insert(ingredient.name.clone()) {
                    from.insert(ingredient.name.clone(), item.clone());
                    queue.push_back(ingredient.name.clone());
                }
            }
        }
    }
    None
}

/// Why a cell's output may not be spent: the chain from something the cell is
/// built or charged with, down to the cell's own product.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CellLoop {
    /// What the cell makes.
    pub item: String,
    /// Seed item first, `item` last. A one-element path means the cell's
    /// product *is* one of the things the cell is built with.
    pub path: Vec<String>,
}

impl std::fmt::Display for CellLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is in its own cell's construction bill ({})",
            self.item,
            self.path.join(" -> ")
        )
    }
}

/// May the cell's output chest carry a ledger entry, and may anything spend it?
///
/// One answer for two call sites -- this module's [`DrawFromCell`] and
/// `method::assemble`'s `Effect::BufferGain`. **Gating only the first leaves
/// the regression standing**, because `method::have::Withdraw` reads the same
/// buffer and would claim the goal first; a ledger entry nothing may safely
/// spend should not be written.
pub(crate) fn output_is_spendable(state: &PlanState, spec: &AssemblySpec) -> bool {
    match cell_output_loop(state, spec) {
        None => true,
        Some(loop_) => {
            factorio_bot_core::tracing::debug!("cell output withheld from the ledger: {}", loop_);
            false
        }
    }
}

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
    // Last, because it is the most expensive question here and the cheap
    // filters above decline almost every goal before it is asked.
    if !output_is_spendable(state, &spec) {
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
        let mut links: Vec<Step> = Vec::new();
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
            let id = ctx.ids.next();
            // The machines have to run before the chest holds this, and that
            // wait belongs on an EDGE rather than in the taking bot's
            // `duration` -- see this module's doc. Every action that charged
            // this chest with this item is a predecessor; `run_steps` recorded
            // them as it emitted them, which is the only way an id allocated
            // inside `method::assemble`'s expansion reaches this one.
            let wait = spec.ticks_per_item.saturating_mul(take);
            for from in ctx
                .buffer_stock
                .get(&(factorio_bot_core::types::Pos::from(&pos), item.clone()))
                .into_iter()
                .flatten()
            {
                links.push(Step::Link {
                    from: *from,
                    to: id,
                    lag: wait,
                });
            }
            steps.push(Step::Act(Box::new(Action {
                id,
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
                // One chest-to-hand transfer, and nothing else. The cell's own
                // tempo is on the `Step::Link` above, where it costs the
                // schedule its ticks without standing a bot at the chest for
                // them.
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!("take {} {} from the cell's output chest", take, item),
            })));
        }
        if drawn == 0 {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        }
        // After the takes, so every `to` names an action the network already
        // holds. An edge whose endpoints are not both in the network is
        // skipped by `infer_edges` and `as_graph`, so ordering here is not a
        // nicety.
        steps.append(&mut links);
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
