//! DRA's five 0-100 ratings, ported from `rdapy/rate/`.
//!
//! These turn raw metrics into the user-facing scores shown in Dave's
//! Redistricting. They are the part of the port most sensitive to rounding:
//! every rating ends in `round(unit_value * 100)`, and Python rounds halves to
//! even. See [`crate::numeric::python_round`].
//!
//! rdapy's [`Normalizer`] asserts its invariants and raises on violation,
//! which `score_plans` catches and turns into a skipped plan. Rather than
//! panic, the functions here return [`RateError`] so the caller decides.
//! On finite inputs they cannot fail.

use crate::numeric::python_round;

/// Ratings are reported on `[0, 100]`.
pub const NORMALIZED_RANGE: i32 = 100;

/// Exponent used by [`Normalizer::decay`].
pub const DISTANCE_WEIGHT: i32 = 2;

/// Assumed average error in an inferred seats-votes curve.
pub const AVG_SV_ERROR: f64 = 0.02;

/// A winning party is allowed twice its vote share in seats before the
/// deviation counts against it.
pub const WINNER_BONUS: f64 = 2.0;

pub const REOCK_MIN: f64 = 0.25;
pub const REOCK_MAX: f64 = 0.50;
pub const POLSBY_MIN: f64 = 0.10;
pub const POLSBY_MAX: f64 = 0.50;

/// Worst tolerable splitting score: 90-10 becomes 95-5 splits.
pub const MAX_SPLITTING: f64 = 1.20;
/// Best achievable splitting score: no splits at all.
pub const MIN_SPLITTING: f64 = 1.00;
/// How much worse than `best` counts as a zero rating.
pub const WORST_MULTIPLIER: f64 = 1.33;

#[derive(Debug, Clone, PartialEq)]
pub enum RateError {
    /// A value required to lie in `[0, 1]` did not, or was NaN. rdapy raises
    /// an `AssertionError` here.
    NotInUnitRange { stage: &'static str, value: f64 },
    /// [`Normalizer::unitize`] was given a value outside its range.
    OutsideRange { value: f64, lo: f64, hi: f64 },
}

impl std::fmt::Display for RateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateError::NotInUnitRange { stage, value } => {
                write!(f, "{stage}: {value} is not in [0, 1]")
            }
            RateError::OutsideRange { value, lo, hi } => {
                write!(f, "unitize: {value} is outside [{lo}, {hi}]")
            }
        }
    }
}

impl std::error::Error for RateError {}

type Rated = Result<i32, RateError>;

/// A value being transformed into a 0-100 rating.
///
/// Mirrors rdapy's `Normalizer` class, including the order in which the
/// ratings apply its steps.
#[derive(Debug, Clone, Copy)]
pub struct Normalizer {
    pub raw: f64,
    pub wip: f64,
}

impl Normalizer {
    pub fn new(raw: f64) -> Self {
        Normalizer { raw, wip: raw }
    }

    /// Leave the value alone.
    pub fn identity(&mut self) -> f64 {
        self.wip
    }

    pub fn positive(&mut self) -> f64 {
        self.wip = self.wip.abs();
        self.wip
    }

    /// Flip a unit-range value so that bigger is better.
    pub fn invert(&mut self) -> Result<f64, RateError> {
        self.require_unit("invert")?;
        self.wip = 1.0 - self.wip;
        Ok(self.wip)
    }

    /// Constrain to a range. The endpoints may be given in either order.
    pub fn clip(&mut self, begin: f64, end: f64) -> f64 {
        let lo = begin.min(end);
        let hi = begin.max(end);
        self.wip = self.wip.min(hi).max(lo);
        self.wip
    }

    /// Re-express as a delta from a baseline.
    pub fn rebase(&mut self, base: f64) -> f64 {
        self.wip -= base;
        self.wip
    }

