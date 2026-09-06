use factorio_bot_core::blueprint::{BlueprintError, UndergroundHalf, decode};
use std::collections::BTreeMap;

/// Builds a real blueprint string (version byte, base64, zlib, JSON) around
/// the given JSON text for the `"blueprint"` object's body -- e.g.
/// `{"item":"blueprint","version":281474976710656,"entities":[...]}`.
///
/// This exists so the allowlist's REFUSAL path has coverage: the two fixture
/// blueprints never carry a disallowed key (that was checked independently),
/// so nothing in the brief's four tests proves `Unsupported` is ever
/// actually returned. A synthetic blueprint can carry exactly the key under
/// test, in isolation, in either direction (legal or not).
fn encode_blueprint(body_json: &str) -> String {
    use base64::Engine;
    use std::io::Write;
    let envelope = format!(r#"{{"blueprint":{body_json}}}"#);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(envelope.as_bytes())
        .expect("in-memory zlib write cannot fail");
    let compressed = encoder.finish().expect("in-memory zlib finish cannot fail");
    let encoded = base64::engine::general_purpose::STANDARD.encode(compressed);
    format!("0{encoded}")
}

fn counts(bp: &factorio_bot_core::blueprint::Blueprint) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for e in &bp.entities {
        *out.entry(e.name.clone()).or_insert(0) += 1;
    }
    out
}

#[test]
fn the_miner_line_decodes_to_its_37_entities() {
    let bp = decode(include_str!("blueprints/miner_line.txt").trim()).expect("decodes");
    assert_eq!(bp.entities.len(), 37);
    let c = counts(&bp);
    assert_eq!(c.get("electric-mining-drill"), Some(&13));
    assert_eq!(c.get("transport-belt"), Some(&21));
    assert_eq!(c.get("small-electric-pole"), Some(&3));
}

/// **The trap this test exists for.** These blueprints are Factorio 1.x
/// (`version` 281474976710656 == 1.0.0.0) and their directions are on the old
/// EIGHT-point scale, where 2 is east. Factorio 2.0 uses SIXTEEN points, where
/// east is 4 and 2 is a diagonal. `import_stack` migrates on import, so the mod
/// never had to care -- decoding here bypasses that entirely, and raw 1.x
/// directions would turn every belt and inserter a half-turn: a factory that
/// places 100% correctly and moves nothing.
#[test]
fn pre_two_point_zero_directions_are_doubled_onto_the_sixteen_point_scale() {
    let bp = decode(include_str!("blueprints/miner_line.txt").trim()).expect("decodes");
    assert_eq!(bp.version, 281474976710656, "fixture is a 1.x blueprint");
    assert!(
        bp.entities.iter().all(|e| e.direction % 4 == 0),
        "every migrated direction must be a cardinal on the 16-point scale: {:?}",
        bp.entities.iter().map(|e| e.direction).collect::<Vec<_>>()
    );
    assert!(
        bp.entities.iter().any(|e| e.direction == 4),
        "the fixture has east-facing entities, which must be 4 and not 2"
    );
}

#[test]
fn an_underground_belt_carries_which_half_it_is() {
    let bp = decode(include_str!("blueprints/furnace_line.txt").trim()).expect("decodes");
    let halves: Vec<Option<UndergroundHalf>> = bp
        .entities
        .iter()
        .filter(|e| e.name == "underground-belt")
        .map(|e| e.underground_half)
        .collect();
    assert_eq!(halves.len(), 2, "the furnace line has one pair");
    assert!(halves.contains(&Some(UndergroundHalf::Input)));
    assert!(halves.contains(&Some(UndergroundHalf::Output)));
}

#[test]
fn a_string_that_is_not_a_blueprint_is_refused_by_name() {
    assert!(matches!(
        decode("not a blueprint"),
        Err(factorio_bot_core::blueprint::BlueprintError::NotBase64)
            | Err(factorio_bot_core::blueprint::BlueprintError::NotDeflate)
    ));
}

