//! Stand a crafting machine up and run one recipe in it, with the machine
//! chosen by **what it crafts**.
//!
//! # The rung this is
//!
//! `Craft` runs the `crafting` category in a character's hands and `Smelt`
//! runs `smelting` in a `stone-furnace`. Those were not a scope decision:
//! they were the only two categories whose machine the planner could *name*,
//! because nothing on our wire said what a machine crafts and
//! `oil-refinery`, `chemical-plant`, `centrifuge` and all three assembling
//! machines share one `entity_type`. See
//! `docs/superpowers/notes/2026-09-07-a-recipe-the-planner-cannot-run.md`.
//!
//! [`crate::method::machine::MachineTable`] closed that, so this method is
//! the third: it names its machine the way [`crate::method::machine`] does —
//! one encoding, shared with [`crate::products::Categories::planner_runs`],
//! rather than a second table agreeing with the first until it does not.
//!
//! # It refuses before it emits
//!
//! The same promise `method::connect` makes: every refusal is returned before
//! a step is emitted or an entity lands in the plan overlay. A half-built
//! machine with its bill half-spent is worse than none.
//!
//! # Fluids arrive by pipe, and that is the whole of what a fluid ingredient
//! means
//!
//! Owner ruling, 2026-09-07: **a fluid ingredient is satisfied by
//! CONNECTIVITY, not by a quantity** -- *"you pipe crude to a refinery, you
//! never carry it"*. There is no `Goal::Stored` and no tenth goal kind; items
//! are counted in an inventory and fluids are piped. So this method:
//!
//! * **sites the machine beside its fluid source** rather than beside the bot
//!   that owns the chain. The run has to be short and local, and a refinery
//!   sited at the roster's feet and a tank thirty tiles away is not a pipe
//!   run, it is the long-distance trunk -- which is a separate rung and is
//!   deliberately not built here;
//! * **adopts a source and never builds one**, because a tank this plan
//!   places is *empty*. The asymmetry with the sink below is the point:
//!   an empty buffer is exactly right for catching an output and exactly
//!   wrong for feeding an input;
//! * **gives a fluid product a sink or refuses by name**, because a machine
//!   whose output has nowhere to go stalls. One fluid out is sited and piped
//!   into a buffer, and so is each of several -- `advanced-oil-processing`
//!   makes three, and each gets its own buffer and its own pipe run off its
//!   own output box.
//!
//! # Two fluids out of one machine may not share a pipe
//!
//! A pipe segment in Factorio holds **one** fluid. An `oil-refinery`'s three
//! output boxes sit two tiles apart on one edge -- derived, not remembered:
//! its `pipe_connections` are at x offsets -2, 0 and +2 of a footprint whose
//! half-width rounds to 2, so the port tiles of boxes 0, 1 and 2 are three
//! tiles in a row. A run leaving one of them that crosses another's port
//! **merges two boxes into one segment**, and a refinery that cannot put
//! heavy-oil anywhere stops producing light-oil and petroleum-gas as well.
//!
//! So every run of this rig is routed against every *other* box's port tiles
//! as obstacles, and the result is then **checked** rather than trusted:
//! `route_between` pushes its own two ends' port tiles in unconditionally,
//! outside its search, so no obstacle grid can be relied on alone. See
//! [`FabricateRefusal::FluidOutputsWouldShareAPipe`].
//!
//! **This is an unguarded invariant made guarded, not an observed defect.**
//! With one fluid out there is nothing to mix, and the neighbouring boxes a
//! run crosses today are empty; the hazard becomes real exactly when a second
//! product arrives.
//!
//! Every refusal is a wall rather than a shortfall, and each names the next
//! missing thing: [`FabricateRefusal::NoFluidSource`],
//! [`FabricateRefusal::ManyFluidIngredients`],
//! [`FabricateRefusal::FluidOutputsWouldShareAPipe`],
//! [`FabricateRefusal::SinkTooSmall`] and [`FabricateRefusal::NoSinkSite`].
//!
//! # What connectivity cannot check, stated because it is load-bearing
//!
//! Nothing in [`PlanState`] models what a standing tank *holds*. The source is
//! chosen because its prototype declares a fluidbox that can supply and
//! because it is the nearest such thing -- **not** because anyone established
//! it holds crude. `docs/superpowers/notes/2026-09-07-a-fluid-arrives-by-pipe.md`
//! says what would close that.
//!
//! # Registration
//!
//! **Second to last, immediately ahead of [`crate::products::NoProducer`].**
//! Every method above it is asked first, so a goal that plans today reaches
//! an applicable method before this one is consulted and **no existing plan
//! can move**. It can only claim goals that previously refused.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ItemId, Ticks};
use crate::method::have::{Demand, demand};
use crate::method::machine::{Machine, MachineTable};
use crate::method::pipe::{
    self, FluidPort, PipeEnd, buffer_prototype, pipe_prototype, place_step, plain_entity,
    route_between,
};
use crate::method::util::{
    CRAFTING_CATEGORY, RecipeGate, SMELTING_CATEGORY, free_area_near, free_area_near_where,
    output_per_craft, recipe_for, recipe_gate, smelting_ticks,
};
use crate::method::{ExpansionCtx, Method, Step};
use crate::products::{Categories, ProductIndex};
use crate::state::PlanState;
use crate::substance::{FluidSource, split_bill};
use factorio_bot_core::types::{FactorioEntity, FactorioRecipe, Position, Rect};
use miette::Diagnostic;
use thiserror::Error;

/// Ticks a bot spends walking up to a machine and placing it.
///
/// The same figure `method::have` charges for a stone furnace; a machine is a
/// machine as far as the placement verb is concerned.
const PLACE_TICKS: Ticks = 30;

/// Ticks one hand-to-machine transfer costs, as everywhere else in the crate.
const TRANSFER_TICKS: Ticks = 10;

/// Ticks spent telling a machine which recipe to run.
const SET_RECIPE_TICKS: Ticks = 10;

/// Why a named machine still cannot run a recipe.
///
/// **Every variant is returned before anything is emitted.** They are walls,
/// not shortfalls: no amount of mining, research or walking moves any of them.
#[derive(Clone, Debug, PartialEq, Error, Diagnostic)]
pub enum FabricateRefusal {
    /// The recipe wants a fluid and **nothing standing could put one into a
    /// pipe**.
    ///
    /// # A fluid ingredient is a connectivity requirement, and this is the
    /// only thing that can go wrong with one
    ///
    /// Owner ruling, 2026-09-07: *"you pipe crude to a refinery, you never
    /// carry it"*. So there is no quantity to satisfy and no shortfall to
    /// bill -- either something is connected or nothing is. The amount is
    /// still quoted because it is the recipe's own number and it tells a
    /// reader which recipe was walked, not because anything counts it.
    ///
    /// **Asked of `production_type`, never of the name `storage-tank`**: see
    /// [`crate::method::pipe::fluidbox_entities`]. A pumpjack and an offshore
    /// pump answer as readily as a tank, which is what sulfur's water needs.
    #[error(
        "{recipe} runs in {machine} (category {category}), and the recipe wants {amount} \
         {fluid} -- a fluid, so it arrives by pipe rather than in a hand. Nothing standing on \
         this map can be shown to supply {fluid}, so there is nothing to connect the {machine} \
         to{}. {produced_by}",
        if considered.is_empty() {
            String::new()
        } else {
            format!(
                " (considered and rejected: {} -- a fluidbox that supplies SOMETHING is not a \
                 source of {fluid})",
                considered.join(", ")
            )
        }
    )]
    #[diagnostic(
        code(planner::no_fluid_source),
        help(
            "a fluid ingredient is satisfied by CONNECTIVITY, so what is missing is a standing \
             source -- `gathered:<fluid>` stands a pumpjack and a tank up on a field, and the \
             trunk from that tank to a tank at the base is the rung above this one"
        )
    )]
    NoFluidSource {
        recipe: String,
        category: String,
        machine: String,
        fluid: String,
        amount: u32,
        produced_by: FluidSource,
        /// Standing fluidboxes that could supply *a* fluid and could not be
        /// attributed to *this* one, capped at five. Named because the first
        /// version of this rung silently piped a refinery to a boiler.
        considered: Vec<String>,
    },

    /// A fluid of the recipe cannot be tied to one of the machine's
    /// fluidboxes.
    ///
    /// # This used to be "more than one fluid in", and that was too wide
    ///
    /// Until 2026-09-08 any recipe with two fluid ingredients refused here,
    /// on the grounds that nothing on the wire said which input box took
    /// which. `sulfur` (water + petroleum-gas) landed on it, and through it
    /// so did the whole rocket ladder -- `researched:rocket-silo`,
    /// `have:rocket-part:1`, `have:low-density-structure:1`.
    ///
    /// **The game does say, and it was measured rather than inferred.** A
    /// standing `chemical-plant` with the `sulfur` recipe set reports
    /// `water` on its first input box and `petroleum-gas` on its second, and
    /// ten more multi-fluid recipes agree. [`pipe::fluid_box_ordinals`] holds
    /// the three rules and the one case that genuinely has no answer: a
    /// machine declaring more boxes of a direction than the recipe has
    /// fluids, where the game merges the surplus and the merge is not
    /// positional. That -- `fluoroketone` in a `cryogenic-plant` -- is what
    /// still refuses, and only for the second fluid onward.
    #[error(
        "{recipe} runs in {machine}, and its {ordinal} of {fluids} {direction} fluids is {fluid}, \
         which cannot be tied to a fluidbox: the {machine} declares {} {direction} box(es), the \
         recipe names no fluidbox_index for it, and when those counts differ the game merges \
         boxes in a way that is not positional -- so a pipe would be a guess",
        boxes.map_or_else(|| "an unknown number of".to_string(), |n| n.to_string())
    )]
    #[diagnostic(
        code(planner::fluidbox_undecidable),
        help(
            "a recipe whose fluid count matches the machine's box count is routed positionally, \
             and an explicit fluidbox_index is honoured; neither applies here. A pipe joined to \
             the wrong box builds 100% correctly and moves nothing"
        )
    )]
    FluidBoxUndecidable {
        recipe: String,
        machine: String,
        fluid: String,
        direction: &'static str,
        ordinal: usize,
        fluids: usize,
        boxes: Option<usize>,
    },

    /// Two of the machine's output boxes would end up on **one pipe
    /// segment**, which in Factorio can hold only one fluid.
    ///
    /// # Why this is checked and not merely routed around
    ///
    /// Every run of the rig is routed with the other boxes' port tiles marked
    /// as obstacles, so the search will not cross them. That is not
    /// sufficient: [`crate::method::pipe::route_between`] pushes **its own two
    /// ends' port tiles into the result unconditionally, outside the search**
    /// -- its own doc says so -- so a tile no obstacle grid was ever consulted
    /// about can still land in the run. This variant is the check that closes
    /// that gap, and it names the tile two boxes both wanted.
    ///
    /// # It is a jam, not a slowdown
    ///
    /// A refinery that cannot put its heavy-oil anywhere stops producing
    /// **all three** of its fluids, so mixing two boxes does not cost a
    /// fraction of the output -- it costs the machine. And it does it
    /// silently: every entity places 100% correctly.
    #[error(
        "{recipe} runs in {machine} at {site} and its {first} and {second} runs would both use \
         the tile at {tile}, joining two of the {machine}'s output fluidboxes into one pipe \
         segment -- and a pipe segment holds one fluid, so the {machine} would jam and produce \
         none of its {} products",
        products
    )]
    #[diagnostic(
        code(planner::fluid_outputs_would_share_a_pipe),
        help(
            "the machine's output connections are only a tile or two apart, so this is a siting \
             problem: give the rig more clear ground on the machine's output side"
        )
    )]
    FluidOutputsWouldShareAPipe {
        recipe: String,
        machine: String,
        site: String,
        first: String,
        second: String,
        tile: String,
        products: usize,
    },

    /// The buffer this world offers cannot hold what this **goal** will put
    /// in it.
    ///
    /// # The arithmetic is over the whole goal, not over one craft
    ///
    /// Until 2026-09-08 this compared one craft's output against the buffer,
    /// which is the wrong quantity by a factor of however many crafts the goal
    /// needs: a goal wanting 200 of a product the recipe makes 45 of at a time
    /// runs the machine five times, and it is the *total* that has to land
    /// somewhere. `runs` is finite for every goal that reaches this method --
    /// [`crate::method::have::demand`] answers only for
    /// [`Goal::Have`](crate::goal::Goal::Have) and
    /// [`Goal::Produced`](crate::goal::Goal::Produced), both of which carry a
    /// count -- so the product is always a number and never an unbounded rate.
    ///
    /// # `volume` is three-valued and absent is neither of the others
    ///
    /// See [`BufferCapacity`]. `map.json` -- the dump all four offline
    /// baselines are measured on -- carries `volume: None` for **every**
    /// fluidbox of every prototype, so reading absence as zero would refuse
    /// every buffer on it and move every baseline. Reading it as infinite
    /// would silence this guard on every archived dump. It is therefore
    /// carried as its own answer and **named in the message**, so a run on an
    /// old capture says it could not check rather than quietly passing.
    #[error(
        "{recipe} makes {per_craft} {fluid} per craft and this goal runs the {machine} {runs} \
         time(s), so {total} {fluid} has to land somewhere; the only buffer this world has is a \
         {buffer}, which holds {volume}: the {machine} would fill it and stall"
    )]
    #[diagnostic(
        code(planner::sink_too_small),
        help("ask for less, or give the plan a consumer to pipe into instead of a buffer")
    )]
    SinkTooSmall {
        recipe: String,
        machine: String,
        fluid: String,
        per_craft: u32,
        runs: u32,
        total: u64,
        buffer: String,
        volume: f64,
    },

    /// A fluid product needs a buffer beside the machine and there is no
    /// clear footprint for one.
    #[error(
        "{recipe} makes {fluid}, which needs a {buffer} beside the {machine} at {site} to land \
         in, and no clear footprint for one was found near it"
    )]
    #[diagnostic(
        code(planner::no_sink_site),
        help("clear the ground around the machine's site, or plan the block somewhere emptier")
    )]
    NoSinkSite {
        recipe: String,
        machine: String,
        fluid: String,
        buffer: String,
        site: String,
    },

    /// The machine is electric, supply exists, and **no pole run this planner
    /// will build carries power to where the machine has to stand.**
    ///
    /// `method::extract`'s `ExtractionNotModelled` and
    /// `method::blueprint`'s `BlueprintRefused` say the same thing for their
    /// own sites; this is the third, and the site is not negotiable here for
    /// the same reason it is not there: a fluid ingredient puts the machine
    /// beside its source, so "somewhere else" is a different rig.
    ///
    /// **A wall, not a shortfall** — like every other variant here — and
    /// returned before the machine's `Place` is emitted, so no half-powered
    /// rig is left in the plan.
    #[error(
        "the {machine} running {recipe} at {site} draws {kw:.0} kW, and no run of poles this \
         planner will build carries power to it"
    )]
    #[diagnostic(
        code(planner::no_power_route),
        help(
            "put a generator or a pole run nearer the source the machine is sited beside -- a \
             refinery on no network sets its recipe and makes nothing"
        )
    )]
    NoPowerRoute {
        recipe: String,
        machine: String,
        kw: f64,
        site: String,
    },
}

