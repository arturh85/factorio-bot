//! A serialisable pairing of a [`Schedule`] with an [`ExecutionLog`].
//!
//! # What this is, and what it deliberately is not
//!
//! This module emits **data**, not a picture. The consumer is a browser view
//! that draws two rows per step — what the scheduler predicted against what the
//! game reported — with a scrubber over them. That view needs to render `Lost`
//! differently from `Failed`, mark walk rows as belief rather than measurement,
//! and derive the wait between two steps by subtracting two *observed*
//! quantities. None of that is expressible as one bar per row with a label, a
//! start and a duration, which is the whole of mermaid gantt's vocabulary.
//!
//! So this is not a renderer, and it is not a replacement for one.
//! [`factorio_bot_planner::render::mermaid_gantt`] still exists, still renders
//! the `Schedule` alone, and is still a **plan** view reaching the browser as
//! script stdout. It never reads an `ExecutionLog` and must not start: a plan
//! view that quietly acquires observed data becomes a replay view that nobody
//! labelled as one. Two views, two data paths.
//!
//! It lives in `crates/executor` and not in `crates/planner` because the
//! planner is pure and knows nothing of execution. The executor already depends
//! on the planner; the reverse would invert that and drag run state into a crate
//! that must not have any.
//!
//! # An absent measurement stays absent
//!
//! This is the rule the whole shape rests on. Every observed tick is an
//! `Option<Ticks>` and serialises as an explicit `null` when the game did not
//! say. It is never zero, never the planned tick, never the previous step's.
//!
//! The peer building the view put it best: *a zero and a missing measurement
//! render identically if I let them.* Tick 0 is a real, plausible, very
//! early-game measurement; a bar drawn from it sits at the origin and looks
//! like a fact. So:
//!
//! - No field carries `#[serde(default)]`, and the reason is sharper than
//!   "defaults are bad". Serde already gives an `Option` field an implicit
//!   `None` when its key is missing, which is the *safe* direction: a truncated
//!   document reads as unmeasured, never as tick zero. What `#[serde(default)]`
//!   would add is precisely the unsafe direction — on a numeric field it
//!   manufactures `0`. So the planned ticks are plain `Ticks` with no default,
//!   and a document missing one is a hard deserialisation error rather than a
//!   step silently planned at the origin. Both halves are tested.
//! - No field carries `#[serde(skip_serializing_if)]`. The key is always
//!   present, holding `null`. An omitted key is indistinguishable from a
//!   producer too old to have the field; an explicit `null` says *we looked and
//!   there was nothing to record*.
//! - The planned interval is **not** an `Option` and the observed interval
//!   **is**. That asymmetry is the point: a schedule always knows where it put a
//!   step, and a run frequently does not know when the game ran it.
//!
//! # Walk rows are belief, not measurement
//!
//! A walk's *ticks* are measured exactly as an action's are. Where the bot
//! **ended up** is not, and that is the distinction this document has to carry.
//!
//! The historical reason is worth stating because it is the sharpest example of
//! the failure mode and because it is now fixed.
//! [`player_path`](factorio_bot_core::factorio::rcon::FactorioRcon::player_path)
//! is best effort: when a goal is unreachable it retries against a *synthesised*
//! goal offset by the radius and returns the path to that. The walk along such a
//! path completes normally, so the mod, the actuator and the log all reported
//! success — while the bot stood **9.30 tiles** from where the plan put it. That
//! is the dangerous shape of wrong: not absent, not even wrong-looking, but
//! confidently right about the wrong thing.
//!
//! `move_player_timed` now judges the path against the caller's own goal before
//! dispatching and refuses one that falls short, so that particular lie no longer
//! reaches this document. **Belief survives the fix**, for two narrower reasons
//! that the fix does not address and does not claim to:
//!
//! - The check is on the *planned path*, before dispatch — not on where the bot
//!   came to rest. The mod's follower halts within a small box of a last waypoint
//!   that itself sits on a tile centre; the same run's legitimate endpoints
//!   landed 0.707, 1.000 and 2.828 tiles from their goals. Nothing re-measures
//!   the resting position afterwards, so an arrival is still *inferred from the
//!   plan*.
//! - A walk whose end cannot be judged at all — unknown player position and an
//!   empty path — is deliberately dispatched unchecked rather than refused on a
//!   guess. Those walks carry no arrival evidence whatsoever.
//!
//! So a position or arrival derived from a walk row remains the executor's
//! belief, and the document says so rather than leaving the consumer to assume.
//!
//! It says so with a per-row [`Evidence`] field rather than letting the consumer
//! infer it from `kind == "walk"`, and the last few hours are the argument for
//! that choice. "Walk implies belief" is a *current* fact about a specific set of
//! defects, not a property of walking; encoded in the consumer it becomes a
//! caveat only the consumer can retract. Here, the fix above narrowed the claim
//! and the only thing that changed was the string in [`WALK_BELIEF`] — no
//! consumer edit, no stale warning left on screen. [`Evidence`] carries the
//! reason with it ([`Evidence::Believed::why`]), so the tooltip a reader sees is
//! written where the fact is known, and a `measured` row is structurally
//! incapable of carrying a caveat at all.

