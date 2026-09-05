#!/usr/bin/env python3
"""Where did a run's time go?

Reads one archived run directory (``workspace/runs/run-<unix>-<pid>/``) and
reports the time accounting: milestone spans, action cost by verb, per-bot
utilisation, the busiest bot's idle gaps and what each was waiting for, walk
failures, what was built, and which bots stopped moving.

    python3 tools/run_analysis.py workspace/runs/run-1788449752-46541
    python3 tools/run_analysis.py --all --summary
    python3 tools/run_analysis.py --json workspace/runs/run-1788449752-46541
    python3 tools/run_analysis.py --compare workspace/runs/run-A workspace/runs/run-B

``--compare`` puts two runs side by side with deltas, under a comparability
block that refuses (exit 3) when the two runs were not produced under the same
conditions and flags every other way they might not be measuring the same
thing. See the comment above :func:`comparability` for what it refuses and
which wrong conclusion each guard prevents.

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
# Below this fraction of the nominal tick rate a run is reported as starved:
# the server was not keeping up, and its wall-clock-denominated waits (RCON
# round trips, anything sleeping in seconds) were worth fewer ticks than the
# same run on a quiet box. 0.8 is a reporting threshold, not a verdict about
# the data -- every tick-denominated number is still exact.
STARVED_RATIO = 0.8
# A heartbeat interval shorter than this measures noise rather than a rate.
MIN_RATE_INTERVAL_MS = 5_000
TICKS_PER_MINUTE = 60 * TICKS_PER_SECOND

# Files a complete run leaves behind. Older runs predate some of them and
# interrupted runs never wrote the rest; every one is optional here and its
# absence is reported, never fatal.
KNOWN_FILES = ("events.jsonl", "manifest.json", "splits.json", "samples.jsonl", "map.jsonl")

# How long a bot's position must stay byte-identical before it is worth
# reporting. 3000 ticks is 50 seconds of game time -- far longer than any walk
# or mine, and short enough to catch a bot that froze near the end.
DEFAULT_FREEZE_TICKS = 3000

# Individual idle gaps kept per window. Enough for the long waits; the rest is
# dispatch overhead and is rolled into a stated tail.
GAP_ROWS = 20

# How far `samples.jsonl` may fall short of the run's own tick span before it
# is reported as a hole rather than a rounding difference.
#
# The mod samples bots every 60 ticks and the force every 300, and the recorder
# archives on a 1800-tick interval (`SAMPLE_INGEST_INTERVAL_TICKS` in
# `crates/core/src/record/mod.rs`), so a healthy run's last sample sits within
# a couple of thousand ticks of its last event -- in either direction, since
# the two clocks are read at different moments. 6000 ticks is 100 seconds:
# comfortably past all of that, and nowhere near the failures it exists to
# catch.
#
# It exists because those failures were invisible. `run-1788459085-32452`
# sampled its whole 281,000 ticks and archived the first 78,840; nothing in
# this report, in the run's own manifest (it never wrote one) or in any log
# line said so, and the loss was found by comparing the last tick of two files
# by hand. Every archived run is now checked for it on every run of this tool.
SAMPLE_COVERAGE_SLACK_TICKS = 6000

# What world and what build a run came from, read from two places and merged.
#
# `run_started` carries `seed`, `factorio` and `git`; they are in the schema and
# null in every one of the twenty-four archived runs. `provenance.json` beside
# the events (`crates/core/src/record/provenance.rs`) carries the same three in
# richer form plus the map fingerprint, the build profile, the workspace and
# the save a run was resumed from. Both are read with `.get()`, a missing file
# is not an error, and a missing or null field is UNKNOWN -- never a match.
PROVENANCE_FILE = "provenance.json"

# Every provenance field `--compare` knows, and what a *difference* in it means
# when both runs recorded one. The severities are not uniform and the
# asymmetry is the point:
#
# `seed`, `git`, `factorio`, `profile`, `resumed_from` -- REFUSE. Each is a
#     difference in what was run or what it was run on, and the numbers cannot
#     be attributed to the change under test. `resumed_from` is on this list on
#     the record's own instruction: a resumed run begins with built furnaces
#     and charged chests, so its timings measure a different thing, and
#     `Some(_)` against `None` is to be treated exactly like two seeds.
#     `profile` is here because a debug and a release build resolve mods and
#     scripts from different places, so the same commit runs different Lua.
#
# `map` -- UNKNOWN, deliberately NOT a refusal. The fingerprint is a digest
#     over the *charted* resource tiles, and charting grows as bots explore, so
#     two runs on the same map diverge as soon as one of them walks further.
#     `ResourceFingerprint`'s own words: equal digests mean the same map,
#     unequal digests mean unknown. Refusing on it would refuse nearly every
#     honest pair and teach the reader to type --force by reflex, which is a
#     worse outcome than the flag it replaced.
#
# `workspace` -- FLAG. A different workspace is a different `level.zip` and a
#     different copy of the mods, which usually shows up in `map` or `seed` as
#     well; it is named separately because when it is the *only* difference,
#     the map identity above may simply not have been recorded.
PROVENANCE_SEVERITY = {
    "seed": "refuse",
    "git": "refuse",
    "factorio": "refuse",
    "profile": "refuse",
    "resumed_from": "refuse",
    "map": "unknown",
    "workspace": "flag",
    # `bot_mode` / `game_speed` -- FLAG. Character bots and clients play with
    #     the same reach, walk speed and craft times, and every timing is in
    #     ticks, so the pair is comparable; but only a client run can be
    #     filmed, and a run at speed 10 on a server that could not keep up
    #     delivers fewer ticks per second than it planned for.
    "bot_mode": "flag",
    "game_speed": "flag",
}
PROVENANCE_FIELDS = tuple(PROVENANCE_SEVERITY)

# What `resumed_from: null` means when the file that defines it is present:
# a run that started on a fresh world. Absent that file it means nothing at
# all, and the field reads as unknown instead.
FRESH_WORLD = "fresh world (not resumed)"


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


def _norm_git(raw: Any) -> str | None:
    """A commit id from either shape `git` is written in.

    `provenance.json` writes `{commit, dirty, source}`; `run_started` writes a
    bare string. The dirty bit is deliberately NOT folded into the returned
    value -- two runs from the same commit with uncommitted edits are not the
    same build, and folding "+dirty" into the string would make them compare
    equal to each other. It is read separately, and equal-but-dirty is reported
    as unknown rather than as a match.
    """
    if isinstance(raw, dict):
        commit = raw.get("commit")
        return str(commit) if commit else None
    return str(raw) if raw is not None else None


def _norm_map(raw: Any) -> str | None:
    """The map's identity: `ResourceFingerprint.digest` when there is one.

    Falls back to the canonical JSON of whatever was written, so a future shape
    still compares field-for-field instead of silently reading as unknown.
    """
    if isinstance(raw, dict):
        digest = raw.get("digest")
        return str(digest) if digest else json.dumps(raw, sort_keys=True)
    return str(raw) if raw is not None else None


def read_provenance(run_dir: str, run_started: dict | None) -> dict:
    """What world and what build produced this run, merged from both writers.

    `provenance.json` wins where it exists because it is the richer record; a
    run that has no such file falls back to `run_started`'s three fields, and a
    run that has neither reports every field as UNKNOWN. That last case is not
    a defect to route around: it is what all twenty-four archived runs are, and
    `--compare` says so in as many words rather than letting two absences read
    as a match.

    The seed is normalised through `str` on both paths on purpose --
    `provenance.json` writes it as a string and `run_started` as an integer, so
    an un-normalised comparison would report the same seed as two different
    ones the first time a run recorded both.

    Every value is a string or None. None means "nobody recorded this", and no
    caller may treat two Nones as agreement.
    """
    sidecar = load_json(os.path.join(run_dir, PROVENANCE_FILE))
    if not isinstance(sidecar, dict):
        sidecar = {}
    started = run_started or {}

    def pick(*candidates) -> dict:
        for source, raw, norm in candidates:
            if raw is None:
                continue
            value = norm(raw)
            if value is not None:
                return {"value": value, "source": source, "raw": raw}
        return {"value": None, "source": None, "raw": None}

    def text(raw: Any) -> str | None:
        return str(raw) if raw is not None else None

    out: dict[str, Any] = {
        "seed": pick(
            (PROVENANCE_FILE, sidecar.get("seed"), text),
            ("run_started", started.get("seed"), text),
        ),
        "git": pick(
            (PROVENANCE_FILE, sidecar.get("git"), _norm_git),
            ("run_started", started.get("git"), _norm_git),
        ),
        "factorio": pick(
            (PROVENANCE_FILE, sidecar.get("factorio"), text),
            ("run_started", started.get("factorio"), text),
        ),
        "map": pick(
            (PROVENANCE_FILE, sidecar.get("map"), _norm_map),
            ("run_started", started.get("map"), _norm_map),
        ),
        "profile": pick((PROVENANCE_FILE, sidecar.get("profile"), text)),
        "workspace": pick((PROVENANCE_FILE, sidecar.get("workspace"), text)),
        "bot_mode": pick((PROVENANCE_FILE, sidecar.get("bot_mode"), text)),
        "game_speed": pick((PROVENANCE_FILE, sidecar.get("game_speed"), text)),
        # Null-with-meaning, but ONLY when the file that defines it is present:
        # `resumed_from: null` there means the run started on a fresh world,
        # where no file at all means nobody ever recorded whether it did.
        "resumed_from": pick(
            (
                PROVENANCE_FILE,
                sidecar.get("resumed_from") or (FRESH_WORLD if sidecar else None),
                text,
            )
        ),
    }
    out["git_dirty"] = bool(
        isinstance(sidecar.get("git"), dict) and sidecar["git"].get("dirty")
    )
    # What the run *asked* for, against `run_started.bots` which is what it
    # got. The pair is the only evidence a run degraded.
    requested = sidecar.get("roster_requested")
    out["roster_requested"] = list(requested) if isinstance(requested, list) else None
    out["file_present"] = bool(sidecar)
    out["schema"] = sidecar.get("schema")
    # Any schema is accepted: the file's whole value is that archived runs stay
    # readable, and refusing a newer one would break that from this end.
    out["map_tiles"] = (
        (sidecar.get("map") or {}).get("tiles") if isinstance(sidecar.get("map"), dict) else None
    )
    return out


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
    # Before `no_path`: the executor appends this to a `found no path` refusal
    # when its mobility probe found every short hop refused too, so the string
    # matches both and the bench is the finding (`WalkFailureKind::BoxedIn`).
    ("boxed_in", re.compile(r"the character is boxed in", re.I)),
    ("no_path", re.compile(r"failed to path find|returned no path", re.I)),
    ("pathfinder_busy", re.compile(r"try again later|refused a re-path", re.I)),
    ("repath_limit", re.compile(r"re-?path limit", re.I)),
    ("stalled", re.compile(r"made no progress|aborted before reaching", re.I)),
    ("timeout", re.compile(r"timed? ?out|no reply", re.I)),
)


def walk_blocker(text: str | None) -> str | None:
    """What BotBridge said was in the way when a leg stopped progressing.

    The clause `control.lua`'s ``walk_stall_cause`` appends to a stall:

        ... to (-10.5/-18.5), moved 0.02 tiles, blocked at (-10.437/-18.702)
        by character #3 (mining) on tile 'grass-1' (+2 more)

    Returns a label for the histogram, and ``None`` -- and only ``None`` --
    when the message carries **no clause at all**, i.e. an archived run from
    before the probe existed. A clause whose class word this reader does not
    know comes back as the mod's own words rather than as a bucket: a reader
    then sees the new wording in the histogram instead of an empty column,
    which is the failure this file has already had twice (19 of 20 walk
    failures sitting in ``other``, and the mining refusal's wording matched for
    a placement refusal). ``"unreadable"`` is reserved for a clause that is
    malformed rather than merely unfamiliar.

    The bot id is dropped from the label on purpose -- *which* bot is in
    ``error`` and in the per-bot table; what this histogram answers is how many
    stalls were caused by a bot at all, and by one that was mining rather than
    idle. ``step_aside_from_footprint`` only steers a blocker that is neither
    walking nor mining, so ``character (mining)`` is a blocker nothing is going
    to ask to move.
    """
    if not text:
        return None
    if "blocker unknown (probe failed:" in text:
        return "probe failed"
    marker = "blocked at "
    i = text.find(marker)
    if i < 0:
        return None
    rest = text[i + len(marker):]
    j = rest.find(" by ")
    if j < 0:
        return "unreadable"
    cause = rest[j + len(" by "):]
    tile = None
    k = cause.find(" on tile '")
    if k >= 0:
        tile = cause[k + len(" on tile '"):].split("'")[0]
        cause = cause[:k]
    cause = cause.split(" (+")[0].strip()
    cause = re.sub(r"character #\d+", "character", cause)
    if cause.startswith("nothing findable"):
        # The ground IS the answer when nothing is standing on it, so a stall
        # on water reads differently from one on grass -- and the distance
        # covered separates the two bugs this bucket otherwise merges.
        # `made no progress` is measured as a LEG TIMEOUT, never as a position
        # delta, so a "stall" that covered three tiles is `walk_leg_timeout_ticks`
        # being wrong and one that covered none is the pathfinder. Reported as
        # the mod measured it, rounded to a tenth so they group; `?` is "not
        # measured", which is not zero.
        moved = re.search(r"\bmoved ([\d.]+) tiles", text)
        moved = f"{float(moved.group(1)):.1f}" if moved else "?"
        return f"nothing findable (on {tile or 'unknown tile'}, moved {moved} tiles)"
    return cause or "unreadable"


ACTION_TEXT_RULES = (
    # `character is standing` rather than `another character is standing`:
    # the mining refusal says "another", the placement refusal
    # (`rcon_place_entity`, `mods/BotBridge/control.lua`) says "a character is
    # standing in the footprint", and the narrower pattern derived "rejected"
    # for every one of the second kind -- including the one that ended
    # `run-1788481380-80843`'s first plan.
    ("blocked", re.compile(r"character is standing|is in the way|blocked", re.I)),
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


def savepoint_rows(events: list[dict], run_dir: str) -> list[dict]:
    """What each milestone's savepoint did, and whether the file is still there.

    Three states, and they are not the same thing:

      * written and present -- a later run can start from it with
        ``--resume-from <run>:<milestone>``;
      * written and **gone** -- the event says a file was made and the file is
        not there, which means somebody deleted it by hand (the reaper deletes
        whole runs, taking the event log with it, so a reaped savepoint cannot
        show up here at all);
      * failed -- the run asked and the engine never finished. This is the one
        worth surfacing: without the ``savepoint_failed`` event, a milestone
        with no savepoint and a run that never asked for one are the same
        silence.
    """
    rows: list[dict] = []
    for e in events:
        kind = e.get("kind")
        if kind == "savepoint_written":
            rel = e.get("file") or ""
            rows.append(
                {
                    "milestone": e.get("milestone_index"),
                    "tick": e.get("tick"),
                    "file": rel,
                    "bytes": e.get("bytes"),
                    "wrote_ms": e.get("wrote_ms"),
                    "on_disk": os.path.exists(os.path.join(run_dir, rel)) if rel else False,
                    "error": None,
                }
            )
        elif kind == "savepoint_failed":
            rows.append(
                {
                    "milestone": e.get("milestone_index"),
                    "tick": e.get("tick"),
                    "file": None,
                    "bytes": None,
                    "wrote_ms": None,
                    "on_disk": False,
                    "error": e.get("error"),
                }
            )
    return rows


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
    result["savepoints"] = savepoint_rows(events, run_dir)
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
    result["provenance"] = read_provenance(run_dir, run_started)
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

    execution = batch_execution(events)
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
                # Zipped by position: `batch_execution` walks the same event
                # list in the same order, so the nth plan there is this one.
                "execution": execution[len(result["plans"])]
                if len(result["plans"]) < len(execution)
                else None,
            }
        )

    result["vision"] = free_vision(events)
    result["deaths"] = bot_deaths(events)
    result["tick_rate"] = delivered_tick_rate(
        events, (result["provenance"].get("game_speed") or {}).get("value")
    )
    result["planning"] = planning_rows(events)

    placed = load_jsonl(os.path.join(run_dir, "map.jsonl"))
    result["map_present"] = placed.present

    samples = load_jsonl(os.path.join(run_dir, "samples.jsonl"))
    result["samples_present"] = samples.present
    result["samples_coverage"] = sample_coverage(samples.rows, lo, hi, samples.present)

    result["windows"] = [score_window(w, events, joined, placed.rows) for w in windows]
    if samples.present:
        result["frozen"] = frozen_bots(samples.rows, freeze_ticks, placed.rows)
        result["production"] = production_at(samples.rows, windows)
        # `None` when the run predates sample schema 2, which is not the same
        # fact as "no machines were idle" -- see `machines_at`.
        result["machines"] = machines_at(samples.rows)
    else:
        result["frozen"] = None
        result["production"] = None
        result["machines"] = None
    return result


def free_vision(events: list[dict]) -> dict:
    """How much ground the model was given without a bot visiting it.

    The mod ingests every entity of every chunk the *engine generates*
    (``on_chunk_generated``) and never consults the force's charted area, so
    the world model knows about ore, water and nests no character has been
    near. In ``run-1788532631-48030`` the furthest bot reached 63.8 tiles
    while the model that run produced held crude oil at 380 and 505.

    ``vision_measured`` is the run's own disclosure of that (see
    ``EventKind::VisionMeasured``). This reads the **last** one in the log:
    the closing measurement is written after ``run_finished``, and the earlier
    beats show the figure growing rather than replacing it.

    Everything here can be absent, and absence is reported as absence. A run
    with no ``vision_measured`` at all is a run whose free vision is
    **unknown** -- every run archived before 2026-09-04, plus any run whose
    recorder was never given a world model -- and that is not the same fact as
    a run that was given none.
    """
    beats = [e for e in events if e.get("kind") == "vision_measured"]
    if not beats:
        return {"present": False, "beats": 0}
    last = beats[-1]
    return {
        "present": True,
        "beats": len(beats),
        "tick": last.get("tick"),
        "model_tiles": last.get("model_tiles"),
        "model_furthest": last.get("model_furthest"),
        "model_furthest_position": last.get("model_furthest_position"),
        "model_resource_tiles": last.get("model_resource_tiles"),
        "model_enemy_structures": last.get("model_enemy_structures"),
        "travelled_tiles": last.get("travelled_tiles"),
        "travelled_bot": last.get("travelled_bot"),
        "travelled_at_tick": last.get("travelled_at_tick"),
        "bot_samples": last.get("bot_samples"),
        "unearned_ratio": last.get("unearned_ratio"),
    }


def bot_deaths(events: list[dict]) -> dict:
    """Every ``bot_died`` / ``bot_respawned`` / ``roster_changed`` in the log.

    Three events, one story: a bot lost its character, got it back, and what
    the supervisor did about it. Before these existed (2026-09-04) a dead bot
    was an ordinary stream of failures with no distinguishing kind, and the
    roster -- computed once at script start -- kept assigning it work.

    Absence is reported as absence. ``present`` is False for a run whose
    build predates the events, which is **every run archived before
    2026-09-04**; that is "could not have said", not "nobody died". A run on
    this build with zero deaths has ``present`` True and empty lists, which is
    the only reading that actually means no bot died.

    ``no_character`` counts the ``action_settled`` / ``walk_settled`` failures
    classified as a missing character -- the failures a death explains. It is
    counted here rather than read off ``deaths`` so the two can disagree: a
    ``no_character`` refusal with no ``bot_died`` beside it is a bot that had
    no character for some other reason (the crash-site cutscene, a
    controller switch), and that is a finding.
    """
    deaths = [e for e in events if e.get("kind") == "bot_died"]
    respawns = [e for e in events if e.get("kind") == "bot_respawned"]
    changes = [e for e in events if e.get("kind") == "roster_changed"]
    no_character = 0
    for e in events:
        if e.get("kind") in ("action_settled", "walk_settled"):
            if (e.get("failure") or {}).get("kind") == "no_character":
                no_character += 1
    # Pair each death with the next respawn of the same bot, so the stretch
    # without a character is a number rather than two ticks to subtract.
    rows: list[dict] = []
    pending: dict[int, dict] = {}
    for e in events:
        kind = e.get("kind")
        if kind == "bot_died":
            row = {
                "bot": e.get("bot"),
                "died_tick": e.get("tick"),
                "position": e.get("position"),
                "cause": e.get("cause"),
                "cause_type": e.get("cause_type"),
                "respawn_in": e.get("respawn_in"),
                "respawned_tick": None,
                "dead_ticks": None,
            }
            rows.append(row)
            pending[e.get("bot")] = row
        elif kind == "bot_respawned":
            row = pending.pop(e.get("bot"), None)
            if row is not None:
                row["respawned_tick"] = e.get("tick")
                row["dead_ticks"] = e.get("tick") - row["died_tick"]
    # `present` is "at least one of the three events is in the log", and it
    # is deliberately NOT inferred from anything else: nothing in a record
    # says which build wrote it, so a run with none of these events is
    # either a run on which nobody died or a run on a build that could not
    # have said so, and this tool cannot tell those apart. It says so.
    present = bool(deaths or respawns or changes)
    return {
        "present": present,
        "deaths": rows,
        "respawns": len(respawns),
        "roster_changes": [
            {
                "tick": e.get("tick"),
                "bots": e.get("bots"),
                "left": e.get("left"),
                "returned": e.get("returned"),
                "reason": e.get("reason"),
            }
            for e in changes
        ],
        "no_character_failures": no_character,
    }


def batch_execution(events: list[dict]) -> list[dict]:
    """What happened to each ``plan_created`` -- did anything ever run it?

    This is the question two whole runs were thrown away over. Every event
    about a plan's execution except one is written *after* the batch finishes
    (the driver calls ``record.actions()`` on the supervisor's "ran"
    transition), so a run killed or crashed mid-batch ends on a
    ``plan_created`` with nothing after it -- and so does a run that planned
    and then genuinely dispatched nothing. Nine of the twenty-four runs
    archived when this was written end that way, and the archive alone cannot
    tell which kind each one was.

    ``batch_progress`` is the one event written *while* a batch runs, and it
    is what makes the two separable. This function does the separating, and it
    is careful about the third answer:

    ``dispatched``      something was dispatched from this plan; the batch ran.
    ``never_dispatched`` a heartbeat looked and found nothing dispatched at
                        all. No threshold decides this -- the counter is zero.
    ``cut_short``       heartbeats show dispatches, but the batch's own
                        per-action lines never arrived: the run stopped while
                        it was working. The last heartbeat says how far it got.
    ``unknown``         no heartbeat and no dispatch. Either the build predates
                        ``batch_progress`` or the run stopped inside the first
                        interval. **Not** reported as a stall: that is exactly
                        the confident-wrong answer this whole function exists
                        to stop giving.

    Attribution is positional: a heartbeat belongs to the ``plan_created`` it
    follows. That is exact while one batch runs at a time, which every shipped
    driver does -- each waits on its run before planning the next.
    """
    plans: list[dict] = []
    current: dict | None = None
    for e in events:
        kind = e.get("kind")
        if kind == "plan_created":
            current = {
                "tick": e.get("tick"),
                "milestone": e.get("milestone_index"),
                "steps": e.get("steps"),
                "dispatched": 0,
                "walks": 0,
                "beats": 0,
                "last_beat": None,
                "first_dispatch_tick": None,
            }
            plans.append(current)
            continue
        if current is None:
            continue
        if kind == "batch_progress":
            current["beats"] += 1
            current["last_beat"] = e
        elif kind in ("action_dispatched", "walk_dispatched"):
            current["dispatched" if kind == "action_dispatched" else "walks"] += 1
            if current["first_dispatch_tick"] is None:
                current["first_dispatch_tick"] = e.get("tick")
    for pl in plans:
        beat = pl["last_beat"] or {}
        beat_dispatched = (beat.get("dispatched") or 0) + (beat.get("walks_dispatched") or 0)
        if pl["dispatched"] or pl["walks"]:
            pl["verdict"] = "dispatched"
        elif pl["beats"] and beat_dispatched == 0:
            pl["verdict"] = "never_dispatched"
        elif pl["beats"]:
            pl["verdict"] = "cut_short"
        else:
            pl["verdict"] = "unknown"
    return plans


def delivered_tick_rate(events: list[dict], game_speed: str | None) -> dict:
    """How many ticks per second the game actually delivered while batches ran.

    ``batch_progress`` carries both clocks -- the event's ``tick`` and the
    batch's ``elapsed_ms`` -- so consecutive heartbeats of one batch give a
    measured tick rate, and the first heartbeat is measured against the
    batch's first dispatch (``elapsed_ms`` ~0 there). The nominal rate is
    ``60 * game_speed`` from provenance. A run at 10x on a box that could
    only deliver 350 tps is a *different run* from one that got its 600, and
    nothing else in the record says so: every tick-denominated number is the
    same either way, only the wall clock knows.

    Intervals shorter than ``MIN_RATE_INTERVAL_MS`` are dropped -- a
    heartbeat 30 ms after a dispatch measures noise, not a rate.
    """
    samples: list[tuple[int, int]] = []  # (ticks, ms) per interval
    origin: tuple[int, int] | None = None  # (tick, elapsed_ms) of the last point
    for e in events:
        kind = e.get("kind")
        if kind == "plan_created":
            origin = None
        elif kind in ("action_dispatched", "walk_dispatched") and origin is None:
            origin = (int(e.get("tick") or 0), 0)
        elif kind == "batch_progress":
            tick = e.get("tick")
            ms = e.get("elapsed_ms")
            if not isinstance(tick, int) or not isinstance(ms, int):
                continue
            if origin is not None:
                dt, dms = tick - origin[0], ms - origin[1]
                if dms >= MIN_RATE_INTERVAL_MS and dt >= 0:
                    samples.append((dt, dms))
            origin = (tick, ms)
    ticks = sum(t for t, _ in samples)
    ms = sum(m for _, m in samples)
    delivered = ticks * 1000 / ms if ms else None
    try:
        speed = float(game_speed) if game_speed is not None else None
    except ValueError:
        speed = None
    nominal = TICKS_PER_SECOND * speed if speed else None
    ratio = (delivered / nominal) if (delivered is not None and nominal) else None
    return {
        "intervals": len(samples),
        "ticks": ticks,
        "ms": ms,
        "delivered_tps": delivered,
        "nominal_tps": nominal,
        "ratio": ratio,
        "starved": ratio is not None and ratio < STARVED_RATIO,
    }


def planning_rows(events: list[dict]) -> list[dict]:
    """Every ``planning_timed`` event: what each plan cost in wall time and
    in game ticks. ``charged`` is ``tick_after - tick_before`` -- zero on a
    plan made against a stopped clock, and ``None`` when either tick is
    unknown, which is not zero."""
    rows = []
    for e in events:
        if e.get("kind") != "planning_timed":
            continue
        before, after = e.get("tick_before"), e.get("tick_after")
        charged = (after - before) if isinstance(before, int) and isinstance(after, int) else None
        rows.append(
            {
                "tick": e.get("tick"),
                "planning_ms": e.get("planning_ms"),
                "paused": bool(e.get("paused")),
                "reason": e.get("reason"),
                "charged": charged,
            }
        )
    return rows


def waiting_lines(beat: dict, limit: int = 4) -> list[str]:
    """What the last heartbeat said each step was waiting for.

    This is the half of ``batch_progress`` that names the *work* rather than a
    bot. Without it a killed-mid-batch run reports frozen counters and a bot
    id, and identifying the action needs a live rcon query against a game that
    is, by then, gone. One run had an action in flight for eleven minutes and
    could never say which one.

    Nothing here is a verdict. ``lag_deadline`` for twenty minutes is a plan
    doing exactly what it was told; ``reply`` for twenty minutes is a
    completion that went missing. The state is printed, the reader decides.

    A run recorded before this field existed has no ``waiting`` key at all, and
    gets a line saying so rather than an empty list -- absence of the field and
    absence of waiters are not the same fact.
    """
    if "waiting" not in beat:
        return ["                          (this run predates the per-step waiting detail)"]
    waiting = beat.get("waiting") or []
    total = beat.get("waiting_total", len(waiting))
    if not waiting:
        if (beat.get("in_flight") or 0) > 0:
            return [
                "                          NOTHING listed as waiting while "
                f"{beat.get('in_flight')} action(s) are in flight -- those two counts",
                "                          come from different writers, so this is a broken "
                "reporter, not a quiet run",
            ]
        return ["                          nothing was waiting"]
    out = [f"                          waiting ({total} step(s), longest first):"]
    for w in waiting[:limit]:
        who = f"bot {w.get('bot')}"
        what = w.get("action") or (
            f"action {w.get('id')}" if w.get("id") is not None else f"walk {w.get('step_index')}"
        )
        extra = ""
        if w.get("blocked_by") is not None:
            extra = f" (blocked by action {w['blocked_by']})"
        elif w.get("deadline_tick") is not None:
            extra = f" (until tick {w['deadline_tick']})"
        out.append(
            f"                            {w.get('waiting_ms', 0) / 1000:7.0f}s  "
            f"{w.get('waiting_on', '?'):<20} {who}  {what}{extra}"
        )
    if total > len(waiting):
        out.append(f"                            ... and {total - len(waiting)} more not reported")
    return out


def sample_coverage(samples: list[dict], lo: int, hi: int, present: bool) -> dict:
    """Does the sample stream span the run the events describe?

    Reported per kind, because the two beats fail independently: the bots
    sampler and the force sampler are separate handlers on separate ticks
    (60 and 300), so one can die while the other keeps writing, and a single
    "last sample" figure would hide that.

    ``verdict`` is one of:

    ``no_samples``  the run archived none at all
    ``covered``     every kind reaches the end of the run
    ``short``       at least one kind stops more than
                    :data:`SAMPLE_COVERAGE_SLACK_TICKS` before it
    ``late``        sampling started well after the run did
    """
    coverage: dict[str, Any] = {
        "run_lo": lo,
        "run_hi": hi,
        "by_kind": {},
        "verdict": "no_samples" if not (present and samples) else "covered",
        "worst_lag_ticks": None,
        "worst_lead_ticks": None,
    }
    if not (present and samples):
        return coverage

    by_kind: dict[str, list[int]] = collections.defaultdict(list)
    for s in samples:
        tick = s.get("tick")
        if isinstance(tick, int):
            by_kind[str(s.get("kind", "?"))].append(tick)

    worst_lag = 0
    worst_lead = 0
    for kind, ticks in sorted(by_kind.items()):
        first, last = min(ticks), max(ticks)
        lag = max(0, hi - last)
        lead = max(0, first - lo)
        worst_lag = max(worst_lag, lag)
        worst_lead = max(worst_lead, lead)
        coverage["by_kind"][kind] = {
            "count": len(ticks),
            "first_tick": first,
            "last_tick": last,
            "lag_ticks": lag,
            "lead_ticks": lead,
        }
    coverage["worst_lag_ticks"] = worst_lag
    coverage["worst_lead_ticks"] = worst_lead
    if worst_lag > SAMPLE_COVERAGE_SLACK_TICKS:
        coverage["verdict"] = "short"
    elif worst_lead > SAMPLE_COVERAGE_SLACK_TICKS:
        coverage["verdict"] = "late"
    return coverage


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
    # What the *actions* claim was built, kept beside what `map.jsonl` saw go
    # up. The two answer different questions and `--compare` prints both: a
    # settle says the executor believed the placement succeeded, a map row says
    # an entity exists. `only_ghosts` runs, and placements the game silently
    # refused, are exactly where they disagree.
    place_ok = collections.Counter()
    place_failed = collections.Counter()

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
        if j["verb"] == "place":
            (place_ok if j["status"] == "success" else place_failed)[j["subject"]] += 1
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
    walk_stall_causes = collections.Counter()
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
        if derived == "stalled":
            # Only stalls carry a cause: `no_path` and the pre-dispatch
            # refusals never reached the follower, so counting them here would
            # make the "no cause recorded" bucket say something it does not
            # mean.
            walk_stall_causes[
                walk_blocker(e.get("error")) or "(no cause recorded -- mod predates the probe)"
            ] += 1
        to = e.get("to") or {}
        repeat_failures[(bot, round(to.get("x", 0), 2), round(to.get("y", 0), 2))] += 1

    bots = sorted(
        {b for b in list(dispatches_by_bot) + list(walks_by_bot) + list(action_ticks_by_bot) if b is not None}
    )
    per_bot = {}
    for b in bots:
        # A SUM, NOT A UNION -- and since the executor gained per-bot
        # concurrency (`crates/executor/src/occupancy.rs`) a bot can have a
        # queued craft or research running alongside an exclusive action, so
        # these intervals can genuinely overlap and this figure can exceed the
        # span. `busy_pct > 100%` is therefore a real reading and not a bug; it
        # says the bot did two things at once, which is the point.
        #
        # The overlap-aware number is `idle_gaps`, which merges the intervals
        # before measuring. Quote that one when the question is "how much of
        # the window was this bot idle"; quote this one only when the question
        # is "how much work did it do".
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

    # The busiest bot is the one whose timeline the window's span is made of, so
    # it is the one whose *gaps* are the window's lost time. Chosen, not
    # hardcoded to bot 1: which bot that is, is a finding.
    critical = max(
        per_bot.items(), key=lambda kv: kv[1]["busy_ticks"], default=(None, None)
    )[0]

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
        "walk_stall_causes": dict(walk_stall_causes.most_common()),
        "repeated_walk_failures": [
            {"bot": b, "to": [x, y], "count": c}
            for (b, x, y), c in repeat_failures.most_common()
            if c >= 2
        ],
        "placements": dict(placements.most_common()),
        "placed_by_action": dict(place_ok.most_common()),
        "place_failures": dict(place_failed.most_common()),
        "idle_gaps": idle_gaps(w, events, joined, critical),
    }


def idle_gaps(w: Window, events: list[dict], joined: list[dict], bot: int | None) -> dict | None:
    """Partition one bot's window into merged busy intervals and the gaps between.

    Busy is every dispatch->settle interval for that bot, actions and walks
    alike, overlaps merged; a gap is what is left. The two sum to the span
    exactly, which is the point: it turns "39% of the milestone is unaccounted
    for" into a list of waits with a name on each.

    A gap is attributed to the action dispatched at the tick it *ends* -- what
    the bot was waiting for, not what it had just finished.

    ZERO-LENGTH INTERVALS ARE KEPT. ``place``/``insert``/``take``/``fuel``
    settle in the tick they dispatch, so as intervals they are points, and
    dropping a point for having no width runs the gap past the very action the
    bot was waiting for and onto whatever it did next. In
    ``run-1788459085-32452`` that reported "waited 12,246 ticks for
    ``craft 3 pipe``" in place of "waited 12,244 for
    ``take 50 iron-plate from the cell``" -- the right magnitude blamed on the
    wrong step.
    """
    if bot is None or not w.span:
        return None
    spans = [(j["start"], j["tick"]) for j in joined if j.get("bot") == bot and "start" in j]
    spans += [
        (e["tick"] - (e.get("elapsed_ticks") or 0), e["tick"])
        for e in events
        if e.get("kind") == "walk_settled" and e.get("bot") == bot
    ]
    clipped = sorted(
        (max(a, w.lo), min(b, w.hi)) for a, b in spans if max(a, w.lo) <= min(b, w.hi)
    )
    merged: list[list[int]] = []
    for a, b in clipped:
        if merged and a <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], b)
        else:
            merged.append([a, b])

    # What a dispatch at tick T was; walks are the fallback because a walk's
    # busy interval starts at settle-minus-elapsed, not at its dispatch tick.
    labels = {
        e["tick"]: (e.get("action") or "?")
        for e in events
        if e.get("kind") == "action_dispatched" and e.get("bot") == bot
    }
    for e in events:
        if e.get("kind") == "walk_dispatched" and e.get("bot") == bot:
            to = e.get("to") or {}
            labels.setdefault(e["tick"], f"walk to [{to.get('x')}, {to.get('y')}]")

    gaps = []
    edges = [(w.lo, w.lo)] + merged + [(w.hi, w.hi)]
    for (_, end), (start, _) in zip(edges, edges[1:]):
        if start > end:
            gaps.append(
                {
                    "ticks": start - end,
                    "from_tick": end,
                    "to_tick": start,
                    "waiting_for": labels.get(start),
                }
            )
    gaps.sort(key=lambda g: (-g["ticks"], g["from_tick"]))
    busy = sum(b - a for a, b in merged)
    by_verb = collections.Counter()
    for g in gaps:
        by_verb[verb_of(g["waiting_for"])] += g["ticks"]
    return {
        "bot": bot,
        "busy_ticks": busy,
        "idle_ticks": w.span - busy,
        "idle_pct": 100.0 * (w.span - busy) / w.span,
        "gaps": len(gaps),
        "idle_ticks_by_waited_verb": dict(by_verb.most_common()),
        # Truncated, but the tail is stated rather than dropped: the shape of
        # this list is a handful of long waits and a long tail of ~10-tick
        # dispatch overhead, and conflating the two is the whole error.
        "top_gaps": gaps[:GAP_ROWS],
        "tail_gaps": max(0, len(gaps) - GAP_ROWS),
        "tail_ticks": sum(g["ticks"] for g in gaps[GAP_ROWS:]),
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


# Machines listed individually in the report before the tail is rolled up.
MACHINE_ROWS = 24

# Entity types that are storage, not production. Sampled with the machines
# because an empty input chest is the commonest reason a working cell stops,
# but scored separately: a chest has no `status` and can never be `working`.
CONTAINER_TYPES = ("container", "logistic-container")


def machines_at(samples: list[dict]) -> dict | None:
    """Per-machine verdicts, from the ``machines`` sample kind (schema 2+).

    This is the half of the record that answers "is this machine actually
    working?". Before it existed the archive held bot inventories and
    force-wide totals and nothing in between, so a run that placed a cell, set
    both recipes, charged the chests three times and produced nothing could not
    say whether the assemblers had power, held ingredients, or ever ran.

    The verdict per machine is ``LuaEntity.status`` *over the whole run*, not
    at the end. A machine that spent 40 samples ``no_ingredients`` and then
    1,200 ``working`` is a different animal from one that never left
    ``no_power``, and the final sample cannot tell them apart -- both may read
    ``working`` at the last tick, or neither. So every status a machine was
    ever seen in is counted, and ``products_finished`` (lifetime completed
    crafts, monotonic) settles whether it ever actually produced.

    When there are no ``machines`` lines at all, returns ``{"absent": ...}``
    rather than ``None``, carrying the highest sample schema the file actually
    contains. Three different facts hide behind "no machine rows" and the
    report must not merge them:

    * every line is schema 1 -- the run predates this data. We never looked.
    * lines are schema 2 but none is a ``machines`` line -- the sampler ran and
      wrote nothing, or the stream was cut before the first 300-tick beat.
    * no samples at all -- handled by the caller, before this is reached.

    Reporting the first wording for the second case would be a lie of exactly
    the kind this tool exists to catch, and the second case is a live
    suspicion in this project (sampling has been seen to stop mid-run).
    """
    rows = [s for s in samples if s.get("kind") == "machines"]
    if not rows:
        return {
            "absent": True,
            "schema_seen": max((s.get("schema") or 0) for s in samples) if samples else 0,
            "sample_lines": len(samples),
        }

    seen: dict[str, dict] = {}
    truncated = 0
    for s in rows:
        truncated = max(truncated, s.get("truncated") or 0)
        for key, m in (s.get("machines") or {}).items():
            entry = seen.get(key)
            if entry is None:
                entry = seen[key] = {
                    "name": m.get("name"),
                    "type": m.get("type"),
                    "position": m.get("position"),
                    "statuses": collections.Counter(),
                    "recipes": set(),
                    "networks": set(),
                    "products_finished": 0,
                    "ever_crafting": False,
                    "samples": 0,
                    # For a chest, the count that matters: how much of the run
                    # it spent with nothing in it.
                    "empty_samples": 0,
                    "last_tick": s.get("tick"),
                }
            entry["samples"] += 1
            entry["last_tick"] = s.get("tick")
            entry["statuses"][m.get("status") or "unreported"] += 1
            if m.get("recipe"):
                entry["recipes"].add(m["recipe"])
            if m.get("network") is not None:
                entry["networks"].add(m["network"])
            # Monotonic, so max is the run total even across a dropped sample.
            entry["products_finished"] = max(
                entry["products_finished"], m.get("products_finished") or 0
            )
            entry["ever_crafting"] = entry["ever_crafting"] or bool(m.get("crafting"))
            if not (m.get("output") or m.get("input") or m.get("fuel")):
                entry["empty_samples"] += 1
            entry["last"] = m

    # Every sub-network the last force sample knew about, and what it
    # generated. A machine whose `network` is in none of these sits on an
    # island no *pole* reaches -- `power.networks` is enumerated from poles, so
    # that is a real finding and not a gap in the sampling.
    force = [s for s in samples if s.get("kind") == "force"]
    networks = {}
    if force:
        networks = ((force[-1].get("power") or {}).get("networks")) or {}
    sub_to_net = {}
    for net_key, net in networks.items():
        for sub_id in net.get("sub_ids") or []:
            sub_to_net[sub_id] = net_key

    machines = []
    containers = []
    for key, entry in seen.items():
        statuses = entry["statuses"]
        # "Worked" means the game said so, or the counter moved. Either alone
        # is enough: a machine can finish a craft between two samples without
        # ever being caught mid-craft, and `working` can be true for a machine
        # whose output is blocked before anything completes.
        worked = statuses.get("working", 0) > 0 or entry["products_finished"] > 0
        nets = sorted(entry["networks"])
        # A chest is sampled alongside the machines because an empty one is
        # why a working cell stops, but it has no `status` and can never be
        # `working` -- so scoring it as "never produced" would file every
        # chest in the run under the same heading as a dead assembler.
        if entry["type"] in CONTAINER_TYPES:
            last = entry.get("last") or {}
            containers.append(
                {
                    "key": key,
                    "name": entry["name"],
                    "position": entry["position"],
                    "samples": entry["samples"],
                    "empty_samples": entry["empty_samples"],
                    "last_contents": last.get("output") or {},
                }
            )
            continue
        machines.append(
            {
                "key": key,
                "name": entry["name"],
                "type": entry["type"],
                "position": entry["position"],
                "samples": entry["samples"],
                "statuses": dict(statuses.most_common()),
                "dominant_status": statuses.most_common(1)[0][0] if statuses else None,
                "recipes": sorted(entry["recipes"]),
                "networks": nets,
                # `None` and `[]` differ: no network id at all versus one that
                # matched nothing. The second is the island; the first is a
                # machine that is not electric, or is wired to nothing.
                "orphan_networks": [n for n in nets if n not in sub_to_net],
                "products_finished": entry["products_finished"],
                "worked": worked,
            }
        )
    machines.sort(key=lambda m: (m["worked"], m["type"] or "", m["name"] or "", m["key"]))
    return {
        "samples": len(rows),
        "first_tick": rows[0].get("tick"),
        "last_tick": rows[-1].get("tick"),
        "truncated": truncated,
        "machines": machines,
        "containers": sorted(containers, key=lambda c: -c["empty_samples"]),
        "networks": networks,
        "idle": [m for m in machines if not m["worked"]],
    }


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
    rate = a.get("tick_rate") or {}
    if rate.get("delivered_tps") is not None:
        nominal = rate.get("nominal_tps")
        against = (
            f" of {nominal:.0f} nominal ({rate['ratio'] * 100:.0f}%)"
            if nominal else " (nominal unknown: no game_speed in provenance)"
        )
        flag = "  ! STARVED -- the server was not keeping up" if rate.get("starved") else ""
        p(f"  delivered tick rate: {rate['delivered_tps']:.0f} tps{against} over "
          f"{rate['intervals']} heartbeat interval(s), {rate['ticks']} ticks in "
          f"{rate['ms'] / 1000:.0f} s{flag}")
    planning = a.get("planning") or []
    if planning:
        total_ms = sum(r.get("planning_ms") or 0 for r in planning)
        charged = [r["charged"] for r in planning if r["charged"] is not None]
        unpaused = sum(1 for r in planning if not r["paused"])
        charged_s = (
            f"{sum(charged)} tick(s) charged to the run" if len(charged) == len(planning)
            else f"{sum(charged)} tick(s) charged over {len(charged)} of them, the rest unknown"
        )
        reasons = sorted({r["reason"] for r in planning if not r["paused"] and r.get("reason")})
        clock = "clock stopped for all" if not unpaused else (
            f"clock RUNNING for {unpaused}" + (f": {'; '.join(reasons)}" if reasons else "")
        )
        p(f"  planning: {len(planning)} plan(s), {total_ms / 1000:.1f} s wall, "
          f"{charged_s} ({clock})")

    j = a["join"]
    if j["orphan_settles"] or j["never_settled"] or j["null_duration"]:
        p(f"  join: {j['orphan_settles']} settle(s) with no dispatch, "
          f"{j['never_settled']} dispatch(es) that never settled, "
          f"{j['null_duration']} settle(s) with no recorded duration "
          f"({j['derived_duration']} recovered from tick difference)")

    for row in a.get("savepoints") or []:
        if row["error"]:
            p(f"  ! milestone {row['milestone']} was NOT saved: {row['error']}")
        elif not row["on_disk"]:
            p(f"  ! milestone {row['milestone']} recorded a savepoint at {row['file']}, "
              f"which is no longer on disk")
        else:
            # `or 0` on both numbers: this tool reads archived files, some of
            # them truncated mid-line, and a report that raises on a malformed
            # event tells the reader nothing about the run at all.
            p(f"  savepoint m{row['milestone']}: {row['file']} "
              f"({(row['bytes'] or 0) / 1e6:.1f} MB, "
              f"{(row['wrote_ms'] or 0) / 1000:.1f} s) "
              f"-- resume with --resume-from {a['run_id']}:{row['milestone']}")

    for d in a.get("splits_disagreements") or []:
        p(f"  ! splits.json says milestone {d['index']} took {d['splits']} ticks; "
          f"events say {d['events']}")

    # Said high up, because "the record just stops" is what a reader is trying
    # to interpret when they open one of these at all, and the answer changed
    # two runs' diagnoses. It is only said when the run has no `run_finished`:
    # a run that closed cleanly has nothing to explain here.
    last_plan = (a["plans"] or [None])[-1]
    if a["outcome"].startswith("OPEN") and last_plan:
        ex = last_plan.get("execution") or {}
        beat = ex.get("last_beat") or {}
        if ex.get("verdict") == "never_dispatched":
            p(f"  ! this run ends on a plan of {last_plan['steps']} step(s) that dispatched "
              f"NOTHING -- {ex['beats']} heartbeat(s) counted zero")
        elif ex.get("verdict") == "cut_short":
            p(f"  ! this run stopped while a batch was executing: "
              f"{beat.get('dispatched')}/{beat.get('total')} dispatched at the last "
              f"heartbeat, {beat.get('elapsed_ms', 0) / 1000:.0f}s in")
        elif ex.get("verdict") == "unknown":
            p(f"  ! this run ends on a plan_created (m{last_plan['milestone']}, "
              f"{last_plan['steps']} steps) with nothing after it, and no heartbeat.")
            p("    That is NOT evidence of a stall: per-action lines are written only "
              "when a batch")
            p("    finishes, so a run killed mid-batch looks exactly like this. Compare "
              "the mod's")
            p("    workspace/server/script-output/botbridge/samples.jsonl, which keeps "
              "writing either way.")

    # Said before anything derived from the samples is printed, because
    # everything below that is derived from them -- FROZEN BOTS and PRODUCTION
    # both read the stream, and both describe only the part of the run it
    # covers while looking like they describe the run.
    cov = a.get("samples_coverage") or {}
    verdict = cov.get("verdict")
    if verdict == "no_samples":
        p("  ! no samples archived -- power, production and bot positions are "
          "unavailable for this run")
    elif verdict in ("short", "late"):
        p(f"  ! samples do NOT cover this run: it spans ticks {cov['run_lo']} -> "
          f"{cov['run_hi']}, and")
        for kind, k in cov["by_kind"].items():
            p(f"      {kind:<6} {k['count']:>6} samples, ticks {k['first_tick']} -> "
              f"{k['last_tick']}  (starts {k['lead_ticks']} late, stops "
              f"{k['lag_ticks']} early)")
        missed = cov.get("worst_lag_ticks") or 0
        if missed:
            p(f"      the last {missed} ticks ({minutes(missed)}) of this run were never "
              "sampled or never archived;")
            p("      nothing below that reads samples describes them")

    p(hr("  FREE VISION  (ground the model was given without a bot going there)"))
    v = a.get("vision") or {}
    if not v.get("present"):
        p("    UNKNOWN -- this run recorded no vision_measured event.")
        p("    Every run before 2026-09-04 is in this state, as is any run whose recorder")
        p("    was never handed a world model. It is NOT evidence that the run had no free")
        p("    vision: on_chunk_generated ingest has been in every run this project has made.")
    else:
        model = v.get("model_tiles")
        if model is None:
            p("    model reach:  NOTHING READ -- the world model held no resource tile and no")
            p(f"                  enemy structure at the last measurement (tick {v.get('tick')}).")
        else:
            at = v.get("model_furthest_position") or {}
            where = f"[{at.get('x')}, {at.get('y')}]" if at else "position unrecorded"
            p(f"    model reach:  {model:8.1f} tiles  ({v.get('model_furthest')} at {where})")
            p(f"                  {v.get('model_resource_tiles')} resource tile(s), "
              f"{v.get('model_enemy_structures')} enemy structure(s) known")
        travelled = v.get("travelled_tiles")
        if travelled is None:
            samples = v.get("bot_samples") or 0
            p(f"    bot travel:   UNKNOWN -- {samples} bot position(s) archived.")
            p("                  This is NOT 'no bot moved': it is 'nothing observed one'.")
            p("                  Travel comes from samples.jsonl, so a run whose sampling")
            p("                  never started reports null here for its whole length.")
        else:
            p(f"    bot travel:   {travelled:8.1f} tiles  (bot {v.get('travelled_bot')} at tick "
              f"{v.get('travelled_at_tick')}, from {v.get('bot_samples')} bot position(s))")
        ratio = v.get("unearned_ratio")
        if ratio is None:
            p("    ratio:        n/a -- one half is unknown, or no bot left the origin.")
        else:
            p(f"    ratio:        {ratio:8.1f}x -- the model saw {ratio:.1f} times further than "
              f"the furthest bot went.")
            p("                  Ore, water and nests out there were free. A run quoted as a")
            p("                  measured result carries this as an asterisk; nothing here")
            p("                  decides how big an asterisk it is.")
        p(f"    ({v.get('beats')} measurement(s) in the log; this is the last one)")

    p(hr("  BOT DEATHS AND ROSTER CHANGES"))
    d = a.get("deaths") or {}
    if not d.get("present"):
        nc = d.get("no_character_failures", 0)
        p("    none recorded. Either no bot died, or this run's build predates the")
        p("    bot_died / bot_respawned / roster_changed events (2026-09-04) -- and the")
        p("    record does not say which. On the older build a dead bot was an ordinary")
        p("    stream of failures with no distinguishing kind, and the roster never")
        p("    noticed, so absence here is NOT evidence that no bot died.")
        if nc:
            p(f"    {nc} failure(s) ARE classified no_character, so this build knows the")
            p("    wording; a missing character with no death recorded is a cutscene, a")
            p("    controller switch, or a death the mod did not see.")
    else:
        for r in d["deaths"]:
            pos = r.get("position") or {}
            where = f"[{pos.get('x')}, {pos.get('y')}]" if pos else "position unrecorded"
            cause = f"killed by {r['cause']}" if r.get("cause") else "no cause named by the game"
            back = (f"respawned at tick {r['respawned_tick']} ({r['dead_ticks']} ticks without a character)"
                    if r.get("respawned_tick") is not None
                    else "NO RESPAWN RECORDED (still dead at the end, or the run stopped first)")
            p(f"    bot {r['bot']}  died at tick {r['died_tick']} at {where}, {cause}; "
              f"respawn_in={r.get('respawn_in')}; {back}")
        for c in d["roster_changes"]:
            p(f"    tick {c['tick']:>7}  ROSTER -> {c['bots']}  left={c['left']} returned={c['returned']}")
            p(f"                  {c['reason']}")
        nc = d.get("no_character_failures", 0)
        if nc:
            p(f"    {nc} action/walk failure(s) classified no_character")
            if not d["deaths"]:
                p("    ... with NO death recorded: a character missing for some other reason")
                p("        (cutscene, controller switch), or a death the mod did not see.")

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
        ex = pl.get("execution") or {}
        verdict = ex.get("verdict")
        beat = ex.get("last_beat") or {}
        if verdict == "dispatched":
            p(f"        executed:         yes -- first dispatch at tick "
              f"{ex['first_dispatch_tick']}, {ex['dispatched']} action(s) and "
              f"{ex['walks']} walk(s) recorded")
        elif verdict == "never_dispatched":
            # The only place this tool says a plan was not executed, and it
            # says it only because a heartbeat looked and counted zero.
            p(f"        executed:         NO -- {ex['beats']} heartbeat(s) over "
              f"{beat.get('elapsed_ms', 0) / 1000:.0f}s and NOTHING was ever dispatched "
              f"from this plan")
            # Why nothing dispatched is a different question from whether it
            # did, and this is the only place the record answers it.
            for line in waiting_lines(beat):
                p(line)
        elif verdict == "cut_short":
            p(f"        executed:         started, then the record stops -- last heartbeat "
              f"{beat.get('elapsed_ms', 0) / 1000:.0f}s in had "
              f"{beat.get('dispatched')}/{beat.get('total')} dispatched, "
              f"{beat.get('settled')} settled, {beat.get('in_flight')} in flight "
              f"(bots {beat.get('bots_in_flight')}), {beat.get('walks_dispatched')} walk(s)")
            p("                          the batch was running when the run stopped; "
              "its per-action lines are only written when it finishes")
            # The identities the counters above cannot carry. For a run that
            # died mid-batch this is the only record of what it was on.
            for line in waiting_lines(beat):
                p(line)
        else:
            p("        executed:         UNKNOWN -- no dispatches and no heartbeat. "
              "Either this build")
            p("                          predates batch_progress or the run stopped "
              "inside the first interval;")
            p("                          this record cannot say whether the plan ran.")

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
        if w.get("walk_stall_causes"):
            p("      what the mod found at the blocked tile (stalls only):")
            for cause, n in w["walk_stall_causes"].items():
                p(f"        {cause:<44} x{n}")
        if w["repeated_walk_failures"]:
            p("      repeated (bot, destination) failures -- the same site re-selected:")
            for r in w["repeated_walk_failures"]:
                p(f"        bot {r['bot']} -> [{r['to'][0]}, {r['to'][1]}]  x{r['count']}")

        g = w.get("idle_gaps")
        if g:
            p(f"\n    idle gaps of bot {g['bot']} (the busiest bot -- its timeline IS this window):")
            p(f"      busy {g['busy_ticks']} + idle {g['idle_ticks']} = {span} ticks; "
              f"idle is {g['idle_pct']:.1f}% of span across {g['gaps']} gap(s)")
            p(f"      {'ticks':>7} {'window':>19}  waiting for")
            for e in g["top_gaps"]:
                at = "{}->{}".format(e["from_tick"], e["to_tick"])
                what = e["waiting_for"] or "(nothing -- the window ended here)"
                p(f"      {e['ticks']:>7} {at:>19}  {what[:52]}")
            if g["tail_gaps"]:
                tail = f"({g['tail_gaps']} shorter gaps)"
                p(f"      {g['tail_ticks']:>7} {tail:>19}  dispatch overhead and other")
            p(f"      idle by the verb waited for: {g['idle_ticks_by_waited_verb']}")

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

    mach = a.get("machines")
    if mach and mach.get("absent"):
        p(hr("  MACHINES"))
        if mach["schema_seen"] < 2:
            p(f"    no `machines` samples: every one of this run's {mach['sample_lines']} "
              f"sample lines is schema {mach['schema_seen']},")
            p("    written before per-machine state existed. That is 'we never looked',")
            p("    NOT 'every machine was fine'.")
        else:
            p(f"    !! this run wrote schema {mach['schema_seen']} samples, which DO carry "
              f"per-machine state,")
            p("    but not one `machines` line is present. The sampler either never ran or")
            p("    stopped before its first 300-tick beat -- a defect in the recording, not")
            p("    a finding about the machines.")
    elif mach:
        p(hr("  POWER PER NETWORK  (at the last force sample)"))
        if not mach["networks"]:
            p("    none recorded -- the force owned no electric poles, or this")
            p("    run predates the per-network split.")
        for net_key, net in sorted(mach["networks"].items()):
            on_it = [
                m for m in mach["machines"]
                if any(n in (net.get("sub_ids") or []) for n in m["networks"])
            ]
            flag = ""
            if net.get("generated_kw", 0) <= 0 and net.get("demanded_kw", 0) > 0:
                flag = "   <-- DEMAND WITH NO GENERATION"
            elif net.get("satisfaction", 1.0) < 0.95:
                flag = "   <-- BROWNING OUT"
            p(f"    network {net_key} (sub {net.get('sub_ids')})  "
              f"gen={net.get('generated_kw', 0):.0f}kW  "
              f"used={net.get('consumed_kw', 0):.0f}kW  "
              f"demand={net.get('demanded_kw', 0):.0f}kW  "
              f"satisfaction={net.get('satisfaction', 0):.2f}  "
              f"machines={len(on_it)}{flag}")
        p("    A force-wide total cannot show this: 900 kW generated is still")
        p("    900 kW when every one of them is on the network the cell is NOT on.")

        p(hr("  MACHINES  (status over the whole run, not at the end)"))
        worked = [m for m in mach["machines"] if m["worked"]]
        p(f"    {mach['samples']} machine samples, ticks {mach['first_tick']} -> "
          f"{mach['last_tick']};  {len(mach['machines'])} machines seen, "
          f"{len(worked)} of them worked at some point")
        if mach["truncated"]:
            p(f"    !! {mach['truncated']} machine(s) exceeded the mod's per-sample cap "
              f"and were never sampled")
        if not mach["idle"]:
            p("    every machine worked at least once")
        else:
            p(f"\n    NEVER PRODUCED ({len(mach['idle'])}) -- the game's own reason, counted "
              f"across every sample:")
            for m in mach["idle"][:MACHINE_ROWS]:
                pos = m["position"] or {}
                recipe = ", ".join(m["recipes"]) or "no recipe ever set"
                p(f"      {(m['name'] or '?'):<22} [{pos.get('x')}, {pos.get('y')}]  "
                  f"recipe: {recipe}")
                p(f"        status: {m['statuses']}   over {m['samples']} samples")
                if m["orphan_networks"]:
                    p(f"        !! electric network {m['orphan_networks']} matches no network "
                      f"any pole reached -- an isolated island")
                elif not m["networks"] and m["type"] in ("assembling-machine", "lab"):
                    p("        !! connected to NO electric network at all -- the pole "
                      "did not reach it")
            if len(mach["idle"]) > MACHINE_ROWS:
                p(f"      ... and {len(mach['idle']) - MACHINE_ROWS} more")

        # A machine that produced *and then stopped* is invisible in the two
        # lists above: it is not "never produced", and its final sample may
        # read `working`. It is also the commonest real outcome -- the cell in
        # `run-1788459085-32452` made about 4 science per charge and then sat
        # `no_ingredients` for ~76,000 ticks. So the run-long status counter is
        # printed for anything that spent a fifth of its life not working.
        stalled = [
            m for m in worked
            if m["statuses"].get("working", 0) < 0.8 * m["samples"]
        ]
        if stalled:
            p(f"\n    WORKED BUT STALLED ({len(stalled)}) -- produced, then spent a fifth "
              f"or more of the run not working:")
            for m in stalled[:MACHINE_ROWS]:
                pos = m["position"] or {}
                pct = 100.0 * m["statuses"].get("working", 0) / m["samples"]
                p(f"      {(m['name'] or '?'):<22} [{pos.get('x')}, {pos.get('y')}]  "
                  f"working {pct:.0f}% of samples, finished {m['products_finished']}")
                p(f"        status: {m['statuses']}")

        if mach.get("containers"):
            p(hr("  BUFFERS  (how much of the run each chest spent empty)"))
            p("    A cell with power, a recipe and ingredients still stops when its input")
            p("    chest runs dry, and that reads as an idle machine, not a broken one.")
            for c in mach["containers"][:MACHINE_ROWS]:
                pos = c["position"] or {}
                pct = 100.0 * c["empty_samples"] / c["samples"] if c["samples"] else 0.0
                contents = ", ".join(f"{k}={v}" for k, v in (c["last_contents"] or {}).items())
                p(f"      {(c['name'] or '?'):<18} [{pos.get('x')}, {pos.get('y')}]  "
                  f"empty for {pct:.0f}% of the run ({c['empty_samples']}/{c['samples']} "
                  f"samples);  last held: {contents or 'nothing'}")

        by_status: collections.Counter = collections.Counter()
        for m in mach["machines"]:
            by_status[m["dominant_status"]] += 1
        p(f"\n    machines by dominant status: {dict(by_status.most_common())}")
        finished = {
            f"{m['name']}@[{(m['position'] or {}).get('x')}, {(m['position'] or {}).get('y')}]":
            m["products_finished"]
            for m in worked if m["products_finished"]
        }
        if finished:
            top_finished = sorted(finished.items(), key=lambda kv: -kv[1])[:MACHINE_ROWS]
            p(f"    products finished: {dict(top_finished)}")
    p("")


def summary_line(a: dict) -> str:
    if a.get("error"):
        return f"{a['run_id']}  ! {a['error']}"
    parts = [f"{a['run_id']}", f"{minutes(a['span_ticks'])}", f"{a['outcome'][:22]:<22}"]
    # Carried into the one-line form too: `--all --summary` is how a whole
    # archive gets scanned, and a truncated sample stream is exactly the kind
    # of defect nobody goes looking for run by run.
    last_plan = (a["plans"] or [None])[-1]
    if a["outcome"].startswith("OPEN") and last_plan:
        verdict = (last_plan.get("execution") or {}).get("verdict")
        if verdict == "never_dispatched":
            parts.append("!dispatched-nothing")
        elif verdict == "cut_short":
            parts.append("!killed-mid-batch")
        elif verdict == "unknown":
            parts.append("?ends-on-a-plan")
    # The free-vision asterisk, in the one-line form too: `--all --summary` is
    # how a whole archive gets scanned, and a run's number should not be read
    # off a list without it. `vision=?` is a run that never measured, which is
    # every run before 2026-09-04 -- unknown, not zero.
    # Deaths in the one-line form: `deaths=N` only when there were any, and
    # `!roster-changed` when the supervisor dropped or re-added a bot -- a run
    # whose roster shrank is not comparable to one whose did not, and this is
    # where somebody scanning the archive would otherwise not see it.
    d = a.get("deaths") or {}
    if d.get("deaths"):
        parts.append(f"deaths={len(d['deaths'])}")
    if d.get("roster_changes"):
        parts.append("!roster-changed")
    v = a.get("vision") or {}
    if not v.get("present"):
        parts.append("vision=?")
    elif v.get("unearned_ratio") is not None:
        parts.append(f"vision={v['unearned_ratio']:.0f}x")
    else:
        parts.append("vision=n/a")
    cov = a.get("samples_coverage") or {}
    if cov.get("verdict") == "no_samples":
        parts.append("!nosamples")
    elif cov.get("verdict") in ("short", "late"):
        parts.append(f"!samples-short-by-{minutes(cov.get('worst_lag_ticks') or 0).strip()}")
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


# --------------------------------------------------------------------------
# comparing two runs
# --------------------------------------------------------------------------
#
# WHY THIS MODE CAN REFUSE
# ------------------------
# Reading two of the reports above side by side is what a person does after
# changing something, and doing it by eye produced several retracted
# conclusions in a single day. The worst of them is the reason this mode has
# an exit code: two runs recorded thirteen hours and about twenty commits
# apart, on different maps, were read side by side as though the only
# difference between them was the change under test, and every difference in
# the numbers was attributed to that change.
#
# So the numbers here are printed *under* a comparability block, never beside
# it, and a comparison that cannot be controlled is refused rather than
# footnoted. The three rules the block encodes:
#
# * A DIFFERENT SEED, COMMIT, GAME VERSION, BUILD PROFILE OR RESUMED SAVE IS A
#   REFUSAL, not a caveat. A caveat at the bottom of a report is a caveat
#   nobody reads. A different MAP FINGERPRINT is not on that list and must not
#   be: the digest covers charted tiles only and grows as bots explore, so two
#   runs on one map diverge honestly -- see `PROVENANCE_SEVERITY`.
# * ABSENT PROVENANCE IS UNKNOWN, never a match. Every run archived when this
#   was written records `null` for seed, git and factorio and carries no map
#   fingerprint at all; saying "same seed" about two nulls would manufacture
#   the exact confidence this exists to withhold.
# * AN INCOMPLETE RUN IS NAMED IN `batch_execution`'s OWN VOCABULARY --
#   `dispatched`, `never_dispatched`, `cut_short`, `unknown` -- and `unknown`
#   is reported as "this record cannot say", never as a stall. Inferring a
#   stall from "a plan with nothing after it" is itself one of the retracted
#   conclusions.


def _milestone_index(label: str) -> int | None:
    m = re.match(r"m(\d+)\b", label or "")
    return int(m.group(1)) if m else None


def _milestone_windows_by_index(a: dict) -> dict[int, list[dict]]:
    """The scored windows of one analysis, grouped by milestone index.

    A *list* per index, not a single window: ``milestone_windows`` pairs by
    index precisely because a milestone can be re-entered, so two windows for
    ``m2`` are two spans that must be summed, not one correcting the other.
    """
    out: dict[int, list[dict]] = collections.defaultdict(list)
    for w in (a.get("windows") or [])[1:]:
        idx = _milestone_index(w.get("label", ""))
        if idx is not None:
            out[idx].append(w)
    return dict(out)


def _whole(a: dict) -> dict:
    """The whole-run window, or an empty one for a run with nothing in it."""
    windows = a.get("windows") or []
    return windows[0] if windows else {}


def _last_execution(a: dict) -> dict:
    """``batch_execution``'s entry for the last plan of a run, or ``{}``."""
    plans = a.get("plans") or []
    return (plans[-1].get("execution") or {}) if plans else {}


