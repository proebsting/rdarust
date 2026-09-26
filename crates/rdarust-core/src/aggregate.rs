//! Aggregating precinct data and shapes by district.
//!
//! Ported from `rdapy/aggregate/aggregate.py`. The aggregates are kept in a
//! reusable buffer: a sampler scoring millions of plans should not allocate a
//! fresh one per plan.

use crate::context::{Context, UNASSIGNED};
use crate::geometry::min_enclosing_circle;

/// Which families of metric to compute.
///
/// Mirrors rdapy's `--mode`. Aggregating only what is wanted matters: shape
/// aggregation dominates the cost, and a partisan-only sweep can skip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    General,
    Partisan,
    Minority,
    Compactness,
    Splitting,
}

impl Mode {
    pub fn general(self) -> bool {
        matches!(self, Mode::All | Mode::General)
    }
    pub fn partisan(self) -> bool {
        matches!(self, Mode::All | Mode::Partisan)
    }
    pub fn minority(self) -> bool {
        matches!(self, Mode::All | Mode::Minority)
    }
    pub fn compactness(self) -> bool {
        matches!(self, Mode::All | Mode::Compactness)
    }
    pub fn splitting(self) -> bool {
        matches!(self, Mode::All | Mode::Splitting)
    }
    /// rdapy aggregates data for every mode except the shape-only one.
    pub fn needs_data(self) -> bool {
        !matches!(self, Mode::Compactness)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateError {
    /// A precinct with population is missing from the plan. rdapy raises
    /// `ValueError` with the same meaning.
    PopulatedPrecinctNotInPlan(String),
    /// A neighbour is absent from the plan and is not skippable border water,
    /// where rdapy raises `KeyError`.
    NeighborNotInPlan { of: String, neighbor: String },
    /// A neighbour that shares a border has no entry in the arc table.
    MissingArc { of: String, neighbor: String },
    /// A district number outside `1..=n_districts`.
    DistrictOutOfRange { geoid: String, district: u32 },
    /// A district came out with no boundary at all, so its diameter is zero
    /// and Reock would be infinite. In practice this means the adjacency
    /// graph is missing -- input files written for the scoring pipeline carry
    /// no neighbour lists, and the graph must be supplied separately.
    DistrictHasNoBoundary(usize),
}

impl std::fmt::Display for AggregateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AggregateError::PopulatedPrecinctNotInPlan(g) => {
                write!(f, "populated geoid ({g}) not in the plan")
            }
            AggregateError::NeighborNotInPlan { of, neighbor } => {
                write!(f, "{of}'s neighbour {neighbor} is not in the plan")
            }
            AggregateError::MissingArc { of, neighbor } => {
                write!(f, "no shared border recorded between {of} and {neighbor}")
            }
            AggregateError::DistrictOutOfRange { geoid, district } => {
                write!(f, "{geoid} is assigned to district {district}, out of range")
            }
            AggregateError::DistrictHasNoBoundary(d) => write!(
                f,
                "district {d} has no boundary; is the adjacency graph loaded?"
            ),
        }
    }
}

impl std::error::Error for AggregateError {}

/// Per-district totals for one plan.
///
/// Every vector is indexed by district with element 0 holding the statewide
/// total, as rdapy does -- except `cxd`, whose rows are districts `1..=D`
/// mapped to `0..D`.
#[derive(Debug, Clone, PartialEq)]
pub struct Aggregates {
    pub pop_by_district: Vec<i64>,
    /// `[election][district]`.
    pub dem_by_district: Vec<Vec<i64>>,
    /// Two-party totals, not total votes cast.
    pub tot_by_district: Vec<Vec<i64>>,
    /// `[demographic][district]`, in the metadata's field order.
    pub vap_by_district: Vec<Vec<i64>>,
    pub cvap_by_district: Vec<Vec<i64>>,
    /// Population by district and county.
    pub cxd: Vec<Vec<f64>>,

    pub area: Vec<f64>,
    pub perimeter: Vec<f64>,
    pub diameter: Vec<f64>,

    /// Filled in by scoring; element 0 is the plan-wide average.
    pub reock: Vec<f64>,
    pub polsby_popper: Vec<f64>,
    /// Filled in by scoring; element 0 is the plan-wide score.
    pub district_splitting: Vec<f64>,

    /// Scratch space for district boundary points, reused across plans.
    exterior: Vec<Vec<(f64, f64)>>,
}

impl Aggregates {
    pub fn new(ctx: &Context) -> Self {
        let d1 = ctx.n_districts + 1;
        Aggregates {
            pop_by_district: vec![0; d1],
            dem_by_district: vec![vec![0; d1]; ctx.elections.len()],
            tot_by_district: vec![vec![0; d1]; ctx.elections.len()],
            vap_by_district: vec![vec![0; d1]; ctx.vap.counts.len()],
            cvap_by_district: ctx
                .cvap
                .as_ref()
                .map(|c| vec![vec![0; d1]; c.counts.len()])
                .unwrap_or_default(),
            cxd: vec![vec![0.0; ctx.n_counties]; ctx.n_districts],
            area: vec![0.0; d1],
            perimeter: vec![0.0; d1],
            diameter: vec![0.0; d1],
            reock: vec![0.0; d1],
            polsby_popper: vec![0.0; d1],
            district_splitting: vec![0.0; d1],
            exterior: vec![Vec::new(); d1],
        }
    }

