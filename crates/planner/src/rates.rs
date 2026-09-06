//! What a plan's own research does to the rates the plan was costed at — and,
//! deliberately, only saying so rather than correcting it.
//!
//! # The defect this reports on
//!
//! A real rate in Factorio is `prototype x force bonus x module effect`. The
//! model has the first factor and, for exactly one term, the second: the
//! acting force's `manual_mining_speed_modifier`, read by
//! [`crate::method::util::character_mining_speed`]. That modifier is a
//! property of the world **snapshot**. `Effect::Researched` inserts a name
//! into a set and changes no rate, and `PlanState::base` is never mutated, so
//! a plan that researches `steel-axe` — vanilla's `character-mining-speed +1`,
//! which doubles hand mining — goes on costing every subsequent mine at the
//! old rate.
//!
//! # Why disclosure and not correction
//!
//! Correcting it means costing an action against the state at the time it
//! runs, which makes `duration` a function of schedule position while the
//! schedule is chosen from the durations. Every escape from that circle costs
//! at least one extra expansion on every replan, and needs a termination
//! argument in a crate whose determinism is load-bearing.
//!
//! And the error was **measured at exactly zero** on 2026-09-06, on both goals
//! this project plans and in all 21 archived runs: neither
//! `researched:automation` nor `producing:logistic-science-pack:6` researches
//! any technology that changes any rate the planner models
//! (`docs/superpowers/notes/2026-09-06-costs-change-as-research-lands.md`).
//! Paying scheduling risk to correct a zero is the wrong order. Counting it is
//! not: the counterfactual in that note put the bound at −9.7% of green's
//! makespan and −29.6% of one plan's bot-ticks, and `steel-axe` sits in the
//! speedrun guide's mandatory research order right after `steel-processing`.
//! **This module is what turns "when it starts to matter, nobody will notice"
//! into a number in every plan report.**
//!
//! # What it counts, and what it cannot
//!
//! Only what the planner actually costs. Today that is one term —
//! `character-mining-speed`, reaching `mining_ticks` through
//! `character_mining_speed` — so [`RateDisclosure::overstated_ticks`] is a
//! statement about `Mine` and `Chop` steps and nothing else.
//!
//! Every *other* rate-changing effect a plan's own research grants is named in
//! [`RateDisclosure::unmodelled`] instead of being silently dropped. That list
//! is the honest half: `laboratory-speed` moves research duration and
//! `research_ticks_in_labs` has no speed term at all, so the error there is
//! not merely uncorrected, it is unsizeable. Reporting the names is the
//! difference between an error nobody has measured and an error nobody knows
//! about.

use crate::action::{ActionKind, Effect};
use crate::ids::Ticks;
use crate::network::ActionNetwork;
use crate::schedule::{Schedule, StepKind};
use crate::state::PlanState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The `ModifierType` string for the one force bonus the planner's durations
/// depend on.
///
/// Spelled here once, kebab-case, exactly as `LuaTechnologyPrototype::effects`
/// reports it and as `mods/BotBridge/types.lua` forwards it. It is a `&str`
/// and not an enum for the same reason
/// [`factorio_bot_core::types::FactorioTechnologyEffect::kind`] is: 2.1.17 has
/// 51 modifier types and a mod may add more, and an unknown one must be data
/// rather than a parse failure.
pub const CHARACTER_MINING_SPEED: &str = "character-mining-speed";

/// Modifier kinds that change *some* rate, whether or not this planner costs
/// it.
///
/// Used only to populate [`RateDisclosure::unmodelled`]. Deliberately
/// excludes the ~40 kinds that change a distance, a slot count, a combat
/// number or an unlock flag: naming those would bury the ones that matter.
/// The six that vanilla base technologies actually grant are all here; the
/// rest are carried because a mod can grant them.
const RATE_KINDS: &[&str] = &[
    CHARACTER_MINING_SPEED,
    "character-crafting-speed",
    "character-running-speed",
    "laboratory-speed",
    "laboratory-productivity",
    "mining-drill-productivity-bonus",
    "inserter-stack-size-bonus",
    "bulk-inserter-capacity-bonus",
    "belt-stack-size-bonus",
    "worker-robot-speed",
    "change-recipe-productivity",
    "beacon-distribution",
];

