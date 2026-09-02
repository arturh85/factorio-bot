# `--host`: renders, gets a character, and has no RCON

A spike against the prerequisite
`docs/superpowers/specs/2026-09-02-server-camera-design.md` §2 gated step 2 on:

> whether `--host` accepts `--rcon-port` and starts RCON; whether `--host` is
> non-interactive; whether it auto-pauses without peers; whether the host's
> player gets a character and counts as a bot.

All five are now measured. **The headline is a negative: `--host` starts no
RCON at all.** Step 2 of the design — replacing `--start-server` with `--host`
so the simulating process is also the rendering one — is dead in that form.
Step 1 is untouched by any of this.

Everything below ran against a throwaway instance `workspace/hostprobe/`
(hardlinked binary, copied `saves/level.zip`, its own `config/config.ini`, its
own `mods/`). `workspace/server/` was never launched against and is unchanged.
The probe instance was deleted afterwards.

---

## 1. `--help` documents `--host`, and lists the RCON flags in a different section

```
$ nix develop -c workspace/server/bin/x64/factorio --help
...
 Running options:
...
      --host FILE               Start hosting a multiplayer game
      --host-interactive FILE   Open multiplayer server settings for hosting
                                the selecting save
      --start-server FILE       start a multiplayer server
...
 Server options:
      --port N                  network port to use
      --bind ADDRESS[:PORT]     IP address (and optionally port) to bind to
      --rcon-port N             Port to use for RCON
      --rcon-bind ADDRESS:PORT  IP address and port to use for RCON
      --rcon-password PASSWORD  Password for RCON
      --server-settings FILE    Path to file with server settings. See
                                data/server-settings.example.json
```

`--host` is under **Running options**; every RCON flag is under **Server
options**. The help says nothing about whether the two groups combine — cxxopts
groups are help formatting, not a constraint — so this answers only half the
question. It does confirm the spec's `strings`-derived reading was right about
what the options are.

**What it does not say, and what turned out to matter:** the parser *accepts*
`--rcon-port`, `--rcon-password`, `--rcon-bind` and `--port` next to `--host`
without an error, a warning, or a log line. It just ignores them. See §3.

## 2. It renders. A window exists and draws the game world.

```
$ nohup nix develop -c env DISPLAY=:0 SDL_VIDEODRIVER=x11 \
    workspace/hostprobe/bin/x64/factorio \
    --host workspace/hostprobe/saves/level.zip \
    --port 34199 --rcon-port 4399 --rcon-password probe \
    --server-settings workspace/hostprobe/server-settings.json \
    --config workspace/hostprobe/config/config.ini > host.log 2>&1 &
```

`host.log`, contrasted with `workspace/server-log.txt:10` (`Running in headless
mode`) from a `--start-server` run of the same binary:

```
   0.000 2026-09-02 19:09:51; Factorio 2.1.17 (build 87315, linux64, full, space-age)
   0.009 Environment: DISPLAY=:0 WAYLAND_DISPLAY=<unset> ...
   0.367 Video driver: x11
   0.367 Available displays: 1
   0.367  [0]: eDP-1 14" - {1, [0,0], 2880x1800, SDL_PIXELFORMAT_XRGB8888, 119.96 Hz}
   0.663 Initialised OpenGL:[1] AMD Radeon 880M Graphics (radeonsi, strix1, ACO, DRM 3.61, 6.12.96); driver: 4.6 (Core Profile) Mesa 26.1.5
  25.980 Sprites loaded
  26.235 Estimated VRAM usage for textures: 3012.20 MB (Atlases: 2804.64 MB, Textures: 207.56 MB)
  26.315 Factorio initialised
```

No `Running in headless mode` line anywhere in the file. Window discovery, the
same check that proved the clients render
(`2026-09-02-video-prerequisites-settled.md`):

```
$ nix develop -c env DISPLAY=:0 xdotool search --pid 3809647
8388663

$ nix develop -c env DISPLAY=:0 xwininfo -id 8388663
xwininfo: Window id: 0x800037 "Factorio: Space Age 2.1.17"
  Absolute upper-left X:  0
  Absolute upper-left Y:  0
  Width: 2880
  Height: 1800
```

