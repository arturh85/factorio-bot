# Three already-diagnosed defects, verified and fixed — 2026-09-03

All three were real. None of the "the code does X" claims handed to me was
wrong, but **one was incomplete in a way that changed the fix** (defect 2), and
one names a reachability I could not construct (also defect 2, below).

Verification was done under a build hold: a live four-client run was executing,
so every cargo command here was `nice -n 19 nix develop -c cargo … -j 2`,
scoped to a single crate. **`just test`, `cargo clippy --workspace
--all-features --all-targets` and `cargo test --workspace` have NOT been run**,
and defect 1 is on the startup path, which is exactly what `just test` gates.
That is the one outstanding piece of verification; see *Still unverified*.

---

## Defect 1 — the connect wait gave up on a clock

**Real.** `crates/core/src/process/process_control.rs:214-249` read
`if wait_started.elapsed() > Duration::from_secs(90) { error!(…); break; }` —
an absolute budget from the first poll, evaluated regardless of whether
clients were still arriving. The error message named only `expected_players`,
a number.

### Cause

The budget measured the wrong thing. In run 30 the clients joined 94.2 s,
101.0 s, 106.6 s and 109.6 s after server start
(`docs/superpowers/notes/2026-09-02-bot-one-idle.md`); the wait quit at 90 s
of its own clock with joins still arriving, and the script that followed froze
its roster from the one bot it could see. Three of four bots did nothing for
162,158 ticks.

### Fix

`crates/core/src/process/connect_wait.rs` (new): `ConnectWatcher`,
`ConnectWait`, `missing_clients`. `process_control` drives it and no longer
owns the decision.

**The number is unchanged; what it measures is not.** Ninety seconds is now
the tolerance for *no progress*, restarted whenever another client appears. A
run where nobody ever connects gives up at exactly the same moment it used to,
so this is not a licence to hang. The total is bounded without a second timer:
progress is a high-water mark that can rise at most `expected` times, so the
wait cannot exceed `(expected + 1) * 90 s`, and only approaches that when
clients really are trickling in.

**What is being measured is stated in the type's doc comment**, because the
handoff was right to insist on it: the count comes from the mod's `players`
remote call, which filters on `player.connected and player.character`. It is
**players with characters, not processes that connected** — a client is
connected and uncounted for the 750 ticks of freeplay's crash-site cutscene.
So "stalled" here means *no additional player became controllable*, which is
what the run needs and is a stricter bar than "no process connected".

`missing_clients` names the absent ones. It reports **slots, not process
identities**: Factorio hands out player ids in join order and the rest of the
crate already treats a `PlayerId` as the bot's identity (`whoami` names
`client{n}` in the same order). A client that connected but is still in the
cutscene is named as missing, which is honest — it is exactly as useful to the
run as one that never launched. The naming poll is best-effort
(`connected_players()`, one extra call at give-up time) so the counting poll
keeps its deliberate tolerance for a reply it cannot deserialise.

### Red first, with the real output

The policy did not exist as a unit before, so the red was produced by
transcribing the *old* policy into the new type — deleting the one line that
restarts the clock — and running the tests against it:

```
thread 'process::connect_wait::tests::a_client_still_arriving_is_waited_for_past_the_old_budget'
panicked at crates/core/src/process/connect_wait.rs:124:9:
assertion `left == right` failed: past the old 90-second budget, but the last
client arrived 18 s ago
  left: Stalled
 right: Waiting
```

Then the line was restored and all five pass.

Two of the five **do not go red** under the old policy, deliberately:

- `a_client_that_never_connects_is_still_given_up_on` passes under both. That
  is the point — it pins that the budget for a dead client is unchanged.
- `a_rejoin_that_regains_lost_ground_is_not_progress` passes under both,
  because the old policy has no clock to restart. It pins the high-water rule
  the new policy needs; mutating `count > self.best` to `count != self.best`
  is what turns it red.

`the_clients_with_no_player_are_named` is a plain unit test of a new function;
it has no "before" to be red against, and it pins that the absentee list is
derived from ids rather than from `expected - count`.

---

## Defect 2 — the abort in the flow graph, and the wrong answer underneath it

