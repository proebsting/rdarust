//! Conformance tests against rdapy, driven by `conformance/cases/traced/`.
//!
//! Those files are recorded by instrumenting rdapy and running rdapy's own
//! pytest suite (`conformance/tools/trace_rdapy_tests.py`), so the arguments
//! are exactly what rdapy's tests exercise. A case carrying a `from` field was
//! asserted directly by that test -- a published reference value -- while the
//! rest are interior calls, sampled under a size budget.

use rdarust_core::equal::calc_population_deviation;
use rdarust_core::minority::majority_minority as mmd;
use rdarust_core::minority::opportunity::{self as opp, DemoShares, DEMOGRAPHICS};
use rdarust_core::partisan::{bias, method, more, responsiveness};
use rdarust_core::splitting::{coi, county};
use serde_json::Value;

const FLOAT_TOL: f64 = 1e-9;

/// A result tree: scalars, lists and maps, compared structurally.
#[derive(Debug, Clone, PartialEq)]
enum Out {
    F(f64),
    I(i64),
    B(bool),
    Null,
    List(Vec<Out>),
    Map(Vec<(String, Out)>),
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("expected a number, got {v}"))
}
fn farr(v: &Value) -> Vec<f64> {
    v.as_array().expect("array").iter().map(f).collect()
}
fn iarr(v: &Value) -> Vec<i64> {
    v.as_array().expect("array").iter().map(|x| x.as_i64().expect("int")).collect()
}
fn matrix(v: &Value) -> Vec<Vec<f64>> {
    v.as_array().expect("matrix").iter().map(farr).collect()
}
fn pairs(v: &Value) -> Vec<(f64, f64)> {
    v.as_array().expect("pairs").iter().map(|p| (f(&p[0]), f(&p[1]))).collect()
}