def _num(va: Any, vb: Any) -> dict:
    """One comparable figure: A, B, and B-A when both are actually numbers."""
    delta = None
    if isinstance(va, (int, float)) and isinstance(vb, (int, float)):
        delta = vb - va
    return {"a": va, "b": vb, "delta": delta}


def _cell(v: Any) -> str:
    if v is None:
        return "n/a"
    if isinstance(v, float):
        return f"{v:.1f}"
    return str(v)


def _delta_cell(v: Any) -> str:
    if v is None:
        return ""
    if isinstance(v, float):
        return f"{v:+.1f}"
    return f"{v:+d}"


def comparability(a: dict, b: dict) -> list[dict]:
    """Every reason these two runs might not be measuring the same thing.

    Each guard is ``{"id", "severity", "headline", "detail"}`` with severity
    one of:

    ``refuse``   the comparison is not controlled and the numbers are withheld
                 (``--force`` overrides, and says so in the output).
    ``flag``     the runs differ in a way that changes what the numbers mean.
    ``unknown``  the record cannot establish whether they are comparable. NOT
                 the same as ``ok`` and never rendered as one.
    ``ok``       checked, and the two agree.

    Printed above the numbers in that order of severity. Ordering matters:
    the guard that would have prevented the worst error of the day is the one
    a reader must hit before the first figure, not after the last.
    """
    guards: list[dict] = []

    for name, r in (("A", a), ("B", b)):
        if r.get("error"):
            guards.append(
                {
                    "id": "readable",
                    "severity": "refuse",
                    "headline": f"run {name} ({r['run_id']}) cannot be read: {r['error']}",
                    "detail": [
                        "There is nothing here to compare. Refused rather than reported as",
                        "zeroes, because zero is a measurement and 'never recorded' is not.",
                    ],
                }
            )
    if guards:
        # Nothing below can say anything about a run with no events: every
        # other guard would read `.get()` off an empty analysis and report
        # "different outcome: None vs stuck", which is an artefact of the
        # missing file and not a fact about the runs.
        return guards

    # --- 1. provenance: same world, same build? -------------------------
    pa = a.get("provenance") or {}
    pb = b.get("provenance") or {}

    def value(prov: dict, field: str) -> str | None:
        return (prov.get(field) or {}).get("value")

    def line(field: str, mark: str) -> str:
        va, vb = value(pa, field), value(pb, field)
        return (
            f"{field:<13} A={'null/absent' if va is None else va}"
            f"   B={'null/absent' if vb is None else vb}   {mark}"
        )

    # The refusal-grade fields, judged together: one difference among them is
    # enough, and every one of them is listed so the reader sees which.
    hard = [f for f in PROVENANCE_FIELDS if PROVENANCE_SEVERITY[f] == "refuse"]
    differing = [f for f in hard if None not in (value(pa, f), value(pb, f))
                 and value(pa, f) != value(pb, f)]
    unknown = [f for f in hard if None in (value(pa, f), value(pb, f))]
    agreeing = [f for f in hard if f not in differing and f not in unknown]
    dirty = [
        name for name, prov in (("A", pa), ("B", pb)) if prov.get("git_dirty")
    ]
    detail = (
        [line(f, "<-- DIFFERENT") for f in differing]
        + [line(f, "<-- unknown") for f in unknown]
        + [line(f, "") for f in agreeing]
    )
    if dirty:
        detail.append(
            f"git in {' and '.join(dirty)} ran from a DIRTY working tree: the commit "
            "does not identify what ran"
        )
    if differing:
        guards.append(
            {
                "id": "provenance",
                "severity": "refuse",
                "headline": (
                    "these two runs were NOT produced under the same conditions "
                    f"({', '.join(differing)} differ)"
                ),
                "detail": detail
                + [
                    "",
                    "Any difference in the numbers would include differences the change",
                    "under test did not make. This is a refusal and not a warning because it",
                    "has already been got wrong: two runs thirteen hours and about twenty",
                    "commits apart, on different maps, were compared as if controlled, and the",
                    "whole difference was credited to the change under test.",
                    "Pass --force if you genuinely mean to compare them anyway.",
                ],
            }
        )
    elif unknown or dirty:
        if unknown:
            headline = "UNKNOWN comparability -- provenance is missing, not matching"
            explain = [
                "A run recorded before provenance existed CANNOT BE SHOWN TO BE",
                "COMPARABLE to anything. That is not the same as being comparable:",
                "absence is unknown, never a match. Every run archived when this was",
                "written records null for seed, factorio and git and has no",
                "provenance.json at all, so a comparison between two of them rests",
                "entirely on the operator remembering what they ran -- which is the",
                "memory that already failed once.",
            ]
        else:
            headline = (
                "UNKNOWN comparability -- every field matches, but a DIRTY working "
                "tree means the commit does not identify what ran"
            )
            explain = [
                "Uncommitted edits are not in the commit id, so two runs at one commit",
                "can have run different code. This is the one case where every recorded",
                "field agrees and the runs still cannot be shown to be comparable.",
            ]
        guards.append(
            {
                "id": "provenance",
                "severity": "unknown",
                "headline": headline,
                "detail": detail + [""] + explain,
            }
        )
    else:
        guards.append(
            {
                "id": "provenance",
                "severity": "ok",
                "headline": f"same {', '.join(agreeing)}",
                "detail": detail,
            }
        )

    # --- 1b. the map, whose inequality is NOT proof of anything ---------
    ma, mb = value(pa, "map"), value(pb, "map")
    if ma is not None and mb is not None and ma == mb:
        guards.append(
            {
                "id": "map",
                "severity": "ok",
                "headline": f"same map fingerprint ({ma})",
                "detail": [],
            }
        )
    elif ma is not None and mb is not None:
        detail_map = [
            f"A={ma}",
            f"B={mb}",
            "The fingerprint is a digest over the CHARTED resource tiles, and charting",
            "grows as bots explore -- so two runs on one map diverge the moment one of",
            "them walks further. Equal digests mean the same map; unequal digests mean",
            "unknown. Flagged rather than refused because refusing here would refuse",
            "nearly every honest pair and teach the reader to type --force by reflex.",
            "The seed is the field that settles it.",
        ]
        if pa.get("map_tiles") or pb.get("map_tiles"):
            # The half a person can actually read: "one map had coal and the
            # other did not" is visible here and nowhere else in a run record.
            detail_map += [
                f"charted tiles A: {pa.get('map_tiles')}",
                f"charted tiles B: {pb.get('map_tiles')}",
            ]
        guards.append(
            {
                "id": "map",
                "severity": "unknown",
                "headline": "map fingerprints DIFFER, which does not establish two maps",
                "detail": detail_map,
            }
        )
    else:
        guards.append(
            {
                "id": "map",
                "severity": "unknown",
                "headline": "map identity unknown for at least one run",
                "detail": [
                    f"A={'null/absent' if ma is None else ma}   "
                    f"B={'null/absent' if mb is None else mb}",
                    "Nothing here says the two runs shared a world. Absence is unknown.",
                ],
            }
        )

    # --- 1c. workspace --------------------------------------------------
    wsa, wsb = value(pa, "workspace"), value(pb, "workspace")
    if wsa is not None and wsb is not None and wsa != wsb:
        guards.append(
            {
                "id": "workspace",
                "severity": "flag",
                "headline": "DIFFERENT WORKSPACE",
                "detail": [
                    f"A={wsa}",
                    f"B={wsb}",
                    "Different workspaces hold different level.zip files and separate copies",
                    "of the mods and scripts, so this is a different world and possibly",
                    "different Lua even at one commit.",
                ],
            }
        )

    # --- 2. roster ------------------------------------------------------
    # `run_started.bots` is what the run GOT. `provenance.json` records what it
    # asked for, and the pair is the only evidence a run degraded -- a four-bot
    # request that obtained one bot reads as an ordinary one-bot run in every
    # other artefact.
    ra, rb = a.get("roster"), b.get("roster")
    for name, r, prov in (("A", a, pa), ("B", b, pb)):
        want = prov.get("roster_requested")
        got = r.get("roster")
        if want is not None and got is not None and list(want) != list(got):
            guards.append(
                {
                    "id": "roster",
                    "severity": "flag",
                    "headline": (
                        f"run {name} DEGRADED: asked for {want!r}, obtained {got!r}"
                    ),
                    "detail": [
                        "Clients that never connected are missing from the run without",
                        "anything else saying so. Every per-bot figure below is over the",
                        "bots it actually had.",
                    ],
                }
            )
    if ra is None or rb is None:
        guards.append(
            {
                "id": "roster",
                "severity": "unknown",
                "headline": f"roster unknown for at least one run (A={ra!r} B={rb!r})",
                "detail": [
                    "No run_started, so nothing says how many bots the run actually got.",
                ],
            }
        )
    elif ra != rb:
        guards.append(
            {
                "id": "roster",
                "severity": "flag",
                "headline": f"DIFFERENT ROSTER: A ran {ra!r}, B ran {rb!r}",
                "detail": [
                    "A run that degraded to one bot is not comparable to a four-bot run.",
                    "steps/bot, planned ticks/bot, walks/bot and fleet utilisation are all",
                    "per-bot figures, and every one of them means something else when the",
                    "denominator changes. `bots: []` is a run that obtained no roster at all.",
                ],
            }
        )
    else:
        guards.append(
            {
                "id": "roster",
                "severity": "ok",
                "headline": f"same roster obtained: {ra!r}",
                "detail": [],
            }
        )

    # --- 3. incomplete runs, in `batch_execution`'s own words ------------
    for name, r in (("A", a), ("B", b)):
        if not str(r.get("outcome", "")).startswith("OPEN"):
            continue
        ex = _last_execution(r)
        verdict = ex.get("verdict")
        beat = ex.get("last_beat") or {}
        plans = r.get("plans") or []
        last = plans[-1] if plans else {}
        head = (
            f"run {name} ({r['run_id']}) has no run_finished and ends on a "
            f"plan_created (m{last.get('milestone')}, {last.get('steps')} steps)"
        )
        if verdict == "cut_short":
            guards.append(
                {
                    "id": "incomplete",
                    "severity": "flag",
                    "headline": f"{head} -- batch_execution says `cut_short`",
                    "detail": [
                        "`cut_short`: heartbeats show dispatches, but the batch's own",
                        "per-action lines never arrived -- the run stopped while it was",
                        f"working. Last heartbeat, {beat.get('elapsed_ms', 0) / 1000:.0f}s in: "
                        f"{beat.get('dispatched')}/{beat.get('total')} dispatched, "
                        f"{beat.get('settled')} settled.",
                        "Whatever this run's totals are, they are the totals of a run that",
                        "was still working when the record stopped. The comparison rests on",
                        "an incomplete run.",
                    ],
                }
            )
        elif verdict == "unknown":
            guards.append(
                {
                    "id": "incomplete",
                    "severity": "flag",
                    "headline": f"{head} -- batch_execution says `unknown`",
                    "detail": [
                        "`unknown`: no heartbeat and no dispatch. Either the build predates",
                        "batch_progress or the run stopped inside the first interval; this",
                        "record cannot say whether the plan ran.",
                        "THIS IS NOT EVIDENCE OF A STALL. Per-action lines are written only",
                        "when a batch finishes, so a run killed mid-batch looks exactly like",
                        "a run that dispatched nothing forever. Reading it as a stall is a",
                        "conclusion that has already been retracted once.",
                        "The comparison rests on an incomplete run.",
                    ],
                }
            )
        elif verdict == "never_dispatched":
            guards.append(
                {
                    "id": "incomplete",
                    "severity": "flag",
                    "headline": f"{head} -- batch_execution says `never_dispatched`",
                    "detail": [
                        "`never_dispatched`: a heartbeat looked and found nothing dispatched",
                        f"at all ({ex.get('beats')} heartbeat(s), counter at zero -- no",
                        "threshold decides this).",
                        "The run's own numbers are real, but it never finished: the",
                        "comparison rests on an incomplete run.",
                    ],
                }
            )
        else:
            guards.append(
                {
                    "id": "incomplete",
                    "severity": "flag",
                    "headline": (
                        f"run {name} ({r['run_id']}) has no run_finished "
                        f"(batch_execution: {verdict or 'no plan to judge'})"
                    ),
                    "detail": [
                        "Its totals are 'so far', not final. The comparison rests on an",
                        "incomplete run.",
                    ],
                }
            )

    # --- 4. outcome -----------------------------------------------------
    oa, ob = a.get("outcome"), b.get("outcome")
    if oa != ob:
        guards.append(
            {
                "id": "outcome",
                "severity": "flag",
                "headline": f"DIFFERENT OUTCOME: A={oa!r}, B={ob!r}",
                "detail": [
                    "A run that finished and a run that got stuck are not two measurements",
                    "of the same thing. 'B was faster' is trivially true of a run that",
                    "stopped early, and every total below is over a different amount of",
                    "attempted work.",
                ],
            }
        )
    else:
        guards.append(
            {"id": "outcome", "severity": "ok", "headline": f"same outcome: {oa}", "detail": []}
        )

    # --- 5. the milestones themselves -----------------------------------
    goals_a = {
        _milestone_index(m["label"]): m["label"].split(" ", 1)[-1]
        for m in a.get("milestones") or []
    }
    goals_b = {
        _milestone_index(m["label"]): m["label"].split(" ", 1)[-1]
        for m in b.get("milestones") or []
    }
    mismatched = [i for i in sorted(set(goals_a) & set(goals_b)) if goals_a[i] != goals_b[i]]
    if mismatched:
        guards.append(
            {
                "id": "goals",
                "severity": "flag",
                "headline": f"milestone {mismatched} pursued a DIFFERENT GOAL in each run",
                "detail": [f"  m{i}: A {goals_a[i]!r}  B {goals_b[i]!r}" for i in mismatched]
                + [
                    "Milestone durations are lined up by index below; where the goal differs",
                    "the two rows are not the same milestone and the delta means nothing.",
                ],
            }
        )
    if set(goals_a) != set(goals_b):
        guards.append(
            {
                "id": "goals",
                "severity": "flag",
                "headline": (
                    f"different milestone sets: A reached m{sorted(goals_a)}, "
                    f"B reached m{sorted(goals_b)}"
                ),
                "detail": [
                    "Only the indices present in both are compared; the rest are listed as",
                    "'A only' / 'B only' and are not deltas.",
                ],
            }
        )
    return guards


