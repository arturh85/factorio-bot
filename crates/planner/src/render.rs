//! Mermaid Gantt and graphviz output for schedules and action networks.

use crate::ids::{BotId, Ticks};
use crate::network::ActionNetwork;
use crate::schedule::{Schedule, StepKind};
use std::collections::BTreeSet;
use std::fmt::Write;

const TICKS_PER_SECOND: Ticks = 60;

pub fn ticks_to_timestamp(ticks: Ticks) -> String {
    let total_seconds = ticks / TICKS_PER_SECOND;
    format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60
    )
}

/// Make a label safe to interpolate into a Mermaid Gantt task line.
///
/// A task line is `label :id, start, duration` — comma and colon are its field
/// separators, so a label containing either splits the line and corrupts the
/// chart. This is not hypothetical: every walk label is `walk to [10, 20]`, and
/// `Position`'s `Display` puts a comma in the middle of it. Both characters
/// become a space, which reads correctly and cannot be misparsed.
fn gantt_label(label: &str) -> String {
    label.replace([',', ':'], " ")
}

/// A Mermaid Gantt chart, one section per bot.
pub fn mermaid_gantt(schedule: &Schedule, title: &str) -> String {
    let mut out = String::new();
    out.push_str("gantt\n");
    let _ = writeln!(out, "    title {}", title);
    out.push_str("    dateFormat HH:mm:ss\n");
    out.push_str("    axisFormat %H:%M:%S\n");

    let bots: BTreeSet<BotId> = schedule.steps.iter().map(|s| s.bot).collect();
    for (bot_index, bot) in bots.iter().enumerate() {
        let _ = writeln!(out, "    section {}", bot);
        for (step_index, step) in schedule.steps_for(*bot).iter().enumerate() {
            let label = match &step.what {
                StepKind::Act { label, .. } => label.clone(),
                StepKind::Walk { to } => format!("walk to {}", to),
            };
            // Round up, and never below one second: a 30-tick walk is a real
            // step and a `0s` bar is invisible in the rendered chart.
            let seconds = step
                .end
                .saturating_sub(step.start)
                .div_ceil(TICKS_PER_SECOND)
                .max(1);
            let _ = writeln!(
                out,
                "    {} :a{}, {}, {}s",
                gantt_label(&label),
                bot_index * 1000 + step_index + 1,
                ticks_to_timestamp(step.start),
                seconds
            );
        }
    }
    out
}

