//! Reading rdapy's input-data JSONL into a [`Context`].
//!
//! The format is one JSON object per line, tagged `metadata` or `precinct`.
//! The metadata record maps logical field names -- `total_pop`, `dem_votes`
//! -- onto the column names a particular dataset uses, so the same reader
//! handles data assembled from different sources.
//!
//! Field *order* in that mapping is load-bearing: it fixes the order minority
//! opportunity is summed in, and floating-point addition is not associative.
//! `serde_json`'s `preserve_order` feature keeps it.
//!
//! # Paths and readers
//!
//! Every reader here comes in two forms. The bare name -- [`load_input_data`],
//! [`load_graph`], [`load_geojson`], [`read_plan_csv`], [`read_plan_jsonl`],
//! [`read_plans_jsonl`] -- opens a path. The `_from` variant takes an already
//! open reader and holds all the logic; the path form is a single
//! `File::open` above it.
//!
//! So a caller that has bytes rather than a filename -- a fetched file, a
//! decompressed stream, a browser upload, an embedder driving this as a
//! library -- can use the `_from` variants and never touch the filesystem.
//! [`records::smart_reader`] and [`records::smart_writer`] remain the
//! command line's edge, where `-` means the standard streams.
//! [`geojson::features_of`] goes one step further and takes an
//! already-parsed document.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use rdarust_core::context::{
    Context, ContextError, DatasetKeys, Demographics, Election, PrecinctInput, OUT_OF_STATE,
};
use serde_json::{Map, Value};

pub mod aggregates;
pub mod extract;
pub mod geojson;
pub mod neighborhoods;
pub mod plans;
pub mod records;
pub mod scores;

