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

/// The ReCom graph has to satisfy GerryChain's reader exactly: node-link
/// shape, dense integer ids, symmetric adjacency, and a population on every
/// node. That GerryChain *accepts* it is checked separately, by
/// `conformance/tools/check_recom_graph.py`, which runs an actual chain.
#[test]
fn recom_graph_has_the_shape_gerrychain_reads() {
    let dir = tempdir("recom");
    let out = dir.join("recom.json");

    run(&[
        "to-recom-graph",
        "--state", "NC",
        "--data", &rdapy(DATA),
        "--graph", &rdapy(GRAPH),
        "--output", out.to_str().unwrap(),
    ]);

    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(doc["directed"], false);
    assert_eq!(doc["multigraph"], false);

    let nodes = doc["nodes"].as_array().expect("nodes");
    let adjacency = doc["adjacency"].as_array().expect("adjacency");
    assert_eq!(nodes.len(), adjacency.len(), "one adjacency list per node");
    assert_eq!(nodes.len(), 2666, "every NC precinct, and no border node");

    let mut total_pop: i64 = 0;
    for (i, node) in nodes.iter().enumerate() {
        assert_eq!(node["id"], i, "ids must be dense and in order");
        let geoid = node["GEOID"].as_str().expect("every node needs a geoid");
        assert_ne!(geoid, "OUT_OF_STATE", "the border node must not be present");
        assert_eq!(
            node["COUNTY"].as_str().unwrap(),
            &geoid[..5],
            "county is the five-character FIPS"
        );
        total_pop += node["TOTAL_POP"].as_i64().expect("every node needs a population");
    }
    assert_eq!(total_pop, 10_439_388, "population is conserved from the data file");

    // Adjacency must be symmetric, or ReCom's spanning trees are wrong.
    let neighbors: Vec<Vec<usize>> = adjacency
        .iter()
        .map(|list| {
            list.as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].as_u64().unwrap() as usize)
                .collect()
        })
        .collect();
    for (i, nbrs) in neighbors.iter().enumerate() {
        for &j in nbrs {
            assert!(neighbors[j].contains(&i), "edge {i}-{j} is not reciprocated");
        }
        assert!(nbrs.windows(2).all(|w| w[0] < w[1]), "node {i}: neighbours sorted and unique");
    }

    // Connected, which ReCom requires.
    let mut seen = vec![false; neighbors.len()];
    let mut stack = vec![0usize];
    seen[0] = true;
    let mut count = 1;
    while let Some(n) = stack.pop() {
        for &m in &neighbors[n] {
            if !seen[m] {
                seen[m] = true;
                count += 1;
                stack.push(m);
            }
        }
    }
    assert_eq!(count, neighbors.len(), "graph must be connected");
}

/// A disconnected graph is refused rather than written, since ReCom cannot
/// reach every precinct on one.
#[test]
fn a_disconnected_graph_is_refused() {
    let dir = tempdir("recom-broken");
    let broken = dir.join("broken.json");
    make_disconnected(&broken, 3);

    let out = rdarust(&[
        "to-recom-graph",
        "--state", "NC",
        "--data", &rdapy(DATA),
        "--graph", broken.to_str().unwrap(),
        "--output", dir.join("recom.json").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not fully connected") && stderr.contains("contiguity-mods"),
        "the error should say what to do about it, got: {stderr}"
    );
}

/// rustrecom's `--assignment-col` is required, so the graph has to carry a
/// starting plan. It is stamped from a plan you already have rather than
/// generated.
#[test]
fn a_seed_plan_can_be_stamped_onto_the_recom_graph() {
    let dir = tempdir("recom-seed");
    let out = dir.join("recom.json");

    run(&[
        "to-recom-graph",
        "--state", "NC",
        "--data", &rdapy(DATA),
        "--graph", &rdapy(GRAPH),
        "--assignment", &rdapy("testdata/plans/NC_congress_plans.tagged.jsonl"),
        "--output", out.to_str().unwrap(),
    ]);

    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let nodes = doc["nodes"].as_array().unwrap();

    let districts: std::collections::BTreeSet<i64> = nodes
        .iter()
        .map(|n| {
            n["INITIAL"]
                .as_i64()
                .expect("every node needs an assignment, or rustrecom panics")
        })
        .collect();

    // rustrecom accepts 0- or 1-indexed and rejects gaps.
    assert_eq!(districts.len(), 14, "North Carolina's fourteen districts");
    let lo = *districts.iter().next().unwrap();
    let hi = *districts.iter().next_back().unwrap();
    assert!(lo == 0 || lo == 1, "must be numbered from 0 or 1, starts at {lo}");
    assert_eq!(
        districts.len() as i64,
        hi - lo + 1,
        "no gaps: a district with no precincts is rejected by rustrecom"
    );
}

/// A seed that does not cover every precinct is refused here, where the
/// message can say so, rather than in rustrecom, where it panics.
#[test]
fn an_incomplete_seed_plan_is_refused() {
    let dir = tempdir("recom-badseed");
    let seed = dir.join("seed.csv");
    std::fs::write(&seed, "GEOID,District\n37001000001,1\n").unwrap();

    let out = rdarust(&[
        "to-recom-graph",
        "--state", "NC",
        "--data", &rdapy(DATA),
        "--graph", &rdapy(GRAPH),
        "--assignment", seed.to_str().unwrap(),
        "--output", dir.join("recom.json").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not assign") && stderr.contains("every"),
        "the error should name the gap, got: {stderr}"
    );
}
