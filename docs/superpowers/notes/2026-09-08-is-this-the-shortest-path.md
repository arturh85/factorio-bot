# Is this the shortest path? A strategy review

2026-09-08, at the owner's request, on master `144603b2`. Read-and-think only:
no code changed. Every number below was either re-measured on this box today
(the debug binary built 2026-09-08 10:28, two and a half hours before HEAD) or
is quoted with the file it came from. Where I reason instead of measure, I say
so.

The brief that asked for this review framed its questions as hypotheses and
asked to be disagreed with. Several of its premises did not survive contact
with the tree, and those corrections are the first section, because a review
built on a stale premise is the thing this project's own memory warns about
most.

## 0. What the brief got wrong, checked first

| the brief said | what the tree says | where |
|---|---|---|
| eight goal kinds | **nine** plus `All` — `Researched` was missed | `crates/planner/src/goal.rs:198-449` |
| eight action kinds | **eleven** — `Evacuate`, `Survey`, `StampGhosts` exist | `crates/planner/src/action.rs:729-887` |
| "nothing reads `threats`" (implicit in "nothing models a threat") | `method/scout.rs` read it before 09-08; `have.rs::Chop` reads it now | `2026-09-08-a-target-inside-a-worms-reach.md` |
| "no live runs on 09-07/08" (a sub-agent's reading, not the brief's) | **55 runs** under `workspace/headless-*/runs`, the latest on 09-08; `workspace/runs` holds only the 1x client runs | `command ls workspace/*/runs` |
| `search.rs` output has no consumer | true of the **planner**; false absolutely — the `factorio-bot search` CLI consumes it, and nothing else | `app/src-tauri/src/cli/search.rs:40` |

And one thing the brief was right about that the tree confirms unambiguously:
**no science pack has ever been made by a machine that a bot did not feed, no
lab has ever been fed by an inserter, and no pumpjack has ever stood in a live
game.** Every one of those is stated in the project's own notes (the timeline
in `2026-09-06-standing-goals.md`, `2026-09-06-a-pumpjack-on-a-well.md`:
*"The one thing this cannot claim is that any of it works in the game"*). The
`factory` attribution has been earned exactly once, by a burner smelting cell
(`run-1788679826-02267`), and once more by an electric block with its own
plant (`2026-09-08-the-block-that-was-dead-is-fed.md`). Neither made a science
pack.

## 1. Shortest path, or shortest path to the next defect?

**Verdict: the work is converging on the capability set the rocket needs, but
it is being driven by whichever defect the last run exposed, and the ordering
that produces has cost at least one day on the critical path that a
one-line policy choice would have saved.** Concretely:

### The sequencing that was wrong, by name

The oil run (`run-1788833726-34821`, 2026-09-08 02:15) died to biters and a
tree. What landed afterwards, in this order:

1. `ffb6e67e` — a bot that shoots back (mod-side standing order, five live
   runs, an eight-mutation sweep).
2. `a5b7072a` — threat-aware target selection (nine tests, eight-mutation
   sweep, **oil plan wall time 93 s → 184 s**).
3. `ba3d8292` — **peaceful mode**, the owner's own suggestion (*"maybe we just
   add peaceful mode for now?"*), one flag and one provenance field.

Item 3 makes items 1 and 2 irrelevant to the oil milestone: under peaceful
mode nothing attacks unprovoked, and the run that motivated both would have
reached its rig with neither. Both are legitimate *foundation* work — the
companion/enemy use case will want them — but the rocket path needed peaceful
first and the other two later, if at all this month. **I would not have done
1 and 2 before 3.** The general form: when a run dies to something the game
has a switch for, flip the switch, record it in provenance, and keep walking;
model the thing when a use case that cannot flip the switch arrives.

The same run also lost **474 of bot 3's 482 actions to a tree at tick 7,980**
(`2026-09-08-the-first-death.md`) — a walk failure, larger than all three
deaths combined, and unrelated to combat. That is the defect on the oil path.
Nothing in the four combat notes touches it.

