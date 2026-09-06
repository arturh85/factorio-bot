# The hand-credit mass balance: a standing rate checked without a lead-in

2026-09-06, branch `sustain-mass-balance` (worktree `massbal`), from master
`e7de6707`. Implements the check §3 of
`docs/superpowers/notes/2026-09-06-standing-goals.md` deferred, and that
`docs/superpowers/notes/2026-09-06-standing-goals-first-rung.md` promoted to
"the next rung, not the second one" after the first live `Sustain` run passed
for the wrong reason.

**Headline: the archived run reads `SUSTAINED` on the lead-in check and
`roster-fed` on the balance, from the same bytes, with no parameter chosen.**

```
  iron-plate 15/min over 7200 ticks (lead-in 9600): SUSTAINED
    window 14675 -> 21875; needed 30, machines made 30, force made 30
    feeding dispatches: 0 in window, 0 in lead-in   (source: counters)
    hand-credit balance (no lead-in): ROSTER-FED
      credit 194 from 2 delivery(ies) (burner-mining-drill/coal 153,
        stone-furnace/coal 194); spent 40 before the window; outstanding 154
      machines made 30 in the window -> -124 unexplained, 30 needed
        (credit read from: label)
```

## The balance, and why this shape

Every hand delivery is a **credit** of output, priced at the most it could ever
explain. Every item a machine makes **spends** that credit. A window is
`sustained` only when the machines made more in it than the roster's
outstanding credit can account for.

```
credit       = per (machine, item) stage, summed; stages combined by MAX
spent        = machine production of the item from the record's start to the window
outstanding  = max(0, credit - spent)
unexplained  = machine_made_in_window - outstanding

unexplained <= 0            -> roster-fed
0 < unexplained < required  -> short
unexplained >= required     -> sustained
```

Four decisions worth defending:

* **Credit is priced as an upper bound, never a best guess.** 23 coal in a
  burner drill is 23 x 1,600 ticks / 240 ticks an ore = 153 plates, whether or
  not the drill ever ran that long. Every approximation in the table therefore
  pushes towards **refusing**, which is the opposite direction to the failure
  this replaces: the lead-in was too short and produced a false *pass*.
* **Stages combine by maximum, not by sum and not by minimum.** A drill's coal
  and a furnace's coal are two independent upper bounds on the same plates —
  the true bound is the smaller, so taking the larger credits the roster with
  the most generous explanation any of its deliveries supports. The archived
  run's two differ (153 against 194) and the verdict is `roster-fed` under
  either; the test asserts that, so the choice is visible rather than
  load-bearing.
* **The ledger draws credit down as it is spent, which is the thing a lead-in
  structurally cannot do.** A lead-in disqualifies a window for a fixed time
  and then stops mattering all at once. A charge of 500 ore against a run that
  had already smelted 1,080 plates explains nothing, and the balance says so
  without being told when the charge ran out.
* **An unpriced delivery is `unknown`, never ignored.** If a single feeding
  dispatch cannot be read, the roster's credit is a lower bound, and a verdict
  resting on a lower bound of the roster's contribution is exactly the false
  pass being removed.

## The blocking gap that was real, and is now closed

`insert`/`fuel`/`stock`/`charge` **did not** record their quantities as data.
The numbers were only ever in the action's prose label — `fuel the
stone-furnace with 23 coal (36800 ticks, 153 iron-plate, then it stops)` —
written at five call sites in five different sentence shapes, where a shape
nobody anticipated reads as *no delivery* rather than as an error.

So `EventKind::ActionDispatched` gained `delivery: Option<Delivery>`
(`item`, `count`, `entity`, `slot`), populated in `record.actions` from the
plan's own step table, which has carried those fields since `goal.plan`
published them. `#[serde(default)]`, so every archived run opens unchanged.
The snapshot seam did its job: the Rust snapshot test failed until
regenerated, then the TypeScript contract test failed until `Delivery` was
mirrored in `app/src/api/types.ts`.

The analyser prefers the field and **falls back to parsing the label**,
reporting which it used (`credit_source: ["label"]` above). Prose is the
fallback for runs already archived, not the design.

## What it cannot see

* **A machine loaded before the record began.** The ledger starts at the first
  event. Every run so far starts on a fresh world where the machines do not
  exist yet, so this is a caveat rather than a defect — but a
  `--resume-from` run inherits loaded machines and its first window would read
  more unexplained output than it earned.
