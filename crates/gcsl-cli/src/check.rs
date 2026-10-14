//! `gcsl check [--message-format json] [PATHS...]`: check workspaces and
//! files, and run nothing, for CI, pre-commit hooks and agents that edit
//! GCSL without an editor.
//!
//! What is checked, each workspace once however many paths reach it:
//!
//! - for a FILE, what `gcsl run FILE` checks: its whole workspace, if a
//!   `workspace.gcsl` is above it ([`gcsl_disk::find_workspace`]), else the
//!   file alone, as a lone file is run; a file of a workspace that is none
//!   of its files as they are listed (one reached through a directory that
//!   is a symbolic link, say) is checked alone, as `gcsl run` runs it;
//! - for a DIRECTORY, every workspace whose root is it or is under it, the
//!   workspaces nested in it included, as `gcsl fmt` finds them (names
//!   starting with `.` left out); and, if it is inside a workspace without
//!   being its root, that enclosing workspace, once. A `.gcsl` file under a
//!   directory that is in no workspace is checked by no directory given,
//!   only by being given itself: unlike `gcsl fmt`, which formats it, `gcsl
//!   check` checks only workspaces under a directory;
//! - with no paths, the workspace the working directory is in
//!   ([`gcsl_disk::find_root`]), its nested workspaces left out, as they are
//!   workspaces of their own; outside any workspace that is an error.
//!
//! A workspace is read and checked exactly as `gcsl run` reads and checks
//! one ([`gcsl_disk::read_workspace`],
//! [`gcsl::workspace::check_with_standard`]), a lone file as `gcsl run`
//! checks one ([`gcsl::workspace::check_lone_with_standard`]), and nothing
//! is run: no file's value, no `sh` or `exec`, no file read but the
//! sources. Every diagnostic, errors and warnings, is rendered to standard
//! error as `gcsl run` renders it, each file named relative to the working
//! directory when it is under it ([`gcsl_disk::shown`]), a lone file given
//! by the path it was given as; a blank line between two. Then one summary
//! line goes to standard error: `N errors, M warnings in K files checked`
//! (`1 error`, `1 warning`, `1 file` for one), or `no problems in K files
//! checked`, K counting the files of each workspace and each lone file
//! checked, the standard library's not among them.
//!
//! With `--message-format json`, nothing is rendered: each diagnostic is
//! one JSON object on a line of standard output, `{"path", "severity",
//! "message", "line", "column", "end_line", "end_column", "labels",
//! "notes"}`, its lines and columns 1-based, columns counted in characters,
//! `severity` `"error"` or `"warning"`; `labels` is a list of `{"path",
//! "line", "column", "end_line", "end_column", "text"}`, the text of the
//! primary span first, at its span, when the diagnostic has one, then each
//! of its labels; `notes` a list of strings. The summary still goes to
//! standard error.
//!
//! The exit code is 2 for a usage error, a path that is not there, or a
//! workspace, a file or the output that cannot be read or written (a path
//! that is not there checks nothing; a workspace that cannot be read is
//! reported, and the others are checked); else 1 if any error was found;
//! else 0, warnings or not.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, ValueEnum};
use gcsl::diagnostic::Renderer;
use gcsl::workspace;
use gcsl::{Diagnostic, Severity, SourceFile, Span};
use serde_json::{Value, json};

use crate::run::file_dir;
use crate::{EXIT_PROGRAM_ERROR, EXIT_USAGE_OR_IO, output};

/// Arguments of `gcsl check`.
#[derive(Args)]
pub struct CheckArgs {
    /// How diagnostics are shown: rendered to standard error, or as one
    /// JSON object per line on standard output.
    #[arg(long, value_enum, default_value_t = MessageFormat::Human)]
    message_format: MessageFormat,
    /// Files to check, each with its workspace, or directories to check
    /// every workspace at or under. With none, the workspace the working
    /// directory is in.
    paths: Vec<PathBuf>,
}

/// How diagnostics are shown.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MessageFormat {
    /// Rendered as `gcsl run` renders them, to standard error.
    Human,
    /// One JSON object per line, on standard output.
    Json,
}

/// What is checked: a workspace, by its canonical root, or a file alone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Unit {
    /// The workspace under this root; if a file given is of it, by its path
    /// relative to the root and its path as given, which is checked alone
    /// if it is none of the workspace's files as they are listed.
    Workspace(PathBuf, Option<(String, PathBuf)>),
    /// A file alone, by its path as given.
    Lone(PathBuf),
}

/// What a run of `gcsl check` has found so far.
struct Found {
    /// How diagnostics are shown.
    format: MessageFormat,
    /// How they are rendered, in the human format.
    renderer: Renderer,
    /// The errors found.
    errors: usize,
    /// The warnings found.
    warnings: usize,
    /// The files checked, the standard library's not among them.
    files: usize,
    /// Whether a diagnostic has been rendered, so the next is set apart by
    /// a blank line.
    rendered: bool,
    /// Whether a workspace or a file could not be read, or the output
    /// written: exit code 2.
    broken: bool,
}

