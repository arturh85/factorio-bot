//! What we want, stated declaratively and without reference to any bot.

use crate::ids::{BotId, ItemId};

/// Who must end up holding the items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Holder {
    /// Satisfied by the sum across every bot. This is what makes multi-bot
    /// gathering parallel: the count can be split into independent per-bot
    /// chains that never coordinate.
    Anyone,
    /// One named bot's inventory.
    Bot(BotId),
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Holder::Anyone => write!(f, "anyone"),
            Holder::Bot(id) => write!(f, "{}", id),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
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
}
