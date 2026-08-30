//! The registry of script execution jobs.
//!
//! A *job* is exactly one `run_lua` call. It is not finished when the script's
//! chunk returns — it is finished when `run_lua` returns, which waits for the
//! work the script started and did not await. Anything that completes a job
//! earlier than that would report success while bots are still moving.
//!
//! Execution is serialized: there is a single slot, and [`JobRegistry::try_start`]
//! either takes it or names the job that already holds it. There is no queue —
//! a second request is refused, not deferred, so the caller learns immediately
//! rather than blocking on an unbounded wait.
//!
//! The registry is per-process and dies with it: ids are a `u64` counter rather
//! than UUIDs, and history lives in memory, capped so that a long-lived server
//! does not accumulate every script's full transcript forever.

use factorio_bot_scripting::{OutputSink, Stream};
use miette::miette;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, watch};
use utoipa::ToSchema;

/// How many events a subscriber may fall behind before it is told it lagged.
///
/// A slow subscriber does not block the script: `broadcast` drops the oldest
/// events and hands the receiver a `RecvError::Lagged`, which the SSE handler
/// renders as a visible gap rather than a dropped connection.
const BROADCAST_CAPACITY: usize = 256;

/// Identifier of a job.
///
/// Serialized as a *string* even though it is a counter: a JSON number large
/// enough to lose precision in a browser would silently rename a job, and the
/// wire contract should not depend on the width of the counter behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema)]
#[schema(value_type = String, example = "1")]
pub struct JobId(pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for JobId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for JobId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map(JobId).map_err(D::Error::custom)
    }
}

impl std::str::FromStr for JobId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(JobId)
    }
}

/// Where a job is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Running,
    Succeeded,
    Failed,
}

/// What a subscriber sees while a job runs.
///
/// Not `Serialize`: [`Stream`] lives in `factorio-bot-scripting`, which is a
/// dependency-free leaf crate, so the SSE handler maps these to its own wire
/// shape rather than this type dictating one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobEvent {
    Output { stream: Stream, text: String },
    Finished { status: JobStatus },
}

/// One script run, live or historical.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Job {
    pub id: JobId,
    /// Path of the script that was run, or `None` for inline code from the
    /// editor, which has no path.
    pub script: Option<String>,
    pub status: JobStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub stdout: String,
    pub stderr: String,
    /// Set only for [`JobStatus::Failed`].
    pub error: Option<String>,
}

/// Milliseconds since the unix epoch, or `0` for a clock set before 1970.
///
/// `duration_since` returns a `Result` precisely because the clock can be
/// behind the epoch; a misconfigured clock must not take the server down.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[derive(Default)]
struct RegistryInner {
    next_id: u64,
    /// The single occupied slot, if any.
    running: Option<JobId>,
    /// Every job the registry still remembers, oldest first, with the running
    /// one — when there is one — always at the back.
    jobs: VecDeque<Job>,
    /// Live subscriptions, one sender per running job. Pruned on completion:
    /// a receiver created after the terminal event would never see it anyway,
    /// so keeping the sender would only leak.
    channels: HashMap<JobId, broadcast::Sender<JobEvent>>,
}

/// Registry of script execution jobs. Always held behind an `Arc` — a
/// [`JobHandle`] keeps a `Weak` back to it so that finishing a job can reach
/// the shared state.
pub struct JobRegistry {
    me: Weak<JobRegistry>,
    /// How many jobs are retained. At least 1, so that the running job is
    /// never evicted from under itself.
    history_limit: usize,
    /// Latched process-wide "we are shutting down" flag. Every SSE stream
    /// selects on it, which is what keeps axum's *unbounded* graceful drain
    /// from waiting on a stream for a job that never finishes -- see
    /// [`crate::webserver::start_with_state`]. A `watch` rather than a
    /// `oneshot` because there is one signal and an unknown number of open
    /// streams, and a receiver created *after* the signal fires must still
    /// see it (`wait_for` inspects the current value first) -- a stream
    /// opened during shutdown must not be born unbounded.
    shutdown: watch::Sender<bool>,
    inner: Mutex<RegistryInner>,
}

impl JobRegistry {
    pub fn new(history_limit: usize) -> Arc<Self> {
        Arc::new_cyclic(|me| JobRegistry {
            me: me.clone(),
            history_limit: history_limit.max(1),
            shutdown: watch::channel(false).0,
            inner: Mutex::new(RegistryInner::default()),
        })
    }

