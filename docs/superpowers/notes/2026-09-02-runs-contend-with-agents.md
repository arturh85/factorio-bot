# A live run and a compiling agent do not fit on one machine

## What happened

Run 26 connected **zero of four** clients and ended in 3 minutes 44 seconds:

```
Timeout waiting for clients to connect (expected 4)
planning for 0 of 4 bot(s): the game has no player for [1, 2, 3, 4] ...
```

`--logs` was on for the first time, which is the only reason this is
answerable. The client logs show no crash, no error, no exception. They show
**loading**:

```
client2-log.txt   ... 70% / 75% / 80% / 85%
client1-log.txt   libpng warning: iCCP: ... (sprite decode, still going)
```

The clients were fine. They were simply not finished loading when the 90-second
connect wait expired.

## Why they were slow

At that moment:

```
loadavg 27.42 on 20 cores
```

A subagent was running `cargo test --workspace` — a full workspace compile plus
test binaries — while I launched four graphical Factorio clients that each have
to decode a sprite atlas. I did that. The run did not fail; **I starved it.**

## The rule

**Do not start a live run while an agent is building or testing.** Check
`/proc/loadavg` first; on this 20-core machine a load above roughly 8 means the
sprite load will not finish inside the connect wait.

This is not a tuning problem and raising the 90-second timeout would be the wrong
fix — it would convert a fast, clear failure into a slow, murky one, and the run
would still be competing for the CPU it needs to simulate.

## What this casts doubt on

Run 25's three clients "dying mid-run" at 12:43 was diagnosed here as a
first-class reliability problem. Agents were compiling then too. It may have been
the same starvation, seen from the middle of a run rather than the start — a
client that cannot keep up with the server gets dropped, and that would look
exactly like what was recorded.

I do not know which it was, and the earlier note should be read with that
doubt attached. The next run made on a quiet machine settles it. What is
certain is that the guard behaved correctly either way: refusing to plan
against players the world does not have is right whether they crashed or were
starved.

## The cheap fix that already paid for itself

`--logs` cost one flag and immediately turned "the clients died, cause unknown"
into "the clients were at 85% and we did not wait". It should have been on from
the first run. It is on permanently now.