    /// Zero everything, keeping the allocations.
    pub fn reset(&mut self) {
        self.pop_by_district.fill(0);
        for v in &mut self.dem_by_district {
            v.fill(0);
        }
        for v in &mut self.tot_by_district {
            v.fill(0);
        }
        for v in &mut self.vap_by_district {
            v.fill(0);
        }
        for v in &mut self.cvap_by_district {
            v.fill(0);
        }
        for row in &mut self.cxd {
            row.fill(0.0);
        }
        self.area.fill(0.0);
        self.perimeter.fill(0.0);
        self.diameter.fill(0.0);
        self.reock.fill(0.0);
        self.polsby_popper.fill(0.0);
        self.district_splitting.fill(0.0);
        for e in &mut self.exterior {
            e.clear();
        }
    }
}

impl Context {
    /// Aggregate a plan into a fresh buffer.
    pub fn aggregate(&self, plan: &[u32], mode: Mode) -> Result<Aggregates, AggregateError> {
        let mut aggs = Aggregates::new(self);
        self.aggregate_into(plan, mode, &mut aggs)?;
        Ok(aggs)
    }

    /// Aggregate a plan, reusing `aggs`.
    pub fn aggregate_into(
        &self,
        plan: &[u32],
        mode: Mode,
        aggs: &mut Aggregates,
    ) -> Result<(), AggregateError> {
        aggs.reset();
        if mode.needs_data() {
            self.aggregate_data(plan, mode, aggs)?;
        }
        if mode.compactness() {
            self.aggregate_shapes(plan, aggs)?;
        }
        Ok(())
    }

    /// The district a precinct belongs to, or an error explaining why it has
    /// none. Unpopulated precincts missing from the plan are skipped, which
    /// is how rdapy tolerates water-only features.
    #[inline]
    fn district_at(&self, i: usize, plan: &[u32]) -> Result<Option<usize>, AggregateError> {
        let d = plan[i];
        if d == UNASSIGNED {
            return if self.pop[i] == 0 {
                Ok(None)
            } else {
                Err(AggregateError::PopulatedPrecinctNotInPlan(
                    self.geoids[i].clone(),
                ))
            };
        }
        if d == 0 || d as usize > self.n_districts {
            return Err(AggregateError::DistrictOutOfRange {
                geoid: self.geoids[i].clone(),
                district: d,
            });
        }
        Ok(Some(d as usize))
    }

    fn aggregate_data(
        &self,
        plan: &[u32],
        mode: Mode,
        aggs: &mut Aggregates,
    ) -> Result<(), AggregateError> {
        for i in 0..self.n_precincts() {
            let Some(d) = self.district_at(i, plan)? else {
                continue;
            };
            let pop = self.pop[i];

            if mode.general() {
                aggs.pop_by_district[d] += pop;
                aggs.pop_by_district[0] += pop;
            }

            if mode.partisan() {
                for (e, election) in self.elections.iter().enumerate() {
                    let dem = election.dem[i];
                    // The two-party total, not total votes cast.
                    let tot = dem + election.rep[i];
                    aggs.dem_by_district[e][d] += dem;
                    aggs.dem_by_district[e][0] += dem;
                    aggs.tot_by_district[e][d] += tot;
                    aggs.tot_by_district[e][0] += tot;
                }
            }

            if mode.minority() {
                for (k, counts) in self.vap.counts.iter().enumerate() {
                    aggs.vap_by_district[k][d] += counts[i];
                    aggs.vap_by_district[k][0] += counts[i];
                }
                if let Some(cvap) = &self.cvap {
                    for (k, counts) in cvap.counts.iter().enumerate() {
                        aggs.cvap_by_district[k][d] += counts[i];
                        aggs.cvap_by_district[k][0] += counts[i];
                    }
                }
            }

            if mode.splitting() {
                aggs.cxd[d - 1][self.county_of[i] as usize] += pop as f64;
            }
        }
        Ok(())
    }

    fn aggregate_shapes(
        &self,
        plan: &[u32],
        aggs: &mut Aggregates,
    ) -> Result<(), AggregateError> {
        for i in 0..self.n_precincts() {
            let Some(d) = self.district_at(i, plan)? else {
                continue;
            };

            aggs.area[d] += self.area[i];
            aggs.area[0] += self.area[i];

            let mut border = 0.0;
            let mut on_boundary = false;

            for (k, &nb) in self.adjacency[i].iter().enumerate() {
                let arc = self.arc_len[i][k];
                let nb = nb as usize;

                let counts = if Some(nb as u32) == self.out_of_state {
                    // The state border contributes only if an arc is recorded.
                    arc.is_some()
                } else if self.is_water_only[nb] && plan[nb] == UNASSIGNED {
                    // Water the plan does not mention acts as border too.
                    arc.is_some()
                } else if plan[nb] == UNASSIGNED {
                    return Err(AggregateError::NeighborNotInPlan {
                        of: self.geoids[i].clone(),
                        neighbor: self.node_name(nb),
                    });
                } else {
                    plan[nb] != d as u32
                };

                if counts {
                    border += arc.ok_or_else(|| AggregateError::MissingArc {
                        of: self.geoids[i].clone(),
                        neighbor: self.node_name(nb),
                    })?;
                    on_boundary = true;
                }
            }

            aggs.perimeter[d] += border;

            // rdapy appends this precinct's whole hull once per qualifying
            // neighbour. The enclosing circle dedupes its input, so adding it
            // once gives the identical circle for a fraction of the memory.
            if on_boundary {
                aggs.exterior[d].extend_from_slice(&self.exterior[i]);
            }
        }

        for d in 1..=self.n_districts {
            if aggs.exterior[d].is_empty() {
                return Err(AggregateError::DistrictHasNoBoundary(d));
            }
            let circle = min_enclosing_circle(&aggs.exterior[d]);
            aggs.diameter[d] = 2.0 * circle.r;
        }

        Ok(())
    }
}
