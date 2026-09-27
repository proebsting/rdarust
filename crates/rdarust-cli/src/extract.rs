//! Turning a DRA GeoJSON into the scorer's inputs.
//!
//! Ports rdapy's `scripts/graphs/extract_graph.py` and
//! `scripts/data/extract_data.py`, which are run once per state.

use anyhow::{anyhow, Context as _, Result};
use rdarust_core::graph::{is_connected, is_consistent};
use rdarust_geo::Geometry;
use rdarust_io::{
    extract::{extract_data as do_extract, extract_graph as do_graph, OUT_OF_STATE},
    geojson::{load_geojson, Feature},
    records::write_record,
};
use serde_json::{json, Map, Value};

use crate::files::{expand, smart_writer};

fn geoids_of(features: &[Feature], field: &str) -> Vec<String> {
    features
        .iter()
        .map(|f| {
            f.properties
                .get(field)
                .and_then(|g| g.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

/// Write JSON the way rdapy's `write_json` does: four-space indent, no
/// trailing newline.
fn write_json_pretty(path: &str, value: &Value) -> Result<()> {
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    serde::Serialize::serialize(value, &mut ser)?;
    std::fs::write(expand(path), buf).with_context(|| format!("writing {path}"))?;
    Ok(())
}

pub fn extract_graph(
    geojson_path: &str,
    graph_path: &str,
    locations_path: Option<&str>,
    geoid_field: &str,
) -> Result<()> {
    let features = load_geojson(expand(geojson_path))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {geojson_path}"))?;
    let geoids = geoids_of(&features, geoid_field);
    let geometries: Vec<Geometry> = features.iter().map(|f| f.geometry.clone()).collect();

    let graph = do_graph(&geoids, &geometries);

    // Check the graph before writing it. rdapy renames the output when either
    // check fails, so a bad graph cannot be mistaken for a good one.
    let index: std::collections::HashMap<&str, u32> = graph
        .iter()
        .enumerate()
        .map(|(i, (g, _))| (g.as_str(), i as u32))
        .collect();
    let adjacency: Vec<Vec<u32>> = graph
        .iter()
        .map(|(_, ns)| ns.iter().filter_map(|n| index.get(n.as_str()).copied()).collect())
        .collect();

    let mut good = true;
    if !is_consistent(&adjacency) {
        eprintln!("rdarust: WARNING: graph is not consistent");
        good = false;
    }
    let all: Vec<u32> = (0..adjacency.len() as u32).collect();
    if !is_connected(&all, &adjacency, index.get(OUT_OF_STATE).copied()) {
        eprintln!("rdarust: WARNING: graph is not fully connected");
        good = false;
    }

    let out_path = if good {
        graph_path.to_string()
    } else {
        let stem = graph_path.strip_suffix(".json").unwrap_or(graph_path);
        format!("{stem}_NOT_CONNECTED.json")
    };

    let mut obj = Map::new();
    for (geoid, neighbors) in &graph {
        obj.insert(geoid.clone(), json!(neighbors));
    }
    write_json_pretty(&out_path, &Value::Object(obj))?;
    eprintln!("rdarust: wrote {} nodes to {out_path}", graph.len());

    if let Some(path) = locations_path {
        let mut locs = Map::new();
        for f in &features {
            let geoid = f.properties.get(geoid_field).and_then(|g| g.as_str()).unwrap_or("");
            locs.insert(
                geoid.to_string(),
                json!([
                    f.properties.get("labelx").and_then(|v| v.as_f64()),
                    f.properties.get("labely").and_then(|v| v.as_f64()),
                ]),
            );
        }
        write_json_pretty(path, &Value::Object(locs))?;
    }
    Ok(())
}

pub fn extract_data(
    geojson_path: &str,
    data_map_path: &str,
    graph_path: &str,
    output: Option<&str>,
) -> Result<()> {
    let features = load_geojson(expand(geojson_path))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {geojson_path}"))?;

    let data_map: Value = serde_json::from_str(
        &std::fs::read_to_string(expand(data_map_path))
            .with_context(|| format!("reading {data_map_path}"))?,
    )
    .with_context(|| format!("parsing {data_map_path}"))?;

    let graph_json: Value = serde_json::from_str(
        &std::fs::read_to_string(expand(graph_path))
            .with_context(|| format!("reading {graph_path}"))?,
    )
    .with_context(|| format!("parsing {graph_path}"))?;
    let graph: Vec<(String, Vec<String>)> = graph_json
        .as_object()
        .ok_or_else(|| anyhow!("{graph_path} is not a JSON object"))?
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
            )
        })
        .collect();

    let records = do_extract(&features, &data_map, &graph).map_err(|e| anyhow!("{e}"))?;

    let mut out = smart_writer(output).context("opening output")?;
    let mut metadata = Map::new();
    metadata.insert("_tag_".into(), json!("metadata"));
    metadata.insert("properties".into(), data_map);
    write_record(&mut out, &Value::Object(metadata))?;
    for r in &records {
        write_record(&mut out, r)?;
    }
    out.flush()?;
    eprintln!("rdarust: wrote {} precincts", records.len());
    Ok(())
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

/// Write the data map naming which datasets and fields to extract.
///
/// Ports `scripts/data/map_scoring_data.py`. Pass `__all__` as the election
/// list to take every election the GeoJSON carries.
#[allow(clippy::too_many_arguments)]
pub fn map_data(
    geojson_path: &str,
    out_path: &str,
    census: &str,
    vap: &str,
    cvap: &str,
    elections: &[String],
    expand_composites: bool,
    version: Option<&str>,
) -> Result<()> {
    let text = std::fs::read_to_string(expand(geojson_path))
        .with_context(|| format!("reading {geojson_path}"))?;
    let doc: Value = serde_json::from_str(&text)
        .with_context(|| format!("parsing {geojson_path}"))?;
    let available = doc
        .get("datasets")
        .and_then(|d| d.as_object())
        .ok_or_else(|| anyhow!("{geojson_path} has no datasets object"))?;

    let mut implied: Vec<String> = Vec::new();
    if elections == ["__all__"] {
        implied.extend(available.keys().filter(|k| k.starts_with("E_")).cloned());
    } else {
        implied.extend(elections.iter().cloned());
        for e in elections {
            let Some(entry) = available.get(e) else {
                eprintln!("rdarust: WARNING: election dataset {e} is not in the GeoJSON");
                continue;
            };
            // A composite election can be expanded into the elections it
            // averages, so each can also be scored on its own.
            if expand_composites {
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

    let path = std::path::Path::new(geojson_path);
    let mut map = Map::new();
    map.insert("version".into(), match version {
        Some(v) => json!(v),
        None => Value::Null,
    });
    map.insert(
        "directory".into(),
        json!(path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()),
    );
    map.insert(
        "file".into(),
        json!(path.file_name().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()),
    );
    map.insert("geoid".into(), json!("id"));
    map.insert("census".into(), dataset_entry("census", &[census.to_string()]));
    map.insert("vap".into(), dataset_entry("vap", &[vap.to_string()]));
    map.insert("cvap".into(), dataset_entry("cvap", &[cvap.to_string()]));
    map.insert("election".into(), dataset_entry("election", &implied));
    map.insert("shapes".into(), dataset_entry("shapes", &["S_20_DRA".to_string()]));

    write_json_pretty(out_path, &Value::Object(map))?;
    eprintln!("rdarust: wrote a data map with {} election(s)", implied.len());
    Ok(())
}

/// Measure shape-based compactness for a set of district shapes.
///
/// rdapy offers this only as a library call; it is exposed here because the
/// model is otherwise unreachable from a command line. Reock and
/// Polsby-Popper are planar, as DRA reports them; the KIWYSI rank is
/// geodesic, and is most of the cost.
pub fn compactness(geojson_path: &str, output: Option<&str>, kiwysi: bool) -> Result<()> {
    let features = load_geojson(expand(geojson_path))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {geojson_path}"))?;
    let shapes: Vec<Geometry> = features.iter().map(|f| f.geometry.clone()).collect();

    let m = rdarust_geo::shapes::calc_compactness_metrics(&shapes, kiwysi);

    let mut out = Map::new();
    out.insert("avgReock".into(), json!(m.avg_reock));
    out.insert("avgPolsby".into(), json!(m.avg_polsby));
    if let Some(k) = m.avg_kiwysi {
        out.insert("avgKIWYSI".into(), json!(k));
    }
    out.insert(
        "byDistrict".into(),
        Value::Array(
            m.by_district
                .iter()
                .map(|d| {
                    let mut e = Map::new();
                    e.insert("reock".into(), json!(d.reock));
                    e.insert("polsby".into(), json!(d.polsby));
                    if let Some(k) = d.kiwysi_rank {
                        e.insert("kiwysiRank".into(), json!(k));
                    }
                    Value::Object(e)
                })
                .collect(),
        ),
    );

    match output {
        Some(path) => write_json_pretty(path, &Value::Object(out))?,
        None => println!("{}", serde_json::to_string_pretty(&Value::Object(out))?),
    }
    Ok(())
}
