//! Reading a world-model divergence off a transfer's verdict.
//!
//! The mod's two transfer handlers (`rcon_insert_to_inventory` /
//! `rcon_remove_from_inventory` in `mods/BotBridge/control.lua`) do not report
//! results; they `complain`, and only when the count that moved is not the
//! count that was asked for. `judge_transfer_reply`
//! (`crates/core/src/factorio/rcon.rs`) turns any surviving complaint into a
//! refusal, so the whole line reaches [`crate::log::Attempt::error`] wrapped as
//! `game rejected the command: Unexpected Response: ["..."]`. Three wordings:
//!
//! ```text
//! tried to remove 64 iron-plate but removed 40
//! tried to insert 17x coal but inserted 3
//! cannot insert 20x iron-ore, because player #1 only has 18. clamping...
//! ```
//!
//! Every one of them says the same thing: **the plan believed a container held
//! something it does not.** That is not a transient. The items that did move
//! are already in the bot's hands (or already in the chest), the container now
//! holds even less than it did, and issuing the identical command against the
//! identical container can only come back with a smaller number.
//!
//! This parser lives in the executor rather than beside the record's
//! classifier (`crates/scripting_lua/src/globals/record.rs`, which now calls
//! it) because `recover` needs the answer and the executor cannot see the
//! record. `ExecutionLog` carried the verdict's *text* all along; what it did
//! not carry was any way to read the *class* off it without a second copy of
//! the wording. This is that way, and there is one copy.
//!
//! A fourth wording is deliberately not here. An `insert` whose destination
//! was full (`tried to insert ... (destination holds H, room for R)`) is judged
//! a *success* upstream and reaches the log as a note on a `Status::Success`,
//! so it never comes back from `recover` at all; see `judge_transfer_reply`.

/// A transfer that moved fewer items than the plan asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The item the transfer named.
    pub item: String,
    /// How many the plan asked to move.
    pub asked: u64,
    /// How many actually moved. Zero is the common second act: a retry of the
    /// identical take against the container the first attempt emptied.
    pub moved: u64,
}

/// Reads a [`Divergence`] out of a transfer's verdict, or `None` if the text
/// is not one of the three wordings above.
///
/// Parsed rather than pattern-matched wholesale so a wording change costs a
/// `None` -- the failure then reads as an ordinary refusal, which is what it
/// read as before this existed -- rather than a wrong number. Nothing here
/// invents a count: both must parse as integers or this declines.
///
/// The first two wordings are checked before the third because they describe
/// what the transfer *did*, while the clamp line describes what it decided to
/// attempt; a reply that clamped and then fell short again prints both, and
/// the transfer line is the one whose numbers are the outcome.
pub fn divergence(error: &str) -> Option<Divergence> {
    fn digits(text: &str) -> Option<u64> {
        let head: &str = text.split(|c: char| !c.is_ascii_digit()).next()?;
        head.parse().ok()
    }
    fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<(&'a str, &'a str)> {
        text.split_once(open)?.1.split_once(close)
    }
    let found = |item: &str, asked, moved| {
        Some(Divergence {
            item: item.to_string(),
            asked,
            moved,
        })
    };

    // `tried to remove <asked> <item> but removed <moved>`
    if let Some((head, tail)) = between(error, "tried to remove ", " but removed ")
        && let Some((asked, item)) = head.split_once(' ')
        && let (Some(asked), Some(moved)) = (digits(asked), digits(tail))
    {
        return found(item, asked, moved);
    }
    // `tried to insert <asked>x <item> but inserted <moved>`
    if let Some((head, tail)) = between(error, "tried to insert ", " but inserted ")
        && let Some((asked, item)) = head.split_once("x ")
        && let (Some(asked), Some(moved)) = (digits(asked), digits(tail))
    {
        return found(item, asked, moved);
    }
    // `cannot insert <asked>x <item>, because player #<id> only has <moved>.
    //  clamping...` -- the insert did happen, at the clamped count.
    if let Some((head, tail)) = between(error, "cannot insert ", ", because ")
        && tail.contains("clamping")
        && let Some((asked, item)) = head.split_once("x ")
        && let Some(have) = tail.split_once("only has ")
        && let (Some(asked), Some(moved)) = (digits(asked), digits(have.1))
    {
        return found(item, asked, moved);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(item: &str, asked: u64, moved: u64) -> Option<Divergence> {
        Some(Divergence {
            item: item.into(),
            asked,
            moved,
        })
    }

    /// The two lines of `run-1788552801-73005`, batch 3, verbatim: the take
    /// that moved 40 of 64, and the retry that moved 0 of 64 against the cell
    /// the first attempt had emptied.
    #[test]
    fn the_take_and_its_doomed_retry_both_read_as_divergence() {
        assert_eq!(
            divergence(
                r#"game rejected the command: Unexpected Response: ["tried to remove 64 iron-plate but removed 40"]"#
            ),
            d("iron-plate", 64, 40)
        );
        assert_eq!(
            divergence(
                r#"game rejected the command: Unexpected Response: ["tried to remove 64 iron-plate but removed 0"]"#
            ),
            d("iron-plate", 64, 0)
        );
    }

    #[test]
    fn the_insert_wordings_read_the_same_way() {
        assert_eq!(
            divergence(r#"["tried to insert 50x coal but inserted 12"]"#),
            d("coal", 50, 12)
        );
        assert_eq!(
            divergence(
                r#"["cannot insert 20x iron-ore, because player #1 only has 18. clamping..."]"#
            ),
            d("iron-ore", 20, 18)
        );
    }

    #[test]
    fn anything_else_is_not_a_divergence() {
        for text in [
            "game rejected the command: cannot insert to inventory of nonexisting entity",
            "tried to remove some iron-plate but removed fewer",
            "no action result received in time",
            "",
        ] {
            assert_eq!(divergence(text), None, "{text:?}");
        }
    }
}
