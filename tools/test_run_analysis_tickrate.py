#!/usr/bin/env python3
"""The delivered tick rate, over time rather than as one average.

    python3 tools/test_run_analysis_tickrate.py

Nothing automated runs this -- ``just test`` runs clippy and ``cargo test``
only, and no CI workflow calls pytest -- so it is run by hand, like its two
neighbours ``test_run_analysis_rates.py`` and ``test_run_analysis_sustain.py``.

The case every test here is built around is a real one:
``run-1788696619-00325`` delivered 60 tps for its first two and a half
minutes, 27-50 tps for the next ninety seconds, and 60 again at the end. Its
run-wide average is 84% of nominal -- *above* ``STARVED_RATIO`` -- so the
analysis reported it with no flag at all, and the record contained nothing
that distinguished it from a run that held 60 tps throughout. An average is
exactly the statistic that cannot answer "when".
"""

from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_analysis as ra  # noqa: E402

BEAT_MS = 30_000  # what `BATCH_PROGRESS_INTERVAL` actually is
NOMINAL = 60


def beats(tps: list[float], start_tick: int = 1000) -> list[dict]:
    """A batch whose heartbeats deliver `tps[i]` ticks a second over the ith
    30-second interval.

    Written as one plan followed by its beats, in tick order, which is the
    order ``analyse`` hands ``delivered_tick_rate`` (it sorts by tick).

    The first beat is the *origin*: it opens the series and is not itself an
    interval, so ``len(tps)`` intervals need ``len(tps) + 1`` beats.
    """
    events: list[dict] = [{"kind": "plan_created", "tick": start_tick, "steps": 10}]
    tick, ms = start_tick, 0
    events.append({"kind": "batch_progress", "tick": tick, "elapsed_ms": ms})
    for rate in tps:
        ms += BEAT_MS
        tick += round(rate * BEAT_MS / 1000)
        events.append({"kind": "batch_progress", "tick": tick, "elapsed_ms": ms})
    return events


class DeliveredTickRate(unittest.TestCase):
    def test_a_run_that_held_its_speed_says_so_and_flags_nothing(self):
        r = ra.delivered_tick_rate(beats([60, 60, 60, 60]), "1.0")
        self.assertEqual(r["intervals"], 4)
        self.assertAlmostEqual(r["delivered_tps"], 60.0, places=6)
        self.assertFalse(r["starved"])
        self.assertFalse(r["sagged"])
        self.assertEqual(r["sag_spans"], [])
        line = " ".join(ra.tick_rate_profile(r, 1000))
        self.assertIn("speed held", line)

    def test_a_run_clean_then_starved_then_clean_is_not_hidden_by_its_average(self):
        # 84% overall -- above STARVED_RATIO, so the old single-average
        # report showed nothing at all. This is run-1788696619-00325's shape.
        r = ra.delivered_tick_rate(beats([60, 60, 60, 30, 30, 60, 60, 60]), "1.0")
        self.assertGreater(r["ratio"], ra.STARVED_RATIO)
        self.assertFalse(r["starved"], "the average alone does not fall short")
        self.assertTrue(r["sagged"], "and that is exactly why the average is not enough")
        self.assertEqual(r["sag_intervals"], 2)
        self.assertEqual(r["sag_ms"], 2 * BEAT_MS)
        self.assertAlmostEqual(r["sag_share"], 2 / 8, places=6)

    def test_the_sag_is_reported_as_one_contiguous_span_with_its_tick_bounds(self):
        r = ra.delivered_tick_rate(beats([60, 60, 30, 30, 60]), "1.0")
        self.assertEqual(len(r["sag_spans"]), 1)
        span = r["sag_spans"][0]
        # Two clean intervals of 1800 ticks precede it.
        self.assertEqual(span["tick_lo"], 1000 + 2 * 1800)
        self.assertEqual(span["tick_hi"], 1000 + 2 * 1800 + 2 * 900)
        self.assertEqual(span["intervals"], 2)
        self.assertAlmostEqual(span["tps"], 30.0, places=6)

    def test_two_sags_separated_by_a_clean_interval_are_two_spans(self):
        r = ra.delivered_tick_rate(beats([30, 60, 30]), "1.0")
        self.assertEqual([s["intervals"] for s in r["sag_spans"]], [1, 1])

    def test_the_worst_interval_is_named_and_it_is_the_worst_one(self):
        r = ra.delivered_tick_rate(beats([60, 20, 40, 60]), "1.0")
        self.assertAlmostEqual(r["worst"]["tps"], 20.0, places=6)
        self.assertEqual(r["worst"]["tick_lo"], 1000 + 1800)

    def test_the_batch_boundary_interval_is_kept_but_never_judged(self):
        # The batch's first dispatch opens the series: its tick is the
        # dispatch's and its wall baseline is the batch epoch, which is
        # earlier -- so it reads systematically slow and must not be reported
        # as the machine sagging.
        events = [
            {"kind": "plan_created", "tick": 1000, "steps": 10},
            {"kind": "action_dispatched", "tick": 1500, "id": 1},
            {"kind": "batch_progress", "tick": 1800, "elapsed_ms": BEAT_MS},
            {"kind": "batch_progress", "tick": 3600, "elapsed_ms": 2 * BEAT_MS},
        ]
        r = ra.delivered_tick_rate(events, "1.0")
        self.assertEqual(r["intervals"], 2)
        self.assertTrue(r["detail"][0]["boundary"])
        self.assertAlmostEqual(r["detail"][0]["tps"], 10.0, places=6)
        self.assertFalse(r["sagged"], "a boundary interval is never a sag")
        self.assertFalse(r["detail"][0].get("sagged"))
        self.assertEqual(r["worst"]["tick_lo"], 1800, "and it is never the worst either")

    def test_the_boundary_interval_is_still_shown_in_a_profile_marked_as_one(self):
        # Kept rather than hidden: a measurement that is not judged is still a
        # measurement, and dropping it would leave a hole in the profile with
        # nothing saying why.
        events = [
            {"kind": "plan_created", "tick": 1000, "steps": 10},
            {"kind": "action_dispatched", "tick": 1500, "id": 1},
            {"kind": "batch_progress", "tick": 1800, "elapsed_ms": BEAT_MS},
            {"kind": "batch_progress", "tick": 2700, "elapsed_ms": 2 * BEAT_MS},
        ]
        r = ra.delivered_tick_rate(events, "1.0")
        self.assertTrue(r["sagged"], "the second, judged interval is 30 tps")
        text = "\n".join(ra.tick_rate_profile(r, 1000))
        self.assertIn("10*", text)
        self.assertEqual(r["sag_intervals"], 1, "and the boundary one is not counted")

    def test_an_interval_too_short_to_measure_is_dropped(self):
        events = [
            {"kind": "plan_created", "tick": 1000, "steps": 10},
            {"kind": "batch_progress", "tick": 1000, "elapsed_ms": 0},
            {"kind": "batch_progress", "tick": 1001, "elapsed_ms": ra.MIN_RATE_INTERVAL_MS - 1},
            {"kind": "batch_progress", "tick": 3000, "elapsed_ms": 2 * ra.MIN_RATE_INTERVAL_MS},
        ]
        r = ra.delivered_tick_rate(events, "1.0")
        self.assertEqual(r["intervals"], 1)

    def test_an_unknown_game_speed_judges_nothing_rather_than_passing_the_run(self):
        r = ra.delivered_tick_rate(beats([60, 10, 10]), None)
        self.assertIsNone(r["nominal_tps"])
        self.assertIsNone(r["ratio"])
        self.assertFalse(r["starved"])
        self.assertFalse(r["sagged"])
        self.assertEqual(
            ra.tick_rate_profile(r, 1000),
            [],
            "with no nominal there is nothing to hold a rate against, and "
            "'speed held' would be a claim nobody measured",
        )

    def test_a_speed_that_is_not_1x_is_judged_against_its_own_nominal(self):
        # 300 tps requested, 300 delivered: fine. The same 300 at 10x is not.
        five = ra.delivered_tick_rate(beats([300, 300]), "5.0")
        self.assertEqual(five["nominal_tps"], 300.0)
        self.assertFalse(five["sagged"])
        ten = ra.delivered_tick_rate(beats([300, 300]), "10.0")
        self.assertEqual(ten["nominal_tps"], 600.0)
        self.assertTrue(ten["starved"])
        self.assertTrue(ten["sagged"])


