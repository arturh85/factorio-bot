# Run Anatomy, Phase 2 — the record reaches the page

2026-09-09. Implements the Phase 2 row of
`docs/superpowers/specs/2026-09-08-run-anatomy-design.md`.

## What landed
- Routes: `/runs/{id}/provenance` (404 when absent), `/runs/{id}/replay`
  (persisted by the run at its end as `replay.json`), `/runs/{id}/savepoints`,
  `?from=&to=` on `/samples` and `/map`, `?kind=` on `/samples`; `RunSummary`
  carries `samples`, `map`, `samples_lag_ticks`.
- The page: provenance chips (absence drawn as "not captured", per field), a
  plan chip from the replay (`abandoned > 0` flagged as a truncated tail),
  one `--resume-from` command per savepoint; the cursor readout and the idle
  share now read the analysis clock.

## What the fixture run reads

Step 2 was run by the controller: a viewer built from this branch
(`--features viewer`, port 7493, this worktree's dist) served the frontend's
new routes, opened through `pnpm start` (app/, port 8080, `/api` proxied to
the viewer).

For `run-1788926478-07032` the chips read:

```
seed 31337 · mode characters · speed 10× · commit 473c7294 · profile release
· 6 mods · map c161fa3f437221d0 · samples lag 19 ticks
```

The headline:

```
rates: iron-plate 61/min at 5:00 (roster-fed · 158 items; no generator until
10:33) · copper-plate 10/min at 5:00 (roster-fed · 19 items) | milestone 1
continuous supply stuck at 15:29
```

and below it, the last refusal:

```
last refusal: nothing can carry coal from the buffer at [32.5,-41.5] to the
iron-chest at [27.5,-40.5]: no belt route, blocked by 1 tile(s): [30.5,-39.5]
```

The ribbon segment for that stuck interval was marked stuck; the lanes showed
`plan 2 · ids restart here · 13:17`. `/replay` answered 404 for this run
because it predates replay persistence, so the page shows no plan chip for
it.

For `run-1788696619-00325` the chips were filled with `mods not captured`
(older provenance, predating the mods field) plus a
`resume --resume-from run-1788696619-00325:1` chip; the verdict read
`roster-fed · 94 items`.

Cross-checked against `just analyse` on `run-1788926478-07032`: the peer
session's offline analysis reports `machines made 14 of the 14 —
assembling-machine-1 x14` at 15:00 (roster-fed) and `factory` over one item
at 20:00. The page's verdict carries that denominator too
(`factory · 1 item`) — that agreement is why the denominator was added to
the page's verdict in the first place, rather than reporting a bare count.

## Gate (this task, commit 473c7294 + Phase 2 changes)

- `cargo clippy --workspace --all-features --all-targets -- --deny warnings`:
  exit 0, clean.
- `cargo test -p factorio-bot-core -p factorio-bot-server
  -p factorio-bot-scripting-lua --features factorio-bot-server/lua`: exit 0,
  all suites green (`core` 702+268+166 passed across lib/integration
  targets, `scripting-lua` 409+1+30+10 passed, `server` 126 passed, doctests
  clean).
- `pnpm lint`: exit 0.
- `pnpm run test:coverage`: exit 0, 75 files / 1109 tests passed. Coverage —
  statements 97.08%, branches 92.91%, functions 96.31%, lines 97.92%.
- `pnpm run build:web`: exit 0 (one pre-existing chunk-size advisory on
  `ScriptPage`, unrelated to this phase — Monaco's own bundle).

## Not in this phase (ruled)
- Folding `/runs/:id/analysis` into the page: needs replay steps joined to
  lanes, and there is no safe join key (schedule order vs. dispatch order).
  Phase 3.
- Client use of the sample slices: every band reads every kind today.
- Heatmap rect merging and keyboard access; plateau annotation; the
  `truncated > 0` count.
- Per-minute verdict labels overlapping on 15-minute runs — thin them in
  Phase 3.
- The machine band distinguishing electric from burner working counts (the
  first non-zero electric count appeared only in `run-1788926478-07032`, so
  there is one real data point and no established shape to render yet).
- `WaitKind::Restock` and success-with-note (`took N in K pieces`) as new
  lane shapes to render.
- Reading a flat tail after a plateau as a hang: it can be the driver
  idling the roster on purpose (the plan's bill was met), not evidence the
  run stalled.

Task 11 (done alongside this one, same phase): abandoned lanes are drawn as
zero-length hollow marks instead of being silently dropped, and a refused
replan is drawn as a dashed critical marker across every row. Real-record
counts: 48, 237 and 80 abandoned steps across the three real runs checked.
The peer session's fix also put the refusal text into
`milestone_stuck.last_error`, which is what the headline's `last refusal:`
line above is reading.