    /// Tells every open event stream to end.
    ///
    /// Latched and idempotent: `send_replace` cannot fail even with no
    /// receivers at all, which is the normal case (nobody has to be streaming
    /// for the server to shut down).
    pub fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    /// The signal [`JobRegistry::shutdown`] fires, for a stream to end on.
    pub fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// Every lock in this module goes through here.
    ///
    /// A poisoned mutex must degrade rather than abort: the release profile
    /// sets `panic = "abort"`, so a `.unwrap()` on a lock a panicking handler
    /// poisoned would kill the whole process from a remote request. The data
    /// behind the lock is a job list; a torn update to it is not worth a
    /// process kill.
    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the single execution slot, or reports who holds it.
    ///
    /// Both the check and the claim happen under one lock, so two concurrent
    /// requests cannot both see an empty slot.
    pub fn try_start(&self, script: Option<String>) -> Result<JobHandle, JobId> {
        let mut inner = self.lock();
        if let Some(running) = inner.running {
            return Err(running);
        }
        inner.next_id += 1;
        let id = JobId(inner.next_id);
        inner.running = Some(id);
        inner.jobs.push_back(Job {
            id,
            script,
            status: JobStatus::Running,
            started_at_ms: now_ms(),
            finished_at_ms: None,
            stdout: String::new(),
            stderr: String::new(),
            error: None,
        });
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        inner.channels.insert(id, sender);
        Self::trim(&mut inner, self.history_limit);
        Ok(JobHandle {
            registry: self.me.clone(),
            id,
            finished: AtomicBool::new(false),
        })
    }

    /// Snapshots a job *and* subscribes to it under one lock. The only way to
    /// watch a job.
    ///
    /// `None` means no such job -- and *only* that, which is the whole point.
    /// A bare `subscribe` returning the channel or nothing existed here until
    /// this replaced it, and its `None` conflated "unknown" with "already
    /// finished": the channel is pruned on completion, so an SSE handler that
    /// read that `None` as a 404 answered 404 for every job that finished
    /// before the client attached -- which is most of them, a fast script
    /// being over long before a browser can open a stream. The inner `Option`
    /// is the subscription: `Some` for a running job, `None` for a finished
    /// one, whose channel [`JobRegistry::complete`] removed.
    ///
    /// One lock, not `get` followed by `subscribe`, because the two orderings
    /// are both wrong: `get` then `subscribe` loses a line recorded in
    /// between (published before the subscription existed, appended after the
    /// snapshot was taken), and `subscribe` then `get` delivers it twice.
    pub fn attach(&self, id: JobId) -> Option<(Job, Option<broadcast::Receiver<JobEvent>>)> {
        let inner = self.lock();
        let job = inner.jobs.iter().find(|job| job.id == id).cloned()?;
        let receiver = inner.channels.get(&id).map(broadcast::Sender::subscribe);
        Some((job, receiver))
    }

    pub fn get(&self, id: JobId) -> Option<Job> {
        self.lock().jobs.iter().find(|job| job.id == id).cloned()
    }

    /// Every remembered job, newest first — the order a job list is read in.
    pub fn list(&self) -> Vec<Job> {
        self.lock().jobs.iter().rev().cloned().collect()
    }

    /// The job holding the slot, if any.
    pub fn running(&self) -> Option<JobId> {
        self.lock().running
    }

    /// Appends a line to the job's transcript and publishes it to subscribers,
    /// both under the same lock so that a subscriber's stream and the stored
    /// transcript cannot disagree about ordering.
    fn record_line(&self, id: JobId, stream: Stream, text: &str) {
        let mut inner = self.lock();
        if let Some(job) = inner.jobs.iter_mut().find(|job| job.id == id) {
            let buffer = match stream {
                Stream::Stdout => &mut job.stdout,
                Stream::Stderr => &mut job.stderr,
            };
            buffer.push_str(text);
            buffer.push('\n');
        }
        if let Some(sender) = inner.channels.get(&id) {
            // `send` fails only when nobody is listening, which is the normal
            // case for a script nobody opened a stream for.
            let _ = sender.send(JobEvent::Output {
                stream,
                text: text.to_owned(),
            });
        }
    }

