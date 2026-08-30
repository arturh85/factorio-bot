# Howto: Lua Scripting

***work in progress***

## The sandbox

Scripts do not run in a stock Lua interpreter. They are reachable from an
unauthenticated HTTP endpoint, so the standard library is an allow-list:
`table`, `string`, `math` and `coroutine` are available, `io`, `os` and
`package` are not loaded, `require`, `dofile` and `loadfile` are removed, and
`load` compiles source text only — it refuses binary chunks.

That leaves `include`, `file_read`, `file_write` and `world.draw` as the only
way to reach the filesystem, and all four are bounded to the scripts
directory. Paths are relative to the calling script, and a path that would
leave the scripts directory is refused rather than clamped.

The per-function rules — which paths must already exist, how symlinks are
treated — are stated on each function in the [Lua API
reference](https://arturh85.github.io/factorio-bot/lua/), which is generated
from the bindings themselves. They are deliberately not repeated here, because
a second copy is a copy that goes stale.
