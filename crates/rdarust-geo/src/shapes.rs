//! Shape-based compactness, including Kaufman and King's KIWYSI model.
//!
//! Ported from `rdapy/compactness/`. This is the only part of the port that
//! needs a polygon-clipping engine: the symmetry features union a district
//! with its own mirror image, which no amount of segment matching can do.
//!
//! None of this is on the plan-scoring path. DRA reports Reock and
//! Polsby-Popper computed from aggregated area, perimeter and diameter, which
//! `rdarust-core` does without any geometry at all. These are the versions
//! that work from the shapes themselves.

use geo::algorithm::BooleanOps;
use geographiclib_rs::{Geodesic, InverseGeodesic, PolygonArea, Winding};
use rdarust_core::geometry::min_enclosing_circle;

use crate::{Geometry, Point, Polygon};

/// Area, perimeter and diameter of a shape.
///
/// Projected: square degrees, degrees, degrees. Geodesic: square kilometres,
/// kilometres, kilometres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attributes {
    pub area: f64,
    pub perimeter: f64,
    pub diameter: f64,
}

/// Measure a shape, either on the plane or on the WGS84 ellipsoid.
pub fn polygon_attributes(shape: &Geometry, geodesic: bool) -> Attributes {
    if geodesic {
        geodesic_attributes(shape)
    } else {
        projected_attributes(shape)
    }
}

/// Planar measurement, in whatever units the coordinates carry.
fn projected_attributes(shape: &Geometry) -> Attributes {
    Attributes {
        area: shape.area(),
        perimeter: shape.perimeter(),
        diameter: 2.0 * hull_circle(shape).r,
    }
}

/// The minimum circle enclosing a shape's convex hull.
fn hull_circle(shape: &Geometry) -> rdarust_core::geometry::Circle {
    let hull = shape.convex_hull();
    let pts: Vec<(f64, f64)> = hull.iter().map(|p| (p[0], p[1])).collect();
    min_enclosing_circle(&pts)
}

/// Signed geodesic perimeter and area of one ring, in metres and square
/// metres.
///
/// The sign follows the ring's winding, which is what lets holes be handled
/// by addition: a hole wound opposite its enclosing ring contributes a
/// negative area.
fn ring_geodesic(ring: &[Point]) -> (f64, f64) {
    let g = Geodesic::wgs84();
    let mut area = PolygonArea::new(&g, Winding::CounterClockwise);
    for p in ring {
        // geographiclib takes latitude first; our points are (lon, lat).
        area.add_point(p[1], p[0]);
    }
    // `sign`, not `reverse`: with sign off, a clockwise ring yields the
    // complement -- the rest of the earth's surface. Python's Compute()
    // defaults to sign=True, and Winding::CounterClockwise then matches its
    // orientation convention exactly.
    let (perimeter, area, _count) = area.compute(true);
    (perimeter, area)
}

/// Geodesic measurement of a single polygon.
fn geodesic_attributes_poly(poly: &Polygon, hull_source: &Geometry) -> Attributes {
    let (perimeter, mut area) = ring_geodesic(&poly.exterior);
    // Holes are *added*, not subtracted: their opposite winding already makes
    // their contribution negative. rdapy relies on this.
    for hole in &poly.interiors {
        let (_, hole_area) = ring_geodesic(hole);
        area += hole_area;
    }

    // Square metres to square kilometres, metres to kilometres.
    let area = area / 1_000_000.0;
    let perimeter = perimeter / 1_000.0;

    // The diameter is approximated: take the enclosing circle in degree
    // space, then measure across it geodesically along both axes and keep the
    // larger. rdapy does this; it is not a true geodesic diameter.
    let c = hull_circle(hull_source);
    let g = Geodesic::wgs84();
    let lon_s12: f64 = g.inverse(c.y, c.x - c.r, c.y, c.x + c.r);
    let lat_s12: f64 = g.inverse(c.y - c.r, c.x, c.y + c.r, c.x);
    let diameter = lon_s12.max(lat_s12) / 1_000.0;

    Attributes {
        area: area.abs(),
        perimeter: perimeter.abs(),
        diameter: diameter.abs(),
    }
}

fn geodesic_attributes(shape: &Geometry) -> Attributes {
    match shape {
        Geometry::Polygon(p) => geodesic_attributes_poly(p, shape),
        Geometry::MultiPolygon(parts) => {
            let mut area = 0.0;
            let mut perimeter = 0.0;
            for part in parts {
                let single = Geometry::Polygon(part.clone());
                let a = geodesic_attributes_poly(part, &single);
                area += a.area;
                perimeter += a.perimeter;
            }
            // The diameter comes from the whole shape's convex hull.
            let hull = Geometry::Polygon(Polygon {
                exterior: shape.convex_hull(),
                interiors: vec![],
            });
            let d = geodesic_attributes_poly(
                match &hull {
                    Geometry::Polygon(p) => p,
                    _ => unreachable!(),
                },
                &hull,
            );
            Attributes {
                area: area.abs(),
                perimeter: perimeter.abs(),
                diameter: d.diameter,
            }
        }
    }
}