use crate::log::{ExecutionLog, Status};
use factorio_bot_core::types::Position;
use factorio_bot_planner::schedule::{Schedule, StepKind};
use factorio_bot_planner::{ActionId, BotId, Ticks};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Why a walk row's observation is belief rather than measurement.
///
/// Written once, here, so every producer says the same thing and the consumer
/// never has to compose the sentence itself. Narrowing the caveat is an edit to
/// this string and nothing else — see the module docs, where doing exactly that
/// is the argument for putting the reason on the row.
pub const WALK_BELIEF: &str = "the walk's ticks are measured, but its arrival is not: the path is checked against the goal before dispatch and the resting position is never re-measured afterwards, so where the bot ended up is inferred from the plan rather than observed";

/// Whether a row's observation means what it says.
///
/// Internally tagged, so a row reads `{"kind":"measured"}` or
/// `{"kind":"believed","why":"..."}`. The reason travels with the claim: a
/// `measured` row cannot carry one, and a `believed` row cannot omit one.
///
/// This classifies the **kind of claim a row makes**, not whether it has
/// anything to claim. A `Pending` row with two null ticks is still `measured`:
/// no caveat applies to it, and its emptiness is carried by those nulls. The two
/// are kept apart deliberately — collapsing them would make *unmeasured* and
/// *unreliable* the same word, and they call for opposite renderings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Evidence {
    /// The game stamped these ticks for this exact step, and the step did what
    /// it was asked to do.
    Measured,
    /// The ticks are real, but what the step *achieved* is the executor's
    /// belief. `why` names the specific reason.
    Believed { why: String },
}

/// What a row is a row of.
///
/// A deliberate copy of [`StepKind`] rather than a re-export of it. `StepKind`
/// is externally tagged (`{"Act":{...}}`), which is awkward to switch on in a
/// view; this is `{"kind":"act",...}`. More importantly it is a wire contract
/// with its own consumer, and pinning it to a planner-internal enum would mean
/// any refactor there silently rewrites the JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayStepKind {
    Act { action: ActionId, label: String },
    Walk { to: Position },
}

