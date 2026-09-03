# Per-bot share sizing — Design

**Status:** **IMPLEMENTED.** Corrected 2026-09-03 — this line read *"design only, not implemented"*. `even_shares` (`crates/planner/src/method/have.rs:2210`), `distinct_bots` (`:2151`), the `BotsNotInterchangeable` guard deleted, all seven tests present, plus a `seats` cap the spec predates. **§4's residual-hazard analysis is inverted by the same work:** it argues a share's chain has no owner and cites a test asserting `owner_of(chain) == None`; that test now asserts `Some(BotId(1))` (`crates/planner/src/method/mod.rs:1991-1995`), and a live four-bot run crashed twice on the hazard §4 called safe.
**Date:** 2026-09-01
**Baseline:** `1506bf45` (`fix(planner,record): scope interchangeable-bots guard, …`)

Every line number below is against that commit. `crates/planner/src/method/have.rs`
was **not** touched by it, so its numbers are also those of the commit before.

---

## Summary of the conclusion

The premise this task was handed — "`SplitAcrossBots` sizes each bot's share
against one bot's inventory" — is **half true, and the false half matters**.
Since `03e89f93` the method already states each bot's target as *that bot's own*
holding plus its share (`have.rs:1033-1037`), so the shares' arithmetic is
already per-bot and already sums to the roster shortfall. What is still
holding-blind is **who participates and who carries the remainder**, and what is
still wrong is that the per-bot read uses the **raw inventory** while the
roster-wide read it is subtracted from uses the **reservation-adjusted** one.

I built a copy of the crate outside the repo, disabled the guard, and measured.
Findings that should change how this is planned:

* **Asymmetric rosters already plan and schedule correctly today** once the
  guard is bypassed — including a 62-action red-science plan on a freeplay
  roster. The predicted "`expand` succeeds and `schedule` fails forty actions
  later" (`docs/superpowers/plans/2026-08-30-planner-hardening.md`, Task 5) did
  not reproduce in any scenario I could construct.
* **The one measurable defect is not about asymmetry at all.** Two top-level
  splits of the same item over four *identical* bots mine **24** iron ore for a
  bill of 16. That is the raw-holding read, and it is 50% over-production on a
  green suite.
* **The obvious "holding-aware" rule is worse.** Levelling final holdings (bot
  holding 8 produces nothing, the other three produce 4 each) measured **2427**
  ticks against equal work's **2156** on the same goal. Holdings must change
  *who* works, not *how much* each worker does.
* Under the proposed rule **all 255 planner lib tests and all 22 integration
  tests pass unchanged**. Only the guard's own five tests need to go.

---

## 1. What the current sizing actually does

### The code

```rust
// crates/planner/src/method/have.rs:1001-1011
fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
    let Goal::Have { item, count, whose } = goal else { return false; };
    if !matches!(whose, Holder::Anyone) { return false; }
    // An empty roster has no share to give out, and `expand` would divide
    // by the chain count.
    !self.bots.is_empty() && shortfall(state, item, *count, whose) > 0
}
```

```rust
// crates/planner/src/method/have.rs:1019-1039
let need = shortfall(&ctx.state, item, *count, whose);
let chains = (self.bots.len() as u32).min(need);
let base = need / chains;
let remainder = need % chains;

let mut steps = Vec::new();
for (index, bot) in self.bots.iter().take(chains as usize).enumerate() {
    let share = base + if (index as u32) < remainder { 1 } else { 0 };
    // A `Have` goal states a holding, not a delivery, so a share of one
    // handed to a bot already holding five is a goal that is already
    // met — and the share evaporates. Ask for what the bot has *plus*
    // its share, so the shortfall the other methods see is the share.
    // Shares sum to the shortfall, so the roster ends up with at least
    // `count` between them however the holdings started.
    let held = ctx.state.inventory_count(*bot, item);
    steps.push(Step::Subgoal(Goal::Have {
        item: item.clone(),
        count: held.saturating_add(share),
        whose: Holder::Share(*bot),
    }));
}
```