/// Runs `gcsl check` and returns the process exit code.
pub fn main(args: &CheckArgs) -> ExitCode {
    let Ok(units) = units(&args.paths) else {
        return ExitCode::from(EXIT_USAGE_OR_IO);
    };
    let mut found = Found {
        format: args.message_format,
        renderer: output::renderer(),
        errors: 0,
        warnings: 0,
        files: 0,
        rendered: false,
        broken: false,
    };
    for unit in units {
        match unit {
            Unit::Workspace(root, given) => found.workspace(&root, given.as_ref()),
            Unit::Lone(path) => found.lone(&path),
        }
    }
    found.summary();
    if found.broken {
        ExitCode::from(EXIT_USAGE_OR_IO)
    } else if found.errors > 0 {
        ExitCode::from(EXIT_PROGRAM_ERROR)
    } else {
        ExitCode::SUCCESS
    }
}

/// What to check for `paths`, each once, in the order they are given;
/// `Err` once what cannot be found is reported.
fn units(paths: &[PathBuf]) -> Result<Vec<Unit>, ()> {
    let mut units = Vec::new();
    if paths.is_empty() {
        let cwd = std::env::current_dir().and_then(fs::canonicalize);
        let cwd = cwd.map_err(|error| {
            output::error(format_args!("cannot find the working directory: {error}"));
        })?;
        let Some(root) = gcsl_disk::find_root(&cwd) else {
            output::error(format_args!(
                "no `{}` in {} or above it: give the files or directories to check",
                gcsl_disk::WORKSPACE,
                cwd.display()
            ));
            return Err(());
        };
        units.push(Unit::Workspace(root.to_path_buf(), None));
        return Ok(units);
    }
    let mut broken = false;
    for path in paths {
        if path.is_dir() {
            match under(path) {
                Ok(found) => units.extend(found),
                Err(()) => broken = true,
            }
        } else if path.is_file() {
            units.push(match gcsl_disk::find_workspace(path) {
                Some((root, relative)) => Unit::Workspace(root, Some((relative, path.clone()))),
                None => Unit::Lone(path.clone()),
            });
        } else {
            output::error(format_args!(
                "cannot read {}: no such file or directory",
                path.display()
            ));
            broken = true;
        }
    }
    if broken {
        return Err(());
    }
    // Each workspace once, with the first file given of it; each lone file
    // once, by its canonical path.
    let mut roots = HashSet::new();
    let mut lone = HashSet::new();
    let mut unique = Vec::new();
    for unit in units {
        let first = match &unit {
            Unit::Workspace(root, _) => roots.insert(root.clone()),
            Unit::Lone(path) => {
                lone.insert(fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            }
        };
        if first {
            unique.push(unit);
        }
    }
    Ok(unique)
}

/// The workspaces at or under the directory `dir`, and the one it is in,
/// each by its canonical root: the enclosing or its own first, then those
/// nested in it, in path order, depth first.
fn under(dir: &Path) -> Result<Vec<Unit>, ()> {
    let dir = fs::canonicalize(dir).map_err(|error| {
        output::error(format_args!("cannot read {}: {error}", dir.display()));
    })?;
    let mut found = Vec::new();
    if let Some(root) = gcsl_disk::find_root(&dir) {
        found.push(Unit::Workspace(root.to_path_buf(), None));
    }
    nested(&dir, &mut found)?;
    Ok(found)
}

/// Adds the workspaces nested under `dir` to `found`, those nested in them
/// too, as [`gcsl_disk::workspace_files`] lists them.
fn nested(dir: &Path, found: &mut Vec<Unit>) -> Result<(), ()> {
    let (_, inner) = gcsl_disk::workspace_files(dir).map_err(|unreadable| {
        output::error(format_args!(
            "cannot read {}: {}",
            gcsl_disk::shown(&unreadable.path).display(),
            unreadable.error
        ));
    })?;
    for name in inner {
        let root = dir.join(name);
        found.push(Unit::Workspace(root.clone(), None));
        nested(&root, found)?;
    }
    Ok(())
}

impl Found {
    /// Checks the workspace under `root`, as `gcsl run` checks one, and
    /// shows what it finds; then, if `given`, a file given of it, is none
    /// of its files as they are listed, that file alone.
    fn workspace(&mut self, root: &Path, given: Option<&(String, PathBuf)>) {
        let sources = match gcsl_disk::read_workspace(root, None) {
            Ok(sources) => sources,
            Err(error) => {
                output::error(error);
                self.broken = true;
                return;
            }
        };
        let root_name = root.to_string_lossy();
        let (standard, checked) =
            workspace::check_with_standard(&sources.relative, &sources.nested, &root_name);
        let diagnostics = match &checked {
            Ok(checked) => checked.warnings(),
            Err(diagnostics) => diagnostics.as_slice(),
        };
        self.files += sources.relative.len();
        let mut shown = standard;
        shown.extend(sources.shown);
        if let Some(fallback) = shown.last() {
            self.show(&shown, fallback, diagnostics);
        }
        if let Some((relative, path)) = given
            && !sources.relative.iter().any(|file| file.name() == relative)
        {
            self.lone(path);
        }
    }

    /// Checks the file at `path` alone, as `gcsl run` checks a file outside
    /// any workspace, named as it is given, and shows what it finds.
    fn lone(&mut self, path: &Path) {
        let name = path.display();
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                output::error(format_args!("cannot read {name}: {error}"));
                self.broken = true;
                return;
            }
        };
        let source = match SourceFile::new(name.to_string(), text) {
            Ok(source) => source,
            Err(error) => {
                output::error(format_args!("cannot load {name}: {error}"));
                self.broken = true;
                return;
            }
        };
        let (standard, checked) = workspace::check_lone_with_standard(&source, &file_dir(path));
        let diagnostics = match &checked {
            Ok(checked) => checked.warnings(),
            Err(diagnostics) => diagnostics.as_slice(),
        };
        self.files += 1;
        let mut shown = standard;
        shown.push(source.clone());
        self.show(&shown, &source, diagnostics);
    }

    /// Counts `diagnostics` and shows them, each part of each against the
    /// source of the input it names, `sources` being indexed by input
    /// number, or against `fallback` when it names none.
    fn show(&mut self, sources: &[SourceFile], fallback: &SourceFile, diagnostics: &[Diagnostic]) {
        let source_of = |input: Option<u32>| {
            input
                .and_then(|input| sources.get(input as usize))
                .unwrap_or(fallback)
        };
        for diagnostic in diagnostics {
            match diagnostic.severity {
                Severity::Error => self.errors += 1,
                Severity::Warning => self.warnings += 1,
            }
        }
        let written = match self.format {
            MessageFormat::Human => {
                let mut text = String::new();
                for diagnostic in diagnostics {
                    if self.rendered {
                        text.push('\n');
                    }
                    text.push_str(&self.renderer.render_in(diagnostic, source_of));
                    self.rendered = true;
                }
                // A failure to write to stderr is ignored: there is nowhere
                // left to report it.
                let _ = io::stderr().lock().write_all(text.as_bytes());
                Ok(())
            }
            MessageFormat::Json => {
                let mut out = io::stdout().lock();
                diagnostics
                    .iter()
                    .try_for_each(|diagnostic| writeln!(out, "{}", json_of(diagnostic, source_of)))
                    .and_then(|()| out.flush())
            }
        };
        if let Err(error) = written {
            if !self.broken {
                output::error(format_args!("cannot write output: {error}"));
            }
            self.broken = true;
        }
    }

    /// Writes the summary line to standard error.
    fn summary(&self) {
        let checked = format!("in {} checked", counted(self.files, "file"));
        let line = if self.errors == 0 && self.warnings == 0 {
            format!("no problems {checked}")
        } else {
            format!(
                "{}, {} {checked}",
                counted(self.errors, "error"),
                counted(self.warnings, "warning")
            )
        };
        let _ = writeln!(io::stderr().lock(), "{line}");
    }
}