### The defect cadence is mostly on-path, and that is the honest reading

Categorising 2026-09-07's 51 notes: 11 capability advances, 19
measurement/record/tooling, 17 defect fixes, 4 methodology, 0 combat. Of the
17 defect fixes, the large majority — fluidbox ports, water-by-yield, power
demand, pole reach, siting parity, the anchor hijack — are *exactly* the
defects a fluid rig and an electric block trip on. They were found by trying
to build the next rung, which is how a defect should be found. The brief's
worry that this is "a treadmill that feels productive because each rung is
measurable" is half right: the *fixing* is on-path; the **stopping** is not.
Each fix ends in a note, a sweep and a merge, and the run that would have
consumed the fix is not re-attempted. The oil milestone, defined by the owner
as *petroleum in the production samples*, has had **one** live attempt.

### What I would not have done, by name

- **Return fire and the worm guard before peaceful mode** (above). Cost: a
  day of the critical path; benefit deferred to a use case not yet started.
- **`2026-09-07-what-the-build-profile-costs.md` and its sibling** — ~500
  lines to establish that `opt-level = 1` makes debug plans 4.8× faster. The
  *result* is excellent and I would keep it; the two full notes and five
  rounds of measurement on a box that "never went quiet" are more than the
  one-line finding needed. One table would have done.
