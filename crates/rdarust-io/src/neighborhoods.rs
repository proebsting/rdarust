//! Reading and writing precinct neighbourhoods.
//!
//! A neighbourhood is a set of precincts, stored as a bit per precinct rather
//! than a list of geoids: a state has a few thousand precincts and every one
//! of them has a neighbourhood, so a list of names would be two orders of
//! magnitude larger than a bitmap.
//!
//! Bit positions index the state's geoids **in sorted order**, which is what
//! ties this format to `Context::sorted_precinct_order`.

use std::io::{BufRead, Write};

use rdarust_core::context::Context;
use serde_json::{json, Map, Value};

use crate::records::to_python_json;
use crate::LoadError;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= if c == b'=' { 0 } else { value(c)? } << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// Pack a set of positions into the nested JSON string the format uses.
///
/// The value is itself a JSON document rather than an object, which is how
/// rdapy writes it.
pub fn pack_neighborhood(positions: &[u32], n_precincts: usize) -> String {
    let mut bits = vec![0u8; n_precincts.div_ceil(8)];
    for &p in positions {
        bits[p as usize / 8] |= 1 << (p as usize % 8);
    }

    let mut doc = Map::new();
    doc.insert("encoding".into(), json!("base64"));
    doc.insert("type".into(), json!("bit_array"));
    doc.insert("length".into(), json!(n_precincts));
    doc.insert("data".into(), json!(b64_encode(&bits)));

    String::from_utf8(to_python_json(&Value::Object(doc)).expect("serialising a bit array"))
        .expect("JSON is valid UTF-8")
}

/// Unpack the positions a neighbourhood holds.
pub fn unpack_neighborhood(packed: &str) -> Result<Vec<u32>, LoadError> {
    let doc: Value = serde_json::from_str(packed)
        .map_err(|source| LoadError::Json { line: 0, source })?;
    let data = doc
        .get("data")
        .and_then(|d| d.as_str())
        .ok_or(LoadError::Malformed { line: 0, what: "bit array has no data".into() })?;
    let bytes = b64_decode(data).ok_or(LoadError::Malformed {
        line: 0,
        what: "bit array data is not valid base64".into(),
    })?;

    let mut out = Vec::new();
    for (i, byte) in bytes.iter().enumerate() {
        for bit in 0..8 {
            if byte & (1 << bit) != 0 {
                out.push((i * 8 + bit) as u32);
            }
        }
    }
    Ok(out)
}

/// One precinct's neighbourhood, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NeighborhoodRecord {
    pub geoid: String,
    /// Positions in the sorted geoid list.
    pub positions: Vec<u32>,
}

/// Write neighbourhoods, one JSON record per line.
///
/// `neighborhoods` pairs each seed with its members as precinct indices;
/// both are translated to sorted-order positions here.
pub fn write_neighborhoods(
    w: &mut dyn Write,
    ctx: &Context,
    neighborhoods: &[(u32, Vec<u32>)],
) -> std::io::Result<()> {
    let order = ctx.sorted_precinct_order();
    let mut position = vec![0u32; ctx.n_precincts()];
    for (pos, &i) in order.iter().enumerate() {
        position[i as usize] = pos as u32;
    }
    let n = order.len();

    for (seed, members) in neighborhoods {
        let mut positions: Vec<u32> =
            members.iter().map(|&m| position[m as usize]).collect();
        positions.sort_unstable();

        // Size and checksum let a reader detect a truncated or corrupted
        // bitmap without unpacking it against the geoid list.
        let checksum: i64 = positions.iter().map(|&p| p as i64).sum();

        let mut rec = Map::new();
        rec.insert("geoid".into(), json!(ctx.geoids[*seed as usize]));
        rec.insert("size".into(), json!(positions.len()));
        rec.insert("checksum".into(), json!(checksum));
        rec.insert("neighborhood".into(), json!(pack_neighborhood(&positions, n)));

        w.write_all(&to_python_json(&Value::Object(rec))?)?;
        w.write_all(b"\n")?;
    }
    Ok(())
}

/// Read neighbourhoods, checking each against its recorded size and checksum.
///
/// Returns them as precinct indices, paired with their seed, in file order --
/// which the baseline sums over, so the order is preserved.
pub fn read_neighborhoods(
    reader: impl BufRead,
    ctx: &Context,
) -> Result<Vec<(u32, Vec<u32>)>, LoadError> {
    let order = ctx.sorted_precinct_order();
    let mut out = Vec::new();

    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: Value = serde_json::from_str(&line)
            .map_err(|source| LoadError::Json { line: n + 1, source })?;

        let geoid = rec
            .get("geoid")
            .and_then(|g| g.as_str())
            .ok_or(LoadError::Malformed { line: n + 1, what: "record has no geoid".into() })?;
        let seed = *ctx.index.get(geoid).ok_or_else(|| LoadError::Malformed {
            line: n + 1,
            what: format!("{geoid} is not a precinct in this state"),
        })?;

        let packed = rec.get("neighborhood").and_then(|v| v.as_str()).ok_or(
            LoadError::Malformed { line: n + 1, what: "record has no neighborhood".into() },
        )?;
        let positions = unpack_neighborhood(packed)?;

        if let Some(size) = rec.get("size").and_then(|v| v.as_u64()) {
            if positions.len() as u64 != size {
                return Err(LoadError::Malformed {
                    line: n + 1,
                    what: format!(
                        "{geoid}: neighbourhood holds {} precincts, not the {size} recorded",
                        positions.len()
                    ),
                });
            }
        }
        if let Some(checksum) = rec.get("checksum").and_then(|v| v.as_i64()) {
            let actual: i64 = positions.iter().map(|&p| p as i64).sum();
            if actual != checksum {
                return Err(LoadError::Malformed {
                    line: n + 1,
                    what: format!("{geoid}: checksum {actual}, not the {checksum} recorded"),
                });
            }
        }

        let members: Vec<u32> = positions
            .iter()
            .map(|&p| {
                order.get(p as usize).copied().ok_or_else(|| LoadError::Malformed {
                    line: n + 1,
                    what: format!("{geoid}: position {p} is past the end of the state"),
                })
            })
            .collect::<Result<_, _>>()?;

        if !members.contains(&seed) {
            return Err(LoadError::Malformed {
                line: n + 1,
                what: format!("{geoid} is not in its own neighbourhood"),
            });
        }

        out.push((seed, members));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for len in 0..40usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let encoded = b64_encode(&bytes);
            assert_eq!(encoded.len() % 4, 0, "length {len} must pad to a multiple of 4");
            assert_eq!(b64_decode(&encoded).as_deref(), Some(&bytes[..]), "length {len}");
        }
    }

    #[test]
    fn base64_matches_known_vectors() {
        // RFC 4648 section 10, which pins the padding cases.
        for (plain, encoded) in [
            ("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(b64_encode(plain.as_bytes()), encoded, "encoding {plain:?}");
            assert_eq!(
                b64_decode(encoded).as_deref(),
                Some(plain.as_bytes()),
                "decoding {encoded:?}"
            );
        }
    }

    #[test]
    fn packing_round_trips() {
        let positions: Vec<u32> = vec![0, 1, 7, 8, 9, 63, 64, 2665];
        let packed = pack_neighborhood(&positions, 2666);
        assert_eq!(unpack_neighborhood(&packed).unwrap(), positions);
    }

    #[test]
    fn packed_form_is_the_nested_document_rdapy_writes() {
        let packed = pack_neighborhood(&[0, 3], 16);
        assert!(
            packed.starts_with(r#"{"encoding": "base64", "type": "bit_array", "length": 16"#),
            "got {packed}"
        );
    }
}
