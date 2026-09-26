//! Planar geometry for redistricting preprocessing.
//!
//! This covers exactly the operations `extract_data.py` and
//! `extract_graph.py` need from shapely: area, perimeter, convex hull, and
//! the length of the boundary two adjacent precincts share.
//!
//! There is deliberately no polygon-clipping engine here. The last of those
//! looks like it needs one -- rdapy computes it as
//! `a.intersection(b).length` -- but precincts form a coverage, so adjacent
//! ones share *identical* boundary segments. Matching those segments gives
//! the same answer with no robustness predicates to get wrong: measured
//! against rdapy's own output for all 15,058 adjacent pairs in North
//! Carolina, the largest disagreement is 7.8e-16 degrees.
//!
//! Coordinates are longitude and latitude in degrees, and everything here is
//! planar. That is what rdapy does; area and perimeter are only ever used as
//! ratios, where the distortion cancels.

use std::collections::HashMap;

pub mod segments;

pub use segments::{shared_border, BoundarySegments, SegmentKey};

/// A vertex, as (longitude, latitude).
pub type Point = [f64; 2];

/// A closed linear ring: the first and last vertices coincide.
pub type Ring = Vec<Point>;

/// A polygon: one outer ring, and a hole for each inner ring.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Polygon {
    pub exterior: Ring,
    pub interiors: Vec<Ring>,
}

/// One precinct's shape. GeoJSON gives us either form.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    Polygon(Polygon),
    MultiPolygon(Vec<Polygon>),
}

impl Geometry {
    /// Every ring, flagged with whether it is an outer ring.
    pub fn rings(&self) -> impl Iterator<Item = (bool, &Ring)> {
        let polys: &[Polygon] = match self {
            Geometry::Polygon(p) => std::slice::from_ref(p),
            Geometry::MultiPolygon(ps) => ps,
        };
        polys.iter().flat_map(|p| {
            std::iter::once((true, &p.exterior))
                .chain(p.interiors.iter().map(|r| (false, r)))
        })
    }

    /// Area in square degrees, holes subtracted.
    ///
    /// The shoelace sum is taken about the ring's first vertex. Without that
    /// shift, cross-products at North Carolina's longitude are around 2,800
    /// while the answer is around 1e-4, so the sum cancels away eight digits
    /// -- enough to put the result outside this port's tolerance. GEOS makes
    /// the same shift internally.
    pub fn area(&self) -> f64 {
        let mut a = 0.0;
        for (is_exterior, ring) in self.rings() {
            let r = ring_area(ring);
            a += if is_exterior { r } else { -r };
        }
        a
    }

    /// Total boundary length in degrees, including the boundaries of holes,
    /// matching shapely's `length`.
    pub fn perimeter(&self) -> f64 {
        self.rings()
            .map(|(_, ring)| ring_perimeter(ring))
            .sum()
    }

    /// The convex hull, as a closed ring.
    ///
    /// Returned closed and starting at the lexicographically smallest vertex,
    /// so the result is canonical rather than dependent on input order.
    pub fn convex_hull(&self) -> Ring {
        let mut pts: Vec<Point> = self
            .rings()
            .flat_map(|(_, r)| r.iter().copied())
            .collect();
        pts.sort_by(cmp_point);
        pts.dedup_by(|a, b| a == b);
        convex_hull(&pts)
    }

    /// Every vertex, for sizing and iteration.
    pub fn vertex_count(&self) -> usize {
        self.rings().map(|(_, r)| r.len()).sum()
    }
}

fn cmp_point(a: &Point, b: &Point) -> std::cmp::Ordering {
    a[0]
        .partial_cmp(&b[0])
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
}

/// Unsigned area of one ring, about its first vertex.
pub fn ring_area(ring: &Ring) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }
    let [x0, y0] = ring[0];
    let mut s = 0.0;
    for w in ring.windows(2) {
        let ([x1, y1], [x2, y2]) = (w[0], w[1]);
        s += (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
    }
    s.abs() / 2.0
}

