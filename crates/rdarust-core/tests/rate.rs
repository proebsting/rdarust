//! Conformance tests for the DRA ratings.
//!
//! Reads `conformance/cases/rate/`, which holds rdapy's own hand-written test
//! values (transcribed and verified against live rdapy by
//! `conformance/tools/gen_rate.py`) alongside broader recorded sweeps.

use rdarust_core::rate::{self, Normalizer};
use serde_json::Value;

/// What a case's `expect` field can hold.
#[derive(Debug, PartialEq)]
enum Out {
    Float(f64),
    Int(i64),
    Bool(bool),
}

/// Relative tolerance for float-valued results. Integer results and booleans
/// must match exactly -- ratings are what users see, so an off-by-one is a
/// real defect, not rounding noise.
const FLOAT_TOL: f64 = 1e-9;

fn case_files() -> Vec<std::path::PathBuf> {
    let dir = format!("{}/../../conformance/cases/rate", env!("CARGO_MANIFEST_DIR"));
    let mut out: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {dir}: {e}\nrun conformance/tools/gen_rate.py"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no case files in {dir}");
    out
}

/// Evaluate the named rdapy function on the case's arguments.
///
/// `Normalizer` steps are exercised one at a time, matching how rdapy's tests
/// drive them.
fn call(function: &str, a: &[f64]) -> Out {
    let n = |i: usize| a[i];
    let i = |k: usize| a[k] as i32;

    match function {
        "normalizer_invert" => {
            let mut z = Normalizer::new(n(0));
            Out::Float(z.invert().expect("invert"))
        }
        "normalizer_clip" => {
            let mut z = Normalizer::new(n(0));
            Out::Float(z.clip(n(1), n(2)))
        }
        "normalizer_rebase" => {
            let mut z = Normalizer::new(n(0));
            Out::Float(z.rebase(n(1)))
        }
        "normalizer_unitize" => {
            let mut z = Normalizer::new(n(0));
            Out::Float(z.unitize(n(1), n(2)).expect("unitize"))
        }
        "normalizer_decay" => {
            let mut z = Normalizer::new(n(0));
            Out::Float(z.decay().expect("decay"))
        }
        "normalizer_rescale" => {
            let mut z = Normalizer::new(n(0));
            Out::Int(z.rescale().expect("rescale") as i64)
        }

        "is_antimajoritarian" => Out::Bool(rate::is_antimajoritarian(n(0), n(1))),
        "extra_bonus" => Out::Float(rate::extra_bonus(n(0))),
        "adjust_deviation" => Out::Float(rate::adjust_deviation(n(0), n(1), n(2))),
        "best_target" => Out::Float(rate::best_target(n(0), n(1))),

        "rate_proportionality" => {
            Out::Int(rate::proportionality(n(0), n(1), n(2)).expect("proportionality") as i64)
        }
        "rate_competitiveness" => {
            Out::Int(rate::competitiveness(n(0)).expect("competitiveness") as i64)
        }
        "rate_minority_opportunity" => {
            Out::Int(rate::minority_opportunity(n(0), n(1), n(2), n(3)) as i64)
        }
        "rate_reock" => Out::Int(rate::reock(n(0)).expect("reock") as i64),
        "rate_polsby" => Out::Int(rate::polsby(n(0)).expect("polsby") as i64),
        "rate_compactness" => Out::Int(rate::compactness(i(0), i(1)) as i64),
        "rate_county_splitting" => {
            Out::Int(rate::county_splitting(n(0), i(1), i(2)).expect("county_splitting") as i64)
        }
        "rate_district_splitting" => Out::Int(
            rate::district_splitting(n(0), i(1), i(2)).expect("district_splitting") as i64,
        ),
        "rate_splitting" => Out::Int(rate::splitting(i(0), i(1)) as i64),

        other => panic!("case file names an unknown function: {other}"),
    }
}

