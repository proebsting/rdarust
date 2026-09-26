//! Population compactness, or "energy". Ported from
//! `rdapy/compactness/energy.py`.
//!
//! The population-weighted moment of inertia about each district's
//! population centroid: lower means people sit closer to their district's
//! centre.
//!
//! Note that rdapy measures distance in raw degrees of longitude and
//! latitude, not a projected distance, so a degree of longitude counts the
//! same as a degree of latitude regardless of where the state is. Reproduced
//! as-is; see KNOWN-DIFFERENCES.md.

/// Marks a precinct that is not assigned to any district.
pub const UNASSIGNED: u32 = u32::MAX;

/// Squared distance between two (lon, lat) points, in square degrees.
#[inline]
fn squared_distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.1 - b.1) * (a.1 - b.1) + (a.0 - b.0) * (a.0 - b.0)
}

/// Population-weighted centroid of each district, indexed 1..=n.
///
/// Returns `None` if a district in `1..=max` has no population, where rdapy
/// divides by zero.
pub fn district_centroids(
    district_of: &[u32],
    pops: &[i64],
    centers: &[(f64, f64)],
) -> Option<Vec<(f64, f64)>> {
    let max_district = district_of
        .iter()
        .filter(|&&d| d != UNASSIGNED)
        .copied()
        .max()? as usize;

    let mut w_lon = vec![0.0f64; max_district + 1];
    let mut w_lat = vec![0.0f64; max_district + 1];
    let mut pop_by = vec![0i64; max_district + 1];

    for (i, &d) in district_of.iter().enumerate() {
        if d == UNASSIGNED {
            continue;
        }
        let d = d as usize;
        pop_by[d] += pops[i];
        w_lon[d] += centers[i].0 * pops[i] as f64;
        w_lat[d] += centers[i].1 * pops[i] as f64;
    }

    let mut out = vec![(0.0, 0.0); max_district + 1];
    for d in 1..=max_district {
        if pop_by[d] == 0 {
            return None;
        }
        out[d] = (
            w_lon[d] / pop_by[d] as f64,
            w_lat[d] / pop_by[d] as f64,
        );
    }
    Some(out)
}

/// The "energy" of a plan.
///
/// `district_of[i]` is the 1-based district of precinct `i`, or
/// [`UNASSIGNED`]. Precinct order must match rdapy's input order, since
/// floating-point addition is not associative.
pub fn calc_energy(district_of: &[u32], pops: &[i64], centers: &[(f64, f64)]) -> Option<f64> {
    let centroids = district_centroids(district_of, pops, centers)?;

    let mut energy = 0.0f64;
    for (i, &d) in district_of.iter().enumerate() {
        if d == UNASSIGNED {
            continue;
        }
        energy += pops[i] as f64 * squared_distance(centroids[d as usize], centers[i]);
    }
    Some(energy)
}
