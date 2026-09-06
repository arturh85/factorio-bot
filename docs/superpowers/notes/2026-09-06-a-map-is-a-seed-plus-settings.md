# A map is a seed plus settings — and the repo shipped a default that changed the settings

2026-09-06, branch `a-map-is-a-seed-plus-settings`.

## The fact

A Factorio map is noise-generated from the seed **plus** the map-gen and map
settings: resource frequency, size and richness (water included — it is a
per-resource knob like any other), trees, cliffs, terrain scale. A seed
reproduces a map only *under fixed settings*, and a map-exchange string encodes
seed and settings together, which is why the string is the complete identity.

The owner's framing, which is the operative one here: *"its not that bad, a seed
also uniquely regenerates a map as long as we keep the default settings."* Every
run this project has made was on empty settings, i.e. Factorio's defaults for
the installed version. **The archived record is reproducible as it stands.** The
seed plus the game version is sufficient identity for it.

So the risk is not the record. It is anything that quietly breaks the "as long
as we keep the default settings" condition.

## What was on disk (checked, not assumed)

```
crates/core/src/data/AppSettings.toml     map_exchange_string = ">>>eNpj…<<<"   (719 chars)
crates/core/src/settings.rs   Default     map_exchange_string = ">>>eNpj…<<<"   (the same string)
~/.local/share/factorio-bot/AppSettings.toml        map_exchange_string = ""
~/.local/share/factorio-bot-dev/AppSettings.toml    map_exchange_string = ""
workspace/runs/*/provenance.json                    map_exchange_string: null   (20 of 20)
workspace/*/server/map-gen-seed.txt                 31337
```

The Rust `Default` matters more than the template: `AppSettings::load` starts
from `AppSettings::default()` and merges the file over it, so a machine with no
`AppSettings.toml` gets the compiled-in string, and a machine with one gets
whatever it says.

The string dates to the project's **first commit**, `0bb136c6` (21 Feb 2021) —
a Factorio 1.x era exchange string, carried through `82129e5d`, `90b5f9c3` and
`eb96a8a7` as pure code motion. Nothing in the git history reads as a
deliberate choice of *that map* for *this record*; it predates the record by
four and a half years. It is preserved in the history of `crates/core/src/settings.rs`
for anyone who wants it back.

## Was it actually applied? Yes — on two of the four start paths

Traced through `crates/core/src/process/instance_setup.rs`:

- `factorio-bot lua` and `factorio-bot start` take the string **only from
  `--map`** (`app/src-tauri/src/cli/lua.rs`, `cli/start.rs`). They never read
  `app_settings.factorio.map_exchange_string`. **Every number in the record came
  through these**, which is why every workspace lacks a
  `map-exchange-string.txt` and every `provenance.map_exchange_string` is null.
- The **REPL** `start` (`app/src-tauri/src/repl/factorio_control.rs`, via
  `config_fallback`) and the **REST API** `POST /api/v1/instance/start`
  (`crates/server/src/manage/instance.rs`) both fall back to the setting, and
  both map empty → `None`.

When it is `Some`, `setup_factorio_instance` starts a temporary server, calls
`rcon.parse_map_exchange_string` to write `map-gen-settings.json` and
`map-settings.json`, and passes **both** to `factorio --create` alongside
`--map-gen-seed`. That is a different map on the same seed. Not inert.

So: a new person who ran `just serve` and pressed start on a fresh machine would
have been silently switched off defaults, and every timing they took would read
as a regression against a record measured on defaults — with the seed matching,
which is exactly what makes it invisible.

**Fixed** by shipping empty in both places, with the reason written next to each.

## The connection worth keeping

`power.rs`'s water constants and the distance columns in CLAUDE.md's 16-seed
scan table are all tuned against **one settings profile**. Under defaults that
is fine and self-consistent. It only matters the day someone turns a knob — and
the shipped string was precisely a knob turned without telling anyone. The trap
and the settings-relativity of every tuned constant are the same fact seen
twice.

## The producer (secondary)

`parse_map_exchange_string` consumed a string; nothing produced one. The
counterpart is now in place, small:

- `mods/BotBridge/control.lua`: `rcon_map_exchange_string()` →
  `rcon.print(game.get_map_exchange_string())`, registered on the `botbridge`
  interface. The name is `get_map_exchange_string`, **not**
  `encode_map_exchange_string` — verified against
  `workspace/factorio-api-docs/runtime-api.json` (2.1.17, API 6). `LuaGameScript`
  gives "the settings that were used to create this map" (the identity wanted);
  `LuaSurface` gives one surface's *current* settings.
- `crates/core/src/factorio/rcon.rs`: `FactorioRcon::map_exchange_string()`,
  with the reply judged by a pure `parse_map_exchange_reply` so the shape is
  testable without a game.
- `crates/core/src/record/provenance.rs`:
  `choose_map_exchange_string(from_game, from_setup_file)` — live answer wins,
  file is the fallback, **every blank candidate is dropped**.
- `crates/scripting_lua/src/globals/record.rs` asks at run start, beside the
  seed, and warns rather than failing.

`None` keeps its single meaning, **"not captured"**. An RCON failure, an old mod
without the function, and a server this process did not start all land there,
and none of them is evidence about the map — the same asymmetry
`ResourceFingerprint` documents. An empty string is never stored.

This *records* the default-settings condition rather than establishing it: it
turns "we believe these were defaults" into "the run says so". Worth having, not
urgent. The resource fingerprint stays; it answers a different question
(charted resources, which grow as bots explore).

## Live confirmation is OUTSTANDING

No Factorio was run. What is proven is structural: the mod function exists and
is registered on the interface, the Rust side accepts what the game's documented
return type would look like and refuses everything else, and a failed capture
yields `None` and not `""`. **Nobody has yet seen a real
`game.get_map_exchange_string()` reply come back through this path.** The first
run that writes a non-null `provenance.map_exchange_string` is the confirmation;
until then, treat the producer as untested against the game.