/// What this method would do for a goal, or nothing.
///
/// Shared by [`Method::applicable`] and [`Fabricate::expand`] so the two
/// cannot disagree about which recipe and which machine — the failure mode
/// where a method claims a goal and then refuses it for a different reason
/// than the one it was selected on.
pub(crate) struct Job {
    pub(crate) recipe: FactorioRecipe,
    pub(crate) machine: String,
}

/// The cheap half of the test, run before a [`ProductIndex`] is built.
///
/// Building an index clones 662 recipes on a real capture, and
/// [`Method::applicable`] is asked for every goal no earlier method claimed.
/// A goal whose product has a same-named recipe in `crafting` or `smelting`
/// is the overwhelming majority and is settled here without touching one.
fn cannot_possibly_apply(state: &PlanState, item: &str) -> bool {
    recipe_for(state, item)
        .is_some_and(|r| r.category == CRAFTING_CATEGORY || r.category == SMELTING_CATEGORY)
}

pub(crate) fn job_for(goal: &Goal, state: &PlanState) -> Option<Job> {
    let Demand {
        item, need, via, ..
    } = demand(goal, state)?;
    if need == 0 {
        return None;
    }
    // Skipped when the caller named a recipe: the shortcut asks whether the
    // *product* has a same-named `crafting`/`smelting` recipe, which answers
    // a different question than "is the recipe you asked for one of mine".
    // A goal naming a non-crafting recipe for a product that also has a
    // crafting one -- the modded case -- would otherwise be declined here and
    // then claimed by `HandCraft`, silently running the other recipe.
    if via.is_none() && cannot_possibly_apply(state, item) {
        return None;
    }
    let machines = MachineTable::from_state(state);
    let index = ProductIndex::from_state(state);
    // `via` is `None` for every goal written before 2026-09-07, and this call
    // then *is* `sole_recipe_producing` -- see `recipe_producing`'s doc.
    let recipe = index
        .recipe_producing(item, &Categories::planner_runs(&machines), via, &machines)
        .ok()?;
    // `crafting` and `smelting` belong to `HandCraft` and `Smelt`, which are
    // registered ahead of this and know far more about them -- banks,
    // adoption, shared ore. Declining them here is what keeps this method
    // incapable of moving an existing plan.
    if recipe.category == CRAFTING_CATEGORY || recipe.category == SMELTING_CATEGORY {
        return None;
    }
    let Machine::Entity(machine) = machines.machine_for(&recipe.category).ok()? else {
        return None;
    };
    if recipe_gate(state, recipe) == RecipeGate::Unobtainable {
        return None;
    }
    Some(Job {
        recipe: recipe.clone(),
        machine,
    })
}

// ---------------------------------------------------------------------------
// The fluid halves
// ---------------------------------------------------------------------------

/// A run of pipe this expansion is going to lay, resolved but not emitted.
struct Run {
    /// The fluid it carries, for the label a reader sees in the plan.
    fluid: String,
    /// The thing at the other end from the machine.
    other: String,
    /// Every tile that needs a pipe, in placement order.
    tiles: Vec<Position>,
}

/// Where the machine goes and what has to be piped to and from it.
///
/// **Resolved in one place, before a single step is emitted.** Siting the
/// machine is part of it because a fluid ingredient *moves the machine*: the
/// run has to be short and local, so the refinery is sited beside the tank
/// the crude arrives in rather than beside whichever bot happens to own the
/// chain.
struct FluidRig {
    site: Position,
    pipe: String,
    /// One run per fluid ingredient, each to its own input fluidbox.
    ///
    /// A `Vec` and not an `Option` since 2026-09-08: `sulfur` needs two, and
    /// the ordinals that say which box each joins come from
    /// [`pipe::fluid_box_ordinals`].
    inbound: Vec<Run>,
    /// One run per fluid **product**, each off its own output fluidbox.
    ///
    /// A `Vec` and not an `Option` since 2026-09-08: `advanced-oil-processing`
    /// makes three, and a machine whose second and third products have nowhere
    /// to go jams and then makes none of the first either. The runs are
    /// resolved one after another against the ground the previous ones took
    /// *and* against every other output box's port tiles -- see the module doc.
    outbound: Vec<Run>,
    /// The buffers to obtain and place, one per fluid product, in recipe
    /// order and index-aligned with `outbound`.
    buffers: Vec<(String, Position)>,
}

/// Every fluid product of `recipe`, in the game's own order, with its amount.
fn fluid_products(ctx: &ExpansionCtx, recipe: &FactorioRecipe) -> Vec<(String, u32)> {
    recipe
        .products
        .iter()
        .filter(|p| ctx.substances().is_fluid(&p.name))
        .map(|p| (p.name.clone(), p.amount))
        .collect()
}

/// What this world says a buffer prototype can hold -- **three answers, and
/// absent is not either of the others.**
///
/// This repo's recurring defect class is a lookup that cannot answer returning
/// the same thing as one answering zero, and `volume` is exactly that shape:
/// measured 2026-09-08, `map.json` and `map-31337-explored.json` carry
/// `volume: None` on every fluidbox of every prototype, while
/// `map-31337-water-and-oil.json` carries `storage-tank` 25,000 and the
/// `oil-refinery`'s three **output** boxes at 100 each. Both are current
/// captures of the same mod set; the field simply arrived between them.
///
/// So a caller must be able to tell the three apart, and the *message* must be
/// able to say which it got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BufferCapacity {
    /// At least one fluidbox declares a volume. The maximum over the boxes
    /// rather than the sum: a buffer's capacity is one box's, and summing
    /// would invent headroom out of a machine that happens to declare several.
    Declared(f64),
    /// The prototype has fluidboxes and **not one of them declares a volume**
    /// -- *"this capture predates the field"*, not *"it holds nothing"*.
    ///
    /// Errs towards **permitting**, which is the unsafe direction, and that is
    /// stated rather than hidden: the same choice `consumer_kw` makes for a
    /// prototype it has never heard of, for the same reason -- refusing on
    /// silence would break every archived dump, and a false refusal is a
    /// plausible lie about the world rather than a diagnosable failure.
    NotReported,
    /// The prototype is unknown to this world, or carries no fluidboxes at
    /// all. A thing with no fluidbox is not a buffer and cannot be piped
    /// into; this is a different fact from a buffer of unknown size, and
    /// collapsing the two would let a rig be planned against something that
    /// can never hold a fluid.
    NoFluidbox,
}

/// The declared capacity of `name`'s fluidboxes. See [`BufferCapacity`].
fn buffer_capacity(state: &PlanState, name: &str) -> BufferCapacity {
    let Some(boxes) = state
        .base()
        .globals
        .entity_prototypes
        .get(name)
        .and_then(|proto| proto.fluidbox_prototypes.clone())
        .filter(|boxes| !boxes.is_empty())
    else {
        return BufferCapacity::NoFluidbox;
    };
    boxes
        .iter()
        .filter_map(|b| b.volume)
        .max_by(|a, b| a.total_cmp(b))
        .map_or(BufferCapacity::NotReported, BufferCapacity::Declared)
}

