//! The committed `Replay` documents the browser view is pinned to.
//!
//! # Why a file and not just a unit test
//!
//! `Replay` is deliberately **absent from the OpenAPI spec**: the server
//! carries the document opaquely so `crates/server` need not depend on
//! `crates/executor`. That is the right call for the dependency graph and it
//! leaves a hole — `app/src/api/openapi.contract.spec.ts` checks the browser's
//! hand-written types against `app/src/api/openapi.snapshot.json`, and `Replay`
//! is not in that document, so nothing on either side of the wire checks a
//! `Replay` declaration against the Rust type it mirrors.
//!
//! A hand-written TypeScript mirror of a Rust type, verified by no one, is the
//! exact defect the OpenAPI snapshot exists to prevent. So this applies the
//! same remedy in the one place the spec cannot reach: a **tracked artifact
//! both sides derive from**.
//!
//! * This test keeps the committed files equal to what [`Replay::new`] really
//!   produces, so they cannot go stale.
//! * The consumer checks its declarations against the committed files, so its
//!   declarations cannot drift from the producer.
//!
//! Either half alone is worthless, exactly as in `crates/server/tests/openapi.rs`.
//! Hand-edit a fixture and this test goes red; change `Replay` and regenerate
//! without telling the consumer and *their* test goes red. **The two cannot
//! both pass unless the producer, the fixture and the consumer all say the same
//! thing.**
//!
//! # Why two documents, and why the hard one is hard
//!
//! A happy-path document pins only the fields that are always present. Every
//! interesting distinction in this type lives in a value that a successful,
//! fully-measured run never produces: a `null` observation, a `Lost` that is
//! not a `Failed`, a `Believed` that is not a `Measured`, a retry count above
//! one. Committing a document without them would pin the *shape* and leave the
//! *distinctions* to the consumer's reading of the enum — which is what a
//! renderer gets wrong.
//!
//! So `replay.snapshot.json` carries all of them at once, and
//! [`the_committed_snapshot_is_still_a_hard_case`] asserts each one is there.
//! That assertion is what keeps the fixture hard: without it, the next
//! regeneration of a softened producer quietly turns the hard case into a happy
//! one and every test still passes.
//!
//! `replay.refused.snapshot.json` is the second half of the one pair a renderer
//! must caption differently — *we attempted this and measured nothing* versus
//! *we never started, because X*. Both documents are the whole schedule with
//! nothing observed; only [`Replay::refused`] separates them. Committing both
//! means a change that collapses the distinction fails here, and not only in
//! the consumer's styling.

use factorio_bot_core::types::Position;
use factorio_bot_executor::log::{ExecutionLog, Status};
use factorio_bot_executor::replay::Replay;
use factorio_bot_executor::ActionTicks;
use factorio_bot_planner::schedule::{Schedule, ScheduledStep, StepKind};
use factorio_bot_planner::{ActionId, BotId, Ticks};
use serde_json::Value;

// ---------------------------------------------------------------------------
// The committed files.
// ---------------------------------------------------------------------------

/// The hard case: a run that was attempted and went several different kinds of
/// wrong. See the module docs for what "hard" is required to mean.
const ATTEMPTED_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/replay.snapshot.json");

/// The other half of the pair: a run that never started.
const REFUSED_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/replay.refused.snapshot.json"
);

/// Printed on every failure below, because "the fixture is out of date" is only
/// half the instruction. The other half is that a moved field is a **consumer**
/// change: the fixture is the seam, and re-blessing it without touching the
/// declarations on the other side just moves the lie one file along.
const REGENERATE_HINT: &str = "\n\nRegenerating is a deliberate act with a visible diff:\n    \
     UPDATE_REPLAY_SNAPSHOT=1 cargo test -p factorio-bot-executor --test replay_snapshot\n    \
     git diff --no-ext-diff crates/executor/tests/replay.snapshot.json \\\n        \
     crates/executor/tests/replay.refused.snapshot.json\n\
     Whatever moved has to be mirrored in the consumer's hand-written Replay,\n\
     ReplayStep, Evidence and ReplayStepKind declarations -- the fixture is the\n\
     only thing that checks them, so a re-blessed fixture nobody mirrored is a\n\
     silently broken view.";

