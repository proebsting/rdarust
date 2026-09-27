#!/usr/bin/env python3
"""
Check that a graph written by `rdarust to-recom-graph` runs ReCom.

Structure can be verified in Rust; that GerryChain accepts it cannot. This
loads the file through GerryChain, seeds a plan, runs a short chain, and
confirms every precinct ends up in exactly one district.

  conformance/tools/check_recom_graph.py <graph.json> [districts] [steps]
"""

import sys
from functools import partial

from gerrychain import Graph, MarkovChain, Partition
from gerrychain.accept import always_accept
from gerrychain.constraints import within_percent_of_ideal_population
from gerrychain.partition.initial_partition_generators import recursive_tree_part
from gerrychain.proposals import recom
from gerrychain.updaters import Tally, cut_edges

POP = "TOTAL_POP"


def main():
    if len(sys.argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(2)
    path = sys.argv[1]
    ndistricts = int(sys.argv[2]) if len(sys.argv) > 2 else 14
    steps = int(sys.argv[3]) if len(sys.argv) > 3 else 10

    graph = Graph.from_json(path)
    nodes = graph.number_of_nodes()
    print(f"loaded {nodes} nodes, {graph.number_of_edges()} edges")

    if not graph.is_connected():
        print("FAIL: graph is not connected; ReCom cannot traverse it", file=sys.stderr)
        sys.exit(1)

    missing = [n for n in graph if POP not in graph.node_data(n)]
    if missing:
        print(f"FAIL: {len(missing)} node(s) have no {POP}", file=sys.stderr)
        sys.exit(1)

    total = sum(graph.node_data(n)[POP] for n in graph)
    ideal = total / ndistricts
    print(f"population {total:,}, ideal district {ideal:,.0f}")

    seed = recursive_tree_part(graph, range(ndistricts), ideal, POP, 0.02)
    part = Partition(
        graph,
        seed,
        {"population": Tally(POP, alias="population"), "cut_edges": cut_edges},
    )

    chain = MarkovChain(
        proposal_fn=partial(
            recom, pop_col=POP, pop_target=ideal, epsilon=0.02, node_repeats=0
        ),
        constraints=[within_percent_of_ideal_population(part, 0.05)],
        acceptance_fn=always_accept,
        initial_partition=part,
        total_steps=steps,
    )

    last = None
    for i, state in enumerate(chain):
        last = state
        pops = state["population"].values()
        assert len(state.parts) == ndistricts, f"step {i}: {len(state.parts)} districts"
        assert sum(pops) == total, f"step {i}: population not conserved"
        if i in (0, steps - 1):
            print(
                f"  step {i}: {len(state['cut_edges'])} cut edges, "
                f"population {min(pops):,}-{max(pops):,}"
            )

    # Every precinct in exactly one district.
    assigned = [n for part_nodes in last.parts.values() for n in part_nodes]
    assert len(assigned) == nodes, f"{len(assigned)} assignments for {nodes} nodes"
    assert len(set(assigned)) == nodes, "a precinct landed in two districts"

    print(f"OK: {steps} ReCom steps, every precinct in exactly one district")


if __name__ == "__main__":
    main()
