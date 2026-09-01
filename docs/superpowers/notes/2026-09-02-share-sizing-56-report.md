# Per-bot share sizing — Steps 5 and 6 report

**Status:** done.

**Commits:**
- `ced7a5e7` — docs(planner,executor): retire the interchangeable-bots prose (Step 5)
- `b6681653` — test(planner): pin the roster-sum trap, re-measure more_bots_finish_sooner (Step 6)

## Step 5

Rewrote the three stale-assumption sites:
- `crates/planner/src/method/mod.rs` (`ExpansionCtx` doc)
- `crates/planner/src/goal.rs` (`Holder::Share` doc)
- `crates/executor/src/recover.rs` (tier-2 comment)

All three now say: a named bot's chain (`Holder::Bot`/`Holder::Share`) is
sized and, since `c470388b`, *owned* by that same bot — true by construction,
not by an interchangeability assumption. What remains a real, observable
choice is the `chain_actor` `expand()` is called with (picks which bot a
goal naming no holder — including `Researched`'s trigger/pack bills — is
sized and run against), which is why `recover.rs`'s tier-2 `min()` is now
justified by determinism alone, with a note that `Researched` makes the
choice observable. No behaviour change.

## Step 6

Added T7 (`a_roster_holding_the_count_between_them_plans_nothing` in
`crates/planner/src/method/have.rs`), pinning design §3: four bots holding 3
iron plates each plan zero actions for `Have{iron-plate, 12, Anyone}` and
exactly 6 actions (one plate) for `Have{iron-plate, 13, Anyone}` — verified
by running it, not guessed.

Re-measured `crates/planner/tests/red_science.rs::more_bots_finish_sooner`:

| point in history | one | many |
| --- | --- | --- |
| stale comment | 6691 | 2100 |
| `3e08f8af` (design doc baseline, re-measured) | 7075 | 2100 |
| current HEAD (all of tonight's work, incl. `c470388b` share binding) | **7075** | **2063** |

- `one`: **unchanged** (7075 vs. 7075) — a solo bot's plan has no share to
  size differently, so per-bot sizing and share-binding can't move it. This
  confirms the prompt's claim that the recorded `6691` was already stale
  before tonight.
- `many`: **improved**, 2100 → 2063 (37 ticks faster), not regressed. This
  particular goal (`automation-science-pack`, no research required in the
  fixture) never opens a `Researched` chain, so it does not exercise the
  share-binding's serialisation cost (the 22,072-tick concurrent-to-serial
  cost documented separately against a live four-bot run). Nothing here
  contradicts that cost existing elsewhere — this test simply doesn't reach it.
- Retightened the absolute-ceiling assert from `2350` to `2310` (same ~12%
  headroom over the new `many`, per the ceiling's own stated retightening
  policy) and extended the ledger comment rather than overwriting it.

## Verification

- `cargo fmt --all -- --check`: clean for all files I touched. (Unrelated
  diffs remain in `crates/core/src/factorio/rcon.rs`, owned by the other
  agent working concurrently in that file — out of scope, not touched.)
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings`:
  clean.
- `cargo test --workspace --all-features`: all tests pass, including all 267
  `factorio-bot-planner` lib tests (266 prior + T7) and all 7 `red_science`
  integration tests. (One earlier run hit a transient doctest compile failure
  in `crates/core/src/process/output_parser.rs` — the other agent's
  in-progress file, mid-edit at that moment; a clean re-run afterward passed
  with exit 0. Not caused by, or related to, my changes.)
