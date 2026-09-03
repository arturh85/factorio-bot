//! A plan reduced to the numbers two plans are compared by.
//!
//! This exists because of what workstream 0 of
//! `docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md` is for:
//! evaluating a planner change without spending a 20-minute live run on it.
//! The comparison a person actually makes is a handful of numbers, and the
//! plan document names them — action count, makespan, `steps/bot`, planned
//! ticks per bot — because those are the ones that moved (and, more often,
//! failed to move) while whole classes of defect were fixed.
//!
//! # Idle is reported, and it is the point
//!
//! The reference run was **65.9% idle for its busiest bot** and 8.8% utilised
//! across the roster. A report that gave only a makespan would say a plan got
//! slower without ever saying that three bots did nothing, so
//! [`BotReport::idle_ticks`] is here beside the work — the same decomposition
//! the plan's own tables use.
//!
//! # Not a substitute for a run
//!
//! Every number here is *planned*, which is what makes it cheap and what
//! bounds what it can be trusted for. `duration` on an action is a nominal
//! estimate, and the plan's own measurements record where nominal and observed
//! diverge (crafts settle at exactly nominal; a smelt's wait does not appear as
//! any action's duration at all). Two of these reports compare two plans. A
//! report against a live run's elapsed ticks compares two different things.

use crate::ids::{BotId, Ticks};
use crate::network::ActionNetwork;
use crate::schedule::{Schedule, StepKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What one bot was given to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotReport {
    pub bot: BotId,
    /// Every scheduled step, walks included. This is the `steps/bot` the plan
    /// document tracks — the number that "has been the number that mattered
    /// all along", and that sat at `{1: 103, 2: 4, 3: 4, 4: 4}` while two
    /// classes of defect were fixed and the headline time did not move.
    pub steps: usize,
    /// Steps that do work.
    pub acts: usize,
    /// Steps that only get the bot somewhere. Counted apart from `acts`
    /// because walking was 21% of the reference run and is invisible in an
    /// action histogram — a walk is not an action and carries no
    /// `elapsed_ticks`.
    pub walks: usize,
    /// Ticks this bot is occupied, summed over its steps. Steps on one bot
    /// never overlap, so this is a duration and not a double count.
    pub planned_ticks: u64,
    /// `makespan - planned_ticks`: the ticks this bot is scheduled to stand
    /// still while the plan is still running.
    pub idle_ticks: u64,
}

/// A whole plan's numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanReport {
    /// Actions in the expanded network. Not the same as the sum of `acts`
    /// below unless every action was scheduled — which `schedule` guarantees
    /// today, and which this makes checkable rather than assumed.
    pub actions: usize,
    pub makespan: Ticks,
    /// One entry per bot with a step, in bot-id order.
    pub bots: Vec<BotReport>,
    /// Ticks of work across the whole roster.
    pub planned_ticks: u64,
    /// Roster utilisation as a percentage: work done over bot-time available
    /// (`makespan * bots`). The reference run's was 8.8%.
    ///
    /// `None` when there is no roster or the makespan is zero, rather than a
    /// zero that would read as "fully idle".
    pub utilisation_percent: Option<f64>,
}

impl PlanReport {
    /// Reads a report off an expanded network and the schedule made from it.
    ///
    /// `bots` is the roster the schedule was made for, not the set of bots
    /// that ended up with a step: a bot given nothing to do is the finding,
    /// and dropping it from the report is how that finding goes unnoticed.
    pub fn of(net: &ActionNetwork, schedule: &Schedule, bots: &[BotId]) -> PlanReport {
        let mut per_bot: BTreeMap<BotId, BotReport> = bots
            .iter()
            .map(|bot| {
                (
                    *bot,
                    BotReport {
                        bot: *bot,
                        steps: 0,
                        acts: 0,
                        walks: 0,
                        planned_ticks: 0,
                        idle_ticks: 0,
                    },
                )
            })
            .collect();
        for step in &schedule.steps {
            let entry = per_bot.entry(step.bot).or_insert(BotReport {
                bot: step.bot,
                steps: 0,
                acts: 0,
                walks: 0,
                planned_ticks: 0,
                idle_ticks: 0,
            });
            entry.steps += 1;
            match step.what {
                StepKind::Act { .. } => entry.acts += 1,
                StepKind::Walk { .. } => entry.walks += 1,
            }
            entry.planned_ticks += u64::from(step.end.saturating_sub(step.start));
        }
        let makespan = u64::from(schedule.makespan);
        for entry in per_bot.values_mut() {
            entry.idle_ticks = makespan.saturating_sub(entry.planned_ticks);
        }
        let bots: Vec<BotReport> = per_bot.into_values().collect();
        let planned_ticks: u64 = bots.iter().map(|bot| bot.planned_ticks).sum();
        let available = makespan * bots.len() as u64;
        let utilisation_percent = if available == 0 {
            None
        } else {
            Some(planned_ticks as f64 * 100. / available as f64)
        };
        PlanReport {
            actions: net.len(),
            makespan: schedule.makespan,
            bots,
            planned_ticks,
            utilisation_percent,
        }
    }

