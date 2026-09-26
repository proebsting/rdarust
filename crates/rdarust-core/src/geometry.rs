//! Minimum enclosing circle (Welzl's algorithm).
//!
//! rdapy's `wl_make_circle` dedupes its input, shuffles it with a seeded
//! *global* RNG, then runs an iterative Welzl. Because the RNG is global and
//! shared, its results depend on how many prior calls consumed RNG state, and
//! on a colinear-triple failure it reshuffles and retries up to ten times.
//!
//! The minimum enclosing circle is mathematically unique, so none of that
//! affects the answer -- only the running time and the floating-point noise.
//! This port is therefore fully deterministic: it uses a fixed-seed PRNG for
//! the permutation (Welzl is correct for *any* permutation; randomisation only
//! buys expected-linear time) and handles colinear triples directly instead of
//! retrying. Results agree with rdapy to well within the 1e-9 bar.

/// A circle in the plane. For geographic use the coordinates are (lon, lat)
/// degrees, matching rdapy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

impl Circle {
    pub const EMPTY: Circle = Circle {
        x: 0.0,
        y: 0.0,
        r: 0.0,
    };

    /// Tolerant containment test.
    ///
    /// rdapy compares `hypot(dx, dy) <= r` with no slack, which can loop on
    /// degenerate input. A small relative slack is used here; since the
    /// enclosing circle is unique, the slack affects only how much redundant
    /// work the search does, not which circle it converges to.
    #[inline]
    fn contains(&self, p: (f64, f64)) -> bool {
        let d = (self.x - p.0).hypot(self.y - p.1);
        d - self.r <= IN_EPS * self.r.abs().max(1.0)
    }
}

const IN_EPS: f64 = 1e-13;

/// Colinearity threshold, matching rdapy's `circleRadius`.
const DET_EPS: f64 = 1.0e-10;

/// The minimum enclosing circle of a set of points.
///
/// Equivalent to rdapy's `wl_make_circle`. An empty input yields a
/// zero-radius circle at the origin, as rdapy's does.
pub fn min_enclosing_circle(points: &[(f64, f64)]) -> Circle {
    let pts = dedupe(points);
    if pts.is_empty() {
        return Circle::EMPTY;
    }
    welzl(&shuffled(pts))
}

/// Remove exact duplicates, preserving first-seen order.
///
/// Districts are assembled from per-precinct convex hulls, so shared boundary
/// vertices appear many times; deduping matters for speed as well as parity
/// with rdapy, which passes its points through a `set`.
fn dedupe(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut seen = std::collections::HashSet::with_capacity(points.len());
    let mut out = Vec::with_capacity(points.len());
    for &(x, y) in points {
        // Normalise -0.0 to 0.0 so it hashes with its equal partner.
        let key = (
            (if x == 0.0 { 0.0 } else { x }).to_bits(),
            (if y == 0.0 { 0.0 } else { y }).to_bits(),
        );
        if seen.insert(key) {
            out.push((x, y));
        }
    }
    out
}

/// Fisher-Yates with a fixed-seed xorshift64*, so the permutation -- and
/// therefore the floating-point result -- is identical on every run and every
/// platform.
fn shuffled(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut state: u64 = 0x2545F4914F6CDD1D;
    for i in (1..pts.len()).rev() {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let r = state.wrapping_mul(0x2545F4914F6CDD1D);
        pts.swap(i, (r % (i as u64 + 1)) as usize);
    }
    pts
}

/// Welzl's algorithm in its iterative three-loop form. No recursion, so deep
/// point sets (a large district exterior can be tens of thousands of points)
/// cannot blow the stack -- the reason rdapy needed its own non-recursive
/// rewrite.
fn welzl(pts: &[(f64, f64)]) -> Circle {
    let mut c = Circle {
        x: pts[0].0,
        y: pts[0].1,
        r: 0.0,
    };

    for i in 1..pts.len() {
        if c.contains(pts[i]) {
            continue;
        }
        c = Circle {
            x: pts[i].0,
            y: pts[i].1,
            r: 0.0,
        };
        for j in 0..i {
            if c.contains(pts[j]) {
                continue;
            }
            c = from_two(pts[i], pts[j]);
            for k in 0..j {
                if c.contains(pts[k]) {
                    continue;
                }
                c = from_three(pts[i], pts[j], pts[k]);
            }
        }
    }
    c
}

#[inline]
fn from_two(a: (f64, f64), b: (f64, f64)) -> Circle {
    Circle {
        x: (a.0 + b.0) / 2.0,
        y: (a.1 + b.1) / 2.0,
        r: (a.0 - b.0).hypot(a.1 - b.1) / 2.0,
    }
}

/// Circumcircle of three points.
///
/// The arithmetic is rdapy's `circleRadius` verbatim, so that a given
/// defining triple yields bit-identical output. Where rdapy raises on a
/// colinear triple and reshuffles, this falls back to the smallest two-point
/// circle covering all three -- which is the correct answer in that case.
fn from_three(b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> Circle {
    let temp = c.0 * c.0 + c.1 * c.1;
    let bc = (b.0 * b.0 + b.1 * b.1 - temp) / 2.0;
    let cd = (temp - d.0 * d.0 - d.1 * d.1) / 2.0;
    let det = (b.0 - c.0) * (c.1 - d.1) - (c.0 - d.0) * (b.1 - c.1);

    if det.abs() < DET_EPS {
        return colinear_fallback(b, c, d);
    }

    let cx = (bc * (c.1 - d.1) - cd * (b.1 - c.1)) / det;
    let cy = ((b.0 - c.0) * cd - (c.0 - d.0) * bc) / det;
    let r = ((cx - b.0).powi(2) + (cy - b.1).powi(2)).sqrt();

    Circle { x: cx, y: cy, r }
}

/// For a degenerate (colinear or near-colinear) triple, the enclosing circle
/// is determined by the two extreme points.
fn colinear_fallback(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> Circle {
    let mut best: Option<Circle> = None;
    for (p, q) in [(a, b), (a, c), (b, c)] {
        let cand = from_two(p, q);
        if cand.contains(a) && cand.contains(b) && cand.contains(c) {
            best = match best {
                Some(cur) if cur.r <= cand.r => Some(cur),
                _ => Some(cand),
            };
        }
    }
    best.unwrap_or(Circle::EMPTY)
}