/// One research action in the plan, and what it does to a rate.
// No `Eq`: `mining_speed_modifier` is an `f64`. Comparisons on this type are
// for tests, and the crate's determinism rule governs float *ordering*, which
// this type does none of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RateResearch {
    /// The technology the plan researches.
    pub technology: String,
    /// The tick the research action is scheduled to finish. Every
    /// rate-sensitive step starting at or after this is mis-costed.
    pub finished_at: Ticks,
    /// The `character-mining-speed` this technology adds, summed over its
    /// effects. Rendered as a plain `f64` rather than the world's `R64`
    /// because this type is a report, not a hash key.
    pub mining_speed_modifier: f64,
}

/// How much a plan's costed work disagrees with the rates the plan itself
/// brings about.
///
/// **Zero is the expected reading today, and a zero here is a measurement, not
/// an absence.** [`RateDisclosure::is_clean`] distinguishes "checked, nothing
/// found" from "nothing to check".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RateDisclosure {
    /// The plan's own research actions that grant `character-mining-speed`,
    /// in scheduled-finish order.
    pub research: Vec<RateResearch>,
    /// Ticks of `Mine`/`Chop` work scheduled to *start* at or after one of
    /// those researches finishes — the work that is costed at the wrong rate.
    ///
    /// A step already running when the research lands is not counted: part of
    /// it is honestly costed and splitting it would need a model of when the
    /// game applies the bonus mid-swing. The omission makes this a floor.
    pub affected_ticks: u64,
    /// How many of [`Self::affected_ticks`] the plan charges and would not
    /// have to, at the rate in force when each step starts.
    ///
    /// **A floor on the mis-costing, not a ceiling on the saving.** It re-costs
    /// the schedule that exists; it does not re-order it, and re-ordering is
    /// where the larger number is (mining 400 ore *after* `steel-axe` rather
    /// than before was worth 24,000 bot-ticks on one measured goal). Nor is it
    /// a makespan claim: mining is often off the critical path, and one
    /// measured goal moved 20.5% on bot-ticks while moving 0.6% on makespan.
    pub overstated_ticks: u64,
    /// [`Self::overstated_ticks`] as a percentage of the plan's total planned
    /// bot-ticks. `None` when the plan has no planned ticks at all.
    pub overstated_percent: Option<f64>,
    /// Technologies this plan researches that change a rate the planner does
    /// **not** cost — `laboratory-speed` and friends — as
    /// `technology: modifier-kind` pairs, sorted.
    ///
    /// Not an error term, because there is no term: nothing in the model
    /// responds to these at all, so their contribution cannot be sized from
    /// here. Named so that it is not silent.
    pub unmodelled: Vec<String>,
}

impl RateDisclosure {
    /// True when nothing in this plan mis-costs a rate — no rate-granting
    /// research either modelled or unmodelled.
    pub fn is_clean(&self) -> bool {
        self.research.is_empty() && self.unmodelled.is_empty()
    }

