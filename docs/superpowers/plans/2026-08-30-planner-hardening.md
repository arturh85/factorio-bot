# Planner Hardening — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Holder::Bot` mean what it says, open chains only where work actually has to converge, and clear four constraints that would otherwise shape the execution increment around them.

**Architecture:** `Holder` splits into a caller instruction (`Bot`) and an internal share marker (`Share`), so a bot named by a caller is honoured while a split's shares stay freely assignable. Chains stop being opened by "a goal named a bot" and start being opened by "this method's decomposition needs several produced items in one inventory" — which is `HandCraft` with two or more ingredients to produce, and nothing else. The remaining tasks are independent: same-chain edge inference, mining beyond one tile, siting a furnace near its ore, and a guard on the driver's interchangeable-bots assumption.

**Tech Stack:** Rust 2024, `factorio-bot-core`, the `crates/planner` engine and method layer.

**Spec:** `docs/superpowers/specs/2026-08-29-multi-agent-planner-design.md`

**Prior plans (both complete):** `2026-08-29-planner-scheduling-engine.md`, `2026-08-30-planner-goals-and-methods.md`. The execution increment — RCON, `ExecutionLog`, the Lua rewrite, deleting `core/plan` — comes after this one.

## Why these five, and why now

Each was found by review or by running the crate, and each is recorded with its evidence in `.superpowers/sdd/2026-08-30-planner-goals-and-methods/progress.md`.

1. **`Holder::Bot(3)` does not mean bot 3.** It steers expansion-time simulation and chain formation; the emitted actions stay unpinned, so the scheduler binds the chain wherever it is cheapest. Demonstrated: with bot 3 parked 300 tiles away, `Have(iron-ore, 3, Bot(3))` yields a plan in which bot 1 mines. `Holder` now derives `Serialize`/`Deserialize`, so an HTTP caller will read `Bot(3)` as an instruction that it is not.
2. **A single-unit goal serialises onto one bot.** `Have(iron-plate, 1)` on four bots went from 846 to 1401 ticks, because a top-level goal always opens a chain. Chains are only load-bearing where work converges, and a smelt never converges — it feeds a furnace through three separate actions, so three bots can each supply one input.
3. **`infer_edges` links every producer of an item to every consumer**, across chains that were never meant to depend on each other. Every chain's craft waits on the slowest chain's smelt. This is a large part of why four bots buy 2.45× rather than 4×. `ChainId` is exactly the provenance that did not exist when quantity-blind matching was justified.
4. **`Mine` needs one tile holding the whole request.** `DEFAULT_RESOURCE_PER_TILE` is 500, so `Have(iron-ore, 600)` fails — and fails as `NoApplicableMethod`, which reads as "we do not know how to obtain iron ore", the opposite of the truth.
5. **`Smelt` sites its furnace near the bot's start position**, which never advances during expansion, so every chain walks ore-patch → origin → ore-patch.
6. **The driver's interchangeable-bots assumption is unenforced.** With asymmetric starting inventories `expand` succeeds and `schedule` fails forty actions later. The execution increment's tier-1 recovery — "re-schedule from observed real state" — *always* produces asymmetric inventories, so this blocks its central failure path.

## Global Constraints

