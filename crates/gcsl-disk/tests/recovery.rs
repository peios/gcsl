//! The lexer's and the parser's recovery from errors, run by hand in release
//! mode (`cargo test --release -p gcsl-disk --test recovery -- --ignored
//! --nocapture`): timings of the front end on valid input, and a robustness
//! run over mutants of every `.gcsl` file of the repository and of Jack's
//! newpkgs, which is only read; and the same for the index a language
//! server answers its queries from (`gcsl::index`): timings of a check that
//! records for it, of a rename's check, of a workspace's symbols and of
//! doc comments, and a hover, a definition, references and a rename's
//! preparation at every token of newpkgs and of the mutants, with each
//! file's symbols (the run is still named for its first two); and the
//! scan's recovery from its own errors, over mutants of newpkgs in its
//! workspace that each make one: a field defined in two files, an import of
//! no module, files that need each other.

use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gcsl::index::{Index, Indexed};
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

#[test]
#[ignore = "a timing, run by hand in release mode"]
fn index_timings() {
    let thread = std::thread::Builder::new()
        .stack_size(gcsl::STACK_SIZE)
        .spawn(|| {
            let sources = gcsl_disk::read_workspace(Path::new(NEWPKGS), None).unwrap();
            let plain = fastest(10, || {
                let (_, checked) =
                    workspace::check_with_standard(&sources.relative, &sources.nested, NEWPKGS);
                checked.unwrap();
            });
            let recorded = fastest(10, || {
                let indexed =
                    workspace::index_with_standard(&sources.relative, &sources.nested, NEWPKGS);
                assert!(indexed.succeeded && indexed.index.is_some());
            });
            println!(
                "newpkgs: {} files, check {plain:?}, check recording and indexing {recorded:?}",
                sources.relative.len()
            );
        })
        .unwrap();
    thread.join().unwrap();
}

#[test]
#[ignore = "a timing, run by hand in release mode"]
fn rename_symbols_and_doc_comment_numbers() {
    let thread = std::thread::Builder::new()
        .stack_size(gcsl::STACK_SIZE)
        .spawn(slice_60_numbers)
        .unwrap();
    thread.join().unwrap();
}

/// The cost of a rename's check, of `workspace/symbol` and the doc comments
/// of newpkgs, printed.
fn slice_60_numbers() {
    let sources = gcsl_disk::read_workspace(Path::new(NEWPKGS), None).unwrap();
    let reindex =
        |files: &[SourceFile]| workspace::index_with_standard(files, &sources.nested, NEWPKGS);
    let start = Instant::now();
    let index = reindex(&sources.relative).index.unwrap();
    println!("newpkgs indexed in {:?}", start.elapsed());
    let standard = index.standard();
    let files = u32::try_from(sources.relative.len()).unwrap();
    // `workspace/symbol`: the first query makes the list, the others match.
    let start = Instant::now();
    let all = index.workspace_symbols("", usize::MAX).len();
    let first = start.elapsed();
    let mut times = Vec::new();
    for query in ["pkg", "SANDBOX", "versions", "zzqq", "b", "org.golang"] {
        let start = Instant::now();
        let found = index.workspace_symbols(query, 200).len();
        times.push((query, found, start.elapsed()));
    }
    println!("workspace/symbol: {all} symbols, first query {first:?}, then {times:?}");
    // Doc comments of the top-level declarations.
    let (mut declarations, mut documented, mut examples) = (0, 0, Vec::new());
    for file in standard..standard + files {
        for symbol in index.document_symbols(file) {
            if symbol.name == "imports" {
                continue;
            }
            declarations += 1;
            // At the name's last segment, the declaration's own node.
            if let Some(doc) = index.documentation(file, symbol.selection.end - 1) {
                documented += 1;
                if examples.len() < 2
                    || symbol.kind == gcsl::index::SymbolKind::Definition && examples.len() < 3
                {
                    let path = index.path(file).unwrap().to_owned();
                    examples.push((path, symbol.name.clone(), doc));
                }
            }
        }
    }
    println!(
        "doc comments: {documented} of {declarations} top-level declarations; examples {examples:#?}"
    );
    sandbox_broken(&sources.relative, &reindex);
    // A rename's check: a `def` of `_lib/sandbox`, used across the
    // packages, and the last segment of a `package` definition (with its
    // meta) and of a `build` one (referred to by its package).
    let mut targets: Vec<(u32, u32, &str)> = Vec::new();
    for file in standard..standard + files {
        let path = index.path(file).unwrap();
        for symbol in index.document_symbols(file) {
            let wanted = match symbol.kind {
                gcsl::index::SymbolKind::Function if path.starts_with("_lib/sandbox/") => "def",
                gcsl::index::SymbolKind::Definition if symbol.name.starts_with("package ") => {
                    "package"
                }
                gcsl::index::SymbolKind::Definition if symbol.name.starts_with("build ") => "build",
                _ => continue,
            };
            if targets.iter().all(|&(_, _, kind)| kind != wanted) {
                let at = symbol.selection.end - 1;
                targets.push((file, at, wanted));
            }
        }
    }
    for (file, at, kind) in targets {
        let references = index.references(file, at, true).len();
        let prepared = index.prepare_rename(file, at);
        let new = match &prepared {
            Ok(prepared) => format!("{}_renamed", prepared.placeholder),
            Err(_) => "renamed".to_owned(),
        };
        let start = Instant::now();
        let renamed = index.rename((file, at), &new, reindex);
        let took = start.elapsed();
        let shown = renamed.as_ref().map(Vec::len);
        println!(
            "rename of a {kind} at {}:{at} ({references} references) to {new}: {took:?}, {shown:?}",
            index.path(file).unwrap()
        );
    }
}

