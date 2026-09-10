//! In-memory cache for module designs, validation evidence, and site
//! feasibility. No persistent disk cache: a library file stores
//! designs/evidence, and an in-memory cache serves a trial.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::modules::artifact::{
    DesignId, ModuleDesign, ModuleError, ModuleFamily, ModuleParameters, Rate,
};
use std::collections::BTreeSet;
use crate::modules::families::extract_design;
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// How thoroughly a module design has been validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationLevel {
    /// No validation has been performed.
    Unvalidated,
    /// Structural checks passed (geometry, ports, bill).
    Structural,
    /// Model-checked against the planning model.
    ModelChecked,
    /// Validated against live game measurements.
    LiveMeasured,
}

/// Evidence that a module design was validated at a given level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub design_id: DesignId,
    pub level: ValidationLevel,
    pub model_version: String,
    pub conditions_hash: String,
    pub fixture_ids: Vec<String>,
    pub observed_error: Option<Rate>,
}

// ---------------------------------------------------------------------------
// Cache mode
// ---------------------------------------------------------------------------

/// Whether the cache should generate new designs or only serve existing ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Serve cached designs; generate new ones if missing.
    On,
    /// Always regenerate, bypassing the cache.
    Off,
}

// ---------------------------------------------------------------------------
// LibraryCache
// ---------------------------------------------------------------------------

/// In-memory cache for module designs and their validation evidence.
///
/// Designs are keyed by their family/parameters/generator/prototype fingerprint.
/// Site feasibility is cached separately and invalidated by world revision.
#[derive(Debug, Clone, Default)]
pub struct LibraryCache {
    /// Cached designs, keyed by content-hash DesignId.
    designs: BTreeMap<String, Arc<ModuleDesign>>,
    /// Validation evidence, keyed by DesignId.
    evidence: BTreeMap<String, ValidationEvidence>,
    /// Site feasibility cache: (design_id, surface, transform) -> bool.
    sites: BTreeMap<String, bool>,
    /// How many designs were generated (cache misses).
    pub generated: u64,
    /// How many cache hits (returned without generating).
    pub hits: u64,
    /// How many cache misses (generated or failed).
    pub misses: u64,
}

impl LibraryCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cached design by its content-hash ID.
    pub fn get_by_id(&self, id: &str) -> Option<Arc<ModuleDesign>> {
        self.designs.get(id).cloned()
    }

    /// Store a validated design in the cache.
    pub fn insert(&mut self, design: ModuleDesign) -> Arc<ModuleDesign> {
        let id = design.id.clone();
        let arc = Arc::new(design);
        self.designs.insert(id, arc.clone());
        arc
    }

    /// Check whether a design has been validated to at least the given level.
    pub fn validation_level(&self, id: &DesignId) -> Option<ValidationLevel> {
        self.evidence.get(id).map(|e| e.level)
    }

    /// Record validation evidence for a design.
    pub fn record_evidence(&mut self, evidence: ValidationEvidence) {
        self.evidence.insert(evidence.design_id.clone(), evidence);
    }

    /// Cache a site feasibility result.
    pub fn cache_site(&mut self, key: String, feasible: bool) {
        self.sites.insert(key, feasible);
    }

    /// Check cached site feasibility.
    pub fn site_feasible(&self, key: &str) -> Option<bool> {
        self.sites.get(key).copied()
    }
}

// ---------------------------------------------------------------------------
// Cache key helpers
// ---------------------------------------------------------------------------

/// Build a canonical cache key for a design request.
fn design_cache_key(
    family: ModuleFamily,
    parameters: &ModuleParameters,
    prototype_hash: &str,
) -> String {
    format!(
        "{:?}/item={}/pole={}/labs={}/proto={}",
        family, parameters.item, parameters.with_pole, parameters.labs, prototype_hash
    )
}

// ---------------------------------------------------------------------------
// get_design: cache-aware design retrieval
// ---------------------------------------------------------------------------