/// One scheduled step, paired with whatever the run learned about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayStep {
    /// Position in [`Schedule::steps`]. Stable identity for selection in a
    /// scrubber.
    pub index: usize,
    pub bot: BotId,
    /// Position among *this bot's* steps, in schedule order. This is the
    /// second half of the `(bot, step_index)` key that
    /// [`ExecutionLog::walk`] uses, and it is exported so a consumer can look a
    /// row back up in a raw log.
    pub bot_step_index: usize,
    pub what: ReplayStepKind,
    /// Where the *scheduler* put this step. Never observed, never absent — a
    /// schedule always knows this.
    pub planned_start_tick: Ticks,
    /// See [`Self::planned_start_tick`].
    pub planned_end_tick: Ticks,
    /// `game.tick` when the game **received** this step's command, or `null`
    /// when the game never said. Never zero-as-unknown; see the module docs.
    pub observed_start_tick: Option<Ticks>,
    /// `game.tick` when the game **reported the outcome**, or `null` when the
    /// game never said. Never zero-as-unknown; see the module docs.
    pub observed_end_tick: Option<Ticks>,
    /// What is known about this step. [`Status::Pending`] when the log holds no
    /// record of it at all, which is a different fact from a record with no
    /// ticks. [`Status::Lost`] is a distinct value from [`Status::Failed`] and
    /// they serialise as distinct strings: `Lost` means this run will not learn
    /// the outcome, not that the step went wrong.
    pub status: Status,
    /// Which attempt this row describes, counting from 1, or `null` when the
    /// log holds no attempt for it.
    ///
    /// **Kept deliberately.** An action attempted three times is a different
    /// story from one attempted once, and this is the only surviving record
    /// that a retry happened at all — the log keeps only the latest attempt.
    /// A renderer collapsing rows to bars would have discarded it.
    ///
    /// Always `null` for walk rows: [`crate::WalkObservation`] carries no
    /// attempt count, because a walk is redispatched by rerunning the step
    /// rather than by retrying an action id.
    pub attempt_number: Option<u32>,
    /// Whether this row's observation is measurement or belief. Walk rows are
    /// [`Evidence::Believed`]; see the module docs for the pathfinder reason.
    pub evidence: Evidence,
    /// The failure message, when there was one.
    pub error: Option<String>,
}

/// A walk observation that no scheduled step claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmatchedWalk {
    pub bot: BotId,
    pub bot_step_index: usize,
}

/// A [`Schedule`] paired with an [`ExecutionLog`], ready to serialise.
///
/// Rows are in [`Schedule::steps`] order, which is stable, so two serialisations
/// of the same pair are byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Replay {
    /// [`Schedule::makespan`] — the *planned* end of the whole run. There is
    /// deliberately no observed counterpart: deriving one means picking a
    /// definition (last reply? last dispatch? last step with any tick at all?)
    /// and that is the consumer's decision, not this document's.
    pub planned_makespan: Ticks,
    pub steps: Vec<ReplayStep>,
    /// Walk observations whose `(bot, bot_step_index)` matches no step in the
    /// schedule — normally empty, and non-empty only when a log is paired with a
    /// schedule it did not come from. Reported rather than dropped so pairing
    /// the wrong two things is visible instead of silently lossy.
    ///
    /// Only walks are checked. [`ExecutionLog`] enumerates them
    /// ([`ExecutionLog::walks`]) and exposes no equivalent iterator over
    /// attempts, so an attempt for an unscheduled action cannot be detected
    /// here and is not claimed to be.
    pub unmatched_walks: Vec<UnmatchedWalk>,
}

