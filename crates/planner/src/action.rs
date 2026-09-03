//! What a bot can do: actions, and the conditions and effects that describe them as data.

use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, ItemId, Ticks};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{Direction, FactorioEntity, Pos, Position};
use serde::{Deserialize, Serialize};

/// Who an action's condition or effect applies to.
///
/// `Role` means "whichever bot runs this action" and is bound by the
/// scheduler. Nothing outside `schedule()` may resolve it without an
/// explicit binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Actor {
    Bound(BotId),
    Role,
}

impl Actor {
    pub fn resolve(&self, binding: BotId) -> BotId {
        match self {
            Actor::Bound(id) => *id,
            Actor::Role => binding,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    HasItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    AtPosition {
        who: Actor,
        pos: Position,
        radius: f64,
        /// The annulus's inner bound: how close is *too* close.
        ///
        /// Zero for every non-placement use of this condition — mining,
        /// inserting, crafting all want a plain disc, satisfied by standing
        /// anywhere from directly on `pos` out to `radius`. A `Place` sets
        /// this to [`crate::state::PlanState::placement_clearance`] instead:
        /// standing on the tile a placement targets satisfies a disc's
        /// `radius` trivially (distance zero), which is exactly how the
        /// planner used to schedule a furnace on the bot's own feet and have
        /// the game refuse it with `player_blocks_placement`. A nonzero
        /// minimum makes that same standing point fail the condition, so
        /// [`crate::schedule::schedule`] emits a real `Walk` instead of
        /// skipping it.
        min_radius: f64,
    },
    EntityAt {
        pos: Position,
        name: ItemId,
    },
    /// One tile is clear.
    ///
    /// **No method emits this**, and none should: a placement wants
    /// [`Condition::AreaFree`], which knows how big the thing being placed is.
    /// It stays because "is this tile clear" is still a meaningful question of
    /// a `PlanState`, and `tests/scheduling.rs` asks it of hand-built networks.
    PositionFree {
        pos: Position,
    },
    /// Room for an entity of `entity` centred on `pos` — its whole collision
    /// box, not just the tile under its centre.
    ///
    /// The condition every `Place` wants. `PositionFree` asks about one tile,
    /// which is true of both tiles of a pair of stone furnaces sited one tile
    /// apart even though the game refuses the second: the furnace is 1.398
    /// tiles across. The size is looked up from the prototype at check time
    /// (`PlanState::is_area_free_facing`), so nothing here has to know it.
    ///
    /// `direction` is Factorio's `defines.direction` on the 2.x scale, and it
    /// is not decoration: a boiler is 3x2 tiles facing north and **2x3 facing
    /// east**, so the same `pos` and `entity` describe different ground at
    /// different angles. Every `Place` this planner emitted before the power
    /// plant carried direction 0, which is why the field is new rather than
    /// old and always zero; the plant turns its boiler and its steam engine to
    /// face whichever way the shoreline does, and a check against the
    /// unrotated box asks about ground the building will not stand on.
    AreaFree {
        pos: Position,
        entity: ItemId,
        direction: u8,
    },
    Researched(String),
    /// Electric supply reaches an entity of `entity` centred at `pos`, with at
    /// least `kw` of generation wired to it.
    ///
    /// **Both halves, deliberately.** Coverage alone — "a pole reaches it" —
    /// is the check that passes on a base with no generator at all, and an
    /// under-supplied network does not degrade into "slow": it reads as
    /// completely dead. Run 30 researched nothing for 60,661 ticks with
    /// `generated_kw = 0.0` in all 541 of its force samples.
    ///
    /// **And it is a network budget, not a per-consumer test.** The kilowatts
    /// this asks for must be *uncommitted*: what
    /// [`crate::state::PlanState::electric_supply_kw`] reports less what
    /// [`crate::state::PlanState::electric_demand_kw`] already spends on the
    /// same network, excluding whatever stands at `pos` itself. Without the
    /// subtraction every consumer is told about the whole engine and thirteen
    /// assemblers pass individually on nine hundred kilowatts — the same
    /// *coverage is not capacity* failure the paragraph above describes, one
    /// level up, and the level at which a factory rather than a lab meets it.
    ///
    /// **Nothing produces this yet**, so `ActionNetwork::infer_edges` draws no
    /// edge to it and no method can satisfy it: it is a statement about the
    /// world, checked at expansion time, and a method that needs it refuses
    /// when [`crate::state::PlanState::electric_supply_kw`] cannot show it.
    /// The day a method builds a boiler and a steam engine, its
    /// `Effect::CreateEntity` lands in the same overlay `electric_supply_kw`
    /// reads and this starts holding with no change here.
    Powered {
        pos: Position,
        entity: ItemId,
        kw: f64,
    },
    /// The machine standing at `from` delivers what it makes into the machine
    /// standing at `to`.
    ///
    /// **The first machine-to-machine claim this planner can state**, and the
    /// reason a `Goal::Producing` is not satisfied by two machines that merely
    /// stand near each other. A burner drill two tiles from a furnace places
    /// 100 %, passes every geometry check and moves nothing if it is facing the
    /// wrong way — the same class of silent failure as `only_ghosts = true`
    /// validating a blueprint whose entities overlap, and as an inserter whose
    /// `direction` names the side it drops into rather than the side it picks
    /// up from.
    ///
    /// **Nothing produces this**, exactly as nothing produces
    /// [`Condition::Powered`]: [`Effect::satisfies`] has no arm for it, so
    /// `ActionNetwork::infer_edges` draws no edge to it and no method can
    /// satisfy it by acting. It becomes true because two
    /// [`Effect::CreateEntity`]s landed with compatible positions and
    /// directions, and it is checked — at expansion time and again by the
    /// scheduler — against [`crate::state::PlanState::delivers_into`].
    ///
    /// Which is why a method that emits one must also order it after both
    /// placements: this condition orders nothing by itself. In stage 1 of the
    /// starter factory the ordering comes free, because the action carrying
    /// this also carries a [`Condition::EntityAt`] for each end.
    Feeds {
        from: Position,
        to: Position,
    },
    ResourceAvailable {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    /// A container or machine standing at `pos` holds at least `count` of
    /// `item`, as far as this plan is concerned.
    ///
    /// The precondition of a withdrawal. It is checked against
    /// [`crate::state::PlanState::buffered`], which is the world's last
    /// reading of that entity minus whatever this plan has already taken out
    /// of it -- so two withdrawals in one plan cannot both spend the same
    /// plates, and the second one fails here rather than at the game.
    ///
    /// # No `slot`, deliberately
    ///
    /// The `ActionKind::Remove` this sits on carries the slot, because the
    /// executor needs it. The condition does not, because a buffer has exactly
    /// one withdrawable inventory by construction: `PlanState::from_world`
    /// records one slot per entity, the one
    /// `LuaEntity::get_output_inventory()` answered for. A slot here would be
    /// a second copy of that decision, able to disagree with the first.
    ///
    /// # Nothing produces it, so it orders nothing
    ///
    /// No `Effect` satisfies this -- see [`Effect::satisfies`] -- exactly as
    /// nothing satisfies [`Condition::ResourceAvailable`], and for the same
    /// reason: items already sitting in a chest are not made by an action in
    /// this plan, any more than ore in the ground is. `infer_edges` therefore
    /// draws no edge to a withdrawal, which is correct; the withdrawal is a
    /// *root*.
    BufferHas {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    /// The crafting machine standing at `pos` is set to make `recipe`.
    ///
    /// **The condition an assembling machine is not finished without.** A
    /// machine placed, powered and fed with no recipe on it is the
    /// placed-but-dead machine this planner spent 2026-09-02 eliminating in
    /// its other forms: it costs materials, occupies ground, reads as built by
    /// every geometry check, and produces nothing at all.
    ///
    /// Read against [`crate::state::PlanState::entity_at`], so it sees the
    /// plan's overlay as well as the world -- `FactorioEntity::recipe` is
    /// carried out of the game by `serialize_entity`
    /// (`mods/BotBridge/types.lua`) for every `assembling-machine`, and
    /// [`Effect::SetRecipe`] writes the same field in the overlay.
    ///
    /// # This condition says nothing about whether the recipe is *available*
    ///
    /// It is a statement about one machine, not about the force. A recipe the
    /// force has not unlocked can be named here and the condition simply will
    /// not hold, because the game will not have set it. A method emitting
    /// [`ActionKind::SetRecipe`] must gate the recipe itself with
    /// [`crate::method::util::recipe_gate`] and honour all four of its
    /// answers -- in particular it must not read
    /// [`crate::method::util::RecipeGate::PlannedResearch`] as `Open`, which
    /// is what dispatched three crafts of a locked recipe at tick zero in
    /// `run-1788338409-63794`. `PlannedResearch` costs no research subgoal and
    /// still needs its `Condition::Researched`, because that condition is the
    /// only thing that orders the recipe after the unlock.
    RecipeSet {
        pos: Position,
        recipe: ItemId,
    },
}

impl Condition {
    pub fn holds(&self, state: &PlanState, binding: BotId) -> bool {
        match self {
            Condition::HasItem { who, item, count } => {
                state.inventory_count(who.resolve(binding), item) >= *count
            }
            Condition::AtPosition {
                who,
                pos,
                radius,
                min_radius,
            } => match state.bot(who.resolve(binding)) {
                Some(bot) => {
                    let distance = calculate_distance(&bot.position, pos);
                    distance.total_cmp(min_radius).is_ge() && distance.total_cmp(radius).is_le()
                }
                None => false,
            },
            Condition::EntityAt { pos, name } => {
                matches!(state.entity_at(pos), Some(e) if &e.name == name)
            }
            Condition::PositionFree { pos } => state.is_position_free(pos),
            Condition::AreaFree {
                pos,
                entity,
                direction,
            } => match Direction::from_u8(*direction) {
                Some(facing) => state.is_area_free_facing(entity, pos, facing),
                // A half-diagonal names no rotation and no building stands on
                // one, so there is no box to compare -- the same "an unknown
                // size is not a guessed size" answer `is_area_free` gives for
                // an unknown prototype.
                None => false,
            },
            Condition::Researched(tech) => state.is_researched(tech),
            Condition::Powered { pos, entity, kw } => match state.collision_area(entity, pos) {
                // No prototype, no footprint, no answer — and the answer this
                // gives is "not powered", matching `is_area_free`'s reading of
                // the same absence: an entity the world cannot size is not one
                // the planner will commit to.
                Some(area) => {
                    // **Uncommitted** capacity, not nameplate. Supply alone is
                    // a per-consumer test that says yes to every consumer for
                    // ever: twelve assemblers on one 900 kW engine each ask
                    // for 75 kW and each are told there is 900. The consumer
                    // being asked about is excluded from the demand, or a
                    // machine this plan already placed is charged against its
                    // own budget and the second check of an identical plan
                    // refuses what the first accepted.
                    let headroom = state.electric_supply_kw(&area)
                        - state.electric_demand_kw(&area, Some(pos));
                    headroom.total_cmp(kw).is_ge()
                }
                None => false,
            },
            Condition::Feeds { from, to } => state.delivers_into(from, to),
            Condition::ResourceAvailable { pos, item, count } => {
                state.resource_available(pos, item) >= *count
            }
            Condition::BufferHas { pos, item, count } => state.buffered(pos, item) >= *count,
            Condition::RecipeSet { pos, recipe } => {
                matches!(state.entity_at(pos), Some(e) if e.recipe.as_deref() == Some(recipe.as_str()))
            }
        }
    }

    /// The position this condition requires the acting bot to stand near, if
    /// any, as `(pos, min_radius, radius)` — the annulus the scheduler must
    /// land the bot inside. `min_radius` is `0.` for every disc (every
    /// non-placement use today), which makes the annulus a disc again.
    pub fn required_position(&self) -> Option<(Position, f64, f64)> {
        match self {
            Condition::AtPosition {
                who: Actor::Role,
                pos,
                radius,
                min_radius,
            } => Some((pos.clone(), *min_radius, *radius)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Condition::HasItem { item, count, .. } => write!(f, "has {} {}", count, item),
            Condition::AtPosition {
                pos,
                radius,
                min_radius,
                ..
            } => {
                if min_radius.total_cmp(&0.0).is_gt() {
                    write!(f, "between {} and {} of {}", min_radius, radius, pos)
                } else {
                    write!(f, "within {} of {}", radius, pos)
                }
            }
            Condition::EntityAt { pos, name } => write!(f, "{} at {}", name, pos),
            Condition::PositionFree { pos } => write!(f, "{} is free", pos),
            Condition::AreaFree {
                pos,
                entity,
                direction,
            } => {
                if *direction == 0 {
                    write!(f, "{} fits at {}", entity, pos)
                } else {
                    write!(f, "{} fits at {} facing {}", entity, pos, direction)
                }
            }
            Condition::Researched(tech) => write!(f, "{} researched", tech),
            Condition::Powered { pos, entity, kw } => {
                write!(f, "{} at {} has {} kW of supply", entity, pos, kw)
            }
            Condition::Feeds { from, to } => {
                write!(f, "the machine at {} feeds the one at {}", from, to)
            }
            Condition::ResourceAvailable { pos, item, count } => {
                write!(f, "{} {} available at {}", count, item, pos)
            }
            Condition::BufferHas { pos, item, count } => {
                write!(f, "{} {} in the buffer at {}", count, item, pos)
            }
            Condition::RecipeSet { pos, recipe } => {
                write!(f, "the machine at {} is set to {}", pos, recipe)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    GainItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    LoseItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    CreateEntity(Box<FactorioEntity>),
    RemoveEntity {
        pos: Position,
    },
    ConsumeResource {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    /// `count` of `item` leave the buffer at `pos`.
    ///
    /// The other half of [`Condition::BufferHas`], and the thing that stops a
    /// plan withdrawing the same plates twice: applying it decrements
    /// [`crate::state::PlanState`]'s overlay, so the next
    /// `Condition::BufferHas` against that tile sees what is really left. It
    /// is also what makes a partial withdrawal *terminate* -- the leftover
    /// `Have` subgoal `Withdraw` emits would otherwise come straight back to
    /// `Withdraw`, find the buffer still full, and expand into itself until
    /// the depth guard fired.
    ///
    /// **There is no `BufferGain`.** Nothing this planner emits puts items
    /// into a buffer *and expects a later goal to count them* -- `smelt_steps`
    /// inserts ore and takes plates inside one method, holding both action ids
    /// and stating the edge itself. A `BufferGain` would exist only to make
    /// that edge inferable, which it already is by other means, and would then
    /// sit in the enum unpaired with anything. Stage 2's chest handover is
    /// where it earns its place; it can be added then, with the method that
    /// needs it.
    BufferLose {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    Researched(String),
    /// The crafting machine standing at `pos` is switched to `recipe`.
    ///
    /// The other half of [`Condition::RecipeSet`], and the reason a stage-2
    /// cell can order its inserters after the recipe rather than hoping.
    /// Applying it rewrites `FactorioEntity::recipe` in the plan's overlay,
    /// which is the same field the game reports out of `serialize_entity`.
    ///
    /// # Idempotent, unlike every other effect here
    ///
    /// `GainItem`, `LoseItem`, `CreateEntity` and `BufferLose` all *change* a
    /// count or a tile, so applying one twice is not applying it once. This
    /// one assigns: setting the same recipe on the same machine again lands on
    /// the same state. That is what makes the matching
    /// [`ActionKind::SetRecipe`] safe to re-dispatch under `recover.rs`'s
    /// tier 1, where a re-run `Insert` would move a second batch of items and
    /// a re-run `Place` would fail on its own building.
    SetRecipe {
        pos: Position,
        recipe: ItemId,
    },
}

impl Effect {
    pub fn apply(&self, state: &mut PlanState, binding: BotId) -> Result<(), PlannerError> {
        match self {
            Effect::GainItem { who, item, count } => {
                state.gain(who.resolve(binding), item, *count);
                Ok(())
            }
            Effect::LoseItem { who, item, count } => state.lose(who.resolve(binding), item, *count),
            Effect::CreateEntity(entity) => {
                state.create_entity((**entity).clone());
                Ok(())
            }
            Effect::RemoveEntity { pos } => {
                state.remove_entity(pos);
                Ok(())
            }
            Effect::ConsumeResource { pos, item, count } => {
                state.consume_resource(pos, item, *count)
            }
            Effect::BufferLose { pos, item, count } => state.take_from_buffer(pos, item, *count),
            Effect::Researched(tech) => {
                state.set_researched(tech);
                Ok(())
            }
            Effect::SetRecipe { pos, recipe } => state.set_recipe(pos, recipe),
        }
    }

    /// Does this effect contribute to making `cond` true? Ordering inference only:
    /// item matching deliberately ignores counts — see `ActionNetwork::infer_edges`.
    pub fn satisfies(&self, cond: &Condition) -> bool {
        match (self, cond) {
            (Effect::GainItem { item, .. }, Condition::HasItem { item: want, .. }) => item == want,
            (Effect::CreateEntity(e), Condition::EntityAt { pos, name }) => {
                &e.name == name && Pos::from(&e.position) == Pos::from(pos)
            }
            (Effect::RemoveEntity { pos }, Condition::PositionFree { pos: want })
            | (Effect::RemoveEntity { pos }, Condition::AreaFree { pos: want, .. }) => {
                Pos::from(pos) == Pos::from(want)
            }
            (Effect::Researched(t), Condition::Researched(want)) => t == want,
            (
                Effect::SetRecipe { pos, recipe },
                Condition::RecipeSet {
                    pos: want,
                    recipe: wanted,
                },
            ) => Pos::from(pos) == Pos::from(want) && recipe == wanted,
            _ => false,
        }
    }
}

/// Which inventory of a target entity an insert or remove addresses.
///
/// Deliberately semantic rather than numeric. Factorio's `defines.inventory`
/// numbers move between game versions, so the executor resolves these names
/// against the running game and never hardcodes an integer.
///
/// The authority for the names is the game's own `defines.inventory`, as
/// published in `runtime-api.json` (`workspace/factorio-api-docs/`). It is not
/// `mods/BotBridge/control.lua`'s `inventory_type_name`: that function is dead
/// 1.1-era code with no callers, it names slots that 2.1 no longer has, and it
/// would raise "table index is nil" if it were ever called. Do not use it as a
/// reference.
///
/// Ordering is derived and load-bearing: the planner is deterministic, so every
/// type reachable from an `ActionNetwork` must order totally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InventorySlot {
    Chest,
    FurnaceSource,
    FurnaceResult,
    Fuel,
    AssemblerInput,
    AssemblerOutput,
    LabInput,
}

impl InventorySlot {
    /// The name used to look this slot up in the game's `defines.inventory`.
    ///
    /// Factorio 2.0 unified furnaces and assemblers into one "crafter" family,
    /// so `FurnaceSource` and `AssemblerInput` both resolve to `crafter_input`,
    /// and both outputs to `crafter_output`. The variants stay distinct because
    /// this enum is the planner's vocabulary, not the game's: a method that
    /// loads a furnace and one that loads an assembler are different methods
    /// even where the game now agrees about the slot.
    ///
    /// The pre-2.0 names `furnace_source`, `furnace_result`,
    /// `assembling_machine_input` and `assembling_machine_output` no longer
    /// exist; asking for them yields no inventory at all.
    pub fn defines_key(self) -> &'static str {
        match self {
            InventorySlot::Chest => "chest",
            InventorySlot::FurnaceSource => "crafter_input",
            InventorySlot::FurnaceResult => "crafter_output",
            InventorySlot::Fuel => "fuel",
            InventorySlot::AssemblerInput => "crafter_input",
            InventorySlot::AssemblerOutput => "crafter_output",
            InventorySlot::LabInput => "lab_input",
        }
    }

    /// Every variant, for exhaustive checks against the game's defines table.
    pub const ALL: [InventorySlot; 7] = [
        InventorySlot::Chest,
        InventorySlot::FurnaceSource,
        InventorySlot::FurnaceResult,
        InventorySlot::Fuel,
        InventorySlot::AssemblerInput,
        InventorySlot::AssemblerOutput,
        InventorySlot::LabInput,
    ];
}

/// What a bot physically does. Carries the payload the executor needs;
/// the planner reasons from `pre` and `eff`, never from this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ActionKind {
    Mine {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    /// Swing at one standing entity that is not a resource -- a tree, a rock --
    /// and take what it yields.
    ///
    /// # Why this is not a `Mine`
    ///
    /// The game makes no distinction: both reach
    /// `rcon_action_start_mining(action_id, player, name, position, count)`,
    /// which does `surface.find_entity(name, position)` and checks
    /// `ent.minable`. The planner does, because [`ActionKind::Mine`] carries
    /// **one** name that is both the entity to swing at and the item that
    /// arrives -- true of every ore and of nothing else. A tree named
    /// `tree-01` yields `wood`. Folding that into `Mine` would mean either
    /// putting `"tree-01"` in a field called `item`, which is a lie a reader
    /// cannot see, or putting `"wood"` there, which the executor would hand
    /// to `find_entity` and the game would answer "no entity to mine".
    ///
    /// `count` is a number of **entities**, not of items: one tree is one
    /// swing and one fixed bill. What arrives is stated by the action's
    /// `Effect::GainItem` and is never inferred from this field.
    Chop {
        pos: Position,
        /// The prototype name the game is asked to find at `pos`.
        entity: String,
        /// What mining it yields -- for the label and for the reader; the
        /// arithmetic lives in the effects.
        item: ItemId,
        count: u32,
    },
    Craft {
        item: ItemId,
        count: u32,
    },
    Place {
        entity: Box<FactorioEntity>,
    },
    Insert {
        pos: Position,
        entity: String,
        slot: InventorySlot,
        item: ItemId,
        count: u32,
    },
    Remove {
        pos: Position,
        entity: String,
        slot: InventorySlot,
        item: ItemId,
        count: u32,
    },
    Research {
        tech: String,
    },
    /// Put `recipe` on the crafting machine named `entity` standing at `pos`.
    ///
    /// `entity` is carried for the same reason [`ActionKind::Insert`] carries
    /// it: the mod addresses an existing building with
    /// `surface.find_entity(name, position)`, which needs both.
    ///
    /// **The one idempotent dispatch in this enum.** See
    /// [`Effect::SetRecipe`].
    SetRecipe {
        pos: Position,
        entity: String,
        recipe: ItemId,
    },
    /// Walk to `to` and stand there. Nothing else.
    ///
    /// The one variant whose dispatch asks the game for nothing beyond the
    /// walk the scheduler already issues to satisfy its own `AtPosition`
    /// precondition -- see `crates/executor::run::perform`. It exists for
    /// exactly one caller, `crate::enclosure::check`: a placement that would
    /// seal a bystander into a pocket with no reachable open ground is
    /// preceded by one of these, pinned to that bystander, so the bot is
    /// clear of the footprint before the walls that would trap it go up.
    ///
    /// No existing variant says "stand here and do nothing else". Reusing one
    /// with a zero count or an already-true condition (`Craft` with `count:
    /// 0`, `SetRecipe` to a recipe already set) would put a real RCON
    /// dispatch where nothing needs to happen once the walk lands, and would
    /// read in the record as the thing it was borrowed from rather than as
    /// what it is.
    Evacuate {
        to: Position,
    },
}

impl ActionKind {
    /// Where this action acts, if it acts anywhere in particular.
    ///
    /// `None` for `Craft` and `Research`: neither touches a tile, so the
    /// absence is a true fact about the action, not a gap in what the
    /// planner knows. Every other variant already carries the position it
    /// needs to do its job -- this just names which field that is, so a
    /// caller does not have to match on the kind itself to ask "where".
    ///
    /// The position returned is the **planner's intent**, straight out of the
    /// plan -- not what the game later reports. Only `Place` has a game-side
    /// answer at all (`RconActuator::place`'s `entity`, carried separately as
    /// `Attempt::placed.actual`); `Mine`/`Insert`/`Remove` name an existing
    /// entity or tile by position and the game never echoes one back, so
    /// there is nothing to prefer over the intent for them, and using the
    /// same field for all four keeps this method's answer meaning one thing.
    ///
    /// For `Mine`, `pos` already carries a resource tile's *centre*
    /// (`resource_tiles_for` -> `EntityGraph::resource_patches` restores the
    /// `.5` offset that tile's flooring `Pos` key would otherwise lose --
    /// see that function's own comment). This method must keep passing it
    /// through unchanged: rounding or flooring it here would reintroduce
    /// exactly the corner-vs-centre bug that once made mining fail on every
    /// map while every test passed.
    pub fn target_position(&self) -> Option<Position> {
        match self {
            ActionKind::Mine { pos, .. }
            | ActionKind::Chop { pos, .. }
            | ActionKind::Insert { pos, .. }
            | ActionKind::Remove { pos, .. }
            | ActionKind::SetRecipe { pos, .. } => Some(pos.clone()),
            ActionKind::Place { entity } => Some(entity.position.clone()),
            ActionKind::Evacuate { to } => Some(to.clone()),
            ActionKind::Craft { .. } | ActionKind::Research { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub kind: ActionKind,
    pub pre: Vec<Condition>,
    pub eff: Vec<Effect>,
    /// Nominal estimate. The observed duration lives in the execution log.
    pub duration: Ticks,
    /// An escape hatch for hand-tuned work. Normally `None`.
    pub pinned: Option<BotId>,
    pub label: String,
}

impl Action {
    /// Where the acting bot must stand, taken from its `AtPosition`
    /// precondition, as `(pos, min_radius, radius)`. The scheduler emits a
    /// walk to satisfy it.
    pub fn required_position(&self) -> Option<(Position, f64, f64)> {
        self.pre.iter().find_map(|c| c.required_position())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{FactorioEntity, Position};
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)])
    }

    // ---- the power budget -------------------------------------------------

    fn put(s: &mut PlanState, name: &str, x: f64, y: f64) {
        s.create_entity(FactorioEntity {
            name: name.into(),
            position: Position::new(x, y),
            ..Default::default()
        });
    }

    /// One steam engine, and a row of small poles all on its network.
    ///
    /// Poles every three tiles along `y = 10.5`, well inside a small pole's
    /// 7.5-tile wire reach, so the whole row is one component. Each pole's
    /// 5x5 supply box is `[x-2.5, x+2.5] x [8, 13]`.
    fn one_engine_and_a_pole_row(poles: usize) -> PlanState {
        let mut s = state();
        for k in 0..poles {
            put(&mut s, "small-electric-pole", 10.5 + 3. * k as f64, 10.5);
        }
        put(&mut s, "steam-engine", 12.5, 10.5);
        s
    }

    fn powered_for(name: &str, x: f64, y: f64, kw: f64) -> Condition {
        Condition::Powered {
            pos: Position::new(x, y),
            entity: name.into(),
            kw,
        }
    }

    /// **Twelve assemblers exhaust a 900 kW engine, and the thirteenth must be
    /// refused.**
    ///
    /// Each one asks "is there 75 kW of supply here" and, on a per-consumer
    /// test, each one is told yes -- for ever. This is "coverage is not
    /// capacity" one level up: the network is fully connected, every check
    /// passes, and the base browns out.
    #[test]
    fn the_thirteenth_assembler_on_one_engine_is_refused() {
        let mut s = one_engine_and_a_pole_row(12);
        // Twelve assembling machines, one per pole, each covered by it:
        // a 3x3 machine at `y = 14` has a box of `[12.8, 15.2]`, which meets
        // the pole row's `[8, 13]`.
        for k in 0..12 {
            put(&mut s, "assembling-machine-1", 10.5 + 3. * k as f64, 14.0);
        }
        // 12 x 75 kW is exactly the engine's 900 kW: the network is full.
        let thirteenth = powered_for("assembling-machine-1", 10.5, 7.6, 75.);
        assert!(
            !thirteenth.holds(&s, BotId(1)),
            "twelve assemblers already draw the whole 900 kW; the thirteenth \
             has nothing left to run on"
        );
    }

    /// The other side of the same rule: a consumer the network *can* still
    /// carry is not refused.
    #[test]
    fn a_consumer_a_full_network_can_still_carry_is_allowed() {
        let mut s = one_engine_and_a_pole_row(12);
        for k in 0..11 {
            put(&mut s, "assembling-machine-1", 10.5 + 3. * k as f64, 14.0);
        }
        // 11 x 75 = 825 kW committed, 75 kW left, and 75 kW asked for.
        let twelfth = powered_for("assembling-machine-1", 10.5, 7.6, 75.);
        assert!(
            twelfth.holds(&s, BotId(1)),
            "825 kW committed of 900 leaves exactly the 75 this one wants"
        );
    }

    /// **Excluding self is not optional.** A consumer already standing must
    /// not be counted against its own budget, or the second check of an
    /// identical plan refuses what the first accepted -- a non-idempotent
    /// predicate, which in a supervisor loop is an oscillation.
    #[test]
    fn a_standing_consumer_is_not_charged_against_itself() {
        let mut s = one_engine_and_a_pole_row(12);
        for k in 0..12 {
            put(&mut s, "assembling-machine-1", 10.5 + 3. * k as f64, 14.0);
        }
        // The twelfth one, already standing, asked about again. Eleven others
        // draw 825 kW; it wants 75; 900 - 825 >= 75.
        let itself = powered_for("assembling-machine-1", 43.5, 14.0, 75.);
        assert!(
            itself.holds(&s, BotId(1)),
            "a machine that already stands pays for itself once, not twice"
        );
    }

    /// A consumer on a *different* network does not eat this one's budget.
    #[test]
    fn a_consumer_on_another_network_spends_nothing_here() {
        let mut s = one_engine_and_a_pole_row(1);
        // Twenty tiles away, past a small pole's 7.5-tile wire reach: its own
        // island, with its own consumers and no generator.
        put(&mut s, "small-electric-pole", 40.5, 10.5);
        for k in 0..12 {
            put(&mut s, "assembling-machine-1", 38.5 + 1.0 * k as f64, 14.0);
        }
        let here = powered_for("assembling-machine-1", 10.5, 7.6, 75.);
        assert!(
            here.holds(&s, BotId(1)),
            "twelve assemblers on an island of their own draw nothing from \
             the engine's network"
        );
    }

    #[test]
    fn role_resolves_to_the_binding_and_bound_ignores_it() {
        assert_eq!(Actor::Role.resolve(BotId(2)), BotId(2));
        assert_eq!(Actor::Bound(BotId(1)).resolve(BotId(2)), BotId(1));
    }

    #[test]
    fn has_item_checks_the_bound_actor_not_the_binding() {
        let mut s = state();
        s.gain(BotId(1), "coal", 5);
        let cond = Condition::HasItem {
            who: Actor::Bound(BotId(1)),
            item: "coal".into(),
            count: 5,
        };
        // Holds for bot 1's inventory even when bot 2 is executing.
        assert!(cond.holds(&s, BotId(2)));
    }

    #[test]
    fn has_item_with_role_follows_the_binding() {
        let mut s = state();
        s.gain(BotId(1), "coal", 5);
        let cond = Condition::HasItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 5,
        };
        assert!(cond.holds(&s, BotId(1)));
        assert!(!cond.holds(&s, BotId(2)));
    }

    #[test]
    fn at_position_respects_the_radius() {
        let mut s = state();
        s.set_position(BotId(1), Position::new(0., 0.));
        let near = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 5.0,
            min_radius: 0.0,
        };
        let far = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 4.9,
            min_radius: 0.0,
        };
        assert!(near.holds(&s, BotId(1)));
        assert!(!far.holds(&s, BotId(1)));
    }

    #[test]
    fn at_position_respects_the_annulus() {
        // The fix in miniature: the exact scenario milestone 4 hit. Standing
        // on the target satisfies a disc (distance zero is `<= radius`) but
        // must fail an annulus whose inner bound is positive, and standing
        // at a legitimate distance must still hold.
        let mut s = state();
        s.set_position(BotId(1), Position::new(3., 4.));
        let annulus = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 5.0,
            min_radius: 1.0,
        };
        assert!(
            !annulus.holds(&s, BotId(1)),
            "standing exactly on the target must fail an annulus with a positive inner bound"
        );

        s.set_position(BotId(1), Position::new(4., 4.));
        assert!(
            annulus.holds(&s, BotId(1)),
            "one tile out clears the inner bound and stays within the outer one"
        );

        // The boundaries themselves: touching either edge holds, matching the
        // disc's own inclusive `<=` at its outer edge.
        s.set_position(BotId(1), Position::new(4., 4.0)); // distance 1.0 == min_radius
        assert!(annulus.holds(&s, BotId(1)), "the inner edge itself holds");
        s.set_position(BotId(1), Position::new(8., 4.0)); // distance 5.0 == radius
        assert!(annulus.holds(&s, BotId(1)), "the outer edge itself holds");
        s.set_position(BotId(1), Position::new(8.1, 4.0)); // distance 5.1 > radius
        assert!(
            !annulus.holds(&s, BotId(1)),
            "past the outer edge must still fail, same as a disc"
        );
    }

    #[test]
    fn gain_and_lose_effects_move_items() {
        let mut s = state();
        Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 4,
        }
        .apply(&mut s, BotId(1))
        .unwrap();
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 4);

        Effect::LoseItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 3,
        }
        .apply(&mut s, BotId(1))
        .unwrap();
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 1);
    }

