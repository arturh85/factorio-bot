# Open decisions left by the dependency and toolchain sweep

Three things the upgrade work surfaced that are **decisions, not defects**.
Each is small, none is urgent, and none should be settled by whoever happens to
be touching the file next.

## 1. `paris` -> `tracing` is not a refactor

The brief called for folding `paris` into `tracing`. It stopped here, because
the two are not the same kind of thing.

`paris` carries **narration**: the loading spinner, colour markup living inside
message strings (`"<bright-blue>{}</>"`), `"serving http://... - press Ctrl-C
to stop"`. `tracing` carries **diagnostics**. Folding narration into tracing
turns a CLI's ordinary output into timestamped, levelled log lines on stderr --
a decision about what the tool looks like to use, not an internal cleanup.

The defensible shape, if it is wanted: convert the diagnostic call sites to
`tracing`, move the spinner to `indicatif` (already a dependency), keep
narration on stdout, drop `paris`. That is a real piece of work with a visible
result, and it wants an owner who is awake.

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
