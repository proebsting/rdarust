//! The ensemble format converters.
//!
//! Checked by round-tripping against the source rather than against stored
//! output: a converter's job is to preserve the assignments, and comparing
//! the result back to what it came from catches a mis-mapped geoid or an
//! off-by-one index, which a golden file would only catch if it happened to
//! be regenerated.
//!
//! Byte-identity with rdapy's own output is verified separately -- `from-json`,
//! `from-csvs` and `sample` all match exactly.

use std::collections::HashMap;
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
    let dir = std::env::temp_dir().join(format!("rdarust-formats-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("creating a temporary directory");
    dir
}

fn records(path: &PathBuf) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("reading JSONL")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("parsing a record"))
        .collect()
}

fn plans(recs: &[Value]) -> Vec<&Value> {
    recs.iter()
        .filter(|r| r["_tag_"].as_str() == Some("plan"))
        .collect()
}

#[test]
fn from_json_preserves_every_plan() {
    let dir = tempdir("json");
    let out = dir.join("plans.jsonl");
    let source = rdapy("testdata/plans/NC_congress_plans.legacy.json");

    run(&["from-json", "--input", &source, "--output", out.to_str().unwrap()]);

    let legacy: Value =
        serde_json::from_str(&std::fs::read_to_string(&source).unwrap()).unwrap();
    let want = legacy["plans"].as_array().unwrap();

    let recs = records(&out);
    assert_eq!(
        recs.iter().filter(|r| r["_tag_"].as_str() == Some("metadata")).count(),
        1,
        "exactly one metadata record"
    );
    let got = plans(&recs);
    assert_eq!(got.len(), want.len());

    for (g, w) in got.iter().zip(want.iter()) {
        assert_eq!(g["name"], w["name"], "plan name");
        assert_eq!(g["plan"], w["plan"], "assignments for {}", w["name"]);
    }

    // The metadata is everything in the source but the plans.
    let meta = &recs[0]["properties"];
    assert!(meta.get("plans").is_none(), "plans must not be in the metadata");
    for key in ["state", "cycle", "plan_type", "ndistricts"] {
        assert_eq!(meta[key], legacy[key], "metadata key {key}");
    }
}

#[test]
fn from_csvs_makes_one_plan_per_file() {
    let dir = tempdir("csvs");
    let out = dir.join("plans.jsonl");
    let pattern = rdapy("testdata/plans/csvs/NC_congress.00*.csv");

    run(&[
        "from-csvs",
        "--files", &pattern,
        "--state", "NC", "--plan-type", "congress",
        "--output", out.to_str().unwrap(),
    ]);

    let recs = records(&out);
    let got = plans(&recs);
    assert_eq!(got.len(), 9, "NC_congress.001 through .009");

    let meta = &recs[0]["properties"];
    assert_eq!(meta["state"], "NC");
    assert_eq!(meta["plan_type"], "congress");
    assert_eq!(meta["size"], 9);
    assert_eq!(meta["method"], "From CSVs");

    // Each plan is named for its file and carries that file's assignments.
    for plan in &got {
        let name = plan["name"].as_str().expect("a file name");
        let source = rdapy(&format!("testdata/plans/csvs/{name}"));
        let want = rdarust_io::read_plan_csv(&source).expect("reading the source CSV");

        let got_plan: HashMap<String, u32> = plan["plan"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_u64().unwrap() as u32))
            .collect();
        assert_eq!(got_plan, want, "assignments for {name}");
    }
}

#[test]
fn from_canonical_maps_each_index_to_its_graph_node() {
    let dir = tempdir("canonical");
    let out = dir.join("plans.jsonl");
    let graph_path = rdapy("testdata/plans/canonical/NC_congress_recom_graph.seeded.json");
    let canonical = rdapy("testdata/plans/canonical/NC_congress_plans.canonical.jsonl");

    run(&[
        "from-canonical",
        "--graph", &graph_path,
        "--input", &canonical,
        "--output", out.to_str().unwrap(),
    ]);

    // Canonical records identify precincts by their index in the graph, so
    // the mapping from index to geoid is the whole job.
    let graph: Value =
        serde_json::from_str(&std::fs::read_to_string(&graph_path).unwrap()).unwrap();
    let geoids: Vec<&str> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["GEOID"].as_str().unwrap())
        .collect();

    let source: Vec<Value> = std::fs::read_to_string(&canonical)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

    let recs = records(&out);
    let got = plans(&recs);
    assert_eq!(got.len(), source.len());

    for (plan, src) in got.iter().zip(source.iter()) {
        // Plans are named by sample number, which is an integer.
        assert_eq!(plan["name"], src["sample"], "plan name");

        let assignment = src["assignment"].as_array().unwrap();
        let obj = plan["plan"].as_object().unwrap();
        assert_eq!(obj.len(), assignment.len(), "one entry per graph node");

        for (i, district) in assignment.iter().enumerate() {
            assert_eq!(
                obj.get(geoids[i]),
                Some(district),
                "node {i} ({}) should be in district {district}",
                geoids[i]
            );
        }
    }
}

