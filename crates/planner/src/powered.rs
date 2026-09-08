//! **Is this thing electric, and did anybody power it?** — asked of every
//! entity a plan places, once, in one place.
//!
//! # Why this exists at all
//!
//! Power was opt-in per method and nothing checked. Measured on 2026-09-08:
//! `method::extract`, `method::blueprint`, `method::have`, `method::power` and
//! `method::assemble` emit a [`Condition::Powered`]; `method::fabricate`,
//! `method::produce`, `method::sustain`, `method::connect` and
//! `method::machine` emit none, and **no validation pass existed** — the only
//! power refusal in the tree was `ResearchNeedsPower`, which is one lab-shaped
//! special case. So a new method starts at zero and the omission is silent.
//!
//! It has already cost twice. `FurnaceLine` shipped as "proven live" with 13
//! poles and **no generator at all**; `method::fabricate` shipped an
//! `oil-refinery` — 420 kW, the largest single consumer this planner has ever
//! placed — with no power of any kind. Two independent methods, the same
//! omission: a property of the system, not two mistakes.
//!
//! # The predicate is the game's own energy source, not a table of names
//!
//! `mods/BotBridge/types.lua` sends `electric_energy_usage` and
//! `electric_buffer_capacity` **only when the prototype has an electric energy
//! source** — the gate is written into the mod, beside the object it reads.
//! So the *presence* of either field is the discriminator, and it survives
//! mods, which a list of names does not.
//!
//! Two facts make it load-bearing, both measured against the 1,028 prototypes
//! of the seed-31337 Space Age dump (`map-31337-water-and-oil.json`):
//!
//! * **`Some(0.0)` is a real answer.** An `inserter` reports
//!   `electric_energy_usage = None` and `electric_buffer_capacity = Some(0.0)`;
//!   a `burner-inserter` reports `None` for both. They share one `entity_type`
//!   and nothing else separates them. Reading either field as
//!   `unwrap_or(0.0)` would silently turn every burner machine into an
//!   electric one — and would wave through the 48 inserters of `FurnaceLine`,
//!   rebuilding the original bug inside its own guard.
//! * **A field's presence does not mean "consumes"**, which is the correction
//!   to the obvious form of this rule: `steam-engine`, `solar-panel`,
//!   `accumulator` and `burner-generator` all report a buffer. They are told
//!   apart by `max_energy_production`, which is what a *producer* declares.
//!
//! 48 of 1,028 prototypes carry either field; [`PowerNeed::of`] calls 41 of
//! them consumers and 7 producers or storage.
//!
//! # Three answers, never two
//!
//! [`PowerNeed`] is `Electric`, `NotElectric` or `Unknown`. The third is the
//! whole point: *absent is not a value*. A prototype the world never described
//! is "I have never heard of this", which is not "it draws nothing" — the
//! direction `state.rs`'s `vanilla_consumer_kw` errs in, and says it errs in.
//!
//! # And a world may predate the fields
//!
//! Every dump archived before 2026-09-07 does — `workspace/scripts/map.json`,
//! the map all three offline baselines are measured on, included. Such a world
//! is silent about *every* prototype, so field presence classifies nothing and
//! reading its silence as "burner" would pass the whole plan. [`audit`]
//! therefore asks [`world_declares_electric_source`] first and falls back
//! to the only positive evidence such a world offers: the draw
//! `PlanState::consumer_draw_kw` prices, which is `vanilla_consumer_kw`'s
//! table. That is **weaker on purpose and narrower than it looks** — it still
//! prices every machine this planner places, the `oil-refinery` and the
//! `pumpjack` among them — and it is stated here rather than hidden, because
//! what it cannot see is a modded consumer on an old dump.

use crate::action::{ActionKind, Condition};
use crate::error::PlannerError;

use crate::network::ActionNetwork;
use crate::state::PlanState;
use factorio_bot_core::types::{Position, Rect};

