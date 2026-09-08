#!/usr/bin/env python3
"""Is the inserter-reach derivation load-bearing, or is the fallback carrying it?

The reason this sweep exists is specific and is the hazard the whole change
lives under. Every world in `workspace/scripts/` predates
`inserter_pickup_position`, and so does `crates/core/tests/
entity-prototype-fixtures.json` -- so **almost every test in the suite runs
the FALLBACK path**, not the derived one. A derivation that is quietly never
consulted would look exactly like a working one.

That is `a fixture cannot falsify what it derives` from the other side: here
the fixture cannot falsify what it *does not* derive. Mutating the derived
half and watching the suite stay green would be the finding -- it would mean
the six new tests are the only readers and nothing else crosses the bridge at
all.

Run: `nix develop -c python3 tools/falsify_inserter_reach.py`
"""

import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from falsify_sweep import main  # noqa: E402

STATE = "crates/planner/src/state.rs"

MUTATIONS = [
    (
        "the derived pickup is ignored and the vanilla table always answers",
        STATE,
        """    let Some(vector) = prototypes
        .get(name)
        .and_then(|prototype| prototype.inserter_pickup_position.clone())
    else {
        return vanilla_inserter_reach(name);
    };
    axis_reach(-vector.y(), vector.x())""",
        """    let _ = prototypes;
    vanilla_inserter_reach(name)""",
    ),
    (
        "the derived drop is ignored and the vanilla table always answers",
        STATE,
        """    let Some(vector) = prototypes
        .get(name)
        .and_then(|prototype| prototype.inserter_drop_position.clone())
    else {
        return vanilla_inserter_reach(name);
    };
    axis_reach(vector.y(), vector.x())""",
        """    let _ = prototypes;
    vanilla_inserter_reach(name)""",
    ),
    (
        "the fallback is deleted -- an old dump reports no reach at all",
        STATE,
        """        return vanilla_inserter_reach(name);
    };
    axis_reach(-vector.y(), vector.x())""",
        """        return None;
    };
    axis_reach(-vector.y(), vector.x())""",
    ),
    (
        "a vector off the axis falls back instead of refusing",
        STATE,
        """    if vector_tile_offset(across) != 0. {
        return None;
    }""",
        """    let _ = across;""",
    ),
    (
        "the tile offset rounds instead of flooring",
        STATE,
        """fn vector_tile_offset(v: f64) -> f64 {
    (0.5 + v).floor()
}""",
        """fn vector_tile_offset(v: f64) -> f64 {
    v.round()
}""",
    ),
    (
        "a long inserter reaches one tile like every other",
        STATE,
        """        "long-handed-inserter" => Some(2.),""",
        """        "long-handed-inserter" => Some(1.),""",
    ),
]

if __name__ == "__main__":
    sys.exit(
        main(
            MUTATIONS,
            "nix develop -c cargo test -p factorio-bot-planner --lib",
            prefix="state::",
        )
    )
