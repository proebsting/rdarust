//! Compactness, ported from `rdapy/compactness/`.
//!
//! Only the parts the scoring pipeline needs are here: the Reock and
//! Polsby-Popper formulas (which work from aggregated area, perimeter and
//! diameter, not from geometry), population compactness, and the discrete
//! graph measures. Shape-based compactness and the KIWYSI model need a
//! geometry library and come in a later phase.

pub mod discrete;
pub mod energy;

use std::f64::consts::PI;

/// Reock: district area over the area of its minimum bounding circle.
///
/// Higher is more compact; the maximum of 1 is a perfect circle.
#[inline]
pub fn reock_formula(area: f64, radius: f64) -> f64 {
    area / (PI * radius.powi(2))
}

/// Polsby-Popper: district area over the area of a circle with the same
/// perimeter.
///
/// Higher is more compact. Punishes ragged boundaries much harder than Reock.
#[inline]
pub fn polsby_formula(area: f64, perimeter: f64) -> f64 {
    (4.0 * PI) * (area / perimeter.powi(2))
}
