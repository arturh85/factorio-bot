#!/usr/bin/env python3
"""Where did a run's time go?

Reads one archived run directory (``workspace/runs/run-<unix>-<pid>/``) and
reports the time accounting: milestone spans, action cost by verb, per-bot
utilisation, walk failures, what was built, and which bots stopped moving.

    python3 tools/run_analysis.py workspace/runs/run-1788449752-46541
    python3 tools/run_analysis.py --all --summary
    python3 tools/run_analysis.py --json workspace/runs/run-1788449752-46541

Stdlib only, and read-only: it never writes into the run directory, so it is
safe to point at a run that is still being written. A truncated final line is
expected in that case and is counted rather than fatal.

WHY THIS IS A COMMITTED TOOL
----------------------------
The numbers below were first computed by a throwaway one-liner, and the
one-liner got two of them wrong in ways that changed what the session believed.
Every trap it fell into is encoded here as behaviour, and named in a comment at
the place that handles it. In particular:

* ``walk_dispatched`` HAS NO ``id`` FIELD. Joining walks on ``id`` gives every
  walk the key ``None``; in Python that is a perfectly good dict key, so the
  join silently collapses every walk in the run onto one entry and attributes
  every failure to whichever bot happened to be last. ``walk_settled`` carries
  ``bot`` directly -- that is where per-bot walk numbers come from here, with no
  join at all. See ``EventKind::WalkDispatched`` in
  ``crates/core/src/record/mod.rs``: the only identity a walk has is
  ``(bot, step_index)``, and that index is neither an ``ActionId`` nor an index
  into ``plan_created``'s ``plan``.

* ACTION IDS RESTART AT 0 WITH EVERY PLAN. A dict keyed on action id across a
  whole run can therefore mis-join a settle from plan N onto a dispatch from
  plan N-1. Joins here are scoped to a plan epoch (see ``join_actions``), and
  the tool reports how many settles it could not match rather than guessing.

* ``failure.kind`` IS NOT COMPLETE FOR OLDER RUNS. Wordings the classifier of
  the day did not know were archived as ``other`` (fixed in ``1f498593``). This
  tool classifies the ``error`` text independently and reports the two side by
  side, so a run recorded before the fix still yields grouped failures.

* MOST VERBS RECORD ZERO DURATION. ``place``/``insert``/``take``/``fuel`` settle
  in the same tick they dispatch, so a "% of action time" figure describes only
  the four verbs the game actually times. The report says so in place rather
  than letting a 90%-hand-mining headline stand unqualified.

* ``samples.jsonl`` DOES carry per-bot ``inventory`` and ``position``, and
  ``map.jsonl`` DOES carry placement keyframes. Both were twice claimed absent.
"""

from __future__ import annotations

import argparse
import collections
import dataclasses
import json
import os
import re
import sys
import time
from typing import Any

TICKS_PER_SECOND = 60
TICKS_PER_MINUTE = 60 * TICKS_PER_SECOND

# Files a complete run leaves behind. Older runs predate some of them and
# interrupted runs never wrote the rest; every one is optional here and its
# absence is reported, never fatal.
KNOWN_FILES = ("events.jsonl", "manifest.json", "splits.json", "samples.jsonl", "map.jsonl")

# How long a bot's position must stay byte-identical before it is worth
# reporting. 3000 ticks is 50 seconds of game time -- far longer than any walk
# or mine, and short enough to catch a bot that froze near the end.
DEFAULT_FREEZE_TICKS = 3000


# --------------------------------------------------------------------------
# loading
# --------------------------------------------------------------------------


@dataclasses.dataclass
class Loaded:
    """The rows of one JSONL file, plus what went wrong reading it."""

    rows: list[dict]
    present: bool
    bad_lines: int = 0
    truncated_tail: bool = False


def load_jsonl(path: str) -> Loaded:
    if not os.path.exists(path):
        return Loaded([], present=False)
    rows: list[dict] = []
    bad = 0
    truncated = False
    with open(path, encoding="utf-8", errors="replace") as fh:
        lines = fh.readlines()
    for i, line in enumerate(lines):
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            # A run still being written ends mid-line. That is expected and is
            # not corruption; anything earlier in the file is.
            if i == len(lines) - 1:
                truncated = True
            else:
                bad += 1
            continue
        if isinstance(obj, dict):
            rows.append(obj)
        else:
            bad += 1
    return Loaded(rows, present=True, bad_lines=bad, truncated_tail=truncated)


def load_json(path: str) -> Any | None:
    if not os.path.exists(path):
        return None
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (json.JSONDecodeError, OSError):
        return None


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------


def minutes(ticks: float | None) -> str:
    if ticks is None:
        return "  n/a"
    return f"{ticks / TICKS_PER_MINUTE:5.1f}m"


def verb_of(action: str) -> str:
    """The first word of a plan's label for a step.

    ``mine 5 stone`` -> ``mine``. Deliberately crude: the labels come from
    ``ActionKind``'s Display in ``crates/planner/src/action.rs`` and the first
    token is the only part guaranteed to be the verb.
    """
    return action.split(" ", 1)[0] if action else "?"


