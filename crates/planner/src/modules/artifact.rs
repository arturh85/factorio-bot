//! Versioned factory module design artifacts.
//!
//! A [`ModuleDesign`] captures the geometry, ports, startup bill, operating
//! contract, and provenance of a reusable factory cell. Designs are
//! identified by a content-hash [`DesignId`] and validated before use.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};


/// SHA-256 content hash of a module design's canonical form.
pub type DesignId = String;

/// Monotonic instance identifier within one persisted planning session.
pub type InstanceId = u64;

/// Stable identifier for a module's input or output port.
pub type PortId = String;

// ---------------------------------------------------------------------------
// Rate
// ---------------------------------------------------------------------------

/// A rational rate: `numerator` units per `ticks` game ticks.
///
/// Always normalized by GCD so that equivalent rates compare equal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rate {
    pub numerator: u64,
    pub ticks: std::num::NonZeroU64,
}

impl Rate {
    /// Create a new rate, normalizing by GCD.
    ///
    /// Returns `Err` if `ticks` is zero.
    pub fn new(numerator: u64, ticks: u64) -> Result<Self, ModuleError> {
        let ticks = std::num::NonZeroU64::new(ticks)
            .ok_or(ModuleError::InvalidArtifact("zero ticks in rate".into()))?;
        let gcd = gcd_u64(numerator, ticks.get());
        Ok(Self {
            numerator: numerator / gcd,
            ticks: std::num::NonZeroU64::new(ticks.get() / gcd).unwrap(),
        })
    }
}

fn gcd_u64(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd_u64(b, a % b) }
}

// ---------------------------------------------------------------------------
// Offsets and directions
// ---------------------------------------------------------------------------

/// An offset in half-tile units relative to a module's origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offset {
    pub half_x: i32,
    pub half_y: i32,
}

// ---------------------------------------------------------------------------
// Module family and knowledge origin
// ---------------------------------------------------------------------------

/// Which family of factory module this design belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModuleFamily {
    OreToPlate,
    RedScience,
}

/// How the design was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum KnowledgeOrigin {
    /// Extracted from the existing native planner layout.
    Extracted,
    /// Imported from an external blueprint string.
    Imported,
    /// Discovered by a search or learning process.
    Discovered,
}

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

/// What a port moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortMode {
    BeltInput,
    InventoryOutput,
    DirectLabOutput,
}

/// Which lane(s) of a belt a port uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lane {
    Left,
    Right,
    Both,
}

/// A single named input or output port of a module design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    pub id: PortId,
    pub mode: PortMode,
    pub item: String,
    pub offset: Offset,
    pub direction: u8,
    pub lane: Option<Lane>,
    pub maximum: Rate,
}

// ---------------------------------------------------------------------------
// Parts (entities)
// ---------------------------------------------------------------------------

/// The underground half of a splitter or underground belt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndergroundHalf {
    Input,
    Output,
}

/// A single entity that makes up part of a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub role: String,
    pub entity: String,
    pub offset: Offset,
    pub direction: u8,
    pub recipe: Option<String>,
    pub underground_half: Option<UndergroundHalf>,
}

// ---------------------------------------------------------------------------
// Module parameters
// ---------------------------------------------------------------------------

/// Parameters that select a specific variant of a module family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ModuleParameters {
    pub item: String,
    pub with_pole: bool,
    pub labs: u8,
}

impl std::hash::Hash for ModuleParameters {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.item.hash(state);
        self.with_pole.hash(state);
        self.labs.hash(state);
    }
}

// ---------------------------------------------------------------------------
// Operating contract
// ---------------------------------------------------------------------------

/// The predicted operating behaviour of a module design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatingContract {
    /// Input items and their required rates.
    pub inputs: BTreeMap<String, Rate>,
    /// Output items and their guaranteed rates.
    pub outputs: BTreeMap<String, Rate>,
    /// Peak electric power draw in watts.
    pub power_watts: u64,
    /// Fuel consumed per tick (coal, etc.).
    pub fuel_per_tick: BTreeMap<String, Rate>,
    /// Ticks from start of construction to first output.
    pub startup_latency_ticks: u64,
    /// Items consumed during startup (bill of materials).
    pub startup_items: BTreeMap<String, u64>,
    /// Internal buffer capacity for each item.
    pub local_buffer_capacity: BTreeMap<String, u64>,
    /// Technologies that must be researched.
    pub required_research: Vec<String>,
    /// Surface this module is designed for.
    pub required_surface: String,
    /// Mechanisms this design uses that the planner cannot model.
    pub unsupported_mechanisms: Vec<String>,
}

// ---------------------------------------------------------------------------
// ModuleDesign
// ---------------------------------------------------------------------------

/// A complete, versioned, reusable factory module design.
///
/// Identified by a content-hash [`DesignId`] derived from all semantic
/// fields. The `id` field is excluded from its own hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDesign {
    /// Schema version (currently 1).
    pub schema: u32,
    /// Stable content-hash identifier (derived from all other fields).
    #[serde(default)]
    pub id: DesignId,
    /// Which family this design belongs to.
    pub family: ModuleFamily,
    /// Version of the generator that produced this design.
    pub generator_version: u32,
    /// How this design was obtained.
    pub origin: KnowledgeOrigin,
    /// Parameters that select this variant.
    pub parameters: ModuleParameters,
    /// Hash of the prototype/recipe data used during generation.
    pub prototype_hash: String,
    /// Mod versions at generation time.
    pub mod_versions: BTreeMap<String, String>,
    /// Parent design IDs (for provenance).
    pub parents: Vec<DesignId>,
    /// Training manifest reference, if discovered.
    pub training_manifest: Option<String>,
    /// All entities in the design, with roles.
    pub parts: Vec<Part>,
    /// Named input/output ports.
    pub ports: Vec<Port>,
    /// Tiles that must be clear around the module (access corridors).
    pub required_clearance: Vec<Offset>,
    /// Tiles reserved for future expansion.
    pub expansion_space: Vec<Offset>,
    /// Total bill of materials (sum of all parts' costs).
    pub bill: BTreeMap<String, u64>,
    /// Partial ordering of construction roles: (prerequisite, dependent).
    pub precedence: Vec<(String, String)>,
    /// Operating contract: what the module consumes and produces.
    pub operation: OperatingContract,
}