    /// Records the outcome, frees the slot and closes the subscription.
    ///
    /// On success the returned transcript *replaces* what `record_line`
    /// accumulated rather than being appended to it: `run_lua` streams each
    /// line and also returns the whole transcript, so appending would store
    /// every line twice.
    fn complete(&self, id: JobId, outcome: miette::Result<(String, String)>) {
        let mut inner = self.lock();
        let status = if outcome.is_ok() {
            JobStatus::Succeeded
        } else {
            JobStatus::Failed
        };
        if let Some(job) = inner.jobs.iter_mut().find(|job| job.id == id) {
            if job.status != JobStatus::Running {
                // Already completed; do not overwrite the real outcome with a
                // later drop.
                return;
            }
            job.status = status;
            job.finished_at_ms = Some(now_ms());
            match outcome {
                Ok((stdout, stderr)) => {
                    job.stdout = stdout;
                    job.stderr = stderr;
                }
                Err(report) => job.error = Some(format!("{report}")),
            }
        }
        if inner.running == Some(id) {
            inner.running = None;
        }
        if let Some(sender) = inner.channels.remove(&id) {
            let _ = sender.send(JobEvent::Finished { status });
        }
        Self::trim(&mut inner, self.history_limit);
    }

    /// Drops the oldest jobs beyond the cap. The running job is at the back
    /// and `history_limit` is at least 1, so it is never the one evicted.
    fn trim(inner: &mut RegistryInner, history_limit: usize) {
        while inner.jobs.len() > history_limit {
            inner.jobs.pop_front();
        }
    }
}

/// The claim on the execution slot, held for the length of one `run_lua` call.
///
/// It is also the [`OutputSink`] that run receives, so a line printed by the
/// script reaches subscribers without any further plumbing.
#[derive(Debug)]
pub struct JobHandle {
    registry: Weak<JobRegistry>,
    id: JobId,
    finished: AtomicBool,
}

impl JobHandle {
    pub fn id(&self) -> JobId {
        self.id
    }

    /// Records the outcome of the run and frees the slot.
    /// Records the run's outcome and frees the execution slot.
    ///
    /// If every strong `Arc<JobRegistry>` has already been dropped, the `Weak`
    /// fails to upgrade and this silently does nothing. That is reachable only
    /// at process teardown -- `AppState` is built once and lives for the
    /// server's lifetime -- but a caller that spawns a detached task holding a
    /// `JobHandle` must not assume the outcome is guaranteed to land, because
    /// at shutdown it will not.
    pub fn finish(self, outcome: miette::Result<(String, String)>) {
        self.finished.store(true, Ordering::SeqCst);
        if let Some(registry) = self.registry.upgrade() {
            registry.complete(self.id, outcome);
        }
    }
}

impl OutputSink for JobHandle {
    fn line(&self, stream: Stream, text: &str) {
        if let Some(registry) = self.registry.upgrade() {
            registry.record_line(self.id, stream, text);
        }
    }
}

