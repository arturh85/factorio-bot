#!/usr/bin/env python3
"""Falsification sweep for the threat-aware target selection tests.

One mutation at a time. Each mutation must match EXACTLY once (asserted), and
the run afterwards must be RED. A mutation that fails to COMPILE reads as green
to a naive harness, so compile failure is reported separately and never counted
as a kill.

Backup/restore is by file COPY plus an explicit `touch`, never `git checkout`:
`cp -p` preserves mtime, so cargo would re-run the mutated binary against
restored source and report a false red.
"""

import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BACKUP = os.path.join(ROOT, "target", "threat-sweep-backup")

STATE = "crates/planner/src/state.rs"
HAVE = "crates/planner/src/method/have.rs"
UTIL = "crates/planner/src/method/util.rs"

# (label, file, find, replace, tests expected to go red)
MUTATIONS = [
    (
        "worm range 25 -> 7",
        STATE,
        '("small-worm-turret", 25.),',
        '("small-worm-turret", 7.),',
        # NOT expected to kill `a_tree_inside_a_worms_reach_is_passed_over...`,
        # and the first run of this sweep proved it does not. That fixture puts
        # its tree 2 tiles from the worm -- deliberately deep inside the reach,
        # because the test is about PREFERENCE (walk past the near one) and not
        # about where the boundary falls. Shrinking 25 to 7 leaves the tree
        # covered, so the test rightly stays green. The boundary is the job of
        # `a_position_is_covered_inside_the_standoff_and_free_outside_it`, which
        # this mutation does kill. The wrong thing here was the expectation,
        # not the test.
        [
            "a_small_worms_standoff_is_the_twenty_five_tiles_the_game_data_states",
            "a_position_is_covered_inside_the_standoff_and_free_outside_it",
            "a_wood_goal_whose_only_tree_is_inside_a_worm_refuses_and_names_the_worm",
        ],
    ),
    (
        "prototype attack_range ignored",
        STATE,
        "            && range > 0.\n        {\n            return ThreatStandoff {\n                tiles: range,\n                source: StandoffSource::Prototype,\n            };\n        }",
        "            && range > 0.\n            && false\n        {\n            return ThreatStandoff {\n                tiles: range,\n                source: StandoffSource::Prototype,\n            };\n        }",
        ["a_prototype_attack_range_outranks_the_named_fallback"],
    ),
    (
        "unknown enemy standoff -> 0 (absent reads as no threat)",
        STATE,
        "pub const WIDEST_KNOWN_STANDOFF: f64 = SPAWNER_CALL_FOR_HELP_RADIUS;",
        "pub const WIDEST_KNOWN_STANDOFF: f64 = 0.;",
        ["an_unrecognised_enemy_structure_is_avoided_widely_rather_than_ignored"],
    ),
    (
        "spawner collapsed onto a worm's reach",
        STATE,
        "pub const SPAWNER_CALL_FOR_HELP_RADIUS: f64 = 50.;",
        "pub const SPAWNER_CALL_FOR_HELP_RADIUS: f64 = 25.;",
        ["a_spawner_is_not_given_a_worms_reach_and_is_never_given_zero"],
    ),
    (
        "threat_covering always answers None",
        STATE,
        "                (distance < standoff.tiles).then_some(ThreatAt {",
        "                (distance < f64::MIN).then_some(ThreatAt {",
        [
            "a_position_is_covered_inside_the_standoff_and_free_outside_it",
            "a_tree_inside_a_worms_reach_is_passed_over_while_a_safe_one_stands",
            "a_wood_goal_whose_only_tree_is_inside_a_worm_refuses_and_names_the_worm",
            "a_worm_clipping_the_iron_field_moves_selection_past_its_reach",
        ],
    ),
    (
        "Chop reads the unfiltered sources again (the original defect)",
        HAVE,
        "        let (mut sources, threatened) = ctx.state.minable_sources_by_safety(&item);",
        "        let mut sources = ctx.state.minable_sources(&item);\n        let threatened: Vec<(String, Position, u32, crate::state::ThreatAt)> = Vec::new();",
        [
            "a_tree_inside_a_worms_reach_is_passed_over_while_a_safe_one_stands",
            "a_wood_goal_whose_only_tree_is_inside_a_worm_refuses_and_names_the_worm",
        ],
    ),
    (
        "threatened_tile always false (ore guard disabled)",
        UTIL,
        "    state.threat_covering(tile).is_some()\n}",
        "    let _ = state.threat_covering(tile);\n    false\n}",
        ["a_worm_clipping_the_iron_field_moves_selection_past_its_reach"],
    ),
    (
        # The control's own falsification. `the_same_tree_is_chopped_when_the_
        # worm_is_out_of_range` guards the OTHER direction -- that the guard
        # does not refuse a clean map -- so no mutation that weakens the guard
        # can kill it. Only one that makes it fire always can, and a control
        # test no mutation can falsify is a test that may assert nothing.
        "threat_covering always answers Some (over-refusing)",
        STATE,
        "                (distance < standoff.tiles).then_some(ThreatAt {",
        "                (distance < f64::MAX).then_some(ThreatAt {",
        ["the_same_tree_is_chopped_when_the_worm_is_out_of_range"],
    ),
]