- Workspace root: `/home/arturh/projects/private/factorio-bot`. Rust edition **2024** (corrected 2026-09-03; this line said 2021, and every crate's `Cargo.toml` says `edition = "2024"`). Any `rustfmt` invocation therefore **needs `--edition 2024`** — the flag is not optional: bare `rustfmt` defaults to Rust 2015, dies on every `async fn` in the file, and chained with `&&` silently skips whatever came next. Branch `master`.
- **Every cargo invocation must be wrapped:** `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`
- **`cargo fmt -p factorio-bot-planner` before every commit — never `cargo fmt --all`.** Other crates belong to other workstreams.
  **CORRECTED 2026-09-03: `cargo fmt -p <crate>` is banned as well.** CLAUDE.md bans every rewriting `cargo fmt` form, `-p` included — it rewrites a *whole crate*, so it clobbers another agent's uncommitted files in that crate exactly as `--all` already did once. Format only the files you edited: `rustfmt --edition 2024 <file>`. **`--edition 2024` is not optional** — bare `rustfmt` defaults to Rust 2015, dies on every `async fn` in the file, and chained with `&&` silently skips whatever came next. Every `cargo fmt -p …` in the steps below is subject to this.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated` must pass. `assert_eq!(x, true)` trips `bool_assert_comparison`; use `assert!(x)`.
- **Another agent may be working in this checkout with files staged.** Commit with the partial-commit form: `git commit -m "<message>" -- crates/planner`, which ignores the index. A **new** file must be `git add`ed by its exact path first — never `-A`, never `.`, never `-a`. Never `git checkout`/`stash`/`reset`/`clean` outside `crates/planner/`, except discarding `crates/scripting_lua/tests/` snapshot churn after a workspace run.
- `Ticks` is `u32`; 60 ticks = 1 second. Multiply durations and counts with `saturating_mul`, never `*`.
- Iterate `BTreeMap`/`BTreeSet`/`Vec`, never `HashMap`/`HashSet`, anywhere a result can influence output. Determinism is required end to end.
- Build structs with functional-update syntax; an `#[allow(clippy::field_reassign_with_default)]` is the wrong fix.
- **Methods emit `Actor::Role` with `pinned: None`. Nothing pins.** A caller's `Holder::Bot` is honoured through a chain *owner* recorded in the network, not by pinning an action — see Task 1.
- **Never adjust an assertion to match observed output.** If a stated expectation fails, report the actual value and stop. Nine controller errors across these plans were caught exactly that way.
- **RED phase for a new file:** an orphaned `.rs` with no `pub mod` line is silently excluded from the build, so its tests neither run nor fail. Declare the module first.

## Starting state

`crates/planner`: 135 lib tests, 6 in `tests/red_science.rs`, 9 in `tests/scheduling.rs`. Workspace green, clippy clean.

The nine tests in `tests/scheduling.rs` pin the engine's guarantees from the first increment. **They must remain byte-identical.** Two assertions elsewhere pin the chain-spread preference's cost and must not be relaxed: `a_new_chain_prefers_a_bot_that_is_not_carrying_one` asserts 1914, and `travel_cost_can_outweigh_an_idle_bot` asserts 1200.

## File Structure

| File | What changes |
|---|---|
| `crates/planner/src/goal.rs` | `Holder` gains `Share(BotId)` |
| `crates/planner/src/network.rs` | `chain_owner` side map; `infer_edges` restricted to same-chain pairs |
| `crates/planner/src/method/mod.rs` | `Method::converges`; chain opening moves after method selection; roster guard |
| `crates/planner/src/method/have.rs` | `SplitAcrossBots` emits `Share`; `HandCraft::converges`; multi-tile mining; furnace siting |
| `crates/planner/src/method/util.rs` | a tile-list helper for multi-tile mining |
| `crates/planner/src/schedule.rs` | the scheduler honours a chain owner |
| `crates/planner/tests/red_science.rs` | regression coverage for the makespan restoration |

---

### Task 1: Split `Holder`, and open chains where work converges

**Files:**
- Modify: `crates/planner/src/goal.rs`, `crates/planner/src/network.rs`, `crates/planner/src/method/mod.rs`, `crates/planner/src/method/have.rs`, `crates/planner/src/schedule.rs`
- Test: inline `mod tests` in each, plus `crates/planner/tests/red_science.rs`

**Interfaces:**
- Consumes: `Holder::{Anyone, Bot}`, `GoalSite { top_level, in_chain }`, `Method::{name, claims, applicable, expand}`, `ExpansionCtx { state, ids, chains, chain, chain_actor, depth, top_level }`, `ActionNetwork::{set_chain, chain_of}`, `schedule()`'s `chain_binding`.
- Produces: `Holder::Share(BotId)`; `Method::converges(&self, goal: &Goal, state: &PlanState) -> bool` defaulted to `false`; `ActionNetwork::{set_chain_owner(ChainId, BotId), owner_of(ChainId) -> Option<BotId>}`; `PlannerError::ChainOwnerInfeasible { chain, bot, condition }`.

**These two changes must land together.** Splitting `Holder` alone would leave `Share` opening no chain while convergence does not yet open one either, so red science would break in between.

- [ ] **Step 1: Write the failing tests**

In `crates/planner/src/goal.rs`, append to `mod tests`:

```rust
    #[test]
    fn a_share_is_not_an_instruction_to_a_bot() {
        assert_ne!(Holder::Share(BotId(1)), Holder::Bot(BotId(1)));
    }

    #[test]
    fn holders_render_distinguishably() {
        assert_eq!(Holder::Bot(BotId(2)).to_string(), "bot 2");
        assert_eq!(Holder::Share(BotId(2)).to_string(), "a share sized for bot 2");
        assert_eq!(Holder::Anyone.to_string(), "anyone");
    }
```

In `crates/planner/src/network.rs`, append to `mod tests`:

```rust
    #[test]
    fn a_chain_owner_round_trips_and_defaults_to_none() {
        let mut net = ActionNetwork::new();
        assert_eq!(net.owner_of(ChainId(0)), None);
        net.set_chain_owner(ChainId(0), BotId(3));
        assert_eq!(net.owner_of(ChainId(0)), Some(BotId(3)));
        assert_eq!(net.owner_of(ChainId(1)), None);
    }
```

Add `use crate::ids::{BotId, ChainId};` to that test module if it is not already in scope.

In `crates/planner/src/method/have.rs`, append to `mod tests`:

```rust
    #[test]
    fn a_split_emits_shares_not_bot_instructions() {
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        // Reach into the method directly: the driver rewrites nothing, so what
        // SplitAcrossBots emits is what the rest of the plan sees.
        let split = SplitAcrossBots { bots: bots.to_vec() };
        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let goal = Goal::Have { item: "iron-ore".into(), count: 4, whose: Holder::Anyone };
        let steps = split.expand(&goal, &mut ctx).unwrap();
        for step in steps {
            match step {
                Step::Subgoal(Goal::Have { whose, .. }) => {
                    assert!(
                        matches!(whose, Holder::Share(_)),
                        "a split emits shares, not instructions: {:?}",
                        whose
                    );
                }
                other => panic!("expected only subgoals, got {:?}", other),
            }
        }
    }

    #[test]
    fn hand_crafting_converges_only_when_two_ingredients_need_producing() {
        let mut s = state(&[BotId(1)]);
        let asp = Goal::Have {
            item: "automation-science-pack".into(),
            count: 1,
            whose: Holder::Anyone,
        };
        // Both copper-plate and iron-gear-wheel must be produced: they have to
        // meet in one inventory, so this converges.
        assert!(HandCraft.converges(&asp, &s));

        // With the copper already held, only the gear needs producing, so
        // nothing has to meet anything.
        s.gain(BotId(1), "copper-plate", 5);
        assert!(!HandCraft.converges(&asp, &s));

        // A single-ingredient recipe never converges.
        let gear = Goal::Have { item: "iron-gear-wheel".into(), count: 1, whose: Holder::Anyone };
        assert!(!HandCraft.converges(&gear, &s));
    }

    #[test]
    fn smelting_never_converges() {
        let s = state(&[BotId(1)]);
        // A furnace is fed by three separate actions — place, insert ore,
        // insert coal — so three different bots can each supply one input.
        // Nothing has to meet in a single inventory.
        let plate = Goal::Have { item: "iron-plate".into(), count: 2, whose: Holder::Anyone };
        assert!(!Smelt.converges(&plate, &s));
    }

    #[test]
    fn a_linear_goal_gets_no_chain_and_stays_parallel() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for b in bots {
            s.gain(b, "stone-furnace", 1);
        }
        let net = expand(
            &[Goal::Have { item: "iron-plate".into(), count: 1, whose: Holder::Anyone }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        assert!(
            net.actions().all(|a| net.chain_of(a.id).is_none()),
            "a smelt converges nowhere, so nothing needs welding to one bot"
        );
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        let used: std::collections::BTreeSet<_> =
            plan.steps.iter().map(|s| s.bot).collect();
        assert!(used.len() > 1, "the mining roots must not serialise onto one bot");
    }

    #[test]
    fn a_converging_goal_gets_one_chain_over_its_whole_subtree() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        for b in bots {
            s.gain(b, "stone-furnace", 2);
        }
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let chains: std::collections::BTreeSet<_> =
            net.actions().filter_map(|a| net.chain_of(a.id)).collect();
        assert_eq!(chains.len(), 1, "one convergence point, one chain");
        assert!(
            net.actions().all(|a| net.chain_of(a.id).is_some()),
            "the ingredients must be welded to the craft that consumes them"
        );
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        let used: std::collections::BTreeSet<_> =
            plan.steps.iter().map(|s| s.bot).collect();
        assert_eq!(used.len(), 1, "a chain runs on one bot");
    }

    #[test]
    fn a_caller_naming_a_bot_gets_that_bot() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        // Park bot 2 far away, so the scheduler would otherwise never choose it.
        s.set_position(BotId(2), Position::new(300., 0.));
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 3, whose: Holder::Bot(BotId(2)) }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        for step in &plan.steps {
            assert_eq!(step.bot, BotId(2), "the caller named bot 2");
        }
    }
```

Add `use factorio_bot_core::types::Position;` to that test module if it is not already in scope.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL to compile — `Holder::Share`, `Method::converges`, `ActionNetwork::{set_chain_owner, owner_of}` do not exist.

- [ ] **Step 3: Add the `Share` variant**

In `crates/planner/src/goal.rs`, extend `Holder` and its `Display`:

```rust
pub enum Holder {
    /// Satisfied by the sum across every bot.
    Anyone,
    /// A caller's instruction: this bot must end up holding the items. The
    /// scheduler honours it — the chain this goal opens is owned by this bot.
    Bot(BotId),
    /// One share of a split. Sized against this bot's starting inventory, but
    /// carrying no commitment about who runs it: the scheduler is free to give
    /// the work to whichever bot suits.
    ///
    /// Naming a bot here is how the driver keeps each share's simulated
    /// inventory separate. That it *works* rests on bots starting
    /// interchangeable — the assumption `ExpansionCtx` documents, made visible
    /// in the type rather than left in prose.
    Share(BotId),
}
```

```rust
            Holder::Bot(id) => write!(f, "{}", id),
            Holder::Share(id) => write!(f, "a share sized for {}", id),
```

Every `match` on `Holder` in the crate must now handle `Share`. `shortfall` treats it exactly like `Bot` — both read that bot's own inventory:

```rust
    let held = match whose {
        Holder::Anyone => state.total_count(item),
        Holder::Bot(id) | Holder::Share(id) => state.inventory_count(*id, item),
    };
```

- [ ] **Step 4: Add the chain-owner map**

In `crates/planner/src/network.rs`, add the field and its accessors beside `chains`:

```rust
    /// Chains a caller pinned to a bot by naming it in a `Holder::Bot` goal.
    /// Distinct from `chains`: that says which actions travel together, this
    /// says a caller demanded a particular runner. Still no `BotId` on any
    /// action — the constraint belongs to the chain.
    chain_owner: BTreeMap<ChainId, BotId>,
```

```rust
    pub fn set_chain_owner(&mut self, chain: ChainId, bot: BotId) {
        self.chain_owner.insert(chain, bot);
    }

    pub fn owner_of(&self, chain: ChainId) -> Option<BotId> {
        self.chain_owner.get(&chain).copied()
    }
```

- [ ] **Step 5: Add `converges` and move where chains open**

In `crates/planner/src/method/mod.rs`, add to the `Method` trait, beside `claims`:

```rust
    /// Does this method's decomposition require several *produced* items to
    /// meet in one inventory?
    ///
    /// If so the driver opens a chain over its subtree, welding the producers
    /// to the consumer that needs them together. Defaults to `false`, which is
    /// right for every method whose inputs arrive through separate actions —
    /// a furnace is loaded by one insert per ingredient, so three bots can each
    /// supply one and nothing has to converge.
    fn converges(&self, _goal: &Goal, _state: &PlanState) -> bool {
        false
    }
```

Then move chain opening out of `expand_goal` and into `expand_goal_body`, **after** the method is selected — convergence is a property of the method, which is not known before `find`. Delete the `if ctx.chain.is_none() { ctx.chain = Some(ctx.chains.next()); }` block from `expand_goal`, keeping the `chain_actor` rebinding and the whole save/restore bracket exactly as they are: `previous_chain` is still saved and restored in `expand_goal`, so a chain opened in the body is still restored on every exit path including errors.

In `expand_goal_body`, immediately after `let method = registry.find(...)?;`:

```rust
    // A chain welds actions to one runner. Two things ask for that: a caller
    // naming a bot, and a method whose decomposition makes several produced
    // items meet in one inventory. Nothing else — a goal that merely sits
    // inside a split does not need welding, and welding it serialises work
    // that could have run in parallel.
    if ctx.chain.is_none() {
        let owner = match goal {
            Goal::Have { whose: Holder::Bot(bot), .. } => Some(*bot),
            _ => None,
        };
        if owner.is_some() || method.converges(goal, &ctx.state) {
            let chain = ctx.chains.next();
            ctx.chain = Some(chain);
            if let Some(bot) = owner {
                net.set_chain_owner(chain, bot);
            }
        }
    }
```

Extend the `chain_actor` rebinding in `expand_goal` to cover shares:

```rust
    if let Goal::Have { whose: Holder::Bot(bot) | Holder::Share(bot), .. } = goal {
        ctx.chain_actor = *bot;
    }
```

- [ ] **Step 6: Emit shares from the split, and declare convergence**

In `crates/planner/src/method/have.rs`, change `SplitAcrossBots::expand`'s subgoal to `whose: Holder::Share(*bot)`, leaving the `held.saturating_add(share)` reasoning and its comment untouched.

`SplitAcrossBots::applicable` must keep declining anything that is not `Holder::Anyone`, which already covers `Share`.

Its `claims` doc comment is now stale and must be corrected: it says "only a `Holder::Bot` goal opens a chain and this method declines those", which stops being true here — convergence opens chains too. `claims` itself is unchanged (`site.top_level && !site.in_chain`) and still correct, because `in_chain` reflects the chain a goal *inherited*, computed before this goal opens one. Say that instead.

Add to `impl Method for HandCraft`:

```rust
    fn converges(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if shortfall(state, item, *count, whose) == 0 {
            return false;
        }
        let Some(recipe) = recipe_for(state, item) else {
            return false;
        };
        // One craft action carries a `HasItem` for every ingredient, so each
        // one that still has to be produced is a separate sub-chain that must
        // land in the same inventory. Two or more of those is a convergence.
        ingredients_of(&recipe)
            .iter()
            .filter(|(ingredient, amount)| {
                shortfall(state, ingredient, amount.saturating_mul(1), whose) > 0
            })
            .count()
            >= 2
    }
```

- [ ] **Step 7: Honour a chain owner in the scheduler**

In `crates/planner/src/schedule.rs`, add the error variant to `crates/planner/src/error.rs`:

```rust
    #[error("{bot} owns chain {chain:?} because a caller named it, but {condition} does not hold there")]
    #[diagnostic(code(planner::chain_owner_infeasible))]
    ChainOwnerInfeasible {
        chain: ChainId,
        bot: BotId,
        condition: String,
    },
```

In the candidate-set construction, an owned chain has exactly one candidate, and — unlike the spread *preference* — there is no fallback tier, because a caller asked for this bot specifically:

```rust
        // A chain owner is a caller's instruction, so it is a hard constraint:
        // no tier falls back past it. The spread preference below is a
        // preference precisely because nobody asked for it.
        if let Some(owner) = net.chain_of(action.id).and_then(|c| net.owner_of(c)) {
            candidate_bots = vec![owner];
        }
```

When an owned chain's only candidate is infeasible and no other ready action can proceed, report `ChainOwnerInfeasible` rather than `PreconditionUnsatisfied`, so the failure names the caller's instruction as the cause.

- [ ] **Step 8: Run the tests**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS, including all 6 `red_science` tests and all 9 `tests/scheduling.rs` tests unchanged.

**Report the red-science makespans.** `Have(iron-plate, 1)` on four bots should return to roughly 846 from 1401; the four-bot 4-pack and 10-pack numbers should not regress. If any makespan moves the wrong way, report it and stop.

- [ ] **Step 9: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): honour a named bot, and chain only where work converges" -- crates/planner
```

---

### Task 2: Restrict inferred edges to one chain

**Files:**
- Modify: `crates/planner/src/network.rs`
- Test: inline `mod tests` in `crates/planner/src/network.rs`, plus a makespan assertion in `crates/planner/tests/red_science.rs`

**Interfaces:**
- Consumes: `ActionNetwork::{chain_of, infer_edges}`, `Effect::satisfies`.
- Produces: no new API. `infer_edges` changes behaviour.

`infer_edges` links every producer of an item to every consumer of it. Before chains existed there was no way to tell whether two actions belonged to the same piece of work, and the doc says so. `ChainId` is exactly that information.

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/planner/src/network.rs`:

```rust
    #[test]
    fn inference_does_not_link_across_chains() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let a_mine = net.add(mine(&mut gen, "iron-plate", 2));
        let a_craft = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        let b_mine = net.add(mine(&mut gen, "iron-plate", 2));
        let b_craft = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        net.set_chain(a_mine, ChainId(0));
        net.set_chain(a_craft, ChainId(0));
        net.set_chain(b_mine, ChainId(1));
        net.set_chain(b_craft, ChainId(1));

        net.infer_edges();

        assert_eq!(net.preds(a_craft), vec![(a_mine, 0)], "chain 0 only");
        assert_eq!(net.preds(b_craft), vec![(b_mine, 0)], "chain 1 only");
    }

    #[test]
    fn inference_still_links_when_either_side_has_no_chain() {
        let mut gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let m = net.add(mine(&mut gen, "iron-plate", 2));
        let c = net.add(craft(&mut gen, "iron-plate", 2, "iron-gear-wheel"));
        net.set_chain(c, ChainId(0));
        // The producer belongs to no chain, so nothing says these are separate
        // work — the edge must stand.
        net.infer_edges();
        assert_eq!(net.preds(c), vec![(m, 0)]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner --lib network'`
Expected: FAIL — `inference_does_not_link_across_chains` sees four predecessors where it expects one.

