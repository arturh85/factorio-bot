#!/usr/bin/env python3
"""Does anything object when `pipe::traced` stops telling the truth?

The edge under test attributes a `storage-tank` this plan just built to the
fluid the machine it is piped to makes. Its failure mode is the silent one
this repository keeps paying for: a wrong attribution builds a perfectly
correct pipe run to a machine that then makes nothing, and no schedule, no
count and no makespan moves. So the tests have to be shown to have teeth.

Seven mutations, each breaking one claim the module's doc makes:

1-2. the two directions of `Traced::Carries` -- attribute everything, and
     attribute nothing;
3.   collapse `Ambiguous` into "the first fluid", the guess the whole
     four-answer shape exists to refuse;
4.   let a named fluid outvote a producer nothing could name;
5-6. confuse two of the three absences with each other (`Unnamed` reported as
     `Unconnected`, and `Unconnected` reported as though something was
     joined);
7.   `supplying_end` forgets which box a recipe names, which is the latent
     ambiguity this change also closes.

Run from the repository root (or a worktree of it):

    nix develop -c python3 tools/falsify_pipe_trace.py
"""

import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from falsify_sweep import main  # noqa: E402

PIPE = "crates/planner/src/method/pipe.rs"

MUTATIONS = [
    (
        "Carries attributes every fluid, not the one it names",
        PIPE,
        "            Traced::Carries(carried) => return carried == fluid,",
        "            Traced::Carries(_carried) => return true,",
    ),
    (
        "Carries attributes nothing (the trace becomes a refusal only)",
        PIPE,
        "            Traced::Carries(carried) => return carried == fluid,",
        "            Traced::Carries(_carried) => return false,",
    ),
    (
        "Ambiguous guesses the first fluid instead of refusing",
        PIPE,
        "        _ => Traced::Ambiguous(fluids),",
        "        _ => match fluids.iter().next() {\n"
        "            Some(first) => Traced::Carries(first.clone()),\n"
        "            None => Traced::Ambiguous(fluids),\n"
        "        },",
    ),
    (
        "a producer nothing can name is ignored when another is nameable",
        PIPE,
        "        (1, false) => Traced::Carries(",
        "        (1, _) => Traced::Carries(",
    ),
    (
        "Unnamed is reported as Unconnected",
        PIPE,
        "        (0, _) => Traced::Unnamed,",
        "        (0, _) => Traced::Unconnected,",
    ),
    (
        "an unjoined buffer reports as joined-but-unnamed",
        PIPE,
        "    if !reached {\n        return Traced::Unconnected;\n    }",
        "    if !reached {\n        return Traced::Unnamed;\n    }",
    ),
    (
        "supplying_end forgets which output box a recipe names",
        PIPE,
        '        Some((_, ordinal)) => (Some("output"), Some(ordinal)),',
        '        Some((_, _ordinal)) => (None, None),',
    ),
]

if __name__ == "__main__":
    sys.exit(
        main(
            MUTATIONS,
            "nix develop -c cargo test -p factorio-bot-planner --lib",
            prefix="method::pipe",
        )
    )