def _plan_totals(a: dict) -> tuple[collections.Counter, collections.Counter]:
    """Steps and planned ticks per bot, summed over every plan in the run.

    Summed over plans on purpose: a run replans per milestone, so a single
    plan's steps/bot describes one batch, and it is the run total that answers
    "did the planner spread the work differently this time".
    """
    steps: collections.Counter = collections.Counter()
    work: collections.Counter = collections.Counter()
    for pl in a.get("plans") or []:
        for bot, n in (pl.get("steps_per_bot") or {}).items():
            steps[bot] += n
        for bot, n in (pl.get("planned_work_per_bot") or {}).items():
            work[bot] += n
    return steps, work


def _bot_keys(*dicts: dict) -> list:
    keys: set = set()
    for d in dicts:
        keys |= set(d or {})
    return sorted(keys, key=lambda k: (k is None, k))


def compare_numbers(a: dict, b: dict) -> dict:
    """Every figure the two runs both have, as ``{a, b, delta}`` triples.

    Deltas are ``B - A`` and exist only where both sides are numbers -- a
    delta against a missing figure would be a subtraction against zero, which
    reads as a real change of exactly the wrong size.

    Durations are GAME TICKS throughout, and minutes derived from ticks. Wall
    clock is deliberately absent: the two runs' wall times include Factorio
    start-up, sprite loading and whatever else the host was doing, and the
    game speed of a run is not a property of the change under test.
    """
    wa, wb = _whole(a), _whole(b)

    steps_a, work_a = _plan_totals(a)
    steps_b, work_b = _plan_totals(b)
    plan_rows = []
    for bot in _bot_keys(steps_a, steps_b, work_a, work_b):
        plan_rows.append(
            {
                "bot": bot,
                "steps": _num(steps_a.get(bot), steps_b.get(bot)),
                "planned_ticks": _num(work_a.get(bot), work_b.get(bot)),
            }
        )

    plans_a, plans_b = a.get("plans") or [], b.get("plans") or []
    per_plan = []
    for i in range(max(len(plans_a), len(plans_b))):
        pa = plans_a[i] if i < len(plans_a) else None
        pb = plans_b[i] if i < len(plans_b) else None
        per_plan.append(
            {
                "ordinal": i + 1,
                "a": pa
                and {
                    "tick": pa.get("tick"),
                    "milestone": pa.get("milestone"),
                    "steps": pa.get("steps"),
                    "makespan": pa.get("makespan"),
                    "steps_per_bot": pa.get("steps_per_bot"),
                    "planned_work_per_bot": pa.get("planned_work_per_bot"),
                    "execution": (pa.get("execution") or {}).get("verdict"),
                },
                "b": pb
                and {
                    "tick": pb.get("tick"),
                    "milestone": pb.get("milestone"),
                    "steps": pb.get("steps"),
                    "makespan": pb.get("makespan"),
                    "steps_per_bot": pb.get("steps_per_bot"),
                    "planned_work_per_bot": pb.get("planned_work_per_bot"),
                    "execution": (pb.get("execution") or {}).get("verdict"),
                },
            }
        )

    va, vb = wa.get("verb_ticks") or {}, wb.get("verb_ticks") or {}
    na, nb = wa.get("verbs") or {}, wb.get("verbs") or {}
    verbs = [
        {
            "verb": verb,
            "n": _num(na.get(verb), nb.get(verb)),
            "ticks": _num(va.get(verb), vb.get(verb)),
        }
        for verb in sorted(
            set(va) | set(vb), key=lambda v: -(va.get(v, 0) + vb.get(v, 0))
        )
    ]

    mw_a, mw_b = _milestone_windows_by_index(a), _milestone_windows_by_index(b)

    def span_of(by_idx: dict[int, list[dict]], idx: int) -> int | None:
        ws = by_idx.get(idx)
        return sum(w.get("span_ticks") or 0 for w in ws) if ws else None

    def outcome_of(by_idx: dict[int, list[dict]], idx: int) -> str | None:
        ws = by_idx.get(idx)
        return "; ".join(str(w.get("outcome")) for w in ws) if ws else None

    def goal_of(r: dict, idx: int) -> str | None:
        for m in r.get("milestones") or []:
            if _milestone_index(m["label"]) == idx:
                return m["label"].split(" ", 1)[-1]
        return None

    milestones = []
    for idx in sorted(set(mw_a) | set(mw_b)):
        ticks = _num(span_of(mw_a, idx), span_of(mw_b, idx))
        milestones.append(
            {
                "index": idx,
                "goal_a": goal_of(a, idx),
                "goal_b": goal_of(b, idx),
                "windows_a": len(mw_a.get(idx, [])),
                "windows_b": len(mw_b.get(idx, [])),
                "ticks": ticks,
                "minutes": _num(
                    None if ticks["a"] is None else ticks["a"] / TICKS_PER_MINUTE,
                    None if ticks["b"] is None else ticks["b"] / TICKS_PER_MINUTE,
                ),
                "outcome_a": outcome_of(mw_a, idx),
                "outcome_b": outcome_of(mw_b, idx),
            }
        )

    def placements(ws: list[dict]) -> tuple[collections.Counter, collections.Counter]:
        m: collections.Counter = collections.Counter()
        act: collections.Counter = collections.Counter()
        for w in ws:
            m.update(w.get("placements") or {})
            act.update(w.get("placed_by_action") or {})
        return m, act

    built = []
    scopes: list[tuple[str, list[dict], list[dict]]] = [
        ("whole run", [wa] if wa else [], [wb] if wb else [])
    ]
    for idx in sorted(set(mw_a) | set(mw_b)):
        scopes.append((f"m{idx}", mw_a.get(idx, []), mw_b.get(idx, [])))
    for label, ws_a, ws_b in scopes:
        map_a, act_a = placements(ws_a)
        map_b, act_b = placements(ws_b)
        names = sorted(set(map_a) | set(map_b) | set(act_a) | set(act_b))
        if not names:
            continue
        built.append(
            {
                "label": label,
                "rows": [
                    {
                        "name": n,
                        "map_rows": _num(map_a.get(n), map_b.get(n)),
                        "place_actions": _num(act_a.get(n), act_b.get(n)),
                    }
                    for n in names
                ],
            }
        )

    # ------------------------------------------------------------------
    # WALKS. `per_bot` is built in `score_window` from `walk_settled`'s own
    # `bot` field with NO join to `walk_dispatched`, which has no `id`. The
    # naive join keys every walk under None and hands every failure to
    # whichever bot dispatched last; that is how "all twelve failed walks were
    # bot 1's" was reported about a run in which bot 1 failed none.
    # ------------------------------------------------------------------
    pb_a, pb_b = wa.get("per_bot") or {}, wb.get("per_bot") or {}
    walks = [
        {
            "bot": bot,
            "walks": _num((pb_a.get(bot) or {}).get("walks"), (pb_b.get(bot) or {}).get("walks")),
            "failed": _num(
                (pb_a.get(bot) or {}).get("walk_failed"), (pb_b.get(bot) or {}).get("walk_failed")
            ),
            "lost": _num(
                (pb_a.get(bot) or {}).get("walk_lost"), (pb_b.get(bot) or {}).get("walk_lost")
            ),
            "walk_ticks": _num(
                (pb_a.get(bot) or {}).get("walk_ticks"), (pb_b.get(bot) or {}).get("walk_ticks")
            ),
            "busy_pct": _num(
                (pb_a.get(bot) or {}).get("busy_pct"), (pb_b.get(bot) or {}).get("busy_pct")
            ),
        }
        for bot in _bot_keys(pb_a, pb_b)
    ]

    span = _num(a.get("span_ticks"), b.get("span_ticks"))
    return {
        "shape": {
            "roster": {"a": a.get("roster"), "b": b.get("roster")},
            "outcome": {"a": a.get("outcome"), "b": b.get("outcome")},
            "span_ticks": span,
            "span_minutes": _num(
                None if span["a"] is None else span["a"] / TICKS_PER_MINUTE,
                None if span["b"] is None else span["b"] / TICKS_PER_MINUTE,
            ),
            "events_read": _num(a.get("events_read"), b.get("events_read")),
            "plans": _num(len(plans_a), len(plans_b)),
            "milestones": _num(len(a.get("milestones") or []), len(b.get("milestones") or [])),
            "walks_dispatched": _num(wa.get("walks_dispatched"), wb.get("walks_dispatched")),
        },
        "plan_totals": plan_rows,
        "per_plan": per_plan,
        "verbs": verbs,
        "milestones": milestones,
        "built": built,
        "walks": walks,
    }


