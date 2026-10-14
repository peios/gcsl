//! The lexer's and the parser's recovery from errors, run by hand in release
//! mode (`cargo test --release -p gcsl-disk --test recovery -- --ignored
//! --nocapture`): timings of the front end on valid input, and a robustness
//! run over mutants of every `.gcsl` file of the repository and of Jack's
//! newpkgs, which is only read.

use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gcsl::names::Names;
use gcsl::workspace;
use gcsl::{Diagnostic, SourceFile, lexer, parser};

/// Jack's workspace of packages, read and never written.
const NEWPKGS: &str = "/home/jack/projects/peios/newpkgs";

/// The fastest of `runs` runs of `work`.
fn fastest(runs: usize, mut work: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let start = Instant::now();
            work();
            start.elapsed()
        })
        .min()
        .unwrap_or_default()
}

/// A file of `count` fields, each read by the next.
fn fields(count: usize) -> String {
    let mut text = String::from("f0 = 1\n");
    for at in 1..count {
        writeln!(text, "f{at} = f{} + {at}", at - 1).unwrap();
    }
    text
}

/// A file of `count` functions, each calling the one before, and a call of
/// the last.
fn functions(count: usize) -> String {
    let mut text = String::from("def g0(x: Int): Int { x + 1 }\n");
    for at in 1..count {
        writeln!(
            text,
            "def g{at}(x: Int, y: Int = 2): Int {{\n    let z = g{}(x)\n    if z > y {{ z }} else {{ y * {at} }}\n}}",
            at - 1
        )
        .unwrap();
    }
    writeln!(text, "result = g{}(1)", count - 1).unwrap();
    text
}

/// A file of `count` records of strings and lists.
fn records(count: usize) -> String {
    let mut text = String::new();
    for at in 0..count {
        writeln!(
            text,
            "r{at}: {{name: String, tags: String[]}} = {{\n    name = \"item {{{at}}} of \\\"many\\\"\"\n    tags = [\"a\", \"b-{at}\", r\"raw\"]\n}}"
        )
        .unwrap();
    }
    text
}

/// Lexes and parses `text`, failing on any error.
fn parse(source: &SourceFile) {
    let (tokens, names, strings) = lexer::tokenize(source).unwrap();
    parser::parse(&tokens, names, strings, source).unwrap();
}

#[test]
#[ignore = "a timing, run by hand in release mode"]
fn timings() {
    for (name, text) in [
        ("fields", fields(20_000)),
        ("functions", functions(5_000)),
        ("records", records(5_000)),
    ] {
        let source = SourceFile::new(format!("{name}.gcsl"), text).unwrap();
        let front = fastest(20, || parse(&source));
        let whole = fastest(10, || {
            workspace::check_lone(&source).unwrap();
        });
        println!(
            "{name}: {} bytes, lex+parse {front:?}, check {whole:?}",
            source.len()
        );
    }
    let root = Path::new(NEWPKGS);
    let sources = gcsl_disk::read_workspace(root, None).unwrap();
    let checked = fastest(5, || {
        let (_, checked) =
            workspace::check_with_standard(&sources.relative, &sources.nested, NEWPKGS);
        checked.unwrap();
    });
    println!(
        "newpkgs: {} files, check {checked:?}",
        sources.relative.len()
    );
}

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
            if !name.starts_with('.') && name != "target" {
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

/// The mutants of `text`: cut at 50 offsets evenly spaced through it,
/// each moved back to a character's boundary, and with one token deleted,
/// at 50 tokens evenly spaced among its tokens.
fn mutants(text: &str) -> (Vec<String>, Vec<String>) {
    let cuts = (1..=50)
        .map(|at| {
            let mut end = text.len() * at / 51;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text[..end].to_owned()
        })
        .collect();
    let source = SourceFile::new("t.gcsl", text).unwrap();
    let lexed = lexer::lex(&source, &mut Names::new());
    let tokens: Vec<_> = lexed
        .tokens
        .iter()
        .filter(|token| token.kind != lexer::TokenKind::Eof && token.span.end > token.span.start)
        .collect();
    let deletions = if tokens.is_empty() {
        Vec::new()
    } else {
        (0..50)
            .map(|at| {
                let token = tokens[tokens.len() * at / 50];
                let (start, end) = (token.span.start as usize, token.span.end as usize);
                format!("{}{}", &text[..start], &text[end..])
            })
            .collect()
    };
    (cuts, deletions)
}

/// What the robustness run found.
#[derive(Debug, Default)]
struct Found {
    /// Mutants parsed and checked.
    mutants: usize,
    /// Of them, those with any error.
    with_errors: usize,
    /// Of those, the syntax errors alone, for an average.
    syntax_errors: usize,
    /// Panics, by what was run.
    panics: Vec<String>,
    /// Internal errors, by what was run.
    internal: Vec<String>,
    /// The slowest mutant and its time.
    slowest: (Duration, String),
}

impl Found {
    /// Runs `work`, the check of the mutant `what`, and records what it
    /// found.
    fn run(
        &mut self,
        what: &str,
        work: impl FnOnce() -> Result<(), Vec<Diagnostic>>,
        syntax: usize,
    ) {
        let start = Instant::now();
        let outcome = panic::catch_unwind(AssertUnwindSafe(work));
        let took = start.elapsed();
        self.mutants += 1;
        if took > self.slowest.0 {
            self.slowest = (took, what.to_owned());
        }
        match outcome {
            Err(_) => self.panics.push(what.to_owned()),
            Ok(Ok(())) => {}
            Ok(Err(diagnostics)) => {
                self.with_errors += 1;
                self.syntax_errors += syntax;
                if diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.notes.iter().any(|note| note.contains("a bug")))
                {
                    self.internal.push(what.to_owned());
                }
            }
        }
    }
}

