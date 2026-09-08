#!/usr/bin/env python3
"""Does anything actually object to a wrong disposal?

Eight mutations of `method::dispose` and its one caller in `method::fabricate`.
Each is a way the rung could be wrong that a plan would still *look* fine
under -- the recursion opened, the surplus under-provisioned, the guard
silenced -- and each must be caught by a named test rather than by a reviewer.

The one this sweep exists for is mutation 1: **a non-terminal recipe must not
be selectable as disposal.** Cracking is fluid-to-fluid, so choosing it puts
the recursion the terminal rule forbids straight back in, and the plan would
still place every entity correctly.

Run: nix develop -c python3 tools/falsify_disposal.py
"""

import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from falsify_sweep import main  # noqa: E402

DISPOSE = "crates/planner/src/method/dispose.rs"
FABRICATE = "crates/planner/src/method/fabricate.rs"

MUTATIONS = [
    (
        "1. a fluid product does not disqualify a recipe -- cracking becomes disposal",
        DISPOSE,
        """fn is_terminal(substances: &SubstanceTable, recipe: &FactorioRecipe) -> bool {
    !recipe
        .products
        .iter()
        .any(|product| substances.is_fluid(&product.name))
}""",
        """fn is_terminal(_substances: &SubstanceTable, _recipe: &FactorioRecipe) -> bool {
    true
}""",
    ),
    (
        "2. the terminal filter is dropped from the ranking entirely",
        DISPOSE,
        "        .filter(|recipe| is_terminal(index.substances(), recipe))\n",
        "",
    ),
    (
        "3. ranked by name alone -- rule 1 (fewest other ingredients) gone",
        DISPOSE,
        "        (x.0, x.1.is_none(), x.1.unwrap_or(0), x.2)",
        "        (0, false, 0, x.2)",
    ),
    (
        "4. the ranking ignores cost -- rules 2 and 3 collapse to name",
        DISPOSE,
        "        (x.0, x.1.is_none(), x.1.unwrap_or(0), x.2)",
        "        (x.0, false, 0, x.2)",
    ),
    (
        "5. the disposal plant is sized by rounding DOWN -- the last of the surplus jams",
        DISPOSE,
        "        let crafts = total.div_ceil(u64::from(self.consumed));",
        "        let crafts = total / u64::from(self.consumed);",
    ),
    (
        "6. the goal's own product is disposed of too, not only the co-products",
        FABRICATE,
        "        if fluid == item {\n            continue;\n        }",
        "        if false {\n            continue;\n        }",
    ),
    (
        "7. the subgoals are resolved and then never emitted",
        FABRICATE,
        "            steps.push(Step::Subgoal(sub));",
        "            let _ = sub;",
    ),
    (
        "8. THE GUARD IS SILENCED: a co-product with nowhere to go is allowed through",
        FABRICATE,
        "        let Some(disposal) = pipe_away(&ctx.state, &fluid, whose) else {",
        "        let Some(disposal) = pipe_away(&ctx.state, &fluid, whose) else {\n            continue;\n            #[allow(unreachable_code)]",
    ),
]

if __name__ == "__main__":
    sys.exit(
        main(
            MUTATIONS,
            "nix develop -c cargo test -p factorio-bot-planner --lib --test suite",
        )
    )
