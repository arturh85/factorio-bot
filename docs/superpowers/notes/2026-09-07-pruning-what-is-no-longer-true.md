# Pruning CLAUDE.md: what was no longer true, 2026-09-07

CLAUDE.md had grown to 1,939 lines by accretion. The pass that produced
`prune-what-is-no-longer-true` was governed by one rule: **keep the reasoning,
cut the redundancy.** A correction with its reasoning intact is not clutter —
this file's value is that it records *why* a wrong belief was wrong, and a
"RETRACTED" entry has already stopped a third session re-deriving a dead
hypothesis.

The net line change is small (1,939 → 1,968) and that is the honest result: the
file has very little literal redundancy left. A repeated-12-gram scan over the
whole document found exactly two repeated spans, and both are deliberate
cross-references. What the file *did* have was **stale claims stated as
current**, which is a different and worse problem than length.

## Claims that were false, and what replaced them

Each was checked against the code on `master` at `bfa14569`, not inferred.

| claim in CLAUDE.md | status | evidence |
|---|---|---|
| `flow_graph` has no reader | **false** | `throughput_at` and `node_at` called from `crates/planner/src/method/sustain.rs` |
| `flow_graph` never refreshes | **false** | `ensure_current` rebuilds on `EntityGraph::generation`; every public reader calls it |
| the rate is hard-coded `1/3.2` in `furnace_output` | **false** | no such function; `smelting_output` derives `product.amount * crafting_speed / recipe.energy`. The constant survives only as a *test expectation*, because vanilla iron really is `energy = 3.2` |
| `FactorioWorld` holds one surface, refuses the second (`SurfaceNotYetSeparable`) | **false** | globals moved to `Arc<GameGlobals>`; the error type is gone, replaced by the narrower `SurfaceGlobalsNotShared` |
| `world.dump` never calls `Planner::refresh_buffers` | **false** | `create_lua_world` builds a `BufferRefresher` and hands it to the binding |
| headless runs at 5x | **false** | `HEADLESS_SPEED := "10"` since 2026-09-07, with a measured table in the `justfile` |
| connect wait is bounded at 90 s | **false** | 300 s of *no progress*; the file already said so 60 lines away |
| `method::connect` has no caller | **false** | `connect_steps_with` from `method::sustain`, in the production registry |

Two shapes recur and are worth naming:

**A claim about absence rots fastest.** "No caller", "never read", "no
refresh" — five of the eight rows above are absence claims, and every one of
them was made true by a session that then did the work. An absence claim needs a
grep before it is quoted, always. It is also the *most useful* kind of claim
while it holds, which is why they keep getting written.

**A superseded claim left standing beside its correction is worse than either
alone.** The `flow_graph` entry had grown to 95 lines in which three paragraphs
described mechanisms in the present tense and a fourth paragraph said the three
above it were wrong. A reader who stops early acts on the wrong half. The entry
now leads with what is true, and keeps the falsified claims in a clearly labelled
tail — because the *shape* of those mistakes is the transferable part.

## What was cut, and why it was safe

- The three superseded `flow_graph` mechanism paragraphs, replaced by a
  "what this entry got wrong" tail that keeps every lesson they carried (a
  removed entity's node stays forever; a reused position keeps the old node;
  a stale answer looks exactly like a current one).
- The 90-second connect-wait line, which contradicted the corrected 300-second
  entry in the same document.
- The `--all-features` / tokio-console explanation duplicated between a code
  comment and the paragraph immediately below it; the comment is now a pointer.
- The `electronics`-trigger restatement in the burner-block entry, which
  repeated the entry directly above it; now a cross-reference.
- The Jan 2026 `config.ini` bug report, compressed from 20 lines to 11. Its
  line numbers (`instance_setup.rs:297-309`) had gone stale — the guard is now
  at `:1215` — and quoting a stale line number is worse than quoting none. The
  root cause and the durable rule are kept: **a guard that means "is this
  artefact present" must name the artefact, not its container.**

## Moved rather than cut

The rule that **a measurement an iteration cap or a wall clock can move is a
broken instrument** was stranded at the end of "Expected Behavior", a section
about client startup. It now opens "Measuring a run", where its three worked
failures sit beside the other measurement traps.

## Found wrong, not fixed

- **The offline baselines.** The file quotes green as 442 / 47,542 and
  451 / 48,829 from 2026-09-06. Both are presented as dated history under an
  explicit **"re-measure, do not quote from here"**, which is the correct
  instruction and was left prominent. Newer figures exist but taking them would
  have meant a build on a tree with a live peer session, so no number was
  changed. The section is self-defending: it tells you not to trust it.
- **`mcp__rust__lsp_*`** was named as the refactoring toolset. Whether those
  tools exist in a given session is a fact about that session's configuration,
  not about this repo, so the advice was generalised to "language-server-backed
  refactoring tools" rather than repointed at a specific server.
