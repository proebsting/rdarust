//! Scoring a plan, ported from `rdapy/score/`.
//!
//! [`Context::score`] turns a plan into a [`Scorecard`]. Scores are always
//! qualified by the dataset they were computed against, because a plan can be
//! scored against several elections at once.

mod categories;
pub use categories::*;

use std::collections::HashMap;

use crate::aggregate::{AggregateError, Aggregates, Mode};
use crate::compactness::discrete::{cut_score, CutScoreError};
use crate::compactness::energy::calc_energy;
use crate::context::{Context, UNASSIGNED};
use crate::numeric::python_round_to;
use crate::partisan::metrics::PartisanError;
use crate::rate::{self, RateError};

/// Floating-point scores are reported to this many decimal places, as rdapy's
/// `trim_scores` does. Integer scores are left alone.
pub const SCORE_PRECISION: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum ScoreError {
    Aggregate(AggregateError),
    Partisan(PartisanError),
    Rate(RateError),
    CutScore(CutScoreError),
    /// A district ended up with no population, so its centroid is undefined.
    EmptyDistrict,
    /// The plan has no districts, or the aggregates are empty.
    NoDistricts,
    /// Scoring one election failed. Which one matters: a run over twenty
    /// elections is usually undone by one degenerate race, and the bare
    /// message does not say which to drop.
    Election { key: String, source: Box<ScoreError> },
}

macro_rules! from_err {
    ($t:ty, $v:ident) => {
        impl From<$t> for ScoreError {
            fn from(e: $t) -> Self {
                ScoreError::$v(e)
            }
        }
    };
}
from_err!(AggregateError, Aggregate);
from_err!(PartisanError, Partisan);
from_err!(RateError, Rate);
from_err!(CutScoreError, CutScore);

impl std::fmt::Display for ScoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScoreError::Aggregate(e) => write!(f, "{e}"),
            ScoreError::Partisan(e) => write!(f, "{e}"),
            ScoreError::Rate(e) => write!(f, "{e}"),
            ScoreError::CutScore(e) => write!(f, "{e}"),
            ScoreError::EmptyDistrict => write!(f, "a district has no population"),
            ScoreError::NoDistricts => write!(f, "the plan has no districts"),
            ScoreError::Election { key, source } => write!(f, "election {key}: {source}"),
        }
    }
}

impl std::error::Error for ScoreError {}

/// How to score.
#[derive(Debug, Clone, Default)]
pub struct ScoreOptions {
    pub mode: ModeOpt,
    /// Count majority-minority districts. rdapy's legacy tests turn this off.
    pub mmd_scoring: bool,
    /// Also report county splitting under Don Leake's reverse weighting.
    pub reverse_weight_splitting: bool,
    /// Whole seats from a precomputed geographic baseline, by election key.
    /// Where present, the election gains a `geographic_advantage` score.
    pub geographic_baselines: HashMap<String, f64>,
}

/// Newtype so [`ScoreOptions`] can derive `Default` with `Mode::All`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeOpt(pub Mode);

impl Default for ModeOpt {
    fn default() -> Self {
        ModeOpt(Mode::All)
    }
}

impl std::ops::Deref for ModeOpt {
    type Target = Mode;
    fn deref(&self) -> &Mode {
        &self.0
    }
}

/// Scores from census data: population equality and county splitting.
#[derive(Debug, Clone, PartialEq)]
pub struct CensusScores {
    pub dataset: String,
    pub population_deviation: Option<f64>,
    pub county_splitting: Option<f64>,
    pub district_splitting: Option<f64>,
    pub counties_split: Option<i32>,
    pub county_splits: Option<i32>,
    /// The 0-100 rating.
    pub splitting: Option<i32>,
    pub county_splitting_reverse: Option<f64>,
    pub splitting_reverse: Option<i32>,
}

