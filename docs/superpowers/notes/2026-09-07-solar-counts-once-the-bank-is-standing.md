# Solar counts once the bank is standing

**2026-09-07.** The daylight channel landed hours earlier
(`2026-09-07-a-surface-knows-its-own-daylight.md`) with both derivations done
and neither wired up, and named its own follow-up: *"a solar arm of
`method::power`: size the array on `solar_average_kw`, size the bank on
`accumulators_per_panel`, and only then credit either."*

This is that arm. It sizes, it gates, and it refuses by name. **It does not
credit, and the reason is a boundary rather than a doubt** — see the handover.

## How a plan says "the bank is standing"

The same way it says anything else stands: `Effect::CreateEntity` puts an
entity in `PlanState`'s overlay, and `PlanState::entities_named` reads the
overlay and the world as one. So an accumulator this plan has *placed* and one
the world already carried are the same fact to `solar_supply_kw`, and no new
machinery was needed to express "planned as well as standing". That was the
open question in the brief and it turned out to have been answered already.

**The network is the one the supply ledger already walked, not a second one.**
`PlanState::powering_entities(area)` returns the poles of every wire-connected
component whose supply area meets the ground asked about — the union-find
`electric_supply_kw` credits generators over. A panel or an accumulator is on
that network exactly when one of those poles would supply its footprint, which
is `PlanState::pole_would_supply`, the same predicate again. So the arm's
notion of "the same network" is not a new one; it is the ledger's, composed out
of two public accessors, and `state.rs` was not touched to get it.

Classification is by `entity_type` — `solar-panel` and `accumulator` — and
**deliberately not by the fields that price them**. Pricing gates on the
presence of `solar_panel_performance_at_day`, and must, because that field's
`subclasses: ["SolarPanel"]` is what says a prototype follows the daylight
curve. Classification must not, or a world that reports a panel it cannot price
— every dump this project archived before yesterday — answers *"there is no
solar here"*, which is a lie, instead of *"there is solar here I cannot size"*,
which is the refusal. **This distinction is load-bearing and falsification
found it**: the mutation that collapses the two kills exactly the no-daylight
test and nothing else.

## What the refusal says

Two variants, both `crates/planner/src/error.rs`:

```
SolarBankShort       {panels} solar panels on this network average {average_kw} kW over a
                     day, and carrying that load through the night needs
                     {accumulators_needed} accumulators of which {accumulators_standing}
                     stand, so the plan credits the array nothing

SolarBankNotSizable  {panels} solar panels stand on this network and the accumulators they
                     need cannot be sized: {because}
```

`because` names the missing term — no daylight curve on the surface, no
day/night endpoints on the panel, no buffer on the accumulator, or more than
one kind of either standing on one network (which refuses rather than guessing
a ratio it cannot state).

**One half of the brief is deliberately not a refusal, and this is the
deviation to argue with if anyone wants to.** It asked the refusal to name
"which of the two is missing — panels or bank". The bank half is
`SolarBankShort`. The panels half — an accumulator on a network with no panel —
answers `Ok(0.)` and refuses nothing, because **an accumulator on a steam
network is an ordinary thing to build** for peak shaving, and refusing a
working base for owning one would be exactly the false refusal this arm exists
to remove. The bank is the half whose absence is silent; that is why it is the
half that speaks.

## Where it is wired, and what that changes

`capacity_refusal`, and only there. That function splits a false headroom
condition into a capacity failure and a routing failure on `supply_kw > 0`.
**A third failure was hiding inside the zero**: solar reads as zero because the
ledger credits it nothing, so a base with panels and no accumulators was
reported as a *pole-routing* failure and sent its reader to look at geometry
that was perfect.

Only the refusal is taken. An array whose bank *is* standing answers `Ok`, and
that still falls through to the routing verdict — the message changes, never
the plan. Nothing here turns a refusal into an acceptance.

## Keeping the two units apart

An accumulator answers two questions that both sound like *"how much power does
it hold"*:

| field | vanilla | what it is |
|---|---|---|
| `max_energy_production` | 300 kW | the **discharge rate** — a ceiling on delivery |
| `electric_buffer_capacity` | 5 MJ | the **store** — joules it can hold |

**The sizing reads the store and nothing in the arm reads the rate.** A bank
sized on 300 kW rather than 5 MJ is wrong by a factor that depends on the
night's length — the same conflation that read the vanilla 25:21 ratio as an
output average one level up.

Three places hold it apart, on purpose:

- **The names.** `average_kw` is power; `accumulators_needed` is a count sized
  from energy. `SolarBankShort` carries both because neither can be computed
  from the other: they are two integrals of one curve.
- **The fixture carries both numbers.** `solar_state` sets the accumulator's
  discharge rate *as well as* its buffer, so a sizing that reached for the
  wrong one would find a plausible number waiting rather than a `None` that
  fails loudly.