- [ ] **Step 3: Restrict the match**

In `infer_edges`, skip a candidate pair whose chains are both known and different:

```rust
                // Two actions in different chains are different pieces of work:
                // chain binding guarantees each runs on one bot and each is
                // self-sufficient, so a consumer never depends on a foreign
                // chain's producer. When either side has no chain, nothing says
                // they are separate and the edge stands.
                if let (Some(p), Some(c)) = (self.chain_of(*producer), self.chain_of(*consumer)) {
                    if p != c {
                        continue;
                    }
                }
```

Update the doc comment: the paragraph explaining that inference cannot see provenance is now wrong, because `ChainId` is provenance. Say instead that item matching still ignores quantities, and that cross-chain pairs are excluded.

- [ ] **Step 4: Run the tests**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS.

**Report the four-bot red-science makespans before and after this task.** This is expected to be the largest single improvement in the crate; if it does not move, say so, because that would mean the over-ordering was not on the critical path after all.

- [ ] **Step 5: Pin the improvement**

Add to `crates/planner/tests/red_science.rs` an assertion that four bots beat one by a factor you observed in step 4, stated as a strict inequality with a comment giving the measured figures. Do not assert an exact makespan — it will move again in later tasks.

- [ ] **Step 6: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "fix(planner): stop inferring edges between separate chains" -- crates/planner
```

---

### Task 3: Mine across as many tiles as the request needs

**Files:**
- Modify: `crates/planner/src/method/util.rs`, `crates/planner/src/method/have.rs`
- Test: inline `mod tests` in both

**Interfaces:**
- Consumes: `PlanState::{resource_patches, resource_available}`, `nearest_resource_tile`.
- Produces: `pub fn resource_tiles_for(state: &PlanState, item: &str, from: &Position, need: u32) -> Vec<(Position, u32)>` — tiles nearest-first with how much to take from each, empty when the total available is short.

`Mine` currently requires one tile holding the whole request, so `Have(iron-ore, 600)` fails against a 500-per-tile cap — and reports `NoApplicableMethod`, which reads as "we do not know how to obtain iron ore".

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/planner/src/method/util.rs`:

```rust
    #[test]
    fn one_tile_is_enough_for_a_small_request() {
        let s = state();
        let tiles = resource_tiles_for(&s, "iron-ore", &Position::new(0., 0.), 5);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].1, 5);
    }

    #[test]
    fn a_large_request_spans_tiles_nearest_first() {
        let s = state();
        // 500 per tile, so 1200 needs three: 500 + 500 + 200.
        let tiles = resource_tiles_for(&s, "iron-ore", &Position::new(0., 0.), 1200);
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles.iter().map(|(_, n)| *n).sum::<u32>(), 1200);
        assert_eq!(tiles[0].1, 500);
        assert_eq!(tiles[1].1, 500);
        assert_eq!(tiles[2].1, 200);
        // Nearest first: distances must be non-decreasing.
        let origin = Position::new(0., 0.);
        for pair in tiles.windows(2) {
            let a = calculate_distance(&origin, &pair[0].0);
            let b = calculate_distance(&origin, &pair[1].0);
            assert!(a <= b, "tiles must come nearest-first: {} then {}", a, b);
        }
    }

    #[test]
    fn a_request_larger_than_the_patch_yields_nothing() {
        let s = state();
        let tiles = resource_tiles_for(&s, "iron-ore", &Position::new(0., 0.), 10_000_000);
        assert!(tiles.is_empty());
    }

    #[test]
    fn an_absent_resource_yields_nothing() {
        let s = state();
        assert!(resource_tiles_for(&s, "uranium-ore", &Position::new(0., 0.), 1).is_empty());
    }
```