// ---- conversions to and from the clipping library ----

fn to_geo(shape: &Geometry) -> geo::MultiPolygon<f64> {
    let ring = |r: &Vec<Point>| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    let poly = |p: &Polygon| {
        geo::Polygon::new(ring(&p.exterior), p.interiors.iter().map(ring).collect())
    };
    match shape {
        Geometry::Polygon(p) => geo::MultiPolygon(vec![poly(p)]),
        Geometry::MultiPolygon(ps) => geo::MultiPolygon(ps.iter().map(poly).collect()),
    }
}

fn from_geo(mp: &geo::MultiPolygon<f64>) -> Geometry {
    let ring = |ls: &geo::LineString<f64>| -> Vec<Point> {
        ls.0.iter().map(|c| [c.x, c.y]).collect()
    };
    let polys: Vec<Polygon> = mp
        .0
        .iter()
        .map(|p| Polygon {
            exterior: ring(p.exterior()),
            interiors: p.interiors().iter().map(ring).collect(),
        })
        .collect();
    if polys.len() == 1 {
        Geometry::Polygon(polys.into_iter().next().unwrap())
    } else {
        Geometry::MultiPolygon(polys)
    }
}

/// The union of two shapes.
pub fn union(a: &Geometry, b: &Geometry) -> Geometry {
    from_geo(&to_geo(a).union(&to_geo(b)))
}

// ---- the seven KIWYSI features ----

/// Mean of every outer-ring vertex.
///
/// Not the area centroid: rdapy follows the model authors' R code, which
/// averages the vertices. Holes are excluded, and the repeated closing vertex
/// of each ring is counted, both of which affect the result.
fn mean_centroid(shape: &Geometry) -> (f64, f64) {
    let mut n = 0usize;
    let (mut sx, mut sy) = (0.0, 0.0);
    for (is_exterior, ring) in shape.rings() {
        if !is_exterior {
            continue;
        }
        for p in ring {
            n += 1;
            sx += p[0];
            sy += p[1];
        }
    }
    (sx / n as f64, sy / n as f64)
}

fn reflect(shape: &Geometry, f: impl Fn(Point) -> Point + Copy) -> Geometry {
    let map_ring = |r: &Vec<Point>| r.iter().map(|p| f(*p)).collect::<Vec<Point>>();
    let map_poly = |p: &Polygon| Polygon {
        exterior: map_ring(&p.exterior),
        interiors: p.interiors.iter().map(map_ring).collect(),
    };
    match shape {
        Geometry::Polygon(p) => Geometry::Polygon(map_poly(p)),
        Geometry::MultiPolygon(ps) => Geometry::MultiPolygon(ps.iter().map(map_poly).collect()),
    }
}

/// How much bigger a shape gets when unioned with its own mirror image.
///
/// 1 for a perfectly symmetric shape, up to 2 for a wholly asymmetric one.
fn symmetry(shape: &Geometry, mirrored: &Geometry, geodesic: bool) -> f64 {
    let combined = union(shape, mirrored);
    polygon_attributes(&combined, geodesic).area / polygon_attributes(shape, geodesic).area
}

/// X-symmetry: reflected about a vertical line through the centroid.
pub fn calc_sym_x(shape: &Geometry, geodesic: bool) -> f64 {
    let (cx, _) = mean_centroid(shape);
    let mirrored = reflect(shape, |p| [2.0 * cx - p[0], p[1]]);
    symmetry(shape, &mirrored, geodesic)
}

/// Y-symmetry: reflected about a horizontal line through the centroid.
pub fn calc_sym_y(shape: &Geometry, geodesic: bool) -> f64 {
    let (_, cy) = mean_centroid(shape);
    let mirrored = reflect(shape, |p| [p[0], 2.0 * cy - p[1]]);
    symmetry(shape, &mirrored, geodesic)
}

/// Reock: area over the area of the minimum enclosing circle.
pub fn calc_reock(shape: &Geometry, geodesic: bool) -> f64 {
    let a = polygon_attributes(shape, geodesic);
    rdarust_core::compactness::reock_formula(a.area, a.diameter / 2.0)
}

/// Polsby-Popper: area over the area of a circle with the same perimeter.
pub fn calc_polsby(shape: &Geometry, geodesic: bool) -> f64 {
    let a = polygon_attributes(shape, geodesic);
    rdarust_core::compactness::polsby_formula(a.area, a.perimeter)
}

