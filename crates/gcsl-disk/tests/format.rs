//! The formatter's invariants over every `.gcsl` file there is (see
//! `gcsl::fmt`): each file that parses formats, and formatting it again
//! changes nothing; `gcsl::fmt::format` checks the rest itself, that the
//! tokens, the comments and the syntax tree are the input's, and fails
//! otherwise. Files are read and formatted in memory: nothing is written.
//!
//! The repository's own files are checked by `cargo test`; Jack's newpkgs,
//! which is only read, pekit's fixtures and tree-sitter-gcsl's test files
//! by hand (`cargo test -p gcsl-disk --test format -- --ignored
//! --nocapture`), which also prints how many files and lines would change,
//! by kind of change, and the changes of the first files that would.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use gcsl::SourceFile;
use gcsl::fmt::format;

/// Jack's workspace of packages, read and never written.
const NEWPKGS: &str = "/home/jack/projects/peios-old/newpkgs";

/// Every `.gcsl` file under `dir`, but in directories whose names start
/// with `.` or are `target`.
fn gcsl_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                gcsl_files(&path, found);
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "gcsl")
        {
            found.push(path);
        }
    }
}

/// The lines of `old` and `new` that differ, as a longest common
/// subsequence of lines leaves them: each run of lines of `old` replaced by
/// a run of lines of `new`.
fn changes<'a>(old: &'a str, new: &'a str) -> Vec<(Vec<&'a str>, Vec<&'a str>)> {
    let before: Vec<&str> = old.lines().collect();
    let after: Vec<&str> = new.lines().collect();
    let (ends, ends_after) = (before.len(), after.len());
    // `common[i][j]`: the longest common subsequence of `before[i..]` and
    // `after[j..]`.
    let mut common = vec![vec![0_u32; ends_after + 1]; ends + 1];
    for i in (0..ends).rev() {
        for j in (0..ends_after).rev() {
            common[i][j] = if before[i] == after[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut hunks = Vec::new();
    let mut hunk: (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    while i < ends || j < ends_after {
        if i < ends && j < ends_after && before[i] == after[j] {
            if !hunk.0.is_empty() || !hunk.1.is_empty() {
                hunks.push(std::mem::take(&mut hunk));
            }
            i += 1;
            j += 1;
        } else if j < ends_after && (i == ends || common[i][j + 1] >= common[i + 1][j]) {
            hunk.1.push(after[j]);
            j += 1;
        } else {
            hunk.0.push(before[i]);
            i += 1;
        }
    }
    if !hunk.0.is_empty() || !hunk.1.is_empty() {
        hunks.push(hunk);
    }
    hunks
}

/// The kind of change from the line `old` to the line `new`.
fn kind(old: &str, new: &str) -> &'static str {
    let squeezed = |line: &str| line.split_whitespace().collect::<String>();
    if old.trim_end() == new {
        "trailing whitespace"
    } else if old.trim() == new.trim() {
        "indentation"
    } else if squeezed(old) == squeezed(new) {
        if old.contains('#') && old.trim_start().starts_with('#') {
            "comment indentation"
        } else if old.contains(" #") || old.contains("\t#") {
            "spacing (maybe of a comment)"
        } else {
            "spacing"
        }
    } else if format!("{},", squeezed(old)) == squeezed(new) {
        "trailing comma"
    } else if squeezed(old).trim_end_matches(';') == squeezed(new) {
        "semicolon removed"
    } else {
        "other"
    }
}

/// What a run over a set of files found.
#[derive(Debug, Default)]
struct Found {
    /// Files read.
    files: usize,
    /// Of them, those with errors, left alone.
    with_errors: Vec<String>,
    /// Of the others, those that would change.
    changed: usize,
    /// Lines of the input changed or removed, and lines added.
    lines: (usize, usize),
    /// Changed lines by kind of change.
    kinds: std::collections::BTreeMap<&'static str, usize>,
    /// Failures: formatting failed on a file that parses, or was not
    /// idempotent.
    failures: Vec<String>,
    /// The changes of the files that would change, for showing.
    shown: Vec<String>,
}

impl Found {
    /// Formats each of `paths`, recording what it finds.
    fn run(paths: &[PathBuf]) -> Self {
        let mut found = Self::default();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            found.files += 1;
            let name = path.display().to_string();
            let source = SourceFile::new(name.clone(), text.as_str()).unwrap();
            let formatted = match format(&source) {
                Ok(formatted) => formatted,
                Err(diagnostics) => {
                    if diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.message.starts_with("internal error"))
                    {
                        found
                            .failures
                            .push(format!("{name}: {}", diagnostics[0].message));
                    } else {
                        found.with_errors.push(name);
                    }
                    continue;
                }
            };
            let again = SourceFile::new(name.clone(), formatted.as_str()).unwrap();
            match format(&again) {
                Ok(twice) if twice == formatted => {}
                Ok(_) => found.failures.push(format!("{name}: not idempotent")),
                Err(diagnostics) => found
                    .failures
                    .push(format!("{name}: again: {}", diagnostics[0].message)),
            }
            if formatted == text {
                continue;
            }
            found.changed += 1;
            let mut shown = format!("--- {name}\n");
            for (old, new) in changes(&text, &formatted) {
                found.lines.0 += old.len();
                found.lines.1 += new.len();
                for line in &old {
                    writeln!(shown, "-{line}").unwrap();
                }
                for line in &new {
                    writeln!(shown, "+{line}").unwrap();
                }
                shown.push_str("@@\n");
                if old.len() == new.len() {
                    for (old, new) in old.iter().zip(&new) {
                        *found.kinds.entry(kind(old, new)).or_default() += 1;
                    }
                } else if new.is_empty() && old.iter().all(|line| line.trim().is_empty()) {
                    *found.kinds.entry("blank line removed").or_default() += old.len();
                } else if old.is_empty() && new.iter().all(|line| line.trim().is_empty()) {
                    *found.kinds.entry("blank line added").or_default() += new.len();
                } else {
                    *found.kinds.entry("other (uneven)").or_default() += old.len().max(new.len());
                }
            }
            found.shown.push(shown);
        }
        found
    }

    /// Prints what was found, for `set`, with the changes of up to `show`
    /// files, and fails on any failure.
    fn report(&self, set: &str, show: usize) {
        println!(
            "{set}: {} files, {} with errors left alone, {} would change, {} lines changed or removed and {} added; by kind {:?}; failures {:?}",
            self.files,
            self.with_errors.len(),
            self.changed,
            self.lines.0,
            self.lines.1,
            self.kinds,
            self.failures,
        );
        for shown in self.shown.iter().take(show) {
            println!("{shown}");
        }
        assert!(self.failures.is_empty(), "{:?}", self.failures);
    }
}

