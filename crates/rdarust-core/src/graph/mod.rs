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

/// One connected piece of a state.
///
/// "Coastal" precincts are the ones touching the state border; a link between
/// two pieces has to start and end on one, since anywhere else is interior to
/// a piece and cannot be the nearest crossing.
#[derive(Debug, Clone, PartialEq)]
pub struct Island {
    pub id: usize,
    pub coastal: Vec<u32>,
    pub inland: Vec<u32>,
}

impl Island {
    pub fn precincts(&self) -> usize {
        self.coastal.len() + self.inland.len()
    }
}

/// A proposed edge joining two pieces of a state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Connection {
    pub from: u32,
    pub to: u32,
    /// Squared distance, which is only ever compared against others.
    pub distance: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContiguityError {
    /// Two pieces have no border precincts between them, so there is nowhere
    /// to put a link. rdapy raises an `IndexError` here.
    NoCoastalPrecincts { island_a: usize, island_b: usize },
}

impl std::fmt::Display for ContiguityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContiguityError::NoCoastalPrecincts { island_a, island_b } => write!(
                f,
                "pieces {island_a} and {island_b} have no border precincts to link"
            ),
        }
    }
}

impl std::error::Error for ContiguityError {}

/// Split a set of precincts into connected pieces, noting which touch the
/// state border.
pub fn islands(
    ids: &[u32],
    adjacency: &[Vec<u32>],
    out_of_state: Option<u32>,
) -> Vec<Island> {
    let on_border: Vec<bool> = match out_of_state {
        Some(b) => {
            let mut v = vec![false; adjacency.len()];
            for &n in &adjacency[b as usize] {
                v[n as usize] = true;
            }
            v
        }
        None => vec![false; adjacency.len()],
    };

    connected_subsets(ids, adjacency, out_of_state)
        .into_iter()
        .enumerate()
        .map(|(id, members)| {
            let (coastal, inland) = members
                .into_iter()
                .partition(|&m| on_border[m as usize]);
            Island { id, coastal, inland }
        })
        .collect()
}

/// The fewest edges that would make a disconnected state contiguous.
///
/// Islands are joined by their closest pair of border precincts, and the
/// choice of which islands to join is a minimum spanning tree over those
/// distances -- so a chain of islands is linked along the chain rather than
/// every one being tied back to the mainland.
///
/// An empty result means the state is already connected. Distances are
/// squared, since they are only compared with each other.
pub fn contiguity_mods(
    ids: &[u32],
    adjacency: &[Vec<u32>],
    out_of_state: Option<u32>,
    centers: &[(f64, f64)],
) -> Result<Vec<Connection>, ContiguityError> {
    let pieces = islands(ids, adjacency, out_of_state);
    if pieces.len() < 2 {
        return Ok(Vec::new());
    }

    // The shortest crossing between each pair of pieces.
    let mut candidates: Vec<(usize, usize, Connection)> = Vec::new();
    for a in 0..pieces.len() {
        for b in (a + 1)..pieces.len() {
            let mut best: Option<Connection> = None;
            for &c1 in &pieces[a].coastal {
                for &c2 in &pieces[b].coastal {
                    let distance = crate::geographic::distance_proxy(
                        centers[c1 as usize],
                        centers[c2 as usize],
                    );
                    if best.is_none_or(|x| distance < x.distance) {
                        best = Some(Connection { from: c1, to: c2, distance });
                    }
                }
            }
            let best = best.ok_or(ContiguityError::NoCoastalPrecincts {
                island_a: a,
                island_b: b,
            })?;
            candidates.push((a, b, best));
        }
    }

    // Kruskal over the pieces. Ties are broken by piece number so the result
    // does not depend on iteration order.
    candidates.sort_by(|x, y| {
        x.2.distance
            .partial_cmp(&y.2.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| (x.0, x.1).cmp(&(y.0, y.1)))
    });

    let mut parent: Vec<usize> = (0..pieces.len()).collect();
    fn find(parent: &mut Vec<usize>, mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    let mut chosen: Vec<(usize, usize, Connection)> = Vec::new();
    for (a, b, edge) in candidates {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent[ra] = rb;
            chosen.push((a, b, edge));
        }
    }

    // Reported by precinct index, so the file is stable and reviewable.
    // rdapy orders by piece number instead, which comes out of Python set
    // iteration and so varies; the set of edges is the same either way.
    chosen.sort_by_key(|(_, _, c)| (c.from, c.to));
    Ok(chosen.into_iter().map(|(_, _, c)| c).collect())
}

