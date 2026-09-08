# A goal a script could not say

2026-09-08. Branch `a-goal-a-script-cannot-say`, off `6ea4c649`.

## The gap

The planner has ten `Goal` variants. Lua could build **seven**.

```
planner (crates/planner/src/goal.rs):
    Have  Researched  Produced  Producing  Sustain
    Extracted  Gathered  Built  Charted  All

lua, before this change:
    have  researched  producing  sustain  built  charted  all
```

`Gathered`, `Produced` and `Extracted` had no constructor, no `goal_from_lua`
arm, no `render_goal` arm and no entry in `KINDS`. They were reachable from the
`plan` CLI's shorthand (`produced:<item>:<count>[:<recipe>]`,
`gathered:<entity>` — note `extracted` is missing there too) and from
`--goal-json`, and from nowhere in Lua.

**Why that was load-bearing.** The owner defines the oil milestone as *a
headless run that builds the rig and where petroleum appears in the production
samples* — a green offline plan explicitly does not count. Every headless run
is driven by a Lua script. The milestone's goal is

```
All[ Gathered{crude-oil},
     Produced{petroleum-gas, count 45, whose Anyone, via basic-oil-processing} ]
```

so the milestone was unreachable from the only path that can satisfy it, for
reasons that had nothing to do with oil, pipes, or the planner.

This is the repo's `absent-is-not-a-value` family in a different dress: a list
that must stay in sync with a parser, where being missing produces a
*confident wrong answer* — `unknown goal kind "gathered"` — about a kind that
exists.

## Seven places, not three

The task brief said "at least three places a kind must appear — constructor,
parser, and `KINDS`". Measured: **seven**, five in `value.rs` and two outside
it. The list is now in `install_goal_constructors`' own doc so the next person
does not reconstruct it:

| # | place | what fails without it |
|---|---|---|
| 1 | the constructor | `goal.gathered` is `nil`; a script calling it dies |
| 2 | `goal_from_lua`'s arm | builds fine, refuses at `goal.plan` as unknown |
| 3 | `render_goal`'s arm | builds and plans fine, `tostring(g)` raises |
| 4 | `KINDS` | works everywhere **except inside `goal.all`** |
| 5 | a test composing it inside `goal.all` | 4 has no other way to fail |
| 6 | `__doc_entry_<kind>` in `goal/mod.rs` | `lua_docs::tests::the_goal_doc_entries_are_exactly_the_goal_surface` |
| 7 | the expected set in `the_goal_table_offers_exactly_the_new_surface` | that test, from both directions |

6 and 7 are the two that *already* fail loudly, and they are the reason this
was a bounded change rather than a hunt: both are set comparisons against the
functions actually installed, so they fail on a missing entry **and** on an
extra one. 1–4 have no such guard between them, which is exactly how `charted`
came to work everywhere but inside `goal.all` for a day.

**A stale comment was doing damage.** `KINDS` carried a doc paragraph calling
`charted`'s absence "a pre-existing defect ... left alone rather than fixed
quietly". `charted` had since been added, one line below, in the same
constant. The paragraph read as a live warning about the current code and was
a description of code that no longer existed. Replaced with the transferable
half — *a kind added to the parser and not to this list fails in exactly one
place* — and the seven-place table above.

## What was exposed, and what `unlocks` is

- **`goal.produced(item, count, opts)`** — `{ bot =, via =, unlocks = }`.
  Shaped like `goal.have` on purpose: same want, different question. `have` is
  satisfied by what a bot already holds; `produced` never subtracts an
  inventory, because a bot carrying six labs has not crafted one and a
  Factorio 2.0 `craft-item` trigger fires on the act.
- **`goal.gathered(entity, opts)`** and **`goal.extracted(entity, opts)`** —
  an **entity**, not an item, and no count. What comes out of a well is a
  fluid no character inventory can hold. `extracted` is the machine working
  the well; `gathered` is that plus a tank for what it pumps.
- `via` is wired on `produced` because four recipes on this install make
  petroleum-gas, so asking for the product alone is ambiguous and is correctly
  refused. The owner's ruling is that **the goal names the recipe**. `via` on
  `Producing`/`Sustain` is out of scope by a separate owner ruling.
- `unlocks` is exposed on all three and documented as **a claim, not a
  grant**. Nothing validates the technology name — a wrong one makes the
  *plan* believe a technology is finished while the game disagrees. The `plan`
  CLI routes it through `--goal-json` only; the Lua option is the same
  capability with the same caveat written down beside it.

