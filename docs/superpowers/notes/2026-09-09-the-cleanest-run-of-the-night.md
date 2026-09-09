# The cleanest run of the whole night, and the first working lab

`run-1788954737-06011`, seed 31337, four headless bots at 10x, `--new`,
release binary built 13:51 from master `a605aad5`. **Nothing cheated.**

## Zero failures, one plan, no replan

```
action verdicts: {'success': 923}
milestone 1: satisfied after 1 iteration(s), best 1092 steps
```

**Every one of 923 actions succeeded. Nothing abandoned, nothing failed, no
replan needed at all.** The first perfectly clean execution of the whole
night, directly downstream of `a605aad5` (the furnace-offtake conflict fix):
the specific failure mode that thrashed `run-1788949638-11792` for 34 minutes
without ever building a machine — `produce` and `sustain` disagreeing about
who owned one furnace's output — is gone.

## Both assembling machines built, and a lab worked for the first time ever

```
assembling-machine   2 standing at 20:00
lab                  1 standing / 1 WORKING at 15:00
automation-science-pack   0 -> 0 -> 0 -> 15 -> 15   at 5/10/15/20/25
```

**This is the first time in this project's recorded history that a lab has
read `working`.** The owner's original bar for automation — *"no lab has ever
been inserter-fed"* — is now met at least once. Research completed:
`automation`.

## Claimed narrowly, as every result tonight has been

**Still a plateau, and still the same known cause.** Output stops at 15,
flat through 25:00 and to the end — `CELL_CHARGE_TICKS` still governs the
science cell's ingredient supply (the chest-free redesign that would remove
this has not landed; it was deliberately held back tonight to avoid
colliding with the furnace-offtake and splitter-tap work). This run does not
show a sustained factory. It shows, for the first time, every piece of the
chain working correctly at least once in the same run: sustain feeding
copper, a furnace's output correctly claimed by exactly one consumer,
assembling machines built and running, and a lab consuming the result.
