# Making the run map comprehensible — 2026-09-02

## Status

Done, frontend-only. `pnpm lint` clean over every file in this change,
`pnpm run test:coverage` green: **871 passed, 0 failed** across 51 files, and
the enforced gate cleared at statements 96.29 / branches 92.71 / functions
94.19 / lines 96.87 (thresholds 90 / 80 / 90 / 90). 68 tests over the
three specs this touches, where there were 9 before: 19 + 20 new lib tests,
and `MapPanel.spec.ts` from 5 to 29.

One thing is deliberately NOT wired and needs a decision — see
"[The one line I did not write](#the-one-line-i-did-not-write)" at the end.

Another agent was adding `app/src/api/videoClock.ts` in this same checkout
while these ran. It failed eslint and one of its own tests briefly mid-flight
and was green again minutes later; none of it is in this commit.

## SVG, and the numbers that decided it

`MapPanel.vue` was canvas, and its header comment gave two reasons. The first
was a performance claim:

> a run's map can hold thousands of entities across a run's lifetime, and this
> view is redrawn on every cursor move, which a large SVG DOM does not do
> cheaply

That claim was true of the shapes the component chose to draw and not true of
the map. I read the last keyframe of every `map.jsonl` in `workspace/runs`
(18 runs) and counted resource tiles, their 4-connected components, and
everything else:

| run | entities | resource tiles | patches | other |
|---|---|---|---|---|
| run-1788320177-77989 | 1141 | 1122 | 3 | 19 |
| run-1788329146-40305 | 1034 | 1007 | 2 | 27 |
| run-1788334911-41961 | 1004 | 989 | 4 | 15 |
| run-1788319014-01846 | 933 | 925 | 4 | 8 |
| run-1788353986-24634 | 881 | 856 | 2 | 25 |

Worst case across all 18: **4 patches** and **27 non-resource entities**. The
"thousands of entities" are 98% ore, and ore is a handful of contiguous blobs.

Rendering the whole of `run-1788353986-24634` at its final tick through the
new component and counting the DOM:

```
entities 881, bots 4, trails 4
  -> 38 SVG elements, 35 of them interactive, 13 legend rows
  -> 33 KB of HTML
  -> the two ore patches are two <path>s, d="..." of 323 and 263 characters
```

856 ore entities become 2 paths of 46 points each. The 510-tile iron patch
exposes 110 unit boundary edges; dropping the vertices where the outline does
not turn takes that to 46 points.

So the performance objection is gone, and — this is the part worth stating
plainly — **the thing that removed it is the same thing that fixed the
readability.** 856 identical cells were an undifferentiated blob. A patch is
what a bot's trail visibly travels *to*, what a tooltip can name ("iron-ore
patch, 510 tiles"), and what a legend row can count. Aggregating was never a
rendering trick applied on top of a legibility change.

The second reason the old comment gave was not an argument for canvas at all:

> jsdom's `getContext('2d')` returns `null` without the native `canvas`
> package, so nothing this component draws can be asserted in a test

That is a cost of canvas, and it was being paid. `MapPanel.spec.ts` could
assert that a `<canvas>` element existed and nothing else — five tests, none of
which touched a number. It now asserts the patch outline `d`, the marker
coordinates, the group transform, the tooltip text and the legend rows. Both
sentences of the old header comment have been rewritten in place rather than
left standing against the change.

## What was built

Two pure libs, both under `app/src/lib/` next to `runMap.ts`:

- **`resourcePatches.ts`** — resource detection, 4-connected components,
  boundary tracing, `d`-string building. 19 tests.
- **`mapFeatures.ts`** — turns `(entities, bots, trail, records)` into
  labelled features carrying the exact sentence each tooltip should say, plus
  `legendFor`. 20 tests. The wording is asserted here rather than in the
  component, because the wording is the claim: "placed by bot 2 at tick 5911"
  and "no placement recorded" are two different statements about a record.

Then `MapPanel.vue` (SVG, 29 tests) and a new `MapLegend.vue` under
`components/map/`. Both names are multi-word, so `eslint.config.mjs` needed no
change.

### Tooltips

Every shape is a `tabindex="0"` `role="button"` with an `aria-label` and a
`<title>`, and the detail panel is `role="status" aria-live="polite"`. Hover,
tap and Tab all open it; Escape and blur close it. Keyboard focus outranks the
pointer, so moving the mouse off the map does not discard what Tab selected —
that has its own test.

What each says:

- **Ore patch** — `iron-ore patch` / `510 tiles` / `centre 1.5, 32.5` /
  `spans 36 x 24 tiles`.
- **Placed entity** — `stone-furnace` / `at 5, 35` /
  `placed by bot 2 at tick 5911`, and when the record carries `drift`, a
  fourth line `drifted from 5, 35 (position, direction)`. An entity with no
  `placed` record says `no placement recorded — seen in a keyframe` rather
  than showing nothing, because crash-site wreckage genuinely was placed by
  nobody.
- **Trail** — `bot 2 trail` / `3 sampled positions` / `from 0, 0` /
  `to -40.5, 12.5`, anchored at the far end, which is where the bot went.
- **Bot** — `bot 1` / `at 20, 30`.

### The legend, and the labels that made it not the whole answer

