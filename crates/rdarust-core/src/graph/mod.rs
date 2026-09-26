//! Contiguity and embeddedness checks, ported from `rdapy/graph/`.
//!
//! Nodes are indices into an adjacency list. `out_of_state` names the virtual
//! node standing for the state border, which is excluded from connectivity:
//! two islands are not contiguous just because both touch the sea.

/// Marks a precinct that is not assigned to any district.
pub use crate::compactness::energy::UNASSIGNED;

/// Is every edge reciprocated?
pub fn is_consistent(adjacency: &[Vec<u32>]) -> bool {
    adjacency.iter().enumerate().all(|(node, neighbors)| {
        neighbors
            .iter()
            .all(|&nb| adjacency[nb as usize].contains(&(node as u32)))
    })
}

/// Is the given set of nodes connected, ignoring the state border?
///
/// An empty set is trivially connected, matching rdapy, which would raise on
/// an empty input.
pub fn is_connected(ids: &[u32], adjacency: &[Vec<u32>], out_of_state: Option<u32>) -> bool {
    let mut in_set = vec![false; adjacency.len()];
    let mut count = 0usize;
    for &id in ids {
        if Some(id) == out_of_state {
            continue;
        }
        if !in_set[id as usize] {
            in_set[id as usize] = true;
            count += 1;
        }
    }
    if count == 0 {
        return true;
    }

    let start = ids
        .iter()
        .copied()
        .find(|&id| Some(id) != out_of_state)
        .expect("a non-border node");

    let mut visited = vec![false; adjacency.len()];
    let mut stack = vec![start];
    visited[start as usize] = true;
    let mut seen = 1usize;

    while let Some(node) = stack.pop() {
        for &nb in &adjacency[node as usize] {
            if Some(nb) == out_of_state {
                continue;
            }
            if in_set[nb as usize] && !visited[nb as usize] {
                visited[nb as usize] = true;
                seen += 1;
                stack.push(nb);
            }
        }
    }

    seen == count
}

/// Partition a set of nodes into its connected components.
///
/// Components come out in the order their first member appears in `ids`, and
/// each component's members are sorted, so the result is deterministic --
/// rdapy's depends on Python set iteration order.
pub fn connected_subsets(
    ids: &[u32],
    adjacency: &[Vec<u32>],
    out_of_state: Option<u32>,
) -> Vec<Vec<u32>> {
    let mut remaining = vec![false; adjacency.len()];
    let mut order = Vec::with_capacity(ids.len());
    for &id in ids {
        if Some(id) == out_of_state || remaining[id as usize] {
            continue;
        }
        remaining[id as usize] = true;
        order.push(id);
    }

    let mut subsets = Vec::new();
    for &start in &order {
        if !remaining[start as usize] {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![start];
        remaining[start as usize] = false;

        while let Some(node) = stack.pop() {
            component.push(node);
            for &nb in &adjacency[node as usize] {
                if Some(nb) == out_of_state {
                    continue;
                }
                if remaining[nb as usize] {
                    remaining[nb as usize] = false;
                    stack.push(nb);
                }
            }
        }
        component.sort_unstable();
        subsets.push(component);
    }
    subsets
}

/// Is a district entirely surrounded by one other district -- a "donut hole"?
///
/// It is not, if any of its precincts touches the state border, or if it has
/// two or more distinct neighbouring districts. An unassigned neighbour is
/// taken to be border water, which also disqualifies it.
pub fn is_embedded(
    district: u32,
    district_of: &[u32],
    members: &[u32],
    adjacency: &[Vec<u32>],
    out_of_state: Option<u32>,
) -> bool {
    if members.is_empty() {
        return true;
    }

    let mut neighbouring: Option<u32> = None;

    for &node in members {
        for &nb in &adjacency[node as usize] {
            if Some(nb) == out_of_state {
                return false;
            }
            let other = district_of[nb as usize];
            if other == UNASSIGNED {
                return false;
            }
            if other != district {
                match neighbouring {
                    None => neighbouring = Some(other),
                    Some(d) if d != other => return false,
                    _ => {}
                }
            }
        }
    }
    true
}
