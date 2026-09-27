//! The rustrecom / GerryChain interchange format.
//!
//! ReCom implementations read a dual graph in networkx's `adjacency_data`
//! JSON: a node per precinct carrying its geoid, county, population and
//! optionally a starting assignment, plus a parallel adjacency list of node
//! indices.
//!
//! Two things make this more than a spelling change, and both live here
//! rather than in the CLI so that anything driving rdarust and a ReCom
//! implementation in one process gets them right:
//!
//! * rdapy represents the state border as a pseudo-node. ReCom would treat
//!   it as a real unit adjacent to half the state, so it is dropped.
//! * rdarust numbers districts from 1, reserving 0 for unassigned; ReCom
//!   implementations number from 0. [`district_shift`] is the one place that
//!   conversion is written down.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use rdarust_core::context::Context;
use rdarust_core::graph::{is_connected, islands};
use serde_json::{json, Map, Value};

/// What each node attribute is called in the graph file.
///
/// The names are not standardised -- the graphs in circulation disagree --
/// so every consumer names the ones it wants on its command line.
#[derive(Debug, Clone, Copy)]
pub struct RecomNames<'a> {
    pub geoid: &'a str,
    pub pop: &'a str,
    pub county: &'a str,
    pub assignment: &'a str,
}

impl Default for RecomNames<'_> {
    fn default() -> Self {
        Self { geoid: "GEOID", pop: "TOTAL_POP", county: "COUNTY", assignment: "INITIAL" }
    }
}

/// A built graph, with the figures a caller wants to report on it.
pub struct RecomGraph {
    pub doc: Value,
    pub n_nodes: usize,
    /// Undirected edges, so each adjacency is counted once.
    pub n_edges: usize,
    pub total_pop: i64,
    /// Distinct districts in the stamped seed plan; zero when there is none.
    pub n_districts: usize,
}

#[derive(Debug)]
pub enum RecomError {
    /// ReCom walks the dual graph looking for balanced cuts, so on a
    /// disconnected graph it cannot reach every precinct.
    Disconnected { pieces: usize },
    /// ReCom takes its starting plan from a node attribute and requires one
    /// on every node.
    Unassigned { geoid: String },
    /// A numbering that starts above 1 cannot be shifted onto either
    /// convention without guessing.
    NotFromZeroOrOne { lowest: i64 },
    DistrictGap { count: usize, lowest: i64, highest: i64 },
    NoNodes,
    NoGeoids { key: String },
}

impl fmt::Display for RecomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected { pieces } => {
                write!(f, "the graph is not fully connected ({pieces} pieces)")
            }
            Self::Unassigned { geoid } => write!(
                f,
                "the seed plan does not assign {geoid}; every node needs an assignment"
            ),
            Self::NotFromZeroOrOne { lowest } => write!(
                f,
                "the seed plan's lowest district is {lowest}; it must be numbered from 0 or 1"
            ),
            Self::DistrictGap { count, lowest, highest } => write!(
                f,
                "the seed plan numbers {count} districts between {lowest} and {highest}, \
                 leaving a gap; every district in the range must have at least one precinct"
            ),
            Self::NoNodes => write!(f, "the graph has no `nodes` array"),
            Self::NoGeoids { key } => {
                write!(f, "no node in the graph has a `{key}` property")
            }
        }
    }
}

impl std::error::Error for RecomError {}