    /// Reads the disclosure off a scheduled plan.
    ///
    /// `state` is the state the plan was *costed* against, which is the whole
    /// point: the base `manual_mining_speed_modifier` it carries is the rate
    /// every duration in `schedule` was computed at.
    pub fn of(net: &ActionNetwork, schedule: &Schedule, state: &PlanState) -> RateDisclosure {
        let base = state.manual_mining_speed_modifier().max(0.);

        let mut research: Vec<RateResearch> = Vec::new();
        let mut unmodelled: BTreeSet<String> = BTreeSet::new();
        for step in &schedule.steps {
            let StepKind::Act { action, .. } = &step.what else {
                continue;
            };
            let Some(action) = net.action(*action) else {
                continue;
            };
            // `Effect::Researched`, not `ActionKind::Research`. **A trigger
            // technology has no research action at all**: `method::have`
            // hangs the effect on whichever action produces the item the
            // trigger names, and `steel-axe` -- the one vanilla technology
            // that changes a rate the planner costs -- is exactly such a
            // technology (craft 50 steel plate). Keying on the action kind
            // read `researched:steel-axe` as researching no rate bonus, on a
            // dump hand-injected to say otherwise. A research action carries
            // the effect too, so this is the wider net and not a different
            // one.
            for tech in action.eff.iter().filter_map(|e| match e {
                Effect::Researched(tech) => Some(tech),
                _ => None,
            }) {
                let Some(technology) = state.technology(tech) else {
                    continue;
                };
                let mut mining = 0.;
                for effect in &technology.effects {
                    if !RATE_KINDS.contains(&effect.kind.as_str()) {
                        continue;
                    }
                    let amount = effect.modifier.as_deref().copied().map(f64::from);
                    if effect.kind == CHARACTER_MINING_SPEED {
                        mining += amount.unwrap_or(0.);
                    } else {
                        unmodelled.insert(format!("{tech}: {}", effect.kind));
                    }
                }
                if mining != 0. {
                    research.push(RateResearch {
                        technology: tech.clone(),
                        finished_at: step.end,
                        mining_speed_modifier: mining,
                    });
                }
            }
        }
        // Deterministic: `total_cmp` per the crate's float rule, and the
        // technology name breaks a tie so two researches finishing on the same
        // tick cannot order by insertion.
        research.sort_by(|a, b| {
            a.finished_at
                .cmp(&b.finished_at)
                .then_with(|| a.technology.cmp(&b.technology))
        });

        let mut affected_ticks = 0u64;
        let mut saved = 0f64;
        let mut planned_ticks = 0u64;
        for step in &schedule.steps {
            let ticks = u64::from(step.end.saturating_sub(step.start));
            planned_ticks += ticks;
            let StepKind::Act { action, .. } = &step.what else {
                continue;
            };
            let Some(kind) = net.action(*action).map(|a| &a.kind) else {
                continue;
            };
            if !matches!(kind, ActionKind::Mine { .. } | ActionKind::Chop { .. }) {
                continue;
            }
            // Only research that has *finished* by the time this step starts.
            let granted: f64 = research
                .iter()
                .filter(|r| r.finished_at <= step.start)
                .map(|r| r.mining_speed_modifier)
                .sum();
            if granted <= 0. {
                continue;
            }
            affected_ticks += ticks;
            // `mining_ticks` divides by `base_speed * (1 + modifier)`, so the
            // whole duration scales by the ratio of the two `(1 + m)` terms.
            let factor = (1. + base) / (1. + base + granted);
            saved += ticks as f64 * (1. - factor);
        }

        let overstated_ticks = saved.floor().max(0.) as u64;
        let overstated_percent = if planned_ticks == 0 {
            None
        } else {
            Some(overstated_ticks as f64 * 100. / planned_ticks as f64)
        };
        RateDisclosure {
            research,
            affected_ticks,
            overstated_ticks,
            overstated_percent,
            unmodelled: unmodelled.into_iter().collect(),
        }
    }

    /// The disclosure as lines a person reads. Always at least one line:
    /// "checked and found nothing" and "not checked" must not look alike.
    pub fn lines(&self) -> Vec<String> {
        if self.is_clean() {
            return vec!["rate drift     none (this plan researches no rate bonus)".to_string()];
        }
        let mut out = Vec::new();
        match self.overstated_percent {
            Some(percent) => out.push(format!(
                "rate drift     {} of {} rate-sensitive ticks overstated ({percent:.1}% of planned)",
                self.overstated_ticks, self.affected_ticks
            )),
            None => out.push(format!(
                "rate drift     {} of {} rate-sensitive ticks overstated",
                self.overstated_ticks, self.affected_ticks
            )),
        }
        for research in &self.research {
            out.push(format!(
                "               {} grants mining +{:.2} at tick {}",
                research.technology, research.mining_speed_modifier, research.finished_at
            ));
        }
        if !self.unmodelled.is_empty() {
            out.push(format!(
                "               unmodelled, size unknown: {}",
                self.unmodelled.join(", ")
            ));
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionKind, Effect};
    use crate::ids::{ActionId, BotId};
    use crate::schedule::ScheduledStep;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{FactorioForce, Position};
    use std::sync::Arc;

    /// A `player` force whose `steel-axe` carries `effects` verbatim as JSON.
    ///
    /// Built by deserialising rather than by struct literal, following
    /// `method::util`'s `state_with_force_json`: these tests then also pin the
    /// wire shape `mods/BotBridge`'s `serialize_technology` sends, rather than
    /// only the Rust type.
    fn state_with_effects(effects: &str, base_modifier: f64) -> PlanState {
        let force: FactorioForce = serde_json::from_str(&format!(
            r#"{{
              "name": "player",
              "force_id": 1,
              "current_research": null,
              "research_progress": null,
              "manual_mining_speed_modifier": {base_modifier},
              "technologies": {{
                "steel-axe": {{
                  "name": "steel-axe", "enabled": true, "upgrade": false,
                  "researched": false, "prerequisites": ["steel-processing"],
                  "research_unit_ingredients": [], "research_unit_count": 0,
                  "research_unit_energy": 0.0, "order": "c", "level": 1,
                  "valid": true, "unlocked_recipes": [],
                  "effects": {effects}
                }}
              }}
            }}"#
        ))
        .expect("the force fixture must parse");
        let world = fixture_world();
        world.update_force(force).expect("update_force");
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    fn act(id: u32, kind: ActionKind, duration: Ticks) -> Action {
        Action {
            id: ActionId(id),
            kind,
            pre: Vec::new(),
            eff: Vec::new(),
            duration,
            pinned: None,
            label: format!("action {id}"),
        }
    }