/// Convex hull ratio: area over the area of the convex hull.
pub fn calc_hull(shape: &Geometry, geodesic: bool) -> f64 {
    let hull = Geometry::Polygon(Polygon {
        exterior: shape.convex_hull(),
        interiors: vec![],
    });
    polygon_attributes(shape, geodesic).area / polygon_attributes(&hull, geodesic).area
}

/// Schwartzberg: perimeter over the circumference of an equal-area circle.
///
/// Note this is perimeter over circumference, the reciprocal of the ratio
/// usually published under that name; rdapy follows the model authors' code
/// rather than the usual definition.
pub fn calc_schwartzberg(shape: &Geometry, geodesic: bool) -> f64 {
    let a = polygon_attributes(shape, geodesic);
    a.perimeter / ((2.0 * std::f64::consts::PI) * (a.area / std::f64::consts::PI).sqrt())
}

/// Bounding-box ratio: area over the area of the smallest enclosing
/// rectangle, at any rotation.
pub fn calc_bbox(shape: &Geometry, geodesic: bool) -> f64 {
    let pts: Vec<Point> = shape
        .rings()
        .filter(|(is_exterior, _)| *is_exterior)
        .flat_map(|(_, r)| r.iter().copied())
        .collect();
    let rect = minimum_bounding_rectangle(&pts);
    let rect_shape = Geometry::Polygon(Polygon {
        exterior: {
            let mut r = rect.to_vec();
            r.push(rect[0]);
            r
        },
        interiors: vec![],
    });
    polygon_attributes(shape, geodesic).area / polygon_attributes(&rect_shape, geodesic).area
}

/// The smallest-area rectangle enclosing a set of points, at any rotation.
///
/// A minimum-area rectangle always has a side flush with an edge of the
/// convex hull, so it is enough to try each hull edge's direction.
pub fn minimum_bounding_rectangle(points: &[Point]) -> [Point; 4] {
    let mut sorted: Vec<Point> = points.to_vec();
    sorted.sort_by(|a, b| {
        a[0].partial_cmp(&b[0])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
    });
    sorted.dedup_by(|a, b| a == b);
    let hull = crate::convex_hull(&sorted);
    if hull.len() < 3 {
        let p = *hull.first().unwrap_or(&[0.0, 0.0]);
        return [p, p, p, p];
    }

    // Edge directions, reduced modulo a quarter turn: a rectangle aligned to
    // an edge is the same rectangle whichever of its four sides is flush.
    let quarter = std::f64::consts::FRAC_PI_2;
    let mut angles: Vec<f64> = hull
        .windows(2)
        .map(|w| {
            let a = (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0]);
            a.rem_euclid(quarter).abs()
        })
        .collect();
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    angles.dedup();

    let mut best: Option<(f64, [Point; 4])> = None;
    for a in angles {
        let (c, s) = (a.cos(), a.sin());
        // Rotation taking the edge direction onto an axis.
        let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &hull {
            let x = c * p[0] + s * p[1];
            let y = -s * p[0] + c * p[1];
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        let area = (max_x - min_x) * (max_y - min_y);
        if best.as_ref().is_none_or(|(b, _)| area < *b) {
            // Back to the original frame.
            let back = |x: f64, y: f64| -> Point { [c * x - s * y, s * x + c * y] };
            best = Some((
                area,
                [
                    back(max_x, min_y),
                    back(min_x, min_y),
                    back(min_x, max_y),
                    back(max_x, max_y),
                ],
            ));
        }
    }
    best.expect("a hull has at least one edge").1
}

// ---- the KIWYSI model ----

/// The seven features, in the order the model expects.
pub fn featureize_shape(shape: &Geometry, geodesic: bool) -> [f64; 7] {
    [
        calc_sym_x(shape, geodesic),
        calc_sym_y(shape, geodesic),
        calc_reock(shape, geodesic),
        calc_bbox(shape, geodesic),
        calc_polsby(shape, geodesic),
        calc_hull(shape, geodesic),
        calc_schwartzberg(shape, geodesic),
    ]
}

/// The corrected model, revised 2021-01-25.
const REVISED: [f64; 7] = [
    3.0428861122, 4.5060390447, -22.7768820155, -24.1176096770, -107.9434473497,
    -67.1088897240, -1.2981693414,
];
const REVISED_INTERCEPT: f64 = 145.6420811716;

/// The original model, which rdapy keeps only so its tests can show that the
/// features themselves are computed correctly.
const ORIGINAL: [f64; 7] = [
    0.317566717356693, 0.32545234315137, 0.32799567316863, 0.411560782484889,
    0.412187169816954, 0.420085928286392, 0.412187169816954,
];