def snapshot():
    """Back up by COPY. Callers must have a clean tree: this is the only
    record of pre-mutation state, deliberately -- `git checkout` restores the
    last COMMIT, silently discarding uncommitted work, which cost this repo
    four files on 2026-09-08."""
    os.makedirs(BACKUP, exist_ok=True)
    for f in (STATE, HAVE, UTIL):
        shutil.copyfile(os.path.join(ROOT, f), os.path.join(BACKUP, f.replace("/", "_")))


def restore():
    for f in (STATE, HAVE, UTIL):
        dst = os.path.join(ROOT, f)
        shutil.copyfile(os.path.join(BACKUP, f.replace("/", "_")), dst)
        os.utime(dst, None)  # touch: cp preserves mtime, cargo would skip


def run_tests(names):
    """Return dict name -> 'pass'|'fail'|'missing', plus compiled: bool."""
    proc = subprocess.run(
        ["nix", "develop", "-c", "cargo", "test", "-p", "factorio-bot-planner", "--lib"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    out = proc.stdout + proc.stderr
    compiled = "error[E" not in out and "could not compile" not in out
    results = {}
    for n in names:
        if f"{n} ... ok" in out:
            results[n] = "pass"
        elif f"{n} ... FAILED" in out:
            results[n] = "fail"
        else:
            results[n] = "missing"
    return results, compiled


def main():
    snapshot()
    all_names = sorted({n for m in MUTATIONS for n in m[4]})

    base, compiled = run_tests(all_names)
    if not compiled or any(v != "pass" for v in base.values()):
        print("BASELINE NOT GREEN:", base, "compiled:", compiled)
        return 1
    print(f"baseline: all {len(all_names)} target tests pass\n")

    failures = []
    killed = set()
    for label, path, find, repl, expect in MUTATIONS:
        full = os.path.join(ROOT, path)
        src = open(full).read()
        count = src.count(find)
        if count != 1:
            print(f"!! {label}: substitution matched {count} times, expected exactly 1")
            failures.append(label)
            continue
        open(full, "w").write(src.replace(find, repl))
        os.utime(full, None)

        res, compiled = run_tests(expect)
        restore()

        if not compiled:
            print(f"!! {label}: DID NOT COMPILE -- reads as green, not a kill")
            failures.append(label)
            continue
        reds = [n for n, v in res.items() if v == "fail"]
        greens = [n for n, v in res.items() if v != "fail"]
        killed.update(reds)
        status = "OK" if not greens else "GREEN MUTATION"
        print(f"[{status}] {label}")
        for n in reds:
            print(f"      red: {n}")
        for n in greens:
            print(f"    GREEN: {n}  <-- finding, not a pass")
        if greens:
            failures.append(label)

    print()
    unkilled = [n for n in all_names if n not in killed]
    if unkilled:
        print("TESTS NEVER FALSIFIED:", unkilled)
    print("SWEEP RESULT:", "CLEAN" if not failures and not unkilled else "PROBLEMS")
    return 0 if not failures and not unkilled else 1


if __name__ == "__main__":
    sys.exit(main())