/// Prints what the index of newpkgs, `files`, indexed by `reindex`, is with
/// `_lib/sandbox` given a type error: the packages that need it are not
/// checked, but are indexed.
fn sandbox_broken(files: &[SourceFile], reindex: &dyn Fn(&[SourceFile]) -> Indexed) {
    let mut broken = files.to_vec();
    let at = broken
        .iter()
        .position(|f| f.name() == "_lib/sandbox/sandbox.gcsl")
        .unwrap();
    let text = format!("{}\nbroken_probe = 1 + \"a\"\n", broken[at].text());
    broken[at] = SourceFile::new(broken[at].name(), text).unwrap();
    let start = Instant::now();
    let indexed = reindex(&broken);
    let took = start.elapsed();
    let mut index = indexed.index.unwrap();
    let mut clean = reindex(files).index.unwrap();
    let (mut checked, mut hovers, mut before, mut places) = (0, 0, 0, 0);
    for (file, source) in (index.standard()..).zip(&broken) {
        checked += usize::from(index.checked(file));
        if source.name().ends_with("com.amd.amd-ucode.gcsl") {
            let lexed = lexer::lex(source, &mut Names::new());
            for token in &lexed.tokens {
                places += 1;
                hovers += usize::from(index.hover(file, token.span.start).is_some());
                before += usize::from(clean.hover(file, token.span.start).is_some());
            }
        }
    }
    println!(
        "with _lib/sandbox broken: {} errors, indexed in {took:?}, {checked} of {} files checked, {hovers} hovers of {places} tokens of com.amd.amd-ucode.gcsl ({before} unbroken)",
        indexed.diagnostics.len(),
        files.len()
    );
}

/// What a run of hovers and definitions found.
#[derive(Debug, Default)]
struct Asked {
    /// Workspaces and files indexed.
    indexed: usize,
    /// Places asked about.
    places: usize,
    /// Hovers answered.
    hovers: usize,
    /// Definitions answered with at least one place.
    definitions: usize,
    /// References answered with at least one place.
    references: usize,
    /// Places a rename was prepared at.
    renamable: usize,
    /// Symbols of the files.
    symbols: usize,
    /// Places a completion offered something at, of the places asked at
    /// every token's start and end.
    completions: usize,
    /// Places signature help answered at, of the same.
    signatures: usize,
    /// Panics, and answers out of their file, by what was asked.
    failures: Vec<String>,
    /// The time spent asking, indexing apart, completion and signature help
    /// apart.
    asking: Duration,
    /// The time spent on completion and signature help.
    completing: Duration,
    /// The time spent indexing.
    indexing: Duration,
}

