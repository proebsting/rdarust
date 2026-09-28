//! The per-state tables must still be what rdapy says they are.
//!
//! `states.rs` is a transcription of `rdapy/base/constants.py`, and a
//! transcription has no way to notice when its source changes. District
//! counts move with each decennial apportionment and county counts move when
//! the Census Bureau revises county equivalents -- Connecticut's planning
//! regions replaced its counties in the 2022 vintage -- so "these numbers
//! were right when I copied them" is not a property that stays true.
//!
//! This reads the Python and compares. It fails when the submodule moves to
//! an rdapy whose constants differ, which is the moment to regenerate rather
//! than the moment somebody notices a wrong ensemble.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rdarust_core::states::{counties_by_state, districts_by_state, Districts};

fn constants_py() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/rdapy/rdapy/base/constants.py");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The body of a top-level `NAME: ... = { ... }` assignment, up to the first
/// line that is a bare `}`.
fn dict_body<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("{name}:"))
        .unwrap_or_else(|| panic!("{name} is not in constants.py"));
    let open = text[start..].find('{').expect("dict opens") + start;
    let close = text[open..].find("\n}").expect("dict closes") + open;
    &text[open + 1..close]
}

/// `"AL": {"congress": 7, "upper": 35, "lower": 105},`
fn parse_districts(body: &str) -> BTreeMap<String, Districts> {
    let mut out = BTreeMap::new();
    for line in body.lines() {
        let line = line.trim().trim_end_matches(',');
        if line.is_empty() {
            continue;
        }
        let (key, rest) = line.split_once(':').expect("a keyed entry");
        let xx = key.trim().trim_matches('"').to_string();

        let field = |name: &str| -> Option<u32> {
            let at = rest.find(&format!("\"{name}\":"))?;
            let value = rest[at..].split_once(':')?.1;
            let value = value.trim_start().split([',', '}']).next()?.trim();
            value.parse().ok()
        };
        out.insert(
            xx,
            Districts {
                congress: field("congress").expect("congress"),
                upper: field("upper").expect("upper"),
                // `None` means the chamber has no districts of its own: the
                // lower house is elected from the upper house's districts,
                // or there is no lower house.
                lower: field("lower"),
            },
        );
    }
    out
}

/// `"AL": 67,`
fn parse_counties(body: &str) -> BTreeMap<String, u32> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| {
            let (key, value) = line.trim_end_matches(',').split_once(':').expect("a keyed entry");
            (
                key.trim().trim_matches('"').to_string(),
                value.trim().parse().expect("a count"),
            )
        })
        .collect()
}

#[test]
fn district_counts_match_rdapy() {
    let text = constants_py();
    let want = parse_districts(dict_body(&text, "DISTRICTS_BY_STATE"));
    assert!(!want.is_empty(), "parsed no states out of constants.py");

    let mut differences = Vec::new();
    for (xx, want) in &want {
        match districts_by_state(xx) {
            Some(got) if got == *want => {}
            other => differences.push(format!("  {xx}: rdapy {want:?}, rdarust {other:?}")),
        }
    }
    assert!(
        differences.is_empty(),
        "states.rs no longer matches rdapy's DISTRICTS_BY_STATE; regenerate it:\n{}",
        differences.join("\n")
    );

    // A state rdapy dropped would not show up above.
    assert_eq!(
        want.len(),
        50,
        "rdapy now lists {} states, not 50; states.rs needs regenerating",
        want.len()
    );
}

#[test]
fn county_counts_match_rdapy() {
    let text = constants_py();
    let want = parse_counties(dict_body(&text, "COUNTIES_BY_STATE"));
    assert!(!want.is_empty(), "parsed no states out of constants.py");

    let mut differences = Vec::new();
    for (xx, want) in &want {
        match counties_by_state(xx) {
            Some(got) if got == *want => {}
            other => differences.push(format!("  {xx}: rdapy {want}, rdarust {other:?}")),
        }
    }
    assert!(
        differences.is_empty(),
        "states.rs no longer matches rdapy's COUNTIES_BY_STATE; regenerate it:\n{}",
        differences.join("\n")
    );
    assert_eq!(want.len(), 50, "rdapy now lists {} states, not 50", want.len());
}

/// A sanity check on the apportionment itself, independent of rdapy: the
/// House has had 435 seats since 1913.
#[test]
fn congressional_seats_total_435() {
    let text = constants_py();
    let total: u32 = parse_districts(dict_body(&text, "DISTRICTS_BY_STATE"))
        .values()
        .map(|d| d.congress)
        .sum();
    assert_eq!(total, 435, "the apportionment does not add up to the House's size");
}
