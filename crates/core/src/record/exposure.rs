//! Whether a run's world was ever **exposed** to a human, and what the game
//! saw while it was.
//!
//! # Why this is not a provenance field
//!
//! [`super::provenance`] is written once, before anything can fail, and every
//! field in it describes what the run was *launched* with. A hold is not a
//! launch fact: it happens mid-run, it can happen more than once, and what it
//! yields is only known when it ends. Putting it in `provenance.json` would
//! mean rewriting a file whose whole contract is that it is never rewritten --
//! and a process killed during that rewrite loses the seed and the commit too.
//!
//! So this is its own file, `exposure.json`, on the same reasoning that split
//! provenance from the manifest in the first place.
//!
//! # What a hold is, and why it needs recording at all
//!
//! Since `c8ed0eb0` a faulting run writes a savepoint, **pauses the game**, and
//! waits up to five minutes for a person to attach with
//! `factorio-bot rcon -s localhost`. Releasing it with `continue` makes the
//! supervisor replan, and `PlanState::from_world` reads the *live* world at
//! plan time -- so anything a human inserted during the hold is simply there on
//! the retry. That is the loop the owner asked for and it is a good one.
//!
//! It also means a run in which somebody hand-inserted 50 coal was, until this
//! file existed, **byte-identical in the record** to one that ran clean. The
//! standing rule is that cheating is fine while developing and measured runs
//! must be honest and disclosed; nothing was disclosing it.
//!
//! # Three states, and they must not collapse
//!
//! This is the project's most-cited defect class -- *absent is not a value* --
//! and a hold record is exactly where it bites, because "held" is an
//! *opportunity* to mutate, not proof of one.
//!
//! | on disk | means |
//! |---|---|
//! | no `exposure.json` | **not captured.** A build older than this file, or a run whose recorder could not write. Says nothing either way. |
//! | `holds: []` | **not exposed.** This build records holds and this run had none. |
//! | `holds: [...]`, every `foreign_console_commands` null | **exposed, mutation unknown.** A human had the opportunity and nothing could tell whether they took it. |
//! | `holds: [...]`, foreign counts all `0` | **exposed, none observed.** Stronger, and still not a proof of innocence -- see the limits below. |
//! | `holds: [...]`, some foreign count `> 0` | **mutated, or at least commanded.** Somebody ran something the run did not. |
//!
//! The empty vector is why this file is written at run *start* as well as at
//! each hold. Without that, an absent file would mean both "no hold happened"
//! and "this build never looked", which is the collapse the table exists to
//! prevent.
//!
//! # What the foreign count can and cannot see
//!
//! It is a **difference of two counters**, both measured across the hold:
//!
//! * what the game saw -- `on_console_command` fires once per console command,
//!   and BotBridge counts them (`console_census`);
//! * what this process sent -- every RCON command in this workspace funnels
//!   through [`crate::factorio::rcon::FactorioRcon::send`], which counts.
//!
//! Foreign is the first minus the second. Measured 2026-09-08 against a live
//! headless game: `on_console_command` **does** fire for an RCON
//! `/silent-command`, so the run's own traffic is inside both counters and
//! cancels; a `/c` typed from a second terminal moved the first counter and not
//! the second, and showed up as exactly one foreign command.
//!
//! Four limits, stated rather than papered over:
//!
//! * **It counts commands, not mutations.** `/c rcon.print(game.tick)` is
//!   foreign and harmless; the field is named for what it measures.
//! * **A hold released by a person always counts at least one**, because the
//!   release *is* a foreign command
//!   (`remote.call('botbridge','hold_release',...)`). Measured on the first
//!   live run of this: three foreign commands for a tick read, a cheat and the
//!   release. So `released` and this count are read together -- `1` beside
//!   `continue` or `stop` is consistent with "somebody released it and touched
//!   nothing else", while `foreign == 0` happens only on a `timeout`, where
//!   nobody came at all. Not subtracted, because nothing here can tell *which*
//!   command was the release, and subtracting a guess would understate.
//! * **It cannot see a mutation made another way** -- a person on a graphical
//!   client clicking items into a chest issues no console command at all.
//!   `0` here is therefore "no foreign command was observed", never "the world
//!   was not touched".
//! * **`game.console_command_used`, the engine's own achievement-disabling
//!   flag, is useless here and was checked rather than assumed.** Every action
//!   this project issues is a `/silent-command`, so it reads `true` from the
//!   first command of every run. It is carried in [`ConsoleCensus`] anyway
//!   because a `false` would be a real strong negative, and because a future
//!   Factorio that stops latching on our traffic would make it informative
//!   without anyone having to notice.

