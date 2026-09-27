//! Repairing adjacency graphs.
//!
//! A state's precincts are often not all connected to each other: islands,
//! and precincts reachable only by crossing water. Scoring needs a connected
//! graph, so the gaps are bridged with explicit edges -- "mods" -- which are
//! generated once, reviewed, and then applied.
//!
//! Ports `scripts/graphs/generate_contiguity_mods.py`, `standalone_mods.py`,
//! `apply_contiguity_mods.py` and `check_graph.py`.

use std::collections::HashMap;
use std::io::Write;

use anyhow::{anyhow, Context as _, Result};
use rdarust_core::context::OUT_OF_STATE;
use rdarust_core::graph::{contiguity_mods, is_connected, is_consistent, islands};
use rdarust_io::{
    load_graph, load_input_data,
    records::{expand, smart_writer},
};
use serde_json::{json, Map, Value};

/// A graph with its geoids interned to indices.
struct Interned {
    names: Vec<String>,
    index: HashMap<String, u32>,
    adjacency: Vec<Vec<u32>>,
    out_of_state: Option<u32>,
}

/// Intern a graph, keeping the border node last so precinct indices stay
/// contiguous from zero.
fn intern(graph: &[(String, Vec<String>)]) -> Result<Interned> {
    let mut names: Vec<String> = graph
        .iter()
        .map(|(g, _)| g.clone())
        .filter(|g| g != OUT_OF_STATE)
        .collect();
    names.sort();
    let has_border = graph.iter().any(|(g, _)| g == OUT_OF_STATE);
    if has_border {
        names.push(OUT_OF_STATE.to_string());
    }

    let index: HashMap<String, u32> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i as u32))
        .collect();

    let by_name: HashMap<&str, &Vec<String>> =
        graph.iter().map(|(g, n)| (g.as_str(), n)).collect();

    let adjacency = names
        .iter()
        .map(|n| {
            by_name
                .get(n.as_str())
                .map(|nbrs| {
                    nbrs.iter()
                        .filter_map(|x| index.get(x.as_str()).copied())
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();

    Ok(Interned {
        out_of_state: index.get(OUT_OF_STATE).copied(),
        names,
        index,
        adjacency,
    })
}

/// Precinct centres, from a locations file or straight from a GeoJSON.
fn load_locations(
    locations: Option<&str>,
    geojson: Option<&str>,
    g: &Interned,
) -> Result<Vec<(f64, f64)>> {
    let mut centers = vec![(0.0, 0.0); g.names.len()];
    let mut seen = 0usize;

    if let Some(path) = locations {
        let text = std::fs::read_to_string(expand(path))
            .with_context(|| format!("reading {path}"))?;
        let doc: Value =
            serde_json::from_str(&text).with_context(|| format!("parsing {path}"))?;
        for (geoid, v) in doc.as_object().ok_or_else(|| anyhow!("{path} is not an object"))? {
            if let Some(&i) = g.index.get(geoid.as_str()) {
                let a = v.as_array().ok_or_else(|| anyhow!("{geoid}: not a coordinate pair"))?;
                centers[i as usize] = (
                    a.first().and_then(|x| x.as_f64()).unwrap_or(0.0),
                    a.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0),
                );
                seen += 1;
            }
        }
    } else if let Some(path) = geojson {
        // DRA's label coordinates, which sit inside their precinct.
        let features = rdarust_io::load_geojson(expand(path))
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("reading {path}"))?;
        for f in &features {
            let Some(geoid) = f.properties.get("id").and_then(|g| g.as_str()) else {
                continue;
            };
            if let Some(&i) = g.index.get(geoid) {
                centers[i as usize] = (
                    f.properties.get("labelx").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.properties.get("labely").and_then(|v| v.as_f64()).unwrap_or(0.0),
                );
                seen += 1;
            }
        }
    } else {
        return Err(anyhow!("give either --locations or --geojson"));
    }

    let precincts = g.names.len() - usize::from(g.out_of_state.is_some());
    if seen < precincts {
        eprintln!("rdarust: WARNING: no location for {} precinct(s)", precincts - seen);
    }
    Ok(centers)
}

pub fn generate_mods(
    graph_path: &str,
    locations: Option<&str>,
    geojson: Option<&str>,
    output: Option<&str>,
) -> Result<()> {
    let graph = load_graph(expand(graph_path))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {graph_path}"))?;
    let g = intern(&graph)?;

    let precincts: Vec<u32> = (0..g.names.len() as u32)
        .filter(|&i| Some(i) != g.out_of_state)
        .collect();

    if is_connected(&precincts, &g.adjacency, g.out_of_state) {
        eprintln!("rdarust: graph is already fully connected");
        return Ok(());
    }

    let centers = load_locations(locations, geojson, &g)?;
    let pieces = islands(&precincts, &g.adjacency, g.out_of_state);
    eprintln!(
        "rdarust: {} disconnected pieces ({} precincts)",
        pieces.len(),
        pieces.iter().map(|p| p.precincts()).sum::<usize>()
    );
    for p in &pieces {
        eprintln!(
            "  piece {}: {} precincts, {} on the border",
            p.id,
            p.precincts(),
            p.coastal.len()
        );
    }

    let connections = contiguity_mods(&precincts, &g.adjacency, g.out_of_state, &centers)
        .map_err(|e| anyhow!("{e}"))?;

    let mut out = smart_writer(output).context("opening output")?;
    for c in &connections {
        writeln!(
            out,
            "+,{},{}",
            g.names[c.from as usize], g.names[c.to as usize]
        )?;
    }
    out.flush()?;
    eprintln!("rdarust: proposed {} connection(s)", connections.len());
    Ok(())
}

pub fn apply_mods(graph_path: &str, mods_path: &str, output: Option<&str>) -> Result<()> {
    let graph = load_graph(expand(graph_path))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {graph_path}"))?;
    let mut by_name: Vec<(String, Vec<String>)> = graph;

    let text = std::fs::read_to_string(expand(mods_path))
        .with_context(|| format!("reading {mods_path}"))?;

    let mut position: HashMap<&str, usize> = HashMap::new();
    for (i, (name, _)) in by_name.iter().enumerate() {
        position.insert(name.as_str(), i);
    }
    let position: HashMap<String, usize> =
        position.into_iter().map(|(k, v)| (k.to_string(), v)).collect();

    let mut applied = 0usize;
    for (n, line) in text.lines().enumerate() {
        // Tolerate a UTF-8 BOM on the first line, which spreadsheets add.
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() < 3 {
            return Err(anyhow!("{mods_path} line {}: expected `+,geoid,geoid`", n + 1));
        }
        // rdapy ignores the first column; requiring it to say `+` means a row
        // meant as a removal cannot be silently applied as an addition.
        if !fields[0].is_empty() && fields[0] != "+" {
            return Err(anyhow!(
                "{mods_path} line {}: unsupported operation {:?}; only `+` adds an edge",
                n + 1,
                fields[0]
            ));
        }

        let (a, b) = (fields[1], fields[2]);
        let (&ia, &ib) = match (position.get(a), position.get(b)) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                return Err(anyhow!(
                    "{mods_path} line {}: both nodes must be in the graph ({a}, {b})",
                    n + 1
                ))
            }
        };
        if !by_name[ia].1.iter().any(|x| x == b) {
            by_name[ia].1.push(b.to_string());
        }
        if !by_name[ib].1.iter().any(|x| x == a) {
            by_name[ib].1.push(a.to_string());
        }
        applied += 1;
    }

    let g = intern(&by_name)?;
    let mut good = true;
    if !is_consistent(&g.adjacency) {
        eprintln!("rdarust: WARNING: graph is not consistent");
        good = false;
    }
    let all: Vec<u32> = (0..g.names.len() as u32).collect();
    if !is_connected(&all, &g.adjacency, g.out_of_state) {
        eprintln!("rdarust: WARNING: graph is not fully connected");
        good = false;
    }
    if !good {
        return Err(anyhow!("the modified graph did not pass its checks"));
    }

    let mut obj = Map::new();
    for (name, nbrs) in &by_name {
        obj.insert(name.clone(), json!(nbrs));
    }
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    serde::Serialize::serialize(&Value::Object(obj), &mut ser)?;
    buf.push(b'\n');

    let mut out = smart_writer(output).context("opening output")?;
    out.write_all(&buf)?;
    out.flush()?;
    eprintln!("rdarust: applied {applied} connection(s)");
    Ok(())
}

