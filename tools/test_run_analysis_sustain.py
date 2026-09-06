#!/usr/bin/env python3
"""The verification half of a standing goal, pinned before it exists.

Design note: ``docs/superpowers/notes/2026-09-06-standing-goals.md``.

**Every test in this file fails today**, with ``AttributeError: module
'run_analysis' has no attribute 'sustained_rate'``. That is the intended
failure: the check is not implemented. Nothing automated runs these -- ``just
test`` runs clippy and ``cargo test`` only, and no CI workflow calls pytest --
so a red file here breaks no one's suite; it is a specification with a runner.

    python3 tools/test_run_analysis_sustain.py

What is being specified is one function::

    ra.sustained_rate(samples, events, item, per_minute,
                      window_ticks, lead_in_ticks, at_tick) -> dict

and the five answers it may give. The reason it cannot be
``attribute_output`` with a longer interval is the third test: an interval can
be entirely free of feeding actions and still be roster-fed, because a machine
hand-charged *before* the interval is still chewing through that charge inside
it. A stone furnace's input slot holds one stack of 50 ore at 3.2 s a plate --
9,600 ticks of hand-fed running -- so the existing green witness's 90-second
window (5,400 ticks, every bot idle) sits comfortably inside a single hand
load. It proves the machines are *running*. It cannot prove anything is
*feeding* them, and that is the whole content of a standing goal.
"""

from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_analysis as ra  # noqa: E402
from test_run_analysis_rates import (  # noqa: E402
    ORIGIN,
    counted_furnace,
    feeding_events,
    force_sample,
    machines_sample,
)

# The first rung's numbers: 15 iron plates a minute -- one burner cell's output
# -- held for two minutes, behind a lead-in one full hand-charged input stack
# long.
ITEM = "iron-plate"
PER_MINUTE = 15
WINDOW = 2 * ra.TICKS_PER_MINUTE  # 7,200 ticks
LEAD_IN = 9600  # one 50-ore stack at 3.2 s a plate
AT = ORIGIN + 20 * ra.TICKS_PER_MINUTE
BEAT = 300


def counted_run(rate_per_beat: int, beats: int = 300) -> list[dict]:
    """A run whose one furnace counts `rate_per_beat` plates every 300 ticks.

    Both records that matter are written on the same beat: the machine's own
    lifetime counter and the force's production statistics, agreeing, because
    in this run nothing is hand-made.
    """
    out = []
    for i in range(beats + 1):
        tick = ORIGIN + i * BEAT
        made = i * rate_per_beat
        out.append(force_sample(tick, {ITEM: made}))
        out.append(machines_sample(tick, {"1": counted_furnace(made)}))
    return out


def sustained(samples, events=(), **kw):
    args = {
        "item": ITEM,
        "per_minute": PER_MINUTE,
        "window_ticks": WINDOW,
        "lead_in_ticks": LEAD_IN,
        "at_tick": AT,
    }
    args.update(kw)
    return ra.sustained_rate(samples, list(events), **args)


