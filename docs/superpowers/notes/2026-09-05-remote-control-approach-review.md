# How this project drives Factorio, against the field — a high-level review

2026-09-05. Requested by the owner while another agent runs speedruns. This
builds on `2026-09-02-prior-art-research.md` (still accurate on the external
projects; re-checked today) and reads what has landed in the three days since.
Nothing in the codebase was changed for this note.

## The three shapes that exist, and where we are

| Shape | Who | Control loop | Multi-bot | Honest play | Iteration cost |
|---|---|---|---|---|---|
| Whole run compiled into a mod, one step per tick | Factorio-TAS-Generator, gotyoke any% (0.18, dead) | in-mod, zero runtime cost | no | yes (declared budget: fixed rock yields) | re-author on every desync; 35k hand steps |
| Python REPL over RCON, headless, turn-based | Factorio Learning Environment 0.3 | `/sc` calls; durative walk mod-side, polled | messaging only; all published experiments single-agent | **no**: `move_to` teleports, placement instant, lab-play starts with stocked inventory | seconds per step: pause between steps, `game.speed` 10, no client |
| LLM decides every action over RCON | factorioctl, FactorioMCP, FactoMCP, factorio-ai-companion | one round trip per primitive; A* walk; `WaitFor*` polling | single player | varies; FactorioMCP is explicitly no-teleport | one LLM call per action |
| **Symbolic planner + executor + mod, graphical clients** | **this project** | goal → network → schedule; mod runs walks/mines/crafts and pushes settles over stdout | **yes, scheduled** | **yes, enforced** (no `cheat_*` in the executor, provenance marks cheats) | **20+ min wall per run at 1x** |

Nobody else is doing what the fourth row does. FLE's roadmap lists multi-agent;
its paper says every experiment is single-agent. The MCP wrappers are what this
project was strictly past in August. The TAS line is the only one that has
honestly launched a rocket, and it did so with a human authoring every step.

## What is genuinely ahead of the field (keep it)

- **Offline planning against a dumped world.** `plan` and `score-map` answer
  a planner question in four seconds with no game. FLE agents re-derive the
  world every step through RCON queries; the TAS line has no planner at all.
  This is the project's best asset and the one the speedrun agent should be
  living in.
- **Completion by event, not by poll.** The mod settles walks, mines, crafts
  and research through `writeout` on stdout and the executor awaits a
  `watch`. FLE polls `get_walking_queue_length` every 0.5 s; the MCP wrappers
  poll too. This is the "middle row" the prior-art note recommended, and it
  was already built.
- **Honesty as a system property.** No `cheat_*` reachable from the executor,
  `provenance.json` at run start, `--compare` refusing cheat-vs-honest, and the
  chunk-ingest finding (the model sees ground nobody walked to) disclosed
  rather than hidden. FLE teleports by default and calls it `move_to`.
- **Measurement discipline.** `just analyse`, `BatchProgress`, samples on the
  sampling session, savepoints per milestone. Nobody in the field records a
  run this well; FLE records rewards and stdout.

## Where the architecture is behind, and why it bites the speedrun work

**1. The world runs at 1x, with a graphical client per bot.** This is the
whole cost structure. A run is 20+ minutes of wall clock, so three days
produced 24 archived runs and a 3,200-line plan document analysing them. FLE
made the opposite choice in 0.3.0: no client, characters created server-side,
`game.tick_paused` between steps, `game.speed = 10` while a step runs. Their
throughput figure is 218 operations per second on a headless server.

Here, `Actuator::game_speed` is stubbed at `1.0` with a comment saying the
RCON read was never plumbed, and the mod refuses a walk unless
`player.connected and player.character` hold, so a bot *must* be a connected
client. The overnight note of 2026-09-02 flagged dropping the clients as "not
mine to decide" and it has sat there since. It is the one decision in this
review that only the owner can make, and it dominates everything else:
a 10x world with no client turns a 20-minute experiment into two.

What it costs: video is filmed from a client window, so a filmed run still
needs a client. The clean design is two modes, not a replacement: a headless
character mode for development and benchmarks at speed N, a client mode for
the filmed, measured runs. The prior-art note's own risk stands unverified:
nobody has yet run the one-afternoon prototype that creates a `character`
entity headless, walks it, calls `mine_entity` and reads the inventory back.

