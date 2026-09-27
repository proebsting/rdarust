//! Turning a DRA GeoJSON into the inputs the scorer reads.
//!
//! Ports `scripts/data/extract_data.py` and `scripts/graphs/extract_graph.py`.
//! Between them they do two things: pull the named data columns out of each
//! feature's nested per-dataset properties, and reduce each shape to the four
//! quantities scoring needs -- area, the border shared with each neighbour,
//! the convex hull, and a centre.

use std::collections::HashMap;

use rdarust_geo::{Coverage, Geometry};
use serde_json::{json, Map, Value};

use crate::geojson::Feature;
use crate::LoadError;

/// Below this, a leftover perimeter is rounding error rather than a real
/// state border. rdapy's `OUT_OF_STATE_THRESHOLD`.
pub const OUT_OF_STATE_THRESHOLD: f64 = 1.0e-12;

pub const OUT_OF_STATE: &str = "OUT_OF_STATE";

/// The dataset types, in the order rdapy emits their fields.
const DATASET_TYPES: [&str; 5] = ["census", "vap", "cvap", "election", "shapes"];

/// Demographic fields that a non-Hispanic VAP dataset spells differently.
const NH_SUFFIXED: [&str; 4] = ["black_vap", "asian_vap", "pacific_vap", "native_vap"];

