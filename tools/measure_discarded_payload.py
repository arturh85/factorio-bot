#!/usr/bin/env python3
"""How much of a chunk's `entities` payload does the INGEST throw away?

Applies `EntityGraph::add`'s own drop gate (crates/core/src/graph/entity_graph.rs,
the `continue` at the top of the loop) to the real wire bytes:

    flying-text, fish, or a zero-width bounding_box  ->  dropped, always

Everything else is at least a rectangle in `blocked_tree`. This is the only
"should not be sent" category that can be stated as a fact rather than a guess.
"""

import json
import sys
from collections import Counter


def compact(v):
    return len(json.dumps(v, separators=(",", ":")))


def record_bytes(o):
    n = 2 + max(0, len(o) - 1)
    for k, v in o.items():
        n += len(k) + 3 + compact(v)
    return n


def width(o):
    bb = o.get("bounding_box")
    if not bb:
        return None
    return bb["right_bottom"]["x"] - bb["left_top"]["x"]


def main(path):
    total = 0
    n = 0
    dropped_bytes = Counter()
    dropped_n = Counter()
    inv_bytes = 0
    inv_n = 0

    with open(path, "rb") as fh:
        for raw in fh:
            if b"\xc2\xa7entities\xc2\xa7" not in raw:
                continue
            line = raw.decode("utf-8", "replace")
            _, _, rest = line.partition("§entities§")
            body = rest[rest.index(":") + 1:].rstrip("\n")
            try:
                objs = json.loads(body)
            except Exception:
                continue
            if isinstance(objs, dict):
                objs = list(objs.values())
            for o in objs:
                n += 1
                b = record_bytes(o)
                total += b
                t = o.get("entity_type")
                w = width(o)
                if t == "flying-text":
                    dropped_bytes["flying-text"] += b
                    dropped_n["flying-text"] += 1
                elif t == "fish":
                    dropped_bytes["fish"] += b
                    dropped_n["fish"] += 1
                elif w == 0.0:
                    dropped_bytes[f"zero-width box ({t})"] += b
                    dropped_n[f"zero-width box ({t})"] += 1
                for k in ("output_inventory", "fuel_inventory"):
                    if k in o:
                        inv_bytes += len(k) + 3 + compact(o[k]) + 1
                        inv_n += 1

    print(f"entity records                {n:,}")
    print(f"bytes (compact, records only) {total:,}")
    print()
    print("=== the ingest's own drop gate, applied to the wire ===")
    d = sum(dropped_bytes.values())
    for k, b in dropped_bytes.most_common():
        print(f"  {k:34s} {b:10,}  {100*b/total:5.2f}%  n={dropped_n[k]:,}")
    print(f"  {'TOTAL discarded on arrival':34s} {d:10,}  {100*d/total:5.2f}%  n={sum(dropped_n.values()):,}")
    print()
    print("=== inventory contents on the wire ===")
    print(f"  bytes {inv_bytes:,}  ({100*inv_bytes/total:.2f}%)  fields present on {inv_n:,} records")


if __name__ == "__main__":
    main(sys.argv[1])
