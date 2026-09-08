#!/usr/bin/env python3
"""Falsification sweep for the machine-choice rule (`method/machine.rs`).

Same shape, and the same two traps avoided, as `tools/falsify_ambiguity.py`,
which this is modelled on:

  * a mutation that fails to COMPILE reads as green, so it is reported as its
    own outcome and never folded into "the tests failed";
  * restoring by `shutil.copy2` / `cp -p` PRESERVES mtime, so cargo would
    re-run the MUTATED binary against restored source. Every restore here
    rewrites the bytes and then touches the file.

A GREEN mutation is a FINDING: no test objects to a wrong implementation.

Usage: nix develop -c python3 tools/falsify_machine_choice.py
"""
import os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MACHINE = "crates/planner/src/method/machine.rs"

MUTATIONS = [
    ("preference becomes a filter: a TIE silently picks the first by name", MACHINE,
     "[(best, winner), (second, _), ..] if best < second => Some((*winner).clone()),",
     "[(_, winner), ..] => Some((*winner).clone()),"),
    ("choose the most expensive machine instead of the cheapest", MACHINE,
     "        priced.sort();",
     "        priced.sort();\n        priced.reverse();"),
    ("choose by name, not by cost", MACHINE,
     "        priced.sort();",
     "        priced.sort_by(|a, b| a.1.cmp(b.1));"),
    ("an unpriceable machine counts as free", MACHINE,
     ".filter_map(|m| self.obtain_cost.get(m).map(|c| (*c, m)))",
     ".map(|m| (self.obtain_cost.get(m).copied().unwrap_or(0), m))"),
    ("price through recycling recipes", MACHINE,
     "        .filter(|r| r.category != crate::products::RECYCLING_CATEGORY)\n        .collect();",
     "        .filter(|_r| true)\n        .collect();"),
    ("one relaxation round instead of a fixpoint", MACHINE,
     "    const RELAX_ROUNDS: usize = 64;",
     "    const RELAX_ROUNDS: usize = 1;"),
    ("ignore how many the recipe yields", MACHINE,
     "                let each = inputs / f64::from(product.amount);",
     "                let each = inputs;"),
    ("ignore how many of each ingredient the recipe wants", MACHINE,
     "Some(c) if c.is_finite() => inputs += f64::from(ingredient.amount) * c,",
     "Some(c) if c.is_finite() => inputs += c,"),
    ("nothing is raw, so nothing can be priced", MACHINE,
     "                cost.insert(&ingredient.name, 1.0);",
     "                let _ = &ingredient.name;"),
    ("runnable set decides for itself again instead of asking machine_for", MACHINE,
     "            if self.machine_for(category).is_ok() {",
     "            if self.declared[category].len() == 1 {"),
    ("ignore what the ground supplies (the defect that made this inert)", MACHINE,
     "    for name in ground {\n        cost.insert(name.as_str(), 1.0);\n    }",
     "    let _ = ground;"),
    ("from_state prices without the ground", MACHINE,
     "            &crate::products::ground_supply(state.base()),",
     "            &BTreeSet::new(),"),
    ("from_state hands the pricing no recipes", MACHINE,
     "            recipes.iter(),\n            &crate::products::ground_supply(state.base()),",
     "            std::iter::empty::<&FactorioRecipe>(),\n            &crate::products::ground_supply(state.base()),"),
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
            r = run("cargo test -p factorio-bot-planner --lib method::machine 2>&1")
            out = r.stdout + r.stderr
            # A compile failure is told apart by the ABSENCE of a test run, not
            # by the word "error": `cargo test` prints `error: test failed` on
            # an ordinary red run too.
            if "test result:" not in out:
                verdict = "DID NOT COMPILE -- reads as green, treat as no evidence"
            elif r.returncode == 0:
                verdict = "GREEN -- A FINDING: no test objects to this"
            else:
                failed = [
                    l.strip()
                    for l in out.splitlines()
                    if l.strip().startswith("test method::machine::") and "FAILED" in l
                ]
                verdict = "red (%d): %s" % (
                    len(failed),
                    ", ".join(f.split()[1].split("::")[-1] for f in failed[:4]),
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