def subject_of(action: str) -> str:
    """What a step acted on, for the second-level breakdown.

    ``mine 5 stone`` -> ``stone``; ``craft 1 stone-furnace`` -> ``stone-furnace``;
    ``place stone-furnace at [-58, 13]`` -> ``stone-furnace``. Coordinates are
    stripped so 33 furnaces at 33 sites group into one row -- which is the
    entire point, because that grouping is what makes over-building visible.
    """
    a = re.sub(r"\s+at \[.*$", "", action).strip()
    parts = a.split()
    if len(parts) < 2:
        return a
    rest = parts[1:]
    if rest and rest[0].isdigit():
        rest = rest[1:]
    # `take 3 iron-plate from the furnace` -> `iron-plate`
    if "from" in rest:
        rest = rest[: rest.index("from")]
    return " ".join(rest) or a


def pos_key(p: Any) -> str:
    """A stable string for a position, used for identity comparisons.

    ``sort_keys`` so two dicts that differ only in key order compare equal --
    the "byte-identical position" test is about the numbers, and the recorder's
    key order is not something a reader should depend on.
    """
    return json.dumps(p, sort_keys=True)


# The classifier the *reader* applies to a walk's `error` text, independent of
# whatever `failure.kind` the writing build managed to produce. Ordered: first
# match wins. Wordings come from `FactorioRcon::player_path_attempt` and from
# BotBridge's own (now removed) re-path, both quoted in `WalkFailureKind`.
WALK_TEXT_RULES = (
    ("no_path", re.compile(r"failed to path find|returned no path", re.I)),
    ("pathfinder_busy", re.compile(r"try again later|refused a re-path", re.I)),
    ("repath_limit", re.compile(r"re-?path limit", re.I)),
    ("stalled", re.compile(r"made no progress|aborted before reaching", re.I)),
    ("timeout", re.compile(r"timed? ?out|no reply", re.I)),
)

ACTION_TEXT_RULES = (
    ("blocked", re.compile(r"another character is standing|is in the way|blocked", re.I)),
    ("missing_item", re.compile(r"not enough|missing|does not have", re.I)),
    ("unreachable", re.compile(r"too far|out of reach|unreachable", re.I)),
    ("partial_transfer", re.compile(r"moved \d+ of \d+", re.I)),
    ("timeout", re.compile(r"timed? ?out|for \d+ ticks", re.I)),
    ("rejected", re.compile(r"game rejected|Unexpected Response", re.I)),
)


def classify(text: str | None, rules) -> str:
    if not text:
        return "unclassified"
    for name, pattern in rules:
        if pattern.search(text):
            return name
    return "unclassified"


# --------------------------------------------------------------------------
# the analysis
# --------------------------------------------------------------------------


@dataclasses.dataclass
class Window:
    """A tick range to score, with a label. The whole run is one of these.

    An unfinished window is closed at the last tick the record reached rather
    than left open, so a live or interrupted run still gets a utilisation
    figure. ``open`` says the number is "so far" and the label says so too --
    the alternative, scoring nothing, is how a killed run becomes unreadable.
    """

    label: str
    lo: int
    hi: int
    outcome: str
    open: bool = False

    def contains(self, tick: int) -> bool:
        return self.lo <= tick <= self.hi

    @property
    def span(self) -> int:
        return self.hi - self.lo


def milestone_windows(events: list[dict], last_tick: int) -> list[Window]:
    """Pair each ``milestone_started`` with its own ``milestone_satisfied``/``_stuck``.

    Paired by ``index``, not by adjacency: a milestone can be re-entered, and
    "the next terminal event" is not necessarily this milestone's.
    """
    open_by_index: dict[int, dict] = {}
    out: list[Window] = []
    order: list[int] = []
    for e in events:
        kind = e.get("kind")
        if kind == "milestone_started":
            idx = e.get("index")
            open_by_index[idx] = e
            order.append(idx)
        elif kind in ("milestone_satisfied", "milestone_stuck"):
            idx = e.get("index")
            start = open_by_index.pop(idx, None)
            if start is None:
                continue
            if kind == "milestone_satisfied":
                # `reason` describes the *final* check, not the whole milestone:
                # a run that worked for 27 minutes and then found the goal met
                # records `already_satisfied` with `iterations: 3`. Print both,
                # so the reason is never read as "this took no time".
                outcome = (
                    f"satisfied reason={e.get('reason', 'unknown')} "
                    f"iters={e.get('iterations', '?')}"
                )
            else:
                outcome = f"stuck best_steps={e.get('best_steps')}"
            out.append(
                Window(f"m{idx} {start.get('goal', '')}", start["tick"], e["tick"], outcome)
            )
    for idx, start in open_by_index.items():
        out.append(
            Window(
                f"m{idx} {start.get('goal', '')}",
                start["tick"],
                last_tick,
                "OPEN (never ended; scored to the last recorded tick)",
                open=True,
            )
        )
    out.sort(key=lambda w: w.lo)
    return out


