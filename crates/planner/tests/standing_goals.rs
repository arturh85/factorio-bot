//! The semantics of the fifth goal kind, pinned before it exists.
//!
//! Design note: `docs/superpowers/notes/2026-09-06-standing-goals.md`.
//!
//! `Have`, `Researched`, `Produced`, `Producing`, `Extracted` and `Built` are
//! all **one-shot or structural**. Nothing in the vocabulary means *keep this
//! true*, which is the cause of which every measured run's plateau is the
//! symptom: production stops at exactly the plan's bill. `Goal::Sustain` is
//! the missing kind, and these are the four claims its implementation has to
//! satisfy.
//!
//! # These were `#[ignore]`d until the kind existed
//!
//! They failed with the exact sentence *"unknown variant `Sustain`"*, and a red
//! `cargo test -p factorio-bot-planner` would have been a red suite for every
//! other agent working in this checkout, so they were kept visible and named in
//! the default run (`4 ignored`) and runnable on demand. The `#[ignore]`s came
//! off with `Goal::Sustain` itself, which is what the design note said to do;
//! they now run by default and this comment is the record of why they did not.
//!
//! # Why the goals are built from JSON rather than written out
//!
//! `Goal::Sustain { .. }` does not compile today, and a test file that does not
//! compile takes the whole crate's test binary with it — no other test in the
//! crate could run. Deserialising the variant instead keeps this file
//! compiling against the tree that has the defect, fails with the exact
//! sentence *"unknown variant `Sustain`"*, and — once the variant lands —
//! pins the **wire shape** as well: the variant's name, its three field names
//! and their types, all of which reach a Lua caller and the HTTP API.

use factorio_bot_core::serde_json;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::{BotId, PlanState, expand, holds, registry_for};
use std::sync::Arc;

/// The goal this design proposes, by its wire shape.
///
/// `window_ticks` is `Ticks` (`u32`), like every duration in this crate, and
/// has **no default** — the same reasoning as `supervisor.witness`'s
/// `within_ticks`, which also refuses one: the window is the number that
/// decides what a failure means, and a library that guessed it would hand back
/// a verdict nobody derived.
fn sustain(item: &str, per_minute: u32, window_ticks: u32) -> Goal {
    let json = format!(
        r#"{{"Sustain":{{"item":"{item}","per_minute":{per_minute},"window_ticks":{window_ticks}}}}}"#
    );
    serde_json::from_str(&json).unwrap_or_else(|e| {
        panic!("Goal::Sustain is not a goal kind yet, so this design is not implemented: {e}")
    })
}

fn state() -> PlanState {
    PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
}

/// A goal that is true of every state, used to prove that a bundle's answer
/// comes from the sustain member and not from an unsatisfied sibling.
fn trivially_true() -> Goal {
    Goal::Have {
        item: "iron-plate".into(),
        count: 0,
        whose: Holder::Anyone,
    }
}

/// The shape: an item, a rate, and the window the rate must hold over.
///
/// Three fields and not two. A rate with no window is what `Goal::Producing`
/// already is — a claim about how much *capacity* stands — and it is satisfied
/// by a cell that has never produced anything. The window is what makes the
/// claim durative, and it is the field the verification reads.
#[test]
fn sustain_names_an_item_a_rate_and_a_window() {
    let goal = sustain("iron-plate", 15, 7200);
    let back: Goal = serde_json::from_str(&serde_json::to_string(&goal).expect("serialises"))
        .expect("round trip");
    assert_eq!(back, goal, "a goal crosses the Lua and HTTP boundaries");
    assert_eq!(
        goal.to_string(),
        "sustain 15 iron-plate/min over 7200 ticks",
        "the rate and the window both have to appear, or a diagnostic cannot \
         tell two sustain goals of the same item apart"
    );
}

/// **The semantic decision of this design.** `holds` is three-valued, and a
/// standing rate is the third value.
///
/// `Some(true)` would be the lie the whole design exists to stop: a
/// structurally satisfied cell has been shown, live, to produce nothing at all
/// — an empty fuel slot, a backed-up output, a patch mined out — and
/// `AlreadySatisfied` is registered ahead of every other method, so a
/// `Some(true)` here means an empty plan and a goal that claims itself.
/// `Some(false)` would be its own lie: it asserts the goal is *unmet*, which
/// nothing in `PlanState` can know either.
///
/// `None` is the honest answer and it already has this exact meaning in this
/// crate — `Goal::Produced` and `Goal::Extracted` return it. It says: this
/// goal names something no reading of the world can settle. For `Produced`
/// that is an event; for `Sustain` it is a **window of history**, which a pure
/// planner with no clock and no I/O is constitutionally unable to observe.
#[test]
fn the_planner_cannot_answer_a_sustain_goal_from_the_world() {
    assert_eq!(
        holds(&sustain("iron-plate", 15, 7200), &state()),
        None,
        "satisfaction of a rate is a fact about a window of history, and this \
         crate has no clock to read one with"
    );
}

/// And the `None` propagates, so no caller can round a bundle up.
///
/// `Goal::All`'s rule is already "false beats None"; this pins the other half —
/// a bundle whose every other member holds is still `None` while a sustain is
/// in it. A supervisor that reported such a ladder complete would be closing a
/// milestone on the strength of the goals that were *not* the point of it.
#[test]
fn a_bundle_containing_a_sustain_is_never_claimed_satisfied() {
    let state = state();
    assert_eq!(
        holds(&trivially_true(), &state),
        Some(true),
        "control: the sibling really is satisfied"
    );
    assert_eq!(
        holds(
            &Goal::All(vec![trivially_true(), sustain("iron-plate", 15, 7200)]),
            &state
        ),
        None,
    );
}

/// The consequence of `None`, and the obligation it puts on the method.
///
/// Because nothing can report a sustain goal already satisfied, every
/// expansion — including every replan — must produce *something*: either a
/// plan (the arrangement, or the part of it not yet standing) or a refusal
/// naming what is missing. An empty network is the one answer that must never
/// come back, because an empty network is how this planner says "done", and
/// "done" is precisely what it cannot know here.
///
/// The other half of that obligation is not testable from outside and is
/// stated in the note: the expansion has to be **idempotent against a world
/// that already has the arrangement**, re-deriving what is not yet standing
/// the way `Goal::Built` does, or every replan builds a second cell beside the
/// first. `tests/standing_site_reuse.rs` is the precedent and the warning.
#[test]
fn a_sustain_goal_is_never_already_satisfied() {
    let state = state();
    let roster = [BotId(1)];
    // A refusal is a legitimate answer -- the fixture world may have no patch a
    // cell can stand on -- as long as it is a refusal and not silence. An `Ok`
    // is the case with something to check.
    if let Ok(net) = expand(
        &[sustain("iron-plate", 15, 7200)],
        &state,
        &registry_for(&roster),
        BotId(1),
    ) {
        assert!(
            !net.is_empty(),
            "an empty network is this planner's word for `done`, and a standing \
             rate is exactly what it cannot know is done"
        );
    }
}