/// Frees the slot even when nobody called [`JobHandle::finish`].
///
/// This is the difference between a request that panics or returns early
/// costing one failed job and it wedging the server permanently: the slot is
/// single, so a handle dropped while still holding it makes every later
/// execution answer 409 for the lifetime of the process.
impl Drop for JobHandle {
    fn drop(&mut self) {
        if self.finished.load(Ordering::SeqCst) {
            return;
        }
        if let Some(registry) = self.registry.upgrade() {
            registry.complete(
                self.id,
                Err(miette!("script execution ended without reporting a result")),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::miette;

    #[test]
    fn a_second_start_is_refused_while_one_is_running() {
        let registry = JobRegistry::new(8);
        let first = registry
            .try_start(Some("a.lua".into()))
            .expect("first start");
        let refused = registry
            .try_start(Some("b.lua".into()))
            .expect_err("second refused");
        assert_eq!(
            refused,
            first.id(),
            "the refusal must name the job that holds the slot"
        );
    }

    #[test]
    fn the_slot_frees_when_a_job_finishes() {
        let registry = JobRegistry::new(8);
        let first = registry
            .try_start(Some("a.lua".into()))
            .expect("first start");
        first.finish(Ok(("out".into(), String::new())));
        registry
            .try_start(Some("b.lua".into()))
            .expect("slot is free again");
    }

    #[test]
    fn the_slot_frees_even_when_a_job_fails() {
        // The failure mode that would wedge the server permanently: an error
        // path that returns without releasing the slot leaves every later
        // request 409.
        let registry = JobRegistry::new(8);
        let first = registry
            .try_start(Some("a.lua".into()))
            .expect("first start");
        first.finish(Err(miette!("boom")));
        registry
            .try_start(Some("b.lua".into()))
            .expect("slot is free after a failure");
    }

    /// The case no happy-path test covers: a handler that panicked or returned
    /// early drops its handle without ever calling `finish`. Without `Drop`
    /// the slot stays taken forever and every later execution answers 409.
    #[test]
    fn a_dropped_handle_frees_the_slot_and_fails_its_job() {
        let registry = JobRegistry::new(8);
        let id = {
            let handle = registry.try_start(Some("a.lua".into())).expect("start");
            handle.id()
        };
        let job = registry.get(id).expect("job is retained");
        assert_eq!(
            job.status,
            JobStatus::Failed,
            "a job whose handle vanished cannot be left reported as running"
        );
        assert!(job.finished_at_ms.is_some());
        registry
            .try_start(Some("b.lua".into()))
            .expect("slot is free after a dropped handle");
    }

    /// Why a detached run has to capture a strong `Arc<JobRegistry>` of its
    /// own, and not merely its [`JobHandle`].
    ///
    /// The handle carries a `Weak` (the registry is built with
    /// `Arc::new_cyclic`, so a strong reference back would be a cycle), and
    /// [`JobHandle::finish`] silently does nothing when that upgrade fails --
    /// no panic, no log, the outcome simply goes nowhere. The server drops
    /// `AppState`, and with it the registry, together with the router at
    /// shutdown, while a spawned script keeps running.
    ///
    /// The loss itself is not observable through this API -- once the registry
    /// is gone there is nothing left to read the outcome out of, which is the
    /// point -- so what is asserted is the property that causes it: a live
    /// handle is not what keeps the registry alive.
    #[test]
    fn a_job_handle_does_not_keep_its_registry_alive() {
        let registry = JobRegistry::new(8);
        let weak = Arc::downgrade(&registry);
        let handle = registry.try_start(Some("a.lua".into())).expect("start");

        drop(registry);
        assert!(
            weak.upgrade().is_none(),
            "a JobHandle must not be what keeps its registry alive"
        );

        // And so this outcome goes nowhere at all. It must at least not panic:
        // under `panic = "abort"` it would take the process down at shutdown.
        handle.finish(Ok(("lost".into(), String::new())));
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn a_finished_job_records_its_output_and_status() {
        let registry = JobRegistry::new(8);
        let handle = registry.try_start(Some("a.lua".into())).expect("start");
        let id = handle.id();
        handle.finish(Ok(("hello".into(), String::new())));
        let job = registry.get(id).expect("job is retained");
        assert_eq!(job.status, JobStatus::Succeeded);
        assert_eq!(job.stdout, "hello");
        assert!(job.finished_at_ms.is_some());
    }

    #[test]
    fn a_failed_job_records_the_error_message() {
        let registry = JobRegistry::new(8);
        let handle = registry.try_start(None).expect("start");
        let id = handle.id();
        handle.finish(Err(miette!("script exploded")));
        let job = registry.get(id).expect("job is retained");
        assert_eq!(job.status, JobStatus::Failed);
        assert!(job
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("script exploded"));
    }

    #[test]
    fn subscribers_see_output_lines_and_the_terminal_event() {
        let registry = JobRegistry::new(8);
        let handle = registry.try_start(Some("a.lua".into())).expect("start");
        let (_job, rx) = registry.attach(handle.id()).expect("the job exists");
        let mut rx = rx.expect("a running job has a live channel");
        handle.line(Stream::Stdout, "first");
        handle.finish(Ok(("first".into(), String::new())));

        assert!(matches!(
            rx.try_recv().expect("an output event"),
            JobEvent::Output { stream: Stream::Stdout, ref text } if text == "first"
        ));
        assert!(matches!(
            rx.try_recv().expect("a terminal event"),
            JobEvent::Finished {
                status: JobStatus::Succeeded
            }
        ));
    }

    /// The boundary between the backlog an attaching subscriber is replayed
    /// and the live events it then receives. Overlapping them duplicates a
    /// line in the browser; leaving a gap loses one.
    #[test]
    fn attaching_to_a_running_job_yields_the_backlog_and_only_later_events() {
        let registry = JobRegistry::new(8);
        let handle = registry.try_start(Some("a.lua".into())).expect("start");
        handle.line(Stream::Stdout, "before");

        let (job, receiver) = registry.attach(handle.id()).expect("the job exists");
        let mut receiver = receiver.expect("a running job still has a live channel");
        assert_eq!(job.stdout, "before\n", "the backlog must carry what ran");

        handle.line(Stream::Stdout, "after");
        assert!(matches!(
            receiver.try_recv().expect("the later line"),
            JobEvent::Output { ref text, .. } if text == "after"
        ));
        assert!(
            receiver.try_recv().is_err(),
            "a line already in the backlog must not also arrive as an event"
        );
    }

    /// The two conditions a bare `subscribe` would have conflated into one
    /// `None`, kept apart: of "the job finished" and "there is no such job",
    /// only the second is a 404.
    #[test]
    fn attaching_separates_a_finished_job_from_an_unknown_one() {
        let registry = JobRegistry::new(8);
        let handle = registry.try_start(Some("a.lua".into())).expect("start");
        let id = handle.id();
        handle.finish(Ok(("done".into(), String::new())));

        let (job, receiver) = registry.attach(id).expect("a finished job is still known");
        assert_eq!(job.status, JobStatus::Succeeded);
        assert!(
            receiver.is_none(),
            "a finished job's channel is pruned, so there is nothing live to watch"
        );
        assert!(
            registry.attach(JobId(9999)).is_none(),
            "an unknown job is the only case that may be reported as absent"
        );
    }

    /// The shutdown flag is latched, so a stream that opens *after* the signal
    /// fired is bounded from birth rather than born unbounded.
    #[tokio::test]
    async fn the_shutdown_signal_is_latched_for_receivers_created_afterwards() {
        let registry = JobRegistry::new(8);
        registry.shutdown();
        let mut receiver = registry.shutdown_signal();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            receiver.wait_for(|fired| *fired),
        )
        .await
        .expect("a receiver created after shutdown must still see it")
        .expect("the sender outlives the registry's receivers");
    }

    #[test]
    fn history_is_capped_and_evicts_the_oldest() {
        let registry = JobRegistry::new(2);
        let mut ids = Vec::new();
        for name in ["a.lua", "b.lua", "c.lua"] {
            let handle = registry.try_start(Some(name.into())).expect("start");
            ids.push(handle.id());
            handle.finish(Ok((String::new(), String::new())));
        }
        assert!(
            registry.get(ids[0]).is_none(),
            "the oldest job should have been evicted"
        );
        assert!(
            registry.get(ids[2]).is_some(),
            "the newest job should be retained"
        );
        assert_eq!(registry.list().len(), 2);
        // `list()` is documented newest-first. Without this the ordering is
        // free to reverse under a refactor with the suite staying green.
        assert_eq!(
            registry.list().iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![ids[2], ids[1]],
            "list() must be newest-first"
        );
    }

    #[test]
    fn a_running_job_is_not_evicted_by_the_history_cap() {
        // Structurally guaranteed today -- insertion is push_back, eviction is
        // pop_front, and only one job runs at a time, so the running job is
        // always the newest element. Asserted anyway: that argument rests on
        // three separate properties, and a change to any one of them would
        // silently start evicting the job whose handle is still live.
        let registry = JobRegistry::new(1);
        let first = registry.try_start(Some("a.lua".into())).expect("start");
        let first_id = first.id();
        first.finish(Ok((String::new(), String::new())));

        let running = registry.try_start(Some("b.lua".into())).expect("start");
        assert!(
            registry.get(running.id()).is_some(),
            "the running job must survive an eviction that the cap forces"
        );
        assert!(
            registry.get(first_id).is_none(),
            "the finished job is the one that should have been evicted"
        );
    }

    #[test]
    fn a_job_id_is_json_a_string() {
        assert_eq!(serde_json::to_string(&JobId(7)).unwrap(), "\"7\"");
        assert_eq!(
            serde_json::from_str::<JobId>("\"7\"").unwrap(),
            JobId(7),
            "the wire form must round-trip"
        );
    }
}