def join_actions(events: list[dict]) -> tuple[list[dict], dict[str, int]]:
    """Match each ``action_settled`` to the ``action_dispatched`` that produced it.

    THE JOIN IS SCOPED TO A PLAN EPOCH. ``ActionId`` restarts at 0 with every
    plan (see ``EventKind::Teleport``'s comment in
    ``crates/core/src/record/mod.rs``), so a run-global dict keyed on id can
    match a settle to a dispatch from a previous plan. Every ``plan_created``
    opens a new epoch here; ids are only ever matched within one.

    Entries are popped on match, so an id reused *within* one epoch still
    matches its own dispatch in order rather than the first one seen.

    Returns joined rows plus a count of what did not join. A settle with no
    dispatch is legal and is a finding, not a bug -- the record documents it --
    so it is counted and surfaced rather than dropped silently.
    """
    epoch = 0
    pending: dict[tuple[int, int], dict] = {}
    joined: list[dict] = []
    stats = {"orphan_settles": 0, "never_settled": 0, "derived_duration": 0, "null_duration": 0}
    for e in events:
        kind = e.get("kind")
        if kind == "plan_created":
            epoch += 1
        elif kind == "action_dispatched":
            pending[(epoch, e.get("id"))] = e
        elif kind == "action_settled":
            disp = pending.pop((epoch, e.get("id")), None)
            if disp is None:
                stats["orphan_settles"] += 1
                joined.append(
                    {
                        "tick": e["tick"],
                        "bot": e.get("bot"),
                        "action": None,
                        "verb": "?unjoined",
                        "subject": "?",
                        "status": e.get("status"),
                        "ticks": e.get("elapsed_ticks") or 0,
                        "error": e.get("error"),
                        "failure_kind": (e.get("failure") or {}).get("kind"),
                    }
                )
                continue
            elapsed = e.get("elapsed_ticks")
            if elapsed is None:
                # A duration nobody measured is not a duration of zero. Fall
                # back to the tick difference and say how often we had to.
                stats["null_duration"] += 1
                elapsed = max(0, e["tick"] - disp["tick"])
                if elapsed:
                    stats["derived_duration"] += 1
            action = disp.get("action", "")
            joined.append(
                {
                    "tick": e["tick"],
                    "start": disp["tick"],
                    "bot": e.get("bot", disp.get("bot")),
                    "action": action,
                    "verb": verb_of(action),
                    "subject": subject_of(action),
                    "status": e.get("status"),
                    "ticks": elapsed,
                    "error": e.get("error"),
                    "failure_kind": (e.get("failure") or {}).get("kind"),
                }
            )
    stats["never_settled"] = len(pending)
    return joined, stats


