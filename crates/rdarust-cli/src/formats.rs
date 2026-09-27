//! Converting ensembles into the tagged JSONL the pipeline reads.
//!
//! Ports `scripts/formats/`. Each converter emits one metadata record
//! followed by one `plan` record per plan, which `rdarust aggregate` and
//! `rdarust score-all` both take as input.

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{anyhow, Context as _, Result};
use rdarust_io::records::{expand, smart_reader, smart_writer, write_record, write_record_sorted};
use rdarust_io::read_plan_csv;
use serde_json::{json, Map, Value};

/// Convert a legacy single-JSON ensemble.
///
/// The file is an object with a `plans` list; everything beside it is treated
/// as the ensemble's metadata.
pub fn from_json(input: &str, output: Option<&str>) -> Result<()> {
    let text = std::fs::read_to_string(expand(input))
        .with_context(|| format!("reading {input}"))?;
    let mut doc: Map<String, Value> = serde_json::from_str(&text)
        .with_context(|| format!("parsing {input}"))?;

    let plans = doc
        .remove("plans")
        .ok_or_else(|| anyhow!("{input} has no `plans` list"))?;
    let plans = plans
        .as_array()
        .ok_or_else(|| anyhow!("{input}: `plans` is not a list"))?;

    let mut out = smart_writer(output).context("opening output")?;

    let mut metadata = Map::new();
    metadata.insert("_tag_".into(), json!("metadata"));
    metadata.insert("properties".into(), Value::Object(doc));
    write_record_sorted(&mut out, &Value::Object(metadata))?;

    for (i, entry) in plans.iter().enumerate() {
        let mut rec = Map::new();
        rec.insert("_tag_".into(), json!("plan"));
        rec.insert(
            "name".into(),
            entry.get("name").cloned().unwrap_or_else(|| json!(format!("{i:06}"))),
        );
        rec.insert(
            "plan".into(),
            entry
                .get("plan")
                .cloned()
                .ok_or_else(|| anyhow!("plan {i} has no `plan` object"))?,
        );
        write_record_sorted(&mut out, &Value::Object(rec))?;
    }

    out.flush()?;
    eprintln!("rdarust: converted {} plans", plans.len());
    Ok(())
}

/// Expand shell globs that the shell has not already expanded.
///
/// Passing `--files 'dir/*.csv'` quoted is the usual way to get more plans
/// than a command line will hold.
fn expand_globs(patterns: &[String]) -> Result<Vec<std::path::PathBuf>> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();

    for pattern in patterns {
        if !pattern.contains('*') && !pattern.contains('?') {
            out.push(expand(pattern));
            continue;
        }
        let path = expand(pattern);
        let (dir, name) = (
            path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let mut matched: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .map(|n| glob_match(&name, &n.to_string_lossy()))
                    .unwrap_or(false)
            })
            .collect();
        matched.sort();
        if matched.is_empty() {
            // Keep an unmatched pattern so the failure names it, as the
            // Python version does.
            out.push(path);
        } else {
            out.extend(matched);
        }
    }
    Ok(out)
}

/// Shell-style `*` and `?` matching.
fn glob_match(pattern: &str, name: &str) -> bool {
    fn go(p: &[u8], n: &[u8]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some(b'*') => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            Some(b'?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some(c) => n.first() == Some(c) && go(&p[1..], &n[1..]),
        }
    }
    go(pattern.as_bytes(), name.as_bytes())
}

/// Convert a set of precinct-assignment CSVs into one ensemble.
///
/// Every file must be for the same state and chamber; each becomes one plan,
/// named after its file.
pub fn from_csvs(
    patterns: &[String],
    output: Option<&str>,
    state: Option<&str>,
    plan_type: Option<&str>,
) -> Result<()> {
    let paths = expand_globs(patterns)?;
    if paths.is_empty() {
        return Err(anyhow!("no files matched"));
    }
    let absolute: Vec<String> = paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| p.clone())
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    let mut out = smart_writer(output).context("opening output")?;

    let mut props = Map::new();
    props.insert("method".into(), json!("From CSVs"));
    if let Some(s) = state {
        props.insert("state".into(), json!(s));
    }
    if let Some(t) = plan_type {
        props.insert("plan_type".into(), json!(t));
    }
    props.insert("size".into(), json!(absolute.len()));
    props.insert("files".into(), json!(absolute));

    let mut metadata = Map::new();
    metadata.insert("_tag_".into(), json!("metadata"));
    metadata.insert("properties".into(), Value::Object(props));
    write_record_sorted(&mut out, &Value::Object(metadata))?;

    for path in &paths {
        let plan = read_plan_csv(path)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("reading {}", path.display()))?;
        let mut assignments = Map::new();
        for (geoid, district) in plan {
            assignments.insert(geoid, json!(district));
        }

        let mut rec = Map::new();
        rec.insert("_tag_".into(), json!("plan"));
        rec.insert(
            "name".into(),
            json!(path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()),
        );
        rec.insert("plan".into(), Value::Object(assignments));
        write_record_sorted(&mut out, &Value::Object(rec))?;
    }

    out.flush()?;
    eprintln!("rdarust: converted {} plans", paths.len());
    Ok(())
}

