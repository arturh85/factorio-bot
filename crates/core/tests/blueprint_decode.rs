use factorio_bot_core::blueprint::{UndergroundHalf, decode};
use std::collections::BTreeMap;

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