impl Asked {
    /// Indexes what `work` checks, `what`, and asks for a hover, a
    /// definition, references and a rename's preparation at every token
    /// start of each file of `files`, its index among the files after the
    /// standard ones with its text, for a completion and signature help at
    /// every token's start and end, and for each file's symbols, recording
    /// what it finds.
    fn run(&mut self, what: &str, work: impl FnOnce() -> Option<Index>, files: &[(u32, &str)]) {
        let start = Instant::now();
        let index = panic::catch_unwind(AssertUnwindSafe(work));
        self.indexing += start.elapsed();
        self.indexed += 1;
        let mut index = match index {
            Ok(Some(index)) => index,
            Ok(None) => return,
            Err(_) => {
                self.failures.push(format!("{what}: indexing panicked"));
                return;
            }
        };
        let start = Instant::now();
        let mut completing = Duration::ZERO;
        for &(file, text) in files {
            let file = file + index.standard();
            let symbols = panic::catch_unwind(AssertUnwindSafe(|| index.document_symbols(file)));
            match symbols {
                Ok(symbols) => self.symbols += symbols.len(),
                Err(_) => self
                    .failures
                    .push(format!("{what}: file {file}'s symbols panicked")),
            }
            let source = SourceFile::new("t.gcsl", text).unwrap();
            let lexed = lexer::lex(&source, &mut Names::new());
            for token in &lexed.tokens {
                let at = token.span.start;
                self.places += 1;
                let asked = panic::catch_unwind(AssertUnwindSafe(|| {
                    let references = index.references(file, at, true);
                    let renamable = index.prepare_rename(file, at).is_ok();
                    let found = (index.hover(file, at), index.definition(file, at));
                    (found, references, renamable)
                }));
                let Ok(((hover, definition), references, renamable)) = asked else {
                    self.failures
                        .push(format!("{what}: file {file} at {at} panicked"));
                    continue;
                };
                self.references += usize::from(!references.is_empty());
                self.renamable += usize::from(renamable);
                for location in references {
                    if location.file < index.standard() || location.span.end <= location.span.start
                    {
                        self.failures.push(format!(
                            "{what}: file {file} at {at}: reference {location:?}"
                        ));
                    }
                }
                if let Some(hover) = hover {
                    self.hovers += 1;
                    if !(hover.span.start <= at && at < hover.span.end) || hover.text.is_empty() {
                        self.failures
                            .push(format!("{what}: file {file} at {at}: hover {hover:?}"));
                    }
                }
                if !definition.is_empty() {
                    self.definitions += 1;
                }
                for location in definition {
                    if location.file < index.standard() || location.span.end < location.span.start {
                        self.failures.push(format!(
                            "{what}: file {file} at {at}: definition {location:?}"
                        ));
                    }
                }
            }
            let started = Instant::now();
            for token in &lexed.tokens {
                for at in [token.span.start, token.span.end] {
                    self.complete(what, &index, (file, at));
                }
            }
            completing += started.elapsed();
        }
        self.completing += completing;
        self.asking += start.elapsed().saturating_sub(completing);
    }

    /// Asks `index` for a completion and signature help at byte `at` of
    /// file `file`, of `what`, recording what it finds.
    fn complete(&mut self, what: &str, index: &Index, (file, at): (u32, u32)) {
        let asked = panic::catch_unwind(AssertUnwindSafe(|| {
            (index.completion(file, at), index.signature(file, at))
        }));
        let Ok((completion, signature)) = asked else {
            self.failures.push(format!(
                "{what}: file {file} at {at}: completion or signature help panicked"
            ));
            return;
        };
        self.completions += usize::from(!completion.is_empty());
        if completion.iter().any(|found| found.label.is_empty()) {
            self.failures
                .push(format!("{what}: file {file} at {at}: an empty label"));
        }
        if let Some(signature) = signature {
            self.signatures += 1;
            let len = u32::try_from(signature.label.len()).unwrap_or(u32::MAX);
            let fits = signature
                .params
                .iter()
                .all(|&(start, end)| start <= end && end <= len);
            let active = signature
                .active
                .is_none_or(|active| (active as usize) < signature.params.len());
            if !fits || !active {
                self.failures.push(format!(
                    "{what}: file {file} at {at}: signature {signature:?}"
                ));
            }
        }
    }

    /// Prints what was found, for `set`, and fails on any failure.
    fn report(&self, set: &str) {
        let per = self.asking / u32::try_from(self.places.max(1)).unwrap_or(u32::MAX);
        // Completion and signature help at each token's start and end.
        let typed = self.completing / u32::try_from(self.places.max(1) * 2).unwrap_or(u32::MAX);
        println!(
            "{set}: {} indexed in {:?}, {} places asked in {:?} ({per:?} each), {} hovers, {} definitions, {} with references, {} renamable, {} symbols; completion and signature help at {} places in {:?} ({typed:?} each), {} with completions, {} with a signature; failures {:?}",
            self.indexed,
            self.indexing,
            self.places,
            self.asking,
            self.hovers,
            self.definitions,
            self.references,
            self.renamable,
            self.symbols,
            self.places * 2,
            self.completing,
            self.completions,
            self.signatures,
            self.failures
        );
        assert!(self.failures.is_empty());
    }
}