class SustainedRateTest(unittest.TestCase):
    """What it takes to say a rate was sustained, and what it takes to refuse."""

    def test_machines_made_the_rate_and_nobody_fed_them(self):
        """The only shape that may be called sustained.

        15/min over 7,200 ticks is 30 plates; the furnace counts 120. No
        feeding verb was dispatched inside the window or inside the lead-in
        before it, so nothing a bot carried can explain the output.

        **This number read 75 when the file was written as a specification,
        and 75 is arithmetically impossible for this fixture.** The window is
        7,200 ticks on a 300-tick beat -- 24 beats -- and the furnace counts
        ``rate_per_beat`` each: 24 x 5 = 120. The sibling test below fixes the
        same arithmetic at ``rate_per_beat=1`` and expects 24, which is the
        same 24 beats, so the two expectations contradicted each other and
        only one of them could be right. Corrected here rather than
        accommodated in ``sustained_rate``, and called out in the
        implementing session's report: a fixture is a statement about the
        world, and this one was wrong about it.
        """
        r = sustained(counted_run(rate_per_beat=5))
        self.assertEqual(r["verdict"], "sustained")
        self.assertEqual(r["required"], 30)
        self.assertEqual(r["machine_made"], 120)
        self.assertEqual(r["feeding_in_window"], 0)
        self.assertEqual(r["feeding_in_lead_in"], 0)
        self.assertEqual(r["source"], "counters")

    def test_output_short_of_the_rate_is_short_not_sustained(self):
        """One plate a beat is 12/min: a working factory, below the goal.

        Refused as ``short`` and not as ``roster-fed``: it names the number
        that was missed rather than blaming a cause nothing here observed.
        """
        r = sustained(counted_run(rate_per_beat=1))
        self.assertEqual(r["verdict"], "short")
        self.assertEqual(r["machine_made"], 24)
        self.assertEqual(r["required"], 30)

    def test_a_bot_feeding_inside_the_window_disqualifies_it(self):
        """The machines made every plate, and a bot carried what went in.

        This is the verdict ``attribute_output`` already reaches from the
        counters plus the feeding count, and a standing goal must inherit it
        rather than settle for "the machines made it".
        """
        r = sustained(counted_run(rate_per_beat=5), feeding_events([AT - 1200]))
        self.assertEqual(r["verdict"], "roster-fed")
        self.assertEqual(r["feeding_in_window"], 1)

    def test_a_bot_feeding_in_the_lead_in_disqualifies_it_too(self):
        """**The case the existing witness cannot see.**

        Not one feeding action inside the window -- every bot idle for the
        whole two minutes -- and the cell still fails, because a hand charge
        5,000 ticks before the window is still being smelted inside it. The
        lead-in is what makes the difference between "the machines are running"
        and "something is feeding them", and it is why the green witness's
        90-second every-bot-idle window is not sufficient as a general rule.
        """
        r = sustained(counted_run(rate_per_beat=5), feeding_events([AT - WINDOW - 5000]))
        self.assertEqual(r["feeding_in_window"], 0)
        self.assertEqual(r["feeding_in_lead_in"], 1)
        self.assertEqual(r["verdict"], "roster-fed")

    def test_force_statistics_alone_never_satisfy_it(self):
        """A run that hand-crafted its way to the rate.

        The force's ``production.made`` grows at 15/min and no machine counter
        moves at all: hand crafting and hand mining pass through no machine.
        Measuring a standing goal on ``production.made`` -- the series every
        rate table in this project is drawn from -- would call this sustained.
        """
        samples = []
        for i in range(301):
            tick = ORIGIN + i * BEAT
            samples.append(force_sample(tick, {ITEM: i * 5}))
            samples.append(machines_sample(tick, {"1": counted_furnace(0)}))
        r = sustained(samples)
        self.assertEqual(r["verdict"], "hand-made")
        self.assertEqual(r["machine_made"], 0)

    def test_a_run_without_counters_is_unknown_and_says_so(self):
        """Absence of the instrument is not a verdict either way.

        Every run archived before ``13d45c6b`` has machine rows with no
        ``produced`` field. The honest answer is ``unknown`` -- never
        ``sustained`` (which would be a claim nothing measured) and never
        ``short`` (which would report a failure that may not have happened).
        """
        samples = []
        for i in range(301):
            tick = ORIGIN + i * BEAT
            m = counted_furnace(i * 5)
            del m["produced"]
            del m["produced_source"]
            samples.append(force_sample(tick, {ITEM: i * 5}))
            samples.append(machines_sample(tick, {"1": m}))
        r = sustained(samples)
        self.assertEqual(r["verdict"], "unknown")
        self.assertEqual(r["source"], "unavailable")
        self.assertIn("counter", r["why"])


class SustainWindowTest(unittest.TestCase):
    """The window is a parameter with no default, like the witness's."""

    def test_the_window_must_be_named(self):
        """A rate with no window is ``Goal::Producing``: capacity, not output.

        Guessing one would hand back a verdict nobody derived -- the same
        reasoning that makes ``supervisor.witness``'s ``within_ticks``
        mandatory.
        """
        with self.assertRaises(TypeError):
            ra.sustained_rate(
                counted_run(rate_per_beat=5),
                [],
                item=ITEM,
                per_minute=PER_MINUTE,
                at_tick=AT,
            )


if __name__ == "__main__":
    unittest.main(verbosity=2)
