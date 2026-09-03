//! When to stop waiting for Factorio clients to connect.
//!
//! Separated from the RCON loop in [`process_control`](super::process_control)
//! so the give-up decision can be tested without a game: everything here is a
//! function of a poll count and a timestamp.

use std::time::{Duration, Instant};

use crate::types::PlayerId;

/// How long the connect wait tolerates **no progress** before proceeding
/// without the clients that never showed up.
///
/// Ninety seconds was the number the wait had always used, until two runs
/// back to back on a loaded machine (2026-09-03 evening, ~17:23-17:26,
/// `free`: 24 of 30 GiB used, `uptime` load average 10.66) both stalled at
/// zero of four clients despite every client eventually connecting: client 1
/// reached `Factorio initialised` at 123.5 s and joined the game at 125.0 s,
/// past a 90 s-from-launch cutoff that had already restarted once at the
/// first poll and had nothing to restart it again before that. Raised to
/// 300 s on the owner's instruction after that pair of failures, not as a
/// blanket slowdown: every other run logged tonight connected within 1-3
/// polls, so this is headroom for a contended machine, not the new normal
/// wait.
///
/// It is unchanged for a run where nobody ever connects, so this is not a
/// licence to hang. What it measures: it used to be an absolute budget from
/// the first poll, which made the wait give up while clients were still
/// arriving — run 30 quit at ~90 s and its third client joined at 106.6 s
/// (`docs/superpowers/notes/2026-09-02-bot-one-idle.md`). Now it restarts
/// every time another client appears.
///
/// The total wait is still bounded, without a second timer: progress is a
/// high-water mark that can rise at most `expected` times, so the wait cannot
/// exceed `(expected + 1) * CONNECT_STALL_TIMEOUT`, and it only approaches
/// that when clients really are trickling in.
pub const CONNECT_STALL_TIMEOUT: Duration = Duration::from_secs(300);

/// What the caller should do after a poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectWait {
    /// Everyone expected is present. Stop waiting, happily.
    Satisfied,
    /// Keep polling.
    Waiting,
    /// Nothing has changed for [`CONNECT_STALL_TIMEOUT`]. Stop waiting and say
    /// who is missing.
    Stalled,
}

/// Tracks how many clients have shown up and when that last changed.
///
/// **What is counted is players with characters**, not processes that opened a
/// TCP connection: the count comes from the mod's `players` remote call, which
/// filters on `player.connected and player.character`. The two differ — a
/// client is connected but uncounted for the 750 ticks (12.5 s) of freeplay's
/// crash-site cutscene, during which `LuaPlayer::character` is nil. So a
/// "stall" here means *no additional player became controllable*, which is the
/// thing the run actually needs, and is a stricter bar than "no process
/// connected".
pub struct ConnectWatcher {
    expected: usize,
    /// High-water mark, not the last count. A client that drops and rejoins
    /// must not be able to extend the wait forever by flapping.
    best: usize,
    last_progress: Instant,
}

impl ConnectWatcher {
    pub fn new(expected: usize, started: Instant) -> Self {
        Self {
            expected,
            best: 0,
            last_progress: started,
        }
    }

    /// Feed one poll result, and get the verdict.
    pub fn observe(&mut self, count: usize, now: Instant) -> ConnectWait {
        if count > self.best {
            self.best = count;
            self.last_progress = now;
        }
        if self.best >= self.expected {
            return ConnectWait::Satisfied;
        }
        if now.duration_since(self.last_progress) > CONNECT_STALL_TIMEOUT {
            return ConnectWait::Stalled;
        }
        ConnectWait::Waiting
    }

    /// The most clients ever seen at once.
    pub fn best(&self) -> usize {
        self.best
    }
}