    /// Rescale into `[0, 1]`, assuming the value has already been clipped.
    ///
    /// Note that the divisor is `end - begin` as given, not `hi - lo`. When
    /// the endpoints arrive reversed -- which the splitting and
    /// proportionality ratings do -- the quotient comes out negative and the
    /// `abs` flips it. Faithfully reproducing that ordering matters.
    pub fn unitize(&mut self, begin: f64, end: f64) -> Result<f64, RateError> {
        let lo = begin.min(end);
        let hi = begin.max(end);
        if !(self.wip >= lo && self.wip <= hi) {
            return Err(RateError::OutsideRange {
                value: self.wip,
                lo,
                hi,
            });
        }
        let ranged = self.wip - lo;
        self.wip = (ranged / (end - begin)).abs();
        Ok(self.wip)
    }

    /// Decay a unit-range value by its distance from zero.
    pub fn decay(&mut self) -> Result<f64, RateError> {
        self.require_unit("decay")?;
        self.wip = self.wip.powi(DISTANCE_WEIGHT);
        Ok(self.wip)
    }

    /// Translate a unit-range value to `[0, 100]`.
    pub fn rescale(&mut self) -> Rated {
        self.require_unit("rescale")?;
        Ok(python_round(self.wip * NORMALIZED_RANGE as f64) as i32)
    }

    fn require_unit(&self, stage: &'static str) -> Result<(), RateError> {
        if self.wip >= 0.0 && self.wip <= 1.0 {
            Ok(())
        } else {
            Err(RateError::NotInUnitRange {
                stage,
                value: self.wip,
            })
        }
    }
}

// ---- PROPORTIONALITY ----

/// Has the minority of voters won a majority of seats?
pub fn is_antimajoritarian(vf: f64, sf: f64) -> bool {
    let dem = vf < (0.5 - AVG_SV_ERROR) && sf > 0.5;
    let rep = (1.0 - vf) < (0.5 - AVG_SV_ERROR) && (1.0 - sf) > 0.5;
    dem || rep
}

/// The winner's bonus a party is allowed before deviation counts against it.
pub fn extra_bonus(vf: f64) -> f64 {
    let over_50 = if vf > 0.5 { vf - 0.5 } else { 0.5 - vf };
    over_50 * (WINNER_BONUS - 1.0)
}

/// Discount deviation by the winner's bonus, but only when the bias runs in
/// the same direction as the statewide vote. Bias against the statewide
/// winner is left alone.
pub fn adjust_deviation(vf: f64, disproportionality: f64, extra: f64) -> f64 {
    if vf > 0.5 && disproportionality < 0.0 {
        (disproportionality + extra).min(0.0)
    } else if vf < 0.5 && disproportionality > 0.0 {
        (disproportionality - extra).max(0.0)
    } else {
        disproportionality
    }
}

/// rdapy `rate_proportionality`.
pub fn proportionality(raw_disproportionality: f64, vf: f64, sf: f64) -> Rated {
    if is_antimajoritarian(vf, sf) {
        return Ok(0);
    }
    let extra = extra_bonus(vf);
    let adjusted = adjust_deviation(vf, raw_disproportionality, extra);

    let best = 0.0;
    let worst = 0.20;

    let mut n = Normalizer::new(adjusted);
    n.positive();
    n.clip(worst, best);
    n.unitize(worst, best)?;
    n.invert()?;
    n.rescale()
}

// ---- COMPETITIVENESS ----

/// rdapy `rate_competitiveness`.
///
/// Raw values run `[0, 1]`, but three quarters is the practical maximum, so
/// that is treated as a perfect score.
pub fn competitiveness(raw_cdf: f64) -> Rated {
    let worst = 0.0;
    let best = 0.75;

    let mut n = Normalizer::new(raw_cdf);
    n.clip(worst, best);
    n.unitize(worst, best)?;
    n.rescale()
}

// ---- MINORITY OPPORTUNITY ----

