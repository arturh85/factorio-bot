# The second concentrator is the belt run

Derived entirely from `run-1788920460-08860`'s own plan record. No build, no
run, no code change — the artifact had the answer.

## The measurement

Fleet utilisation **25.1%**, `steps/bot {1: 520, 2: 136, 3: 119, 4: 106}`. The
`yields_at` fix (`76540e32`) moved `producing:iron-plate:261` from 27.0% to
44.2% and moved **this** goal not at all, so a second concentrator had to exist
and had to be something other than `Chop`/`Stockpile`.

Broken down by verb, the plan is not lopsided in the way the step count suggests
— it is lopsided in *what kind of work* each bot does:

| verb | bot 1 | bot 2 | bot 3 | bot 4 |
|---|---:|---:|---:|---:|
| place | **200** | 9 | 5 | 3 |
| take | 91 | 0 | 0 | 6 |
| craft | 76 | 12 | 4 | 5 |
| fuel | 62 | 7 | 4 | 4 |
| insert | 53 | 11 | 11 | 14 |
| mine | 23 | 50 | 50 | 42 |
| stock | 0 | 39 | 40 | 30 |

**Gathering is divided four ways. Construction is not divided at all** — bot 1
does 200 of the plan's 217 placements, 91% of them.

By *planned ticks* it looks tamer (28,262 / 24,326 / 15,410 / 14,560) because
mining is slow and placing is fast. That is exactly the trap this repo has
recorded before: **a verb histogram cannot see waiting.** The other three bots
finish their mining and then have nothing to do while bot 1 builds.

## And 156 of those 200 are one belt run

```
bot 1: 200 placements -> transport-belt 156, burner-inserter 11,
                         stone-furnace 9, burner-mining-drill 5,
                         small-electric-pole 5, iron-chest 3
```

**A belt run is emitted as one sequence of `Place` steps owned by one bot.**
156 belts, laid end to end, by one pair of hands, while three bots stand idle.

## Why this is the fifteenth instance of the dominant defect class

The capability to split this exists **twice over**, and the belt run uses
neither:

- `method::blueprint` bands entities across bots — its own first line is *"a
  blueprint, an anchor, and one band per bot"*, and a band is a region precisely
  so that *"a bot never crosses another's band"*. `FurnaceLine`'s 179 entities
  split 45/45/45/44.
- `method::assemble` has `deal_bundles` — *"Deal a cell's bundles across
  `builders`, heaviest first, each to whoever is lightest at that moment"* —
  written for exactly this symptom: `run-1788604520-39283` left bot 1 alone for
  the last 47 actions and ~35,000 ticks while the others had finished.
  `grep` gives it **exactly one call site**, inside `assemble.rs` itself.

`method::connect`'s belt run calls neither.

## Why a belt run should be easier to split than a cell

`BuildAssemblyCell::converges` is `true` for a stated and correct reason: *"Ten
buildings, two recipes and two chest charges have to meet in one pair of hands:
three parts of a cell arriving on three bots is a cell nobody can assemble."*

**A belt run has no such requirement.** Each belt is placed from the placer's
own inventory; nothing downstream needs them to have arrived in one pair of
hands. And `deal_bundles`' own safety argument transfers directly: *"a placed
chest, inserter or machine is a **map fact**. Everything that comes after it
names a position and no bot."* A placed belt is a map fact in exactly that
sense.

The shape a fix probably wants is `blueprint.rs`'s, not `assemble.rs`'s: a belt
run is a **line**, which is the easiest thing there is to cut into contiguous
bands, and contiguity is what keeps two bots from walking through each other.

## What is NOT claimed here

That splitting it will make the run faster. Three bots that currently idle would
start walking to their segments, and this project has already measured that
contention on the ground is real — eight bots produced a *shorter plan* and a
*longer run* (1.42x against 1.18x for four). The claim is only that **156
sequential placements by one bot while three stand idle is the concentrator**,
and that the two mechanisms which would address it were both written for other
callers.

Unowned. `method/connect.rs` is currently held by the perimeter-refusal work.
