# Howto: Setup Factorio Bot

Four things have to be true before the first run works. Each one fails the run
on its own, and each one has its own error.

## 1. Factorio 2.1

The bundled BotBridge mod declares which game it targets:

```json
{ "name": "BotBridge", "factorio_version": "2.1", ... }
```

(`mods/BotBridge/info.json`)

Factorio compares only the `major.minor` part. If the installed game is not
2.1.x, it refuses to load BotBridge — and without BotBridge there is no RCON
bridge, so nothing else in this project works. Factorio's own report of the
problem is a mod-load line buried in its output followed by a misleading
`failed to create factorio level`.

The program checks this before it launches the game
(`preflight_mod_factorio_version`, called from `setup_factorio_instance` in
`crates/core/src/process/instance_setup.rs`) and reports it as

> mod BotBridge targets Factorio 2.1 but the installed game is 2.0.x

with a help text naming both `info.json` files it read and the two ways out:
edit the mod's declared version, or install a matching archive. When either
manifest is missing or unparseable the check warns and lets the run continue,
rather than blocking on something it cannot judge.

## 2. A Factorio archive

Download Factorio as a `.zip` or `.tar.xz` — **not** the headless build, since
the clients need graphics — and point the settings at it. The program extracts
it once per instance into the workspace; it never downloads anything itself.

With no archive configured, every run stops here:

```
Error:   × failed to start Factorio
  ╰─▶ no factorio archive configured
```

## 3. The settings file

Settings live in the platform's local-data directory (`dirs_next`), in a
directory named after the binary — with a `-dev` suffix for debug builds, so a
development build and an installed one never share state:

| build   | Linux path                                             |
| ------- | ------------------------------------------------------ |
| release | `~/.local/share/factorio-bot/AppSettings.toml`          |
| debug   | `~/.local/share/factorio-bot-dev/AppSettings.toml`      |

Don't hand-write it. The file is **nested** — `[factorio]`, `[restapi]`,
`[gui]` — and a key written outside its section parses as valid TOML and then
merges into nothing, silently keeping its default. Let the program write it
instead:

```
$ factorio-bot config init
wrote /home/you/.local/share/factorio-bot-dev/AppSettings.toml
```

`config init` generates the file from the `AppSettings` struct itself, so it
cannot drift out of shape with what the program reads. It refuses to overwrite
an existing file unless you pass `--force`. What it writes:

```toml
[factorio]
client_count = 2
factorio_archive_path = ""
map_exchange_string = ">>>eNpjZICDBnsQycGSnJ+YA+EdcABhruT8goLUIt38olRkYc7ko ... <<<"
rcon_pass = "foobar"
rcon_port = 4321
recreate = false
restapi_port = 1234
seed = ""
workspace_path = ""

[restapi]
port = 7492

[gui]
enable_autostart = false
enable_restapi = false
```

An empty `workspace_path` means `<data dir>/workspace`. Fill in
`factorio_archive_path` and you are done.

`restapi.web_root` (the directory holding the built SPA) is optional and is
omitted from the generated file when unset; with no web root the server serves
the API only. See the commented entry in `crates/core/src/data/AppSettings.toml`.

To see what the program will actually use — file plus overrides, merged:

```
$ factorio-bot config show
[factorio]
client_count = 0
factorio_archive_path = "/home/you/factorio-bot/workspace/factorio-space-age_linux_2.1.17.tar.xz"
map_exchange_string = ""
rcon_pass = "foobar"
rcon_port = 4321
recreate = false
restapi_port = 1234
seed = ""
workspace_path = "/home/you/factorio-bot/workspace"

[restapi]
port = 7492

[gui]
enable_autostart = false
enable_restapi = false
```

## 4. Overrides, if you don't want to edit the file

Three options are global — they work before or after the subcommand name, and
on every subcommand:

```
      --settings <path>          read settings from this file instead of <data dir>/AppSettings.toml
      --workspace-path <path>    override factorio.workspace_path from the settings file
      --factorio-archive <path>  override factorio.factorio_archive_path from the settings file
```

Precedence, as printed under `--help`:

```
Settings precedence (highest wins):
  1. command line options (--workspace-path, --factorio-archive, ...)
  2. the settings file (--settings <path>, default: <data dir>/AppSettings.toml)
  3. built-in defaults
```

`config init` bakes any override you pass into the file it writes, so
`factorio-bot --factorio-archive /path/to/factorio.tar.xz config init` is a
one-line setup.

## Clients and bots are different numbers

`lua` takes both, and they mean different things:

```
  -c, --clients <clients>        number of graphical Factorio clients to start (0 = server only) [default: 1]
  -b, --bots <bots>              number of bots the script plans for [default: same as --clients]
```

A client is a graphical Factorio process: it needs a display, takes roughly
half a minute to load its sprites, and the server then waits up to 90 seconds
for it to connect. A bot is something the script plans for; bots that no client
brought are synthesised by the planner with a default inventory.

So `--clients 0 --bots N` is a fast planning loop — no window, no connect wait:

```
$ factorio-bot lua goal_smoke.lua --clients 0 --bots 4
Planning-only run: no graphical clients, 4 bot(s)
Starting Factorio to run script: goal_smoke.lua
start waiting
waiting finished
Factorio started, running script...
...
Script completed
```

The server still starts (the planner reads recipes and prototypes from the live
game), which took about 15 seconds here. Nothing moves in the world, because
there are no real players to move — use it to iterate on goal decomposition,
then re-run with `--clients N` to execute.

`start` has only `--clients`: it launches processes and never plans, so it has
no bot count to conflate with them.

## Verifying the flags yourself

Every option list above is copied from the binary. `--help` is generated from
the same code that parses the arguments, so it is the authoritative list:

```bash
factorio-bot --help
factorio-bot config --help
factorio-bot lua --help
factorio-bot start --help
factorio-bot serve --help
```