/// **The rejection path itself, proven, not just the acceptance path.**
/// Neither fixture blueprint ever carries `filters`, so nothing above shows
/// `Unsupported` is ever actually returned -- refusing unsupported content
/// by name was the entire point of the allowlist ruling. This builds a
/// one-entity synthetic blueprint that carries `filters` (a real Factorio
/// key -- inserter and splitter item filters -- that the allowed five never
/// name) alongside otherwise-legal keys, and checks it is refused BY NAME,
/// not merely refused.
#[test]
fn an_entity_with_an_unsupported_key_is_refused_by_name() {
    let text = encode_blueprint(
        r#"{"item":"blueprint","version":281474976710656,"entities":[
            {"entity_number":1,"name":"transport-belt","position":{"x":0,"y":0},"direction":0,"filters":[]}
        ]}"#,
    );
    assert_eq!(
        decode(&text).unwrap_err(),
        BlueprintError::Unsupported("filters".into())
    );
}

/// The other direction of the same helper: an entity carrying only allowed
/// keys must decode cleanly, so a failure above is attributable to the
/// `filters` key and not to the synthetic-encoding helper itself being
/// broken.
#[test]
fn the_synthetic_helper_correctly_encodes_a_legal_blueprint() {
    let text = encode_blueprint(
        r#"{"item":"blueprint","version":281474976710656,"entities":[
            {"entity_number":1,"name":"transport-belt","position":{"x":0,"y":0},"direction":0}
        ]}"#,
    );
    let bp = decode(&text).expect("a blueprint carrying only allowed entity keys decodes");
    assert_eq!(bp.entities.len(), 1);
    assert_eq!(bp.entities[0].name, "transport-belt");
}

/// **The allowlist extends to the blueprint object's own keys, not just an
/// entity's.** Factorio 2.0 moved circuit wiring out of per-entity
/// `connections` into a top-level `wires` array on the blueprint object
/// itself; a train blueprint likewise carries a top-level `schedules`.
/// Neither `Envelope` nor `Body` used `deny_unknown_fields`, so before this
/// check a blueprint carrying `wires` decoded successfully with its wiring
/// silently dropped -- exactly the failure mode the ruling exists to
/// prevent, one level up from where it was first applied. `wires` is the
/// concrete case because it is the one that will actually arrive from a
/// real 2.0 export.
#[test]
fn a_blueprint_object_with_an_unsupported_top_level_key_is_refused_by_name() {
    let text = encode_blueprint(
        r#"{"item":"blueprint","version":281474976710656,"entities":[],"wires":[[1,1,2,1]]}"#,
    );
    assert_eq!(
        decode(&text).unwrap_err(),
        BlueprintError::Unsupported("wires".into())
    );
}

/// **The duplicated migration had already drifted.** `crates/core/src/blueprint.rs`
/// carried its own `VERSION_2_0` and `migrate_direction` alongside the
/// `BLUEPRINT_VERSION_2_0` / `blueprint_direction` pair that predates this
/// branch. The canonical one folds (`% 8` before doubling, `% 16` after);
/// the copy used `saturating_mul(2)` and folded nothing, so a 1.x blueprint
/// carrying a direction of 8 or more decoded to 16..=254 -- a number outside
/// `defines.direction` entirely, handed straight to `create_entity`. This
/// pins the fold in the only place a caller can see it: through `decode`.
#[test]
fn an_out_of_range_direction_is_folded_rather_than_doubled_past_the_scale() {
    // 1.0.0.0, so the 8-point scale: 9 folds to 1, which doubles to 2.
    let text = encode_blueprint(
        r#"{"item":"blueprint","version":281474976710656,"entities":[
             {"entity_number":1,"name":"transport-belt","position":{"x":0.5,"y":0.5},"direction":9}]}"#,
    );
    let bp = decode(&text).expect("decodes");
    assert_eq!(
        bp.entities[0].direction, 2,
        "a 1.x direction of 9 is 1 on the eight-point scale, so 2 on the sixteen-point one"
    );

    // 2.x, so the 16-point scale is already correct and only folds.
    let text = encode_blueprint(
        r#"{"item":"blueprint","version":562949953421312,"entities":[
             {"entity_number":1,"name":"transport-belt","position":{"x":0.5,"y":0.5},"direction":20}]}"#,
    );
    let bp = decode(&text).expect("decodes");
    assert_eq!(bp.entities[0].direction, 4, "20 folds to 4, not to 20");
}

