#!/usr/bin/env python3
"""``--compare`` and peaceful mode: a refusal, and the absence that is not one.

    python3 tools/test_run_analysis_peaceful.py
    python3 -m pytest tools/                       # if pytest is installed

Two claims, and the second is the one worth the file.

1. A peaceful run and a hostile run are **refused**, the way a resumed run and
   a fresh one are. They differ in whether anything hunted the bots, and every
   other provenance field can agree exactly.

2. A run recorded before the field existed reads as **unknown**, and unknown
   against a value is not a refusal and not a match. Every one of the archived
   runs is that case, so a rule that read absence as "hostile" would refuse or
   silently mislabel the entire archive on the strength of a missing key.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import run_analysis as ra  # noqa: E402


def provenance(**overrides) -> dict:
    """Two runs identical in every field `--compare` refuses on."""
    base = {
        "schema": 1,
        "run_id": "run-1-2",
        "started_unix": 1_788_000_000,
        "started_tick": 0,
        "seed": "31337",
        "map_exchange_string": None,
        "map": None,
        "factorio": "2.1.17",
        "mods": {"base": "2.1.17"},
        "git": {"commit": "a" * 40, "dirty": False, "source": "working-tree-at-run-start"},
        "profile": "debug",
        "roster_requested": [1, 2, 3, 4],
        "workspace": "/ws",
        "resumed_from": None,
        "bot_mode": "characters",
        "game_speed": 10.0,
    }
    base.update(overrides)
    return base


def read(prov: dict) -> dict:
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, ra.PROVENANCE_FILE), "w") as f:
            json.dump(prov, f)
        return ra.read_provenance(d, None)


def guards_for(a: dict, b: dict) -> list[dict]:
    return ra.comparability(
        {"run_id": "A", "provenance": read(a)},
        {"run_id": "B", "provenance": read(b)},
    )


class PeacefulComparability(unittest.TestCase):
    def test_the_field_is_refusal_grade(self):
        """Stated once, in the table `comparability` reads, so the guard and
        this test cannot disagree about the severity."""
        self.assertEqual(ra.PROVENANCE_SEVERITY["peaceful"], "refuse")
        self.assertIn("peaceful", ra.PROVENANCE_FIELDS)

    def test_peaceful_against_hostile_is_refused(self):
        guards = guards_for(
            provenance(peaceful=True),
            provenance(peaceful=False),
        )
        self.assertTrue(
            any(g["severity"] == "refuse" for g in guards),
            f"expected a refusal, got {[(g['id'], g['severity']) for g in guards]}",
        )
        detail = "\n".join(
            line for g in guards for line in (g.get("detail") or [])
        )
        self.assertIn("peaceful", detail)

    def test_two_peaceful_runs_are_not_refused_on_this_field(self):
        """The control. Without it, a refusal on every pair would pass the test
        above for the wrong reason."""
        guards = guards_for(provenance(peaceful=True), provenance(peaceful=True))
        self.assertFalse(
            any(g["severity"] == "refuse" for g in guards),
            f"unexpected refusal: {[(g['id'], g['severity']) for g in guards]}",
        )

    def test_an_archived_run_reads_as_unknown_not_as_hostile(self):
        """The whole point of the field being `Option`.

        A run with no `peaceful` key at all is every run in the archive. It
        must not compare equal to a hostile run (that would assert something
        nobody established) and it must not compare *unequal* to one either
        (that would refuse the entire archive). Unknown is its own answer.
        """
        absent = read(provenance())
        self.assertIsNone(absent["peaceful"]["value"])

        hostile = read(provenance(peaceful=False))
        self.assertEqual(hostile["peaceful"]["value"], "hostile")
        self.assertNotEqual(absent["peaceful"]["value"], hostile["peaceful"]["value"])

        # And the comparison of the two is not a refusal: unknown withholds
        # judgement rather than manufacturing one.
        guards = guards_for(provenance(), provenance(peaceful=False))
        self.assertFalse(
            any(g["severity"] == "refuse" for g in guards),
            f"unknown must not refuse: {[(g['id'], g['severity']) for g in guards]}",
        )

    def test_the_word_printed_does_not_claim_biters_are_absent(self):
        """Peaceful mode leaves every nest, worm and unit standing; it stops
        them attacking unprovoked. A reader who takes `peaceful` as "no
        enemies" is making a claim the run does not support, so the rendered
        value says so."""
        value = read(provenance(peaceful=True))["peaceful"]["value"]
        self.assertIn("biters present", value)

    def test_a_value_that_is_not_a_boolean_is_unknown(self):
        """A hand-edited or future record must not be coerced into a verdict."""
        for junk in ["true", 1, "peaceful", {}, []]:
            self.assertIsNone(
                read(provenance(peaceful=junk))["peaceful"]["value"],
                f"{junk!r} should read as unknown",
            )


if __name__ == "__main__":
    unittest.main(verbosity=2)