pub use aggregates::{aggregates_from_value, aggregates_to_value, scored_aggregates_to_value};
pub use plans::{
    read_plan_csv, read_plan_csv_from, read_plan_jsonl, read_plan_jsonl_from, read_plans_jsonl,
    read_plans_jsonl_from, Assignments,
};
pub use extract::{extract_data, extract_graph};
pub use geojson::{features_of, load_geojson, load_geojson_from};
pub use neighborhoods::{read_neighborhoods_from, write_neighborhoods};
pub use records::{smart_reader, smart_writer, write_record};
pub use scores::{flatten_scores, format_score, scorecard_to_value, ScoresCsv};

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json { line: usize, source: serde_json::Error },
    /// A record was structurally wrong.
    Malformed { line: usize, what: String },
    /// The metadata record was missing or came after the first precinct.
    MissingMetadata,
    /// A precinct lacked a field the metadata promised.
    MissingField { geoid: String, field: String },
    Context(ContextError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::Json { line, source } => write!(f, "line {line}: {source}"),
            LoadError::Malformed { line, what } => write!(f, "line {line}: {what}"),
            LoadError::MissingMetadata => write!(f, "no metadata record before the first precinct"),
            LoadError::MissingField { geoid, field } => {
                write!(f, "precinct {geoid} has no field {field}")
            }
            LoadError::Context(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}
impl From<ContextError> for LoadError {
    fn from(e: ContextError) -> Self {
        LoadError::Context(e)
    }
}

/// The dataset key for a type, with rdapy's legacy fallbacks.
///
/// An empty dataset name means "the only one", which becomes `default` --
/// except for CVAP, whose absence is the sentinel `N/A`.
fn get_dataset(meta: &Map<String, Value>, ty: &str) -> String {
    let fallback = if ty == "cvap" { "N/A" } else { "default" };
    meta.get(ty)
        .and_then(|m| m.get("datasets"))
        .and_then(|d| d.as_array())
        .and_then(|d| d.first())
        .and_then(|d| d.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Is this the old metadata layout, where column names are spelled out
/// rather than prefixed with the dataset?
///
/// rdapy's test is `"version" not in metadata or metadata["version"] == "1"`.
/// That comparison is against the *string* `"1"`, so a numeric version --
/// including a numeric `1` -- is not legacy. Reproduced exactly: reading a
/// current file as legacy looks for unprefixed columns that do not exist.
fn is_legacy(meta: &Map<String, Value>) -> bool {
    match meta.get("version") {
        None => true,
        Some(Value::String(v)) => v == "1",
        Some(_) => false,
    }
}

/// Every dataset of a type. Legacy metadata names only one.
fn get_datasets(meta: &Map<String, Value>, ty: &str) -> Vec<String> {
    if is_legacy(meta) {
        return vec![get_dataset(meta, ty)];
    }
    meta.get(ty)
        .and_then(|m| m.get("datasets"))
        .and_then(|d| d.as_array())
        .map(|d| {
            d.iter()
                .filter_map(|x| x.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The field mapping for a type, in metadata order.
///
/// Legacy metadata spells column names out; newer metadata prefixes them with
/// the dataset name.
fn get_fields(meta: &Map<String, Value>, ty: &str, dataset: &str) -> Vec<(String, String)> {
    let Some(fields) = meta.get(ty).and_then(|m| m.get("fields")).and_then(|f| f.as_object())
    else {
        return Vec::new();
    };
    let legacy = is_legacy(meta);
    fields
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .map(|(k, v)| {
            let column = if legacy { v } else { format!("{dataset}_{v}") };
            (k, column)
        })
        .collect()
}

/// rdapy reads every count through `int(...)`, which truncates. Values arrive
/// as JSON numbers that may be integral or not.
fn as_count(v: &Value) -> Option<i64> {
    if let Some(i) = v.as_i64() {
        return Some(i);
    }
    v.as_f64().map(|f| f.trunc() as i64)
}

/// Data loaded from one input file, ready to become a [`Context`].
pub struct InputData {
    pub keys: DatasetKeys,
    pub precincts: Vec<PrecinctInput>,
    pub border_neighbors: Option<Vec<String>>,
    pub elections: Vec<Election>,
    pub vap: Demographics,
    pub cvap: Option<Demographics>,
    /// The raw metadata record, passed through by the CLI.
    pub metadata: Value,
}

impl InputData {
    /// Build a scoring context for a state and chamber.
    pub fn into_context(
        self,
        xx: &str,
        plan_type: &str,
        districts_override: Option<u32>,
    ) -> Result<Context, LoadError> {
        Ok(Context::new(
            xx,
            plan_type,
            self.precincts,
            self.border_neighbors,
            self.elections,
            self.vap,
            self.cvap,
            self.keys,
            districts_override,
        )?)
    }
}

/// Read an input-data JSONL file.
pub fn load_input_data(path: impl AsRef<Path>) -> Result<InputData, LoadError> {
    load_input_data_from(BufReader::new(File::open(path.as_ref())?))
}

/// Read input data from an open reader.
pub fn load_input_data_from(reader: impl BufRead) -> Result<InputData, LoadError> {
    let mut metadata: Option<Value> = None;
    let mut keys: Option<DatasetKeys> = None;
    let mut election_fields: Vec<(String, String, String)> = Vec::new();
    let mut vap_fields: Vec<(String, String)> = Vec::new();
    let mut cvap_fields: Vec<(String, String)> = Vec::new();
    let mut pop_field = String::new();

    let mut precincts: Vec<PrecinctInput> = Vec::new();
    let mut border_neighbors: Option<Vec<String>> = None;
    let mut dem: Vec<Vec<i64>> = Vec::new();
    let mut rep: Vec<Vec<i64>> = Vec::new();
    let mut vap_counts: Vec<Vec<i64>> = Vec::new();
    let mut cvap_counts: Vec<Vec<i64>> = Vec::new();

    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: Value = serde_json::from_str(&line)
            .map_err(|source| LoadError::Json { line: n + 1, source })?;

        match rec.get("_tag_").and_then(|t| t.as_str()) {
            Some("metadata") => {
                let props = rec.get("properties").and_then(|p| p.as_object()).ok_or(
                    LoadError::Malformed {
                        line: n + 1,
                        what: "metadata record has no properties object".into(),
                    },
                )?;

                let census = get_dataset(props, "census");
                let vap_key = get_dataset(props, "vap");
                let cvap_key = get_dataset(props, "cvap");
                let shapes = get_dataset(props, "shapes");
                let elections = get_datasets(props, "election");

                pop_field = get_fields(props, "census", &census)
                    .into_iter()
                    .find(|(k, _)| k == "total_pop")
                    .map(|(_, v)| v)
                    .ok_or(LoadError::Malformed {
                        line: n + 1,
                        what: "metadata does not name a total_pop field".into(),
                    })?;

                for e in &elections {
                    let f = get_fields(props, "election", e);
                    let look = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
                    match (look("dem_votes"), look("rep_votes")) {
                        (Some(d), Some(r)) => election_fields.push((e.clone(), d, r)),
                        _ => {
                            return Err(LoadError::Malformed {
                                line: n + 1,
                                what: format!("election {e} lacks dem_votes or rep_votes"),
                            })
                        }
                    }
                }

                vap_fields = get_fields(props, "vap", &vap_key);
                if cvap_key != "N/A" {
                    cvap_fields = get_fields(props, "cvap", &cvap_key);
                }

                dem = vec![Vec::new(); election_fields.len()];
                rep = vec![Vec::new(); election_fields.len()];
                vap_counts = vec![Vec::new(); vap_fields.len()];
                cvap_counts = vec![Vec::new(); cvap_fields.len()];

                keys = Some(DatasetKeys {
                    census,
                    vap: vap_key,
                    cvap: (cvap_key != "N/A").then_some(cvap_key),
                    elections,
                    shapes,
                });
                metadata = Some(rec.get("properties").cloned().unwrap_or(Value::Null));
            }

            Some("precinct") => {
                if keys.is_none() {
                    return Err(LoadError::MissingMetadata);
                }
                let data = rec.get("data").and_then(|d| d.as_object()).ok_or(
                    LoadError::Malformed {
                        line: n + 1,
                        what: "precinct record has no data object".into(),
                    },
                )?;
                let geoid = data
                    .get("geoid")
                    .and_then(|g| g.as_str())
                    .ok_or(LoadError::Malformed {
                        line: n + 1,
                        what: "precinct record has no geoid".into(),
                    })?
                    .to_string();

                let neighbors: Vec<String> = data
                    .get("neighbors")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();

                // The border node carries nothing but its neighbour list.
                if geoid == OUT_OF_STATE {
                    border_neighbors = Some(neighbors);
                    continue;
                }

                let field = |name: &str| -> Result<i64, LoadError> {
                    data.get(name).and_then(as_count).ok_or_else(|| LoadError::MissingField {
                        geoid: geoid.clone(),
                        field: name.to_string(),
                    })
                };

                for (i, (_, d, r)) in election_fields.iter().enumerate() {
                    dem[i].push(field(d)?);
                    rep[i].push(field(r)?);
                }
                for (i, (_, column)) in vap_fields.iter().enumerate() {
                    vap_counts[i].push(field(column)?);
                }
                for (i, (_, column)) in cvap_fields.iter().enumerate() {
                    cvap_counts[i].push(field(column)?);
                }

                let pairs = |key: &str| -> Vec<(f64, f64)> {
                    data.get(key)
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|p| {
                                    Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let center = data
                    .get("center")
                    .and_then(|v| v.as_array())
                    .and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)))
                    .unwrap_or((0.0, 0.0));
                let arcs: Vec<(String, f64)> = data
                    .get("arcs")
                    .and_then(|v| v.as_object())
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| v.as_f64().map(|l| (k.clone(), l)))
                            .collect()
                    })
                    .unwrap_or_default();

                precincts.push(PrecinctInput {
                    pop: field(&pop_field)?,
                    geoid,
                    center,
                    area: data.get("area").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    arcs,
                    exterior: pairs("exterior"),
                    neighbors,
                });
            }

            // Anything else -- an adjacency graph record, say -- is skipped,
            // as rdapy skips it.
            _ => continue,
        }
    }

    let keys = keys.ok_or(LoadError::MissingMetadata)?;

    let elections = election_fields
        .iter()
        .enumerate()
        .map(|(i, (key, _, _))| Election {
            key: key.clone(),
            dem: std::mem::take(&mut dem[i]),
            rep: std::mem::take(&mut rep[i]),
        })
        .collect();

    Ok(InputData {
        keys,
        precincts,
        border_neighbors,
        elections,
        vap: Demographics {
            names: vap_fields.iter().map(|(k, _)| k.clone()).collect(),
            counts: vap_counts,
        },
        cvap: (!cvap_fields.is_empty()).then(|| Demographics {
            names: cvap_fields.iter().map(|(k, _)| k.clone()).collect(),
            counts: cvap_counts,
        }),
        metadata: metadata.unwrap_or(Value::Null),
    })
}



/// Read an adjacency graph: a JSON object mapping each geoid to its
/// neighbours.
///
/// The scoring pipeline takes the graph from a separate file -- the precinct
/// data carries only geometry and counts -- so this is how the CLI supplies
/// adjacency.
pub fn load_graph(path: impl AsRef<Path>) -> Result<Vec<(String, Vec<String>)>, LoadError> {
    load_graph_from(File::open(path.as_ref())?)
}

/// Read an adjacency graph from an open reader.
pub fn load_graph_from(
    mut reader: impl std::io::Read,
) -> Result<Vec<(String, Vec<String>)>, LoadError> {
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|source| LoadError::Json { line: 0, source })?;
    let obj = v.as_object().ok_or(LoadError::Malformed {
        line: 0,
        what: "graph file is not a JSON object".into(),
    })?;
    Ok(obj
        .iter()
        .map(|(k, v)| {
            let nbrs = v
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            (k.clone(), nbrs)
        })
        .collect())
}

impl InputData {
    /// Replace each precinct's neighbours from a separately loaded graph.
    ///
    /// Input files written for the scoring pipeline leave `neighbors` out,
    /// because the graph is maintained separately and can be corrected
    /// without rewriting the data. Without this the precincts have no
    /// adjacency at all, and every district comes out with no boundary.
    pub fn with_graph(mut self, graph: Vec<(String, Vec<String>)>) -> Self {
        let mut by_geoid: HashMap<String, Vec<String>> = graph.into_iter().collect();
        if let Some(border) = by_geoid.remove(OUT_OF_STATE) {
            self.border_neighbors = Some(border);
        }
        for p in &mut self.precincts {
            if let Some(nbrs) = by_geoid.remove(&p.geoid) {
                p.neighbors = nbrs;
            }
        }
        self
    }
}
