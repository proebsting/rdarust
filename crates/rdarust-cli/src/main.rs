//! `rdarust` -- redistricting analytics on the command line.
//!
//! The three pipeline stages take the same arguments as rdapy's scripts, so
//! an existing `SCORE.sh` works unchanged:
//!
//! ```text
//! rdarust aggregate | rdarust score | rdarust write
//! ```
//!
//! `rdarust score-all` does the same work in one process. The stages exist to
//! be diffed against Python stage by stage; `score-all` is what you run.

mod fused;
mod stages;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "rdarust",
    about = "Redistricting analytics",
    version,
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Where a plan's data comes from. Shared by the stages that need a context.
#[derive(clap::Args, Debug, Clone)]
pub struct DataArgs {
    /// State abbreviation, e.g. NC.
    #[arg(long)]
    pub state: String,
    /// Chamber: congress, upper or lower.
    #[arg(long = "plan-type")]
    pub plan_type: String,
    /// Precinct data, as JSONL.
    #[arg(long)]
    pub data: String,
    /// Adjacency graph, as JSON.
    #[arg(long)]
    pub graph: String,
    /// Score only one family of metrics.
    #[arg(long, default_value = "all",
          value_parser = ["all", "general", "partisan", "minority", "compactness", "splitting"])]
    pub mode: String,
    /// Override the district count. For experiments with multi-member plans.
    #[arg(long = "districts-override")]
    pub districts_override: Option<u32>,
}

/// How to handle a plan that cannot be processed.
#[derive(clap::Args, Debug, Clone)]
pub struct ResilienceArgs {
    /// Log and skip a bad record instead of stopping.
    ///
    /// rdapy always behaves this way, which is why a plan can vanish from the
    /// output without anyone noticing. Off by default; the count of skipped
    /// records is reported at exit.
    #[arg(long = "continue-on-error")]
    pub continue_on_error: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Aggregate data and shapes by district for a stream of plans.
    Aggregate {
        #[command(flatten)]
        data: DataArgs,
        #[command(flatten)]
        resilience: ResilienceArgs,
        /// Input stream of plans. Defaults to stdin.
        #[arg(long)]
        input: Option<String>,
        /// Output stream of plans with aggregates. Defaults to stdout.
        #[arg(long)]
        output: Option<String>,
    },
    /// Score a stream of plans that already carry aggregates.
    Score {
        #[command(flatten)]
        data: DataArgs,
        #[command(flatten)]
        resilience: ResilienceArgs,
        /// Precomputed values, currently the geographic baseline.
        #[arg(long)]
        precomputed: Option<String>,
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        output: Option<String>,
        /// Also report county splitting under reverse weighting.
        #[arg(short = 'r', long = "reverse-weight-splitting")]
        reverse_weight_splitting: bool,
    },
    /// Write scores to CSV and aggregates to JSONL.
    Write {
        #[arg(long)]
        input: Option<String>,
        /// Precinct data, for the metadata written alongside the scores.
        #[arg(long)]
        data: String,
        /// Path for the scores CSV.
        #[arg(long)]
        scores: String,
        /// Path for the by-district aggregates JSONL.
        #[arg(long = "by-district")]
        by_district: String,
        /// Always prefix a metric with its dataset.
        #[arg(long)]
        prefixes: bool,
    },
    /// Aggregate, score and write in one pass.
    ///
    /// Skips serialising the aggregates between stages, which for a large
    /// ensemble is most of the work.
    ScoreAll {
        #[command(flatten)]
        data: DataArgs,
        #[command(flatten)]
        resilience: ResilienceArgs,
        #[arg(long)]
        precomputed: Option<String>,
        /// Input stream of plans. Defaults to stdin.
        #[arg(long)]
        plans: Option<String>,
        #[arg(long)]
        scores: String,
        #[arg(long = "by-district")]
        by_district: String,
        #[arg(long)]
        prefixes: bool,
        #[arg(short = 'r', long = "reverse-weight-splitting")]
        reverse_weight_splitting: bool,
        /// Worker threads. Defaults to one per core.
        #[arg(long)]
        jobs: Option<usize>,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Aggregate { data, resilience, input, output } => {
            stages::aggregate(&data, &resilience, input.as_deref(), output.as_deref())
        }
        Command::Score {
            data, resilience, precomputed, input, output, reverse_weight_splitting,
        } => stages::score(
            &data, &resilience, precomputed.as_deref(),
            input.as_deref(), output.as_deref(), reverse_weight_splitting,
        ),
        Command::Write { input, data, scores, by_district, prefixes } => {
            stages::write(input.as_deref(), &data, &scores, &by_district, prefixes)
        }
        Command::ScoreAll {
            data, resilience, precomputed, plans, scores, by_district, prefixes,
            reverse_weight_splitting, jobs,
        } => fused::score_all(
            &data, &resilience, precomputed.as_deref(), plans.as_deref(),
            &scores, &by_district, prefixes, reverse_weight_splitting, jobs,
        ),
    };

    if let Err(e) = result {
        eprintln!("rdarust: {e:#}");
        std::process::exit(1);
    }
}
