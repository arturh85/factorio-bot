//! **A craft is durative, and its refusals must not be waited out.**
//!
//! The Lua half of this is `botbridge_craft_action.rs`, which drives the real
//! `control.lua`. This is the Rust half: the dispatch is a real RCON round
//! trip against a fake server, so "has the call come back yet" is observable,
//! and the completion is delivered the way `OutputParser` delivers it — as an
//! `ActionOutcome` in `world.actions`.
//!
//! The two claims are the ones run `run-1788347034-00981` disproved for
//! crafting: a completion has to *arrive* for the call to return, and a
//! refusal has to return without one. Eleven crafts in that run returned
//! neither, at `ACTION_RESULT_DEADLINE` — 360 wall-clock seconds — apiece.
//!
//! The fake server is a copy of the one in `research_is_awaited.rs`, which is
//! itself a copy of `rcon_oversized_reply.rs`'s: each integration test binary
//! in this crate carries its own, and sharing it would make the harness a
//! dependency of tests that are meant to be readable on their own.

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
const QUEUED_AT: u64 = 50177;
/// The tick the completion carries. Far from [`QUEUED_AT`], so a result that
/// merely echoes the dispatch tick cannot pass for an observed finish.
const CRAFTED_AT: u64 = 50220;

/// How long to let the dispatch land and the waiter start polling before
/// asserting it has *not* returned. `sleep_for_action_result` polls every
/// 50 ms, so this is several polls' worth.
const SETTLE: Duration = Duration::from_millis(400);

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
/// actually sent, rather than assumed from `FactorioSurface`'s counter.
fn action_id_of(command: &str) -> u32 {
    let args = command
        .split_once("'action_start_crafting', ")
        .unwrap_or_else(|| panic!("no craft dispatch in: {command}"))
        .1;
    let id = args.split(',').next().expect("an action id argument");
    id.trim()
        .parse()
        .unwrap_or_else(|e| panic!("action id {id:?} in {command}: {e}"))
}

/// **The bug, stated as a test.** The dispatch is answered and the crafts are
/// queued; nothing has been crafted. The call must still be waiting, and may
/// only return once a completion for *its own* action id arrives.
#[tokio::test]
async fn a_craft_reports_success_only_when_the_game_finishes_it() {
    let server = spawn_fake_server(vec![format!("§tick§{QUEUED_AT}")]).await;
    let rcon = connect_to(&server).await;
    let world = Arc::new(FactorioSurface::new());

    let waiting = {
        let (rcon, world) = (rcon.clone(), world.clone());
        tokio::spawn(async move { rcon.player_craft_timed(&world, 1, "stone-furnace", 1).await })
    };

    tokio::time::sleep(SETTLE).await;
    assert!(
        !waiting.is_finished(),
        "a queued craft is not a finished one; the action must still be \
         outstanding"
    );

    let commands = server.commands();
    let dispatch = commands
        .iter()
        .find(|c| c.contains("action_start_crafting"))
        .unwrap_or_else(|| panic!("nothing dispatched a craft action: {commands:?}"));
    assert!(
        dispatch.contains("'stone-furnace'"),
        "the dispatch names the recipe, got {dispatch}"
    );
    let action_id = action_id_of(dispatch);

    // What `OutputParser` does when the mod's `action_completed` writeout
    // arrives on the server's stdout.
    world.actions.insert(
        action_id,
        ActionOutcome {
            tick: CRAFTED_AT,
            result: "ok".to_string(),
        },
    );

    let ticks = tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .expect("the completion has to wake the waiter")
        .expect("the waiting task")
        .expect("a completed craft is a success");
    assert_eq!(
        ticks.dispatched,
        Some(QUEUED_AT),
        "the queue tick is what the game stamped on the dispatch"
    );
    assert_eq!(
        ticks.replied,
        Some(CRAFTED_AT),
        "the finish tick is the game's own, not the dispatch tick echoed back"
    );
}

/// **A refusal fails at once.** The mod answers a craft it will not start in
/// the reply body and registers nothing, so there is no completion coming.
/// Awaiting one anyway is what turns a refusal into a six-minute silence.
#[tokio::test]
async fn a_refused_craft_fails_at_once_rather_than_waiting() {
    let server = spawn_fake_server(vec!["Error: no such recipe: nonsuch".to_string()]).await;
    let rcon = connect_to(&server).await;
    let world = Arc::new(FactorioSurface::new());

    let failure = tokio::time::timeout(
        Duration::from_secs(5),
        rcon.player_craft_timed(&world, 1, "nonsuch", 1),
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
        format!("{:?}", failure.error).contains("no such recipe"),
        "the game's own words have to survive, got {:?}",
        failure.error
    );
}

/// A completion for a *different* action must not release this one.
#[tokio::test]
async fn a_completion_for_another_action_does_not_release_this_craft() {
    let server = spawn_fake_server(vec![format!("§tick§{QUEUED_AT}")]).await;
    let rcon = connect_to(&server).await;
    let world = Arc::new(FactorioSurface::new());

    let waiting = {
        let (rcon, world) = (rcon.clone(), world.clone());
        tokio::spawn(async move { rcon.player_craft_timed(&world, 1, "stone-furnace", 1).await })
    };
    tokio::time::sleep(SETTLE).await;

    let commands = server.commands();
    let dispatch = commands
        .iter()
        .find(|c| c.contains("action_start_crafting"))
        .expect("a craft dispatch");
    let action_id = action_id_of(dispatch);

    world.actions.insert(
        action_id.wrapping_add(1),
        ActionOutcome {
            tick: CRAFTED_AT,
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
