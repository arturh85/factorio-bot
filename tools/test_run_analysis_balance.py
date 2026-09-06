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
# The belted cell: the run that showed credit pooling by prototype.
BELTED_FIXTURE = os.path.join(HERE, "fixtures", "run-1788679826-02267")
BELTED_ARCHIVE = os.path.join(
    os.path.dirname(HERE), "..", "..", "workspace", "headless-v", "runs", "run-1788679826-02267"
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

        Both are read from the run's prose labels, and **both labels carry the
        machine's position**, which is what the instance grouping joins on.
        """
        stages = self.new["by_stage"]
        self.assertEqual(
            sorted(stages),
            ["burner-mining-drill@[-13.0, -14.0]/coal", "stone-furnace@[-13.0, -12.0]/coal"],
        )
        self.assertAlmostEqual(stages["burner-mining-drill@[-13.0, -14.0]/coal"], 153.33, places=1)
        self.assertAlmostEqual(stages["stone-furnace@[-13.0, -12.0]/coal"], 194.40, places=1)
        self.assertEqual(self.new["credit_source"], ["label"])

    def test_the_furnaces_coal_is_attributed_and_the_drills_floats(self):
        """The drill mines ore; the plates come out of the furnace.

        So the furnace's 194 is credit against *that furnace's* counter, and
        the drill's 153 -- a bound on the plates some other machine makes from
        its ore -- can be pinned to no plate-producing instance and floats.
        Both refuse; only one of them belongs to a machine.
        """
        self.assertAlmostEqual(self.new["attributed_credit"], 194.40, places=1)
        self.assertAlmostEqual(self.new["floating_credit"], 153.33, places=1)
        self.assertEqual(
            list(self.new["floating_stages"]), ["burner-mining-drill@[-13.0, -14.0]"]
        )
        self.assertAlmostEqual(self.new["hand_credit"], 347.73, places=1)

    def test_the_one_furnace_is_balanced_against_its_own_output(self):
        """Every plate in this run came out of one machine, and it is named.

        194 of credit, 40 plates spent before the window, 154 still standing,
        30 made inside it: the window is inside its own machine's credit
        without borrowing any from the drill or the pool.
        """
        rows = self.new["per_machine"]
        self.assertEqual([r["position"] for r in rows], [[-13.0, -12.0]])
        row = rows[0]
        self.assertEqual(row["name"], "stone-furnace")
        self.assertEqual(row["spent"], 40)
        self.assertEqual(row["made"], 30)
        self.assertAlmostEqual(row["outstanding"], 154.40, places=1)
        self.assertGreater(row["outstanding"], row["made"])

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


class InstanceGroupingTest(unittest.TestCase):
    """Credit belongs to a machine, not to a prototype.

    Two stone furnaces standing apart: one hand-fed and eating its own credit,
    one fed by something the record cannot see. Under the old grouping the
    hand-fed one's coal explained the belted one's plates, because both are
    `stone-furnace`. Every fixture in this class was written by the author of
    the change under test and assumes the analyser's sample shape; the
    archived runs above were not, and are the evidence.
    """

    HAND = (10.0, 10.0)
    BELT = (20.0, 20.0)

    def two_furnaces(self, hand_made, belt_made, beats: int = 300) -> list[dict]:
        """`hand_made(beat)` and `belt_made(beat)` plates, at two positions."""
        out = []
        for i in range(beats + 1):
            tick = ORIGIN + i * BEAT
            a = counted_furnace(hand_made(i))
            a["position"] = {"x": self.HAND[0], "y": self.HAND[1]}
            b = counted_furnace(belt_made(i))
            b["position"] = {"x": self.BELT[0], "y": self.BELT[1]}
            out.append(force_sample(tick, {ITEM: hand_made(i) + belt_made(i)}))
            out.append(machines_sample(tick, {"1": a, "2": b}))
        return out

    def charge(self, count: int, where):
        """One hand delivery of ore into the furnace at ``where``."""
        e = delivery_event(
            ORIGIN,
            f"insert {count} iron-ore into stone-furnace",
            {"item": "iron-ore", "count": count, "entity": "stone-furnace", "slot": "furnace_source"},
        )
        e["target"] = {"x": where[0], "y": where[1]}
        return e

    def test_a_hand_fed_furnace_does_not_explain_its_neighbours_output(self):
        """THE DEFECT. 900 ore into one furnace; the other one makes 120.

        The hand-fed furnace makes 3 plates a beat and has smelted 900 by the
        window, so its charge is spent and nothing of it is left. The second
        furnace makes 5 a beat -- 120 in the window -- and was never touched.
        Pooling by prototype leaves 900 - 1,980 = 0... but the pooled `spent`
        is both furnaces' output, so the arithmetic that mattered was the
        credit: one charge covering two machines.

        Here the second furnace's 120 stand alone, which is `sustained`.
        """
        r = balance(
            self.two_furnaces(lambda i: i * 3, lambda i: i * 5),
            [self.charge(900, self.HAND)],
        )
        self.assertEqual(r["attributed_credit"], 900.0)
        self.assertEqual(r["floating_credit"], 0.0)
        rows = {tuple(m["position"]): m for m in r["per_machine"]}
        self.assertEqual(rows[self.HAND]["credit"], 900.0)
        self.assertEqual(rows[self.BELT]["credit"], 0.0)
        self.assertEqual(rows[self.BELT]["made"], 120)
        self.assertEqual(rows[self.BELT]["outstanding"], 0.0)
        self.assertEqual(r["unexplained"], 120.0)
        self.assertEqual(r["verdict"], "sustained")

    def test_a_delivery_with_no_position_is_attributed_to_nobody(self):
        """The same charge, stripped of its `target`, must not pick a machine.

        Every archived run has a position on every `insert`, but a record that
        lost one must fall back to the pool rather than to a guess -- and the
        pool refuses more, not less, because it can explain any machine.

        Same samples and same delivery as the test above; the only difference
        is the missing position, which is the substitution this test exists to
        make.
        """
        blind = self.charge(900, self.HAND)
        del blind["target"]
        self.assertNotIn("target", blind)
        r = balance(self.two_furnaces(lambda i: i * 3, lambda i: i * 5), [blind])
        self.assertEqual(r["attributed_credit"], 0.0)
        self.assertEqual(r["floating_credit"], 900.0)
        rows = {tuple(m["position"]): m for m in r["per_machine"]}
        self.assertEqual(rows[self.HAND]["credit"], 0.0)
        self.assertEqual(rows[self.BELT]["credit"], 0.0)
        # 1,620 plates were made before the window, so the pool is empty and
        # the window's 120 + 72 stand unexplained -- `sustained`, and reached
        # without ever pinning the charge to a furnace.
        self.assertEqual(r["floating_outstanding"], 0.0)
        self.assertEqual(r["verdict"], "sustained")

    def test_surplus_credit_stays_with_its_own_machine(self):
        """A furnace with credit to spare does not subsidise the one next door.

        The hand-fed furnace gets 5,000 ore and makes nothing at all, so 5,000
        of credit stands at it forever. The belted furnace makes 120 in the
        window from nothing anyone carried, and that is `sustained` -- under
        the pooled grouping the 5,000 would have swallowed it whole.
        """
        r = balance(
            self.two_furnaces(lambda i: 0, lambda i: i * 5),
            [self.charge(5000, self.HAND)],
        )
        rows = {tuple(m["position"]): m for m in r["per_machine"]}
        self.assertEqual(rows[self.HAND]["outstanding"], 5000.0)
        self.assertEqual(rows[self.BELT]["outstanding"], 0.0)
        self.assertEqual(r["outstanding_credit"], 5000.0)
        self.assertEqual(r["unexplained"], 120.0)
        self.assertEqual(r["verdict"], "sustained")

    def test_a_charge_bigger_than_its_own_machines_output_still_refuses(self):
        """The direction is kept: credit that IS at the machine still refuses.

        1,500 ore into the belted furnace, which makes 1,080 before the window
        and 120 inside it. 420 of its own credit is still standing, so its
        output explains nothing about a standing supply.
        """
        r = balance(
            self.two_furnaces(lambda i: 0, lambda i: i * 5),
            [self.charge(1500, self.BELT)],
        )
        rows = {tuple(m["position"]): m for m in r["per_machine"]}
        self.assertEqual(rows[self.BELT]["spent"], 1080)
        self.assertEqual(rows[self.BELT]["outstanding"], 420.0)
        self.assertEqual(r["unexplained"], 0.0)
        self.assertEqual(r["verdict"], "roster-fed")

    def test_a_target_that_matches_no_sampled_machine_floats(self):
        """An unjoined delivery is not attributed to a guess.

        A position no machine ever stood on -- a machine placed and removed
        before the first sample, or a planner intent the game resolved
        elsewhere. It becomes floating credit, which can explain any machine's
        output: the direction that refuses.
        """
        r = balance(
            self.two_furnaces(lambda i: 0, lambda i: i * 1),
            [self.charge(500, (99.0, 99.0))],
        )
        self.assertEqual(r["attributed_credit"], 0.0)
        self.assertEqual(r["floating_credit"], 500.0)
        self.assertEqual(r["verdict"], "roster-fed")

    def test_a_chests_credit_names_no_machine_and_floats(self):
        """`stock`/`charge` names a container, and containers make nothing.

        The chest is at a real sampled position, but its `item` is never
        `iron-plate`, so its credit cannot be pinned to a plate-producing
        machine and joins the floating pool.
        """
        samples = self.two_furnaces(lambda i: 0, lambda i: i * 5)
        chest = {
            "name": "wooden-chest",
            "type": "container",
            "position": {"x": 30.0, "y": 30.0},
            "status": "normal",
            "produced_source": "not-a-producer",
            "input": {},
            "output": {},
            "fuel": {},
        }
        for s in samples:
            if s.get("kind") == "machines":
                s["machines"]["3"] = chest
        e = delivery_event(
            ORIGIN,
            "stock the wooden-chest with 200 iron-ore",
            {"item": "iron-ore", "count": 200, "entity": "wooden-chest", "slot": "chest"},
        )
        e["target"] = {"x": 30.0, "y": 30.0}
        r = balance(samples, [e])
        self.assertEqual(r["attributed_credit"], 0.0)
        self.assertEqual(r["floating_credit"], 200.0)
        # 1,080 plates were made before the window, which is more than the
        # chest could ever have explained, so the pool is empty by then.
        self.assertEqual(r["floating_outstanding"], 0.0)
        self.assertEqual(r["verdict"], "sustained")


class BeltedCellTest(unittest.TestCase):
    """`run-1788679826-02267`: the first cell that ran with no bot in the loop.

    **This fixture is not mine either.** ``tools/fixtures/run-1788679826-02267``
    is the archived run copied out of ``workspace/headless-v/runs/`` unchanged
    except that its 711 ``bots`` sample rows were dropped -- recorded on
    `0831cc3c`, before this change existed, by a session that reported the
    defect rather than fixing it
    (``docs/superpowers/notes/2026-09-06-a-cell-that-feeds-itself.md``).

    Pooled by prototype it read `roster-fed` on 333 of credit, 98 of it still
    outstanding: a cell rated at 15/min would have had to make 64/min to be
    believed. Keyed by instance it reads `short`, 11 of 30 -- which is the
    truth about it, because nothing takes its plates away and it throttles on
    `full_output`.
    """

    AT = 42979
    WINDOW = 7200

    def setUp(self):
        self.samples, self.events = load(BELTED_FIXTURE)
        self.r = ra.hand_credit_balance(
            self.samples,
            self.events,
            item=ITEM,
            per_minute=PER_MINUTE,
            window_ticks=self.WINDOW,
            at_tick=self.AT,
        )

    def test_it_is_short_rather_than_roster_fed(self):
        """11 plates in the window, 30 asked for, and none of them explained.

        `short` is a refusal too. Nothing here was tuned to make it pass, and
        the reason it does not is the one the run's own machine line gives:
        `full_output` on 15 of 129 samples.
        """
        self.assertEqual(self.r["verdict"], "short")
        self.assertEqual(self.r["machine_made"], 11)
        self.assertEqual(self.r["unexplained"], 11.0)
        self.assertEqual(self.r["required"], 30)

    def test_the_belted_furnace_ate_its_own_credit(self):
        """[-5, -27]: 69 of hand coal against 107 plates of its own, before.

        This is the whole change in one row. That furnace received one coal
        charge worth 69 plates and had made 107 by the time the window opened,
        so it has nothing outstanding and its 11 plates stand alone. The four
        other furnaces' 264 of coal is credit at *their* positions, where it is
        drawn down by *their* 128 plates.
        """
        rows = {tuple(m["position"]): m for m in self.r["per_machine"]}
        belted = rows[(-5.0, -27.0)]
        self.assertAlmostEqual(belted["credit"], 69.43, places=1)
        self.assertEqual(belted["spent"], 107)
        self.assertEqual(belted["outstanding"], 0.0)
        self.assertEqual(belted["made"], 11)
        self.assertEqual(
            sorted(rows),
            sorted([
                (-14.0, -13.0), (-10.0, -17.0), (-10.0, -15.0), (-9.0, -13.0), (-5.0, -27.0),
            ]),
        )
        self.assertEqual(sum(1 for m in self.r["per_machine"] if m["made"]), 1)

    def test_what_is_still_unattributable(self):
        """79 plates from a chest and 47 from two drills, pinned to nothing.

        The chest names a container and no record says which machine an
        inserter fed from it; a drill's coal bounds the plates some furnace
        later makes from its ore. Combined by maximum, not sum -- they are
        alternative explanations of the same plates -- and drawn down by all
        235 plates made before the window, which empties the pool here.
        """
        self.assertAlmostEqual(self.r["floating_credit"], 79.0, places=1)
        self.assertEqual(
            sorted(self.r["floating_stages"]),
            [
                "burner-mining-drill@[-7.0, -27.0]",
                "burner-mining-drill@[16.0, -28.0]",
                "wooden-chest@[-10.5, -13.5]",
            ],
        )
        self.assertEqual(self.r["floating_outstanding"], 0.0)

    def test_every_delivery_was_priced_and_read_from_fields(self):
        """90 structured `delivery` records, and the prose fallback unused.

        An unpriced delivery would have made this `unknown`, which is a
        different refusal from `short` and not a measurement.
        """
        self.assertEqual(self.r["deliveries"], 90)
        self.assertEqual(self.r["credit_source"], ["fields"])
        self.assertEqual(self.r["unreadable"], [])
        self.assertEqual(self.r["unpriced"], [])

    def test_the_fixture_agrees_with_the_live_archive(self):
        """The trimmed fixture and the run it was cut from give one answer."""
        if not os.path.isdir(BELTED_ARCHIVE):
            self.skipTest(f"no live archive at {BELTED_ARCHIVE}")
        samples, events = load(BELTED_ARCHIVE)
        live = ra.hand_credit_balance(
            samples,
            events,
            item=ITEM,
            per_minute=PER_MINUTE,
            window_ticks=self.WINDOW,
            at_tick=self.AT,
        )
        for field in ("verdict", "machine_made", "hand_credit", "unexplained"):
            self.assertEqual(live[field], self.r[field], field)


class ModuleSanityTest(unittest.TestCase):
    """No two top-level functions in `run_analysis` share a name.

    Written because this change shipped a `pos_key` that shadowed an existing
    one 1,600 lines above it. Every test in this file passed -- none of them
    call `frozen_bots` -- and the CLI died on the first run against a real
    archive. A silent redefinition is invisible to a suite that does not
    happen to exercise the loser.
    """

    def test_no_top_level_name_is_defined_twice(self):
        import ast

        tree = ast.parse(open(ra.__file__, encoding="utf-8").read())
        seen: dict[str, int] = {}
        clashes = []
        for node in tree.body:
            if not isinstance(node, (ast.FunctionDef, ast.ClassDef)):
                continue
            if node.name in seen:
                clashes.append(f"{node.name} at lines {seen[node.name]} and {node.lineno}")
            seen[node.name] = node.lineno
        self.assertEqual(clashes, [])


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
