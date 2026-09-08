#!/usr/bin/env python3
"""Falsification sweep for the chemistry rung and the obtain-cost tie-break.

Same shape and the same two traps as `tools/falsify_ambiguity.py`, which this
is modelled on rather than reinventing:

  * a mutation that fails to COMPILE reads as green, so it is reported as its
    own outcome and never folded into "the tests failed";
  * a restore that preserves mtime makes cargo re-run the MUTATED binary
    against restored source, so every restore here rewrites the bytes and
    then touches the file.

Runs the WHOLE planner lib, not one module: the rules under test live in
`products.rs` and `method/supply.rs` and are exercised from both.

Usage: nix develop -c python3 tools/falsify_fluid_supply.py
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PRODUCTS = "crates/planner/src/products.rs"
SUPPLY = "crates/planner/src/method/supply.rs"
FABRICATE = "crates/planner/src/method/fabricate.rs"

MUTATIONS = [
    # ---- the chemistry rung -----------------------------------------------
    (
        "supply: bottle a fluid the ground already yields",
        SUPPLY,
        "    if index.ground_supplies(fluid) {\n        return false;\n    }",
        "    if false && index.ground_supplies(fluid) {\n        return false;\n    }",
    ),
    (
        "supply: derive `Produced` for any fluid, whether or not a recipe makes it",
        SUPPLY,
        "        } else if makeable(state, fluid, &recipe) {",
        "        } else if true {",
    ),
    (
        "supply: prefer a plant over the ground",
        SUPPLY,
        "        if state.has_resource_patches(fluid) && extract::extractor_for(state, fluid).is_ok() {",
        "        if false && state.has_resource_patches(fluid) && extract::extractor_for(state, fluid).is_ok() {",
    ),
    (
        "supply: ask for one unit instead of the recipe's own amount",
        SUPPLY,
        "                amount: ingredient.amount,",
        "                amount: 1,",
    ),
    (
        "supply: let a recipe feed itself",
        SUPPLY,
        "    recipe.name != consumer.name && recipe_gate(state, recipe) != RecipeGate::Unobtainable",
        "    recipe_gate(state, recipe) != RecipeGate::Unobtainable",
    ),
    # ---- rule 3: cheapest to obtain ---------------------------------------
    (
        "rule 3: never consult obtain cost",
        PRODUCTS,
        "        let runnable = if runnable.len() > 1 {\n            self.cheapest_to_obtain(&runnable, product)",
        "        let runnable = if false && runnable.len() > 1 {\n            self.cheapest_to_obtain(&runnable, product)",
    ),
    (
        "rule 3: let a TIE win, so the preference becomes a filter",
        PRODUCTS,
        "            [(best, _, winner), (second, ..), ..] if best < second => vec![*winner],",
        "            [(_, _, winner), ..] => vec![*winner],",
    ),
    (
        "rule 3: count ingredients instead of pricing them",
        PRODUCTS,
        "                    Some(c) => inputs += u128::from(ingredient.amount) * u128::from(*c),",
        "                    Some(_) => inputs += u128::from(ingredient.amount),",
    ),
    (
        "rule 3: ignore how much a recipe yields",
        PRODUCTS,
        "                priced.push((inputs * YIELD_SCALE / amount, recipe.name.as_str(), recipe));",
        "                priced.push((inputs * YIELD_SCALE, recipe.name.as_str(), recipe));",
    ),
    # ---- siting ------------------------------------------------------------
    (
        "fabricate: site a fluid machine without checking a run can reach it",
        FABRICATE,
        "|candidate: &Position| ports_fit(candidate) && routes_from_every_source(candidate);",
        "|candidate: &Position| ports_fit(candidate);",
    ),
]


def run(cmd):
    return subprocess.run(cmd, cwd=ROOT, shell=True, capture_output=True, text=True)


def main():
    results = []
    for label, path, old, new in MUTATIONS:
        full = os.path.join(ROOT, path)
        original = open(full).read()
        n = original.count(old)
        if n != 1:
            results.append((label, f"SUBSTITUTION MATCHED {n} TIMES -- not run"))
            continue
        try:
            open(full, "w").write(original.replace(old, new))
            os.utime(full, (time.time(), time.time()))
            r = run("cargo test -p factorio-bot-planner --lib 2>&1")
            out = r.stdout + r.stderr
            if "test result:" not in out:
                verdict = "DID NOT COMPILE -- reads as green, treat as no evidence"
            elif r.returncode == 0:
                verdict = "GREEN -- A FINDING: no test objects to this"
            else:
                failed = [
                    line.strip()
                    for line in out.splitlines()
                    if line.strip().startswith("test ") and "FAILED" in line
                ]
                verdict = "red (%d): %s" % (
                    len(failed),
                    ", ".join(f.split()[1] for f in failed[:4]),
                )
        finally:
            open(full, "w").write(original)
            os.utime(full, (time.time(), time.time()))
        results.append((label, verdict))
    print()
    for label, verdict in results:
        print(f"  {label}\n      {verdict}")
    return 0


sys.exit(main())
