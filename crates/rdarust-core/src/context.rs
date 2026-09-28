//! Everything about a state that does not vary from plan to plan.
//!
//! [`Context`] is built once and then scores plans. It interns geoids to dense
//! indices at construction, so the scoring loop never hashes a string --
//! rdapy does, on every edge visit, which is exactly the cost this port exists
//! to remove.
//!
//! Precincts keep the order they arrived in. Floating-point addition is not
//! associative, so aggregating in a different order gives a different last
//! bit, and matching rdapy means matching its order.

use std::collections::HashMap;

/// Marks a precinct not assigned to any district.
pub use crate::compactness::energy::UNASSIGNED;

/// rdapy's name for the virtual node standing in for the state border.
pub const OUT_OF_STATE: &str = "OUT_OF_STATE";

/// Which dataset of each type a score is computed against.
///
/// A plan can be scored against several elections at once, so partisan scores
/// are always qualified by the election they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetKeys {
    pub census: String,
    pub vap: String,
    /// `None` where rdapy uses the sentinel `"N/A"`, meaning no CVAP data.
    pub cvap: Option<String>,
    pub elections: Vec<String>,
    pub shapes: String,
}

/// Per-precinct vote counts for one election.
#[derive(Debug, Clone, PartialEq)]
pub struct Election {
    pub key: String,
    pub dem: Vec<i64>,
    pub rep: Vec<i64>,
}

/// Per-precinct counts for a set of demographics.
///
/// `names` are rdapy's field keys in metadata order -- `total_vap`,
/// `white_vap`, and so on. The first is the total; the rest are shares of it.
/// Order is load-bearing, since it decides the order opportunity is summed in.
#[derive(Debug, Clone, PartialEq)]
pub struct Demographics {
    pub names: Vec<String>,
    /// `counts[demo][precinct]`.
    pub counts: Vec<Vec<i64>>,
}

impl Demographics {
    pub fn total(&self) -> &[i64] {
        &self.counts[0]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextError {
    /// Two precincts share a geoid.
    DuplicateGeoid(String),
    /// A neighbour names a precinct that has no record.
    UnknownNeighbor { of: String, neighbor: String },
    /// The state or chamber is not one rdapy knows.
    UnknownState { xx: String, plan_type: String },
    /// Per-precinct arrays disagree in length.
    LengthMismatch { what: &'static str, got: usize, want: usize },
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextError::DuplicateGeoid(g) => write!(f, "duplicate geoid {g}"),
            ContextError::UnknownNeighbor { of, neighbor } => {
                write!(f, "{of} lists neighbour {neighbor}, which has no record")
            }
            ContextError::UnknownState { xx, plan_type } => {
                write!(f, "no district count for {xx} {plan_type}")
            }
            ContextError::LengthMismatch { what, got, want } => {
                write!(f, "{what} has {got} entries, expected {want}")
            }
        }
    }
}

impl std::error::Error for ContextError {}

/// One precinct as it arrives from the input data.
#[derive(Debug, Clone, PartialEq)]
pub struct PrecinctInput {
    pub geoid: String,
    pub pop: i64,
    pub center: (f64, f64),
    pub area: f64,
    /// Shared border length with each neighbour, by geoid.
    pub arcs: Vec<(String, f64)>,
    /// Convex hull of the precinct, as (lon, lat).
    pub exterior: Vec<(f64, f64)>,
    pub neighbors: Vec<String>,
}

/// A state, ready to score plans.
pub struct Context {
    pub xx: String,
    pub plan_type: String,
    pub n_districts: usize,
    /// The statutory county count, which sizes the county-district matrix.
    /// It can exceed the number of counties the data mentions.
    pub n_counties: usize,

    pub geoids: Vec<String>,
    pub index: HashMap<String, u32>,
    /// Column of the county-district matrix for each precinct.
    pub county_of: Vec<u32>,
    /// County FIPS codes actually present, in column order.
    pub counties: Vec<String>,

    /// Adjacency, in indices. Index `n_precincts` is the border node when
    /// [`Context::out_of_state`] is set.
    pub adjacency: Vec<Vec<u32>>,
    pub out_of_state: Option<u32>,
    /// rdapy's lexical test: a geoid ending in `ZZZZZZ` is water only.
    pub is_water_only: Vec<bool>,

    pub pop: Vec<i64>,
    pub center: Vec<(f64, f64)>,
    pub area: Vec<f64>,
    pub exterior: Vec<Vec<(f64, f64)>>,
    /// Shared border with each neighbour, as (index, length).
    pub arcs: Vec<Vec<(u32, f64)>>,
    /// The same lengths, aligned one-to-one with [`Context::adjacency`].
    ///
    /// rdapy walks the neighbour list and looks each arc up in a dict. Laying
    /// them out parallel turns that into an indexed read, which matters
    /// because perimeter accumulation is the inner loop of shape aggregation.
    /// `None` marks a neighbour the arc table does not mention.
    pub arc_len: Vec<Vec<Option<f64>>>,