Append to `mod tests` in `crates/planner/src/method/have.rs`:

```rust
    #[test]
    fn a_request_larger_than_one_tile_mines_several() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 1200, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let mined: u32 = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { count, .. } => *count,
                other => panic!("expected only mines, got {:?}", other),
            })
            .sum();
        assert_eq!(mined, 1200);
        assert_eq!(net.len(), 3, "500 + 500 + 200");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `resource_tiles_for` does not exist, and the 1200 goal currently yields `NoApplicableMethod`.

- [ ] **Step 3: Write the helper**

In `crates/planner/src/method/util.rs`:

```rust
/// Tiles of `item` to draw `need` from, nearest first, with how much to take
/// from each. Empty when the patches cannot supply `need` in total.
///
/// Ties on distance break on `(x, y)`, like `nearest_resource_tile`, so the
/// result depends only on the tile set and the origin.
pub fn resource_tiles_for(
    state: &PlanState,
    item: &str,
    from: &Position,
    need: u32,
) -> Vec<(Position, u32)> {
    let mut candidates: Vec<(f64, Position, u32)> = Vec::new();
    for patch in state.resource_patches(item) {
        for tile in patch.elements {
            let available = state.resource_available(&tile, item);
            if available == 0 {
                continue;
            }
            candidates.push((calculate_distance(from, &tile), tile, available));
        }
    }
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.1.y.total_cmp(&b.1.y))
    });

    let mut out = Vec::new();
    let mut remaining = need;
    for (_, tile, available) in candidates {
        if remaining == 0 {
            break;
        }
        let take = available.min(remaining);
        remaining -= take;
        out.push((tile, take));
    }
    if remaining > 0 {
        return Vec::new();
    }
    out
}
```

- [ ] **Step 4: Use it in `Mine`**

`Mine::applicable` now asks whether the tiles can supply the need in total:

```rust
        // Position-independent, as before: `resource_tiles_for` returns empty
        // exactly when the patches cannot supply `need` in total, whatever the
        // origin, so applicability does not depend on which bot is asking.
        // Which tiles are nearest is `expand`'s business, where the chain actor
        // is known.
        !resource_tiles_for(state, item, &Position::default(), need).is_empty()