fn field_map(data_map: &Value, ty: &str) -> Vec<(String, String)> {
    data_map
        .get(ty)
        .and_then(|m| m.get("fields"))
        .and_then(|f| f.as_object())
        .map(|f| {
            f.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn datasets_of(data_map: &Value, ty: &str) -> Vec<String> {
    data_map
        .get(ty)
        .and_then(|m| m.get("datasets"))
        .and_then(|d| d.as_array())
        .map(|d| d.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Pull the mapped data columns out of one feature.
///
/// Values live under `properties.datasets[<dataset>][<column>]`; this
/// flattens them to `{dataset}_{column}`. A dataset a feature does not carry
/// is zero-filled with a warning, as rdapy does, rather than failing -- a
/// precinct genuinely can be missing an election.
pub fn abstract_data(feature: &Feature, data_map: &Value, warnings: &mut usize) -> Map<String, Value> {
    let mut out = Map::new();
    let props = feature.properties.get("datasets");

    for ty in DATASET_TYPES {
        for dataset in datasets_of(data_map, ty) {
            if ty == "shapes" {
                out.insert("geometry".into(), feature.raw_geometry.clone());
                continue;
            }

            let mut fields = field_map(data_map, ty);
            // A "non-Hispanic" VAP dataset names its race columns differently.
            if ty == "vap" && dataset.ends_with("VAP_NH") {
                for (k, v) in fields.iter_mut() {
                    if NH_SUFFIXED.contains(&k.as_str()) {
                        v.push_str("Alone");
                    }
                }
            }

            let derived = format!("minority_{ty}");
            for (key, column) in &fields {
                // The minority figure is derived below, not read.
                if *key == derived {
                    continue;
                }
                let qualified = format!("{dataset}_{column}");
                let value = props
                    .and_then(|d| d.get(&dataset))
                    .and_then(|d| d.get(column))
                    .cloned();
                match value {
                    Some(v) => {
                        out.insert(qualified, v);
                    }
                    None => {
                        *warnings += 1;
                        out.insert(qualified, json!(0));
                    }
                }
            }

            // Minority population is everyone who is not white.
            if ty == "vap" || ty == "cvap" {
                let look = |k: &str| {
                    fields
                        .iter()
                        .find(|(key, _)| key == k)
                        .map(|(_, c)| format!("{dataset}_{c}"))
                };
                if let (Some(m), Some(t), Some(w)) =
                    (look(&derived), look(&format!("total_{ty}")), look(&format!("white_{ty}")))
                {
                    let get = |k: &String| out.get(k).cloned().unwrap_or(json!(0));
                    let (tv, wv) = (get(&t), get(&w));
                    // Python subtracts the values as they came: two integers
                    // give an integer, but CVAP figures are fractional after
                    // disaggregation and give a float. Preserving that keeps
                    // the extracted file identical, not merely equivalent.
                    let both_ints = tv.is_i64() || tv.is_u64();
                    let both_ints = both_ints && (wv.is_i64() || wv.is_u64());
                    let minority = tv.as_f64().unwrap_or(0.0) - wv.as_f64().unwrap_or(0.0);
                    out.insert(
                        m,
                        if both_ints { json!(minority as i64) } else { json!(minority) },
                    );
                }
            }
        }
    }
    out
}

/// A precinct's shape, reduced to what scoring needs.
pub struct ShapeAbstract {
    pub area: f64,
    /// Shared border per neighbour, in the neighbour order the graph gives,
    /// with the state border last when there is one.
    pub arcs: Vec<(String, f64)>,
    pub exterior: Vec<[f64; 2]>,
}

/// Reduce one precinct's shape.
///
/// rdapy also computes a centre here -- a centroid, falling back to a
/// representative point when the centroid lies outside the shape -- and then
/// `extract_data.py` overwrites it unconditionally with DRA's label
/// coordinates. That calculation is skipped, since nothing can observe it.
pub fn abstract_shape(
    coverage: &Coverage,
    geometry: &Geometry,
    index: u32,
    neighbors: &[(String, u32)],
) -> ShapeAbstract {
    let mut arcs = Vec::with_capacity(neighbors.len() + 1);
    let mut total_shared = 0.0;

    for (geoid, nb) in neighbors {
        let shared = coverage.shared_border(index, *nb);
        arcs.push((geoid.clone(), shared));
        total_shared += shared;
    }

    let perimeter = geometry.perimeter();
    let remaining = perimeter - total_shared;
    if remaining > OUT_OF_STATE_THRESHOLD {
        arcs.push((OUT_OF_STATE.to_string(), remaining));
    }

    ShapeAbstract {
        area: geometry.area(),
        arcs,
        exterior: geometry.convex_hull(),
    }
}

/// Build the rook adjacency graph, including the virtual state-border node.
///
/// Two precincts are neighbours when they share a boundary segment -- an
/// edge, not merely a corner. A precinct whose perimeter is not fully
/// accounted for by its neighbours also borders the state boundary.
pub fn extract_graph(
    geoids: &[String],
    geometries: &[Geometry],
) -> Vec<(String, Vec<String>)> {
    let coverage = Coverage::build(geometries);

    let mut border_nodes: Vec<String> = Vec::new();
    let mut graph: Vec<(String, Vec<String>)> = Vec::with_capacity(geoids.len() + 1);

    for (i, geoid) in geoids.iter().enumerate() {
        let mut names: Vec<String> = Vec::new();
        let mut total_shared = 0.0;
        for nb in coverage.neighbors(i as u32) {
            names.push(geoids[nb as usize].clone());
            total_shared += coverage.shared_border(i as u32, nb);
        }
        if geometries[i].perimeter() - total_shared > OUT_OF_STATE_THRESHOLD {
            names.push(OUT_OF_STATE.to_string());
            border_nodes.push(geoid.clone());
        }
        graph.push((geoid.clone(), names));
    }

    // rdapy emits the border node first.
    let mut out = vec![(OUT_OF_STATE.to_string(), border_nodes)];
    out.extend(graph);
    out
}

/// Extract precinct records from a GeoJSON, as `extract-data` writes them.
pub fn extract_data(
    features: &[Feature],
    data_map: &Value,
    graph: &[(String, Vec<String>)],
) -> Result<Vec<Value>, LoadError> {
    let geoid_field = data_map
        .get("geoid")
        .and_then(|g| g.as_str())
        .unwrap_or("id");

    let geoids: Vec<String> = features
        .iter()
        .map(|f| {
            f.properties
                .get(geoid_field)
                .and_then(|g| g.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let index: HashMap<&str, u32> = geoids
        .iter()
        .enumerate()
        .map(|(i, g)| (g.as_str(), i as u32))
        .collect();

    let geometries: Vec<Geometry> = features.iter().map(|f| f.geometry.clone()).collect();
    let coverage = Coverage::build(&geometries);

    let by_geoid: HashMap<&str, &Vec<String>> =
        graph.iter().map(|(g, n)| (g.as_str(), n)).collect();

    let mut warnings = 0usize;
    let mut records = Vec::with_capacity(features.len());

    for (i, feature) in features.iter().enumerate() {
        let geoid = &geoids[i];
        let mut data = Map::new();
        data.insert("geoid".into(), json!(geoid));
        for (k, v) in abstract_data(feature, data_map, &mut warnings) {
            data.insert(k, v);
        }

        // Only neighbours the graph lists, in its order, and never the border
        // node -- that arc is the leftover perimeter.
        let neighbors: Vec<(String, u32)> = by_geoid
            .get(geoid.as_str())
            .map(|ns| {
                ns.iter()
                    .filter(|n| n.as_str() != OUT_OF_STATE)
                    .filter_map(|n| index.get(n.as_str()).map(|&j| (n.clone(), j)))
                    .collect()
            })
            .unwrap_or_default();

        let shape = abstract_shape(&coverage, &feature.geometry, i as u32, &neighbors);

        // DRA's label coordinates, which are guaranteed to sit inside the
        // shape, rather than a computed centroid.
        let center = json!([
            feature.properties.get("labelx").and_then(|v| v.as_f64()).unwrap_or(0.0),
            feature.properties.get("labely").and_then(|v| v.as_f64()).unwrap_or(0.0),
        ]);
        data.insert("center".into(), center);
        data.insert("area".into(), json!(shape.area));

        let mut arcs = Map::new();
        for (g, len) in shape.arcs {
            arcs.insert(g, json!(len));
        }
        data.insert("arcs".into(), Value::Object(arcs));
        data.insert(
            "exterior".into(),
            Value::Array(shape.exterior.iter().map(|p| json!([p[0], p[1]])).collect()),
        );

        let mut record = Map::new();
        record.insert("_tag_".into(), json!("precinct"));
        record.insert("data".into(), Value::Object(data));
        records.push(Value::Object(record));
    }

    if warnings > 0 {
        eprintln!("rdarust: {warnings} missing dataset value(s) filled with zero");
    }
    if coverage.overshared_segments() > 0 {
        eprintln!(
            "rdarust: {} boundary segment(s) are shared by more than two precincts; \
             the shapes overlap and shared borders may be wrong",
            coverage.overshared_segments()
        );
    }

    Ok(records)
}

/// The field table `map_scoring_data.py` writes.
///
/// It names, for each kind of data, the logical field names scoring uses and
/// the column each maps to inside a dataset. `DERIVED` marks a figure that is
/// computed rather than read.
fn field_table(ty: &str) -> Vec<(&'static str, &'static str)> {
    match ty {
        "census" => vec![("total_pop", "Total")],
        "vap" => vec![
            ("total_vap", "Total"), ("white_vap", "White"),
            ("hispanic_vap", "Hispanic"), ("black_vap", "Black"),
            ("native_vap", "Native"), ("asian_vap", "Asian"),
            ("pacific_vap", "Pacific"), ("minority_vap", "DERIVED"),
        ],
        "cvap" => vec![
            ("total_cvap", "Total"), ("white_cvap", "White"),
            ("hispanic_cvap", "Hispanic"), ("black_cvap", "Black"),
            ("native_cvap", "Native"), ("asian_cvap", "Asian"),
            ("pacific_cvap", "Pacific"), ("minority_cvap", "DERIVED"),
        ],
        "election" => vec![
            ("tot_votes", "Total"), ("dem_votes", "Dem"), ("rep_votes", "Rep"),
        ],
        "shapes" => vec![("geometry", "geometry")],
        _ => vec![],
    }
}

fn dataset_entry(ty: &str, datasets: &[String]) -> Value {
    let mut fields = Map::new();
    for (k, v) in field_table(ty) {
        fields.insert(k.into(), json!(v));
    }
    let mut entry = Map::new();
    entry.insert("fields".into(), Value::Object(fields));
    entry.insert("datasets".into(), json!(datasets));
    Value::Object(entry)
}

/// Which datasets a data map should name.
pub struct DataMapSpec<'a> {
    pub census: &'a str,
    pub vap: &'a str,
    pub cvap: &'a str,
    /// Election datasets to score. A single `__all__` takes every election
    /// the GeoJSON carries.
    pub elections: &'a [String],
    /// Also score the elections a composite averages, each on its own.
    pub expand_composites: bool,
    /// The GeoJSON version, recorded in the map.
    pub version: Option<&'a str>,
    /// How the GeoJSON is named in the map: its directory and file name.
    pub directory: &'a str,
    pub file: &'a str,
}

/// Build the data map naming which datasets and fields to extract.
///
/// Ports `scripts/data/map_scoring_data.py`. `doc` is the parsed GeoJSON,
/// which is read only for its `datasets` object. Unknown election datasets
/// are reported through `warnings` rather than printed, so a library caller
/// decides how to surface them.
pub fn map_data(
    doc: &Value,
    spec: &DataMapSpec<'_>,
    warnings: &mut Vec<String>,
) -> Result<Value, LoadError> {
    let available = doc
        .get("datasets")
        .and_then(|d| d.as_object())
        .ok_or(LoadError::Malformed {
            line: 0,
            what: "the GeoJSON has no datasets object".into(),
        })?;

    let mut implied: Vec<String> = Vec::new();
    if spec.elections == ["__all__"] {
        implied.extend(available.keys().filter(|k| k.starts_with("E_")).cloned());
    } else {
        implied.extend(spec.elections.iter().cloned());
        for e in spec.elections {
            let Some(entry) = available.get(e) else {
                warnings.push(format!("election dataset {e} is not in the GeoJSON"));
                continue;
            };
            // A composite election can be expanded into the elections it
            // averages, so each can also be scored on its own.
            if spec.expand_composites {
                if let Some(members) = entry.get("members").and_then(|m| m.as_object()) {
                    for v in members.values() {
                        if let Some(name) = v.as_str() {
                            if available.contains_key(name) {
                                implied.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let mut map = Map::new();
    map.insert("version".into(), match spec.version {
        Some(v) => json!(v),
        None => Value::Null,
    });
    map.insert("directory".into(), json!(spec.directory));
    map.insert("file".into(), json!(spec.file));
    map.insert("geoid".into(), json!("id"));
    map.insert("census".into(), dataset_entry("census", &[spec.census.to_string()]));
    map.insert("vap".into(), dataset_entry("vap", &[spec.vap.to_string()]));
    map.insert("cvap".into(), dataset_entry("cvap", &[spec.cvap.to_string()]));
    map.insert("election".into(), dataset_entry("election", &implied));
    map.insert("shapes".into(), dataset_entry("shapes", &["S_20_DRA".to_string()]));
    Ok(Value::Object(map))
}
