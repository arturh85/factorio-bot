#!/usr/bin/env bash
# Does a worm shoot a bot that merely PASSES BY, or only one that lingers?
# And does a PEACEFUL worm shoot at all?
#
# The planner's threat standoff (2026-09-07) guards where a bot is sent to
# WORK. Whether it must also guard how the bot GETS there -- walk routing, belt
# routing -- depends entirely on the answer to those two questions, and nobody
# had measured either. This script is the apparatus, committed so the result is
# reproducible rather than a number in a note.
#
# It is a PROBE, not a measurement: it cheats a worm into existence and moves a
# character by teleport. No timing here is comparable to anything.
#
# USAGE -- needs a game already held open on the given settings file:
#
#   factorio-bot lua hold_long.lua --headless --bots 1 --seed 31337 --new \
#       --game-speed 1 --settings <toml>            # in another shell
#   [--peaceful]  for the peaceful half
#
#   scripts/threat_pass_probe.sh <settings.toml> [worm-prototype]
#
# Every window is bounded in GAME TICKS, never in polls or wall clock: this
# repo has published two wrong numbers by counting polls.
#
# STATE LIVES IN A REMOTE INTERFACE, NOT IN `storage`. A `/c` console command
# gets a fresh `storage` binding that does not survive to the next command
# (measured: a value written by one `/c` reads back `nil` in the next, which
# turned the first version of this script's poll loop into an infinite one).
# A `remote.add_interface` registered from the console persists for the
# session, and its closures keep their upvalues, so the arena's state rides
# there.
set -uo pipefail

SETTINGS="${1:?usage: threat_pass_probe.sh <settings.toml> [worm]}"
WORM="${2:-small-worm-turret}"
BIN="${BIN:-./target/debug/factorio-bot}"

# The arena is far from spawn so the run's own bot and the map's own nests
# cannot contribute. Chunks there are ungenerated on a fresh map, so they are
# forced into existence before anything is placed.
WX=200
WY=0
# Ticks a stationary trial stands still for. 300 ticks is 5 s of game time --
# far longer than a worm's ~1 s attack cooldown, so a worm that will shoot has
# shot several times.
STATIC_WINDOW=300
# A character's own walk speed, so the pass crosses the worm's reach in the
# time a real walk would take rather than in one frame.
SPEED=0.15
# Half-length of the pass, in tiles: it starts this far west of the worm and
# ends this far east, so every offset sweeps through the worm's whole reach.
HALF=60

rc() {
  "$BIN" rcon -s localhost --settings "$SETTINGS" -- "$1" 2>&1 |
    sed -n 's/.*rcon ⮞ //p'
}

echo "== threat pass probe: $WORM at ($WX,$WY) =="
echo "peaceful:        $(rc "/c rcon.print(tostring(game.surfaces[1].peaceful_mode))")"
echo "enemy hostile:   $(rc "/c rcon.print(tostring(game.forces['enemy'].is_enemy(game.forces['player'])))")"
# The game's own number, for comparison with `PlanState::threat_standoff`'s
# hard-coded `WORM_ATTACK_RANGE` fallback. It IS readable here; the mod simply
# does not send it, which is why that table answers 100% of the planner's calls.
echo "prototype range: $(rc "/c local p=prototypes.entity['$WORM'] rcon.print(tostring(p and p.attack_parameters and p.attack_parameters.range))")"

# ---------------------------------------------------------------------------
# The arena, and the interface every later call drives.
# ---------------------------------------------------------------------------
# ONE LINE, deliberately: a `/c` whose body contains a newline is rejected as
# `Unknown command "c`, which reads like a broken build rather than a broken
# quote. Lua needs no statement separators, so the whole arena fits.
rc "/c local s=game.surfaces[1] s.request_to_generate_chunks({x=$WX,y=$WY},5) s.force_generate_chunk_requests() for _,e in pairs(s.find_entities_filtered{area={{$((WX-100)),-100},{$((WX+100)),100}},force='enemy'}) do e.destroy() end local worm=s.create_entity{name='$WORM',position={$WX,$WY},force='enemy'} local minhp,t0,finished,hp0=0,0,true,0 local function chr() return s.find_entities_filtered{name='character'}[1] end local function reset(x,y) local c=chr() c.teleport({x,y}) c.health=c.max_health minhp=c.health hp0=c.health t0=game.tick finished=false return c end remote.remove_interface('probe') remote.add_interface('probe',{ static=function(d) reset($WX+d,$WY) script.on_event(defines.events.on_tick,function() local c=chr() if not (c and c.valid) then minhp=0 finished=true return end if c.health<minhp then minhp=c.health end if game.tick-t0>=$STATIC_WINDOW then finished=true end end) return true end, pass=function(off) reset($WX-$HALF,$WY+off) script.on_event(defines.events.on_tick,function() local c=chr() if not (c and c.valid) then minhp=0 finished=true return end if c.health<minhp then minhp=c.health end if c.position.x>=$WX+$HALF then finished=true return end c.teleport({c.position.x+$SPEED,c.position.y}) end) return true end, poll=function() if finished then script.on_event(defines.events.on_tick,nil) end return string.format('%s %.1f %.1f %d',tostring(finished),minhp,hp0-minhp,game.tick-t0) end, provoke=function(d) reset($WX+d,$WY) worm.damage(50,game.forces['player']) script.on_event(defines.events.on_tick,function() local c=chr() if not (c and c.valid) then minhp=0 finished=true return end if c.health<minhp then minhp=c.health end if game.tick-t0>=$STATIC_WINDOW then finished=true end end) return true end, worm_hp=function() return worm.valid and worm.health or -1 end, }) rcon.print('arena ready, worm hp '..tostring(worm.health))"

run_trial() { # $1 = interface fn, $2 = argument
  rc "/c rcon.print(tostring(remote.call('probe','$1',$2)))" >/dev/null
  while :; do
    out=$(rc "/c rcon.print(remote.call('probe','poll'))")
    case "$out" in true*) echo "${out#true }"; return;; esac
    [ -z "$out" ] && { echo "RCON LOST"; return; }
  done
}

echo
echo "-- stationary, $STATIC_WINDOW ticks, east of the worm --"
printf '%8s %8s %8s %8s\n' dist hp_min damage ticks
for D in 40 34 30 26 24 20 15 10 5; do
  printf '%8s %s\n' "$D" "$(run_trial static "$D")"
done

echo
echo "-- transient pass, x from $((WX-HALF)) to $((WX+HALF)) at $SPEED tiles/tick --"
printf '%8s %8s %8s %8s\n' offset hp_min damage ticks
for OFF in 40 34 30 26 24 20 15 10 5 0; do
  printf '%8s %s\n' "$OFF" "$(run_trial pass "$OFF")"
done

# --------------------------------------------------------------------------
# 3. THE CONTROL. A peaceful run in which nothing is ever shot proves the worm
#    is peaceful OR that the arena is broken, and those look identical. So:
#    stand at 10 tiles, hit the worm once, and watch. A worm that returns fire
#    is a working worm, and "peaceful worms do not shoot" then means what it
#    says rather than "no worm was ever there".
# --------------------------------------------------------------------------
echo
echo "-- provoked: struck once by the player force, bot standing at 10 tiles --"
printf '%8s %8s %8s %8s\n' dist hp_min damage ticks
printf '%8s %s\n' 10 "$(run_trial provoke 10)"

echo
echo "worm hp after:   $(rc "/c rcon.print(tostring(remote.call('probe','worm_hp')))")"
rc "/c script.on_event(defines.events.on_tick, nil) rcon.print('handler cleared')"
echo "== done =="
