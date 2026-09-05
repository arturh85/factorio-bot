#!/usr/bin/env python3
"""Tests for the production-at-fixed-marks block of ``run_analysis.py``.

Runs under ``python3 -m pytest tools/`` when pytest is installed and under
``python3 tools/test_run_analysis_rates.py`` when it is not. Stdlib only, like
the tool.

The synthetic run below is a fresh world (origin tick 3600) with force samples
on the real 300-tick beat: iron grows 10 per beat for 12 minutes and then
stops, copper never starts, red packs begin at minute 8. The run ends at
minute 18, so mark 20 is past its end.
"""

from __future__ import annotations

import io
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_analysis as ra  # noqa: E402

TPM = ra.TICKS_PER_MINUTE
ORIGIN = 3600
END = ORIGIN + 18 * TPM


def force_sample(tick: int, made: dict | list) -> dict:
    return {
        "schema": 2,
        "tick": tick,
        "kind": "force",
        "research": None,
        "techs_unlocked": 0,
        "production": {"made": made, "consumed": []},
        "power": {"generated_kw": 0.0, "consumed_kw": 0.0, "satisfaction": 1.0, "networks": []},
    }


def synthetic_samples() -> list[dict]:
    out = []
    for tick in range(ORIGIN, END + 1, 300):
        minute = (tick - ORIGIN) / TPM
        iron = int(min(minute, 12) * 12)  # 10 per beat = 120/min, flat after 12:00
        red = int(max(0.0, minute - 8) * 3)  # 3/min from minute 8
        made: dict | list = {}
        if iron:
            made = {"iron-plate": iron, "iron-ore": iron}
        if red:
            made["automation-science-pack"] = red
        out.append(force_sample(tick, made if made else []))
    return out