/// Scores from one election.
#[derive(Debug, Clone, PartialEq)]
pub struct ElectionScores {
    pub dataset: String,
    pub estimated_vote_pct: f64,
    pub pr_deviation: f64,
    pub estimated_seats: f64,
    pub fptp_seats: i32,
    pub disproportionality: f64,
    pub efficiency_gap: f64,
    pub efficiency_gap_fptp: f64,
    pub efficiency_gap_wasted_votes: Option<f64>,
    pub seats_bias: Option<f64>,
    pub votes_bias: f64,
    pub geometric_seats_bias: f64,
    pub declination: Option<f64>,
    pub mean_median_statewide: f64,
    pub mean_median_average_district: f64,
    pub turnout_bias: f64,
    pub lopsided_outcomes: Option<f64>,
    pub competitive_district_count: i32,
    pub competitive_districts: f64,
    pub average_margin: f64,
    pub responsiveness: f64,
    pub responsive_districts: f64,
    pub overall_responsiveness: Option<f64>,
    /// Seats above what geography alone would give, if a baseline was supplied.
    pub geographic_advantage: Option<f64>,
    /// The 0-100 ratings.
    pub proportionality: i32,
    pub competitiveness: i32,
}

/// Minority opportunity, from voting-age population.
#[derive(Debug, Clone, PartialEq)]
pub struct VapScores {
    pub dataset: String,
    pub opportunity_districts: f64,
    pub proportional_opportunities: i32,
    pub coalition_districts: f64,
    pub proportional_coalitions: i32,
    /// The 0-100 rating.
    pub minority: i32,
}

/// Majority-minority counts, from citizen voting-age population.
#[derive(Debug, Clone, PartialEq)]
pub struct CvapScores {
    pub dataset: String,
    pub mmd_black: i32,
    pub mmd_hispanic: i32,
    pub mmd_coalition: i32,
}

/// Compactness, from aggregated shape properties and the adjacency graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeScores {
    pub dataset: String,
    pub reock: f64,
    pub polsby_popper: f64,
    pub cut_score: i32,
    pub population_compactness: f64,
    /// The 0-100 rating.
    pub compactness: i32,
}

/// Everything scored for one plan.
///
/// A family is `None` when the [`Mode`] excluded it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scorecard {
    pub census: Option<CensusScores>,
    pub vap: Option<VapScores>,
    pub cvap: Option<CvapScores>,
    pub elections: Vec<ElectionScores>,
    pub shapes: Option<ShapeScores>,
}

impl Context {
    /// Aggregate and score a plan.
    ///
    /// Returns the aggregates too: they carry the by-district Reock,
    /// Polsby-Popper and splitting figures that scoring fills in, which the
    /// CLI writes alongside the scores.
    pub fn score(
        &self,
        plan: &[u32],
        opts: &ScoreOptions,
    ) -> Result<(Scorecard, Aggregates), ScoreError> {
        let mut aggs = Aggregates::new(self);
        let card = self.score_into(plan, opts, &mut aggs)?;
        Ok((card, aggs))
    }

    /// Aggregate and score, reusing `aggs`.
    pub fn score_into(
        &self,
        plan: &[u32],
        opts: &ScoreOptions,
        aggs: &mut Aggregates,
    ) -> Result<Scorecard, ScoreError> {
        self.aggregate_into(plan, *opts.mode, aggs)?;
        self.score_aggregates(plan, opts, aggs)
    }

    /// Score already-computed aggregates.
    ///
    /// Fills the by-district Reock, Polsby-Popper and splitting figures into
    /// `aggs`, as rdapy folds them back into its aggregate record.
    pub fn score_aggregates(
        &self,
        plan: &[u32],
        opts: &ScoreOptions,
        aggs: &mut Aggregates,
    ) -> Result<Scorecard, ScoreError> {
        let mode = *opts.mode;
        let n = self.n_districts;
        if n == 0 {
            return Err(ScoreError::NoDistricts);
        }
        let mut card = Scorecard::default();

        let mut census = (mode.general() || mode.splitting()).then(|| CensusScores {
            dataset: self.keys.census.clone(),
            population_deviation: None,
            county_splitting: None,
            district_splitting: None,
            counties_split: None,
            county_splits: None,
            splitting: None,
            county_splitting_reverse: None,
            splitting_reverse: None,
        });

        if mode.general() {
            let c = census.as_mut().expect("census scores");
            c.population_deviation = Some(general_category(&aggs.pop_by_district, n));
        }

        if mode.partisan() {
            for (e, election) in self.elections.iter().enumerate() {
                card.elections.push(
                    self.score_election(
                        e,
                        election,
                        aggs,
                        opts.geographic_baselines.get(&election.key).copied(),
                    )
                    .map_err(|source| ScoreError::Election {
                        key: election.key.clone(),
                        source: Box::new(source),
                    })?,
                );
            }
        }

        if mode.minority() {
            card.vap = Some(self.score_minority(aggs)?);
            if opts.mmd_scoring {
                card.cvap = self.score_mmd(aggs);
            }
        }

        if mode.compactness() {
            card.shapes = Some(self.score_compactness(plan, aggs)?);
        }

        if mode.splitting() {
            let c = census.as_mut().expect("census scores");
            self.score_splitting(opts, aggs, c)?;
        }

        card.census = census;
        card.trim();
        Ok(card)
    }

