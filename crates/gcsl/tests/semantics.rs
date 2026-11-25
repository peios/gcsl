//! The guard of [`gcsl::SEMANTICS`]: the sources of what GCSL code means
//! beyond its text are those `semantics.digest` was written for.
//!
//! When this fails, either raise `SEMANTICS` and write the digest again, if
//! what code means changed, or write the digest again with a line saying
//! why it did not, as `semantics.digest` shows.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

/// The sources, under `src/`, whose change can change what code means.
const SOURCES: [&str; 7] = [
    "eval",
    "eval.rs",
    "lower",
    "lower.rs",
    "value",
    "value.rs",
    "check/builtin.rs",
];

/// Every `.rs` file at or under `path`.
fn rust_files(path: &Path, found: &mut Vec<PathBuf>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            rust_files(&entry.unwrap().path(), found);
        }
    } else if path.extension().is_some_and(|extension| extension == "rs") {
        found.push(path.to_path_buf());
    }
}

/// The digest of the sources, each by its path under `src/` and its text,
/// in path order.
fn digest() -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for source in SOURCES {
        rust_files(&src.join(source), &mut files);
    }
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let name = file
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hasher.update(
            fs::read_to_string(&file)
                .unwrap()
                .replace("\r\n", "\n")
                .as_bytes(),
        );
        hasher.update(b"\0");
    }
    let mut hex = String::new();
    for byte in hasher.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[test]
fn what_code_means_is_versioned() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("semantics.digest");
    let written = fs::read_to_string(&path).unwrap_or_default();
    let field = |key: &str| {
        written
            .lines()
            .find_map(|line| line.strip_prefix(key).map(str::trim).map(str::to_owned))
    };
    let now = digest();
    assert_eq!(
        field("semantics ").as_deref(),
        Some(gcsl::SEMANTICS.to_string().as_str()),
        "semantics.digest is of another SEMANTICS than {}",
        gcsl::SEMANTICS
    );
    assert_eq!(
        field("digest ").as_deref(),
        Some(now.as_str()),
        "the evaluator, lowering, values or built-ins changed: raise gcsl::SEMANTICS if what \
         code means changed, and write `digest {now}` into semantics.digest, with a line \
         saying why if it did not"
    );
}
