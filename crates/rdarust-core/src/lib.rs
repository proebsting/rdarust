//! Redistricting analytics, ported from [dra2020/rdapy](https://github.com/dra2020/rdapy).
//!
//! The crate is layered:
//!
//! * [`numeric`], [`spline`], [`geometry`] -- low-level primitives that must
//!   reproduce CPython / NumPy / SciPy behaviour exactly. Everything else is
//!   built on these, so they are pinned against recorded Python values in
//!   `conformance/cases/primitives/`.
//!
//! Later milestones add the formula layer (pure functions on slices) and the
//! pipeline layer (`Context` + plan scoring).

pub mod geometry;
pub mod numeric;
pub mod spline;