    fn score_election(
        &self,
        e: usize,
        election: &crate::context::Election,
        aggs: &Aggregates,
        baseline_whole_seats: Option<f64>,
    ) -> Result<ElectionScores, ScoreError> {
        let mut scores = partisan_category(
            &election.key,
            &aggs.dem_by_district[e],
            &aggs.tot_by_district[e],
            baseline_whole_seats,
        )?;

        // rdapy rates proportionality against the estimated seat *share*.
        let estimated_seat_pct = scores.estimated_seats / self.n_districts as f64;
        scores.proportionality = rate::proportionality(
            scores.pr_deviation,
            scores.estimated_vote_pct,
            estimated_seat_pct,
        )?;
        scores.competitiveness =
            rate::competitiveness(scores.competitive_districts / self.n_districts as f64)?;
        Ok(scores)
    }

    fn score_minority(&self, aggs: &Aggregates) -> Result<VapScores, ScoreError> {
        // The revised ratings do not clip Black VAP below 37%.
        let m = minority_category(&aggs.vap_by_district, &self.vap.names, self.n_districts);
        Ok(VapScores {
            dataset: self.keys.vap.clone(),
            opportunity_districts: m.opportunity_districts,
            proportional_opportunities: m.proportional_opportunities,
            coalition_districts: m.coalition_districts,
            proportional_coalitions: m.proportional_coalitions,
            minority: rate::minority_opportunity(
                m.opportunity_districts,
                m.proportional_opportunities as f64,
                m.coalition_districts,
                m.proportional_coalitions as f64,
            ),
        })
    }

    fn score_mmd(&self, aggs: &Aggregates) -> Option<CvapScores> {
        let cvap = self.cvap.as_ref()?;
        let dataset = self.keys.cvap.clone()?;
        let find = |name: &str| {
            cvap.names
                .iter()
                .position(|n| n == name)
                .map(|k| &aggs.cvap_by_district[k][1..])
        };
        let (black, hispanic, total) =
            (find("black_cvap")?, find("hispanic_cvap")?, find("total_cvap")?);

        let counts = crate::minority::majority_minority::calculate_mmd_simple(
            &black.iter().map(|&x| x as f64).collect::<Vec<_>>(),
            &hispanic.iter().map(|&x| x as f64).collect::<Vec<_>>(),
            &total.iter().map(|&x| x as f64).collect::<Vec<_>>(),
        );
        Some(CvapScores {
            dataset,
            mmd_black: counts.mmd_black,
            mmd_hispanic: counts.mmd_hispanic,
            mmd_coalition: counts.mmd_coalition,
        })
    }

    fn score_compactness(
        &self,
        plan: &[u32],
        aggs: &mut Aggregates,
    ) -> Result<ShapeScores, ScoreError> {
        compactness_category(aggs, self.n_districts);

        // rdapy skips the border node, and water-only precincts the plan
        // does not mention.
        let skip: Vec<bool> = (0..self.adjacency.len())
            .map(|i| {
                Some(i as u32) == self.out_of_state
                    || (self.is_water_only[i] && plan[i] == UNASSIGNED)
            })
            .collect();
        let cuts = cut_score(&self.adjacency, plan, &skip)?;

        let n = self.n_precincts();
        let energy = calc_energy(&plan[..n], &self.pop, &self.center)
            .ok_or(ScoreError::EmptyDistrict)?;

        let reock = aggs.reock[0];
        let polsby = aggs.polsby_popper[0];

        Ok(ShapeScores {
            dataset: self.keys.shapes.clone(),
            reock,
            polsby_popper: polsby,
            cut_score: cuts,
            population_compactness: energy,
            compactness: rate::compactness(rate::reock(reock)?, rate::polsby(polsby)?),
        })
    }