#[cfg(test)]
mod contiguity_tests {
    use super::*;

    /// Four precincts in a row, plus a border node touching all of them.
    /// Cutting the middle link leaves two pieces.
    fn split_state() -> (Vec<Vec<u32>>, Option<u32>, Vec<(f64, f64)>) {
        //  0 - 1     2 - 3     and a border node, 4, adjacent to every one
        let adjacency = vec![
            vec![1, 4],
            vec![0, 4],
            vec![3, 4],
            vec![2, 4],
            vec![0, 1, 2, 3],
        ];
        let centers = vec![(0.0, 0.0), (1.0, 0.0), (5.0, 0.0), (6.0, 0.0), (0.0, 0.0)];
        (adjacency, Some(4), centers)
    }

    #[test]
    fn a_connected_state_needs_no_mods() {
        let adjacency = vec![vec![1, 2], vec![0, 2], vec![0, 1]];
        let centers = vec![(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)];
        let mods = contiguity_mods(&[0, 1, 2], &adjacency, None, &centers).unwrap();
        assert!(mods.is_empty());
    }

    #[test]
    fn two_pieces_are_joined_at_their_closest_border_precincts() {
        let (adjacency, border, centers) = split_state();
        let pieces = islands(&[0, 1, 2, 3], &adjacency, border);
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().all(|p| p.inland.is_empty()), "all touch the border");

        let mods = contiguity_mods(&[0, 1, 2, 3], &adjacency, border, &centers).unwrap();
        assert_eq!(mods.len(), 1, "one edge joins two pieces");
        // Precincts 1 and 2 are the closest pair across the gap, at 4 apart;
        // 0 to 3 would be 6.
        assert_eq!((mods[0].from, mods[0].to), (1, 2));
    }

    #[test]
    fn a_chain_of_islands_is_linked_along_the_chain() {
        // Four single-precinct islands in a line, each 1 apart, all touching
        // the border. A spanning tree links neighbours; tying every island to
        // the first would cost more.
        let adjacency = vec![vec![4], vec![4], vec![4], vec![4], vec![0, 1, 2, 3]];
        let centers = vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (0.0, 0.0)];

        let mods = contiguity_mods(&[0, 1, 2, 3], &adjacency, Some(4), &centers).unwrap();
        assert_eq!(mods.len(), 3, "n islands need n-1 edges");
        let total: f64 = mods.iter().map(|m| m.distance).sum();
        assert!((total - 3.0).abs() < 1e-12, "three unit hops, not longer ones");
    }

    #[test]
    fn applying_the_mods_connects_the_state() {
        let (mut adjacency, border, centers) = split_state();
        let ids = [0u32, 1, 2, 3];
        assert!(!is_connected(&ids, &adjacency, border));

        for m in contiguity_mods(&ids, &adjacency, border, &centers).unwrap() {
            adjacency[m.from as usize].push(m.to);
            adjacency[m.to as usize].push(m.from);
        }
        assert!(is_connected(&ids, &adjacency, border));
        assert!(is_consistent(&adjacency));
    }

    #[test]
    fn a_piece_with_no_border_precinct_cannot_be_linked() {
        // Precinct 2 is isolated and does not touch the border, so there is
        // nowhere to put a crossing.
        let adjacency = vec![vec![1, 3], vec![0, 3], vec![], vec![0, 1]];
        let centers = vec![(0.0, 0.0), (1.0, 0.0), (5.0, 0.0), (0.0, 0.0)];
        let err = contiguity_mods(&[0, 1, 2], &adjacency, Some(3), &centers);
        assert!(matches!(err, Err(ContiguityError::NoCoastalPrecincts { .. })));
    }
}
