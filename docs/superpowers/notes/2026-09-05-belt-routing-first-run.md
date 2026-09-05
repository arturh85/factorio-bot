# Belt routing, first run: it cannot be run yet

Task 5 of the belt-routing plan (`.superpowers/sdd/2026-09-05-belt-routing/`)
was to verify Tasks 1-4 offline, then live, then write down what does and does
not work. The headline finding surfaced at the *offline* step, before any game
was started: **`connect_steps` — the whole deliverable of Tasks 1-4 — has no
caller anywhere in the tree.** It is reachable from its own unit tests and
nothing else. A live run was not attempted, for reasons given below, and that
is itself the result this note reports.

## Step 1 — workspace green

```
CARGO_TARGET_DIR=.../route/target nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings
CARGO_TARGET_DIR=.../route/target nix develop -c cargo test --workspace
```

Both clean at `ea07a855`: clippy zero warnings, `cargo test --workspace`
all green (every `test result: ok`, `0 failed` in every crate, including the
`factorio-bot-planner`, `factorio-bot-core` and doctests). This matches Task
4's own report, which said the branch had been compiled, tested,
clippy-cleaned and rustfmt'd in review round 2 — confirmed independently here.

## Step 2 — offline plan, and the finding

Override 2 applies: `workspace/scripts/map.json` is the seed-31337 t=0 dump
(fingerprint confirmed via `score-map` below), not `map-t0-baseline.json` as
the brief said — the baseline is a different, older map, and the live run in
Step 3 was going to be seed 31337, so both halves need to read the same world.

```
$ ./target/debug/factorio-bot score-map --world workspace/scripts/map.json --bots 1,2,3,4
...
fingerprint    c161fa3f437221d0
verdict        VIABLE for rung 1
actions        177
makespan       22044 ticks (00:06:07)
```

Fingerprint matches the override's `c161fa3f437221d0`. Good map.

Then the actual comparison, `producing:iron-plate:60`, on the **same map
file**, run twice: once with the master build (`/home/arturh/projects/private/factorio-bot/target/debug/factorio-bot`,
no belt-routing code at all) and once with this branch's own build
(`ea07a855`, `.worktrees/route/target/debug/factorio-bot`):

| | master (no routing) | `belt-routing` (`ea07a855`) |
|---|---|---|
| actions | 45 | 45 |
| makespan | 10262 ticks (00:02:51) | 10262 ticks (00:02:51) |
| `transport-belt` steps | 0 | 0 |
| `place inserter` steps beyond existing | 0 | 0 |

**The two plans are byte-identical** except for a `Git tree is dirty` banner
line that has nothing to do with this branch's code. Every iron delivery in
both plans is a bot standing at the furnace doing `#19 insert 1 iron-ore`,
`#26 insert 5 iron-ore`, etc. — hand-inserts, exactly as on master.

This is not "the router chose not to route here" — it is that **nothing in
the goal-driven planner ever calls the router**:

```
$ grep -rln 'connect_steps\|route_belt\|ConnectRefusal' crates/ --include='*.rs'
crates/planner/src/method/connect.rs   (definition + its own tests)
crates/planner/src/test_world.rs       (fixture used only by connect.rs's tests)
crates/core/src/graph/route.rs         (definition + its own tests)
crates/core/tests/route_grid.rs        (route.rs's own fixture tests)
```

No hit in `produce.rs`, `assemble.rs`, `have.rs`, `extract.rs`, or `power.rs`
— the five method modules that actually run under `expand()`. Confirmed
structurally, not just by grep:

- `Goal` (`crates/planner/src/goal.rs`) has exactly four variants — `Have`,
  `Produced`, `Producing`, `Extracted` — no `Connect` variant exists to expand
  into `connect_steps` in the first place.
- The CLI's `parse_goal` (`app/src-tauri/src/cli/plan.rs`) only ever parses
  `researched:`, `produced:`, `producing:`, `extracted:` goal strings.
- No Lua binding in `crates/scripting_lua/src/globals/` names `connect`,
  `route_belt`, or anything from `method::connect`. The one `--connect` in
  `app/src-tauri/src/cli/lua.rs` is the unrelated "attach to an already
  running server" flag.
