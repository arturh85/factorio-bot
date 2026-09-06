use crate::ids::{ActionId, BotId, ChainId, ItemId};
use crate::state::ChartingSummary;
use factorio_bot_core::types::HandMiningObstacle;
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
    #[error("a power plant needs water, and the plan can see none within {radius} tiles")]
    #[diagnostic(
        code(planner::power_plant_needs_water),
        help(
            "the plant is sited at the water because water is the one input that cannot be \
             carried; a world attached from a snapshot carries no tiles at all, and an owned run \
             knows only the chunks the game has charted"
        )
    )]
    PowerPlantNeedsWater { radius: f64 },

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
    #[error(
        "that needs {needed_kw} kW and the largest plant this planner lays out generates \
         {plant_kw} kW"
    )]
    #[diagnostic(
        code(planner::power_plant_too_small),
        help(
            "this is a limit of the LAYOUT, not of the game: one boiler drives at most two \
             steam engines (1.8 MW of boiler over 900 kW of engine), and this planner lays out \
             exactly one boiler in a rigid pump-pipes-boiler-engines row. The water behind one \
             offshore pump would carry about twenty boilers and forty engines -- ~36 MW -- \
             because the pump moves 1200 water/s and a boiler burns 60/s (verified against \
             base/prototypes/entity/entities.lua). Ask for less, or site a second plant"
        )
    )]
    PowerPlantTooSmall { needed_kw: f64, plant_kw: f64 },

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
    /// **This is the refusal the first rung was most likely to produce and it
    /// is a measurement, not a failure**: `method::connect` refuses rather
    /// than tunnelling where a route needs an underground pair, and its search
    /// window is `2 * enclosure::SEARCH_RADIUS` tiles across, so two patches
    /// further apart than that cannot be joined by one call however clear the
    /// ground is. The message carries the primitive's own sentence.
    #[error("nothing can carry {fuel} from the buffer at {from} to the {machine} at {to}: {why}")]
    #[diagnostic(
        code(planner::sustain_no_route_for_fuel),
        help(
            "a belt run is planned in one window centred on the buffer; a machine outside it, or \
             one an obstacle walls off, cannot be fed without an underground pair, which this \
             planner deliberately does not place"
        )
    )]
    SustainNoRouteForFuel {
        fuel: ItemId,
        machine: String,
        from: String,
        to: String,
        why: String,
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
    ProductNotMakeable(#[from] crate::products::ProductRefusal),
}