`legendFor` derives rows from the features actually rendered — never from a
table, because `entityColor.ts` is a hash and a hand-written legend would go
stale the first time an unlisted entity appeared. Rows count what they
actually mean: resource rows count tiles (`510 tiles in 1 patch`), entity rows
count entities (`19 on map` — not "placed", see above), bot rows count
nothing. Swatches are drawn in the three shapes the map uses, so the key does
not depend on telling two hues apart.

Rendering it and looking at the result changed one decision. A legend still
makes you look away from the map and match a colour, and the brief's complaint
was that the map is not comprehensible *on its own*. There are only ever a
handful of patches, so the map now writes their names across them. That is
what actually made the picture readable.

Looking at the render also caught a mistake that no test would have. I had
made each trail a single fat translucent polyline, one element serving as both
the visible line and the touch target — and it drew a wide smear across the
map, which is the exact failure mode being fixed. Trails are now a hairline
plus a separate invisible 16px `pointer-events="stroke"` polyline.

### Colours

`entityColor.ts` keeps its independence from `crates/core/src/draw.rs` — no
colour is copied. It now varies saturation and lightness as well as hue,
because 360 hue buckets drawn at random by a hash are not enough for a
thirty-row legend: on the run I developed against, `crash-site-spaceship` and
`iron-ore` landed on hue 114 and hue 117, two greens nobody could separate.

`resourcePatches.ts` does share the six resource *names* with `draw.rs`. Names
are a fact about Factorio, not a palette, and the rule is a `-ore` suffix plus
`{coal, stone, crude-oil}` so a modded ore is covered without an entry
anywhere. `EntitySnapshot` carries no `entity_type`, so a name test is the
only signal available.

## Two traps paid for during this

- **A tile is not its position.** Resources sit at tile centres (`-40.5`), so
  the tile is `floor()` and its extent is `[x, x+1]`. Losing the half-tile is
  the bug CLAUDE.md already records once; it has a test here
  (`recovers the tile from a negative half-tile centre without drifting`).
- **`Array.sort()` on `"x,y"` keys sorts them as strings**, which puts
  `"10,4"` before `"4,4"`. That is still deterministic, so nothing failed —
  but it started each ring at whichever corner had the fewest digits, and a
  patch's `d` would silently reshape in a diff the moment it crossed a power
  of ten. Found because a hand-computed expected `d` in a test came back
  starting at the top-right corner. Sorted numerically now, with a test.

The outline tracer orients every edge with the filled area on its right, which
makes outer rings clockwise and hole rings counter-clockwise for free, so
`fill-rule="nonzero"` empties holes without a winding pass. At a "pinch" — two
tiles meeting at a single corner — the walk prefers a right turn, which keeps
it hugging the blob it arrived on instead of chaining two blobs into one
self-intersecting ring. Covered by an invariant test that every boundary edge
is consumed exactly once.

## The one line I did not write

`MapPanel` takes a new **optional** `records?: MapRecord[]`. It is what feeds
"placed by bot 2 at tick 5911". Without it the map is complete and every
entity honestly reads `no placement recorded`.

Nothing passes it today, because both call sites are files outside my
boundary — `app/src/pages/RunsPage.vue` and `app/src/pages/RunAnalysisPage.vue`
— and `RunsPage.vue` is plausibly where the concurrent video-playback work
lives. The store already holds the data; no store change is needed. The whole
wiring is one attribute:

```diff
                 <MapPanel
                     v-else
                     :entities="store.entities"
                     :bots="store.mapBots"
                     :trail="store.trail"
+                    :records="store.map"
                     :bounds="store.mapBounds"
                 />
```

Until someone adds it, bot attribution is built, tested and unreachable.

## What I could not verify

- **No browser.** Everything visual was verified by rendering the component's
  own SVG output through jsdom, re-attaching the scoped CSS by hand, and
  screenshotting it with headless chromium. That is what caught the trail
  smear and drove the on-map labels. It is *not* the same as the real page:
  the hand-copied CSS could disagree with the `<style scoped>` block, and the
  panel was never seen inside `RunsPage`'s own layout at a real width.
- **Focus behaviour on a `<g>`.** SVG2 allows `tabindex` on any element and
  the major engines honour it, but jsdom's `trigger('focus')` dispatches the
  event without exercising real focus management, so "Tab reaches the trail"
  is asserted at the handler level, not by a browser actually tabbing there.
  The same applies to `:focus-visible`, which never matches in jsdom — the
  visible focus cue rides on the `is-active` class instead, precisely so it is
  observable.
- **Touch.** A tap is asserted as a `click`, which is what a touch screen
  synthesises, but no touch device was involved.
- **Only these 18 runs.** The element-count argument rests on the archive that
  exists. A map with genuinely scattered single-tile resources — heavily
  depleted ore, or a mod that scatters — would aggregate into many more
  patches than 4, and at some count SVG stops being the right call. The
  projection stayed in `mapProjection.ts` as a uniform scale plus offsets,
  which is trivially invertible, so a canvas-plus-hit-testing version remains
  a small change rather than a rewrite.
- **`crude-oil`.** Wells are isolated single tiles, so each becomes a one-tile
  patch with its own legend row. Correct, and never seen on a real recorded
  map here, because no run in the archive contains any.
