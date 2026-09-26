//! Discrete compactness: cut edges and spanning trees.
//!
//! From "Discrete Geometry for Electoral Geography" by Duchin and Tenner,
//! ported from `rdapy/compactness/discrete_compactness.py`.

use crate::compactness::energy::UNASSIGNED;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutScoreError {
    /// A node that is neither out-of-state nor skippable water has no district.
    /// rdapy raises a `KeyError` here.
    UnassignedNode(usize),
}

impl std::fmt::Display for CutScoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CutScoreError::UnassignedNode(i) => {
                write!(f, "node {i} is in the graph but not in the plan")
            }
        }
    }
}

impl std::error::Error for CutScoreError {}

/// The number of edges whose endpoints lie in different districts.
///
/// `skip[i]` marks nodes excluded from the count: the virtual out-of-state
/// node, and water-only precincts that are absent from the plan.
///
/// rdapy counts each edge from both ends and halves the total, which requires
/// the adjacency to be symmetric. That halving is integer division, so an
/// asymmetric graph quietly loses the odd edge; reproduced.
pub fn cut_score(
    adjacency: &[Vec<u32>],
    district_of: &[u32],
    skip: &[bool],
) -> Result<i32, CutScoreError> {
    let mut cuts2x: i64 = 0;

    for (node, neighbors) in adjacency.iter().enumerate() {
        if skip[node] {
            continue;
        }
        if district_of[node] == UNASSIGNED {
            return Err(CutScoreError::UnassignedNode(node));
        }
        for &nb in neighbors {
            let nb = nb as usize;
            if skip[nb] {
                continue;
            }
            if district_of[nb] == UNASSIGNED {
                return Err(CutScoreError::UnassignedNode(nb));
            }
            if district_of[node] != district_of[nb] {
                cuts2x += 1;
            }
        }
    }

    Ok((cuts2x / 2) as i32)
}

/// The natural log of the number of spanning trees, by Kirchhoff's theorem.
///
/// The count itself overflows any float for a graph of real size, so this
/// returns the log determinant of the reduced Laplacian directly.
///
/// Returns `None` for a graph whose reduced Laplacian is singular -- a
/// disconnected graph, where rdapy returns negative infinity.
pub fn spanning_tree_score(adjacency: &[Vec<u32>]) -> Option<f64> {
    let n = adjacency.len();
    if n < 2 {
        return None;
    }

    // Laplacian = degree - adjacency, with the last row and column dropped.
    let m = n - 1;
    let mut a = vec![0.0f64; m * m];
    for (i, neighbors) in adjacency.iter().enumerate() {
        // rdapy builds a 0/1 adjacency matrix, so parallel edges collapse.
        let mut degree = 0.0;
        let mut seen = vec![false; n];
        for &nb in neighbors {
            let nb = nb as usize;
            if nb != i && !seen[nb] {
                seen[nb] = true;
                degree += 1.0;
                if i < m && nb < m {
                    a[i * m + nb] -= 1.0;
                }
            }
        }
        if i < m {
            a[i * m + i] += degree;
        }
    }

    log_abs_det(&mut a, m)
}

/// Log of |det| by LU decomposition with partial pivoting.
///
/// Returns `None` if the matrix is singular or the determinant is negative,
/// matching rdapy's `sign <= 0` guard.
fn log_abs_det(a: &mut [f64], n: usize) -> Option<f64> {
    let mut sign = 1.0f64;
    let mut log_det = 0.0f64;

    for k in 0..n {
        // Partial pivot.
        let mut pivot = k;
        let mut best = a[k * n + k].abs();
        for i in (k + 1)..n {
            let v = a[i * n + k].abs();
            if v > best {
                best = v;
                pivot = i;
            }
        }
        if best == 0.0 {
            return None;
        }
        if pivot != k {
            for j in 0..n {
                a.swap(k * n + j, pivot * n + j);
            }
            sign = -sign;
        }

        let d = a[k * n + k];
        if d < 0.0 {
            sign = -sign;
        }
        log_det += d.abs().ln();

        for i in (k + 1)..n {
            let factor = a[i * n + k] / d;
            if factor == 0.0 {
                continue;
            }
            for j in k..n {
                a[i * n + j] -= factor * a[k * n + j];
            }
        }
    }

    if sign <= 0.0 {
        return None;
    }
    Some(log_det)
}