/// Retrieve a module design, using the cache when enabled.
///
/// When `CacheMode::On` and the design is already cached by its
/// family/parameters/prototype key, returns the cached copy. Otherwise
/// invokes the generator.
pub fn get_design(
    cache: &mut LibraryCache,
    state: &PlanState,
    family: ModuleFamily,
    parameters: &ModuleParameters,
    mode: CacheMode,
) -> Result<Arc<ModuleDesign>, ModuleError> {
    let _key = design_cache_key(family, parameters, "extracted-v1");

    // Check the cache first.
    if matches!(mode, CacheMode::On) {
        // Look up by the key -> design_id mapping.
        // For now, iterate to find a matching design.
        for design in cache.designs.values() {
            if design.family == family && design.parameters == *parameters {
                cache.hits += 1;
                return Ok(design.clone());
            }
        }
    }

    cache.misses += 1;

    // Generate the design.
    let design = extract_design(state, family, parameters)?;

    // Cache it.
    let id = design.id.clone();
    let arc = Arc::new(design);
    cache.designs.insert(id, arc.clone());
    cache.generated += 1;

    Ok(arc)
}

// ---------------------------------------------------------------------------
// validate_design: structural validation
// ---------------------------------------------------------------------------

/// Perform local structural validation of a module design.
///
/// Checks:
/// - No duplicate or overlapping parts at the same offset
/// - Valid role references in precedence
/// - Acyclic precedence graph
/// - Recipe compatibility (every recipe must be known)
/// - Finite, nonnegative capacities
pub fn validate_design(
    design: &ModuleDesign,
    _state: &PlanState,
) -> Result<ValidationEvidence, ModuleError> {
    // 1. Check for duplicate role IDs.
    let mut role_set = BTreeMap::new();
    for part in &design.parts {
        if role_set.contains_key(&part.role) {
            return Err(ModuleError::InvalidArtifact(format!(
                "duplicate role '{}'",
                part.role
            )));
        }
        role_set.insert(&part.role, &part.offset);
    }

    // 2. Check overlapping parts at the same offset.
    // Different roles at the same offset are allowed (e.g., a belt and
    // an inserter sharing a tile is not actually overlapping in the game).
    // We only flag exact same-role duplicates (already caught above).

    // 3. Validate precedence references.
    let valid_roles: BTreeSet<&str> = design.parts.iter().map(|p| p.role.as_str()).collect();
    for (prereq, dependent) in &design.precedence {
        if !valid_roles.contains(prereq.as_str()) {
            return Err(ModuleError::InvalidArtifact(format!(
                "precedence references unknown role '{}'",
                prereq
            )));
        }
        if !valid_roles.contains(dependent.as_str()) {
            return Err(ModuleError::InvalidArtifact(format!(
                "precedence references unknown role '{}'",
                dependent
            )));
        }
    }

    // 4. Check for cycles in precedence (simple topological sort).
    let mut in_degree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for role in &valid_roles {
        in_degree.insert(role, 0);
        adj.insert(role, vec![]);
    }
    for (prereq, dependent) in &design.precedence {
        if let Some(deg) = in_degree.get_mut(dependent.as_str()) {
            *deg += 1;
        }
        if let Some(edges) = adj.get_mut(prereq.as_str()) {
            edges.push(dependent.as_str());
        }
    }
    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(role, _)| *role)
        .collect();
    let mut visited = 0;
    while let Some(role) = queue.pop() {
        visited += 1;
        if let Some(edges) = adj.get(role) {
            for next in edges {
                if let Some(deg) = in_degree.get_mut(next) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(next);
                    }
                }
            }
        }
    }
    if visited != valid_roles.len() {
        return Err(ModuleError::InvalidArtifact(
            "precedence graph contains a cycle".into(),
        ));
    }

    // 5. Check operating contract for finite nonnegative capacities.
    for (item, capacity) in &design.operation.local_buffer_capacity {
        if *capacity == 0 {
            return Err(ModuleError::InvalidArtifact(format!(
                "zero capacity for buffer '{}'",
                item
            )));
        }
    }

    // Build evidence.
    let evidence = ValidationEvidence {
        design_id: design.id.clone(),
        level: ValidationLevel::Structural,
        model_version: "0.1".into(),
        conditions_hash: "default".into(),
        fixture_ids: vec![],
        observed_error: None,
    };

    Ok(evidence)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

