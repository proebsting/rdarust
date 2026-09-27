//! The three pipeline stages, argument-compatible with rdapy's scripts.

use std::collections::HashMap;
use std::io::{BufRead, Write};

use anyhow::{anyhow, Context as _, Result};
use rdarust_core::aggregate::{Aggregates, Mode};
use rdarust_core::context::Context;
use rdarust_core::score::{ModeOpt, ScoreOptions};
use rdarust_io::{
    aggregates_from_value, aggregates_to_value, load_graph, load_input_data,
    records::{smart_reader, smart_writer, write_record, write_record_sorted},
    scorecard_to_value, scored_aggregates_to_value, ScoresCsv,
};
use serde_json::{json, Map, Value};

use crate::{DataArgs, ResilienceArgs};

pub fn parse_mode(s: &str) -> Mode {
    match s {
        "general" => Mode::General,
        "partisan" => Mode::Partisan,
        "minority" => Mode::Minority,
        "compactness" => Mode::Compactness,
        "splitting" => Mode::Splitting,
        _ => Mode::All,
    }
}

pub fn build_context(args: &DataArgs) -> Result<Context> {
    if args.graph.is_empty() {
        return Err(anyhow!("--graph is required for this command"));
    }
    let input = load_input_data(&args.data)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading precinct data from {}", args.data))?;
    let graph = load_graph(&args.graph)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading adjacency graph from {}", args.graph))?;
    input
        .with_graph(graph)
        .into_context(&args.state, &args.plan_type, args.districts_override)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("building a context for {} {}", args.state, args.plan_type))
}

/// Whole seats per election from a precomputed geographic baseline.
pub fn load_geographic_baselines(path: Option<&str>) -> Result<HashMap<String, f64>> {
    let Some(path) = path else {
        return Ok(HashMap::new());
    };
    let text = std::fs::read_to_string(rdarust_io::records::expand(path))
        .with_context(|| format!("reading precomputed values from {path}"))?;
    let v: Value = serde_json::from_str(&text)
        .with_context(|| format!("parsing {path}"))?;
    Ok(v.get("geographic_baseline")
        .and_then(|b| b.as_object())
        .map(|b| {
            b.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.get("whole_seats")?.as_f64()?)))
                .collect()
        })
        .unwrap_or_default())
}

/// A plan taken from an input record.
struct PlanRecord {
    name: String,
    /// The assignment map exactly as it arrived, so it round-trips unchanged.
    plan: Value,
}

/// Recognise a plan in an input record.
///
/// Plans arrive either tagged, or as a bare geoid-to-district object, which
/// is what the ensemble tools emit. `index` names the untagged ones.
fn as_plan(record: &Value, index: usize) -> Option<PlanRecord> {
    match record.get("_tag_").and_then(|t| t.as_str()) {
        Some("plan") => Some(PlanRecord {
            // Canonical records name plans with an integer sample number, so
            // a name is not necessarily a string.
            name: plan_name(record.get("name"))?,
            plan: record.get("plan")?.clone(),
        }),
        Some(_) => None,
        None => {
            // A flat object of scalars is an untagged plan.
            let obj = record.as_object()?;
            obj.values()
                .all(|v| v.is_number() || v.is_string())
                .then(|| PlanRecord {
                    name: format!("{index:06}"),
                    plan: record.clone(),
                })
        }
    }
}

/// A plan's name, which may arrive as a string or a number.
fn plan_name(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn assignments_of(plan: &Value) -> Vec<(String, u32)> {
    plan.as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| {
                    let d = v.as_u64().or_else(|| v.as_str()?.parse().ok())?;
                    Some((k.clone(), d as u32))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_metadata(record: &Value) -> bool {
    record.get("_tag_").and_then(|t| t.as_str()) == Some("metadata")
}

/// Run `f` over every record, handling metadata passthrough and errors.
fn for_each_record(
    input: Option<&str>,
    out: &mut dyn Write,
    resilience: &ResilienceArgs,
    mut f: impl FnMut(&Value, usize, &mut dyn Write) -> Result<()>,
) -> Result<()> {
    let reader = smart_reader(input).with_context(|| {
        format!("opening {}", input.unwrap_or("stdin"))
    })?;
    let mut skipped = 0usize;
    let mut index = 0usize;

    for (n, line) in reader.lines().enumerate() {
        let line = line.context("reading input")?;
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                if resilience.continue_on_error {
                    eprintln!("rdarust: line {}: invalid JSON: {e}", n + 1);
                    skipped += 1;
                    continue;
                }
                return Err(anyhow!("line {}: invalid JSON: {e}", n + 1));
            }
        };

        if is_metadata(&record) {
            write_record(out, &record).context("writing metadata")?;
            continue;
        }

        match f(&record, index, out) {
            Ok(()) => index += 1,
            Err(e) => {
                if resilience.continue_on_error {
                    eprintln!("rdarust: line {}: {e:#}", n + 1);
                    skipped += 1;
                    continue;
                }
                return Err(e.context(format!("line {}", n + 1)));
            }
        }
    }

    if skipped > 0 {
        eprintln!("rdarust: skipped {skipped} record(s)");
    }
    out.flush().context("flushing output")?;
    Ok(())
}

pub fn aggregate(
    args: &DataArgs,
    resilience: &ResilienceArgs,
    input: Option<&str>,
    output: Option<&str>,
) -> Result<()> {
    let ctx = build_context(args)?;
    let mode = parse_mode(&args.mode);
    let mut aggs = Aggregates::new(&ctx);
    let mut out = smart_writer(output).context("opening output")?;

    for_each_record(input, &mut out, resilience, |record, index, out| {
        let Some(p) = as_plan(record, index) else {
            return Ok(());
        };
        let plan = ctx.plan_from_assignments(assignments_of(&p.plan));
        ctx.aggregate_into(&plan, mode, &mut aggs)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("aggregating plan {}", p.name))?;

        let mut rec = Map::new();
        rec.insert("_tag_".into(), json!("plan"));
        rec.insert("name".into(), json!(p.name));
        rec.insert("plan".into(), p.plan.clone());
        rec.insert("aggregates".into(), aggregates_to_value(&aggs, &ctx, mode));
        write_record(out, &Value::Object(rec)).context("writing aggregates")?;
        Ok(())
    })
}