/// Whether this run was asked to rewrite the committed fixtures instead of
/// checking them -- and a hard stop if that request arrives from CI.
///
/// Same rule, same reason, as `crates/server/tests/openapi.rs`. At a
/// developer's terminal the point is to see the diff before committing it. In
/// an automated run it would be a disaster of the quiet kind: the job would
/// rewrite two tracked files, report `ok`, and agree with whatever the producer
/// had just started emitting. No job in this repo sets it today; this makes
/// sure that stays a fact rather than a hope.
///
/// It panics rather than ignoring the variable and checking anyway, because a
/// CI job that sets it is misconfigured, and a misconfiguration that silently
/// does the right thing is one nobody fixes.
fn snapshot_update_requested() -> bool {
    let requested = std::env::var_os("UPDATE_REPLAY_SNAPSHOT").is_some();
    assert!(
        !(requested && std::env::var_os("CI").is_some()),
        "UPDATE_REPLAY_SNAPSHOT is set in a CI environment.\n\
         Regenerating a committed replay fixture is a deliberate local act with a\n\
         reviewed diff; doing it automatically would re-bless every producer change\n\
         and report a green run. Unset UPDATE_REPLAY_SNAPSHOT in the CI environment,\n\
         and regenerate locally instead:{REGENERATE_HINT}"
    );
    requested
}

fn read_fixture(path: &str) -> Value {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("cannot read the committed replay fixture at {path}: {err}{REGENERATE_HINT}")
    });
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("{path} is not valid JSON: {err}{REGENERATE_HINT}"))
}

// ---------------------------------------------------------------------------
// The inputs. The fixtures are produced from these by `Replay::new` and are
// never hand-authored -- a hand-written JSON file would be a third mirror and
// would defeat the whole point of having one.
// ---------------------------------------------------------------------------

fn act(bot: u8, id: u32, label: &str, start: Ticks, end: Ticks) -> ScheduledStep {
    ScheduledStep {
        what: StepKind::Act {
            action: ActionId(id),
            label: label.to_string(),
        },
        bot: BotId(bot),
        start,
        end,
    }
}

fn walk(bot: u8, x: f64, y: f64, start: Ticks, end: Ticks) -> ScheduledStep {
    ScheduledStep {
        what: StepKind::Walk {
            to: Position::new(x, y),
            radius: 3.0,
        },
        bot: BotId(bot),
        start,
        end,
    }
}

/// A run that was attempted and went wrong in every distinguishable way at
/// once.
///
/// Every row below exists to pin one distinction a renderer has to draw, and no
/// two rows pin the same one:
///
/// | row | pins |
/// |-----|------|
/// | 0 | a walk with both ticks observed, and `Evidence::Believed` |
/// | 1 | an act with both ticks observed, and `Evidence::Measured` |
/// | 2 | `attempt_number > 1`, `Status::Failed`, half a measurement, an error |
/// | 3 | `Status::Lost` -- distinct from `Failed`, on a walk |
/// | 4 | `Status::Pending` -- never dispatched, `attempt_number` absent |
/// | 5 | `Status::Success` with **both** observations `null` |
/// | 6 | `Status::Running` -- in flight, no reply tick yet |
///
/// plus one walk observation belonging to no scheduled step, so
/// `unmatched_walks` is pinned non-empty and the consumer's element type is
/// checked against a real row rather than against `[]`.
fn attempted() -> (Schedule, ExecutionLog) {
    let schedule = Schedule {
        steps: vec![
            walk(1, 10.0, 20.0, 0, 70),
            act(1, 0, "mine iron-ore", 70, 190),
            act(1, 1, "craft iron-gear-wheel", 190, 250),
            walk(2, 40.0, 5.0, 0, 120),
            act(2, 2, "place stone-furnace", 120, 150),
            act(2, 3, "insert coal", 150, 180),
            act(2, 4, "research automation", 180, 240),
        ],
        makespan: 250,
    };

    let mut log = ExecutionLog::default();

    // Row 0. A walk the game stamped at both ends. Measured ticks, believed
    // arrival -- the two halves of a walk row, and the reason `Evidence` is a
    // field rather than something the consumer infers from `kind == "walk"`.
    log.start_walk(BotId(1), 0, Position::new(10.0, 20.0), 0, 70);
    log.observe_walk(BotId(1), 0, ActionTicks::new(Some(100), Some(205)));
    log.succeed_walk(BotId(1), 0);

    // Row 1. A fully measured action on its first attempt: the boring row, kept
    // so the interesting ones have something to be unlike.
    log.start(ActionId(0), 70);
    log.observe(ActionId(0), ActionTicks::new(Some(205), Some(330)));
    log.succeed(ActionId(0), 190);

    // Row 2. Third attempt, dispatched and then refused. The game stamped the
    // dispatch and never the reply, so half the measurement survives and the
    // other half must stay `null` rather than being invented from the half that
    // is there.
    log.start(ActionId(1), 190);
    log.fail(ActionId(1), 250, "not enough iron-plate".to_string());
    log.start(ActionId(1), 190);
    log.fail(ActionId(1), 250, "not enough iron-plate".to_string());
    log.start(ActionId(1), 190);
    log.observe(ActionId(1), ActionTicks::new(Some(340), None));
    log.fail(ActionId(1), 250, "not enough iron-plate".to_string());

    // Row 3. Dispatched, and this run will never learn the outcome. `Lost` is
    // not `Failed`: nothing went wrong, the thread was dropped.
    log.start_walk(BotId(2), 0, Position::new(40.0, 5.0), 0, 120);
    log.observe_walk(BotId(2), 0, ActionTicks::new(Some(100), None));
    log.lose_track_walk(BotId(2), 0, "run ended holding the dispatch");

    // Row 4 (`ActionId(2)`). Never dispatched at all -- deliberately no entry
    // in the log, which is what makes the row `Pending` with no attempt number.

    // Row 5. Succeeded with no clock. Nothing went wrong and nothing was
    // learned about when: a green row whose observations are both `null`, which
    // is a different fact from row 4's identical-looking pair.
    log.start(ActionId(3), 150);
    log.observe(ActionId(3), ActionTicks::UNKNOWN);
    log.succeed(ActionId(3), 180);

    // Row 6. Still in flight. Dispatched and stamped, no verdict yet -- and
    // `Running` must render as a bot at work, where row 3's `Lost` must not.
    log.start(ActionId(4), 180);
    log.observe(ActionId(4), ActionTicks::new(Some(400), None));

    // A walk observation no scheduled step claims: bot 3 is not in this
    // schedule at all. Reported rather than dropped, so pairing a log with a
    // schedule it did not come from is visible instead of silently lossy.
    log.start_walk(BotId(3), 0, Position::new(-5.0, 12.5), 0, 40);
    log.succeed_walk(BotId(3), 0);

    (schedule, log)
}

