# Share-binding report (option 1)

**Status:** Done. All gates green.

**Commit:** `c470388b` — fix(planner): bind a Holder::Share chain's ownership
to the bot it is sized against. (Landed on `master`, the branch checked out
at commit time — not `feat/axum-server`, which is what this conversation's
initial git status reported; see Concerns.)

**Test summary:** `cargo fmt` clean; `cargo clippy --workspace --all-features
--all-targets -- --deny warnings` clean; `cargo test --workspace` green, 0
failed across every crate. New regression test
`method::have::tests::a_cheaper_bot_does_not_steal_a_share_chain_sized_for_another`
(`crates/planner/src/method/have.rs`) confirmed to fail pre-fix with
`PreconditionUnsatisfied { bot: BotId(4), condition: "has 50 iron-ore" }` —
exactly the note's predicted repro — and passes post-fix with the chain bound
to and running entirely on `BotId(2)`.

**Existing tests that changed, and why:**

- `method::mod::tests::a_share_welds_its_whole_subtree_to_one_chain` and
  `method::have::tests::a_single_unit_goal_becomes_one_action_not_a_split`:
  asserted `owner_of(chain) == None` for a share's chain. **Genuinely
  different now, not a wrong old expectation** — that assertion was pinning
  the exact defect this task fixes. Both updated to `Some(bot)`.
- `schedule::tests::an_owner_that_cannot_run_its_chain_blames_the_caller_not_the_world`:
  pinned `ChainOwnerInfeasible`'s old message text verbatim. Updated to the
  corrected wording — a message-text fix, not a behaviour change.
- `crates/planner/tests/seeded_roster.rs::a_plan_split_over_a_roster_is_not_schedulable_on_a_subset_of_it`:
  **the one real surprise.** It schedules a network expanded for a 2- or
  4-bot roster against a narrower roster containing only bot 1. Before this
  change, an ownerless share chain let bot 1 pick up every chain and fail on
  `PreconditionUnsatisfied` (out of furnaces) several steps in. Now the
  chains opened for bots 2/3/4 are *owned* by bots not present in the
  narrower roster at all, so scheduling fails immediately with
  `PlannerError::UnknownBot` instead. Judged a strictly more honest failure
  (names the actual mistake — scheduling on a roster missing a bot the plan
  was built for — rather than an incidental resource exhaustion) and updated
  accordingly, not loosened.

**Concerns:**

1. **Branch drift.** This conversation's initial git status named
   `feat/axum-server` as the current branch; by commit time the checkout was
   `master`, with no checkout event in the visible reflog — it appears the
   shared working tree was already on `master` when this task began
   (consistent with the ongoing multi-agent session's commit history, e.g.
   `f3a22e29`, `9a62dd82`, `886e9c6b` immediately preceding this work, all on
   the same line). Worth confirming this is the intended integration branch
   for tonight's planner work.
2. **Option 3 not attempted**, as instructed — `Researched`'s bill is still
   sized against one bot's inventory, just now honestly bound to that bot
   rather than silently mis-sized. The parallelism cost (22,072 ticks in the
   repro) is real and will show up as slower live runs until someone gives
   `Researched` a real multi-bot decomposition.
3. **Not verified against a live Factorio run** (out of scope per
   instructions — no Factorio was run). The fix is proven only at the
   planner-unit level; the executor/live-run path that originally surfaced
   this (two crashes in one run, naming bots 2 then 3) was not re-run.
4. Touched only `crates/planner/*` and one docs note, per the task's
   boundary; did not touch `mods/BotBridge/control.lua` or
   `crates/core/src/process/output_parser.rs`.
