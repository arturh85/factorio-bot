#!/usr/bin/env python3
"""``exposure.json``: three states, and neither of the collapses.

    python3 tools/test_run_analysis_exposure.py
    python3 -m pytest tools/                       # if pytest is installed

A run can be **held** -- paused mid-run with an invitation for a person to
attach and, in the owner's words, "cheat some missing items in". Releasing with
``continue`` replans against the live world, so anything inserted during the
hold is simply present on the retry. Until ``exposure.json`` existed such a run
was byte-identical in the record to one that ran clean.

Being held is an *opportunity* to mutate, not proof of one, and this file
exists to hold three answers apart that a careless reader would make two:

* no ``exposure.json``           -> UNKNOWN (a build older than the feature)
* ``holds: []``                  -> not held
* ``holds: [...]``               -> held; mutation separately reported, and
                                    possibly unknown

The peer file ``test_run_analysis_peaceful.py`` makes the same argument for
peaceful mode; the shape is the project's ``absent is not a value`` rule.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import run_analysis as ra  # noqa: E402


def provenance() -> dict:
    """A run identical to its peer in every field ``--compare`` refuses on."""
    return {
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
        "peaceful": False,
    }


def hold(foreign: int | None, tick: int = 429, released: str = "continue") -> dict:
    h = {"paused_at_tick": tick, "held_seconds": 60, "released": released}
    if foreign is not None:
        h["console_commands_observed"] = 20 + foreign
        h["console_commands_ours"] = 20
        h["foreign_console_commands"] = foreign
        h["console_command_used"] = True
    return h


def read(exposure: dict | None) -> dict:
    """A run directory with provenance and, optionally, an exposure file."""
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, ra.PROVENANCE_FILE), "w") as f:
            json.dump(provenance(), f)
        if exposure is not None:
            with open(os.path.join(d, ra.EXPOSURE_FILE), "w") as f:
                json.dump(exposure, f)
        return ra.read_provenance(d, None)


def held_value(exposure: dict | None) -> str | None:
    return (read(exposure).get("held") or {}).get("value")


def guards_for(a: dict | None, b: dict | None) -> list[dict]:
    return ra.comparability(
        {"run_id": "A", "provenance": read(a)},
        {"run_id": "B", "provenance": read(b)},
    )


def refusals(guards: list[dict]) -> set[str]:
    return {g["id"] for g in guards if g["severity"] == "refuse"}


class ThreeStates(unittest.TestCase):
    def test_no_file_is_unknown_and_not_not_held(self):
        """The whole archive is this case. Reading it as 'not held' would
        declare every run before the feature clean on no evidence."""
        self.assertIsNone(held_value(None))

    def test_an_empty_holds_list_is_the_positive_answer(self):
        """Which is why the recorder writes the file at run START, empty,
        rather than only when a hold happens."""
        self.assertEqual(held_value({"schema": 1, "holds": []}), ra.NOT_HELD)

    def test_a_hold_says_held_and_names_where(self):
        value = held_value({"schema": 1, "holds": [hold(0)]})
        self.assertIsNotNone(value)
        self.assertTrue(value.startswith("HELD x1"))
        self.assertIn("tick 429", value)
        self.assertIn("no foreign console command observed", value)

    def test_an_unreadable_census_is_unknown_not_zero(self):
        value = held_value({"schema": 1, "holds": [hold(None)]})
        self.assertIn("UNKNOWN", value)
        self.assertNotIn("no foreign console command observed", value)

    def test_a_foreign_command_is_reported_as_one(self):
        value = held_value({"schema": 1, "holds": [hold(3)]})
        self.assertIn("3 FOREIGN console command(s) observed", value)

    def test_one_unreadable_hold_poisons_the_total(self):
        """A clean reading beside an unreadable one has not shown the run
        clean. Mirrors `Exposure::foreign_console_commands` in Rust."""
        value = held_value({"schema": 1, "holds": [hold(0), hold(None, tick=800)]})
        self.assertIn("UNKNOWN", value)


class Comparability(unittest.TestCase):
    def test_the_field_is_refusal_grade(self):
        """Stated once, in the table `comparability` reads, so the guard and
        this test cannot disagree about the severity."""
        self.assertEqual(ra.PROVENANCE_SEVERITY["held"], "refuse")
        self.assertIn("held", ra.PROVENANCE_FIELDS)

    def test_a_held_run_against_a_fresh_one_is_refused(self):
        """The precedent is `resumed_from`, and the reason is the same: the
        two are not measuring the same thing."""
        guards = guards_for({"schema": 1, "holds": [hold(0)]}, {"schema": 1, "holds": []})
        self.assertIn("provenance", refusals(guards))

    def test_two_unheld_runs_are_not_refused_on_this(self):
        guards = guards_for({"schema": 1, "holds": []}, {"schema": 1, "holds": []})
        self.assertNotIn("provenance", refusals(guards))
        self.assertNotIn("exposure", refusals(guards))

    def test_two_runs_with_no_exposure_file_are_unknown_never_a_match(self):
        """Absence on both sides is two unknowns, not agreement -- and it must
        not become a refusal either, or the whole archive is unusable."""
        guards = guards_for(None, None)
        self.assertNotIn("exposure", refusals(guards))
        self.assertIsNone(held_value(None))

    def test_a_run_held_with_an_unknown_census_is_refused_even_against_another(self):
        """The standalone guard, and the point of it: two runs each with an
        unmeasured human in them are no more comparable to each other than to
        a clean one."""
        both = {"schema": 1, "holds": [hold(None)]}
        guards = guards_for(both, both)
        self.assertIn("exposure", refusals(guards))

    def test_a_run_with_a_foreign_command_is_refused_even_against_another(self):
        both = {"schema": 1, "holds": [hold(2)]}
        self.assertIn("exposure", refusals(guards_for(both, both)))

    def test_a_clean_census_does_not_trip_the_standalone_guard(self):
        """A deliberate limit, not an oversight. The census sees console
        commands, so a zero is evidence and not a proof -- the provenance
        field still refuses it against a run that was never held."""
        both = {"schema": 1, "holds": [hold(0)]}
        self.assertNotIn("exposure", refusals(guards_for(both, both)))

    def test_the_refusal_says_unknown_is_not_nothing(self):
        both = {"schema": 1, "holds": [hold(None)]}
        guard = next(
            g for g in guards_for(both, both) if g["id"] == "exposure"
        )
        text = " ".join(guard["detail"])
        self.assertIn("NOT that", text)
        self.assertIn("graphical client", text)


if __name__ == "__main__":
    unittest.main()