**Real, and the handoff understated it.** `crates/core/src/graph/flow_graph.rs`
did not merely `unwrap()` an `Option`; at a `MiningDrill` source it read
**`entity_root.miner_ore` and `entity_root.entity_name` — the entity the walk
started at, not the drill producing on the edge being drawn.**

### Cause

`update()` walks outward from each root and matches on `source_node.entity_type`,
but computed the mining rate from `entity_root`. Those are the same entity only
when the drill *is* the root. A drill reached through a fuel inserter — which
is how a burner drill gets its coal — is a different drill mining a different
ore, and it was reported as producing the root's ore at the root's speed.

Reading the root is also what made the `unwrap` look safe: the root filter
(`MiningDrill && miner_ore.is_some()`) guarantees a `miner_ore` for a drill
root. An `OffshorePump` root passes the same filter with `miner_ore == None`,
so a pump-rooted walk reaching a drill aborted the process — `[profile.release]`
sets `panic = "abort"`, so nothing catches it.

**I could not construct that pump-rooted walk, and I am saying so rather than
claiming I reproduced it.** Under the current edge rules in
`EntityGraph::connect`, an offshore pump's reachable set is the fluid network:
`OffshorePump` points at an `is_fluid_input()` neighbour, and `is_fluid_input`
is `Pipe | StorageTank | PipeToGround | Boiler`. None of those has a
`drop_position` or `pickup_position`, so no item edge leaves the fluid network
into a drill. The realistic power plant — coal drill → belt → inserter →
boiler ← pipe ← pump — is walked from the *drill's* root, not the pump's. The
abort is therefore one edge rule away rather than live today. The **reachable**
half of the same defect was already returning wrong numbers, which is what the
tests pin.

### Fix