use serde::{Deserialize, Serialize};

/// The name of the file this module writes, inside the run directory.
pub const EXPOSURE_FILE: &str = "exposure.json";

/// Every hold a run performed, and therefore every window in which a human
/// could have changed the world under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exposure {
    /// Bumped when a field changes meaning, never when one is added -- the
    /// same rule [`super::provenance::Provenance::schema`] follows, so that
    /// archived runs stay readable by newer tools.
    pub schema: u32,
    /// In the order they happened. **Empty is a positive statement**: this
    /// build records holds and this run had none. The absence of the whole
    /// file is the other answer, and they are not the same.
    pub holds: Vec<HoldExposure>,
}

impl Exposure {
    /// The schema version this build writes.
    pub const SCHEMA: u32 = 1;

    /// A run that has not been held. What [`super::RunRecorder`] writes at run
    /// start, so that "no hold happened" is on disk rather than inferred from
    /// a missing file.
    pub fn none_yet() -> Self {
        Self {
            schema: Self::SCHEMA,
            holds: Vec::new(),
        }
    }

    /// Whether this run was ever held -- i.e. whether a human had the
    /// opportunity. Says nothing about whether they took it.
    pub fn was_held(&self) -> bool {
        !self.holds.is_empty()
    }

    /// Foreign console commands across every hold, or `None` when **any** hold
    /// could not tell.
    ///
    /// The `None`-poisons-the-sum rule is deliberate. A run held twice, once
    /// with a clean reading of zero and once with no reading at all, has not
    /// been shown to be clean -- and summing the readable half would report
    /// `0`, which is the strongest claim available and the one thing the
    /// evidence does not support.
    pub fn foreign_console_commands(&self) -> Option<u64> {
        self.holds
            .iter()
            .map(|h| h.foreign_console_commands)
            .try_fold(0u64, |sum, next| Some(sum + next?))
    }
}

/// One hold: the game was paused, a person was invited, and this is what
/// happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldExposure {
    /// `game.tick` the pause took effect at. A hold advances no ticks, so this
    /// is also the tick the run resumed from.
    pub paused_at_tick: u64,
    /// Wall-clock seconds the hold lasted. Wall clock and not ticks because a
    /// paused game has none -- the one measurement in this project that could
    /// not be tick-bounded even in principle.
    pub held_seconds: u64,
    /// `"continue"`, `"stop"` or `"timeout"`. `"continue"` is the one that
    /// makes the run go on to replan against whatever the world now holds, so
    /// it is the value a reader of a *result* cares about most.
    pub released: String,
    /// The fault that caused the hold, when the caller passed one.
    #[serde(default)]
    pub reason: Option<String>,
    /// Console commands the **game** saw during the hold, this process's own
    /// included. `None` when the census could not be read at both ends.
    #[serde(default)]
    pub console_commands_observed: Option<u64>,
    /// Console commands **this process** sent during the same window: the
    /// release-channel polls and the heartbeat tick reads.
    #[serde(default)]
    pub console_commands_ours: Option<u64>,
    /// The difference, which is what somebody else ran.
    ///
    /// `None` means the two counters could not both be read, or that they
    /// disagreed in the impossible direction (the game seeing *fewer* commands
    /// than we sent). A negative difference is not clamped to zero: zero is
    /// the strongest available claim and a counter disagreement is no evidence
    /// for it.
    #[serde(default)]
    pub foreign_console_commands: Option<u64>,
    /// `game.console_command_used` as the game answered it at the end of the
    /// hold. Expected `true` on every run of this project -- see the module
    /// doc. Recorded so that a `false` is not thrown away.
    #[serde(default)]
    pub console_command_used: Option<bool>,
}