### Whose inventory each quantity reads

`applicable` guarantees `whose == Holder::Anyone` (`have.rs:1005-1007`), so:

* **`need`** — `shortfall(state, item, count, Holder::Anyone)`
  (`have.rs:71-73`) is `count.saturating_sub(state.available(&Holder::Anyone, item))`,
  and that arm (`state.rs:352-360`) is

  ```rust
  self.total_count(item).saturating_sub(promised)
  ```

  where `promised` is **every per-bot reservation plus the `Anyone`
  reservations**. So `need` is roster-scoped *and* reservation-adjusted.

* **`held`** — `inventory_count(*bot, item)` (`state.rs:307-314`) is the raw
  `BTreeMap` lookup with **no reservation subtraction at all**.

* **the subgoal's own shortfall**, computed one frame later by whichever method
  claims `Have { count: held + share, whose: Share(bot) }`, reads
  `available(&Holder::Share(bot), item)` (`state.rs:361-363`) —
  `inventory_count − reserved_for`.

So the shortfall each chain is actually asked to produce is

```
(held_i + share_i) ⊖ (held_i − reserved_i)  =  share_i + reserved_i
```

not `share_i`. The comment at `have.rs:1031-1032` ("Shares sum to the
shortfall") is true only while `reserved_i == 0`.

### Where interchangeability enters

Exactly three places, and only the third is what the guard names:

1. **`have.rs:1020` + `have.rs:1025`** — `chains = min(len, need)` and
   `self.bots.iter().take(chains)`. Participants are chosen by **position in
   the caller's slice**. That is only defensible if any bot is as good as any
   other, and it also makes the plan depend on the *order* a Lua caller listed
   its roster in, not just on the set.
2. **`have.rs:1026`** — `share = base + [index < remainder]`. Work per
   participant is holding-blind, and the remainder goes to the lowest-indexed
   bots.
3. **`have.rs:1033`** — `inventory_count(*bot, item)`. This is the crate's
   **only** cross-bot inventory read, as the narrowed guard's own doc comment
   now says (`mod.rs:239-252`): "Nothing else in this crate compares two bots'
   inventories against each other."

And the assumption is stated in prose in three more:
`mod.rs:38-42` (`ExpansionCtx`), `goal.rs:33-36` (`Holder::Share`),
`recover.rs:301-310` (executor tier 2).

### What the guard is today

```rust
// crates/planner/src/method/mod.rs:263-276
for item in &scattered_items(goals) {
    let first_count = state.inventory_count(first, item);
    for &other in rest {
        let other_count = state.inventory_count(other, item);
        if first_count != other_count {
            return Err(PlannerError::BotsNotInterchangeable { a: first, b: other, item: item.clone() });
        }
    }
}
```

`scattered_items` (`mod.rs:293-312`) collects the items of every top-level
`Goal::Have { whose: Holder::Anyone }`, flattened through `Goal::All`. So the
guard now protects **exactly line 1033** and nothing else.

---

## 2. What per-bot sizing should compute

### The measurements that pick the rule

All figures from a copy of `crates/planner` built outside the repo with the
guard bypassed. Roster of four on `fixture_world()`, positions `x = 0,10,20,30`,
two stone furnaces each unless stated. Old = code as at `1506bf45`;
New = the rule below.

| scenario | old | new |
| --- | --- | --- |
| `All[have(iron-ore,8), have(iron-ore,8)]`, identical bots | 8 actions, **24 ore**, 1191 ticks | 8 actions, **16 ore**, 951 ticks |
| same, bot 1 holds 6 ore | 4 actions, 6 ore, 711 | 4 actions, **4 ore**, 591 |
| `have(iron-ore,10)`, bot 1 holds 3 | mines `2,2,2,1` — bot 1 gets **2** | mines `1,2,2,2` — bot 1 gets **1** |
| `have(iron-ore,10)`, bot 1 holds 8 | 2 mines of 1, 431 | identical |
| `have(iron-plate,20)`, bot 1 holds 8 | 24 actions, 12 plates, 2156 | identical |
| *levelling* variant of that goal (`0/4/4/4`) | — | **2427** — worse |
| `have(automation-science-pack,10)`, freeplay-asymmetric | 62 actions, 7092 | identical |
| `have(iron-gear-wheel,20)`, freeplay-asymmetric | 34 actions, 5203 | identical |
| every symmetric fixture in the suite | — | byte-identical |

Two conclusions. **Levelling final holdings is the wrong rule**: it concentrates
the same total work on fewer bots and lengthens the makespan (2427 vs 2156).
**Equal work per participant is right**; what holdings should decide is *which*
bots are participants and *who carries the remainder*.

### The rule

Let the roster be the distinct bots of `self.bots`. For each bot `b`:

```
spare(b) = state.available(&Holder::Share(b), item)      // state.rs:361-363
```

and, unchanged,

```
need = count.saturating_sub(state.available(&Holder::Anyone, item))
```

`applicable` still guarantees `need > 0` and a non-empty roster.

1. **Order.** Sort the distinct bots ascending by the pair `(spare(b), b)`.
   Poorest first; `BotId` breaks ties.
2. **Participants.** `k = min(distinct_count, need)`; take the first `k` of that
   order. `k ≥ 1` because `need ≥ 1` and the roster is non-empty, so the
   division below cannot divide by zero.
3. **Work.** `base = need / k`, `rem = need % k`. The participant at position
   `j` (0-based, in the sorted order) gets

   ```
   w_j = base + if j < rem { 1 } else { 0 }
   ```

   `Σ w_j = k·base + rem = need`, exactly. The remainder goes to the **poorest**
   participants — the direct expression of "a bot holding 8 of the 10 needed is
   asked for less than one holding none", and the only place holdings change
   the amount of work.
4. **Target.** Emit, for each participant `b` with work `w`:

   ```rust
   Goal::Have { item, count: spare(b).saturating_add(w), whose: Holder::Share(b) }
   ```

   Because the subgoal's own shortfall is `count ⊖ spare(b)`, the shortfall each
   chain sees is now **exactly `w`**, whatever the reservations — which is what
   `have.rs:1031-1032` already claims and does not currently deliver.
5. **Emission order.** Emit the subgoals in ascending `BotId`, *not* in the
   sorted order. Emission order fixes `ActionId` allocation and therefore the
   scheduler's `(end, ActionId, BotId)` tie-break, and ascending `BotId` is what
   the current code emits, so a symmetric roster keeps a byte-identical plan.

### Worked example (the prompt's case)

`Have { iron-plate, 10, Anyone }`, four bots, bot 1 holds 8, bots 2-4 hold none,
no reservations.

* `need = 10 − 8 = 2`; `k = min(4, 2) = 2`; `base = 1`, `rem = 0`.
* Order by `(spare, bot)`: `(0,2), (0,3), (0,4), (8,1)`. Participants: bots 2
  and 3.
* Work: 1 each. Targets: `Have{iron-plate, 1, Share(2)}`,
  `Have{iron-plate, 1, Share(3)}`. Bot 1 is asked for nothing.
* Roster ends at `8 + 1 + 1 = 10`. Today it would be bots 1 and 2 with targets
  9 and 1, and bot 1 would smelt one plate it did not need to.

Second example, where the remainder is the whole story —
`Have { iron-ore, 10, Anyone }`, bot 1 holds 3:

* `need = 7`, `k = 4`, `base = 1`, `rem = 3`. Order: `(0,2),(0,3),(0,4),(3,1)`.
* Work: bots 2, 3, 4 get 2 each; bot 1 gets 1. Sum 7. Measured: mines
  `1,2,2,2` (new) against `2,2,2,1` (old).

### When the output differs from today

Only in these cases; otherwise byte-identical.

* Any participant has a non-zero reservation on the item (`spare ≠ held`).
* `self.bots` contains a duplicate (see below).
* The item's `spare` differs across bots **and** (`need < n`, changing who
  participates, or `need % k ≠ 0`, changing who carries the remainder).

