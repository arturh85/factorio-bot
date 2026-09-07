# A test written beside the code agrees with the code

2026-09-06. Four instances in one night, across two sessions, each found
only by measuring against the running game. This is not four coincidences;
it is one failure mode with four faces, and it is the most expensive thing
in this project's history apart from unrepeatable measurements.

## The four

*(Five, with the harness below, and seven with the two entries after it —
the heading is kept because the four are the original set.)*

1. **A blueprint fixture placed a stone furnace where a 2×2 entity cannot
   legally stand.** The belt-routing primitive passed four clean task
   reviews and a full suite while being unable to connect any real machine.
   Only a whole-branch review that checked the fixture against real
   prototype geometry caught it.
2. **A resource fixture gave its ore tile a `unit_number`.** Resource
   entities do not have one. The drill accumulator keyed on it, credited
   nothing to any drill, and reported `produced: 0` for a drill that had
   just mined 133 ore — which was then written into the plan record as a
   finding about the *planner* before it was retracted.
3. **An obstruction test placed a decoy facing the blueprint's own default
   direction**, so the rule "an entity standing as designed is not an
   obstruction" read the decoy as the block already standing. The tests
   would have passed while asserting nothing.
4. **A replan-stability test never moved the roster**, so it would have
   passed under the roster-centroid seed it exists to forbid.

## Why it keeps happening

In each case **the same task wrote the code and the fixture**. A fixture is
a statement about the world; when its author is the code's author, it is
written to be the world the code expects. Nothing in a green suite can
detect this, because the suite is exactly the thing that has been made to
agree.

Two constants had the same shape without a fixture: `R + 1.1` as a walk
margin, carried from one observation in August against a measured worst
case of 0.301; and a walk speed of 0.15 taken from the prototype against a
measured 0.1413 over 196,717 ticks. Documented prose is a fixture too.

## The move that would have caught all four, in seconds

**Make the test fail on purpose before believing it.** Swap in the value the
test exists to forbid, watch it fail with the actual mismatch, revert, watch
it pass. The siting session's implementer did exactly that for the
replan-stability Critical — substituted the forbidden roster-centroid seed,
watched `Pos(0,0)` against `Pos(500,500)`, restored — and its re-reviewer
then **independently reproduced the experiment rather than trusting the
report**, doing the same for a second finding by flipping a flag back and
watching the bystander test fail.

Every one of the four above would have been caught this way. The drill
accumulator reporting `produced: 0` would have been obvious the moment
someone asked it for a number it should have been unable to produce; the
decoy furnace would have failed the instant it faced a direction the
blueprint did not; the 2×2 furnace fixture would have failed on any legal
position.

A green test proves nothing until it has been seen to go red for the right
reason. Treat "it passes" as an unverified claim about the test.

**And the rule has its own failure mode, found the same night by an agent
following it.** A scripted forbidden-value substitution **silently matched
nothing** against reflowed source, so the test passed — and a pass under
substitution reads exactly like *"the forbidden value changes nothing, so
the test is hollow"*, which was the opposite of the truth. The agent refused
that green and redid the experiment by hand. Two guards follow:

- **Assert the substitution actually matched** before drawing any conclusion
  from what happened next.
- **An unexpected green under substitution is a broken experiment, not a
  finding.** Investigate the experiment first; only after it is shown to
  have bitten may the green be read as evidence about the test.

A fixture that agrees with its code is the first trap; a falsification
performed as a ritual without its effect is the second, and it wears the
costume of the cure.

## The sibling: a crate-scoped check cannot see a crate it does not compile

Same night, same shape, different surface. A `PlannerError::NoSiteFound`
variant was added three tasks before anyone noticed that
`crates/scripting_lua/src/globals/goal/mod.rs:355` matches `PlannerError`
exhaustively. **Three tasks passed and two reviews signed off**, because
every brief said `cargo test -p factorio-bot-planner` — so the crate that
breaks was never compiled once. The Global Constraints did say
`--workspace`, but no task *step* ever did, which made the constraint
decorative.

**A task that adds a variant to a shared enum must build every crate that
matches on it.** A crate-scoped check cannot see a break in a crate it does
not compile, and neither can a crate-scoped review. If a brief names a
crate-scoped command in its steps, that is the command that will be run,
whatever the preamble says.

