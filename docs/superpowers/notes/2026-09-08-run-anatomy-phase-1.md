# Run Anatomy, Phase 1 — what landed and what it showed

2026-09-08. Implements Phase 1 of
`docs/superpowers/specs/2026-09-08-run-anatomy-design.md`.

## What landed
- `/runs/:id`: headline, milestone ribbon, tick axis, items/min with the
  attribution verdict painted per minute, power, research, bot lanes with idle
  hatched and replans dashed, machine status heatmap, record coverage, one
  cursor bar; map with machine status fills, video as an on-demand tab.
- Six pure libs under `app/src/lib/`; the attribution and rate libs are pinned
  to `just analyse --json` for `run-1788696619-00325` by a golden test.
- Dark theme tokens and a per-viewer toggle; `--color-warn-dark` now exists.

## What the fixture run reads, against the tool

Both sides are `run-1788696619-00325`. The controller ran the built page
(vite dev server, headless Chrome) after the `@theme static` fix; `just
analyse` was run against the same run's record.

**Headline sentence, page (`/runs/run-1788696619-00325`):**

```
rates: iron-plate 8/min at 5:00 (roster-fed; no generator until 4:06) ·
copper-plate 0/min at 5:00 (roster-fed) |
milestone 1 research automation satisfied at 6:06
```

**Headline sentence, `just analyse`:**

```
rates: iron 19 /min at 5 (roster-fed; no generator until 4:06); red packs 2;
green 0 at 5, 0 at end 6:06; run ended 6:06 before mark 10 |
milestone 1 satisfied at 6:06
```

Both agree on the generator timing (4:06), the research milestone tick (6:06)
and the attribution verdict (roster-fed) for iron. **The one deliberate
difference: 8/min vs 19/min for iron at the 5:00 mark, and it is not a
disagreement.** The page's headline quotes `rate_window`, the trailing
2-minute window rate; the tool's headline quotes `rate_interval`, the interval
rate over the whole first 5 minutes. Both numbers live in the tool's own JSON
output and both are pinned by the golden tests — the page did not choose the
smaller-looking number, it chose the more locally-relevant one for a moving
cursor.

**5:00 verdict rows, page vs `just analyse`:**

| item | page (production/verdict panel) | `just analyse` |
|---|---|---|
| iron-plate | roster-fed | roster-fed |
| copper-plate | roster-fed | roster-fed |
| automation-science-pack | hand-made (dotted) | hand-made |

**Other panels, as read from the page:**

- **Plates band**: hatched fill (roster-fed) across every producing minute for
  both iron-plate and copper-plate; iron-gear-wheel, electronic-circuit and
  automation-science-pack render dotted (hand-made); logistic-science-pack
  reports "no output in this run".
- **Power**: "no generator until 4:06", then "generated 900 kW" for the rest
  of the run.
- **Bot lanes**: four rows; bots 2, 3 and 4 read idle 64%, 74%, 76%. (`just
  analyse` reports bot 1 separately: 6,984 idle ticks, 31.8%, 85 gaps — the
  two readings are of different bots and are not in tension.)
- **Machine heatmap**: 17 machine rows, drill first, chests last.
- **Record coverage**: band reads "samples cover the run to its end"; chip
  reads "samples cover to end".
- **Rendering**: after the `@theme static` fix, every verb and status colour
  resolves in the browser. Before the fix they rendered black, because
  Tailwind v4 prunes `@theme` tokens that only an SVG `var()` reference uses,
  and nothing else in the page referenced them by class.

## What is not here (Phase 2/3)
- provenance chips read "not captured" — no route yet;
- `/samples` and `/map` load whole files; slices are Phase 2;
- the replay's `Evidence` is not on disk, so lane titles say "believed" for
  every walk by rule, not per row;
- no flow view;
- **the plateau annotation (spec §1.4)**: `just analyse` detects a plateau and
  says which kind it is; the Items/min band draws the curve and the per-minute
  verdict and marks no plateau at all;
- **the `truncated > 0` count on the machine heatmap (spec §4)**: a `machines`
  sample carries how many machines it dropped, and the band does not say so —
  a heatmap missing rows looks exactly like a run with fewer machines;
- **bot inventory at the cursor is no longer reachable in the UI.** The old
  inventory panel was removed with the side panel's rework; `runsStore`'s
  `bot`, `botState`, `bots` and `selectBot` are all still there and still
  tested, feeding nothing. Whether that panel comes back, moves into the map's
  bot tooltip, or the getters go is an **open owner question** — the getters
  are deliberately left in place rather than removed on a guess.

## The two clocks, after the final fix wave
Every band now takes both a `scale` (the drawn axis, which may trim a lead-in)
and a `clock` (the analysis window, from `run_started`). Positions come off
`scale`; every game-time label and every fixed mark comes off `clock`, which
is where `just analyse` measures them. Before the wave the milestone ribbon
read the drawn axis and said **6:03** for the same tick the headline called
**6:06**, and the Items/min band's "5:00" mark sat at tick 21,417 against the
tool's 21,242. Both now read the window; `RunPage.spec.ts` pins it.

## Deferred minors (ledgered, not fixed)
- SVG pattern ids are document-global — fine with one page instance today,
  will collide if two run pages ever render at once;
- the machine heatmap draws one `<rect>` per sample cell; mitigation is to
  merge same-status runs, and the heatmap has no keyboard access;
- tabs lack `aria-controls`/tabpanel linkage;
- `CoverageBand`'s header text can overlap row 0 in the heatmap;
- the events-error state replaces the whole headline rather than only the
  sentence that actually failed.

## Verification
Frontend gate only (`cd app && pnpm lint && pnpm run test:coverage && pnpm run
build:web`), run individually to read real exit codes: lint exit 0, coverage
exit 0 (74 files / 1038 tests passed; 96.9% statements, 92.04% branches,
96.18% functions, 97.96% lines), build exit 0. **After the final fix wave**,
same three commands: lint exit 0, coverage exit 0 (75 files / 1059 tests
passed; 97.27% statements, 92.99% branches, 96.86% functions, 98.17% lines),
build exit 0. `pnpm run precommit:check` and
every `cargo` command were skipped on this task by controller ruling: no Rust
changed on this branch, and another session asked that cargo stay off this
machine's main checkout during a measured run.