### Duplicates in `self.bots`

`registry_for` copies the caller's slice verbatim, so a caller can hand it the
same `BotId` twice. Today that emits two `Share(b)` subgoals for one bot; the
second is sized after the first reserved, so it over-asks. The rule above must
**dedupe before computing `k`**, or a `BTreeMap` keyed on `BotId` would silently
collapse two shares into one and under-produce. Deduping is a fix, not a
regression, but it is a behaviour change and gets its own step and test.

---

## 3. When the roster meets the goal but no single bot does

Measured, four bots holding 3 iron plates each:

| goal | actions planned |
| --- | --- |
| `have(iron-plate, 10)` | **0** |
| `have(iron-plate, 12)` | **0** |
| `have(iron-plate, 13)` | 6 (one plate produced) |

`AlreadySatisfied` claims all of them at or below 12, because
`shortfall(.., Holder::Anyone) == 0` (`have.rs:164`) and `Holder::Anyone` is
*defined* as "satisfied by the sum across every bot" (`goal.rs:9-12`).

**This should not change, and per-bot sizing does not change it.** Three
reasons:

1. It is the goal's stated meaning. Making `AlreadySatisfied` per-bot would
   redefine `Holder::Anyone`, not fix the sizing — a caller who needs the items
   in one pair of hands already has `Holder::Bot` and `Holder::Share` to say so.
