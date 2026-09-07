//! **A plan's own entities must survive that plan's blueprint stamp.**
//!
//! `FactorioRcon::place_blueprint` used to clear its build area by mining
//! every non-character, non-resource entity inside it, unconditionally --
//! including under `only_ghosts = true`, where nothing is being built and
//! there is no footprint to clear. `ActionKind::StampGhosts` is its first
//! executor caller, and the pole run that powers a block is emitted in the
//! same expansion with no ordering against the stamp. Measured live twice on
//! seed 31337 (`docs/superpowers/notes/2026-09-07-the-stamp-mines-the-pole-run.md`):
//!
//! ```text
//! WARN mining entity in build area: small-electric-pole @ 3.5/-2.5
//! build: done=true failed=0 lost=0 pending=0
//! ```
//!
//! `failed = 0` because the placement had already succeeded -- mining is not a
//! failure of anything -- so the block goes dark with every counter green.
//!
//! The same call also swept the **wrong ground**: `blueprint_build_area`
//! answers the extent in the blueprint's own offset space, and the sweep
//! re-centred a same-sized rectangle on the anchor instead of translating it.
//! Measured with two markers that are each other's control: a chest 6.5 tiles
//! clear of the block was mined, and a chest inside the block's own footprint
//! survived.
//!
//! Two defects, two fixes, and they close different halves:
//!
//! * skipping the sweep under `only_ghosts` is what saves the pole -- the pole
//!   at (3.5, -2.5) is *inside* the correctly offset area too, so offsetting
//!   alone would still have mined it;
//! * offsetting the area is what makes a real (`only_ghosts = false`) build
//!   clear the ground it is about to occupy, and only that ground.
//!
//! The RCON half of this file talks the wire protocol to a fake server that
//! records every command, so what is asserted is what the game was actually
//! asked to do.

use factorio_bot_core::factorio::rcon::{FactorioRcon, RconSettings};
use factorio_bot_core::factorio::util::{
    blueprint_build_area, blueprint_build_area_at, entities_to_clear,
};
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioEntity, FactorioPlayer, Position, Rect};
use parking_lot::RwLock;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The blueprint under test. `miner_line.txt` decodes to 37 entities whose
/// prototypes the shared fixture world all carries, and -- the property that
/// matters here -- its own extent does **not** straddle its origin, which is
/// exactly the shape that makes a re-centred sweep miss.
const BLUEPRINT: &str = include_str!("blueprints/miner_line.txt");

/// Somewhere that is not the origin, so "translated by the anchor" and
/// "re-centred on the anchor" cannot coincide.
fn anchor() -> Position {
    Position::new(20.0, -8.0)
}

// ---------------------------------------------------------------------------
// A fake RCON server that records what it was asked
// ---------------------------------------------------------------------------

struct FakeServer {
    address: SocketAddr,
    commands: Arc<parking_lot::Mutex<Vec<String>>>,
}

impl FakeServer {
    fn commands(&self) -> Vec<String> {
        self.commands.lock().clone()
    }

    fn commands_naming(&self, needle: &str) -> Vec<String> {
        self.commands()
            .into_iter()
            .filter(|c| c.contains(needle))
            .collect()
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

/// Answers by command name rather than by turn, because the number of
/// commands is the thing under test and a queue would tie the fixture to it.
fn reply_for(command: &str) -> String {
    if command.contains("find_entities_filtered") {
        // The sweep's own query. Empty: what the sweep would have *done* with
        // a hit is covered by `entities_to_clear` below, which needs no game.
        "[]".to_string()
    } else {
        // `place_blueprint` answers a JSON array of what it stamped.
        "[]".to_string()
    }
}

async fn spawn_fake_server() -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding a port");
    let address = listener.local_addr().expect("local_addr");
    let commands: Arc<parking_lot::Mutex<Vec<String>>> =
        Arc::new(parking_lot::Mutex::new(Vec::new()));
    let recorded = commands.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            tokio::spawn(async move {
                while let Some((id, packet_type, body)) = read_packet(&mut socket).await {
                    if packet_type == 3 {
                        write_packet(&mut socket, id, 2, "").await;
                        continue;
                    }
                    recorded.lock().push(body.clone());
                    let reply = reply_for(&body);
                    write_packet(&mut socket, id, 0, &format!("{reply}\n")).await;
                }
            });
        }
    });
    FakeServer { address, commands }
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