class ReportedProfile(unittest.TestCase):
    def test_the_sag_window_is_named_in_game_time_from_the_run_start(self):
        r = ra.delivered_tick_rate(beats([60, 60, 30, 30, 60], start_tick=1000), "1.0")
        lines = ra.tick_rate_profile(r, 1000)
        text = "\n".join(lines)
        self.assertIn("did NOT hold that speed", text)
        # The sag opens 3600 ticks after the run's first tick (1:00) and
        # covers 1800 more ticks of *game* time -- two 30-second wall
        # intervals at half speed -- so it closes at 1:30, not at 2:00.
        self.assertIn("sagged 1:00 -> 1:30 game time", text)
        self.assertIn("60 s of wall at 30 tps average", text)
        self.assertIn("worst 30 tps", text)
        self.assertIn("profile", text)

    def test_the_profile_carries_every_interval_including_the_recovery(self):
        r = ra.delivered_tick_rate(beats([60, 30, 60]), "1.0")
        cells = "\n".join(ra.tick_rate_profile(r, 1000))
        self.assertEqual(cells.count(" 60"), 2, "both clean intervals are in the profile")
        self.assertIn(" 30", cells)


class SummaryLine(unittest.TestCase):
    """`--all --summary` is how a whole archive gets scanned."""

    BASE = {
        "run_id": "run-x",
        "span_ticks": 1000,
        "outcome": "done",
        "plans": [],
        "deaths": {},
        "vision": {"present": False},
        "samples_coverage": {},
        "milestones": [],
        "windows": [{"per_bot": {}, "span_ticks": 1000, "verb_ticks": {}}],
    }

    def line(self, rate: dict) -> str:
        return ra.summary_line({**self.BASE, "tick_rate": rate})

    def test_a_starved_run_is_flagged(self):
        self.assertIn("!starved=50%", self.line({"starved": True, "ratio": 0.5}))

    def test_a_run_that_only_sagged_is_flagged_differently(self):
        line = self.line({"starved": False, "sagged": True, "sag_share": 0.25})
        self.assertIn("!sagged=25%", line)
        self.assertNotIn("!starved", line)

    def test_a_clean_run_is_flagged_neither_way(self):
        line = self.line({"starved": False, "sagged": False})
        self.assertNotIn("!starved", line)
        self.assertNotIn("!sagged", line)


if __name__ == "__main__":
    unittest.main(verbosity=2)
