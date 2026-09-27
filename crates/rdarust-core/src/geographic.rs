//! Jon Eguia and Jeff Barton's geographic baseline.
//!
//! The idea: grow a district-sized "neighbourhood" around every precinct by
//! reaching outwards to the nearest connected precincts, and ask how that
//! neighbourhood votes. Weighting those answers by population gives the
//! number of seats each party would win from geography alone -- how the
//! voters happen to be spread out -- against which a plan's actual result can
//! be compared. That comparison is the `geographic_advantage` score.
//!
//! Neighbourhoods depend only on the state, not on any plan, so they are
//! computed once and kept. Ported from `rdapy/partisan/geographic.py`.

use crate::context::{Context, OUT_OF_STATE};
use crate::numeric::approx_equal;
use crate::partisan::method::est_seat_probability;

/// Squared distance between two (lon, lat) points.
///
/// Squared, because these are only ever compared with each other. rdapy calls
/// this a "distance proxy" and caches it per precinct pair; here it is two
/// multiplications, which is cheaper than a cache lookup would be.
#[inline]
pub fn distance_proxy(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    dy * dy + dx * dx
}

/// A precinct on the frontier of a growing neighbourhood.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frontier {
    index: u32,
    /// Distance from the *seed*, not from where it was reached.
    distance: f64,
    pop: i64,
}

impl Context {
    /// The population a neighbourhood aims for: one district's worth.
    ///
    /// rdapy computes this as `int((state_pop // n_districts) * size)`, so
    /// the division floors before the scaling.
    pub fn neighborhood_target_pop(&self, size: f64) -> i64 {
        let per_district = self.state_pop() / self.n_districts as i64;
        (per_district as f64 * size) as i64
    }

    /// Total population across all precincts.
    pub fn state_pop(&self) -> i64 {
        self.pop.iter().sum()
    }

    /// Grow a neighbourhood outwards from `seed` until it holds about
    /// `target_pop` people.
    ///
    /// Precincts are taken in order of distance from the seed, but only ones
    /// connected to what has already been taken, so a neighbourhood is always
    /// contiguous. The last precinct is included only if doing so lands
    /// closer to the target than stopping short would.
    ///
    /// Returns precinct indices in the order they were reached, seed first.
    pub fn make_neighborhood(&self, seed: u32, target_pop: i64) -> Vec<u32> {
        let n = self.adjacency.len();
        let mut yielded = vec![false; n];
        let mut queued = vec![false; n];
        let seed_center = self.center[seed as usize];

        let mut queue = vec![Frontier {
            index: seed,
            distance: 0.0,
            pop: self.pop[seed as usize],
        }];
        queued[seed as usize] = true;

        let mut members: Vec<u32> = Vec::new();
        let mut pop: i64 = 0;

        while let Some(next) = queue.pop() {
            if members.is_empty() {
                members.push(next.index);
                pop = next.pop;
            } else {
                let new_pop = pop + next.pop;
                if new_pop <= target_pop {
                    members.push(next.index);
                    pop = new_pop;
                } else {
                    // Overshooting by less than undershooting would means
                    // taking it is the closer fit; either way we stop here.
                    if new_pop - target_pop < target_pop - pop {
                        members.push(next.index);
                    }
                    break;
                }
            }

            queued[next.index as usize] = false;
            yielded[next.index as usize] = true;

            for &nb in &self.adjacency[next.index as usize] {
                let i = nb as usize;
                if Some(nb) == self.out_of_state || yielded[i] || queued[i] {
                    continue;
                }
                queued[i] = true;
                queue.push(Frontier {
                    index: nb,
                    distance: distance_proxy(seed_center, self.center[i]),
                    pop: self.pop[i],
                });
            }

            // Sorted furthest-first so the nearest is at the end, where
            // popping is cheap. A stable sort, so precincts equidistant from
            // the seed keep the order they were reached in.
            queue.sort_by(|a, b| {
                b.distance
                    .partial_cmp(&a.distance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        members
    }

    /// Neighbourhoods for every precinct, in sorted-geoid order.
    ///
    /// That order matters: the packed form identifies precincts by position
    /// in the sorted list, and the baseline sums over the file in order.
    pub fn all_neighborhoods(&self, target_pop: i64) -> Vec<(u32, Vec<u32>)> {
        self.sorted_precinct_order()
            .into_iter()
            .map(|i| (i, self.make_neighborhood(i, target_pop)))
            .collect()
    }

    /// How a neighbourhood votes, and what that is worth in seats.
    ///
    /// A neighbourhood at exactly even counts as half a seat, which keeps the
    /// baseline from swinging on a single vote.
    pub fn eval_partisan_lean(&self, neighborhood: &[u32], election: usize) -> PartisanLean {
        let e = &self.elections[election];
        let mut dem: i64 = 0;
        let mut tot: i64 = 0;
        for &i in neighborhood {
            let i = i as usize;
            dem += e.dem[i];
            // Two-party votes, not total votes cast.
            tot += e.dem[i] + e.rep[i];
        }

        let vf = if tot > 0 { dem as f64 / tot as f64 } else { 0.0 };
        let whole_seats = if approx_equal(vf, 0.5, 6) {
            0.5
        } else if vf > 0.5 {
            1.0
        } else {
            0.0
        };

        PartisanLean {
            vf,
            fractional_seats: est_seat_probability(vf),
            whole_seats,
        }
    }

    /// The geographic baseline for one election.
    ///
    /// Each precinct's neighbourhood contributes in proportion to the
    /// precinct's share of the state, so the total is on the same scale as
    /// the number of districts.
    pub fn geographic_baseline(
        &self,
        neighborhoods: &[(u32, Vec<u32>)],
        election: usize,
    ) -> GeographicBaseline {
        let state_pop = self.state_pop() as f64;
        let n_districts = self.n_districts as f64;

        let mut fractional_seats = 0.0;
        let mut whole_seats = 0.0;

        for (seed, members) in neighborhoods {
            let lean = self.eval_partisan_lean(members, election);
            let proportion = n_districts * (self.pop[*seed as usize] as f64 / state_pop);
            fractional_seats += lean.fractional_seats * proportion;
            whole_seats += lean.whole_seats * proportion;
        }

        GeographicBaseline {
            fractional_seats,
            whole_seats,
        }
    }

    /// Precinct indices ordered by geoid, which is the order the
    /// neighbourhood file uses.
    ///
    /// The virtual border node is not a precinct and is excluded, as rdapy's
    /// `sorted_geoids` excludes it.
    pub fn sorted_precinct_order(&self) -> Vec<u32> {
        let mut order: Vec<u32> = (0..self.n_precincts() as u32)
            .filter(|&i| self.geoids[i as usize] != OUT_OF_STATE)
            .collect();
        order.sort_by(|&a, &b| self.geoids[a as usize].cmp(&self.geoids[b as usize]));
        order
    }
}

/// How a neighbourhood leans.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartisanLean {
    /// Democratic share of the two-party vote.
    pub vf: f64,
    /// Expected seats, from the seat-probability curve.
    pub fractional_seats: f64,
    /// Seats under a straight win or lose, with a tie worth half.
    pub whole_seats: f64,
}

/// Seats attributable to geography alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeographicBaseline {
    pub fractional_seats: f64,
    pub whole_seats: f64,
}