/// Convert GerryTools canonical output into geoid assignments.
///
/// Canonical records identify precincts by their index in the ReCom graph
/// rather than by geoid, so the graph is needed to name them. It is a
/// networkx node-link JSON, and node order is the index order.
pub fn from_canonical(
    graph_path: &str,
    input: Option<&str>,
    output: Option<&str>,
    geoid_key: &str,
    keep_district_numbers: bool,
) -> Result<()> {
    let text = std::fs::read_to_string(expand(graph_path))
        .with_context(|| format!("reading {graph_path}"))?;
    let graph: Value = serde_json::from_str(&text)
        .with_context(|| format!("parsing {graph_path}"))?;

    let geoids: Vec<String> = graph
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| anyhow!("{graph_path} has no `nodes` array"))?
        .iter()
        .map(|n| {
            n.get(geoid_key)
                .and_then(|g| g.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    if geoids.iter().all(|g| g.is_empty()) {
        return Err(anyhow!(
            "no node in {graph_path} has a `{geoid_key}` property; try --geoid"
        ));
    }

    let reader = smart_reader(input).context("opening the canonical stream")?;
    let mut out = smart_writer(output).context("opening output")?;
    let mut count = 0usize;
    let mut shifted = false;

    for (n, line) in reader.lines().enumerate() {
        let line = line.context("reading input")?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: Value = serde_json::from_str(&line)
            .with_context(|| format!("line {}: invalid JSON", n + 1))?;

        let assignment = rec
            .get("assignment")
            .and_then(|a| a.as_array())
            .ok_or_else(|| anyhow!("line {}: record has no `assignment` list", n + 1))?;
        if assignment.len() != geoids.len() {
            return Err(anyhow!(
                "line {}: {} assignments for {} graph nodes",
                n + 1,
                assignment.len(),
                geoids.len()
            ));
        }

        // rustrecom normalises district labels to 0-based internally, by
        // subtracting the lowest, and writes them out that way whatever the
        // seed used. Scoring numbers districts from 1, so shift them back.
        // The shift preserves identity -- it is an offset, not a relabelling.
        let shift: i64 = if keep_district_numbers {
            0
        } else {
            match assignment.iter().filter_map(|d| d.as_i64()).min() {
                Some(lo) if lo < 1 => 1 - lo,
                _ => 0,
            }
        };
        if shift != 0 && !shifted {
            eprintln!(
                "rdarust: districts are numbered from {}; shifting to start at 1 \
                 (--keep-district-numbers to leave them)",
                1 - shift
            );
            shifted = true;
        }

        // Keyed in precinct order. rdapy groups by district first, which
        // leaves the keys in an order that depends on Python's set
        // iteration; the pairs are the same either way.
        let mut plan = Map::new();
        for (i, district) in assignment.iter().enumerate() {
            let d = match district.as_i64() {
                Some(d) => json!(d + shift),
                None => district.clone(),
            };
            plan.insert(geoids[i].clone(), d);
        }

        let mut record = Map::new();
        record.insert("_tag_".into(), json!("plan"));
        record.insert(
            "name".into(),
            rec.get("sample").cloned().unwrap_or_else(|| json!(format!("{count:06}"))),
        );
        record.insert("plan".into(), Value::Object(plan));
        // rdapy writes these unsorted, unlike the other converters.
        write_record(&mut out, &Value::Object(record))?;
        count += 1;
    }

    out.flush()?;
    eprintln!("rdarust: converted {count} plans");
    Ok(())
}

/// Keep every `k`th line, for subsampling a large ensemble.
///
/// Ports `scripts/throughput/SAMPLE.sh`, which keeps lines k, 2k, 3k and so
/// on -- so the first line, often a metadata record, is dropped unless k is 1.
pub fn sample(input: Option<&str>, output: Option<&str>, k: usize) -> Result<()> {
    if k == 0 {
        return Err(anyhow!("the sample rate must be at least 1"));
    }
    let reader = smart_reader(input).context("opening input")?;
    let mut out = smart_writer(output).context("opening output")?;

    let mut kept = 0usize;
    for (i, line) in reader.lines().enumerate() {
        let line = line.context("reading input")?;
        if (i + 1) % k == 0 {
            out.write_all(line.as_bytes())?;
            out.write_all(b"\n")?;
            kept += 1;
        }
    }
    out.flush()?;
    eprintln!("rdarust: kept {kept} of every {k} lines");
    Ok(())
}