#[test]
#[ignore = "a robustness run of minutes, run by hand in release mode"]
fn every_mutant_parses_and_checks_without_a_panic_or_an_internal_error() {
    let thread = std::thread::Builder::new()
        .stack_size(gcsl::STACK_SIZE)
        .spawn(robustness)
        .unwrap();
    thread.join().unwrap();
}

/// The robustness run: every mutant of every file alone, and two of each
/// of newpkgs's files in their workspace.
fn robustness() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut ours = Vec::new();
    gcsl_files(&repository.join("crates"), &mut ours);
    gcsl_files(&repository.join("stdlib"), &mut ours);
    let mut theirs = Vec::new();
    gcsl_files(Path::new(NEWPKGS), &mut theirs);
    for (set, files) in [("gcsl", &ours), ("newpkgs", &theirs)] {
        let mut found = Found::default();
        for path in files {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let (cuts, deletions) = mutants(&text);
            for (kind, mutant) in cuts
                .iter()
                .map(|m| ("cut", m))
                .chain(deletions.iter().map(|m| ("deletion", m)))
            {
                let what = format!("{} {kind} of {} bytes", path.display(), mutant.len());
                let source = SourceFile::new("mutant.gcsl", mutant.as_str()).unwrap();
                let syntax = parser::parse_source(&source).1.len();
                found.run(&what, || workspace::check_lone(&source).map(|_| ()), syntax);
            }
        }
        println!(
            "{set}: {} files, {} mutants alone, {} with errors ({} syntax errors), slowest {:?} ({}), panics {:?}, internal errors {:?}",
            files.len(),
            found.mutants,
            found.with_errors,
            found.syntax_errors,
            found.slowest.0,
            found.slowest.1,
            found.panics,
            found.internal
        );
        assert!(found.panics.is_empty() && found.internal.is_empty());
    }
    // Each of newpkgs's files cut in its middle, and with its middle token
    // deleted, in its workspace.
    let sources = gcsl_disk::read_workspace(Path::new(NEWPKGS), None).unwrap();
    let mut found = Found::default();
    for at in 0..sources.relative.len() {
        let original = &sources.relative[at];
        let (cuts, deletions) = mutants(original.text());
        for (kind, mutant) in [("cut", cuts.get(24)), ("deletion", deletions.get(25))] {
            let Some(mutant) = mutant else {
                continue;
            };
            let mut files = sources.relative.clone();
            files[at] = SourceFile::new(original.name(), mutant.as_str()).unwrap();
            let what = format!("{} {kind} in newpkgs", original.name());
            found.run(
                &what,
                || {
                    workspace::check_with_standard(&files, &sources.nested, NEWPKGS)
                        .1
                        .map(|_| ())
                },
                0,
            );
        }
    }
    println!(
        "newpkgs workspaces: {} mutants, {} with errors, slowest {:?} ({}), panics {:?}, internal errors {:?}",
        found.mutants,
        found.with_errors,
        found.slowest.0,
        found.slowest.1,
        found.panics,
        found.internal
    );
    assert!(found.panics.is_empty() && found.internal.is_empty());
}