- **The search prototype's live half.** `2026-09-08-a-search-that-ranks-plans-
  by-flow.md` is a genuinely good piece of work — the calibration table
  (model 75.0 vs game 75.2 on the balanced block) is the best flow-graph
  validation in the tree. But its own §5 says the ranking cannot see fuel,
  cannot score any fed block, and cannot reach a refinery, and its ranking is
  consumed by nothing. Four fresh-map headless runs were spent calibrating a
  ranker over a family (burner drills × stone furnaces) that the owner's own
  regime arc calls scaffolding to be torn down. The owner ruled "a working
  prototype", and got one; I would have stopped at the offline table and the
  two rates the record already had.

### What I would have done instead, in that time

A **peaceful-mode oil run** to the owner's definition. `scripts/oil_milestone.
lua` exists, its three recording defects are fixed, the charting is proven
(one ring, 4,159 ticks, 87% utilisation), and the composed plan's action count
matches offline to the action. The remaining blocker is the tree-walk failure,
which is a `recover.rs` question and is where the day should have gone.

## 2. What is systematically not being worked on

Candidates tested against the tree, not accepted from the brief:

| candidate absence | grep result | verdict |
|---|---|---|
| pollution | 0 hits in `crates/`, `mods/` | absent; **cheap to leave absent** under peaceful mode, because peaceful removes the *consequence* (attacks) though not evolution itself. Becomes real the day a hostile-mode number is quoted. |
| evolution | 0 hits | same |
| research **order** | `Researched::expand` (`have.rs:4098`) recurses prerequisites; nothing chooses *targets* | absent, and it is the owner's *"how do we decide at all"* question in its cheapest form (§2.2) |
| flow graph optimised against | only `search.rs`, only through the CLI; the planner reads it as a boolean veto and a diagnostic string (`sustain.rs:299,823`) | confirmed; but the fix is not "optimise more", see §2.1 |
| `search.rs` consumer | the CLI only | confirmed |
| a base that grows | `supervisor.lua` is a finite list with `max_iterations = 50`; `Sustain` is one-shot; no goal generator anywhere | **confirmed, and it is the absence that costs most** |

### 2.1 The absence that costs most: nothing can *say* "keep growing"

This is not a missing feature on a list; it is why every run plateaus. The
record already knows the symptom — *"production plateaus at the plan's bill"*
(CLAUDE.md), *"Everything tonight made the plan smaller and more honest; none
of it made the factory feed itself"* (plan, run 21) — and it has been treated
as a series of defects (furnace count, output slot, fuel charge, drain).
It is one fact: **every goal kind is one-shot or structural.** `Have`,
`Produced`, `Researched`, `Built`, `Charted` complete; `Producing` is
satisfied by machines *standing*; `Extracted`/`Gathered` by a rig standing.
The one kind that means *keep this true* is `Sustain`, and today, measured:

```
sustain:iron-plate:15:36000   plans   376 actions / 20,128 ticks
sustain:iron-plate:30:36000   REFUSES  "nothing within reach can take it away"
sustain:iron-plate:60:36000   REFUSES  (same)
sustain:copper-plate:15:36000 plans   469 / 29,582
```

**The only standing goal in the vocabulary cannot ask for more than one stone
furnace's output** (18.75/min). The owner's target — *"keep upgrading our
base, automating all the science packs … for fast research"* — is a standing
goal over the whole factory, and the system has no sentence for it. The
supervisor is the closest thing and it is a hand-written ladder that exits.

What "growth" needs is not a scheduler capability but a **goal source**: a
function from the current world (research state, measured rates, what stands)
to the next goal, called forever. The pieces exist and are not joined:
`goal.holds` answers whether a goal holds; `just analyse`'s rate marks and the
per-machine counters answer what is being made; `Researched` recurses
prerequisites; `Site::Beside` tiles a second block. Nothing calls them in a
loop that does not terminate. That is a Lua script first (a `supervisor`
source that never returns `nil`), and only later a planner concept.

The measured evidence that this matters more than any single defect: the
flow-graph validation against the world-record base puts the model within
1–32%, and the dominant error term is **idleness** — machines standing still
because nothing downstream pulls. A planner that only ever plans a bill has
no reason to build a sink. The rate model is right; the goal that would use
it does not exist.

### 2.2 Research order, in its cheap form

`researched:rocket-silo` is a single goal; the planner will recurse the whole
prerequisite DAG for it. Whether that expands at all offline is the cheapest
possible measurement of how far the vocabulary reaches, and it was run today
(§6). Nothing chooses *which* technologies to pursue when, which is what
"automating all the science packs for fast research" is about: the order in
which packs are automated is the research order. A static ordered list of
targets (automation → logistics → steel → electricity → oil → …) handed to
the goal source above is a day's work and turns the owner's question from
"how do we decide at all" into "which list", which is answerable by
measurement.

### 2.3 Fluids stop recursion, and no goal crosses the boundary

Items recurse: `have:iron-plate` expands to smelt, to mine, to place a
furnace. Fluids do not. Measured today on `map-31337-explored-with-
categories.json`:

```
produced:petroleum-gas:45:basic-oil-processing   REFUSED  nothing standing supplies crude-oil
have:plastic-bar:10                              REFUSED  nothing standing supplies petroleum-gas
producing:chemical-science-pack:1                REFUSED  no method
researched:oil-processing                        2,012 / 308,577 (1:25:42)
gathered:crude-oil                               2,117 / 325,138 (per 2026-09-08 note)
gathered + produced petroleum, as one goal.all   2,295 / 330,406 (per note; the CLI cannot say `all`)
```

The owner's ruling that a fluid is *connectivity, not amount* is correct and
landed correctly — but connectivity is not a **subgoal**. A plan for plastic
cannot say "then stand a refinery, then a rig"; a human has to compose
`goal.all{gathered, produced}` by hand, and plastic is one more hop that no
script has written. Every science pack from blue upward is behind this. It is
one method — "a fluid ingredient with no standing supplier is met by a
`Produced` of that fluid" — and it is the single most valuable planner change
on the path.

### 2.4 Teardown and upgrade are unsayable

The owner's regime arc is *burner chains → electric drills → electric
smelters*, and *"smelters force a teardown"*. There is no action that removes
a placed entity (`Remove` is inventory removal; `Mine` and `Chop` take
resources and rocks) and no notion of `fast_replaceable_group` anywhere in
the planner or executor (0 hits). A base that upgrades cannot be planned in
this vocabulary; it can only be planned *beside* the old one. This matters
later than 2.1–2.3, but it is the second structural gap the compounding goal
hits, and it is worth knowing before the first electric-furnace block is
sited on ground a burner block occupies.

## 3. Is the abstraction right?

**It fits the last hundred hours well and the next hundred only partly.**
The vocabulary is *item-and-entity* scoped: every goal names an `ItemId` and a
count or a rate; every action names an entity at a position. That is exactly
the hand era — a bot carrying things — and it is why the hand era went from
nothing to green science in a week. Three places where the next hundred
hours push against it:

- **Force-level acts.** `create_space_platform` has no `LuaEntity` receiver
  and cannot be any of the eleven verbs; the note establishing that is
  correct and the one new `ActionKind` it names is small. `Research` is
  already force-level, so there is precedent. Not urgent — it is the *last*
  step — but it proves the vocabulary is entity-shaped by assumption, not by
  design.
- **Standing over one-shot** (§2.1). `Sustain` is the right *kind*; its one
  expansion (`method/sustain.rs`) is a bespoke burner cell, and the refusal
  at 30/min is the method, not the goal. The goal should stay; the method
  should become "a `Built` block chosen for the rate, plus its
  `connect`s", which is what `search.rs` ranks and nothing consumes. **That
  is the wire-or-delete decision for `search.rs`: wire it here.**
- **Fluids as a boundary** (§2.3). The goal kinds are fine; `Produced` can
  name a fluid. The recursion is what stops.

What is *not* wrong: the eleven actions map one-to-one onto mod functions
(`run.rs:1232-1352`, one match), every goal is reachable from Lua, and the
"a kind lives in seven places" cost is real but bounded and documented. I
would not reshape the enum. I would add one goal-source loop above it, one
fluid-crossing method inside it, and one force-level verb when the silo
exists.

Combat needing a mod-side standing order rather than a verb was the right
call — a reflex is not a plan — and the same reasoning applies to the tree
blocking bot 3: that is recovery, not vocabulary.

## 4. Is the measurement discipline paying for itself?

**Mostly yes, and the part that is not is not the sweeps.**

### Earning its keep

- **Falsification sweeps: about one in two finds something.** Across 17
  notes reporting a sweep, 9 found a real defect — two accidental passes
  (`8b1350fc`, `which-port-takes-which-fluid`), a field with no reader, a
  test asserting the right outcome for the wrong reason (*"nothing but the
  sweep would have found that"*), a harness that reported a genuine red as
  "no test matched". That yield is high, and the class of defect it catches
  (a green suite proving nothing) is the class nothing else here catches.
  Keep.
- **The offline planner as the first tier.** Every planner claim today was
  checkable in 1–34 s on a debug binary; the whole oil ladder measured in
  under three minutes. This is the single best investment in the tree.
- **Retractions.** Ten distinct claims withdrawn before they compounded,
  including one retraction that overshot and was itself corrected.
- **The build-profile result** (not the notes about it): 4.8× faster
  iteration for one line.

### Ritual, or paying more than it returns

- **CLAUDE.md at 2,091 lines / 18,700 words, plus 36 memory files.** Every
  agent reads ~25k tokens of scar tissue before its first tool call, and the
  scar tissue *still* did not prevent, today, a sub-agent producing a
  denominator artefact ("live runs stopped") of exactly the shape CLAUDE.md
  warns about, or the `justfile` carrying *"The seed has NOT been validated by
  a run yet"* two days after CLAUDE.md said it was. **A number in prose is a
  cache; a 2,000-line file of them is a cache with no eviction.** Split it:
  a short operational file (build, run, the three-tier ladder, the six
  aliases) and move every incident narrative into the notes with a one-line
  index entry. Target under 600 lines. This is the practice I would change
  first, because its cost is paid on *every* task by *every* agent.
- **Notes are append-only: 43,530 lines added, 578 deleted since 09-02.**
  The corpus is a ledger, which is right, but there is no index, and the
  record is now large enough that its own authors re-derive things it
  already holds (the `ElectricOreToPlate` mixed-lane control was re-read as
  a fixture defect for a day, thirty lines below the note that explained it).
  A weekly `INDEX.md` — one line per note, grouped by subsystem — is an hour
  and would have saved that day.
- **Re-stating the three baselines in every note.** Cheap (~15 s) and
  correct; keep, but as a script that prints the table (`tools/baselines.sh`)
  rather than four hand-typed numbers that the memory says go stale.
- **`cargo test --workspace` after every merge, and its cost is the one
  number the project never measured** — the build-profile note says so
  ("attempted and contaminated"). With 3,088 tests and 58.7% of planner
  source inside `#[cfg(test)]`, this is likely the largest single line item
  in wall time per merge, and it is unknown. Measure it once, then decide
  whether crate-scoped tests plus a nightly full run is enough. I suspect it
  is.
