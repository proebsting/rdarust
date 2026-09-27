//! Opening the files named on the command line.
//!
//! Two shell conventions live here rather than in `rdarust-io`: `-` for the
//! standard streams, and a leading `~` for the home directory. Both are
//! things a *command* means by a path, not things a *format* needs, and
//! keeping them out of the library leaves it with no notion of a filesystem
//! beyond a handful of `File::open` wrappers -- which is what lets an
//! embedder feed it bytes instead.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

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
pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest);
        }
    }
    Path::new(path).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::expand;
    use std::path::Path;

    #[test]
    fn expand_only_touches_a_leading_tilde() {
        let home = std::env::var_os("HOME").expect("HOME is set");
        assert_eq!(expand("~/data/NC.jsonl"), Path::new(&home).join("data/NC.jsonl"));
        // Not a home reference: another user's home, a bare tilde, and a
        // tilde that is simply part of a name.
        assert_eq!(expand("~other/x"), Path::new("~other/x"));
        assert_eq!(expand("~"), Path::new("~"));
        assert_eq!(expand("data/~backup.jsonl"), Path::new("data/~backup.jsonl"));
    }
}
