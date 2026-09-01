//! What the bots built, and whether the game agreed.
//!
//! Two line kinds share `map.jsonl`. `placed`/`removed` are deltas carrying
//! the map; `keyframe` bounds how far a reconstruction from those deltas can
//! drift. Deltas are exact and tiny; keyframes at delta frequency would be
//! megabytes of repetition.

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use crate::types::{Pos, Position};

/// One line of `map.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MapRecord {
    pub tick: u64,
    #[serde(flatten)]
    pub kind: MapKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapKind {
    Placed {
        bot: u32,
        /// What the executor asked for.
        intent: EntitySnapshot,
        /// What the game reports it created.
        actual: EntitySnapshot,
        /// Field names that differ, or null when they agree. Null and empty
        /// would mean the same thing; only one of them is written.
        drift: Option<Vec<String>>,
    },
    Removed {
        bot: u32,
        entity: EntitySnapshot,
    },
    Keyframe {
        bounds: Bounds,
        /// What the game reports inside `bounds`.
        game: Vec<EntitySnapshot>,
        /// What our `EntityGraph` believes is inside `bounds`.
        model: Vec<EntitySnapshot>,
        /// Entities present in exactly one of them.
        divergence: Vec<Divergence>,
    },
    /// A kind this build does not know. Readers skip it; writers never emit it.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EntitySnapshot {
    pub name: String,
    /// Unrounded. A resource sits at a tile centre and stays there.
    pub position: Position,
    /// Factorio 2.0 uses 16 values. For an inserter this is the side it PICKS
    /// UP from, not the side it drops into.
    pub direction: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Bounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Divergence {
    pub entity: EntitySnapshot,
    /// Which side has it: `"game"` or `"model"`.
    pub only_in: String,
}

/// The placement the executor performed, kept on its `Attempt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub intent: EntitySnapshot,
    pub actual: EntitySnapshot,
    pub drift: Option<Vec<String>>,
}

/// Which fields of a placement the game did not honour.
///
/// `None` when they agree. Compares position exactly: the executor asks for a
/// position the game either accepts or snaps, and a tolerance here would hide
/// exactly the snapping we want to see.
pub fn drift_between(intent: &EntitySnapshot, actual: &EntitySnapshot) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    if intent.name != actual.name {
        fields.push("name".to_string());
    }
    if intent.position != actual.position {
        fields.push("position".to_string());
    }
    if intent.direction != actual.direction {
        fields.push("direction".to_string());
    }
    (!fields.is_empty()).then_some(fields)
}

/// The position a resource actually occupies, given the floored key
/// `EntityGraph` stores it under.
///
/// Every real resource entity sits at a tile centre -- `(-40.5, -48.5)`, never
/// `(-41, -49)` -- and the mod's `surface.find_entity(name, position)` matches
/// exactly. `Pos(i32, i32)` floors, so reading one back out of the graph must
/// add back the half tile the key threw away, or every resource in `model`
/// ends up 0.5 off every resource in `game` and the divergence list becomes
/// noise instead of a signal.
pub fn resource_position_from_pos(pos: Pos) -> Position {
    Position::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5)
}

/// Entities present on exactly one side.
///
/// Order is `game`-only first, then `model`-only, each preserving the order
/// given, so the list is stable across runs and a diff of two runs' output is
/// readable rather than shuffled.
pub fn divergence_between(game: &[EntitySnapshot], model: &[EntitySnapshot]) -> Vec<Divergence> {
    let mut out = Vec::new();
    for entity in game {
        if !model.contains(entity) {
            out.push(Divergence {
                entity: entity.clone(),
                only_in: "game".to_string(),
            });
        }
    }
    for entity in model {
        if !game.contains(entity) {
            out.push(Divergence {
                entity: entity.clone(),
                only_in: "model".to_string(),
            });
        }
    }
    out
}

/// What [`read_map`] found.
pub struct ReadMap {
    pub records: Vec<MapRecord>,
    /// Lines that did not parse -- in practice the truncated final line of a
    /// crashed run. Reported rather than swallowed, exactly like
    /// [`super::read_events`] and [`super::read_samples`].
    pub skipped: usize,
}