- **The threat guard's 2× plan wall time** (93 → 184 s on the oil goal) is
  offline, documented, and only bites on the one goal that reaches threats.
  Not a problem today; it becomes one the first time a search loops over
  plans. Leave it, with the spatial index on the list.

### The cost that is not measurement: coordination

1,015 commits and 163 merges in six days, from several sessions each running
several agents. The record documents the coordination failures honestly —
an amended commit that was someone else's, a mod symlink pointing into a
removed worktree, an experiment whose apparatus died with its worktree,
briefs wrong "eleven times in one session" — but it does not price them. My
read: **the parallelism is right for orthogonal foundation work and wrong
for the critical path.** Oil, the fluid-crossing method and the science cell
are one lane; splitting them across sessions produced two competing siting
stories (`search::site` vs `search_site`, noted in the search note itself)
and a recovery-by-geometry hijack between blocks built by different sessions
on one map. One session owns the path to petroleum; everyone else stays off
`method/{gather,extract,fabricate,pipe,sustain}.rs` until it lands.

## 5. The next week, ranked

Costs are my estimates in agent-days; each item states why it beats the one
below it.

1. **Petroleum in the production samples, headless, peaceful, on 31337.**
   ~1–2 days. Beats everything below because it is the owner's stated
   milestone, every piece is proven except one, and the one is a recovery
   defect (the tree at tick 7,980) that will bite every long walk from now
   on. Fix that in `recover.rs`, re-run `oil_milestone.lua --peaceful`,
   re-plan on the milestone savepoint if it stalls. **No combat work until
   this lands.**