impl Replay {
    /// Join a schedule with a log. Pure: reads both, mutates neither.
    pub fn new(schedule: &Schedule, log: &ExecutionLog) -> Self {
        let mut next_index: BTreeMap<BotId, usize> = BTreeMap::new();
        let mut matched: BTreeMap<BotId, Vec<usize>> = BTreeMap::new();
        let mut steps = Vec::with_capacity(schedule.steps.len());

        for (index, step) in schedule.steps.iter().enumerate() {
            let counter = next_index.entry(step.bot).or_insert(0);
            let bot_step_index = *counter;
            *counter += 1;

            let row = match &step.what {
                StepKind::Act { action, label } => {
                    let attempt = log.attempt(*action);
                    ReplayStep {
                        index,
                        bot: step.bot,
                        bot_step_index,
                        what: ReplayStepKind::Act {
                            action: *action,
                            label: label.clone(),
                        },
                        planned_start_tick: step.start,
                        planned_end_tick: step.end,
                        observed_start_tick: attempt.and_then(|a| a.dispatched_tick),
                        observed_end_tick: attempt.and_then(|a| a.replied_tick),
                        status: attempt.map_or(Status::Pending, |a| a.status),
                        attempt_number: attempt.map(|a| a.number),
                        evidence: Evidence::Measured,
                        error: attempt.and_then(|a| a.error.clone()),
                    }
                }
                StepKind::Walk { to } => {
                    let walk = log.walk(step.bot, bot_step_index);
                    if walk.is_some() {
                        matched.entry(step.bot).or_default().push(bot_step_index);
                    }
                    ReplayStep {
                        index,
                        bot: step.bot,
                        bot_step_index,
                        what: ReplayStepKind::Walk { to: to.clone() },
                        planned_start_tick: step.start,
                        planned_end_tick: step.end,
                        observed_start_tick: walk.and_then(|w| w.dispatched_tick),
                        observed_end_tick: walk.and_then(|w| w.replied_tick),
                        status: walk.map_or(Status::Pending, |w| w.status),
                        // A walk has no attempt count to carry. See the field.
                        attempt_number: None,
                        evidence: Evidence::Believed {
                            why: WALK_BELIEF.to_string(),
                        },
                        error: walk.and_then(|w| w.error.clone()),
                    }
                }
            };
            steps.push(row);
        }

        let unmatched_walks = log
            .walks()
            .filter(|(bot, i, _)| !matched.get(bot).is_some_and(|indices| indices.contains(i)))
            .map(|(bot, bot_step_index, _)| UnmatchedWalk {
                bot,
                bot_step_index,
            })
            .collect();

        Replay {
            planned_makespan: schedule.makespan,
            steps,
            unmatched_walks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActionTicks;
    use factorio_bot_planner::schedule::ScheduledStep;
    use serde_json::{json, Value};

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
            },
            bot: BotId(bot),
            start,
            end,
        }
    }

    /// A run that is *partly* observed, which is the only interesting kind.
    ///
    /// Bot 1 walked and mined under a working clock, then failed a craft on its
    /// third attempt with only half a measurement. Bot 2 lost track of its walk,
    /// never reached its next step at all, and finished a third step
    /// successfully but unobserved. Every absence below is a different absence.
    fn realistic() -> (Schedule, ExecutionLog) {
        let schedule = Schedule {
            steps: vec![
                walk(1, 10.0, 20.0, 0, 70),
                act(1, 0, "mine iron-ore", 70, 190),
                act(1, 1, "craft iron-gear-wheel", 190, 250),
                walk(2, 40.0, 5.0, 0, 120),
                act(2, 2, "place stone-furnace", 120, 150),
                act(2, 3, "insert coal", 150, 180),
            ],
            makespan: 250,
        };

        let mut log = ExecutionLog::default();

        // Bot 1, step 0: a walk the game stamped at both ends.
        log.start_walk(BotId(1), 0, Position::new(10.0, 20.0), 0, 70);
        log.observe_walk(BotId(1), 0, ActionTicks::new(Some(100), Some(205)));
        log.succeed_walk(BotId(1), 0);

        // Bot 1, step 1: fully measured action, first attempt.
        log.start(ActionId(0), 70);
        log.observe(ActionId(0), ActionTicks::new(Some(205), Some(330)));
        log.succeed(ActionId(0), 190);

        // Bot 1, step 2: third attempt, dispatched then refused. The game
        // stamped the dispatch and never the reply.
        log.start(ActionId(1), 190);
        log.start(ActionId(1), 190);
        log.start(ActionId(1), 190);
        log.observe(ActionId(1), ActionTicks::new(Some(340), None));
        log.fail(ActionId(1), 250, "not enough iron-plate".to_string());

        // Bot 2, step 0: dispatched, and this run will never learn the outcome.
        log.start_walk(BotId(2), 0, Position::new(40.0, 5.0), 0, 120);
        log.observe_walk(BotId(2), 0, ActionTicks::new(Some(100), None));
        log.lose_track_walk(BotId(2), 0, "run ended holding the dispatch");

        // Bot 2, step 1: never dispatched at all. No entry in the log.

        // Bot 2, step 2: succeeded with no clock. Nothing went wrong and
        // nothing was learned about when.
        log.start(ActionId(3), 150);
        log.observe(ActionId(3), ActionTicks::UNKNOWN);
        log.succeed(ActionId(3), 180);

        (schedule, log)
    }

    fn doc() -> Value {
        let (schedule, log) = realistic();
        serde_json::to_value(Replay::new(&schedule, &log)).expect("the document must serialise")
    }

    /// Rows are the schedule's rows, in the schedule's order, one for one.
    #[test]
    fn every_scheduled_step_gets_exactly_one_row_in_schedule_order() {
        let v = doc();
        assert_eq!(v["planned_makespan"], json!(250));
        let steps = v["steps"].as_array().expect("steps must be an array");
        assert_eq!(steps.len(), 6, "one row per scheduled step");
        let indices: Vec<&Value> = steps.iter().map(|s| &s["index"]).collect();
        assert_eq!(
            indices,
            vec![
                &json!(0),
                &json!(1),
                &json!(2),
                &json!(3),
                &json!(4),
                &json!(5)
            ]
        );
        let bots: Vec<&Value> = steps.iter().map(|s| &s["bot"]).collect();
        assert_eq!(
            bots,
            vec![
                &json!(1),
                &json!(1),
                &json!(1),
                &json!(2),
                &json!(2),
                &json!(2)
            ]
        );
        // The walk key: index among *this bot's* steps, not the global index.
        assert_eq!(steps[3]["bot_step_index"], json!(0));
        assert_eq!(steps[5]["bot_step_index"], json!(2));
        assert!(
            v["unmatched_walks"].as_array().unwrap().is_empty(),
            "every walk in this log belongs to a step in this schedule"
        );
    }

    /// The planned interval is the schedule's, and it is never optional.
    #[test]
    fn the_planned_interval_comes_from_the_schedule_and_is_always_present() {
        let v = doc();
        assert_eq!(v["steps"][2]["planned_start_tick"], json!(190));
        assert_eq!(v["steps"][2]["planned_end_tick"], json!(250));

        // The step the run never reached. Its plan is still fully known — the
        // schedule knew where it put it — and taking these from the log instead
        // would collapse an unobserved step onto the origin, where it reads as a
        // step that ran instantly at tick zero.
        let never_run = &v["steps"][4];
        assert_eq!(never_run["status"], json!("Pending"));
        assert_eq!(never_run["planned_start_tick"], json!(120));
        assert_eq!(never_run["planned_end_tick"], json!(150));

        for step in v["steps"].as_array().unwrap() {
            assert!(
                step["planned_start_tick"].is_number() && step["planned_end_tick"].is_number(),
                "a schedule always knows where it put a step: {step}"
            );
        }
    }

    /// The observed interval is the game's stamps, and only the game's.
    #[test]
    fn the_observed_interval_comes_from_the_games_stamps() {
        let v = doc();
        assert_eq!(v["steps"][1]["observed_start_tick"], json!(205));
        assert_eq!(v["steps"][1]["observed_end_tick"], json!(330));
        assert_eq!(v["steps"][0]["observed_start_tick"], json!(100));
        assert_eq!(v["steps"][0]["observed_end_tick"], json!(205));
    }

    /// **The rule the whole shape rests on.** A tick the game never gave must
    /// cross the wire as `null` — not `0`, not the planned tick, not the
    /// previous step's tick — and it must still be a *key*, so a consumer can
    /// tell "no measurement" from "producer too old to have the field".
    #[test]
    fn an_unmeasured_tick_is_null_in_the_json_and_never_zero() {
        let v = doc();

        // Bot 2's third step: succeeded, no clock. Both ends absent.
        let unobserved = &v["steps"][5];
        assert_eq!(unobserved["status"], json!("Success"));
        for field in ["observed_start_tick", "observed_end_tick"] {
            let tick = unobserved
                .as_object()
                .expect("a row is an object")
                .get(field)
                .unwrap_or_else(|| panic!("{field} must be present as a key, not omitted"));
            assert_eq!(tick, &Value::Null, "{field} must be null");
            assert_ne!(tick, &json!(0), "{field} must not be tick zero");
        }
        assert_ne!(
            unobserved["observed_start_tick"], unobserved["planned_start_tick"],
            "an absent measurement must not be filled in from the plan"
        );

        // Bot 2's second step: never dispatched. Same absence, different status.
        let pending = &v["steps"][4];
        assert_eq!(pending["status"], json!("Pending"));
        assert_eq!(pending["observed_start_tick"], Value::Null);
        assert_eq!(pending["observed_end_tick"], Value::Null);
        assert_eq!(
            pending["attempt_number"],
            Value::Null,
            "no attempt was made, so there is no attempt number"
        );

        // Half a measurement stays half: the dispatch survives, the reply does
        // not get invented from the one that is there.
        let half = &v["steps"][2];
        assert_eq!(half["observed_start_tick"], json!(340));
        assert_eq!(half["observed_end_tick"], Value::Null);

        // And the same read against the raw text, because the text is the
        // contract. `0` must appear nowhere as an observed tick.
        let text = serde_json::to_string(&{
            let (s, l) = realistic();
            Replay::new(&s, &l)
        })
        .unwrap();
        assert!(
            text.contains("\"observed_end_tick\":null"),
            "absence must serialise as an explicit null: {text}"
        );
        assert!(
            !text.contains("\"observed_start_tick\":0")
                && !text.contains("\"observed_end_tick\":0"),
            "no observed tick may serialise as zero: {text}"
        );
    }

    /// Neither direction of the round trip may invent a number.
    ///
    /// Serde hands an `Option` field an implicit `None` when its key is absent,
    /// so a truncated document reads as *unmeasured* — which is the safe
    /// direction and is asserted here so a later `#[serde(default)]` on a
    /// numeric observed field would be caught turning it into `Some(0)`.
    ///
    /// The planned ticks are the other half. They are plain `Ticks`, so a
    /// document missing one must be a hard error: a `#[serde(default)]` there
    /// would quietly plan the step at the origin, which reads as a fact.
    #[test]
    fn a_missing_tick_never_deserialises_into_a_number() {
        let (schedule, log) = realistic();
        let v = serde_json::to_value(Replay::new(&schedule, &log)).unwrap();

        // Round trip with the nulls intact: still absent, still not zero.
        let back: Replay = serde_json::from_value(v.clone()).expect("null round-trips");
        assert_eq!(back.steps[5].observed_start_tick, None);
        assert_eq!(back.steps[5].observed_end_tick, None);
        assert_ne!(back.steps[5].observed_start_tick, Some(0));

        // Observed tick key removed entirely: absent, never zero.
        let mut truncated = v.clone();
        truncated["steps"][1]
            .as_object_mut()
            .unwrap()
            .remove("observed_start_tick");
        let back: Replay = serde_json::from_value(truncated)
            .expect("a missing observed tick is simply unmeasured");
        assert_eq!(
            back.steps[1].observed_start_tick, None,
            "a missing measurement must stay missing"
        );
        assert_ne!(
            back.steps[1].observed_start_tick,
            Some(0),
            "a missing measurement must never become tick zero"
        );

        // Planned tick key removed: a hard error, because there is no honest
        // value to supply and `0` would look like a step planned at the origin.
        let mut truncated = v;
        truncated["steps"][1]
            .as_object_mut()
            .unwrap()
            .remove("planned_start_tick");
        let err = serde_json::from_value::<Replay>(truncated)
            .expect_err("a missing planned tick must not default to zero");
        assert!(
            err.to_string().contains("planned_start_tick"),
            "the error must name the field it refused to invent; got {err}"
        );
    }

    /// `Lost` and `Failed` are two different facts and must not collapse into
    /// one another on the wire. `Lost` says this run will never learn the
    /// outcome; `Failed` says a verdict arrived and it was bad.
    #[test]
    fn a_lost_step_is_distinguishable_from_a_failed_one() {
        let v = doc();
        let failed = &v["steps"][2];
        let lost = &v["steps"][3];

        assert_eq!(failed["status"], json!("Failed"));
        assert_eq!(lost["status"], json!("Lost"));
        assert_ne!(
            failed["status"], lost["status"],
            "a view that draws these the same is drawing a lie"
        );
        assert_eq!(
            failed["error"],
            json!("not enough iron-plate"),
            "a verdict arrived and it must survive"
        );
    }

    /// The only surviving record that a retry happened at all. The log keeps one
    /// attempt per action, so if this number is dropped here the fact that an
    /// action was dispatched three times is gone for good.
    #[test]
    fn the_attempt_number_survives_serialisation() {
        let v = doc();
        assert_eq!(
            v["steps"][2]["attempt_number"],
            json!(3),
            "this craft was dispatched three times"
        );
        assert_eq!(
            v["steps"][1]["attempt_number"],
            json!(1),
            "and this mine exactly once"
        );
        assert!(
            serde_json::to_string(&{
                let (s, l) = realistic();
                Replay::new(&s, &l)
            })
            .unwrap()
            .contains("\"attempt_number\":3"),
            "the retry count must cross the wire, not just exist in Rust"
        );
    }

    /// Walk rows carry their caveat with them. See the module docs: a walk's
    /// ticks are measured but its arrival is not, so where the bot ended up is
    /// the executor's belief.
    #[test]
    fn walk_rows_are_marked_belief_and_action_rows_measurement() {
        let v = doc();
        for (i, step) in v["steps"].as_array().unwrap().iter().enumerate() {
            let is_walk = step["what"]["kind"] == json!("walk");
            if is_walk {
                assert_eq!(
                    step["evidence"]["kind"],
                    json!("believed"),
                    "walk row {i} must not claim measurement"
                );
                let why = step["evidence"]["why"]
                    .as_str()
                    .expect("a believed row must say why");
                assert!(
                    why.contains("arrival is not") && why.contains("never re-measured"),
                    "the caveat must name what is unmeasured, not merely warn; got {why}"
                );
            } else {
                assert_eq!(step["evidence"]["kind"], json!("measured"));
                assert!(
                    step["evidence"].get("why").is_none(),
                    "a measured row is structurally incapable of carrying a caveat"
                );
            }
        }
        // Belief is about what the walk achieved, not about whether it was
        // timed: a fully stamped walk is still belief.
        assert_eq!(v["steps"][0]["observed_end_tick"], json!(205));
        assert_eq!(v["steps"][0]["evidence"]["kind"], json!("believed"));
    }

    /// The step's identity and label survive, tagged so a view can switch on it
    /// without unwrapping an externally tagged enum.
    #[test]
    fn the_step_kind_is_internally_tagged_and_carries_its_payload() {
        let v = doc();
        assert_eq!(
            v["steps"][1]["what"],
            json!({"kind": "act", "action": 0, "label": "mine iron-ore"})
        );
        assert_eq!(
            v["steps"][3]["what"],
            json!({"kind": "walk", "to": {"x": 40.0, "y": 5.0}})
        );
    }

    /// Pairing a log with a schedule it did not come from must be visible, not
    /// silently lossy: the walk observation that no step claims is reported.
    #[test]
    fn a_walk_that_no_scheduled_step_claims_is_reported_not_dropped() {
        let (schedule, mut log) = realistic();
        log.start_walk(BotId(2), 7, Position::new(99.0, 99.0), 900, 999);
        let v = serde_json::to_value(Replay::new(&schedule, &log)).unwrap();
        assert_eq!(
            v["unmatched_walks"],
            json!([{"bot": 2, "bot_step_index": 7}]),
            "a walk keyed outside the schedule must surface"
        );
    }
}