class ProductionRatesTest(unittest.TestCase):
    def setUp(self):
        self.r = ra.production_rates(synthetic_samples(), ORIGIN, END, marks=(5, 10, 15, 20))

    def mark(self, minute):
        return next(m for m in self.r["marks"] if m["minute"] == minute)

    def test_cumulative_and_interval_rate_at_marks(self):
        m5 = self.mark(5)["items"]["iron-plate"]
        self.assertEqual(m5["cumulative"], 60)
        self.assertAlmostEqual(m5["rate_interval"], 12.0)
        self.assertAlmostEqual(m5["rate_window"], 12.0)
        m10 = self.mark(10)["items"]["iron-plate"]
        self.assertEqual(m10["cumulative"], 120)
        self.assertAlmostEqual(m10["rate_interval"], 12.0)
        # Minute 15: iron stopped at 12:00, so the interval average is 24/5
        # and the trailing 2-minute window is zero -- the window sees the stop.
        m15 = self.mark(15)["items"]["iron-plate"]
        self.assertEqual(m15["cumulative"], 144)
        self.assertAlmostEqual(m15["rate_interval"], 24 / 5)
        self.assertAlmostEqual(m15["rate_window"], 0.0)

    def test_item_never_made_is_zero_not_missing(self):
        self.assertEqual(self.mark(10)["items"]["copper-plate"]["cumulative"], 0)
        self.assertEqual(self.mark(10)["items"]["logistic-science-pack"]["cumulative"], 0)

    def test_threshold_item_joins_the_table(self):
        self.assertIn("iron-ore", self.r["items"])
        self.assertNotIn("stone-furnace", self.r["items"])

    def test_mark_past_the_end_is_named_not_zeroed(self):
        m20 = self.mark(20)
        self.assertEqual(m20["status"], "run_ended")
        self.assertEqual(m20["items"], {})
        self.assertIn("run ended 18:00 before mark 20", self.r["headline"])

    def test_plateau_detected_with_time_before_end(self):
        pl = self.r["plateaus"]["iron-plate"]
        self.assertIsNotNone(pl)
        self.assertEqual(pl["count"], 144)
        self.assertAlmostEqual(pl["at_minute"], 12.0)
        self.assertAlmostEqual(pl["idle_minutes"], 6.0)
        lines = ra.plateau_lines(self.r)
        self.assertIn("iron-plate plateaus at 12:00 (144) -- production stopped 6.0 min before the run ended", lines)
        # Still growing at the end: not a plateau. Never made: not a plateau.
        self.assertIsNone(self.r["plateaus"]["automation-science-pack"])
        self.assertIsNone(self.r["plateaus"]["copper-plate"])

    def test_headline_shape(self):
        self.assertTrue(self.r["headline"].startswith("rates: iron 12->12->5 /min at 5/10/15; red packs 0->1->3; green 0 at 15, 0 at end 18:00"),
                        self.r["headline"])

    def test_no_force_samples_is_absence_not_zero(self):
        bots_only = [{"kind": "bots", "tick": ORIGIN + 60, "bots": []}]
        r = ra.production_rates(bots_only, ORIGIN, END)
        self.assertFalse(r["present"])
        self.assertIn("no production samples", r["reason"])
        self.assertEqual(r["marks"], [])
        self.assertIn("no production samples", ra.rates_headline(r))
        r2 = ra.production_rates([], ORIGIN, END, present=False)
        self.assertFalse(r2["present"])

    def test_samples_short_of_the_run_end(self):
        short = [s for s in synthetic_samples() if s["tick"] <= ORIGIN + 12 * TPM]
        r = ra.production_rates(short, ORIGIN, END, marks=(10, 15))
        self.assertEqual(next(m for m in r["marks"] if m["minute"] == 15)["status"], "samples_end")
        # No plateau can be claimed from the part of the run the samples never saw.
        self.assertIsNone(r["plateaus"]["iron-plate"])

    def test_resumed_run_counts_relative_to_origin_sample(self):
        samples = [force_sample(ORIGIN, {"iron-plate": 500}), force_sample(ORIGIN + 5 * TPM, {"iron-plate": 560})]
        r = ra.production_rates(samples, ORIGIN, ORIGIN + 6 * TPM, marks=(5,))
        self.assertEqual(r["marks"][0]["items"]["iron-plate"]["cumulative"], 60)
        self.assertIn("NON-ZERO", r["baseline"])

    def test_end_column_when_the_run_ends_between_marks(self):
        end = next(m for m in self.r["marks"] if m.get("is_end"))
        self.assertEqual(end["label"], "end 18:00")
        self.assertEqual(end["items"]["automation-science-pack"]["cumulative"], 30)
        # Its interval rate runs from the last reached configured mark (15:00).
        self.assertAlmostEqual(end["items"]["automation-science-pack"]["rate_interval"], 3.0)
        # A run ending on a mark gets no extra column.
        r = ra.production_rates(synthetic_samples(), ORIGIN, END, marks=(18,))
        self.assertFalse(any(m.get("is_end") for m in r["marks"]))

    def test_markdown_table(self):
        a = {"run_id": "run-A", "rates": self.r}
        md = ra.rates_markdown([a], ("iron-plate", "automation-science-pack"))
        lines = md.splitlines()
        self.assertEqual(lines[0], "| minute | run-A iron / red packs |")
        self.assertIn("| 10 | 120 / 6 |", lines)
        self.assertIn("| 20 | run ended at 18:00 |", lines)
        self.assertEqual(lines[-1], "| end | 144 / 30 (18:00) |")


class CompareRatesTest(unittest.TestCase):
    def test_verdict_names_the_run_ahead_and_the_deciding_item(self):
        s_a = synthetic_samples()
        s_b = [dict(s, production={"made": {k: v * 2 for k, v in (s["production"]["made"] or {}).items()}
                                       if isinstance(s["production"]["made"], dict) else [], "consumed": []})
               for s in s_a]
        a = {"rates": ra.production_rates(s_a, ORIGIN, END, marks=(5, 10, 20))}
        b = {"rates": ra.production_rates(s_b, ORIGIN, END, marks=(5, 10, 20))}
        c = ra.compare_rates(a, b)
        self.assertTrue(c["present"])
        by = {m["minute"]: m for m in c["marks"]}
        self.assertEqual(by[5]["ahead"], "B")
        self.assertEqual(by[5]["decided_by"], "iron-plate")
        # At minute 10 red packs exist in both and differ: they outrank iron.
        self.assertEqual(by[10]["decided_by"], "automation-science-pack")
        self.assertEqual(by[10]["verdict"], "at 10:00 B ahead (red packs 12 vs 6)")
        self.assertEqual(by[10]["items"]["iron-plate"]["cumulative"]["delta"], 120)
        self.assertIsNone(by[20]["ahead"])
        self.assertIn("neither measured", by[20]["verdict"])
        end = by["end"]
        self.assertEqual(end["ahead"], "B")
        self.assertTrue(end["label"].startswith("end (A 18:00, B 18:00)"), end["label"])

    def test_level_and_absent(self):
        s = synthetic_samples()
        a = {"rates": ra.production_rates(s, ORIGIN, END, marks=(5,))}
        c = ra.compare_rates(a, a)
        self.assertEqual(c["marks"][0]["verdict"], "at 5:00 level")
        c2 = ra.compare_rates(a, {"rates": ra.production_rates([], ORIGIN, END)})
        self.assertFalse(c2["present"])
        self.assertIn("run B has no production samples", c2["reason"])