2. **The fluid-crossing method** (§2.3): a fluid ingredient with no standing
   supplier expands to `Produced` of that fluid. ~1–2 days. Beats 3 because
   without it every pack from blue up needs a hand-composed `goal.all`, and
   `have:plastic-bar:10` is the offline test that says when it is done.
3. **One science pack made by machines that nobody feeds**, measured
   `factory` at the 10- and 15-minute marks. ~2 days. Use what exists: a
   `Built` smelting block with its plant (proven 09-08), `connect` into the
   red-science cell (proven 09-06), an inserter into the lab (never done —
   the lab has only ever been hand-`insert`ed). This is the first end-to-end
   automation the project will have shown and it retires the `roster-fed`
   verdict that has stood on every run. Beats 4 because 4 needs something to
   sustain.
4. **A goal source that does not terminate** (§2.1): a supervisor source
   over a static research-order list plus rate floors per pack, in Lua, with
   `max_iterations` removed for that source. ~2 days. Then `Sustain`'s method
   re-expressed as "a `Built` block ranked by `search::evaluate` for the
   asked rate" — which is the wire-or-delete answer for `search.rs`. Beats 5
   because it is the compounding term the owner named.
5. **Shrink CLAUDE.md and index the notes.** ~0.5 day. Cheap, and it lowers
   every subsequent task's cost; below 4 only because it moves no milestone.
6. **Measure `cargo test --workspace` once**, then set the per-merge check to
   crate scope plus a nightly full run. ~0.25 day.

**Stop doing**, this week: combat modelling (`shooting_selected`, ammo
resupply, a forward base); calibrating the search ranker on burner families;
notes over ~150 lines for a result that fits in a table; measured 1x client
runs of anything the headless tier already answered (the record shows the
owner has said this twice).

**Not decided by this review, and should not be inferred from it:** whether
"oil is not a t=0 target on this seed" — one peaceful run answers it, and
that is item 1.

