//! Reading plans.
//!
//! Each reader comes in two forms: the bare name opens a path, and the
//! `_from` variant takes an already-open reader. Only the `_from` variants
//! hold the logic, so a caller with bytes in hand -- a fetched file, a
//! decompressed stream, a test fixture -- never has to go through the
//! filesystem.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde_json::Value;

use crate::LoadError;

/// A plan: geoid to district number.
pub type Assignments = HashMap<String, u32>;

const GEOID_FIELDS: [&str; 3] = ["GEOID", "GEOID20", "GEOID30"];
const DISTRICT_FIELDS: [&str; 2] = ["District", "DISTRICT"];

/// Read a precinct-assignment CSV file.
pub fn read_plan_csv(path: impl AsRef<Path>) -> Result<Assignments, LoadError> {
    read_plan_csv_from(File::open(path.as_ref())?)
}

/// Read a precinct-assignment CSV from an open reader.
///
/// Column names vary between vintages, so the geoid and district columns are
/// found by name from a small set of known spellings, as rdapy does.
pub fn read_plan_csv_from(reader: impl Read) -> Result<Assignments, LoadError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(reader);

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

/// Call `f` with each `plan`-tagged record and its line number, in file
/// order, stopping early when it returns `false`.
///
/// Records that are not tagged `plan` -- metadata, adjacency graphs -- are
/// skipped without counting.
fn for_each_plan_record(
    reader: impl BufRead,
    mut f: impl FnMut(usize, &Value) -> Result<bool, LoadError>,
) -> Result<(), LoadError> {
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: Value = serde_json::from_str(&line)
            .map_err(|source| LoadError::Json { line: n + 1, source })?;
        if rec.get("_tag_").and_then(|t| t.as_str()) != Some("plan") {
            continue;
        }
        if !f(n + 1, &rec)? {
            break;
        }
    }
    Ok(())
}

/// The assignments carried by one `plan` record.
fn plan_of(line: usize, rec: &Value) -> Result<Assignments, LoadError> {
    let obj = rec.get("plan").and_then(|p| p.as_object()).ok_or(LoadError::Malformed {
        line,
        what: "plan record has no plan object".into(),
    })?;
    Ok(obj
        .iter()
        .filter_map(|(k, v)| {
            let d = v.as_u64().or_else(|| v.as_str()?.parse().ok())?;
            Some((k.clone(), d as u32))
        })
        .collect())
}

/// Read the `n`th plan from a tagged JSONL ensemble file.
pub fn read_plan_jsonl(path: impl AsRef<Path>, index: usize) -> Result<Assignments, LoadError> {
    read_plan_jsonl_from(BufReader::new(File::open(path.as_ref())?), index)
}

/// Read the `n`th plan from a tagged JSONL ensemble.
///
/// Plans before `index` are counted but not parsed, so seeking deep into a
/// large ensemble costs one pass of line splitting rather than `index`
/// assignment maps.
pub fn read_plan_jsonl_from(
    reader: impl BufRead,
    index: usize,
) -> Result<Assignments, LoadError> {
    let mut seen = 0usize;
    let mut found = None;
    for_each_plan_record(reader, |line, rec| {
        if seen == index {
            found = Some(plan_of(line, rec)?);
            return Ok(false);
        }
        seen += 1;
        Ok(true)
    })?;
    found.ok_or_else(|| LoadError::Malformed {
        line: 0,
        what: format!("ensemble has fewer than {} plans", index + 1),
    })
}

/// Read every plan from a tagged JSONL ensemble file, up to `limit`.
pub fn read_plans_jsonl(
    path: impl AsRef<Path>,
    limit: Option<usize>,
) -> Result<Vec<Assignments>, LoadError> {
    read_plans_jsonl_from(BufReader::new(File::open(path.as_ref())?), limit)
}

/// Read every plan from a tagged JSONL ensemble, up to `limit`.
///
/// One pass over the stream, unlike calling [`read_plan_jsonl_from`] per
/// index.
pub fn read_plans_jsonl_from(
    reader: impl BufRead,
    limit: Option<usize>,
) -> Result<Vec<Assignments>, LoadError> {
    let mut out = Vec::new();
    for_each_plan_record(reader, |line, rec| {
        out.push(plan_of(line, rec)?);
        Ok(!limit.is_some_and(|l| out.len() >= l))
    })?;
    Ok(out)
}
