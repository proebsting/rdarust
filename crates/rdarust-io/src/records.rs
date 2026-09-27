//! Tagged JSONL records, and stdin/stdout plumbing.
//!
//! The pipeline stages exchange one JSON object per line, each carrying a
//! `_tag_` saying what it is. A stage passes `metadata` through untouched,
//! acts on the records it understands, and ignores the rest -- so an ensemble
//! file can carry an adjacency graph or provenance alongside its plans.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// Read from a file, or stdin when the path is absent or `-`.
pub fn smart_reader(path: Option<&str>) -> io::Result<Box<dyn BufRead>> {
    match path {
        None | Some("-") => Ok(Box::new(BufReader::new(io::stdin()))),
        Some(p) => Ok(Box::new(BufReader::new(File::open(expand(p))?))),
    }
}

/// Write to a file, or stdout when the path is absent or `-`.
pub fn smart_writer(path: Option<&str>) -> io::Result<Box<dyn Write>> {
    match path {
        None | Some("-") => Ok(Box::new(BufWriter::new(io::stdout()))),
        Some(p) => Ok(Box::new(BufWriter::new(File::create(expand(p))?))),
    }
}

/// Expand a leading `~`, which the shell will not have done for a path that
/// arrived quoted.
pub fn expand(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest);
        }
    }
    Path::new(path).to_path_buf()
}

/// Python's default JSON separators.
///
/// `json.dump` with no `indent` writes `", "` between items and `": "` after
/// a key; serde_json writes neither. Matching it keeps the JSONL the pipeline
/// stages exchange byte-identical to rdapy's, which is what makes a plain
/// `diff` a useful check.
#[derive(Clone, Debug, Default)]
pub struct PythonFormatter;

impl serde_json::ser::Formatter for PythonFormatter {
    fn begin_array_value<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }

    fn begin_object_key<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }

    fn begin_object_value<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }
}

/// Serialise a value the way Python's `json.dumps` would.
pub fn to_python_json(value: &serde_json::Value) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, PythonFormatter);
    serde::Serialize::serialize(value, &mut ser).map_err(io::Error::other)?;
    Ok(buf)
}

/// Write one record as a JSONL line.
///
/// Keys stay in insertion order, which is what fixes the column order of the
/// scores CSV downstream.
pub fn write_record(w: &mut dyn Write, record: &serde_json::Value) -> io::Result<()> {
    w.write_all(&to_python_json(record)?)?;
    w.write_all(b"\n")
}

/// Write one record with its keys sorted, as rdapy's `write_record` does for
/// the by-district output.
pub fn write_record_sorted(w: &mut dyn Write, record: &serde_json::Value) -> io::Result<()> {
    write_record(w, &sorted(record))
}

fn sorted(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), sorted(&m[k]));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(sorted).collect())
        }
        other => other.clone(),
    }
}