Fullscreen, because `config.ini` says `[FullScreen: true]` and nothing in the
`--host` argument list overrides it. `ffmpeg -f x11grab -window_id 8388663`
captured the rendered game world (terrain, entities, the character's quickbar
and gun/ammo slots) — so this is a real render, not an allocated-but-blank
window.

**Sprite loading costs ~26 s**, matching the spec's estimate: `Factorio
initialised` at 26.315 s on the first run, 17.2–17.7 s on later runs with a warm
page cache, against ~3.7 s to `Hosting game` for the headless control in §3.

**Not measured, and it should not be assumed:** whether
`game.take_screenshot` actually writes files from this process. The API doc's
only stated restriction is "if Factorio is running headless, this function will
do nothing", and this process is demonstrably not headless — but the mod's
capture is started over RCON, and there is no RCON (§3), so the capture was
never triggered. This one is an inference, flagged as such.

## 3. RCON: no. Not with `--rcon-port`, not with `--rcon-bind`.

The `--host` process opens **two sockets in total**, both UDP game traffic:

```
$ # inode->port mapping for every socket fd of the --host pid
$ python3 ... /proc/<pid>/fd + /proc/net/{tcp,tcp6,udp,udp6}
socket fds: 10
udp  port 34197 state 07
udp6 port 34197 state 07

$ ss -ltn '( sport = :4399 )'
State  Recv-Q Send-Q  Local Address:Port  Peer Address:Port
      (no rows)

$ python3 rcon.py 4399 probe '/silent-command rcon.print("hi")'
ConnectionRefusedError: [Errno 111] Connection refused
```

No `RemoteCommandProcessor` line appears in the log at any point — not before
hosting, not after, not minutes later. `grep -n RemoteCommandProcessor host.log`
returns nothing.

Retried with the other documented form, `--rcon-bind ADDRESS:PORT`, in case
`--rcon-port` alone was the problem:

```
$ ... --host workspace/hostprobe/saves/level.zip \
      --rcon-bind 127.0.0.1:4399 --rcon-password probe ...
$ grep -n "RemoteCommandProcessor" host4.log     -> nothing
$ ss -ltn '( sport = :4399 )'                    -> no rows
socket fds: 10
udp  port 34197 state 07
udp6 port 34197 state 07
```

Identical. **`--host` starts no TCP listener whatsoever.**

### The control, which is what makes this conclusive

Same binary, same instance directory, same `config.ini`, same
`server-settings.json`, same save, same flags — only `--host` swapped for
`--start-server`:

```
$ ... --start-server workspace/hostprobe/saves/level.zip \
      --port 34199 --rcon-port 4399 --rcon-password probe ...

   0.037 Running in headless mode
   3.723 Hosting game at IP ADDR:({0.0.0.0:34199})
   3.724 Info RemoteCommandProcessor.cpp:119: Starting RCON interface at IP ADDR:({0.0.0.0:4399})

$ ss -ltn '( sport = :4399 )'
LISTEN 0  4096  0.0.0.0:4399  0.0.0.0:*

