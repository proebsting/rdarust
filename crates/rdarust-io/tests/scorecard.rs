//! End-to-end conformance: score real plans and compare whole scorecards
//! against rdapy.
//!
//! The per-function corpus checks the formulas in isolation. This checks the
//! pipeline that feeds them -- reading precinct data, interning geoids,
//! aggregating by district -- on the actual North Carolina and New Jersey
//! data rdapy's own `test_scorecard.py` uses, plus a sample of real ensemble
//! plans.

use std::collections::HashMap;
use std::path::PathBuf;

use rdarust_core::aggregate::Mode;
use rdarust_core::score::{ModeOpt, ScoreOptions};
use rdarust_io::{
    load_graph, load_input_data, read_plan_csv, read_plan_jsonl, scorecard_to_value,
};
use serde_json::Value;

/// Floats must agree to this, relative. Integers and ratings must be exact.
const FLOAT_TOL: f64 = 1e-9;

fn rdapy_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/rdapy")
        .join(rel)
}

fn cases_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/cases/scorecard/scorecards.json")
}

/// Compare a scored value against rdapy's, returning the worst relative error.
fn compare(got: &Value, want: &Value, path: &str, label: &str) -> f64 {
    match (got, want) {
        (Value::Null, Value::Null) => 0.0,
        (Value::Number(g), Value::Number(w)) => {
            // An integer score is something a user reads; it must be exact.
            if w.is_i64() || w.is_u64() {
                assert_eq!(
                    g.as_i64(),
                    w.as_i64(),
                    "{label}{path}: integer score differs"
                );
                return 0.0;
            }
            let (g, w) = (g.as_f64().unwrap(), w.as_f64().unwrap());
            let err = (g - w).abs() / w.abs().max(1.0);
            assert!(err <= FLOAT_TOL, "{label}{path}: got {g:?}, want {w:?} (rel err {err:e})");
            err
        }
        (Value::Array(g), Value::Array(w)) => {
            assert_eq!(g.len(), w.len(), "{label}{path}: length");
            let mut worst = 0.0f64;
            for (i, (gi, wi)) in g.iter().zip(w.iter()).enumerate() {
                worst = worst.max(compare(gi, wi, &format!("{path}[{i}]"), label));
            }
            worst
        }
        (Value::Object(g), Value::Object(w)) => {
            let mut gk: Vec<&String> = g.keys().collect();
            let mut wk: Vec<&String> = w.keys().collect();
            gk.sort();
            wk.sort();
            assert_eq!(gk, wk, "{label}{path}: metric names differ");
            let mut worst = 0.0f64;
            for (k, wv) in w {
                worst = worst.max(compare(&g[k], wv, &format!("{path}.{k}"), label));
            }
            worst
        }
        (g, w) => panic!("{label}{path}: got {g}, want {w}"),
    }
}

/// Whole seats per election from a precomputed geographic baseline.
///
/// rdapy nests these under `geographic_baseline -> election -> whole_seats`;
/// an election missing from the file simply gets no geographic advantage
/// score.
fn load_geographic_baselines(path: &std::path::Path) -> HashMap<String, f64> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("cannot read {}: {e}", path.display())
    });
    let v: Value = serde_json::from_str(&text).unwrap();
    v.get("geographic_baseline")
        .and_then(|b| b.as_object())
        .map(|b| {
            b.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.get("whole_seats")?.as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn scorecards_match_rdapy() {
    let text = std::fs::read_to_string(cases_path()).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\nrun conformance/tools/gen_scorecards.py",
            cases_path().display()
        )
    });
    let doc: Value = serde_json::from_str(&text).unwrap();
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());

    // Loading a state's precinct data is the expensive part; reuse it.
    let mut loaded: HashMap<String, rdarust_core::context::Context> = HashMap::new();
    let mut worst = 0.0f64;
    let mut worst_where = String::new();

    for case in cases {
        let name = case["name"].as_str().unwrap_or("?");
        let state = case["state"].as_str().expect("state");
        let plan_type = case["plan_type"].as_str().expect("plan type");
        let data_rel = case["data"].as_str().expect("data path");

        // Files written for the scoring pipeline carry no neighbour lists;
        // the graph is a separate input.
        let graph_rel = case["graph"].as_str();
        let key = format!("{state}/{plan_type}/{data_rel}/{graph_rel:?}");
        let ctx = loaded.entry(key).or_insert_with(|| {
            let mut input = load_input_data(rdapy_path(data_rel))
                .unwrap_or_else(|e| panic!("{name}: loading {data_rel}: {e}"));
            if let Some(g) = graph_rel {
                input = input.with_graph(
                    load_graph(rdapy_path(g)).unwrap_or_else(|e| panic!("{name}: loading {g}: {e}")),
                );
            }
            input
                .into_context(state, plan_type, None)
                .unwrap_or_else(|e| panic!("{name}: building context: {e}"))
        });

        let spec = &case["plan"];
        let assignments = match spec["kind"].as_str() {
            Some("csv") => read_plan_csv(rdapy_path(spec["path"].as_str().unwrap())),
            Some("jsonl") => read_plan_jsonl(
                rdapy_path(spec["path"].as_str().unwrap()),
                spec["index"].as_u64().unwrap_or(0) as usize,
            ),
            other => panic!("{name}: unknown plan kind {other:?}"),
        }
        .unwrap_or_else(|e| panic!("{name}: reading plan: {e}"));

        let plan = ctx.plan_from_assignments(assignments.iter().map(|(g, d)| (g.as_str(), *d)));

        let opts = ScoreOptions {
            mode: ModeOpt(Mode::All),
            mmd_scoring: case["options"]["mmd_scoring"].as_bool().unwrap_or(true),
            reverse_weight_splitting: case["options"]["reverse_weight_splitting"]
                .as_bool()
                .unwrap_or(false),
            geographic_baselines: match case["precomputed"].as_str() {
                Some(p) => load_geographic_baselines(&rdapy_path(p)),
                None => HashMap::new(),
            },
        };

        let (card, aggs) = ctx
            .score(&plan, &opts)
            .unwrap_or_else(|e| panic!("{name}: scoring: {e}"));

        let got = scorecard_to_value(&card, &ctx.keys, Mode::All);
        let err = compare(&got, &case["expect"], "", name);
        if err > worst {
            worst = err;
            worst_where = name.to_string();
        }

        // The by-district series scoring folds back into the aggregates.
        let bd = &case["by_district"];
        for (metric, series) in [
            ("reock", &aggs.reock),
            ("polsby_popper", &aggs.polsby_popper),
            ("district_splitting", &aggs.district_splitting),
        ] {
            let Some(want) = bd.get(metric).and_then(|v| v.as_array()) else {
                continue;
            };
            assert_eq!(series.len(), want.len(), "{name}: {metric} length");
            for (i, w) in want.iter().enumerate() {
                let w = w.as_f64().unwrap();
                let e = (series[i] - w).abs() / w.abs().max(1.0);
                assert!(
                    e <= FLOAT_TOL,
                    "{name}: by-district {metric}[{i}] got {}, want {w} (rel err {e:e})",
                    series[i]
                );
                if e > worst {
                    worst = e;
                    worst_where = format!("{name} by-district {metric}");
                }
            }
        }
    }

    eprintln!(
        "scorecard: {} plans scored end to end match rdapy; worst rel err {worst:e} at {worst_where}",
        cases.len()
    );
}