`extracted` was cheap and honest to add, contrary to the brief's hedge: it is
`{ entity, unlocks }`, the same shape as `gathered`, and `method::extract`
has claimed it since 2026-09-06. `extract.rs`'s own test at line ~1011 already
carries the comment *"A goal with no `unlocks` -- `goal.extracted("crude-oil")`
from a script"*, describing a call that could not be written until today.

## Verification

- **The goal crosses into the planner.** On the stub world, the oil
  composition refuses — there is no crude oil there — and the test asserts the
  refusal does **not** contain `unknown goal kind` and *is* classifiable by
  `goal.refusal`. That distinction is the whole point: a kind missing from
  `KINDS` refuses with a fact about `value.rs` dressed up as a fact about the
  world.
- **Each new kind composes inside `goal.all`**, which is the seam that broke
  last time and the only construct that reads `KINDS`.
- **`__tostring` agrees with `Goal::Display`** for all three. `have` and
  `producing` predate that rule and are deliberately left with their own
  wording; nothing new should add a third dialect.
- `cargo test --workspace --no-fail-fast`: **3,054 passed, 0 failed**, cargo's
  own exit code 0 (captured, not read off a pipeline — a pipeline reports
  grep's status). 3,044 on master plus the 10 tests added here.
- Every new test falsified one at a time, backed up by file copy and restored
  by copy plus `touch` — never `git checkout --`, which restores the last
  *committed* state and ate an agent's uncommitted work in four files the
  night before. **10 of 10 RED**, every anchor matching exactly once, no
  mutation failing to compile (a compile failure reads as red too). See
  `scratch/falsify.py` on the branch.
- `cargo clippy -p factorio-bot-scripting-lua --all-targets -- --deny
  warnings`: clean. `pnpm lint` not run and not applicable — no wire type
  moved, so neither `openapi.snapshot.json` nor `api/types.ts` is touched.

### Baselines: none moved

Measured on **one binary**, this branch's own debug build of `6ea4c649` plus
this change (`cargo build --no-default-features --features cli,lua`), four bots:

| goal string | world | actions | ticks |
|---|---|---:|---:|
| `researched:automation` | `map.json` | 176 | 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 | 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 | 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,115 | 317,283 |

All four are identical to the figures the brief carried, which is what the
diff predicts: this change touches `crates/scripting_lua` and one script, and
no planner file. The goal string is stated beside each number deliberately —
attaching a right number to the wrong goal cost three builds of bisecting a
regression that did not exist.

### And the composed goal reaches the planner against a real world

```
factorio-bot plan --world workspace/scripts/map-31337-explored-with-categories.json \
  --goal-json '{"All":[{"Gathered":{"entity":"crude-oil"}},
                       {"Produced":{"item":"petroleum-gas","count":45,
                                    "whose":"Anyone","via":"basic-oil-processing"}}]}' \
  --bots 1,2,3,4

Error: the goal did not expand: bot 1 owns chain ChainId(451) because its bill
       was sized against it, but has 5 iron-ore does not hold there
```

**That refusal is the success condition, not a failure.** It is a fact about
the map and the schedule; `unknown goal kind "gathered"` would have been a
fact about `value.rs`. The `Goal` value in that JSON is asserted equal to what
`goal.all { goal.gathered(...), goal.produced(...) }` converts to, in
`the_oil_milestone_goal_is_expressible_from_lua`, so the two cannot drift.

The chain-ownership refusal itself is somebody else's work and has since
merged to `master` as `9a981d54`; this branch is off `6ea4c649` and does not
carry it.

## What this does not do

It does not make the oil milestone pass. `scripts/oil_milestone.lua` states
the composition and drives it, and reports a refusal as a stuck milestone
rather than pretending. Whether it plans depends on the chain-ownership work
that merged separately as `9a981d54`, which this branch is not based on.

And the script makes **no attribution claim**. A rising production curve is
not evidence of a working factory — `production.made` counts what a machine
produced, and a machine a bot hand-loaded is a machine. The script starts the
sampling session, holds an idle roster for a tick-bounded window so the
samples cover the period petroleum would appear in, and points the reader at
`just analyse`'s `roster-fed` / `factory` / `unclear` verdict. Oil happens to
be the one commodity hand-feeding cannot fake, since no inventory holds a
fluid — but that is still the analyser's call and not the script's.
