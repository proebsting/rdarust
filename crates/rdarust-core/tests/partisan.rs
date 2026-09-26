//! Conformance tests for the partisan suite and population deviation.
//!
//! Reads `conformance/cases/traced/`, recorded by instrumenting rdapy and
//! running rdapy's own pytest suite (see
//! `conformance/tools/trace_rdapy_tests.py`). Cases carrying a `from` field
//! were asserted directly by the named test -- those are rdapy's published
//! reference values. The rest are interior calls, sampled.

use rdarust_core::equal::calc_population_deviation;
use rdarust_core::partisan::{bias, method, more, responsiveness};
use serde_json::Value;

const FLOAT_TOL: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
enum Out {
    F(f64),
    I(i64),
    B(bool),
    Null,
    Pairs(Vec<(f64, f64)>),
    Map(Vec<(String, f64)>),
}

fn f(v: &Value) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("expected a number, got {v}"))
}

fn farr(v: &Value) -> Vec<f64> {
    v.as_array().expect("array").iter().map(f).collect()
}

fn iarr(v: &Value) -> Vec<i64> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_i64().expect("integer"))
        .collect()
}

fn pairs(v: &Value) -> Vec<(f64, f64)> {
    v.as_array()
        .expect("array of pairs")
        .iter()
        .map(|p| (f(&p[0]), f(&p[1])))
        .collect()
}

fn opt(o: Option<f64>) -> Out {
    match o {
        Some(v) => Out::F(v),
        None => Out::Null,
    }
}

/// Call the Rust port of the named rdapy function on a traced case's inputs.
fn call(function: &str, a: &[Value]) -> Out {
    match function {
        // ---- method ----
        "est_seat_probability" => Out::F(method::est_seat_probability(f(&a[0]))),
        "est_district_responsiveness" => {
            Out::F(method::est_district_responsiveness(f(&a[0])))
        }
        "est_seats" => Out::F(method::est_seats(&farr(&a[0]))),
        "est_fptp_seats" => Out::I(method::est_fptp_seats(&farr(&a[0])) as i64),
        "infer_sv_points" => {
            let proportional = a.get(2).and_then(|v| v.as_bool()).unwrap_or(true);
            Out::Pairs(method::infer_sv_points(f(&a[0]), &farr(&a[1]), proportional))
        }
        "infer_inverse_sv_points" => Out::Pairs(method::infer_inverse_sv_points(
            &pairs(&a[0]),
            f(&a[1]) as i32,
        )),
        "infer_geometric_seats_bias_points" => Out::Pairs(
            method::infer_geometric_seats_bias_points(&pairs(&a[0]), &pairs(&a[1])),
        ),

        // ---- bias ----
        "calc_best_seats" => Out::I(bias::calc_best_seats(f(&a[0]) as i32, f(&a[1])) as i64),
        "calc_disproportionality" => {
            Out::F(bias::calc_disproportionality(f(&a[0]), f(&a[1])))
        }
        "calc_disproportionality_from_best" => {
            Out::F(bias::calc_disproportionality_from_best(f(&a[0]), f(&a[1])))
        }
        "calc_efficiency_gap" => Out::F(bias::calc_efficiency_gap(f(&a[0]), f(&a[1]))),
        "calc_gamma" => Out::F(bias::calc_gamma(f(&a[0]), f(&a[1]), f(&a[2]))),
        "est_seats_bias" => opt(bias::est_seats_bias(&pairs(&a[0]), f(&a[1]) as i32)),
        "est_votes_bias" => Out::F(
            bias::est_votes_bias(&pairs(&a[0]), f(&a[1]) as i32).expect("votes bias"),
        ),
        "est_geometric_seats_bias" => Out::F(
            bias::est_geometric_seats_bias(f(&a[0]), &pairs(&a[1]), &pairs(&a[2]))
                .expect("geometric seats bias"),
        ),
        "calc_global_symmetry" => Out::F(bias::calc_global_symmetry(
            &pairs(&a[0]),
            &pairs(&a[1]),
            f(&a[2]),
            f(&a[3]) as i32,
        )),
        "key_RV_points" => {
            let k = bias::key_rv_points(&farr(&a[0]));
            // Sorted by key, so it lines up with the expected side.
            Out::Map(vec![
                ("Ra".into(), k.ra),
                ("Rb".into(), k.rb),
                ("Sb".into(), k.sb),
                ("Va".into(), k.va),
                ("Vb".into(), k.vb),
            ])
        }
        "is_sweep" => Out::B(bias::is_sweep(f(&a[0]), f(&a[1]) as usize)),
        "calc_declination" => opt(bias::calc_declination(&farr(&a[0]))),
        "calc_mean_median_difference" => Out::F(bias::calc_mean_median_difference(
            &farr(&a[0]),
            a.get(1).map(f),
        )),
        "calc_turnout_bias" => Out::F(bias::calc_turnout_bias(f(&a[0]), &farr(&a[1]))),
        "calc_lopsided_outcomes" => opt(bias::calc_lopsided_outcomes(&farr(&a[0]))),

        // ---- responsiveness ----
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
            f(&a[0]),
            &pairs(&a[1]),
            f(&a[2]) as i32,
        )),
        "calc_big_R" => opt(responsiveness::calc_big_r(f(&a[0]), f(&a[1]))),
        "calc_minimal_inverse_responsiveness" => opt(
            responsiveness::calc_minimal_inverse_responsiveness(f(&a[0]), f(&a[1])),
        ),

        // ---- more ----
        "calc_efficiency_gap_wasted_votes" => opt(
            more::calc_efficiency_gap_wasted_votes(&iarr(&a[0]), &iarr(&a[1])),
        ),
        "calc_average_margin" => Out::F(more::calc_average_margin(&farr(&a[0]))),

        // ---- equal ----
        "calc_population_deviation" => Out::F(calc_population_deviation(
            f(&a[0]) as i64,
            f(&a[1]) as i64,
            f(&a[2]) as i64,
        )),

        other => panic!("no Rust binding for traced function {other}"),
    }
}