    /// The report as lines a person reads, one per line, no trailing newline.
    ///
    /// Rendering lives here rather than at the call site so that the offline
    /// planner and anything else that grows one — seed scoring is next — print
    /// the same shape, and so that the shape is testable without a CLI.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("actions        {}", self.actions),
            format!(
                "makespan       {} ticks ({})",
                self.makespan,
                crate::render::ticks_to_timestamp(self.makespan)
            ),
        ];
        match self.utilisation_percent {
            Some(percent) => out.push(format!(
                "utilisation    {percent:.1}% of {} bot-ticks",
                self.makespan as u64 * self.bots.len() as u64
            )),
            None => out.push("utilisation    n/a (nothing to do)".to_string()),
        }
        out.push("bot   steps  acts  walks  planned  idle".to_string());
        for bot in &self.bots {
            out.push(format!(
                "{:<5} {:>5}  {:>4}  {:>5}  {:>7}  {:>6}",
                bot.bot.0, bot.steps, bot.acts, bot.walks, bot.planned_ticks, bot.idle_ticks
            ));
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::goal::{Goal, Holder};
    use crate::method::expand;
    use crate::method::have::registry_for;
    use crate::schedule::schedule;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn report_for(bots: &[BotId]) -> PlanReport {
        let state = PlanState::from_world(Arc::new(fixture_world()), bots);
        let goal = Goal::Have {
            item: "automation-science-pack".into(),
            count: 4,
            whose: Holder::Anyone,
        };
        let net = expand(&[goal], &state, &registry_for(bots), bots[0]).expect("expands");
        let plan = schedule(&net, &state, bots).expect("schedules");
        PlanReport::of(&net, &plan, bots)
    }

    #[test]
    fn every_action_is_an_act_somewhere() {
        let report = report_for(&[BotId(1), BotId(2)]);
        let acts: usize = report.bots.iter().map(|bot| bot.acts).sum();
        assert_eq!(
            acts, report.actions,
            "an action went missing between the network and the schedule"
        );
    }

    /// A bot with nothing to do is the finding, so it has a row.
    ///
    /// The reference run gave bots 2, 3 and 4 three actions each out of 103.
    /// A report built only from the bots that appear in the schedule would
    /// have printed a tidy roster of one.
    #[test]
    fn a_bot_given_nothing_still_has_a_row() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let report = report_for(&bots);
        assert_eq!(report.bots.len(), 4);
        for bot in &report.bots {
            assert_eq!(
                bot.idle_ticks + bot.planned_ticks,
                u64::from(report.makespan),
                "bot {} accounts for something other than the whole plan",
                bot.bot.0
            );
        }
    }

    #[test]
    fn walks_are_counted_apart_from_acts() {
        let report = report_for(&[BotId(1)]);
        let bot = &report.bots[0];
        assert_eq!(bot.steps, bot.acts + bot.walks);
        assert!(bot.walks > 0, "this plan reaches an ore field on foot");
    }

    /// An empty roster is not 0% utilised, it is unanswerable.
    #[test]
    fn utilisation_is_absent_rather_than_zero_when_there_is_nothing_to_divide_by() {
        let report = PlanReport::of(&ActionNetwork::new(), &Schedule::default(), &[]);
        assert_eq!(report.utilisation_percent, None);
        assert!(
            report.lines().iter().any(|line| line.contains("n/a")),
            "a reader is told, not shown a zero"
        );
    }

    #[test]
    fn the_rendered_report_names_every_bot() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let lines = report_for(&bots).lines();
        // Three fixed lines, a header, then one per bot.
        assert_eq!(lines.len(), 4 + bots.len());
    }
}