## 6. The numbers taken today

Debug binary `target/debug/factorio-bot` (built 2026-09-08 10:28, master
between `bee324dd` and `144603b2`), `--bots 1,2,3,4`, load average 1.5.

| goal | world | actions | ticks | wall |
|---|---|---:|---:|---:|
| `researched:automation` | t=0 | 176 | 21,784 (6:03) | 1 s |
| `producing:automation-science-pack:6` | t=0 | 316 | 22,457 | 3 s |
| `producing:logistic-science-pack:6` | t=0 | 441 | 47,478 (13:11) | 7 s |
| `researched:logistic-science-pack` | t=0 | 267 | 37,425 | 3 s |
| `have:steel-plate:10` | t=0 | 318 | 35,690 | 2 s |
| `researched:advanced-material-processing` | t=0 | 787 | 117,062 (32:31) | 15 s |
| `researched:electric-energy-distribution-1` | t=0 | 904 | 158,042 (43:54) | 24 s |
| `have:electric-mining-drill:5` | t=0 | 422 | 29,191 | 4 s |
| `have:solar-panel:1` | t=0 | 1,447 | 246,432 (1:08:27) | 34 s |
| `sustain:iron-plate:15:36000` | t=0 | 376 | 20,128 | 2 s |
| `sustain:iron-plate:30:36000` | t=0 | REFUSED | offtake arm has no fuel branch | 2 s |
| `sustain:iron-plate:60:36000` | t=0 | REFUSED | same | 2 s |
| `producing:military-science-pack:1` | t=0 | REFUSED | no method | 4 s |
| `researched:oil-processing` | explored | 2,012 | 308,577 (1:25:42) | 134 s |
| `produced:petroleum-gas:45:basic-oil-processing` | explored | REFUSED | no standing crude supplier | 1 s |
| `have:plastic-bar:10` | explored | REFUSED | no standing petroleum supplier | 0 s |
| `producing:chemical-science-pack:1` | explored | REFUSED | no method | 1 s |
| `have:electric-furnace:1` | explored | REFUSED | sulfur: which chemical-plant port takes which fluid is not in this dump | 59 s |
| `researched:rocket-silo` | explored | REFUSED | same, after 51 s of recursing the whole prerequisite DAG | 51 s |

The last two refusals are worth reading precisely. `researched:rocket-silo`
recursed the entire technology DAG down to sulfur and stopped at the
two-fluid wall — the wall that `4d4cf2b3` ("the game says which fluidbox
takes which fluid, so ask it") removed on 09-08. It stands here because
`map-31337-explored-with-categories.json` **predates the field**, and the
planner says so rather than guessing. Two consequences: the dump needs
regenerating before any oil-ladder number is quoted again (the third
dump-freshness trap in a week, after `inventories: []` and
`crafting_categories`), and the ladder reached sulfur before refusing, which says the recursion
gets at least that far. **Whether the rest of the DAG expands is unknown** —
a refusal is the first wall hit, not a census of the walls — and I am not
claiming it does. Regenerate the dump, re-run the two goals, and read the
next refusal; that is a ten-minute measurement and it is the cheapest
statement available of how far the vocabulary is from the rocket.

The three baselines match the record to the action, on a binary 2.5 h older
than HEAD — which is itself a small piece of evidence that the day's twenty
merges did not move the plan, as each of their notes claimed.

Two readings of the table that are not in any note:

- **The green-to-oil cliff is a factor of six in game time** (13 min → 86
  min) and it is *walking*: the `researched:oil-processing` plan is 69%
  utilised over four bots, the same as green, so the extra time is distance,
  not idleness. Peaceful mode does not shorten it. A forward base, or a
  policy that oil waits for a rate the base can afford to send bots away
  from, is a real question and item 1 will put a number on it.
- **Solar at 1:08 and `electric-energy-distribution-1` at 0:44 are both
  research-walks**, and nothing in either plan builds a second furnace to
  make the walk shorter — because no goal asks for a rate. §2.1 again.
