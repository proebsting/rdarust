//! The conformance corpus must match the rdapy it was recorded from.
//!
//! Every expected value in `conformance/cases/` came out of one specific
//! rdapy commit. Move the submodule without regenerating and the tests keep
//! passing -- but they are then checking this port against values that no
//! longer describe the code being ported, which is the one failure a test
//! suite cannot report on its own.
//!
//! This compares the commit recorded with the corpus against the submodule as
//! it actually stands.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The commit the submodule is checked out at, or `None` when that cannot be
/// determined -- an export without git, or without the submodule initialised.
fn submodule_head() -> Option<String> {
    let out = Command::new("git")
        .args(["-C", repo().join("vendor/rdapy").to_str()?, "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (sha.len() == 40).then_some(sha)
}

/// Every JSON file under `conformance/cases/`, recursively.
fn case_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            case_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
}

#[test]
fn the_corpus_matches_the_pinned_rdapy() {
    let cases = repo().join("conformance/cases");
    let provenance = cases.join("PROVENANCE.json");

    let text = std::fs::read_to_string(&provenance).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\nrun conformance/tools/regenerate.sh",
            provenance.display()
        )
    });
    let doc: Value = serde_json::from_str(&text).expect("PROVENANCE.json is not valid JSON");
    let recorded = doc["rdapy_commit"]
        .as_str()
        .expect("PROVENANCE.json has no rdapy_commit");

    assert!(
        !doc["rdapy_dirty"].as_bool().unwrap_or(false),
        "the corpus was recorded from a modified submodule, so it cannot be \
         reproduced from {recorded} alone; commit or discard those changes and \
         run conformance/tools/regenerate.sh"
    );

    let Some(head) = submodule_head() else {
        // A source export, or the submodule is not initialised. The corpus is
        // still self-consistent; there is just nothing to compare it against.
        eprintln!(
            "provenance: cannot read the submodule's commit, so the corpus \
             cannot be checked against it (recorded: {recorded})"
        );
        return;
    };

    assert_eq!(
        head, recorded,
        "\n  the conformance corpus was recorded from rdapy {recorded}\n  \
         but vendor/rdapy is checked out at {head}\n\n  \
         Every expected value in conformance/cases/ came from the recorded \
         commit, so the tests are no longer checking this port against the \
         rdapy it is pinned to. Either:\n    \
         git -C vendor/rdapy checkout {recorded}\n  or, to adopt the new \
         commit:\n    conformance/tools/regenerate.sh\n"
    );

    // Each generator stamps the commit into the files it writes; those must
    // agree too, or part of the corpus was regenerated on its own.
    let mut files = Vec::new();
    case_files(&cases, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no case files found under {}", cases.display());

    let mut checked = 0usize;
    for path in &files {
        let doc: Value = match std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
        {
            Some(v) => v,
            None => continue,
        };
        let Some(stamped) = doc.get("source").and_then(|s| s.get("rdapy_commit")).and_then(|c| c.as_str())
        else {
            continue;
        };
        assert_eq!(
            stamped,
            recorded,
            "{} was recorded from rdapy {stamped}, but the rest of the corpus \
             came from {recorded}; regenerate the whole corpus rather than one \
             part of it",
            path.strip_prefix(repo()).unwrap_or(path).display()
        );
        checked += 1;
    }

    eprintln!(
        "provenance: {checked} of {} case files stamped with rdapy {}, matching the submodule",
        files.len(),
        &recorded[..12]
    );
}