Known exhaustive matches in this workspace, as of 2026-09-06: over
`PlannerError`, only `goal/mod.rs:355` — everything else uses a wildcard or
`matches!`. Over `Goal`, three: `method/have.rs`'s `holds()`,
`scripting_lua/globals/goal/value.rs`, and `server/game/control.rs`.

## What actually caught them

Measurement against the live game, every time. Not review, not more tests,
not more careful reading. The ladder this project already keeps — offline
plan in seconds, headless run in minutes, 1x client run for a number — is
what makes that affordable, and the middle rung is where all four surfaced.

## The rules that follow

- **Build test worlds from prototype data or a real dump**, or assert the
  fixture against one. `workspace/scripts/map.json` is a real world; a
  hand-typed position is a guess.
- **A new instrument's first reading is evidence about the instrument.**
  Validate a counter, a margin or a classifier against something that
  already knows the answer — the force's own production statistics, a
  second query, a hand count — before believing what it says about the
  system.
- **A constant with no measurement behind it is a defect waiting.** If a
  number in this repo has no note saying who measured it and when, treat
  quoting it as a claim you are making, not a fact you are citing.
- **Prefer measuring to quoting this repository's own prose.** Three
  documented constants were wrong in one day (`R + 1.1`, the walk speed,
  the MinerLine block dimensions).
- **When a task writes both the code and its fixture, say so in the report**
  and name what the fixture assumes. That sentence is cheap and is the only
  warning a later reader gets.

## The same asymmetry, one level up: what a loaded run proves

A related rule settled between the two sessions the same night, because both
had been applying a broader one than the evidence supports.

**A validity check does not need a quiet machine; a measurement does.**
Starvation corrupts a *number*. It cannot make a block stand in the wrong
place: whether entities are where the plan said is equally true at 2 tps and
at 300. So a run whose question is "did this work" may share the box, and
only a run whose number someone will quote needs a clear floor.

**But the licence is one-directional.** A starved server times actions out,
trips executor deadlines and leaves entities unplaced, so a validity run
that comes back *short* is ambiguous between "the code chose wrong" and "the
box was loaded".

- **A clean pass at any tick rate is trustworthy.**
- **A failure at a bad tick rate is not** — the delivered rate is then part
  of the diagnosis, not a footnote.

**Two fragile runs cannot share a floor; they have to queue.** A validity
check can share with anything, including another validity check. But
starvation makes a 96-second plan read as 137 — the same case measured 99.5 s
and 137.7 s in one night, 38% apart, from load alone — so two *timing*
measurements are noise to each other and must be sequenced, whoever asked
first.

**A timing is a claim about a *binary*, not about a program — name the
profile as well as the commit.** Two sessions compared the same blueprint,
same roster, same dump and got 96 s against 8.57 s, and spent a round of
messages on plausible explanations (CLI path versus in-process) before
anyone measured it. The answer was **debug versus release**, measured on one
machine minutes later:

| | trivial goal | green, 4 bots |
|---|---|---|
| debug | 2.70 s | **31.04 s** |
| release | 0.77 s | **4.55 s** |

~7× on this workload, and the 865 MB dump load is 2.7 s debug against 0.75 s
release either way. Both numbers were right about their own binary; neither
message said which. Quote release for anything anyone will act on, because
whether a per-plan cost is a problem at all can change with the profile.

**And then the ratio itself failed to transfer, one level along.** I offered
that 7× to convert the other session's block figure; they built release and
measured instead, and got **~4.4×** (48 s → 10–13 s, against my predicted
4–5 s). A ratio measured on one goal does not transfer to another: **a
conversion factor is a claim about a binary *and* a workload.** The
difference mattered — 4–5 s would have made the cost a non-issue, whereas
~10 s per expansion, paid on every replan, is 40–70 s for a run that
replans four to seven times, and it is paid **precisely when things are
going wrong**, since that is when replans happen.

The general form, and the third time tonight this shape appeared: **do not
extrapolate a measurement you could take.** Taking it cost them minutes.

**A baseline is only a baseline against a stated commit.** Both sessions
nearly made the mirror-image mistake within an hour: one about to compare a
pre-change planner against a post-change one and attribute the difference to
its own work, the other about to do the same in reverse. Every figure in a
record that does not name the commit it was taken on is weaker than it
looks.

**And publish what got worse in the same table as what got better.** From
the `second` session, whose sentence this is: *"a result that only lists
what got better is the same shape as a test that only checks the happy
path."* Tonight's furnace change improved red and green and cost two to
three percent on three deeper goals; a table showing only the first half
would have been accurate and misleading.