/// What the world says about one prototype's relationship to an electric
/// network.
#[derive(Clone, Debug, PartialEq)]
pub enum PowerNeed {
    /// It has an electric energy source and it consumes: nothing it is asked
    /// to do happens unless a network reaches it with capacity to spare.
    ///
    /// `kw` is what the demand ledger will charge for it, or `None` when the
    /// planner has an electric consumer it **cannot price**. Those are not the
    /// same answer and are not collapsed: an unpriced consumer put on a
    /// network is headroom that is not there, which is exactly the direction
    /// `BlockDemand::unpriced` exists to make visible.
    Electric { kw: Option<f64> },
    /// It has an electrical existence and **produces or stores** rather than
    /// consuming: a `steam-engine`, a `solar-panel`, an `accumulator`.
    ///
    /// # Two obligations, not one, and this is the half that was missing
    ///
    /// A consumer needs **supply** (a network with headroom) and **connection**
    /// (a pole reaching it). A generator needs only the second — and it needs
    /// it just as much, because *a steam engine wired to nothing generates into
    /// nothing*. It stands, it burns, it reports as built, and the network it
    /// was meant to feed reads exactly as if it were not there.
    ///
    /// Collapsing this into [`PowerNeed::Inert`] would be `FurnaceLine`'s
    /// failure with the polarity reversed — there, 48 consumers and no
    /// generator; here, a generator joined to nobody — and both place 100%
    /// correctly and do nothing. It is also the same *absent-is-not-a-value*
    /// shape as `Some(0.0)` one level down: "needs no supply" and "has no
    /// electrical existence" are different answers with different remedies, and
    /// the remedy is the reason they must not share a verdict. A pole, not a
    /// plant.
    ///
    /// `kw` is its nameplate output, as
    /// [`PlanState::generator_output_kw`](crate::state::PlanState::generator_output_kw)
    /// credits it — quoted in the refusal so a reader can see what was being
    /// thrown away.
    Generating { kw: f64 },
    /// No electrical existence at all: a burner machine, a chest, a belt, a
    /// pipe. Either the world describes the prototype and it declares no
    /// electric energy source, or the world predates the fields and nothing
    /// prices it.
    Inert,
    /// The world does not describe this prototype at all, so nothing here can
    /// say. **Not a pass**: [`audit`] refuses it by name.
    Unknown,
}

impl PowerNeed {
    /// Classify `name` against `state`.
    ///
    /// The order matters and each step is positive evidence:
    ///
    /// 1. no prototype at all → [`PowerNeed::Unknown`];
    /// 2. it declares `max_energy_production > 0` → [`PowerNeed::Generating`],
    ///    a producer or a store. Checked *before* the buffer, because every
    ///    generator carries a buffer of its own and would otherwise read as a
    ///    consumer;
    /// 3. it declares `electric_energy_usage` or `electric_buffer_capacity` →
    ///    [`PowerNeed::Electric`], priced by
    ///    [`PlanState::consumer_draw_kw`](crate::state::PlanState::consumer_draw_kw)
    ///    or unpriced;
    /// 4. the world declares those fields *somewhere* and is silent about this
    ///    prototype → [`PowerNeed::Inert`], and the silence is evidence
    ///    because the sender demonstrably sends the field when it applies;
    /// 5. otherwise the world predates the fields: fall back to the two priced
    ///    tables — `consumer_draw_kw` and `generator_output_kw`, which are the
    ///    same pair the ledger bills and credits — and call a name neither
    ///    knows [`PowerNeed::Inert`] rather than refusing a plan for the age of
    ///    its dump. See the module doc.
    pub fn of(state: &PlanState, name: &str) -> PowerNeed {
        let prototypes = &state.base().globals.entity_prototypes;
        let Some(prototype) = prototypes.get(name) else {
            return PowerNeed::Unknown;
        };
        let produces = prototype
            .max_energy_production
            .is_some_and(|joules| joules > 0.);
        let declares = prototype.electric_energy_usage.is_some()
            || prototype.electric_buffer_capacity.is_some();
        drop(prototype);
        if produces {
            // Nameplate from the same table `electric_supply_kw` credits, so a
            // refusal quotes the figure the ledger would have gained.
            return PowerNeed::Generating {
                kw: state.generator_output_kw(name).unwrap_or(0.),
            };
        }
        if declares {
            return PowerNeed::Electric {
                kw: state.consumer_draw_kw(name),
            };
        }
        if world_declares_electric_source(state) {
            return PowerNeed::Inert;
        }
        // A world predating the fields: `generator_output_kw` carries the same
        // `vanilla` fallback `consumer_draw_kw` does, so a steam engine on an
        // archived dump is still known to be a generator.
        if let Some(kw) = state.generator_output_kw(name) {
            return PowerNeed::Generating { kw };
        }
        match state.consumer_draw_kw(name) {
            Some(kw) => PowerNeed::Electric { kw: Some(kw) },
            None => PowerNeed::Inert,
        }
    }
}