def analyse(run_dir: str, freeze_ticks: int = DEFAULT_FREEZE_TICKS) -> dict:
    run_id = os.path.basename(os.path.normpath(run_dir))
    result: dict[str, Any] = {"run_id": run_id, "dir": os.path.abspath(run_dir)}

    present = {name: os.path.exists(os.path.join(run_dir, name)) for name in KNOWN_FILES}
    result["missing_files"] = [n for n, ok in present.items() if not ok]

    ev = load_jsonl(os.path.join(run_dir, "events.jsonl"))
    result["events_read"] = len(ev.rows)
    result["events_bad_lines"] = ev.bad_lines
    result["events_truncated_tail"] = ev.truncated_tail
    if not ev.present:
        result["error"] = "no events.jsonl -- nothing to account for"
        return result

    events = sorted(ev.rows, key=lambda e: e.get("tick", 0))
    result["event_counts"] = dict(
        collections.Counter(e.get("kind", "?") for e in events).most_common()
    )

    manifest = load_json(os.path.join(run_dir, "manifest.json"))
    result["manifest"] = manifest
    started = manifest.get("started_unix") if manifest else None
    finished = manifest.get("finished_unix") if manifest else None
    if started is None:
        m = re.match(r"run-(\d+)-", run_id)
        started = int(m.group(1)) if m else None
    result["started_unix"] = started
    result["finished_unix"] = finished
    result["wall_seconds"] = (finished - started) if (started and finished) else None
    if started and not finished:
        # Live or interrupted: the newest file mtime is the best "last seen".
        mtimes = [
            os.path.getmtime(os.path.join(run_dir, n)) for n, ok in present.items() if ok
        ]
        result["last_seen_unix"] = int(max(mtimes)) if mtimes else None
        result["wall_seconds_so_far"] = (
            int(max(mtimes)) - started if mtimes else None
        )

    run_started = next((e for e in events if e.get("kind") == "run_started"), None)
    run_finished = next((e for e in events if e.get("kind") == "run_finished"), None)
    lo = run_started["tick"] if run_started else (events[0]["tick"] if events else 0)
    hi = run_finished["tick"] if run_finished else (events[-1]["tick"] if events else 0)
    result["roster"] = (run_started or {}).get("bots")
    result["outcome"] = (run_finished or {}).get("outcome", "OPEN (no run_finished)")
    result["tick_lo"] = lo
    result["tick_hi"] = hi
    result["span_ticks"] = hi - lo

    windows = [Window("whole run", lo, hi, result["outcome"], open=run_finished is None)]
    windows += milestone_windows(events, hi)
    result["milestones"] = [
        {
            "label": w.label,
            "lo": w.lo,
            "hi": w.hi,
            "span_ticks": w.span,
            "outcome": w.outcome,
            "open": w.open,
        }
        for w in windows[1:]
    ]

    # splits.json is derived from the same events; if it disagrees, the record
    # is inconsistent and that is worth knowing.
    splits = load_json(os.path.join(run_dir, "splits.json"))
    if isinstance(splits, list):
        disagreements = []
        by_index = {s.get("index"): s for s in splits if isinstance(s, dict)}
        for w in windows[1:]:
            m = re.match(r"m(\d+)", w.label)
            if not m or w.open:
                continue
            s = by_index.get(int(m.group(1)))
            if s and s.get("elapsed_ticks") not in (None, w.span):
                disagreements.append(
                    {"index": int(m.group(1)), "splits": s.get("elapsed_ticks"), "events": w.span}
                )
        result["splits_disagreements"] = disagreements

    joined, join_stats = join_actions(events)
    result["join"] = join_stats

    result["plans"] = []
    for e in events:
        if e.get("kind") != "plan_created":
            continue
        per_bot_steps = collections.Counter()
        per_bot_work = collections.Counter()
        verbs = collections.Counter()
        for s in e.get("plan", []) or []:
            per_bot_steps[s.get("bot")] += 1
            per_bot_work[s.get("bot")] += s.get("planned_duration", 0)
            verbs[verb_of(s.get("action", ""))] += 1
        result["plans"].append(
            {
                "tick": e["tick"],
                "milestone": e.get("milestone_index"),
                "steps": e.get("steps"),
                "makespan": e.get("makespan"),
                "roster": e.get("bots"),
                "steps_per_bot": dict(sorted(per_bot_steps.items(), key=lambda kv: (kv[0] is None, kv[0]))),
                "planned_work_per_bot": dict(sorted(per_bot_work.items(), key=lambda kv: (kv[0] is None, kv[0]))),
                "planned_verbs": dict(verbs.most_common()),
            }
        )

    placed = load_jsonl(os.path.join(run_dir, "map.jsonl"))
    result["map_present"] = placed.present

    samples = load_jsonl(os.path.join(run_dir, "samples.jsonl"))
    result["samples_present"] = samples.present

    result["windows"] = [score_window(w, events, joined, placed.rows) for w in windows]
    if samples.present:
        result["frozen"] = frozen_bots(samples.rows, freeze_ticks, placed.rows)
        result["production"] = production_at(samples.rows, windows)
    else:
        result["frozen"] = None
        result["production"] = None
    return result


