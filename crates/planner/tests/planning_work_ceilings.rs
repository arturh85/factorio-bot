//! Ceilings on how much *work* the three `map.json` baselines cost to plan.
//!
//! # Why this test exists
//!
//! `CLAUDE.md` advertised offline planning as "~4 seconds" for months. By
//! 2026-09-08 the real figure on an explored dump was **319 seconds**, and
//! nobody noticed, because a number written in prose has no invalidation: it
//! is a cache with no key, and the only thing that would ever have falsified
//! it was somebody running the command and caring about the answer. The same
//! file already knows this about itself -- *"re-measure, do not quote from
//! here"* -- and the reason the advice keeps failing is that a reader has no
//! way to tell a stale number from a fresh one.
//!
//! A count is not a duration. This box ranged from load 1 to load 80 in a
//! single day; a wall-clock assertion on it either fails for reasons that have
//! nothing to do with the change, or is loose enough to catch nothing.
//! `goals_expanded`, `resource_patches`, `threat_queries`, `preds` and `forks`
//! are **exactly** reproducible for a given world and goal -- they do not move
//! when the box is busy -- so a ceiling on them is a guard a test can hold.
//!
//! # What the ceilings are, and are not
//!
//! Each is **twice** the count measured on `b5f44da3` (see the table beside
//! each assertion), so this catches a 2x regression and ignores an ordinary
//! planner change. It is deliberately *not* an equality: the counts move
//! whenever the planner legitimately gets better or worse at a goal, exactly
//! as the action counts in `CLAUDE.md`'s baseline table do, and a test that
//! pinned them would be a test somebody re-baselines without thinking every
//! week. The four `actions`/`makespan` baselines stay the equality check;
//! this is the cost of *reaching* them.
//!
//! Tighten a ceiling when a remedy lands that lowers a count for good -- that
//! is the point of the ceiling being a number in a test rather than a number
//! in a document.
//!
//! # The dump, and why a miss is loud
//!
//! `workspace/scripts/map.json` is 864 MB and is not in the repository, so a
//! fresh clone cannot run this. It prints a named line and passes rather than
//! failing -- but it prints one, because a check that reports nothing when it
//! does not run is the failure mode this repository has found five separate
//! times. Regenerate the dump with
//! `factorio-bot lua dump_31337.lua --headless --bots 4 --seed 31337 --new`.

use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::plan_work::{WorkCounts, measure};
use factorio_bot_planner::{BotId, Goal, PlanState, pick_chain_actor, plan_best, registry_for};
use std::path::PathBuf;
use std::sync::Arc;

/// The seed-31337 t=0 dump, or `None` with a loud line saying it is absent.
fn dump() -> Option<Arc<FactorioSurface>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../workspace/scripts/map.json")
        .canonicalize()
        .ok()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let world: FactorioSurface =
        factorio_bot_core::serde_json::from_str(&raw).expect("map.json is a world dump");
    Some(Arc::new(world))
}

fn absent() {
    eprintln!(
        "SKIPPED: workspace/scripts/map.json is not present, so the planning-work \
         ceilings were NOT checked. This is not a pass. Regenerate the dump with \
         `factorio-bot lua dump_31337.lua --headless --bots 4 --seed 31337 --new`."
    );
}

/// Plans `goal` for the four-bot roster and reports what it cost.
fn work_for(world: Arc<FactorioSurface>, goal: Goal) -> (usize, WorkCounts) {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = PlanState::from_world(world, &bots);
    let chain_actor = pick_chain_actor(&state, &bots).expect("the roster has a chain actor");
    let (planned, work) = measure(|| {
        plan_best(
            std::slice::from_ref(&goal),
            &state,
            &registry_for(&bots),
            chain_actor,
            &bots,
        )
    });
    let (net, _schedule) = planned.expect("the baseline goal plans");
    (net.len(), work)
}