pub fn ring_perimeter(ring: &Ring) -> f64 {
    ring.windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

/// Monotone-chain convex hull of pre-sorted, deduplicated points.
///
/// Returns a closed ring. Collinear points are dropped, as GEOS does.
pub fn convex_hull(sorted: &[Point]) -> Ring {
    if sorted.len() < 3 {
        let mut r = sorted.to_vec();
        if !r.is_empty() {
            r.push(r[0]);
        }
        return r;
    }

    fn half(points: impl Iterator<Item = Point>) -> Vec<Point> {
        let mut out: Vec<Point> = Vec::new();
        for p in points {
            while out.len() >= 2 {
                let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
                // Keep only strict right turns, so collinear runs collapse.
                if (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) > 0.0 {
                    break;
                }
                out.pop();
            }
            out.push(p);
        }
        out
    }

    let mut lower = half(sorted.iter().copied());
    let upper = half(sorted.iter().rev().copied());
    lower.pop();
    let mut hull = lower;
    hull.extend(upper);
    // `upper` ends where `lower` began, so the ring is already closed.
    hull
}

/// Which precincts border which, and by how much.
///
/// Built once for a whole state: every boundary segment is indexed by the
/// precincts that carry it, so adjacency and shared-border length both fall
/// out of the same table.
pub struct Coverage {
    boundaries: Vec<BoundarySegments>,
    /// Segment to the precincts carrying it. A segment in a clean coverage is
    /// carried by one precinct (a state border) or two (an internal border).
    owners: HashMap<SegmentKey, Vec<u32>>,
}

impl Coverage {
    pub fn build(geometries: &[Geometry]) -> Self {
        let boundaries: Vec<BoundarySegments> =
            geometries.iter().map(BoundarySegments::of).collect();

        let mut owners: HashMap<SegmentKey, Vec<u32>> = HashMap::new();
        for (i, b) in boundaries.iter().enumerate() {
            for key in b.keys() {
                owners.entry(*key).or_default().push(i as u32);
            }
        }
        Coverage { boundaries, owners }
    }

    pub fn len(&self) -> usize {
        self.boundaries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boundaries.is_empty()
    }

    /// Precincts sharing at least one boundary segment with `i`, sorted.
    ///
    /// This is rook adjacency: sharing an edge, not merely a corner, which is
    /// what `libpysal`'s `Rook` computes.
    pub fn neighbors(&self, i: u32) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        for key in self.boundaries[i as usize].keys() {
            for &owner in &self.owners[key] {
                if owner != i {
                    out.push(owner);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Length of the boundary `i` and `j` share.
    pub fn shared_border(&self, i: u32, j: u32) -> f64 {
        shared_border(&self.boundaries[i as usize], &self.boundaries[j as usize])
    }

    pub fn boundary(&self, i: u32) -> &BoundarySegments {
        &self.boundaries[i as usize]
    }

    /// Segments carried by more than two precincts.
    ///
    /// A clean coverage has none. Any hits mean overlapping polygons, and the
    /// shared-border lengths for those should not be trusted.
    pub fn overshared_segments(&self) -> usize {
        self.owners.values().filter(|o| o.len() > 2).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, w: f64) -> Geometry {
        Geometry::Polygon(Polygon {
            exterior: vec![[x, y], [x + w, y], [x + w, y + w], [x, y + w], [x, y]],
            interiors: vec![],
        })
    }

    #[test]
    fn area_and_perimeter_of_a_square() {
        let s = square(0.0, 0.0, 2.0);
        assert!((s.area() - 4.0).abs() < 1e-15);
        assert!((s.perimeter() - 8.0).abs() < 1e-15);
    }

    #[test]
    fn holes_are_subtracted_but_add_to_the_perimeter() {
        let g = Geometry::Polygon(Polygon {
            exterior: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]],
            interiors: vec![vec![
                [1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0], [1.0, 1.0],
            ]],
        });
        assert!((g.area() - 15.0).abs() < 1e-15, "16 less the unit hole");
        // shapely's `length` counts the hole's boundary too.
        assert!((g.perimeter() - 20.0).abs() < 1e-15);
    }

    #[test]
    fn area_survives_far_from_the_origin() {
        // The reason the shoelace sum is taken about the first vertex: at
        // these coordinates the naive form loses eight digits to cancellation.
        let s = square(-79.123456789, 35.987654321, 1e-4);
        let expected = 1e-8;
        let err = (s.area() - expected).abs() / expected;
        assert!(err < 1e-9, "relative error {err:e} at realistic coordinates");
    }

    #[test]
    fn convex_hull_drops_interior_and_collinear_points() {
        let g = Geometry::Polygon(Polygon {
            exterior: vec![
                [0.0, 0.0], [1.0, 0.0], [2.0, 0.0], // collinear along the bottom
                [2.0, 2.0], [0.0, 2.0],
                [1.0, 1.0],                          // interior
                [0.0, 0.0],
            ],
            interiors: vec![],
        });
        let hull = g.convex_hull();
        let corners: std::collections::HashSet<[u64; 2]> = hull
            .iter()
            .map(|p| [p[0].to_bits(), p[1].to_bits()])
            .collect();
        assert_eq!(corners.len(), 4, "only the four corners survive: {hull:?}");
        assert_eq!(hull.first(), hull.last(), "the hull ring is closed");
    }

    #[test]
    fn adjacent_squares_share_their_common_edge() {
        let a = square(0.0, 0.0, 1.0);
        let b = square(1.0, 0.0, 1.0);
        let cov = Coverage::build(&[a, b]);
        assert_eq!(cov.neighbors(0), vec![1]);
        assert!((cov.shared_border(0, 1) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn squares_touching_at_a_corner_are_not_rook_neighbours() {
        // Queen adjacency would count this; rook, which is what rdapy uses,
        // does not.
        let a = square(0.0, 0.0, 1.0);
        let b = square(1.0, 1.0, 1.0);
        let cov = Coverage::build(&[a, b]);
        assert!(cov.neighbors(0).is_empty());
        assert_eq!(cov.shared_border(0, 1), 0.0);
    }

    #[test]
    fn separated_squares_share_nothing() {
        let cov = Coverage::build(&[square(0.0, 0.0, 1.0), square(5.0, 5.0, 1.0)]);
        assert!(cov.neighbors(0).is_empty());
        assert_eq!(cov.shared_border(0, 1), 0.0);
    }

    #[test]
    fn a_segment_is_the_same_edge_from_either_side() {
        let f = SegmentKey::new([1.0, 2.0], [3.0, 4.0]);
        let r = SegmentKey::new([3.0, 4.0], [1.0, 2.0]);
        assert_eq!(f, r, "direction of travel must not change the key");
    }

    #[test]
    fn overlapping_shapes_are_detected() {
        // Three squares all carrying the same edge: not a valid coverage.
        let cov = Coverage::build(&[
            square(0.0, 0.0, 1.0),
            square(1.0, 0.0, 1.0),
            square(1.0, 0.0, 1.0),
        ]);
        assert!(cov.overshared_segments() > 0);
    }
}