/// `count` and `noun`, plural but for one: `1 file`, `2 files`.
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// `diagnostic` as the JSON object `--message-format json` prints for it,
/// each of its parts against the source `source_of` gives for the input it
/// names (see the module documentation).
fn json_of<'a>(
    diagnostic: &Diagnostic,
    source_of: impl Fn(Option<u32>) -> &'a SourceFile,
) -> Value {
    let place = |source: &SourceFile, span: Span| {
        let (line, column) = source.line_col(span.start);
        let (end_line, end_column) = source.line_col(span.end);
        (line, column, end_line, end_column)
    };
    let label = |source: &SourceFile, span: Span, text: &str| {
        let (line, column, end_line, end_column) = place(source, span);
        json!({
            "path": source.name(),
            "line": line,
            "column": column,
            "end_line": end_line,
            "end_column": end_column,
            "text": text,
        })
    };
    let source = source_of(diagnostic.input);
    let mut labels = Vec::new();
    if let Some(text) = diagnostic.label.as_deref().filter(|text| !text.is_empty()) {
        labels.push(label(source, diagnostic.span, text));
    }
    for other in &diagnostic.labels {
        let at = source_of(other.input.or(diagnostic.input));
        labels.push(label(at, other.span, &other.text));
    }
    let (line, column, end_line, end_column) = place(source, diagnostic.span);
    json!({
        "path": source.name(),
        "severity": diagnostic.severity.to_string(),
        "message": diagnostic.message,
        "line": line,
        "column": column,
        "end_line": end_line,
        "end_column": end_column,
        "labels": labels,
        "notes": diagnostic.notes,
    })
}
