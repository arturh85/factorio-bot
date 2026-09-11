//! Reusable factory module artifacts, families, and planning components.
//!
//! This module implements the phase-1 module-backed planner: explicit
//! versioned module designs, instance identity, caching, supply ledgers,
//! selection, and compilation into existing action networks.
//!
//! The public API is re-exported through the planner crate root.

pub mod artifact;
pub mod cache;
pub mod compile;
pub mod demand;
pub mod error;
pub mod families;
pub mod instance;
pub mod ledger;
pub mod select;

pub use demand::{DemandSet, normalize_goals, required_gross};

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn module_module_is_loaded() {
        // Placeholder: verifies the module tree compiles.
        assert!(true);
    }
}
