#!/usr/bin/env python3
"""Census of failed walks across every archived run.

Counts by `failure.kind` AND by the wording of `error`, because the two
disagree: CLAUDE.md records that 19 of 20 walk failures were once archived as
kind "other" simply because `classify_walk_failure` did not know the wording
this build emits. The string is what a person reads; the kind is what a query
groups by, and a census must not trust the kind alone.

Also extracts the geometry of every "stuck while walking" failure -- the leg
endpoints named in the message -- so the staircase hypothesis (stalls cluster
on legs that are not axis-aligned) can be tested against the archive rather
than assumed.

Usage: python3 tools/walk_stall_census.py [<repo-root>]
"""
import json
import glob
import re
import sys
import os
import collections

ROOT = sys.argv[1] if len(sys.argv) > 1 else "."

# "from (206.12890625/-189.86328125) to (205.5/-189.5)"
LEG_RE = re.compile(
    r"from \((-?[\d.]+)/(-?[\d.]+)\) to \((-?[\d.]+)/(-?[\d.]+)\)")
LEGN_RE = re.compile(r"leg (\d+) of (\d+)")
BLOCK_RE = re.compile(r"blocked at \((-?[\d.]+),\s*(-?[\d.]+)\) by (\S+) '([^']+)'")
STEER_RE = re.compile(r"steering (\w+) at ([\d.]+) tiles/tick")


def wording(err):
    if not err:
        return "(none)"
    e = err.lower()
    for probe, label in [
        ("stuck while walking", "stuck-while-walking"),
        ("no character", "no-character"),
        ("walled in", "walled-in"),
        ("no path", "no-path"),
        ("timed out", "timed-out"),
        ("timeout", "timed-out"),
        ("cancel", "cancelled"),
    ]:
        if probe in e:
            return label
    return "other:" + err[:60]


def main():
    files = sorted(glob.glob(os.path.join(ROOT, "workspace/*/runs/run-*/events.jsonl")))
    files += sorted(glob.glob(os.path.join(ROOT, "workspace/runs/run-*/events.jsonl")))

    by_kind = collections.Counter()
    by_wording = collections.Counter()
    kind_x_wording = collections.Counter()
    runs_with_stall = set()
    total_walks = 0
    failed_walks = 0
    legs = []

    for f in files:
        run = os.path.basename(os.path.dirname(f))
        for line in open(f):
            try:
                e = json.loads(line)
            except Exception:
                continue
            if e.get("kind") != "walk_settled":
                continue
            total_walks += 1
            status = e.get("status")
            if status in ("success", "ok"):
                continue
            failed_walks += 1
            err = e.get("error")
            fail = e.get("failure")
            k = fail if isinstance(fail, str) else (
                fail.get("kind") if isinstance(fail, dict) else None)
            w = wording(err)
            by_kind[k] += 1
            by_wording[w] += 1
            kind_x_wording[(k, w)] += 1
            if w == "stuck-while-walking":
                runs_with_stall.add(run)
                m = LEG_RE.search(err or "")
                n = LEGN_RE.search(err or "")
                b = BLOCK_RE.search(err or "")
                s = STEER_RE.search(err or "")
                if m:
                    x0, y0, x1, y1 = (float(g) for g in m.groups())
                    legs.append({
                        "run": run,
                        "bot": e.get("bot"),
                        "tick": e.get("tick"),
                        "leg": int(n.group(1)) if n else None,
                        "of": int(n.group(2)) if n else None,
                        "dx": x1 - x0,
                        "dy": y1 - y0,
                        "blocker": b.group(4) if b else None,
                        "blocker_type": b.group(3) if b else None,
                        "steer": s.group(1) if s else None,
                        "error": err,
                    })

    print(f"event files      : {len(files)}")
    print(f"walk_settled     : {total_walks}")
    print(f"failed walks     : {failed_walks}")
    print()
    print("by failure.kind:")
    for k, n in by_kind.most_common():
        print(f"  {str(k):24s} {n}")
    print()
    print("by wording of error:")
    for k, n in by_wording.most_common():
        print(f"  {str(k):40s} {n}")
    print()
    print("kind x wording (the disagreement):")
    for (k, w), n in kind_x_wording.most_common():
        print(f"  {str(k):20s} {w:40s} {n}")
    print()
    print(f"runs containing a stuck-while-walking : {len(runs_with_stall)}")

    if legs:
        print()
        print("stall leg geometry (does the stall cluster on diagonals?):")
        axis = collections.Counter()
        for L in legs:
            dx, dy = L["dx"], L["dy"]
            if abs(dx) < 1e-9 and abs(dy) < 1e-9:
                cls = "zero"
            elif abs(dx) < 1e-9 or abs(dy) < 1e-9:
                cls = "axis-aligned"
            else:
                cls = "diagonal"
            axis[cls] += 1
            L["class"] = cls
        for k, n in axis.most_common():
            print(f"  {k:16s} {n}")
        print()
        print("  run/bot/tick   leg     dx      dy     class         blocker  steer")
        for L in legs:
            print(f"  {L['run'][-8:]}/{L['bot']}/{L['tick']:<7} "
                  f"{L['leg']}/{L['of']:<5} {L['dx']:+7.3f} {L['dy']:+7.3f} "
                  f"{L['class']:14s} {str(L['blocker']):10s} {L['steer']}")
        print()
        print("blockers:")
        for k, n in collections.Counter(L["blocker"] for L in legs).most_common():
            print(f"  {str(k):20s} {n}")


if __name__ == "__main__":
    main()
