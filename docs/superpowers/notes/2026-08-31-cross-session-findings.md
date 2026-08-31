# Handover from the peer session (factorio-bot-30), night of 2026-08-30/31

Recorded by the other session before it stopped, so it survives that session.
Their area was planning/controlling bots; mine was the frontend transport swap.

## What they landed
    6ec58f36  walk ticks recorded (were measured by Actuator::walk and discarded)
    783b995d  exec_bounded: OS watchdog thread; the tokio timeout could not fire
    4809ff19  mockall unified on 0.14
    0d93dc3b  hand-mining speed: mining_time / (character.mining_speed x (1 + force.manual_mining_speed_modifier))

## Their open items, in the order they wanted them picked up

1. **Smelting ignores the furnace's own `crafting_speed`.** Correct for a
   stone furnace, wrong for an electric one. **Top item**: it produces silently
   wrong plans during ordinary play — build an electric furnace, which everyone
   does, and every smelting estimate is wrong with nothing failing.
   (Hand-crafting is fine; character `crafting_speed` is 1.)

2. **Research-trigger technologies are not costed.** `electronics`,
   `steam-power`, `automation-science-pack`, and now `steel-axe` — which after
   0d93dc3b gets its mining RATE right and its research DURATION wrong. Plans
   through them are correctly ordered but under-costed.

3. **`ExecutionLog` needs a fifth state, "lost track".** Additive. Unblocks the
   replay scrubber rendering a lost action distinctly from a failed one.

4. **The dispatch tick is discarded on failure** — the last absent-by-plumbing
   value left in the design, same family as the walk ticks.

## Two rules from the night worth keeping

**"Is it connected?" before "is it correct?"** Five artifacts tonight looked
like they were doing a job and thereby stopped anyone asking whether the job
existed: a comment asserting walks had no identity; a guard whose timeout could
not fire; `[build-dependencies]` in `crates/core` with no `build.rs`; the same
in `app/src-tauri`; and `character.mining_speed: 0.5` already in the snapshot,
correct, and never read.

**Bidirectionality is a property of the comparison, not of having two
assertions.** Two checks against one wrong reference is one check wearing a
disguise — the two sides must derive from genuinely independent sources. Found
twice: the OpenAPI contract table whose `type` had no link to the declared TS
type (fixed in 07375cc5), and their doc-entry test made bidirectional against a
hardcoded literal rather than against the goal table.

**Deadness and safety-to-remove are different properties.** Before deleting
something dead, ask what silently depended on its mere presence — e.g.
`restapi = ["dep:factorio-bot-server"]` would have stopped activating the
feature had the build-dependency been its only declaration.

## Closed, deliberately
The intermittent `goal` test hang is unreproducible — 8451 runs across three
commits, zero hangs. Most likely it lived in uncommitted working-tree code at
~16:55 on 30 Aug, which git cannot reproduce at any budget. The mechanism
(a synchronous non-yielding future that `tokio::time::timeout` cannot bound) is
fixed in 783b995d. The artifact is gone; the class is handled.

---

## Later additions (peer session, same night)

### `Status::Lost` is plumbed — and its ASYMMETRY matters for the renderer
`e3e50cb7` added the state, `5f6c3fa6` made `RconActuator` able to produce it.
Lua string `"lost"`, plus `obs.lost`. Additive; the existing four untouched.

Design principle worth reusing: **the phase is recorded at the statement that
knows, not inferred from the error.** `Dispatch::NoVerdict` has exactly one
producer — `sleep_for_action_result` timing out — reachable only after the game
acknowledged the dispatch RPC. Everything weaker defaults to `NotDispatched`,
via the `From<Report>` impl every pre-dispatch `?` uses. **The honest answer is
the default; you must opt in to claiming more.**

`dispatch` is a separate field, not `ticks.dispatched.is_some()`. Evidence that
a dispatch happened and the measurement of when are different facts. Collapsing
them would make a parse failure look like a command that never went out.

**FOR THE SCRUBBER LEGEND — write this footnote in:**
  `Lost` appearing is trustworthy. `Lost` NOT appearing is not yet proof the
  run didn't lose track.