/// Build the dual graph for a ReCom implementation.
///
/// Nodes are in geoid order, so the file is reproducible, and the border
/// node is dropped. `seed` stamps a starting plan onto each node; without
/// one the graph carries no assignment attribute at all.
pub fn build_graph(
    ctx: &Context,
    names: RecomNames<'_>,
    seed: Option<&HashMap<String, u32>>,
) -> Result<RecomGraph, RecomError> {
    let order = ctx.sorted_precinct_order();
    let mut position = vec![u32::MAX; ctx.n_precincts()];
    for (pos, &i) in order.iter().enumerate() {
        position[i as usize] = pos as u32;
    }

    if !is_connected(&order, &ctx.adjacency, ctx.out_of_state) {
        let pieces = islands(&order, &ctx.adjacency, ctx.out_of_state);
        return Err(RecomError::Disconnected { pieces: pieces.len() });
    }

    let mut nodes = Vec::with_capacity(order.len());
    let mut adjacency = Vec::with_capacity(order.len());
    let mut total_pop: i64 = 0;
    let mut edges = 0usize;
    let mut districts: BTreeSet<u32> = BTreeSet::new();

    for (id, &i) in order.iter().enumerate() {
        let geoid = &ctx.geoids[i as usize];
        let mut node = Map::new();
        node.insert(names.geoid.to_string(), json!(geoid));
        // The full five-character county FIPS, as the ReCom graphs in
        // circulation carry it.
        node.insert(
            names.county.to_string(),
            json!(if geoid.len() >= 5 { &geoid[..5] } else { geoid.as_str() }),
        );
        node.insert(names.pop.to_string(), json!(ctx.pop[i as usize]));
        if let Some(seed) = seed {
            let d = *seed
                .get(geoid)
                .ok_or_else(|| RecomError::Unassigned { geoid: geoid.clone() })?;
            districts.insert(d);
            node.insert(names.assignment.to_string(), json!(d));
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

    // A ReCom implementation accepts a 0- or 1-indexed seed and rejects
    // gaps, so catch both here where the message can say which plan is at
    // fault.
    if let (Some(&lo), Some(&hi)) = (districts.iter().next(), districts.iter().next_back()) {
        if lo > 1 {
            return Err(RecomError::NotFromZeroOrOne { lowest: lo as i64 });
        }
        if districts.len() as u32 != hi - lo + 1 {
            return Err(RecomError::DistrictGap {
                count: districts.len(),
                lowest: lo as i64,
                highest: hi as i64,
            });
        }
    }

    let mut doc = Map::new();
    doc.insert("directed".into(), json!(false));
    doc.insert("multigraph".into(), json!(false));
    doc.insert("graph".into(), json!([]));
    doc.insert("nodes".into(), Value::Array(nodes));
    doc.insert("adjacency".into(), Value::Array(adjacency));

    Ok(RecomGraph {
        doc: Value::Object(doc),
        n_nodes: order.len(),
        n_edges: edges / 2,
        total_pop,
        n_districts: districts.len(),
    })
}

/// The geoids of a dual graph, in node-index order.
///
/// Chain output identifies precincts by node index rather than by geoid, so
/// the graph is what names them again.
pub fn graph_geoids(graph: &Value, geoid_key: &str) -> Result<Vec<String>, RecomError> {
    let nodes = graph
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or(RecomError::NoNodes)?;
    let geoids: Vec<String> = nodes
        .iter()
        .map(|n| {
            n.get(geoid_key).and_then(|g| g.as_str()).unwrap_or_default().to_string()
        })
        .collect();
    if geoids.iter().all(|g| g.is_empty()) {
        return Err(RecomError::NoGeoids { key: geoid_key.to_string() });
    }
    Ok(geoids)
}

/// The offset that moves a district numbering onto rdarust's, which starts
/// at 1 because 0 marks an unassigned precinct.
///
/// ReCom implementations normalise district labels to 0-based internally, by
/// subtracting the lowest, and write them out that way whatever the seed
/// used. The shift preserves identity: it is an offset, not a relabelling,
/// and a numbering that already starts at 1 or above is left alone.
pub fn district_shift(districts: impl IntoIterator<Item = i64>) -> i64 {
    match districts.into_iter().min() {
        Some(lowest) if lowest < 1 => 1 - lowest,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::district_shift;

    #[test]
    fn shift_moves_zero_based_numbering_up_by_one() {
        assert_eq!(district_shift([0, 1, 2, 3]), 1);
        assert_eq!(district_shift([3, 0, 1, 2]), 1);
    }

    #[test]
    fn shift_leaves_one_based_numbering_alone() {
        assert_eq!(district_shift([1, 2, 3]), 0);
        // A plan that happens to use none of the low districts is still
        // 1-based; nothing is renumbered to close the gap.
        assert_eq!(district_shift([4, 5, 6]), 0);
    }

    #[test]
    fn shift_handles_negative_and_empty() {
        assert_eq!(district_shift([-2, -1, 0]), 3);
        assert_eq!(district_shift(std::iter::empty()), 0);
    }
}
