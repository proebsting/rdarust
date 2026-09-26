//! The by-district aggregates exchanged between pipeline stages.
//!
//! `aggregate` writes these, `score` reads them back. The layout is rdapy's:
//! dataset type, then the dataset the numbers came from, then the series.
//! Element 0 of each series is the statewide total.

use rdarust_core::aggregate::{Aggregates, Mode};
use rdarust_core::context::Context;
use serde_json::{json, Map, Value};

use crate::LoadError;

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|&x| json!(x)).collect())
}

fn floats(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|&x| json!(x)).collect())
}

fn wrap(dataset: &str, inner: Map<String, Value>) -> Value {
    let mut m = Map::new();
    m.insert(dataset.to_string(), Value::Object(inner));
    Value::Object(m)
}

/// Serialise aggregates as the `aggregate` stage emits them.
pub fn aggregates_to_value(aggs: &Aggregates, ctx: &Context, mode: Mode) -> Value {
    let mut out = Map::new();

    if mode.general() || mode.splitting() {
        let mut m = Map::new();
        if mode.general() {
            m.insert("pop_by_district".into(), ints(&aggs.pop_by_district));
        }
        if mode.splitting() {
            m.insert(
                "CxD".into(),
                Value::Array(aggs.cxd.iter().map(|r| floats(r)).collect()),
            );
        }
        out.insert("census".into(), wrap(&ctx.keys.census, m));
    }

    if mode.partisan() {
        let mut elections = Map::new();
        for (e, election) in ctx.elections.iter().enumerate() {
            let mut m = Map::new();
            m.insert("dem_by_district".into(), ints(&aggs.dem_by_district[e]));
            m.insert("tot_by_district".into(), ints(&aggs.tot_by_district[e]));
            elections.insert(election.key.clone(), Value::Object(m));
        }
        out.insert("election".into(), Value::Object(elections));
    }

    if mode.minority() {
        let mut m = Map::new();
        for (k, name) in ctx.vap.names.iter().enumerate() {
            m.insert(name.clone(), ints(&aggs.vap_by_district[k]));
        }
        out.insert("vap".into(), wrap(&ctx.keys.vap, m));

        if let (Some(cvap), Some(key)) = (&ctx.cvap, ctx.keys.cvap.as_deref()) {
            let mut m = Map::new();
            for (k, name) in cvap.names.iter().enumerate() {
                m.insert(name.clone(), ints(&aggs.cvap_by_district[k]));
            }
            out.insert("cvap".into(), wrap(key, m));
        }
    }

    if mode.compactness() {
        let mut m = Map::new();
        m.insert("area".into(), floats(&aggs.area));
        m.insert("perimeter".into(), floats(&aggs.perimeter));
        m.insert("diameter".into(), floats(&aggs.diameter));
        out.insert("shapes".into(), wrap(&ctx.keys.shapes, m));
    }

    Value::Object(out)
}

/// Serialise aggregates as the `score` stage emits them.
///
/// Scoring folds its by-district results back in -- Reock and Polsby-Popper
/// under shapes, split scores under census -- and drops the county-district
/// matrix, which was an input rather than a result.
pub fn scored_aggregates_to_value(aggs: &Aggregates, ctx: &Context, mode: Mode) -> Value {
    let mut v = aggregates_to_value(aggs, ctx, mode);

    if mode.compactness() {
        if let Some(m) = v
            .get_mut("shapes")
            .and_then(|s| s.get_mut(&ctx.keys.shapes))
            .and_then(|d| d.as_object_mut())
        {
            m.insert("reock".into(), floats(&aggs.reock));
            m.insert("polsby_popper".into(), floats(&aggs.polsby_popper));
        }
    }
    if mode.splitting() {
        if let Some(m) = v
            .get_mut("census")
            .and_then(|s| s.get_mut(&ctx.keys.census))
            .and_then(|d| d.as_object_mut())
        {
            m.insert("district_splitting".into(), floats(&aggs.district_splitting));
            m.remove("CxD");
        }
    }
    v
}

fn read_ints(v: &Value, what: &str) -> Result<Vec<i64>, LoadError> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect::<Vec<_>>())
        .ok_or_else(|| LoadError::Malformed { line: 0, what: format!("{what} is not an array") })
}

fn read_floats(v: &Value, what: &str) -> Result<Vec<f64>, LoadError> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_f64()).collect::<Vec<_>>())
        .ok_or_else(|| LoadError::Malformed { line: 0, what: format!("{what} is not an array") })
}

/// Read aggregates back in, as the `score` stage does.
///
/// Series the record does not carry are left zeroed, so a partisan-only
/// aggregate scores partisan metrics and nothing else.
pub fn aggregates_from_value(
    v: &Value,
    ctx: &Context,
    aggs: &mut Aggregates,
) -> Result<(), LoadError> {
    aggs.reset();

    let by_dataset = |ty: &str, key: &str| -> Option<&Map<String, Value>> {
        v.get(ty)?.get(key)?.as_object()
    };

    if let Some(census) = by_dataset("census", &ctx.keys.census) {
        if let Some(p) = census.get("pop_by_district") {
            aggs.pop_by_district = read_ints(p, "pop_by_district")?;
        }
        if let Some(c) = census.get("CxD") {
            let rows = c.as_array().ok_or_else(|| LoadError::Malformed {
                line: 0,
                what: "CxD is not an array".into(),
            })?;
            aggs.cxd = rows
                .iter()
                .map(|r| read_floats(r, "CxD row"))
                .collect::<Result<_, _>>()?;
        }
    }

    for (e, election) in ctx.elections.iter().enumerate() {
        if let Some(m) = by_dataset("election", &election.key) {
            if let Some(d) = m.get("dem_by_district") {
                aggs.dem_by_district[e] = read_ints(d, "dem_by_district")?;
            }
            if let Some(t) = m.get("tot_by_district") {
                aggs.tot_by_district[e] = read_ints(t, "tot_by_district")?;
            }
        }
    }

    if let Some(m) = by_dataset("vap", &ctx.keys.vap) {
        for (k, name) in ctx.vap.names.iter().enumerate() {
            if let Some(s) = m.get(name) {
                aggs.vap_by_district[k] = read_ints(s, name)?;
            }
        }
    }
    if let (Some(cvap), Some(key)) = (&ctx.cvap, ctx.keys.cvap.as_deref()) {
        if let Some(m) = by_dataset("cvap", key) {
            for (k, name) in cvap.names.iter().enumerate() {
                if let Some(s) = m.get(name) {
                    aggs.cvap_by_district[k] = read_ints(s, name)?;
                }
            }
        }
    }

    if let Some(m) = by_dataset("shapes", &ctx.keys.shapes) {
        for (name, target) in [
            ("area", &mut aggs.area),
            ("perimeter", &mut aggs.perimeter),
            ("diameter", &mut aggs.diameter),
        ] {
            if let Some(s) = m.get(name) {
                *target = read_floats(s, name)?;
            }
        }
    }

    Ok(())
}
