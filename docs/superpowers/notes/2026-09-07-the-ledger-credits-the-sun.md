# The ledger credits the sun

**2026-09-07.** The solar arm landed this morning
(`2026-09-07-solar-counts-once-the-bank-is-standing.md`) and closed with a
handover it called *"the last wire"*: `PlanState::electric_supply_kw` credited
solar nothing, so an array the arm would credit was still invisible to every
`Condition::Powered`. This is that wire.

## The one-call description survived contact with the code

Verbatim, in `crates/planner/src/state.rs`:

```rust
total += crate::method::power::solar_supply_kw(self, area).unwrap_or(0.);
```

**A call, deliberately, and not a computation.** The constraint that mattered
was *credit only what the arm would credit*: the function that refuses
`SolarBankShort` and the ledger that credits kW must not be able to hold two
opinions about one array. Reusing the arm's sizing was possible — `state.rs`
already reaches into `crate::method::` in a dozen places — so nothing was
reimplemented and there is still exactly one encoding of "may this array be
counted, and for how much".

Crediting it in `method::power` instead was considered and rejected *by the arm's
own author*, for a reason this session did not have to re-derive: `supply_for`
would adopt a network the scheduler then refuses.

## How unknown stays distinct from zero

`electric_supply_kw` returns `f64`, which has no room to carry "unknown". So the
distinction is not made here — it is made **one level down and before the number
arrives**, and this call must only avoid *collapsing* it:

| `solar_supply_kw` | what it means | credited |
|---|---|---:|
| `Ok(0.)` | no panel is on this network | 0 kW |
| `Ok(kw)` | panels stand, bank stands | `kw` |
| `Err(SolarBankShort)` | panels stand, bank is short | 0 kW |
| `Err(SolarBankNotSizable)` | panels stand, this world cannot price them | 0 kW |

The last two credit the same number as the first and are **not the same fact**,
and the place that says so is `capacity_refusal`, which asks the same function
directly and replaces "no pole run carries power here" with the refusal by name.
The two zeros were already distinguishable; this change had to not undo that,
and `M2-credit-a-refusal` below is the experiment that says it did not.

## The two stale test docs did NOT invert, and the reason is the interesting part

The brief expected them to. They do not, and the tests are right:

- `a_solar_panel_the_world_carries_is_visible_and_still_not_power`
- `a_solar_panel_is_not_counted_as_generation`

**Both build their world from `fixture_world()`, which ships `solar-panel` with
no `solar_panel_performance_at_*` and the surface with no daylight curve.** So
those two worlds cannot *price* a panel, `solar_supply_kw` answers
`Err(SolarBankNotSizable)`, and zero is the correct credit. That is not the old
determinism objection surviving — it is the third row of the table above, which
is *every dump this project archived before 2026-09-07*.

So the docs were rewritten to the condition that is actually load-bearing, and
each test gained the non-accidental half its zero was missing: the first now
asserts the refusal is `SolarBankNotSizable { panels: 1 }` rather than an empty
network, the second that the unconditional generator table still does not carry
`solar-panel`.

**The inversion exists — it just needed a world that can price a panel.**
`a_solar_array_with_its_bank_standing_is_credited_its_daily_average` is that
test, and it is the one `M1` (the change reverted) kills.

## Falsification

Six mutations, one at a time, each asserting its substitution matched **exactly
once** in the breaking edit (`scratch/falsify.py`, in the branch's worktree).
**None came back green.**

| mutation | kills |
|---|---:|
| M1 the ledger does not credit solar | 1 |
| M2 an `Err` credits kW anyway | 4 |
| M3 solar credited at twice the arm's figure | 1 |
| M4 a panel priced at noon, not at its average | 4 |
| M5 a bank of zero satisfies any array | 4 |
| M6 an accumulator's discharge rate is generation | 3 |

Every test touched or added here is killed by at least one. The multi-kills are
structural rather than redundant: M2 kills the three tests that each assert *a
refusal credits nothing* from a different world (world-carried and unpriceable,
overlay and unpriceable, priceable and bankless) plus the refusal-message test
that necessarily depends on there being a refusal to surface. Separating those
three needs different **worlds**, not a different code knob — no mutation
available from `state.rs` can isolate them, and that is a property of the
worlds, not a gap in the tests.

### One test was unfalsifiable, and the mutations are what found it

`an_accumulator_on_a_steam_network_is_neither_supply_nor_a_refusal` asserted 900
kW and `Ok(0.)`, and **all six mutations left it green** — including M6, which
lists `accumulator` as a generator type and should have added 300 kW to it.

It built its world with the `powered()` fixture, whose `accumulator` prototype
declares **no `max_energy_production` at all**. There was no 300 kW to add, so
the test refused to credit a number that was not there. It now builds on
`solar_on_a_pole`, which sets the discharge rate, and asserts that presence
first — *"or the 900 below is an accident"*. M6 kills it now.

This is the second unfalsifiable test found by mutation in this area in two
days, and both had the same shape: **an assertion whose expected value the
system produces by accident.**

## Baselines: four unmoved numbers, and what they cannot prove

Re-measured by me on **one binary** on this branch, before and after the change,
`plan --bots 1,2,3,4`, `--release --no-default-features --features cli,lua`:

| goal | actions | ticks | before | after |
|---|---:|---:|---|---|
| `researched:automation` | 176 | 21,784 | ✔ | ✔ |
| `producing:automation-science-pack:6` | 316 | 22,457 | ✔ | ✔ |
| `producing:logistic-science-pack:6` | 441 | 47,478 | ✔ | ✔ |
| `gathered:crude-oil` (`map-31337-explored.json`) | 2,115 | 317,283 | ✔ | ✔ |

`gathered:crude-oil` still refuses on `map.json` for want of charted ground,
which is correct.

**These four prove nothing about the change, and saying so is the point.** No
archived dump has a solar panel standing anywhere, so `standing_solar` finds
none and `solar_supply_kw` answers `Ok(0.)` before any sizing happens — the
credit path is **unreachable offline**, exactly as it was for the arm and for
the daylight channel before it. The fixtures are what carry this. What the four
numbers *do* establish is a negative worth having: the new call did not perturb
any existing plan, and it costs nothing measurable — the crude-oil goal, the one
that runs for 106 s and exercises the supply ledger hardest, moved from 106 s to
107 s, inside noise.

That last figure is also the only evidence about handover 2 (`standing_solar`
reaching its candidates through `entities_named`, a whole-tree walk per
prototype, now on a per-condition path rather than a refusal path). **At the
scale of the largest world this project owns it is not measurable.** It is still
worth inlining over `net.nearby` when somebody is in there; it is not urgent,
and it should not be done blind — the two do not have the same reach, since
`entities_named` is unbounded and `net.nearby` is bounded by
`POWER_SEARCH_RADIUS`, so an inlined version would credit *less* than the arm on
a network with a distant panel. That is a behaviour change, not an optimisation.

## Verification

- `nix develop -c cargo test --workspace` — exit **0**, taken from the command
  itself and redirected to a file. 104 `test result: ok`.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  exit 0.
- `rustfmt --edition 2024 crates/planner/src/state.rs`.

One environmental note, because it reads exactly like a broken tree: the first
two clippy runs died with `sccache: Failed to create temp dir ... /tmp/
nix-shell.XXXX`, and `error: could not compile syn`. The long-lived sccache
server holds the `TMPDIR` of whichever nix shell started it, and another
session's shell had exited and taken the directory with it. Recreating that one
directory fixed it; nothing was wrong with the code, and `TMPDIR=` on your own
invocation does **not** help, because the server is the one holding the stale
path.