/// The fixture world with one bot standing on the anchor and enough build
/// reach to place there, so nothing in `place_blueprint` walks first.
fn world_with_a_bot_on_the_anchor() -> Arc<FactorioSurface> {
    let world = fixture_world();
    world.globals.players.insert(
        1,
        FactorioPlayer {
            player_id: 1,
            position: anchor(),
            build_distance: 100,
            ..Default::default()
        },
    );
    Arc::new(world)
}

// ---------------------------------------------------------------------------
// Defect 1: the stamp mines the plan's own poles
// ---------------------------------------------------------------------------

/// **The property.** A ghost stamp asks the game to clear nothing, so a pole
/// the same plan placed a moment earlier is still standing afterwards.
///
/// Asserted as "the game was never asked": the sweep's query
/// (`find_entities_filtered`) is the only way `place_blueprint` learns what to
/// mine, and `player_mine` / `action_start_mining` is the only way it mines.
/// Neither may appear.
#[tokio::test]
async fn a_ghost_stamp_asks_the_game_to_mine_nothing() {
    let server = spawn_fake_server().await;
    let rcon = connect_to(&server).await;
    let world = world_with_a_bot_on_the_anchor();

    rcon.place_blueprint(
        1,
        BLUEPRINT.trim().to_string(),
        &anchor(),
        0,
        false,
        true, // only_ghosts
        Vec::new(),
        &world,
    )
    .await
    .expect("the fake server answers an empty stamp");

    assert!(
        server.commands_naming("find_entities_filtered").is_empty(),
        "a ghost stamp must not go looking for entities to clear, asked: {:?}",
        server.commands()
    );
    assert!(
        server.commands_naming("mining").is_empty() && server.commands_naming("mine").is_empty(),
        "a ghost stamp must mine nothing, asked: {:?}",
        server.commands()
    );
    // The non-accidental half: a call that never reached the game would pass
    // every assertion above.
    assert_eq!(
        server.commands_naming("place_blueprint").len(),
        1,
        "the stamp itself must still have been dispatched, asked: {:?}",
        server.commands()
    );
}

/// The control for the test above, at the level where it can be asked without
/// a game: **the pole is inside the area the sweep considers.** So its
/// survival is the `only_ghosts` skip doing its job, not the pole happening to
/// sit somewhere the sweep never looked.
///
/// (3.5, -2.5) and an anchor of (0, 0) are the live run's own numbers.
#[test]
fn the_pole_the_live_run_lost_is_inside_the_area_the_sweep_considers() {
    let world = fixture_world();
    let area = blueprint_build_area_at(
        world.globals.entity_prototypes.clone(),
        BLUEPRINT.trim(),
        &anchor(),
    );
    let pole = FactorioEntity {
        name: "small-electric-pole".to_string(),
        entity_type: "electric-pole".to_string(),
        position: area.center(),
        ..Default::default()
    };
    assert!(
        area.contains(&pole.position),
        "the fixture must place the marker inside the swept area for this to be a control"
    );
    let doomed = entities_to_clear(vec![pole.clone()], &area);
    assert_eq!(
        doomed.len(),
        1,
        "an ordinary entity standing inside the build area is what the sweep mines"
    );
    assert_eq!(doomed[0].position, pole.position);
}

/// And the sweep still spares what it always spared. A control in the other
/// direction: `entities_to_clear` returning nothing above would also satisfy
/// the "nothing was mined" assertion, so the exclusions have to be shown to be
/// exclusions rather than the whole answer being empty.
#[test]
fn the_sweep_spares_characters_and_resources_and_nothing_else() {
    let area = Rect::new(&Position::new(-5.0, -5.0), &Position::new(5.0, 5.0));
    let at = |name: &str, entity_type: &str| FactorioEntity {
        name: name.to_string(),
        entity_type: entity_type.to_string(),
        position: Position::new(0.0, 0.0),
        ..Default::default()
    };
    let entities = vec![
        at("character", "character"),
        at("iron-ore", "resource"),
        // The live sweep's other victim: marker A, 6.5 tiles clear of the
        // block. A chest rather than a pole so that this test and the pole
        // test above can fail independently of one another.
        at("iron-chest", "container"),
    ];
    let doomed = entities_to_clear(entities, &area);
    assert_eq!(
        doomed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["iron-chest"],
        "characters and resources are spared, everything else is mined"
    );
}

