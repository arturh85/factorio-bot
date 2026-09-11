//! Source capacity accounting and operating support ledger.
//!
//! An [`OperatingLedger`] tracks how much of each source's capacity has been
//! promised to which consumers, preventing double-spend. Capacity is
//! accounted in rates (units per tick) over time intervals and in stock
//! (deliveries and claims at specific ticks).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::modules::artifact::{InstanceId, ModuleError, Rate};

// ---------------------------------------------------------------------------
// Interval
// ---------------------------------------------------------------------------

/// A half-open interval `[start, end)`. `end > start` must hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interval {
    pub start: u64,
    pub end: u64,
}

impl Interval {
    /// Create a new interval. Returns `None` if `end <= start`.
    pub fn new(start: u64, end: u64) -> Option<Self> {
        if end <= start {
            None
        } else {
            Some(Self { start, end })
        }
    }

    /// Duration of this interval in ticks.
    pub fn duration(&self) -> u64 {
        self.end - self.start
    }

    /// Whether `tick` is within `[start, end)`.
    pub fn contains(&self, tick: u64) -> bool {
        tick >= self.start && tick < self.end
    }

    /// Whether this interval overlaps with another.
    pub fn overlaps(&self, other: &Interval) -> bool {
        self.start < other.end && other.start < self.end
    }
}

// ---------------------------------------------------------------------------
// SourceCapacity
// ---------------------------------------------------------------------------

/// A source of items with a maximum rate over a period of availability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCapacity {
    pub id: String,
    pub item: String,
    pub rate: Rate,
    /// When this source is available.
    pub available: Interval,
}

// ---------------------------------------------------------------------------
// FlowClaim
// ---------------------------------------------------------------------------

/// A consumer's claim on a portion of a source's throughput.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowClaim {
    pub source: String,
    pub consumer: InstanceId,
    pub item: String,
    pub rate: Rate,
    pub interval: Interval,
}

// ---------------------------------------------------------------------------
// StockDelivery and StockClaim
// ---------------------------------------------------------------------------

/// A scheduled delivery of stock to a buffer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StockDelivery {
    pub at: u64,
    pub quantity: u64,
}

/// A consumer's claim on stock from a buffer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StockClaim {
    pub consumer: InstanceId,
    pub at: u64,
    pub quantity: u64,
}

// ---------------------------------------------------------------------------
// OperatingLedger
// ---------------------------------------------------------------------------

/// Central ledger for module supply, power, and fuel accounting.
///
/// Prevents double-spend of any source's capacity. A source is any
/// provider of items, power, or belt/pipe throughput with a finite rate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperatingLedger {
    /// Declared sources of items/rates.
    pub sources: BTreeMap<String, SourceCapacity>,
    /// Committed flow reservations.
    pub flows: Vec<FlowClaim>,
    /// Initial stock present at the start of the operating horizon.
    pub initial_stock: BTreeMap<String, u64>,
    /// Scheduled deliveries of stock.
    pub stock_deliveries: BTreeMap<String, Vec<StockDelivery>>,
    /// Committed stock claims.
    pub stock_claims: BTreeMap<String, Vec<StockClaim>>,
}

impl OperatingLedger {
    /// Declare a source of capacity.
    pub fn add_source(&mut self, capacity: SourceCapacity) {
        self.sources.insert(capacity.id.clone(), capacity);
    }