**2. The horizon is misnamed.** The owner's framing is "how far we can get in
Space Age". The furthest honest live state is red science at a rate and green
science `exhausted` at 200 steps. Automation at 6:11 on seed `31337`
(`run-1788582657-14978`, 2026-09-05; 7:04 on the bench that morning, 8:17 the
day before) is a real result and a real improvement. It is **not** a record
comparison: the human 6:12 was set on a map nobody here can regenerate, while
`31337` was chosen for having ore close to spawn, which flatters every timing.
The two numbers are within a second of each other and measure different maps —
quote them side by side only with that said. The automation chain is
identical in Space Age and vanilla; nothing Space Age-specific has been
touched. The human Space Age any% record is 3h17m. Between here and a rocket
sit belts as a routed primitive (FLE's `connect_entities`, its slowest and
most complex tool), oil, electric mining at scale, and a build method that
places dozens of entities per plan rather than ones and twos. No public agent
system has launched a rocket without a human-authored step list. The
achievable milestone for the next weeks is a rocket on Nauvis, and it should
be named as such so a "Space Age" number is not quoted against it.

**3. Roster utilisation is 17-22%.** Bot 1 does 88% of dispatches; the
executor gives each bot one action at a time. This is the speedrun agent's
current workstream (drills over hands, drains, cell siting) and the world
record replay note has the right lesson: the lever is count, not design.
There is no external reference for multi-bot scheduling to steal from; this
is original work and the offline loop is the right place to do it.

**4. Recovery is forward-only.** Recovery tiers in `recover.rs` and milestone
savepoints. FLE restores an entity-level snapshot and re-synthesises from the
pre-error state, worth +6% on their benchmark. Savepoints cover the coarse
case; a per-step entity snapshot is not worth building until runs are cheap.

## Recommendations, ranked by leverage

1. **Decide the client question, then prototype it in one afternoon.** Headless
   characters plus a speed and pause knob in the executor. Keep the client mode
   for filmed runs. Every other item gets cheaper once this lands.
2. **Shape `just bench` into a task suite.** It ran for the first time on
   2026-09-05: automation in 7:04 game time on a fresh seed-`31337` map
   (`run-1788565090-80288`, release, one plan, zero failures), so the seed is
   validated. Next, give it FLE's lab-play form:
   a fixed start state, a success predicate, an action budget and a holdout
   window measured from production statistics. `score-map` already produces the
   offline half.
3. **Rename the goal to "rocket on Nauvis, honest, four bots" and list the
   missing primitives**: routed belts and pipes with a dry-run materials check,
   power pole routing, oil, and a build path that places a cell's entities from
   a ghost layout with real materials rather than one `place` per action.
4. **Keep the honesty ledger and flip chunk-ingest** to charted-only once the
   milestones survive it, as already decided on 2026-09-04.
5. **Do not adopt**: an LLM in the per-action loop (the MCP wrappers), pixel
   observation, `replay.dat` verification (headless cannot record), or the
   compiled-TAS shape (no recovery).

## Sources checked today

- FLE code: <https://github.com/JackHopkins/factorio-learning-environment>,
  paper <https://arxiv.org/html/2503.09617v1>, 0.3.0 notes
  <https://jackhopkins.github.io/factorio-learning-environment/versions/0.3.0.html>
- factorioctl: <https://github.com/MarkMcCaskey/factorioctl>
- FactorioMCP: <https://github.com/sbarisic/FactorioMCP>; FactoMCP:
  <https://github.com/WidAmi/FactoMCP>; companion (chat bridge only so far):
  <https://github.com/lveillard/factorio-ai-companion>
- Factorio-TAS-Generator: <https://github.com/theis999/Factorio-TAS-Generator>;
  gotyoke any% TAS: <https://github.com/gotyoke/Factorio-AnyPct-TAS>
- In-repo: `2026-09-02-prior-art-research.md`, `2026-09-04-world-record-replays.md`,
  `plans/2026-09-03-closing-the-idle-gap.md`, `crates/executor/src/actuator.rs`,
  `mods/BotBridge/control.lua`