* **`BURN_TICKS` is approximate and admitted so in the Rust it mirrors.** It
  ignores a partial burn carried between crafts, and the mod does not send
  `energy_usage`, which is why a machine outside the table cannot be priced
  at all rather than guessed at. Sending `energy_usage` deletes both constants
  and is still the follow-up it was in `method::have`.
* **Which machine a chest's contents reached.** `stock`/`charge` deliver into
  a chest; the credit is taken at the most generous burner or the recipe,
  whichever applies.
* **A steel furnace under the word "furnace".** `method::have` writes "fuel the
  furnace with N coal at P" and builds stone furnaces; the alias says so, and
  under-credits a steel furnace by half. Harmless while nothing places one.
* **Why a window is short.** Same refusal as `sustained_rate`: nothing here
  observed a cause.
* **Belted material — deliberately.** Nothing a belt delivers appears in
  `action_dispatched`, so a factory that feeds itself accumulates output
  against a credit that has stopped growing. That asymmetry is the whole
  mechanism.

## Can the lead-in be retired?

**Not yet, and on evidence rather than caution.** Both verdicts are printed;
the balance has been shown right about exactly one run, and that run is one
where it *refuses*. Nothing has yet shown it **passing a cell that genuinely
feeds itself** — every `sustained` case in the suite is synthetic and was
written by the same task as the code, which is the failure mode
`2026-09-06-fixtures-agree-with-their-code.md` exists to name.

What retirement takes: one run of a belted cell (the `method::connect` caller
the first rung still owes) reading `sustained` on the balance, and a second
archived run reading the same verdict from both checks where they should
agree. Then `--sustain <item>:<rate>:<window>` drops its fourth field and
`sustained_rate` becomes the balance.

## What a live run would add that the archive cannot

Two things, and neither is the verdict:

1. **The `delivery` field has never been written by a real run.** It is proven
   by a Lua-level test through `record.actions` and by nothing else. A single
   short headless run would show it in `events.jsonl` and move the archived
   run's `credit_source` from `label` to `fields`.
2. **A `sustained` reading from a real factory.** The balance's refusal is
   evidenced; its pass is not. That run needs a standing coal supply, which
   does not exist yet — which is precisely the finding the first rung was
   built to produce.

No live run was made for this change: nothing in it can be shown by a run that
the archive does not already show, and the run that *would* add something needs
a belt the planner cannot yet route.

## Written alongside its own tests

Stated because the house rule asks. This task wrote `hand_credit_balance`, the
`Delivery` field, and every test of both except one: the regression fixture
`tools/fixtures/run-1788674059-90744` is the archived run itself, recorded
yesterday by a session that could not have written it to agree, trimmed only by
dropping the 328 `bots` sample rows neither function reads. A test also
re-runs against the live archive where it still exists and asserts the two give
one answer, so the trim cannot drift.

Every new test was watched failing under a substitution of the value it
forbids, **with the substitution count asserted first** (`scratch/falsify.py`,
seven cases, each matched exactly once, none came back green):

| substitution | went red |
|---|---|
| stage combination `max` -> `min` | the credit test |
| `fuel` is not a delivery verb | the archive's `roster-fed`, the credit, the priced-deliveries test |
| credit never drawn down by production | three ledger tests |
| an unreadable delivery is skipped | both `unknown` tests |
| chest coal priced at the least generous burner | the chest test |
| a stone furnace burns coal for 192 ticks | three credit tests |
| structured fields ignored in favour of the label | the fields test |

The Rust half was falsified by hand the same way (`kind == "insert"` ->
`"remove"`, matched once, reddening both new tests), and the TypeScript
contract by declaring `Delivery.count` a string (matched once, two failures
naming the field).

## Verification

* `nix develop -c cargo test --workspace` — green.
* `nix develop -c cargo clippy --workspace --all-features --all-targets --
  --deny warnings` — clean.
* `app/src/api/openapi.contract.spec.ts` — 277 passed.
* `python3 tools/test_run_analysis_balance.py` — 18 passed;
  `tools/test_run_analysis_sustain.py` and `tools/test_run_analysis_rates.py`
  unchanged and green.
* Offline plans on `workspace/scripts/map.json`, release build, unchanged:
  `researched:automation` 176 / 21,784, `producing:automation-science-pack:6`
  316 / 22,463, `producing:logistic-science-pack:6` 442 / 47,542.