def score_window(w: Window, events: list[dict], joined: list[dict], map_rows: list[dict]) -> dict:
    """Everything time-related, for one tick window."""
    span = w.span
    verbs = collections.Counter()
    verb_ticks = collections.Counter()
    subjects = collections.Counter()
    subject_ticks = collections.Counter()
    action_ticks_by_bot = collections.Counter()
    dispatches_by_bot = collections.Counter()
    settles_by_bot = collections.Counter()
    status_counts = collections.Counter()
    action_failures = collections.Counter()

    for e in events:
        if not w.contains(e.get("tick", 0)):
            continue
        if e.get("kind") == "action_dispatched":
            dispatches_by_bot[e.get("bot")] += 1

    for j in joined:
        if not w.contains(j["tick"]):
            continue
        verbs[j["verb"]] += 1
        verb_ticks[j["verb"]] += j["ticks"]
        key = f"{j['verb']} {j['subject']}"
        subjects[key] += 1
        subject_ticks[key] += j["ticks"]
        action_ticks_by_bot[j["bot"]] += j["ticks"]
        settles_by_bot[j["bot"]] += 1
        status_counts[j["status"]] += 1
        if j["status"] != "success":
            recorded = j["failure_kind"] or "none"
            derived = classify(j["error"], ACTION_TEXT_RULES)
            action_failures[(recorded, derived)] += 1

    # ------------------------------------------------------------------
    # WALKS. Read `bot` straight off `walk_settled`; DO NOT join to
    # `walk_dispatched`, which has no `id` and whose only identity is
    # `(bot, step_index)`. Joining on a field that is not there gives every
    # walk the key None and silently attributes every failure to one bot.
    # ------------------------------------------------------------------
    walk_ticks_by_bot = collections.Counter()
    walks_by_bot = collections.Counter()
    walk_fail_by_bot = collections.Counter()
    walk_lost_by_bot = collections.Counter()
    walk_failure_kinds = collections.Counter()
    repeat_failures = collections.Counter()
    walk_dispatched = 0
    for e in events:
        if not w.contains(e.get("tick", 0)):
            continue
        kind = e.get("kind")
        if kind == "walk_dispatched":
            walk_dispatched += 1
            continue
        if kind != "walk_settled":
            continue
        bot = e.get("bot")
        walks_by_bot[bot] += 1
        walk_ticks_by_bot[bot] += e.get("elapsed_ticks") or 0
        status = e.get("status")
        if status == "success":
            continue
        if status == "lost":
            walk_lost_by_bot[bot] += 1
        else:
            walk_fail_by_bot[bot] += 1
        recorded = (e.get("failure") or {}).get("kind") or "none"
        derived = classify(e.get("error"), WALK_TEXT_RULES)
        walk_failure_kinds[(recorded, derived)] += 1
        to = e.get("to") or {}
        repeat_failures[(bot, round(to.get("x", 0), 2), round(to.get("y", 0), 2))] += 1

    bots = sorted(
        {b for b in list(dispatches_by_bot) + list(walks_by_bot) + list(action_ticks_by_bot) if b is not None}
    )
    per_bot = {}
    for b in bots:
        busy = action_ticks_by_bot[b] + walk_ticks_by_bot[b]
        per_bot[b] = {
            "dispatches": dispatches_by_bot[b],
            "settles": settles_by_bot[b],
            "action_ticks": action_ticks_by_bot[b],
            "walks": walks_by_bot[b],
            "walk_ticks": walk_ticks_by_bot[b],
            "walk_failed": walk_fail_by_bot[b],
            "walk_lost": walk_lost_by_bot[b],
            "busy_ticks": busy,
            "busy_pct": (100.0 * busy / span) if span else None,
        }

    placements = collections.Counter()
    for m in map_rows:
        if m.get("kind") != "placed" or not w.contains(m.get("tick", 0)):
            continue
        name = ((m.get("actual") or m.get("intent") or {}).get("name")) or "?"
        placements[name] += 1

    measured = sum(verb_ticks.values())
    timed_verbs = sorted(v for v, t in verb_ticks.items() if t > 0)
    untimed_verbs = sorted(v for v, t in verb_ticks.items() if t == 0)

    return {
        "label": w.label,
        "lo": w.lo,
        "hi": w.hi,
        "span_ticks": span,
        "outcome": w.outcome,
        "verbs": dict(verbs.most_common()),
        "verb_ticks": dict(verb_ticks.most_common()),
        "timed_verbs": timed_verbs,
        "untimed_verbs": untimed_verbs,
        "measured_action_ticks": measured,
        "measured_pct_of_span": (100.0 * measured / span) if span else None,
        "subject_ticks": dict(subject_ticks.most_common(20)),
        "subject_counts": {k: subjects[k] for k, _ in subject_ticks.most_common(20)},
        "statuses": dict(status_counts.most_common()),
        "action_failures": [
            {"recorded_kind": r, "derived_from_text": d, "count": c}
            for (r, d), c in action_failures.most_common()
        ],
        "per_bot": per_bot,
        "walks_dispatched": walk_dispatched,
        "walk_failure_kinds": [
            {"recorded_kind": r, "derived_from_text": d, "count": c}
            for (r, d), c in walk_failure_kinds.most_common()
        ],
        "repeated_walk_failures": [
            {"bot": b, "to": [x, y], "count": c}
            for (b, x, y), c in repeat_failures.most_common()
            if c >= 2
        ],
        "placements": dict(placements.most_common()),
    }


ENCLOSURE_RADIUS = 4.0