    #[test]
    fn losing_items_the_bot_lacks_is_an_error() {
        let mut s = state();
        let result = Effect::LoseItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 1,
        }
        .apply(&mut s, BotId(1));
        assert!(result.is_err());
    }

    #[test]
    fn entity_at_matches_by_name_and_tile() {
        let mut s = state();
        let pos = Position::new(3., 3.);
        let cond = Condition::EntityAt {
            pos: pos.clone(),
            name: "stone-furnace".into(),
        };
        assert!(!cond.holds(&s, BotId(1)), "nothing stands there yet");

        s.create_entity(FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        });
        assert!(cond.holds(&s, BotId(1)));

        // The right tile, the wrong entity.
        let wrong_name = Condition::EntityAt {
            pos,
            name: "wooden-chest".into(),
        };
        assert!(!wrong_name.holds(&s, BotId(1)));
        // The right entity, the wrong tile.
        let wrong_tile = Condition::EntityAt {
            pos: Position::new(4., 3.),
            name: "stone-furnace".into(),
        };
        assert!(!wrong_tile.holds(&s, BotId(1)));
    }

    #[test]
    fn removing_an_entity_frees_its_tile() {
        let mut s = state();
        let pos = Position::new(3., 3.);
        Effect::CreateEntity(Box::new(FactorioEntity {
            name: "stone-furnace".into(),
            position: pos.clone(),
            ..Default::default()
        }))
        .apply(&mut s, BotId(1))
        .unwrap();
        assert!(!Condition::PositionFree { pos: pos.clone() }.holds(&s, BotId(1)));

        Effect::RemoveEntity { pos: pos.clone() }
            .apply(&mut s, BotId(1))
            .unwrap();
        assert!(Condition::PositionFree { pos }.holds(&s, BotId(1)));
    }

    #[test]
    fn researching_a_technology_satisfies_the_condition() {
        let mut s = state();
        let cond = Condition::Researched("automation".into());
        assert!(!cond.holds(&s, BotId(1)));

        Effect::Researched("automation".into())
            .apply(&mut s, BotId(1))
            .unwrap();
        assert!(cond.holds(&s, BotId(1)));
        // Research is global, not per bot, so the binding is irrelevant.
        assert!(cond.holds(&s, BotId(2)));
        assert!(!Condition::Researched("logistics".into()).holds(&s, BotId(1)));
    }

    #[test]
    fn target_position_is_none_for_craft_and_research_and_some_for_the_rest() {
        let pos = Position::new(-40.5, -48.5);
        assert_eq!(
            ActionKind::Mine {
                pos: pos.clone(),
                item: "iron-ore".into(),
                count: 1,
            }
            .target_position(),
            Some(pos.clone()),
            "a resource tile's real centre, not a floored corner"
        );
        assert_eq!(
            ActionKind::Insert {
                pos: pos.clone(),
                entity: "stone-furnace".into(),
                slot: InventorySlot::FurnaceSource,
                item: "iron-ore".into(),
                count: 1,
            }
            .target_position(),
            Some(pos.clone())
        );
        assert_eq!(
            ActionKind::Remove {
                pos: pos.clone(),
                entity: "stone-furnace".into(),
                slot: InventorySlot::FurnaceResult,
                item: "iron-plate".into(),
                count: 1,
            }
            .target_position(),
            Some(pos.clone())
        );
        assert_eq!(
            ActionKind::Place {
                entity: Box::new(FactorioEntity {
                    name: "stone-furnace".into(),
                    position: pos.clone(),
                    ..Default::default()
                }),
            }
            .target_position(),
            Some(pos)
        );
        assert_eq!(
            ActionKind::Craft {
                item: "iron-gear-wheel".into(),
                count: 1,
            }
            .target_position(),
            None,
            "crafting acts on no location"
        );
        assert_eq!(
            ActionKind::Research {
                tech: "automation".into(),
            }
            .target_position(),
            None,
            "research acts on no location"
        );
    }

    #[test]
    fn an_action_survives_a_json_round_trip() {
        // Every payload an action can carry, including the boxed core entity.
        use factorio_bot_core::serde_json;
        let action = Action {
            id: crate::ids::ActionId(7),
            kind: ActionKind::Place {
                entity: Box::new(FactorioEntity {
                    name: "stone-furnace".into(),
                    entity_type: "furnace".into(),
                    position: Position::new(3., 4.),
                    ..Default::default()
                }),
            },
            pre: vec![
                Condition::HasItem {
                    who: Actor::Role,
                    item: "stone-furnace".into(),
                    count: 1,
                },
                Condition::AtPosition {
                    who: Actor::Bound(BotId(1)),
                    pos: Position::new(3., 4.),
                    radius: 10.0,
                    min_radius: 1.5,
                },
                Condition::PositionFree {
                    pos: Position::new(3., 4.),
                },
                Condition::Researched("automation".into()),
                Condition::ResourceAvailable {
                    pos: Position::new(3., 4.),
                    item: "iron-ore".into(),
                    count: 2,
                },
                Condition::EntityAt {
                    pos: Position::new(3., 4.),
                    name: "stone-furnace".into(),
                },
            ],
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: "stone-furnace".into(),
                    count: 1,
                },
                Effect::GainItem {
                    who: Actor::Role,
                    item: "iron-plate".into(),
                    count: 1,
                },
                Effect::RemoveEntity {
                    pos: Position::new(3., 4.),
                },
                Effect::ConsumeResource {
                    pos: Position::new(3., 4.),
                    item: "iron-ore".into(),
                    count: 1,
                },
                Effect::Researched("automation".into()),
            ],
            duration: 30,
            pinned: Some(BotId(1)),
            label: "place stone-furnace".into(),
        };
        let json = serde_json::to_string(&action).expect("serialises");
        let back: Action = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, action);
    }

    #[test]
    fn conditions_render_for_error_messages() {
        let cond = Condition::HasItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 2,
        };
        assert_eq!(cond.to_string(), "has 2 coal");
    }

    #[test]
    fn an_insert_names_the_entity_and_slot_it_targets() {
        let a = ActionKind::Insert {
            pos: Position::new(3.0, 4.0),
            entity: "stone-furnace".to_string(),
            slot: InventorySlot::FurnaceSource,
            item: "iron-ore".into(),
            count: 8,
        };
        match a {
            ActionKind::Insert { entity, slot, .. } => {
                assert_eq!(entity, "stone-furnace");
                assert_eq!(slot, InventorySlot::FurnaceSource);
            }
            _ => panic!("expected Insert"),
        }
    }

    /// Pins the exact strings sent to the game. These are Factorio 2.1 names,
    /// read from `defines.inventory` in `runtime-api.json`; the executor has
    /// the test that checks them against the game's own table.
    #[test]
    fn every_slot_maps_to_its_2_1_defines_name() {
        assert_eq!(InventorySlot::Chest.defines_key(), "chest");
        assert_eq!(InventorySlot::Fuel.defines_key(), "fuel");
        assert_eq!(InventorySlot::LabInput.defines_key(), "lab_input");
        // 2.0 unified furnaces and assemblers under `crafter_*`.
        assert_eq!(InventorySlot::FurnaceSource.defines_key(), "crafter_input");
        assert_eq!(InventorySlot::AssemblerInput.defines_key(), "crafter_input");
        assert_eq!(InventorySlot::FurnaceResult.defines_key(), "crafter_output");
        assert_eq!(
            InventorySlot::AssemblerOutput.defines_key(),
            "crafter_output"
        );
    }

    #[test]
    fn no_slot_still_uses_a_removed_pre_2_0_name() {
        // These four exist in `mods/BotBridge/control.lua`'s dead
        // `inventory_type_name` and nowhere in the 2.1 game.
        let gone = [
            "furnace_source",
            "furnace_result",
            "assembling_machine_input",
            "assembling_machine_output",
        ];
        for slot in InventorySlot::ALL {
            let key = slot.defines_key();
            assert!(!gone.contains(&key), "{slot:?} uses removed name {key}");
        }
    }

    #[test]
    fn all_lists_every_variant() {
        // Guards the exhaustiveness the executor's defines check relies on.
        let mut keys: Vec<&str> = InventorySlot::ALL.iter().map(|s| s.defines_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(InventorySlot::ALL.len(), 7);
        assert_eq!(
            keys,
            vec![
                "chest",
                "crafter_input",
                "crafter_output",
                "fuel",
                "lab_input"
            ]
        );
    }

    // ---- set_recipe ------------------------------------------------------

    /// An assembling machine at `pos`, with whatever recipe it already carries.
    fn machine(s: &mut PlanState, pos: &Position, recipe: Option<&str>) {
        s.create_entity(FactorioEntity {
            name: "assembling-machine-1".into(),
            entity_type: "assembling-machine".into(),
            position: pos.clone(),
            recipe: recipe.map(str::to_string),
            ..Default::default()
        });
    }

    /// **A machine with no recipe on it is not finished**, and this is the
    /// condition that says so. Every other check a stage-2 cell makes --
    /// placed, powered, fed -- passes for a machine that produces nothing.
    #[test]
    fn a_machine_with_no_recipe_does_not_satisfy_recipe_set() {
        let mut s = state();
        let pos = Position::new(12.5, 8.5);
        machine(&mut s, &pos, None);
        let cond = Condition::RecipeSet {
            pos: pos.clone(),
            recipe: "automation-science-pack".into(),
        };
        assert!(
            !cond.holds(&s, BotId(1)),
            "a machine the game reports no recipe for is not set to one"
        );
    }

    /// The world's own reading counts: a machine that already carries the
    /// recipe satisfies the condition with no action in the plan at all.
    #[test]
    fn a_machine_already_carrying_the_recipe_satisfies_it() {
        let mut s = state();
        let pos = Position::new(12.5, 8.5);
        machine(&mut s, &pos, Some("automation-science-pack"));
        assert!(
            Condition::RecipeSet {
                pos,
                recipe: "automation-science-pack".into(),
            }
            .holds(&s, BotId(1))
        );
    }

    /// The recipe is compared, not merely its presence. A machine set to gears
    /// is not a machine set to science packs, and a condition that answered
    /// yes to both would call a wrongly-configured cell finished.
    #[test]
    fn a_machine_carrying_another_recipe_does_not_satisfy_it() {
        let mut s = state();
        let pos = Position::new(12.5, 8.5);
        machine(&mut s, &pos, Some("iron-gear-wheel"));
        assert!(
            !Condition::RecipeSet {
                pos,
                recipe: "automation-science-pack".into(),
            }
            .holds(&s, BotId(1))
        );
    }

    /// No machine, no recipe. Deliberately `false` rather than a panic and
    /// deliberately not "vacuously true": an empty tile is exactly the state a
    /// cell is in before its assembler is placed, and reporting the recipe set
    /// there would let the plan's checks pass on ground with nothing on it.
    #[test]
    fn an_empty_tile_does_not_satisfy_recipe_set() {
        let s = state();
        assert!(
            !Condition::RecipeSet {
                pos: Position::new(12.5, 8.5),
                recipe: "automation-science-pack".into(),
            }
            .holds(&s, BotId(1))
        );
    }

    /// Applying the effect makes the condition hold -- the pairing the plan
    /// rests on. Without it a method could emit the action, the state would
    /// never move, and expansion would emit it again for ever.
    #[test]
    fn setting_a_recipe_makes_the_condition_hold() {
        let mut s = state();
        let pos = Position::new(12.5, 8.5);
        machine(&mut s, &pos, None);
        let eff = Effect::SetRecipe {
            pos: pos.clone(),
            recipe: "automation-science-pack".into(),
        };
        eff.apply(&mut s, BotId(1)).expect("a machine is there");
        assert!(
            Condition::RecipeSet {
                pos,
                recipe: "automation-science-pack".into(),
            }
            .holds(&s, BotId(1)),
            "the overlay must carry what the effect claims"
        );
    }

    /// **Idempotent, and its neighbours are the counter-examples.** Applying
    /// `GainItem` twice gains twice; applying this twice lands on the same
    /// state. That is what makes the matching dispatch safe to re-run under
    /// `recover.rs`'s tier 1, where a repeated `Insert` moves a second batch.
    #[test]
    fn setting_the_same_recipe_twice_changes_nothing() {
        let mut s = state();
        let pos = Position::new(12.5, 8.5);
        machine(&mut s, &pos, None);
        let eff = Effect::SetRecipe {
            pos: pos.clone(),
            recipe: "automation-science-pack".into(),
        };
        eff.apply(&mut s, BotId(1)).expect("a machine is there");
        let once = s.entity_at(&pos).expect("the machine");
        eff.apply(&mut s, BotId(1)).expect("still there");
        let twice = s.entity_at(&pos).expect("the machine");
        assert_eq!(once, twice, "a second application is a no-op");
    }

    /// A recipe replaces the one before it rather than being added beside it:
    /// a machine has exactly one.
    #[test]
    fn setting_a_recipe_replaces_the_one_before_it() {
        let mut s = state();
        let pos = Position::new(12.5, 8.5);
        machine(&mut s, &pos, Some("iron-gear-wheel"));
        Effect::SetRecipe {
            pos: pos.clone(),
            recipe: "automation-science-pack".into(),
        }
        .apply(&mut s, BotId(1))
        .expect("a machine is there");
        assert_eq!(
            s.entity_at(&pos).expect("the machine").recipe.as_deref(),
            Some("automation-science-pack")
        );
        assert!(
            !Condition::RecipeSet {
                pos,
                recipe: "iron-gear-wheel".into(),
            }
            .holds(&s, BotId(1)),
            "the old recipe is gone, not kept alongside"
        );
    }

    /// Setting a recipe on empty ground is a **fault in the plan**, not a
    /// silent no-op. A method that emits this without ordering it after the
    /// placement has written a plan whose later checks would pass against a
    /// machine nobody built, and the error names the tile so the method that
    /// wrote it can be found.
    #[test]
    fn setting_a_recipe_where_there_is_no_machine_is_an_error() {
        let mut s = state();
        let err = Effect::SetRecipe {
            pos: Position::new(12.5, 8.5),
            recipe: "automation-science-pack".into(),
        }
        .apply(&mut s, BotId(1))
        .expect_err("nothing stands there");
        let text = err.to_string();
        assert!(
            text.contains("automation-science-pack"),
            "the refusal names the recipe, got {text}"
        );
        assert!(
            text.contains("12.5") && text.contains("8.5"),
            "and the tile it was aimed at, got {text}"
        );
    }

    /// The ordering half. `ActionNetwork::infer_edges` draws an edge from an
    /// effect to a condition it satisfies, so a `SetRecipe` must satisfy the
    /// `RecipeSet` for the same machine and the same recipe -- otherwise an
    /// inserter loading a machine could be scheduled before the recipe that
    /// decides what goes in it.
    #[test]
    fn a_set_recipe_effect_satisfies_the_matching_condition() {
        let pos = Position::new(12.5, 8.5);
        let eff = Effect::SetRecipe {
            pos: pos.clone(),
            recipe: "automation-science-pack".into(),
        };
        assert!(eff.satisfies(&Condition::RecipeSet {
            pos,
            recipe: "automation-science-pack".into(),
        }));
    }

    /// ...and satisfies neither another machine's nor another recipe's.
    /// An edge drawn on a mismatch would order two unrelated actions and,
    /// worse, report a condition as achievable that nothing in the plan
    /// achieves.
    #[test]
    fn a_set_recipe_effect_satisfies_nothing_else() {
        let eff = Effect::SetRecipe {
            pos: Position::new(12.5, 8.5),
            recipe: "automation-science-pack".into(),
        };
        assert!(
            !eff.satisfies(&Condition::RecipeSet {
                pos: Position::new(20.5, 8.5),
                recipe: "automation-science-pack".into(),
            }),
            "another machine's recipe is not this one"
        );
        assert!(
            !eff.satisfies(&Condition::RecipeSet {
                pos: Position::new(12.5, 8.5),
                recipe: "iron-gear-wheel".into(),
            }),
            "another recipe on this machine is not this one"
        );
        assert!(
            !eff.satisfies(&Condition::EntityAt {
                pos: Position::new(12.5, 8.5),
                name: "assembling-machine-1".into(),
            }),
            "setting a recipe does not build the machine"
        );
    }

    /// `target_position` is what the executor's log and `recover.rs` read to
    /// ask "where did this act". A `SetRecipe` acts at the machine, and
    /// answering `None` would put it in the same class as `Craft` -- an action
    /// that touches no tile.
    #[test]
    fn a_set_recipe_acts_at_the_machine() {
        let pos = Position::new(12.5, 8.5);
        let kind = ActionKind::SetRecipe {
            pos: pos.clone(),
            entity: "assembling-machine-1".into(),
            recipe: "automation-science-pack".into(),
        };
        assert_eq!(kind.target_position(), Some(pos));
    }

    #[test]
    fn inventory_slots_order_deterministically() {
        let mut v = vec![
            InventorySlot::FurnaceResult,
            InventorySlot::Chest,
            InventorySlot::FurnaceSource,
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                InventorySlot::Chest,
                InventorySlot::FurnaceSource,
                InventorySlot::FurnaceResult
            ]
        );
    }

    /// `Condition::AreaFree` asks about the ground the building will really
    /// stand on, which depends on which way it faces.
    ///
    /// A boiler is 3x2 tiles facing north and 2x3 facing east. The pole below
    /// sits 1.05 tiles east of the site: inside the north-facing box (half
    /// width 1.289) and clear of the east-facing one (half width 0.789). So
    /// the same `pos` and the same `entity` must answer differently at the two
    /// directions, and a check that ignored the direction would answer the
    /// same both times.
    ///
    /// This is the fluid-connection trap in its structural form: nothing else
    /// in the plan notices a boiler checked against the wrong footprint until
    /// the game refuses the build.
    #[test]
    fn an_area_free_check_turns_with_the_building() {
        let mut s = state();
        let site = Position::new(100.5, 100.);
        s.create_entity(FactorioEntity {
            name: "small-electric-pole".into(),
            entity_type: "electric-pole".into(),
            position: Position::new(site.x() + 1.05, site.y()),
            ..Default::default()
        });
        let facing = |direction: u8| Condition::AreaFree {
            pos: site.clone(),
            entity: "boiler".into(),
            direction,
        };
        assert!(
            !facing(0).holds(&s, BotId(1)),
            "a north-facing boiler is 1.289 tiles wide each way and reaches the pole"
        );
        assert!(
            facing(4).holds(&s, BotId(1)),
            "an east-facing boiler is only 0.789 wide each way and clears it"
        );
        // A half-diagonal names no rotation and therefore no footprint.
        assert!(
            !facing(2).holds(&s, BotId(1)),
            "no building stands on a half-diagonal, so there is no box to compare"
        );
    }
}