/// Site the machine, resolve both pipe runs, and refuse before any of it is
/// emitted.
///
/// # The asymmetry between a source and a sink is deliberate
///
/// **A source is adopted and never built; a sink is built when none stands.**
/// A buffer this plan places is *empty*, which is exactly right for catching
/// an output and exactly wrong for feeding an input -- building a tank and
/// calling it a crude supply would be the "factory that quietly stops" the
/// owner ruled against, in its purest form. So the input end demands a
/// standing fluidbox and says so by name when there is none.
///
/// # What connectivity cannot check
///
/// Nothing in [`PlanState`] models what a standing tank *holds*, and no dump
/// this project has carries fluid contents. So the source is chosen by *"its
/// prototype has a box that can supply"* and by distance, and the plan's own
/// labels name it so a reader can see which one was picked. That is the
/// honest edge of the connectivity rule.
fn plan_fluid_rig(
    ctx: &ExpansionCtx,
    goal: &Goal,
    recipe: &FactorioRecipe,
    machine: &str,
    bill_fluids: &[(String, u32)],
    origin: &Position,
    runs: u32,
) -> Result<FluidRig, PlannerError> {
    let state = &ctx.state;
    // **Which box takes which fluid, before anything is sited.** A refusal
    // here costs nothing; a wrong answer builds a pipe that moves nothing.
    let undecidable = |direction: &'static str, u: pipe::BoxUndecidable| {
        PlannerError::CannotFabricate(Box::new(FabricateRefusal::FluidBoxUndecidable {
            recipe: recipe.name.clone(),
            machine: machine.to_string(),
            fluid: u.fluid,
            direction,
            ordinal: u.ordinal,
            fluids: u.fluids,
            boxes: u.boxes,
        }))
    };
    let input_ordinals = pipe::fluid_box_ordinals(state, machine, recipe, "input")
        .map_err(|u| undecidable("input", u))?;
    let output_ordinals = pipe::fluid_box_ordinals(state, machine, recipe, "output")
        .map_err(|u| undecidable("output", u))?;
    let box_of = |fluid: &str, ordinals: &[(String, usize)]| -> usize {
        ordinals
            .iter()
            .find(|(name, _)| name == fluid)
            .map_or(0, |(_, ordinal)| *ordinal)
    };
    let products = fluid_products(ctx, recipe);

    // Nothing fluid at all: the machine is sited where every other method
    // sites one, beside the bot that owns the chain, and this is the whole of
    // the answer.
    if bill_fluids.is_empty() && products.is_empty() {
        let Some(site) = free_area_near(state, origin, machine) else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        return Ok(FluidRig {
            site,
            pipe: String::new(),
            inbound: Vec::new(),
            outbound: Vec::new(),
            buffers: Vec::new(),
        });
    }

    let fluid_for_naming = bill_fluids
        .first()
        .map(|(f, _)| f.clone())
        .or_else(|| products.first().map(|(f, _)| f.clone()))
        .unwrap_or_default();
    let pipe = pipe_prototype(state, &fluid_for_naming, machine)?;

    // ---- the machine's site, which the source decides when there is one ----
    //
    // **Every fluid ingredient needs its own standing source, and every one
    // is resolved before the machine is sited.** Two fluids in is `sulfur`
    // (water and petroleum-gas), and a rig with one of the two connected is
    // the "factory that quietly stops" the owner ruled against: the plant
    // stands, the pipe is right, and nothing ever comes out.
    let mut sources: Vec<(String, FactorioEntity)> = Vec::with_capacity(bill_fluids.len());
    for (fluid, amount) in bill_fluids {
        let (attributable, rejected) = pipe::sources_of(state, fluid, origin);
        let Some(source) = attributable.into_iter().next() else {
            let recipes: Vec<FactorioRecipe> = state
                .base()
                .globals
                .recipes
                .iter()
                .map(|entry| entry.value().clone())
                .collect();
            return Err(PlannerError::CannotFabricate(Box::new(
                FabricateRefusal::NoFluidSource {
                    recipe: recipe.name.clone(),
                    category: recipe.category.clone(),
                    machine: machine.to_string(),
                    fluid: fluid.clone(),
                    amount: *amount,
                    produced_by: FluidSource::of(recipes.iter(), fluid),
                    considered: rejected
                        .iter()
                        .take(5)
                        .map(|e| format!("the {} at {}", e.name, e.position))
                        .collect(),
                },
            )));
        };
        sources.push((fluid.clone(), source));
    }
    let anchor = sources
        .first()
        .map(|(_, entity)| entity.position.clone())
        .unwrap_or_else(|| origin.clone());
    // **The machine goes where its own ports fit.** A site flush against the
    // source puts the machine's input port inside the source; see
    // `pipe::port_is_placeable`, which is that refusal turned into a siting
    // predicate.
    // **Every box this recipe uses has to fit, not just the first.** A site
    // chosen on one input port and refused on the other is the two-searches
    // failure the outbound run already paid for once, one fluid further in.
    let in_boxes: Vec<usize> = sources
        .iter()
        .map(|(fluid, _)| box_of(fluid, &input_ordinals))
        .collect();
    // **Every product's box, not just the first.** A site chosen on one
    // output port and refused on another is the same "two searches that never
    // agree on a site" the inbound runs already paid for, one product further
    // out -- and with three products there are three chances to hit it.
    let out_boxes: Vec<usize> = products
        .iter()
        .map(|(fluid, _)| box_of(fluid, &output_ordinals))
        .collect();
    let ports_fit = |candidate: &Position| {
        in_boxes.iter().all(|ordinal| {
            pipe::port_is_placeable(
                state,
                machine,
                candidate,
                Some("input"),
                Some(*ordinal),
                &pipe,
            )
        }) && out_boxes.iter().all(|ordinal| {
            pipe::port_is_placeable(
                state,
                machine,
                candidate,
                Some("output"),
                Some(*ordinal),
                &pipe,
            )
        })
    };
    // **A machine's site is only clear if every inbound run can reach it** --
    // the buffer's own acceptance test (see `routes_to` below), applied one
    // level up to the machine that buffer hangs off.
    //
    // It was missing here, and the asymmetry showed the day a *second* fluid
    // machine was sited beside a first: `have:plastic-bar:10` on the
    // seed-31337 water-and-oil dump stood a pumpjack, a refinery running
    // `basic-oil-processing` and its output tank, then sited the
    // `chemical-plant` at `[139.5, -359.5]` and refused with `no pipe route
    // from the oil-refinery at [143.5, -358.5] ... the tile at [140.5,
    // -361.5] cannot hold a pipe`. Nothing was wrong with either half.
    // `port_is_placeable` asks whether **any** tile of the chosen box is
    // free; `route_between` then has to start from a **particular** one. So
    // siting could accept a footprint routing could not use, which is the
    // same "two searches that never agree on a site" the buffer paid for.
    //
    // The route is therefore the acceptance test here too, and the ring
    // search takes the first site that fits *and* routes -- the nearest such
    // site by construction.
    //
    // **Necessary, not sufficient, and deliberately so.** It routes against
    // bare ground (`&[]`) because the tiles a *later* run will want are not
    // known until a site is chosen -- `other_port_tiles` is derived from the
    // site. A multi-source machine can therefore still refuse below, on the
    // second run, exactly as it does today; what this removes is the case
    // where a perfectly routable site two rings further out was never tried.
    let routes_from_every_source = |candidate: &Position| {
        let Some(area) = state.collision_area(machine, candidate) else {
            return false;
        };
        sources.iter().all(|(fluid, entity)| {
            let source_area = state
                .collision_area(&entity.name, &entity.position)
                .unwrap_or_else(|| entity.bounding_box.clone());
            route_between(
                state,
                &PipeEnd {
                    name: &entity.name,
                    position: &entity.position,
                    area: source_area,
                    production_type: None,
                    port_index: None,
                },
                &PipeEnd {
                    name: machine,
                    position: candidate,
                    area: area.clone(),
                    production_type: Some("input"),
                    port_index: Some(box_of(fluid, &input_ordinals)),
                },
                &pipe,
                &[],
            )
            .is_ok()
        })
    };
    let site_is_usable =
        |candidate: &Position| ports_fit(candidate) && routes_from_every_source(candidate);
    let Some(site) = free_area_near_where(state, &anchor, machine, site_is_usable) else {
        return Err(PlannerError::NoApplicableMethod {
            goal: goal.to_string(),
        });
    };
    let Some(machine_area) = state.collision_area(machine, &site) else {
        return Err(PlannerError::FluidPortUnknown {
            prototype: machine.to_string(),
            why: "the world has no collision box for it, so its footprint cannot be reserved"
                .to_string(),
        });
    };

    // **The machine's *other* port is ground the second run will need.**
    //
    // The two runs are routed one after the other against a world where
    // neither is emitted, and `route_between` pushes its two ends' port tiles
    // into the result unconditionally -- outside the search, so no obstacle
    // grid can keep them apart. So the inbound run, routed first and free to
    // pick any side of the refinery, ran straight over the output port the
    // outbound run then had to start from: `[134.5, -354.5]` and
    // `[134.5, -355.5]` in both runs of `gathered:crude-oil` + `producing
    // petroleum-gas:45` on seed 31337. Two `Place`s on one tile, and `expand`
    // died on the second's own `AreaFree` -- `precondition pipe fits at
    // [134.5, -354.5] ... does not hold` -- naming a tile and saying nothing
    // about the run that had taken it.
    //
    // Reserved here rather than inside `route_between` because only this
    // function knows a *second* run is coming: a lone run to a machine may
    // use whichever side it likes.
    //
    // **With more than one product this stops being an ordering nicety and
    // becomes the jam guard.** The three runs off an `oil-refinery` leave
    // from port tiles three in a row, so each one has to keep off the other
    // two -- see the module doc, and `FluidOutputsWouldShareAPipe` for the
    // check that follows, because reserving ground is necessary and not
    // sufficient.
    let port_tiles = |direction: &'static str,
                      wanted: &[usize]|
     -> Result<Vec<(usize, Position, Rect)>, PlannerError> {
        Ok(pipe::fluid_ports(state, machine, &site, Some(direction))?
            .into_iter()
            .filter(|port| wanted.contains(&port.box_ordinal))
            .flat_map(|port| {
                let ordinal = port.box_ordinal;
                port.tiles().into_iter().map(move |tile| (ordinal, tile))
            })
            .filter_map(|(ordinal, tile)| {
                state
                    .collision_area(&pipe, &tile)
                    .map(|area| (ordinal, tile, area))
            })
            .collect())
    };
    // Every output port of every product, kept whole so each outbound run
    // below can exclude the *others* by ordinal.
    let output_port_tiles: Vec<(usize, Position, Rect)> = if products.is_empty() {
        Vec::new()
    } else {
        port_tiles("output", &out_boxes)?
    };
    // The inbound runs keep off all of them: an inbound run is free to pick
    // any side of the machine, and whichever output port it crosses is one
    // this rig is about to stand a pipe on.
    let mut other_port_tiles: Vec<Rect> = if sources.is_empty() {
        Vec::new()
    } else {
        output_port_tiles
            .iter()
            .map(|(_, _, area)| area.clone())
            .collect()
    };

    // ---- in ----
    //
    // One run per fluid, each routed against the ground the previous ones
    // took. The same reservation argument as the outbound run below, one
    // level up: two inbound runs into one chemical plant are routed one after
    // the other against a world where neither is emitted, so the second would
    // happily lay a pipe on the first's tiles and die on its own `AreaFree`.
    let mut inbound: Vec<Run> = Vec::with_capacity(sources.len());
    for (fluid, entity) in &sources {
        let area = state
            .collision_area(&entity.name, &entity.position)
            .unwrap_or_else(|| entity.bounding_box.clone());
        let tiles = route_between(
            state,
            &PipeEnd {
                name: &entity.name,
                position: &entity.position,
                area,
                production_type: None,
                port_index: None,
            },
            &PipeEnd {
                name: machine,
                position: &site,
                area: machine_area.clone(),
                production_type: Some("input"),
                port_index: Some(box_of(fluid, &input_ordinals)),
            },
            &pipe,
            &other_port_tiles,
        )?;
        other_port_tiles.extend(
            tiles
                .iter()
                .filter_map(|tile| state.collision_area(&pipe, tile)),
        );
        inbound.push(Run {
            fluid: fluid.clone(),
            other: entity.name.clone(),
            tiles,
        });
    }

    // ---- out ----
    //
    // **One buffer and one run per fluid product, resolved in order and each
    // against everything the previous ones took.**
    //
    // The single-product version of this loop is unchanged in every detail
    // below -- the buffer's own port tiles, the machine's footprint, the
    // route-as-acceptance-test. What is new is that `taken` now also carries
    // the *other* products' output port tiles and the runs and tanks already
    // resolved, so three runs off one refinery cannot converge on one tile.
    let mut buffers: Vec<(String, Position)> = Vec::with_capacity(products.len());
    let mut outbound: Vec<Run> = Vec::with_capacity(products.len());
    for (fluid, per_craft) in &products {
        let tank = buffer_prototype(state, fluid, machine)?;
        let ordinal = box_of(fluid, &output_ordinals);
        // **The whole goal's output, not one craft's.** `runs` is finite for
        // every goal that reaches this method (see `SinkTooSmall`'s doc), so
        // this is a number rather than a rate; `u64` because the product of
        // two `u32`s is not a `u32`.
        let total = u64::from(*per_craft) * u64::from(runs);
        match buffer_capacity(state, &tank) {
            BufferCapacity::Declared(volume) if volume < total as f64 => {
                return Err(PlannerError::CannotFabricate(Box::new(
                    FabricateRefusal::SinkTooSmall {
                        recipe: recipe.name.clone(),
                        machine: machine.to_string(),
                        fluid: fluid.clone(),
                        per_craft: *per_craft,
                        runs,
                        total,
                        buffer: tank.clone(),
                        volume,
                    },
                )));
            }
            // The world states a capacity and it is enough.
            BufferCapacity::Declared(_) => {}
            // **This capture predates `volume`.** Permit, which is the unsafe
            // direction, and say so where a reader will see it rather than
            // here -- the buffer's own placement label carries it.
            BufferCapacity::NotReported => {}
            // Not a buffer at all. `buffer_prototype` chose it, so this is a
            // statement about the world rather than about the choice, and it
            // is the one case where silence must not permit: a pipe into a
            // thing with no fluidbox is a pipe into nothing.
            BufferCapacity::NoFluidbox => {
                return Err(PlannerError::FluidPortUnknown {
                    prototype: tank.clone(),
                    why: "this world gives it no fluidbox_prototypes at all, so nothing can be \
                          piped into it and it cannot be the sink for a fluid product"
                        .to_string(),
                });
            }
        }

        // The inbound runs' tiles are ground already spoken for: they are
        // resolved and not emitted, so nothing on any grid knows about them
        // and a tank sited blindly would stand on one.
        let mut taken: Vec<Rect> = inbound
            .iter()
            .flat_map(|run| run.tiles.iter())
            .filter_map(|tile| state.collision_area(&pipe, tile))
            .collect();
        // **And the machine's own footprint, for the same reason.** It is
        // resolved and not emitted either, so `free_area_near_where` reads
        // the ground under it as clear and hands the buffer the machine's
        // own site: `place oil-refinery at [137.5, -352.5]` and `place
        // storage-tank at [137.5, -352.5]` in one plan, on
        // `gathered:crude-oil` + `producing:petroleum-gas:45`, seed 31337.
        // Whichever is placed second fails its own `AreaFree`, and the
        // refusal names a tile rather than the two things that wanted it.
        taken.push(machine_area.clone());
        // **And every OTHER output box's port tiles.** This is the jam guard
        // of the module doc: a run leaving box 1 that crosses box 0's port
        // joins two boxes into one pipe segment, a segment holds one fluid,
        // and the machine then produces none of its three products. Its own
        // box is deliberately not excluded -- that is where this run starts.
        taken.extend(
            output_port_tiles
                .iter()
                .filter(|(other, _, _)| *other != ordinal)
                .map(|(_, _, area)| area.clone()),
        );
        // **And the input ports, and everything the earlier products took.**
        // `other_port_tiles` accumulated the inbound runs' own tiles above.
        taken.extend(other_port_tiles.iter().cloned());
        for run in &outbound {
            taken.extend(
                run.tiles
                    .iter()
                    .filter_map(|tile| state.collision_area(&pipe, tile)),
            );
        }
        for (earlier, position) in &buffers {
            if let Some(area) = state.collision_area(earlier, position) {
                taken.push(area);
            }
        }
        // **A site is only clear if a run can actually reach it.**
        //
        // Siting and routing were two searches: `free_area_near_where`
        // returned the nearest footprint that fitted, and the route was
        // asked afterwards and could only refuse. Excluding the machine's
        // own ground (the line above) is what exposed that -- the buffer
        // moved off the refinery onto the next fitting tile, and the
        // outbound run then had to cross the inbound one to get there:
        // `no route to the storage-tank's connection ..., blocked by 4
        // tile(s)`, on eight of this module's own fixtures at once.
        // Nothing was wrong with either half; they simply never agreed on
        // a site.
        //
        // So the route is the acceptance test. `free_area_near_where`
        // walks its rings outward and takes the first site that *fits and
        // routes*, which is the nearest such site by construction. The
        // winner is routed twice -- once here, once below -- because
        // `accept` is `Fn` and cannot hand the route back; that is one
        // extra search per expansion, against a refusal for a rig that was
        // buildable two tiles further out.
        let routes_to = |candidate: &Position, area: Rect| {
            route_between(
                state,
                &PipeEnd {
                    name: machine,
                    position: &site,
                    area: machine_area.clone(),
                    production_type: Some("output"),
                    port_index: Some(ordinal),
                },
                &PipeEnd {
                    name: &tank,
                    position: candidate,
                    area,
                    production_type: None,
                    port_index: None,
                },
                &pipe,
                &taken,
            )
            .is_ok()
        };
        // **A buffer's own PORT TILES are ground it needs, and they are
        // not inside its footprint.**
        //
        // A `storage-tank`'s pipe connections sit at its corners, one
        // tile diagonally *out*, so a tank cleared of the machine by its
        // footprint alone still reaches under it: on
        // `gathered:crude-oil` + `produced:petroleum-gas:45:
        // basic-oil-processing`, seed 31337, the ring search took the
        // first ring whose 2.59-wide box clears a 4.4-wide `oil-refinery`
        // -- and **every candidate on that ring** has a connection tile
        // on the refinery's own bottom row (`[141.5, -360.5]` ...
        // `[145.5, -360.5]` for a refinery at `[143.5, -358.5]`).
        //
        // `route_between` then pushes both ends' port tiles into the run
        // *unconditionally, outside the search* -- its own doc says so --
        // and checks them with `is_area_free`, which cannot see a machine
        // that is not emitted yet. So the run laid a pipe on the
        // refinery's footprint, nothing ordered the two placements, and
        // the plan died at SCHEDULE time on the refinery's own
        // `AreaFree`: `oil-refinery fits at [143.5, -358.5] ... does not
        // hold there`, which named a tile and blamed a chain owner for a
        // fact about the ground.
        //
        // This is the mirror of `pipe::port_is_placeable`, which asks
        // whether the *machine's* port survives the *source*. Nobody
        // asked the reverse until an oil field with a charted shoreline
        // put a refinery four tiles from its buffer.
        let ports_clear = |candidate: &Position| {
            let Ok(ports) = pipe::fluid_ports(state, &tank, candidate, None) else {
                return false;
            };
            ports.iter().flat_map(FluidPort::tiles).all(|tile| {
                state
                    .collision_area(&pipe, &tile)
                    .is_some_and(|area| !taken.iter().any(|t| overlaps(&area, t)))
            })
        };
        let clear = |candidate: &Position| {
            state.collision_area(&tank, candidate).is_some_and(|area| {
                !taken.iter().any(|t| overlaps(&area, t))
                    && ports_clear(candidate)
                    && routes_to(candidate, area.clone())
            })
        };
        let Some(tank_site) = free_area_near_where(state, &site, &tank, clear) else {
            return Err(PlannerError::CannotFabricate(Box::new(
                FabricateRefusal::NoSinkSite {
                    recipe: recipe.name.clone(),
                    machine: machine.to_string(),
                    fluid: fluid.clone(),
                    buffer: tank.clone(),
                    site: site.to_string(),
                },
            )));
        };
        let Some(tank_area) = state.collision_area(&tank, &tank_site) else {
            return Err(PlannerError::FluidPortUnknown {
                prototype: tank.clone(),
                why: "the world has no collision box for it, so its footprint cannot be \
                      reserved"
                    .to_string(),
            });
        };
        let tiles = route_between(
            state,
            &PipeEnd {
                name: machine,
                position: &site,
                area: machine_area.clone(),
                production_type: Some("output"),
                // The box the recipe's own `fluidbox_index` names, or the
                // one its position in the recipe implies. Never `None`:
                // `select(ports, None)` returns *every* port, and on a
                // three-output refinery that is a run to whichever box the
                // pathfinder reaches first.
                port_index: Some(ordinal),
            },
            &PipeEnd {
                name: &tank,
                position: &tank_site,
                area: tank_area,
                production_type: None,
                port_index: None,
            },
            &pipe,
            &taken,
        )?;

        // **Reserving the ground was necessary and is not sufficient.**
        //
        // `route_between` pushes both ends' port tiles into the result
        // *unconditionally, outside its own search*, so `taken` cannot
        // stop a foreign port tile arriving in the run. Nothing before
        // this line has actually established the thing the module doc
        // promises, and a refinery whose boxes share a segment jams
        // silently. So the run is inspected.
        if let Some((other, tile, _)) = output_port_tiles
            .iter()
            .find(|(other, tile, _)| *other != ordinal && tiles.contains(tile))
        {
            let name_of = |wanted: usize| {
                products
                    .iter()
                    .zip(&out_boxes)
                    .find(|(_, o)| **o == wanted)
                    .map_or_else(|| format!("box {wanted}"), |((f, _), _)| f.clone())
            };
            return Err(PlannerError::CannotFabricate(Box::new(
                FabricateRefusal::FluidOutputsWouldShareAPipe {
                    recipe: recipe.name.clone(),
                    machine: machine.to_string(),
                    site: site.to_string(),
                    first: fluid.clone(),
                    second: name_of(*other),
                    tile: tile.to_string(),
                    products: products.len(),
                },
            )));
        }

        buffers.push((tank.clone(), tank_site));
        outbound.push(Run {
            fluid: fluid.clone(),
            other: tank,
            tiles,
        });
    }

    Ok(FluidRig {
        site,
        pipe,
        inbound,
        outbound,
        buffers,
    })
}