So: record the delivered tick rate in every run's note (`just analyse`
prints it and flags below 80% of nominal), share the box freely for checks
you expect to pass, and re-run on a quiet floor before believing any
failure that arrived on a loaded one.

## A falsification that never applied, and reported green over nothing

A seventh way onto this list, found by the other session's agent and one neither
of us would have predicted: **a substitution matched zero times because
`rustfmt` had wrapped the call across four lines.** The edit did not apply, the
test ran, and it would have reported green over an unmodified file.

That is worse than the fourth cause ("the break was not a break"), because there
the edit *did* apply and merely failed to change behaviour. Here nothing
happened at all, and the only difference visible from outside is a test that
stays green — which reads as *"the test is vacuous"*, sending you to rewrite a
test that was fine.

**The rule: assert the substitution COUNT, not that the edit succeeded.**
`s.replace(...)` returns a string whether or not it matched; `assert
s.count(old) == 1` before replacing is what turns a silent miss into a stop.

**I am exposed to this and it is worth saying so.** Every falsification I ran
tonight was a string substitution, and in the falsification edits specifically I
asserted nothing:

```python
s.replace('<= POLE_WIRE_REACH_TILES', '<= 0.5')                  # no assert
s.replace('...pole_would_supply(POLE, p, &area)', '...false')     # no assert
s.replace('crate::enclosure::check(...)', '...Clear')             # no assert
```

All three went red, so all three applied — but **the safety came from the
outcome, not from the method**. Had any matched zero times I would have seen a
green test and concluded the assertion was vacuous, which is precisely the wrong
repair. The guarded form was in my *editing* code and absent from my *breaking*
code, which is the half where a silent miss actually costs something.

Same shape as the load guard that could not run and shrugged: the check existed
and did not check.

## One level out: instrumentation is code, and mine failed open

Every entry above is a check that was **weak**. This one is a check that was
**absent and reported as present**, which is a different and worse shape — and
it was in my own measuring harness rather than in the code under test.

Measuring what a bot costs in tick rate needs a quiet machine: this repo already
records that a cargo build in a worktree starved a server to 2-10 tps and froze
a walking bot. So the runs were gated on 1-minute load:

```bash
if [ "$(echo "$L < 4" | bc -l)" = "1" ]; then break; fi
```

**`bc` is not installed here.** Every comparison therefore evaluated false, the
guard waited its whole window, and then measured anyway — on a box that had
climbed from load 24 to 37 while it waited. The same missing `bc` meant no wall
times were computed, so the run produced no usable number in either direction.
Forty `command not found` lines went into a log nobody was reading.

**A guard that cannot run must refuse, not shrug.** The failure is exactly
`only_ghosts = true` validating nothing, the `Using mods directory` line that
printed on no run at all, and `0 uncovered` passing because the loop never ran —
except that those are in the product and this was in the instrument.

The rule that follows is narrow and worth stating on its own:

- **Instrumentation is code and gets the same bar.** A harness, a probe, a
  timing gate, a load check — falsify it before trusting a number it produced.
  Spending a day rigorously falsifying the *code's* checks while never
  falsifying the harness is precisely how this happened.
- **Self-test a comparator in BOTH directions before using it.** A comparator
  stuck at false and one stuck at true are both broken, and asserting one
  direction catches half of them. The fix here asserts `lt(1,2)` is true *and*
  `lt(2,1)` is false, and refuses to measure if either fails.
- **A refusal must say what it could not do.** The rewrite prints "load still N
  after 600s -- a tick-rate number taken here would measure the build farm, not
  the bots" instead of a figure. Absence of a number is a result; a number taken
  under unknown conditions is not.

## The rarer, opposite case: an independent oracle that disagrees

Most of the night's defects were a check agreeing with its subject. One was
the reverse and is worth naming, because the instinct to dismiss it is
strong.

The siting session wrote a Python oracle, independent of the Rust, that
found **359 legal anchors** for a block the Rust refused to site. The
disagreement was explained away by a known missing constraint in the oracle
— and then that constraint was removed from the Rust by the ore fix, so the
explanation evaporated and **the disagreement became live again**. Three
candidates were named and none asserted.

**An independent oracle disagreeing with the implementation is a finding,
not noise.** The value of an oracle is exactly that it was not written to
agree; when it stops disagreeing for a reason you have since deleted, the
disagreement is new evidence rather than an old annoyance.

