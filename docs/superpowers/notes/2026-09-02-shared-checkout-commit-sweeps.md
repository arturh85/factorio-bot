# Explicit paths do not prevent a sweep — 2026-09-02

Three times now, one agent's uncommitted work has landed in another agent's
commit. Nothing was lost any of the three times; the content was correct and
present, under the wrong message. But the rule I had been enforcing does not
actually prevent it, and it is worth being precise about why.

## The rule, and what it protects

> `git add <paths>` protects the *add*, not the *commit*. Use
> `git commit -m "..." -- <explicit paths>`.

That is true, and it stops the first failure mode: a bare `git commit` sweeping
up whatever someone else happened to have staged.

## What it does not protect

Today's sweep was different. `e562847a` ("retire an ore tile the moment a mine
empties it") legitimately needed `crates/core/src/factorio/rcon.rs` —
`player_mine_timed` is the single choke point both the executor and the Lua
binding pass through, and wiring there is exactly why that fix could avoid
touching `crates/executor` at all. It committed that file with an explicit path.

The other agent also had uncommitted edits in `crates/core/src/factorio/rcon.rs`.

**An explicit path is still a path to the whole file.** Two agents editing one
file cannot be separated by naming it carefully; `git commit -- <file>` takes the
file's current contents, whoever wrote them.

## What I got wrong when dispatching

I assigned ownership by *crate* — "you hold `crates/executor` and
`control.lua`", "you hold `crates/core/src/graph` and `crates/core/src/record`".
Both agents obeyed. The collision happened in a file neither had been assigned,
which each reached for legitimately from its own side of the problem.

Assigning by crate leaves every file outside the named directories unowned, and
the interesting fixes are exactly the ones that reach into a shared choke point.
`rcon.rs` is a choke point by design — that is the property that made it the
right place for the tile-retirement fix.

## The rule that would have worked

Tell each agent: **any file outside your assigned set, report it before editing
it.** I gave that instruction narrowly, naming only the other agent's crate:

> If a fix genuinely requires touching `crates/executor/`, tell me instead of
> editing it.

Scoped to one directory, it did not cover the file that actually collided. The
instruction has to be about the boundary, not about a list of forbidden
neighbours.

## What this costs, and what it does not

It does not lose work — git had both versions, and the file was correct after.
What it costs is attribution: `git log --follow` on `rcon.rs` now points at a
commit message about ore tiles for a change about research. Anyone bisecting or
reading history for the research change will not find it there.

It also costs a red test in whoever runs next: for a while `cargo test
--workspace` failed in the shared checkout on files belonging to an agent that
had not committed yet. Both agents handled that correctly — each built a
detached worktree at HEAD to get a trustworthy green rather than assuming the
red was someone else's. That is the right move and it should be the default when
the tree is shared.