pub fn check_graph(state: &str, data: &str, graph_path: &str) -> Result<()> {
    let input = load_input_data(expand(data))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {data}"))?;
    let graph = load_graph(expand(graph_path))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {graph_path}"))?;
    let g = intern(&graph)?;

    // Check the precincts the data actually carries, which is what scoring
    // will traverse.
    let mut missing = 0usize;
    let precincts: Vec<u32> = input
        .precincts
        .iter()
        .filter_map(|p| {
            let i = g.index.get(p.geoid.as_str()).copied();
            if i.is_none() {
                missing += 1;
            }
            i
        })
        .collect();
    if missing > 0 {
        eprintln!("rdarust: WARNING: {missing} precinct(s) are not in the graph");
    }

    if is_connected(&precincts, &g.adjacency, g.out_of_state) {
        println!("Graph for {state} is fully connected.");
        Ok(())
    } else {
        let pieces = islands(&precincts, &g.adjacency, g.out_of_state);
        println!("WARNING: Graph for {state} is not fully connected!");
        println!("{} disconnected pieces:", pieces.len());
        for p in &pieces {
            println!(
                "  piece {}: {} precincts, {} on the border",
                p.id,
                p.precincts(),
                p.coastal.len()
            );
        }
        Err(anyhow!("graph is not fully connected"))
    }
}
