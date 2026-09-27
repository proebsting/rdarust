//! Reading DRA's GeoJSON.

use rdarust_geo::{Geometry, Point, Polygon, Ring};
use serde_json::Value;

use crate::LoadError;

/// One GeoJSON feature: its properties and its shape.
pub struct Feature {
    pub properties: Value,
    pub geometry: Geometry,
    /// The geometry exactly as it appeared, which `extract-data` copies
    /// through to its output.
    pub raw_geometry: Value,
}

fn ring(v: &Value) -> Option<Ring> {
    Some(
        v.as_array()?
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?] as Point)
            })
            .collect(),
    )
}

fn polygon(v: &Value) -> Option<Polygon> {
    let rings = v.as_array()?;
    let mut it = rings.iter();
    Some(Polygon {
        exterior: ring(it.next()?)?,
        interiors: it.filter_map(ring).collect(),
    })
}

/// Parse a geometry. Only polygons and multipolygons occur in precinct data.
pub fn parse_geometry(v: &Value) -> Option<Geometry> {
    match v.get("type")?.as_str()? {
        "Polygon" => Some(Geometry::Polygon(polygon(v.get("coordinates")?)?)),
        "MultiPolygon" => Some(Geometry::MultiPolygon(
            v.get("coordinates")?
                .as_array()?
                .iter()
                .filter_map(polygon)
                .collect(),
        )),
        _ => None,
    }
}

/// Read a GeoJSON file's features, in file order.
pub fn load_geojson(path: impl AsRef<std::path::Path>) -> Result<Vec<Feature>, LoadError> {
    load_geojson_from(std::fs::File::open(path.as_ref())?)
}

/// Read GeoJSON features from an open reader, in file order.
///
/// The whole document is read in before parsing: DRA publishes one JSON
/// object per state, so there is nothing to stream.
pub fn load_geojson_from(mut reader: impl std::io::Read) -> Result<Vec<Feature>, LoadError> {
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    let doc: Value = serde_json::from_str(&text)
        .map_err(|source| LoadError::Json { line: 0, source })?;
    features_of(&doc)
}

/// The features of an already-parsed GeoJSON document, in file order.
pub fn features_of(doc: &Value) -> Result<Vec<Feature>, LoadError> {
    let features = doc
        .get("features")
        .and_then(|f| f.as_array())
        .ok_or(LoadError::Malformed {
            line: 0,
            what: "GeoJSON has no features array".into(),
        })?;

    features
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let raw = f.get("geometry").cloned().unwrap_or(Value::Null);
            let geometry = parse_geometry(&raw).ok_or_else(|| LoadError::Malformed {
                line: i + 1,
                what: "feature has no polygon geometry".into(),
            })?;
            Ok(Feature {
                properties: f.get("properties").cloned().unwrap_or(Value::Null),
                geometry,
                raw_geometry: raw,
            })
        })
        .collect()
}