- `assemble.rs`'s cell layouts place their inserters directly, by a
  hand-written convention predating this branch (`INSERTER` const, fixed
  offsets) — they do not call `inserter_facing()` or route anything.

So `connect_steps` is live, tested, correct-by-its-own-tests code with **zero
production callers**. Tasks 1-4 built the primitives (grid search, the
underground-disabled routing rule, the one-owner inserter-facing rule, and
refusal-before-build action emission) exactly as specified and all green, but
no task in this plan wired them into `expand()`, into a new `Goal` variant, or
into a Lua entry point. That wiring is not listed in any of the four task
briefs, and the plan's own self-review confirms scope: "splitters, balancing,
throughput sizing, fluid pipes" are named as out of scope, but nothing in the
self-review names the *integration point* as in scope either. It fell through
a gap between "the primitives are built" and "something calls them."

## Step 3 — live run: not run, and why

The brief's Step 3 runs `factory_stage2.lua --headless ... --seed 31337`. That
script drives entirely through `goal.researched(...)` and
`goal.producing(...)` (`scripts/factory_stage2.lua` lines 218-219), i.e.
through the exact same `Goal` → `expand()` path exercised offline in Step 2.
Since that path has no route to `connect_steps` — no `Goal` variant, no method
module call — a live run of this script cannot reach the belt-routing code
for the identical structural reason the offline plan didn't. It would
reproduce the same schedule (hand-inserts, no belts) that master already
produces, at the cost of the shared machine's ~15-minute paused-build window,
for no new evidence beyond what the static check above already establishes
with certainty.

Given that, a live run was not started. This is a judgement call, stated
plainly so it can be checked: if there is a path to `connect_steps` I missed
(a feature flag, a different script, a Lua binding under a name I didn't
grep for), that would invalidate this reasoning and the live run should be
redone. I looked for one (see the grep and structural checks above) and found
none.

## What this means for "does it work"

Unknown, and unknowable from this branch as merged into a goal or a live run
today. What Tasks 1-4 established (via their own fixture tests, not this
task) is that `route_belt`, `inserter_facing`, and `connect_steps`'s
refusal-before-build behaviour are internally consistent and pass their own
tests. What this task adds is: **that correctness has never been observed
producing a placed, functioning belt in a real game**, because nothing in the
shipped system calls the function. The two failure modes this project has
already paid for once each — a placement that looks right and does nothing
(inserter facing backwards; ghosts that don't collide) — could not even be
checked for, because there is no placement to inspect. A future task that
wires `connect_steps` into `assemble`/`produce` (as Task 4's own report flagged
as a scope note about `blocking_boxes_within` not seeing the plan's own
`added` overlay) needs to re-run exactly this offline/live check once that
wiring exists, and this time watch `samples.jsonl` for a destination
container's contents actually rising across consecutive samples — a
placement count, ghost or real, is not evidence.

## Commands run

```bash
CARGO_TARGET_DIR=$PWD/target nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings
CARGO_TARGET_DIR=$PWD/target nix develop -c cargo test --workspace
CARGO_TARGET_DIR=$PWD/target nix develop -c cargo build --no-default-features --features cli,lua

nix develop -c ./target/debug/factorio-bot score-map \
  --world workspace/scripts/map.json --bots 1,2,3,4      # main checkout binary, fingerprint check

nix develop -c <main-checkout>/target/debug/factorio-bot plan \
  --world workspace/scripts/map.json \
  --goal producing:iron-plate:60 --bots 1,2,3,4 --steps   # "before" (no routing code)

nix develop -c ./target/debug/factorio-bot plan \
  --world /home/arturh/projects/private/factorio-bot/workspace/scripts/map.json \
  --goal producing:iron-plate:60 --bots 1,2,3,4 --steps   # "after" (belt-routing @ ea07a855)
```

No Factorio server was started. Ports 34200/4324 were never bound; the
settings file prepared for the live run
(`.worktrees/route/scratch/route-headless-settings.toml`, ports 34200/4324,
its own workspace under `~/.local/share/factorio-bot-dev/workspace-route-belt`)
was not used.

---

## Addendum: the whole-branch review found the code could not connect any real machine

The section above says the primitives "are internally consistent and pass
their own tests". That was true and it was not worth much. The final
whole-branch review, run before any merge decision, found a **Critical**:
`connect_steps` treated `from` and `to` as 1×1 entities whose position is a
tile centre, and **every machine the spec names is neither**.

Two independent halves, both on open ground with no obstacle anywhere:

- **Footprint.** The inserter tile was the first free cardinal neighbour of
  the machine's *centre cell*. A 3×3 (assembling machine, lab, electric
  mining drill) has all four of those inside its own rasterised footprint, so
  the connection refused `NoRoute`. A 2×2 (stone furnace, burner drill) has
  two of four inside it, so the chain bent and the belt endpoint ended up
  diagonal from the machine.
