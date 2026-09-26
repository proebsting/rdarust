//! Reading plans.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::LoadError;

/// A plan: geoid to district number.
pub type Assignments = HashMap<String, u32>;

const GEOID_FIELDS: [&str; 3] = ["GEOID", "GEOID20", "GEOID30"];
const DISTRICT_FIELDS: [&str; 2] = ["District", "DISTRICT"];

/// Read a precinct-assignment CSV.
///
/// Column names vary between vintages, so the geoid and district columns are
/// found by name from a small set of known spellings, as rdapy does.
pub fn read_plan_csv(path: impl AsRef<Path>) -> Result<Assignments, LoadError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path.as_ref())
        .map_err(csv_err)?;

    let headers = reader.headers().map_err(csv_err)?.clone();
    let find = |names: &[&str]| {
        headers.iter().position(|h| {
            // Tolerate a UTF-8 BOM on the first column, which these files carry.
            let h = h.trim_start_matches('\u{feff}');
            names.contains(&h)
        })
    };
    let geoid_col = find(&GEOID_FIELDS).ok_or_else(|| LoadError::Malformed {
        line: 1,
        what: format!("no geoid column; looked for {GEOID_FIELDS:?}"),
    })?;
    let district_col = find(&DISTRICT_FIELDS).ok_or_else(|| LoadError::Malformed {
        line: 1,
        what: format!("no district column; looked for {DISTRICT_FIELDS:?}"),
    })?;

    let mut plan = Assignments::new();
    for (n, record) in reader.records().enumerate() {
        let record = record.map_err(csv_err)?;
        let geoid = record.get(geoid_col).unwrap_or_default().to_string();
        let district: u32 = record
            .get(district_col)
            .unwrap_or_default()
            .trim()
            .parse()
            .map_err(|_| LoadError::Malformed {
                line: n + 2,
                what: format!("district {:?} is not a number", record.get(district_col)),
            })?;
        plan.insert(geoid, district);
    }
    Ok(plan)
}

fn csv_err(e: csv::Error) -> LoadError {
    LoadError::Malformed { line: 0, what: e.to_string() }
}

/// Read the `n`th plan from a tagged JSONL ensemble.
///
/// Records that are not tagged `plan` -- metadata, adjacency graphs -- are
/// skipped without counting.
pub fn read_plan_jsonl(path: impl AsRef<Path>, index: usize) -> Result<Assignments, LoadError> {
    let reader = BufReader::new(File::open(path.as_ref())?);
    let mut seen = 0usize;

    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: serde_json::Value = serde_json::from_str(&line)
            .map_err(|source| LoadError::Json { line: n + 1, source })?;
        if rec.get("_tag_").and_then(|t| t.as_str()) != Some("plan") {
            continue;
        }
        if seen == index {
            let obj = rec.get("plan").and_then(|p| p.as_object()).ok_or(
                LoadError::Malformed { line: n + 1, what: "plan record has no plan object".into() },
            )?;
            return Ok(obj
                .iter()
                .filter_map(|(k, v)| {
                    let d = v.as_u64().or_else(|| v.as_str()?.parse().ok())?;
                    Some((k.clone(), d as u32))
                })
                .collect());
        }
        seen += 1;
    }
    Err(LoadError::Malformed {
        line: 0,
        what: format!("ensemble has fewer than {} plans", index + 1),
    })
}

/// Read every plan from a tagged JSONL ensemble, up to `limit`.
///
/// One pass over the file, unlike calling [`read_plan_jsonl`] per index.
pub fn read_plans_jsonl(
    path: impl AsRef<Path>,
    limit: Option<usize>,
) -> Result<Vec<Assignments>, LoadError> {
    let reader = BufReader::new(File::open(path.as_ref())?);
    let mut out = Vec::new();

    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: serde_json::Value = serde_json::from_str(&line)
            .map_err(|source| LoadError::Json { line: n + 1, source })?;
        if rec.get("_tag_").and_then(|t| t.as_str()) != Some("plan") {
            continue;
        }
        let obj = rec.get("plan").and_then(|p| p.as_object()).ok_or(
            LoadError::Malformed { line: n + 1, what: "plan record has no plan object".into() },
        )?;
        out.push(
            obj.iter()
                .filter_map(|(k, v)| {
                    let d = v.as_u64().or_else(|| v.as_str()?.parse().ok())?;
                    Some((k.clone(), d as u32))
                })
                .collect(),
        );
        if limit.is_some_and(|l| out.len() >= l) {
            break;
        }
    }
    Ok(out)
}