Read `source_node`. A drill with no ore under it is **not** an invariant
violation — `EntityGraph::add` stores `None` for it and warns `no ore found
under miner …` as it does so — so it now draws no flow edge and warns naming
the entity. That is the policy `get_or_create_flow_node` already documents
("the edge that wanted it is then dropped rather than drawn against a
fabricated node"), restated at the site because this walk is what turns it into
a missing flow. It is not a silent `if let`: a drill mining nothing produces
nothing, so *no edge* is the correct answer, and it is announced.

The two `panic!`s on missing `mining_speed` / `mining_time` prototypes were
left alone; they are pre-existing, name their entity, and are outside this
defect.

### Red first, with the real output

`a_drill_downstream_of_another_drill_reports_its_own_ore` builds iron drill →
belt → inserter → coal burner drill → belt. Before the fix:

```
  left:  3 -> 4 [ label = "Single([(\"iron-ore\", 0.5)])" ]
 right:  3 -> 4 [ label = "Single([(\"coal\", 0.25)])" ]
```

Node 3's own label in the same graph reads `coal burner-mining-drill at [1, 3]`.

`a_drill_with_no_ore_under_it_produces_nothing` removes the coal. Before the
fix the burner drill claimed `Single([("iron-ore", 0.5)])` out of a drill
mining nothing.

### Mutations, each failing exactly its own test

- `source_node.miner_ore.as_ref()` → `Some(… .unwrap())`, i.e. the original
  abort with the corrected subject: `a_drill_with_no_ore_under_it_produces_nothing`
  panics at that line, `a_drill_downstream_…` still passes. This is the
  process-killing path, demonstrated.
- `get(&source_node.entity_name)` → `get(&entity_root.entity_name)` for the
  mining speed only: `a_drill_downstream_…` fails (coal at 0.5 instead of
  0.25), `a_drill_with_no_ore…` still passes.

`test_splitters` and `test_furnace` — the pre-existing root-is-the-drill cases
— stay green throughout, which is how we know the common path is untouched.

---

## Defect 3 — `BOT_FORCE` defined twice

**Real.** `crates/executor/src/rcon_actuator.rs:128` and
`crates/planner/src/state.rs:65` both defined `BOT_FORCE = "player"`, each with
its own doc comment explaining the same reasoning.

### The argument, since one was invited

**One constant, not two.** The invited counter-argument is that two constants
with two tests are safer than one shared one. They would be, if they were two
independent facts that happened to coincide. They are not.
`mods/BotBridge/control.lua` hardcodes `game.forces["player"]` in
`collect_recipes`, `collect_player_force` and `start_research` — one fact about
the mod. Two copies do not give two chances to be right; they give one chance
to be *inconsistent*, and a plan made for one force and executed against
another is a strictly worse failure than either copy being wrong alone. It is
also the exact failure the planner already paid for: `forces.keys().min()`
returned `enemy` and cost two milestones
(`docs/superpowers/notes/2026-09-02-recipe-not-enabled.md`).

The "two tests" half of the argument survives unification intact, which is the
decisive point. Neither test asserts the constant's *value*; both assert
behaviour — that the lookup is by name and not by sort. They keep doing that
against a shared constant.

### Fix

`factorio_bot_core::constants::BOT_FORCE`, with the merged reasoning.
`crates/executor` imports it; its local definition is gone.

### No new test, deliberately

With one definition, "the two crates agree" is true by construction and a test
asserting it would be a tautology. A test asserting the literal `"player"` pins
nothing the mod does not already decide, and would have to be edited in lockstep
with any legitimate change — a hollow test.

The guard is the existing behavioural pair, and it still works. Mutating the
shared constant to `"enemy"`:

```
test rcon_actuator::tests::the_bots_force_answers_for_a_technology_it_has_finished ... FAILED
test rcon_actuator::tests::the_other_forces_in_the_world_do_not_get_a_vote ... FAILED
```

with every other test in the crate green. Unification costs no coverage.

### ~~Not done: the planner's copy~~ — CLOSED 2026-09-03 (`4bfccea7`)

`crates/planner/src/state.rs` now imports `factorio_bot_core::constants::
BOT_FORCE` and defines nothing. Net +1/-13, no behaviour change, no makespan
pin moved. The defect is fully closed; the two strings are one.

The deleted doc comment had gone stale in both of its claims, which is worth
recording because it is why this sat open longer than it needed to: it said
`crates/executor` held a sibling *definition* (it does not — `rcon_actuator.rs:3`
*imports* the constant), and it named the blocker as "the only place both crates
can see is `crates/core`, which belongs to other work right now" — but the
planner already depended on core and core already defined the constant. The
stated reason for the copy had evaporated some time before anyone re-read it.
The reasoning worth keeping lives in `crates/core/src/constants.rs:13-31`, and
is deliberately not duplicated back into the planner.

### `snapshot.rs`'s literal is a tripwire, not a third spelling — RESOLVED

This note previously listed `crates/core/src/factorio/snapshot.rs:476` as "same
fact, third spelling ... left". **That reading was wrong, and the item is
closed as not-a-defect.** All three `"player"` literals in that file (`:219`,
`:454`, `:476`) are inside *test fixtures*. Production reads
`forces.get(BOT_FORCE)`, so changing `BOT_FORCE` makes those tests **fail
loudly** rather than drift quietly. They are the thing that would catch a bad
change, not an instance of the defect. Do not "unify" them.

---

## Still unverified

- **`just test` has not run**, nor `cargo clippy --workspace --all-features
  --all-targets --deny warnings`, nor `cargo test --workspace`, nor
  `cargo fmt --check`. Individual files were formatted with `rustfmt` and
  `cargo clippy -p factorio-bot-core -p factorio-bot-executor --all-targets
  --deny warnings` is clean.
- **Defect 1 cannot be finished without a live run.** Everything about the
  policy is unit-tested, but that the extracted watcher is wired correctly into
  the RCON loop — that `observe` is called with a fresh `Instant`, that the
  give-up branch's second RCON call does not hang, that the warning renders —
  is only observable when four clients actually start. The value to look for in
  a run log is the give-up line naming client numbers, or its absence because
  everyone connected.
- **The pump-rooted abort in defect 2 was reasoned about, not reproduced.** See
  above; if someone adds a fluid-side edge into a mining drill (a pumpjack
  output, uranium's sulfuric-acid input), the walk becomes constructible and
  the guard is already in place.
- Someone else's in-flight edit to `crates/core/src/factorio/world.rs` made the
  core lib **test target** fail to compile for part of this session (a
  `derive(Eq)` on a struct holding a `Position`, plus a missing `inventories`
  field at construction sites). It was not mine and it resolved on its own;
  full core lib tests finished 396 passed / 0 failed afterwards.
