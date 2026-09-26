//! Boundary segments, and the borders two shapes share.
//!
//! A segment is keyed by its two endpoints in a canonical order, so the same
//! edge traversed in opposite directions by two neighbouring precincts hashes
//! to one key.

use std::collections::HashMap;

use crate::{Geometry, Point};

/// A boundary segment, identified by its endpoints.
///
/// Bit patterns rather than floats, so it can be hashed. Coordinates come
/// from the same file for both precincts of a shared edge, so they are
/// bit-identical and compare exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SegmentKey([u64; 4]);

impl SegmentKey {
    pub fn new(a: Point, b: Point) -> Self {
        let a = normalize(a);
        let b = normalize(b);
        // Canonical order, so direction of travel does not matter.
        let (p, q) = if (a[0], a[1]) <= (b[0], b[1]) { (a, b) } else { (b, a) };
        SegmentKey([
            p[0].to_bits(),
            p[1].to_bits(),
            q[0].to_bits(),
            q[1].to_bits(),
        ])
    }
}

/// Map -0.0 to 0.0, which compares equal but hashes differently.
#[inline]
fn normalize(p: Point) -> Point {
    [if p[0] == 0.0 { 0.0 } else { p[0] }, if p[1] == 0.0 { 0.0 } else { p[1] }]
}

/// One shape's boundary, as segments with their lengths.
///
/// Lengths accumulate per key, so a shape that traverses the same segment
/// twice -- a pinched boundary -- counts it twice, as shapely does.
#[derive(Debug, Clone, Default)]
pub struct BoundarySegments {
    lengths: HashMap<SegmentKey, f64>,
}

impl BoundarySegments {
    pub fn of(geometry: &Geometry) -> Self {
        let mut lengths: HashMap<SegmentKey, f64> =
            HashMap::with_capacity(geometry.vertex_count());
        for (_, ring) in geometry.rings() {
            for w in ring.windows(2) {
                let (a, b) = (w[0], w[1]);
                if a == b {
                    continue;
                }
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                *lengths.entry(SegmentKey::new(a, b)).or_insert(0.0) += len;
            }
        }
        BoundarySegments { lengths }
    }

    pub fn keys(&self) -> impl Iterator<Item = &SegmentKey> {
        self.lengths.keys()
    }

    pub fn len(&self) -> usize {
        self.lengths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lengths.is_empty()
    }

    /// Total length of every segment, i.e. the perimeter.
    pub fn total_length(&self) -> f64 {
        self.lengths.values().sum()
    }
}

/// Length of the boundary two shapes share.
///
/// Equivalent to `a.intersection(b).length` for shapes in a coverage. The
/// smaller boundary is scanned, so a precinct with few vertices is not made
/// to pay for a neighbour with many.
pub fn shared_border(a: &BoundarySegments, b: &BoundarySegments) -> f64 {
    let (small, large) = if a.lengths.len() <= b.lengths.len() {
        (a, b)
    } else {
        (b, a)
    };
    // Sorted so the sum accumulates in a fixed order regardless of hashing.
    let mut shared: Vec<f64> = small
        .lengths
        .iter()
        .filter(|(k, _)| large.lengths.contains_key(*k))
        .map(|(_, &l)| l)
        .collect();
    shared.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    shared.iter().sum()
}