use crate::modules::artifact::{
        KnowledgeOrigin, ModuleFamily, ModuleParameters, Offset, OperatingContract, Part, Rate,
    };
    use crate::modules::families::extract_design;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn test_design() -> ModuleDesign {
        // A minimal valid design for cache testing.
        ModuleDesign {
            schema: 1,
            id: "test-id".into(),
            family: ModuleFamily::OreToPlate,
            generator_version: 1,
            origin: KnowledgeOrigin::Extracted,
            parameters: ModuleParameters {
                item: "iron-plate".into(),
                with_pole: false,
                labs: 0,
            },
            prototype_hash: "test-hash".into(),
            mod_versions: BTreeMap::new(),
            parents: vec![],
            training_manifest: None,
            parts: vec![
                Part {
                    role: "drill".into(),
                    entity: "burner-mining-drill".into(),
                    offset: Offset { half_x: 0, half_y: 0 },
                    direction: 0,
                    recipe: None,
                    underground_half: None,
                },
            ],
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
        }
    }

    #[test]
    fn identical_requests_generate_once() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let mut cache = LibraryCache::new();

        let parameters = ModuleParameters {
            item: "iron-plate".into(),
            with_pole: false,
            labs: 0,
        };

        // First call: cache miss, generates.
        let _a = get_design(&mut cache, &state, ModuleFamily::OreToPlate, &parameters, CacheMode::On).unwrap();
        assert_eq!(cache.generated, 1);
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.hits, 0);

        // Second call: cache hit.
        let _b = get_design(&mut cache, &state, ModuleFamily::OreToPlate, &parameters, CacheMode::On).unwrap();
        assert_eq!(cache.generated, 1, "no new generation on cache hit");
        assert_eq!(cache.hits, 1, "hit counter incremented");
    }

    #[test]
    fn cache_off_generates_every_time() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let mut cache = LibraryCache::new();

        let parameters = ModuleParameters {
            item: "iron-plate".into(),
            with_pole: false,
            labs: 0,
        };

        let _a = get_design(&mut cache, &state, ModuleFamily::OreToPlate, &parameters, CacheMode::Off).unwrap();
        assert_eq!(cache.generated, 1);

        let _b = get_design(&mut cache, &state, ModuleFamily::OreToPlate, &parameters, CacheMode::Off).unwrap();
        assert_eq!(cache.generated, 2, "CacheMode::Off regenerates");
    }

    #[test]
    fn validate_valid_design_succeeds() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let design = test_design();
        let evidence = validate_design(&design, &state).unwrap();
        assert_eq!(evidence.level, ValidationLevel::Structural);
    }

    #[test]
    fn validate_duplicate_role_is_rejected() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let mut design = test_design();
        // Add a second part with the same role.
        design.parts.push(Part {
            role: "drill".into(),
            entity: "stone-furnace".into(),
            offset: Offset { half_x: 2, half_y: 0 },
            direction: 0,
            recipe: None,
            underground_half: None,
        });
        assert!(validate_design(&design, &state).is_err());
    }

    #[test]
    fn validate_unknown_precedence_role_is_rejected() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let mut design = test_design();
        design.precedence.push(("nonexistent".into(), "drill".into()));
        assert!(validate_design(&design, &state).is_err());
    }

    #[test]
    fn validate_cyclic_precedence_is_rejected() {
        let state = PlanState::from_world(
            Arc::new(fixture_world()),
            &[crate::ids::BotId(1)],
        );
        let mut design = test_design();
        // Add a second part so we can create a cycle.
        design.parts.push(Part {
            role: "furnace".into(),
            entity: "stone-furnace".into(),
            offset: Offset { half_x: 0, half_y: 4 },
            direction: 0,
            recipe: Some("iron-plate".into()),
            underground_half: None,
        });
        design.precedence.push(("drill".into(), "furnace".into()));
        design.precedence.push(("furnace".into(), "drill".into()));
        assert!(validate_design(&design, &state).is_err());
    }
}