fn expected(v: &Value) -> Out {
    match v {
        Value::Bool(b) => Out::Bool(*b),
        Value::Number(num) if num.is_i64() || num.is_u64() => Out::Int(num.as_i64().unwrap()),
        Value::Number(num) => Out::Float(num.as_f64().unwrap()),
        other => panic!("unsupported expected value: {other}"),
    }
}

#[test]
fn rate_matches_rdapy() {
    let mut total = 0usize;
    let mut published = 0usize;
    let mut worst = 0.0f64;
    let mut worst_where = String::new();

    for path in case_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        let doc: Value = serde_json::from_str(&text).unwrap();
        let function = doc["function"].as_str().expect("function name");

        for (idx, c) in doc["cases"].as_array().unwrap().iter().enumerate() {
            let args: Vec<f64> = c["input"]
                .as_array()
                .expect("input array")
                .iter()
                .map(|v| v.as_f64().expect("numeric argument"))
                .collect();
            let want = expected(&c["expect"]);
            let got = call(function, &args);

            let origin = c["origin"].as_str().unwrap_or("?");
            let name = c["name"].as_str().unwrap_or("");
            let label = format!("{function}[{idx}]{} ({origin}) args={args:?}",
                if name.is_empty() { String::new() } else { format!(" \"{name}\"") });

            // Dispatch on what *we* return, not on the JSON type. Python's
            // min/max yield the int literal when it wins -- `max(x - extra, 0)`
            // returns int 0, not 0.0 -- so a float-valued rdapy function can
            // land in the case file as an integer.
            match &got {
                Out::Float(g) => {
                    let w = match &want {
                        Out::Float(w) => *w,
                        Out::Int(w) => *w as f64,
                        Out::Bool(_) => panic!("{label}: expected a bool, got a float"),
                    };
                    let err = (g - w).abs() / w.abs().max(1.0);
                    if err > worst {
                        worst = err;
                        worst_where = label.clone();
                    }
                    assert!(err <= FLOAT_TOL, "{label}: got {g:?}, want {w:?} (rel err {err:e})");
                }
                // A rating is an integer a user sees; off-by-one is a defect,
                // so these must agree exactly.
                Out::Int(g) => {
                    let w = match &want {
                        Out::Int(w) => *w,
                        Out::Float(w) => {
                            assert_eq!(w.fract(), 0.0, "{label}: expected {w} is not an integer");
                            *w as i64
                        }
                        Out::Bool(_) => panic!("{label}: expected a bool, got an int"),
                    };
                    assert_eq!(*g, w, "{label}");
                }
                Out::Bool(g) => assert_eq!(Out::Bool(*g), want, "{label}"),
            }

            total += 1;
            if origin == "rdapy-test" {
                published += 1;
            }
        }
    }

    eprintln!(
        "rate: {total} cases pass ({published} transcribed from rdapy's own tests); \
         worst float rel err {worst:e} at {worst_where}"
    );
}

/// rdapy raises when a Normalizer invariant is violated; we return an error
/// rather than panicking or silently producing a bogus rating.
#[test]
fn normalizer_reports_invariant_violations() {
    let mut z = Normalizer::new(1.5);
    assert!(z.invert().is_err(), "invert must reject a value above 1");

    let mut z = Normalizer::new(f64::NAN);
    assert!(z.rescale().is_err(), "rescale must reject NaN");

    let mut z = Normalizer::new(5.0);
    assert!(z.unitize(0.0, 1.0).is_err(), "unitize must reject out-of-range");
}

/// The 100 rating is reserved for plans with no splitting at all.
#[test]
fn perfect_splitting_rating_is_reserved() {
    assert_eq!(rate::county_splitting(1.0, 99, 4).unwrap(), 100);
    // Any splitting at all caps the rating at 99, even if it would round to 100.
    let nearly = rate::county_splitting(1.0001, 99, 4).unwrap();
    assert_eq!(nearly, 99, "a split plan must not be able to score 100");
    assert_eq!(rate::splitting(100, 100), 100);
    assert_eq!(rate::splitting(99, 100), 99);
}
