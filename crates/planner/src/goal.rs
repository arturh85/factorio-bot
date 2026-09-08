//! What we want, stated declaratively and without reference to any bot.

use crate::ids::{BotId, ItemId};

/// A recipe's own name, as the game's recipe table keys it.
///
/// A plain `String` alias like [`ItemId`], and deliberately **not** the same
/// alias: a recipe name and an item name coincide for 268 of 662 vanilla
/// recipes and differ for the other 394 (`crate::products`), so a signature
/// that said `ItemId` where it meant a recipe would be right often enough to
/// be believed and wrong wherever it mattered.
pub type RecipeName = String;

/// Renders the ` via <recipe>` suffix, or nothing at all when no recipe was
/// named.
///
/// One helper for both goal kinds. It writes **nothing** for `None`, which is
/// what keeps every existing goal's rendering byte-identical -- and those
/// renderings are quoted verbatim in `PlannerError::NoApplicableMethod`.
fn write_via(f: &mut std::fmt::Formatter<'_>, via: &Option<RecipeName>) -> std::fmt::Result {
    match via {
        Some(recipe) => write!(f, " via {}", recipe),
        None => Ok(()),
    }
}
use factorio_bot_core::types::Position;
use serde::{Deserialize, Serialize};

/// Who must end up holding the items.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Holder {
    /// Satisfied by the sum across every bot. This is what makes multi-bot
    /// gathering parallel: the count can be split into independent per-bot
    /// chains that never coordinate.
    Anyone,
    /// A caller's instruction: this bot must end up holding the items. The
    /// scheduler honours it — the chain this goal opens is owned by this bot.
    Bot(BotId),
    /// One share of a split, and — the part that is easy to miss — a claim
    /// that the holding ends up in **one** inventory.
    ///
    /// Sized against this bot's starting inventory, and — since 2026-09-02 —
    /// also committing that bot to run it: the driver opens a chain over a
    /// share's subtree, welding its actions to a single runner, and gives
    /// that chain **this bot as its owner**, exactly as a `Holder::Bot` would.
    ///
    /// It did not always. Before 2026-09-02 a share named a bot only to size
    /// itself and left who ran it to the scheduler, on the theory that bots
    /// are interchangeable so it does not matter. That theory holds only
    /// while it holds: a live four-bot run left the roster unequal after two
    /// gathering milestones (8 / 8 / 4 iron-ore), the scheduler bound a
    /// share's chain to whichever bot was cheapest rather than the one its
    /// bill was sized against, and a downstream action needing the full count
    /// failed for a bot that never held it — naming a different bot on each
    /// of two crashes of the same run. See
    /// `docs/superpowers/notes/2026-09-02-rung-3-4-findings.md` for the full
    /// diagnosis and the alternatives weighed. The cost of binding is real
    /// and was measured on that same run: two subtrees that ran concurrently
    /// on two bots for 22,072 ticks now serialise onto one. Accepted anyway —
    /// a slower correct plan beats a faster crashing one. The fix that
    /// removes the trade-off rather than choosing a side landed on
    /// 2026-09-05, and it is a decomposition, not a relaxation: `Researched`
    /// deals its pack bill, its trigger prerequisite and its labs across the
    /// roster as `Holder::Share(b)` goals inside `Step::Owned { whose:
    /// Holder::Share(b) }` blocks, so every share is still sized against and
    /// bound to one bot — this rule, unchanged — and the shares run in
    /// parallel because they are different bots' shares, not because the
    /// binding was loosened.
    ///
    /// The welding is not decoration. A share is what `SplitAcrossBots` hands
    /// a bot to mine, smelt and craft on its own, and what `Researched` asks
    /// for because "the research is one action reading one bot's inventory".
    /// Both statements are only true while something keeps the share's
    /// producers and its consumers together: a smelt's ore, coal and furnace
    /// feed three separate actions, so nothing in the network holds them in
    /// one pair of hands, and left unwelded the scheduler mines the coal onto
    /// one bot and loads the furnace from another.
    ///
    /// Naming a bot here is also how the driver keeps each share's simulated
    /// inventory separate. That the *sizing* is trustworthy no longer rests on
    /// bots starting interchangeable — binding the owner makes it true by
    /// construction, for a share and its whole subtree alike. What is left is
    /// narrower than an assumption: `expand`'s own `chain_actor` argument is
    /// the bot a goal naming none — including `Researched`'s own top-level
    /// shortfall check, before it descends into `Share(chain_actor)`
    /// subgoals — is simulated against, and that choice is no longer inert
    /// either, since it feeds straight into the binding above. See the
    /// `ExpansionCtx` doc (`crates/planner/src/method/mod.rs`) and
    /// `crates/executor/src/recover.rs`'s tier 2 for why callers pin it
    /// deterministically rather than treat it as arbitrary.
    Share(BotId),
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Holder::Anyone => write!(f, "anyone"),
            Holder::Bot(id) => write!(f, "{}", id),
            Holder::Share(id) => write!(f, "a share sized for {}", id),
        }
    }
}