def frozen_bots(samples: list[dict], threshold: int, map_rows: list[dict]) -> list[dict]:
    """Stretches over which a bot's position never changed at all.

    The signal that found a walled-in bot in ``run-1788432181-42528``: two bots
    reported byte-identical positions for 77% of the run while every other
    artefact looked like an ordinary scheduling quirk. ``walk_settled`` cannot
    say this -- ``no_path`` is about a destination, and nineteen of them in a
    row still do not say the bot could reach nothing.

    Compared on the serialised position rather than on floats, because "did
    this value change" is the question, not "did it change much".

    A frozen bot has two very different causes and this cannot tell them apart
    on its own: a bot with nothing to do stands still legitimately. So each
    stretch is annotated with what the run *built around it* -- entities within
    ``ENCLOSURE_RADIUS`` tiles from ``map.jsonl`` -- split by whether they went
    up before the bot stopped moving or after. Neighbours placed at or just
    after the freeze point are the enclosure case; none at all is the idle
    case. Neither is asserted here; both are shown.
    """
    placements = [
        {
            "tick": m.get("tick", 0),
            "name": ((m.get("actual") or m.get("intent") or {}).get("name")) or "?",
            "pos": (m.get("actual") or m.get("intent") or {}).get("position") or {},
        }
        for m in map_rows
        if m.get("kind") == "placed"
    ]

    def neighbours(pos: dict | None, freeze_tick: int) -> dict:
        if not pos or not placements:
            return {}
        before, after = collections.Counter(), collections.Counter()
        first_after = None
        for pl in placements:
            q = pl["pos"]
            if not q:
                continue
            dx = (q.get("x", 0) - pos.get("x", 0))
            dy = (q.get("y", 0) - pos.get("y", 0))
            if (dx * dx + dy * dy) ** 0.5 > ENCLOSURE_RADIUS:
                continue
            if pl["tick"] <= freeze_tick:
                before[pl["name"]] += 1
            else:
                after[pl["name"]] += 1
                if first_after is None or pl["tick"] < first_after:
                    first_after = pl["tick"]
        return {
            "placed_before_freeze": dict(before.most_common()),
            "placed_after_freeze": dict(after.most_common()),
            "first_placement_after_freeze": first_after,
        }

    runs: dict[int, dict] = {}
    out: list[dict] = []
    last_tick = 0

    def emit(bot: int, cur: dict, reached_end: bool) -> None:
        if (cur["hi"] - cur["lo"]) < threshold:
            return
        pos = json.loads(cur["key"]) if cur["key"] != "null" else None
        entry = {
            "bot": bot,
            "from_tick": cur["lo"],
            "to_tick": cur["hi"],
            "ticks": cur["hi"] - cur["lo"],
            "position": pos,
            "reached_end": reached_end,
        }
        entry.update(neighbours(pos, cur["lo"]))
        out.append(entry)

    for s in samples:
        if s.get("kind") != "bots":
            continue
        tick = s.get("tick", 0)
        last_tick = max(last_tick, tick)
        for b in s.get("bots", []) or []:
            bot = b.get("id")
            key = pos_key(b.get("position"))
            cur = runs.get(bot)
            if cur is None or cur["key"] != key:
                if cur is not None:
                    emit(bot, cur, False)
                runs[bot] = {"key": key, "lo": tick, "hi": tick}
            else:
                cur["hi"] = tick
    for bot, cur in runs.items():
        emit(bot, cur, cur["hi"] >= last_tick)
    out.sort(key=lambda r: (-r["ticks"], r["bot"]))
    return out


def production_at(samples: list[dict], windows: list[Window]) -> list[dict]:
    """The force's cumulative made/consumed totals at the end of each window.

    Cumulative, so a per-window figure is the difference from the previous
    window's end. Reported because it is the cheapest check on over-building:
    the item counts here are what the mining totals were spent on.
    """
    force = [s for s in samples if s.get("kind") == "force"]
    if not force:
        return []
    out = []
    prev_made: collections.Counter = collections.Counter()
    for w in windows:
        end = w.hi if w.hi is not None else 10**18
        latest = None
        for s in force:
            if s.get("tick", 0) <= end:
                latest = s
            else:
                break
        if latest is None:
            continue
        made = collections.Counter((latest.get("production") or {}).get("made") or {})
        delta = made - prev_made if w.label != "whole run" else made
        entry = {
            "label": w.label,
            "at_tick": latest.get("tick"),
            "techs_unlocked": latest.get("techs_unlocked"),
            "research": latest.get("research"),
            "power": latest.get("power"),
            "made_cumulative": dict(made.most_common()),
        }
        if w.label != "whole run":
            entry["made_in_window"] = dict(delta.most_common())
            prev_made = made
        out.append(entry)
    return out


# --------------------------------------------------------------------------
# reporting
# --------------------------------------------------------------------------


def hr(title: str) -> str:
    return f"\n{title}\n{'-' * len(title)}"