#[test]
#[ignore = "a robustness run of minutes, run by hand in release mode"]
fn every_token_of_newpkgs_and_of_the_mutants_has_a_hover_and_a_definition_without_a_panic() {
    let thread = std::thread::Builder::new()
        .stack_size(gcsl::STACK_SIZE)
        .spawn(hovers_everywhere)
        .unwrap();
    thread.join().unwrap();
}

/// The robustness run of the index: newpkgs as it is, every mutant of every
/// file alone, and two of each of newpkgs's files in their workspace.
fn hovers_everywhere() {
    let sources = gcsl_disk::read_workspace(Path::new(NEWPKGS), None).unwrap();
    let files = &sources.relative;
    // newpkgs as it is: every token of every file.
    let mut asked = Asked::default();
    let numbered: Vec<(u32, &str)> = (0u32..).zip(files).map(|(at, f)| (at, f.text())).collect();
    asked.run(
        "newpkgs",
        || workspace::index_with_standard(files, &sources.nested, NEWPKGS).index,
        &numbered,
    );
    asked.report("newpkgs");
    // Every mutant of every file alone.
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut ours = Vec::new();
    gcsl_files(&repository.join("crates"), &mut ours);
    gcsl_files(&repository.join("stdlib"), &mut ours);
    let mut theirs = Vec::new();
    gcsl_files(Path::new(NEWPKGS), &mut theirs);
    for (set, paths) in [
        ("gcsl mutants alone", &ours),
        ("newpkgs mutants alone", &theirs),
    ] {
        let mut asked = Asked::default();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let (cuts, deletions) = mutants(&text);
            for mutant in cuts.iter().chain(&deletions) {
                let what = format!("{} mutant of {} bytes", path.display(), mutant.len());
                let source = SourceFile::new("mutant.gcsl", mutant.as_str()).unwrap();
                asked.run(
                    &what,
                    || workspace::index_lone_with_standard(&source, "/").index,
                    &[(0, mutant.as_str())],
                );
            }
        }
        asked.report(set);
    }
    // Each of newpkgs's files cut in its middle, and with its middle token
    // deleted, in its workspace.
    let mut asked = Asked::default();
    for at in 0..files.len() {
        let original = &files[at];
        let (cuts, deletions) = mutants(original.text());
        for (kind, mutant) in [("cut", cuts.get(24)), ("deletion", deletions.get(25))] {
            let Some(mutant) = mutant else {
                continue;
            };
            let mut changed = files.clone();
            changed[at] = SourceFile::new(original.name(), mutant.as_str()).unwrap();
            let what = format!("{} {kind} in newpkgs", original.name());
            let file = u32::try_from(at).unwrap();
            let mut diagnostics = Vec::new();
            asked.run(
                &what,
                || {
                    let indexed =
                        workspace::index_with_standard(&changed, &sources.nested, NEWPKGS);
                    diagnostics = indexed.diagnostics;
                    indexed.index
                },
                &[(file, mutant.as_str())],
            );
            // The index's diagnostics are the check's: the files checked
            // for the index alone report nothing.
            let (_, checked) = workspace::check_with_standard(&changed, &sources.nested, NEWPKGS);
            let reported = match &checked {
                Ok(checked) => checked.warnings(),
                Err(errors) => errors.as_slice(),
            };
            if format!("{diagnostics:?}") != format!("{reported:?}") {
                asked
                    .failures
                    .push(format!("{what}: the index's diagnostics differ"));
            }
        }
    }
    asked.report("newpkgs mutants in their workspace");
}

#[test]
#[ignore = "a robustness run of minutes, run by hand in release mode"]
fn every_scan_error_mutant_of_newpkgs_recovers_without_a_panic() {
    let thread = std::thread::Builder::new()
        .stack_size(gcsl::STACK_SIZE)
        .spawn(scan_errors_everywhere)
        .unwrap();
    thread.join().unwrap();
}