**And a refusal that moves is a diagnosis; a refusal that vanishes is only a
hope.** The ore fix did not clear this refusal, it changed it from "no route
for the belts" to "no anchor puts every drill on ore" — which is more
informative than a pass would have been.

## The worst shape a record can have

From the `second` session, 2026-09-06, on discovering that a run's
`git.commit` described the checkout while the mod came from a worktree:

> Provenance was accurate about the thing it measures and silent about the
> thing that mattered. That is the worst shape a record can have: **not
> blank, but confidently about the wrong object.**

Two of the night's instances have exactly this shape, and neither is a
missing field:

- **`git.commit` names the checkout, not the bytes the game loaded.** A run
  recorded a clean commit whose mod contains the mechanism under test; the
  mod that loaded was three weeks older and contained none of it. Two
  sessions reasoned from the result and one sized a constant on it.
- **`obs.done` reports "no further progress possible" in the voice of "the
  plan is complete."** A milestone was recorded satisfied at 89 of 179
  entities standing.

A blank field prompts a question. A confident field about the wrong object
answers one that was never asked, and the reader cannot tell. When adding
any field to a record, state precisely which object it describes — and if
that object is not the one a reader will assume, either rename it or record
the one they meant.

## A module with no caller is a hypothesis

The sharpest form of everything above, from the `second` session on the
morning its belt router finally ran:

> A module with no caller is a hypothesis. **No unit test could have found
> that** — every fixture I wrote handed the module a world where the bill was
> satisfiable, because I wrote both. A module with no caller cannot discover
> that its bill is unbuildable, and my four clean reviews are the proof.

The concrete defect: `connect_steps` hard-coded the **electric** `inserter`,
which is not enabled at t=0 on seed 31337. Its first real caller would have
refused on the **materials bill**, before geometry was ever reached — after
four clean task reviews, a full suite, and a live direction test that proved
its geometry. Nothing in the module's own world was wrong; the module had
never been asked a question from outside it.

Two rules follow, and they are not the same rule:

- **Fixtures answer the questions their author thought of.** A module tested
  only by its author's fixtures is tested against that author's model of the
  world, which is the same model that produced the code.
- **A caller asks questions the module's author did not.** The bill, the tech
  tree at t=0, the entity that is not a valid pickup, the chest that cannot
  host three runs — every one of those was forced by a refusal from outside,
  not designed.

So: **wire a caller early, even a poor one.** A primitive that has never been
invoked from real code is not "done and waiting"; it is untested in the only
way that counts.

## Confirm the test RAN before you confirm it can fail

The step before "make the test fail on purpose", and the one that nearly ate a
whole fix on 2026-09-06.

I added `bridge_resolves_tests` to `instance_setup.rs`, ran
`cargo test -p factorio-bot-core --lib instance_setup`, and read:

```
test result: ok. 36 passed; 0 failed; 2 ignored; 0 measured; 530 filtered out
```

**None of the 36 were mine.** The filter matched the module path, the code was
in the file, the guard it tested had compiled — the call site was in the
binary — and the new test module was simply not in the test list. Had I read
the green and moved on, I would have shipped a guard proven by nothing, and
proven it *with a passing suite as the evidence*.

What caught it was counting by name rather than reading the verdict:

```
grep -c "bridge_resolves_tests" <output>   # 0
```

**A suite that never compiled your test is greener than one that did.** Every
other failure in this note is a test that ran and agreed with the wrong thing;
this is the cheaper and more embarrassing one, where nothing ran at all and the
summary line looked identical. `0 failed` counts what executed, and a test that
does not exist in the binary cannot fail.

So the order is:

1. run the new test and **find it by name in the output**, with a count, not
   by eye over a scrolling list;
2. *then* break the code on purpose and confirm it fails;
3. *then* restore and confirm it passes again.

Step 1 is not implied by step 2. A falsification run also comes back green when
the test is absent — the neutered code and the missing test produce the same
`ok`, and I would have read that as "the falsification failed to reproduce"
rather than "there is no test here."

Related: **assert the substitution matched**, from the same night. A
falsification whose `str.replace` matched zero occurrences also passes, for the
same reason and with the same reassuring output. Both are the same defect
wearing different clothes: *an empty operation reports success.*

### And I hit the sibling trap in the same hour

Having just written the section above, I ran the workspace suite as

```
cargo test --workspace 2>&1 | tail -30
```