$ python3 rcon.py 4399 probe '/silent-command rcon.print("tick="..game.tick.." players="..#game.players)'
AUTH OK (id=1 type=2)
$ /silent-command rcon.print("tick="..game.tick.." players="..#game.players)
'tick=724 players=0\n'
```

So nothing about the probe instance, the config or the argument spelling is at
fault. RCON is a `--start-server` feature.

### `--host` also silently ignores `--port`

Same runs, same flag:

| mode | flag passed | log line |
| --- | --- | --- |
| `--host` | `--port 34199` | `Hosting game at IP ADDR:({0.0.0.0:34197})` |
| `--start-server` | `--port 34199` | `Hosting game at IP ADDR:({0.0.0.0:34199})` |

`--host` used the default 34197. This is the same silence as the RCON flags:
accepted, unmentioned, not applied. Anything built on `--host` must not assume
a passed port took effect.

## 4. The host gets a player **with a character** — it would be counted as a bot

At tick 0, before any peer connects:

```
  17.239 Hosting game at IP ADDR:({0.0.0.0:34197})
  17.239 Info ServerMultiplayerManager.cpp:809: updateTick(0) changing state from(CreatingGame) to(InGame)
  17.293 Info InputActionHandler.cpp:5062: UpdateTick (0) processed PlayerJoinGame peerID(0) playerIndex(0) mode(create)
```

`mode(create)` — a character was created. BotBridge's own stdout writeouts from
that same process confirm it is a full player, not a spectator:

```
§0§on_player_main_inventory_changed§{"player_id":1,"main_inventory":[{"name":"burner-mining-drill","quality":"normal","count":1},{"name":"stone-furnace","quality":"normal","count":1},{"name":"wood","quality":"normal","count":1}]}
§0§on_player_changed_distance§{"player_id":1,"build_distance":10,"reach_distance":10,"drop_item_distance":10,"item_pickup_distance":1,"loot_pickup_distance":2,...}
§0§on_player_changed_position§{"player_id":1,"position":{"y":0,"x":0}}
§4446§on_player_changed_position§{"player_id":1,"position":{"y":0,"x":-0.1484375}}
```

Player index 1, the standard starting inventory, a reach distance, a character
position. The screen capture shows the character's armour/gun/ammo panel and
the engineer sprite standing at the crash site.

`rcon_players()` (`mods/BotBridge/control.lua`) returns every player where
`player.connected and player.character`. The host is connected (it is peer 0)
and has a character, so **it would be returned by `rcon_players()` and counted
by `connected_player_count()`**, which is what drives the startup wait and what
the executor addresses bots by. The spec's stated risk is real and confirmed.

Contrast, from the headless control in §3 — same save, same mod:

```
$ /silent-command rcon.print("tick="..game.tick.." players="..#game.players)
'tick=724 players=0\n'
$ /silent-command remote.call('botbridge', 'players')
'{}\n'
$ grep -c PlayerJoinGame control.log
0
```

A headless `--start-server` has no player at all. A `--host` server has exactly
one, indistinguishable from a bot by every check this project makes.

## 5. It does not auto-pause. 59.7 UPS with zero peers.

Measured on a `--host` process with nothing connected to it, by sampling the
tick number in the mod's stdout writeouts 30 s apart:

```
ticks 14367 -> 16159 over 30.0s = 59.7 UPS
```

Ticks advanced at full speed for the whole ~4-minute run. `game.tick` reached
16 159 with no peer ever connecting.

**Two caveats, because this is weaker evidence than it looks.**

- This project's `server-settings.json` sets `"auto_pause": false`. So the
  measurement shows that `--host` *honours* `auto_pause: false`; it is **not**
  a test of what `--host` does with `auto_pause: true`. That was not probed.
- Even under `auto_pause: true` the question may be vacuous, because per §4 the
  host is itself a connected player — "no players are present" is never true on
  a `--host` server. Plausible, not measured.

## 6. Unasked-for finding: BotBridge crashes at tick 0 under `--host`

The first `--host` run that got past the dialog died immediately:

```
  52.101 Info InputActionHandler.cpp:5062: UpdateTick (0) processed PlayerJoinGame peerID(0) playerIndex(0) mode(create)
  52.111 Error MainLoop.cpp:1488: Exception at tick 0: The mod BotBridge (0.0.1) caused a non-recoverable error.
Error while running event BotBridge::on_player_joined_game (ID 52)
__BotBridge__/control.lua:2225: attempt to index upvalue 'client_local_data' (a nil value)
  52.112 Info ServerMultiplayerManager.cpp:809: updateTick(0) changing state from(InGame) to(Failed)
```

`client_local_data` is `local client_local_data = nil` at `control.lua:55` and
only becomes a table inside `on_tick` (`if (client_local_data == nil) then
client_local_data = {} ...`). Under `--start-server` the server ticks for a
while before any client joins, so it is always a table by the time
`on_player_joined_game` runs. Under `--host` the host's own player joins **at
tick 0**, before `on_tick` has ever fired, and the unguarded
`if client_local_data.whoami == "client1"` in `on_player_joined_game` throws.

The line is `mods/BotBridge/control.lua:2336` in the working tree as of writing
(it was `:2225` when the probe ran; the file is being edited by another agent
right now, and the bug is present in both revisions). A one-token nil guard
fixes it. **I did not touch it** — the probe used a private copy of the mod
under `workspace/hostprobe/mods/` with the guard applied, purely to get past
the crash; `mods/BotBridge/` and `workspace/mods/BotBridge/` are byte-identical
to what they were before this spike.

## 7. Unasked-for finding: `--host` is interactive *once*, then not

The first two `--host` launches stopped on a modal dialog and never hosted:

> **LAN username** — Please choose your LAN player name. `Username [____]`
> `Back` `OK`

No RCON, no UDP socket, no map load — it sits at the menu indefinitely. Copying
`workspace/server/player-data.json` into the instance did **not** help, because
its `service-username` is `""`.

After answering the dialog once (typed via `xdotool`), Factorio wrote the name
into `player-data.json` as `service-username`, and every later launch was fully
non-interactive:

```
$ python3 -c "import json; print(json.load(open('.../player-data.json'))['service-username'])"
amprobe

  17.734 Factorio initialised
  17.750 ... changing state from(Ready) to(PreparedToHostGame)
  17.783 Hosting game at IP ADDR:({0.0.0.0:34197})
  17.839 ... processed PlayerJoinGame peerID(0) playerIndex(0) mode(create)
```

50 ms from initialised to hosting, no input given. So: **`--host` is
non-interactive provided `player-data.json` carries a non-empty
`service-username`, and blocks forever on a GUI modal if it does not.** That is
a provisioning requirement nobody would have guessed, and it fails as a hang
rather than an error.

---

## Verdict

**Step 2 of the design — a graphical `--host` server — dies as specified.**

RCON is the only control channel this project has. `rcon.rs` is how the world is
discovered, how the planner gets prototypes and recipes, how the executor
dispatches every action, and how frame capture is started in the first place. A
server with no RCON is a server this codebase cannot talk to. There is no
workaround visible from here: both documented RCON flags were tried, the failure
is silent, and the control run proves the flags themselves are fine.

Two further findings would each independently have forced a redesign even if
RCON had worked:

- the host occupies a player slot with a character, so it is a bot as far as
  `rcon_players()`, `connected_player_count()` and the executor are concerned —
  the spec's "the thing that could sink the design", confirmed;
- `--host` needs a provisioned `service-username` or it hangs on a GUI modal,
  and it silently ignores `--port`, so the process is harder to place and
  address than `--start-server`.

**Step 1 — one renderer, one director camera — is unaffected and still stands.**
Nothing in it depends on where the renderer runs; it only requires that every
`take_screenshot` name the *same* `by_player`. The reshape that survives this
probe is: keep the headless `--start-server` (it has RCON, it has no player),
and designate one of the existing graphical *client* peers as the renderer. That
is step 1 exactly as written, with `options.renderer` naming a client instead of
the server. The `source: String` axis (`"server"` / `"client<N>"`) proposed in
§4.2 loses its `"server"` case and can stay a client id.

**Ambiguity left on the table, deliberately unresolved:**

- Whether `take_screenshot` writes files from a `--host` process (§2). Cannot be
  tested without RCON; the non-headless inference is strong but is an inference.
- Whether `--host` respects `auto_pause: true` (§5). Probably moot given §4, but
  not measured.
- `--host-interactive` was not probed at all.
- Whether a `--host` process is *cheaper or dearer* in UPS than the current
  arrangement. It ran at 59.7 UPS with no peers and no capture running, which
  says nothing about a four-bot run with a camera. The spec's §1.3 warning that
  the 53.6-UPS shortfall was never A/B-measured still applies to everything here.

One observation, not a conclusion: the working tree is currently being changed
by another agent in a direction that retires the per-camera screenshots in
favour of video capture. If that lands, the question this note answers may stop
mattering — but the four facts about `--host` (no RCON, gets a character,
ignores `--port`, needs a provisioned username) hold regardless of what replaces
the screenshots.
