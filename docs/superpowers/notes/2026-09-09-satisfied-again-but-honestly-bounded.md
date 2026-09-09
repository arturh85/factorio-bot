# `satisfied` fires again — this time on a real capacity, and the retry bound did its job

`run-1788931904-77495`, seed 31337, four headless bots at 10x, `--new`, release
binary built from master `2f3a5a05`. **Nothing cheated.**

**Disclosure**: a peer agent's build overlapped part of this run (up to 5 live
rustc/cargo alongside the game). Marks are game-tick-bounded, so the production
curve, census and attribution below are unaffected — CLAUDE.md's own rule. The
delivered tick rate (91%, 548/600 tps) and wall duration (3.6 min) are **not**
trustworthy as clean numbers and are not quoted as such.

## The result: the same 15-pack plateau, reached the same way

```
automation-science-pack   0 → 0 → 14 → 15 → 15   at 5/10/15/20/25
census at 15:00: assembling-machine 2/1, generator 1/1, boiler 1/1
work e/b 40/343 at 15:00
```
Materially identical to `run-1788926478-07032`. The offtake-arm fix
(`701b2f6f`) did not change this run's shape, and per its own stated limit it
should not have: the electric leg needs a *standing* powered network before a
new cell's offtake is sited electric, and this run builds its first cell before
power exists.

## The milestone reports `satisfied`, and this time it may be defensible

```
milestone 1: satisfied after 3 iteration(s), best 93 steps, last error:
tried to remove 8 copper-plate but removed 0 in 0 pieces over 2037 ticks;
nothing more arrived in the last 2037 ticks -- the source is not producing
```

**The bounded retry from `473c7294` fired correctly.** It is designed to give
up after 1,800 idle ticks with exactly this message, and it did — not a hang,
not a silent failure, a named refusal after the bound.

**What "satisfied" means here is narrower than the word suggests.** Per the
`a27811f1` fix, a `SustainSupplyNotStanding`-shaped refusal is held back while
other conjuncts still have work, and re-raised — read as "done" — only when the
bundle has nothing left to build. Here `producing:automation-science-pack:6`'s
conjunct genuinely **is** standing (2 assembling machines, correct recipe), so
that half is honestly satisfied. But the `sustain:copper-plate:15:36000`
conjunct is reported the same way, and its copper flow had **genuinely
stopped** — not because the sustain arrangement is adequate, but because the
source stopped producing. **"Nothing left to build" and "the target rate is
being sustained" are being read as the same fact, and they are not.** This is
not the same false green as before (there is real capacity behind it this
time), but it is the same shape one layer down: the word "satisfied" still
does not distinguish *achieved* from *given up on, with nothing else to try*.
Unowned; not fixed here.

## The three-way check does NOT trivially agree, and that is worth knowing before anyone builds on it

```
events.jsonl (whole run, 3 replans):  {success: 1012, abandoned: 107, failed: 4}  = 1,123
replay.json:                          {Success: 49, Failed: 2, Abandoned: 33, Pending: 9} = 93
```

**`replay.json` holds 93 steps against the run's 1,123 total actions.** Its
`index` range is a contiguous 0–92 with its own numbering, and every one of the
plan's 3 `plan_created` events has a different step count (884, 165, 74) — none
of which is 93 either. So `replay.json` is not a straightforward slice of any
one plan and is far short of the whole run. This was flagged to the author as a
finding rather than diagnosed further here, since it is their code and the
check was explicitly "verify, do not assume".