def report(a: dict, out=sys.stdout, top: int = 12) -> None:
    p = lambda *args: print(*args, file=out)

    p(f"\n{'=' * 78}")
    p(f"{a['run_id']}")
    p(f"{'=' * 78}")

    if a["missing_files"]:
        p(f"  missing files: {', '.join(a['missing_files'])} (reported, not fatal)")
    if a.get("error"):
        p(f"  ! {a['error']}")
        return

    if a["events_bad_lines"]:
        p(f"  ! {a['events_bad_lines']} unparseable line(s) inside events.jsonl")
    if a["events_truncated_tail"]:
        p("  note: events.jsonl ends mid-line -- this run is still being written")

    started = a.get("started_unix")
    when = time.strftime("%Y-%m-%d %H:%M", time.localtime(started)) if started else "?"
    wall = a.get("wall_seconds")
    wall_s = f"{wall / 60:.1f} min wall" if wall else (
        f"{a['wall_seconds_so_far'] / 60:.1f} min wall so far" if a.get("wall_seconds_so_far") else "wall unknown"
    )
    p(f"  started {when}   outcome: {a['outcome']}   {wall_s}")
    p(f"  roster: {a.get('roster')}   ticks {a['tick_lo']} -> {a['tick_hi']} "
      f"= {a['span_ticks']} ({minutes(a['span_ticks'])} game time)")
    if wall and a["span_ticks"]:
        p(f"  game speed: {a['span_ticks'] / TICKS_PER_SECOND / wall:.2f}x realtime")

    j = a["join"]
    if j["orphan_settles"] or j["never_settled"] or j["null_duration"]:
        p(f"  join: {j['orphan_settles']} settle(s) with no dispatch, "
          f"{j['never_settled']} dispatch(es) that never settled, "
          f"{j['null_duration']} settle(s) with no recorded duration "
          f"({j['derived_duration']} recovered from tick difference)")

    for d in a.get("splits_disagreements") or []:
        p(f"  ! splits.json says milestone {d['index']} took {d['splits']} ticks; "
          f"events say {d['events']}")

    p(hr("  MILESTONES"))
    if not a["milestones"]:
        p("    none recorded")
    for m in a["milestones"]:
        span = f"{m['span_ticks']:>7} ticks {minutes(m['span_ticks'])}" if m["span_ticks"] is not None else "   still open"
        p(f"    {m['label'][:56]:<56} {m['lo']:>7} -> {str(m['hi']):>7}  {span}  {m['outcome']}")

    p(hr("  PLANS  (who the planner gave the work to)"))
    if not a["plans"]:
        p("    none recorded")
    for pl in a["plans"]:
        p(f"    tick {pl['tick']:>7}  m{pl['milestone']}  steps={pl['steps']:<4} makespan={pl['makespan']:<7} roster={pl['roster']}")
        p(f"        steps/bot        {pl['steps_per_bot']}")
        p(f"        planned ticks/bot {pl['planned_work_per_bot']}")

    for w in a["windows"]:
        span = w["span_ticks"]
        p(hr(f"  WINDOW: {w['label'][:60]}  [{w['lo']} -> {w['hi']}]  "
             f"{span if span is not None else '?'} ticks {minutes(span)}  {w['outcome']}"))

        if not w["verbs"]:
            p("    no actions in this window")
        else:
            p("    action cost by verb (dispatch -> settle ticks, SUMMED OVER BOTS --")
            p("    so 'of span' can exceed 100% when bots work at the same time):")
            total = w["measured_action_ticks"] or 1
            for v, t in w["verb_ticks"].items():
                n = w["verbs"][v]
                p(f"      {v:<12} n={n:<5} {t:>7} ticks {minutes(t)}  "
                  f"{100 * t / total:5.1f}% of measured  "
                  f"{(100 * t / span) if span else 0:5.1f}% of span ")
            tail = f"{w['measured_pct_of_span']:5.1f}% of span " if span else ""
            p(f"      {'TOTAL':<12} n={sum(w['verbs'].values()):<5} "
              f"{w['measured_action_ticks']:>7} ticks {minutes(w['measured_action_ticks'])}"
              f"{'':>21}{tail}")
            if w["untimed_verbs"]:
                p(f"      NOTE: these verbs settle in the tick they dispatch and contribute 0: "
                  f"{', '.join(w['untimed_verbs'])}.")
                p("            A '% of action time' figure therefore describes only "
                  f"{', '.join(w['timed_verbs'])}.")

            p("\n    where those ticks went, by what was acted on:")
            for k, t in list(w["subject_ticks"].items())[:top]:
                if t == 0:
                    continue
                p(f"      {k:<34} n={w['subject_counts'][k]:<5} {t:>7} ticks {minutes(t)}")

        p("\n    per bot:")
        p(f"      {'bot':<4} {'disp':>5} {'settle':>6} {'act ticks':>10} {'walks':>6} "
          f"{'walk tk':>8} {'wfail':>5} {'wlost':>5} {'busy%':>7}")
        for b, s in sorted(w["per_bot"].items()):
            busy = f"{s['busy_pct']:6.1f}%" if s["busy_pct"] is not None else "     ?"
            p(f"      {b:<4} {s['dispatches']:>5} {s['settles']:>6} {s['action_ticks']:>10} "
              f"{s['walks']:>6} {s['walk_ticks']:>8} {s['walk_failed']:>5} {s['walk_lost']:>5} {busy:>7}")
        if w["per_bot"] and span:
            fleet = sum(s["busy_ticks"] for s in w["per_bot"].values())
            cap = span * len(w["per_bot"])
            p(f"      fleet utilisation: {fleet}/{cap} bot-ticks = {100 * fleet / cap:.1f}%")

        if w["statuses"]:
            p(f"\n    action verdicts: {w['statuses']}")
        for f in w["action_failures"]:
            p(f"      action failure: recorded={f['recorded_kind']:<18} "
              f"from-text={f['derived_from_text']:<18} n={f['count']}")

        p(f"\n    walks: {w['walks_dispatched']} dispatched, "
          f"{sum(s['walks'] for s in w['per_bot'].values())} settled")
        for f in w["walk_failure_kinds"]:
            flag = ""
            if f["recorded_kind"] in ("other", "none") and f["derived_from_text"] != "unclassified":
                flag = "   <- recorded before the classifier knew this wording (see 1f498593)"
            p(f"      walk failure: recorded={f['recorded_kind']:<18} "
              f"from-text={f['derived_from_text']:<18} n={f['count']}{flag}")
        if w["repeated_walk_failures"]:
            p("      repeated (bot, destination) failures -- the same site re-selected:")
            for r in w["repeated_walk_failures"]:
                p(f"        bot {r['bot']} -> [{r['to'][0]}, {r['to'][1]}]  x{r['count']}")

        if w["placements"]:
            p(f"\n    entities placed: {w['placements']}")

    if a.get("frozen") is None:
        p(hr("  FROZEN BOTS"))
        p("    no samples.jsonl -- cannot tell whether any bot stopped moving")
    elif a["frozen"]:
        p(hr("  FROZEN BOTS  (position byte-identical across the stretch)"))
        p("    A bot with no work stands still legitimately. What separates that from")
        p(f"    being walled in is what the run built within {ENCLOSURE_RADIUS:.0f} tiles, and when.")
        shown = a["frozen"][:top]
        for f in shown:
            tail = "  (still frozen at the last sample)" if f["reached_end"] else ""
            pos = f["position"] or {}
            p(f"    bot {f['bot']}  ticks {f['from_tick']} -> {f['to_tick']}  "
              f"= {f['ticks']} {minutes(f['ticks'])} at "
              f"[{pos.get('x')}, {pos.get('y')}]{tail}")
            before = f.get("placed_before_freeze") or {}
            after = f.get("placed_after_freeze") or {}
            if before:
                p(f"        already there when it stopped: {before}")
            if after:
                p(f"        built around it afterwards:     {after} "
                  f"(first at tick {f.get('first_placement_after_freeze')})")
            if not before and not after:
                p("        nothing built nearby -- idle, not walled in")
        if len(a["frozen"]) > len(shown):
            p(f"    ... and {len(a['frozen']) - len(shown)} shorter stretch(es); "
              f"raise --freeze-ticks or --top to see them")
    else:
        p(hr("  FROZEN BOTS"))
        p("    none over the threshold")

    prod = a.get("production")
    if prod:
        p(hr("  PRODUCTION (cumulative force totals at each window's end)"))
        for e in prod:
            if e["label"] == "whole run":
                continue
            p(f"    {e['label'][:58]:<58} @tick {e['at_tick']}  techs={e['techs_unlocked']}")
            made = e.get("made_in_window") or {}
            if made:
                items = ", ".join(f"{k}={v}" for k, v in list(made.items())[:18])
                p(f"        made in window: {items}")
    p("")


