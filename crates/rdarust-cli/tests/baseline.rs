//! The geographic baseline.
//!
//! Neighbourhoods are compared against the ones checked into rdapy's test
//! data, which `find_neighborhoods.py` produced from the same inputs -- the
//! output is byte-identical, packed bitmaps and all. The baseline computed
//! from them is compared against a recording of rdapy's own run on the same
//! data.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rdapy(rel: &str) -> String {
    repo().join("vendor/rdapy").join(rel).to_string_lossy().into_owned()
}

fn run(args: &[&str]) {
    let out = Command::new(env!("CARGO_BIN_EXE_rdarust"))
        .args(args)
        .output()
        .expect("running rdarust");
    assert!(
        out.status.success(),
        "rdarust {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rdarust-baseline-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("creating a temporary directory");
    dir
}

const DATA: &str = "testdata/examples/NC_input_data.jsonl";
const GRAPH: &str = "testdata/examples/NC_graph.json";
const NEIGHBORHOODS: &str = "testdata/examples/NC_congress_neighborhoods.jsonl";

#[test]
fn neighborhoods_match_rdapy_byte_for_byte() {
    let dir = tempdir("find");
    let out = dir.join("neighborhoods.jsonl");

    run(&[
        "find-neighborhoods",
        "--state", "NC", "--plan-type", "congress",
        "--data", &rdapy(DATA), "--graph", &rdapy(GRAPH),
        "--output", out.to_str().unwrap(),
    ]);

    let got = std::fs::read(&out).expect("reading the neighbourhoods");
    let want = std::fs::read(rdapy(NEIGHBORHOODS)).expect("reading rdapy's neighbourhoods");

    if got != want {
        let (g, w) = (String::from_utf8_lossy(&got), String::from_utf8_lossy(&want));
        for (i, (gl, wl)) in g.lines().zip(w.lines()).enumerate() {
            if gl != wl {
                panic!(
                    "line {} differs\n  got:  {}\n  want: {}",
                    i + 1,
                    &gl[..120.min(gl.len())],
                    &wl[..120.min(wl.len())]
                );
            }
        }
        panic!("line counts differ: {} against {}", g.lines().count(), w.lines().count());
    }
}

#[test]
fn baselines_match_rdapy() {
    let dir = tempdir("precompute");
    let out = dir.join("precomputed.json");

    run(&[
        "precompute-baselines",
        "--state", "NC", "--plan-type", "congress",
        "--data", &rdapy(DATA),
        "--neighborhoods", &rdapy(NEIGHBORHOODS),
        "--output", out.to_str().unwrap(),
    ]);

    let got: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let golden = repo().join("conformance/cases/baseline/NC_precomputed.json");
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&golden).unwrap_or_else(
        |e| panic!("cannot read {}: {e}\nrun conformance/tools/gen_baseline.sh", golden.display()),
    ))
    .unwrap();

    let (g, w) = (
        got["geographic_baseline"].as_object().unwrap(),
        want["geographic_baseline"].as_object().unwrap(),
    );
    assert_eq!(
        g.keys().collect::<Vec<_>>(),
        w.keys().collect::<Vec<_>>(),
        "elections differ"
    );

    let mut worst = 0.0f64;
    for (election, wv) in w {
        for field in ["fractional_seats", "whole_seats"] {
            let a = g[election][field].as_f64().unwrap();
            let b = wv[field].as_f64().unwrap();
            let err = (a - b).abs() / b.abs().max(1.0);
            worst = worst.max(err);
            assert!(err <= 1e-9, "{election}.{field}: {a} against {b} (rel err {err:e})");
        }
    }
    eprintln!("geographic baseline: worst relative error {worst:e}");
}

/// A neighbourhood must be contiguous and about the right size; both are
/// properties of the search rather than of any recorded output.
#[test]
fn neighborhoods_are_contiguous_and_near_the_target() {
    use rdarust_core::graph::is_connected;
    use rdarust_io::{load_graph, load_input_data};

    let ctx = load_input_data(rdapy(DATA))
        .expect("reading precinct data")
        .with_graph(load_graph(rdapy(GRAPH)).expect("reading the graph"))
        .into_context("NC", "congress", None)
        .expect("building a context");

    let target = ctx.neighborhood_target_pop(1.0);
    assert!(target > 0);

    // A sample, since growing all 2,666 in a debug build is slow.
    for seed in (0..ctx.n_precincts() as u32).step_by(211) {
        let members = ctx.make_neighborhood(seed, target);

        assert!(members.contains(&seed), "a precinct must be in its own neighbourhood");
        assert!(
            is_connected(&members, &ctx.adjacency, ctx.out_of_state),
            "neighbourhood of {} is not contiguous",
            ctx.geoids[seed as usize]
        );

        // Taking one fewer must undershoot and one more must overshoot by at
        // least as much -- that is what "closest to the target" means.
        let pop: i64 = members.iter().map(|&i| ctx.pop[i as usize]).sum();
        let last = ctx.pop[*members.last().unwrap() as usize];
        if members.len() > 1 && pop > target {
            assert!(
                pop - target <= target - (pop - last),
                "{}: overshooting by {} beats undershooting by {}",
                ctx.geoids[seed as usize],
                pop - target,
                target - (pop - last)
            );
        }
    }
}
