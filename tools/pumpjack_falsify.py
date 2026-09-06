#!/usr/bin/env python3
"""Break `method::extract` on purpose, one substitution at a time, and record
which of its tests go red.

Every substitution asserts its own match count *before* anything is compiled:
a `str.replace` that matched nothing leaves the suite green, and a green run
under substitution is indistinguishable from "the forbidden value changes
nothing, so the test is hollow". See
docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md.
"""
import subprocess, sys, os, shutil, json

import os
os.chdir(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
SRC = "crates/planner/src/method/extract.rs"
BAK = SRC + ".falsify-backup"

CASES = [
    (
        "site rounded to an integer position (the half-tile trap)",
        "            return Ok(tile.clone());",
        "            return Ok(Position::new(tile.x().round(), tile.y().round()));",
        1,
    ),
    (
        "pole spacing widened past a small pole's wire reach",
        "    let steps = [POLE_STEP, POLE_STEP * 0.66, POLE_STEP * 0.4];",
        "    let steps = [WIRE_REACH * 1.5];",
        1,
    ),
    (
        "the build-grid guard removed, so an even footprint is centred anyway",
        "        if !on_grid(tile) {",
        "        if false {",
        1,
    ),
    (
        "the route's final Condition::Powered check removed",
        "    if !powered.holds(trial, actor) {\n        return Ok(None);\n    }\n    Ok(Some(path))",
        "    Ok(Some(path))",
        1,
    ),
    (
        "the placement no longer states Condition::Powered",
        "                powered,\n            ],",
        "            ],",
        1,
    ),
    (
        "the power placements are no longer ordered before the extractor",
        "        for id in power_ids {",
        "        for id in Vec::<ActionId>::new() {",
        1,
    ),
    (
        "the goal is claimed whatever the recipe gate says",
        "        matches!(\n            recipe_gate(state, &recipe),\n            RecipeGate::Open | RecipeGate::PlannedResearch(_)\n        )",
        "        let _ = recipe;\n        true",
        1,
    ),
    (
        "the unlock is dropped instead of riding on the placement",
        "        if let Some(tech) = unlocks {\n            eff.push(Effect::Researched(tech.clone()));\n        }",
        "        let _ = unlocks;",
        1,
    ),
    (
        "an occupied tile is accepted as a site",
        "        if state.is_area_free_facing(extractor, tile, Direction::North) {",
        "        if true {",
        1,
    ),
]


def run_tests():
    out = subprocess.run(
        ["nix", "develop", "-c", "cargo", "test", "-p", "factorio-bot-planner",
         "--lib", "extract", "--", "--nocapture"],
        capture_output=True, text=True,
    )
    text = out.stdout + out.stderr
    return out.returncode, text


def main():
    shutil.copy(SRC, BAK)
    original = open(SRC).read()
    results = []
    try:
        for name, old, new, expected in CASES:
            count = original.count(old)
            assert count == expected, (
                f"SUBSTITUTION MATCHED {count} TIMES, EXPECTED {expected}: {name!r}\n"
                f"pattern: {old!r}"
            )
            open(SRC, "w").write(original.replace(old, new))
            code, text = run_tests()
            ran = text.count("method::extract")
            failed = sorted(
                line.split("::")[-1].strip()
                for line in text.splitlines()
                if line.strip().startswith("extract_siting_tests::")
                and "FAILED" not in line
                and line.strip().count(" ") == 0
            )
            # `failures:` block lists bare test paths, one per line, indented.
            block = []
            grab = False
            for line in text.splitlines():
                if line.strip() == "failures:":
                    grab = True
                    continue
                if grab:
                    s = line.strip()
                    if not s:
                        grab = False
                        continue
                    if s.startswith("method::extract"):
                        block.append(s.split("::")[-1])
                    else:
                        grab = False
            results.append({
                "case": name,
                "exit": code,
                "compiled": "error[E" not in text and "error: could not compile" not in text,
                "failed_tests": sorted(set(block)),
            })
            open(SRC, "w").write(original)
    finally:
        shutil.copy(BAK, SRC)
        os.remove(BAK)
    print(json.dumps(results, indent=1))
    hollow = [r for r in results if r["exit"] == 0]
    if hollow:
        print("\nHOLLOW (green under substitution -- investigate the experiment):")
        for r in hollow:
            print("  " + r["case"])
        sys.exit(1)


main()