/// rdapy `rate_minority_opportunity`.
///
/// Opportunity and coalition district counts can exceed what the statewide
/// share would make proportional, because of how opportunity is estimated, so
/// both are capped before scoring.
pub fn minority_opportunity(od: f64, pod: f64, cd: f64, pcd: f64) -> i32 {
    let cd_weight = 0.5;

    let od_capped = od.min(pod);
    let cd_capped = cd.min(pcd);

    let opportunity_score = if pod > 0.0 {
        python_round((od_capped / pod) * 100.0)
    } else {
        0.0
    };
    let coalition_score = if pcd > 0.0 {
        python_round((cd_capped / pcd) * 100.0)
    } else {
        0.0
    };

    let combined =
        opportunity_score + cd_weight * (coalition_score - opportunity_score).max(0.0);
    python_round(combined.min(100.0)) as i32
}

// ---- COMPACTNESS ----

/// rdapy `rate_reock`.
pub fn reock(raw: f64) -> Rated {
    let mut n = Normalizer::new(raw);
    n.clip(REOCK_MIN, REOCK_MAX);
    n.unitize(REOCK_MIN, REOCK_MAX)?;
    n.rescale()
}

/// rdapy `rate_polsby`.
pub fn polsby(raw: f64) -> Rated {
    let mut n = Normalizer::new(raw);
    n.clip(POLSBY_MIN, POLSBY_MAX);
    n.unitize(POLSBY_MIN, POLSBY_MAX)?;
    n.rescale()
}

/// rdapy `rate_compactness`: an even blend of the two.
pub fn compactness(reock_rating: i32, polsby_rating: i32) -> i32 {
    let reock_weight = 50;
    let polsby_weight = NORMALIZED_RANGE - reock_weight;

    python_round(
        ((reock_rating * reock_weight + polsby_rating * polsby_weight) as f64)
            / NORMALIZED_RANGE as f64,
    ) as i32
}

// ---- SPLITTING ----

/// The practically-best splitting score given the counts of counties and
/// districts. With many more of one than the other, some splitting is
/// unavoidable, so the target relaxes towards [`MAX_SPLITTING`].
pub fn best_target(n: f64, m: f64) -> f64 {
    let more = n.max(m);
    let less = n.min(m);

    let w1 = (less - 1.0) / more;
    let w2 = 1.0 - w1;

    w1 * MAX_SPLITTING + w2 * MIN_SPLITTING
}

/// rdapy `rate_county_splitting`.
pub fn county_splitting(raw: f64, n_counties: i32, n_districts: i32) -> Rated {
    let best = if n_counties > n_districts {
        best_target(n_counties as f64, n_districts as f64)
    } else {
        MAX_SPLITTING
    };
    split_rating(raw, best)
}

/// rdapy `rate_district_splitting`.
///
/// The mirror of [`county_splitting`]: whichever of counties or districts is
/// the more numerous drives which side gets the relaxed target.
pub fn district_splitting(raw: f64, n_counties: i32, n_districts: i32) -> Rated {
    let best = if n_counties > n_districts {
        MAX_SPLITTING
    } else {
        best_target(n_counties as f64, n_districts as f64)
    };
    split_rating(raw, best)
}

fn split_rating(raw: f64, best: f64) -> Rated {
    let worst = best * WORST_MULTIPLIER;

    let mut n = Normalizer::new(raw);
    n.clip(best, worst);
    n.unitize(best, worst)?;
    n.invert()?;
    let rating = n.rescale()?;

    // A perfect 100 is reserved for no splitting at all.
    if rating == NORMALIZED_RANGE && raw > 1.0 {
        return Ok(NORMALIZED_RANGE - 1);
    }
    Ok(rating)
}

/// rdapy `rate_splitting`: an even blend, with 100 again reserved for a plan
/// that splits nothing.
pub fn splitting(county_rating: i32, district_rating: i32) -> i32 {
    let county_weight = 50;
    let district_weight = NORMALIZED_RANGE - county_weight;

    let rating = python_round(
        ((county_rating * county_weight + district_rating * district_weight) as f64)
            / NORMALIZED_RANGE as f64,
    ) as i32;

    if rating == NORMALIZED_RANGE
        && (county_rating < NORMALIZED_RANGE || district_rating < NORMALIZED_RANGE)
    {
        return NORMALIZED_RANGE - 1;
    }
    rating
}
