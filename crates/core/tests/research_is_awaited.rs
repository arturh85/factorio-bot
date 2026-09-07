//! **Research is durative, so the dispatch that starts it must not report
//! success.**
//!
//! `LuaForce.add_research` puts a technology on the back of the research
//! queue. Finishing it takes labs, science packs and minutes. The Rust half
//! used to send that command and return the moment the reply came back, so
//! `Actuator::research` reported success at *queue* time and rung 7 of the
//! milestone ladder could report a technology researched that was not.
//!
//! These tests speak the RCON wire protocol to a fake server — the same
//! harness `rcon_oversized_reply.rs` uses — so the dispatch is real, the reply
//! is real, and "did the call come back yet" is observable. The completion is
//! delivered the way `OutputParser` delivers it: as an `ActionOutcome` in
//! `world.actions`, which is the mod's `action_completed` writeout after
//! parsing.
//!
//! What this cannot prove: that Factorio raises `on_research_finished` for
//! every technology it accepts. See
//! `docs/superpowers/notes/2026-09-02-awaited-research.md`.

use factorio_bot_core::factorio::rcon::{Dispatch, FactorioRcon, RconSettings};
use factorio_bot_core::factorio::ticks::ActionOutcome;
use factorio_bot_core::factorio::world::FactorioSurface;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The tick the fake server stamps the dispatch with.
const QUEUED_AT: u64 = 64738;
/// The tick the completion carries. Far from [`QUEUED_AT`], so a result that
/// merely echoes the dispatch tick cannot pass for an observed finish.
const FINISHED_AT: u64 = 91230;

/// How long to let the dispatch land and the waiter start polling before
/// asserting it has *not* returned. `sleep_for_action_result` polls every
/// 50 ms, so this is several polls' worth.
const SETTLE: Duration = Duration::from_millis(400);

/// A fake RCON server: accepts any password, answers each command with the
/// next body from `replies` (repeating the last once the queue runs dry), and
/// keeps every command body it was sent.
struct FakeServer {
    address: SocketAddr,
    commands: Arc<parking_lot::Mutex<Vec<String>>>,
}

impl FakeServer {
    fn commands(&self) -> Vec<String> {
        self.commands.lock().clone()
    }
}

async fn read_packet(socket: &mut TcpStream) -> Option<(i32, i32, String)> {
    let mut header = [0u8; 12];
    socket.read_exact(&mut header).await.ok()?;
    let length = i32::from_le_bytes(header[0..4].try_into().unwrap());
    let id = i32::from_le_bytes(header[4..8].try_into().unwrap());
    let packet_type = i32::from_le_bytes(header[8..12].try_into().unwrap());
    let mut body = vec![0u8; (length - 10).max(0) as usize];
    socket.read_exact(&mut body).await.ok()?;
    let mut terminator = [0u8; 2];
    socket.read_exact(&mut terminator).await.ok()?;
    Some((id, packet_type, String::from_utf8_lossy(&body).into_owned()))
}

async fn write_packet(socket: &mut TcpStream, id: i32, packet_type: i32, body: &str) {
    let mut out = Vec::with_capacity(body.len() + 14);
    out.extend_from_slice(&(10 + body.len() as i32).to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&packet_type.to_le_bytes());
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(&[0, 0]);
    socket.write_all(&out).await.expect("fake server writes");
}

async fn spawn_fake_server(replies: Vec<String>) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding a port");
    let address = listener.local_addr().expect("local_addr");
    let commands: Arc<parking_lot::Mutex<Vec<String>>> = Arc::new(parking_lot::Mutex::new(vec![]));
    let seen = commands.clone();
    let replies: Arc<parking_lot::Mutex<VecDeque<String>>> =
        Arc::new(parking_lot::Mutex::new(replies.into()));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let replies = replies.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                while let Some((id, packet_type, body)) = read_packet(&mut socket).await {
                    if packet_type == 3 {
                        // Auth: accept anything, a negative id would mean refusal.
                        write_packet(&mut socket, id, 2, "").await;
                        continue;
                    }
                    seen.lock().push(body);
                    let reply = {
                        let mut queue = replies.lock();
                        if queue.len() > 1 {
                            queue.pop_front().unwrap_or_default()
                        } else {
                            queue.front().cloned().unwrap_or_default()
                        }
                    };
                    // Factorio terminates every reply with a newline, and
                    // `FactorioRcon::send` strips exactly that one byte.
                    write_packet(&mut socket, id, 0, &format!("{reply}\n")).await;
                }
            });
        }
    });
    FakeServer { address, commands }
}

async fn connect_to(server: &FakeServer) -> Arc<FactorioRcon> {
    let settings = RconSettings::new(
        server.address.port(),
        "password",
        Some(server.address.ip().to_string()),
    );
    Arc::new(
        FactorioRcon::new(&settings, Arc::new(RwLock::new(true)))
            .await
            .expect("connecting to the fake server"),
    )
}

/// The action id the dispatch carried, read off the command the server was
/// actually sent. Reading it back rather than assuming `FactorioSurface`'s
/// counter is at 1 keeps the test honest about *which* id the mod was told to
/// bind the technology to — the whole point of the change.
fn action_id_of(command: &str) -> u32 {
    let args = command
        .split_once("'action_start_research', ")
        .unwrap_or_else(|| panic!("no research dispatch in: {command}"))
        .1;
    let id = args.split(',').next().expect("an action id argument");
    id.trim()
        .parse()
        .unwrap_or_else(|e| panic!("action id {id:?} in {command}: {e}"))
}

