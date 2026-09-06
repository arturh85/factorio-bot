#!/usr/bin/env python3
"""The hand-credit mass balance: the standing-rate check with no lead-in.

    python3 tools/test_run_analysis_balance.py
    python3 -m pytest tools/                       # if pytest is installed

Design note: ``docs/superpowers/notes/2026-09-06-standing-goals.md`` §3 (the
deferred second rung) and the run that made it urgent,
``docs/superpowers/notes/2026-09-06-standing-goals-first-rung.md``.

**The regression fixture in this file is not mine.**
``tools/fixtures/run-1788674059-90744`` is the archived run that
``sustained_rate`` called ``SUSTAINED`` for the wrong reason, copied out of
``workspace/headless-t/runs/`` unchanged except that the 328 ``bots`` sample
rows were dropped (neither function under test reads them) -- 56 KB instead of
280. It was recorded before this check existed, by a session that could not
have written it to agree, which is worth more than every synthetic case below
put together. :class:`ArchivedRunTest` also re-runs against the live archive
when it is still on this machine and asserts the two agree, so a trimmed
fixture cannot quietly drift from the run it claims to be.

The synthetic cases *were* written alongside the implementation, and say what
they assume in each docstring.
"""

from __future__ import annotations

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_analysis as ra  # noqa: E402
from test_run_analysis_rates import (  # noqa: E402
    ORIGIN,
    counted_furnace,
    force_sample,
    machines_sample,
)

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "fixtures", "run-1788674059-90744")
# Where the run was archived. Present on the machine that made it and nowhere
# else, which is why the fixture exists at all.
LIVE_ARCHIVE = os.path.join(
    os.path.dirname(HERE), "..", "..", "workspace", "headless-t", "runs", "run-1788674059-90744"
)

ITEM = "iron-plate"
PER_MINUTE = 15
WINDOW = 2 * ra.TICKS_PER_MINUTE  # 7,200 ticks
LEAD_IN = 9600  # the number the first rung chose, and the reason it passed
AT = ORIGIN + 20 * ra.TICKS_PER_MINUTE
BEAT = 300

# The run's own numbers, read off `events.jsonl` and quoted here so a reader
# can check the arithmetic without opening it: the last dispatch is at tick
# 4,520 and the window opens at 14,675.
ARCHIVE_AT = 21875
ARCHIVE_WINDOW = 7200


