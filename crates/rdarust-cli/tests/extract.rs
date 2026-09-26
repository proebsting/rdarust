//! Extraction from a DRA GeoJSON must reproduce rdapy's output.
//!
//! These run the real North Carolina GeoJSON -- 2,666 precincts -- through
//! `extract-graph` and `extract-data`, and then all the way to a scores CSV,
//! which is diffed against the golden file recorded from rdapy's pipeline.
//! That last check is the one that matters: it says the whole chain agrees,
//! with no Python anywhere in it.

use std::collections::{HashMap, HashSet};
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
    let dir = std::env::temp_dir().join(format!("rdarust-extract-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("creating a temporary directory");
    dir
}

const GEOJSON: &str = "testdata/examples/NC_vtd_datasets.geojson";
const DATA_MAP: &str = "testdata/examples/NC_data_map.json";

fn load_json(path: &PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("reading JSON")).unwrap()
}

#[test]
fn extracted_graph_matches_rdapy() {
    let dir = tempdir("graph");
    let graph = dir.join("graph.json");
    run(&[
        "extract-graph",
        "--geojson", &rdapy(GEOJSON),
        "--graph", graph.to_str().unwrap(),
    ]);

    let got = load_json(&graph);
    let want = load_json(&PathBuf::from(rdapy("testdata/examples/NC_graph.json")));
    let (got, want) = (got.as_object().unwrap(), want.as_object().unwrap());

    assert_eq!(
        got.keys().collect::<HashSet<_>>(),
        want.keys().collect::<HashSet<_>>(),
        "node sets differ"
    );
    // Neighbour *order* is an artefact of how each implementation iterates;
    // what has to agree is who borders whom.
    for (node, want_nbrs) in want {
        let as_set = |v: &Value| -> HashSet<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            as_set(&got[node]),
            as_set(want_nbrs),
            "neighbours of {node} differ"
        );
    }
}

#[test]
fn extracted_data_matches_rdapy() {
    let dir = tempdir("data");
    let data = dir.join("data.jsonl");
    run(&[
        "extract-data",
        "--geojson", &rdapy(GEOJSON),
        "--data-map", &rdapy(DATA_MAP),
        "--graph", &rdapy("testdata/examples/NC_graph.json"),
        "--data", data.to_str().unwrap(),
    ]);

    let read = |p: &PathBuf| -> HashMap<String, Value> {
        std::fs::read_to_string(p)
            .expect("reading JSONL")
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|r| r.get("_tag_").and_then(|t| t.as_str()) == Some("precinct"))
            .map(|r| {
                let d = r.get("data").unwrap().clone();
                (d["geoid"].as_str().unwrap().to_string(), d)
            })
            .collect()
    };

    let got = read(&data);
    let want = read(&PathBuf::from(rdapy("testdata/examples/NC_input_data.jsonl")));
    assert_eq!(got.len(), want.len(), "precinct counts differ");

    // Geometry is reproduced to a few ulps rather than exactly; everything
    // else -- the data columns, the centre, the geoid -- must be identical.
    const GEOMETRIC_TOL: f64 = 1e-9;
    let mut worst_area = 0.0f64;
    let mut worst_arc = 0.0f64;

    for (geoid, w) in &want {
        let g = got.get(geoid).unwrap_or_else(|| panic!("{geoid} missing"));
        let (wo, go) = (w.as_object().unwrap(), g.as_object().unwrap());
        assert_eq!(
            wo.keys().collect::<Vec<_>>(),
            go.keys().collect::<Vec<_>>(),
            "{geoid}: field order differs"
        );

        for (k, wv) in wo {
            match k.as_str() {
                "area" => {
                    let (a, b) = (go[k].as_f64().unwrap(), wv.as_f64().unwrap());
                    worst_area = worst_area.max((a - b).abs() / b.abs().max(1e-30));
                }
                "arcs" => {
                    let (a, b) = (go[k].as_object().unwrap(), wv.as_object().unwrap());
                    assert_eq!(
                        a.keys().collect::<HashSet<_>>(),
                        b.keys().collect::<HashSet<_>>(),
                        "{geoid}: neighbours differ"
                    );
                    for (n, bv) in b {
                        let (x, y) = (a[n].as_f64().unwrap(), bv.as_f64().unwrap());
                        worst_arc = worst_arc.max((x - y).abs() / y.abs().max(1e-30));
                    }
                }
                "exterior" => {
                    // The hull's starting vertex is arbitrary; its point set
                    // is not, and only the set reaches the enclosing circle.
                    let pts = |v: &Value| -> HashSet<[u64; 2]> {
                        v.as_array()
                            .unwrap()
                            .iter()
                            .map(|p| {
                                [
                                    p[0].as_f64().unwrap().to_bits(),
                                    p[1].as_f64().unwrap().to_bits(),
                                ]
                            })
                            .collect()
                    };
                    assert_eq!(pts(&go[k]), pts(wv), "{geoid}: convex hull differs");
                }
                _ => assert_eq!(&go[k], wv, "{geoid}: field {k} differs"),
            }
        }
    }

    assert!(worst_area < GEOMETRIC_TOL, "worst area error {worst_area:e}");
    assert!(worst_arc < GEOMETRIC_TOL, "worst arc error {worst_arc:e}");
    eprintln!("extract-data: worst area {worst_area:e}, worst shared border {worst_arc:e}");
}

/// GeoJSON to scores with nothing Python in the chain, byte-identical to what
/// rdapy's pipeline produces from the same input.
#[test]
fn the_whole_chain_from_geojson_matches_rdapy() {
    let dir = tempdir("chain");
    let graph = dir.join("graph.json");
    let data = dir.join("data.jsonl");
    let scores = dir.join("scores.csv");

    // Derive the data map too, so nothing in the chain comes from rdapy.
    let data_map = dir.join("data-map.json");
    run(&[
        "map-data",
        "--geojson", &rdapy(GEOJSON),
        "--data-map", data_map.to_str().unwrap(),
        "--elections", "E_16-20_COMP,E_20_PRES,E_20_GOV,E_20_SEN,E_16_PRES,E_20_AG,E_16_SEN",
    ]);
    run(&[
        "extract-graph",
        "--geojson", &rdapy(GEOJSON),
        "--graph", graph.to_str().unwrap(),
    ]);
    run(&[
        "extract-data",
        "--geojson", &rdapy(GEOJSON),
        "--data-map", data_map.to_str().unwrap(),
        "--graph", graph.to_str().unwrap(),
        "--data", data.to_str().unwrap(),
    ]);
    run(&[
        "score-all",
        "--state", "NC", "--plan-type", "congress",
        "--data", data.to_str().unwrap(),
        "--graph", graph.to_str().unwrap(),
        "--precomputed", &rdapy("testdata/examples/NC_congress_precomputed.json"),
        "--plans", &rdapy("testdata/plans/NC_congress_plans.tagged.jsonl"),
        "--scores", scores.to_str().unwrap(),
        "--by-district", dir.join("bd.jsonl").to_str().unwrap(),
    ]);

    let got = std::fs::read(&scores).expect("reading scores");
    let want = std::fs::read(repo().join("conformance/cases/cli/NC_scores.csv"))
        .expect("reading the golden CSV");
    assert_eq!(
        got, want,
        "scores from the Rust-only chain differ from rdapy's pipeline"
    );
}
