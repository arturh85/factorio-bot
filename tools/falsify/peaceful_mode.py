#!/usr/bin/env python3
"""Falsify every test added by the peaceful-mode change, one mutation at a time.

    python3 tools/falsify/peaceful_mode.py      # 10 mutations, all must go RED

Each entry breaks exactly one thing the implementation promises and names the
test that must go red. The substitution is asserted to match **exactly once**:
a mutation that matched zero times, or twice, is not the experiment it claims.

Backup is by file copy and restore is by copy plus `touch` -- `cp -p` preserves
mtime, so cargo would re-run the *mutated* binary against restored source and
report a false red (CLAUDE.md, 2026-09-07).
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys

# <repo>/tools/falsify/<this file>
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RCON = "crates/core/src/factorio/rcon.rs"
PROV = "crates/core/src/record/provenance.rs"
ANAL = "tools/run_analysis.py"

RUST = "rust"
PY = "py"

MUTATIONS = [
    # --- rcon.rs: the parser -------------------------------------------
    (
        "an unparseable reply defaults to hostile instead of unknown",
        RCON, RUST,
        "        // `none`, and anything else the game might print: unknown. Never a\n"
        "        // default, and specifically never `false`.\n"
        "        _ => None,",
        "        _ => Some(false),",
        "an_unanswered_query_is_none_and_never_false",
    ),
    (
        "`false` is read as `true`",
        RCON, RUST,
        '        "false" => Some(false),\n        // `none`, and anything else',
        '        "false" => Some(true),\n        // `none`, and anything else',
        "a_stamped_boolean_is_read_as_that_boolean",
    ),
    (
        "the reply is not trimmed, so a padded stamp is unreadable",
        RCON, RUST,
        "?\n        .trim();",
        "?;",
        "the_stamp_is_found_on_a_later_line",
    ),
    # --- rcon.rs: the setter -------------------------------------------
    (
        "the setter echoes the request instead of reading the game back",
        RCON, RUST,
        '         local all=true local any=false for _,s in pairs(game.surfaces) do any=true \\\n'
        '         if not s.peaceful_mode then all=false end end \\\n'
        '         rcon.print(\\"{PEACEFUL_STAMP_PREFIX}\\"..(any and tostring(all) or \\"none\\"))"',
        '         rcon.print(\\"{PEACEFUL_STAMP_PREFIX}{peaceful}\\")"',
        "the_setter_writes_the_request_and_then_reads_the_game_back",
    ),
    (
        "the query and the setter answer on two different stamps",
        RCON, RUST,
        'rcon.print(\\"§peaceful§\\"..(any and tostring(all) or \\"none\\"))";',
        'rcon.print(\\"§peace§\\"..(any and tostring(all) or \\"none\\"))";',
        "the_setter_and_the_query_share_one_stamp",
    ),
    # --- provenance.rs -------------------------------------------------
    (
        "an absent `peaceful` key deserialises as hostile",
        PROV, RUST,
        "    #[serde(default)]\n    pub peaceful: Option<bool>,",
        '    #[serde(default = "hostile")]\n    pub peaceful: Option<bool>,\n}\n'
        "fn hostile() -> Option<bool> {\n    Some(false)",
        "a_run_recorded_before_peaceful_existed_reads_as_unknown_not_as_hostile",
    ),
    (
        "unknown is serialised as hostile, so the two are one value on disk",
        PROV, RUST,
        "    #[serde(default)]\n    pub peaceful: Option<bool>,",
        "    #[serde(default, serialize_with = \"as_hostile\")]\n"
        "    pub peaceful: Option<bool>,\n}\n"
        "fn as_hostile<S: serde::Serializer>(_: &Option<bool>, s: S) -> Result<S::Ok, S::Error> {\n"
        "    s.serialize_bool(false)",
        "hostile_and_unknown_are_different_values_on_the_wire",
    ),
    # --- run_analysis.py -----------------------------------------------
    (
        "a peaceful/hostile difference is a flag rather than a refusal",
        ANAL, PY,
        '    "peaceful": "refuse",',
        '    "peaceful": "flag",',
        "test_peaceful_against_hostile_is_refused",
    ),
    (
        "an unrecorded run reads as hostile",
        ANAL, PY,
        '    if raw is False:\n        return "hostile"\n    return None',
        '    return "hostile"',
        "test_an_archived_run_reads_as_unknown_not_as_hostile",
    ),
    (
        "the rendered word claims there are no biters",
        ANAL, PY,
        '        return "peaceful (biters present, not hunting)"',
        '        return "peaceful"',
        "test_the_word_printed_does_not_claim_biters_are_absent",
    ),
]


def run(cmd: list[str]) -> tuple[int, str]:
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def verify(kind: str, test: str) -> tuple[int, str]:
    if kind == RUST:
        return run(["nix", "develop", "-c", "cargo", "test", "-p",
                    "factorio-bot-core", "--lib", test])
    return run([sys.executable, "tools/test_run_analysis_peaceful.py", test])


def main() -> int:
    bad = 0
    for name, path, kind, old, new, test in MUTATIONS:
        full = os.path.join(ROOT, path)
        backup = full + ".falsify-backup"
        shutil.copy(full, backup)
        try:
            src = open(full).read()
            hits = src.count(old)
            if hits != 1:
                print(f"SKIP  {name}: substitution matched {hits} times, not 1")
                bad += 1
                continue
            open(full, "w").write(src.replace(old, new, 1))
            code, out = verify(kind, test)
            if code == 0:
                print(f"GREEN {name}: `{test}` PASSED under the mutation -- a finding")
                bad += 1
            elif "error[E" in out or "error: could not compile" in out:
                print(f"BUILD {name}: mutation did not compile, so the test proved nothing")
                bad += 1
            else:
                print(f"RED   {name}: `{test}` failed as it must")
        finally:
            shutil.copy(backup, full)
            os.utime(full, None)  # never cp -p: cargo must see it as newer
            os.remove(backup)
    return bad


if __name__ == "__main__":
    sys.exit(main())
