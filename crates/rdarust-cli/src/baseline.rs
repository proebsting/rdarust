//! The geographic baseline: how many seats geography alone would give.
//!
//! Two steps, both once per state. `find-neighborhoods` grows a
//! district-sized neighbourhood around every precinct, which depends only on
//! the map and is the expensive part. `precompute-baselines` then asks how
//! each neighbourhood votes, for every election.
//!
//! Ports `scripts/geographic-baseline/`.

use anyhow::{anyhow, Context as _, Result};
use rayon::prelude::*;
use rdarust_core::context::Context;
use rdarust_io::{
    load_graph, load_input_data,
    neighborhoods::{read_neighborhoods, write_neighborhoods},
    records::{expand, smart_reader, smart_writer, write_record},
};
use serde_json::{json, Map, Value};

use crate::DataArgs;

/// Build a context, with the graph optional.
///
/// Evaluating neighbourhoods needs no adjacency -- only growing them does --
/// so the graph can be left out of the second step.
fn context(args: &DataArgs, graph: Option<&str>) -> Result<Context> {
    let mut input = load_input_data(&args.data)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading precinct data from {}", args.data))?;
    if let Some(path) = graph {
        input = input.with_graph(
            load_graph(path)
                .map_err(|e| anyhow!("{e}"))
                .with_context(|| format!("reading adjacency graph from {path}"))?,
        );
    }
    input
        .into_context(&args.state, &args.plan_type, args.districts_override)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("building a context for {} {}", args.state, args.plan_type))
}

pub fn find_neighborhoods(
    args: &DataArgs,
    output: Option<&str>,
    size: f64,
    jobs: Option<usize>,
) -> Result<()> {
    if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .context("setting up the thread pool")?;
    }

    let ctx = context(args, Some(&args.graph))?;
    let target = ctx.neighborhood_target_pop(size);
    let order = ctx.sorted_precinct_order();

    eprintln!(
        "rdarust: growing {} neighbourhoods of about {target} people",
        order.len()
    );

    // Each neighbourhood is independent of the others, and the results are
    // collected in order, so the output does not depend on the thread count.
    let neighborhoods: Vec<(u32, Vec<u32>)> = order
        .par_iter()
        .map(|&seed| (seed, ctx.make_neighborhood(seed, target)))
        .collect();

    let mut out = smart_writer(output).context("opening output")?;
    write_neighborhoods(&mut out, &ctx, &neighborhoods).context("writing neighbourhoods")?;
    out.flush()?;
    Ok(())
}

pub fn precompute_baselines(
    args: &DataArgs,
    neighborhoods_path: Option<&str>,
    output: Option<&str>,
) -> Result<()> {
    // The graph is only needed if it was supplied; neighbourhoods already
    // encode the adjacency that produced them.
    let graph = (!args.graph.is_empty()).then_some(args.graph.as_str());
    let ctx = context(args, graph)?;

    let reader = smart_reader(neighborhoods_path).context("opening the neighbourhoods")?;
    let neighborhoods = read_neighborhoods(reader, &ctx)
        .map_err(|e| anyhow!("{e}"))
        .context("reading neighbourhoods")?;
    eprintln!("rdarust: read {} neighbourhoods", neighborhoods.len());

    let mut by_dataset = Map::new();
    for (i, election) in ctx.elections.iter().enumerate() {
        let b = ctx.geographic_baseline(&neighborhoods, i);
        let mut entry = Map::new();
        entry.insert("fractional_seats".into(), json!(b.fractional_seats));
        entry.insert("whole_seats".into(), json!(b.whole_seats));
        by_dataset.insert(election.key.clone(), Value::Object(entry));
    }

    let mut record = Map::new();
    record.insert("geographic_baseline".into(), Value::Object(by_dataset));

    let mut out = smart_writer(output).context("opening output")?;
    write_record(&mut out, &Value::Object(record))?;
    out.flush()?;
    Ok(())
}

/// Check stored neighbourhoods against the state they claim to describe.
///
/// Ports `check_neighborhoods.py`. Every record is verified on read -- its
/// size, its checksum, and that a precinct is in its own neighbourhood -- so
/// this reports what those checks found.
pub fn check_neighborhoods(args: &DataArgs, neighborhoods_path: Option<&str>) -> Result<()> {
    let graph = (!args.graph.is_empty()).then_some(args.graph.as_str());
    let ctx = context(args, graph)?;

    let reader = smart_reader(neighborhoods_path).context("opening the neighbourhoods")?;
    let neighborhoods = read_neighborhoods(reader, &ctx)
        .map_err(|e| anyhow!("{e}"))
        .context("reading neighbourhoods")?;

    let expected = ctx.sorted_precinct_order().len();
    if neighborhoods.len() != expected {
        return Err(anyhow!(
            "{} neighbourhoods for {expected} precincts",
            neighborhoods.len()
        ));
    }

    let sizes: Vec<usize> = neighborhoods.iter().map(|(_, m)| m.len()).collect();
    let pops: Vec<i64> = neighborhoods
        .iter()
        .map(|(_, m)| m.iter().map(|&i| ctx.pop[i as usize]).sum())
        .collect();
    let target = ctx.neighborhood_target_pop(1.0);

    let mean = |v: &[i64]| v.iter().sum::<i64>() as f64 / v.len() as f64;
    eprintln!(
        "rdarust: {} neighbourhoods, {}-{} precincts each (mean {:.1})",
        neighborhoods.len(),
        sizes.iter().min().unwrap_or(&0),
        sizes.iter().max().unwrap_or(&0),
        sizes.iter().sum::<usize>() as f64 / sizes.len() as f64
    );
    eprintln!(
        "rdarust: population {}-{} against a target of {target} (mean {:.0}, {:+.2}%)",
        pops.iter().min().unwrap_or(&0),
        pops.iter().max().unwrap_or(&0),
        mean(&pops),
        100.0 * (mean(&pops) - target as f64) / target as f64
    );
    println!("ok");
    Ok(())
}

/// Report a precomputed baseline against the number of districts.
///
/// Ports `report_baselines.py`.
pub fn report_baselines(path: &str, n_districts: Option<u32>) -> Result<()> {
    let text = std::fs::read_to_string(expand(path))
        .with_context(|| format!("reading {path}"))?;
    let doc: Value = serde_json::from_str(&text).with_context(|| format!("parsing {path}"))?;
    let baselines = doc
        .get("geographic_baseline")
        .and_then(|b| b.as_object())
        .ok_or_else(|| anyhow!("{path} has no geographic_baseline"))?;

    if n_districts.is_some() {
        println!("{:<20} {:>10} {:>10} {:>10}", "election", "fractional", "whole", "of");
    } else {
        println!("{:<20} {:>10} {:>10}", "election", "fractional", "whole");
    }
    for (election, v) in baselines {
        let f = v.get("fractional_seats").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let w = v.get("whole_seats").and_then(|x| x.as_f64()).unwrap_or(0.0);
        match n_districts {
            Some(n) => println!("{election:<20} {f:>10.4} {w:>10.4} {n:>10}"),
            None => println!("{election:<20} {f:>10.4} {w:>10.4}"),
        }
    }
    Ok(())
}