- **Alignment.** An entity covering an even number of tiles on an axis has its
  centre on a tile *boundary* — a stone furnace sits at an integer.
  `method::util::tile_alignment` had already written that rule down, and says
  getting it wrong cost this project a day once. Every position
  `cell_to_position` produces is a half-integer, so for any even-footprint
  machine both `dx` and `dy` came out non-zero half-integers,
  `inserter_facing` returned `None`, and the connection refused
  `NotCardinal`. Always.

Drill → furnace → chest, the spec's own first consumer, therefore refused at
**both** machine ends; only the chest (1×1) would have worked.

### Why four reviews missed it, and what that says about the no-caller ruling

**The fixtures had been built to fit the code.** `test_world`'s furnace sat at
`(0.5, 0.5)` — not a position a 2×2 entity can occupy in Factorio — with its
box shrunk to `1.3984375` and a doc comment explaining, approvingly, that this
made it block *exactly one cell*. The production constructor
`FactorioEntity::new_stone_furnace` uses `1.8`. A fixture that is 1×1 and
half-integer is precisely the one input for which "treat the machine as a
tile" is lossless, so every geometry assertion passed.

The plan's closing ruling — stop before wiring `connect_steps` to a caller,
because the wiring lands in files another session was editing — is still
defensible on its own terms. But **the absence of a caller is what hid this**:
nothing but a hand-written fixture ever ran the code, and the fixture was
written by the same task. A primitive with no caller is not "complete and
tested on its own terms"; it is untested against reality, and the ledger
sentence claiming otherwise has been corrected.

### What the fix wave changed

- `connect_steps` now takes the two `FactorioEntity`s and reads each one's
  `bounding_box`. Both footprints are marked occupied on the grid; each end is
  three collinear cells (the footprint cell the inserter reaches into, the
  inserter, the belt) taken from the machine's **perimeter**, scanned N/E/S/W.
  Facings are computed between two cell centres of one grid, so
  `ConnectRefusal::NotCardinal` is now unreachable by construction and is kept
  as an assertion.
- The obstacle grid is rasterised with the belt's own half-box (`0.4`), not
  zero. With zero a cell counted as blocked only when an obstacle covered its
  exact *centre*: grid-aligned buildings were accidentally safe, but **a tree
  or a rock sits at an arbitrary sub-tile position** and a 0.8-wide box can
  miss a tile centre while leaving no room for a belt — so the route was
  planned straight through it and the build would have failed partway,
  breaking the module's own refuse-before-placing promise.
- The materials bill now exists: `Goal::Have` subgoals plus `HasItem`
  preconditions and `LoseItem`/`CreateEntity` effects on every `Place`, with
  real `ActionId`s, `AreaFree`, `AtPosition` and `PLACE_TICKS` — the shape
  `power.rs` emits. Three of the spec's four refusal paths existed only on
  paper before this.
- The fixtures are rebuilt on legal Factorio positions: a `stone-furnace` from
  the production constructor at the integer `(5.0, 5.0)`, a `lab` with its
  real `2.3984375` box at the half-integer `(12.5, 5.5)`.

**Each of the three defects was reproduced against the new tests before the
fix was accepted**, by reverting the fix in place and watching the tests go
red: forcing a 1×1 footprint fails all three connection tests; taking the
facing from the machine's raw position fails them with `NotCardinal`; setting
the half-box back to zero routes the belt straight through the tree.

### What is still not established

Everything the section above says. There is still no caller, so no belt has
been placed in a real game, and the two failure modes this project has paid
for — a placement that looks right and does nothing, and a ghost count that
validates nothing — still cannot be checked for here. The fix wave makes the
geometry *capable* of connecting a real machine; it does not show that one
ever has.
