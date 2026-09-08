#!/usr/bin/env python3
"""Does anything actually test how a fluid machine's site is anchored?

Five mutations of `method::fabricate`'s siting seam. Each one is a way the
change could be wrong that a reader would not see, and a GREEN verdict on any
of them says the tests around it are decoration.

The first is the defect itself, put back: anchoring on `sources.first()` is what
made `have:sulfur:10` report 234 route failures on clear ground.

Run from anywhere; `falsify_sweep` resolves the repo root from its own path, so
run it from the worktree whose code you mean to falsify.
"""

import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from falsify_sweep import main  # noqa: E402

FILE = "crates/planner/src/method/fabricate.rs"

MUTATIONS = [
    (
        "the defect itself: anchor on the FIRST source again",
        FILE,
        """    Position::new(f64::midpoint(min_x, max_x), f64::midpoint(min_y, max_y))""",
        """    let _ = (min_x, max_x, min_y, max_y);
    first.clone()""",
    ),
    (
        "the pre-check never fires (bound raised past any map)",
        FILE,
        """    let reach = PIPE_RUN_REACH;""",
        """    let reach = PIPE_RUN_REACH * 1_000_000.;""",
    ),
    (
        "the loose bound the fixture caught: reach + the ring radius",
        FILE,
        """    let reach = PIPE_RUN_REACH;""",
        """    let reach = PIPE_RUN_REACH + ring_reach();""",
    ),
    (
        "distance measured Euclidean instead of Chebyshev",
        FILE,
        """            let away = (entity.position.x() - anchor.x())
                .abs()
                .max((entity.position.y() - anchor.y()).abs());""",
        """            let away = (entity.position.x() - anchor.x())
                .hypot(entity.position.y() - anchor.y());""",
    ),
    (
        "the anchor takes the bounding box's CORNER, not its centre",
        FILE,
        """    Position::new(f64::midpoint(min_x, max_x), f64::midpoint(min_y, max_y))""",
        """    Position::new(min_x, min_y)""",
    ),
]

if __name__ == "__main__":
    sys.exit(
        main(
            MUTATIONS,
            "nix develop -c cargo test -p factorio-bot-planner --lib",
            prefix="method::fabricate",
        )
    )
