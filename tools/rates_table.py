#!/usr/bin/env python3
"""Print the plan record's production table for one or more runs, as Markdown.

    python3 tools/rates_table.py workspace/runs/run-A workspace/runs/run-B
    python3 tools/rates_table.py --what rate --marks 5,10,15,20 workspace/runs/run-A
    python3 tools/rates_table.py --items iron-plate,automation-science-pack ...

One row per mark (game minutes from ``run_started``), one column per run, each
cell the items in ``--items`` order -- the shape the record already uses by
hand (``docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md``). A mark a
run never reached prints the reason ("run ended at 17:59"), never a zero, and a
run with no force samples says so in its column.

Everything is computed by :mod:`run_analysis`; this file only chooses the
columns. ``run_analysis.py --rates-md`` prints the same table with defaults.
"""

from __future__ import annotations

import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_analysis as ra  # noqa: E402

# The four the record has been quoting by hand.
DEFAULT_MD_ITEMS = (
    "iron-plate",
    "copper-plate",
    "automation-science-pack",
    "logistic-science-pack",
)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("dirs", nargs="+", help="run directories, one column each")
    ap.add_argument("--marks", default=",".join(str(m) for m in ra.DEFAULT_MARKS))
    ap.add_argument("--items", default=",".join(DEFAULT_MD_ITEMS),
                    help="items per cell, in order")
    ap.add_argument("--what", choices=("cumulative", "rate"), default="cumulative",
                    help="cumulative counts (default) or /min over the previous mark interval")
    ap.add_argument("--headlines", action="store_true",
                    help="also print each run's one-line headline above the table")
    args = ap.parse_args(argv)
    marks = tuple(float(m) for m in args.marks.split(",") if m.strip())
    items = tuple(i.strip() for i in args.items.split(",") if i.strip())
    analyses = []
    for d in args.dirs:
        if not os.path.isdir(d):
            print(f"no such run directory: {d}", file=sys.stderr)
            return 2
        # Every item asked for is forced into the table so a rarely made one
        # is a 0 rather than a missing column.
        analyses.append(ra.analyse(d, marks=marks, rate_items=items))
    if args.headlines:
        for a in analyses:
            print(f"- `{a['run_id']}`: {a.get('headline')}")
        print()
    print(ra.rates_markdown(analyses, items, args.what))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