#[test]
fn sample_keeps_every_kth_line() {
    let dir = tempdir("sample");
    let out = dir.join("sampled.jsonl");
    let source = rdapy("testdata/plans/canonical/NC_congress_plans.canonical.jsonl");

    run(&[
        "sample",
        "--input", &source,
        "-k", "7",
        "--output", out.to_str().unwrap(),
    ]);

    let all: Vec<String> = std::fs::read_to_string(&source)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let kept: Vec<String> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();

    // Lines 7, 14, 21 and so on -- so line 1, often a metadata record, goes.
    let want: Vec<&String> = all.iter().skip(6).step_by(7).collect();
    assert_eq!(kept.len(), want.len());
    for (k, w) in kept.iter().zip(want.iter()) {
        assert_eq!(&k, w);
    }
}

/// The JSONL the stages exchange uses Python's separators, so it can be
/// diffed directly against rdapy's output.
#[test]
fn records_are_written_with_python_separators() {
    let dir = tempdir("separators");
    let out = dir.join("plans.jsonl");
    run(&[
        "from-json",
        "--input", &rdapy("testdata/plans/NC_congress_plans.legacy.json"),
        "--output", out.to_str().unwrap(),
    ]);

    let first = std::fs::read_to_string(&out).unwrap();
    let first = first.lines().next().unwrap();
    assert!(
        first.starts_with(r#"{"_tag_": "metadata", "properties": {"#),
        "expected Python's `\", \"` and `\": \"` separators, got: {}",
        &first[..60.min(first.len())]
    );
}

/// rustrecom normalises district labels to 0-based internally and writes them
/// out that way whatever the seed used, while scoring numbers districts from
/// 1. `from-canonical` shifts them back.
#[test]
fn zero_based_canonical_districts_are_shifted_to_start_at_one() {
    let dir = tempdir("zero-based");
    let graph = dir.join("graph.json");
    let canonical = dir.join("canonical.jsonl");

    // Four precincts, two districts, labelled from 0 as rustrecom writes them.
    std::fs::write(
        &graph,
        r#"{"directed":false,"multigraph":false,"graph":[],
            "nodes":[{"GEOID":"a","id":0},{"GEOID":"b","id":1},
                     {"GEOID":"c","id":2},{"GEOID":"d","id":3}],
            "adjacency":[[{"id":1}],[{"id":0}],[{"id":3}],[{"id":2}]]}"#,
    )
    .unwrap();
    std::fs::write(&canonical, "{\"assignment\":[0,0,1,1],\"sample\":1}\n").unwrap();

    let shifted = dir.join("shifted.jsonl");
    run(&[
        "from-canonical",
        "--graph", graph.to_str().unwrap(),
        "--input", canonical.to_str().unwrap(),
        "--output", shifted.to_str().unwrap(),
    ]);
    let plan = &records(&shifted)[0]["plan"];
    assert_eq!(plan["a"], 1, "district 0 becomes 1");
    assert_eq!(plan["c"], 2, "district 1 becomes 2");

    // Already 1-based, so nothing moves.
    std::fs::write(&canonical, "{\"assignment\":[1,1,2,2],\"sample\":1}\n").unwrap();
    let untouched = dir.join("untouched.jsonl");
    run(&[
        "from-canonical",
        "--graph", graph.to_str().unwrap(),
        "--input", canonical.to_str().unwrap(),
        "--output", untouched.to_str().unwrap(),
    ]);
    let plan = &records(&untouched)[0]["plan"];
    assert_eq!(plan["a"], 1);
    assert_eq!(plan["c"], 2);

    // And the shift can be declined.
    std::fs::write(&canonical, "{\"assignment\":[0,0,1,1],\"sample\":1}\n").unwrap();
    let kept = dir.join("kept.jsonl");
    run(&[
        "from-canonical",
        "--graph", graph.to_str().unwrap(),
        "--input", canonical.to_str().unwrap(),
        "--output", kept.to_str().unwrap(),
        "--keep-district-numbers",
    ]);
    let plan = &records(&kept)[0]["plan"];
    assert_eq!(plan["a"], 0, "left as it arrived");
}
