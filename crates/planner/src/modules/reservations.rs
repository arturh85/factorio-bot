//! Ground reservation system for module instances.
//!
//! A [`ReservationSet`] tracks which module instances claim which areas of a
//! surface. The planner uses it to prevent two modules from overlapping on the
//! same surface, while allowing the same module to reserve multiple sub-rectangles
//! (e.g. access corridors and escape routes alongside the footprint).
//!
//! Rectangles use half-tile coordinates (2 units per tile), matching the
//! module system's offset convention. Rectangles are half-open:
//! `[left, right)` × `[top, bottom)`.

use crate::modules::artifact::{InstanceId, ModuleError};

// ---------------------------------------------------------------------------
// HalfRect
// ---------------------------------------------------------------------------

/// A rectangle in half-tile coordinates.
///
/// Half-open: `[left, right)` × `[top, bottom)`. Uses i32 to match
/// [`Offset`](crate::modules::artifact::Offset) conventions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HalfRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl HalfRect {
    /// Create a new rectangle. Returns an error if width or height is
    /// non-positive.
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Result<Self, ModuleError> {
        if right <= left || bottom <= top {
            return Err(ModuleError::InvalidArtifact(format!(
                "HalfRect: non-positive extent ({left},{top})-({right},{bottom})"
            )));
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    /// Whether this rectangle overlaps another.
    ///
    /// Two rectangles overlap when their intervals intersect on both axes:
    /// `a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom`.
    pub fn overlaps(&self, other: &HalfRect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    /// Width in half-tiles.
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    /// Height in half-tiles.
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Whether this rectangle contains the given half-tile point (inclusive
    /// on the left/top edges, exclusive on right/bottom).
    pub fn contains(&self, hx: i32, hy: i32) -> bool {
        hx >= self.left && hx < self.right && hy >= self.top && hy < self.bottom
    }
}

// ---------------------------------------------------------------------------
// ReservationSet
// ---------------------------------------------------------------------------

/// A set of ground reservations keyed by (owner, surface, area).
///
/// Each reservation records which [`InstanceId`] owns which rectangle on
/// which surface. The planner uses this to prevent two different module
/// instances from claiming overlapping ground on the same surface.
///
/// The same owner may reserve multiple (even overlapping) rectangles on the
/// same surface — for example, a module's footprint plus its access corridor.
/// Different owners are refused overlapping reservations on the same surface.
/// Different surfaces are always independent.
#[derive(Debug, Clone, Default)]
pub struct ReservationSet {
    /// List of (owner_id, surface_name, rectangle) reservations.
    pub rects: Vec<(InstanceId, String, HalfRect)>,
}

impl ReservationSet {
    /// Reserve `area` on `surface` for `owner`.
    ///
    /// Returns `Err(ModuleError::NoSite(...))` if a *different* owner already
    /// holds an overlapping reservation on the same surface. Same-owner
    /// overlapping reservations are allowed.
    ///
    /// Different surfaces never conflict.
    pub fn reserve(
        &mut self,
        owner: InstanceId,
        surface: &str,
        area: HalfRect,
    ) -> Result<(), ModuleError> {
        // Check for conflicts: same surface, different owner, overlapping rect.
        for (existing_owner, existing_surface, existing_rect) in &self.rects {
            if existing_surface != surface {
                continue;
            }
            if *existing_owner == owner {
                // Same owner: overlapping reservations are allowed.
                continue;
            }
            if existing_rect.overlaps(&area) {
                return Err(ModuleError::NoSite(format!(
                    "reservation conflict on surface '{surface}': owner {owner} claims \
                     ({},{})-({},{}) which overlaps owner {existing_owner}'s reservation \
                     ({},{})-({},{})",
                    area.left,
                    area.top,
                    area.right,
                    area.bottom,
                    existing_rect.left,
                    existing_rect.top,
                    existing_rect.right,
                    existing_rect.bottom,
                )));
            }
        }

        self.rects.push((owner, surface.to_string(), area));
        Ok(())
    }

    /// Check whether `area` on `surface` is free (no conflicting reservation).
    ///
    /// Returns `true` if no *different* owner has a reservation overlapping
    /// `area` on the same surface. Same-owner overlaps are not considered
    /// conflicts.
    pub fn is_free(&self, owner: InstanceId, surface: &str, area: &HalfRect) -> bool {
        for (existing_owner, existing_surface, existing_rect) in &self.rects {
            if existing_surface != surface {
                continue;
            }
            if *existing_owner == owner {
                continue;
            }
            if existing_rect.overlaps(area) {
                return false;
            }
        }
        true
    }

    /// Remove all reservations owned by `owner`.
    pub fn release(&mut self, owner: InstanceId) {
        self.rects.retain(|(o, _, _)| *o != owner);
    }

    /// Remove all reservations.
    pub fn clear(&mut self) {
        self.rects.clear();
    }

    /// The number of reservations held.
    pub fn len(&self) -> usize {
        self.rects.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// Build a bounding rectangle enclosing all reservations on `surface`
    /// owned by `owner`, if any.
    pub fn bounding_rect(&self, owner: InstanceId, surface: &str) -> Option<HalfRect> {
        let mut left: Option<i32> = None;
        let mut top: Option<i32> = None;
        let mut right: Option<i32> = None;
        let mut bottom: Option<i32> = None;

        for (o, s, r) in &self.rects {
            if *o != owner || s != surface {
                continue;
            }
            left = Some(left.map_or(r.left, |v| v.min(r.left)));
            top = Some(top.map_or(r.top, |v| v.min(r.top)));
            right = Some(right.map_or(r.right, |v| v.max(r.right)));
            bottom = Some(bottom.map_or(r.bottom, |v| v.max(r.bottom)));
        }

        match (left, top, right, bottom) {
            (Some(l), Some(t), Some(r), Some(b)) => Some(HalfRect {
                left: l,
                top: t,
                right: r,
                bottom: b,
            }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_module_cannot_reuse_the_first_footprint() {
        let mut r = ReservationSet::default();
        let a = HalfRect::new(0, 0, 8, 8).unwrap();
        r.reserve(1, "nauvis", a.clone()).unwrap();
        assert!(r.reserve(2, "nauvis", a.clone()).is_err());
        assert!(r.reserve(2, "other", a).is_ok());
        assert_eq!(r.rects.len(), 2);
    }

    #[test]
    fn same_owner_can_overlap() {
        let mut r = ReservationSet::default();
        let a = HalfRect::new(0, 0, 8, 8).unwrap();
        r.reserve(1, "nauvis", a.clone()).unwrap();
        // Same owner, same surface, same rectangle: allowed.
        assert!(r.reserve(1, "nauvis", a.clone()).is_ok());
        assert_eq!(r.rects.len(), 2);
    }

    #[test]
    fn different_surfaces_never_conflict() {
        let mut r = ReservationSet::default();
        let a = HalfRect::new(0, 0, 8, 8).unwrap();
        let b = HalfRect::new(0, 0, 8, 8).unwrap();
        r.reserve(1, "nauvis", a).unwrap();
        // Same rectangle but different surface: allowed.
        assert!(r.reserve(2, "gleba", b).is_ok());
        assert_eq!(r.rects.len(), 2);
    }

    #[test]
    fn partial_overlap_is_still_a_conflict() {
        let mut r = ReservationSet::default();
        r.reserve(1, "nauvis", HalfRect::new(0, 0, 10, 10).unwrap())
            .unwrap();
        // Partially overlapping rect: refused.
        assert!(
            r.reserve(2, "nauvis", HalfRect::new(5, 5, 15, 15).unwrap())
                .is_err()
        );
        // Adjacent but not overlapping: allowed.
        assert!(
            r.reserve(2, "nauvis", HalfRect::new(10, 0, 20, 10).unwrap())
                .is_ok()
        );
    }

    #[test]
    fn half_rect_overlaps_correctly() {
        let a = HalfRect::new(0, 0, 8, 8).unwrap();
        let b = HalfRect::new(4, 4, 12, 12).unwrap();
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));

        let c = HalfRect::new(8, 0, 16, 8).unwrap();
        assert!(!a.overlaps(&c)); // a.right == c.left, so no overlap (half-open)
        assert!(!c.overlaps(&a));

        let d = HalfRect::new(0, 8, 8, 16).unwrap();
        assert!(!a.overlaps(&d)); // a.bottom == d.top, so no overlap
    }

    #[test]
    fn is_free_checks_other_owners() {
        let mut r = ReservationSet::default();
        let a = HalfRect::new(0, 0, 8, 8).unwrap();
        r.reserve(1, "nauvis", a.clone()).unwrap();

        // Same area, different owner: not free.
        assert!(!r.is_free(2, "nauvis", &a));
        // Same area, same owner: free (from the checker's perspective).
        assert!(r.is_free(1, "nauvis", &a));
        // Different surface: free.
        assert!(r.is_free(2, "vulcanus", &a));
    }

    #[test]
    fn release_removes_all_owner_reservations() {
        let mut r = ReservationSet::default();
        r.reserve(1, "nauvis", HalfRect::new(0, 0, 4, 4).unwrap())
            .unwrap();
        r.reserve(1, "nauvis", HalfRect::new(4, 0, 8, 4).unwrap())
            .unwrap();
        r.reserve(2, "nauvis", HalfRect::new(0, 4, 4, 8).unwrap())
            .unwrap();
        assert_eq!(r.rects.len(), 3);

        r.release(1);
        assert_eq!(r.rects.len(), 1);
        assert_eq!(r.rects[0].0, 2);
    }

    #[test]
    fn bounding_rect_is_correct() {
        let mut r = ReservationSet::default();
        r.reserve(1, "nauvis", HalfRect::new(0, 0, 4, 4).unwrap())
            .unwrap();
        r.reserve(1, "nauvis", HalfRect::new(6, 2, 10, 6).unwrap())
            .unwrap();

        let bbox = r.bounding_rect(1, "nauvis").unwrap();
        assert_eq!(bbox.left, 0);
        assert_eq!(bbox.top, 0);
        assert_eq!(bbox.right, 10);
        assert_eq!(bbox.bottom, 6);
    }

    #[test]
    fn half_rect_new_rejects_non_positive_extent() {
        assert!(HalfRect::new(0, 0, 0, 8).is_err());
        assert!(HalfRect::new(0, 0, 8, 0).is_err());
        assert!(HalfRect::new(0, 0, -1, 8).is_err());
    }

    #[test]
    fn multiple_owners_on_different_surfaces_never_block() {
        let mut r = ReservationSet::default();
        r.reserve(1, "nauvis", HalfRect::new(0, 0, 100, 100).unwrap())
            .unwrap();
        // Second owner on different surface: fine.
        assert!(
            r.reserve(2, "vulcanus", HalfRect::new(0, 0, 100, 100).unwrap())
                .is_ok()
        );
        // Third owner on another different surface: fine.
        assert!(
            r.reserve(3, "gleba", HalfRect::new(0, 0, 100, 100).unwrap())
                .is_ok()
        );
        // Fourth owner on original surface: blocked.
        assert!(
            r.reserve(4, "nauvis", HalfRect::new(0, 0, 10, 10).unwrap())
                .is_err()
        );
    }
}