fn expected(v: &Value) -> Out {
    match v {
        Value::Null => Out::Null,
        Value::Bool(b) => Out::B(*b),
        Value::Number(n) if n.is_i64() || n.is_u64() => Out::I(n.as_i64().unwrap()),
        Value::Number(n) => Out::F(n.as_f64().unwrap()),
        Value::Array(_) => Out::Pairs(pairs(v)),
        Value::Object(m) => {
            let mut kv: Vec<(String, f64)> =
                m.iter().map(|(k, x)| (k.clone(), f(x))).collect();
            kv.sort_by(|a, b| a.0.cmp(&b.0));
            Out::Map(kv)
        }
        other => panic!("unsupported expected value: {other}"),
    }
}

fn close(g: f64, w: f64) -> (bool, f64) {
    let err = (g - w).abs() / w.abs().max(1.0);
    (err <= FLOAT_TOL, err)
}

#[test]
fn partisan_matches_rdapy() {
    let dir = format!(
        "{}/../../conformance/cases/traced",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            panic!("cannot read {dir}: {e}\nrun conformance/tools/trace_rdapy_tests.py")
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no traced case files in {dir}");

    let mut total = 0usize;
    let mut direct = 0usize;
    let mut worst = 0.0f64;
    let mut worst_where = String::new();

    for path in files {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let function = doc["function"].as_str().expect("function name");

        for (idx, c) in doc["cases"].as_array().unwrap().iter().enumerate() {
            let args = c["input"].as_array().expect("input array");
            let want = expected(&c["expect"]);
            let got = call(function, args);

            let from = c["from"].as_str();
            let label = format!(
                "{function}[{idx}]{}",
                from.map(|s| format!(" (from {s})")).unwrap_or_default()
            );

            let mut track = |err: f64, label: &str| {
                if err > worst {
                    worst = err;
                    worst_where = label.to_string();
                }
            };

            match (&got, &want) {
                (Out::F(g), Out::F(w)) => {
                    let (ok, err) = close(*g, *w);
                    track(err, &label);
                    assert!(ok, "{label}: got {g:?}, want {w:?} (rel err {err:e})");
                }
                // Python's min/max can return an int literal, so a float
                // result may have been recorded as an integer.
                (Out::F(g), Out::I(w)) => {
                    let (ok, err) = close(*g, *w as f64);
                    track(err, &label);
                    assert!(ok, "{label}: got {g:?}, want {w} (rel err {err:e})");
                }
                (Out::Pairs(g), Out::Pairs(w)) => {
                    assert_eq!(g.len(), w.len(), "{label}: point count");
                    for (k, (gp, wp)) in g.iter().zip(w.iter()).enumerate() {
                        for (gv, wv, which) in
                            [(gp.0, wp.0, "x"), (gp.1, wp.1, "y")]
                        {
                            let (ok, err) = close(gv, wv);
                            track(err, &label);
                            assert!(
                                ok,
                                "{label}: point {k}.{which} got {gv:?}, want {wv:?} \
                                 (rel err {err:e})"
                            );
                        }
                    }
                }
                (Out::Map(g), Out::Map(w)) => {
                    assert_eq!(g.len(), w.len(), "{label}: key count");
                    for ((gk, gv), (wk, wv)) in g.iter().zip(w.iter()) {
                        assert_eq!(gk, wk, "{label}: key");
                        let (ok, err) = close(*gv, *wv);
                        track(err, &label);
                        assert!(ok, "{label}: {gk} got {gv:?}, want {wv:?} (rel err {err:e})");
                    }
                }
                (g, w) => assert_eq!(g, w, "{label}"),
            }

            total += 1;
            if from.is_some() {
                direct += 1;
            }
        }
    }

    eprintln!(
        "partisan: {total} traced cases pass ({direct} asserted directly by rdapy's \
         tests); worst rel err {worst:e} at {worst_where}"
    );
}
