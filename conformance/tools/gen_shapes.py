#!/usr/bin/env python3
"""
Record shape-based compactness from rdapy, with the shapes themselves.

rdapy's compactness tests read ESRI shapefiles, which no Rust runner should
have to. The shapes are written out here as plain coordinates alongside every
intermediate rdapy computes from them, so a port can be debugged against the
step that actually diverges rather than only against the final rank.

  PYTHONPATH=vendor/rdapy ~/ext/rdapy/.venv/bin/python \
      conformance/tools/gen_shapes.py conformance/cases/shapes
"""

import json
import os
import platform
import subprocess
import sys

REPO = os.path.abspath(os.path.dirname(os.path.dirname(os.path.dirname(__file__))))
RDAPY = os.path.join(REPO, "vendor", "rdapy")
sys.path.insert(0, RDAPY)

import numpy as np  # noqa: E402
import shapely  # noqa: E402

from rdapy import (  # noqa: E402
    calc_bbox, calc_hull, calc_polsby, calc_reock, calc_schwartzberg,
    calc_sym_x, calc_sym_y, featureize_shape, kiwysi_rank_shape, load_features,
    load_shapes, score_features, trim_kiwysi_rank,
)
from rdapy.compactness.pypoly import (  # noqa: E402
    get_polygon_attributes, minimum_bounding_rectangle,
)

SCHEMA = "rdarust.cases/1"

SETS = [
    ("first20", "OBJECTID", "testdata/compactness/first20"),
    ("evenlyspaced20", "GEOID", "testdata/compactness/evenlyspaced20"),
    ("NC-116th-Congressional", "id", "testdata/compactness/NC-116th-Congressional"),
]


def provenance():
    try:
        sha = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=RDAPY, text=True
        ).strip()
    except Exception:
        sha = "unknown"
    return {"rdapy_commit": sha, "python": platform.python_version(),
            "numpy": np.__version__, "shapely": shapely.__version__,
            "platform": platform.platform()}


def coords(shp):
    """The shape as GeoJSON-style coordinates."""
    def poly(p):
        return [[list(c) for c in p.exterior.coords]] + \
               [[list(c) for c in r.coords] for r in p.interiors]
    if shp.geom_type == "Polygon":
        return {"type": "Polygon", "coordinates": poly(shp)}
    return {"type": "MultiPolygon", "coordinates": [poly(p) for p in shp.geoms]}


def attrs(shp, geodesic):
    a, p, d = get_polygon_attributes(shp, geodesic)
    return {"area": a, "perimeter": p, "diameter": d}


def main():
    outdir = os.path.abspath(
        sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "conformance/cases/shapes")
    )
    os.makedirs(outdir, exist_ok=True)
    os.chdir(RDAPY)

    for name, id_field, path in SETS:
        shapes, _ = load_shapes(path, id=id_field)
        cases = []
        for shape_id, shp in shapes:
            # The exterior points the feature code works from: outer rings
            # only, holes excluded.
            exteriors = [[list(c) for c in p.exterior.coords]
                         for p in (shp.geoms if shp.geom_type == "MultiPolygon" else [shp])]
            flat = np.array([pt for ring in exteriors for pt in ring])

            cases.append({
                "id": str(shape_id),
                "geometry": coords(shp),
                "projected": attrs(shp, False),
                "geodesic": attrs(shp, True),
                "min_bounding_rectangle": [list(p) for p in minimum_bounding_rectangle(flat)],
                # The seven KIWYSI features, all geodesic.
                "features": {
                    "sym_x": calc_sym_x(shp, True),
                    "sym_y": calc_sym_y(shp, True),
                    "reock": calc_reock(shp, True),
                    "bbox": calc_bbox(shp, True),
                    "polsby": calc_polsby(shp, True),
                    "hull": calc_hull(shp, True),
                    "schwartzberg": calc_schwartzberg(shp, True),
                },
                # What DRA reports, which is planar.
                "projected_reock": calc_reock(shp, False),
                "projected_polsby": calc_polsby(shp, False),
                "kiwysi_revised": kiwysi_rank_shape(shp, geodesic=True, revised=True),
                "kiwysi_original": kiwysi_rank_shape(shp, geodesic=True, revised=False),
                "kiwysi_trimmed": trim_kiwysi_rank(
                    score_features(featureize_shape(shp, True), revised=True)
                ),
            })

        doc = {
            "schema": SCHEMA, "suite": "shapes", "set": name,
            "note": ("Shape-based compactness from rdapy, with the shapes. Every "
                     "intermediate is recorded so a port can be debugged at the "
                     "step that diverges, not only at the final rank."),
            "source": provenance(), "cases": cases,
        }
        out = os.path.join(outdir, f"{name}.json")
        with open(out, "w") as f:
            json.dump(doc, f, indent=1)
            f.write("\n")
        vertices = sum(len(r) for c in cases for p in
                       ([c["geometry"]["coordinates"]] if c["geometry"]["type"] == "Polygon"
                        else c["geometry"]["coordinates"]) for r in p)
        print(f"  {name}: {len(cases)} shapes, {vertices} vertices "
              f"-> {os.path.getsize(out)/1024:.0f} KB")

    # The published predictions rdapy's KIWYSI tests check against.
    for name, csv, revised in [
        ("first20", "testdata/compactness/first20/smartfeats_first20.csv", False),
        ("evenlyspaced20", "testdata/compactness/evenlyspaced20/evenlyspaced20.csv", True),
    ]:
        _, predictions = load_features(csv)
        doc = {
            "schema": SCHEMA, "suite": "shapes", "set": f"{name}-published",
            "note": ("Published KIWYSI predictions from the model's authors. "
                     "rdapy's tests assert its ranks match these to within 1."),
            "source": provenance(),
            "revised_model": revised,
            "cases": [{"index": i, "expect": v} for i, v in predictions],
        }
        with open(os.path.join(outdir, f"{name}-published.json"), "w") as f:
            json.dump(doc, f, indent=1)
            f.write("\n")
        print(f"  {name}-published: {len(predictions)} predictions")

    print("done")


if __name__ == "__main__":
    main()
