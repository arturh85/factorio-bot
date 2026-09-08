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
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from falsify_sweep import main  # noqa: E402  (after sys.path)

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


sys.exit(main(MUTATIONS, "cargo test -p factorio-bot-planner --lib products::", "products::"))