/// Where a block goes.
///
/// `Goal::Built` used to carry a bare `anchor: Position`, which made siting
/// the caller's problem: a 37-entity `MinerLine` was attempted at three
/// anchors and never got past planning, because one obstructed tile anywhere
/// along its 21-tile belt run makes the whole block infeasible. On real
/// terrain that is the normal case.
///
/// Derives `PartialEq` but not `Eq`/`Ord`, matching [`Goal`] itself: both
/// carry a [`Position`], whose `f64` fields cannot implement either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Site {
    /// This exact anchor, or refuse. The pre-siting behaviour, kept because a
    /// caller that has already chosen must still be able to say so — and
    /// because every existing test and script says it.
    At(Position),
    /// Search outward from here.
    Near(Position),
    /// Search outward from the roster's centroid.
    Anywhere,
    /// **A resolved anchor, recorded. Authoritative — recovery does not
    /// override it, and no search runs.**
    ///
    /// The owner's ruling of 2026-09-07, and the fix for a defect that had no
    /// workaround: `resolve_site` calls `recover_anchor` first and
    /// unconditionally, and recovery trusts an anchor once **two** of a
    /// blueprint's entities stand at the right relative offsets. Blocks that
    /// share a sub-layout therefore recover into each other —
    /// `ElectricSmelter` finds 21 of its 28 entities inside a standing
    /// `FurnaceLine` (`crates/core/tests/recovery_crosstalk_probe.rs`) — so a
    /// second block planned on a map that already carries one is sited *inside
    /// it*, and no threshold can fix that: 21 of 28 is 75%, and any threshold
    /// loose enough to resume a genuinely half-built block accepts it.
    ///
    /// # Why this is not [`Site::At`]
    ///
    /// `At` is a **hint** and is deliberately outranked by recovery — see
    /// `resolve_site_prefers_the_recovered_anchor_over_an_explicit_site_at`.
    /// A caller naming an anchor may be working from a stale script, and
    /// standing entities are the better evidence of where a block actually is;
    /// letting a stale `At` win starts a second half-block with no error.
    ///
    /// `Anchored` is a **recorded fact**: this anchor was *resolved by siting*
    /// and handed back, so it is not a guess that standing entities could
    /// improve on — it is the answer standing entities were being consulted to
    /// reconstruct. Recovery reconstructs an anchor by geometry precisely
    /// because nothing had recorded one; once one is recorded, geometry is the
    /// weaker evidence and must not override it.
    ///
    /// The two variants are kept apart rather than merged because they encode
    /// different claims, and collapsing them would either make every stale
    /// caller anchor authoritative or leave persistence impossible.
    Anchored(Position),
    /// **One pitch along from a block already placed.** The other half of
    /// anchor persistence: `Anchored` says *which* anchor a block has, and this
    /// says where the NEXT one goes.
    ///
    /// `of` is the anchor of the block to sit beside — normally one a caller
    /// recorded from an earlier plan — and `steps` counts whole pitches along
    /// each axis, so `(1, 0)` is "immediately east of it, one block along".
    ///
    /// # Why the blueprint's own pitch and not its bounding box
    ///
    /// The pitch is `snap-to-grid`, the author's statement of the period at
    /// which the design tiles, and **it cannot be derived from the entities**.
    /// `MinerLine` spans 5 tiles of x and declares a pitch of 7: the two extra
    /// tiles are room the author deliberately left beside the block, which a
    /// bounding box cannot see because nothing was drawn there. Tiling by the
    /// extent would pack the blocks 5 apart and consume that space.
    ///
    /// So a block with no declared pitch cannot answer this question and is
    /// refused by name rather than tiled at its extent, which would silently
    /// discard whatever the author was reserving.
    ///
    /// # Why it outranks recovery, like [`Site::Anchored`]
    ///
    /// It is the answer to "this is a NEW block, put it *there*", which is the
    /// question that had no expression at all. Consulting standing geometry
    /// first would defeat it exactly as it defeats `Site::At`: on a map where
    /// `FurnaceLine` stands, `ElectricSmelter` matches 21 of its 28 entities
    /// inside it, so recovery hands the second block the first block's anchor.
    ///
    /// It stays replan-stable without recovery because it is a pure function
    /// of `of`, `steps` and the blueprint's own declared pitch — none of which
    /// move while a run is going.
    Beside {
        /// The anchor of the block to sit beside.
        of: Position,
        /// Whole pitches along x and y. `(1, 0)` is one block east.
        steps: (i32, i32),
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    Have {
        item: ItemId,
        count: u32,
        whose: Holder,
        /// **Which recipe**, when asking for the product is ambiguous.
        ///
        /// See [`Goal::Produced::via`] — one meaning, two goal kinds, and
        /// [`crate::method::have::demand`] reads it for both.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<RecipeName>,
    },
    Researched(String),
    /// Cause `count` of `item` to come into existence.
    ///
    /// Distinct from [`Goal::Have`], which is satisfied by what a bot already
    /// holds. `Produced` never subtracts the current inventory: a bot carrying
    /// six labs has not *crafted* one, and a Factorio 2.0 `craft-item` trigger
    /// fires on the act of production, not on possession.
    ///
    /// `unlocks` names a technology this production triggers, if any. It rides
    /// on the goal because the resulting `Effect::Researched` has to land on
    /// whichever action ends up producing the item -- craft, smelt or mine --
    /// and only the producing method knows which action that is. A method
    /// cannot reach into its subgoals' actions to attach it afterwards; it
    /// never sees their ids, by design.
    Produced {
        item: ItemId,
        count: u32,
        /// Whose inventory the ingredients are sized against -- the same
        /// meaning as [`Goal::Have`]'s, so one helper can serve both and the
        /// two cannot drift apart.
        whose: Holder,
        unlocks: Option<String>,
        /// **Which recipe** is to make it, by recipe name, when asking for
        /// the *product* does not say.
        ///
        /// # Absent is not a value
        ///
        /// `None` is *the caller did not choose*, and it is the only shape
        /// every existing goal, script and baseline has. It means exactly
        /// what it meant before this field existed: the planner picks, and
        /// [`crate::products::ProductIndex::sole_recipe_producing`] refuses
        /// with [`crate::products::ProductRefusal::Ambiguous`] when more than
        /// one admitted recipe produces the product. `Some(name)` is a
        /// *choice*, and a wrong one is refused by name rather than quietly
        /// replaced with the planner's own pick — see
        /// [`crate::method::named_recipe_refusal`], which runs **before any
        /// method is asked** for the same reason the fluid guard beside it
        /// does: a goal whose recipe does not produce its product cannot be
        /// stated, so nothing should get the chance to satisfy it some other
        /// way.
        ///
        /// # Why this is a qualifier and not a goal kind
        ///
        /// "petroleum gas via basic-oil-processing" is the same *want* as
        /// "petroleum gas"; what differs is how it is to be met. A tenth
        /// `Goal` variant would have to restate the item, the count, the
        /// holder and the unlock, and every exhaustive match over `Goal`
        /// would gain an arm that duplicated this one.
        ///
        /// # It is a recipe name, not a machine name
        ///
        /// The machine follows from the recipe's category
        /// ([`crate::method::machine::MachineTable`]). Naming the machine
        /// instead would leave `advanced-oil-processing` and
        /// `basic-oil-processing` — the choice this field exists for —
        /// indistinguishable, since both run in an `oil-refinery`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<RecipeName>,
    },
    /// A standing arrangement that yields `item` at `per_minute` without
    /// further intervention.
    ///
    /// **Satisfied by structure, not by observation.** See
    /// [`crate::method::have::holds`]: this asks whether machines capable of
    /// the rate stand, stand on the resource they consume, and *deliver into
    /// one another*. It does **not** ask whether anything is coming out. A
    /// drill with an empty fuel slot reads as a drill and a furnace with a
    /// full output reads as a furnace, because [`crate::state::PlanState`]
    /// models neither. The observation lives in `supervisor.witness` — a
    /// milestone that dispatches no actions at all and asserts a terminal
    /// machine's output inventory rose anyway — and nowhere in this crate.
    ///
    /// # `per_minute` is an integer on purpose
    ///
    /// It was `rate: f64` until stage 1 of the starter factory. The machine
    /// count is `ceil(target / per-machine)`, and a target that is exactly one
    /// machine's output — 15 iron plates a minute against a burner drill's
    /// 15 — is a ceiling sitting on a representation boundary. This crate's
    /// defining constraint is byte-identical plans for identical inputs, so
    /// the numerator is an integer and
    /// [`crate::method::produce`] does the whole division in integer ticks.
    /// Rational arithmetic over a recipe matrix is stage 3's problem, where a
    /// cell consumes its own output and one division no longer suffices.
    Producing {
        item: ItemId,
        per_minute: u32,
    },
    /// `item` comes out of **machines** at `per_minute` or better, for
    /// `window_ticks` continuously, with nothing a bot carried able to explain
    /// it.
    ///
    /// The fifth kind, and the first one in this vocabulary that means *keep
    /// this true*. Design note:
    /// `docs/superpowers/notes/2026-09-06-standing-goals.md`.
    ///
    /// # Why it is not [`Goal::Producing`] with a window bolted on
    ///
    /// `Producing` is satisfied **structurally** — enough machines stand on
    /// the right ore, delivering into one another — and its own doc admits
    /// what that cannot see: a drill whose fuel ran out, a furnace whose
    /// output backed up, a patch mined out from under a drill. It is a claim
    /// about *capacity*, and capacity has never been the thing that failed in
    /// a measured run here. Supply is: every run's rate table reads
    /// `roster-fed`, and every plateau classifies as *input ran out*. Adding a
    /// window to `Producing` would make one goal answer `Some(true)`
    /// structurally *and* be measured over a window, which is exactly the
    /// ambiguity the plateau came from.
    ///
    /// # This goal is a contract between two halves, and neither half is it
    ///
    /// The planner owes a **structure with no bot in the loop**: every input
    /// of every counted machine has a standing deliverer, checkable at plan
    /// time and refusable by name. The record owes a **measurement over a
    /// window**: machine counters (`produced` / `produced_source`, not
    /// `force.production.made`, which cannot tell a hand craft from a machine)
    /// plus an idle roster, over the trailing `window_ticks` and a stated
    /// lead-in before it. `tools/run_analysis.py`'s `sustained_rate` is that
    /// half.
    ///
    /// A structure with no observation is `Producing`, which has been shown
    /// standing and dead. An observation with no structural requirement is the
    /// green witness, which has already passed on a hand-charged cell.
    ///
    /// # `window_ticks` has no default
    ///
    /// The same refusal as `supervisor.witness`'s `within_ticks`: the window
    /// is the number that decides what a failure means, and a library that
    /// guessed it would hand back a verdict nobody derived. Ticks, because
    /// they are the only clock the record, the planner and the mod share.
    ///
    /// See [`crate::method::have::holds`] for why this answers `None`.
    Sustain {
        item: ItemId,
        per_minute: u32,
        /// The trailing window the rate must hold over, in game ticks.
        window_ticks: crate::ids::Ticks,
    },
    /// Cause something to be *extracted* from `entity` -- a resource a hand
    /// cannot work -- by a machine standing on it: a pumpjack on a crude-oil
    /// well, a drill on uranium ore with acid piped in.
    ///
    /// Distinct from [`Goal::Produced`], which names the *product*: a
    /// Factorio 2.0 `mine-entity` trigger fires on mining a named entity and
    /// does not care what comes out or where it goes, and for crude oil the
    /// product is a fluid no inventory can hold. `unlocks` rides on it for
    /// the same reason it rides on `Produced` -- only the method that stands
    /// the machine up knows which action to hang `Effect::Researched` on.
    ///
    /// No method claims this yet: [`crate::method::extract::Extract`] refuses
    /// it by name, saying which prerequisite is missing. It is a goal rather
    /// than an error so that the refusal is asked of the world *after* the
    /// technology's own prerequisites have been planned, and so that the
    /// method that eventually sites a pumpjack has a goal to claim.
    Extracted {
        entity: String,
        unlocks: Option<String>,
    },
    /// The next rung above [`Goal::Extracted`]: an extractor stands on a well
    /// of `entity` **and what it pumps has somewhere to go** -- a storage tank
    /// sited for the whole field, with pipe between the two.
    ///
    /// # Why this is a goal of its own and not a better `Extracted`
    ///
    /// `Extracted` names an *act* the game rewards: Factorio 2.0's
    /// `mine-entity` trigger fires the moment a pumpjack works a well, and a
    /// pumpjack with nothing connected does fire it -- it fills its own output
    /// fluidbox and stops, which is several extractions after the first. So
    /// `Extracted` is honestly satisfied by a machine alone, and widening it
    /// to demand a tank would make `researched:oil-processing` refuse on maps
    /// where it currently plans end to end.
    ///
    /// This one names a *standing arrangement*: after it, crude exists
    /// somewhere the model can point at. That is the precondition for
    /// everything above -- **no character inventory can hold a fluid**
    /// (`crate::substance`, and
    /// `docs/superpowers/notes/2026-09-06-a-fluid-is-not-an-item.md`), so
    /// until a tank stands there is nowhere for crude to *be*, and the trunk
    /// to a second tank at the base has no near end to start from.
    ///
    /// `unlocks` rides on it for the same reason it rides on `Extracted` and
    /// `Produced`: only the method that stands the machine up knows which
    /// action to hang the `Effect::Researched` on.
    ///
    /// The topology is the owner's, recorded in
    /// `docs/superpowers/notes/2026-09-06-how-oil-is-actually-played.md`:
    /// pumpjacks pipe to one tank at the patch, and one long trunk crosses to
    /// a second tank at the base. **This goal is the left-hand half.**
    Gathered {
        entity: String,
        unlocks: Option<String>,
    },
    /// This blueprint stands at this anchor.
    ///
    /// **Shaped to survive replanning.** Expanding it means *the entities not
    /// yet standing*, re-derived against the world every time, so it is
    /// verifiable rather than a remembered instruction, and building it twice
    /// is a no-op. Every other goal here is item-shaped for the same reason:
    /// this planner replans constantly, and a goal naming particular machines
    /// would be stale the moment a replan sited a different one.
    Built {
        /// The blueprint string, decoded on each expansion.
        blueprint: String,
        /// Where the block goes: a fixed anchor, a hint, or nothing.
        site: Site,
    },
    /// The ground within `radius` of `around` has been *looked at*.
    ///
    /// The exploration primitive, and deliberately the smallest one that is
    /// honest. Every other goal here names a thing to end up with; this one
    /// names ground to end up having seen, because the refusal it answers --
    /// [`crate::PlannerError::NotCharted`] -- is the one refusal in the crate
    /// that no amount of crafting, research or building can clear. A plan
    /// that needs copper it has never seen cannot want copper harder; it has
    /// to send somebody to look.
    ///
    /// # Why a disc and not a resource name
    ///
    /// "Chart me some crude oil" is not a goal a planner can honestly claim:
    /// whether a well exists out there is exactly the thing nobody knows
    /// until the ground is charted, so a method claiming it would be
    /// promising an outcome it cannot deliver and would have no terminating
    /// condition when the map genuinely has none. A disc is checkable before
    /// and after -- [`crate::state::PlanState::charting`] answers it with the
    /// same seventeen probes `NotCharted` already reports against -- so the
    /// goal is satisfied by an act the bots actually performed rather than by
    /// the map's luck.
    ///
    /// # It is satisfied by *charting*, not by finding
    ///
    /// A survey that walks the whole disc and comes back having seen no
    /// copper has **succeeded**. That is the correct answer to "go and look",
    /// and it converts `NotCharted` ("unexplored, so unknown") into the
    /// genuinely different `NoApplicableMethod` ("looked, and it is not
    /// there") -- which is the whole value of the distinction piece 1 of
    /// `docs/superpowers/specs/2026-09-04-exploration-design.md` drew.
    Charted {
        around: Position,
        radius: f64,
    },
    All(Vec<Goal>),
}

