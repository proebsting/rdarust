//! Population equality, ported from `rdapy/equal/population.py`.

/// The spread between the largest and smallest district, as a fraction of the
/// ideal district population.
///
/// Note that rdapy's callers compute `target_pop` as `int(total / n)`, i.e.
/// truncated, not rounded.
#[inline]
pub fn calc_population_deviation(max_pop: i64, min_pop: i64, target_pop: i64) -> f64 {
    (max_pop - min_pop) as f64 / target_pop as f64
}