    fn score_splitting(
        &self,
        opts: &ScoreOptions,
        aggs: &mut Aggregates,
        census: &mut CensusScores,
    ) -> Result<(), ScoreError> {
        let s = splitting_category(&aggs.cxd, false, &mut aggs.district_splitting);
        census.county_splitting = Some(s.county_splitting);
        census.district_splitting = Some(s.district_splitting);
        census.counties_split = Some(s.counties_split);
        census.county_splits = Some(s.county_splits);
        census.splitting = Some(rate::splitting(
            rate::county_splitting(s.county_splitting, self.n_counties as i32, self.n_districts as i32)?,
            rate::district_splitting(s.district_splitting, self.n_counties as i32, self.n_districts as i32)?,
        ));

        if opts.reverse_weight_splitting {
            // District splitting is unaffected by the county weighting, so
            // only the county figure is recomputed.
            let mut scratch = vec![0.0; aggs.district_splitting.len()];
            let r = splitting_category(&aggs.cxd, true, &mut scratch);
            census.county_splitting_reverse = Some(r.county_splitting);
            census.splitting_reverse = Some(rate::splitting(
                rate::county_splitting(r.county_splitting, self.n_counties as i32, self.n_districts as i32)?,
                rate::district_splitting(s.district_splitting, self.n_counties as i32, self.n_districts as i32)?,
            ));
        }
        Ok(())
    }
}

impl Scorecard {
    /// Round every floating-point score to [`SCORE_PRECISION`] places, as
    /// rdapy's `trim_scores` does. Integer scores and ratings are untouched.
    fn trim(&mut self) {
        let r = |x: f64| python_round_to(x, SCORE_PRECISION);
        let ro = |x: &mut Option<f64>| {
            if let Some(v) = x {
                *v = r(*v);
            }
        };

        if let Some(c) = &mut self.census {
            ro(&mut c.population_deviation);
            ro(&mut c.county_splitting);
            ro(&mut c.district_splitting);
            ro(&mut c.county_splitting_reverse);
        }
        if let Some(v) = &mut self.vap {
            v.opportunity_districts = r(v.opportunity_districts);
            v.coalition_districts = r(v.coalition_districts);
        }
        if let Some(s) = &mut self.shapes {
            s.reock = r(s.reock);
            s.polsby_popper = r(s.polsby_popper);
            s.population_compactness = r(s.population_compactness);
        }
        for e in &mut self.elections {
            e.estimated_vote_pct = r(e.estimated_vote_pct);
            e.pr_deviation = r(e.pr_deviation);
            e.estimated_seats = r(e.estimated_seats);
            e.disproportionality = r(e.disproportionality);
            e.efficiency_gap = r(e.efficiency_gap);
            e.efficiency_gap_fptp = r(e.efficiency_gap_fptp);
            e.votes_bias = r(e.votes_bias);
            e.geometric_seats_bias = r(e.geometric_seats_bias);
            e.mean_median_statewide = r(e.mean_median_statewide);
            e.mean_median_average_district = r(e.mean_median_average_district);
            e.turnout_bias = r(e.turnout_bias);
            e.competitive_districts = r(e.competitive_districts);
            e.average_margin = r(e.average_margin);
            e.responsiveness = r(e.responsiveness);
            e.responsive_districts = r(e.responsive_districts);
            ro(&mut e.efficiency_gap_wasted_votes);
            ro(&mut e.seats_bias);
            ro(&mut e.declination);
            ro(&mut e.lopsided_outcomes);
            ro(&mut e.overall_responsiveness);
            ro(&mut e.geographic_advantage);
        }
    }
}