def read_jsonl(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def load(run_dir: str) -> tuple[list[dict], list[dict]]:
    return (
        read_jsonl(os.path.join(run_dir, "samples.jsonl")),
        read_jsonl(os.path.join(run_dir, "events.jsonl")),
    )


def counted_run(rate_per_beat: int, beats: int = 300) -> list[dict]:
    """One furnace counting `rate_per_beat` plates every 300 ticks, from tick 0.

    The machine's own lifetime counter and the force's statistics agree,
    because nothing in this run is hand-made.
    """
    out = []
    for i in range(beats + 1):
        tick = ORIGIN + i * BEAT
        made = i * rate_per_beat
        out.append(force_sample(tick, {ITEM: made}))
        out.append(machines_sample(tick, {"1": counted_furnace(made)}))
    return out


def delivery_event(tick: int, label: str, delivery: dict | None = None) -> dict:
    e = {"tick": tick, "kind": "action_dispatched", "id": 1, "bot": 1, "action": label}
    if delivery is not None:
        e["delivery"] = delivery
    return e


def balance(samples, events=(), **kw):
    args = {
        "item": ITEM,
        "per_minute": PER_MINUTE,
        "window_ticks": WINDOW,
        "at_tick": AT,
    }
    args.update(kw)
    return ra.hand_credit_balance(samples, list(events), **args)


class ArchivedRunTest(unittest.TestCase):
    """`run-1788674059-90744`: SUSTAINED on the lead-in, roster-fed on the balance.

    Four character bots at 5x on seed 31337, whose entire feeding history is
    two `fuel` dispatches at ticks 4,520 and 4,521. The window it was judged on
    opens 10,154 ticks after the last of them, so a 9,600-tick lead-in reaches
    back to 5,075 and sees nothing -- and the coal it did not see was worth
    36,800 ticks.
    """

    def setUp(self):
        self.samples, self.events = load(FIXTURE)
        self.old = ra.sustained_rate(
            self.samples,
            self.events,
            item=ITEM,
            per_minute=PER_MINUTE,
            window_ticks=ARCHIVE_WINDOW,
            lead_in_ticks=LEAD_IN,
            at_tick=ARCHIVE_AT,
        )
        self.new = ra.hand_credit_balance(
            self.samples,
            self.events,
            item=ITEM,
            per_minute=PER_MINUTE,
            window_ticks=ARCHIVE_WINDOW,
            at_tick=ARCHIVE_AT,
        )

    def test_the_lead_in_check_still_says_sustained(self):
        """The old verdict is unchanged, on purpose.

        The balance is an additional verdict, not a replacement, until it has
        earned one. If this test goes red the lead-in check has been altered
        as a side effect, which is the thing this task promised not to do.
        """
        self.assertEqual(self.old["verdict"], "sustained")
        self.assertEqual(self.old["machine_made"], 30)
        self.assertEqual(self.old["required"], 30)
        self.assertEqual(self.old["feeding_in_window"], 0)
        self.assertEqual(self.old["feeding_in_lead_in"], 0)

    def test_the_balance_calls_it_roster_fed(self):
        """The pass the run should not have got, refused with no lead-in.

        Nothing was passed to this function about how long a fuel charge
        lasts, how much a slot holds, or how quiet the roster had to be
        beforehand.
        """
        self.assertEqual(self.new["verdict"], "roster-fed")
        self.assertEqual(self.new["machine_made"], 30)

    def test_the_credit_is_the_two_coal_charges(self):
        """153 plates behind the drill's coal, 194 behind the furnace's.

        23 coal x 1,600 ticks / 240 ticks an ore = 153.3, which is the number
        the planner wrote into its own action label ("36800 ticks, 153
        iron-plate, then it stops") -- an oracle written by neither this
        function nor its author. 14 coal x 2,666 / 192 = 194.4.

        The two are alternative bounds on the same plates and the larger is
        taken; the verdict is `roster-fed` under either, which is asserted
        below so the choice is visible rather than load-bearing.
        """
        stages = self.new["by_stage"]
        self.assertEqual(sorted(stages), ["burner-mining-drill/coal", "stone-furnace/coal"])
        self.assertAlmostEqual(stages["burner-mining-drill/coal"], 153.33, places=1)
        self.assertAlmostEqual(stages["stone-furnace/coal"], 194.40, places=1)
        self.assertAlmostEqual(self.new["hand_credit"], 194.40, places=1)
        # Under the smaller bound as well: 153 - 40 spent = 113 outstanding
        # against 30 made in the window.
        self.assertGreater(153.33 - self.new["spent_before_window"], self.new["machine_made"])
        self.assertEqual(self.new["credit_source"], ["label"])

    def test_every_delivery_in_the_run_was_priced(self):
        """An unpriced delivery would have made the verdict `unknown`.

        Stated separately because "roster-fed" and "unknown" are both
        refusals, and only one of them is a measurement.
        """
        self.assertEqual(self.new["unreadable"], [])
        self.assertEqual(self.new["unpriced"], [])
        self.assertEqual(self.new["deliveries"], 2)

    def test_the_fixture_agrees_with_the_live_archive(self):
        """The trimmed fixture and the run it was cut from give one answer.

        Skipped where the archive no longer exists, which is every machine but
        the one that made it.
        """
        if not os.path.isdir(LIVE_ARCHIVE):
            self.skipTest(f"no live archive at {LIVE_ARCHIVE}")
        samples, events = load(LIVE_ARCHIVE)
        live = ra.hand_credit_balance(
            samples,
            events,
            item=ITEM,
            per_minute=PER_MINUTE,
            window_ticks=ARCHIVE_WINDOW,
            at_tick=ARCHIVE_AT,
        )
        for field in ("verdict", "machine_made", "hand_credit", "spent_before_window"):
            self.assertEqual(live[field], self.new[field], field)


class BalanceTest(unittest.TestCase):
    """What the ledger says about a run nobody archived.

    Every fixture here was written by the author of the code under test. Each
    assumes the analyser's own sample shape (`counted_furnace`, which belongs
    to the rates tests) and the label wording the planner writes today.
    """

    def test_a_factory_nobody_fed_is_sustained(self):
        """No delivery anywhere in the run: nothing to explain the output with.

        120 plates in the window against 30 required, and zero credit.
        """
        r = balance(counted_run(rate_per_beat=5))
        self.assertEqual(r["verdict"], "sustained")
        self.assertEqual(r["hand_credit"], 0.0)
        self.assertEqual(r["machine_made"], 120)
        self.assertEqual(r["unexplained"], 120)

    def test_a_hand_charge_bigger_than_the_output_is_roster_fed(self):
        """500 ore delivered, 216 plates made before the window, 24 inside it.

        The charge has 284 plates of credit left when the window opens, which
        is more than everything the window contains -- so nothing in it needs a
        standing supply to explain, and no lead-in had to be chosen to say so.
        """
        r = balance(
            counted_run(rate_per_beat=1),
            [delivery_event(ORIGIN, "insert 500 iron-ore into stone-furnace")],
        )
        self.assertEqual(r["verdict"], "roster-fed")
        self.assertEqual(r["hand_credit"], 500.0)
        self.assertEqual(r["spent_before_window"], 216)
        self.assertEqual(r["outstanding_credit"], 284.0)
        self.assertEqual(r["machine_made"], 24)

    def test_credit_already_eaten_does_not_count_twice(self):
        """The same 500 ore against a run that made 1,080 plates before the window.

        The lead-in check has no way to express this: a delivery is a
        disqualification for a fixed time and then stops mattering all at once.
        A ledger draws it down as it is used, so a charge long since smelted
        explains nothing and the window stands on its own.
        """
        r = balance(
            counted_run(rate_per_beat=5),
            [delivery_event(ORIGIN, "insert 500 iron-ore into stone-furnace")],
        )
        self.assertEqual(r["spent_before_window"], 1080)
        self.assertEqual(r["outstanding_credit"], 0.0)
        self.assertEqual(r["verdict"], "sustained")

    def test_unexplained_output_below_the_rate_is_short(self):
        """1,180 ore of credit, 100 of it outstanding, 120 plates in the window.

        20 plates are unexplained where 30 are asked for: `short`, and named by
        the shortfall rather than blamed on a cause nothing observed.
        """
        r = balance(
            counted_run(rate_per_beat=5),
            [delivery_event(ORIGIN, "insert 1180 iron-ore into stone-furnace")],
        )
        self.assertEqual(r["outstanding_credit"], 100.0)
        self.assertEqual(r["unexplained"], 20)
        self.assertEqual(r["verdict"], "short")

    def test_a_delivery_it_cannot_read_is_unknown_not_sustained(self):
        """An `insert` whose label matches no known shape.

        The credit is then a lower bound, and a verdict resting on a lower
        bound of the roster's contribution is exactly the false pass this check
        exists to remove. `unknown` names the label it could not read.
        """
        r = balance(
            counted_run(rate_per_beat=5),
            [delivery_event(ORIGIN, "insert some iron-ore somewhere")],
        )
        self.assertEqual(r["verdict"], "unknown")
        self.assertEqual(len(r["unreadable"]), 1)

    def test_an_item_with_no_recipe_is_unknown_not_sustained(self):
        """A delivery of something whose relationship to the goal is unknown.

        `RECIPES` is a short table of stated recipes, not a prototype dump. A
        target item missing from it cannot be balanced, and saying so is the
        instruction to add the row with its provenance.
        """
        r = balance(
            counted_run(rate_per_beat=5),
            [delivery_event(ORIGIN, "insert 5 uranium-ore into centrifuge")],
            item="processing-unit",
        )
        self.assertEqual(r["verdict"], "unknown")
        self.assertEqual(len(r["unpriced"]), 1)

    def test_a_delivery_that_cannot_explain_the_goal_credits_nothing(self):
        """5 gears into a chest cannot account for a plate.

        Credited zero rather than refused: the recipe for the goal IS known, so
        this is a fact and not an absence.
        """
        r = balance(
            counted_run(rate_per_beat=5),
            [delivery_event(ORIGIN, "stock the wooden-chest with 5 iron-gear-wheel")],
        )
        self.assertEqual(r["hand_credit"], 0.0)
        self.assertEqual(r["verdict"], "sustained")

    def test_structured_fields_beat_the_label(self):
        """`EventKind::ActionDispatched.delivery`, when a run has one.

        The label says 999 coal and the field says 10; the field wins, because
        prose is the fallback for runs archived before the field existed and
        never the source of record. 10 coal in a stone furnace is 10 x 2,666 /
        192 = 138.9 plates.
        """
        r = balance(
            counted_run(rate_per_beat=1),
            [
                delivery_event(
                    ORIGIN,
                    "fuel the stone-furnace with 999 coal",
                    {"item": "coal", "count": 10, "entity": "stone-furnace", "slot": "fuel"},
                )
            ],
        )
        self.assertAlmostEqual(r["hand_credit"], 138.85, places=1)
        self.assertEqual(r["credit_source"], ["fields"])

    def test_coal_in_a_chest_is_priced_at_the_most_generous_burner(self):
        """Nothing says which machine a chest's coal reached.

        A stone furnace turns one coal into 13.9 plates and a drill into 6.7,
        so a chest's coal is credited at the furnace's rate: the largest
        explanation available to the roster, which is the direction that
        refuses rather than the one that passes.
        """
        r = balance(
            counted_run(rate_per_beat=1),
            [delivery_event(ORIGIN, "stock the wooden-chest with 10 coal")],
        )
        self.assertAlmostEqual(r["hand_credit"], 138.85, places=1)

    def test_mining_and_taking_are_not_deliveries(self):
        """`mine` and `take` move material into a bot's hands, not a machine.

        Both are in `FEEDING_VERBS` -- the lead-in check disqualifies a window
        for either -- and neither credits anything here, because a machine
        cannot produce from a bot's inventory.
        """
        r = balance(
            counted_run(rate_per_beat=5),
            [
                delivery_event(ORIGIN, "mine 50 iron-ore at [1, 2]"),
                delivery_event(ORIGIN, "take 50 iron-plate from stone-furnace"),
            ],
        )
        self.assertEqual(r["deliveries"], 0)
        self.assertEqual(r["hand_credit"], 0.0)
        self.assertEqual(r["verdict"], "sustained")

    def test_a_run_without_counters_is_unknown(self):
        """No per-machine `produced`: nothing to balance the deliveries against.

        Never `sustained`, which would be a claim nothing measured.
        """
        samples = []
        for i in range(301):
            tick = ORIGIN + i * BEAT
            m = counted_furnace(i * 5)
            del m["produced"]
            del m["produced_source"]
            samples.append(force_sample(tick, {ITEM: i * 5}))
            samples.append(machines_sample(tick, {"1": m}))
        r = balance(samples)
        self.assertEqual(r["verdict"], "unknown")
        self.assertEqual(r["source"], "unavailable")


class NoLeadInTest(unittest.TestCase):
    """The parameter is gone, and cannot be smuggled back in."""

    def test_the_balance_takes_no_lead_in(self):
        """A caller passing one is refused rather than having it ignored.

        The whole cost of the first rung was a lead-in that was too short and
        looked like a measurement. A function that accepted and ignored one
        would read as though the number still mattered.
        """
        with self.assertRaises(TypeError):
            ra.hand_credit_balance(
                counted_run(rate_per_beat=5),
                [],
                item=ITEM,
                per_minute=PER_MINUTE,
                window_ticks=WINDOW,
                at_tick=AT,
                lead_in_ticks=LEAD_IN,
            )

    def test_sustained_rate_reports_both_verdicts(self):
        """The lead-in check keeps its own verdict and carries the balance's.

        Additional, not a replacement: one report, two answers, and a reader
        who can see them disagree.
        """
        r = ra.sustained_rate(
            counted_run(rate_per_beat=5),
            [delivery_event(AT - WINDOW - 5000, "insert 10 iron-ore into stone-furnace")],
            item=ITEM,
            per_minute=PER_MINUTE,
            window_ticks=WINDOW,
            lead_in_ticks=LEAD_IN,
            at_tick=AT,
        )
        # The lead-in refuses it: a bot fed a machine 5,000 ticks before the
        # window opened.
        self.assertEqual(r["verdict"], "roster-fed")
        # The balance does not: ten ore cannot explain 120 plates, and the run
        # had already smelted its way through that credit besides.
        self.assertEqual(r["balance"]["verdict"], "sustained")


if __name__ == "__main__":
    unittest.main(verbosity=2)