- **`a_bank_is_sized_on_stored_energy_and_not_on_discharge_rate`** is the
  experiment: doubling the store halves the bank; multiplying the discharge
  rate by ten changes nothing at all.

Nothing in the arm multiplies or scales the ratio — `accumulators_per_panel`
computes it, the arm multiplies by a panel count and rounds up. There is one
copy of that arithmetic and this is not it. No `0.7`, `42` or `0.84` appears
anywhere in the code; the results are quoted in doc comments only.

## Verification

**Four baselines, on one binary, this branch, `plan --bots 1,2,3,4`:**

| goal | actions | ticks |
|---|---:|---:|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,457 |
| `producing:logistic-science-pack:6` | 441 | 47,478 |
| `gathered:crude-oil` (`map-31337-explored.json`) | 2,115 | 317,283 |

**Nothing moved** — byte-identical to the figures the brief quoted, and
`gathered:crude-oil` still refuses on `map.json` for want of a charted field,
which is correct. It could not have been otherwise and the reason is worth
stating rather than assuming: no archived dump has a solar panel standing, so
`standing_solar` finds none and `solar_supply_kw` answers `Ok(0.)` before the
sizing is reached. **The offline basis cannot exercise this path**, exactly as
it could not exercise the daylight channel yesterday; the fixtures are what
carry it.

- `nix develop -c cargo test --workspace` — exit **0**, taken from cargo and
  redirected to a file, never read off a pipe. 104 `test result: ok`.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  exit 0.
- **Eight falsification mutations** (`scratch/falsify.py`), one at a time, each
  asserting its substitution matched **exactly once** in the breaking edit.
  Seven kill exactly one test. The eighth kills two, and that is structural
  rather than redundant: a test that asserts `capacity_refusal` *surfaces* a
  solar refusal necessarily depends on there being one to surface. Every test
  has at least one mutation that kills it alone, so none is redundant or
  vacuous.

**Three mutations came back wrong on the first pass, and all three were real.**

- **One came back green — a broken experiment.** The mutation for "an
  unreported curve is not a bank of zero" left every test passing, because
  without daylight *both* `solar_average_kw` and `accumulators_per_panel`
  answer `None`: **two independent guards hold that property and breaking
  either alone is invisible.** The same shape the peer session found yesterday
  from the other direction. Fixed by adding
  `an_accumulator_with_no_buffer_cannot_size_a_bank` — a surface with a good
  curve and a buffer-less accumulator, the one case only the second guard can
  reach — and by re-aiming the first mutation at what the no-daylight test
  actually pins, which is the *classification*.
- **Two killed a test each that they should not have**, and both were genuine
  overlaps in the wiring test: its control ("no panels, no solar verdict")
  restated the empty-network test's claim, and it took its refusal from the
  short-bank comparison. The control moved to the test that owns that claim.
- And one test restated the credit formula the credit test owns; it asserts a
  *difference between two worlds* now, so it pins the exclusion and nothing
  else.

## Handovers

Three, all in `crates/planner/src/state.rs`, which another agent held for
`maximum_wire_distance` while this was written.

1. **The last wire.** `electric_supply_kw` credits solar nothing, and every
   feasibility check goes through it — so an array this arm would credit is
   still invisible to `Condition::Powered`. Closing it is one call there,
   guarded exactly as `solar_supply_kw` guards: credit the average only when
   the bank is standing, refuse otherwise. **Crediting it in `method::power`
   instead was considered and rejected**: `supply_for` would adopt a network
   the scheduler then refuses, which is a worse failure than the one it fixes.
2. **`standing_solar` should be inlined over the network's own entity list
   when that happens.** It reaches its candidates through
   `PlanState::entities_named`, which reads the whole entity tree once per
   prototype — two walks on vanilla. That is fine on a refusal path and is
   **not** fine per condition check. `electric_supply_kw` already holds
   `net.nearby`; classifying it in place is free.
3. **Two tests' docs are now stale in their reason, though not in their
   verdict.** `a_solar_panel_the_world_carries_is_visible_and_still_not_power`
   and `a_solar_panel_is_not_counted_as_generation` both say solar is excluded
   because *"its output depends on the in-game time of day"*. That objection
   was answered yesterday: an average over the surface's own curve is
   deterministic. The verdict is unchanged and both tests should stay — the
   exclusion is real — but the reason is now **storage**, not determinism.
   Left alone rather than edited, because the file was another agent's.

## What this deliberately does not model

**Discharge rate.** The bank is sized in joules; an accumulator's 300 kW is a
per-unit ceiling on delivery, and a bank with enough joules can still fail to
hand them over fast enough at 03:00. Nobody has asked that question, and
answering it wants a charge-state-over-time model that nothing needs yet. Named
in `solar_bank_for`'s own doc so a reader does not mistake the sizing for it.

**A general storage model.** Same reason. Sizing a bank against a known night
length is a closed-form integral over four boundaries; reasoning about charge
over time is a different program.
