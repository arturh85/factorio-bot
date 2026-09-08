#!/usr/bin/env python3
"""Falsification sweep for the recycling/reachability rules.

One mutation at a time, each asserted to match EXACTLY ONCE in the file it
edits, then the planner's own tests are run and the mutation reverted.

Two traps this deliberately avoids, both of which have produced a wrong
answer in this repo:

  * a mutation that fails to COMPILE reads as green -- so a compile failure
    is reported as its own outcome, never folded into "the tests failed";
  * `shutil.copy2` and `cp -p` PRESERVE mtime, so restoring a file leaves it
    older than the artifact cargo built from the mutation and cargo re-runs
    the MUTATED binary against restored source. Every restore here rewrites
    the bytes and then touches the file.

Usage: nix develop -c python3 tools/falsify_ambiguity.py
"""
import os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PRODUCTS = "crates/planner/src/products.rs"
GRAPH = "crates/core/src/graph/entity_graph.rs"

MUTATIONS = [
    ("rule 1: stop excluding recycling from the candidate set", PRODUCTS,
     '            .filter(|r| r.category != RECYCLING_CATEGORY)\n            .collect();\n        let all: Vec<&FactorioRecipe> = if productive.is_empty() {',
     '            .filter(|_r| true)\n            .collect();\n        let all: Vec<&FactorioRecipe> = if productive.is_empty() {'),
    ("rule 1: drop the fallback that keeps a recycler in the diagnostic", PRODUCTS,
     '        let all: Vec<&FactorioRecipe> = if productive.is_empty() {\n            every\n        } else {\n            productive\n        };',
     '        let all: Vec<&FactorioRecipe> = productive;\n        let _ = every;'),
    ("rule 2: never consult the ground", PRODUCTS,
     '        let runnable = if runnable.len() > 1 {',
     '        let runnable = if false && runnable.len() > 1 {'),
    ("rule 2: make the preference a filter (no fallback)", PRODUCTS,
     '        if fed.is_empty() {\n            candidates.to_vec()\n        } else {\n            fed\n        }',
     '        fed'),
    ("closure: walk through recycling recipes", PRODUCTS,
     '            .filter(|r| r.category != RECYCLING_CATEGORY && categories.admits(&r.category))',
     '            .filter(|r| categories.admits(&r.category))'),
    ("closure: one pass instead of a fixpoint", PRODUCTS,
     '            if !grew {\n                return reachable;\n            }',
     '            let _ = grew;\n            return reachable;'),
    ("seed: forget the fluid the ground yields", PRODUCTS,
     '        if let Some(fluid) = tile.fluid.named() {\n            supply.insert(fluid.to_string());\n        }',
     '        let _ = tile;'),
    ("seed: report every declared resource prototype as charted", GRAPH,
     '        let mut names: Vec<String> = self\n            .resources\n            .iter()\n            .map(|entry| entry.key().clone())\n            .collect();',
     '        let mut names: Vec<String> = self\n            .entity_prototypes\n            .iter()\n            .filter(|e| e.value().entity_type == "resource")\n            .map(|entry| entry.key().clone())\n            .collect();'),
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
            r = run("cargo test -p factorio-bot-planner --lib products:: 2>&1")
            out = r.stdout + r.stderr
            # **A compile failure is told apart by the ABSENCE of a test
            # run, not by the word "error".** `cargo test` prints
            # `error: test failed, to rerun ...` on an ordinary red run, and
            # matching that string reported six live mutations as
            # uncompilable -- which is exactly the "reads as green" failure
            # this sweep exists to avoid, arriving through the detector
            # instead of through the compiler.
            if "test result:" not in out:
                verdict = "DID NOT COMPILE -- reads as green, treat as no evidence"
            elif r.returncode == 0:
                verdict = "GREEN -- A FINDING: no test objects to this"
            else:
                failed = [l.strip() for l in out.splitlines() if l.strip().startswith("test products::") and "FAILED" in l]
                verdict = "red (%d): %s" % (len(failed), ", ".join(f.split()[1] for f in failed[:4]))
        finally:
            open(full, "w").write(original)
            os.utime(full, (time.time(), time.time()))
        results.append((label, verdict))
    print()
    for label, verdict in results:
        print(f"  {label}\n      {verdict}")
    return 0


sys.exit(main())
