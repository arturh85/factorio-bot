use crate::jobs::JobRegistry;
use factorio_bot_core::app_settings::SharedAppSettings;
use factorio_bot_core::process::process_control::{FactorioInstance, SharedFactorioInstance};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::RwLock;

/// How many finished script runs the server remembers. Past this, the oldest
/// is evicted: a job keeps its whole transcript, so an uncapped history grows
/// without bound on a long-lived server.
const JOB_HISTORY_LIMIT: usize = 50;

#[derive(Clone)]
pub struct AppState {
    pub instance: SharedFactorioInstance,
    pub settings: SharedAppSettings,
    /// Where `PUT /api/v1/settings` persists. Callers pass this to
    /// [`AppState::new`] rather than it being derived here: production
    /// wiring (`serve.rs`) hands over whatever path `settings` was actually
    /// loaded from -- the data-dir default, or an explicit `--settings` file
    /// -- and the two must always agree, or a server started against a named
    /// settings file would read it but silently overwrite the default one
    /// instead. Tests that exercise persistence should pass a path inside a
    /// `tempfile::TempDir`, so the suite never touches a real developer's
    /// settings file.
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
    /// How many stops have been requested. Bumped by
    /// `POST /api/v1/instance/stop`, read by an in-flight start just before it
    /// publishes: if the count moved, the user pressed Stop while Factorio was
    /// still coming up and the finished instance must be thrown away instead
    /// of published. A counter rather than a flag so a stop cannot be
    /// "consumed" by the wrong start, and read under the `instance` write lock
    /// rather than beside it -- see [`AppState::publish_started_instance`].
    pub stop_generation: Arc<AtomicU64>,
    /// Script executions, live and historical. Shared rather than cloned with
    /// the state: `AppState` is cloned per request, and every clone must see
    /// the same single execution slot.
    pub jobs: Arc<JobRegistry>,
}

impl AppState {
    /// Builds state from its three real inputs. `settings_path` is not
    /// derived here -- the caller must pass the exact path `settings` was
    /// loaded from (production: `factorio_bot_core::paths::settings_file()`,
    /// or the resolved `--settings` path when one was given); deriving it
    /// independently is how a server could once read one file and write
    /// another. Test helpers that only exercise routes unrelated to
    /// persistence (i.e. everything but `put_settings`) can pass
    /// `factorio_bot_core::paths::settings_file()` too -- production's value
    /// is never written to unless a test's `put_settings` calls actually
    /// exercise it. Use this rather than repeating the struct literal, so a
    /// future field addition only touches this constructor.
    pub fn new(
        instance: SharedFactorioInstance,
        settings: SharedAppSettings,
        settings_path: PathBuf,
    ) -> Self {
        AppState {
            instance,
            settings,
            settings_path,
            starting: Arc::new(AtomicBool::new(false)),
            last_start_error: Arc::new(RwLock::new(None)),
            stop_generation: Arc::new(AtomicU64::new(0)),
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

    /// The number of stops requested so far. A start captures this before it
    /// begins and hands it back to [`AppState::publish_started_instance`].
    pub fn stop_requests(&self) -> u64 {
        self.stop_generation
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Records a stop request.
    ///
    /// Takes the write **guard** rather than just `&self`, so it cannot be
    /// called without holding the instance write lock -- the same lock
    /// [`AppState::publish_started_instance`] reads the count under. That is
    /// what makes the two atomic with respect to each other: a check beside
    /// the lock instead of under it is the read-then-write shape
    /// [`AppState::claim_start_slot`] exists to avoid.
    ///
    /// The guard type is load-bearing. This used to take
    /// `&mut Option<FactorioInstance>`, which any caller can conjure from a
    /// local -- `let mut forged = None; state.note_stop_request(&mut forged);`
    /// compiled and bumped the counter with no lock held anywhere, so the
    /// docstring above was a hint rather than a fact. A `RwLockWriteGuard` can
    /// only come from `self.instance.write().await`. Pinned by the
    /// `compile_fail` example below.
    ///
    /// ```compile_fail
    /// use factorio_bot_core::app_settings::AppSettings;
    /// use factorio_bot_core::process::process_control::FactorioInstance;
    /// use factorio_bot_server::state::AppState;
    /// use std::sync::Arc;
    /// use tokio::sync::RwLock;
    ///
    /// let state = AppState::new(
    ///     Arc::new(RwLock::new(None)),
    ///     AppSettings::default().into_shared(),
    ///     factorio_bot_core::paths::settings_file(),
    /// );
    /// // No lock is held here, and there must be no way to say otherwise.
    /// let mut forged: Option<FactorioInstance> = None;
    /// state.note_stop_request(&mut forged);
    /// ```
    pub fn note_stop_request(
        &self,
        _under_instance_lock: &mut tokio::sync::RwLockWriteGuard<'_, Option<FactorioInstance>>,
    ) {
        self.stop_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Publishes a freshly started instance, unless a stop arrived while it
    /// was starting.
    ///
    /// Returns `None` when it published, or `Some(started)` -- handing the
    /// instance straight back -- when it did not, in which case the caller
    /// owns it and must stop it: the user pressed Stop, and a Factorio the
    /// state does not know about is one nothing can ever shut down.
    ///
    /// `started_after` is [`AppState::stop_requests`] as it was before the
    /// start began. The comparison happens while the write lock is held, so
    /// the two orderings are the only ones possible: either this publishes and
    /// a later stop takes the instance away, or the stop lands first and this
    /// sees the bumped count. There is no interleaving in which a stop
    /// observes no instance and then one appears behind it.
    pub async fn publish_started_instance(
        &self,
        started_after: u64,
        started: FactorioInstance,
    ) -> Option<FactorioInstance> {
        let mut instance = self.instance.write().await;
        if self.stop_requests() != started_after {
            return Some(started);
        }
        *instance = Some(started);
        None
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
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
            factorio_bot_core::paths::settings_file(),
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
