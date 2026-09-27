# Notes from porting rdapy to Rust

Observations gathered while porting [dra2020/rdapy](https://github.com/dra2020/rdapy)
to Rust, written for the rdapy maintainers.

**They live as a document, not in this file:**

<https://claude.ai/code/artifact/20fde9f4-9c4a-49ec-89b1-d7c75936767c>

That is the single copy, so it can be revised in place as people comment on
it. This file is a pointer; do not paste the content back here, or the two
will drift.

## What is in it

Grouped by what a reader would act on, with a one-line fix alongside each
where there is one:

- two places where a change would move a number
- performance opportunities, the largest a 14x one
- robustness suggestions
- approximations worth documenting
- compatibility, including one script that no longer runs on a fresh install
- test coverage opportunities
- an appendix of things that looked like problems and were not

## How they were found

By making the two implementations agree to within 1e-9 on real data, and
chasing down every place they did not. That is a much tighter bar than
rdapy's own tests aim at, and it surfaces things invisible at normal
tolerances.

Verified against rdapy at commit `2b702b4`, with Python 3.13.15, NumPy 2.5.3,
SciPy 1.18.1, shapely 2.1.2 and GerryChain 1.0.0. Anything checked against a
later rdapy should be re-verified.