/// The text of the first top-level declaration of `text` (a field, a `def`,
/// a definition or a meta), if it has one.
fn first_declaration(text: &str) -> Option<String> {
    use gcsl::ast::ItemKind;
    let source = SourceFile::new("t.gcsl", text).ok()?;
    let (ast, _) = parser::parse_source(&source);
    let item = ast.items(ast.root()).iter().find(|&&item| {
        matches!(
            ast.item(item).kind,
            ItemKind::Field { .. } | ItemKind::DefTypeDef { .. } | ItemKind::DefMeta { .. }
        )
    })?;
    Some(source.slice(ast.item(*item).span).to_owned())
}

/// `text` with the path of its first `import` made one of no module, if it
/// has one.
fn import_of_nothing(text: &str) -> Option<String> {
    use gcsl::ast::ItemKind;
    let source = SourceFile::new("t.gcsl", text).ok()?;
    let (ast, _) = parser::parse_source(&source);
    let path = ast.items(ast.root()).iter().find_map(|&item| {
        let ItemKind::Import(id) = ast.item(item).kind else {
            return None;
        };
        let path = ast.path(ast.import_decl(id).path);
        Some((path.first()?.span, path.last()?.span))
    })?;
    let (start, end) = (path.0.start as usize, path.1.end as usize);
    Some(format!("{}zz_no_module{}", &text[..start], &text[end..]))
}

/// The module path of the workspace's file at `path`, as an import names it.
fn module_path(path: &str) -> String {
    path.trim_end_matches(".gcsl").replace('/', ".")
}

/// What the run of scan errors found.
#[derive(Debug, Default)]
struct Scanned {
    /// Mutants, by kind.
    mutants: [usize; 3],
    /// Files with no mutant of a kind, by kind.
    skipped: [usize; 3],
    /// Mutants whose check reported a scan error of their kind.
    reported: [usize; 3],
    /// Panics, internal errors, and diagnostics that differ between two
    /// checks or between the check and the index, by what was run.
    failures: Vec<String>,
    /// Errors reported in a file the mutant did not touch, which may be
    /// cascades of its error, and the first few of them.
    spilled: usize,
    spills: Vec<String>,
    /// The slowest mutant and its time.
    slowest: (Duration, String),
}

impl Scanned {
    /// Checks `files`, the mutant `what` of kind `kind` whose scan error's
    /// message holds `marker`, twice, and indexes it, recording what it
    /// finds.
    fn run(
        &mut self,
        (what, kind, marker): (&str, usize, &str),
        (files, touched): (&[SourceFile], &[&str]),
        nested: &[String],
    ) {
        let start = Instant::now();
        let reported = |checked: Result<workspace::Workspace, Vec<Diagnostic>>| match checked {
            Ok(checked) => checked.warnings().to_vec(),
            Err(errors) => errors,
        };
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            let first = reported(workspace::check_with_standard(files, nested, NEWPKGS).1);
            let second = reported(workspace::check_with_standard(files, nested, NEWPKGS).1);
            let indexed = workspace::index_with_standard(files, nested, NEWPKGS);
            (first, second, indexed)
        }));
        let took = start.elapsed();
        if took > self.slowest.0 {
            self.slowest = (took, what.to_owned());
        }
        if let Some(entry) = self.mutants.get_mut(kind) {
            *entry += 1;
        }
        let Ok((first, second, indexed)) = outcome else {
            self.failures.push(format!("{what}: panicked"));
            return;
        };
        if format!("{first:?}") != format!("{second:?}") {
            self.failures.push(format!("{what}: two checks differ"));
        }
        if format!("{first:?}") != format!("{:?}", indexed.diagnostics) {
            self.failures
                .push(format!("{what}: the index's diagnostics differ"));
        }
        if indexed.index.is_none() {
            self.failures.push(format!("{what}: no index"));
        }
        if first
            .iter()
            .any(|diagnostic| diagnostic.notes.iter().any(|note| note.contains("a bug")))
        {
            self.failures.push(format!("{what}: an internal error"));
        }
        if first.iter().any(|d| d.message.contains(marker))
            && let Some(entry) = self.reported.get_mut(kind)
        {
            *entry += 1;
        }
        // An error in a file the mutant did not touch may be a cascade.
        let mut sorted: Vec<&str> = files.iter().map(SourceFile::name).collect();
        sorted.sort_unstable();
        let standard = indexed.standard.len();
        for error in first.iter().filter(|d| d.severity == gcsl::Severity::Error) {
            let path = error
                .input
                .and_then(|input| (input as usize).checked_sub(standard))
                .and_then(|at| sorted.get(at))
                .copied()
                .unwrap_or("<stdlib>");
            if !touched.contains(&path) {
                self.spilled += 1;
                if self.spills.len() < 10 {
                    self.spills
                        .push(format!("{what}: {} in {path}", error.message));
                }
            }
        }
    }
}

