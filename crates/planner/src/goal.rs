//! What we want, stated declaratively and without reference to any bot.

use crate::ids::{BotId, ItemId};
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
    /// Sized against this bot's starting inventory, but carrying no commitment
    /// about *who* runs it: the driver opens a chain over a share's subtree,
    /// so its actions are welded to a single runner, and gives that chain no
    /// owner, so which bot that is stays the scheduler's decision.
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
    /// inventory separate. That it *works* rests on bots starting
    /// interchangeable — the assumption `ExpansionCtx` documents, made visible
    /// in the type rather than left in prose.
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    Have {
        item: ItemId,
        count: u32,
        whose: Holder,
    },
    Researched(String),
    /// The functorio bridge: `BusLane item rate` transcribed into Rust. No
    /// method satisfies this yet; blueprint generation is a later increment.
    Producing {
        item: ItemId,
        rate: f64,
    },
    All(Vec<Goal>),
}

impl std::fmt::Display for Goal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Goal::Have { item, count, whose } => write!(f, "have {} {} ({})", count, item, whose),
            Goal::Researched(tech) => write!(f, "research {}", tech),
            Goal::Producing { item, rate } => write!(f, "produce {} {}/min", rate, item),
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
        };
        let b = Goal::Have {
            item: "iron-plate".into(),
            count: 2,
            whose: Holder::Anyone,
        };
        let c = Goal::Have {
            item: "iron-plate".into(),
            count: 3,
            whose: Holder::Anyone,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
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
            },
            Goal::Researched("automation".into()),
            Goal::Producing {
                item: "iron-plate".into(),
                rate: 30.0,
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
        };
        assert_eq!(g.to_string(), "have 4 coal (bot 2)");
        let g = Goal::Have {
            item: "coal".into(),
            count: 4,
            whose: Holder::Anyone,
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
}