/// Reads `map.jsonl`, tolerating a truncated tail.
///
/// A [`MapKind::Unknown`] line parses fine here -- `#[serde(other)]` sees to
/// that -- and is returned like any other record; a reader that does not
/// understand a kind is expected to skip it when it matters, not here.
pub fn read_map(path: &Path) -> io::Result<ReadMap> {
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<MapRecord>(&line) {
            Ok(record) => records.push(record),
            Err(_) => skipped += 1,
        }
    }
    Ok(ReadMap { records, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str, x: f64, y: f64, direction: u8) -> EntitySnapshot {
        EntitySnapshot {
            name: name.to_string(),
            position: Position::new(x, y),
            direction,
        }
    }

    #[test]
    fn agreement_is_no_drift() {
        let a = snap("stone-furnace", -12.0, 8.0, 0);
        assert_eq!(drift_between(&a, &a), None);
    }

    #[test]
    fn a_differing_field_is_named() {
        // The inserter-direction trap is exactly this shape: a layout that
        // places 100% correctly and does nothing, because placement and
        // function are separate concerns.
        let intent = snap("burner-inserter", 4.0, 2.0, 0);
        let actual = snap("burner-inserter", 4.0, 2.0, 12);
        assert_eq!(
            drift_between(&intent, &actual),
            Some(vec!["direction".to_string()])
        );
    }

    #[test]
    fn several_differing_fields_are_all_named() {
        let intent = snap("stone-furnace", -12.0, 8.0, 0);
        let actual = snap("steel-furnace", -12.5, 8.0, 0);
        assert_eq!(
            drift_between(&intent, &actual),
            Some(vec!["name".to_string(), "position".to_string()])
        );
    }

    #[test]
    fn a_placed_record_round_trips() {
        let record = MapRecord {
            tick: 62010,
            kind: MapKind::Placed {
                bot: 3,
                intent: snap("stone-furnace", -12.0, 8.0, 0),
                actual: snap("stone-furnace", -12.0, 8.0, 0),
                drift: None,
            },
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains(r#""kind":"placed""#));
        assert_eq!(serde_json::from_str::<MapRecord>(&json).unwrap(), record);
    }

    #[test]
    fn identical_sides_do_not_diverge() {
        let e = vec![snap("stone-furnace", -12.0, 8.0, 0)];
        assert!(divergence_between(&e, &e).is_empty());
    }

    #[test]
    fn an_entity_only_the_game_has_is_reported_as_such() {
        let game = vec![snap("stone-furnace", -12.0, 8.0, 0)];
        assert_eq!(
            divergence_between(&game, &[]),
            vec![Divergence {
                entity: game[0].clone(),
                only_in: "game".to_string()
            }]
        );
    }

    #[test]
    fn an_entity_only_the_model_has_is_reported_as_such() {
        let model = vec![snap("stone-furnace", -12.0, 8.0, 0)];
        assert_eq!(
            divergence_between(&[], &model),
            vec![Divergence {
                entity: model[0].clone(),
                only_in: "model".to_string()
            }]
        );
    }

    #[test]
    fn a_resource_read_out_of_the_graph_keeps_its_tile_centre() {
        // EntityGraph keys resources by Pos(i32, i32), which FLOORS. Reading
        // one back without restoring the half tile puts every resource 0.5
        // off, and the divergence list becomes pure noise instead of a
        // signal.
        //
        // This exact round trip once made mining fail with "no entity to
        // mine" for every ore on every map, while every test passed -- the
        // test fixture built ore at integer positions, the one input for
        // which the lossy round trip is lossless.
        let pos = Pos(-41, -49);
        assert_eq!(
            resource_position_from_pos(pos.clone()),
            Position::new(-40.5, -48.5)
        );

        let game = vec![snap("iron-ore", -40.5, -48.5, 0)];
        let model = vec![EntitySnapshot {
            name: "iron-ore".to_string(),
            position: resource_position_from_pos(pos),
            direction: 0,
        }];
        assert!(divergence_between(&game, &model).is_empty());
    }

    fn write_lines(dir: &Path, lines: &[&str]) -> std::path::PathBuf {
        let path = dir.join("map.jsonl");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    #[test]
    fn read_map_reads_every_record() {
        let tmp = tempfile::tempdir().unwrap();
        let record = MapRecord {
            tick: 100,
            kind: MapKind::Placed {
                bot: 1,
                intent: snap("stone-furnace", -12.0, 8.0, 0),
                actual: snap("stone-furnace", -12.0, 8.0, 0),
                drift: None,
            },
        };
        let line = serde_json::to_string(&record).unwrap();
        let path = write_lines(tmp.path(), &[&line]);

        let read = read_map(&path).unwrap();
        assert_eq!(read.skipped, 0);
        assert_eq!(read.records, vec![record]);
    }

    #[test]
    fn read_map_skips_a_truncated_final_line() {
        let tmp = tempfile::tempdir().unwrap();
        let good = serde_json::to_string(&MapRecord {
            tick: 100,
            kind: MapKind::Removed {
                bot: 1,
                entity: snap("stone-furnace", -12.0, 8.0, 0),
            },
        })
        .unwrap();
        let path = write_lines(tmp.path(), &[&good, r#"{"tick":200,"kind":"remo"#]);

        let read = read_map(&path).unwrap();
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.skipped, 1);
    }

    #[test]
    fn read_map_returns_an_unknown_kind_rather_than_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_lines(
            tmp.path(),
            &[r#"{"tick":50,"kind":"invented_later","whatever":true}"#],
        );

        let read = read_map(&path).unwrap();
        assert_eq!(read.skipped, 0, "an unknown kind is not a parse failure");
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.records[0].kind, MapKind::Unknown);
    }
}