2. The opposite change has a recorded scar. `needs_producing`'s doc comment
   (`have.rs:44-55`) describes what happens when a roster-wide question is
   answered for something that reads one inventory: four bots holding one stone
   furnace each satisfy `Have{stone-furnace, 1, Anyone}` four times over and the
   second `Place` fails. The answer there was to stop asking `Anyone` at
   *subgoal* sites, which `SplitAcrossBots::claims` (`have.rs:997-999`) and the
   `Holder::Share` propagation already enforce. It was not to redefine `Anyone`.
3. `AlreadySatisfied` remains the only method returning zero steps for a `Have`
   goal, and after this change it is also the only place a top-level `Have`
   consults the roster sum. That is a property worth keeping in one place.

**But it is a live trap and must be documented.** Measured on a freeplay
roster — bot 1 holding the starting inventory, bots 2-4 holding nothing —
`have(iron-plate, 5)`, which is exactly `scripts/goal_smoke.lua`'s goal, plans
**zero actions**, because bot 1's eight starting plates satisfy the roster sum
on their own. The run then does nothing and looks stuck. That is correct by the
type's definition and surprising in practice, so:

* add a test that pins it (listed below), rather than leaving it accidental;
* consider a narration line at the `goal.plan` boundary when an expansion comes
  back with an empty network, naming the holder — out of scope for this spec,
  flagged for the supervisor work.

---

## 4. Does the guard survive?

**No. Delete it.**

The guard's justification, in its own current words (`mod.rs:239-252`), is that
`SplitAcrossBots` reads a per-bot count at `have.rs:1033` and assumes any bot
would do. After this change that line reads each bot's own `spare` and uses the
differences between them — it no longer assumes agreement, it consumes
disagreement. There is nothing left to protect.

Nothing else in the crate needs the property either:

* `needs_producing` (`have.rs:56-61`) walks every bot but asks "does **any**
  single bot hold `count`", which is well defined under any distribution.
* `available(Holder::Anyone, ..)` and `item_totals` are sums, likewise.
* Every other shortfall is taken against one `Holder`.