/// The robustness run of the scan's recovery: for each file of newpkgs, in
/// its workspace, in memory, its first declaration copied into a new file
/// of its directory (a field defined in two files), the path of its first
/// import made one of no module, and a read of a file that needs it (files
/// that need each other).
fn scan_errors_everywhere() {
    let sources = gcsl_disk::read_workspace(Path::new(NEWPKGS), None).unwrap();
    let files = &sources.relative;
    let scan = workspace::scan_with(files, &sources.nested).unwrap();
    let standard = scan.standard() as usize;
    let mut found = Scanned::default();
    for (at, original) in files.iter().enumerate() {
        let name = original.name();
        // A field defined in two files.
        match first_declaration(original.text()) {
            Some(declaration) => {
                let dir = name.rsplit_once('/').map_or("", |(dir, _)| dir);
                let path = if dir.is_empty() {
                    "zz-mutant-dup.gcsl".to_owned()
                } else {
                    format!("{dir}/zz-mutant-dup.gcsl")
                };
                let mut changed = files.clone();
                changed.push(SourceFile::new(path.as_str(), declaration).unwrap());
                let what = format!("{name}: its first declaration in a sibling");
                let touched = [path.as_str()];
                found.run((&what, 0, "defined"), (&changed, &touched), &sources.nested);
            }
            None => found.skipped[0] += 1,
        }
        // An import of no module.
        match import_of_nothing(original.text()) {
            Some(text) => {
                let mut changed = files.clone();
                changed[at] = SourceFile::new(name, text).unwrap();
                let what = format!("{name}: an import of no module");
                found.run(
                    (&what, 1, "no module"),
                    (&changed, &[name]),
                    &sources.nested,
                );
            }
            None => found.skipped[1] += 1,
        }
        // Files that need each other: the file reads a root of a file of its
        // directory that needs it, or a file that needs it whole.
        let index = at + standard;
        let user = scan
            .needs
            .iter()
            .position(|needs| needs.contains(&u32::try_from(index).unwrap()))
            .and_then(|user| scan.files.get(user))
            .filter(|user| !user.path.starts_with(workspace::STANDARD_DIR));
        let read = user.and_then(|user| {
            let own = scan.files.get(index)?;
            if user.dir == own.dir {
                let tree = &user.mentions.tree;
                let root = tree.roots.iter().find_map(|&root| {
                    let name = scan.names.get(tree.nodes.get(root as usize)?.name);
                    lexer::is_identifier(name).then_some(name)
                })?;
                return Some(format!("{}\nzz_cycle_read = {root}\n", original.text()));
            }
            // A module path names no directory with a `.` in its name.
            let path = user.path.trim_end_matches(".gcsl");
            let parts: Vec<&str> = path.split('/').collect();
            parts.iter().all(|part| !part.contains('.')).then(|| {
                format!(
                    "{}\nimport {} as zz_cycle\nzz_cycle_read = zz_cycle\n",
                    original.text(),
                    module_path(&user.path)
                )
            })
        });
        match user.zip(read) {
            Some((user, text)) => {
                let mut changed = files.clone();
                changed[at] = SourceFile::new(name, text).unwrap();
                let what = format!("{name}: a read of {}", user.path);
                found.run(
                    (&what, 2, "need"),
                    (&changed, &[name, user.path.as_str()]),
                    &sources.nested,
                );
            }
            None => found.skipped[2] += 1,
        }
    }
    println!(
        "scan errors of newpkgs ({} files): mutants (duplicate, no module, cycle) {:?}, skipped {:?}, \
         reporting their error {:?}, slowest {:?} ({}), errors in files not touched {} {:#?}, failures {:?}",
        files.len(),
        found.mutants,
        found.skipped,
        found.reported,
        found.slowest.0,
        found.slowest.1,
        found.spilled,
        found.spills,
        found.failures
    );
    assert!(found.failures.is_empty());
}
