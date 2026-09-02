# Overnight report — 2026-09-02

Priority order you set: **reliability first, watchability second.** Both moved.

## Where the ladder stands

| rung | state |
|---|---|
| 1 gather iron ore x20 | **satisfied, 1 iteration** |
| 2 gather copper ore x20 | **satisfied, 1 iteration** |
| 3 smelt iron plates x10 | **satisfied** |
| 4 research automation | **plans 114 steps**, executes deep into them, not yet closed |
| 5-7 power, belts, oil | not reached |

At midnight this system crashed on rung 1 because a guard compared whole
inventories. It now dispatches 200+ actions per run at ~99% success across
mining, smelting, crafting, placing, fuelling, inserting and researching.

**I did not launch a rocket and was never going to.** I said so before you slept
and it remains true.

## Twenty defects fixed, every one found by a live run

The ones worth knowing about:

1. **Telemetry killed the game.** `character.mining_target` belongs to
   `LuaEntity`, not a character — my own code, and it only fired the first tick a
   bot really mined, so every planning-only test passed.
2. **Bots did not walk.** The "stuck" check measured leg duration, so every path
   leg over ~9.2 tiles was teleported by an unbounded jump, and a sibling branch
   reported the walk *successful* while leaving the bot walking forever. Every
   walk duration in every earlier record was fiction.
3. **A mine action reported success at half its count** — `on_mined_entity`
   completed *any* bot's task matching the entity, so two bots on one tile
   decremented each other. Fixing it made rungs 1-3 twelve times faster.
4. **A forest read as open ground.** `is_area_clear` consulted the tree holding
   factory entities; trees and cliffs live in another whose only reader was the
   debug drawing code. 10,510 tree records arrived every run and nobody asked.
5. **Machine time was measured in game ticks and waited for in wall-clock
   seconds.** Equal only at 60 UPS, which our server does not sustain — because
   we screenshot six 1920x1080 cameras every five seconds.

## A tension you should decide, not me

**The watchability feature is degrading the reliability one.** Frame capture is
why the server runs at ~53 UPS instead of 60, which is what broke the smelt
timing. Fewer cameras, lower resolution or a longer interval each buy back
server speed at the cost of the video.

## Watchability: the record now diagnoses failures by itself

Run 13's placement failure was diagnosed **from the record alone** — the target
tile from `events.jsonl`, the model ruled out by `map.jsonl`'s keyframe
agreement, and the cause named by `samples.jsonl`: a bot parked motionless
inside the furnace's footprint. No code reading required.

Failures went from an 18-minute silent hang to
`expected coal at (-37.5/5.5), found crash-site-spaceship-wreck-medium-3`.

Both deep runs render at `http://127.0.0.1:7500/#/runs`:

| run | frames | samples | map | events |
|---|---|---|---|---|
| `run-1788320177-77989` | 1217 | 1218 | 20 | 304 |
| `run-1788319014-01846` | 260 | 313 | 4 | 93 |

`skipped: 0` on every stream.

## Open, ranked

1. **Rung 4's placement refusal** — third distinct cause behind one message,
   under diagnosis now.
2. **Nothing retires a mined-out tile**, so exhausted ore is offered forever.
3. **A refusal is discarded** — the replanner re-chose the same refused tile
   twice.
4. **`Researched` sizes its whole bill against one bot**; option 3 from the
   share-sizing spec (a real multi-bot decomposition) remains the honest fix.
5. **`MapKind::Removed` has no writer**, documented as reserved.
6. **`wall_ms` in `events.jsonl` is badly wrong** (33780 to 738866 while the
   tick moves 10).

## Two process notes

- `git reset --hard` and `git stash` both destroyed or endangered another
  agent's work in this shared checkout. Use a worktree.
- A spurious STALE warning about `workspace/plans` — a directory with no
  production reader — trained me past the real one about `workspace/scripts`,
  which made a "teleports: 0" I reported to you meaningless.