Two reasons: (1) not field-verified, no SDL video for a client and a headless
server with no players cannot build an actuator, so it rests on tests; (2) a
failed RCON round trip cannot distinguish a failed connect from a failed read,
so it stays `NotDispatched` — under-claiming deliberately. `Lost` means less
often than it could, never more often than it should.

### A pattern that defeats my sweep methodology — record it before I reuse it
Every instance so far was a field that arrived **never**: `crafting_speed` 0 of
1028, `durability` nil on the one tool that has it. Both were findable by
presence-checking a capture.

`technology.prototype.effects` is **partly working**. `automation.unlocked_recipes`
arrives populated, so a coarse "do effects arrive?" check passes — and that is
exactly the check we ran and passed. But a read surviving for one
`TechnologyModifier` variant while silently dropping another is invisible to it.

**A field that arrives SOMETIMES, for some variants of a union, defeats
presence-checking entirely, and the variant that works is the one you will
happen to test.** Enumerating the variants and checking which survive is the
only honest form of that check.

**This is a stated limitation of my own `types.lua` sweep** (1c75b866): it
checked whether each field resolves, not whether it resolves for every variant
of what it can hold. The sweep's clean verdict covers the first question only.

### THREE failure shapes, each defeating the check that caught the last
Established across both sessions' sweeps. `14246fcc` added the third.

  1. **Arrives never.** `crafting_speed` 0/1028; `durability` nil on the one
     tool that has it. A presence count over a live capture finds it.
  2. **Arrives sometimes.** Variant-dependent — one `TechnologyModifier`
     variant survives, another silently drops. Presence checking PASSES.
     Only enumerating the variants and checking which survive is honest.
  3. **Arrives always and is wrong.** `collision_mask` shipped a plausible,
     non-empty `["layers"]` on 796 of 1028 prototypes — the read iterated the
     2.0 `CollisionMask` wrapper instead of its `.layers`. Presence checking
     passes AND the data looks reasonable. Only checking content against what
     it should contain finds it.

**My `types.lua` sweep (1c75b866) is silent on 2 and 3, and `collision_mask`
was among the 13 call sites it cleared.** The honest verdict: clean on
*resolves-at-all*, silent on *resolves-for-every-variant*, silent on
*resolves-to-the-right-thing*.

### A fourth shape with no catch-all to find
`connection_type` read 0/95 under its 1.1 name `type`. `conn` is a plain Lua
**table**, not userdata, so the missing key yielded nil with **no error raised
and no `pcall` involved**. Every sweep either session ran hunted catch-alls;
this class has none. What finds it is a presence count over a live capture —
the tool, not the pattern-match.

### The trap that sits inside the sweep method itself
`manual_mining_speed_modifier` read 0/1 in the committed fixture, which is
exactly the `crafting_speed` signature — **only because the fixture predates
the field.** Fresh capture: 1/1. A method whose primary evidence is "count
field presence in a capture" has "the capture is stale" as its own primary
failure mode. Re-capture before believing an absence.

### Housekeeping that affects anyone touching the mod
`workspace/mods/` had drifted from the repo copy before `1c75b866` and is now
identical again. Per CLAUDE.md the workspace copy wins in debug builds and
there is no refresh path, so **check both when editing the mod.**

### TWO CLASSES OF CONSUMER BREAK, and grepping for values finds only one
From the Direction 8→16 widening. Both keep running; neither raises.

  - hardcoded a **VALUE** — `direction: 2` meant east, now means northeast —
    **silently does the wrong thing**
  - hardcoded a **COUNT** — `#directions_all() == 8` — **loudly reports the
    wrong thing**

`scripts/api_test.lua:14` is the second kind, and it is not a test: it is a
script shipped to users that prints its own verdict, so it prints `FAIL ✗`
with no build going red and no CI catching it. A user's first read is "the API
is broken" while the API is correct. A failing test costs a developer five
minutes; this delivers a wrong diagnosis to the person we are trying not to
confuse.

Anyone migrating scripts must check for both. **Grepping for numeric direction
arguments finds only the first class.**

Relevance to plan 6: my task briefs carry measured counts ("seven `<Button>`
call sites"). Those are deliberately framed as TRIPWIRES — "if you count
differently, stop and tell me" — never as completion tests. A count used as a
completion test is the same defect in a different medium.