A guard that no longer guards anything is decoration, so `check_bots_interchangeable`,
`scattered_items`, `collect_scattered_items`, `PlannerError::BotsNotInterchangeable`
(`error.rs:149-158`) and the five tests around them go together.

### What is *not* protected, before or after

One residual hazard, named so nobody thinks the guard was covering it. **A
share's chain has no owner** (`mod.rs:422-442`, and
`a_share_welds_its_whole_subtree_to_one_chain` asserts `owner_of(chain) == None`).
A chain sized against `b_j`'s spare stock may be scheduled onto `b_k`. That is
safe for two reasons, neither of which is the guard:

* For the split item itself, the shares sum to `need` and each chain *produces*
  its `w_j` wherever it runs, so the roster total is right under any assignment.
* For anything the chain treated as already held, `Smelt` and `HandCraft` emit
  `Condition::HasItem` at the full count on the consuming action
  (`have.rs:406-410`, `have.rs:714-718`), and `schedule` judges feasibility per
  candidate bot against a forked state (`schedule.rs:323-341`), falling back
  through every tier to the whole roster. A bot that does not hold the items is
  never offered the action.

The guard never covered this — it compared only the split item — so removing it
does not widen the hazard. `seeded_roster.rs::a_plan_split_over_a_roster_is_not_schedulable_on_a_subset_of_it`
already pins the shape of it and must keep passing.