class EndToEndTest(unittest.TestCase):
    """A run directory on disk, through ``analyse`` and ``report``."""

    def write_run(self, root: str, name: str, samples: list[dict] | None) -> str:
        d = os.path.join(root, name)
        os.makedirs(d)
        events = [
            {"tick": ORIGIN, "kind": "run_started", "run_id": name, "bots": [1, 2]},
            {"tick": ORIGIN, "kind": "milestone_started", "index": 1, "goal": "researched:automation"},
            {"tick": ORIGIN + 7 * TPM, "kind": "milestone_satisfied", "index": 1, "iterations": 1},
            {"tick": END, "kind": "run_finished", "outcome": "done", "elapsed_ticks": END - ORIGIN},
        ]
        with open(os.path.join(d, "events.jsonl"), "w") as f:
            f.write("".join(json.dumps(e) + "\n" for e in events))
        if samples is not None:
            with open(os.path.join(d, "samples.jsonl"), "w") as f:
                f.write("".join(json.dumps(s) + "\n" for s in samples))
        return d

    def test_report_opens_with_the_table_then_milestones(self):
        with tempfile.TemporaryDirectory() as root:
            d = self.write_run(root, "run-1-1", synthetic_samples())
            a = ra.analyse(d)
            self.assertIn("| milestone 1 satisfied at 7:00", a["headline"])
            out = io.StringIO()
            ra.report(a, out=out)
            text = out.getvalue()
            i_rates = text.index("PRODUCTION AT FIXED MARKS")
            i_ms = text.index("MILESTONES")
            i_vision = text.index("FREE VISION")
            self.assertLess(i_rates, i_ms)
            self.assertLess(i_ms, i_vision)
            self.assertIn("20:00: run ended at 18:00", text)
            self.assertIn("iron-plate plateaus at 12:00 (144)", text)

    def test_report_without_samples_says_so(self):
        with tempfile.TemporaryDirectory() as root:
            d = self.write_run(root, "run-2-2", None)
            a = ra.analyse(d)
            self.assertFalse(a["rates"]["present"])
            out = io.StringIO()
            ra.report(a, out=out)
            self.assertIn("no samples.jsonl archived -- no production samples", out.getvalue())

    def test_compare_prints_rates_first_and_says_game_time(self):
        with tempfile.TemporaryDirectory() as root:
            a = ra.analyse(self.write_run(root, "run-3-3", synthetic_samples()))
            b = ra.analyse(self.write_run(root, "run-4-4", synthetic_samples()))
            c = ra.compare(a, b)
            self.assertFalse(c["numbers_withheld"])
            out = io.StringIO()
            ra.report_compare(c, out=out)
            text = out.getvalue()
            self.assertLess(text.index("PRODUCTION AT FIXED MARKS"), text.index("RUN SHAPE"))
            self.assertIn("ARE comparable here", text)
            self.assertIn("at 5:00 level", text)

    def test_rates_json_and_md_cli(self):
        with tempfile.TemporaryDirectory() as root:
            d = self.write_run(root, "run-5-5", synthetic_samples())
            saved = sys.stdout
            try:
                sys.stdout = io.StringIO()
                rc = ra.main(["--rates-json", "--marks", "5,10", d])
                js = json.loads(sys.stdout.getvalue())
                sys.stdout = io.StringIO()
                rc2 = ra.main(["--rates-md", d])
                md = sys.stdout.getvalue()
            finally:
                sys.stdout = saved
            self.assertEqual(rc, 0)
            self.assertEqual([m["minute"] for m in js["marks"] if not m["is_end"]], [5.0, 10.0])
            self.assertEqual(js["marks"][0]["items"]["iron-plate"]["cumulative"], 60)
            self.assertEqual(rc2, 0)
            self.assertTrue(md.startswith("| minute |"), md)


if __name__ == "__main__":
    unittest.main()