def summary_line(a: dict) -> str:
    if a.get("error"):
        return f"{a['run_id']}  ! {a['error']}"
    parts = [f"{a['run_id']}", f"{minutes(a['span_ticks'])}", f"{a['outcome'][:22]:<22}"]
    for m in a["milestones"]:
        word = m["outcome"].split()[0]
        parts.append(f"{m['label'].split()[0]}={minutes(m['span_ticks'])}/{word}")
    whole = a["windows"][0]
    busiest = max(whole["per_bot"].items(), key=lambda kv: kv[1]["busy_ticks"], default=(None, None))
    if busiest[0] is not None and whole["span_ticks"]:
        fleet = sum(s["busy_ticks"] for s in whole["per_bot"].values())
        cap = whole["span_ticks"] * len(whole["per_bot"])
        parts.append(f"busiest=bot{busiest[0]}@{busiest[1]['busy_pct']:.0f}%")
        parts.append(f"fleet={100 * fleet / cap:.0f}%")
    if whole["verb_ticks"]:
        top_verb, top_ticks = next(iter(whole["verb_ticks"].items()))
        measured = whole["measured_action_ticks"] or 1
        parts.append(f"top={top_verb}@{100 * top_ticks / measured:.0f}%")
    return "  ".join(parts)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n\n")[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Reads only. Safe to point at a run that is still being written.",
    )
    ap.add_argument("dirs", nargs="*", help="run directories")
    ap.add_argument("--all", action="store_true",
                    help="analyse every run under --runs-root")
    ap.add_argument("--runs-root", default="workspace/runs",
                    help="where --all looks (default: workspace/runs)")
    ap.add_argument("--summary", action="store_true",
                    help="one line per run instead of the full report")
    ap.add_argument("--json", action="store_true", help="dump the analysis as JSON")
    ap.add_argument("--freeze-ticks", type=int, default=DEFAULT_FREEZE_TICKS,
                    help=f"frozen-position threshold in ticks (default {DEFAULT_FREEZE_TICKS})")
    ap.add_argument("--top", type=int, default=12, help="rows in the by-subject breakdown")
    args = ap.parse_args(argv)

    dirs = list(args.dirs)
    if args.all:
        root = args.runs_root
        if not os.path.isdir(root):
            print(f"no such directory: {root}", file=sys.stderr)
            return 2
        dirs += [
            os.path.join(root, d)
            for d in sorted(os.listdir(root))
            if os.path.isdir(os.path.join(root, d))
        ]
    if not dirs:
        ap.print_help()
        return 2

    results = []
    for d in dirs:
        if not os.path.isdir(d):
            print(f"no such run directory: {d}", file=sys.stderr)
            continue
        results.append(analyse(d, args.freeze_ticks))

    if args.json:
        json.dump(results if len(results) > 1 else results[0], sys.stdout, indent=2)
        print()
    elif args.summary:
        for a in results:
            print(summary_line(a))
    else:
        for a in results:
            report(a, top=args.top)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
