//! What happens when an RCON reply does not arrive whole.
//!
//! `FactorioRcon` builds its connections with `enable_factorio_quirks(true)`,
//! which makes the `rcon` crate read exactly **one** packet per command. A
//! reply the server splits across packets therefore arrives cut short, and the
//! remainder stays in the socket. Nothing in the `rcon` crate reports that:
//! `cmd` hands back the first packet's body as a plain `Ok`.
//!
//! Both consequences used to be silent. The parse error named serde rather
//! than the size, and the connection went back into the `bb8` pool still
//! holding the tail, so every later command on it read someone else's reply.
//!
//! These tests speak the RCON wire protocol to a fake server, so the split is
//! reproduced rather than described. The `world_snapshot` reply measured
//! 393 kB on a fresh map and the entity read 1.9 MB, both under the 16 MB a
//! probe carried in one packet — so this is about the day a bigger base or a
//! wider radius crosses that line, not about today.

use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const WHOLE_SNAPSHOT: &str =
    r#"{"entity_prototypes":[],"item_prototypes":[],"recipes":[],"forces":[]}"#;

/// A fake RCON server: accepts any password, and answers each command with the
/// next body from `replies` (repeating the last one once the queue runs dry).
struct FakeServer {
    address: SocketAddr,
    /// How many TCP connections have been accepted. The pool opening a second
    /// one is the observable proof that it threw the first away.
    connections: Arc<AtomicUsize>,
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
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = connections.clone();
    let replies: Arc<parking_lot::Mutex<VecDeque<String>>> =
        Arc::new(parking_lot::Mutex::new(replies.into()));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            accepted.fetch_add(1, Ordering::SeqCst);
            let replies = replies.clone();
            tokio::spawn(async move {
                while let Some((id, packet_type, _body)) = read_packet(&mut socket).await {
                    if packet_type == 3 {
                        // Auth: accept anything, a negative id would mean refusal.
                        write_packet(&mut socket, id, 2, "").await;
                        continue;
                    }
                    let body = {
                        let mut queue = replies.lock();
                        if queue.len() > 1 {
                            queue.pop_front().unwrap_or_default()
                        } else {
                            queue.front().cloned().unwrap_or_default()
                        }
                    };
                    // Factorio terminates every reply with a newline, and
                    // `FactorioRcon::send` strips exactly that one byte.
                    write_packet(&mut socket, id, 0, &format!("{body}\n")).await;
                }
            });
        }
    });
    FakeServer {
        address,
        connections,
    }
}

async fn connect_to(server: &FakeServer) -> FactorioRcon {
    let settings = RconSettings::new(
        server.address.port(),
        "password",
        Some(server.address.ip().to_string()),
    );
    FactorioRcon::new(&settings, Arc::new(RwLock::new(true)))
        .await
        .expect("connecting to the fake server")
}

/// A reply cut off mid-document still starts with `{`, so the "is this JSON at
/// all" guard waves it through and serde is left to complain about a column
/// number. The error has to carry the size, because the size is the cause.
#[tokio::test]
async fn a_truncated_reply_reports_its_length() {
    let truncated = WHOLE_SNAPSHOT[..40].to_string();
    let server = spawn_fake_server(vec![truncated.clone(), WHOLE_SNAPSHOT.to_string()]).await;
    let rcon = connect_to(&server).await;

    let Err(error) = rcon.world_snapshot().await else {
        panic!("a truncated reply must not parse");
    };
    let message = format!("{error:?}");
    assert!(
        message.contains("not a complete JSON document"),
        "the error must name truncation, got: {message}"
    );
    assert!(
        message.contains(&format!("{} bytes arrived", truncated.len())),
        "the error must carry the byte count, got: {message}"
    );
}

/// The desync half. A connection that read a short reply may be holding the
/// rest of it, so it must not go back into the pool: the next command would
/// read that tail instead of its own answer, and every command after it would
/// be one reply behind, for as long as the process lives.
#[tokio::test]
async fn a_connection_that_read_a_short_reply_leaves_the_pool() {
    let server = spawn_fake_server(vec![
        WHOLE_SNAPSHOT[..40].to_string(),
        WHOLE_SNAPSHOT.to_string(),
    ])
    .await;
    let rcon = connect_to(&server).await;

    rcon.world_snapshot()
        .await
        .expect_err("a truncated reply must not parse");
    rcon.world_snapshot()
        .await
        .expect("the second call gets a whole reply");

    assert_eq!(
        server.connections.load(Ordering::SeqCst),
        2,
        "the desynced connection was handed back out instead of being dropped"
    );
}

/// The control for the test above: without a desync the pool reuses its one
/// connection, so a connection count of 2 there means something was thrown
/// away rather than that this pool never reuses anything.
#[tokio::test]
async fn a_healthy_connection_stays_in_the_pool() {
    let server = spawn_fake_server(vec![WHOLE_SNAPSHOT.to_string()]).await;
    let rcon = connect_to(&server).await;

    rcon.world_snapshot().await.expect("first snapshot");
    rcon.world_snapshot().await.expect("second snapshot");

    assert_eq!(
        server.connections.load(Ordering::SeqCst),
        1,
        "a healthy connection should have been reused"
    );
}

/// A server whose BotBridge predates `world_snapshot` answers with the game's
/// own error text. That is a *complete* reply, just not the one asked for, so
/// the connection is fine and only the message needs to be honest.
#[tokio::test]
async fn an_old_mod_is_reported_by_its_own_words_and_costs_no_connection() {
    let server = spawn_fake_server(vec![
        "Cannot execute command. Error: No such function: world_snapshot".to_string(),
        WHOLE_SNAPSHOT.to_string(),
    ])
    .await;
    let rcon = connect_to(&server).await;

    let Err(error) = rcon.world_snapshot().await else {
        panic!("a non-JSON reply must not parse");
    };
    let message = format!("{error:?}");
    assert!(
        message.contains("No such function"),
        "the error must quote the server, got: {message}"
    );

    rcon.world_snapshot().await.expect("second snapshot");
    assert_eq!(
        server.connections.load(Ordering::SeqCst),
        1,
        "a complete reply, even an unwanted one, leaves the connection usable"
    );
}
