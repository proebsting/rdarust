//! Time scoring an ensemble, for comparison against the Python pipeline.
//!
//!   cargo run --release -p rdarust-io --example score_ensemble -- \
//!       <data.jsonl> <plans.jsonl> <XX> <chamber> [graph.json]

use std::time::Instant;

use rdarust_core::aggregate::{Aggregates, Mode};
use rdarust_core::score::{ModeOpt, ScoreOptions};
use rdarust_io::{load_graph, load_input_data, read_plans_jsonl};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: score_ensemble <data.jsonl> <plans.jsonl> <XX> <chamber>");
        std::process::exit(2);
    }
    let (data, plans, xx, chamber) = (&args[1], &args[2], &args[3], &args[4]);

    let t0 = Instant::now();
    let mut input = load_input_data(data).expect("loading input data");
    if let Some(graph) = args.get(5) {
        input = input.with_graph(load_graph(graph).expect("loading graph"));
    }
    let ctx = input
        .into_context(xx, chamber, None)
        .expect("building context");
    let load = t0.elapsed();
    println!(
        "loaded {} precincts, {} districts, {} elections in {:.2}s",
        ctx.n_precincts(),
        ctx.n_districts,
        ctx.elections.len(),
        load.as_secs_f64()
    );

    let opts = ScoreOptions {
        mode: ModeOpt(Mode::All),
        mmd_scoring: true,
        ..Default::default()
    };

    let t1 = Instant::now();
    let ensemble = read_plans_jsonl(plans, None).expect("reading plans");
    println!(
        "read {} plans in {:.2}s",
        ensemble.len(),
        t1.elapsed().as_secs_f64()
    );

    // Interning the assignments is part of scoring a plan that arrived as
    // geoids; an in-process caller holding indices already skips it.
    let t2 = Instant::now();
    let dense: Vec<Vec<u32>> = ensemble
        .iter()
        .map(|a| ctx.plan_from_assignments(a.iter().map(|(g, d)| (g.as_str(), *d))))
        .collect();
    let intern = t2.elapsed();

    // One reusable buffer, as an in-process caller would keep.
    let mut aggs = Aggregates::new(&ctx);
    let mut checksum = 0.0f64;

    let t3 = Instant::now();
    for plan in &dense {
        let card = ctx.score_into(plan, &opts, &mut aggs).expect("scoring");
        // Touch a result so nothing is optimised away.
        checksum += card.shapes.as_ref().map(|s| s.reock).unwrap_or(0.0);
    }
    let elapsed = t3.elapsed();
    let n = dense.len().max(1);

    println!(
        "interned {} plans in {:.3}s ({:.2} ms/plan)",
        dense.len(),
        intern.as_secs_f64(),
        intern.as_secs_f64() * 1000.0 / n as f64
    );
    println!(
        "scored {} plans in {:.3}s ({:.2} ms/plan), checksum {checksum:.4}",
        dense.len(),
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / n as f64
    );
}
