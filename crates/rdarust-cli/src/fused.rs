//! Aggregate, score and write in one pass.
//!
//! The three stages exist so each can be diffed against rdapy's. This is the
//! one to run: it skips serialising the aggregates between stages, and scores
//! plans across all cores.

use std::io::{BufRead, Write};

use anyhow::{anyhow, Context as _, Result};
use rayon::prelude::*;
use rdarust_core::aggregate::Aggregates;
use rdarust_core::score::{ModeOpt, ScoreOptions};
use rdarust_io::{
    records::{smart_reader, smart_writer, write_record_sorted},
    scorecard_to_value, scored_aggregates_to_value, ScoresCsv,
};
use serde_json::{json, Map, Value};

use crate::stages::{build_context, load_geographic_baselines, parse_mode};
use crate::{DataArgs, ResilienceArgs};

/// One plan as read from the input stream.
struct Incoming {
    name: String,
    assignments: Vec<(String, u32)>,
}

#[allow(clippy::too_many_arguments)]
pub fn score_all(
    args: &DataArgs,
    resilience: &ResilienceArgs,
    precomputed: Option<&str>,
    plans: Option<&str>,
    scores_path: &str,
    by_district_path: &str,
    prefixes: bool,
    reverse_weight_splitting: bool,
    jobs: Option<usize>,
) -> Result<()> {
    if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .context("setting up the thread pool")?;
    }

    let ctx = build_context(args)?;
    let mode = parse_mode(&args.mode);
    let opts = ScoreOptions {
        mode: ModeOpt(mode),
        mmd_scoring: true,
        reverse_weight_splitting,
        geographic_baselines: load_geographic_baselines(precomputed)?,
    };

    // Read the whole ensemble first. Plans are small next to the precinct
    // data, and having them all lets the scoring run across cores.
    let mut incoming: Vec<Incoming> = Vec::new();
    let mut metadata: Option<Value> = None;
    let reader = smart_reader(plans).context("opening the plans stream")?;

    for (n, line) in reader.lines().enumerate() {
        let line = line.context("reading plans")?;
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) if resilience.continue_on_error => {
                eprintln!("rdarust: line {}: invalid JSON: {e}", n + 1);
                continue;
            }
            Err(e) => return Err(anyhow!("line {}: invalid JSON: {e}", n + 1)),
        };

        match record.get("_tag_").and_then(|t| t.as_str()) {
            Some("metadata") => {
                metadata = Some(record);
                continue;
            }
            Some("plan") => {
                let name = record
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                incoming.push(Incoming {
                    name,
                    assignments: assignments_of(record.get("plan").unwrap_or(&Value::Null)),
                });
            }
            Some(_) => continue,
            None => {
                // A flat object of scalars is an untagged plan.
                let Some(obj) = record.as_object() else { continue };
                if !obj.values().all(|v| v.is_number() || v.is_string()) {
                    continue;
                }
                let name = format!("{:06}", incoming.len());
                incoming.push(Incoming { name, assignments: assignments_of(&record) });
            }
        }
    }

    // Score in parallel, keeping one aggregate buffer per worker. Results stay
    // in input order, so the output does not depend on the thread count.
    let scored: Vec<Result<(String, Value, Value), String>> = incoming
        .par_iter()
        .map_init(
            || Aggregates::new(&ctx),
            |aggs, plan| {
                let dense = ctx.plan_from_assignments(
                    plan.assignments.iter().map(|(g, d)| (g.as_str(), *d)),
                );
                match ctx.score_into(&dense, &opts, aggs) {
                    Ok(card) => Ok((
                        plan.name.clone(),
                        scorecard_to_value(&card, &ctx.keys, mode),
                        scored_aggregates_to_value(aggs, &ctx, mode),
                    )),
                    Err(e) => Err(format!("plan {}: {e}", plan.name)),
                }
            },
        )
        .collect();

    let mut csv = ScoresCsv::new(
        std::fs::File::create(rdarust_io::records::expand(scores_path))
            .with_context(|| format!("creating {scores_path}"))?,
    );
    let mut by_district = smart_writer(Some(by_district_path))
        .with_context(|| format!("creating {by_district_path}"))?;

    if let Some(meta) = &metadata {
        crate::stages::write_scores_metadata_pub(scores_path, meta, &ctx_metadata(args)?)?;
    }

    let mut skipped = 0usize;
    for result in scored {
        match result {
            Ok((name, scores, aggs)) => {
                csv.write(&name, &scores, prefixes)
                    .with_context(|| format!("writing scores for {name}"))?;
                let mut rec = Map::new();
                rec.insert("_tag_".into(), json!("by-district"));
                rec.insert("name".into(), json!(name));
                rec.insert("by-district".into(), aggs);
                write_record_sorted(&mut by_district, &Value::Object(rec))
                    .context("writing by-district aggregates")?;
            }
            Err(e) => {
                if !resilience.continue_on_error {
                    return Err(anyhow!("{e}"));
                }
                eprintln!("rdarust: {e}");
                skipped += 1;
            }
        }
    }
    if skipped > 0 {
        eprintln!("rdarust: skipped {skipped} plan(s)");
    }

    csv.flush().context("flushing scores")?;
    by_district.flush().context("flushing by-district")?;
    Ok(())
}

fn ctx_metadata(args: &DataArgs) -> Result<Value> {
    Ok(rdarust_io::load_input_data(&args.data)
        .map_err(|e| anyhow!("{e}"))?
        .metadata)
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