/// The run that never started.
///
/// The same shape of schedule -- both step kinds, several bots -- against an
/// **empty** log, so every row is `Pending` and every observation is `null`.
/// The only thing separating this document from a run that dispatched
/// everything and learned nothing is [`Replay::refused`], which is the entire
/// reason that field exists.
fn refused() -> (Schedule, ExecutionLog) {
    let schedule = Schedule {
        steps: vec![
            walk(1, 10.0, 20.0, 0, 70),
            act(1, 0, "mine iron-ore", 70, 190),
            walk(2, 40.0, 5.0, 0, 120),
            act(2, 2, "place stone-furnace", 120, 150),
        ],
        makespan: 190,
    };
    (schedule, ExecutionLog::default())
}

const REFUSAL: &str =
    "the wait graph has a cycle: s1 waits on s3, which waits on s1; nothing was dispatched";

// ---------------------------------------------------------------------------
// Comparison.
// ---------------------------------------------------------------------------

/// Every leaf on which two documents disagree, as a JSON pointer with both
/// values.
///
/// Naming the field is the whole job. A snapshot test that reports "the
/// documents differ" and dumps two blobs gets regenerated blindly, which is
/// indistinguishable from having no test.
fn differences(committed: &Value, produced: &Value) -> Vec<String> {
    fn walk_values(path: &str, committed: &Value, produced: &Value, out: &mut Vec<String>) {
        match (committed, produced) {
            (Value::Object(a), Value::Object(b)) => {
                for (key, av) in a {
                    match b.get(key) {
                        Some(bv) => walk_values(&format!("{path}/{key}"), av, bv, out),
                        None => out.push(format!(
                            "  - {path}/{key} is in the fixture but no longer produced (was {av})"
                        )),
                    }
                }
                for (key, bv) in b {
                    if !a.contains_key(key) {
                        out.push(format!(
                            "  + {path}/{key} is produced but missing from the fixture (is {bv})"
                        ));
                    }
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    out.push(format!(
                        "  ~ {path} has {} entries in the fixture, {} produced",
                        a.len(),
                        b.len()
                    ));
                }
                for (i, (av, bv)) in a.iter().zip(b.iter()).enumerate() {
                    walk_values(&format!("{path}/{i}"), av, bv, out);
                }
            }
            (a, b) if a == b => {}
            (a, b) => out.push(format!("  ~ {path}: fixture has {a}, producer says {b}")),
        }
    }

    let mut out = Vec::new();
    walk_values("", committed, produced, &mut out);
    out
}

