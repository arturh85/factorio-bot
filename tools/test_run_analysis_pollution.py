#!/usr/bin/env python3
"""Tests for the pollution / enemy-evolution block of ``run_analysis.py``.

Runs under ``python3 -m pytest tools/`` and under
``python3 tools/test_run_analysis_pollution.py``. Stdlib only, like the tool.

**What these are really defending.** The reading exists to answer whether our
own production provokes the biters, and the way that answer goes wrong is not
an arithmetic slip -- it is a run that never looked being rendered as a run
that measured a calm world. Every test here is about keeping those two apart:
`absent` must not become `0.0` at any level, and the tool must say so in
words rather than leaving a reader to infer a flat curve from empty columns.
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
END = ORIGIN + 15 * TPM


def force_sample(tick: int, pollution: dict | None) -> dict:
    """One force line. ``pollution=None`` writes NO key, like a schema-2 mod."""
    out = {
        "schema": 3,
        "tick": tick,
        "kind": "force",
        "research": None,
        "techs_unlocked": 0,
        "production": {"made": {}, "consumed": {}},
        "power": {"generated_kw": 0.0, "consumed_kw": 0.0, "satisfaction": 1.0,
                  "networks": {}},
    }
    if pollution is not None:
        out["pollution"] = pollution
    return out


def rising() -> list[dict]:
    """A run whose evolution rises, and rises mostly by POLLUTION.

    Both terms grow, because both do in a real game -- so a verdict that
    simply reported "it rose" would pass on a world where the clock did all
    the work. `by_pollution` outgrows `by_time` here by construction.
    """
    out = []
    for tick in range(ORIGIN, END + 1, 300):
        minute = (tick - ORIGIN) / TPM
        out.append(force_sample(tick, {"surfaces": {"nauvis": {
            "pollutant": "pollution",
            "total": 100.0 * minute,
            "at_spawn": 10.0 * minute,
            "produced": {"steam-engine": 60.0 * minute, "radar": 20.0 * minute},
            "absorbed": {"tree-01": 5.0 * minute},
            "evolution": {
                "factor": 0.0004 * minute,
                "by_pollution": 0.0003 * minute,
                "by_time": 0.0001 * minute,
                "by_killing_spawners": 0.0,
            },
        }}}))
    return out


def never_looked() -> list[dict]:
    return [force_sample(t, None) for t in range(ORIGIN, END + 1, 300)]


class PollutionMarksTest(unittest.TestCase):
    def marks(self, samples, **kw):
        return ra.pollution_marks(samples, ORIGIN, END, marks=(5, 10, 15), present=True, **kw)

    def test_a_run_that_never_looked_is_not_a_calm_run(self):
        """The failure this whole feature is built around.

        Every archived run predates the field. Rendering those as zeros would
        answer the owner's question with a number nobody measured.
        """
        r = self.marks(never_looked())
        self.assertEqual(r["status"], "not-captured")
        self.assertIn("never looked", r["reason"])
        self.assertEqual(r["marks"], [])
        self.assertIsNone(r["verdict"])

    def test_no_samples_at_all_is_its_own_status(self):
        """Distinct from `not-captured`: one run has no samples, the other has
        samples that do not carry the reading. The remedies differ."""
        r = ra.pollution_marks([], ORIGIN, END, marks=(5,), present=False)
        self.assertEqual(r["status"], "no-samples")

    def test_the_four_evolution_terms_arrive_at_each_mark(self):
        r = self.marks(rising())
        self.assertEqual(r["status"], "ok")
        self.assertEqual(r["surfaces"], ["nauvis"])
        m10 = next(m for m in r["marks"] if m["minute"] == 10)
        row = m10["surfaces"]["nauvis"]
        self.assertAlmostEqual(row["total"], 1000.0)
        self.assertAlmostEqual(row["at_spawn"], 100.0)
        e = row["evolution"]
        self.assertAlmostEqual(e["factor"], 0.004)
        self.assertAlmostEqual(e["by_pollution"], 0.003)
        self.assertAlmostEqual(e["by_time"], 0.001)
        self.assertEqual(e["by_killing_spawners"], 0.0)

    def test_emitters_are_ranked_so_the_cause_is_nameable(self):
        """"Was it the steam engines or the radar" is the owner's question in
        its concrete form, and the ranking is what answers it."""
        r = self.marks(rising())
        m15 = next(m for m in r["marks"] if m["minute"] == 15)
        top = m15["surfaces"]["nauvis"]["top_emitters"]
        self.assertEqual([name for name, _ in top], ["steam-engine", "radar"])

    def test_the_verdict_names_the_dominant_cause_not_just_the_rise(self):
        r = self.marks(rising())
        self.assertIn("mostly by pollution", r["verdict"])

    def test_a_flat_run_says_so_rather_than_inventing_a_cause(self):
        """A flat curve is a COMPLETE result -- it says combat can wait -- so
        it must not be reported as a small rise attributed to something."""
        flat = []
        for tick in range(ORIGIN, END + 1, 300):
            flat.append(force_sample(tick, {"surfaces": {"nauvis": {
                "pollutant": "pollution", "total": 0.0, "at_spawn": 0.0,
                "produced": {}, "absorbed": {},
                "evolution": {"factor": 0.0, "by_pollution": 0.0,
                              "by_time": 0.0, "by_killing_spawners": 0.0},
            }}}))
        r = self.marks(flat)
        self.assertIn("did not move", r["verdict"])

    def test_a_surface_we_failed_to_read_renders_as_unknown_not_zero(self):
        """The mod writes a key only on a successful read, so a surface whose
        reads raised arrives as `{}`. It must stay `None` all the way to the
        rendered line."""
        samples = [force_sample(t, {"surfaces": {"nauvis": {}}})
                   for t in range(ORIGIN, END + 1, 300)]
        r = self.marks(samples)
        self.assertEqual(r["status"], "ok")
        row = r["marks"][0]["surfaces"]["nauvis"]
        self.assertIsNone(row["total"])
        self.assertIsNone(row["at_spawn"])
        self.assertIsNone(row["evolution"])
        lines: list[str] = []
        ra.render_pollution(r, lines.append)
        text = "\n".join(lines)
        self.assertIn("?", text)
        self.assertNotIn("0.00000", text)

    def test_the_rendered_not_captured_paragraph_is_loud(self):
        lines: list[str] = []
        ra.render_pollution(self.marks(never_looked()), lines.append)
        text = "\n".join(lines)
        self.assertIn("NOT CAPTURED", text)
        # And it must not print a table a reader could mistake for a flat one.
        self.assertNotIn("by_poll", text)


class PollutionEndToEndTest(unittest.TestCase):
    """Through `analyse`, so the section is actually wired into a run report
    rather than only reachable by calling the function directly."""

    def test_a_run_directory_reports_its_pollution_section(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "events.jsonl"), "w") as f:
                f.write(json.dumps({"kind": "run_started", "tick": ORIGIN,
                                    "run_id": "r", "git": None}) + "\n")
            with open(os.path.join(d, "samples.jsonl"), "w") as f:
                for s in rising():
                    f.write(json.dumps(dict(s, run="r")) + "\n")
            # `report`'s `out=` default is bound to the real stdout at
            # definition time, so swapping `sys.stdout` captures nothing --
            # pass the buffer in instead.
            buf = io.StringIO()
            ra.report(ra.analyse(d), out=buf)
            text = buf.getvalue()
        self.assertIn("POLLUTION AND ENEMY EVOLUTION", text)
        self.assertIn("by_poll", text)


if __name__ == "__main__":
    unittest.main()
