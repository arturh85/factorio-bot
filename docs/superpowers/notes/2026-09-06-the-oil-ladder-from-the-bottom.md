# The oil ladder, measured from the bottom — three rungs down, one refusal that is a gap

2026-09-06, master at `1b28f6fa`, release binary, offline against
`workspace/scripts/map-31337-explored.json`, four bots.

**Headline: the oil research plans on the real map, and the wall that stopped
this work all morning is gone.** What remains is one refusal that turns out to
be inconsistent with the rest of the planner, and one goal kind that has no
method at all.

Every number below is from `factorio-bot plan --world <dump> --goal <g>
--bots 1,2,3,4`, with the command's own exit code captured rather than a
pipeline's.

## What plans now

| goal | exit | actions | makespan | utilisation |
|---|---|---|---|---|
| `researched:oil-gathering` | 0 | 1,587 | 311,773 (1:26:36) | 74.4% |
| `researched:oil-processing` | 0 | 2,012 | 308,577 (1:25:42) | 75.3% |

This is the first time either has planned end to end here. The binding
constraint this morning was `a power plant needs water, and the plan can see
none within 128 tiles`; with the plant growing to `BOILERS_PER_PUMP ×
MAX_ENGINES_PER_BOILER` engines the ceiling moved to the water, and the water
was never the problem on this map (48.1 tiles from the origin).

**An oddity recorded rather than explained**: `oil-gathering` has *fewer*
actions than `oil-processing` (1,587 vs 2,012) and a *longer* makespan
(1:26:36 vs 1:25:42). A smaller plan taking longer is a scheduling result, not
a contradiction — but it is the kind of thing that is worth a look before
either number is quoted as a target. Not chased here.

Neither is a good time. Both are ~86 minutes of game time, against a green
milestone at 12:49. That is the honest state: **plannable is not yet good**,
and the first number to move is probably not the one that looks worst.

## The refusal that is a gap, not a fact

```
gathered:crude-oil   REFUSES: "extracting from crude-oil takes a pumpjack,
                     whose recipe needs oil-gathering researched first"
```

The sentence is true. It is also a prerequisite the planner can satisfy —
`researched:oil-gathering` plans, on this map, from this state, in 1,587
actions.

The control says this is inconsistent rather than deliberate-looking:

| goal | needs researched | exit | actions |
|---|---|---|---|
| `have:assembling-machine-1:1` | `automation` | 0 | 223 |
| `have:electric-mining-drill:1` | `electronics`, `automation` | 0 | 316 |

**`have:` already bills its unlock research, silently and correctly.**
`gathered:` states the same class of fact as a refusal. Dispatched as
`gathering-bills-its-own-unlock`, with the instruction to *falsify this
reading first* — if the refusal is a recorded policy choice, it stands, and
the reason gets written down instead.

Two constraints on that fix, both with scars behind them: the pumpjack's
unlock must be **derived from the prototypes, not hard-coded** (a hard-coded
unlock is a mod-compatibility defect, and `flow_graph.rs` already violates the
same rule with a test asserting its own constants), and the refusal that
survives must distinguish *"not yet researched, and here is the plan"* from
*"nothing in this game unlocks this"*.

## The rung with no method at all

```
have:petroleum-gas:100   REFUSES: no method can satisfy goal: have 25
                         petroleum-gas (a share sized for bot 1)
```

This is not a gap in a method; it is the **fluid** gap. A fluid is not an item
you carry, so `Have` is arguably the wrong goal kind for it — the share-sizing
in the message ("a share sized for bot 1") is itself the tell that the planner
is treating petroleum gas as something four bots divide between them. It is
not. Fluid storage, and what goal kind expresses "there is petroleum in that
tank", is the open design question, and `crates/planner/src/substance.rs`
(`Substance{Item,Fluid}`) is where the distinction already exists.

The `recipe_for` product index answering `None` for every fluid sits under
this same rung.

## Where this leaves the ladder

1. ~~power plant refuses for want of water~~ — **down**, the ceiling was the
   layout and the layout grew.
2. `gathered:crude-oil` refuses on a satisfiable prerequisite — **dispatched**.
3. pumpjack siting on a real oil field — untested, blocked behind rung 2.
4. fluids as a substance the planner can hold, move and count — **open, and
   the largest**. Not a bug list; a design question.
