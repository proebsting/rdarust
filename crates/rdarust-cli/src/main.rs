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

mod baseline;
mod extract;
mod formats;
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
    /// Adjacency graph, as JSON. Not needed by the commands that only read
    /// precinct data.
    #[arg(long, default_value = "")]
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
    /// Grow a district-sized neighbourhood around every precinct.
    ///
    /// Depends only on the map, so it is done once per state. The result
    /// feeds `precompute-baselines`.
    FindNeighborhoods {
        #[command(flatten)]
        data: DataArgs,
        /// Where to write the neighbourhoods. Defaults to stdout.
        #[arg(long)]
        output: Option<String>,
        /// Neighbourhood size, as a fraction of one district's population.
        #[arg(long, default_value_t = 1.0)]
        size: f64,
        /// Worker threads. Defaults to one per core.
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Compute the geographic baseline from stored neighbourhoods.
    PrecomputeBaselines {
        #[command(flatten)]
        data: DataArgs,
        /// The neighbourhoods. Defaults to stdin.
        #[arg(long)]
        neighborhoods: Option<String>,
        #[arg(long)]
        output: Option<String>,
    },
    /// Check stored neighbourhoods against the state they describe.
    CheckNeighborhoods {
        #[command(flatten)]
        data: DataArgs,
        #[arg(long)]
        neighborhoods: Option<String>,
    },
    /// Print a precomputed baseline.
    ReportBaselines {
        /// The precomputed JSON.
        #[arg(long)]
        precomputed: String,
        /// Show the district count alongside, for comparison.
        #[arg(long = "districts")]
        n_districts: Option<u32>,
    },
    /// Convert a legacy single-JSON ensemble to tagged JSONL.
    FromJson {
        #[arg(long)]
        input: String,
        #[arg(long)]
        output: Option<String>,
    },
    /// Build an ensemble from precinct-assignment CSVs, one plan per file.
    FromCsvs {
        /// Paths or glob patterns. Quote a pattern to let rdarust expand it,
        /// which avoids the shell's argument limit on large ensembles.
        #[arg(long, num_args = 1.., required = true)]
        files: Vec<String>,
        #[arg(long)]
        output: Option<String>,
        /// Recorded in the ensemble metadata.
        #[arg(long)]
        state: Option<String>,
        #[arg(long = "plan-type")]
        plan_type: Option<String>,
    },
    /// Convert GerryTools canonical output to geoid assignments.
    FromCanonical {
        /// The ReCom graph, which names the precincts canonical records index.
        #[arg(long)]
        graph: String,
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        output: Option<String>,
        /// The graph node property holding the geoid.
        #[arg(long, default_value = "GEOID")]
        geoid: String,
    },
    /// Keep every kth line, for subsampling an ensemble.
    Sample {
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        output: Option<String>,
        /// Sample rate.
        #[arg(short = 'k', long, default_value_t = 100)]
        rate: usize,
    },
    /// Measure compactness from district shapes.
    ///
    /// Works from the shapes themselves, unlike `score-all`, which computes
    /// Reock and Polsby-Popper from aggregated area, perimeter and diameter.
    /// This is the only way to get the KIWYSI rank.
    Compactness {
        /// A GeoJSON of district shapes.
        #[arg(long)]
        geojson: String,
        /// Where to write the result. Defaults to stdout.
        #[arg(long)]
        output: Option<String>,
        /// Skip the KIWYSI rank, which is most of the cost.
        #[arg(long = "no-kiwysi")]
        no_kiwysi: bool,
    },
    /// Write the data map naming which datasets and fields to extract.
    MapData {
        #[arg(long)]
        geojson: String,
        /// Where to write the data map.
        #[arg(long = "data-map")]
        data_map: String,
        #[arg(long, default_value = "T_20_CENS")]
        census: String,
        #[arg(long, default_value = "V_20_VAP")]
        vap: String,
        #[arg(long, default_value = "V_20_CVAP")]
        cvap: String,
        /// Elections to score, comma separated. `__all__` takes every one the
        /// GeoJSON carries.
        #[arg(long, default_value = "E_16-20_COMP", value_delimiter = ',')]
        elections: Vec<String>,
        /// Also score the elections a composite averages, individually.
        #[arg(short = 'x', long = "expand-composites")]
        expand_composites: bool,
        /// The GeoJSON version, recorded in the map.
        #[arg(long)]
        version: Option<String>,
    },
    /// Build the adjacency graph from a DRA GeoJSON.
    ExtractGraph {
        /// The GeoJSON to read.
        #[arg(long)]
        geojson: String,
        /// Where to write the graph. Gains a _NOT_CONNECTED suffix if the
        /// graph turns out inconsistent or disconnected.
        #[arg(long)]
        graph: String,
        /// Also write each precinct's label coordinates here.
        #[arg(long)]
        locations: Option<String>,
        /// Property holding the geoid.
        #[arg(long = "geoid-field", default_value = "id")]
        geoid_field: String,
    },
    /// Extract precinct data and shape summaries from a DRA GeoJSON.
    ExtractData {
        #[arg(long)]
        geojson: String,
        /// The data map naming which datasets and fields to pull.
        #[arg(long = "data-map")]
        data_map: String,
        #[arg(long)]
        graph: String,
        /// Where to write the precinct JSONL. Defaults to stdout.
        #[arg(long)]
        data: Option<String>,
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
        Command::FindNeighborhoods { data, output, size, jobs } => {
            baseline::find_neighborhoods(&data, output.as_deref(), size, jobs)
        }
        Command::PrecomputeBaselines { data, neighborhoods, output } => {
            baseline::precompute_baselines(&data, neighborhoods.as_deref(), output.as_deref())
        }
        Command::CheckNeighborhoods { data, neighborhoods } => {
            baseline::check_neighborhoods(&data, neighborhoods.as_deref())
        }
        Command::ReportBaselines { precomputed, n_districts } => {
            baseline::report_baselines(&precomputed, n_districts)
        }
        Command::FromJson { input, output } => {
            formats::from_json(&input, output.as_deref())
        }
        Command::FromCsvs { files, output, state, plan_type } => formats::from_csvs(
            &files, output.as_deref(), state.as_deref(), plan_type.as_deref(),
        ),
        Command::FromCanonical { graph, input, output, geoid } => {
            formats::from_canonical(&graph, input.as_deref(), output.as_deref(), &geoid)
        }
        Command::Sample { input, output, rate } => {
            formats::sample(input.as_deref(), output.as_deref(), rate)
        }
        Command::Compactness { geojson, output, no_kiwysi } => {
            extract::compactness(&geojson, output.as_deref(), !no_kiwysi)
        }
        Command::MapData {
            geojson, data_map, census, vap, cvap, elections, expand_composites, version,
        } => extract::map_data(
            &geojson, &data_map, &census, &vap, &cvap, &elections,
            expand_composites, version.as_deref(),
        ),
        Command::ExtractGraph { geojson, graph, locations, geoid_field } => {
            extract::extract_graph(&geojson, &graph, locations.as_deref(), &geoid_field)
        }
        Command::ExtractData { geojson, data_map, graph, data } => {
            extract::extract_data(&geojson, &data_map, &graph, data.as_deref())
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
