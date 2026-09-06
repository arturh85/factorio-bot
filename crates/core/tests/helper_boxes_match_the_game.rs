//! `FactorioEntity::new_*` helpers must carry the collision box the game does.
//!
//! **Asserted against a captured prototype snapshot, never against numbers
//! typed here.** A test that hardcodes the size it expects is the helper's own
//! belief written down twice — it agrees with the code by construction and
//! notices nothing. `live-2.1.17-world-snapshot.json` is what the game actually
//! reported, so the two sides have independent origins.
//!
//! Why it is worth a test: three helpers disagreed with the game and the whole
//! suite passed. Two were oversized, which is merely conservative. **One was a
//! third too small** — `new_electric_mining_drill` built a 1.8 box where the
//! game's is 2.6953125 — and undersized is the dangerous direction, because it
//! lets a layout that cannot exist pass a geometry check. That is exactly how
//! `method::connect`'s geometry defect survived four clean reviews: fixtures
//! built with a shrunken box, so they fitted the code.

use factorio_bot_core::types::{Direction, FactorioEntity, Position};
use std::collections::HashMap;

/// name -> collision-box width/height, from the captured snapshot.
fn captured_boxes() -> HashMap<String, (f64, f64)> {
    let raw = include_str!("live-2.1.17-world-snapshot.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("snapshot parses");
    let mut out = HashMap::new();
    for p in v["entity_prototypes"].as_array().expect("prototype array") {
        let (Some(name), Some(cb)) = (p["name"].as_str(), p.get("collision_box")) else {
            continue;
        };
        let (lt, rb) = (&cb["left_top"], &cb["right_bottom"]);
        let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
            lt["x"].as_f64(),
            lt["y"].as_f64(),
            rb["x"].as_f64(),
            rb["y"].as_f64(),
        ) else {
            continue;
        };
        out.insert(name.to_string(), (x1 - x0, y1 - y0));
    }
    out
}

#[test]
fn every_helper_carries_the_collision_box_the_game_reports() {
    let captured = captured_boxes();
    assert!(
        captured.len() > 500,
        "snapshot yielded only {} prototypes with a collision box -- if its \
         shape changed, fix this reader rather than letting the test silently \
         check nothing",
        captured.len()
    );

    let at = Position::new(0.0, 0.0);
    let north = Direction::North;
    let subjects: Vec<(&str, FactorioEntity)> = vec![
        (
            "stone-furnace",
            FactorioEntity::new_stone_furnace(&at, north),
        ),
        (
            "burner-mining-drill",
            FactorioEntity::new_burner_mining_drill(&at, north),
        ),
        (
            "electric-mining-drill",
            FactorioEntity::new_electric_mining_drill(&at, north),
        ),
        (
            "transport-belt",
            FactorioEntity::new_transport_belt(&at, north),
        ),
        ("splitter", FactorioEntity::new_splitter(&at, north)),
    ];

    let mut wrong = Vec::new();
    for (name, entity) in &subjects {
        let Some((want_w, want_h)) = captured.get(*name) else {
            panic!("{name} is not in the captured snapshot; the test cannot check it");
        };
        let (got_w, got_h) = (entity.bounding_box.width(), entity.bounding_box.height());
        // Exact: these are fixed constants on both sides, not measurements.
        if (got_w - want_w).abs() > 1e-9 || (got_h - want_h).abs() > 1e-9 {
            let how = if got_w < *want_w {
                "TOO SMALL"
            } else {
                "too big"
            };
            wrong.push(format!(
                "{name}: helper {got_w:.7}x{got_h:.7}, game {want_w:.7}x{want_h:.7} ({how})"
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "helper collision boxes disagree with the game: {}\n\
         Undersized is the dangerous direction -- it lets a layout that cannot \
         exist pass a geometry check.",
        wrong.join("; ")
    );
}