I probed for the failure the guard was introduced against ("`expand` succeeds
and `schedule` fails forty actions later") and **could not reproduce it** in any
scenario: a 62-action `have(automation-science-pack, 10)` on a freeplay-asymmetric
roster expands and schedules (7092 ticks), as do `have(iron-gear-wheel, 20)` (34
actions) and `have(iron-plate, 40)` (30 actions). See §7 for what that does and
does not establish.

---

## 5. Blast radius

### Source

| file | change | why |
| --- | --- | --- |
| `crates/planner/src/method/have.rs:1019-1039` | the rule | the whole spec |
| `crates/planner/src/method/mod.rs:227` | drop the guard call | dead |
| `crates/planner/src/method/mod.rs:238-312` | delete `check_bots_interchangeable`, `scattered_items`, `collect_scattered_items` | dead |
| `crates/planner/src/method/mod.rs:38-42` | rewrite the `ExpansionCtx` doc | it asserts the assumption being removed |
| `crates/planner/src/error.rs:149-158` | delete `BotsNotInterchangeable` | no constructor left |
| `crates/planner/src/goal.rs:33-36` | rewrite the `Holder::Share` doc | same claim, second copy |
| `crates/executor/src/recover.rs:301-310` | rewrite the tier-2 comment | it justifies `min()` by "`expand` rejects a state whose bots are not interchangeable, so *which* bot is chosen cannot change the expansion". After removal that is false — `Researched` sizes its packs against `Holder::Share(ctx.chain_actor)` (`have.rs:887, 908`), so the choice does change the plan. The `min()` **behaviour** stays; only the reason changes, to determinism alone. |

`PlannerError` is not on the HTTP/OpenAPI surface (no reference in
`crates/server/src`), so no snapshot in `app/src/api/openapi.snapshot.json`
moves and the TypeScript contract is untouched. `just test`'s frontend half is
not in the blast radius at all.

### Tests

Measured: with the proposed rule in place, **all 255 planner lib tests and all
22 integration tests pass unchanged**. The list below is therefore short and
specific.

| test | file | verdict |
| --- | --- | --- |
| `expansion_rejects_bots_that_differ_in_the_goals_own_item` | `mod.rs:1485` | **behaviour genuinely different** — delete. The rejection is the thing being removed. |
| `bots_differing_only_in_an_item_the_goal_does_not_touch_may_still_expand` | `mod.rs:1509` | **delete**. It asserts `is_ok()` and would still pass, but it exists only to bound the guard's scope; kept, it would read as though a guard were still there. |
| `expansion_checks_every_item_a_top_level_all_names` | `mod.rs:1531` | **behaviour genuinely different** — delete. |
| `identical_bots_are_accepted` | `mod.rs:1827` | **delete**. Passes either way, and becomes vacuous once nothing can reject. |
| `a_single_bot_is_trivially_interchangeable` | `mod.rs:1843` | **delete**, same reason. |
| `a_share_is_added_to_what_the_bot_already_holds` | `have.rs:2666` | **the expectation was right, the comment was wrong.** The assertion (`mined == vec![1, 1]`) still holds. Its doc comment says "an asymmetric fixture here would describe a plan whose chains are only feasible on the bot they were sized for" — that is precisely the claim this change refutes, so rewrite the comment and add the asymmetric sibling as a new test. |
| `more_bots_finish_sooner` | `red_science.rs:97` | **no change measured** (all 7 red-science tests pass unchanged), but its comment is the crate's only record of the makespan figures. Re-measure and, if `many`/`one` move at all, extend the comment rather than silently retuning the 2350 ceiling. |
| `an_uneven_split_distributes_the_remainder` | `have.rs:2308` | **negative control** — must keep passing byte-identically (`[2,2,3,3]`). Equal holdings ⇒ sorted order is `BotId` order ⇒ nothing moves. |
| `expansion_is_deterministic` | `red_science.rs:206` | **negative control** — must keep passing. |
| `a_shared_goal_splits_into_one_chain_per_bot`, `a_count_smaller_than_the_roster_uses_only_as_many_chains_as_needed`, `a_single_unit_goal_becomes_one_action_not_a_split`, `a_split_emits_shares_not_bot_instructions` | `have.rs:2281, 2335, 2571, 2701` | unchanged, verified passing. |
| `seeded_roster.rs` (3), `smelt_roots.rs` (3), `scheduling.rs` (9) | — | unchanged, verified passing. |

### New tests

**T1 — the case that passes today and must fail after.**
`two_top_level_splits_do_not_double_count_what_the_first_produced`, in
`have.rs`. Four identical bots, `Goal::All[Have{iron-ore, 8, Anyone},
Have{iron-ore, 8, Anyone}]`. Sum the `ActionKind::Mine` counts. **Today: 24.
After: 16.** Write the assertion as `16`; run it before the change to see it
fail with 24, which is the whole point of the test.

**T2 — the remainder goes to the poorest.**
`the_remainder_of_a_split_goes_to_the_bots_holding_least`. Four bots,
`Have{iron-ore, 10, Anyone}`, bot 1 holds 3. Assert the per-bot mine counts are
`{bot 1: 1}` and 2 for each of the others, by reading `Mine.count` alongside the
share's `Holder::Share(bot)` — or, more simply, assert the emitted subgoals from
`SplitAcrossBots::expand` directly, as `a_split_emits_shares_not_bot_instructions`
already does. Today bot 1 gets 2.

**T3 — the small-need case.** `a_share_skips_the_bots_that_already_hold_the_item`.
Four bots, `Have{iron-plate, 10, Anyone}`, bot 1 holds 8. Assert the two emitted
subgoals name bots 2 and 3, not 1 and 2, and that their counts are 1 each.

**T4 — asymmetric end to end.** `a_freeplay_roster_plans_and_schedules`. Four
bots, only bot 1 holding a starting inventory (8 iron plates, 1 stone furnace, 1
burner mining drill, 1 wood), goal `Have{automation-science-pack, 10, Anyone}`.
Assert it expands and schedules. Today this is refused only if the split item
itself differs; make the goal `Have{iron-plate, 40, Anyone}` for a case that is
refused today by `BotsNotInterchangeable` and must succeed after.

**T5 — determinism against roster order.** `the_split_does_not_depend_on_the_order_the_roster_was_listed_in`.
Expand the same asymmetric goal with `registry_for(&[1,2,3,4])` and with
`registry_for(&[4,3,2,1])`; assert the two networks' action labels are equal.
This fails today (the `take(chains)` prefix follows the slice) and is a property
the new rule gives for free.

**T6 — duplicates.** `a_roster_listing_a_bot_twice_still_splits_the_whole_shortfall`.
`SplitAcrossBots { bots: vec![BotId(1), BotId(1), BotId(2)] }`, `need = 4`:
assert two subgoals, one per distinct bot, whose shares sum to 4.

**T7 — the documented trap.** `a_roster_holding_the_count_between_them_plans_nothing`.
Four bots holding 3 iron plates each, `Have{iron-plate, 12, Anyone}` → zero
actions; `Have{iron-plate, 13, Anyone}` → one plate produced. Pins §3.

---

## 6. Determinism

The planner is pure: no I/O, no async, no wall clock, ordered collections only,
floats via `total_cmp`. The rule keeps plan output byte-identical across runs
because every ordering it introduces is over integers with a total order:

* **The sort key is `(u32, BotId)`.** `spare(b)` is a `u32`
  (`state.rs:350-365`); `BotId` is `#[derive(… PartialOrd, Ord …)] pub struct BotId(pub u8)`
  (`ids.rs:22-23`). **No float is involved**, so no `total_cmp` is needed here
  and none should be introduced — a comparison written on floats would be a
  different, worse rule.
* **The key is unique.** `BotId` is unique within the deduped roster, so the
  order is total and `sort_unstable` is as deterministic as a stable sort. State
  that in the comment so a later reader does not "fix" it to `sort`.
* **`spare(b)` is read from `PlanState`**, whose bots live in a `BTreeMap`
  (`state.rs`), whose reservations live in `BTreeMap`s, and which is forked (not
  shared mutably) per expansion. Reading it four times in a loop yields the same
  four numbers in the same order every run.
* **Emission is in ascending `BotId`**, sourced from a `BTreeMap<BotId, _>` or an
  explicit re-sort — deliberately *not* the participation order, so that
  `ActionId` allocation, and hence `schedule`'s `(end, ActionId, BotId)`
  tie-break (`schedule.rs:94`), does not move when holdings do.
* **The rule no longer depends on the caller's slice order**, only on the set of
  bots and their holdings. That is strictly more deterministic than today, and
  T5 pins it.

The one ordering it *relies* on beyond that is `PlanState::bot_ids`' `BTreeMap`
order, which the existing guard already relies on for the same reason
(`mod.rs:259-262`) and which `expansion_is_deterministic` already exercises.

---

## 7. What I could not determine from reading, and what I did not measure

* **Everything empirical here is against `fixture_world()`.** It has no
  players, a synthetic ore layout and no forces, so every makespan quoted is a
  model figure, not observed play. Whether a live four-bot Factorio run behaves
  the same is untested and untestable from this crate.
* **The research path is unmeasured under asymmetry.** `Researched` emits
  `Holder::Share(ctx.chain_actor)` (`have.rs:887, 908`), which at top level is
  the caller's bot, so a research goal's whole pack bill is sized against one
  bot and never split. `crate::test_world` is `#[cfg(test)]` and unreachable
  from an integration test, so I could not build a technology fixture in the
  probe. If asymmetric research misbehaves, this spec does not fix it and does
  not claim to — it is the next thing to look at.
* **I could not reproduce the failure the guard was added for.** The hardening
  plan asserts that asymmetric inventories make `expand` succeed and `schedule`
  fail forty actions later. Six scenarios up to 62 actions all scheduled. Either
  the failure was hypothetical, or it needs a shape I did not find. That is the
  single claim in this spec a reviewer should push hardest on: I am recommending
  the deletion of a guard on the strength of not having found what it was
  guarding against.
* **`Holder::Anyone` reservations (`reserved_by_anyone`, `state.rs:352-359`)
  are never created by any method I found** — `run_steps` reserves under the
  subgoal's own `whose` (`mod.rs:556`), which is `Anyone` only for an
  unsplit top-level goal. I did not chase whether that path is reachable, and
  the rule is indifferent to it either way (`need` already accounts for it).
* **Whether the makespan should be the objective at all.** "Poorest first" is
  justified above by measurement on a handful of goals, not by an argument that
  it is optimal. A different objective (say, concentrating a holding so that a
  later single-inventory consumer can use it) would prefer the *opposite* rule.
  If the supervisor loop ever states an objective, revisit this.

---

## 8. Implementation plan

Each step is independently reviewable, leaves the suite green, and is committed
on its own. Steps 1-3 are correctness and can land while the guard is still
there; step 4 is what unblocks the live run.

### Step 1 — the target reads `available`, not `inventory_count`

*Files:* `crates/planner/src/method/have.rs`.

Write T1 first and watch it fail with 24. Then replace `have.rs:1033`'s
`inventory_count(*bot, item)` with
`ctx.state.available(&Holder::Share(*bot), item)`, and rewrite the comment at
`have.rs:1027-1032` to say why: the shortfall the subgoal computes is taken
against `available`, so the target must be stated in the same ledger or the
chain is asked for `share + reserved` instead of `share`.

Expected: T1 goes 24 → 16. Every other test unchanged (verified).

### Step 2 — dedupe the roster

*Files:* `crates/planner/src/method/have.rs`.

Write T6 first. Build the participant set from the distinct bots of `self.bots`
before computing `chains`, and say in a comment that a duplicated `BotId` used
to get two shares sized one after the other.

### Step 3 — poorest-first participation and remainder

*Files:* `crates/planner/src/method/have.rs`.

Write T2, T3 and T5 first. Implement §2's rule: sort by `(spare, BotId)`, take
`k`, allocate `base`/`rem` in that order, emit in ascending `BotId`. The comment
must carry the measurement — equal work 2156 ticks against levelling's 2427 —
because the obvious alternative reads more principled than it is, and without
the number someone will "simplify" it into levelling.

Expected: T2, T3, T5 pass; `an_uneven_split_distributes_the_remainder` and
`expansion_is_deterministic` still pass byte-identically (the negative controls).

### Step 4 — delete the guard

*Files:* `crates/planner/src/method/mod.rs`, `crates/planner/src/error.rs`.

Write T4 first — it fails today with `BotsNotInterchangeable`. Then delete
`check_bots_interchangeable` (`mod.rs:238-291`), `scattered_items` and
`collect_scattered_items` (`mod.rs:293-312`), the call at `mod.rs:227`, the
`BotsNotInterchangeable` variant (`error.rs:149-158`) and the five tests listed
in §5. Drop the now-unused imports (`BTreeMap`/`BTreeSet` usage in `mod.rs`
should be re-checked, and `ItemId` may become unused there).

### Step 5 — the prose that asserts the assumption

*Files:* `crates/planner/src/method/mod.rs:38-42`,
`crates/planner/src/goal.rs:33-36`, `crates/executor/src/recover.rs:301-310`.

No behaviour change. Rewrite each to say what is now true: each share is
simulated against its own bot's real spare stock; what remains unpinned is that
the *scheduler* may hand a share's chain to a different bot, which is safe
because feasibility is judged per candidate (`schedule.rs:323-341`), not because
bots are alike. `recover.rs`'s `min()` keeps its behaviour and loses its reason:
rewrite it to justify the choice by determinism alone, and record that
`Researched` makes the choice observable.

### Step 6 — pin §3, and re-measure

*Files:* `crates/planner/src/method/have.rs`, `crates/planner/tests/red_science.rs`.

Add T7. Re-run `red_science.rs::more_bots_finish_sooner` and report `one` and
`many`; if they moved, extend the comment's ledger rather than retuning the
ceiling silently. Then `just test`.