    pub elections: Vec<Election>,
    pub vap: Demographics,
    pub cvap: Option<Demographics>,
    pub keys: DatasetKeys,

    /// Things worth telling the user that are not errors. Construction has
    /// no way to print, and a caller that ignores these still gets a usable
    /// context, so they are carried rather than raised.
    pub warnings: Vec<String>,
}

/// rdapy's lexical test for a water-only precinct.
#[inline]
pub fn is_water_only(geoid: &str) -> bool {
    geoid.ends_with("ZZZZZZ")
}

/// The county part of a 15-character geoid: characters 2..5, the county FIPS
/// within the state.
#[inline]
pub fn county_of_geoid(geoid: &str) -> &str {
    if geoid.len() >= 5 {
        &geoid[2..5]
    } else {
        geoid
    }
}

impl Context {
    /// Assemble a context from precinct records.
    ///
    /// `precincts` must be in the order rdapy reads them. `border_neighbors`
    /// is the neighbour list of the virtual out-of-state node, if the input
    /// has one.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        xx: &str,
        plan_type: &str,
        precincts: Vec<PrecinctInput>,
        border_neighbors: Option<Vec<String>>,
        elections: Vec<Election>,
        vap: Demographics,
        cvap: Option<Demographics>,
        keys: DatasetKeys,
        districts_override: Option<u32>,
    ) -> Result<Self, ContextError> {
        let n = precincts.len();

        let n_districts = match districts_override {
            Some(d) => d,
            None => crate::states::districts_for(xx, plan_type).ok_or_else(|| {
                ContextError::UnknownState {
                    xx: xx.to_string(),
                    plan_type: plan_type.to_string(),
                }
            })?,
        } as usize;
        // Resolved once the data's counties are known, below: the table has
        // no entry for DC or Puerto Rico, and the count only sizes a matrix
        // that already tolerates being widened.
        let statutory_counties = crate::states::counties_by_state(xx).map(|c| c as usize);

        // Intern geoids. The border node, if present, sits one past the end.
        let mut index: HashMap<String, u32> = HashMap::with_capacity(n + 1);
        for (i, p) in precincts.iter().enumerate() {
            if index.insert(p.geoid.clone(), i as u32).is_some() {
                return Err(ContextError::DuplicateGeoid(p.geoid.clone()));
            }
        }
        // The border node may arrive as its own record, or only as a
        // neighbour -- the CLI takes the graph from a separate file, so the
        // precinct data names OUT_OF_STATE without defining it. Either way it
        // needs an index. Its own neighbour list is never walked during
        // scoring, so an empty one is fine when the input does not supply it.
        let references_border = border_neighbors.is_some()
            || precincts.iter().any(|p| {
                p.neighbors.iter().any(|g| g == OUT_OF_STATE)
                    || p.arcs.iter().any(|(g, _)| g == OUT_OF_STATE)
            });
        let border_neighbors = if references_border {
            Some(border_neighbors.unwrap_or_default())
        } else {
            None
        };
        let out_of_state = border_neighbors.as_ref().map(|_| {
            index.insert(OUT_OF_STATE.to_string(), n as u32);
            n as u32
        });
        let n_nodes = if out_of_state.is_some() { n + 1 } else { n };

        // Counties, sorted so the matrix column order is deterministic.
        // rdapy's order comes out of a Python set, which is arbitrary; column
        // order only affects the order sums accumulate in, never the result.
        let mut counties: Vec<String> = precincts
            .iter()
            .map(|p| county_of_geoid(&p.geoid).to_string())
            .collect();
        counties.sort();
        counties.dedup();

        // The county-district matrix is sized from the statutory count but
        // indexed by the counties the data holds, so more of the latter runs
        // off the end. rdapy raises an IndexError here; widen the matrix
        // instead, which costs a column of zeros and is right whichever way
        // the mismatch arose. See KNOWN-DIFFERENCES.md.
        let mut warnings = Vec::new();
        let mut n_counties = statutory_counties.unwrap_or_else(|| {
            // DC and Puerto Rico are published by DRA but absent from
            // rdapy's table. The data says how many county equivalents it
            // covers, which is all this number is for.
            warnings.push(format!(
                "no statutory county count for {xx}; using the {} the data covers",
                counties.len()
            ));
            counties.len()
        });
        if counties.len() > n_counties {
            // Every geoid opens with its state's FIPS code, so the data can
            // say which state it is even though nothing here maps a FIPS
            // code back to an abbreviation. One prefix means the data is
            // coherent and `xx` is probably wrong; several means the data is
            // mixed.
            let mut prefixes: Vec<&str> = precincts
                .iter()
                .filter_map(|p| p.geoid.get(..2))
                .collect();
            prefixes.sort_unstable();
            prefixes.dedup();
            let whose = match prefixes.as_slice() {
                [one] => format!("every geoid is in state FIPS {one}"),
                many => format!("the geoids span state FIPS {}", many.join(", ")),
            };
            warnings.push(format!(
                "the data covers {} counties but {xx} has {n_counties}; {whose}. \
                 Scoring {} counties. If {xx} is not this data's state the county \
                 splitting scores will be meaningless.",
                counties.len(),
                counties.len()
            ));
            n_counties = counties.len();
        }
        let county_index: HashMap<&str, u32> = counties
            .iter()
            .enumerate()
            .map(|(i, c)| (c.as_str(), i as u32))
            .collect();

        let resolve = |of: &str, g: &str| -> Result<u32, ContextError> {
            index.get(g).copied().ok_or_else(|| ContextError::UnknownNeighbor {
                of: of.to_string(),
                neighbor: g.to_string(),
            })
        };

        let mut adjacency: Vec<Vec<u32>> = Vec::with_capacity(n_nodes);
        let mut arcs: Vec<Vec<(u32, f64)>> = Vec::with_capacity(n);
        let mut geoids = Vec::with_capacity(n);
        let mut county_of = Vec::with_capacity(n);
        let mut is_water = Vec::with_capacity(n_nodes);
        let mut pop = Vec::with_capacity(n);
        let mut center = Vec::with_capacity(n);
        let mut area = Vec::with_capacity(n);
        let mut exterior = Vec::with_capacity(n);

        for p in &precincts {
            let mut nbrs = Vec::with_capacity(p.neighbors.len());
            for g in &p.neighbors {
                nbrs.push(resolve(&p.geoid, g)?);
            }
            adjacency.push(nbrs);

            let mut a = Vec::with_capacity(p.arcs.len());
            for (g, len) in &p.arcs {
                a.push((resolve(&p.geoid, g)?, *len));
            }
            arcs.push(a);

            county_of.push(county_index[county_of_geoid(&p.geoid)]);
            is_water.push(is_water_only(&p.geoid));
            geoids.push(p.geoid.clone());
            pop.push(p.pop);
            center.push(p.center);
            area.push(p.area);
            exterior.push(p.exterior.clone());
        }

        if let Some(border) = &border_neighbors {
            let mut nbrs = Vec::with_capacity(border.len());
            for g in border {
                nbrs.push(resolve(OUT_OF_STATE, g)?);
            }
            adjacency.push(nbrs);
            is_water.push(false);
        }

        let check = |what: &'static str, got: usize| -> Result<(), ContextError> {
            if got == n {
                Ok(())
            } else {
                Err(ContextError::LengthMismatch { what, got, want: n })
            }
        };
        for e in &elections {
            check("election dem votes", e.dem.len())?;
            check("election rep votes", e.rep.len())?;
        }
        for c in &vap.counts {
            check("vap counts", c.len())?;
        }
        if let Some(c) = &cvap {
            for v in &c.counts {
                check("cvap counts", v.len())?;
            }
        }

