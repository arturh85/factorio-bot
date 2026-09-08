#!/usr/bin/env python3
"""Does anything object when the plant stops growing?

Three mutations, each of which puts the pre-2026-09-08 ceiling back by a
different route: drop the committed demand from the sizing sum, drop it inside
the fold, and stop recognising the plant's own standing engine.
"""
import os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from falsify_sweep import main

P = "crates/planner/src/method/power.rs"

MUTATIONS = [
    (
        "the sizing forgets the network's own load (the defect, restored)",
        P,
        "plant_size_for(state, kw + committed_kw(state, &pump.position, facing))",
        "plant_size_for(state, kw)",
    ),
    (
        "committed_kw folds every reading away and answers zero",
        P,
        ".fold(0., f64::max)",
        ".fold(0., |carried, _| carried)",
    ),
    (
        "committed_kw does not recognise the plant's own engine",
        P,
        "if entity.name == ENGINE",
        "if entity.name != ENGINE",
    ),
]

sys.exit(main(MUTATIONS, "nix develop -c cargo test -p factorio-bot-planner --lib -j 4", "method::power::tests::"))