/// Does any condition in `net` claim that a consumer standing at `pos` is
/// powered?
///
/// Coverage is asked over the **whole network**, not over the placing action,
/// because the two shapes in the tree state it differently: `method::extract`
/// and `method::assemble` put a [`Condition::Powered`] on the placement
/// itself, while `method::blueprint` states one
/// [`Condition::BlockPowered`] for a whole block and places its entities with
/// ordinary `AreaFree` preconditions. Both are honest claims about the same
/// plan; demanding the condition per action would refuse the second for a
/// fault it does not have.
///
/// The rectangle test is **inclusive** on all four edges, mirroring
/// `state::Excluded::covers` — the ground is drawn from footprints and an
/// outermost entity's position lies exactly on the edge of it. That pairing is
/// measured rather than reasoned; see `Excluded::covers`' own doc.
fn is_claimed_powered(net: &ActionNetwork, pos: &Position) -> bool {
    net.actions()
        .flat_map(|action| action.pre.iter())
        .any(|c| match c {
            Condition::Powered { pos: at, .. } => at == pos,
            Condition::BlockPowered {
                own_ground: ground, ..
            } => covers(ground, pos),
            _ => false,
        })
}

/// Inclusive-edge containment, the twin of `state::Excluded::covers`.
fn covers(ground: &Rect, pos: &Position) -> bool {
    pos.x() >= ground.left_top.x()
        && pos.x() <= ground.right_bottom.x()
        && pos.y() >= ground.left_top.y()
        && pos.y() <= ground.right_bottom.y()
}

/// How far from a generator this looks for a pole, in tiles.
///
/// The largest supply half-extent in 2.1.17 is a substation's 9, and the
/// largest generator this planner could place is 3 tiles across, so 16 covers
/// every pair with room to spare. It is a *search* bound, not a rule: the rule
/// is `PlanState::pole_would_supply`, which reads each pole's own
/// `supply_area_distance` from its prototype rather than assuming one.
const POLE_SEARCH_TILES: f64 = 16.;

/// Does a pole standing in this plan's world reach `pos`?
///
/// The question a **generator** has to answer, and it is not the question a
/// consumer answers. A consumer asks `Condition::Powered`, which is coverage
/// *and* capacity; a generator has no capacity question — it *is* the capacity
/// — and only has to be joined to the network it feeds.
///
/// Asked of the state rather than of the network, because a pole is a fact
/// about the ground: the plan's own poles are in the overlay by the time this
/// runs (`PlanState::create_entity`), and a pole that was already standing
/// counts exactly as much as one this plan places. `pole_would_supply` reads
/// the pole's own prototype, so a substation's 9 tiles and a small pole's 2.5
/// are not one constant here.
fn a_pole_reaches(state: &PlanState, name: &str, pos: &Position) -> bool {
    let area = state.collision_area(name, pos).unwrap_or_else(|| {
        // A prototype with no collision box: ask about the tile it stands on
        // rather than about nothing, the same fallback `method::blueprint`
        // uses for its pole probe.
        Rect::new(
            &Position::new(pos.x() - 0.05, pos.y() - 0.05),
            &Position::new(pos.x() + 0.05, pos.y() + 0.05),
        )
    });
    state
        .entities_within(pos, POLE_SEARCH_TILES)
        .iter()
        .any(|pole| state.pole_would_supply(&pole.name, &pole.position, &area))
}

