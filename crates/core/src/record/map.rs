//! What the bots built, and whether the game agreed.
//!
//! Two line kinds share `map.jsonl`. `placed`/`removed` are deltas carrying
//! the map; `keyframe` bounds how far a reconstruction from those deltas can
//! drift. Deltas are exact and tiny; keyframes at delta frequency would be
//! megabytes of repetition.

use serde::{Deserialize, Serialize};

use crate::types::Position;

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
}