    /// Reserve a portion of a source's flow capacity.
    ///
    /// Returns `Err` if the claim would exceed the source's available capacity
    /// during any part of the requested interval.
    pub fn reserve_flow(&mut self, claim: FlowClaim) -> Result<(), ModuleError> {
        let source = self
            .sources
            .get(&claim.source)
            .ok_or_else(|| ModuleError::Unfunded(format!("unknown source '{}'", claim.source)))?;

        // Validate that the claim interval is non-empty.
        if claim.interval.start >= claim.interval.end {
            return Err(ModuleError::InvalidArtifact(format!(
                "claim interval [{}, {}) is empty",
                claim.interval.start, claim.interval.end
            )));
        }

        // The source must fully contain the claim interval.
        if source.available.start > claim.interval.start
            || source.available.end < claim.interval.end
        {
            return Err(ModuleError::Unfunded(format!(
                "claim interval {:?} is not fully contained in source '{}' availability {:?}",
                claim.interval, claim.source, source.available
            )));
        }

        // The claim item must match the source item.
        if claim.item != source.item {
            return Err(ModuleError::Incompatible(format!(
                "claim item '{}' does not match source '{}' item '{}'",
                claim.item, claim.source, source.item
            )));
        }

        // Compute the total claimed rate during the overlap of all claims.
        // This is an interval-sweep check: collect all boundary points,
        // sort them, and verify each segment's total claimed rate.
        let overlap_start = claim.interval.start.max(source.available.start);
        let overlap_end = claim.interval.end.min(source.available.end);
        let overlap = Interval::new(overlap_start, overlap_end)
            .ok_or_else(|| ModuleError::ArithmeticOverflow)?;

        // Collect all relevant claim boundaries.
        let mut boundaries: Vec<u64> = vec![overlap.start, overlap.end];
        for existing in &self.flows {
            if existing.source == claim.source {
                let ex_start = existing.interval.start.max(overlap.start);
                let ex_end = existing.interval.end.min(overlap.end);
                if ex_start < ex_end {
                    boundaries.push(ex_start);
                    boundaries.push(ex_end);
                }
            }
        }
        boundaries.sort();
        boundaries.dedup();

        // Check each segment.
        for window in boundaries.windows(2) {
            let seg_start = window[0];
            let seg_end = window[1];
            if seg_start >= seg_end {
                continue;
            }

            // Compute total rate claimed by existing flows in this segment.
            let mut total_existing = 0u128;
            for existing in &self.flows {
                if existing.source == claim.source && existing.interval.contains(seg_start) {
                    total_existing = total_existing
                        .checked_add(existing.rate.numerator as u128)
                        .ok_or(ModuleError::ArithmeticOverflow)?;
                }
            }

            // Add the new claim's rate.
            let total_claimed = total_existing
                .checked_add(claim.rate.numerator as u128)
                .ok_or(ModuleError::ArithmeticOverflow)?;

            // Compare against the source's capacity rate.
            let capacity = source.rate.numerator as u128;
            // Normalize to the same tick denominator.
            let source_per_claim_tick = capacity
                .checked_mul(claim.rate.ticks.get() as u128)
                .ok_or(ModuleError::ArithmeticOverflow)?;
            let claimed_per_source_tick = total_claimed
                .checked_mul(source.rate.ticks.get() as u128)
                .ok_or(ModuleError::ArithmeticOverflow)?;

            if claimed_per_source_tick > source_per_claim_tick {
                return Err(ModuleError::Unfunded(format!(
                    "flow capacity exceeded for source '{}' in interval [{}, {})",
                    claim.source, seg_start, seg_end
                )));
            }
        }

        // Commit the claim.
        self.flows.push(claim);
        Ok(())
    }

    /// Reserve a quantity of stock from a named buffer at a specific tick.
    ///
    /// Returns `Err` if the claim would make the stock balance negative.
    pub fn reserve_stock(&mut self, key: &str, claim: StockClaim) -> Result<(), ModuleError> {
        // Compute available stock at the claim time.
        let initial = self.initial_stock.get(key).copied().unwrap_or(0);
        let total_delivered: u64 = self
            .stock_deliveries
            .get(key)
            .map(|d| {
                d.iter()
                    .filter(|delivery| delivery.at <= claim.at)
                    .map(|d| d.quantity)
                    .sum()
            })
            .unwrap_or(0);
        let total_claimed_before: u64 = self
            .stock_claims
            .get(key)
            .map(|c| {
                c.iter()
                    .filter(|existing| existing.at <= claim.at)
                    .map(|c| c.quantity)
                    .sum()
            })
            .unwrap_or(0);

        let available = initial
            .checked_add(total_delivered)
            .and_then(|v| v.checked_sub(total_claimed_before))
            .ok_or(ModuleError::ArithmeticOverflow)?;

        if claim.quantity > available {
            return Err(ModuleError::Unfunded(format!(
                "stock claim of {} for '{}' at tick {} exceeds available {}",
                claim.quantity, key, claim.at, available
            )));
        }

        self.stock_claims
            .entry(key.to_string())
            .or_default()
            .push(claim);
        Ok(())
    }

    /// Deliver stock to a named buffer.
    pub fn deliver_stock(&mut self, key: &str, delivery: StockDelivery) {
        self.stock_deliveries
            .entry(key.to_string())
            .or_default()
            .push(delivery);
    }