def compare(a: dict, b: dict, force: bool = False) -> dict:
    """Guards first, numbers second -- and no numbers at all after a refusal.

    ``--force`` prints them anyway, and the output says it was forced, because
    a forced comparison pasted into a note is indistinguishable from a clean
    one unless the output itself carries the fact.

    An unreadable run withholds the numbers even under ``--force``: there is
    nothing to print.
    """
    guards = comparability(a, b)
    refused = any(g["severity"] == "refuse" for g in guards)
    unreadable = any(g["id"] == "readable" for g in guards)
    out: dict[str, Any] = {
        "a": a.get("run_id"),
        "b": b.get("run_id"),
        "guards": guards,
        "refused": refused,
        "forced": bool(force and refused),
        "numbers_withheld": (refused and not force) or unreadable,
    }
    if not out["numbers_withheld"]:
        out["numbers"] = compare_numbers(a, b)
    return out


SEVERITY_MARK = {
    "refuse": "!! REFUSE ",
    "flag": "!! FLAG   ",
    "unknown": "?? UNKNOWN",
    "ok": "ok        ",
}
SEVERITY_ORDER = {"refuse": 0, "flag": 1, "unknown": 2, "ok": 3}


def report_compare(c: dict, out=sys.stdout, top: int = 12) -> None:
    p = lambda *args: print(*args, file=out)
    a_id, b_id = str(c["a"]), str(c["b"])

    p(f"\n{'=' * 78}")
    p(f"COMPARE   A = {a_id}")
    p(f"          B = {b_id}")
    p(f"{'=' * 78}")

    p(hr("  COMPARABILITY  (read this before any number below)"))
    for g in sorted(c["guards"], key=lambda g: SEVERITY_ORDER.get(g["severity"], 9)):
        p(f"  {SEVERITY_MARK.get(g['severity'], '?')}  {g['headline']}")
        for line in g["detail"]:
            p(f"              {line}".rstrip())
    if c["numbers_withheld"] and any(g["id"] == "readable" for g in c["guards"]):
        p("\n  Nothing to print: one of these runs recorded no events at all.")
        p("  --force does not help here -- there is no other side to the comparison.")
        return
    if c["numbers_withheld"]:
        p("\n  REFUSED: the numbers are not printed, because printing them under this")
        p("  block is how they get quoted without it. Re-run with --force if you mean")
        p("  to compare these two runs anyway.")
        return
    if c["forced"]:
        p("\n  !! FORCED: a REFUSE above was overridden with --force. Every figure below")
        p("  !! includes whatever else changed between these two runs.")

    n = c["numbers"]
    s = n["shape"]

    def row(label: str, va: Any, vb: Any, d: Any = None) -> None:
        p(f"    {label:<24} {_cell(va):>22} {_cell(vb):>22} {_delta_cell(d):>12}")

    p(hr("  RUN SHAPE"))
    p(f"    {'':<24} {('A ' + a_id)[-22:]:>22} {('B ' + b_id)[-22:]:>22} {'B - A':>12}")
    row("roster obtained", s["roster"]["a"], s["roster"]["b"])
    row("outcome", s["outcome"]["a"], s["outcome"]["b"])
    for label, key in (
        ("span (game ticks)", "span_ticks"),
        ("span (game minutes)", "span_minutes"),
        ("events recorded", "events_read"),
        ("plans created", "plans"),
        ("milestones entered", "milestones"),
        ("walks dispatched", "walks_dispatched"),
    ):
        row(label, s[key]["a"], s[key]["b"], s[key]["delta"])
    p("    (game time throughout -- wall clock is never compared here: it includes")
    p("     Factorio start-up and whatever else the host was doing.)")

    p(hr("  PLANNED WORK PER BOT  (summed over every plan in the run)"))
    p(f"    {'bot':>4} {'steps A':>9} {'steps B':>9} {'delta':>8}   "
      f"{'planned tk A':>13} {'planned tk B':>13} {'delta':>9}")
    for r in n["plan_totals"]:
        st, pt = r["steps"], r["planned_ticks"]
        p(f"    {str(r['bot']):>4} {_cell(st['a']):>9} {_cell(st['b']):>9} "
          f"{_delta_cell(st['delta']):>8}   {_cell(pt['a']):>13} {_cell(pt['b']):>13} "
          f"{_delta_cell(pt['delta']):>9}")
    if not n["plan_totals"]:
        p("    no plans in either run")

    if len(n["per_plan"]) > 1 or any(r["a"] is None or r["b"] is None for r in n["per_plan"]):
        p("\n    per plan, aligned by ordinal (NOT by milestone -- two runs that replan a")
        p("    different number of times do not line up, and this says so rather than")
        p("    quietly pairing plan 3 of one with plan 3 of the other):")
        for r in n["per_plan"]:
            for side, key in (("A", "a"), ("B", "b")):
                pl = r[key]
                if pl is None:
                    p(f"      #{r['ordinal']} {side}  -- no such plan in this run")
                    continue
                p(f"      #{r['ordinal']} {side}  tick {pl['tick']:>7} m{pl['milestone']} "
                  f"steps={pl['steps']:<5} makespan={pl['makespan']:<7} "
                  f"{pl['execution'] or 'no verdict'}")
                p(f"           steps/bot {pl['steps_per_bot']}  "
                  f"planned ticks/bot {pl['planned_work_per_bot']}")

    p(hr("  ACTION COST BY VERB  (dispatch -> settle ticks, whole run, summed over bots)"))
    p(f"    {'verb':<12} {'n A':>6} {'n B':>6} {'delta':>7}   "
      f"{'ticks A':>9} {'ticks B':>9} {'delta':>9} {'delta min':>10}")
    for v in n["verbs"]:
        cnt, tk = v["n"], v["ticks"]
        dmin = None if tk["delta"] is None else tk["delta"] / TICKS_PER_MINUTE
        p(f"    {v['verb']:<12} {_cell(cnt['a']):>6} {_cell(cnt['b']):>6} "
          f"{_delta_cell(cnt['delta']):>7}   {_cell(tk['a']):>9} {_cell(tk['b']):>9} "
          f"{_delta_cell(tk['delta']):>9} {_delta_cell(dmin):>10}")
    if not n["verbs"]:
        p("    no actions settled in either run")
    else:
        p("    Verbs that settle in the tick they dispatch contribute 0 ticks in both")
        p("    columns; a 0 delta there means 'never measured', not 'no change'.")

    p(hr("  MILESTONE DURATIONS  (GAME TIME -- ticks and game minutes, never wall clock)"))
    p(f"    {'ms':<4} {'ticks A':>9} {'min A':>7} {'ticks B':>9} {'min B':>7} "
      f"{'delta tk':>9} {'delta min':>10}  goal")
    for m in n["milestones"]:
        tk, mi = m["ticks"], m["minutes"]
        goal = m["goal_a"] or m["goal_b"] or ""
        note = ""
        if m["goal_a"] and m["goal_b"] and m["goal_a"] != m["goal_b"]:
            note = f"   <-- DIFFERENT GOAL: A {m['goal_a']!r} vs B {m['goal_b']!r}"
            goal = ""
        elif tk["a"] is None:
            note = "   <-- B only"
        elif tk["b"] is None:
            note = "   <-- A only"
        if m["windows_a"] > 1 or m["windows_b"] > 1:
            note += f"   (re-entered: {m['windows_a']} window(s) A, {m['windows_b']} B, summed)"
        p(f"    m{m['index']:<3} {_cell(tk['a']):>9} {_cell(mi['a']):>7} "
          f"{_cell(tk['b']):>9} {_cell(mi['b']):>7} {_delta_cell(tk['delta']):>9} "
          f"{_delta_cell(mi['delta']):>10}  {goal[:28]}{note}")
        if m["outcome_a"] != m["outcome_b"]:
            p(f"         outcome A: {m['outcome_a']}")
            p(f"         outcome B: {m['outcome_b']}")
    if not n["milestones"]:
        p("    no milestones recorded in either run")

    p(hr("  ENTITIES BUILT  (map.jsonl keyframes, and what the place actions claimed)"))
    p("    map rows are entities the mod saw exist; place actions are settles the")
    p("    executor called a success. They disagree for ghosts and for placements the")
    p("    game refused, so both are shown rather than one standing in for the other.")
    for scope in n["built"]:
        p(f"    {scope['label']}:")
        p(f"      {'entity':<24} {'map A':>6} {'map B':>6} {'delta':>7}   "
          f"{'act A':>6} {'act B':>6} {'delta':>7}")
        for r in scope["rows"][:top]:
            mr, ac = r["map_rows"], r["place_actions"]
            p(f"      {r['name'][:24]:<24} {_cell(mr['a']):>6} {_cell(mr['b']):>6} "
              f"{_delta_cell(mr['delta']):>7}   {_cell(ac['a']):>6} {_cell(ac['b']):>6} "
              f"{_delta_cell(ac['delta']):>7}")
        if len(scope["rows"]) > top:
            p(f"      ... and {len(scope['rows']) - top} more entity type(s); raise --top")
    if not n["built"]:
        p("    nothing placed in either run")

    p(hr("  WALKS PER BOT  (bot read off walk_settled -- NEVER joined on a missing id)"))
    p(f"    {'bot':>4} {'walks A':>8} {'walks B':>8} {'fail A':>7} {'fail B':>7} "
      f"{'lost A':>7} {'lost B':>7} {'busy% A':>8} {'busy% B':>8}")
    for r in n["walks"]:
        p(f"    {str(r['bot']):>4} {_cell(r['walks']['a']):>8} {_cell(r['walks']['b']):>8} "
          f"{_cell(r['failed']['a']):>7} {_cell(r['failed']['b']):>7} "
          f"{_cell(r['lost']['a']):>7} {_cell(r['lost']['b']):>7} "
          f"{_cell(r['busy_pct']['a']):>8} {_cell(r['busy_pct']['b']):>8}")
    if not n["walks"]:
        p("    no bot activity recorded in either run")
    else:
        p("    `walk_dispatched` carries no id, so these counts come from walk_settled's")
        p("    own `bot` field. Joining on id keys every walk under None and blames one")
        p("    bot for all of them -- that is how twelve failures were once attributed to")
        p("    a bot that had none.")
    p("")


