//! Terminal evidence for a rocket launch and space platform establishment.
//!
//! The observer joins matching force/request/silo/platform events before
//! constructing this record; [`Self::achieved`] is not a substitute for that
//! attribution validation.

use serde::{Deserialize, Serialize};

/// Evidence that a rocket was launched, carrying a specific payload to a
/// specific platform, and that the platform was established.
///
/// Each tick field is optional and set only when the corresponding event has
/// been observed: `launch_ordered_tick` from the request, `launched_tick`
/// from the game's `on_rocket_launched` event, and
/// `platform_established_tick` from the game's `on_space_platform_started`
/// event (or equivalent).
///
/// `run_key` is the run-scoped unique string the request was made under.
/// `payload` is the item prototype name of the cargo, which must match the
/// request's starter pack for [`Self::achieved`] to return true.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RocketLaunchEvidence {
    pub run_key: String,
    pub silo_unit_number: u32,
    pub platform_index: u32,
    pub payload: String,
    pub launch_ordered_tick: Option<u64>,
    pub launched_tick: Option<u64>,
    pub platform_established_tick: Option<u64>,
}

impl RocketLaunchEvidence {
    /// Whether this evidence record constitutes a completed launch cycle.
    ///
    /// Returns true only when the payload is `space-platform-starter-pack`
    /// AND the rocket has been observed launching AND the platform has been
    /// observed establishing. This is a convenience check, not a substitute
    /// for attribution validation: the caller must still have joined matching
    /// force/request/silo/platform events before constructing this record.
    pub fn achieved(&self) -> bool {
        self.payload == "space-platform-starter-pack"
            && self.launched_tick.is_some()
            && self.platform_established_tick.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn achieved_requires_all_fields() {
        // launch_ordered is not required for achieved; launched + platform is.
        let evidence = RocketLaunchEvidence {
            run_key: "test-1".to_string(),
            silo_unit_number: 42,
            platform_index: 1,
            payload: "space-platform-starter-pack".to_string(),
            launch_ordered_tick: Some(1000),
            launched_tick: None,
            platform_established_tick: None,
        };
        assert!(
            !evidence.achieved(),
            "launch_ordered alone should not be achieved"
        );
    }

    #[test]
    fn achieved_requires_launched_tick() {
        let evidence = RocketLaunchEvidence {
            run_key: "test-2".to_string(),
            silo_unit_number: 42,
            platform_index: 1,
            payload: "space-platform-starter-pack".to_string(),
            launch_ordered_tick: Some(1000),
            launched_tick: None,
            platform_established_tick: Some(5000),
        };
        assert!(
            !evidence.achieved(),
            "missing launched_tick should not be achieved"
        );
    }

    #[test]
    fn achieved_requires_platform_tick() {
        let evidence = RocketLaunchEvidence {
            run_key: "test-3".to_string(),
            silo_unit_number: 42,
            platform_index: 1,
            payload: "space-platform-starter-pack".to_string(),
            launch_ordered_tick: Some(1000),
            launched_tick: Some(3000),
            platform_established_tick: None,
        };
        assert!(
            !evidence.achieved(),
            "missing platform_established_tick should not be achieved"
        );
    }

    #[test]
    fn achieved_requires_correct_payload() {
        let evidence = RocketLaunchEvidence {
            run_key: "test-4".to_string(),
            silo_unit_number: 42,
            platform_index: 1,
            payload: "satellite".to_string(),
            launch_ordered_tick: Some(1000),
            launched_tick: Some(3000),
            platform_established_tick: Some(5000),
        };
        assert!(!evidence.achieved(), "wrong payload should not be achieved");
    }

    #[test]
    fn achieved_full_success() {
        let evidence = RocketLaunchEvidence {
            run_key: "test-5".to_string(),
            silo_unit_number: 42,
            platform_index: 1,
            payload: "space-platform-starter-pack".to_string(),
            launch_ordered_tick: Some(1000),
            launched_tick: Some(3000),
            platform_established_tick: Some(5000),
        };
        assert!(evidence.achieved(), "all fields present should be achieved");
    }

    #[test]
    fn achieved_null_launch_ordered_still_achieved() {
        // launch_ordered_tick is informational; achieved does not require it.
        let evidence = RocketLaunchEvidence {
            run_key: "test-6".to_string(),
            silo_unit_number: 42,
            platform_index: 1,
            payload: "space-platform-starter-pack".to_string(),
            launch_ordered_tick: None,
            launched_tick: Some(3000),
            platform_established_tick: Some(5000),
        };
        assert!(
            evidence.achieved(),
            "null launch_ordered_tick should still allow achieved"
        );
    }
}