// ---------------------------------------------------------------------------
// ModuleError
// ---------------------------------------------------------------------------

/// Errors during module design, validation, or compilation.
#[derive(Debug, Clone)]
pub enum ModuleError {
    Unsupported(String),
    InvalidArtifact(String),
    Incompatible(String),
    NoSite(String),
    Unfunded(String),
    ArithmeticOverflow,
    Cancelled,
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleError::Unsupported(s) => write!(f, "unsupported: {s}"),
            ModuleError::InvalidArtifact(s) => write!(f, "invalid artifact: {s}"),
            ModuleError::Incompatible(s) => write!(f, "incompatible: {s}"),
            ModuleError::NoSite(s) => write!(f, "no site: {s}"),
            ModuleError::Unfunded(s) => write!(f, "unfunded: {s}"),
            ModuleError::ArithmeticOverflow => write!(f, "arithmetic overflow"),
            ModuleError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for ModuleError {}

// ---------------------------------------------------------------------------
// Canonical serialization and ID derivation
// ---------------------------------------------------------------------------

/// Serialize a `ModuleDesign` into its canonical byte representation for
/// content-hashing. The `id` field is excluded from the hash.
pub fn canonical_bytes(design: &ModuleDesign) -> Result<Vec<u8>, ModuleError> {
    let mut value = factorio_bot_core::serde_json::to_value(design)
        .map_err(|e| ModuleError::InvalidArtifact(e.to_string()))?;
    // Remove the id field from the top-level object for hashing.
    if let Some(obj) = value.as_object_mut() {
        obj.remove("id");
    }
    // Recursively sort keys for deterministic serialization.
    let sorted = sort_json_value(value);
    factorio_bot_core::serde_json::to_vec(&sorted)
        .map_err(|e| ModuleError::InvalidArtifact(e.to_string()))
}

fn sort_json_value(value: factorio_bot_core::serde_json::Value) -> factorio_bot_core::serde_json::Value {
    match value {
        factorio_bot_core::serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, factorio_bot_core::serde_json::Value> = map
                .into_iter()
                .map(|(k, v)| (k, sort_json_value(v)))
                .collect();
            factorio_bot_core::serde_json::Value::Object(sorted.into_iter().collect())
        }
        factorio_bot_core::serde_json::Value::Array(arr) => {
            factorio_bot_core::serde_json::Value::Array(arr.into_iter().map(sort_json_value).collect())
        }
        other => other,
    }
}

/// Compute the content-hash [`DesignId`] for a module design.
pub fn design_id(design: &ModuleDesign) -> Result<DesignId, ModuleError> {
    use sha2::{Digest, Sha256};
    let bytes = canonical_bytes(design)?;
    let hash = Sha256::digest(&bytes);
    Ok(format!("{:x}", hash))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_rates_compare_exactly() {
        assert_eq!(Rate::new(6, 3600).unwrap(), Rate::new(1, 600).unwrap());
        assert!(Rate::new(1, 0).is_err());
    }

    #[test]
    fn design_id_changes_with_semantic_fields() {
        let base = ModuleDesign {
            schema: 1,
            id: String::new(),
            family: ModuleFamily::OreToPlate,
            generator_version: 1,
            origin: KnowledgeOrigin::Extracted,
            parameters: ModuleParameters {
                item: "iron-plate".into(),
                with_pole: false,
                labs: 0,
            },
            prototype_hash: "abc123".into(),
            mod_versions: BTreeMap::new(),
            parents: vec![],
            training_manifest: None,
            parts: vec![Part {
                role: "drill".into(),
                entity: "burner-mining-drill".into(),
                offset: Offset { half_x: 0, half_y: 0 },
                direction: 0,
                recipe: None,
                underground_half: None,
            }],
            ports: vec![],
            required_clearance: vec![],
            expansion_space: vec![],
            bill: BTreeMap::new(),
            precedence: vec![],
            operation: OperatingContract {
                inputs: BTreeMap::from([("iron-ore".into(), Rate::new(1, 600).unwrap())]),
                outputs: BTreeMap::from([("iron-plate".into(), Rate::new(1, 600).unwrap())]),
                power_watts: 0,
                fuel_per_tick: BTreeMap::new(),
                startup_latency_ticks: 0,
                startup_items: BTreeMap::new(),
                local_buffer_capacity: BTreeMap::new(),
                required_research: vec![],
                required_surface: "nauvis".into(),
                unsupported_mechanisms: vec![],
            },
        };

        let id1 = design_id(&base).unwrap();

        // Changing a recipe should change the id.
        let mut modified = base.clone();
        modified.operation.outputs.insert(
            "copper-plate".into(),
            Rate::new(1, 600).unwrap(),
        );
        let id2 = design_id(&modified).unwrap();
        assert_ne!(id1, id2, "different content must produce different ids");

        // Reordering parts should NOT change the id (we sort before hashing).
        let mut reversed = base.clone();
        reversed.parts.reverse();
        let id3 = design_id(&reversed).unwrap();
        assert_eq!(id1, id3, "reordering parts must not change the id");
    }
}