in the background, and read `[exited with code 0]` as a green suite. **It was
`tail`'s exit code**, the trap CLAUDE.md already documents — and worse, the
captured file held only the last 30 lines, so the `grep -c` I had just
prescribed for counting my own tests by name reported `0` because the unit-test
section had been thrown away, not because the tests were missing.

So the check I invented one hour earlier produced a *false alarm* on the very
next run, for a third reason neither the check nor the trap it was written for
covers: **the evidence was truncated before it was searched.**

The fix is the same shape every time and worth stating as a rule of its own:
**redirect to a file, capture `$?` from the command itself, and search the
whole file.** Not a pipe, not a tail, not a summary line. Three defects in one
night — a test that did not compile, a substitution that matched nothing, a
pipeline reporting the wrong process — all reported success, and all three were
caught by counting something in the full output rather than by reading a
verdict.

## A falsification that comes back GREEN is not a passed check

The strongest result of 2026-09-06, because it happened **three times, in three
different hands, on three unrelated pieces of code**, and in each case the
green reading was the defect.

**1. The test that never compiled.** `cargo test --lib instance_setup` reported
`36 passed` and none of the 36 were the new module. Caught by `grep -c` on the
module name.

**2. The test that agreed with broken arithmetic.** `mining_drill_radius`:
`tiles()` was deliberately broken from `ceil` to `floor`, and
`only_the_electric_drill_reaches_beyond_its_own_tiles` **stayed green**. It
asserted only *relations* — that the electric drill's worked tiles exceed its
footprint, and exceed it by two — and `floor` happens to preserve both on these
numbers (4 > 2, and 4 == 2 + 2). A relation between two wrong numbers is not
evidence. Fixed by asserting the absolute counts: 2, 2, 3, 5.

**3. The test that asserted geometry instead of the rule.** In
`power-as-capacity-over-time`, two of nine falsifications came back green:
weakening the pole-siting rule to "reaches the first engine" changed nothing,
because the ring search starts at the joint and lands on a both-covering tile
*by geometry* whatever the rule says. The test asserted a property of the
returned tile, not of the rule meant to guarantee it. Fixed by extracting one
`pole_site` implementation and rewriting the fixture to block every
both-reaching tile while leaving a first-only tile free.

### The three ways a falsification lies, and they are distinct

| the break was… | and the test passed because… | caught by |
|---|---|---|
| never compiled in | the test does not exist in the binary | counting tests by name |
| compiled, ran, agreed | the assertion is weaker than the claim | asserting absolute values |
| compiled, ran, irrelevant | the property holds for another reason | making the fixture hostile |

**The third is the dangerous one**, because the test is real, runs, and is
about the right subject — it simply cannot distinguish the rule from the
accident. A fixture that satisfies the property incidentally can never falsify
the rule, no matter how the rule is broken. The fix is always to make the
fixture *hostile*: arrange the world so that only the rule can produce the
answer, and assert the fixture's own preconditions so it fails loudly rather
than vacuously when someone later changes it.

### A FOURTH cause, and it points the opposite way: the break was not a break

Added the same day by the peer session, who nearly discarded a good test over
it. All three above are "the test is vacuous". This one is not.

They rewrote **both** occupancy predicates to prove a siting test was
load-bearing. It stayed green — the exact symptom. But the defect under test
was the **disagreement between the two predicates**, so changing both restored
their agreement and *hid the bug the test was written to catch*. Flipping only
one made it fail immediately, with the real error.

So the symptom is shared and the diagnosis is not:

| green falsification means… | and the fix is… |
|---|---|
| the test never ran | count tests by name |
| the assertion is weaker than the claim | assert absolute values |
| the property holds for another reason | make the fixture hostile |
| **your break was not a break** | **check the break moved something** |

The fourth is the one that punishes a *thorough* falsifier. Breaking more of
the code is the instinct, and here breaking both halves of a pair is precisely
what restores the invariant. A defect that lives in the *relationship* between
two things cannot be exposed by changing both.

### The rule

**A green falsification means either the test is vacuous or your break was not
a break — and you cannot tell which without confirming the break changed
behaviour.** So falsify by breaking exactly one thing, and when nothing goes
red, first ask whether the edit could have been self-cancelling before
concluding anything about the test.

**When a falsification comes back green, do not conclude the code is
load-bearing. Conclude the test is not.** Then find out which of the three it
is before writing another line. Every one of these was found by an author
falsifying their own work, which is the only reason they are in this note
rather than in a run record six weeks from now.