/// Mutants of `text`: with one token deleted, at 25 tokens evenly spaced
/// among its tokens; with every space doubled; and with every line's
/// indentation removed.
fn mutants(text: &str) -> Vec<String> {
    let source = SourceFile::new("t.gcsl", text).unwrap();
    let lexed = gcsl::lexer::lex(&source, &mut gcsl::names::Names::new());
    let tokens: Vec<_> = lexed
        .tokens
        .iter()
        .filter(|token| token.span.end > token.span.start)
        .collect();
    let mut mutants: Vec<String> = (0..25)
        .filter_map(|at| tokens.get(tokens.len() * at / 25))
        .map(|token| {
            let (start, end) = (token.span.start as usize, token.span.end as usize);
            format!("{}{}", &source.text()[..start], &source.text()[end..])
        })
        .collect();
    mutants.push(source.text().replace(' ', "  "));
    mutants.push(
        source
            .text()
            .lines()
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    mutants
}

/// Formats each mutant of each of `paths` ([`mutants`]): each that parses
/// must format, to a text that formats to itself, and none may fail with
/// an internal error. Returns the number of mutants, those that formatted,
/// and the failures.
fn run_mutants(paths: &[PathBuf]) -> (usize, usize, Vec<String>) {
    let (mut count, mut formatted, mut failures) = (0, 0, Vec::new());
    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (at, mutant) in mutants(&text).into_iter().enumerate() {
            count += 1;
            let what = format!("{} mutant {at}", path.display());
            let source = SourceFile::new("m.gcsl", mutant.as_str()).unwrap();
            match format(&source) {
                Ok(once) => {
                    formatted += 1;
                    let again = SourceFile::new("m.gcsl", once.as_str()).unwrap();
                    if format(&again).ok().as_ref() != Some(&once) {
                        failures.push(format!("{what}: not idempotent"));
                    }
                }
                Err(diagnostics) => {
                    if let Some(internal) = diagnostics
                        .iter()
                        .find(|diagnostic| diagnostic.message.starts_with("internal error"))
                    {
                        failures.push(format!("{what}: {}", internal.message));
                    }
                }
            }
        }
    }
    (count, formatted, failures)
}

/// The repository's own `.gcsl` files: its tests' and its standard
/// library's.
fn repository_files() -> Vec<PathBuf> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    gcsl_files(&repository.join("crates"), &mut files);
    gcsl_files(&repository.join("stdlib"), &mut files);
    files
}

#[test]
fn every_file_of_the_repository_that_parses_formats_and_formats_to_itself_again() {
    let files = repository_files();
    assert!(files.len() > 100);
    let found = Found::run(&files);
    found.report("gcsl", 0);
}

#[test]
#[ignore = "reads newpkgs and the sibling repositories; run by hand with --nocapture"]
fn every_file_of_newpkgs_pekit_and_tree_sitter_formats_and_formats_to_itself_again() {
    let shown = std::env::var("GCSL_FMT_SHOW")
        .ok()
        .and_then(|show| show.parse().ok())
        .unwrap_or(0);
    let peios = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut sets = Vec::new();
    sets.push(("gcsl", repository_files()));
    for (set, dir) in [
        ("pekit", peios.join("pekit-gcsl/tests")),
        ("tree-sitter-gcsl", peios.join("tree-sitter-gcsl/test")),
        ("newpkgs", PathBuf::from(NEWPKGS)),
    ] {
        let mut files = Vec::new();
        gcsl_files(&dir, &mut files);
        assert!(!files.is_empty(), "no files under {}", dir.display());
        sets.push((set, files));
    }
    let mut failed = Vec::new();
    for (set, files) in &sets {
        let found = Found::run(files);
        println!("{set}: left alone with errors: {:?}", found.with_errors);
        if std::panic::catch_unwind(|| found.report(set, shown)).is_err() {
            failed.push(*set);
        }
        let (count, formatted, failures) = run_mutants(files);
        println!("{set}: {count} mutants, {formatted} formatted, failures {failures:?}");
        if !failures.is_empty() {
            failed.push(*set);
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}