```

`Mine::expand` emits one action per tile, each with its own `AtPosition`, `ResourceAvailable`, `ConsumeResource` and `GainItem` for that tile's share, and a duration of `mining_ticks(...).saturating_mul(take)`. Keep `Actor::Role` and `pinned: None` on every one. Label them `format!("mine {} {}", take, item)`.

- [ ] **Step 5: Run the tests**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS.

- [ ] **Step 6: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): mine across as many tiles as the request needs" -- crates/planner
```

---

### Task 4: Site a furnace near its ore

**Files:**
- Modify: `crates/planner/src/method/have.rs`
- Test: inline `mod tests` in `crates/planner/src/method/have.rs`

**Interfaces:**
- Consumes: `free_tile_near`, `nearest_resource_tile`, `recipe_for`, `ingredients_of`.
- Produces: no new API.

`Smelt` sites its furnace with `free_tile_near(state, bot_start_position)`, and the bot's position never advances during expansion, so every chain walks ore-patch → origin → ore-patch.

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/planner/src/method/have.rs`:

```rust
    #[test]
    fn a_furnace_is_sited_near_the_ore_it_smelts() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have { item: "iron-plate".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();

        let furnace = net
            .actions()
            .find_map(|a| match &a.kind {
                ActionKind::Place { entity } => Some(entity.position.clone()),
                _ => None,
            })
            .expect("a placement");
        let ore = net
            .actions()
            .find_map(|a| match &a.kind {
                ActionKind::Mine { pos, item, .. } if item == "iron-ore" => Some(pos.clone()),
                _ => None,
            })
            .expect("an iron-ore mine");

        let to_ore = calculate_distance(&furnace, &ore);
        let to_origin = calculate_distance(&furnace, &Position::new(0., 0.));
        assert!(
            to_ore < to_origin,
            "the furnace should sit by the ore ({} away) not the bot's start ({} away)",
            to_ore,
            to_origin
        );
    }
```

Add `use factorio_bot_core::factorio::util::calculate_distance;` to the test module if it is not already in scope.

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner --lib a_furnace_is_sited'`
Expected: FAIL — the furnace sits near the origin, so `to_ore` exceeds `to_origin`.

- [ ] **Step 3: Site from the ore**

In `Smelt::expand`, choose the reference point for `free_tile_near` as the nearest tile of the recipe's first ingredient, falling back to the bot's position when there is none:

```rust
        // Site the furnace by the ore rather than by the bot's start, which
        // never advances during expansion — otherwise every chain walks
        // ore-patch, origin, ore-patch.
        let anchor = ingredients_of(&recipe)
            .first()
            .and_then(|(ingredient, _)| {
                nearest_resource_tile(&ctx.state, ingredient, &from, 1)
            })
            .unwrap_or(from.clone());
        let pos = free_tile_near(&ctx.state, &anchor).ok_or_else(|| { /* as before */ })?;
```

- [ ] **Step 4: Run the tests**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS. **Report the red-science makespans** — this should reduce them, and by how much is worth knowing.

- [ ] **Step 5: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): site a furnace by its ore rather than the bot's start" -- crates/planner
```

---

### Task 5: Fail loudly when bots are not interchangeable

**Files:**
- Modify: `crates/planner/src/method/mod.rs`, `crates/planner/src/error.rs`
- Test: inline `mod tests` in `crates/planner/src/method/mod.rs`

**Interfaces:**
- Consumes: `PlanState::{bot_ids, bot}`, `ExpansionCtx`.
- Produces: `PlannerError::BotsNotInterchangeable { a, b, item }`.

The driver simulates chain *j* against bot *j* on the assumption that any bot would experience the same thing. With asymmetric starting inventories `expand` succeeds and `schedule` fails forty actions later. The execution increment's tier-1 recovery re-schedules from observed real state, which *always* produces asymmetric inventories — so this must fail at the boundary, loudly, rather than deep inside a plan.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/planner/src/method/mod.rs`:

