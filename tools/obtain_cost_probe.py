#!/usr/bin/env python3
"""Rank a product's recipes by obtain cost, against a real world dump.

A **mirror** of `crates/planner/src/method/machine.rs::obtain_costs`, and
deliberately not production code: it exists so a question about the metric can
be answered against 662 real recipes in a few seconds, before any Rust is
written. It produced the barrelling table quoted in
`ProductIndex::cheapest_to_obtain`'s own doc, and it is committed because a
number in a doc comment whose source lives only in somebody's scratch
directory cannot be re-derived -- this repo has already lost three published
results that way.

**It is a mirror, so it can drift.** The authority is `obtain_costs`; the tests
in `products.rs` are what hold the rule. Use this to look, never to decide.

The ground supply is passed in rather than read off the dump, because
`products::ground_supply` reads `EntityGraph::resource_names_present` and the
tiles' own `fluid`, neither of which is a plain field of the JSON.

    GROUND=crude-oil,water,iron-ore,copper-ore,coal,stone,uranium-ore \
      python3 tools/obtain_cost_probe.py \
      workspace/scripts/map-31337-explored-with-categories.json petroleum-gas

Cost is in raw input units per unit of the product; a raw input is anything no
recipe produces, plus anything the ground supplies. Recycling is excluded, as
it is in `obtain_costs`.
"""

import json
import math
import os
import sys

path = sys.argv[1]
with open(path) as f:
    world = json.load(f)

recipes_raw = world["recipes"]
recipes = list(recipes_raw.values()) if isinstance(recipes_raw, dict) else recipes_raw
print(f"recipes: {len(recipes)}", file=sys.stderr)

ground = set(os.environ.get("GROUND", "").split(",")) - {""}
print("ground:", sorted(ground), file=sys.stderr)

RECYCLING = "recycling"
useful = [r for r in recipes if r.get("category") != RECYCLING]
produced = {p["name"] for r in useful for p in (r.get("products") or [])}

cost = {n: math.inf for n in produced}
for r in useful:
    for i in r.get("ingredients") or []:
        if i["name"] not in produced:
            cost[i["name"]] = 1.0
for n in ground:
    cost[n] = 1.0

for _ in range(64):
    improved = False
    for r in useful:
        total = 0.0
        known = True
        for i in r.get("ingredients") or []:
            c = cost.get(i["name"])
            if c is None or not math.isfinite(c):
                known = False
                break
            total += i["amount"] * c
        if not known:
            continue
        for p in r.get("products") or []:
            if p["amount"] == 0:
                continue
            each = total / p["amount"]
            if each < cost.get(p["name"], math.inf):
                cost[p["name"]] = each
                improved = True
    if not improved:
        break


def rank(product):
    rows = []
    for r in useful:
        if not any(p["name"] == product for p in (r.get("products") or [])):
            continue
        total = 0.0
        known = True
        for i in r.get("ingredients") or []:
            c = cost.get(i["name"])
            if c is None or not math.isfinite(c):
                known = False
                break
            total += i["amount"] * c
        amount = next(p["amount"] for p in r["products"] if p["name"] == product)
        per = total / amount if known and amount else math.inf
        rows.append((per, r["name"], r.get("category"), [p["name"] for p in r["products"]]))
    rows.sort(key=lambda t: (t[0], t[1]))
    print(f"\n=== {product} (item cost {cost.get(product)}) ===")
    for per, name, cat, prods in rows:
        print(f"  {per:12.4f}  {name:42s} [{cat}] -> {prods}")


for product in sys.argv[2:]:
    rank(product)
