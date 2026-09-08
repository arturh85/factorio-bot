#!/usr/bin/env python3
"""Tests for the machines-at-fixed-marks block of ``run_analysis.py``.

Runs under ``python3 -m pytest tools/`` when pytest is installed and under
``python3 tools/test_run_analysis_census.py`` when it is not. Stdlib only,
like the tool.

The three paths no archived run exercises are the ones that matter here, and
each is the same defect class -- *absent is not a value*:

* a mark with no machine sample must read UNKNOWN, never zero;
* a row with no ``status`` (every run archived before 2026-09-07) must read
  unknown, never "not working";
* a ``truncated`` sample makes its counts a FLOOR, not a count.
"""

from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_analysis as ra  # noqa: E402

TPM = ra.TICKS_PER_MINUTE
ORIGIN = 3600
END = ORIGIN + 18 * TPM


def machine(name: str, mtype: str, x: float, status: str | None) -> dict:
    row = {"name": name, "type": mtype, "position": {"x": x, "y": 0.5}}
    if status is not None:
        row["status"] = status
    return row


def sample(tick: int, machines: dict, truncated: int | None = 0) -> dict:
    s = {"schema": 2, "tick": tick, "kind": "machines", "machines": machines}
    if truncated is not None:
        s["truncated"] = truncated
    return s


def synthetic(status: str | None = "working", truncated: int | None = 0) -> list[dict]:
    """Two furnaces from the start, a third and a drill from minute 8."""
    out = []
    for tick in range(ORIGIN, END + 1, 300):
        minute = (tick - ORIGIN) / TPM
        rows = {
            "1": machine("stone-furnace", "furnace", 0.5, status),
            "2": machine("stone-furnace", "furnace", 3.5, "no_fuel" if status else None),
        }
        if minute >= 8:
            rows["3"] = machine("stone-furnace", "furnace", 6.5, status)
            rows["4"] = machine("burner-mining-drill", "mining-drill", 9.5, status)
        out.append(sample(tick, rows, truncated))
    return out


class CensusTest(unittest.TestCase):
    def setUp(self):
        self.r = ra.machine_census(synthetic(), ORIGIN, END, marks=(5, 10, 15, 20))

    def test_counts_and_working_are_separate_facts(self):
        at5, at10 = self.r["marks"][0], self.r["marks"][1]
        self.assertEqual(at5["kinds"]["furnace"]["count"], 2)
        self.assertEqual(at5["kinds"]["furnace"]["working"], 1)
        self.assertNotIn("mining-drill", at5["kinds"])
        self.assertEqual(at10["kinds"]["furnace"]["count"], 3)
        self.assertEqual(at10["kinds"]["furnace"]["working"], 2)
        self.assertEqual(at10["kinds"]["mining-drill"]["count"], 1)
        self.assertEqual(at10["total"], 4)
        self.assertEqual(at10["total_working"], 3)

    def test_kinds_are_ordered_producers_first(self):
        self.assertEqual(self.r["kinds"][:2], ["furnace", "mining-drill"])

    def test_a_mark_past_the_run_is_unknown_not_zero(self):
        past = self.r["marks"][3]
        self.assertEqual(past["minute"], 20)
        self.assertEqual(past["status"], "run_ended")
        self.assertIsNone(past["total"])
        self.assertEqual(past["kinds"], {})

    def test_a_mark_before_the_first_sample_is_no_sample(self):
        # Samples start two minutes into the window this census covers.
        late = [s for s in synthetic() if s["tick"] >= ORIGIN + 2 * TPM]
        r = ra.machine_census(late, ORIGIN, END, marks=(1, 5))
        self.assertEqual(r["marks"][0]["status"], "no_sample")
        self.assertIsNone(r["marks"][0]["total"])
        self.assertEqual(r["marks"][1]["status"], "ok")

    def test_a_run_predating_entity_status_reads_unknown_not_idle(self):
        r = ra.machine_census(synthetic(status=None), ORIGIN, END, marks=(10,))
        k = r["marks"][0]["kinds"]["furnace"]
        self.assertEqual(k["count"], 3)
        self.assertEqual(k["working"], 0)
        self.assertEqual(k["status_unknown"], 3)
        out: list[str] = []
        ra.report_machine_census(r, out.append)
        # `?`, not `0`: the sample cannot say, and the table must not either.
        self.assertTrue(any("/    ?" in line for line in out), out)

    def test_a_truncated_sample_is_a_floor(self):
        r = ra.machine_census(synthetic(truncated=17), ORIGIN, END, marks=(10,))
        m = r["marks"][0]
        self.assertEqual(m["truncated"], 17)
        self.assertTrue(m["is_floor"])
        out: list[str] = []
        ra.report_machine_census(r, out.append)
        self.assertTrue(any("FLOOR, not a count" in line for line in out), out)

    def test_a_sample_with_no_truncated_field_is_unknown_not_complete(self):
        r = ra.machine_census(synthetic(truncated=None), ORIGIN, END, marks=(10,))
        self.assertIsNone(r["marks"][0]["truncated"])
        self.assertIsNone(r["marks"][0]["is_floor"])
        out: list[str] = []
        ra.report_machine_census(r, out.append)
        self.assertTrue(any("no `truncated` field" in line for line in out), out)

    def test_no_machine_rows_is_reported_as_absence(self):
        r = ra.machine_census([], ORIGIN, END, marks=(5,))
        self.assertFalse(r["present"])
        self.assertIn("no machines rows", r["reason"])
        r2 = ra.machine_census([], ORIGIN, END, marks=(5,), present=False)
        self.assertIn("no samples.jsonl", r2["reason"])


if __name__ == "__main__":
    unittest.main()