impl std::fmt::Display for Goal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Goal::Have {
                item,
                count,
                whose,
                via,
            } => {
                write!(f, "have {} {} ({})", count, item, whose)?;
                write_via(f, via)
            }
            Goal::Researched(tech) => write!(f, "research {}", tech),
            Goal::Produced {
                item,
                count,
                unlocks,
                via,
                ..
            } => {
                match unlocks {
                    Some(tech) => write!(f, "produce {} {} to unlock {}", count, item, tech)?,
                    None => write!(f, "produce {} {}", count, item)?,
                }
                write_via(f, via)
            }
            Goal::Producing { item, per_minute } => {
                write!(f, "produce {} {}/min", per_minute, item)
            }
            Goal::Sustain {
                item,
                per_minute,
                window_ticks,
            } => write!(
                f,
                "sustain {} {}/min over {} ticks",
                per_minute, item, window_ticks
            ),
            Goal::Extracted { entity, unlocks } => match unlocks {
                Some(tech) => write!(f, "extract from {} to unlock {}", entity, tech),
                None => write!(f, "extract from {}", entity),
            },
            Goal::Gathered { entity, unlocks } => match unlocks {
                Some(tech) => write!(f, "gather {} into a tank to unlock {}", entity, tech),
                None => write!(f, "gather {} into a tank", entity),
            },
            Goal::Built { blueprint, site } => {
                let where_ = match site {
                    Site::At(p) => format!("at {p}"),
                    Site::Near(p) => format!("near {p}"),
                    Site::Anywhere => "anywhere".to_string(),
                    Site::Anchored(p) => format!("at its recorded anchor {p}"),
                    Site::Beside { of, steps } => {
                        format!(
                            "{} pitch(es) from the block at {of}",
                            format_args!("({}, {})", steps.0, steps.1)
                        )
                    }
                };
                write!(f, "build {}-byte block {}", blueprint.len(), where_)
            }
            Goal::Charted { around, radius } => {
                write!(f, "chart within {:.0} tiles of {}", radius, around)
            }
            Goal::All(goals) => write!(f, "all of {} goals", goals.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;

    #[test]
    fn holders_distinguish_anyone_from_a_named_bot() {
        assert_ne!(Holder::Anyone, Holder::Bot(BotId(1)));
        assert_eq!(Holder::Bot(BotId(1)), Holder::Bot(BotId(1)));
    }

    #[test]
    fn have_goals_compare_by_all_three_fields() {
        let a = Goal::Have {
            item: "iron-plate".into(),
            count: 2,
            whose: Holder::Anyone,
            via: None,
        };
        let b = Goal::Have {
            item: "iron-plate".into(),
            count: 2,
            whose: Holder::Anyone,
            via: None,
        };
        let c = Goal::Have {
            item: "iron-plate".into(),
            count: 3,
            whose: Holder::Anyone,
            via: None,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// **The `via` suffix, which nothing asserted until a mutation went green.**
    ///
    /// `Goal::Display`'s output is quoted verbatim into
    /// `PlannerError::NoApplicableMethod`, so a goal whose recipe vanished
    /// from its rendering would refuse under a description of a *different*
    /// goal. Found by breaking `write_via` to emit nothing for `Some` and
    /// watching the whole suite stay green: the Lua binding's `render_goal`
    /// was pinned and this was not.
    ///
    /// Paired with the unqualified rendering, which must be **unchanged** --
    /// that half is what every pre-existing diagnostic depends on.
    #[test]
    fn a_named_recipe_appears_in_the_rendering_and_absence_changes_nothing() {
        let have = |via: Option<&str>| Goal::Have {
            item: "petroleum-gas".into(),
            count: 100,
            whose: Holder::Anyone,
            via: via.map(str::to_string),
        };
        assert_eq!(have(None).to_string(), "have 100 petroleum-gas (anyone)");
        assert_eq!(
            have(Some("basic-oil-processing")).to_string(),
            "have 100 petroleum-gas (anyone) via basic-oil-processing"
        );

        let produced = |via: Option<&str>, unlocks: Option<&str>| Goal::Produced {
            item: "iron-plate".into(),
            count: 50,
            whose: Holder::Anyone,
            unlocks: unlocks.map(str::to_string),
            via: via.map(str::to_string),
        };
        assert_eq!(produced(None, None).to_string(), "produce 50 iron-plate");
        assert_eq!(
            produced(Some("iron-plate"), None).to_string(),
            "produce 50 iron-plate via iron-plate"
        );
        // Both qualifiers at once, so neither swallows the other.
        assert_eq!(
            produced(Some("iron-plate"), Some("steam-power")).to_string(),
            "produce 50 iron-plate to unlock steam-power via iron-plate"
        );
    }

    /// A named recipe survives JSON, and a document written before the field
    /// existed still parses -- which is what `--goal-json` documents already
    /// on disk depend on.
    #[test]
    fn a_named_recipe_survives_json_and_an_older_document_still_parses() {
        use factorio_bot_core::serde_json;
        let goal = Goal::Produced {
            item: "petroleum-gas".into(),
            count: 100,
            whose: Holder::Anyone,
            unlocks: None,
            via: Some("basic-oil-processing".into()),
        };
        let json = serde_json::to_string(&goal).expect("serialises");
        assert!(json.contains("basic-oil-processing"), "{json}");
        assert_eq!(
            serde_json::from_str::<Goal>(&json).expect("deserialises"),
            goal
        );

        // `skip_serializing_if`: an unqualified goal writes no `via` key at
        // all, so a document round-tripped through this crate is byte-for-byte
        // what it was before the field existed.
        let plain = Goal::Have {
            item: "iron-plate".into(),
            count: 4,
            whose: Holder::Anyone,
            via: None,
        };
        let json = serde_json::to_string(&plain).expect("serialises");
        assert!(!json.contains("via"), "{json}");

        // And `serde(default)`: a hand-written document with no `via` is
        // *absent*, not malformed.
        let older: Goal =
            serde_json::from_str(r#"{"Have":{"item":"iron-plate","count":4,"whose":"Anyone"}}"#)
                .expect("a document written before the field parses");
        assert_eq!(older, plain);
    }

    #[test]
    fn goals_survive_a_json_round_trip() {
        // The next increment ships schedules to an HTTP server and a Vue
        // frontend; a goal is what a caller sends in.
        use factorio_bot_core::serde_json;
        let goal = Goal::All(vec![
            Goal::Have {
                item: "iron-plate".into(),
                count: 4,
                whose: Holder::Bot(BotId(2)),
                via: None,
            },
            Goal::Researched("automation".into()),
            Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 30,
            },
            Goal::Extracted {
                entity: "crude-oil".into(),
                unlocks: Some("oil-processing".into()),
            },
        ]);
        let json = serde_json::to_string(&goal).expect("serialises");
        let back: Goal = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, goal);
    }

    #[test]
    fn goals_render_for_diagnostics() {
        let g = Goal::Have {
            item: "coal".into(),
            count: 4,
            whose: Holder::Bot(BotId(2)),
            via: None,
        };
        assert_eq!(g.to_string(), "have 4 coal (bot 2)");
        let g = Goal::Have {
            item: "coal".into(),
            count: 4,
            whose: Holder::Anyone,
            via: None,
        };
        assert_eq!(g.to_string(), "have 4 coal (anyone)");
    }

    #[test]
    fn a_share_is_not_an_instruction_to_a_bot() {
        assert_ne!(Holder::Share(BotId(1)), Holder::Bot(BotId(1)));
    }

    #[test]
    fn holders_render_distinguishably() {
        assert_eq!(Holder::Bot(BotId(2)).to_string(), "bot 2");
        assert_eq!(
            Holder::Share(BotId(2)).to_string(),
            "a share sized for bot 2"
        );
        assert_eq!(Holder::Anyone.to_string(), "anyone");
    }

    #[test]
    fn a_built_goal_displays_its_siting_mode() {
        let at = Goal::Built {
            blueprint: "0eJyrVkrKz1cCoxQlK6VEJR2lYqVYHQVjIz0DPQMDPUM9IwMlHaVSJStDPQNTMDbUM9AzMlXSUcpMUbIy0jMwBWMDsFCsDgBnexPQ".to_string(),
            site: Site::At(Position::new(3.0, 4.0)),
        };
        assert!(format!("{at}").contains("at [3, 4]"));

        let near = Goal::Built {
            blueprint: "x".to_string(),
            site: Site::Near(Position::new(-8.0, 2.0)),
        };
        assert!(format!("{near}").contains("near [-8, 2]"));

        let anywhere = Goal::Built {
            blueprint: "x".to_string(),
            site: Site::Anywhere,
        };
        assert!(format!("{anywhere}").contains("anywhere"));
    }
}