    fn mine(id: u32) -> Action {
        act(
            id,
            ActionKind::Mine {
                pos: Position::new(0.5, 0.5),
                item: "iron-ore".into(),
                count: 1,
            },
            1_000,
        )
    }

    /// A network and schedule with `research steel-axe` over `[0, 100)` and
    /// two 1,000-tick mines, one before it and one after.
    ///
    /// The two mines are the discriminator: only the later one may be counted,
    /// so a check that counted every mine in the plan would read 2,000 here
    /// and 1,000 is the correct answer.
    fn plan_with_research_between_two_mines() -> (ActionNetwork, Schedule) {
        let mut net = ActionNetwork::new();
        let before = net.add(mine(1));
        let mut research_action = act(
            2,
            ActionKind::Research {
                tech: "steel-axe".into(),
            },
            100,
        );
        research_action.eff = vec![Effect::Researched("steel-axe".into())];
        let research = net.add(research_action);
        let after = net.add(mine(3));
        let step = |action: ActionId, start: Ticks, end: Ticks| ScheduledStep {
            what: StepKind::Act {
                action,
                label: String::new(),
            },
            bot: BotId(1),
            start,
            end,
        };
        let schedule = Schedule {
            steps: vec![
                step(before, 0, 1_000),
                step(research, 1_000, 1_100),
                step(after, 1_100, 2_100),
            ],
            makespan: 2_100,
        };
        (net, schedule)
    }

    const MINING_SPEED_EFFECT: &str = r#"[{"kind": "character-mining-speed", "modifier": 1.0}]"#;

    #[test]
    fn a_plan_that_researches_nothing_is_clean() {
        let state = state_with_effects(MINING_SPEED_EFFECT, 0.);
        let mut net = ActionNetwork::new();
        let only = net.add(mine(1));
        let schedule = Schedule {
            steps: vec![ScheduledStep {
                what: StepKind::Act {
                    action: only,
                    label: String::new(),
                },
                bot: BotId(1),
                start: 0,
                end: 1_000,
            }],
            makespan: 1_000,
        };
        let disclosure = RateDisclosure::of(&net, &schedule, &state);
        assert!(
            disclosure.is_clean(),
            "a plan with no research action must disclose nothing: {disclosure:?}"
        );
        assert_eq!(disclosure.overstated_ticks, 0);
        assert_eq!(
            disclosure.lines(),
            vec!["rate drift     none (this plan researches no rate bonus)".to_string()],
            "a clean plan must still print a line -- silence is not success"
        );
    }

    /// The arithmetic, from literals rather than from the code under test.
    ///
    /// `steel-axe` grants `character-mining-speed +1` and the force starts at
    /// 0, so hand mining goes from `speed * 1` to `speed * 2`: the mine
    /// scheduled after the research costs half what the plan charged.
    /// 1,000 ticks charged, 500 of them wrongly. The mine *before* the
    /// research is correctly costed and contributes nothing.
    #[test]
    fn work_scheduled_after_a_mining_research_is_disclosed_as_overstated() {
        let state = state_with_effects(MINING_SPEED_EFFECT, 0.);
        let (net, schedule) = plan_with_research_between_two_mines();
        let disclosure = RateDisclosure::of(&net, &schedule, &state);

        assert_eq!(
            disclosure.research.len(),
            1,
            "exactly one rate-granting research: {disclosure:?}"
        );
        assert_eq!(disclosure.research[0].technology, "steel-axe");
        assert_eq!(disclosure.research[0].finished_at, 1_100);
        assert_eq!(
            disclosure.affected_ticks, 1_000,
            "only the mine that starts after the research is affected"
        );
        assert_eq!(
            disclosure.overstated_ticks, 500,
            "1000 ticks at 1x where the plan will run at 2x"
        );
        // 500 of 2,100 planned ticks.
        assert_eq!(
            disclosure.overstated_percent.map(|p| (p * 10.).round()),
            Some(238.),
            "23.8% of planned bot-ticks"
        );
        assert!(disclosure.unmodelled.is_empty());
    }