fn nums(v: impl IntoIterator<Item = f64>) -> Out {
    Out::List(v.into_iter().map(Out::F).collect())
}
fn mat(v: Vec<Vec<f64>>) -> Out {
    Out::List(v.into_iter().map(nums).collect())
}
fn pairs_out(v: Vec<(f64, f64)>) -> Out {
    Out::List(v.into_iter().map(|(a, b)| Out::List(vec![Out::F(a), Out::F(b)])).collect())
}
fn opt(o: Option<f64>) -> Out {
    o.map(Out::F).unwrap_or(Out::Null)
}
/// Build a map sorted by key, matching how expected maps are normalised.
fn map(kv: Vec<(&str, Out)>) -> Out {
    let mut v: Vec<(String, Out)> = kv.into_iter().map(|(k, o)| (k.to_string(), o)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    Out::Map(v)
}

/// rdapy passes demographics as dicts keyed by name; "white" may be absent.
fn shares(v: &Value) -> DemoShares {
    let obj = v.as_object().expect("demographics object");
    let mut d = DemoShares::default();
    for (idx, name) in DEMOGRAPHICS.iter().enumerate() {
        if let Some(x) = obj.get(*name) {
            d.set(idx, f(x));
        }
    }
    d
}

fn kwarg_bool(kw: Option<&Value>, name: &str, default: bool) -> bool {
    kw.and_then(|k| k.get(name)).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn call(function: &str, a: &[Value], kw: Option<&Value>) -> Out {
    match function {
        // ---------------- partisan / method ----------------
        "est_seat_probability" => Out::F(method::est_seat_probability(f(&a[0]))),
        "est_district_responsiveness" => Out::F(method::est_district_responsiveness(f(&a[0]))),
        "est_seats" => Out::F(method::est_seats(&farr(&a[0]))),
        "est_fptp_seats" => Out::I(method::est_fptp_seats(&farr(&a[0])) as i64),
        "infer_sv_points" => {
            let prop = a.get(2).and_then(|v| v.as_bool())
                .unwrap_or_else(|| kwarg_bool(kw, "proportional", true));
            pairs_out(method::infer_sv_points(f(&a[0]), &farr(&a[1]), prop))
        }
        "infer_inverse_sv_points" => {
            pairs_out(method::infer_inverse_sv_points(&pairs(&a[0]), f(&a[1]) as i32))
        }
        "infer_geometric_seats_bias_points" => pairs_out(
            method::infer_geometric_seats_bias_points(&pairs(&a[0]), &pairs(&a[1])),
        ),

        // ---------------- partisan / bias ----------------
        "calc_best_seats" => Out::I(bias::calc_best_seats(f(&a[0]) as i32, f(&a[1])) as i64),
        "calc_disproportionality" => Out::F(bias::calc_disproportionality(f(&a[0]), f(&a[1]))),
        "calc_disproportionality_from_best" => {
            Out::F(bias::calc_disproportionality_from_best(f(&a[0]), f(&a[1])))
        }
        "calc_efficiency_gap" => Out::F(bias::calc_efficiency_gap(f(&a[0]), f(&a[1]))),
        "calc_gamma" => Out::F(bias::calc_gamma(f(&a[0]), f(&a[1]), f(&a[2]))),
        "est_seats_bias" => opt(bias::est_seats_bias(&pairs(&a[0]), f(&a[1]) as i32)),
        "est_votes_bias" => {
            Out::F(bias::est_votes_bias(&pairs(&a[0]), f(&a[1]) as i32).expect("votes bias"))
        }
        "est_geometric_seats_bias" => Out::F(
            bias::est_geometric_seats_bias(f(&a[0]), &pairs(&a[1]), &pairs(&a[2]))
                .expect("geometric seats bias"),
        ),
        "calc_global_symmetry" => Out::F(bias::calc_global_symmetry(
            &pairs(&a[0]), &pairs(&a[1]), f(&a[2]), f(&a[3]) as i32,
        )),
        "key_RV_points" => {
            let k = bias::key_rv_points(&farr(&a[0]));
            map(vec![
                ("Sb", Out::F(k.sb)), ("Ra", Out::F(k.ra)), ("Rb", Out::F(k.rb)),
                ("Va", Out::F(k.va)), ("Vb", Out::F(k.vb)),
            ])
        }
        "is_sweep" => Out::B(bias::is_sweep(f(&a[0]), f(&a[1]) as usize)),
        "calc_declination" => opt(bias::calc_declination(&farr(&a[0]))),
        "calc_mean_median_difference" => {
            Out::F(bias::calc_mean_median_difference(&farr(&a[0]), a.get(1).map(f)))
        }
        "calc_turnout_bias" => Out::F(bias::calc_turnout_bias(f(&a[0]), &farr(&a[1]))),
        "calc_lopsided_outcomes" => opt(bias::calc_lopsided_outcomes(&farr(&a[0]))),

        // ---------------- partisan / responsiveness ----------------
        "count_competitive_districts" => {
            Out::I(responsiveness::count_competitive_districts(&farr(&a[0])) as i64)
        }
        "est_competitive_districts" => {
            Out::F(responsiveness::est_competitive_districts(&farr(&a[0])))
        }
        "est_district_competitiveness" => {
            Out::F(responsiveness::est_district_competitiveness(f(&a[0])))
        }
        "est_responsive_districts" => {
            Out::F(responsiveness::est_responsive_districts(&farr(&a[0])))
        }
        "est_responsiveness" => opt(responsiveness::est_responsiveness(
            f(&a[0]), &pairs(&a[1]), f(&a[2]) as i32,
        )),
        "calc_big_R" => opt(responsiveness::calc_big_r(f(&a[0]), f(&a[1]))),
        "calc_minimal_inverse_responsiveness" => {
            opt(responsiveness::calc_minimal_inverse_responsiveness(f(&a[0]), f(&a[1])))
        }

        // ---------------- partisan / more ----------------
        "calc_efficiency_gap_wasted_votes" => {
            opt(more::calc_efficiency_gap_wasted_votes(&iarr(&a[0]), &iarr(&a[1])))
        }
        "calc_average_margin" => Out::F(more::calc_average_margin(&farr(&a[0]))),

        // ---------------- equal ----------------
        "calc_population_deviation" => Out::F(calc_population_deviation(
            f(&a[0]) as i64, f(&a[1]) as i64, f(&a[2]) as i64,
        )),

        // ---------------- splitting / county ----------------
        "calc_splitting_metrics" => {
            let rw = kwarg_bool(kw, "reverse_weight", false);
            let m = county::calc_splitting_metrics(&matrix(&a[0]), rw);
            map(vec![("county", Out::F(m.county)), ("district", Out::F(m.district))])
        }
        "split_score" => Out::F(county::split_score(&farr(&a[0]))),
        "_county_totals" => nums(county::county_totals(&matrix(&a[0]))),
        "_district_totals" => nums(county::district_totals(&matrix(&a[0]))),
        "_reduce_county_splits" => {
            mat(county::reduce_county_splits(&matrix(&a[0]), &farr(&a[1])))
        }
        "_reduce_district_splits" => {
            mat(county::reduce_district_splits(&matrix(&a[0]), &farr(&a[1])))
        }
        "_calc_county_weights" => {
            let rw = a.get(1).and_then(|v| v.as_bool())
                .unwrap_or_else(|| kwarg_bool(kw, "reverse_weight", false));
            nums(county::calc_county_weights(&farr(&a[0]), rw))
        }
        "_calc_district_weights" => nums(county::calc_district_weights(&farr(&a[0]))),
        "_calc_county_fractions" => {
            mat(county::calc_county_fractions(&matrix(&a[0]), &farr(&a[1])))
        }
        "_calc_district_fractions" => {
            mat(county::calc_district_fractions(&matrix(&a[0]), &farr(&a[1])))
        }
        "_county_split_score" => {
            Out::F(county::county_split_score(f(&a[0]) as usize, &matrix(&a[1])))
        }
        "_district_split_score" => {
            Out::F(county::district_split_score(f(&a[0]) as usize, &matrix(&a[1])))
        }
        "_county_splitting" => Out::F(county::county_splitting(&matrix(&a[0]), &farr(&a[1]))),
        "_district_splitting" => {
            Out::F(county::district_splitting(&matrix(&a[0]), &farr(&a[1])))
        }
        // rdapy takes both totals for symmetry; each uses only one of them.
        "_calc_county_splitting" => {
            Out::F(county::calc_county_splitting(&matrix(&a[0]), &farr(&a[2])))
        }
        "_calc_county_splitting_reduced" => Out::F(county::calc_county_splitting_reduced(
            &matrix(&a[0]), &farr(&a[1]), &farr(&a[2]),
            kwarg_bool(kw, "reverse_weight", false),
        )),
        "_calc_district_splitting" => {
            Out::F(county::calc_district_splitting(&matrix(&a[0]), &farr(&a[1])))
        }
        "_calc_district_splitting_reduced" => Out::F(county::calc_district_splitting_reduced(
            &matrix(&a[0]), &farr(&a[1]), &farr(&a[2]),
        )),
        "_population_weight" => {
            Out::F(county::population_weight(f(&a[0]), f(&a[1]) as usize, f(&a[2])))
        }
        "_reverse_weight" => {
            Out::F(county::reverse_weight(f(&a[0]), f(&a[1]) as usize, f(&a[2])))
        }

        // ---------------- splitting / coi ----------------
        "calc_coi_splitting" => {
            let communities: Vec<(String, Vec<f64>)> = a[0]
                .as_array()
                .expect("communities")
                .iter()
                .map(|c| (c["name"].as_str().expect("name").to_string(), farr(&c["splits"])))
                .collect();
            // rdapy wraps the per-community list in {"byCOI": ...}; that is a
            // serialisation shape, not something the Rust API should carry.
            map(vec![(
                "byCOI",
                Out::List(
                    coi::calc_coi_splitting(&communities)
                        .into_iter()
                        .map(|r| {
                            map(vec![
                                ("name", Out::Null),
                                ("effectiveSplits", Out::F(r.effective_splits)),
                                ("uncertainty", Out::F(r.uncertainty)),
                            ])
                        })
                        .collect(),
                ),
            )])
        }
        "uncertainty_of_membership" => Out::F(coi::uncertainty_of_membership(&farr(&a[0]))),
        "effective_splits" => Out::F(coi::effective_splits(&farr(&a[0]))),

        // ---------------- minority ----------------
        "calc_proportional_districts" => {
            Out::I(opp::calc_proportional_districts(f(&a[0]), f(&a[1]) as i32) as i64)
        }
        "est_minority_opportunity" => {
            let demo = a.get(1).and_then(|v| v.as_str());
            Out::F(opp::est_minority_opportunity(
                f(&a[0]), demo, kwarg_bool(kw, "clip", true),
            ))
        }
        "calc_minority_metrics" => {
            let statewide = shares(&a[0]);
            let by_district: Vec<DemoShares> =
                a[1].as_array().expect("districts").iter().map(shares).collect();
            let m = opp::calc_minority_metrics(
                &statewide, &by_district, kwarg_bool(kw, "clip", true),
            );
            map(vec![
                ("opportunity_districts", Out::F(m.opportunity_districts)),
                ("proportional_opportunities", Out::I(m.proportional_opportunities as i64)),
                ("coalition_districts", Out::F(m.coalition_districts)),
                ("proportional_coalitions", Out::I(m.proportional_coalitions as i64)),
            ])
        }
        "calculate_mmd_simple" => {
            let o = a[0].as_object().expect("cvap aggregates");
            let c = mmd::calculate_mmd_simple(
                &farr(&o["black_cvap"])[1..],
                &farr(&o["hispanic_cvap"])[1..],
                &farr(&o["total_cvap"])[1..],
            );
            map(vec![
                ("mmd_black", Out::I(c.mmd_black as i64)),
                ("mmd_hispanic", Out::I(c.mmd_hispanic as i64)),
                ("mmd_coalition", Out::I(c.mmd_coalition as i64)),
            ])
        }
        "_is_single_demo_mmd" => Out::B(mmd::is_single_demo_mmd(f(&a[0]), f(&a[1]))),
        "_is_coalition_mmd" => Out::B(mmd::is_coalition_mmd(&farr(&a[0]), f(&a[1]))),

        other => panic!("no Rust binding for traced function {other}"),
    }
}

fn expected(v: &Value) -> Out {
    match v {
        Value::Null => Out::Null,
        Value::Bool(b) => Out::B(*b),
        Value::Number(n) if n.is_i64() || n.is_u64() => Out::I(n.as_i64().unwrap()),
        Value::Number(n) => Out::F(n.as_f64().unwrap()),
        Value::Array(xs) => Out::List(xs.iter().map(expected).collect()),
        Value::Object(m) => {
            // String fields carry labels, not results; normalise them away so
            // the numeric comparison is what decides.
            let mut kv: Vec<(String, Out)> = m
                .iter()
                .map(|(k, x)| {
                    (k.clone(), if x.is_string() { Out::Null } else { expected(x) })
                })
                .collect();
            kv.sort_by(|a, b| a.0.cmp(&b.0));
            Out::Map(kv)
        }
        other => panic!("unsupported expected value: {other}"),
    }
}

/// Compare recursively, returning the worst relative error seen.
fn compare(got: &Out, want: &Out, path: &str, label: &str) -> f64 {
    match (got, want) {
        // Python's min/max return the int literal when it wins, so a float
        // result can have been recorded as an integer.
        (Out::F(g), Out::F(w)) => check_float(*g, *w, path, label),
        (Out::F(g), Out::I(w)) => check_float(*g, *w as f64, path, label),
        (Out::I(g), Out::F(w)) => {
            assert_eq!(w.fract(), 0.0, "{label}{path}: expected {w} is not an integer");
            assert_eq!(*g, *w as i64, "{label}{path}");
            0.0
        }
        (Out::List(g), Out::List(w)) => {
            assert_eq!(g.len(), w.len(), "{label}{path}: length");
            let mut worst = 0.0f64;
            for (i, (gi, wi)) in g.iter().zip(w.iter()).enumerate() {
                worst = worst.max(compare(gi, wi, &format!("{path}[{i}]"), label));
            }
            worst
        }
        (Out::Map(g), Out::Map(w)) => {
            assert_eq!(
                g.iter().map(|x| &x.0).collect::<Vec<_>>(),
                w.iter().map(|x| &x.0).collect::<Vec<_>>(),
                "{label}{path}: keys"
            );
            let mut worst = 0.0f64;
            for ((gk, gv), (_, wv)) in g.iter().zip(w.iter()) {
                worst = worst.max(compare(gv, wv, &format!("{path}.{gk}"), label));
            }
            worst
        }
        (g, w) => {
            assert_eq!(g, w, "{label}{path}");
            0.0
        }
    }
}

fn check_float(g: f64, w: f64, path: &str, label: &str) -> f64 {
    let err = (g - w).abs() / w.abs().max(1.0);
    assert!(err <= FLOAT_TOL, "{label}{path}: got {g:?}, want {w:?} (rel err {err:e})");
    err
}

#[test]
fn traced_calls_match_rdapy() {
    // "traced" is recorded from rdapy's own suite; "supplement" covers
    // functions that suite never calls.
    let mut files: Vec<_> = ["traced", "supplement"]
        .iter()
        .flat_map(|sub| {
            let dir = format!("{}/../../conformance/cases/{sub}", env!("CARGO_MANIFEST_DIR"));
            std::fs::read_dir(&dir)
                .unwrap_or_else(|e| {
                    panic!("cannot read {dir}: {e}\nrun the generators in conformance/tools/")
                })
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect::<Vec<_>>()
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no case files found");

    let (mut total, mut direct, mut worst) = (0usize, 0usize, 0.0f64);
    let mut worst_where = String::new();
    let mut functions = 0usize;

    for path in files {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let function = doc["function"].as_str().expect("function name");
        functions += 1;

        for (idx, c) in doc["cases"].as_array().unwrap().iter().enumerate() {
            let args = c["input"].as_array().expect("input array");
            let kw = c.get("kwargs");
            let from = c["from"].as_str();
            let label = format!(
                "{function}[{idx}]{}",
                from.map(|s| format!(" (from {s})")).unwrap_or_default()
            );

            let err = compare(&call(function, args, kw), &expected(&c["expect"]), "", &label);
            if err > worst {
                worst = err;
                worst_where = label.clone();
            }

            total += 1;
            if from.is_some() {
                direct += 1;
            }
        }
    }

    eprintln!(
        "traced: {total} cases across {functions} functions pass \
         ({direct} asserted directly by rdapy's tests); \
         worst rel err {worst:e} at {worst_where}"
    );
}
