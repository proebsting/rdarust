//! Shape-based compactness must reproduce rdapy's.
//!
//! The fixtures in `conformance/cases/shapes/` carry the shapes themselves --
//! converted once from ESRI shapefiles -- alongside every intermediate rdapy
//! computes, so a divergence points at the step that caused it rather than
//! only at the final rank.

use std::collections::BTreeMap;

use rdarust_geo::shapes::{
    calc_polsby, calc_reock, featureize_shape, kiwysi_rank_shape, minimum_bounding_rectangle,
    polygon_attributes,
};
use rdarust_geo::{Geometry, Point, Polygon};
use serde_json::Value;

/// Tracks the worst relative error seen for each quantity.
#[derive(Default)]
struct Worst(BTreeMap<String, (f64, String)>);

impl Worst {
    fn note(&mut self, what: &str, got: f64, want: f64, where_: &str) {
        let err = (got - want).abs() / want.abs().max(1.0);
        let e = self.0.entry(what.to_string()).or_insert((0.0, String::new()));
        if err > e.0 {
            *e = (err, where_.to_string());
        }
    }

    fn report(&self) {
        for (what, (err, where_)) in &self.0 {
            eprintln!("  {what:<28} {err:.3e}  ({where_})");
        }
    }

    fn worst(&self) -> f64 {
        self.0.values().map(|(e, _)| *e).fold(0.0, f64::max)
    }

    fn worst_of(&self, what: &str) -> f64 {
        self.0.get(what).map(|(e, _)| *e).unwrap_or(0.0)
    }
}

fn ring(v: &Value) -> Vec<Point> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect()
}

fn polygon(v: &Value) -> Polygon {
    let rings = v.as_array().unwrap();
    Polygon {
        exterior: ring(&rings[0]),
        interiors: rings[1..].iter().map(ring).collect(),
    }
}

fn geometry(v: &Value) -> Geometry {
    match v["type"].as_str().unwrap() {
        "Polygon" => Geometry::Polygon(polygon(&v["coordinates"])),
        "MultiPolygon" => Geometry::MultiPolygon(
            v["coordinates"].as_array().unwrap().iter().map(polygon).collect(),
        ),
        other => panic!("unexpected geometry {other}"),
    }
}

fn case_files() -> Vec<std::path::PathBuf> {
    let dir = format!("{}/../../conformance/cases/shapes", env!("CARGO_MANIFEST_DIR"));
    let mut out: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {dir}: {e}\nrun conformance/tools/gen_shapes.py"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && !p.to_string_lossy().contains("-published")
        })
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no shape fixtures in {dir}");
    out
}

/// Area of a quadrilateral, so the rectangle can be compared without
/// depending on which corner each implementation lists first.
fn quad_area(q: &[Point; 4]) -> f64 {
    let mut s = 0.0;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s.abs() / 2.0
}

#[test]
fn shape_compactness_matches_rdapy() {
    let mut w = Worst::default();
    let mut shapes = 0usize;

    for path in case_files() {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let set = doc["set"].as_str().unwrap_or("?");

        for case in doc["cases"].as_array().unwrap() {
            let id = case["id"].as_str().unwrap_or("?");
            let at = format!("{set}/{id}");
            let g = geometry(&case["geometry"]);
            shapes += 1;

            for (geodesic, key) in [(false, "projected"), (true, "geodesic")] {
                let a = polygon_attributes(&g, geodesic);
                let e = &case[key];
                w.note(&format!("{key} area"), a.area, e["area"].as_f64().unwrap(), &at);
                w.note(&format!("{key} perimeter"), a.perimeter, e["perimeter"].as_f64().unwrap(), &at);
                w.note(&format!("{key} diameter"), a.diameter, e["diameter"].as_f64().unwrap(), &at);
            }

            // Compared by area: the corner listed first is arbitrary.
            let pts: Vec<Point> = g
                .rings()
                .filter(|(is_ext, _)| *is_ext)
                .flat_map(|(_, r)| r.iter().copied())
                .collect();
            let want_rect = ring(&case["min_bounding_rectangle"]);
            let want: [Point; 4] = [want_rect[0], want_rect[1], want_rect[2], want_rect[3]];
            w.note(
                "bounding rectangle area",
                quad_area(&minimum_bounding_rectangle(&pts)),
                quad_area(&want),
                &at,
            );

            let f = featureize_shape(&g, true);
            let names = ["sym_x", "sym_y", "reock", "bbox", "polsby", "hull", "schwartzberg"];
            for (i, name) in names.iter().enumerate() {
                w.note(name, f[i], case["features"][name].as_f64().unwrap(), &at);
            }

            w.note("projected reock", calc_reock(&g, false),
                   case["projected_reock"].as_f64().unwrap(), &at);
            w.note("projected polsby", calc_polsby(&g, false),
                   case["projected_polsby"].as_f64().unwrap(), &at);
            w.note("kiwysi (revised)", kiwysi_rank_shape(&g, true, true),
                   case["kiwysi_revised"].as_f64().unwrap(), &at);
            w.note("kiwysi (original)", kiwysi_rank_shape(&g, true, false),
                   case["kiwysi_original"].as_f64().unwrap(), &at);
        }
    }

    eprintln!("shape compactness, {shapes} shapes -- worst relative error:");
    w.report();

    // Everything but the bounding rectangle is arithmetic on the same
    // coordinates, including the symmetry features, which go through a
    // polygon union in a different clipping library entirely.
    const TIGHT: f64 = 1e-8;
    for what in [
        "projected area", "projected perimeter", "projected diameter",
        "projected reock", "projected polsby",
        "geodesic area", "geodesic perimeter", "geodesic diameter",
        "sym_x", "sym_y", "reock", "polsby", "hull", "schwartzberg",
    ] {
        let e = w.worst_of(what);
        assert!(e <= TIGHT, "{what}: {e:e}");
    }

    // The bounding rectangle is the one place this deliberately disagrees.
    // rdapy builds its edge list from an unclosed hull, dropping the hull's
    // closing edge, and on some shapes that dropped edge is the one giving
    // the true minimum -- so its rectangle can be up to about 0.8% too
    // large. See KNOWN-DIFFERENCES.md. Ours is the real minimum; the bound
    // here is a regression check, not an endorsement of the gap.
    const RECTANGLE: f64 = 1e-2;
    for what in ["bbox", "bounding rectangle area"] {
        let e = w.worst_of(what);
        assert!(e <= RECTANGLE, "{what}: {e:e}");
    }
    // Which moves the rank by well under a tenth of a point.
    for what in ["kiwysi (revised)", "kiwysi (original)"] {
        let e = w.worst_of(what);
        assert!(e <= 5e-3, "{what}: {e:e}");
    }
}