/// **The bug, stated as a test.**
///
/// The dispatch is answered, the technology is queued, and nothing has
/// finished. The call must still be waiting. It may only return once a
/// completion for *its own* action id arrives, and the ticks it reports are
/// then the queue tick and the finish tick — two different numbers, because
/// the action really did take time.
#[tokio::test]
async fn research_reports_success_only_when_the_game_finishes_it() {
    let server = spawn_fake_server(vec![format!("§tick§{QUEUED_AT}")]).await;
    let rcon = connect_to(&server).await;
    let world = Arc::new(FactorioSurface::new());

    let waiting = {
        let (rcon, world) = (rcon.clone(), world.clone());
        tokio::spawn(async move { rcon.research_timed(&world, "automation", 0).await })
    };

    tokio::time::sleep(SETTLE).await;
    assert!(
        !waiting.is_finished(),
        "a queued technology is not a researched one; the action must still \
         be outstanding"
    );

    let commands = server.commands();
    let dispatch = commands
        .iter()
        .find(|c| c.contains("action_start_research"))
        .unwrap_or_else(|| panic!("nothing dispatched a research action: {commands:?}"));
    assert!(
        dispatch.contains("'automation'"),
        "the dispatch names the technology, got {dispatch}"
    );
    let action_id = action_id_of(dispatch);

    // What `OutputParser` does when the mod's `action_completed` writeout
    // arrives on the server's stdout.
    world.globals.actions.insert(
        action_id,
        ActionOutcome {
            tick: FINISHED_AT,
            result: "ok".to_string(),
        },
    );

    let ticks = tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .expect("the completion has to wake the waiter")
        .expect("the waiting task")
        .expect("a completed research is a success");
    assert_eq!(
        ticks.dispatched,
        Some(QUEUED_AT),
        "the queue tick is what the game stamped on the dispatch"
    );
    assert_eq!(
        ticks.replied,
        Some(FINISHED_AT),
        "the finish tick is the game's own, not the dispatch tick echoed back"
    );
}

/// A completion for a *different* action must not release this one. The mod
/// joins by technology name and hands back the id it was given; if the Rust
/// half ever stopped keying on its own action id, an unrelated research
/// finishing would report this one done.
#[tokio::test]
async fn a_completion_for_another_action_does_not_release_this_one() {
    let server = spawn_fake_server(vec![format!("§tick§{QUEUED_AT}")]).await;
    let rcon = connect_to(&server).await;
    let world = Arc::new(FactorioSurface::new());

    let waiting = {
        let (rcon, world) = (rcon.clone(), world.clone());
        tokio::spawn(async move { rcon.research_timed(&world, "automation", 0).await })
    };
    tokio::time::sleep(SETTLE).await;

    let commands = server.commands();
    let dispatch = commands
        .iter()
        .find(|c| c.contains("action_start_research"))
        .expect("a research dispatch");
    let action_id = action_id_of(dispatch);

    world.globals.actions.insert(
        action_id.wrapping_add(1),
        ActionOutcome {
            tick: FINISHED_AT,
            result: "ok".to_string(),
        },
    );
    tokio::time::sleep(SETTLE).await;
    assert!(
        !waiting.is_finished(),
        "somebody else's completion is not this action's verdict"
    );
    waiting.abort();
}

/// A refusal still comes back in the reply body, still immediately, and still
/// as [`Dispatch::Refused`] — the game saw the command and said no, so nothing
/// is outstanding and there is nothing to wait for. Awaiting the completion
/// must not turn a refusal into a six-minute silence.
#[tokio::test]
async fn a_refused_research_fails_at_once_rather_than_waiting() {
    let server = spawn_fake_server(vec!["Error: no such technology: nonsuch".to_string()]).await;
    let rcon = connect_to(&server).await;
    let world = Arc::new(FactorioSurface::new());

    let failure = tokio::time::timeout(
        Duration::from_secs(5),
        rcon.research_timed(&world, "nonsuch", 0),
    )
    .await
    .expect("a refusal must not be waited out")
    .expect_err("the game refused this");
    assert_eq!(
        failure.dispatch,
        Dispatch::Refused,
        "the game answered, and the answer was no"
    );
    assert!(
        format!("{:?}", failure.error).contains("no such technology"),
        "the game's own words have to survive, got {:?}",
        failure.error
    );
}

/// The unawaited queue-only path is still there, still answers as soon as the
/// technology is queued, and is what the Lua binding and the REST endpoint
/// use. Kept as a *separate* call so that "queue this" and "research this"
/// cannot be confused for one another.
#[tokio::test]
async fn queueing_research_still_returns_at_once() {
    let server = spawn_fake_server(vec![format!("§tick§{QUEUED_AT}")]).await;
    let rcon = connect_to(&server).await;

    tokio::time::timeout(Duration::from_secs(5), rcon.add_research("automation"))
        .await
        .expect("queueing does not wait for the research")
        .expect("the game accepted the technology");
    let commands = server.commands();
    assert!(
        commands.iter().any(|c| c.contains("'add_research'")),
        "the queue-only path uses the queue-only mod entry, got {commands:?}"
    );
}
