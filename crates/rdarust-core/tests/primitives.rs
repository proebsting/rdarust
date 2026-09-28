//! Conformance tests for the low-level primitives.
//!
//! These read the language-neutral case files in `conformance/cases/primitives/`,
//! which were recorded from CPython / NumPy / SciPy by
//! `conformance/tools/gen_primitives.py`. The same files are (or will be) read
//! by the Python runner, so both implementations are checked against one
//! corpus.

use rdarust_core::geometry::{min_enclosing_circle, Circle};
use rdarust_core::numeric::{arange, erf, isclose, python_round, python_round_to};
use rdarust_core::spline::CubicSpline;
use serde_json::Value;

fn load(name: &str) -> Value {
    let path = format!(
        "{}/../../conformance/cases/primitives/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {path}: {e}\nrun conformance/tools/gen_primitives.py"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {path}: {e}"))
}

fn cases(doc: &Value) -> &Vec<Value> {
    doc["cases"].as_array().expect("cases array")
}

fn f(v: &Value) -> f64 {
    v.as_f64().expect("number")
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array().expect("array").iter().map(f).collect()
}

/// Exact bit equality, so a 1-ulp drift cannot hide.
fn assert_bits(actual: f64, expect: f64, ctx: &str) {
    assert!(
        actual.to_bits() == expect.to_bits(),
        "{ctx}: got {actual:?} ({:#x}), want {expect:?} ({:#x})",
        actual.to_bits(),
        expect.to_bits()
    );
}

#[test]
fn python_round_matches_cpython() {
    let doc = load("python_round");
    for (i, c) in cases(&doc).iter().enumerate() {
        let x = f(&c["input"][0]);
        let want = f(&c["expect"]);
        assert_bits(python_round(x), want, &format!("python_round case {i}: round({x:?})"));
    }
}

#[test]
fn python_round_differs_from_rust_round() {
    // Guard against someone "simplifying" python_round to f64::round later.
    // These are the ties where the two disagree.
    for x in [0.5f64, 2.5, 12.5, -0.5, -2.5, 100.5] {
        assert_ne!(
            python_round(x),
            x.round(),
            "round-half-to-even and round-half-away must differ at {x}"
        );
    }
}

#[test]
fn python_round_to_matches_cpython() {
    let doc = load("python_round_to");
    for (i, c) in cases(&doc).iter().enumerate() {
        let x = f(&c["input"][0]);
        let nd = c["input"][1].as_u64().unwrap() as usize;
        let want = f(&c["expect"]);
        assert_bits(
            python_round_to(x, nd),
            want,
            &format!("python_round_to case {i}: round({x:?}, {nd})"),
        );
    }
}

#[test]
fn arange_matches_numpy() {
    let doc = load("numpy_arange");
    for (i, c) in cases(&doc).iter().enumerate() {
        let a = floats(&c["input"]);
        let want = floats(&c["expect"]);
        let got = arange(a[0], a[1], a[2]);
        assert_eq!(
            got.len(),
            want.len(),
            "arange case {i} ({:?}): length", a
        );
        for (k, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert_bits(*g, *w, &format!("arange case {i} element {k}"));
        }
    }
}

#[test]
fn shift_range_has_101_points_and_no_exact_half() {
    let sr = rdarust_core::numeric::shift_range();
    assert_eq!(sr.len(), 101);
    assert!(
        !sr.iter().any(|&v| v == 0.5),
        "0.5 must not be exactly present -- est_seats_bias relies on isclose"
    );
    assert_eq!(sr.iter().filter(|&&v| (v - 0.5).abs() < 1e-12).count(), 1);
}

#[test]
fn erf_matches_cpython() {
    let doc = load("erf");
    let mut worst = 0.0f64;
    for (i, c) in cases(&doc).iter().enumerate() {
        let x = f(&c["input"][0]);
        let want = f(&c["expect"]);
        let got = erf(x);
        let err = (got - want).abs();
        worst = worst.max(err);
        assert!(
            err <= 1e-15 * want.abs().max(1e-300) || err <= 1e-300,
            "erf case {i}: erf({x:?}) got {got:?} want {want:?} (abs err {err:e})"
        );
    }
    eprintln!("erf: worst relative error {worst:e}");
}

#[test]
fn isclose_matches_cpython() {
    let doc = load("math_isclose");
    for (i, c) in cases(&doc).iter().enumerate() {
        let a = floats(&c["input"]);
        let want = c["expect"].as_bool().expect("bool");
        let got = isclose(a[0], a[1], a[2], a[3]);
        assert_eq!(
            got, want,
            "isclose case {i}: isclose({}, {}, rel={}, abs={})",
            a[0], a[1], a[2], a[3]
        );
    }
}

#[test]
fn spline_matches_scipy_interp1d_cubic() {
    let doc = load("not_a_knot_spline");
    for c in cases(&doc) {
        let name = c["name"].as_str().unwrap_or("?");
        let x = floats(&c["x"]);
        let y = floats(&c["y"]);
        let q = floats(&c["query"]);
        let want = floats(&c["expect"]);

        let sp = CubicSpline::new(&x, &y)
            .unwrap_or_else(|e| panic!("spline {name}: construction failed: {e}"));
        let got = sp
            .eval_all(&q)
            .unwrap_or_else(|e| panic!("spline {name}: evaluation failed: {e}"));

        for (k, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            let tol = 1e-9 * w.abs().max(1.0);
            assert!(
                (g - w).abs() <= tol,
                "spline {name} query {k} (x={}): got {g:?} want {w:?} (err {:e})",
                q[k],
                (g - w).abs()
            );
        }
    }
}

#[test]
fn spline_rejects_too_few_points() {
    assert!(CubicSpline::new(&[0.0, 1.0, 2.0], &[0.0, 1.0, 2.0]).is_err());
}

#[test]
fn spline_is_out_of_bounds_error_not_extrapolation() {
    let x = [0.0, 1.0, 2.0, 3.0, 4.0];
    let y = [0.0, 1.0, 4.0, 9.0, 16.0];
    let sp = CubicSpline::new(&x, &y).unwrap();
    assert!(sp.eval(-0.001).is_err());
    assert!(sp.eval(4.001).is_err());
    assert!(sp.eval(0.0).is_ok());
    assert!(sp.eval(4.0).is_ok());
}

#[test]
fn min_enclosing_circle_matches_rdapy() {
    let doc = load("welzl_min_enclosing_circle");
    for c in cases(&doc) {
        let name = c["name"].as_str().unwrap_or("?");
        let pts: Vec<(f64, f64)> = c["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (f(&p[0]), f(&p[1])))
            .collect();
        let want = Circle {
            x: f(&c["expect"]["x"]),
            y: f(&c["expect"]["y"]),
            r: f(&c["expect"]["r"]),
        };
        let got = min_enclosing_circle(&pts);

        let scale = want.r.abs().max(1.0);
        let tol = 1e-9 * scale;
        assert!(
            (got.r - want.r).abs() <= tol,
            "MEC {name}: radius got {} want {} (err {:e})",
            got.r,
            want.r,
            (got.r - want.r).abs()
        );
        assert!(
            (got.x - want.x).abs() <= tol && (got.y - want.y).abs() <= tol,
            "MEC {name}: centre got ({}, {}) want ({}, {})",
            got.x,
            got.y,
            want.x,
            want.y
        );
    }
}

#[test]
fn min_enclosing_circle_actually_encloses() {
    // A property check independent of the recorded values: whatever circle we
    // return must contain every input point.
    let doc = load("welzl_min_enclosing_circle");
    for c in cases(&doc) {
        let name = c["name"].as_str().unwrap_or("?");
        let pts: Vec<(f64, f64)> = c["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (f(&p[0]), f(&p[1])))
            .collect();
        let got = min_enclosing_circle(&pts);
        for (i, p) in pts.iter().enumerate() {
            let d = (got.x - p.0).hypot(got.y - p.1);
            assert!(
                d <= got.r + 1e-9 * got.r.abs().max(1.0),
                "MEC {name}: point {i} {p:?} lies outside the circle (d={d}, r={})",
                got.r
            );
        }
    }
}

#[test]
fn min_enclosing_circle_is_deterministic() {
    let doc = load("welzl_min_enclosing_circle");
    for c in cases(&doc) {
        let pts: Vec<(f64, f64)> = c["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (f(&p[0]), f(&p[1])))
            .collect();
        let a = min_enclosing_circle(&pts);
        let b = min_enclosing_circle(&pts);
        assert_eq!(a, b, "MEC must be bit-stable across calls");
    }
}

#[test]
fn min_enclosing_circle_empty_input() {
    assert_eq!(min_enclosing_circle(&[]), Circle::EMPTY);
}

/// Not an assertion -- reports how much headroom we actually have against the
/// 1e-9 acceptance bar, so a regression shows up as shrinking margin.
#[test]
fn report_agreement_margins() {
    let doc = load("not_a_knot_spline");
    let mut worst_spline = 0.0f64;
    for c in cases(&doc) {
        let (x, y, q, want) = (
            floats(&c["x"]),
            floats(&c["y"]),
            floats(&c["query"]),
            floats(&c["expect"]),
        );
        let sp = CubicSpline::new(&x, &y).unwrap();
        for (g, w) in sp.eval_all(&q).unwrap().iter().zip(want.iter()) {
            worst_spline = worst_spline.max((g - w).abs() / w.abs().max(1.0));
        }
    }

    let doc = load("welzl_min_enclosing_circle");
    let mut worst_mec = 0.0f64;
    let mut exact = 0usize;
    let mut total = 0usize;
    for c in cases(&doc) {
        let pts: Vec<(f64, f64)> = c["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (f(&p[0]), f(&p[1])))
            .collect();
        let want = Circle {
            x: f(&c["expect"]["x"]),
            y: f(&c["expect"]["y"]),
            r: f(&c["expect"]["r"]),
        };
        let got = min_enclosing_circle(&pts);
        let scale = want.r.abs().max(1.0);
        let e = ((got.r - want.r).abs())
            .max((got.x - want.x).abs())
            .max((got.y - want.y).abs())
            / scale;
        worst_mec = worst_mec.max(e);
        total += 1;
        if got.r.to_bits() == want.r.to_bits() && got.x.to_bits() == want.x.to_bits() {
            exact += 1;
        }
    }

    eprintln!("spline worst relative error vs scipy : {worst_spline:e}  (bar 1e-9)");
    eprintln!("MEC    worst relative error vs rdapy : {worst_mec:e}  (bar 1e-9)");
    eprintln!("MEC    bit-identical to rdapy        : {exact}/{total} cases");
}


/// A state's data scored as another has more counties than the matrix is
/// sized for. rdapy raises an IndexError; this widens the matrix and says so,
/// because the same mismatch can mean a stale county table rather than a
/// wrong state, and refusing would block legitimate data.
#[test]
fn more_counties_than_the_state_has_widens_the_matrix() {
    use rdarust_core::context::{Context, DatasetKeys, Demographics, PrecinctInput};

    // Delaware has three counties; four distinct FIPS codes in the data.
    let precincts: Vec<PrecinctInput> = (0..4)
        .map(|i| PrecinctInput {
            geoid: format!("1000{i}000001"),
            pop: 100,
            center: (0.0, 0.0),
            area: 1.0,
            arcs: Vec::new(),
            exterior: Vec::new(),
            neighbors: Vec::new(),
        })
        .collect();

    let ctx = Context::new(
        "DE",
        "congress",
        precincts,
        None,
        Vec::new(),
        Demographics { names: Vec::new(), counts: Vec::new() },
        None,
        DatasetKeys {
            census: "c".into(),
            vap: "v".into(),
            cvap: None,
            elections: Vec::new(),
            shapes: "s".into(),
        },
        Some(2),
    )
    .expect("a county count mismatch is a warning, not a refusal");

    assert_eq!(ctx.n_counties, 4, "the matrix should be sized for what the data holds");
    assert_eq!(ctx.warnings.len(), 1, "the mismatch should be reported");
    let w = &ctx.warnings[0];
    assert!(w.contains("4 counties but DE has 3"), "should name both counts: {w}");
    // Every geoid here opens with FIPS 10, which is the evidence a reader
    // needs to tell a wrong --state from a stale table.
    assert!(w.contains("state FIPS 10"), "should name the data's state: {w}");
}
