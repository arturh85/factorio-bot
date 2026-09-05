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
        # The plateau now says which kind it is, and with neither machine
        # samples nor events it says the only honest thing.
        self.assertIn(
            "iron-plate plateaus at 12:00 (144) -- production stopped 6.0 min before the run "
            "ended: UNCLEAR -- no machine samples and no bot activity after the plateau",
            lines,
        )
        # Still growing at the end: not a plateau. Never made: not a plateau.
        self.assertIsNone(self.r["plateaus"]["automation-science-pack"])
        self.assertIsNone(self.r["plateaus"]["copper-plate"])

    def test_headline_shape(self):
        # The headline carries the attribution: these samples have no power at
        # all and no events to say who fed anything, so it is `unclear`, and
        # the fact that nothing ever generated is stated outright.
        self.assertTrue(self.r["headline"].startswith(
            "rates: iron 12->12->5 /min at 5/10/15 (unclear; no generator all run); "
            "red packs 0->1->3; green 0 at 15, 0 at end 18:00"),
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


def powered_force_sample(tick: int, made: dict | list, gen: float, con: float) -> dict:
    s = force_sample(tick, made)
    s["power"] = {
        "generated_kw": gen,
        "consumed_kw": con,
        "satisfaction": 1.0 if gen >= con else 0.0,
        "networks": {},
    }
    return s


def machines_sample(tick: int, machines: dict) -> dict:
    return {"schema": 2, "tick": tick, "kind": "machines", "machines": machines, "truncated": 0}


def furnace(status: str, fuel: dict | None = None) -> dict:
    """A stone furnace: a burner, identified as one by its fuel slot."""
    return {
        "name": "stone-furnace",
        "type": "furnace",
        "position": {"x": 0.0, "y": 0.0},
        "status": status,
        "products_finished": 1,
        "input": {},
        "output": {},
        "fuel": fuel if fuel is not None else {"coal": 3},
    }


def assembler(status: str) -> dict:
    """An assembling machine: electric, identified by its network id."""
    return {
        "name": "assembling-machine-1",
        "type": "assembling-machine",
        "position": {"x": 5.0, "y": 5.0},
        "status": status,
        "network": 1,
        "recipe": "iron-gear-wheel",
        "products_finished": 1,
        "input": {},
        "output": {},
        "fuel": {},
    }


def lab(status: str) -> dict:
    """A lab: electric, and NOT a producer -- it makes research, not items."""
    return {
        "name": "lab",
        "type": "lab",
        "position": {"x": 9.0, "y": 9.0},
        "status": status,
        "network": 1,
        "input": {},
        "output": {},
        "fuel": {},
    }


def feeding_events(ticks: list[int], bot: int = 1, verb: str = "insert") -> list[dict]:
    """Dispatch/settle pairs that settle in the tick they dispatch, as the real ones do."""
    out = []
    for i, t in enumerate(ticks):
        out.append({"tick": t, "kind": "action_dispatched", "id": i, "bot": bot,
                    "action": f"{verb} 10 iron-ore into stone-furnace"})
        out.append({"tick": t, "kind": "action_settled", "id": i, "bot": bot,
                    "status": "ok", "elapsed_ticks": 0})
    return out


def other_events(ticks: list[int], bot: int = 1) -> list[dict]:
    out = []
    for i, t in enumerate(ticks):
        out.append({"tick": t, "kind": "action_dispatched", "id": 500 + i, "bot": bot,
                    "action": "craft 1 iron-gear-wheel"})
        out.append({"tick": t + 60, "kind": "action_settled", "id": 500 + i, "bot": bot,
                    "status": "ok", "elapsed_ticks": 60})
    return out


class AttributionTest(unittest.TestCase):
    """Who earned the output: the roster carrying ore, or a factory.

    Each case is one five-minute interval with the same rising curve; only the
    power and what the bots did differ, which is exactly the confusion this
    block exists to end. A rising `production.made` looks identical in all of
    them.
    """

    def rates(self, samples, events, marks=(5,), hi=None):
        joined, _ = ra.join_actions(sorted(events, key=lambda e: e["tick"]))
        act = ra.bot_activity(sorted(events, key=lambda e: e["tick"]), joined, [1])
        return ra.production_rates(
            samples, ORIGIN, hi or (ORIGIN + 5 * TPM), marks=marks, activity=act
        )

    def curve(self, gen=0.0, con=0.0, upto=5):
        return [
            powered_force_sample(ORIGIN + i * 300, {"iron-plate": i * 5}, gen, con)
            for i in range(0, upto * 12 + 1)
        ]

    def test_no_generation_is_named_as_hand_fed(self):
        r = self.rates(self.curve(), feeding_events([ORIGIN + 600 * i for i in range(1, 15)]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "roster-fed")
        self.assertIn("no generator: this output was hand-fed", item["why"])
        self.assertTrue(r["marks"][0]["attribution"]["no_generator"])
        self.assertIsNone(r["first_generation_tick"])
        self.assertIn("no generator all run", r["headline"])
        self.assertIn("roster-fed", r["headline"])

    def test_no_generation_and_no_feeding_is_unclear_not_a_verdict(self):
        r = self.rates(self.curve(), other_events([ORIGIN + 600 * i for i in range(1, 5)]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "unclear")
        self.assertIn("burner", item["why"])

    def test_power_and_an_idle_roster_is_a_factory(self):
        samples = self.curve(gen=900.0, con=150.0)
        samples += [machines_sample(ORIGIN + i * 300, {"1": assembler("working")})
                    for i in range(0, 61)]
        r = self.rates(samples, other_events([ORIGIN + 600]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "factory")
        self.assertIn("electric producers worked", item["why"])
        self.assertEqual(r["first_generation_tick"], ORIGIN)

    def test_power_while_bots_insert_is_unclear(self):
        samples = self.curve(gen=900.0, con=150.0)
        samples += [machines_sample(ORIGIN + i * 300, {"1": assembler("working")})
                    for i in range(0, 61)]
        r = self.rates(samples, feeding_events([ORIGIN + 600 * i for i in range(1, 15)]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "unclear")
        self.assertIn("could be either", item["why"])
        self.assertEqual(r["marks"][0]["attribution"]["roster"]["feed_actions"], 14)

    def test_power_to_a_lab_while_burners_smelt_is_still_roster_fed(self):
        """The peer session's case with the lights on: kW drawn, no electric producer."""
        samples = self.curve(gen=900.0, con=120.0)
        samples += [
            machines_sample(ORIGIN + i * 300, {"1": furnace("working"), "2": lab("working")})
            for i in range(0, 61)
        ]
        r = self.rates(samples, feeding_events([ORIGIN + 600 * i for i in range(1, 15)]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "roster-fed")
        self.assertIn("no ELECTRIC machine that makes items worked", item["why"])
        self.assertIn("lab", item["why"])
        att = r["marks"][0]["attribution"]["machines"]
        self.assertEqual(att["working_electric"], 0)
        self.assertGreater(att["working_burner"], 0)

    def test_feed_tick_percentage_is_near_zero_while_the_count_is_not(self):
        """Five of the six feeding verbs settle in their dispatch tick."""
        r = self.rates(self.curve(), feeding_events([ORIGIN + 600 * i for i in range(1, 15)]))
        act = r["marks"][0]["attribution"]["roster"]
        self.assertEqual(act["feed_actions"], 14)
        self.assertEqual(act["feed_ticks"], 0)
        self.assertEqual(act["feed_pct"], 0.0)

    def test_plateau_because_input_ran_out(self):
        flat = [powered_force_sample(ORIGIN + i * 300, {"iron-plate": min(i, 12) * 5}, 900.0, 120.0)
                for i in range(0, 121)]
        flat += [machines_sample(ORIGIN + i * 300,
                                 {"1": furnace("working" if i <= 12 else "no_ingredients", fuel={})})
                 for i in range(0, 121)]
        r = self.rates(flat, feeding_events([ORIGIN + 300]), marks=(5, 10),
                       hi=ORIGIN + 10 * TPM)
        pl = r["plateaus"]["iron-plate"]
        self.assertEqual(pl["kind"], "input ran out")
        self.assertIn("no_ingredients", pl["why"])
        self.assertIn("INPUT RAN OUT", "".join(ra.plateau_lines(r)))

    def test_plateau_because_the_factory_stopped(self):
        flat = [powered_force_sample(ORIGIN + i * 300, {"iron-plate": min(i, 12) * 5},
                                     900.0 if i <= 12 else 0.0, 120.0)
                for i in range(0, 121)]
        flat += [machines_sample(ORIGIN + i * 300,
                                 {"1": assembler("working" if i <= 12 else "no_power")})
                 for i in range(0, 121)]
        r = self.rates(flat, feeding_events([ORIGIN + 300]), marks=(5, 10),
                       hi=ORIGIN + 10 * TPM)
        pl = r["plateaus"]["iron-plate"]
        self.assertEqual(pl["kind"], "the factory stopped")
        self.assertIn("no_power", pl["why"])

    def test_report_prints_the_attribution_columns(self):
        r = self.rates(self.curve(), feeding_events([ORIGIN + 600 * i for i in range(1, 15)]))
        out = io.StringIO()
        ra.report_rates(r, lambda line="": out.write(line + "\n"))
        text = out.getvalue()
        self.assertIn("feed acts", text)
        self.assertIn("no generator: this output was hand-fed", text)
        self.assertIn("verdict per item and interval", text)


def counted_furnace(produced: int, status: str = "working") -> dict:
    """A stone furnace whose lifetime item count the game supplied."""
    m = furnace(status)
    m["recipe"] = "iron-plate"
    m["produced"] = produced
    m["produced_source"] = "game"
    return m


def counted_drill(produced: int, shared: bool = False) -> dict:
    """A burner drill, counted by the mod because Factorio counts nothing."""
    m = {
        "name": "burner-mining-drill",
        "type": "mining-drill",
        "position": {"x": 3.0, "y": 3.0},
        "status": "working",
        "mining": "iron-ore",
        "produced": produced,
        "produced_source": "accumulated",
        "input": {},
        "output": {},
        "fuel": {"coal": 2},
    }
    if shared:
        m["produced_shared"] = True
    return m


class CounterAttributionTest(unittest.TestCase):
    """The split by arithmetic: what each machine counted, against the force total.

    Hand crafting and hand mining pass through no machine, so the difference
    between "what the force made" and "what the machines say they made" is the
    roster's own hands. Before the counters existed this was inferred from
    feeding-verb counts and machine statuses and was often honestly
    ``unclear``; these cases are the ones the inference could not settle.
    """

    def rates(self, samples, events=(), marks=(5,), hi=None):
        events = sorted(events, key=lambda e: e["tick"])
        joined, _ = ra.join_actions(events)
        act = ra.bot_activity(events, joined, [1]) if events else None
        return ra.production_rates(
            samples, ORIGIN, hi or (ORIGIN + 5 * TPM), marks=marks, activity=act
        )

    def curve(self, per_beat=5, gen=0.0, con=0.0, upto=5):
        return [
            powered_force_sample(ORIGIN + i * 300, {"iron-plate": i * per_beat}, gen, con)
            for i in range(0, upto * 12 + 1)
        ]

    def machines(self, rows_at):
        """``rows_at(i) -> machines dict`` on the same 300-tick beat."""
        return [machines_sample(ORIGIN + i * 300, rows_at(i)) for i in range(0, 61)]

    def test_machines_made_all_of_it_and_nobody_fed_them(self):
        samples = self.curve(gen=900.0, con=150.0)
        samples += self.machines(lambda i: {"1": counted_furnace(i * 5)})
        r = self.rates(samples, other_events([ORIGIN + 600]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "factory")
        self.assertEqual(item["source"], "counters")
        self.assertEqual(item["machine_made"], 300)
        self.assertEqual(item["roster_made"], 0)
        self.assertIn("machines made 300 of the 300", item["why"])

    def test_machines_made_it_while_bots_carried_the_inputs(self):
        """The case the inference calls `unclear`: power drawn AND bots feeding."""
        samples = self.curve(gen=900.0, con=150.0)
        samples += self.machines(lambda i: {"1": counted_furnace(i * 5)})
        r = self.rates(samples, feeding_events([ORIGIN + 600 * i for i in range(1, 15)]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "roster-fed")
        self.assertEqual(item["machine_made"], 300)
        self.assertIn("the bots carried what went in", item["why"])

    def test_no_machine_made_any_of_it_is_hand_made(self):
        """A furnace stood there all run and produced nothing; the plates are hand crafts."""
        samples = self.curve(gen=900.0, con=150.0)
        samples += self.machines(lambda i: {"1": counted_furnace(0, status="no_fuel")})
        r = self.rates(samples, other_events([ORIGIN + 600]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "hand-made")
        self.assertEqual(item["machine_made"], 0)
        self.assertEqual(item["roster_made"], 300)
        self.assertIn("no machine produced any", item["why"])

    def test_both_at_once_is_mixed_not_unclear(self):
        samples = self.curve(gen=900.0, con=150.0)
        samples += self.machines(lambda i: {"1": counted_furnace(i * 2)})
        r = self.rates(samples, feeding_events([ORIGIN + 600]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["verdict"], "mixed")
        self.assertEqual(item["machine_made"], 120)
        self.assertEqual(item["roster_made"], 180)
        self.assertIn("the remaining 180 was hand-made", item["why"])

    def test_a_drill_is_attributed_to_the_ore_it_mines(self):
        samples = [
            powered_force_sample(ORIGIN + i * 300, {"iron-ore": i * 4, "iron-plate": i * 5}, 0.0, 0.0)
            for i in range(0, 61)
        ]
        samples += self.machines(
            lambda i: {"1": counted_furnace(i * 5), "2": counted_drill(i * 4)}
        )
        r = self.rates(samples, other_events([ORIGIN + 600]))
        ore = r["marks"][0]["items"]["iron-ore"]
        self.assertEqual(ore["machine_made"], 240)
        self.assertEqual(ore["verdict"], "factory")
        prod = r["marks"][0]["attribution"]["produced"]
        self.assertEqual(prod["by_item"], {"iron-plate": 300, "iron-ore": 240})
        self.assertEqual(prod["total"], 540)

    def test_two_drills_on_one_tile_are_reported_as_a_contradiction(self):
        """Both saw the same fall, so their sum exceeds what the force made."""
        samples = [
            powered_force_sample(ORIGIN + i * 300, {"iron-ore": i * 4}, 0.0, 0.0)
            for i in range(0, 61)
        ]
        samples += self.machines(lambda i: {
            "1": counted_drill(i * 4, shared=True),
            "2": counted_drill(i * 4, shared=True),
        })
        r = self.rates(samples, other_events([ORIGIN + 600]))
        ore = r["marks"][0]["items"]["iron-ore"]
        self.assertEqual(ore["verdict"], "unclear")
        self.assertIn("cannot both be right", ore["why"])
        self.assertIn("burner-mining-drill", ore["why"])

    def test_an_uncountable_producer_is_named_not_counted_as_zero(self):
        samples = self.curve()
        pump = {
            "name": "pumpjack", "type": "mining-drill", "position": {"x": 1.0, "y": 1.0},
            "status": "working", "mining": "crude-oil", "produced_source": "unavailable",
            "input": {}, "output": {}, "fuel": {},
        }
        samples += self.machines(lambda i: {"1": counted_furnace(i * 5), "2": dict(pump)})
        r = self.rates(samples, other_events([ORIGIN + 600]))
        prod = r["marks"][0]["attribution"]["produced"]
        self.assertEqual(prod["unavailable"], {"pumpjack": 1})
        self.assertEqual(prod["by_item"], {"iron-plate": 300})

    def test_an_idle_furnace_names_no_recipe_and_is_still_attributed(self):
        """Measured on `run-1788638239-43349`, not imagined.

        `get_recipe()` on a stone furnace with an empty input answers nil, so
        the last machine sample of a run whose furnaces have gone quiet names
        no item at all. Reading only that row filed all 859 items of that
        run's furnace output under "unattributed".
        """
        samples = self.curve(gen=900.0, con=150.0)

        def rows(i):
            m = counted_furnace(i * 5, "working" if i < 40 else "no_ingredients")
            if i >= 40:
                m.pop("recipe")  # the game answers nil, so the mod writes no key
            return {"1": m}

        samples += self.machines(rows)
        r = self.rates(samples, other_events([ORIGIN + 600]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["machine_made"], 300)
        self.assertEqual(item["verdict"], "factory")
        self.assertEqual(r["marks"][0]["attribution"]["produced"]["unattributed"], 0)

    def test_a_run_without_counters_still_gets_the_old_inference(self):
        samples = self.curve(gen=900.0, con=150.0)
        samples += self.machines(lambda i: {"1": assembler("working")})
        r = self.rates(samples, other_events([ORIGIN + 600]))
        item = r["marks"][0]["items"]["iron-plate"]
        self.assertEqual(item["source"], "inference")
        self.assertEqual(item["verdict"], "factory")
        self.assertNotIn("machine_made", item)
        self.assertFalse(r["marks"][0]["attribution"]["produced"]["available"])

    def test_plateau_says_the_machines_stopped(self):
        flat = [powered_force_sample(ORIGIN + i * 300, {"iron-plate": min(i, 12) * 5}, 900.0, 120.0)
                for i in range(0, 121)]
        flat += [machines_sample(ORIGIN + i * 300,
                                 {"1": counted_furnace(min(i, 12) * 5,
                                                       "working" if i <= 12 else "no_fuel")})
                 for i in range(0, 121)]
        r = self.rates(flat, feeding_events([ORIGIN + 300]), marks=(5, 10), hi=ORIGIN + 10 * TPM)
        pl = r["plateaus"]["iron-plate"]
        self.assertEqual(pl["kind"], "the machines stopped")
        self.assertEqual(pl["source"], "counters")
        self.assertIn("not one machine produced a single item", pl["why"])

    def test_plateau_says_the_machines_kept_working_on_something_else(self):
        """Iron stops; the drills keep mining ore. Not the same plateau at all."""
        flat = [powered_force_sample(ORIGIN + i * 300,
                                     {"iron-plate": min(i, 12) * 5, "iron-ore": i * 4}, 900.0, 120.0)
                for i in range(0, 121)]
        flat += [machines_sample(ORIGIN + i * 300, {
            "1": counted_furnace(min(i, 12) * 5, "working" if i <= 12 else "no_ingredients"),
            "2": counted_drill(i * 4),
        }) for i in range(0, 121)]
        r = self.rates(flat, feeding_events([ORIGIN + 300]), marks=(5, 10), hi=ORIGIN + 10 * TPM)
        pl = r["plateaus"]["iron-plate"]
        self.assertEqual(pl["kind"], "the machines kept working, but none made this item")
        self.assertIn("none of them was iron-plate", pl["why"])

    def test_report_prints_the_per_machine_block(self):
        samples = self.curve(gen=900.0, con=150.0)
        samples += self.machines(lambda i: {"1": counted_furnace(i * 5), "2": counted_drill(i * 2)})
        r = self.rates(samples, feeding_events([ORIGIN + 600]))
        out = io.StringIO()
        ra.report_rates(r, lambda line="": out.write(line + "\n"))
        text = out.getvalue()
        self.assertIn("per-machine production", text)
        self.assertIn("stone-furnace 300", text)
        self.assertIn("counter arithmetic, not inference", text)


class MachineLifetimeTest(unittest.TestCase):
    """The owner's number, per machine, over the whole run."""

    def test_lifetime_counts_are_kept_per_machine_and_summarised(self):
        samples = [machines_sample(ORIGIN + i * 300, {
            "1": counted_furnace(i * 5),
            "2": counted_drill(i * 2),
            "3": {**lab("working"), "produced_source": "not-a-producer"},
        }) for i in range(0, 13)]
        mach = ra.machines_at(samples)
        by_key = {m["key"]: m for m in mach["machines"]}
        self.assertEqual(by_key["1"]["produced"], 60)
        self.assertEqual(by_key["1"]["produced_source"], "game")
        self.assertEqual(by_key["2"]["produced"], 24)
        self.assertEqual(by_key["2"]["produced_source"], "accumulated")
        # A drill is rarely caught `working` and has no craft counter; its
        # accumulated count is the only evidence it ever mined anything.
        self.assertTrue(by_key["2"]["worked"])
        # A lab makes research, not items: no count, and it says why.
        self.assertEqual(by_key["3"]["produced"], 0)
        self.assertEqual(by_key["3"]["produced_source"], "not-a-producer")


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