/// **The nine-entity `MovingBlock` fixture (2026-09-06): a chest, a burner
/// inserter, five belts, a burner inserter, a chest.** This is the block the
/// "block that moves" work builds live -- pins its entity count, every
/// entity's name and offset, and every direction, so a hand-edited blueprint
/// string (or a future re-generation of it) cannot silently drift from the
/// geometry the design table names.
///
/// `version` is `562949953421312` == `BLUEPRINT_VERSION_2_0`
/// (`2u64 << 48`), so directions are read on the 16-point scale with no
/// doubling -- unlike `furnace_line.txt` / `miner_line.txt`, which are 1.x
/// fixtures and exist specifically to prove the doubling path. Both
/// inserters carry `direction = 12` (west, per
/// `blueprint_direction`/`Direction::West`), which the CLAUDE.md "Known
/// Issues" note establishes as PICKS UP FROM THE WEST -- i.e. moves items
/// west to east -- confirmed here by construction: the first inserter's
/// west neighbour is the source chest and its east neighbour is the belt
/// run; the second inserter's west neighbour is the last belt and its east
/// neighbour is the destination chest. Every belt is `direction = 4`
/// (east), which is the direction the whole line actually carries items.
#[test]
fn the_moving_block_decodes_to_its_nine_entities_with_every_direction_pinned() {
    let bp = decode(include_str!("blueprints/moving_block.txt").trim()).expect("decodes");
    assert_eq!(bp.version, 562949953421312, "fixture is a 2.0+ blueprint");
    assert_eq!(bp.entities.len(), 9);

    let c = counts(&bp);
    assert_eq!(c.get("iron-chest"), Some(&2));
    assert_eq!(c.get("burner-inserter"), Some(&2));
    assert_eq!(c.get("transport-belt"), Some(&5));

    // (name, x, y, direction), in the design table's own order.
    let expected: Vec<(&str, f64, f64, u8)> = vec![
        ("iron-chest", 0.5, 0.5, 0),
        ("burner-inserter", 1.5, 0.5, 12),
        ("transport-belt", 2.5, 0.5, 4),
        ("transport-belt", 3.5, 0.5, 4),
        ("transport-belt", 4.5, 0.5, 4),
        ("transport-belt", 5.5, 0.5, 4),
        ("transport-belt", 6.5, 0.5, 4),
        ("burner-inserter", 7.5, 0.5, 12),
        ("iron-chest", 8.5, 0.5, 0),
    ];
    assert_eq!(bp.entities.len(), expected.len());
    for (entity, (name, x, y, direction)) in bp.entities.iter().zip(expected.iter()) {
        assert_eq!(entity.name, *name);
        assert_eq!(entity.offset.x(), *x, "{name} x");
        assert_eq!(entity.offset.y(), *y, "{name} y");
        assert_eq!(
            entity.direction, *direction,
            "{name} @ ({x}, {y}) direction"
        );
        assert!(
            entity.underground_half.is_none(),
            "no underground belts here"
        );
    }
}

/// **A few kilobytes must not be allowed to become gigabytes.**
/// `POST /api/v1/scripts/execute` is unauthenticated and a script hands any
/// string it likes to `goal.built`, so the zlib stream reaching `decode` is
/// attacker-shaped. An unbounded `read_to_end` on the decoder is a remote
/// memory-exhaustion primitive that reports nothing until the allocator
/// gives up. This builds a genuine zlib bomb -- highly compressible zeroes,
/// a couple of hundred kilobytes on the wire -- and asserts the decoder
/// refuses it by name rather than materialising it.
#[test]
fn a_zlib_bomb_is_refused_by_size_rather_than_inflated() {
    use base64::Engine;
    use factorio_bot_core::blueprint::MAX_DECOMPRESSED_BYTES;
    use std::io::Write;

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    // One megabyte at a time so the test itself never holds the whole
    // expansion; the payload is not valid JSON, and must never be reached.
    let chunk = vec![b'0'; 1024 * 1024];
    for _ in 0..(MAX_DECOMPRESSED_BYTES / (1024 * 1024) + 2) {
        encoder.write_all(&chunk).expect("in-memory zlib write");
    }
    let compressed = encoder.finish().expect("in-memory zlib finish");
    assert!(
        compressed.len() < 1024 * 1024,
        "the point of the test is that a small input inflates hugely: {} bytes",
        compressed.len()
    );
    let text = format!(
        "0{}",
        base64::engine::general_purpose::STANDARD.encode(compressed)
    );

    assert_eq!(
        decode(&text).err(),
        Some(BlueprintError::TooLarge(MAX_DECOMPRESSED_BYTES)),
        "an over-sized expansion is refused by name, not decoded"
    );
}
