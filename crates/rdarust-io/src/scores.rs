//! Serialising a [`Scorecard`] into the nested shape rdapy emits.
//!
//! Scores are grouped by dataset type, then by the dataset they were computed
//! against, then by metric -- because a plan can be scored against several
//! elections at once and the results have to stay distinguishable.

use rdarust_core::aggregate::Mode;
use rdarust_core::context::DatasetKeys;
use rdarust_core::score::Scorecard;
use serde_json::{json, Map, Value};

fn num(v: f64) -> Value {
    json!(v)
}

fn opt(v: Option<f64>) -> Value {
    match v {
        Some(x) => num(x),
        None => Value::Null,
    }
}

fn wrap(dataset: &str, metrics: Map<String, Value>) -> Value {
    let mut outer = Map::new();
    outer.insert(dataset.to_string(), Value::Object(metrics));
    Value::Object(outer)
}

/// Convert to rdapy's `type -> dataset -> metric` structure.
///
/// `mode` is needed because rdapy emits an empty dataset object for a family
/// it computed nothing for -- notably CVAP when majority-minority scoring is
/// off -- and dropping those would make outputs diff-incompatible.
pub fn scorecard_to_value(card: &Scorecard, keys: &DatasetKeys, mode: Mode) -> Value {
    let mut out = Map::new();

    if mode.general() || mode.splitting() {
        let mut m = Map::new();
        if let Some(c) = &card.census {
            if let Some(v) = c.population_deviation {
                m.insert("population_deviation".into(), num(v));
            }
            if let Some(v) = c.county_splitting {
                m.insert("county_splitting".into(), num(v));
            }
            if let Some(v) = c.district_splitting {
                m.insert("district_splitting".into(), num(v));
            }
            if let Some(v) = c.counties_split {
                m.insert("counties_split".into(), json!(v));
            }
            if let Some(v) = c.county_splits {
                m.insert("county_splits".into(), json!(v));
            }
            if let Some(v) = c.splitting {
                m.insert("splitting".into(), json!(v));
            }
            if let Some(v) = c.county_splitting_reverse {
                m.insert("county_splitting_reverse".into(), num(v));
            }
            if let Some(v) = c.splitting_reverse {
                m.insert("splitting_reverse".into(), json!(v));
            }
        }
        out.insert("census".into(), wrap(&keys.census, m));
    }

    if mode.minority() {
        let mut m = Map::new();
        if let Some(v) = &card.vap {
            m.insert("opportunity_districts".into(), num(v.opportunity_districts));
            m.insert("proportional_opportunities".into(), json!(v.proportional_opportunities));
            m.insert("coalition_districts".into(), num(v.coalition_districts));
            m.insert("proportional_coalitions".into(), json!(v.proportional_coalitions));
            m.insert("minority".into(), json!(v.minority));
        }
        out.insert("vap".into(), wrap(&keys.vap, m));

        let mut m = Map::new();
        if let Some(v) = &card.cvap {
            m.insert("mmd_black".into(), json!(v.mmd_black));
            m.insert("mmd_hispanic".into(), json!(v.mmd_hispanic));
            m.insert("mmd_coalition".into(), json!(v.mmd_coalition));
        }
        // rdapy uses "N/A" as the key when there is no CVAP dataset.
        out.insert("cvap".into(), wrap(keys.cvap.as_deref().unwrap_or("N/A"), m));
    }

    if mode.partisan() {
        let mut elections = Map::new();
        for e in &card.elections {
            let mut m = Map::new();
            m.insert("pr_deviation".into(), num(e.pr_deviation));
            m.insert("estimated_seats".into(), num(e.estimated_seats));
            m.insert("fptp_seats".into(), json!(e.fptp_seats));
            m.insert("disproportionality".into(), num(e.disproportionality));
            m.insert("efficiency_gap".into(), num(e.efficiency_gap));
            m.insert("efficiency_gap_FPTP".into(), num(e.efficiency_gap_fptp));
            m.insert("seats_bias".into(), opt(e.seats_bias));
            m.insert("votes_bias".into(), num(e.votes_bias));
            m.insert("geometric_seats_bias".into(), num(e.geometric_seats_bias));
            m.insert("declination".into(), opt(e.declination));
            m.insert("mean_median_statewide".into(), num(e.mean_median_statewide));
            m.insert(
                "mean_median_average_district".into(),
                num(e.mean_median_average_district),
            );
            m.insert("turnout_bias".into(), num(e.turnout_bias));
            m.insert("lopsided_outcomes".into(), opt(e.lopsided_outcomes));
            m.insert(
                "competitive_district_count".into(),
                json!(e.competitive_district_count),
            );
            m.insert("competitive_districts".into(), num(e.competitive_districts));
            m.insert("average_margin".into(), num(e.average_margin));
            m.insert("responsiveness".into(), num(e.responsiveness));
            m.insert("responsive_districts".into(), num(e.responsive_districts));
            m.insert("overall_responsiveness".into(), opt(e.overall_responsiveness));
            m.insert("estimated_vote_pct".into(), num(e.estimated_vote_pct));
            m.insert(
                "efficiency_gap_wasted_votes".into(),
                opt(e.efficiency_gap_wasted_votes),
            );
            if let Some(v) = e.geographic_advantage {
                m.insert("geographic_advantage".into(), num(v));
            }
            m.insert("proportionality".into(), json!(e.proportionality));
            m.insert("competitiveness".into(), json!(e.competitiveness));
            elections.insert(e.dataset.clone(), Value::Object(m));
        }
        out.insert("election".into(), Value::Object(elections));
    }

    if mode.compactness() {
        let mut m = Map::new();
        if let Some(s) = &card.shapes {
            m.insert("reock".into(), num(s.reock));
            m.insert("polsby_popper".into(), num(s.polsby_popper));
            m.insert("cut_score".into(), json!(s.cut_score));
            m.insert("population_compactness".into(), num(s.population_compactness));
            m.insert("compactness".into(), json!(s.compactness));
        }
        out.insert("shapes".into(), wrap(&keys.shapes, m));
    }

    Value::Object(out)
}