/// Check one committed fixture against one freshly produced document, or
/// rewrite it when explicitly asked to.
fn check_fixture(path: &str, produced: &Replay) {
    let produced = serde_json::to_value(produced).expect("a Replay serialises to JSON");

    // Opt-in regeneration, never automatic: a fixture the build rewrites on its
    // own is a fixture that agrees with every change, including the ones that
    // break the view.
    if snapshot_update_requested() {
        let mut rendered =
            serde_json::to_string_pretty(&produced).expect("the document serialises back to JSON");
        rendered.push('\n');
        // Written through a temporary and renamed into place, because
        // `the_committed_snapshot_is_still_a_hard_case` reads the same file
        // from another thread of the same run. A plain `write` would let it
        // read a half-written document and report a fixture that is not hard
        // when the truth is only that it was mid-flight.
        let temporary = format!("{path}.new");
        std::fs::write(&temporary, rendered)
            .unwrap_or_else(|err| panic!("cannot write {temporary}: {err}"));
        std::fs::rename(&temporary, path)
            .unwrap_or_else(|err| panic!("cannot move {temporary} onto {path}: {err}"));
        eprintln!("wrote {path}; review the diff before committing it");
        return;
    }

    let committed = read_fixture(path);
    let diff = differences(&committed, &produced);
    assert!(
        diff.is_empty(),
        "the committed replay fixture no longer matches what Replay::new produces.\n\
         {path}\n{}{REGENERATE_HINT}",
        diff.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The guards.
// ---------------------------------------------------------------------------

/// The guard that makes the fixture worth having: rename a `ReplayStep` field,
/// retag `Evidence`, change how an unobserved tick serialises, and this fails
/// **by name** here -- in the same `cargo test` run that made the change
/// compile, and before the consumer's hand-written types silently stop
/// describing reality.
#[test]
fn the_committed_replay_snapshot_matches_what_replay_new_produces() {
    let (schedule, log) = attempted();
    check_fixture(ATTEMPTED_PATH, &Replay::new(&schedule, &log, None));
}

/// The other half of the pair. Same guard, on the document whose only
/// distinguishing content is [`Replay::refused`].
#[test]
fn the_committed_refused_replay_snapshot_matches_what_replay_new_produces() {
    let (schedule, log) = refused();
    check_fixture(
        REFUSED_PATH,
        &Replay::new(&schedule, &log, Some(REFUSAL.to_string())),
    );
}

/// **This is the assertion that keeps the fixture hard.**
///
/// Everything above only proves the committed file equals what the producer
/// emits. Soften the producer and regenerate, and both halves agree again on a
/// document that pins nothing. So this reads the *committed file* -- not the
/// producer -- and insists every distinction the consumer has to render is
/// present in it as data.
#[test]
fn the_committed_snapshot_is_still_a_hard_case() {
    let v = read_fixture(ATTEMPTED_PATH);
    let steps = v["steps"].as_array().expect("steps must be an array");
    let hint = format!(
        "\n{ATTEMPTED_PATH} has stopped being a hard case. A fixture without this \n\
         value pins the field's presence and nothing about its meaning; the consumer \n\
         then renders it from its own reading of the enum, which is the defect this \n\
         file exists to prevent. Restore the case rather than deleting the assertion.\
         {REGENERATE_HINT}"
    );

    // A run that was attempted, so the pair with the refused document is a pair.
    assert_eq!(
        v.as_object().expect("a Replay is an object").get("refused"),
        Some(&Value::Null),
        "`refused` must be present and null -- an omitted key is indistinguishable \
         from a producer too old to have the field{hint}"
    );

    // Observation, and its absence, both present.
    assert!(
        steps
            .iter()
            .any(|s| s["observed_start_tick"].is_number() && s["observed_end_tick"].is_number()),
        "no step with both ticks observed{hint}"
    );
    assert!(
        steps
            .iter()
            .any(|s| s["observed_start_tick"].is_null() && s["observed_end_tick"].is_null()),
        "no step with both observations null{hint}"
    );
    assert!(
        steps
            .iter()
            .any(|s| s["observed_start_tick"].is_number() && s["observed_end_tick"].is_null()),
        "no half-measured step -- the case where a reply tick could be invented \
         from the dispatch tick{hint}"
    );

    // The statuses a renderer must keep apart. `Lost` beside `Failed` is the
    // pair most worth pinning: they are not the same fact and must not be drawn
    // as one.
    let statuses: Vec<&str> = steps.iter().filter_map(|s| s["status"].as_str()).collect();
    for required in ["Pending", "Running", "Success", "Failed", "Lost"] {
        assert!(
            statuses.contains(&required),
            "no step with status {required}{hint}"
        );
    }

    // Both `Evidence` variants, and the reason travelling with the belief.
    assert!(
        steps.iter().any(|s| s["evidence"]["kind"] == "measured"),
        "no measured step{hint}"
    );
    let believed = steps
        .iter()
        .find(|s| s["evidence"]["kind"] == "believed")
        .unwrap_or_else(|| panic!("no believed step{hint}"));
    assert!(
        believed["evidence"]["why"]
            .as_str()
            .is_some_and(|why| !why.is_empty()),
        "a believed step must carry the reason with it{hint}"
    );

    // Both step kinds.
    for kind in ["act", "walk"] {
        assert!(
            steps.iter().any(|s| s["what"]["kind"] == kind),
            "no {kind} step{hint}"
        );
    }

    // A retry actually happened. This is the only surviving record that it did
    // -- the log keeps just the latest attempt -- so a fixture where every row
    // is attempt 1 leaves the field untested against any value but its default.
    assert!(
        steps
            .iter()
            .any(|s| s["attempt_number"].as_u64().is_some_and(|n| n > 1)),
        "no step with attempt_number > 1{hint}"
    );
    assert!(
        steps.iter().any(|s| s["attempt_number"].is_null()),
        "no step with a null attempt_number{hint}"
    );

    // An error message on the row that has one, and none on the rows that do not.
    assert!(
        steps
            .iter()
            .any(|s| s["error"].as_str().is_some_and(|e| !e.is_empty())),
        "no step carrying an error message{hint}"
    );

    // `unmatched_walks` pinned non-empty, so the consumer's element type is
    // checked against a real row rather than against `[]`.
    let unmatched = v["unmatched_walks"]
        .as_array()
        .expect("unmatched_walks must be an array");
    assert!(!unmatched.is_empty(), "unmatched_walks is empty{hint}");
    assert!(
        unmatched[0]["bot"].is_number() && unmatched[0]["bot_step_index"].is_number(),
        "an unmatched walk must name the bot and the step index{hint}"
    );
}

/// The refused document's half of the same job. It is only worth committing if
/// it really is the *identical-looking* document -- every row `Pending`,
/// nothing observed anywhere -- separated from the attempted one by `refused`
/// alone.
#[test]
fn the_committed_refused_snapshot_is_a_run_that_never_started() {
    let v = read_fixture(REFUSED_PATH);
    let hint = format!("\n{REFUSED_PATH}{REGENERATE_HINT}");

    assert!(
        v["refused"].as_str().is_some_and(|why| !why.is_empty()),
        "the refused fixture must carry the refusal's own words, not a flag{hint}"
    );

    let steps = v["steps"].as_array().expect("steps must be an array");
    assert!(!steps.is_empty(), "an empty schedule pins nothing{hint}");
    for step in steps {
        assert_eq!(
            step["status"], "Pending",
            "a refused run dispatched nothing, so every row is Pending{hint}"
        );
        assert!(
            step["observed_start_tick"].is_null() && step["observed_end_tick"].is_null(),
            "a refused run measured nothing{hint}"
        );
        assert!(
            step["attempt_number"].is_null(),
            "a refused run attempted nothing{hint}"
        );
        assert!(
            step["planned_start_tick"].is_number() && step["planned_end_tick"].is_number(),
            "the plan is still fully known -- that is what the view greys out{hint}"
        );
    }

    // Both kinds, so the greyed-out plan is a plan and not a list of one shape.
    for kind in ["act", "walk"] {
        assert!(
            steps.iter().any(|s| s["what"]["kind"] == kind),
            "no {kind} step{hint}"
        );
    }
}

/// The pair is only a pair if the two documents differ in `refused` and are
/// otherwise the same *kind* of thing. Stated as a test because the argument
/// for committing both is that a change collapsing the distinction fails here.
#[test]
fn the_two_fixtures_differ_in_refused_and_that_is_the_point() {
    let attempted = read_fixture(ATTEMPTED_PATH);
    let refused = read_fixture(REFUSED_PATH);

    assert_eq!(attempted["refused"], Value::Null);
    assert!(refused["refused"].is_string());

    // Belt and braces on the type, not just the fixtures: `Status` must keep
    // serialising `Lost` and `Failed` as distinct strings, since the whole
    // renderer distinction rests on the text.
    assert_ne!(
        serde_json::to_value(Status::Lost).unwrap(),
        serde_json::to_value(Status::Failed).unwrap(),
        "Lost and Failed must never serialise to the same string"
    );
}