```rust
    #[test]
    fn expansion_rejects_bots_that_are_not_interchangeable() {
        let bots = [BotId(1), BotId(2)];
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        state.gain(BotId(1), "iron-plate", 12);
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let goal = Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone };
        assert!(matches!(
            expand(&[goal], &state, &reg, BotId(1)),
            Err(PlannerError::BotsNotInterchangeable { .. })
        ));
    }

    #[test]
    fn identical_bots_are_accepted() {
        let bots = [BotId(1), BotId(2)];
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &bots);
        for b in bots {
            state.gain(b, "stone-furnace", 2);
        }
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let goal = Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone };
        assert!(expand(&[goal], &state, &reg, BotId(1)).is_ok());
    }

    #[test]
    fn a_single_bot_is_trivially_interchangeable() {
        let mut state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        state.gain(BotId(1), "iron-plate", 12);
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let goal = Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone };
        assert!(expand(&[goal], &state, &reg, BotId(1)).is_ok());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner --lib interchangeable'`
Expected: FAIL — `BotsNotInterchangeable` does not exist and expansion currently accepts anything.

- [ ] **Step 3: Add the variant**

In `crates/planner/src/error.rs`:

```rust
    #[error("{a} and {b} hold different amounts of {item}; expansion sizes each share against one bot's inventory and assumes any bot would do")]
    #[diagnostic(
        code(planner::bots_not_interchangeable),
        help("re-plan per bot, or extend the driver to size shares against the bot that will run them")
    )]
    BotsNotInterchangeable { a: BotId, b: BotId, item: ItemId },
```

- [ ] **Step 4: Check at the boundary**

At the top of `expand`, before forking the state, compare every bot's inventory against the first bot's and return on the first difference. Compare only inventories — positions differ legitimately and are handled by travel cost. Report the lexicographically first differing item so the message is deterministic.

- [ ] **Step 5: Run the tests**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS, and **all 6 `red_science` tests must still pass** — its fixture gives every bot the same two furnaces. If any test that previously passed now trips this guard, report which and stop: that would mean an existing test relies on asymmetry, which is worth knowing before the guard lands.

- [ ] **Step 6: Verify lints, run the workspace suite, and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test --workspace'
git checkout -- crates/scripting_lua/tests/
git commit -m "feat(planner): reject expansion when bots are not interchangeable" -- crates/planner
```

---

## Self-Review

**Coverage.** Each of the six motivations maps to a task: `Holder::Bot` honoured and single-unit serialisation both in Task 1 (they are one change — the trigger for opening a chain), cross-chain inference in Task 2, multi-tile mining in Task 3, furnace siting in Task 4, and the interchangeability guard in Task 5.

**Why Task 1 is one task rather than two.** Splitting `Holder` and moving the chain trigger have to land together. `Share` not opening a chain, while convergence does not yet open one either, would leave red science red in between — and this plan should never have a deliberately red intermediate state, because the previous increment already showed how much noise that creates.

**Deliberate omissions, flagged.** `#[non_exhaustive]` on `GoalSite` (a breaking change later if a third site fact appears, though only tests build one today); `critical_path` on `Schedule`; `edges()` and merge on `ActionNetwork`; `ActionId`s unique only within one `expand` call, which matters once `ExecutionLog` keys on them across a re-plan. All belong with the execution increment, which is when they first bite.

**A risk this plan does not close.** Multiple top-level `Have` goals that feed each other remain unsound: goal 2's chain is bound independently of goal 1's, so items the expansion handed between them through the simulated inventory may not be there at run time. It fails at `schedule` with a precondition error rather than silently. Task 2 narrows it — cross-chain edges no longer imply a dependency that assignment cannot honour — but does not eliminate it. The real fix is a `Consolidate` method, which needs container inventory that `PlanState` does not model.

**Type consistency.** `Holder::{Anyone, Bot, Share}` is matched the same way in `shortfall`, the `chain_actor` rebinding and the chain-opening check. `Method::converges(&Goal, &PlanState) -> bool` has one signature, defaulted once, overridden once. `ActionNetwork::{set_chain, chain_of, set_chain_owner, owner_of}` are used with those exact names in Tasks 1 and 2. `resource_tiles_for(state, item, from, need) -> Vec<(Position, u32)>` is defined in Task 3 and used only there.

**One thing an implementer should push back on.** Task 1's `HandCraft::converges` calls `shortfall` per ingredient with `amount.saturating_mul(1)`, which is `amount` — written that way to make the units obvious rather than to compute anything. If it reads as noise, simplify it to `amount` and say so.