/// **Every electric consumer a plan places must be shown to be powered, and
/// every prototype it places must be one the world can classify.**
///
/// Run over the finished network by
/// [`method::expand`](crate::method::expand), after `infer_edges` and
/// `validate`, so it sees the plan as a whole: a method may state its power
/// on a different action from the one that places the machine, and
/// `method::blueprint` states one claim for 179 of them.
///
/// # What a reader sees when it fails
///
/// The prototype, the tile, the draw, and the remedy — the shape
/// `PlannerError::BlockDrillUnfed` established for the sibling question about
/// ground. Three refusals, and they are three because they have three
/// different remedies:
///
/// * [`PlannerError::UnpoweredConsumer`] — the method that placed it never
///   asked for power. Call `method::power::ensure_powered` there.
/// * [`PlannerError::UnpricedConsumer`] — it is electric and nothing knows
///   what it draws, so no plant can be sized for it. Give it a row, or send
///   its `energy_usage`.
/// * [`PlannerError::GeneratorNotWired`] — it produces power and no pole
///   reaches it, so it generates into nothing. **The remedy is a pole, not a
///   plant**, which is exactly why this is not the same refusal as the first.
/// * [`PlannerError::UnknownPrototypePlaced`] — the world never described it.
///
/// # Supply and connection are two obligations
///
/// A consumer needs both; a generator needs only connection. Until 2026-09-08
/// a generator was skipped outright, which left one route open in practice and
/// not merely in theory: `method::blueprint` calls `ensure_powered` only when
/// the block has **consumers**, and `blueprint_power` checks coverage only for
/// consumers — so `StarterSteamEngineBoiler`, a fixture this repo ships (one
/// boiler, two steam engines, two poles, **zero consumers**), was audited by
/// nothing at all. Its own poles happen to reach; nothing asked.
///
/// # It only sees `ActionKind::Place`
///
/// A ghost draws nothing until something builds it, and nothing in this
/// planner builds a ghost, so `ActionKind::StampGhosts` is deliberately out of
/// scope. The day construction robots land, a stamped block's own consumers
/// become real and this is the function that has to grow a second arm.
pub fn audit(net: &ActionNetwork, state: &PlanState) -> Result<(), PlannerError> {
    for action in net.actions() {
        let ActionKind::Place { entity } = &action.kind else {
            continue;
        };
        match PowerNeed::of(state, &entity.name) {
            PowerNeed::Inert => {}
            PowerNeed::Generating { kw } => {
                if !a_pole_reaches(state, &entity.name, &entity.position) {
                    return Err(PlannerError::GeneratorNotWired {
                        prototype: entity.name.clone(),
                        pos: entity.position.to_string(),
                        kw,
                        label: action.label.clone(),
                    });
                }
            }
            PowerNeed::Unknown => {
                return Err(PlannerError::UnknownPrototypePlaced {
                    prototype: entity.name.clone(),
                    pos: entity.position.to_string(),
                    label: action.label.clone(),
                });
            }
            PowerNeed::Electric { kw: None } => {
                return Err(PlannerError::UnpricedConsumer {
                    prototype: entity.name.clone(),
                    pos: entity.position.to_string(),
                });
            }
            PowerNeed::Electric { kw: Some(kw) } => {
                if !is_claimed_powered(net, &entity.position) {
                    return Err(PlannerError::UnpoweredConsumer {
                        prototype: entity.name.clone(),
                        pos: entity.position.to_string(),
                        kw,
                        label: action.label.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Does this world's sender describe electric energy sources **at all**?
///
/// The gate between the two modes in [`PowerNeed::of`], and it is a question
/// about the *sender*, not about any one prototype: the mod emits
/// `electric_energy_usage` / `electric_buffer_capacity` for every prototype
/// that has an electric energy source, so one such prototype anywhere proves
/// the field is being sent and makes silence about a second prototype
/// evidence. Nowhere at all means the world predates the fields — every dump
/// archived before 2026-09-07, `workspace/scripts/map.json` included — and
/// silence there means nothing.
///
/// The same shape as `MachineTable::world_declares_categories`, for the same
/// reason: **a world that never declared a field must say so**, rather than
/// having its silence read as a value.
#[must_use]
pub fn world_declares_electric_source(state: &PlanState) -> bool {
    state.base().globals.entity_prototypes.iter().any(|entry| {
        entry.value().electric_energy_usage.is_some()
            || entry.value().electric_buffer_capacity.is_some()
    })
}

#[cfg(test)]
mod powered_tests {
    use super::*;
    use crate::action::{Action, ActionKind};
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::types::{FactorioEntity, FactorioEntityPrototype};

    /// Give the fixture world one prototype with the fields a live 2.1.17
    /// sender puts on it, and classify it.
    ///
    /// The values are transcribed from the seed-31337 dump
    /// (`map-31337-water-and-oil.json`), not invented: `usage` and `buffer`
    /// are exactly what `entity_prototypes` carries for that name.
    fn plain_state() -> PlanState {
        PlanState::from_world(
            std::sync::Arc::new(factorio_bot_core::test_utils::fixture_world()),
            &[BotId(1)],
        )
    }

    fn classify(
        name: &str,
        usage: Option<f64>,
        buffer: Option<f64>,
        produces: Option<f64>,
    ) -> PowerNeed {
        let state = plain_state();
        let json = |v: Option<f64>| match v {
            Some(x) => format!("{x}"),
            None => "null".to_string(),
        };
        // Deserialised rather than built as a literal, for the reason
        // `machine.rs`'s own fixture gives: the serde attributes decide how
        // the mod's payload is read, and a literal bypasses them -- which is
        // exactly the distinction (absent versus present-and-zero) under test.
        let prototype: FactorioEntityPrototype = factorio_bot_core::serde_json::from_str(&format!(
            r#"{{ "name": "{name}", "entity_type": "inserter",
                  "collision_mask": [],
                  "collision_box": {{ "left_top": {{"x": -0.4, "y": -0.4}},
                                     "right_bottom": {{"x": 0.4, "y": 0.4}} }},
                  "electric_energy_usage": {},
                  "electric_buffer_capacity": {},
                  "max_energy_production": {} }}"#,
            json(usage),
            json(buffer),
            json(produces)
        ))
        .expect("a prototype in the shape the mod sends");
        state
            .base()
            .globals
            .entity_prototypes
            .insert(name.to_string(), prototype);
        PowerNeed::of(&state, name)
    }

    /// **The case that would defeat the naive predicate.** An `inserter` and a
    /// `burner-inserter` share one `entity_type` and neither declares
    /// `energy_usage`; the only thing that separates them is a buffer of
    /// **zero**, present on one and absent on the other.
    ///
    /// If either field were ever read as `unwrap_or(0.0)`, or typed as a bare
    /// `f64`, both rows below would answer the same and every burner machine
    /// in the game would become an electric one — silently, since a plan that
    /// powers too much still plans.
    #[test]
    fn a_zero_buffer_is_electric_and_a_missing_one_is_not() {
        assert!(
            matches!(
                classify("inserter", None, Some(0.0), Some(0.0)),
                PowerNeed::Electric { .. }
            ),
            "an inserter declares a zero-sized electric buffer and is electric"
        );
        assert_eq!(
            classify("burner-inserter", None, None, None),
            PowerNeed::Inert,
            "a burner inserter declares neither field"
        );
    }

    /// A generator declares a buffer too, and is **not** a consumer — but it
    /// is not inert either, which is the distinction this enum exists to keep.
    ///
    /// Three answers where a two-valued predicate would have had two: it needs
    /// no supply, it does need a pole, and a chest needs neither.
    #[test]
    fn a_generator_is_not_a_consumer_and_is_not_inert_either() {
        assert_eq!(
            classify("steam-engine", None, Some(0.0), Some(15_000.0)),
            PowerNeed::Generating { kw: 900.0 },
            "a steam engine's buffer is its output, not a draw, and 900 kW is what the \
             supply ledger credits it"
        );
        assert_eq!(
            classify("wooden-chest", None, None, None),
            PowerNeed::Inert,
            "a chest has no electrical existence at all"
        );
    }

    /// The third answer. A prototype this world never described cannot be
    /// classified, and saying "it needs no power" would be inventing evidence.
    #[test]
    fn a_prototype_the_world_never_described_is_unknown_not_free() {
        let state = plain_state();
        assert_eq!(
            PowerNeed::of(&state, "no-such-entity-anywhere"),
            PowerNeed::Unknown
        );
    }

    /// An electric consumer nobody can price is **not** priced at zero.
    #[test]
    fn an_electric_consumer_with_no_figure_is_unpriced_rather_than_free() {
        assert_eq!(
            classify("laser-turret", None, Some(801_000.0), Some(0.0)),
            PowerNeed::Electric { kw: None },
            "the world says it is electric and nothing knows its draw"
        );
    }

    /// A world whose sender predates both fields classifies from the priced
    /// table instead, so the offline baselines on `map.json` still plan.
    #[test]
    fn a_world_that_predates_the_fields_falls_back_to_what_is_priced() {
        let state = plain_state();
        assert!(
            !world_declares_electric_source(&state),
            "the fixture world predates the electric fields, as every archived dump does"
        );
        assert!(
            matches!(
                PowerNeed::of(&state, "oil-refinery"),
                PowerNeed::Electric { kw: Some(_) }
            ),
            "a refinery is still known to be electric on such a world"
        );
        assert_eq!(
            PowerNeed::of(&state, "stone-furnace"),
            PowerNeed::Inert,
            "and a burner furnace is still not"
        );
    }

    /// **A generator wired to nothing is refused, and a pole is what fixes
    /// it.** The paired halves are the point: the same plan, the same engine,
    /// one pole apart.
    ///
    /// This is the guard for a hole that was reachable rather than
    /// hypothetical. `method::blueprint` asks `ensure_powered` only when a
    /// block has consumers, and `blueprint_power` checks coverage only for
    /// consumers -- so `StarterSteamEngineBoiler` (`scripts/rcontest.lua`: one
    /// boiler, two steam engines, two poles, zero consumers) passes through
    /// both untouched. Its own poles do reach; nothing in the tree asked, and
    /// nothing would have noticed a variant whose poles did not.
    #[test]
    fn a_generator_no_pole_reaches_is_refused_and_a_pole_settles_it() {
        let place = |net: &mut ActionNetwork| {
            net.add(Action {
                id: crate::ids::ActionId(1),
                kind: ActionKind::Place {
                    entity: Box::new(FactorioEntity {
                        name: "steam-engine".into(),
                        entity_type: "generator".into(),
                        position: Position::new(60.5, 60.5),
                        ..Default::default()
                    }),
                },
                pre: Vec::new(),
                eff: Vec::new(),
                duration: 30,
                pinned: None,
                label: "place steam-engine at [60.5, 60.5]".into(),
            });
        };

        let mut alone = ActionNetwork::new();
        place(&mut alone);
        let bare = plain_state();
        let refusal = audit(&alone, &bare).expect_err("a generator no pole reaches is refused");
        let said = refusal.to_string();
        assert!(said.contains("steam-engine"), "{said}");
        assert!(said.contains("generates 900 kW"), "{said}");
        assert!(said.contains("no pole reaches it"), "{said}");

        // The one difference: a pole within its supply area. The engine, the
        // network and the audit are otherwise identical, so nothing but the
        // pole can account for the change.
        let mut wired = plain_state();
        wired.create_entity(FactorioEntity {
            name: "small-electric-pole".into(),
            entity_type: "electric-pole".into(),
            position: Position::new(62.5, 60.5),
            ..Default::default()
        });
        let mut with_pole = ActionNetwork::new();
        place(&mut with_pole);
        audit(&with_pole, &wired).expect("a pole beside it is the whole remedy");
    }
}