/// rdapy's own KIWYSI tests: the ranks must match the model authors'
/// published predictions to within one rank out of a hundred.
#[test]
fn kiwysi_ranks_match_the_published_predictions() {
    let dir = format!("{}/../../conformance/cases/shapes", env!("CARGO_MANIFEST_DIR"));

    for set in ["first20", "evenlyspaced20"] {
        let shapes: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{dir}/{set}.json")).unwrap(),
        )
        .unwrap();
        let published: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{dir}/{set}-published.json")).unwrap(),
        )
        .unwrap();
        let revised = published["revised_model"].as_bool().unwrap();

        let cases = shapes["cases"].as_array().unwrap();
        let want = published["cases"].as_array().unwrap();
        assert_eq!(cases.len(), want.len(), "{set}: shape count");

        let mut worst = 0.0f64;
        for (case, w) in cases.iter().zip(want.iter()) {
            let g = geometry(&case["geometry"]);
            let got = kiwysi_rank_shape(&g, true, revised);
            let expect = w["expect"].as_f64().unwrap();
            worst = worst.max((got - expect).abs());
            assert!(
                (got - expect).abs() <= 1.0,
                "{set} shape {}: rank {got:.3} against published {expect:.3}",
                case["id"].as_str().unwrap_or("?")
            );
        }
        eprintln!("{set}: worst departure from the published rank {worst:.4} (allowed 1.0)");
    }
}

/// rdapy's compactness test: the North Carolina 116th plan, against the
/// figures DRA published for it.
#[test]
fn nc_116th_compactness_matches_the_published_figures() {
    let dir = format!("{}/../../conformance/cases/shapes", env!("CARGO_MANIFEST_DIR"));
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{dir}/NC-116th-Congressional.json")).unwrap(),
    )
    .unwrap();
    let shapes: Vec<Geometry> = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| geometry(&c["geometry"]))
        .collect();

    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/../../vendor/rdapy/testdata/compactness/NC-116th-Congressional/expected.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();

    let got = rdarust_geo::shapes::calc_compactness_metrics(&shapes, true);

    // rdapy asserts these to two decimal places.
    let close = |a: f64, b: f64| (a - b).abs() <= 0.5e-2;
    assert!(
        close(got.avg_reock, expected["avgReock"].as_f64().unwrap()),
        "average Reock {} against {}", got.avg_reock, expected["avgReock"]
    );
    assert!(
        close(got.avg_polsby, expected["avgPolsby"].as_f64().unwrap()),
        "average Polsby-Popper {} against {}", got.avg_polsby, expected["avgPolsby"]
    );
    assert!(
        close(got.avg_kiwysi.unwrap() as f64, expected["avgKIWYSI"].as_f64().unwrap()),
        "average KIWYSI {:?} against {}", got.avg_kiwysi, expected["avgKIWYSI"]
    );

    for (i, d) in got.by_district.iter().enumerate() {
        let e = &expected["byDistrict"][i];
        assert!(close(d.reock, e["reock"].as_f64().unwrap()), "district {i} Reock");
        assert!(close(d.polsby, e["polsby"].as_f64().unwrap()), "district {i} Polsby");
        let want = e["kiwysiRank"].as_f64().unwrap();
        let got_rank = d.kiwysi_rank.unwrap();
        assert!(
            (got_rank.round() - want.round()).abs() <= 1.0,
            "district {i} KIWYSI rank {got_rank} against {want}"
        );
    }
    eprintln!(
        "NC 116th: Reock {:.4}, Polsby {:.4}, KIWYSI {:?}",
        got.avg_reock, got.avg_polsby, got.avg_kiwysi
    );
}