/// Do two footprints share any ground? Touching edges do not count.
fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.left_top.x() < b.right_bottom.x()
        && b.left_top.x() < a.right_bottom.x()
        && a.left_top.y() < b.right_bottom.y()
        && b.left_top.y() < a.right_bottom.y()
}

/// Run one recipe in the machine that runs its category.
pub struct Fabricate;

impl Method for Fabricate {
    fn name(&self) -> &'static str {
        "fabricate"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        job_for(goal, state).is_some()
    }

    /// **The machine, its buffer and every pipe of its rig are placed out of a
    /// hand this method's own subgoals filled.**
    ///
    /// Each is stated as `Goal::Have { whose }` -- the goal's own holder,
    /// propagated verbatim -- and then placed by a sibling `Step::Act`
    /// carrying a `HasItem { who: Role }`. They arrive through separate
    /// subgoals and nothing has to *meet*, so [`Method::converges`] is
    /// honestly `false`; what is true is the weaker claim
    /// [`Method::hands_over`] names, and without it the whole rig went
    /// unchained while its bill was sized against `ctx.chain_actor`.
    ///
    /// Measured on `gathered:crude-oil` + `producing:petroleum-gas:45`, seed
    /// 31337: the refinery's nine unchained `place pipe` actions settled on
    /// bot 1 and spent the ten pipes bot 1 had crafted for `craft 1
    /// oil-refinery` -- an action of a chain **owned** by bot 1, sized against
    /// it, and right about what it needed. The plan died naming that chain.
    fn hands_over(&self, goal: &Goal, state: &PlanState) -> bool {
        job_for(goal, state).is_some()
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some(Demand {
            item,
            need,
            whose,
            unlocks,
            ..
        }) = demand(goal, &ctx.state)
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let Some(Job { recipe, machine }) = job_for(goal, &ctx.state) else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };

        // ---- refuse first, emit nothing until every wall is behind us ----

        // The expansion's own cached table -- built once per expansion, the
        // rule `SubstanceTable`'s doc states.
        let bill = split_bill(ctx.substances(), &recipe);
        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        // **How many times the machine runs, computed before the rig and not
        // after.** It used to be derived below, among the emission; the sink
        // test needs it, because what has to fit in a buffer is the whole
        // goal's output and not one craft's. Moving it up changes no value --
        // it reads only `recipe`, `item` and `need`, all of which are already
        // settled here.
        let per_craft = output_per_craft(&recipe, item);
        let runs = need.div_ceil(per_craft);
        // Both fluid ends, and the machine's site with them: a fluid
        // ingredient moves the machine next to the thing that supplies it.
        // Every refusal in here is returned before a step is emitted.
        let rig = plan_fluid_rig(ctx, goal, &recipe, &machine, &bill.fluids, &from, runs)?;
        let site = rig.site.clone();

        // ---- nothing below refuses; from here it is all emission ----

        // The machine's own speed against the recipe's own energy. Named
        // `smelting_ticks` for its first caller and generic in what it does:
        // it reads `crafting_speed` for whichever entity is acting.
        let per_run = smelting_ticks(&ctx.state, &recipe, &machine);

        let build = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.build_distance)
            .unwrap_or(10.0);
        let reach = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.reach_distance)
            .unwrap_or(10.0);
        let min_radius = ctx.state.placement_clearance(&machine).unwrap_or(0.0);

        let mut steps: Vec<Step> = Vec::new();
        let mut research_pre: Vec<Condition> = Vec::new();
        match recipe_gate(&ctx.state, &recipe) {
            RecipeGate::NeedsResearch(tech) => {
                steps.push(Step::Subgoal(Goal::Researched(tech.clone())));
                research_pre.push(Condition::Researched(tech));
            }
            RecipeGate::PlannedResearch(tech) => research_pre.push(Condition::Researched(tech)),
            RecipeGate::Open | RecipeGate::Unobtainable => {}
        }

        // The machine itself, as a subgoal: obtaining it is somebody else's
        // problem, and `have:oil-refinery:1` already plans.
        steps.push(Step::Subgoal(Goal::Have {
            item: machine.clone(),
            count: 1,
            whose: whose.clone(),
            via: None,
        }));
        for (ingredient, amount) in &bill.items {
            steps.push(Step::Subgoal(Goal::Have {
                item: ingredient.clone(),
                count: amount.saturating_mul(runs),
                whose: whose.clone(),
                via: None,
            }));
        }

        // ---- power, before the machine is placed and not after ------------
        //
        // **An `oil-refinery` draws 420 kW and this method used to emit none
        // of it.** No `Condition::Powered`, no call to `ensure_powered`, no
        // pole: the plan stood the largest consumer this planner has ever
        // placed on no network at all, and `gathered:crude-oil` +
        // `produced:petroleum-gas:45:basic-oil-processing` planned 2,296
        // actions of it (seed 31337, 2026-09-08). A machine on no network
        // stands there, sets its recipe, draws nothing and makes nothing --
        // and it reads as built.
        //
        // The same call `method::extract` makes for a pumpjack and
        // `method::blueprint` makes for a block, with the same three answers:
        // `Err` is supply itself being impossible and says why by name,
        // `Ok(None)` is "no pole run this planner will build carries power
        // here", and `Ok(Some(_))` carries the steps, the ids to order the
        // placement after, and the headroom condition to state on it.
        //
        // **It provides GENERATION as well as connection when it has to**:
        // `supply_anchor` adopts a standing network where one is in reach and
        // otherwise builds a plant inline (offshore pump, boiler, engine).
        // What it does *not* do is guess -- a site it cannot reach refuses.
        //
        // Whether it is needed at all is asked of `crate::powered::PowerNeed`,
        // which reads the game's own energy source through the two fields the
        // mod gates on an electric one. A burner machine and a machine the
        // world never described are two different answers and neither is
        // "draws nothing"; an electric machine nobody can price is refused by
        // `powered::audit` over the finished plan rather than silently
        // budgeted at zero.
        let machine_area = ctx.state.collision_area(&machine, &site);
        let mut powered_pre: Vec<Condition> = Vec::new();
        let mut power_ids: Vec<crate::ids::ActionId> = Vec::new();
        if let (crate::powered::PowerNeed::Electric { kw: Some(kw) }, Some(area)) = (
            crate::powered::PowerNeed::of(&ctx.state, &machine),
            machine_area,
        ) {
            // The ground the rig is about to take: the machine itself, every
            // pipe of every run, and the buffer. Handed over as occupants so
            // a pole cannot be sited on a tile this method is about to place
            // a pipe on -- the same reservation `method::gather` passes
            // `method::extract` for exactly that failure, where the pipe's own
            // `AreaFree` fails at execution after the plan was called good.
            //
            // It has one consequence worth stating: with more than one
            // occupant `ensure_powered` states a `BlockPowered`, whose
            // exclusion is the rig's whole ground rather than the machine's
            // tile, so a consumer already standing inside that rectangle is
            // not charged against this machine's draw. The realistic instance
            // is the pumpjack this rig is piped from, at 90 kW, and it was
            // itself charged when `method::extract` sized the supply that
            // answered for it.
            let mut occupants = vec![plain_entity(&ctx.state, &machine, &site)];
            for run in rig.inbound.iter().chain(rig.outbound.iter()) {
                for tile in &run.tiles {
                    occupants.push(plain_entity(&ctx.state, &rig.pipe, tile));
                }
            }
            for (tank, tank_site) in &rig.buffers {
                occupants.push(plain_entity(&ctx.state, tank, tank_site));
            }
            let powering = crate::method::power::ensure_powered(
                ctx,
                &machine,
                &site,
                &area,
                kw,
                crate::method::extract::SUPPLY_SEARCH_RADIUS,
                &occupants,
            )?
            .ok_or_else(|| {
                PlannerError::CannotFabricate(Box::new(FabricateRefusal::NoPowerRoute {
                    recipe: recipe.name.clone(),
                    machine: machine.clone(),
                    kw,
                    site: site.to_string(),
                }))
            })?;
            steps.extend(powering.steps);
            power_ids = powering.ids;
            powered_pre.push(powering.powered);
        }

        let entity = FactorioEntity {
            name: machine.clone(),
            entity_type: ctx
                .state
                .base()
                .globals
                .entity_prototypes
                .get(machine.as_str())
                .map(|p| p.entity_type.clone())
                .unwrap_or_default(),
            position: site.clone(),
            ..Default::default()
        };
        let place_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: place_id,
            kind: ActionKind::Place {
                entity: Box::new(entity.clone()),
            },
            pre: {
                let mut pre = vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: site.clone(),
                        radius: build,
                        min_radius,
                    },
                    Condition::AreaFree {
                        pos: site.clone(),
                        entity: machine.clone(),
                        direction: 0,
                    },
                    Condition::HasItem {
                        who: Actor::Role,
                        item: machine.clone(),
                        count: 1,
                    },
                ];
                // Coverage is not capacity: the headroom test `ensure_powered`
                // chose the plant and the pole run for, stated on the
                // placement so the scheduler re-checks it. Empty for a machine
                // that needs no network.
                pre.extend(powered_pre.iter().cloned());
                pre
            },
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: machine.clone(),
                    count: 1,
                },
                Effect::CreateEntity(Box::new(entity)),
            ],
            duration: PLACE_TICKS,
            pinned: None,
            label: format!("place {machine} at {site}"),
        })));
        ctx.state.create_entity(FactorioEntity {
            name: machine.clone(),
            position: site.clone(),
            ..Default::default()
        });
        // Nothing satisfies `Condition::Powered`, so `infer_edges` draws no
        // edge from the plant or the poles to the placement that needs them.
        // The method holds both ends, so the method states the edges -- the
        // same close `method::extract` makes.
        for id in power_ids {
            steps.push(Step::Link {
                from: id,
                to: place_id,
                lag: 0,
            });
        }

        // A furnace picks its recipe from what it is fed; every other crafting
        // machine has to be told. Emitted unconditionally rather than gated on
        // `entity_type`, because a machine reached through this method is by
        // construction not the furnace `Smelt` owns.
        let recipe_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: recipe_id,
            kind: ActionKind::SetRecipe {
                pos: site.clone(),
                entity: machine.clone(),
                recipe: recipe.name.clone(),
            },
            pre: {
                let mut pre = vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: site.clone(),
                        radius: reach,
                        min_radius: 0.0,
                    },
                    Condition::EntityAt {
                        pos: site.clone(),
                        name: machine.clone(),
                    },
                ];
                pre.extend(research_pre.iter().cloned());
                pre
            },
            eff: vec![Effect::SetRecipe {
                pos: site.clone(),
                recipe: recipe.name.clone(),
            }],
            duration: SET_RECIPE_TICKS,
            pinned: None,
            label: format!("set {machine} to {}", recipe.name),
        })));
        ctx.state.set_recipe(&site, &recipe.name)?;

        let mut insert_ids = Vec::new();
        for (ingredient, amount) in &bill.items {
            let total = amount.saturating_mul(runs);
            let id = ctx.ids.next();
            insert_ids.push(id);
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::Insert {
                    pos: site.clone(),
                    entity: machine.clone(),
                    slot: InventorySlot::AssemblerInput,
                    item: ingredient.clone(),
                    count: total,
                },
                pre: vec![
                    Condition::AtPosition {
                        who: Actor::Role,
                        pos: site.clone(),
                        radius: reach,
                        min_radius: 0.0,
                    },
                    Condition::EntityAt {
                        pos: site.clone(),
                        name: machine.clone(),
                    },
                    Condition::HasItem {
                        who: Actor::Role,
                        item: ingredient.clone(),
                        count: total,
                    },
                ],
                eff: vec![Effect::LoseItem {
                    who: Actor::Role,
                    item: ingredient.clone(),
                    count: total,
                }],
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!("load the {machine} with {total} {ingredient}"),
            })));
        }

        // ---- the pipes, and the buffer the output lands in -----------------
        //
        // Emitted after the machine so a reader follows the arrangement in
        // the order it comes into being, and after `SetRecipe` for no
        // stronger reason than that: no ordering edge is needed, because the
        // route was searched around the machine's own footprint and the
        // `AreaFree` on every placement is the executor's check.
        let mut last_pipe: Option<usize> = None;
        for run in rig.inbound.iter().chain(rig.outbound.iter()) {
            let count = u32::try_from(run.tiles.len()).unwrap_or(u32::MAX);
            steps.push(Step::Subgoal(Goal::Have {
                item: rig.pipe.clone(),
                count,
                whose: whose.clone(),
                via: None,
            }));
            for position in &run.tiles {
                let entity = plain_entity(&ctx.state, &rig.pipe, position);
                steps.push(place_step(
                    ctx,
                    entity,
                    &format!(
                        "carry {} between the {machine} and the {}",
                        run.fluid, run.other
                    ),
                ));
                last_pipe = Some(steps.len() - 1);
            }
        }
        // One buffer per fluid product, index-aligned with `rig.outbound` so
        // the label can name the fluid it catches. With three of them a plan
        // reader needs to know which tank is which, and "catch what the
        // oil-refinery makes" three times over says nothing.
        for ((tank, tank_site), run) in rig.buffers.iter().zip(rig.outbound.iter()) {
            steps.push(Step::Subgoal(Goal::Have {
                item: tank.clone(),
                count: 1,
                whose: whose.clone(),
                via: None,
            }));
            let entity = plain_entity(&ctx.state, tank, tank_site);
            // **The label says when the capacity could not be checked.** A
            // capture that predates `volume` permits the buffer (see
            // `BufferCapacity::NotReported`), and erring towards permitting is
            // only honest if a reader can see that it happened.
            let unchecked = matches!(
                buffer_capacity(&ctx.state, tank),
                BufferCapacity::NotReported
            );
            steps.push(place_step(
                ctx,
                entity,
                &format!(
                    "catch the {} the {machine} at {site} makes{}",
                    run.fluid,
                    if unchecked {
                        " (this capture reports no fluidbox volume, so its capacity was not \
                         checked)"
                    } else {
                        ""
                    }
                ),
            ));
        }

        // A fluid product is not taken: no inventory holds one, and the
        // arrangement above is what "produced" means for it. So the take step
        // -- and the `Effect::GainItem` in it -- is emitted only for an item,
        // and the technology this production triggers moves to the last pipe
        // laid, which is the action after which the machine can actually run.
        if ctx.substances().is_fluid(item) {
            if let Some(tech) = unlocks
                && let Some(index) = last_pipe
                && let Step::Act(action) = &mut steps[index]
            {
                action.eff.push(Effect::Researched(tech.to_string()));
            }
            steps.push(Step::Link {
                from: place_id,
                to: recipe_id,
                lag: 0,
            });
            debug_assert!(matches!(
                whose,
                Holder::Anyone | Holder::Bot(_) | Holder::Share(_)
            ));
            return Ok(steps);
        }

        let take = runs.saturating_mul(per_craft).min(need);
        let take_id = ctx.ids.next();
        let mut take_eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: ItemId::from(item.clone()),
            count: take,
        }];
        // The technology this production triggers, if any -- it has to land on
        // whichever action produces the item, and here that is the take. The
        // same contract `Goal::Produced`'s doc states.
        if let Some(tech) = unlocks {
            take_eff.push(Effect::Researched(tech.to_string()));
        }
        steps.push(Step::Act(Box::new(Action {
            id: take_id,
            kind: ActionKind::Remove {
                pos: site.clone(),
                entity: machine.clone(),
                slot: InventorySlot::AssemblerOutput,
                item: ItemId::from(item.clone()),
                count: take,
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: site.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::EntityAt {
                    pos: site.clone(),
                    name: machine.clone(),
                },
            ],
            eff: take_eff,
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("take {take} {item} from the {machine} at {site}"),
        })));

        // The machine's own time, charged as a lag on each insert -- the
        // executor's rule for a take is `max over preds (finish(pred) + lag)`,
        // so charging the whole run on every insert is exact whichever lands
        // last, and a max is not a sum.
        let lag = per_run.saturating_mul(runs);
        for id in insert_ids {
            steps.push(Step::Link {
                from: id,
                to: take_id,
                lag,
            });
        }
        steps.push(Step::Link {
            from: place_id,
            to: recipe_id,
            lag: 0,
        });

        // The holder the goal named keeps the product; nothing here shares.
        debug_assert!(matches!(
            whose,
            Holder::Anyone | Holder::Bot(_) | Holder::Share(_)
        ));
        Ok(steps)
    }
}