pub fn score(
    args: &DataArgs,
    resilience: &ResilienceArgs,
    precomputed: Option<&str>,
    input: Option<&str>,
    output: Option<&str>,
    reverse_weight_splitting: bool,
) -> Result<()> {
    let ctx = build_context(args)?;
    let mode = parse_mode(&args.mode);
    let opts = ScoreOptions {
        mode: ModeOpt(mode),
        mmd_scoring: true,
        reverse_weight_splitting,
        geographic_baselines: load_geographic_baselines(precomputed)?,
    };
    let mut aggs = Aggregates::new(&ctx);
    let mut out = smart_writer(output).context("opening output")?;

    for_each_record(input, &mut out, resilience, |record, index, out| {
        let Some(p) = as_plan(record, index) else {
            return Ok(());
        };
        let stored = record.get("aggregates").ok_or_else(|| {
            anyhow!("plan {} carries no aggregates; run `rdarust aggregate` first", p.name)
        })?;
        aggregates_from_value(stored, &ctx, &mut aggs)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("reading aggregates for plan {}", p.name))?;

        let plan = ctx.plan_from_assignments(assignments_of(&p.plan));
        let card = ctx
            .score_aggregates(&plan, &opts, &mut aggs)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("scoring plan {}", p.name))?;

        let mut rec = Map::new();
        rec.insert("_tag_".into(), json!("scores"));
        rec.insert("name".into(), json!(p.name));
        rec.insert("scores".into(), scorecard_to_value(&card, &ctx.keys, mode));
        rec.insert("aggregates".into(), scored_aggregates_to_value(&aggs, &ctx, mode));
        write_record(out, &Value::Object(rec)).context("writing scores")?;
        Ok(())
    })
}

pub fn write(
    input: Option<&str>,
    data: &str,
    scores_path: &str,
    by_district_path: &str,
    prefixes: bool,
) -> Result<()> {
    // Only the metadata is needed here, but it arrives with the precinct data.
    let data_map = load_input_data(data)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {data}"))?
        .metadata;

    let reader = smart_reader(input).context("opening input")?;
    let mut csv = ScoresCsv::new(
        std::fs::File::create(rdarust_io::records::expand(scores_path))
            .with_context(|| format!("creating {scores_path}"))?,
    );
    let mut by_district = smart_writer(Some(by_district_path))
        .with_context(|| format!("creating {by_district_path}"))?;

    for (n, line) in reader.lines().enumerate() {
        let line = line.context("reading input")?;
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(&line)
            .with_context(|| format!("line {}: invalid JSON", n + 1))?;

        if is_metadata(&record) {
            write_scores_metadata(scores_path, &record, &data_map)?;
            continue;
        }
        let name = record
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("line {}: record has no name", n + 1))?;
        let scores = record
            .get("scores")
            .ok_or_else(|| anyhow!("line {}: record has no scores", n + 1))?;

        csv.write(name, scores, prefixes)
            .with_context(|| format!("writing scores for {name}"))?;

        let mut agg_rec = Map::new();
        agg_rec.insert("_tag_".into(), json!("by-district"));
        agg_rec.insert("name".into(), json!(name));
        agg_rec.insert(
            "by-district".into(),
            record.get("aggregates").cloned().unwrap_or(Value::Null),
        );
        write_record_sorted(&mut by_district, &Value::Object(agg_rec))
            .context("writing by-district aggregates")?;
    }

    csv.flush().context("flushing scores")?;
    by_district.flush().context("flushing by-district")?;
    Ok(())
}

/// Metadata about the run, written beside the scores CSV.
pub fn write_scores_metadata_pub(p: &str, r: &Value, d: &Value) -> Result<()> {
    write_scores_metadata(p, r, d)
}

fn write_scores_metadata(scores_path: &str, record: &Value, data_map: &Value) -> Result<()> {
    let path = scores_path.replace(".csv", "_metadata.json");
    let mut merged = record
        .get("properties")
        .and_then(|p| p.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(dm) = data_map.as_object() {
        for (k, v) in dm {
            merged.insert(k.clone(), v.clone());
        }
    }
    // rdapy writes this with a four-space indent and no trailing newline.
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    serde::Serialize::serialize(&Value::Object(merged), &mut ser)?;
    std::fs::write(rdarust_io::records::expand(&path), buf)
        .with_context(|| format!("writing {path}"))?;
    Ok(())
}
