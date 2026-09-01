# Open decisions left by the dependency and toolchain sweep

Three things the upgrade work surfaced that are **decisions, not defects**.
Each is small, none is urgent, and none should be settled by whoever happens to
be touching the file next.

## 1. `paris` -> `tracing` -- SETTLED, and not the way the brief assumed

The brief called for folding `paris` into `tracing`. It stopped here, because
the two are not the same kind of thing.

`paris` carries **narration**: the loading spinner, colour markup living inside
message strings (`"<bright-blue>{}</>"`), `"serving http://... - press Ctrl-C
to stop"`. `tracing` carries **diagnostics**. Folding narration into tracing
turns a CLI's ordinary output into timestamped, levelled log lines on stderr --
a decision about what the tool looks like to use, not an internal cleanup.

**Decision: diagnostics moved to `tracing`; `paris` stays for narration.**
Done in `c22dd17c` -- 46 call sites in `graph/entity_graph.rs`,
`graph/flow_graph.rs` and `factorio/rcon.rs`.

The removal was approved and then withdrawn on a fact that changed the
premise: **`paris` has zero dependencies.** It does colours, glyphs and
timestamps unaided. So removing it is not deleting a redundancy, it is trading
one zero-dependency unmaintained crate for `console` plus a local-time crate --
and `chrono` reaches the tree only via `reedline`, so it would be genuinely new
in the `--no-default-features --features cli,lua` builds used for iterating.

That is a different decision from the one that was approved, and approval does
not carry across a changed premise. Asked again, answered: keep it.

What was actually broken is fixed either way. The seven `tracing` call sites
emitted nothing at all until a subscriber was installed (`74269106`); the
diagnostics that moved are now reachable and `RUST_LOG`-filterable. `paris`
being quiet since 2023 was never the fault.

### The rule this leaves

**Narration is `paris` on stdout. Diagnostics are `tracing` on stderr.** A
line a user is meant to read while the tool runs -- progress, the spinner, an
address, "press Ctrl-C to stop" -- is narration. A line explaining something to
whoever debugs it later is a diagnostic.

`crates/core/src/lib.rs` carries `#[macro_use] pub extern crate paris`, so
paris' macros resolve with no import anywhere in that crate: **148 call sites
name no logging system at all.** `tracing` is imported explicitly at every site
so a converted line says which system it uses, and `lib.rs` now says in place
why it must never be given the same global.

Note that `crates/core/src/process/instance_setup.rs`'s `"Using mods directory
<path> (<why>)"` must stay a plain stdout line with its substring intact: two
tests assert it prints, and it is the only guard against the stale
`workspace/mods` trap.

## 2. `types.lua` lost declaration order

schemars 1.x uses `serde_json::Map` for schemas, so the `preserve_order`
feature now forwards to `serde_json/preserve_order` and switches that map from
BTreeMap to IndexMap **for every crate in the graph** -- reordering the keys of
every JSON object this workspace emits, `runs/<id>/events.jsonl` included.

It was dropped, and the cost is that `types.lua` fields are alphabetical rather
than in declaration order: `FactorioEntity` opens with `amount` rather than
`name`, `entity_type`, `position`. That ordering said something.

The trade was taken on asymmetry rather than on harm: a docs file's field order
against the serialisation of every crate in the graph. Recoverable if schemars
ever exposes ordering without the global flag.

## 3. A missing `/assets/` file answers 200, not 404

The SPA fallback serves `index.html` for any unmatched path. For a client-side
route that is correct. For a content-hashed asset it means a browser that asked
for JavaScript receives HTML and reports a **MIME error rather than a missing
file** -- an error that names the wrong problem.

Pre-existing on both `tower-http` 0.6 and 0.7; the bump did not cause it and
did not change it.

The narrow fix is to serve `/assets/` with no fallback, since everything under
it is content-hashed and a miss there is always a real miss. Hardcoding a list
of client routes instead would be a second source of truth against the router.
Small and well-founded, but it changes the SPA host's behaviour and should land
where someone can watch it.
