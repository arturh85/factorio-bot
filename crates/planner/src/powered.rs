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
    /// Positively established to need no electric network: a burner, a
    /// generator, a chest, a belt, a pipe. Either the world describes the
    /// prototype and it declares no electric energy source, or it declares one
    /// and produces rather than consumes.
    NotElectric,
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
    /// 2. it declares `max_energy_production > 0` → a producer or a store,
    ///    [`PowerNeed::NotElectric`]. Checked *before* the buffer, because
    ///    every generator carries a buffer of its own;
    /// 3. it declares `electric_energy_usage` or `electric_buffer_capacity` →
    ///    [`PowerNeed::Electric`], priced by
    ///    [`PlanState::consumer_draw_kw`](crate::state::PlanState::consumer_draw_kw)
    ///    or unpriced;
    /// 4. the world declares those fields *somewhere* and is silent about this
    ///    prototype → [`PowerNeed::NotElectric`], and the silence is evidence
    ///    because the sender demonstrably sends the field when it applies;
    /// 5. otherwise the world predates the fields: fall back to the priced
    ///    table, and call an unpriced name [`PowerNeed::NotElectric`] rather
    ///    than refusing a plan for the age of its dump. See the module doc.
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
            return PowerNeed::NotElectric;
        }
        if declares {
            return PowerNeed::Electric {
                kw: state.consumer_draw_kw(name),
            };
        }
        if world_declares_electric_source(state) {
            return PowerNeed::NotElectric;
        }
        match state.consumer_draw_kw(name) {
            Some(kw) => PowerNeed::Electric { kw: Some(kw) },
            None => PowerNeed::NotElectric,
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
/// * [`PlannerError::UnknownPrototypePlaced`] — the world never described it.
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
            PowerNeed::NotElectric => {}
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
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::types::FactorioEntityPrototype;

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
            PowerNeed::NotElectric,
            "a burner inserter declares neither field"
        );
    }

    /// A generator declares a buffer too, and is not something to be powered.
    #[test]
    fn a_generator_is_not_a_consumer() {
        assert_eq!(
            classify("steam-engine", None, Some(0.0), Some(15_000.0)),
            PowerNeed::NotElectric,
            "a steam engine's buffer is its output, not a draw"
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
            PowerNeed::NotElectric,
            "and a burner furnace is still not"
        );
    }
}
