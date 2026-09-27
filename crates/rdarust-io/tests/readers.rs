//! Every reader works from bytes, not just from a path.
//!
//! The path-taking readers are thin wrappers over `_from` variants that take
//! an open reader, so a caller holding bytes -- a fetched file, a
//! decompressed stream, a browser upload -- never has to go through the
//! filesystem. These tests pin that: each `_from` variant is exercised
//! against an in-memory buffer, and the wrappers are checked to agree with
//! them on real data.

use std::io::Cursor;
use std::path::PathBuf;

use rdarust_io::{
    features_of, load_geojson_from, load_graph, load_graph_from, load_input_data,
    load_input_data_from, read_plan_csv, read_plan_csv_from, read_plan_jsonl_from,
    read_plans_jsonl_from,
};
use serde_json::json;

fn rdapy_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/rdapy")
        .join(rel)
}

const NC_DATA: &str = "testdata/score/NC_input_data.v1.jsonl";
const NC_PLAN: &str = "testdata/score/NC20C_baseline_100.csv";
const SAMPLE_GRAPH: &str = "testdata/graph/SAMPLE-BG-graph.json";

#[test]
fn input_data_from_bytes_matches_the_path_reader() {
    let bytes = std::fs::read(rdapy_path(NC_DATA)).expect("reading NC input data");

    let from_path = load_input_data(rdapy_path(NC_DATA))
        .expect("path reader")
        .into_context("NC", "congress", None)
        .expect("context from path");
    let from_bytes = load_input_data_from(Cursor::new(&bytes))
        .expect("reader")
        .into_context("NC", "congress", None)
        .expect("context from bytes");

    assert_eq!(from_bytes.n_precincts(), from_path.n_precincts());
    assert_eq!(from_bytes.geoids, from_path.geoids);
    assert_eq!(from_bytes.adjacency, from_path.adjacency);
    assert_eq!(from_bytes.arc_len, from_path.arc_len);
}

#[test]
fn graph_from_bytes_matches_the_path_reader() {
    let bytes = std::fs::read(rdapy_path(SAMPLE_GRAPH)).expect("reading the sample graph");
    assert_eq!(
        load_graph_from(Cursor::new(&bytes)).expect("reader"),
        load_graph(rdapy_path(SAMPLE_GRAPH)).expect("path reader"),
    );
}

#[test]
fn plan_csv_from_bytes_matches_the_path_reader() {
    let bytes = std::fs::read(rdapy_path(NC_PLAN)).expect("reading the plan");
    assert_eq!(
        read_plan_csv_from(Cursor::new(&bytes)).expect("reader"),
        read_plan_csv(rdapy_path(NC_PLAN)).expect("path reader"),
    );
}

#[test]
fn plan_csv_reads_a_hand_written_buffer() {
    // The BOM and the `GEOID20` spelling are both what DRA's exports carry.
    let csv = "\u{feff}GEOID20,District\n001,1\n002,2\n003, 2 \n";
    let plan = read_plan_csv_from(Cursor::new(csv)).expect("reading the plan");
    assert_eq!(plan.len(), 3);
    assert_eq!(plan["001"], 1);
    assert_eq!(plan["003"], 2);
}

#[test]
fn graph_reads_a_hand_written_buffer() {
    let text = r#"{"a": ["b"], "b": ["a", "c"], "c": []}"#;
    let graph = load_graph_from(Cursor::new(text)).expect("reading the graph");
    assert_eq!(
        graph,
        vec![
            ("a".to_string(), vec!["b".to_string()]),
            ("b".to_string(), vec!["a".to_string(), "c".to_string()]),
            ("c".to_string(), vec![]),
        ]
    );
}

/// A three-plan ensemble with the untagged records a real one carries.
const ENSEMBLE: &str = concat!(
    r#"{"_tag_": "metadata", "ndistricts": 2}"#,
    "\n",
    r#"{"_tag_": "plan", "plan": {"a": 1, "b": 2}}"#,
    "\n\n",
    r#"{"_tag_": "graph", "graph": {}}"#,
    "\n",
    r#"{"_tag_": "plan", "plan": {"a": "2", "b": "1"}}"#,
    "\n",
    r#"{"_tag_": "plan", "plan": {"a": 1, "b": 1}}"#,
    "\n",
);

#[test]
fn plans_jsonl_reads_a_hand_written_buffer() {
    let all = read_plans_jsonl_from(Cursor::new(ENSEMBLE), None).expect("reading plans");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0]["a"], 1);
    // Districts written as strings are parsed, as rdapy accepts them.
    assert_eq!(all[1]["a"], 2);

    let limited = read_plans_jsonl_from(Cursor::new(ENSEMBLE), Some(2)).expect("reading plans");
    assert_eq!(limited, all[..2]);
}

#[test]
fn plan_jsonl_indexes_past_untagged_records() {
    for (i, want) in [("a", 1u32), ("a", 2), ("a", 1)].iter().enumerate() {
        let plan = read_plan_jsonl_from(Cursor::new(ENSEMBLE), i).expect("reading a plan");
        assert_eq!(plan[want.0], want.1, "plan {i}");
    }
    let err = read_plan_jsonl_from(Cursor::new(ENSEMBLE), 3).unwrap_err();
    assert!(
        err.to_string().contains("fewer than 4 plans"),
        "unexpected error: {err}"
    );
}

#[test]
fn geojson_reads_a_hand_written_buffer() {
    let doc = json!({
        "type": "FeatureCollection",
        "features": [{
            "type": "Feature",
            "properties": {"GEOID20": "001"},
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0], [0.0, 0.0]]],
            },
        }],
    });
    let text = doc.to_string();

    let from_reader = load_geojson_from(Cursor::new(&text)).expect("reader");
    let from_value = features_of(&doc).expect("already-parsed document");

    for features in [&from_reader, &from_value] {
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].properties["GEOID20"], "001");
        assert_eq!(features[0].geometry.area(), 2.0);
        // `extract-data` copies the geometry through untouched.
        assert_eq!(features[0].raw_geometry, doc["features"][0]["geometry"]);
    }
}