/// The client numbers that have no player among `present`.
///
/// **Slots, not process identities.** Factorio hands out player ids 1, 2, 3 …
/// in join order and the rest of this crate already treats a `PlayerId` as the
/// bot's identity (`whoami` names `client{n}` in the same order), so absence of
/// id *n* is reported as client *n* missing. If a client connected but is not
/// yet controllable — the cutscene above — it is named here, because it is
/// exactly as useful to the run as one that never launched.
pub fn missing_clients(expected: usize, present: &[PlayerId]) -> Vec<PlayerId> {
    (1..=expected)
        .filter_map(|n| PlayerId::try_from(n).ok())
        .filter(|n| !present.contains(n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run 30, to the second. The clients joined 94.2 s, 101.0 s, 106.6 s and
    /// 109.6 s after the server started; the wait began somewhere before the
    /// first of those and quit on an absolute 90-second budget while joins
    /// were still arriving 16 seconds apart. Three of four bots did nothing
    /// for the whole run.
    #[test]
    fn a_client_still_arriving_is_waited_for_past_the_old_budget() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut watcher = ConnectWatcher::new(4, t0);

        assert_eq!(watcher.observe(0, at(30)), ConnectWait::Waiting);
        assert_eq!(watcher.observe(1, at(70)), ConnectWait::Waiting);
        assert_eq!(
            watcher.observe(2, at(77)),
            ConnectWait::Waiting,
            "a second client arrived; nothing is stalled"
        );
        assert_eq!(
            watcher.observe(2, at(95)),
            ConnectWait::Waiting,
            "past the old 90-second budget, but the last client arrived 18 s ago"
        );
        assert_eq!(
            watcher.observe(3, at(83 + 20)),
            ConnectWait::Waiting,
            "run 30's third client, which the old wait was not there to see"
        );
        assert_eq!(
            watcher.observe(4, at(106)),
            ConnectWait::Satisfied,
            "all four, at a wall-clock time the old wait had already abandoned"
        );
    }

    /// The budget is unchanged when there is no progress: this is not a licence
    /// to hang on a client that will never come.
    #[test]
    fn a_client_that_never_connects_is_still_given_up_on() {
        let t0 = Instant::now();
        let mut watcher = ConnectWatcher::new(2, t0);
        assert_eq!(
            watcher.observe(0, t0 + CONNECT_STALL_TIMEOUT - Duration::from_secs(1)),
            ConnectWait::Waiting
        );
        assert_eq!(
            watcher.observe(0, t0 + CONNECT_STALL_TIMEOUT + Duration::from_secs(1)),
            ConnectWait::Stalled
        );
    }

    /// Progress is a high-water mark. A client that disconnects and rejoins
    /// returns the count to a value already seen, and that must not restart
    /// the clock — otherwise one flapping client keeps the run waiting for
    /// ever.
    #[test]
    fn a_rejoin_that_regains_lost_ground_is_not_progress() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut watcher = ConnectWatcher::new(4, t0);
        assert_eq!(watcher.observe(2, at(10)), ConnectWait::Waiting);
        assert_eq!(watcher.observe(1, at(50)), ConnectWait::Waiting);
        assert_eq!(
            watcher.observe(2, at(60)),
            ConnectWait::Waiting,
            "back to two, which is not two more than we ever had"
        );
        assert_eq!(
            watcher.observe(2, at(10) + CONNECT_STALL_TIMEOUT + Duration::from_secs(1)),
            ConnectWait::Stalled,
            "stalled CONNECT_STALL_TIMEOUT after the last real progress at 10 s, not after 60 s"
        );
        assert_eq!(watcher.best(), 2);
    }

    /// Satisfaction wins over a stall reported in the same poll, and a count
    /// above what was asked for still satisfies.
    #[test]
    fn arriving_late_all_at_once_still_satisfies() {
        let t0 = Instant::now();
        let mut watcher = ConnectWatcher::new(2, t0);
        assert_eq!(
            watcher.observe(3, t0 + Duration::from_secs(500)),
            ConnectWait::Satisfied
        );
    }

    #[test]
    fn the_clients_with_no_player_are_named() {
        assert_eq!(missing_clients(4, &[2, 3]), vec![1, 4]);
        assert_eq!(missing_clients(4, &[1, 2, 3, 4]), Vec::<PlayerId>::new());
        assert_eq!(missing_clients(2, &[]), vec![1, 2]);
        assert_eq!(
            missing_clients(2, &[1, 2, 7]),
            Vec::<PlayerId>::new(),
            "a player nobody asked for is not a missing client"
        );
    }
}
