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

use factorio_bot_core::plan_work::{WorkCounts, measure};
use factorio_bot_core::factorio::world::FactorioSurface;
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

    // Measured on b5f44da3, four bots, `workspace/scripts/map.json`:
    //
    //   goal                                actions  goals  patches  threats    preds  forks
    //   researched:automation                   176    260      479   20,964   18,176  2,619
    //   producing:automation-science-pack:6     316    430      935   22,008   59,834  9,438
    //   producing:logistic-science-pack:6       441  1,321    2,587   46,551  297,049 41,687
    //
    // Ceilings are twice each of those.
    let cases: Vec<(&str, Goal, usize, WorkCounts)> = vec![
        (
            "researched:automation",
            Goal::Researched("automation".to_owned()),
            176,
            WorkCounts {
                goals_expanded: 520,
                resource_patches: 958,
                threat_queries: 41_928,
                preds: 36_352,
                forks: 5_238,
            },
        ),
        (
            "producing:automation-science-pack:6",
            Goal::Producing {
                item: "automation-science-pack".to_owned(),
                per_minute: 6,
            },
            316,
            WorkCounts {
                goals_expanded: 860,
                resource_patches: 1_870,
                threat_queries: 44_016,
                preds: 119_668,
                forks: 18_876,
            },
        ),
        (
            "producing:logistic-science-pack:6",
            Goal::Producing {
                item: "logistic-science-pack".to_owned(),
                per_minute: 6,
            },
            441,
            WorkCounts {
                goals_expanded: 2_642,
                resource_patches: 5_174,
                threat_queries: 93_102,
                preds: 594_098,
                forks: 83_374,
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
        check("goals expanded", work.goals_expanded, ceiling.goals_expanded);
        check(
            "resource patches",
            work.resource_patches,
            ceiling.resource_patches,
        );
        check("threat queries", work.threat_queries, ceiling.threat_queries);
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
