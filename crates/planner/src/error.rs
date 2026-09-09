use crate::ids::{ActionId, BotId, ChainId, ItemId};
use crate::state::ChartingSummary;
use factorio_bot_core::types::{HandMiningObstacle, Position};
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum PlannerError {
    #[error("{bot} has {available} {item}, needs {required}")]
    #[diagnostic(code(planner::insufficient_items))]
    InsufficientItems {
        bot: BotId,
        item: ItemId,
        required: u32,
        available: u32,
    },

    /// A plan tried to take more out of a buffer than the plan itself believes
    /// is in there.
    ///
    /// **A fault, not a verdict.** It says nothing about the world: it says
    /// two parts of one expansion counted the same items towards themselves,
    /// which is `InsufficientItems`' defect in the other ledger. The game
    /// catching the same thing at dispatch is a *different*, real failure --
    /// somebody emptied the chest between planning and walking there -- and
    /// that one surfaces as a refused `Remove`, because the mod complains when
    /// it moves fewer items than asked and `judge_transfer_reply` reads any
    /// complaint as failure.
    #[error("the buffer at {position} holds {available} {item}, and the plan wants {required}")]
    #[diagnostic(code(planner::buffer_short))]
    BufferShort {
        item: ItemId,
        position: String,
        required: u32,
        available: u32,
    },

    #[error("no bot {0:?} in this plan state")]
    #[diagnostic(code(planner::unknown_bot))]
    UnknownBot(BotId),

    #[error("action network contains a cycle involving {0:?}")]
    #[diagnostic(code(planner::cyclic_network))]
    CyclicNetwork(ActionId),

    #[error("action {action:?} is unreachable: its predecessors can never all complete")]
    #[diagnostic(code(planner::deadlock))]
    Deadlock { action: ActionId },

    #[error("precondition {condition} of action {action:?} does not hold for {bot}")]
    #[diagnostic(code(planner::precondition_unsatisfied))]
    PreconditionUnsatisfied {
        action: ActionId,
        bot: BotId,
        condition: String,
    },

    /// Deliberately not a `PreconditionUnsatisfied`: that variant means "the
    /// world was not as planned", which the spec answers by re-planning from
    /// observed state. A pin that contradicts a chain binding is not about the
    /// world at all — re-planning would produce the same contradiction forever.
    ///
    /// `bound_to` is whichever committed the chain to a bot: its **owner**, if
    /// the goal named one (`Holder::Bot`, or — since 2026-09-02 — the bot a
    /// `Holder::Share` was sized against), and otherwise the bot the scheduler
    /// bound it to when the chain opened. Ownership is checked first because
    /// it is the harder constraint — stated before scheduling begins, with no
    /// fallback tier — and because a chain with an owner is only ever bound to
    /// that owner, so the owner is the original cause.
    #[error(
        "action {action:?} is pinned to {pinned_to}, but its chain {chain:?} already belongs to {bound_to}"
    )]
    #[diagnostic(code(planner::chain_conflict))]
    ChainConflict {
        chain: ChainId,
        action: ActionId,
        bound_to: BotId,
        pinned_to: BotId,
    },

    #[error("no bots supplied to the scheduler")]
    #[diagnostic(code(planner::no_bots))]
    NoBots,

    /// Deliberately does not say "because a caller named it": since
    /// 2026-09-02 a chain is also owned when a `Holder::Share` goal was sized
    /// against `bot`, which the planner binds itself rather than a caller
    /// asking for it. Saying "a caller named it" there would blame the wrong
    /// party for a bill the planner itself decided to size and to run on the
    /// same bot.
    #[error(
        "{bot} owns chain {chain:?} because its bill was sized against it, but {condition} does not hold there"
    )]
    #[diagnostic(code(planner::chain_owner_infeasible))]
    ChainOwnerInfeasible {
        chain: ChainId,
        /// The action that could not be run, carried like every sibling
        /// variant carries one: tier-1 recovery has to know which action to
        /// re-plan around, and the chain alone does not say.
        action: ActionId,
        bot: BotId,
        condition: String,
    },

    /// Deliberately not a `NoApplicableMethod`. That variant means "this world
    /// offers no route to the thing you asked for", which sends a caller
    /// looking at prerequisites and resources. A technology no force in this
    /// world defines is not a routing problem at all — the name itself is
    /// wrong, or the world was never told about the technology — and no amount
    /// of mining will fix it.
    #[error("the force this plan acts for defines no technology named {technology}")]
    #[diagnostic(
        code(planner::unknown_technology),
        help("check the spelling, or whether the world's forces have been loaded yet")
    )]
    UnknownTechnology { technology: String },

    #[error("no method can satisfy goal: {goal}")]
    #[diagnostic(code(planner::no_applicable_method))]
    NoApplicableMethod { goal: String },

    /// The item comes out of the ground, and a character cannot dig it.
    ///
    /// **Deliberately not a `NoApplicableMethod`**, and deliberately raised
    /// even when the wells are right there. Until 2026-09-04 the planner
    /// gated hand-mining on `has_resource_patches(item)` alone, and a
    /// resource's name and its product's name are both `crude-oil`, so a
    /// world holding twelve charted wells (the provenance of
    /// `run-1788538389-09170`, resumed from a savepoint) planned
    /// `mine 10 crude-oil` and would have dispatched it. The game answers
    /// `character.mine_entity(crude-oil) -> false` and leaves the well
    /// untouched -- verified live that day -- so the action can only ever
    /// fail, and a plan that cannot finish is worse than one that refuses.
    ///
    /// `mineable_properties.minable` is **not** the discriminator: it is true
    /// for crude oil, because a pumpjack mines it. The game's own rule is the
    /// resource's category against the character's `resource_categories`,
    /// and that is what `obstacle` reports, along with the two other
    /// prototype facts that refuse a hand -- a required fluid, or a product
    /// that is not an item. See
    /// [`factorio_bot_core::types::FactorioEntityPrototype::hand_mining_obstacle`].
    ///
    /// A verdict about the prototypes, not the map: exploring finds more of
    /// the same wells. A script acts on it by not asking a bot's hands for
    /// the item.
    #[error("{item} comes from {resource}, which a character cannot mine by hand: {obstacle}")]
    #[diagnostic(
        code(planner::not_hand_minable),
        help(
            "`minable` on the prototype is what a mining drill or a pumpjack uses; a character \
             mines only the resource categories its own prototype lists, needs no fluid piped \
             in, and can only hold items"
        )
    )]
    NotHandMinable {
        item: ItemId,
        /// The resource entity the item would be mined from.
        resource: String,
        obstacle: HandMiningObstacle,
    },

    /// The item comes out of the ground, and no ground the plan can see has
    /// any.
    ///
    /// **"Unexplored", not "absent".** This is piece 1 of the exploration
    /// design (`docs/superpowers/specs/2026-09-04-exploration-design.md`,
    /// Q4): the model holds only the chunks the game has charted, so a
    /// resource with no patch anywhere in it has *not been seen*, which a map
    /// that genuinely lacks it and a map nobody has walked both produce.
    /// Before this, that state fell through `Mine::applicable` to
    /// `NoApplicableMethod`, whose text -- "no method can satisfy goal: have
    /// 10 crude-oil" -- is true and says nothing a caller can act on.
    ///
    /// `charting` says what *was* seen, from where, and where seen ground
    /// ends, so a supervisor script can walk a bot to the frontier and
    /// re-plan; because the mod ingests charting as the bots walk, the
    /// re-plan sees the new ground with no world-model change. That script
    /// is piece 2 of the design and lives outside this crate; a first-class
    /// exploration goal is piece 3 and is deliberately not built here.
    #[error(
        "no {resource} is charted anywhere this plan can see, so {item} has nowhere to come \
         from; {charting}"
    )]
    #[diagnostic(
        code(planner::not_charted),
        help(
            "the model holds only the chunks the game has charted, and a world attached from a \
             snapshot holds only what the snapshot held; walk a bot towards the uncharted \
             ground and plan again"
        )
    )]
    NotCharted {
        item: ItemId,
        /// The resource entity the item would be mined from.
        resource: String,
        /// Boxed for `clippy::result_large_err`: the summary carries an
        /// origin, a probe list and a census, and every `Result<_,
        /// PlannerError>` in the crate would otherwise grow to carry it.
        charting: Box<ChartingSummary>,
    },

    /// Every candidate for a gathering target stands inside a charted enemy
    /// structure's standoff, so there is nowhere safe left to send a bot.
    ///
    /// **The refusal half of "prefer, then refuse".** While one safe target
    /// exists the planner simply passes the threatened ones over and nothing
    /// is said; this fires only when passing them all over leaves nothing, and
    /// it names the nearest threat, how far it was from the target that was
    /// given up, and whether that radius is the game's number or an assumed
    /// one -- because a refusal quoting an assumption as a fact is worse than
    /// no refusal at all.
    ///
    /// It exists because the alternative is what the tree did before: send the
    /// bot, and lose it. On seed 31337 two bots died this way at ticks 34,873
    /// and 53,619, the second while chopping a rock with three
    /// `small-worm-turret`s at ~20 tiles -- all three already in the dump the
    /// planner had read.
    ///
    /// Boxed for `clippy::result_large_err`, exactly as `NotCharted` is and
    /// for the same reason: the payload carries two positions, three strings
    /// and a standoff, and every `Result<_, PlannerError>` in the crate would
    /// otherwise grow to carry it.
    #[error("{0}")]
    #[diagnostic(code(planner::target_inside_threat))]
    TargetInsideThreat(Box<TargetInsideThreatDetail>),

    /// A goal several bots could have shared, in a world with nowhere for even
    /// one of them to work on it.
    ///
    /// Deliberately not a `NoApplicableMethod`, which is what this used to be
    /// and which says only "no method can satisfy goal: have 40 iron-ore
    /// (anyone)" — true, and silent about *why*. The reader then has to guess
    /// between "this world has no iron ore", "the recipe is not unlocked" and
    /// the real answer, which is that the plan itself has already committed
    /// every seat on the patch. Naming the shortage is the whole point of the
    /// variant: a refusal a reader cannot act on costs as much as no refusal.
    ///
    /// `holders` is how many bots could have shared the goal, so a reader can
    /// tell "nobody fits" from "not everybody fits" — the latter is not an
    /// error at all any more, it plans a narrower split.
    #[error(
        "nothing in this world can seat a bot to work on {goal}; {holders} were available to share it"
    )]
    #[diagnostic(
        code(planner::no_room_to_work),
        help(
            "a mining goal is seated by its patch, and a tile inside another miner's standing \
             room is not a seat — so a patch this plan has already committed to offers none"
        )
    )]
    NoRoomToWork { goal: String, holders: u32 },

    /// [`crate::Goal::Orbiting`] on a world with no rocket silo standing.
    ///
    /// A platform is delivered by a rocket, and this planner does none of the
    /// three things that would put a silo under one: siting a 9x9 building,
    /// feeding it rocket parts, or knowing whether it is currently mid-build.
    /// So the goal refuses here rather than emitting a placement it cannot
    /// cost -- [`PlannerError::NoExtractor`]'s reasoning, one rung up. Named
    /// so a caller learns *which* step is not modelled instead of receiving a
    /// makespan that is quietly missing a rocket silo.
    #[error(
        "a platform in orbit of {planet} is delivered by a rocket, and no {silo} stands in this \
         world for one to launch from"
    )]
    #[diagnostic(
        code(planner::platform_needs_silo),
        help(
            "build a rocket silo first; siting one, feeding it rocket parts and telling whether \
             it is mid-build are all outside what this planner models"
        )
    )]
    PlatformNeedsSilo {
        /// The planet the platform was to orbit, as the goal named it.
        planet: String,
        /// The prototype that was looked for, so the message does not have to
        /// be trusted to have spelled it the same way the search did.
        silo: String,
    },

    /// A Factorio 2.0 `research_trigger` technology whose trigger this planner
    /// has no goal for.
    ///
    /// Deliberately an error rather than a zero cost. These technologies carry
    /// no science-pack bill and no research time at all, so a planner that
    /// reads only the pack fields plans them as *free* — the plan comes out
    /// correctly ordered and wrongly timed, and nothing in it says so. A
    /// makespan that is quietly missing several steps is worse than a refusal,
    /// because a caller cannot tell it happened. Refusing names the technology
    /// and the trigger kind, so a caller can see exactly what is not modelled.
    ///
    /// `act` says what the game wants done, in the trigger's own words
    /// (`build 1 asteroid-collector`, `capture a spawner`, `create a space
    /// platform`), so a reader learns the *act* that is missing and not only
    /// the kind. Shipped 2.1.17 has three such technologies --
    /// `space-science-pack` (build an asteroid collector), `biter-egg-handling`
    /// (capture a spawner) and `space-platform` (create one) -- and no action
    /// in this project's executor or mod performs any of those acts, so the
    /// refusal is about the world model, not about a missing emulation.
    #[error(
        "{technology} is unlocked by a {trigger} trigger -- {act} -- which this planner cannot \
         express as a goal"
    )]
    #[diagnostic(
        code(planner::unsupported_research_trigger),
        help(
            "only `craft-item` and `mine-entity` triggers can be planned; costing this one at \
             zero would silently under-report the plan's makespan"
        )
    )]
    UnsupportedResearchTrigger {
        technology: String,
        /// The trigger's `type` string, e.g. `build-entity`.
        trigger: String,
        /// The act the trigger names, as `ResearchTrigger` displays it.
        act: String,
    },

    /// A `craft-item` trigger asking for an item whose recipe only this same
    /// technology unlocks.
    ///
    /// Shipped 2.1.17 really contains six of these — `foundry` is triggered by
    /// crafting a foundry and is the only technology unlocking the foundry
    /// recipe, and `biochamber`, `big-mining-drill`, `cryogenic-plant`,
    /// `tungsten-carbide` and `electromagnetic-plant` are the same shape. The
    /// game resolves them by routes outside this planner's world model.
    ///
    /// Diagnosed here rather than left to recurse: expanding it naively goes
    /// `Researched(t)` -> `Have(item)` -> "that recipe needs `t`" ->
    /// `Researched(t)` until the driver's depth guard fires, and
    /// `ExpansionTooDeep` then reports "a method is probably expanding into
    /// itself" — which is true, and tells a caller nothing about which
    /// technology or why.
    #[error(
        "{technology} is triggered by crafting {item}, but only {technology} unlocks that recipe"
    )]
    #[diagnostic(
        code(planner::self_unlocking_research_trigger),
        help("this technology cannot be reached from the current world state")
    )]
    SelfUnlockingResearchTrigger { technology: String, item: ItemId },

    /// A trigger technology whose trigger *kind* this planner can plan, in a
    /// world whose mod did not say what the trigger names.
    ///
    /// Distinct from [`PlannerError::UnsupportedResearchTrigger`] on purpose.
    /// Until 2026-09-05 `mods/BotBridge/types.lua` sent every trigger but
    /// `craft-item` as its bare type, so every archived dump holds
    /// `{"type": "mine-entity"}` with no entity list, and a planner reading
    /// one cannot tell `oil-processing` (crude oil) from
    /// `uranium-processing` (uranium ore). That is a fact about the *capture*,
    /// not about the trigger kind, and the fix is a new dump -- which is what
    /// this says, where "unsupported" would send a reader to the planner.
    #[error(
        "{technology} is unlocked by a {trigger} trigger that names no entity; the mod that \
         wrote this world could not describe it"
    )]
    #[diagnostic(
        code(planner::undescribed_research_trigger),
        help(
            "dump the world again with the current BotBridge mod, which sends the trigger's \
             entity list"
        )
    )]
    UndescribedResearchTrigger {
        technology: String,
        /// The trigger's `type` string, e.g. `mine-entity`.
        trigger: String,
    },

    /// Nothing in the world's prototypes can mine `entity` at all.
    ///
    /// A verdict about the prototype table, not the map: no `mining-drill`
    /// lists the entity's resource category among the categories it mines,
    /// or the entity has no resource prototype the planner could ask, or the
    /// machine that would mine it has no recipe. Exploring changes none of
    /// that, so this is raised before any prerequisite is planned.
    #[error("nothing in this world's prototypes can extract from {entity}: {why}")]
    #[diagnostic(
        code(planner::no_extractor),
        help(
            "a resource is mined by the machines whose `resource_categories` list its \
             `resource_category`; both fields are sent by the mod since 2026-09-04, so a world \
             captured earlier cannot answer this"
        )
    )]
    NoExtractor { entity: String, why: String },

    /// Mining `entity` takes a machine whose recipe this force has not
    /// unlocked, and nothing in this plan unlocks it.
    ///
    /// The ordinary answer for `oil-processing` planned on its own:
    /// crude oil is mined by a pumpjack, whose recipe `oil-gathering`
    /// unlocks. Planned as a prerequisite of `oil-processing` -- which it is
    /// -- that research is already in the plan and this is not raised; it is
    /// raised for a `Goal::Extracted` stated directly, or by a mod whose
    /// extractor is unlocked outside the technology's own prerequisites.
    #[error(
        "extracting from {entity} takes a {extractor}, whose recipe needs {technology} \
         researched first"
    )]
    #[diagnostic(
        code(planner::extractor_locked),
        help(
            "plan `researched:{technology}` first, or state the research goal that has it as a prerequisite"
        )
    )]
    ExtractorLocked {
        entity: String,
        extractor: String,
        technology: String,
    },

    /// Everything the world needs is in place, and this planner cannot yet
    /// site and run the extractor.
    ///
    /// The honest end of the ladder for `oil-processing` as of 2026-09-05:
    /// the well is charted, a pumpjack mines it, its recipe is reachable --
    /// and no method knows how to stand a pumpjack on a well, power it and
    /// give its output somewhere to go. Refused by name rather than costed at
    /// zero, exactly as an inexpressible trigger is, so the next piece of
    /// work is named by the refusal instead of hidden in a makespan.
    #[error(
        "extracting from {entity} takes a {extractor} standing on it and running, which this \
         planner does not yet know how to site, power or drain"
    )]
    #[diagnostic(
        code(planner::extraction_not_modelled),
        help(
            "the extractor cell -- siting on the patch, power, and where the output goes -- is \
             the next thing to build; see docs/superpowers/specs/2026-09-04-exploration-design.md"
        )
    )]
    ExtractionNotModelled { entity: String, extractor: String },

    /// [`Goal::Gathered`](crate::goal::Goal::Gathered) asked for a fluid
    /// buffer and this world has no prototype that is one, or the one it has
    /// cannot be obtained.
    ///
    /// Tier 2 of `method::gather`'s ladder, and asked of the **world** rather
    /// than of a hard-coded `storage-tank`: the prototype is found by
    /// `entity_type`, so a modded tank answers and a capture that predates the
    /// field refuses by name instead of quietly picking nothing.
    #[error("nothing in this world can gather the {fluid} a {machine} pumps: {why}")]
    #[diagnostic(
        code(planner::no_fluid_buffer),
        help(
            "a fluid cannot be carried in an inventory, so gathering needs a tank standing at \
             the patch; without one there is nowhere for the output to go"
        )
    )]
    NoFluidBuffer {
        fluid: String,
        /// The extractor whose output has nowhere to go. **Named `machine`
        /// and not `source`**: `thiserror` treats a field called `source` as
        /// the error's `std::error::Error` cause and tries to call
        /// `as_dyn_error` on it, which a `String` does not implement.
        machine: String,
        why: String,
    },

    /// A tank was wanted within reach of a resource field and every candidate
    /// footprint out to the search bound was occupied.
    ///
    /// Distinct from [`PlannerError::NoSiteFound`], which is the generic
    /// block-siting refusal: this one names the **field** it was anchored on
    /// and how many of its tiles it was averaging, because the two questions a
    /// reader has are "where did it look" and "was it looking at the right
    /// field at all".
    #[error(
        "no clear {tank} site within {searched} tiles of the {wells}-well {entity} field centred \
         on {centroid}; nearest obstruction: {nearest_obstruction}"
    )]
    #[diagnostic(code(planner::no_tank_site))]
    NoTankSite {
        tank: String,
        entity: String,
        wells: usize,
        centroid: String,
        searched: i32,
        nearest_obstruction: String,
    },

    /// Two fluid machines could not be joined by pipe.
    ///
    /// **Returned before anything is placed**, the same promise
    /// `method::connect`'s `ConnectRefusal` makes and for the same reason: a
    /// pipe run that stops halfway is worse than no pipe run, because the
    /// machine at the near end fills up and stops with nothing to show for the
    /// iron.
    ///
    /// `from` and `to` each name a machine *and* where it stands, in one
    /// string rather than in two fields, because `PlannerError` is returned by
    /// value everywhere and clippy's `result_large_err` is measured against
    /// the **largest** variant: five `String`s here would have pushed the
    /// whole enum over the threshold and cost every `Result` in the crate a
    /// box.
    #[error("no pipe route from {from} to {to}: {why}")]
    #[diagnostic(code(planner::no_pipe_route))]
    NoPipeRoute {
        from: String,
        to: String,
        why: String,
    },

    /// A machine this planner wants to pipe into has no pipe connection it can
    /// read off the prototype table.
    ///
    /// **Never guessed.** A fluid connection placed at the wrong tile yields a
    /// layout that builds 100% correctly and moves nothing -- the same silent
    /// class as an inserter facing the wrong way -- so a prototype whose
    /// `fluidbox_prototypes` are missing, empty, or shaped in a way this
    /// module does not understand is refused by name.
    #[error("cannot tell where a {prototype} takes fluid in or out: {why}")]
    #[diagnostic(
        code(planner::fluid_port_unknown),
        help(
            "fluidbox_prototypes with pipe_connections.positions is what answers this; a world \
             captured before the mod sent them cannot be piped on"
        )
    )]
    FluidPortUnknown { prototype: String, why: String },

    /// A `Step::Owned` whose holder names no bot.
    ///
    /// Deliberately an error rather than "keep the current chain". A handover
    /// exists precisely to move work off the chain it was written in, so a
    /// block of steps addressed to `Holder::Anyone` would quietly weld the
    /// supplier's work back onto the consumer — the very defect convergence
    /// exists to remove — and the plan would look correct while doing the old
    /// thing. There is no world state that makes this right and no replan that
    /// fixes it: it is a mistake in the method that wrote the step, and it
    /// fails where it was written.
    #[error("a handover names {holder}, which is nobody; a Step::Owned must name a bot")]
    #[diagnostic(code(planner::unowned_handover))]
    UnownedHandover { holder: String },

    /// A research whose lab would have no electric supply.
    ///
    /// **Refusing is the point.** Until 2026-09-02 the planner treated
    /// `Researched(tech)` as satisfied by *crafting* a lab: run 30
    /// (`workspace/runs/run-1788365280-15443/`) crafted one on each of five
    /// milestone-7 plans, placed none of them, generated `0.0 kW` in all 541
    /// of its force samples, and sat at `research_progress 0.0` for 60,661
    /// ticks before the action timed out as `lost`. Nothing in the plan said
    /// anything was wrong; a plan that cannot finish is worse than one that
    /// refuses, because a caller cannot tell the first from success in
    /// progress.
    ///
    /// The planner can place the lab and feed it, and cannot yet build a
    /// boiler, a steam engine and the pipes between them — that is a whole
    /// subsystem, and the offshore pump alone needs shoreline geometry nothing
    /// here models. So it states what research needs, checks it, and says so
    /// by name when it is missing.
    ///
    /// `supply_kw` is what [`crate::state::PlanState::electric_supply_kw`]
    /// could actually see, which is **not** the same as what the game has: the
    /// entity graph does not expose poles or generators a live world already
    /// contains, so a hand-built power plant reads as `0` here. See that
    /// method's own doc.
    #[error(
        "{technology} needs a lab with {needed_kw} kW of electric supply, and the plan can show only {supply_kw} kW"
    )]
    #[diagnostic(
        code(planner::research_needs_power),
        help(
            "the planner can place and feed a lab but cannot yet build a generator; a lab with no \
             power researches nothing at all rather than researching slowly"
        )
    )]
    ResearchNeedsPower {
        technology: String,
        needed_kw: f64,
        supply_kw: f64,
    },

    /// There is power, and there is ground, and no tile has both.
    ///
    /// **Deliberately not a `NoApplicableMethod`**, which is what this was and
    /// which printed "no method can satisfy goal: research
    /// logistic-science-pack" — read, correctly and uselessly, as "green
    /// science is out of reach in this world". It is not: a plant stands, the
    /// lab is affordable, and every free tile within reach is simply outside
    /// any pole's supply area.
    ///
    /// The case it was found on is worth stating, because it is structural
    /// rather than unlucky. A small pole's supply area is 5x5; a two-feed
    /// assembly cell (`crate::method::assemble`) fills one, and the cell is
    /// sited *inline* while the research it unlocks is a subgoal expanded
    /// afterwards — so the cell takes the ground first. A red-science cell
    /// leaves a lab-sized hole and a green one does not, which is why green
    /// was the first goal to hit this.
    ///
    /// `powered_blocked` and `free_unpowered` are the two ways a candidate
    /// failed, counted separately, because they call for opposite fixes:
    /// blocked-but-powered ground says "build somewhere else or clear it",
    /// free-but-unpowered ground says "bring a pole".
    ///
    /// **The second of those fixes is one the planner now applies itself.**
    /// `lab_site_with_pole` widens the search to free-but-unlit ground and
    /// brings the pole that lights it, so reaching this error means *that*
    /// failed too — every free tile in range is either out of a pole's supply
    /// area of anything, or the pole it would need is itself out of wire reach
    /// of a generator. `free_unpowered` is therefore no longer a suggestion to
    /// the reader; it is how much ground was tried and rejected.
    #[error(
        "{technology} needs a lab, and no ground within {radius} tiles of [{anchor_x}, {anchor_y}] is \
         both free and inside a supply area — nor can any free ground there be given a pole that \
         reaches a generator ({powered_blocked} powered tiles are built on, \
         {free_unpowered} free ones have no supply)"
    )]
    #[diagnostic(
        code(planner::research_needs_room),
        help(
            "the plan has power and space but not both in one place, and extending the network \
             to the space does not reach either; a pole's supply area is 5x5 and its wire reach \
             7.5 tiles, and whatever this plan sited first has taken the ground inside both"
        )
    )]
    ResearchNeedsRoom {
        technology: String,
        radius: i32,
        anchor_x: f64,
        anchor_y: f64,
        powered_blocked: u32,
        free_unpowered: u32,
    },

    /// No water within `PLANT_WATER_WIDE_SCAN_RADIUS` of the acting bot.
    ///
    /// **The only distance refusal a power plant has left.** There used to be
    /// a second, `PowerPlantTooFarFromWater`, which refused water the planner
    /// could see but judged too far to carry a plant to. Its 64-tile bound was
    /// borrowed from a pole's supply area and guarded a walk that
    /// [`crate::schedule`] already prices, and it halted run
    /// `run-1788379071-00467` at rung 7 over 3.8 tiles -- in a run whose four
    /// bots had each already been 68 to 72 tiles from spawn. Distance is now a
    /// cost, not a veto; see `crate::method::power`'s two scan radii for the
    /// derivation.
    ///
    /// So this says what was *looked at*, not what is *allowed*, and the two
    /// other things that produce it matter more than ever, because both are
    /// worth telling apart from a genuinely dry map:
    ///
    /// * a world attached from a snapshot (`crates/core`'s `attach_world`)
    ///   fetches **no tiles at all**, so every question about terrain answers
    ///   "nothing there";
    /// * an owned run only knows the chunks the game has charted.
    ///
    /// # It says where it stood, and how much of that disc it could see
    ///
    /// Both additions are about the same failure: **absence of data read as
    /// absence of water.**
    ///
    /// `radius` alone named a distance without an origin, so "none within 128
    /// tiles" was a true sentence about an unstated place. The search has two
    /// anchors — the caller's position, and [`crate::method::power`]'s world
    /// anchor on the retry — and a reader chasing "but the lake is at 48
    /// tiles" cannot tell which one refused. `anchor_x` / `anchor_y` say.
    /// That ambiguity cost an hour on 2026-09-06.
    ///
    /// `covered_probes` / `probes` are [`crate::state::ChartingScore`] over
    /// the same disc, and they carry the asymmetry this repo insists on
    /// elsewhere (`EntityGraph::resource_fingerprint`, `runMatch.ts`):
    ///
    /// * `covered_probes == probes` — the ground was written out and it is
    ///   dry. A statement about the **map**.
    /// * `covered_probes < probes` — part of that disc was never generated,
    ///   so nothing has ever been seen there and there is nothing to be
    ///   stale about. A statement about the **dump**, and walking a bot into
    ///   it and replanning can change the answer.
    ///
    /// Measured 2026-09-06: on `map.json` and `map-31337-explored.json` alike
    /// the world anchor's disc is 17/17 covered, so this is a state the
    /// current maps do not reach — but the same anchor at (255, 249) is 7/17,
    /// and a refusal from there would have been the blind kind with no way to
    /// say so. See `docs/superpowers/notes/2026-09-06-a-water-refusal-says-
    /// where-it-stood.md`.
    #[error(
        "a power plant needs water, and the plan can see none within {radius} tiles of \
         ({anchor_x:.1}, {anchor_y:.1}), where charted ground covers {covered_probes} of \
         {probes} probes"
    )]
    #[diagnostic(
        code(planner::power_plant_needs_water),
        help(
            "the plant is sited at the water because water is the one input that cannot be \
             carried; a world attached from a snapshot carries no tiles at all, and an owned run \
             knows only the chunks the game has charted -- so all probes covered means the \
             ground was there and it is dry, and fewer means part of that disc was never \
             generated and walking a bot into it can change the answer"
        )
    )]
    PowerPlantNeedsWater {
        radius: f64,
        /// Where the search stood. Not always the caller's own position:
        /// `method::power`'s retry re-asks from `plant_world_anchor()`.
        anchor_x: f64,
        anchor_y: f64,
        /// Probes of the same disc that landed on ground the model has a tile
        /// for, out of `probes`. Equal means charted and dry; fewer means
        /// blind.
        covered_probes: usize,
        probes: usize,
    },

    /// Water near enough, but no piece of its edge with room behind it.
    ///
    /// A pump needs a straight shoreline — one ground tile under it and a
    /// three-by-two block of water in front — and the boiler, the engine and
    /// the pipes need about five by nine tiles of clear ground behind that.
    #[error(
        "the nearest water is {distance:.1} tiles away, but no shoreline within {radius} tiles of \
         it has room for a pump, a boiler, a steam engine and the pipes between them"
    )]
    #[diagnostic(
        code(planner::power_plant_needs_shore),
        help(
            "the pump wants one ground tile with a three-by-two block of water in front of it, \
             and the plant behind it wants about five tiles by nine of clear ground"
        )
    )]
    PowerPlantNeedsShore { distance: f64, radius: f64 },

    /// The demand asked for is more than one boiler's worth of steam engines.
    ///
    /// **Roadmap item 3's refusal: power modelled as capacity, not coverage.**
    /// Until 2026-09-06 `method::power::plan_plant` took no `kw` at all — it
    /// built one pump, three pipes, one boiler, one steam engine and one pole,
    /// 900 kW, and handed it back for any demand whatever. Three of
    /// `supply_for`'s four tiers checked the kilowatts asked for; the fourth,
    /// the one that *builds*, did not. So a plan wanting 2,000 kW got a plant
    /// short by 1,100 and an `Ok`, and the shortfall surfaced later as a
    /// `Condition::Powered` that would not hold — the symptom, several layers
    /// from the decision that caused it.
    ///
    /// The plant now sizes its engines from the demand. This is the bound on
    /// that sizing: vanilla's boiler states `energy_consumption = "1.8MW"` and
    /// a steam engine's 900 kW falls out of `fluid_usage_per_tick = 0.5` at
    /// 165 °C, so **one boiler carries exactly two engines** — a fact about
    /// the prototypes rather than a chosen constant.
    ///
    /// # 1.8 MW is this planner's ceiling and **not the game's**
    ///
    /// Saying only "one boiler drives two engines" reads as though 1.8 MW were
    /// a fact about Factorio. It is not. Verified against
    /// `workspace/server/data/base/prototypes/entity/entities.lua` on
    /// 2026-09-06:
    ///
    /// * `offshore-pump` states `pumping_speed = 20`, which is fluid units per
    ///   **tick** — 1,200 water/s;
    /// * `boiler` states `energy_consumption = "1.8MW"` and
    ///   `target_temperature = 165`. Water carries 0.2 kJ per unit per degree,
    ///   so 150 °C above the 15 °C default is 30 kJ a unit and a boiler burns
    ///   1.8 MW / 30 kJ = **60 water/s**.
    ///
    /// 1,200 over 60 is **one pump to 20 boilers to 40 engines, about 36 MW**.
    /// So the water behind a single offshore pump supports twenty times what
    /// this planner will lay out, and what refuses here is the *layout*: the
    /// plant is a rigid row (pump, [`PIPE_COUNT`](crate::method::power::PIPE_COUNT)
    /// pipes, one boiler, up to
    /// [`MAX_ENGINES_PER_BOILER`](crate::method::power::MAX_ENGINES_PER_BOILER)
    /// engines) rotated as one body about the pump's tile centre, and `Plant`
    /// carries exactly one `boiler` position that `plant_steps` fuels once.
    ///
    /// **An owner-supplied figure of "1 pump : 200 boilers : 400 engines" does
    /// not survive the prototypes** — it is ten times the measured ratio, and
    /// the arithmetic above is written out so the next reader can check it
    /// rather than pick between two numbers. Growing the plant is designed in
    /// `docs/superpowers/notes/2026-09-06-one-place-that-decides-power.md` and
    /// deliberately not built here.
    ///
    /// Refusing by name is the point. An undersized plant returned as success
    /// is the *coverage is not capacity* failure one level up: everything
    /// places, everything is wired, and the network browns out.
    /// # Two numbers, because the margin must not masquerade as the demand
    ///
    /// Since 2026-09-07 a plant is sized to
    /// [`PLANT_HEADROOM`](crate::method::power::PLANT_HEADROOM) times the draw
    /// it is asked for, so a demand can be refused here that would have fitted
    /// unmargined. `needed_kw` is **what the caller asked for** and `sized_kw`
    /// is what this planner tried to build; quoting only the second would
    /// report a number nobody supplied, which is the confusion this project
    /// keeps fixing elsewhere. When they differ and `needed_kw` alone would
    /// have fitted, the margin is the reason and the message shows both so a
    /// reader can see it rather than infer it.
    #[error(
        "that needs {needed_kw} kW -- sized with headroom to {sized_kw} kW -- and the largest \
         plant this planner lays out generates {plant_kw} kW"
    )]
    #[diagnostic(
        code(planner::power_plant_too_small),
        help(
            "this is now a limit of the WATER, not of the layout: one offshore pump moves \
             1200 water/s and a boiler burns 60/s, so one pump carries twenty boilers and \
             forty engines -- about 36 MW. A twenty-first boiler would stand on the shore \
             with nothing to boil. (Both figures read from base/prototypes/entity/\
             entities.lua; one boiler still drives at most two engines, 1.8 MW of boiler \
             over 900 kW of engine.) Until 2026-09-06 this said the opposite -- the planner \
             laid out exactly one boiler and the ceiling was 1.8 MW, a fact about our row \
             rather than about the game. Ask for less, or site a second plant against \
             different water"
        )
    )]
    PowerPlantTooSmall {
        needed_kw: f64,
        sized_kw: f64,
        plant_kw: f64,
    },

    /// The poles reach, and what they reach is not big enough.
    ///
    /// # The two answers `Ok(None)` used to give at once
    ///
    /// [`ensure_powered`](crate::method::power::ensure_powered) had one
    /// refusal for "supply exists but no pole run carries it there" and for
    /// "a pole run carries it there and the network has no room", and its
    /// message named only the first. A peer session wiring `Goal::Built` to
    /// it spent **two iterations on pole geometry** for a block that routed
    /// perfectly and was short of kilowatts; the discriminator that finally
    /// split them was replacing the block's draw with a trivial 10 kW.
    ///
    /// "I cannot route to it" and "I routed to it and it is too small" send a
    /// reader to completely different places — the first to the ground between
    /// the plant and the site, the second to the plant. The second is not a
    /// refusal about the site at all.
    ///
    /// # Why not `PowerPlantTooSmall`
    ///
    /// That one is a statement about the **layout**: the largest plant this
    /// planner lays out cannot make `needed_kw`, true from every anchor on
    /// every map, and raised before a single pole is sited. This one is a
    /// statement about **one network at one moment**: a plant that would be
    /// big enough on its own is already committed to other consumers, or the
    /// site is joined to a standing network rather than to a fresh plant.
    /// Adopting a small standing plant and building a large fresh one are
    /// different remedies, so they are different errors.
    ///
    /// `committed_kw` excludes the draw being asked about — it is what the
    /// ledger charges to consumers that are somebody else's, which is the
    /// only figure that makes `supply_kw - committed_kw < needed_kw` read as
    /// arithmetic the reader can check.
    #[error(
        "the network reaching {entity} at {site} generates {supply_kw} kW with {committed_kw} kW \
         already committed elsewhere, leaving {headroom_kw} kW for a draw of {needed_kw} kW"
    )]
    #[diagnostic(
        code(planner::power_headroom_short),
        help(
            "the poles route: this is capacity, not geometry. Build or grow a plant (one boiler \
             drives at most two steam engines, 900 kW each), site this away from the consumers \
             already on that network, or ask for less"
        )
    )]
    PowerHeadroomShort {
        entity: ItemId,
        site: String,
        needed_kw: f64,
        supply_kw: f64,
        committed_kw: f64,
        headroom_kw: f64,
    },

    /// Solar panels stand on this network and the accumulators to carry their
    /// night do not.
    ///
    /// **The refusal exists so the failure is visible at plan time.** A solar
    /// array with no bank does not degrade into a slow base: it runs all
    /// afternoon and dies at 03:00, and this repo twice records that an
    /// under-supplied network reads as *completely dead* rather than as slow.
    /// So the choice is between a refusal a reader sees while planning and a
    /// mystery stall a reader sees in a run log at a tick nobody can explain.
    /// Refusing is the visible one; crediting an unbanked array is the
    /// reckless option wearing the conservative option's clothes.
    ///
    /// # The two numbers are integrals of different things
    ///
    /// `average_kw` is **power** — what the array makes averaged over a whole
    /// day, `crate::state::PlanState::solar_average_kw`, 0.7 of the noon
    /// nameplate on vanilla Nauvis. `accumulators_needed` is sized from
    /// **energy** — the night's shortfall in joules against an accumulator's
    /// `electric_buffer_capacity`, 0.85 accumulators per panel on Nauvis.
    /// They come from the same daylight curve by two different integrals and
    /// neither can be computed from the other, which is why both appear here.
    ///
    /// **An accumulator's own `max_energy_production` is not in either.** That
    /// is its 300 kW *discharge rate*, a per-unit ceiling on delivery, and a
    /// bank sized on it instead of on its 5 MJ store is wrong by a factor that
    /// depends on how long the night is. See
    /// `crate::method::power::solar_bank_for`.
    #[error(
        "{panels} solar panels on this network average {average_kw} kW over a day, and carrying \
         that load through the night needs {accumulators_needed} accumulators of which \
         {accumulators_standing} stand, so the plan credits the array nothing"
    )]
    #[diagnostic(
        code(planner::solar_bank_short),
        help(
            "build the accumulators, or power this from a steam plant; a solar array with no \
             bank does not run slowly at night, it stops, and the plan cannot tell that apart \
             from a base that was never built"
        )
    )]
    SolarBankShort {
        panels: u32,
        average_kw: f64,
        accumulators_needed: u32,
        accumulators_standing: u32,
    },

    /// Solar panels stand and this world cannot say how large a bank they need.
    ///
    /// **Unknown, never zero.** Every term of the sizing comes from the
    /// surface's own daylight curve and the two prototypes' fields, and any of
    /// them can be absent — most commonly on a dump taken before the daylight
    /// channel existed, which is every world this project has archived. A
    /// planner that read an absent curve as "no night" would size a bank of
    /// zero and credit the array in full, which is the midnight failure with
    /// an extra step.
    ///
    /// `because` names which term was missing, so a reader can tell a stale
    /// dump from an unrecognised prototype without re-deriving anything.
    #[error(
        "{panels} solar panels stand on this network and the accumulators they need cannot be \
         sized: {because}"
    )]
    #[diagnostic(
        code(planner::solar_bank_not_sizable),
        help(
            "re-dump the world from a game that reports its surface daylight, or power this \
             from a steam plant, whose output is the same at every hour and needs no bank"
        )
    )]
    SolarBankNotSizable { panels: u32, because: String },

    /// The rate asked for needs more cells than one plan may build.
    ///
    /// A bound on work rather than a claim about what a map could hold. Siting
    /// a cell walks an ore patch, so a rate needing millions of them would hang
    /// rather than refuse; this makes it refuse, and names both numbers so the
    /// caller can see how far past the bound it is.
    #[error("that rate needs {cells} cells and a plan may build at most {max}")]
    #[diagnostic(
        code(planner::too_many_cells),
        help("ask for a smaller rate, or build it up over several plans")
    )]
    TooManyCells { cells: u64, max: u32 },

    /// Nothing this planner knows how to build produces `item` by machine.
    ///
    /// Stage 1 of the starter factory builds exactly one shape of cell: a
    /// burner mining drill dropping into a stone furnace. So the items it can
    /// make are the ones with a **smelting** recipe taking a single ore
    /// ingredient the map actually carries — the four plates, and nothing else.
    /// Everything a bot can still craft by hand remains reachable through
    /// [`crate::goal::Goal::Have`]; what this says is that no *machine* the
    /// planner can build makes it.
    #[error("no cell this planner can build produces {item} by machine")]
    #[diagnostic(
        code(planner::no_cell_produces),
        help(
            "the two cell shapes are a burner drill dropping into a stone furnace (anything \
             that smelts from one ore) and an assembling machine fed by inserters (anything \
             crafted from two ingredients, one of which is itself crafted from one); a bot \
             can still make this by hand through `goal.have`"
        )
    )]
    NoCellProduces { item: ItemId },

    /// Ore is there, but no legal ground for a cell on it.
    ///
    /// A cell wants a drill standing **on** the patch with a furnace two tiles
    /// ahead of it standing **off** the patch, both clear of everything else,
    /// so it fits at a patch edge and nowhere else. A fully enclosed patch, a
    /// patch already built over, or one whose edges the plan has committed
    /// elsewhere produces this.
    #[error(
        "no room for a {ore} cell within {radius} tiles of the patch: a drill needs to stand on \
         the ore with a furnace two tiles ahead of it standing off it"
    )]
    #[diagnostic(
        code(planner::no_room_for_cell),
        help(
            "a cell fits at a patch edge; try a different patch, or clear the ground beside \
             this one"
        )
    )]
    NoRoomForCell { ore: ItemId, radius: i32 },

    /// The map carries no patch of the ore a cell for this item would mine.
    #[error("producing {item} by machine needs a {ore} patch, and the plan can see none")]
    #[diagnostic(
        code(planner::no_patch_for_cell),
        help(
            "a world attached from a snapshot carries only what the snapshot held, and an owned \
             run knows only the chunks the game has charted"
        )
    )]
    NoPatchForCell { item: ItemId, ore: ItemId },

    /// There is power, but no legal ground for an assembly cell inside it.
    ///
    /// A stage-2 cell is eight buildings and a five-tile servicing lane, and
    /// it has to sit close enough to a supplying pole for its own pole to be
    /// wired to that one -- a small pole reaches 7.5 tiles. So this is "the
    /// ground beside your power plant is taken", and the answers are to clear
    /// it, to build the plant somewhere with room, or to ask for fewer cells.
    ///
    /// Distinct from [`PlannerError::NoRoomForCell`], which is about an *ore
    /// patch* having no edge: nothing in this refusal is about ore, and a
    /// caller that conflated the two would chart more map to fix a problem
    /// standing next to a boiler.
    #[error(
        "no room for a {item} cell within {radius} tiles of the pole that would supply it: a \
         cell needs eight clear tiles of ground and a lane to reach its chests from"
    )]
    #[diagnostic(
        code(planner::no_room_for_cell_near_power),
        help(
            "clear the ground beside the power plant, or put the plant somewhere with room \
             around it; a cell must be within one pole's wire reach of a supplied network"
        )
    )]
    NoRoomForCellNearPower { item: ItemId, radius: i32 },

    /// A recipe aimed at ground with no machine on it.
    ///
    /// **A fault in the plan, not a verdict about the world** -- the same
    /// class as [`PlannerError::BufferShort`]. A `SetRecipe` is only ever
    /// emitted for a machine the same expansion placed or the world already
    /// had, so reaching empty ground means the effect was written without the
    /// ordering that puts it after the placement. Silently doing nothing would
    /// leave a later `Condition::RecipeSet` passing against a machine nobody
    /// built, which is exactly the placed-but-dead failure the condition
    /// exists to catch.
    ///
    /// The recipe being locked for the acting force is a *different* refusal
    /// and is not this: see `crate::method::util::recipe_gate`, and the mod's
    /// own by-name refusal in `rcon_set_recipe`.
    #[error("no crafting machine at {position} to set the recipe {recipe} on")]
    #[diagnostic(
        code(planner::no_machine_for_recipe),
        help(
            "a SetRecipe must be ordered after the placement that builds the machine; the \
             condition that does it is EntityAt on the same tile"
        )
    )]
    NoMachineForRecipe { position: String, recipe: ItemId },

    #[error(
        "expansion of {goal} exceeded {depth} levels; a method is probably expanding into itself"
    )]
    #[diagnostic(code(planner::expansion_too_deep))]
    ExpansionTooDeep { goal: String, depth: u32 },

    /// A [`crate::goal::Goal::Built`] named a blueprint string
    /// `factorio_bot_core::blueprint::decode` refuses -- not base64, not
    /// zlib, not JSON, or content outside its allowlist. `reason` is the
    /// decoder's own `Debug` rendering of the `BlueprintError`, which for
    /// `Unsupported` names the offending key.
    #[error("blueprint refused: {reason}")]
    #[diagnostic(code(planner::blueprint_refused))]
    BlueprintRefused { reason: String },

    /// A [`crate::goal::Goal::Built`] cannot be built at the anchor it names,
    /// because the ground one of its entities needs is not clear.
    ///
    /// **The spec's fourth refusal, and it was never built.** Until
    /// 2026-09-05 `BuildBlock::expand` checked only whether an entity was
    /// already standing and emitted the placement regardless of what else was
    /// on the tile; occupancy then surfaced from `schedule()` as
    /// [`PlannerError::ChainOwnerInfeasible`], which blames an internal
    /// scheduling decision for a fact about the ground. Four runs across
    /// three anchors were spent distinguishing hypotheses that a named tile
    /// answers in one line, and the note recording them still ends
    /// unresolved.
    ///
    /// `occupant` names what is there -- an entity by prototype name, water,
    /// terrain, a footprint the game already refused, or a **character**,
    /// (ore is not among them: nothing buildable collides with it, so the game
    /// builds over a patch and since `ore-does-not-block` so does this),
    /// which is called out separately when it is one of this plan's own bots:
    /// a roster bot's body blocks a fixed-offset placement exactly like a
    /// rock does, and it is the case a researcher building a block near their
    /// bots hits first. It is cleared by walking, not by moving the block.
    #[error("cannot build {entity} at tile {tile}: {occupant}")]
    #[diagnostic(code(planner::block_ground_occupied))]
    BlockGroundOccupied {
        entity: String,
        tile: String,
        occupant: String,
    },

    /// A [`crate::goal::Goal::Built`] would stand a mining drill on ground
    /// with nothing under it that the drill can mine.
    ///
    /// **This is the refusal siting already made, applied to the anchors
    /// siting never saw.** `drills_are_fed` has always run inside
    /// `search_site`, so a `Site::Near`/`Site::Anywhere` block is screened
    /// per drill before an anchor is chosen. The other two ways an anchor is
    /// produced were not screened at all: a caller-chosen `Site::At`, and --
    /// the one that cost two sessions -- a **recovered** anchor, which
    /// `resolve_site` returns before it reaches the `Site` match, because an
    /// anchor must not move across a replan.
    ///
    /// Unscreened, such an anchor plans cleanly and the *game* refuses the
    /// drill mid-build. That refusal is durable and names no blocker (there
    /// is nothing on the tile -- the problem is what is absent), so the
    /// footprint is remembered as refused, the next replan reports it as
    /// occupied ground, and the block is stranded with a message pointing at
    /// terrain that was never there. Two sessions hunted a tree that did not
    /// exist. See `docs/superpowers/notes/2026-09-07-the-stranded-tile-was-
    /// siting-all-along.md`.
    ///
    /// `provenance` says which unscreened path produced the anchor, because
    /// the remedy differs: a caller-chosen anchor is moved, while a recovered
    /// one means a half-built block is standing on ground it cannot finish on
    /// and must be cleared before it can be rebuilt elsewhere.
    ///
    /// This is deliberately **not** a quality threshold. A drill sharing one
    /// ore tile with three neighbours is a slow block, and slow blocks work;
    /// refusing them would refuse layouts that run. Zero is different in
    /// kind: it is a placement the game itself will reject, and it is
    /// knowable before anything is committed.
    #[error("cannot build this block at anchor {anchor}: {reason}")]
    #[diagnostic(
        code(planner::block_drill_unfed),
        help("the anchor came from {provenance}, which siting never screened for ore")
    )]
    BlockDrillUnfed {
        anchor: String,
        reason: String,
        provenance: String,
    },

    /// **A plan stands an electric machine that nothing powers.**
    ///
    /// Raised by [`crate::powered::audit`] over the finished network, not by
    /// the method that emitted the placement — deliberately, because the
    /// methods that forget are by definition the ones that would not raise it.
    /// Power was opt-in per method and nothing checked; five methods stated a
    /// `Condition::Powered` and five did not, and both of this project's
    /// unpowered-factory incidents (`FurnaceLine`'s 13 poles and no generator,
    /// `method::fabricate`'s 420 kW `oil-refinery`) came out of that silence.
    ///
    /// **Coverage is not capacity, and this is the coverage half.** It says
    /// nobody even *claimed* the machine is powered. The capacity half is
    /// `Condition::Powered`'s own headroom arithmetic, which the claim is
    /// checked by when it exists.
    #[error(
        "the {prototype} this plan places at {pos} draws {kw:.0} kW and nothing in the plan \
         powers it ({label})"
    )]
    #[diagnostic(
        code(planner::unpowered_consumer),
        help(
            "the method that emitted this placement must call \
             `method::power::ensure_powered` and put the `Condition::Powered` it hands back on \
             the placement -- a machine on no network stands there drawing nothing and reads \
             as built"
        )
    )]
    UnpoweredConsumer {
        prototype: String,
        pos: String,
        kw: f64,
        label: String,
    },

    /// The plan places an **electric consumer nobody can price**, so no plant
    /// can be sized for it and no headroom test can be stated about it.
    ///
    /// *Absent is not a value*: the world says this prototype has an electric
    /// energy source, and `state.rs`'s `consumer_kw` has no figure for it.
    /// Charging it zero is the direction that table's own doc calls unsafe —
    /// headroom that is not there — and it is how three `small-lamp`s were
    /// budgeted at nothing.
    #[error(
        "the {prototype} this plan places at {pos} is electric and nothing knows what it draws"
    )]
    #[diagnostic(
        code(planner::unpriced_consumer),
        help(
            "send its `energy_usage` over the bridge, or give it a row in \
             `state.rs`'s `vanilla_consumer_kw` -- a consumer priced at nothing is headroom \
             that does not exist"
        )
    )]
    UnpricedConsumer { prototype: String, pos: String },

    /// The plan places a **generator that no pole reaches**, so it generates
    /// into nothing.
    ///
    /// The other half of [`Self::UnpoweredConsumer`], and a separate refusal
    /// because the **remedy is different**: a consumer with no power needs a
    /// plant *or* a pole run, while a generator needs only the pole. A steam
    /// engine joined to nobody stands, burns its fuel, reports as built, and
    /// leaves the network reading exactly as if it were not there — the
    /// `FurnaceLine` failure with the polarity reversed.
    #[error(
        "the {prototype} this plan places at {pos} generates {kw:.0} kW and no pole reaches it          ({label})"
    )]
    #[diagnostic(
        code(planner::generator_not_wired),
        help(
            "put a pole within its supply area -- a generator wired to nothing feeds no              network, and coverage is what it is missing, not capacity"
        )
    )]
    GeneratorNotWired {
        prototype: String,
        pos: String,
        kw: f64,
        label: String,
    },

    /// The plan places a prototype **the world never described**, so nothing
    /// can say whether it needs power, what it draws, or how big it is.
    ///
    /// The third answer of three. A prototype-less name is "I have never heard
    /// of this", which is not "it needs no power" — and treating the two alike
    /// is the defect class this whole check exists to close.
    #[error(
        "the plan places a {prototype} at {pos} and this world has no prototype for it ({label})"
    )]
    #[diagnostic(
        code(planner::unknown_prototype_placed),
        help(
            "dump a world that describes it, or place something the world knows -- nothing \
             here can say whether it needs power"
        )
    )]
    UnknownPrototypePlaced {
        prototype: String,
        pos: String,
        label: String,
    },

    /// A caller-supplied anchor sits on the wrong grid for this block, so the
    /// game would silently move every entity in it.
    ///
    /// **Factorio snaps a building to the grid its own footprint belongs on,
    /// and does not fail when asked for the wrong one** — an even footprint to
    /// a tile boundary, an odd one to a tile centre. Measured 2026-09-07
    /// (`scripts/does_the_game_snap.lua`): a `stone-furnace` asked for at
    /// (20.5, 20.5) stands at (21.0, 21.0). So a wrong-parity anchor does not
    /// produce a refusal from the game; it produces a **block standing half a
    /// tile from where the planner believes it is**, which then reads as
    /// missing when anything asks `already_stands`, and takes a whole
    /// investigation to find.
    ///
    /// `Site::Anywhere` and `Site::Near` cannot hit this — `search_site`
    /// aligns its seed. This is only reachable for the two anchors a caller
    /// supplies, and both are refused rather than quietly corrected:
    ///
    /// * [`Site::At`](crate::goal::Site::At) promises *"this exact anchor, or
    ///   refuse"*, and moving it would break that promise in the one direction
    ///   the caller cannot see.
    /// * [`Site::Anchored`](crate::goal::Site::Anchored) is a *recorded* fact.
    ///   An anchor that was recorded from a real siting is already aligned, so
    ///   a misaligned one means something upstream is wrong — and silently
    ///   moving it would defeat the entire point of recording it.
    ///
    /// `aligned` names the nearest anchor that would work, so the caller does
    /// not have to know the parity rule to act on this.
    #[error(
        "cannot build this block at anchor {anchor}: it sits on the wrong tile grid,          and the game would move every entity in the block. Use {aligned}"
    )]
    #[diagnostic(
        code(planner::block_anchor_misaligned),
        help(
            "the anchor came from {provenance}; siting aligns its own anchors, callers must align theirs"
        )
    )]
    BlockAnchorMisaligned {
        anchor: String,
        aligned: String,
        provenance: String,
    },

    /// Siting searched out to its bound and every candidate footprint was
    /// occupied.
    ///
    /// Distinct from [`PlannerError::BlockGroundOccupied`], which is about one
    /// named anchor the CALLER chose. This one is about the planner's own
    /// search, so it carries how far it looked — without that, "cannot site"
    /// is indistinguishable from "looked one tile".
    #[error(
        "no clear site for a {entities}-entity block within {searched} tiles of {seed}; \
         nearest obstruction: {nearest_obstruction}"
    )]
    #[diagnostic(code(planner::no_site_found))]
    NoSiteFound {
        entities: usize,
        seed: String,
        searched: i32,
        nearest_obstruction: String,
    },

    /// The capacity for a [`Goal::Sustain`](crate::goal::Goal::Sustain)
    /// stands, and nothing in this planner can make its inputs arrive without
    /// a bot carrying them.
    ///
    /// **This refusal is the honest statement of a gap, not a bug.** A stage-1
    /// burner cell is fed by hand: its ore and its coal are `insert` actions,
    /// and an `insert` is an event with a completion, which is precisely what
    /// a standing supply is not. `method::connect` (`connect_steps`,
    /// `route_belt`, `inserter_facing`) is the primitive that would close it
    /// and **has no caller anywhere in the tree**; until it has one, a
    /// `Sustain` whose window outlives one hand charge cannot be planned and
    /// saying so by name is better than planning a cell that will be measured
    /// `roster-fed`.
    ///
    /// It is a *refusal* and not an empty plan for the reason
    /// [`crate::method::have::holds`] gives: an empty network is this
    /// planner's word for "done", and a standing rate is exactly what it
    /// cannot know is done.
    #[error(
        "{per_minute} {item}/min: {inputs}. Whether the rate held over {window_ticks} ticks \
         is a window of history and no reading of the world settles it"
    )]
    #[diagnostic(
        code(planner::sustain_supply_not_standing),
        help(
            "a burner cell's ore and coal arrive as `insert` actions, which is a bot's hands; \
             belting them in needs `method::connect`, which has no caller yet"
        )
    )]
    SustainSupplyNotStanding {
        item: ItemId,
        per_minute: u32,
        window_ticks: crate::ids::Ticks,
        /// The inputs that have no standing deliverer, comma-separated.
        inputs: String,
    },

    /// No ground within reach holds a drill that could mine the cell's fuel
    /// and drop it into a buffer.
    ///
    /// Distinct from [`PlannerError::NoPatchForCell`], which means the map
    /// carries none of the resource at all: this one means the patch is there
    /// and no *site* on it takes a drill with a free tile in front of it.
    #[error(
        "no site within {radius} tiles of the {fuel} patch takes a drill with a buffer in front \
         of it, so the cell has no standing fuel source"
    )]
    #[diagnostic(
        code(planner::sustain_no_fuel_source),
        help(
            "the drill needs {fuel} under its whole footprint and one clear tile at its drop \
             point for the buffer chest"
        )
    )]
    SustainNoFuelSource { fuel: ItemId, radius: i32 },

    /// The fuel could be mined and buffered, and no belt run joins the buffer
    /// to a machine that has to burn it.
    ///
    /// **This is a measurement, not a failure**: `method::connect`'s search
    /// window is `2 * enclosure::SEARCH_RADIUS` tiles across, so two patches
    /// further apart than that cannot be joined by one call however clear the
    /// ground is, and an obstacle wider than the `underground-belt`
    /// prototype's reach cannot be tunnelled under (since 2026-09-09 a
    /// narrower one is). The message carries the primitive's own sentence.
    #[error("nothing can carry {fuel} from the buffer at {from} to the {machine} at {to}: {why}")]
    #[diagnostic(
        code(planner::sustain_no_route_for_fuel),
        help(
            "a belt run is planned in one window centred on the buffer; a machine outside it \
             cannot be reached, and an obstacle wider than an underground pair's reach cannot \
             be tunnelled under"
        )
    )]
    SustainNoRouteForFuel {
        fuel: ItemId,
        machine: String,
        from: String,
        to: String,
        why: String,
    },

    /// A stage-1 cell already makes the ingredient an assembly cell has to be
    /// supplied with, and no belt run joins the two.
    ///
    /// **This is the difference between a cell and a factory, refused by
    /// name.** An assembly cell's supply chest is charged once with
    /// `CELL_CHARGE_TICKS` worth of ingredients and nothing refills it; the
    /// live cell of `run-1788730139-71923` earned this project's first
    /// `factory` verdict and then stopped dead at 9:48 when its charge ran
    /// out. So when a renewing source of that ingredient *is* standing, the
    /// run to it is not an optional improvement, and quietly falling back to
    /// the hand charge would rebuild exactly the arrangement that stopped.
    ///
    /// Reached only when a source was found: a world with no stage-1 cell for
    /// the supplied item plans exactly as it did before, hand charge and all.
    #[error(
        "a cell already makes {item} at {from} and nothing can carry it to the supply chest at          {to}: {why}"
    )]
    #[diagnostic(
        code(planner::assembly_no_route_for_supply),
        help(
            "the run is planned in one window centred on the source container, so a supply              chest further away than that cannot be reached; site the assembly cell nearer the              cell that feeds it, or clear the ground between them"
        )
    )]
    AssemblyNoRouteForSupply {
        item: ItemId,
        from: String,
        to: String,
        why: String,
    },

    /// An assembly cell eats a **smelted** ingredient by belt, and no standing
    /// stage-1 cell delivers enough of it.
    ///
    /// **This is the owner's "no chests" made a refusal.** A cell used to be
    /// charged with `CELL_CHARGE_TICKS` of every ingredient by hand and
    /// stopped dead when the charge ran out -- 15 packs, 9:48 into every run.
    /// Since 2026-09-09 a smelted ingredient (one `produce::cell_spec` can
    /// make from ore) has no chest in the cell at all: it arrives on a belt
    /// out of a standing stage-1 cell's plate chest, and a cell asked for
    /// with no such source is refused by name rather than planned as a cell
    /// that dies. The remedy is composition -- `goal.all{ goal.sustain(
    /// <ingredient>, <rate>, <window>), goal.producing(<item>, <rate>) }`
    /// -- which `Goal::All`'s in-order expansion puts in the overlay before
    /// this cell is sited from it.
    ///
    /// `standing` names what *was* found: nothing, or sources whose rate the
    /// cells already asked for have spoken for. A source is one furnace with
    /// an offtake arm into a container, and one furnace is `3600 /
    /// smelting_ticks` plates a minute; a second cell wants a second source.
    #[error(
        "a cell making {item} eats {per_minute} {ingredient}/min by belt and no standing cell \
         delivers it: {standing}"
    )]
    #[diagnostic(
        code(planner::assembly_no_standing_source),
        help(
            "a smelted ingredient is belted straight into the machine from a stage-1 cell's plate \
             chest, and nothing here charges a chest by hand any more; compose the goal with a \
             `sustain` of that ingredient at that rate (`goal.all{{ goal.sustain(\"{ingredient}\", \
             {per_minute}, window), goal.producing(\"{item}\", ...) }}`)"
        )
    )]
    AssemblyNoStandingSource {
        item: ItemId,
        ingredient: ItemId,
        per_minute: u32,
        standing: String,
    },

    /// The cell stands and nothing can take its product away.
    ///
    /// **Measured, in `run-1788679826-02267`.** A belted burner cell ran for
    /// 27,249 ticks with no bot in the loop and still came back `SHORT`, at
    /// 100% of nominal tick rate, because its furnace's own status line read
    /// `working 80, no_ingredients 19, no_fuel 15, full_output 15`: with
    /// nothing emptying the output slot, iron-plate production decayed
    /// 166 -> 72 -> 8 across the run while coal and ore held flat. A cell with
    /// no offtake sustains a *window*, not a rate.
    ///
    /// So this is a refusal and not a silently-omitted improvement: an
    /// arrangement that cannot be emptied is one this method knows will
    /// throttle, and saying so by name is worth more than building it anyway.
    #[error("the {machine} at {at} makes {item} and nothing within reach can take it away: {why}")]
    #[diagnostic(
        code(planner::sustain_no_offtake),
        help(
            "an offtake is one inserter on the machine's perimeter with a container on the tile \
             beyond it, plus a belt run that keeps that inserter fuelled; all three need free \
             ground that is not the machine's own"
        )
    )]
    SustainNoOfftake {
        item: ItemId,
        machine: String,
        at: String,
        why: String,
    },

    /// No recipe this planner can run produces the item that was asked for.
    ///
    /// **Deliberately not a `NoApplicableMethod`.** That variant says "nobody
    /// here knows how", which for a fluid is read as "this is hard" rather
    /// than "the recipe that makes it is one no character and no furnace can
    /// run". The wrapped [`crate::products::ProductRefusal`] carries the three
    /// ordered tiers and names every producing recipe with its category, which
    /// is the whole diagnosis.
    ///
    /// The refusal exists because the lookup underneath it used to be
    /// `recipes.get(item)` -- a product looked up by *recipe* name, which
    /// answers `None` for all 62 products in vanilla 2.1.17 that no recipe is
    /// named after, every fluid among them. See `crate::products`.
    #[error("{0}")]
    #[diagnostic(
        code(planner::product_not_makeable),
        help(
            "this planner runs `crafting` (a character's hands) and `smelting` (a furnace) and \
             nothing else; a product made only by an oil refinery, a chemical plant or a foundry \
             needs a method that stands one up first"
        )
    )]
    /// Boxed, like [`PlannerError::CannotFabricate`] beside it: the refusal
    /// carries a `Vec<Candidate>` per tier and, since a caller can name a
    /// recipe, up to two of them at once. Unboxed it made every
    /// `Result<_, PlannerError>` in the crate 128 bytes wide
    /// (`clippy::result_large_err`), which is a cost paid by every function
    /// that never refuses.
    ProductNotMakeable(#[from] Box<crate::products::ProductRefusal>),

    /// A [`crate::goal::Goal::Have`] was stated about a fluid.
    ///
    /// **Not a shortfall and not a missing method.** `Have` means "this is in
    /// an inventory", and the game will not put a fluid in one -- so the goal
    /// is not unsatisfiable, it is *inexpressible*, and no amount of mining,
    /// research or exploration moves it. The driver refuses it before any
    /// method is asked, which is what stops the roster splitter dividing 100
    /// petroleum-gas into four shares of 25 and asking a character to carry
    /// one; that message ("a share sized for bot 1") named the wrong thing
    /// entirely and is the reason this variant exists.
    ///
    /// The wrapped [`crate::substance::FluidRefusal`] carries the fluid, the
    /// count that was asked for, and -- via
    /// [`crate::substance::FluidSource`] -- which recipes in *this* world
    /// produce it and in what categories, so a reader learns where the fluid
    /// would have to come from instead.
    #[error("{0}")]
    #[diagnostic(
        code(planner::fluid_not_item),
        help(
            "a fluid lives in a fluidbox -- a pipe, a storage tank, a machine's own -- and this \
             planner has no goal that names one; `gathered:<fluid>` stands a pumpjack and a tank \
             up on a field, which is as close as the planner gets today"
        )
    )]
    FluidNotItem(#[from] crate::substance::FluidRefusal),

    /// A machine was named for the recipe's category and still cannot run it.
    ///
    /// **Deliberately not a `NoApplicableMethod`, and deliberately distinct
    /// from [`PlannerError::ProductNotMakeable`].** That one says "no recipe
    /// this planner can run makes it", which was the honest answer while the
    /// world model could not say what a machine crafts. Since
    /// `crafting_categories` crosses the bridge the machine *is* named, so a
    /// reader who still gets "no machine" would go looking for the wrong
    /// thing entirely. The wrapped
    /// [`crate::method::fabricate::FabricateRefusal`] names the machine, the
    /// recipe, its category and the fluid.
    #[error("{0}")]
    #[diagnostic(transparent)]
    ///
    /// **Boxed**, and that is not cosmetic: `FabricateRefusal::NoFluidSource`
    /// carries five owned strings and a [`crate::substance::FluidSource`], and
    /// inlining it took `PlannerError` past `clippy::result_large_err` -- every
    /// `Result<_, PlannerError>` in the crate would have paid for a refusal
    /// almost nothing returns.
    CannotFabricate(#[from] Box<crate::method::fabricate::FabricateRefusal>),
}

/// The payload of [`PlannerError::TargetInsideThreat`].
///
/// A struct rather than variant fields so the variant can be boxed; the
/// `Display` here is the refusal's whole message, and it names the standoff's
/// **provenance** along with its size -- `25 tiles (named fallback; the mod
/// sends no attack range)` rather than a bare `25`, because a refusal that
/// quotes an assumption as though the game had said it is worse than no
/// refusal at all.
#[derive(Debug)]
pub struct TargetInsideThreatDetail {
    pub item: ItemId,
    /// The prototype the plan wanted to gather from, e.g. `huge-rock`.
    pub entity: String,
    /// The target that was given up -- the nearest of the threatened ones.
    pub candidate: Position,
    /// The enemy structure covering it.
    pub threat: String,
    pub threat_at: Position,
    pub distance: f64,
    /// Rendered with its provenance; see [`crate::state::ThreatStandoff`].
    pub standoff: crate::state::ThreatStandoff,
    /// How many candidates were passed over. `0` is impossible -- this
    /// refusal is only reachable with at least one.
    pub passed_over: usize,
}

impl std::fmt::Display for TargetInsideThreatDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "every {} that yields {} is inside an enemy's reach ({} passed over); the nearest \
             candidate at {} is {:.1} tiles from a {} at {}, whose standoff is {}",
            self.entity,
            self.item,
            self.passed_over,
            self.candidate,
            self.distance,
            self.threat,
            self.threat_at,
            self.standoff,
        )
    }
}