/// The action network as a graphviz digraph, edges labelled with their lag.
pub fn graphviz(net: &ActionNetwork) -> String {
    let mut out = String::from("digraph {\n");
    for action in net.actions() {
        let _ = writeln!(
            out,
            "    {} [label=\"{}\"];",
            action.id.0,
            action.label.replace('"', "'")
        );
    }
    for action in net.actions() {
        for (pred, lag) in net.preds(action.id) {
            let _ = writeln!(
                out,
                "    {} -> {} [label=\"{}t\"];",
                pred.0, action.id.0, lag
            );
        }
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ActionId;
    use crate::schedule::{Schedule, ScheduledStep, StepKind};
    use factorio_bot_core::types::Position;

    fn schedule() -> Schedule {
        Schedule {
            steps: vec![
                ScheduledStep {
                    what: StepKind::Walk {
                        to: Position::new(10., 0.),
                    },
                    bot: BotId(1),
                    start: 0,
                    end: 60,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: ActionId(0),
                        label: "mine 5 iron-ore".into(),
                    },
                    bot: BotId(1),
                    start: 60,
                    end: 360,
                },
                ScheduledStep {
                    what: StepKind::Act {
                        action: ActionId(1),
                        label: "craft iron-gear-wheel".into(),
                    },
                    bot: BotId(2),
                    start: 0,
                    end: 120,
                },
            ],
            makespan: 360,
        }
    }

    #[test]
    fn ticks_render_as_hours_minutes_seconds() {
        assert_eq!(ticks_to_timestamp(0), "00:00:00");
        assert_eq!(ticks_to_timestamp(60), "00:00:01");
        assert_eq!(ticks_to_timestamp(3600), "00:01:00");
        assert_eq!(ticks_to_timestamp(216_000), "01:00:00");
    }

    #[test]
    fn the_gantt_has_a_section_per_bot() {
        let out = mermaid_gantt(&schedule(), "Test");
        assert!(out.starts_with("gantt"));
        assert!(out.contains("title Test"));
        assert!(out.contains("section bot 1"));
        assert!(out.contains("section bot 2"));
    }

    #[test]
    fn the_gantt_names_every_action_and_walk() {
        let out = mermaid_gantt(&schedule(), "Test");
        assert!(out.contains("mine 5 iron-ore"));
        assert!(out.contains("craft iron-gear-wheel"));
        // The comma in the position is replaced: it would otherwise split the
        // Mermaid task line.
        assert!(out.contains("walk to [10  0]"));
    }

    #[test]
    fn the_gantt_places_steps_at_their_start_time() {
        let out = mermaid_gantt(&schedule(), "Test");
        // Bot 1's walk is its first step (a1), the mining act its second (a2):
        // ids are `bot_index * 1000 + step_index + 1`.
        assert!(
            out.contains("walk to [10  0] :a1, 00:00:00, 1s"),
            "unexpected gantt body:\n{}",
            out
        );
        // The mining step starts one second in and runs for five.
        assert!(
            out.contains("mine 5 iron-ore :a2, 00:00:01, 5s"),
            "unexpected gantt body:\n{}",
            out
        );
        // Bot 2's only step opens a fresh id block.
        assert!(
            out.contains("craft iron-gear-wheel :a1001, 00:00:00, 2s"),
            "unexpected gantt body:\n{}",
            out
        );
    }

    #[test]
    fn a_short_step_still_gets_a_visible_bar() {
        let schedule = Schedule {
            steps: vec![ScheduledStep {
                what: StepKind::Walk {
                    to: Position::new(1., 0.),
                },
                bot: BotId(1),
                start: 0,
                end: 30,
            }],
            makespan: 30,
        };
        let out = mermaid_gantt(&schedule, "Test");
        // Integer division would render this half-second walk as `0s`.
        assert!(out.contains(", 1s"), "unexpected gantt body:\n{}", out);
    }

    #[test]
    fn a_label_cannot_break_the_task_line() {
        let schedule = Schedule {
            steps: vec![ScheduledStep {
                what: StepKind::Act {
                    action: ActionId(0),
                    label: "craft 1 gear, then: rest".into(),
                },
                bot: BotId(1),
                start: 0,
                end: 60,
            }],
            makespan: 60,
        };
        let out = mermaid_gantt(&schedule, "Test");
        assert!(
            out.contains("craft 1 gear  then  rest :a1, 00:00:00, 1s"),
            "unexpected gantt body:\n{}",
            out
        );
    }

    #[test]
    fn an_empty_schedule_still_renders_a_valid_chart() {
        let out = mermaid_gantt(&Schedule::default(), "Empty");
        assert!(out.starts_with("gantt"));
        assert!(out.contains("title Empty"));
    }

    #[test]
    fn the_gantt_orders_sections_by_bot() {
        let out = mermaid_gantt(&schedule(), "Test");
        let first = out.find("section bot 1").expect("bot 1 section");
        let second = out.find("section bot 2").expect("bot 2 section");
        assert!(first < second, "sections must be ordered by bot id");
    }

    #[test]
    fn graphviz_renders_nodes_and_lag_labelled_edges() {
        use crate::action::{Action, ActionKind};
        use crate::ids::ActionIdGen;
        use crate::network::ActionNetwork;

        fn node(gen: &mut ActionIdGen, label: &str) -> Action {
            Action {
                id: gen.next(),
                kind: ActionKind::Craft {
                    item: "iron-gear-wheel".into(),
                    count: 1,
                },
                pre: vec![],
                eff: vec![],
                duration: 60,
                pinned: None,
                label: label.into(),
            }
        }

        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a = net.add(node(&mut gen, "insert ore"));
        let b = net.add(node(&mut gen, "remove \"plate\""));
        net.link(a, b, 192);

        let out = graphviz(&net);
        assert!(out.starts_with("digraph {\n"));
        assert!(out.ends_with("}\n"));
        assert!(out.contains("0 [label=\"insert ore\"];"));
        // A double quote in a label becomes a single quote so the DOT stays well formed.
        assert!(out.contains("1 [label=\"remove 'plate'\"];"));
        // The edge carries its lag, and only in the direction it was linked.
        assert!(out.contains("0 -> 1 [label=\"192t\"];"));
        assert!(!out.contains("1 -> 0"));
    }
}