        // Lay the arc lengths out parallel to the adjacency lists.
        let arc_len: Vec<Vec<Option<f64>>> = adjacency
            .iter()
            .take(n)
            .enumerate()
            .map(|(i, nbrs)| {
                nbrs.iter()
                    .map(|nb| arcs[i].iter().find(|(g, _)| g == nb).map(|(_, l)| *l))
                    .collect()
            })
            .collect();

        Ok(Context {
            xx: xx.to_string(),
            plan_type: plan_type.to_string(),
            n_districts,
            n_counties,
            warnings,
            geoids,
            index,
            county_of,
            counties,
            arc_len,
            adjacency,
            out_of_state,
            is_water_only: is_water,
            pop,
            center,
            area,
            exterior,
            arcs,
            elections,
            vap,
            cvap,
            keys,
        })
    }

    pub fn n_precincts(&self) -> usize {
        self.geoids.len()
    }

    /// Name of a graph node, including the virtual border node.
    pub fn node_name(&self, i: usize) -> String {
        self.geoids
            .get(i)
            .cloned()
            .unwrap_or_else(|| OUT_OF_STATE.to_string())
    }

    /// Turn a geoid-keyed plan into the dense form the scorer wants.
    ///
    /// Precincts the plan does not mention come back as [`UNASSIGNED`];
    /// aggregation rejects those only if they have population, matching rdapy.
    /// Geoids the context does not know are ignored, as rdapy ignores them.
    pub fn plan_from_assignments<S: AsRef<str>>(
        &self,
        assignments: impl IntoIterator<Item = (S, u32)>,
    ) -> Vec<u32> {
        let mut plan = vec![UNASSIGNED; self.adjacency.len()];
        for (geoid, district) in assignments {
            if let Some(&i) = self.index.get(geoid.as_ref()) {
                plan[i as usize] = district;
            }
        }
        plan
    }
}
