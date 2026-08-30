use crate::jobs::JobRegistry;
use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::SharedFactorioInstance;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;

/// How many finished script runs the server remembers. Past this, the oldest
/// is evicted: a job keeps its whole transcript, so an uncapped history grows
/// without bound on a long-lived server.
const JOB_HISTORY_LIMIT: usize = 50;

#[derive(Clone)]
pub struct AppState {
    pub instance: SharedFactorioInstance,
    pub settings: SharedAppSettings,
    /// Where `PUT /api/v1/settings` persists. Production wiring points this
    /// at `factorio_bot_core::paths::settings_file()` via [`AppState::new`];
    /// tests that exercise persistence should build `AppState` directly with
    /// a path inside a `tempfile::TempDir` instead, so the suite never
    /// overwrites a real developer's settings file.
    pub settings_path: PathBuf,
    /// True between `POST /api/v1/instance/start` accepting and the spawned
    /// start finishing, win or lose. Starting Factorio takes 12-17 seconds
    /// normally and 8-10 minutes when the archive still has to be extracted,
    /// so the request cannot wait for it: it answers 202 and the client polls
    /// `GET /api/v1/instance`. `AtomicBool` rather than a lock because the
    /// slot is claimed with a single `compare_exchange` -- a read-then-write
    /// pair would let two simultaneous requests both start Factorio.
    pub starting: Arc<AtomicBool>,
    /// Why the last start attempt failed. Set by the spawned task, cleared
    /// when a new attempt is accepted. Without it a failed background start
    /// is invisible to the browser: `starting` simply goes false again.
    pub last_start_error: Arc<RwLock<Option<String>>>,
    /// Script executions, live and historical. Shared rather than cloned with
    /// the state: `AppState` is cloned per request, and every clone must see
    /// the same single execution slot.
    pub jobs: Arc<JobRegistry>,
}

impl AppState {
    /// Builds production state: `settings_path` is the real on-disk settings
    /// file. Test helpers that only exercise routes unrelated to persistence
    /// (i.e. everything but `put_settings`) should use this too, rather than
    /// repeating the struct literal, so a future field addition only touches
    /// this constructor.
    pub fn new(instance: SharedFactorioInstance, settings: SharedAppSettings) -> Self {
        AppState {
            instance,
            settings,
            settings_path: factorio_bot_core::paths::settings_file(),
            starting: Arc::new(AtomicBool::new(false)),
            last_start_error: Arc::new(RwLock::new(None)),
            jobs: JobRegistry::new(JOB_HISTORY_LIMIT),
        }
    }

    /// Claims the start slot, reporting whether this caller got it.
    ///
    /// One `compare_exchange` rather than a load followed by a store: two
    /// requests arriving together would both see `false` in the gap between a
    /// load and a store and both spawn a start, and the loser's Factorio would
    /// then die on the winner's ports and lock files.
    ///
    /// A method rather than three lines inside the handler so the atomicity is
    /// reachable from a test: `the_start_slot_is_claimed_by_exactly_one_caller`
    /// below hammers it from many threads at once and fails on the
    /// read-then-write version. Inline, the property could only be argued.
    pub fn claim_start_slot(&self) -> bool {
        self.starting
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Releases a slot taken by [`AppState::claim_start_slot`]. Called on every
    /// path the spawned start can take, including failure: a slot left set
    /// wedges the server into permanent `409`s with nothing running.
    pub fn release_start_slot(&self) {
        self.starting
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::app_settings::AppSettings;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    /// Pins the atomicity of the start claim, which the route tests cannot see:
    /// `a_second_start_while_one_is_in_flight_is_a_conflict` occupies the slot
    /// up front and passes just as happily against a read-then-write claim.
    ///
    /// Many threads are released onto the same free slot by a barrier, so they
    /// reach the claim within nanoseconds of each other -- which is exactly the
    /// width of the window a load-then-store pair leaves open. Repeated over
    /// many rounds so a single lucky interleaving cannot carry the mutant
    /// through: replacing `claim_start_slot`'s `compare_exchange` with
    /// `if starting.load() { false } else { starting.store(true); true }` fails
    /// this test within the first few rounds.
    #[test]
    fn the_start_slot_is_claimed_by_exactly_one_caller() {
        const THREADS: usize = 32;
        const ROUNDS: usize = 200;

        let state = AppState::new(
            Arc::new(RwLock::new(None)),
            AppSettings::default().into_shared(),
        );
        // `THREADS + 1`: the racers plus this thread, which inspects the result
        // of each round and frees the slot for the next one.
        let barrier = Arc::new(Barrier::new(THREADS + 1));
        let winners = Arc::new(AtomicUsize::new(0));

        let racers: Vec<_> = (0..THREADS)
            .map(|_| {
                let state = state.clone();
                let barrier = barrier.clone();
                let winners = winners.clone();
                std::thread::spawn(move || {
                    for _ in 0..ROUNDS {
                        barrier.wait();
                        if state.claim_start_slot() {
                            winners.fetch_add(1, Ordering::SeqCst);
                        }
                        barrier.wait();
                    }
                })
            })
            .collect();

        for round in 0..ROUNDS {
            barrier.wait();
            barrier.wait();
            let observed = winners.swap(0, Ordering::SeqCst);
            assert_eq!(
                observed, 1,
                "round {round}: {observed} callers claimed the start slot at once; \
                 the claim is not atomic and every one of them would spawn a Factorio"
            );
            state.release_start_slot();
        }

        for racer in racers {
            racer.join().expect("racer thread");
        }
    }
}
