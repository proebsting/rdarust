//! The CLI must produce a scores CSV byte-identical to rdapy's.
//!
//! The golden files in `conformance/cases/cli/` were written by rdapy's own
//! pipeline (see `conformance/tools/gen_cli_golden.sh`). Byte identity is a
//! strong and cheap check: it covers every metric, the column order, the
//! number formatting and the line endings at once.

use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rdapy(rel: &str) -> String {
    repo().join("vendor/rdapy").join(rel).to_string_lossy().into_owned()
}

fn golden(name: &str) -> PathBuf {
    repo().join("conformance/cases/cli").join(name)
}

const DATA: &str = "testdata/examples/NC_input_data.jsonl";
const GRAPH: &str = "testdata/examples/NC_graph.json";
const PRE: &str = "testdata/examples/NC_congress_precomputed.json";
const PLANS: &str = "testdata/plans/NC_congress_plans.tagged.jsonl";

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

/// Compare against the golden CSV, reporting the first differing line rather
/// than just "files differ".
fn assert_matches_golden(produced: &PathBuf, golden_name: &str) {
    let want_path = golden(golden_name);
    let want = std::fs::read(&want_path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\nrun conformance/tools/gen_cli_golden.sh",
            want_path.display()
        )
    });
    let got = std::fs::read(produced).expect("reading produced CSV");

    if got == want {
        return;
    }
    let (g, w) = (String::from_utf8_lossy(&got), String::from_utf8_lossy(&want));
    for (i, (gl, wl)) in g.lines().zip(w.lines()).enumerate() {
        if gl != wl {
            let (gc, wc): (Vec<_>, Vec<_>) = (gl.split(',').collect(), wl.split(',').collect());
            let col = gc.iter().zip(wc.iter()).position(|(a, b)| a != b);
            panic!(
                "{golden_name}: line {} differs{}\n  got:  {gl}\n  want: {wl}",
                i + 1,
                col.map(|c| format!(" at column {c}")).unwrap_or_default()
            );
        }
    }
    panic!(
        "{golden_name}: line counts differ ({} vs {})",
        g.lines().count(),
        w.lines().count()
    );
}

#[test]
fn score_all_matches_rdapy_byte_for_byte() {
    let dir = tempdir("score-all");
    let scores = dir.join("scores.csv");

    run(&[
        "score-all",
        "--state", "NC", "--plan-type", "congress",
        "--data", &rdapy(DATA), "--graph", &rdapy(GRAPH),
        "--precomputed", &rdapy(PRE), "--plans", &rdapy(PLANS),
        "--scores", scores.to_str().unwrap(),
        "--by-district", dir.join("bd.jsonl").to_str().unwrap(),
    ]);
    assert_matches_golden(&scores, "NC_scores.csv");
}

#[test]
fn prefixed_output_matches_rdapy() {
    let dir = tempdir("prefixes");
    let scores = dir.join("scores.csv");

    run(&[
        "score-all",
        "--state", "NC", "--plan-type", "congress",
        "--data", &rdapy(DATA), "--graph", &rdapy(GRAPH),
        "--precomputed", &rdapy(PRE), "--plans", &rdapy(PLANS),
        "--scores", scores.to_str().unwrap(),
        "--by-district", dir.join("bd.jsonl").to_str().unwrap(),
        "--prefixes",
    ]);
    assert_matches_golden(&scores, "NC_scores.prefixed.csv");
}

/// The three-stage pipeline exists so it can drop into an existing SCORE.sh,
/// so it has to agree with the fused path and with rdapy.
#[test]
fn staged_pipeline_matches_fused() {
    let dir = tempdir("stages");
    let aggs = dir.join("aggs.jsonl");
    let scored = dir.join("scored.jsonl");
    let scores = dir.join("scores.csv");

    run(&[
        "aggregate",
        "--state", "NC", "--plan-type", "congress",
        "--data", &rdapy(DATA), "--graph", &rdapy(GRAPH),
        "--input", &rdapy(PLANS), "--output", aggs.to_str().unwrap(),
    ]);
    run(&[
        "score",
        "--state", "NC", "--plan-type", "congress",
        "--data", &rdapy(DATA), "--graph", &rdapy(GRAPH),
        "--precomputed", &rdapy(PRE),
        "--input", aggs.to_str().unwrap(), "--output", scored.to_str().unwrap(),
    ]);
    run(&[
        "write",
        "--data", &rdapy(DATA),
        "--input", scored.to_str().unwrap(),
        "--scores", scores.to_str().unwrap(),
        "--by-district", dir.join("bd.jsonl").to_str().unwrap(),
    ]);

    assert_matches_golden(&scores, "NC_scores.csv");
}

/// A populated precinct missing from the plan is a data error. rdapy skips
/// the plan and carries on, which is how a plan disappears from the output
/// unnoticed; we stop unless told otherwise.
#[test]
fn a_broken_plan_stops_the_run_by_default() {
    let dir = tempdir("broken");
    let plans = dir.join("plans.jsonl");
    // A plan naming only one precinct leaves every other one unassigned.
    std::fs::write(
        &plans,
        "{\"_tag_\": \"plan\", \"name\": \"broken\", \"plan\": {\"37021000101\": 1}}\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_rdarust"))
        .args([
            "score-all",
            "--state", "NC", "--plan-type", "congress",
            "--data", &rdapy(DATA), "--graph", &rdapy(GRAPH),
            "--plans", plans.to_str().unwrap(),
            "--scores", dir.join("s.csv").to_str().unwrap(),
            "--by-district", dir.join("b.jsonl").to_str().unwrap(),
        ])
        .output()
        .expect("running rdarust");

    assert!(!out.status.success(), "a broken plan should fail the run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not in the plan"),
        "the error should say what is wrong, got: {stderr}"
    );
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rdarust-cli-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("creating a temporary directory");
    dir
}
