//! Contiguity repair, end to end on a real state.
//!
//! North Carolina's graph is fully connected, so the test cuts precincts
//! loose from it to make a state that needs repair, then checks the full
//! cycle: detect, propose, apply, verify.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rdapy(rel: &str) -> String {
    repo().join("vendor/rdapy").join(rel).to_string_lossy().into_owned()
}

fn rdarust(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rdarust"))
        .args(args)
        .output()
        .expect("running rdarust")
}

fn run(args: &[&str]) {
    let out = rdarust(args);
    assert!(
        out.status.success(),
        "rdarust {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rdarust-graphs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("creating a temporary directory");
    dir
}

const GEOJSON: &str = "testdata/examples/NC_vtd_datasets.geojson";
const GRAPH: &str = "testdata/examples/NC_graph.json";
const DATA: &str = "testdata/examples/NC_input_data.jsonl";

/// Cut `count` border precincts loose, leaving them attached only to the
/// state border -- which does not count towards connectivity.
fn make_disconnected(dest: &PathBuf, count: usize) -> Vec<String> {
    let text = std::fs::read_to_string(rdapy(GRAPH)).expect("reading the graph");
    let mut graph: Value = serde_json::from_str(&text).unwrap();

    let border: Vec<String> = graph["OUT_OF_STATE"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut sorted = border.clone();
    sorted.sort();

    let stride = sorted.len() / count.max(1);
    let cut: Vec<String> = sorted.iter().step_by(stride.max(1)).take(count).cloned().collect();

    for c in &cut {
        let neighbors: Vec<String> = graph[c]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .filter(|n| n != "OUT_OF_STATE")
            .collect();
        for n in neighbors {
            for (a, b) in [(c.clone(), n.clone()), (n, c.clone())] {
                let list = graph[&a].as_array_mut().unwrap();
                list.retain(|v| v.as_str() != Some(b.as_str()));
            }
        }
    }

    std::fs::write(dest, serde_json::to_string(&graph).unwrap()).unwrap();
    cut
}

fn locations(dest: &PathBuf) {
    let dir = dest.parent().unwrap();
    run(&[
        "extract-graph",
        "--geojson", &rdapy(GEOJSON),
        "--graph", dir.join("throwaway.json").to_str().unwrap(),
        "--locations", dest.to_str().unwrap(),
    ]);
}

#[test]
fn a_connected_graph_needs_no_repair() {
    let dir = tempdir("connected");
    let locs = dir.join("locations.json");
    locations(&locs);
    let out = dir.join("mods.csv");

    run(&[
        "contiguity-mods",
        "--graph", &rdapy(GRAPH),
        "--locations", locs.to_str().unwrap(),
        "--output", out.to_str().unwrap(),
    ]);
    let proposed = std::fs::read_to_string(&out).unwrap_or_default();
    assert!(proposed.trim().is_empty(), "nothing to repair, got: {proposed}");
}

#[test]
fn the_repair_cycle_reconnects_a_broken_graph() {
    let dir = tempdir("repair");
    let locs = dir.join("locations.json");
    locations(&locs);

    let broken = dir.join("broken.json");
    let cut = make_disconnected(&broken, 5);
    assert_eq!(cut.len(), 5);

    // Detect.
    let out = rdarust(&[
        "check-graph",
        "--state", "NC",
        "--data", &rdapy(DATA),
        "--graph", broken.to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "a broken graph should fail the check");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("not fully connected"),
        "the check should say what is wrong"
    );

    // Propose.
    let mods = dir.join("mods.csv");
    run(&[
        "contiguity-mods",
        "--graph", broken.to_str().unwrap(),
        "--locations", locs.to_str().unwrap(),
        "--output", mods.to_str().unwrap(),
    ]);
    let proposed: Vec<String> = std::fs::read_to_string(&mods)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(proposed.len(), 5, "five islands need five edges back to the mainland");

    // Every proposal must name one of the precincts we cut loose.
    let cut_set: BTreeSet<&str> = cut.iter().map(String::as_str).collect();
    for line in &proposed {
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(fields[0], "+", "an addition");
        assert!(
            cut_set.contains(fields[1]) || cut_set.contains(fields[2]),
            "{line} does not touch any cut precinct"
        );
    }

    // Apply, and verify.
    let repaired = dir.join("repaired.json");
    run(&[
        "apply-mods",
        "--graph", broken.to_str().unwrap(),
        "--mods", mods.to_str().unwrap(),
        "--output", repaired.to_str().unwrap(),
    ]);
    run(&[
        "check-graph",
        "--state", "NC",
        "--data", &rdapy(DATA),
        "--graph", repaired.to_str().unwrap(),
    ]);

    // A repaired graph scores, which is the point of repairing it.
    run(&[
        "score-all",
        "--state", "NC", "--plan-type", "congress",
        "--data", &rdapy(DATA),
        "--graph", repaired.to_str().unwrap(),
        "--plans", &rdapy("testdata/plans/NC_congress_plans.tagged.jsonl"),
        "--scores", dir.join("scores.csv").to_str().unwrap(),
        "--by-district", dir.join("bd.jsonl").to_str().unwrap(),
    ]);
}

/// A mods row that is not an addition is rejected rather than being applied
/// as one. rdapy ignores the operation column entirely.
#[test]
fn a_non_addition_row_is_rejected() {
    let dir = tempdir("badmod");
    let mods = dir.join("mods.csv");
    std::fs::write(&mods, "-,37001000001,37001000002\n").unwrap();

    let out = rdarust(&[
        "apply-mods",
        "--graph", &rdapy(GRAPH),
        "--mods", mods.to_str().unwrap(),
        "--output", dir.join("out.json").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("only `+` adds an edge"),
        "the error should explain, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