/// What BotBridge's `console_census` answers: a running count and the engine's
/// latching flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleCensus {
    /// Console commands the game has processed since it loaded, monotonic and
    /// never reset. Only differences of it mean anything.
    pub commands: u64,
    /// `game.console_command_used`.
    pub command_used: bool,
}

impl ConsoleCensus {
    /// Parses the mod's `<count>,<bool>` reply.
    ///
    /// Deliberately strict: a reply this cannot read yields `None`, which the
    /// caller records as "not captured". Guessing a count from a malformed
    /// line would put a number where there is no measurement.
    pub fn parse(reply: &str) -> Option<Self> {
        let (count, used) = reply.trim().split_once(',')?;
        Some(Self {
            commands: count.trim().parse().ok()?,
            command_used: match used.trim() {
                "true" => true,
                "false" => false,
                _ => return None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_exposure_says_not_held_rather_than_saying_nothing() {
        let e = Exposure::none_yet();
        assert!(!e.was_held());
        // The positive answer, which is what distinguishes this from an
        // absent file.
        assert_eq!(e.foreign_console_commands(), Some(0));
    }

    #[test]
    fn one_unreadable_hold_poisons_the_total_rather_than_being_skipped() {
        let e = Exposure {
            schema: Exposure::SCHEMA,
            holds: vec![
                HoldExposure {
                    paused_at_tick: 1,
                    held_seconds: 1,
                    released: "stop".into(),
                    reason: None,
                    console_commands_observed: Some(10),
                    console_commands_ours: Some(10),
                    foreign_console_commands: Some(0),
                    console_command_used: Some(true),
                },
                HoldExposure {
                    paused_at_tick: 2,
                    held_seconds: 1,
                    released: "continue".into(),
                    reason: None,
                    console_commands_observed: None,
                    console_commands_ours: None,
                    foreign_console_commands: None,
                    console_command_used: None,
                },
            ],
        };
        assert!(e.was_held());
        assert_eq!(
            e.foreign_console_commands(),
            None,
            "a clean reading beside an unreadable one is not a clean run"
        );
    }

    #[test]
    fn foreign_commands_sum_across_holds_when_every_one_could_be_read() {
        let hold = |foreign| HoldExposure {
            paused_at_tick: 1,
            held_seconds: 1,
            released: "continue".into(),
            reason: None,
            console_commands_observed: Some(0),
            console_commands_ours: Some(0),
            foreign_console_commands: Some(foreign),
            console_command_used: Some(true),
        };
        let e = Exposure {
            schema: Exposure::SCHEMA,
            holds: vec![hold(2), hold(3)],
        };
        assert_eq!(e.foreign_console_commands(), Some(5));
    }

    #[test]
    fn the_census_reply_is_parsed_or_refused_never_guessed() {
        assert_eq!(
            ConsoleCensus::parse("17,true"),
            Some(ConsoleCensus {
                commands: 17,
                command_used: true
            })
        );
        assert_eq!(
            ConsoleCensus::parse(" 0 , false "),
            Some(ConsoleCensus {
                commands: 0,
                command_used: false
            })
        );
        // A mod that predates the function answers with something else
        // entirely; every shape but the one above is "not captured".
        assert_eq!(ConsoleCensus::parse("17"), None);
        assert_eq!(ConsoleCensus::parse("17,yes"), None);
        assert_eq!(ConsoleCensus::parse(""), None);
        assert_eq!(ConsoleCensus::parse("nil,true"), None);
    }

    #[test]
    fn an_archived_exposure_without_the_later_fields_still_loads() {
        // Every optional field is `serde(default)` for the reason provenance's
        // are: a tool must keep reading files older than itself.
        let json = r#"{"schema":1,"holds":[{"paused_at_tick":429,
            "held_seconds":60,"released":"timeout"}]}"#;
        let e: Exposure = serde_json::from_str(json).unwrap();
        assert!(e.was_held());
        assert_eq!(e.foreign_console_commands(), None);
    }
}
