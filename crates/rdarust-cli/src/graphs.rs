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

/// Write the dual graph GerryChain's ReCom consumes.
///
/// ReCom needs a connected dual graph with a population figure on every node;
/// that is the whole of its requirement, and everything else here is to make
/// the result usable afterwards. The geoid lets a plan that comes back as
/// node indices be mapped to precincts, and the county code is there for
/// region-aware ReCom.
///
/// rdapy keeps adjacency and precinct data in separate files and represents
/// the state border as a pseudo-node. This joins the two and drops the border
/// node, which ReCom would otherwise treat as a real unit adjacent to half
/// the state.
/// Read a plan from a CSV or a tagged JSONL, by extension.
fn read_seed_plan(path: &str) -> Result<HashMap<String, u32>> {
    let p = expand(path);
    let plan = if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
        rdarust_io::read_plan_csv(&p)
    } else {
        rdarust_io::read_plan_jsonl(&p, 0)
    };
    plan.map_err(|e| anyhow!("{e}")).with_context(|| format!("reading {path}"))
}

#[allow(clippy::too_many_arguments)]
pub fn to_recom_graph(
    state: &str,
    plan_type: &str,
    data: &str,
    graph_path: &str,
    output: Option<&str>,
    pop_name: &str,
    geoid_name: &str,
    county_name: &str,
    assignment: Option<&str>,
    assignment_name: &str,
) -> Result<()> {
    let input = load_input_data(expand(data))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {data}"))?;
    let graph = load_graph(expand(graph_path))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("reading {graph_path}"))?;
    let ctx = input
        .with_graph(graph)
        .into_context(state, plan_type, None)
        .map_err(|e| anyhow!("{e}"))
        .context("joining the precinct data to the graph")?;

    // Node order is by geoid, so the file is reproducible.
    let order = ctx.sorted_precinct_order();
    let mut position = vec![u32::MAX; ctx.n_precincts()];
    for (pos, &i) in order.iter().enumerate() {
        position[i as usize] = pos as u32;
    }

    // ReCom walks the dual graph looking for balanced cuts; on a
    // disconnected graph it cannot reach every precinct, so refuse rather
    // than emit something that will misbehave quietly.
    if !is_connected(&order, &ctx.adjacency, ctx.out_of_state) {
        let pieces = islands(&order, &ctx.adjacency, ctx.out_of_state);
        return Err(anyhow!(
            "the graph is not fully connected ({} pieces); \
             run `rdarust contiguity-mods` and `rdarust apply-mods` first",
            pieces.len()
        ));
    }

    // A chain needs somewhere to start. rustrecom takes its starting plan
    // from a node attribute (`--assignment-col`) and requires one, so a seed
    // plan is stamped on here rather than generated.
    let seed = assignment.map(read_seed_plan).transpose()?;

    let mut nodes = Vec::with_capacity(order.len());
    let mut adjacency = Vec::with_capacity(order.len());
    let mut total_pop: i64 = 0;
    let mut edges = 0usize;
    let mut districts: std::collections::BTreeSet<u32> = Default::default();

    for (id, &i) in order.iter().enumerate() {
        let geoid = &ctx.geoids[i as usize];
        let mut node = Map::new();
        node.insert(geoid_name.to_string(), json!(geoid));
        // The full five-character county FIPS, as the ReCom graphs in
        // circulation carry it.
        node.insert(
            county_name.to_string(),
            json!(if geoid.len() >= 5 { &geoid[..5] } else { geoid.as_str() }),
        );
        node.insert(pop_name.to_string(), json!(ctx.pop[i as usize]));
        if let Some(seed) = &seed {
            let d = *seed.get(geoid).ok_or_else(|| {
                anyhow!(
                    "the seed plan does not assign {geoid}; rustrecom needs every \
                     node to carry an assignment"
                )
            })?;
            districts.insert(d);
            node.insert(assignment_name.to_string(), json!(d));
        }
        node.insert("id".into(), json!(id));
        nodes.push(Value::Object(node));
        total_pop += ctx.pop[i as usize];

        let mut nbrs: Vec<u32> = ctx.adjacency[i as usize]
            .iter()
            .filter(|&&nb| Some(nb) != ctx.out_of_state)
            .map(|&nb| position[nb as usize])
            .filter(|&p| p != u32::MAX)
            .collect();
        nbrs.sort_unstable();
        nbrs.dedup();
        edges += nbrs.len();

        adjacency.push(Value::Array(
            nbrs.into_iter()
                .map(|p| {
                    let mut e = Map::new();
                    e.insert("id".into(), json!(p));
                    Value::Object(e)
                })
                .collect(),
        ));
    }

    // rustrecom accepts a 0- or 1-indexed seed and rejects gaps, so catch
    // both here where the message can say which plan is at fault.
    if !districts.is_empty() {
        let lo = *districts.iter().next().unwrap();
        let hi = *districts.iter().next_back().unwrap();
        if lo > 1 {
            return Err(anyhow!(
                "the seed plan's lowest district is {lo}; it must be numbered from 0 or 1"
            ));
        }
        if districts.len() as u32 != hi - lo + 1 {
            return Err(anyhow!(
                "the seed plan numbers {} districts between {lo} and {hi}, leaving a gap; \
                 every district in the range must have at least one precinct",
                districts.len()
            ));
        }
    }

    let mut doc = Map::new();
    doc.insert("directed".into(), json!(false));
    doc.insert("multigraph".into(), json!(false));
    doc.insert("graph".into(), json!([]));
    doc.insert("nodes".into(), Value::Array(nodes));
    doc.insert("adjacency".into(), Value::Array(adjacency));

    // networkx writes this with Python's JSON separators and no trailing
    // newline, so a file written here round-trips through its reader.
    let mut out = smart_writer(output).context("opening output")?;
    out.write_all(&rdarust_io::records::to_python_json(&Value::Object(doc))?)?;
    out.flush()?;

    eprintln!(
        "rdarust: {} nodes, {} edges, total population {total_pop}",
        order.len(),
        edges / 2
    );
    match districts.len() {
        0 => eprintln!(
            "rdarust: no seed plan; pass --assignment to write one, which \
             rustrecom's --assignment-col requires"
        ),
        n => eprintln!(
            "rdarust: seeded with {n} districts in `{assignment_name}`"
        ),
    }
    Ok(())
}