#[cfg(test)]
mod fabricate_fluid_tests {
    use super::*;
    use crate::ids::BotId;
    use crate::test_world::{OilFixture, PumpjackRecipe, world_with_oil};
    use factorio_bot_core::factorio::world::FactorioSurface;
    use factorio_bot_core::types::FactorioEntity;
    use std::sync::Arc;

    /// The oil ladder's fixture with the wells charted, as `method::gather`'s
    /// tests use it. **Not written for this code**, which is the point.
    const OIL: OilFixture = OilFixture {
        wells: true,
        categories: true,
        pumpjack: PumpjackRecipe::LockedBy { researched: true },
        prerequisite: false,
    };

    /// The wells run east from (20.5, 20.5); this is clear ground beside them.
    fn tank_site() -> Position {
        Position::new(24.5, 26.5)
    }

    /// The fixture, plus what the *game* has and the capture predates: an
    /// `oil-refinery` that says it crafts `oil-processing`, and the recipe.
    ///
    /// Both are transcribed rather than invented -- the category from
    /// 2.1.17's `entities.lua`, the recipe's amounts (100 crude in, 45
    /// petroleum out) from the live capture the CLI measurements use.
    fn oil_world(refinery_category: bool) -> FactorioSurface {
        let world = world_with_oil(OIL);
        if refinery_category {
            world
                .globals
                .entity_prototypes
                .get_mut("oil-refinery")
                .expect("the fixture has an oil-refinery prototype")
                .crafting_categories = Some(vec!["oil-processing".into()]);
        }
        let recipe: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "basic-oil-processing", "valid": true, "enabled": true,
              "category": "oil-processing",
              "ingredients": [
                { "name": "crude-oil", "ingredient_type": "fluid", "amount": 100 }
              ],
              "products": [
                { "name": "petroleum-gas", "product_type": "fluid", "amount": 45,
                  "probability": 1.0 }
              ],
              "hidden": false, "energy": 5.0, "order": "a-a",
              "group": "intermediate-products", "subgroup": "fluid-recipes"
            }"#,
        )
        .expect("the basic-oil-processing recipe parses");
        world
            .update_recipes(vec![recipe])
            .expect("update_recipes cannot fail for a well-formed recipe");
        world
    }

    /// The same world, with the recipe naming **which output fluidbox** the
    /// petroleum-gas comes out of (1-based, the game's own convention).
    ///
    /// The fixture above leaves it unstated, which resolves to ordinal 0 --
    /// the refinery's westmost output connection. Naming box 3 moves the
    /// chosen port to the **eastmost** one, and that is the whole difference
    /// between a rig whose buffer happens to miss the port and one whose
    /// buffer lands on it. See
    /// [`a_buffer_may_not_swallow_the_machines_own_output_port`].
    fn oil_world_with_output_box(index: u32) -> FactorioSurface {
        let world = oil_world(true);
        let recipe: FactorioRecipe = serde_json::from_str(&format!(
            r#"{{
              "name": "basic-oil-processing", "valid": true, "enabled": true,
              "category": "oil-processing",
              "ingredients": [
                {{ "name": "crude-oil", "ingredient_type": "fluid", "amount": 100 }}
              ],
              "products": [
                {{ "name": "petroleum-gas", "product_type": "fluid", "amount": 45,
                  "probability": 1.0, "fluidbox_index": {index} }}
              ],
              "hidden": false, "energy": 5.0, "order": "a-a",
              "group": "intermediate-products", "subgroup": "fluid-recipes"
            }}"#
        ))
        .expect("the basic-oil-processing recipe parses");
        world
            .update_recipes(vec![recipe])
            .expect("update_recipes cannot fail for a well-formed recipe");
        world
    }

    fn state_of(world: FactorioSurface) -> PlanState {
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// A world with the crude arriving in a tank at the field, which is the
    /// topology the owner stated: tank -> refinery, the trunk given.
    fn state_with_tank() -> PlanState {
        state_with_tank_in(oil_world(true))
    }

    /// [`state_with_tank`] over any of the worlds above.
    fn state_with_tank_in(world: FactorioSurface) -> PlanState {
        let mut state = state_of(world);
        state.create_entity(FactorioEntity {
            name: "storage-tank".into(),
            entity_type: "storage-tank".into(),
            position: tank_site(),
            direction: 0,
            ..Default::default()
        });
        stock(&mut state);
        state
    }

    /// Everything the plan would otherwise have to make. The subject here is
    /// the pipe run, not the bill.
    fn stock(state: &mut PlanState) {
        for (item, count) in [
            ("oil-refinery", 4u32),
            ("storage-tank", 4),
            ("pipe", 400),
            ("iron-plate", 400),
            ("copper-plate", 400),
            ("steel-plate", 400),
        ] {
            state.gain(BotId(1), item, count);
        }
    }

    fn goal() -> Goal {
        Goal::Produced {
            item: "petroleum-gas".into(),
            count: 45,
            whose: Holder::Bot(BotId(1)),
            unlocks: None,
            via: Some("basic-oil-processing".into()),
        }
    }

    fn expand(state: PlanState, goal: &Goal) -> Result<Vec<Step>, PlannerError> {
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        Fabricate.expand(goal, &mut ctx)
    }

    /// Every `Place` of `name`, in emission order.
    fn placed(steps: &[Step], name: &str) -> Vec<Position> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) => match &action.kind {
                    ActionKind::Place { entity } if entity.name == name => {
                        Some(entity.position.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Ore everywhere but three holes.
    fn oil_world_ringed(index: u32) -> FactorioSurface {
        use factorio_bot_core::types::Direction;
        let world = oil_world_with_output_box(index);
        let hole = |x: f64, y: f64| -> bool {
            (26.0..=30.0).contains(&x) && (29.0..=33.0).contains(&y)
                || (30.0..=32.0).contains(&x) && (26.0..=28.0).contains(&y)
                || (33.0..=35.0).contains(&x) && (33.0..=35.0).contains(&y)
        };
        let mut blanket = Vec::new();
        let mut x = 14.0;
        while x <= 46.0 {
            let mut y = 10.0;
            while y <= 44.0 {
                if !hole(x, y) {
                    blanket.push(FactorioEntity::new_resource(
                        &Position::new(x + 0.5, y + 0.5),
                        Direction::North,
                        "crude-oil",
                    ));
                }
                y += 1.0;
            }
            x += 1.0;
        }
        world
            .update_chunk_entities(blanket)
            .expect("a chunk of ore");
        world
    }

    fn message(error: &PlannerError) -> String {
        error.to_string()
    }

    /// **A buffer may not stand on the machine's own output connection**, and
    /// this is the fixture that shows it, by going RED when the guard is
    /// removed.
    ///
    /// [`no_pipe_of_the_rig_stands_on_the_machine_or_its_buffer`] pins the
    /// same invariant and — measured by mutation, and it says so in its own
    /// doc — could not falsify the guard: its rig is sited where the
    /// connection happens to miss. This one reproduces the tight ring the
    /// live plan met on seed 31337, where a `storage-tank` at
    /// `[145.5, -362.5]` swallowed the refinery's output connection at
    /// `[145.5, -361.5]`, `route_between` pushed that tile into the run
    /// anyway — unconditionally, outside its own search, as its doc says —
    /// and the plan died at SCHEDULE time on the refinery's own `AreaFree`.
    ///
    /// # What had to be true before the guard could be reached at all
    ///
    /// Three separate rules reject a swallowing candidate, and the plain
    /// fixture never gets past the first two. Measured while building this,
    /// each by direct probe rather than by reading the code:
    ///
    /// * **`ports_clear`** — the *tank's own* corner connections sit one tile
    ///   diagonally outside its footprint, so every candidate over the
    ///   refinery's two western output connections puts one on the refinery
    ///   and is refused before the third rule is consulted.
    /// * **`routes_to`** — a machine connection in the *middle* of the tank's
    ///   edge is walled in by the tank itself: probed at `(30.5, 19.5)` in the
    ///   plain fixture, `route_between` answers `no route to the
    ///   storage-tank's connection at [29.5, 17.5], blocked by 4 tile(s)`. So
    ///   the swallow is only reachable at a **corner** of the buffer, where a
    ///   neighbouring tile is still open.
    /// * only then **`taken.extend(other_port_tiles)`**, the guard under test.
    ///
    /// So the fixture states three things the plain one leaves unstated, and
    /// every one is an *input* rather than something read back off the
    /// geometry — the `a-fixture-cannot-falsify-what-it-derives` trap:
    ///
    /// 1. the recipe's own `fluidbox_index`, 1-based and 3, which is the
    ///    game's way of saying "this product comes out of the third box" and
    ///    moves the chosen connection to the refinery's **eastmost**;
    /// 2. ore over the whole neighbourhood with three holes in it, because
    ///    `free_area_near_where` prefers ore-free ground and will otherwise
    ///    walk away from a contested ring entirely. The holes are the
    ///    refinery's own footprint, a 3x3 pocket at its north-east corner,
    ///    and one further out;
    /// 3. that pocket, whose only legal tank site — `(31.5, 27.5)` — covers
    ///    the refinery's eastmost output connection at `(30.5, 28.5)` while
    ///    keeping all four of its own corner connections clear.
    ///
    /// # The mutation, and what each side of it produces
    ///
    /// ```text
    /// guard present:  tank at (34.5, 34.5), the far hole; no pipe on it
    /// guard removed:  tank at (31.5, 27.5), and `place pipe at [30.5, 28.5]`
    ///                 -- inside the tank's own footprint, with no ordering
    ///                 edge between them
    /// ```
    #[test]
    fn a_buffer_may_not_swallow_the_machines_own_output_port() {
        let state = state_with_tank_in(oil_world_ringed(3));
        let steps = expand(state.fork(), &goal()).expect("the goal expands");
        let refineries = placed(&steps, "oil-refinery");
        let tanks = placed(&steps, "storage-tank");
        let pipes = placed(&steps, "pipe");
        assert_eq!(refineries.len(), 1, "one refinery is sited, in {steps:?}");
        assert_eq!(tanks.len(), 1, "one buffer is sited, in {steps:?}");
        assert!(!pipes.is_empty(), "the rig lays pipes, in {steps:?}");

        // The connection the recipe named, asked of the same function the rig
        // asks -- not recomputed from an offset written here, which would be a
        // second encoding of the machine's shape.
        let ports = crate::method::pipe::fluid_ports(
            &state,
            "oil-refinery",
            &refineries[0],
            Some("output"),
        )
        .expect("the fixture's refinery declares output fluidboxes");
        let chosen: Vec<Position> = ports
            .iter()
            .filter(|port| port.box_ordinal == 2)
            .flat_map(FluidPort::tiles)
            .collect();
        assert!(
            !chosen.is_empty(),
            "the fixture's refinery has a third output box for the recipe to name"
        );

        let tank_area = state
            .collision_area("storage-tank", &tanks[0])
            .expect("the fixture has a collision box for the tank");
        for tile in &chosen {
            let port_area = state
                .collision_area("pipe", tile)
                .expect("the fixture has a collision box for a pipe");
            assert!(
                !overlaps(&tank_area, &port_area),
                "the buffer at {} stands on the refinery's own output connection at {tile}",
                tanks[0]
            );
        }

        // And the invariant the sibling states, over ground that is genuinely
        // contested this time: nothing this rig places stands on anything else
        // it places.
        let refinery_area = state
            .collision_area("oil-refinery", &refineries[0])
            .expect("the fixture has a collision box for the refinery");
        for pipe in &pipes {
            let area = state
                .collision_area("pipe", pipe)
                .expect("the fixture has a collision box for a pipe");
            assert!(
                !overlaps(&area, &tank_area),
                "a pipe at {pipe} stands on the buffer at {}, in {steps:?}",
                tanks[0]
            );
            assert!(
                !overlaps(&area, &refinery_area),
                "a pipe at {pipe} stands on the refinery at {}, in {steps:?}",
                refineries[0]
            );
        }
    }

    /// **The claim of this whole branch**: a fluid ingredient is met by a pipe
    /// run from something standing that can supply it, and the plan builds
    /// that run.
    ///
    /// Paired assertions on purpose. "Pipes were placed" is an accidental pass
    /// for any expansion that ran at all, so the machine's own placement and
    /// its recipe are asserted beside it -- if expansion never reached the
    /// emission half, both are absent and the test fails for the right
    /// reason.
    #[test]
    fn a_fluid_ingredient_is_met_by_a_pipe_run_from_a_standing_tank() {
        let steps = expand(state_with_tank(), &goal()).expect("the goal expands");
        let refineries = placed(&steps, "oil-refinery");
        let pipes = placed(&steps, "pipe");
        assert_eq!(refineries.len(), 1, "one refinery, in {steps:?}");
        assert!(
            steps.iter().any(|step| matches!(
                step,
                Step::Act(action)
                    if matches!(&action.kind, ActionKind::SetRecipe { recipe, .. }
                        if recipe == "basic-oil-processing")
            )),
            "the refinery is told what to run"
        );
        assert!(!pipes.is_empty(), "a pipe run was laid");
        // Every pipe stands on a tile of its own, and none on the refinery.
        let mut sorted = pipes.clone();
        sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        sorted.dedup();
        assert_eq!(sorted.len(), pipes.len(), "no tile is piped twice");
    }

    /// **The buffer does not stand on the machine it is catching from.**
    ///
    /// Both are resolved before either is emitted, so the overlay knows about
    /// neither and `free_area_near_where` read the ground under the refinery
    /// as clear: `place oil-refinery at [137.5, -352.5]` and `place
    /// storage-tank at [137.5, -352.5]`, the same tile, on
    /// `gathered:crude-oil` + `producing:petroleum-gas:45` (seed 31337).
    /// Whichever went second failed its own `AreaFree` at expansion, naming a
    /// tile and not the two things that wanted it.
    #[test]
    fn the_buffer_is_not_sited_on_the_machine_it_catches_from() {
        let state = state_with_tank();
        let steps = expand(state.fork(), &goal()).expect("the goal expands");
        let refineries = placed(&steps, "oil-refinery");
        let tanks = placed(&steps, "storage-tank");
        // Non-accidental: both really were sited, so there are two footprints
        // to compare. An expansion that emitted neither would otherwise pass.
        assert_eq!(refineries.len(), 1, "one refinery is sited, in {steps:?}");
        assert_eq!(tanks.len(), 1, "one buffer is sited, in {steps:?}");
        let refinery = state
            .collision_area("oil-refinery", &refineries[0])
            .expect("the fixture has a collision box for the refinery");
        let tank = state
            .collision_area("storage-tank", &tanks[0])
            .expect("the fixture has a collision box for the tank");
        assert!(
            !overlaps(&refinery, &tank),
            "the buffer at {} overlaps the refinery at {}",
            tanks[0],
            refineries[0]
        );
    }

    /// **Nothing this rig places stands on anything else this rig places.**
    ///
    /// The footprint-versus-footprint check above is not enough, because a
    /// fluid connection is a tile *outside* the entity that owns it and
    /// [`crate::method::pipe::route_between`] pushes both ends' port tiles
    /// into the run unconditionally, outside its own search. A buffer cleared
    /// of the machine by footprint alone can still have a corner connection
    /// under it -- a `storage-tank`'s connections sit one tile diagonally out
    /// -- and the pipe that lands there is emitted with **no ordering edge**
    /// to the machine's own `Place`.
    ///
    /// So the collision is invisible at expansion and surfaces at SCHEDULE
    /// time as the machine's own `Condition::AreaFree`: on seed 31337,
    /// `gathered:crude-oil` + `produced:petroleum-gas:45:basic-oil-processing`
    /// died with `oil-refinery fits at [143.5, -358.5] ... does not hold
    /// there`, blaming a chain owner for a fact about the ground.
    ///
    /// # THIS TEST DOES NOT FALSIFY THE GUARD, AND SAYS SO
    ///
    /// Checked by mutation on 2026-09-08: comment out the
    /// `taken.extend(other_port_tiles ...)` above and this test still passes.
    /// The fixture's rig is sited where the corner connection happens to miss,
    /// so it pins the *invariant* and would catch a gross regression, and it
    /// is **not** evidence that the guard is exercised. The thing that
    /// falsifies is the offline plan against a dump with a charted oil field:
    ///
    /// ```text
    /// factorio-bot plan --world workspace/scripts/map-31337-water-and-oil.json \
    ///     --goal gathered:crude-oil \
    ///     --goal produced:petroleum-gas:45:basic-oil-processing --bots 1
    /// ```
    ///
    /// Without either guard that refuses; with both it plans. A fixture that
    /// reproduces the tight ring is the missing piece of work.
    #[test]
    fn no_pipe_of_the_rig_stands_on_the_machine_or_its_buffer() {
        let state = state_with_tank();
        let steps = expand(state.fork(), &goal()).expect("the goal expands");
        let refineries = placed(&steps, "oil-refinery");
        let tanks = placed(&steps, "storage-tank");
        let pipes = placed(&steps, "pipe");
        // Non-accidental: an expansion that placed no pipes, or no machine,
        // would pass a bare "nothing overlaps".
        assert_eq!(refineries.len(), 1, "one refinery is sited, in {steps:?}");
        assert_eq!(tanks.len(), 1, "one buffer is sited, in {steps:?}");
        assert!(!pipes.is_empty(), "the rig lays pipes, in {steps:?}");
        for (name, at) in [
            ("oil-refinery", &refineries[0]),
            ("storage-tank", &tanks[0]),
        ] {
            let footprint = state
                .collision_area(name, at)
                .expect("the fixture has a collision box for it");
            for pipe in &pipes {
                let area = state
                    .collision_area("pipe", pipe)
                    .expect("the fixture has a collision box for a pipe");
                assert!(
                    !overlaps(&area, &footprint),
                    "a pipe at {pipe} stands on the {name} at {at}"
                );
            }
        }
    }

    /// **The rig is one bot's errand, and nothing in it converges.**
    ///
    /// The machine, its buffer and every pipe are stated as separate
    /// `Goal::Have` subgoals and then placed out of a hand, so `converges` is
    /// honestly `false` while [`Method::hands_over`] is true. Without the
    /// second, the rig went unchained with its bill sized against
    /// `ctx.chain_actor`, and nine unowned `place pipe` actions spent the ten
    /// pipes an *owned* chain had crafted for `craft 1 oil-refinery`.
    #[test]
    fn the_fluid_rig_hands_over_without_converging() {
        let state = state_with_tank();
        assert!(
            !Fabricate.converges(&goal(), &state),
            "each item is its own subgoal: nothing has to meet anything"
        );
        assert!(
            Fabricate.hands_over(&goal(), &state),
            "and yet every placement spends what this method's own subgoal bought"
        );
        // Not accidental: there really is something placed out of a hand.
        let steps = expand(state.fork(), &goal()).expect("the goal expands");
        let from_a_hand = steps
            .iter()
            .filter(|step| match step {
                Step::Act(action) => action.pre.iter().any(|c| {
                    matches!(
                        c,
                        Condition::HasItem {
                            who: Actor::Role,
                            ..
                        }
                    )
                }),
                _ => false,
            })
            .count();
        assert!(
            from_a_hand >= 2,
            "control: the rig must place at least two things out of a hand, got {from_a_hand}"
        );
    }

    /// The other half of the ruling: **a fluid product needs a sink or the
    /// machine stalls**, and one output is given a buffer rather than
    /// refused.
    #[test]
    fn a_fluid_product_lands_in_a_buffer_this_plan_builds() {
        let steps = expand(state_with_tank(), &goal()).expect("the goal expands");
        // Two tanks stand afterwards: the one that was already there and the
        // one built to catch the petroleum. Only the second is *placed*.
        let tanks = placed(&steps, "storage-tank");
        assert_eq!(tanks.len(), 1, "exactly one buffer is built, in {steps:?}");
        assert_ne!(tanks[0], tank_site(), "the source is adopted, not rebuilt");
    }

    /// A fluid cannot be taken into a hand, so no `Remove` is emitted for one
    /// -- **and the same method does emit one for an item**, which is the
    /// control that makes the absence mean something.
    #[test]
    fn a_fluid_goal_takes_nothing_and_an_item_goal_takes() {
        let steps = expand(state_with_tank(), &goal()).expect("the goal expands");
        assert!(
            !steps.iter().any(|step| matches!(
                step,
                Step::Act(action) if matches!(action.kind, ActionKind::Remove { .. })
            )),
            "nothing is taken out of the refinery by hand"
        );

        // The control: an item-producing recipe in a nameable category, same
        // method, same world.
        let world = oil_world(true);
        world
            .globals
            .entity_prototypes
            .get_mut("chemical-plant")
            .expect("the fixture has a chemical-plant")
            .crafting_categories = Some(vec!["chemistry".into()]);
        let recipe: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "solid-fuel-from-nothing", "valid": true, "enabled": true,
              "category": "chemistry",
              "ingredients": [
                { "name": "iron-plate", "ingredient_type": "item", "amount": 1 }
              ],
              "products": [
                { "name": "solid-fuel", "product_type": "item", "amount": 1, "probability": 1.0 }
              ],
              "hidden": false, "energy": 1.0, "order": "a-a", "group": "g", "subgroup": "s"
            }"#,
        )
        .expect("the control recipe parses");
        world.update_recipes(vec![recipe]).expect("recipes update");
        let mut state = state_of(world);
        stock(&mut state);
        state.gain(BotId(1), "chemical-plant", 2);
        let control = Goal::Produced {
            item: "solid-fuel".into(),
            count: 1,
            whose: Holder::Bot(BotId(1)),
            unlocks: None,
            via: Some("solid-fuel-from-nothing".into()),
        };
        let steps = expand(state, &control).expect("the control expands");
        assert!(
            steps.iter().any(|step| matches!(
                step,
                Step::Act(action) if matches!(action.kind, ActionKind::Remove { .. })
            )),
            "an item IS taken, so the absence above is about fluids"
        );
    }

    /// The machine follows its source. A refinery sited beside the bot and a
    /// tank thirty tiles away is not a pipe run -- it is the long-distance
    /// trunk, which is a separate rung.
    #[test]
    fn the_machine_is_sited_beside_its_source_not_beside_the_bot() {
        let mut state = state_with_tank();
        state.set_position(BotId(1), Position::new(-200.5, -200.5));
        let steps = expand(state, &goal()).expect("the goal expands");
        let refinery = placed(&steps, "oil-refinery")
            .first()
            .expect("a refinery is placed")
            .clone();
        let distance =
            factorio_bot_core::factorio::util::calculate_distance(&refinery, &tank_site());
        assert!(
            distance < 12.0,
            "the refinery stands beside its tank, not beside the bot: {distance} tiles"
        );
    }

    /// **A buffer outranks an extractor, and that is the owner's topology**:
    /// *"the fluid tank the oil arrives in from far away should be connected
    /// to the refineries"*. A pumpjack can supply crude and is often nearer,
    /// so choosing by distance alone ties one refinery to one well.
    ///
    /// The pumpjack stands on a well, so it is attributable by rule 2 and the
    /// only thing separating them is the ranking.
    #[test]
    fn a_buffer_outranks_an_extractor_even_when_the_extractor_is_nearer() {
        let mut state = state_with_tank();
        // A pumpjack on the first well, closer to the bot than the tank is.
        state.create_entity(FactorioEntity {
            name: "pumpjack".into(),
            entity_type: "mining-drill".into(),
            position: Position::new(20.5, 20.5),
            direction: 0,
            ..Default::default()
        });
        let steps = expand(state, &goal()).expect("the goal expands");
        let refinery = placed(&steps, "oil-refinery")
            .first()
            .expect("a refinery is placed")
            .clone();
        let to_tank =
            factorio_bot_core::factorio::util::calculate_distance(&refinery, &tank_site());
        let to_well = factorio_bot_core::factorio::util::calculate_distance(
            &refinery,
            &Position::new(20.5, 20.5),
        );
        assert!(
            to_tank < to_well,
            "the refinery follows the tank ({to_tank}) not the pumpjack ({to_well})"
        );
    }

    // -----------------------------------------------------------------------
    // The refusals
    // -----------------------------------------------------------------------

    /// With nothing standing that can supply crude, the plan says so by name
    /// rather than building a tank and calling it a supply.
    #[test]
    fn no_standing_source_refuses_by_name() {
        let mut state = state_of(oil_world(true));
        stock(&mut state);
        let error = expand(state, &goal()).expect_err("nothing can supply crude");
        let said = message(&error);
        assert!(said.contains("crude-oil"), "{said}");
        assert!(said.contains("can be shown to supply"), "{said}");
    }

    /// **The measured regression**: the first version of this code took the
    /// nearest supplying fluidbox and chose a `boiler`, because
    /// `method::power` sites a plant at the wellhead and a boiler's steam box
    /// supplies. A refinery piped to a boiler builds perfectly and makes
    /// nothing.
    #[test]
    fn a_boiler_is_not_a_crude_source_and_the_refusal_names_it() {
        let mut state = state_of(oil_world(true));
        stock(&mut state);
        state.create_entity(FactorioEntity {
            name: "boiler".into(),
            entity_type: "boiler".into(),
            position: Position::new(22.0, 24.0),
            direction: 0,
            ..Default::default()
        });
        let error = expand(state, &goal()).expect_err("a boiler supplies steam, not crude");
        let said = message(&error);
        assert!(said.contains("boiler"), "the rejection is named: {said}");
        assert!(said.contains("rejected"), "{said}");
    }

    /// The two-fluid recipe this whole change exists for: `sulfur`, water and
    /// petroleum-gas into a machine declaring exactly two input boxes.
    ///
    /// Standing in the `oil-refinery` rather than the `chemical-plant` only
    /// because the fixture capture has the refinery's prototype; the fact
    /// under test is the box count, which is two either way.
    fn sulfur_recipe() -> FactorioRecipe {
        serde_json::from_str(
            r#"{
              "name": "sulfur", "valid": true, "enabled": true, "category": "oil-processing",
              "ingredients": [
                { "name": "water", "ingredient_type": "fluid", "amount": 30 },
                { "name": "petroleum-gas", "ingredient_type": "fluid", "amount": 30 }
              ],
              "products": [
                { "name": "sulfur", "product_type": "item", "amount": 2, "probability": 1.0 }
              ],
              "hidden": false, "energy": 1.0, "order": "a-a", "group": "g", "subgroup": "s"
            }"#,
        )
        .expect("the two-fluid recipe parses")
    }

    fn sulfur_goal() -> Goal {
        Goal::Produced {
            item: "sulfur".into(),
            count: 2,
            whose: Holder::Bot(BotId(1)),
            unlocks: None,
            via: Some("sulfur".into()),
        }
    }

    /// A recipe whose only product is `fluid`, so a machine standing with it
    /// set is attributable as a source of that fluid by
    /// `pipe::sources_of`'s recipe rule.
    fn makes(name: &str, fluid: &str) -> FactorioRecipe {
        serde_json::from_str(&format!(
            r#"{{
              "name": "{name}", "valid": true, "enabled": true, "category": "oil-processing",
              "ingredients": [],
              "products": [
                {{ "name": "{fluid}", "product_type": "fluid", "amount": 50,
                   "probability": 1.0 }}
              ],
              "hidden": false, "energy": 1.0, "order": "z-{name}", "group": "g",
              "subgroup": "s"
            }}"#
        ))
        .expect("the source recipe parses")
    }

    /// A world that can run `sulfur`: the recipe, plus one standing machine
    /// per fluid whose own recipe produces it.
    ///
    /// **A standing source and not a tank.** A tank is attributable only by
    /// the ground it sits on, and neither water nor petroleum-gas is a
    /// charted resource -- which is the honest shape of the problem and the
    /// reason the sulfur goal still refuses on a real map until something
    /// upstream stands a supply up.
    fn sulfur_state() -> PlanState {
        let world = oil_world(true);
        world
            .update_recipes(vec![
                sulfur_recipe(),
                makes("water-source", "water"),
                makes("petroleum-source", "petroleum-gas"),
            ])
            .expect("recipes update");
        let mut state = state_of(world);
        for (recipe, at) in [
            ("water-source", Position::new(24.5, 26.5)),
            ("petroleum-source", Position::new(24.5, 36.5)),
        ] {
            state.create_entity(FactorioEntity {
                name: "oil-refinery".into(),
                entity_type: "assembling-machine".into(),
                position: at,
                direction: 0,
                recipe: Some(recipe.into()),
                ..Default::default()
            });
        }
        stock(&mut state);
        state
    }

    /// **The claim of this change.** Two fluids in used to refuse outright,
    /// and through that refusal the whole rocket ladder did too. A standing
    /// `chemical-plant` set to `sulfur` reports `water` on its first input box
    /// and `petroleum-gas` on its second -- measured live on 2026-09-08 -- so
    /// the assignment is a read fact and the plan lays one run per fluid.
    ///
    /// Asserted on the *ports* and not merely on "two runs exist": two runs
    /// into the same box would satisfy a count and is exactly the failure this
    /// guards against.
    #[test]
    fn two_fluid_ingredients_get_one_run_each_to_their_own_input_box() {
        let state = sulfur_state();
        let steps = expand(state, &sulfur_goal()).expect("both fluids have a standing source");
        let refineries = placed(&steps, "oil-refinery");
        assert_eq!(refineries.len(), 1, "one machine is placed: {refineries:?}");
        let labels: Vec<String> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) => Some(action.label.clone()),
                _ => None,
            })
            .collect();
        for fluid in ["water", "petroleum-gas"] {
            assert!(
                labels.iter().any(|label| label.contains(&format!(
                    "carry {fluid} between the oil-refinery"
                ))),
                "a run carries {fluid}: {labels:?}"
            );
        }
    }

    /// The two runs join **different** ports of the machine, which is the
    /// whole point of resolving an ordinal per fluid.
    ///
    /// # This assertion had to be sharpened, and the mutant is why
    ///
    /// The first version asked which ports the plan's pipes overlap, over all
    /// pipes at once. **A mutation that sent both runs to box 0 passed it.**
    /// The refinery's two input ports are two tiles apart on one edge, so the
    /// second run's *route* lies across the first port's tile on its way past
    /// -- the set of ports touched is the same either way, and the question
    /// "which port did this run aim at" is simply not answerable from a flat
    /// list of tiles.
    ///
    /// So the pipes are partitioned by the fluid in their own `Place` label,
    /// which is per run, and each run is required to reach its own box's
    /// junction. Under the mutant petroleum-gas's run ends at box 0's
    /// junction and the test fails, which is what a falsification is for.
    #[test]
    fn the_two_runs_do_not_share_a_port() {
        let state = sulfur_state();
        let steps = expand(state, &sulfur_goal()).expect("both fluids have a standing source");
        let site = placed(&steps, "oil-refinery")
            .first()
            .expect("a refinery is placed")
            .clone();
        let plain = PlanState::from_world(Arc::new(oil_world(true)), &[BotId(1)]);
        let ports =
            pipe::fluid_ports(&plain, "oil-refinery", &site, Some("input")).expect("input ports");
        // Every pipe this plan places, grouped by the fluid its own label
        // names -- which is the only per-run identity the emitted steps carry.
        let laid_for = |fluid: &str| -> Vec<Position> {
            let needle = format!("carry {fluid} between the oil-refinery");
            steps
                .iter()
                .filter_map(|step| match step {
                    Step::Act(action) if action.label.contains(&needle) => match &action.kind {
                        ActionKind::Place { entity } if entity.name == "pipe" => {
                            Some(entity.position.clone())
                        }
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        };
        let junction_of = |ordinal: usize| {
            ports
                .iter()
                .find(|port| port.box_ordinal == ordinal)
                .unwrap_or_else(|| panic!("the refinery has an input box {ordinal}"))
                .junction
                .clone()
        };
        for (fluid, ordinal) in [("water", 0usize), ("petroleum-gas", 1usize)] {
            let tiles = laid_for(fluid);
            assert!(!tiles.is_empty(), "a run carries {fluid}");
            assert!(
                tiles.contains(&junction_of(ordinal)),
                "the {fluid} run reaches input box {ordinal}'s junction \
                 {:?}; it laid {tiles:?}",
                junction_of(ordinal)
            );
        }
        assert_ne!(
            junction_of(0),
            junction_of(1),
            "the two boxes are distinct ground, or the assertion above is vacuous"
        );
    }

    /// **The latent defect this change also fixes, end to end.**
    ///
    /// The real `basic-oil-processing` declares `fluidbox_index = 2` for
    /// crude oil and `= 3` for petroleum-gas -- read off a live 2.1.17 dump
    /// on 2026-09-08, where exactly 3 of 662 recipes carry the field at all.
    /// The fixture's copy of the recipe omits it, as every dump taken before
    /// that date does, so the two halves are run side by side here: the same
    /// world, the same goal, the recipe with and without the field, and the
    /// ports the emitted pipes actually touch.
    ///
    /// Without it the run joins input box 0 and output box 0. With it, input
    /// box 1 and output box 2 -- and on a live refinery box 0 of the input
    /// side is the one the game leaves empty.
    ///
    /// **The output side is asserted by containment, and that is not
    /// sloppiness.** A refinery's three output ports sit along one edge, so
    /// the run leaving box 2 lays a pipe over box 1's connection tile on its
    /// way out -- and in the game that tile joins box 1 as well. Reading
    /// "which port did we aim at" off tile overlap therefore cannot be an
    /// equality; what discriminates is that box 2 is touched *only* when the
    /// recipe names it. The incidental join is harmless for
    /// `basic-oil-processing`, whose other two output boxes do not exist
    /// while that recipe is set, and is not something this change addresses.
    #[test]
    fn the_declared_fluidbox_moves_the_run_to_the_box_the_game_uses() {
        let ports_touched = |declared: bool| -> (Vec<usize>, Vec<usize>) {
            let world = oil_world(true);
            if declared {
                let recipe: FactorioRecipe = serde_json::from_str(
                    r#"{
                      "name": "basic-oil-processing", "valid": true, "enabled": true,
                      "category": "oil-processing",
                      "ingredients": [
                        { "name": "crude-oil", "ingredient_type": "fluid", "amount": 100,
                          "fluidbox_index": 2 }
                      ],
                      "products": [
                        { "name": "petroleum-gas", "product_type": "fluid", "amount": 45,
                          "probability": 1.0, "fluidbox_index": 3 }
                      ],
                      "hidden": false, "energy": 5.0, "order": "a-a",
                      "group": "intermediate-products", "subgroup": "fluid-recipes"
                    }"#,
                )
                .expect("the recipe parses");
                world.update_recipes(vec![recipe]).expect("recipes update");
            }
            let mut state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
            state.create_entity(FactorioEntity {
                name: "storage-tank".into(),
                entity_type: "storage-tank".into(),
                position: tank_site(),
                direction: 0,
                ..Default::default()
            });
            stock(&mut state);
            let steps = expand(state, &goal()).expect("the goal expands");
            let site = placed(&steps, "oil-refinery")
                .first()
                .expect("a refinery is placed")
                .clone();
            let laid = placed(&steps, "pipe");
            let plain = PlanState::from_world(Arc::new(oil_world(true)), &[BotId(1)]);
            let touched = |direction: &str| {
                let mut seen: Vec<usize> =
                    pipe::fluid_ports(&plain, "oil-refinery", &site, Some(direction))
                        .expect("ports")
                        .iter()
                        .filter(|port| port.tiles().iter().any(|tile| laid.contains(tile)))
                        .map(|port| port.box_ordinal)
                        .collect();
                seen.sort_unstable();
                seen.dedup();
                seen
            };
            (touched("input"), touched("output"))
        };

        let (silent_in, silent_out) = ports_touched(false);
        assert_eq!(
            (silent_in, silent_out.contains(&2)),
            (vec![0], false),
            "a recipe that names no box falls to the positional rule -- which is what every \
             dump taken before 2026-09-08 does"
        );
        let (declared_in, declared_out) = ports_touched(true);
        assert_eq!(
            (declared_in, declared_out.contains(&2)),
            (vec![1], true),
            "the recipe's own fluidbox_index moves both runs onto the boxes the game uses"
        );
    }

    /// The refusal that is left: a machine declaring more boxes of a
    /// direction than the recipe has fluids, where the game merges them in a
    /// way that is not positional. Three fluids into the refinery's two input
    /// boxes is the same shape from the other side.
    #[test]
    fn a_fluid_with_no_decidable_box_refuses_by_name() {
        let world = oil_world(true);
        let recipe: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "sulfur", "valid": true, "enabled": true, "category": "oil-processing",
              "ingredients": [
                { "name": "water", "ingredient_type": "fluid", "amount": 30 },
                { "name": "steam", "ingredient_type": "fluid", "amount": 30 },
                { "name": "petroleum-gas", "ingredient_type": "fluid", "amount": 30 }
              ],
              "products": [
                { "name": "sulfur", "product_type": "item", "amount": 2, "probability": 1.0 }
              ],
              "hidden": false, "energy": 1.0, "order": "a-a", "group": "g", "subgroup": "s"
            }"#,
        )
        .expect("the three-fluid recipe parses");
        world.update_recipes(vec![recipe]).expect("recipes update");
        let mut state = state_of(world);
        stock(&mut state);
        let error = expand(state, &sulfur_goal()).expect_err("three fluids into two boxes");
        let said = message(&error);
        assert!(said.contains("steam"), "the fluid is named: {said}");
        assert!(
            said.contains("cannot be tied to a fluidbox"),
            "the reason is the box and not the source: {said}"
        );
    }

    /// Three fluids out is `advanced-oil-processing`, whose sink rule the
    /// owner ruled on separately. It is refused by name, not half-built.
    #[test]
    fn three_fluid_products_refuse_as_the_multi_output_rule() {
        let world = oil_world(true);
        let recipe: FactorioRecipe = serde_json::from_str(
            r#"{
              "name": "advanced-oil-processing", "valid": true, "enabled": true,
              "category": "oil-processing",
              "ingredients": [
                { "name": "crude-oil", "ingredient_type": "fluid", "amount": 100 }
              ],
              "products": [
                { "name": "heavy-oil", "product_type": "fluid", "amount": 25, "probability": 1.0 },
                { "name": "light-oil", "product_type": "fluid", "amount": 45, "probability": 1.0 },
                { "name": "petroleum-gas", "product_type": "fluid", "amount": 55,
                  "probability": 1.0 }
              ],
              "hidden": false, "energy": 5.0, "order": "a-b", "group": "g", "subgroup": "s"
            }"#,
        )
        .expect("the three-output recipe parses");
        world.update_recipes(vec![recipe]).expect("recipes update");
        let mut state = state_of(world);
        state.create_entity(FactorioEntity {
            name: "storage-tank".into(),
            entity_type: "storage-tank".into(),
            position: tank_site(),
            direction: 0,
            ..Default::default()
        });
        stock(&mut state);
        let error = expand(
            state,
            &Goal::Produced {
                item: "light-oil".into(),
                count: 45,
                whose: Holder::Bot(BotId(1)),
                unlocks: None,
                via: Some("advanced-oil-processing".into()),
            },
        )
        .expect_err("three fluids out need three sinks");
        let said = message(&error);
        assert!(said.contains("3 fluids"), "{said}");
        assert!(said.contains("stalls"), "{said}");
    }

    /// A world that states a capacity too small for what was asked refuses.
    /// **Derived from the prototype's own `volume`** -- and the silent case is
    /// the control below.
    #[test]
    fn a_buffer_the_world_says_is_too_small_refuses() {
        let world = oil_world(true);
        {
            let mut tank = world
                .globals
                .entity_prototypes
                .get_mut("storage-tank")
                .expect("the fixture has a storage-tank");
            if let Some(boxes) = tank.fluidbox_prototypes.as_mut() {
                for b in boxes.iter_mut() {
                    b.volume = Some(10.0);
                }
            }
        }
        let mut state = state_of(world);
        state.create_entity(FactorioEntity {
            name: "storage-tank".into(),
            entity_type: "storage-tank".into(),
            position: tank_site(),
            direction: 0,
            ..Default::default()
        });
        stock(&mut state);
        let error = expand(state, &goal()).expect_err("10 units cannot hold 45");
        assert!(
            message(&error).contains("would fill it and stall"),
            "{error}"
        );
    }

    /// The control for the one above: **`volume: None` is "the sender did not
    /// say", never zero.** Every dump this project holds is silent about
    /// volume, so refusing on silence would break the whole archive.
    #[test]
    fn a_world_silent_about_volume_is_not_refused() {
        let world = oil_world(true);
        assert!(
            world
                .globals
                .entity_prototypes
                .get("storage-tank")
                .expect("the fixture has a storage-tank")
                .fluidbox_prototypes
                .as_ref()
                .expect("it has fluidboxes")
                .iter()
                .all(|b| b.volume.is_none()),
            "the fixture is silent about volume, which is what makes this a control"
        );
        expand(state_with_tank(), &goal()).expect("silence permits");
    }

    /// The machine whose category nothing crafts is not this method's, and it
    /// must not be claimed -- the registration promise that no existing plan
    /// can move.
    #[test]
    fn a_category_no_machine_runs_is_not_claimed() {
        let state = {
            let mut state = state_of(oil_world(false));
            stock(&mut state);
            state
        };
        assert!(
            !Fabricate.applicable(&goal(), &state),
            "with no refinery category on the prototype, nothing here can name the machine"
        );
    }
}
