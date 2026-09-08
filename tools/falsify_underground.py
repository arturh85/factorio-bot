#!/usr/bin/env python3
"""Falsification sweep for the underground-belt routing (2026-09-09).

One mutation at a time, each asserted to match EXACTLY ONCE in the file it
edits; the two crates' own tests are run and the mutation reverted through
`falsify_sweep`'s guarded runner (control first, dirty files skipped, restore
verified and touched).

Every mutation below is a way the route could be wrong that would place
perfectly and move nothing, or refuse what the game allows. Each is expected
RED. A GREEN line is a finding: the suite could not tell the mutated tree
from the real one.

Usage: nix develop -c python3 tools/falsify_underground.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from falsify_sweep import main  # noqa: E402  (after sys.path)

ROUTE = "crates/core/src/graph/route.rs"
CONNECT = "crates/planner/src/method/connect.rs"
SUSTAIN = "crates/planner/src/method/sustain.rs"

MUTATIONS = [
    ("route: launch a jump from a turning tile (side-fed entry)", ROUTE,
     "            if !node.surfaced\n                && dir == node.facing\n                && let Some(max) = max_underground",
     "            if !node.surfaced\n                && let Some(max) = max_underground"),
    ("route: allow a turn on the tile after an exit", ROUTE,
     "                && (!node.surfaced || dir == node.facing)\n",
     "\n"),
    ("route: count hidden tiles under the prototype's name (off by one)", ROUTE,
     "                for span in 2..=(max as i64) {",
     "                for span in 2..=(max as i64 + 1) {"),
    ("route: ignore an existing same-axis tunnel beneath the span", ROUTE,
     "                    if under.iter().any(|c| tunnels[cell_index(c.0, c.1)] & axis != 0) {\n                        break;\n                    }\n",
     "\n"),
    ("route: ignore an existing same-axis tunnel at the exit", ROUTE,
     "                    if tunnels[cell_index(exit.0, exit.1)] & axis != 0 {",
     "                    if false {"),
    ("route: price a pair like walking (tunnel under a tree)", ROUTE,
     "const UNDERGROUND_PENALTY: u32 = 16 * STEP;",
     "const UNDERGROUND_PENALTY: u32 = 0;"),
    ("route: report the wall's width instead of the pair it needs", ROUTE,
     "        let needed = widest_blocked_run(blocked, from, to) + 1;",
     "        let needed = widest_blocked_run(blocked, from, to);"),
    ("connect: transpose the two halves", CONNECT,
     "        TileKind::UndergroundEntry => Some(UndergroundHalf::Input),\n        TileKind::UndergroundExit => Some(UndergroundHalf::Output),",
     "        TileKind::UndergroundEntry => Some(UndergroundHalf::Output),\n        TileKind::UndergroundExit => Some(UndergroundHalf::Input),"),
    ("connect: reach from a constant, not the prototype", CONNECT,
     "        .and_then(|proto| proto.max_underground_distance)\n}",
     "        .and_then(|proto| proto.max_underground_distance.map(|_| 4))\n}"),
    ("connect: tunnel first, surface second", CONNECT,
     "        .and_then(|grid| search(grid, None).ok())\n        .or_else(|| search(&blocked, None).ok())",
     "        .and_then(|grid| search(grid, reach).ok())\n        .or_else(|| search(&blocked, reach).ok())"),
    ("connect: bill the pair as belts", CONNECT,
     "    if undergrounds > 0 {\n        steps.push(Step::Subgoal(Goal::Have {\n            item: UNDERGROUND.into(),",
     "    if undergrounds > 0 {\n        steps.push(Step::Subgoal(Goal::Have {\n            item: BELT.into(),"),
    ("connect: let a route hug the chest it serves", CONNECT,
     "        if ours && !chosen {\n            blocked[enclosure::cell_index(cell.0, cell.1)] = true;\n        }",
     "        let _ = (ours, chosen, cell);"),
    ("connect: do not reserve the ground beneath an existing pair", CONNECT,
     "            for cell in reserved {\n                blocked[enclosure::cell_index(cell.0, cell.1)] = true;\n            }\n            tunnels",
     "            let _ = reserved;\n            tunnels"),
    ("sustain: a chest with any side is a chest with enough", SUSTAIN,
     "                        && free_sides(&ctx.state, &e.position) >= runs_needed",
     "                        && free_sides(&ctx.state, &e.position) >= runs_needed.min(1)"),
    ("sustain: always haul from the source", SUSTAIN,
     "                let origin = hauls_from.into_iter().next().unwrap_or_else(|| buffer.clone());",
     "                let origin = buffer.clone();\n                let _ = hauls_from;"),
    ("sustain: never reverse the feed order", SUSTAIN,
     "            let order = if drill_first_fits {",
     "            let order = if drill_first_fits || true {"),
]

sys.exit(main(
    MUTATIONS,
    "cargo test -p factorio-bot-core --test suite route_grid && cargo test -p factorio-bot-planner --lib",
))