def compare_main(args: argparse.Namespace) -> int:
    """Exit 0 when the comparison stands, 3 when it was refused, 2 on usage."""
    for d in args.compare:
        if not os.path.isdir(d):
            print(f"no such run directory: {d}", file=sys.stderr)
            return 2
    a = analyse(args.compare[0], args.freeze_ticks)
    b = analyse(args.compare[1], args.freeze_ticks)
    c = compare(a, b, force=args.force)
    if args.json:
        json.dump(c, sys.stdout, indent=2, default=str)
        print()
    else:
        report_compare(c, top=args.top)
    # Non-zero whenever the numbers were withheld -- which is a refusal that
    # was not forced, and an unreadable run whether it was forced or not.
    return 3 if c["numbers_withheld"] else 0


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
    ap.add_argument("--compare", nargs=2, metavar=("A", "B"),
                    help="two run directories, side by side with deltas. Prints a "
                         "comparability block first and REFUSES (exit 3) when the two "
                         "runs were not produced under the same conditions")
    ap.add_argument("--force", action="store_true",
                    help="with --compare, print the numbers even after a REFUSE "
                         "(the output says it was forced)")
    ap.add_argument("--freeze-ticks", type=int, default=DEFAULT_FREEZE_TICKS,
                    help=f"frozen-position threshold in ticks (default {DEFAULT_FREEZE_TICKS})")
    ap.add_argument("--top", type=int, default=12, help="rows in the by-subject breakdown")
    args = ap.parse_args(argv)

    if args.compare:
        if args.all or args.dirs:
            print("--compare takes exactly the two runs to compare; drop --all and any "
                  "positional directories", file=sys.stderr)
            return 2
        return compare_main(args)

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
