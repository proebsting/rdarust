//! Not-a-knot cubic spline interpolation.
//!
//! `scipy.interpolate.interp1d(x, y, kind="cubic")` is bit-identical to
//! `make_interp_spline(x, y, k=3)`, i.e. a cubic B-spline with **not-a-knot**
//! end conditions (verified empirically against SciPy 1.18: max difference
//! 0.0). It is *not* a natural spline -- using natural end conditions is off
//! by ~1e-2 on realistic seats-votes data, which would be catastrophic here.
//!
//! rdapy uses it twice, both in the partisan suite:
//!
//! * `est_votes_bias` interpolates votes as a function of seats,
//! * `est_geometric_seats_bias` interpolates the bias curve at the statewide
//!   vote share.

use core::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum SplineError {
    /// A cubic interpolant needs at least 4 knots, matching SciPy's
    /// `interp1d(kind="cubic")`.
    TooFewPoints(usize),
    /// `x` and `y` had different lengths.
    LengthMismatch { x: usize, y: usize },
    /// `x` was not strictly increasing.
    NotStrictlyIncreasing(usize),
    /// Evaluated outside `[x[0], x[n-1]]`. SciPy's `interp1d` defaults to
    /// `bounds_error=True`, so this is an error rather than an extrapolation.
    OutOfBounds { x: f64, lo: f64, hi: f64 },
}

impl fmt::Display for SplineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SplineError::TooFewPoints(n) => {
                write!(f, "cubic spline needs at least 4 points, got {n}")
            }
            SplineError::LengthMismatch { x, y } => {
                write!(f, "x has {x} points but y has {y}")
            }
            SplineError::NotStrictlyIncreasing(i) => {
                write!(f, "x is not strictly increasing at index {i}")
            }
            SplineError::OutOfBounds { x, lo, hi } => {
                write!(f, "{x} is outside the interpolation range [{lo}, {hi}]")
            }
        }
    }
}

impl std::error::Error for SplineError {}

/// A cubic spline through the given knots with not-a-knot end conditions.
#[derive(Debug, Clone)]
pub struct CubicSpline {
    x: Vec<f64>,
    y: Vec<f64>,
    /// First derivative at each knot; the spline is the Hermite interpolant
    /// these induce.
    s: Vec<f64>,
}

impl CubicSpline {
    /// Build the interpolant. `x` must be strictly increasing.
    pub fn new(x: &[f64], y: &[f64]) -> Result<Self, SplineError> {
        if x.len() != y.len() {
            return Err(SplineError::LengthMismatch {
                x: x.len(),
                y: y.len(),
            });
        }
        let n = x.len();
        if n < 4 {
            return Err(SplineError::TooFewPoints(n));
        }
        for i in 1..n {
            if !(x[i] > x[i - 1]) {
                return Err(SplineError::NotStrictlyIncreasing(i));
            }
        }

        // h[i] = x[i+1] - x[i]; slope[i] = (y[i+1] - y[i]) / h[i]
        let h: Vec<f64> = (0..n - 1).map(|i| x[i + 1] - x[i]).collect();
        let slope: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();

        // Solve a tridiagonal system for the knot derivatives s[i].
        //
        // Interior rows enforce C2 continuity:
        //   h[i]*s[i-1] + 2*(h[i-1]+h[i])*s[i] + h[i-1]*s[i+1]
        //       = 3*(h[i]*slope[i-1] + h[i-1]*slope[i])
        //
        // The not-a-knot conditions (continuous third derivative at x[1] and
        // x[n-2]) each involve three unknowns, so they are combined with the
        // adjacent C2 row to eliminate the third one and keep the system
        // tridiagonal. This reproduces SciPy's boundary rows exactly.
        let mut lower = vec![0.0; n]; // sub-diagonal
        let mut diag = vec![0.0; n];
        let mut upper = vec![0.0; n]; // super-diagonal
        let mut rhs = vec![0.0; n];

        // Left boundary, after eliminating s[2]:
        //   h[1]*s[0] + (h[0]+h[1])*s[1]
        //       = ((h[0] + 2*d)*h[1]*slope[0] + h[0]^2*slope[1]) / d
        // with d = x[2] - x[0].
        let d0 = x[2] - x[0];
        diag[0] = h[1];
        upper[0] = d0;
        rhs[0] = ((h[0] + 2.0 * d0) * h[1] * slope[0] + h[0] * h[0] * slope[1]) / d0;

        for i in 1..n - 1 {
            lower[i] = h[i];
            diag[i] = 2.0 * (h[i - 1] + h[i]);
            upper[i] = h[i - 1];
            rhs[i] = 3.0 * (h[i] * slope[i - 1] + h[i - 1] * slope[i]);
        }

        // Right boundary, the mirror image of the left.
        let dn = x[n - 1] - x[n - 3];
        lower[n - 1] = dn;
        diag[n - 1] = h[n - 3];
        rhs[n - 1] = (h[n - 2] * h[n - 2] * slope[n - 3]
            + (2.0 * dn + h[n - 2]) * h[n - 3] * slope[n - 2])
            / dn;

        let s = solve_tridiagonal(&lower, &diag, &upper, &rhs);

        Ok(CubicSpline {
            x: x.to_vec(),
            y: y.to_vec(),
            s,
        })
    }

    /// Evaluate at `xq`. Out-of-range inputs are an error, matching
    /// `interp1d`'s default `bounds_error=True`.
    pub fn eval(&self, xq: f64) -> Result<f64, SplineError> {
        let n = self.x.len();
        let lo = self.x[0];
        let hi = self.x[n - 1];
        if !(xq >= lo && xq <= hi) {
            return Err(SplineError::OutOfBounds { x: xq, lo, hi });
        }
        // Index of the interval containing xq.
        let i = match self.x.partition_point(|&v| v <= xq) {
            0 => 0,
            k if k >= n => n - 2,
            k => k - 1,
        };

        let h = self.x[i + 1] - self.x[i];
        let slope = (self.y[i + 1] - self.y[i]) / h;
        let (si, sj) = (self.s[i], self.s[i + 1]);

        // Hermite cubic on [x[i], x[i+1]].
        let c2 = (3.0 * slope - 2.0 * si - sj) / h;
        let c3 = (si + sj - 2.0 * slope) / (h * h);
        let dx = xq - self.x[i];

        Ok(self.y[i] + dx * (si + dx * (c2 + dx * c3)))
    }

    /// Evaluate at many points.
    pub fn eval_all(&self, xs: &[f64]) -> Result<Vec<f64>, SplineError> {
        xs.iter().map(|&v| self.eval(v)).collect()
    }
}

/// Thomas algorithm for a tridiagonal system.
fn solve_tridiagonal(lower: &[f64], diag: &[f64], upper: &[f64], rhs: &[f64]) -> Vec<f64> {
    let n = diag.len();
    let mut c = vec![0.0; n];
    let mut d = vec![0.0; n];

    c[0] = upper[0] / diag[0];
    d[0] = rhs[0] / diag[0];

    for i in 1..n {
        let denom = diag[i] - lower[i] * c[i - 1];
        c[i] = if i + 1 < n { upper[i] / denom } else { 0.0 };
        d[i] = (rhs[i] - lower[i] * d[i - 1]) / denom;
    }

    let mut out = vec![0.0; n];
    out[n - 1] = d[n - 1];
    for i in (0..n - 1).rev() {
        out[i] = d[i] - c[i] * out[i + 1];
    }
    out
}
