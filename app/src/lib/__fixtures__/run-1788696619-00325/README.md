# Fixture run `run-1788696619-00325`

Seed 31337, four client bots, 1.0x, release `492e513a` clean, `research automation`
satisfied at 6:06 (tick 25216). Copied from `workspace/runs/` on 2026-09-08.

- `events.jsonl` — verbatim.
- `samples.jsonl` — `force` and `machines` lines only (`bots` lines dropped; nothing here reads them).
- `lanes.json` — `GET /api/v1/runs/{id}/lanes` verbatim.
- `rates.json` — the `rates` object of `python3 tools/run_analysis.py <dir> --json`.
  **This is the golden output for `runAttribution.spec.ts` and `runRates.spec.ts`.**
  Regenerate it only together with a deliberate change to the Python rule.

The run's window is ticks 3242 (`run_started`) to 25224 (`run_finished`).
