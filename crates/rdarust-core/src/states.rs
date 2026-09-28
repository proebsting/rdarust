//! Per-state constants, transcribed from `rdapy/base/constants.py`.
//!
//! District counts are the 2020 apportionment, effective for elections from
//! 2022; county counts are census county equivalents. Fifty states, no DC or
//! territories, as rdapy has it.
//!
//! `tests/states_match_rdapy.rs` compares these against the submodule on
//! every run, because a transcription cannot notice its source changing.
//! Regenerate rather than edit.
//!
//! Two things worth knowing before trusting a number here:
//!
//! * `lower: None` means the chamber has no districts of its own, not that
//!   the state has no lower house. Arizona, Idaho, New Jersey and Washington
//!   elect theirs from the upper house's districts; Nebraska is unicameral.
//! * County counts move off the decennial clock. Connecticut's eight
//!   counties were replaced by nine planning regions as county equivalents
//!   in the Census Bureau's 2022 vintage, and the count here is still eight.
//!   Data built on the newer vintage will have more counties than this says.

/// Districts in each chamber for a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Districts {
    pub congress: u32,
    pub upper: u32,
    /// `None` for the unicameral legislatures (DC aside: NE, and the states
    /// rdapy records without a lower chamber).
    pub lower: Option<u32>,
}

/// Districts for `plan_type`, which is "congress", "upper" or "lower".
pub fn districts_for(xx: &str, plan_type: &str) -> Option<u32> {
    let d = districts_by_state(xx)?;
    match plan_type {
        "congress" => Some(d.congress),
        "upper" => Some(d.upper),
        "lower" => d.lower,
        _ => None,
    }
}

/// Number of counties in a state. Note this is the *statutory* count, which
/// can exceed the number of counties any particular dataset mentions; the
/// county-district matrix is sized from it, leaving empty columns.
pub fn counties_by_state(xx: &str) -> Option<u32> {
    COUNTIES.iter().find(|(k, _)| *k == xx).map(|(_, v)| *v)
}

pub fn districts_by_state(xx: &str) -> Option<Districts> {
    DISTRICTS.iter().find(|(k, _)| *k == xx).map(|(_, v)| *v)
}

pub const DISTRICTS: [(&str, Districts); 50] = [
    ("AL", Districts { congress: 7, upper: 35, lower: Some(105) }),
    ("AK", Districts { congress: 1, upper: 20, lower: Some(40) }),
    ("AZ", Districts { congress: 9, upper: 30, lower: None }),
    ("AR", Districts { congress: 4, upper: 35, lower: Some(100) }),
    ("CA", Districts { congress: 52, upper: 40, lower: Some(80) }),
    ("CO", Districts { congress: 8, upper: 35, lower: Some(65) }),
    ("CT", Districts { congress: 5, upper: 36, lower: Some(151) }),
    ("DE", Districts { congress: 1, upper: 21, lower: Some(41) }),
    ("FL", Districts { congress: 28, upper: 40, lower: Some(120) }),
    ("GA", Districts { congress: 14, upper: 56, lower: Some(180) }),
    ("HI", Districts { congress: 2, upper: 25, lower: Some(51) }),
    ("ID", Districts { congress: 2, upper: 35, lower: None }),
    ("IL", Districts { congress: 17, upper: 59, lower: Some(118) }),
    ("IN", Districts { congress: 9, upper: 50, lower: Some(100) }),
    ("IA", Districts { congress: 4, upper: 50, lower: Some(100) }),
    ("KS", Districts { congress: 4, upper: 40, lower: Some(125) }),
    ("KY", Districts { congress: 6, upper: 38, lower: Some(100) }),
    ("LA", Districts { congress: 6, upper: 39, lower: Some(105) }),
    ("ME", Districts { congress: 2, upper: 35, lower: Some(151) }),
    ("MD", Districts { congress: 8, upper: 47, lower: Some(67) }),
    ("MA", Districts { congress: 9, upper: 40, lower: Some(160) }),
    ("MI", Districts { congress: 13, upper: 38, lower: Some(110) }),
    ("MN", Districts { congress: 8, upper: 67, lower: Some(134) }),
    ("MS", Districts { congress: 4, upper: 52, lower: Some(122) }),
    ("MO", Districts { congress: 8, upper: 34, lower: Some(163) }),
    ("MT", Districts { congress: 2, upper: 50, lower: Some(100) }),
    ("NE", Districts { congress: 3, upper: 49, lower: None }),
    ("NV", Districts { congress: 4, upper: 21, lower: Some(42) }),
    ("NH", Districts { congress: 2, upper: 24, lower: Some(164) }),
    ("NJ", Districts { congress: 12, upper: 40, lower: None }),
    ("NM", Districts { congress: 3, upper: 42, lower: Some(70) }),
    ("NY", Districts { congress: 26, upper: 63, lower: Some(150) }),
    ("NC", Districts { congress: 14, upper: 50, lower: Some(120) }),
    ("ND", Districts { congress: 1, upper: 47, lower: Some(49) }),
    ("OH", Districts { congress: 15, upper: 33, lower: Some(99) }),
    ("OK", Districts { congress: 5, upper: 48, lower: Some(101) }),
    ("OR", Districts { congress: 6, upper: 30, lower: Some(60) }),
    ("PA", Districts { congress: 17, upper: 50, lower: Some(203) }),
    ("RI", Districts { congress: 2, upper: 38, lower: Some(75) }),
    ("SC", Districts { congress: 7, upper: 46, lower: Some(124) }),
    ("SD", Districts { congress: 1, upper: 35, lower: Some(37) }),
    ("TN", Districts { congress: 9, upper: 33, lower: Some(99) }),
    ("TX", Districts { congress: 38, upper: 31, lower: Some(150) }),
    ("UT", Districts { congress: 4, upper: 29, lower: Some(75) }),
    ("VT", Districts { congress: 1, upper: 13, lower: Some(104) }),
    ("VA", Districts { congress: 11, upper: 40, lower: Some(100) }),
    ("WA", Districts { congress: 10, upper: 49, lower: None }),
    ("WV", Districts { congress: 2, upper: 17, lower: Some(100) }),
    ("WI", Districts { congress: 8, upper: 33, lower: Some(99) }),
    ("WY", Districts { congress: 1, upper: 31, lower: Some(62) }),
];

pub const COUNTIES: [(&str, u32); 50] = [
    ("AL", 67),
    ("AK", 30),
    ("AZ", 15),
    ("AR", 75),
    ("CA", 58),
    ("CO", 64),
    ("CT", 8),
    ("DE", 3),
    ("FL", 67),
    ("GA", 159),
    ("HI", 5),
    ("ID", 44),
    ("IL", 102),
    ("IN", 92),
    ("IA", 99),
    ("KS", 105),
    ("KY", 120),
    ("LA", 64),
    ("ME", 16),
    ("MD", 24),
    ("MA", 14),
    ("MI", 83),
    ("MN", 87),
    ("MS", 82),
    ("MO", 115),
    ("MT", 56),
    ("NE", 93),
    ("NV", 17),
    ("NH", 10),
    ("NJ", 21),
    ("NM", 33),
    ("NY", 62),
    ("NC", 100),
    ("ND", 53),
    ("OH", 88),
    ("OK", 77),
    ("OR", 36),
    ("PA", 67),
    ("RI", 5),
    ("SC", 46),
    ("SD", 66),
    ("TN", 95),
    ("TX", 254),
    ("UT", 29),
    ("VT", 14),
    ("VA", 133),
    ("WA", 39),
    ("WV", 55),
    ("WI", 72),
    ("WY", 23),
];
