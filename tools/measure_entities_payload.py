#!/usr/bin/env python3
"""Measure what a chunk's `entities` writeout is actually made of.

Reads a Factorio server log, pulls every `§<tick>§entities§<header><json>` line
the BotBridge mod printed, and reports bytes by category.

The input is the REAL wire payload -- the exact bytes `writeout_entities`
printed to stdout and `output_parser.rs` read back -- not a re-serialisation of
a dump. Nothing here is estimated.
"""

import json
import sys
from collections import Counter, defaultdict

# Fields `serialize_entity` (mods/BotBridge/types.lua:571) can attach.
IDENTITY = {"name", "entity_type", "position", "surface"}
GEOMETRY = {"bounding_box", "direction", "drop_position", "pickup_position"}
CONTENTS = {"output_inventory", "fuel_inventory"}
EXTRA = {"amount", "recipe", "ghost_name", "ghost_type", "underground_half"}

# Types the planner arguably does not need as individual records.
# `blocked_tree` / `minables` do consume tree, simple-entity and cliff; the
# question this script answers is how much they cost, not whether to drop them.
SCENERY = {"tree", "simple-entity", "cliff", "fish", "decorative"}
CORPSE = {"corpse", "rail-remnants", "item-entity", "particle-source"}


def compact(v):
    return len(json.dumps(v, separators=(",", ":")))


def main(path):
    total_line_bytes = 0
    total_json_bytes = 0
    n_lines = 0
    n_entities = 0

    field_bytes = Counter()
    field_count = Counter()
    type_bytes = Counter()
    type_count = Counter()
    # per-entity envelope: the `{}` and the commas between fields
    envelope_bytes = 0
    empty_inventories = 0
    nonempty_inventories = 0
    inv_present = 0

    with open(path, "rb") as fh:
        for raw in fh:
            if b"\xc2\xa7entities\xc2\xa7" not in raw:
                continue
            n_lines += 1
            total_line_bytes += len(raw)
            line = raw.decode("utf-8", "replace")
            # §tick§entities§<x,y;x,y:><json>
            _, _, rest = line.partition("§entities§")
            body = rest[rest.index(":") + 1:].rstrip("\n")
            total_json_bytes += len(body.encode())
            try:
                objs = json.loads(body)
            except Exception as exc:  # noqa: BLE001
                print(f"skip unparseable line {n_lines}: {exc}", file=sys.stderr)
                continue
            if isinstance(objs, dict):
                objs = list(objs.values())
            for o in objs:
                n_entities += 1
                etype = o.get("entity_type", "?")
                type_count[etype] += 1
                per = 2  # {}
                for k, v in o.items():
                    b = len(k) + 3 + compact(v)  # "k":v
                    field_bytes[k] += b
                    field_count[k] += 1
                    per += b
                    if k in CONTENTS:
                        inv_present += 1
                        if v:
                            nonempty_inventories += 1
                        else:
                            empty_inventories += 1
                per += max(0, len(o) - 1)  # commas
                envelope_bytes += 2 + max(0, len(o) - 1)
                type_bytes[etype] += per

    print(f"file                       {path}")
    print(f"entities lines             {n_lines}")
    print(f"entity records             {n_entities}")
    print(f"bytes, whole lines         {total_line_bytes:,}")
    print(f"bytes, JSON body only      {total_json_bytes:,}")
    print(f"bytes/entity (JSON body)   {total_json_bytes / max(1, n_entities):.1f}")
    print()

    print("=== bytes by field (compact JSON, key+quotes+colon+value) ===")
    tot = sum(field_bytes.values())
    for k, b in field_bytes.most_common():
        print(f"  {k:22s} {b:12,}  {100*b/tot:5.1f}%   n={field_count[k]:,}")
    print(f"  {'(record envelope)':22s} {envelope_bytes:12,}  {100*envelope_bytes/tot:5.1f}%")
    print()

    print("=== bytes by category ===")
    cats = defaultdict(int)
    for k, b in field_bytes.items():
        if k in IDENTITY:
            cats["identity"] += b
        elif k in GEOMETRY:
            cats["geometry"] += b
        elif k in CONTENTS:
            cats["inventory contents"] += b
        elif k in EXTRA:
            cats["extra (amount/recipe/ghost/ug)"] += b
        else:
            cats[f"UNCLASSIFIED {k}"] += b
    cats["record envelope"] = envelope_bytes
    grand = sum(cats.values())
    for k, b in sorted(cats.items(), key=lambda kv: -kv[1]):
        print(f"  {k:32s} {b:12,}  {100*b/grand:5.1f}%")
    print()

    print("=== inventory fields ===")
    print(f"  present on               {inv_present:,} entity-field slots")
    print(f"  EMPTY ({{}})               {empty_inventories:,}")
    print(f"  non-empty                {nonempty_inventories:,}")
    print()

    print("=== bytes by entity_type (top 25) ===")
    gt = sum(type_bytes.values())
    for t, b in type_bytes.most_common(25):
        print(f"  {t:24s} {b:12,}  {100*b/gt:5.1f}%   n={type_count[t]:,}")
    print()

    scen = sum(b for t, b in type_bytes.items() if t in SCENERY)
    corp = sum(b for t, b in type_bytes.items() if t in CORPSE)
    scen_n = sum(c for t, c in type_count.items() if t in SCENERY)
    corp_n = sum(c for t, c in type_count.items() if t in CORPSE)
    print("=== 'arguably should not be sent' ===")
    print(f"  scenery (tree/rock/cliff/fish/decorative) {scen:12,}  {100*scen/gt:5.1f}%  n={scen_n:,}")
    print(f"  corpses/remnants/items-on-ground          {corp:12,}  {100*corp/gt:5.1f}%  n={corp_n:,}")


if __name__ == "__main__":
    main(sys.argv[1])