/// Ceilings for the two composed bundles: (goals expanded, threat queries,
/// preds, forks), twice what each read on the first binary that planned them
/// (2026-09-09, this dump, four bots):
///
/// ```text
///   goal                                        actions  goals  threats  preds   forks
///   all{iron:12, copper:6, red:6}                 1,559  1,739   64,125  3,118  65,544
///   all{iron:30, transport-belt:6}                1,501  1,740   62,610  3,002  59,413
/// ```
///
/// Forks sit at 50% of these by construction; the old green row's 60,756
/// ceiling stood at 81%, so the composed red bundle has more headroom than
/// the lone green cell had, not less.
const CEIL_RED: (u64, u64, u64, u64) = (3_478, 128_250, 6_236, 131_088);
const CEIL_BELT: (u64, u64, u64, u64) = (3_480, 125_220, 6_004, 118_826);

/// Every ceiling in one place, so one dump load covers all three goals.
///
/// One test rather than three: loading an 864 MB dump takes longer than
/// planning does, and three tests would load it three times -- `cargo test`
/// runs them in parallel and would hold three copies at once.
#[test]
fn the_three_map_json_baselines_stay_within_their_work_ceilings() {
    let Some(world) = dump() else {
        absent();
        return;
    };

    // Measured on f2530be6, four bots, `workspace/scripts/map.json`, after the
    // four 2026-09-08 remedies (patch memo, `any_resource_at` union index,
    // incremental cycle check, scheduler adjacency and fork elision):
    //
    //   goal                                actions  goals  patches  threats  preds   forks
    //   researched:automation                   176    260        4   20,964    352   1,926
    //   producing:automation-science-pack:6     316    430        4   22,008    632   6,830
    //   producing:logistic-science-pack:6       441  1,321        4   46,551  2,002  30,378
    //
    // **Green moved 441 -> 571 and this file did not notice for a day**, because
    // the test returns early wherever `workspace/scripts/map.json` is absent --
    // which is every worktree, since a worktree has no `workspace/`. Three
    // agents reported "the planner suite is green" in a run where this test
    // never executed. `absent()` says "This is not a pass" and that is exactly
    // right; nobody read it. The 571 is the canonical BASELINES.md figure and
    // was measured on both sides of the `yields_at` change, so it is the cell
    // work that moved it, not the method-ordering fix.
    //
    // Re-measured on 76540e32, same four bots and dump, with green at 571:
    //
    //   goal                                actions  goals  patches  threats  preds   forks
    //   producing:logistic-science-pack:6       571  1,668        0   61,193  2,550  48,984
    //
    // **The ceilings are deliberately NOT re-derived from these.** Twice the
    // new forks figure would be 97,968 against the 60,756 standing here, so
    // "2x the latest measurement" would SLACKEN a guard that still passes --
    // the opposite of what a ceiling is for. They stay where they are. What is
    // worth saying out loud is the headroom: forks now sits at 81% of its
    // ceiling and threats at 66%, so the next legitimate growth in green trips
    // this test. That is the tripwire working, and whoever it stops should
    // re-measure and decide, not raise the number to make it quiet.
    //
    // Ceilings are twice each of those, except `resource_patches`, whose
    // ceiling is 16 rather than 8: the memo means the count is now the number
    // of *distinct resource names a plan asks about*, a small integer that a
    // legitimate change (a goal reaching one more ore) moves by one, and a
    // doubling rule on a number that small is a tripwire rather than a guard.
    //
    // On b5f44da3, before those remedies, the same three goals read
    // 479/935/2,587 patches and 18,176/59,834/297,049 preds. The ceilings were
    // twice *those*, and are tightened here for the reason the file's own doc
    // gives: a ceiling that survives its own remedy is not measuring anything.
    // **Re-keyed 2026-09-09, when the science cell lost its chests.** A red
    // cell is belted from a standing stage-1 cell for each plate and REFUSES
    // by name without one (`AssemblyNoStandingSource`), so a lone
    // `producing:automation-science-pack:6` is no longer a plan; the number
    // that matters is the composed bundle `continuous_supply.lua` runs --
    // both sustains at the cell's own demand, then the cell. Green needs a
    // gear cell `assembly_spec` cannot express and did not survive the same
    // composition on this dump (no room within the ring), so the third case
    // is the belt cell on one iron sustain, which is the same shape with one
    // belted plate. Ceilings are twice what these read on the first binary
    // that planned them (this worktree, `8d3cbe27` plus the change), except
    // `resource_patches`, kept at 16 for the reason above.
    let bundle = |item: &str, per_minute: u32, sustains: &[(&str, u32)]| {
        let mut goals: Vec<Goal> = sustains
            .iter()
            .map(|(plate, rate)| Goal::Sustain {
                item: (*plate).to_owned(),
                per_minute: *rate,
                window_ticks: 36_000,
            })
            .collect();
        goals.push(Goal::Producing {
            item: item.to_owned(),
            per_minute,
        });
        Goal::All(goals)
    };
    let cases: Vec<(&str, Goal, usize, WorkCounts)> = vec![
        (
            "researched:automation",
            Goal::Researched("automation".to_owned()),
            176,
            WorkCounts {
                goals_expanded: 520,
                resource_patches: 16,
                threat_queries: 41_928,
                preds: 704,
                forks: 3_852,
            },
        ),
        (
            "all{sustain:iron-plate:12, sustain:copper-plate:6, producing:automation-science-pack:6}",
            bundle(
                "automation-science-pack",
                6,
                &[("iron-plate", 12), ("copper-plate", 6)],
            ),
            1_559,
            WorkCounts {
                goals_expanded: CEIL_RED.0,
                resource_patches: 16,
                threat_queries: CEIL_RED.1,
                preds: CEIL_RED.2,
                forks: CEIL_RED.3,
            },
        ),
        (
            "all{sustain:iron-plate:30, producing:transport-belt:6}",
            bundle("transport-belt", 6, &[("iron-plate", 30)]),
            1_501,
            WorkCounts {
                goals_expanded: CEIL_BELT.0,
                resource_patches: 16,
                threat_queries: CEIL_BELT.1,
                preds: CEIL_BELT.2,
                forks: CEIL_BELT.3,
            },
        ),
    ];

    let mut over: Vec<String> = Vec::new();
    for (name, goal, expected_actions, ceiling) in cases {
        let (actions, work) = work_for(Arc::clone(&world), goal);
        // Stated here as well as in `CLAUDE.md` so that a work regression and
        // a *plan* regression are told apart in the same run: a count that
        // halved because the plan collapsed is not a speedup.
        assert_eq!(
            actions, expected_actions,
            "{name} planned {actions} actions, not the baseline {expected_actions} -- \
             the work counts below say nothing until this agrees"
        );
        let mut check = |label: &str, got: u64, cap: u64| {
            if got > cap {
                over.push(format!("{name}: {label} {got} exceeds the ceiling {cap}"));
            }
        };
        check(
            "goals expanded",
            work.goals_expanded,
            ceiling.goals_expanded,
        );
        check(
            "resource patches",
            work.resource_patches,
            ceiling.resource_patches,
        );
        check(
            "threat queries",
            work.threat_queries,
            ceiling.threat_queries,
        );
        check("preds scans", work.preds, ceiling.preds);
        check("state forks", work.forks, ceiling.forks);
        eprintln!("{name}: {}", work.lines().join(" | "));
    }
    assert!(
        over.is_empty(),
        "planning work grew past its ceilings:\n  {}\n\nIf the growth is \
         deliberate, raise the ceiling in this file and say what bought it. If \
         it is not, this is the regression `CLAUDE.md`'s \"~4 seconds\" hid for \
         months.",
        over.join("\n  ")
    );
}