/// An entity the query returned but which lies outside the rectangle is not
/// mined -- `find_entities_filtered` answers by bounding box overlap, so it
/// hands back entities whose *position* is outside the area.
#[test]
fn the_sweep_judges_by_position_not_by_what_the_query_returned() {
    let area = Rect::new(&Position::new(-5.0, -5.0), &Position::new(5.0, 5.0));
    let outside = FactorioEntity {
        name: "small-electric-pole".to_string(),
        entity_type: "electric-pole".to_string(),
        position: Position::new(5.5, 0.0),
        ..Default::default()
    };
    assert!(entities_to_clear(vec![outside], &area).is_empty());
}

// ---------------------------------------------------------------------------
// Defect 2: the build area was computed in the wrong space
// ---------------------------------------------------------------------------

/// **The two-marker experiment, offline.** A marker clear of the block must be
/// outside the swept area and a marker inside the block's footprint must be
/// inside it. The live run got both backwards.
#[test]
fn the_swept_area_is_the_ground_the_block_will_occupy() {
    let world = fixture_world();
    let own_space = blueprint_build_area(world.globals.entity_prototypes.clone(), BLUEPRINT.trim());
    let swept = blueprint_build_area_at(
        world.globals.entity_prototypes.clone(),
        BLUEPRINT.trim(),
        &anchor(),
    );

    assert_eq!(
        swept.left_top,
        Position::new(
            own_space.left_top.x() + anchor().x(),
            own_space.left_top.y() + anchor().y()
        ),
        "the area is the blueprint's own extent translated by the anchor"
    );
    assert_eq!(
        swept.right_bottom,
        Position::new(
            own_space.right_bottom.x() + anchor().x(),
            own_space.right_bottom.y() + anchor().y()
        )
    );

    // Marker B: inside the block's own footprint. The re-centring bug left
    // this one standing, which is how the block failed to clear its own
    // ground.
    let inside = Position::new(
        (swept.left_top.x() + swept.right_bottom.x()) / 2.0,
        (swept.left_top.y() + swept.right_bottom.y()) / 2.0,
    );
    assert!(
        swept.contains(&inside),
        "the footprint's own middle is swept"
    );

    // Marker A: clear of the block on the side the re-centred rectangle used
    // to reach into.
    let clear = Position::new(swept.left_top.x() - 6.5, inside.y());
    assert!(
        !swept.contains(&clear),
        "ground the block will never occupy must not be swept"
    );

    // The non-accidental half: the OLD rectangle really did contain that
    // marker and really did miss the footprint's middle, so the assertions
    // above are about the fix and not about an arbitrary geometry.
    let old = Rect::new(
        &Position::new(
            anchor().x() - own_space.width() / 2.0,
            anchor().y() - own_space.height() / 2.0,
        ),
        &Position::new(
            anchor().x() + own_space.width() / 2.0,
            anchor().y() + own_space.height() / 2.0,
        ),
    );
    assert_ne!(old, swept, "the fixture anchor must expose the difference");
}

/// The sweep's query goes to the game as a rectangle, and that rectangle is
/// what decides which entities are candidates. Asserted on the wire, because
/// a correct area computed and then not sent would still be a mis-swept build.
#[tokio::test]
async fn a_real_build_asks_the_game_about_the_ground_it_will_occupy() {
    let server = spawn_fake_server().await;
    let rcon = connect_to(&server).await;
    let world = world_with_a_bot_on_the_anchor();

    rcon.place_blueprint(
        1,
        BLUEPRINT.trim().to_string(),
        &anchor(),
        0,
        false,
        false, // a real build: the sweep is legitimate here
        Vec::new(),
        &world,
    )
    .await
    .expect("the fake server answers an empty build");

    let queries = server.commands_naming("find_entities_filtered");
    assert_eq!(
        queries.len(),
        1,
        "a real build clears its footprint exactly once, asked: {:?}",
        server.commands()
    );

    let expected = blueprint_build_area_at(
        world.globals.entity_prototypes.clone(),
        BLUEPRINT.trim(),
        &anchor(),
    );
    let wanted = factorio_bot_core::factorio::util::rect_to_lua(&expected);
    assert!(
        queries[0].contains(&wanted),
        "the query must name the translated area {wanted}, asked: {}",
        queries[0]
    );
}