    /// The same plan against a force that already has the bonus. The research
    /// still grants +1, but from a base of 1 rather than 0, so the mine gets
    /// 2x -> 3x and a third of it is overstated, not a half. A check that
    /// ignored the base would report 500 here.
    #[test]
    fn the_disclosure_is_measured_from_the_force_bonus_already_in_effect() {
        let state = state_with_effects(MINING_SPEED_EFFECT, 1.);
        let (net, schedule) = plan_with_research_between_two_mines();
        let disclosure = RateDisclosure::of(&net, &schedule, &state);
        assert_eq!(
            disclosure.overstated_ticks, 333,
            "1000 * (1 - 2/3), floored"
        );
    }

    /// `research-speed-1` changes a rate the planner does not cost at all.
    /// It must be named, and it must not be silently folded into a number
    /// that would then claim to be the whole error.
    #[test]
    fn a_rate_effect_the_planner_does_not_model_is_named_and_not_counted() {
        let state = state_with_effects(r#"[{"kind": "laboratory-speed", "modifier": 0.2}]"#, 0.);
        let (net, schedule) = plan_with_research_between_two_mines();
        let disclosure = RateDisclosure::of(&net, &schedule, &state);
        assert!(disclosure.research.is_empty());
        assert_eq!(disclosure.overstated_ticks, 0);
        assert_eq!(disclosure.unmodelled, vec!["steel-axe: laboratory-speed"]);
        assert!(
            !disclosure.is_clean(),
            "an unmodelled rate effect is a disclosure, not a clean bill"
        );
        let lines = disclosure.lines().join("\n");
        assert!(
            lines.contains("unmodelled, size unknown: steel-axe: laboratory-speed"),
            "the reader must see the name: {lines}"
        );
    }

    /// An `unlock-recipe`, which every technology has and which changes no
    /// rate, must not reach either list.
    #[test]
    fn an_unlock_recipe_effect_is_not_a_rate_change() {
        let state = state_with_effects(
            r#"[{"kind": "unlock-recipe", "target": "steel-plate"}]"#,
            0.,
        );
        let (net, schedule) = plan_with_research_between_two_mines();
        let disclosure = RateDisclosure::of(&net, &schedule, &state);
        assert!(
            disclosure.is_clean(),
            "unlock-recipe is not a rate: {disclosure:?}"
        );
    }

    /// **The case that actually matters, and the one a first version missed.**
    ///
    /// `steel-axe` is a `research_trigger` technology (craft 50 steel plate),
    /// so a plan that earns it contains **no research action at all**:
    /// `method::have` hangs `Effect::Researched` on whichever action produces
    /// the item. Keying the check on `ActionKind::Research` reported "this
    /// plan researches no rate bonus" for a plan that earns the only vanilla
    /// technology this whole module exists for — and it did so on the live
    /// 865 MB dump with the effect hand-injected, which is how it was caught.
    #[test]
    fn a_trigger_technology_earned_by_a_craft_is_disclosed_too() {
        let state = state_with_effects(MINING_SPEED_EFFECT, 0.);
        let mut net = ActionNetwork::new();
        let mut craft = act(
            1,
            ActionKind::Craft {
                item: "steel-plate".into(),
                count: 50,
            },
            100,
        );
        // No ActionKind::Research anywhere in this plan.
        craft.eff = vec![Effect::Researched("steel-axe".into())];
        let earner = net.add(craft);
        let after = net.add(mine(2));
        let step = |action: ActionId, start: Ticks, end: Ticks| ScheduledStep {
            what: StepKind::Act {
                action,
                label: String::new(),
            },
            bot: BotId(1),
            start,
            end,
        };
        let schedule = Schedule {
            steps: vec![step(earner, 0, 100), step(after, 100, 1_100)],
            makespan: 1_100,
        };
        let disclosure = RateDisclosure::of(&net, &schedule, &state);
        assert!(
            !net.actions()
                .any(|a| matches!(a.kind, ActionKind::Research { .. })),
            "the fixture must contain no research action, or it proves nothing"
        );
        assert_eq!(
            disclosure.research.len(),
            1,
            "a trigger technology is still a rate change: {disclosure:?}"
        );
        assert_eq!(disclosure.overstated_ticks, 500);
    }
}