/// Turn the features into a rank. Smaller is more compact.
pub fn score_features(features: &[f64; 7], revised: bool) -> f64 {
    let dot = |m: &[f64; 7]| -> f64 {
        // Plain accumulation, matching numpy's dot for small vectors.
        features.iter().zip(m.iter()).map(|(f, c)| f * c).sum()
    };
    if revised {
        dot(&REVISED) + REVISED_INTERCEPT
    } else {
        dot(&ORIGINAL) * 11.0 + 50.0
    }
}

/// Constrain a rank to [1, 100].
pub fn trim_kiwysi_rank(raw: f64) -> f64 {
    raw.clamp(1.0, 100.0)
}

/// Rank a shape [1-100] for how compact it looks. Smaller is better.
pub fn kiwysi_rank_shape(shape: &Geometry, geodesic: bool, revised: bool) -> f64 {
    score_features(&featureize_shape(shape, geodesic), revised)
}

/// Reock, Polsby-Popper and KIWYSI for one district.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistrictCompactness {
    pub reock: f64,
    pub polsby: f64,
    pub kiwysi_rank: Option<f64>,
}

/// Compactness for a whole plan, from its district shapes.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactnessMetrics {
    pub avg_reock: f64,
    pub avg_polsby: f64,
    pub avg_kiwysi: Option<i64>,
    pub by_district: Vec<DistrictCompactness>,
}

/// Measure every district.
///
/// Reock and Polsby-Popper are planar here, matching what DRA reports. KIWYSI
/// is geodesic and is most of the cost, which is why it can be skipped.
pub fn calc_compactness_metrics(shapes: &[Geometry], kiwysi: bool) -> CompactnessMetrics {
    let mut tot_reock = 0.0;
    let mut tot_polsby = 0.0;
    let mut tot_kiwysi = 0.0;
    let mut by_district = Vec::with_capacity(shapes.len());

    for shape in shapes {
        let reock = calc_reock(shape, false);
        let polsby = calc_polsby(shape, false);
        tot_reock += reock;
        tot_polsby += polsby;

        let kiwysi_rank = kiwysi.then(|| {
            let r = trim_kiwysi_rank(kiwysi_rank_shape(shape, true, true));
            tot_kiwysi += r;
            r
        });
        by_district.push(DistrictCompactness { reock, polsby, kiwysi_rank });
    }

    let n = shapes.len() as f64;
    CompactnessMetrics {
        avg_reock: tot_reock / n,
        avg_polsby: tot_polsby / n,
        avg_kiwysi: kiwysi
            .then(|| rdarust_core::numeric::python_round(tot_kiwysi / n) as i64),
        by_district,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(lon: f64, lat: f64, w: f64) -> Vec<Point> {
        vec![
            [lon, lat], [lon + w, lat], [lon + w, lat + w], [lon, lat + w], [lon, lat],
        ]
    }

    /// Locks in the geodesic sign convention.
    ///
    /// The area must be signed by winding and must be the polygon's own area,
    /// not the complement. Getting this wrong yields the surface of the earth
    /// less the polygon -- about 5.1e14 square metres -- which is wrong by
    /// seven orders of magnitude rather than by a little.
    #[test]
    fn geodesic_area_is_signed_by_winding_and_is_not_the_complement() {
        let ccw = square(-79.0, 35.0, 0.1);
        let cw: Vec<Point> = ccw.iter().rev().copied().collect();

        let (_, a_ccw) = ring_geodesic(&ccw);
        let (_, a_cw) = ring_geodesic(&cw);

        assert!(a_ccw > 0.0, "counter-clockwise area must be positive");
        assert!(a_cw < 0.0, "clockwise area must be negative");
        assert!((a_ccw + a_cw).abs() < 1e-3, "the two must be negatives");
        // A tenth of a degree square near 35N is roughly 100 square km.
        assert!(
            (a_ccw / 1e6 - 101.2).abs() < 1.0,
            "area should be about 101 km^2, got {}",
            a_ccw / 1e6
        );
    }

    #[test]
    fn a_hole_reduces_the_geodesic_area() {
        let outer = Polygon { exterior: square(-79.0, 35.0, 0.2), interiors: vec![] };
        let holed = Polygon {
            exterior: square(-79.0, 35.0, 0.2),
            // Wound the other way, as GeoJSON holes are.
            interiors: vec![square(-78.95, 35.05, 0.1).into_iter().rev().collect()],
        };
        let whole = Geometry::Polygon(outer);
        let with_hole = Geometry::Polygon(holed);
        let a = polygon_attributes(&whole, true).area;
        let b = polygon_attributes(&with_hole, true).area;
        assert!(b < a, "the hole must reduce the area: {b} vs {a}");
        assert!((a - b - 101.2).abs() < 1.5, "the hole is about 101 km^2");
    }
}