    /// Set initial stock for a named buffer.
    pub fn set_initial_stock(&mut self, key: &str, quantity: u64) {
        self.initial_stock.insert(key.to_string(), quantity);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rate_per_min(n: u64) -> Rate {
        Rate::new(n, 3600).unwrap()
    }

    #[test]
    fn consumers_cannot_each_claim_the_entire_source() {
        let mut ledger = OperatingLedger::default();
        ledger.add_source(SourceCapacity {
            id: "iron-out".into(),
            item: "iron-plate".into(),
            rate: rate_per_min(15),
            available: Interval {
                start: 100,
                end: 18100,
            },
        });

        let claim = FlowClaim {
            source: "iron-out".into(),
            consumer: 1,
            item: "iron-plate".into(),
            rate: rate_per_min(10),
            interval: Interval {
                start: 100,
                end: 18100,
            },
        };
        ledger.reserve_flow(claim.clone()).unwrap();
        assert!(ledger
            .reserve_flow(FlowClaim {
                consumer: 2,
                ..claim
            })
            .is_err());
        assert_eq!(ledger.flows.len(), 1);
    }

    #[test]
    fn overlapping_claims_on_belt_capacity() {
        let mut ledger = OperatingLedger::default();
        ledger.add_source(SourceCapacity {
            id: "belt-1".into(),
            item: "iron-plate".into(),
            rate: rate_per_min(15), // one yellow belt
            available: Interval {
                start: 0,
                end: 36000,
            },
        });

        // First consumer takes 10/min
        ledger
            .reserve_flow(FlowClaim {
                source: "belt-1".into(),
                consumer: 1,
                item: "iron-plate".into(),
                rate: rate_per_min(10),
                interval: Interval {
                    start: 0,
                    end: 18000,
                },
            })
            .unwrap();

        // Second consumer takes 5/min in a different sub-interval = OK
        ledger
            .reserve_flow(FlowClaim {
                source: "belt-1".into(),
                consumer: 2,
                item: "iron-plate".into(),
                rate: rate_per_min(5),
                interval: Interval {
                    start: 18000,
                    end: 36000,
                },
            })
            .unwrap();

        assert_eq!(ledger.flows.len(), 2);
    }

    #[test]
    fn disjoint_claims_on_same_source() {
        let mut ledger = OperatingLedger::default();
        ledger.add_source(SourceCapacity {
            id: "coal-out".into(),
            item: "coal".into(),
            rate: rate_per_min(30),
            available: Interval {
                start: 0,
                end: 72000,
            },
        });

        // Two non-overlapping intervals should both succeed.
        ledger
            .reserve_flow(FlowClaim {
                source: "coal-out".into(),
                consumer: 1,
                item: "coal".into(),
                rate: rate_per_min(20),
                interval: Interval {
                    start: 0,
                    end: 10000,
                },
            })
            .unwrap();

        ledger
            .reserve_flow(FlowClaim {
                source: "coal-out".into(),
                consumer: 2,
                item: "coal".into(),
                rate: rate_per_min(20),
                interval: Interval {
                    start: 10000,
                    end: 20000,
                },
            })
            .unwrap();

        assert_eq!(ledger.flows.len(), 2);
    }

    #[test]
    fn stock_claim_exhausts_available() {
        let mut ledger = OperatingLedger::default();
        ledger.set_initial_stock("iron-chest", 100);

        ledger
            .reserve_stock(
                "iron-chest",
                StockClaim {
                    consumer: 1,
                    at: 1000,
                    quantity: 60,
                },
            )
            .unwrap();

        // Second claim of 50 should fail (only 40 remaining).
        assert!(ledger
            .reserve_stock(
                "iron-chest",
                StockClaim {
                    consumer: 2,
                    at: 1000,
                    quantity: 50,
                }
            )
            .is_err());
    }

    #[test]
    fn delivery_arrives_before_claim() {
        let mut ledger = OperatingLedger::default();
        ledger.set_initial_stock("coal-chest", 10);

        // Deliver 50 coal at tick 500.
        ledger.deliver_stock(
            "coal-chest",
            StockDelivery {
                at: 500,
                quantity: 50,
            },
        );

        // Claim 30 at tick 1000 (after delivery) should succeed.
        ledger
            .reserve_stock(
                "coal-chest",
                StockClaim {
                    consumer: 1,
                    at: 1000,
                    quantity: 30,
                },
            )
            .unwrap();

        // Remaining: 10 + 50 - 30 = 30. Claiming 40 should fail.
        assert!(ledger
            .reserve_stock(
                "coal-chest",
                StockClaim {
                    consumer: 2,
                    at: 1000,
                    quantity: 40,
                }
            )
            .is_err());
    }

    #[test]
    fn partial_overlap_does_not_fund_a_longer_claim() {
        let mut l = OperatingLedger::default();
        l.add_source(SourceCapacity {
            id: "iron".into(),
            item: "iron-plate".into(),
            rate: Rate::new(60, 3600).unwrap(),
            available: Interval::new(100, 200).unwrap(),
        });
        let claim = FlowClaim {
            source: "iron".into(),
            consumer: 1,
            item: "iron-plate".into(),
            rate: Rate::new(30, 3600).unwrap(),
            interval: Interval::new(50, 150).unwrap(),
        };
        assert!(l.reserve_flow(claim).is_err());
        assert!(l.flows.is_empty());
    }

    #[test]
    fn fractional_overlap_is_rejected() {
        let mut l = OperatingLedger::default();
        l.add_source(SourceCapacity {
            id: "copper".into(),
            item: "copper-plate".into(),
            rate: Rate::new(60, 3600).unwrap(),
            available: Interval::new(100, 200).unwrap(),
        });
        // Claim starts before source, ends inside = partial overlap.
        let claim = FlowClaim {
            source: "copper".into(),
            consumer: 1,
            item: "copper-plate".into(),
            rate: Rate::new(30, 3600).unwrap(),
            interval: Interval::new(50, 199).unwrap(),
        };
        assert!(l.reserve_flow(claim).is_err());
    }

    #[test]
    fn disjoint_claim_is_rejected() {
        let mut l = OperatingLedger::default();
        l.add_source(SourceCapacity {
            id: "stone".into(),
            item: "stone".into(),
            rate: Rate::new(60, 3600).unwrap(),
            available: Interval::new(100, 200).unwrap(),
        });
        // Entirely before source.
        assert!(l
            .reserve_flow(FlowClaim {
                source: "stone".into(),
                consumer: 1,
                item: "stone".into(),
                rate: Rate::new(10, 3600).unwrap(),
                interval: Interval::new(0, 50).unwrap(),
            })
            .is_err());
        // Entirely after source.
        assert!(l
            .reserve_flow(FlowClaim {
                source: "stone".into(),
                consumer: 1,
                item: "stone".into(),
                rate: Rate::new(10, 3600).unwrap(),
                interval: Interval::new(300, 400).unwrap(),
            })
            .is_err());
    }

    #[test]
    fn interval_id_overflow_is_rejected() {
        let err = Interval::new(5, 5);
        assert!(err.is_none());
        let err2 = Interval::new(10, 5);
        assert!(err2.is_none());
    }

    #[test]
    fn empty_claim_interval_is_rejected() {
        let mut l = OperatingLedger::default();
        l.add_source(SourceCapacity {
            id: "coal".into(),
            item: "coal".into(),
            rate: Rate::new(60, 3600).unwrap(),
            available: Interval::new(0, 1000).unwrap(),
        });
        // Empty interval should fail validation.
        let result = l.reserve_flow(FlowClaim {
            source: "coal".into(),
            consumer: 1,
            item: "coal".into(),
            rate: Rate::new(10, 3600).unwrap(),
            interval: Interval {
                start: 500,
                end: 500,
            },
        });
        assert!(result.is_err());
    }

    #[test]
    fn failed_claim_does_not_commit() {
        let mut l = OperatingLedger::default();
        l.add_source(SourceCapacity {
            id: "iron".into(),
            item: "iron-plate".into(),
            rate: Rate::new(30, 3600).unwrap(),
            available: Interval::new(0, 36000).unwrap(),
        });
        // Claim fits.
        l.reserve_flow(FlowClaim {
            source: "iron".into(),
            consumer: 1,
            item: "iron-plate".into(),
            rate: Rate::new(20, 3600).unwrap(),
            interval: Interval::new(0, 18000).unwrap(),
        })
        .unwrap();
        assert_eq!(l.flows.len(), 1);
        // Second claim over capacity should fail and NOT be committed.
        assert!(l
            .reserve_flow(FlowClaim {
                source: "iron".into(),
                consumer: 2,
                item: "iron-plate".into(),
                rate: Rate::new(20, 3600).unwrap(),
                interval: Interval::new(0, 18000).unwrap(),
            })
            .is_err());
        assert_eq!(l.flows.len(), 1);
    }

    #[test]
    fn missing_source_is_rejected() {
        let mut ledger = OperatingLedger::default();
        let claim = FlowClaim {
            source: "nonexistent".into(),
            consumer: 1,
            item: "iron-plate".into(),
            rate: rate_per_min(10),
            interval: Interval {
                start: 0,
                end: 1000,
            },
        };
        assert!(ledger.reserve_flow(claim).is_err());
    }

    #[test]
    fn source_outside_interval_is_rejected() {
        let mut ledger = OperatingLedger::default();
        ledger.add_source(SourceCapacity {
            id: "iron-out".into(),
            item: "iron-plate".into(),
            rate: rate_per_min(15),
            available: Interval {
                start: 5000,
                end: 10000,
            },
        });

        // Claim entirely outside the source's available interval.
        assert!(ledger
            .reserve_flow(FlowClaim {
                source: "iron-out".into(),
                consumer: 1,
                item: "iron-plate".into(),
                rate: rate_per_min(10),
                interval: Interval {
                    start: 0,
                    end: 1000
                },
            })
            .is_err());
    }
}
