//! `gcsl run FILE`: run a file, or dump its intermediate stages.
//!
//! A file runs alone unless it is a file of a *workspace*: the directory
//! tree under the nearest directory above it that holds a file named
//! `workspace.gcsl` ([`gcsl::workspace`]). Then every `.gcsl` file of the
//! tree is read (except names starting with `.`, and directories that
//! hold their own `workspace.gcsl`, which are workspaces of their own),
//! the whole workspace is checked, and `FILE` runs after the files it
//! needs; its own value is printed, with the warnings of every file.
//! Finding and reading the files is this module's: the library performs
//! no I/O. Each file is given to the library by its path relative to the
//! root, and its diagnostics name it by its path from the current
//! directory, `FILE` by the path given. The dumps are `FILE`'s alone: its
//! tokens and syntax tree as a file's, its types and IR as checked and
//! lowered in the workspace.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use clap::Args;
use gcsl::ast::Ast;
use gcsl::lexer::TokenKind;
use gcsl::strings::Quoted;
use gcsl::{
    Diagnostic, SourceFile, TypeTable, Value, check, diagnostic, eval, lexer, lower, parser,
    workspace,
};

use crate::{EXIT_PROGRAM_ERROR, EXIT_USAGE_OR_IO, output};

/// Arguments of `gcsl run`.
#[derive(Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent command-line flag"
)]
pub struct RunArgs {
    /// The GCSL source file to run.
    file: PathBuf,
    /// Print the tokens.
    #[arg(long)]
    dump_tokens: bool,
    /// Print the syntax tree.
    #[arg(long)]
    dump_ast: bool,
    /// Print the syntax tree annotated with types.
    #[arg(long)]
    dump_types: bool,
    /// Print the lowered IR.
    #[arg(long)]
    dump_ir: bool,
}

impl RunArgs {
    /// The last stage to run, if any dump was requested. Running stops after
    /// it instead of evaluating.
    fn last_dump(&self) -> Option<Stage> {
        [
            (self.dump_ir, Stage::Ir),
            (self.dump_types, Stage::Types),
            (self.dump_ast, Stage::Ast),
            (self.dump_tokens, Stage::Tokens),
        ]
        .into_iter()
        .find_map(|(requested, stage)| requested.then_some(stage))
    }
}

/// The dumpable pipeline stages, in pipeline order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Tokens,
    Ast,
    Types,
    Ir,
}

/// Why a run did not succeed.
enum Failure {
    /// The program has errors: exit code 1.
    Program(Vec<Diagnostic>),
    /// Writing the output failed: exit code 2.
    Output(io::Error),
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Failure::Output(error)
    }
}

impl From<Diagnostic> for Failure {
    fn from(diagnostic: Diagnostic) -> Self {
        Failure::Program(vec![diagnostic])
    }
}

/// Runs `gcsl run` and returns the process exit code.
pub fn main(args: &RunArgs) -> ExitCode {
    let path = args.file.display();
    let text = match fs::read_to_string(&args.file) {
        Ok(text) => text,
        Err(error) => {
            output::error(format_args!("cannot read {path}: {error}"));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    let source = match SourceFile::new(path.to_string(), text) {
        Ok(source) => source,
        Err(error) => {
            output::error(format_args!("cannot load {path}: {error}"));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    match find_workspace(&args.file) {
        Some((root, relative)) => run_workspace(args, &source, &root, &relative),
        None => run_alone(args, &source),
    }
}

/// Runs `source` alone, as a file outside any workspace.
fn run_alone(args: &RunArgs, source: &SourceFile) -> ExitCode {
    match execute(args, source) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Program(diagnostics)) => {
            output::diagnostics(source, &diagnostics);
            ExitCode::from(EXIT_PROGRAM_ERROR)
        }
        Err(failure) => exit_for(failure),
    }
}

/// The exit code of a failure to write the output, after reporting it, or
/// of a program error already reported.
fn exit_for(failure: Failure) -> ExitCode {
    match failure {
        Failure::Program(_) => ExitCode::from(EXIT_PROGRAM_ERROR),
        Failure::Output(error) => {
            output::error(format_args!("cannot write output: {error}"));
            ExitCode::from(EXIT_USAGE_OR_IO)
        }
    }
}

/// Runs the pipeline, printing each requested dump and stopping after the
/// last one; with no dumps, prints the value, as text, unless it is `{}`.
///
/// Warnings go to stderr once the run has succeeded (after the dumps, or
/// before the value). If a later stage fails, they are returned with its
/// error instead, to be printed together in source order.
fn execute(args: &RunArgs, source: &SourceFile) -> Result<(), Failure> {
    let last = args.last_dump();
    let mut out = io::stdout().lock();
    let Some(ast) = syntax(args, source, &mut out)? else {
        return Ok(out.flush()?);
    };

    let mut types = TypeTable::new();
    let (checked, warnings) = check::check(&ast, &mut types).map_err(Failure::Program)?;
    if args.dump_types {
        let annotation = checked.types.annotation(&types, ast.names());
        writeln!(out, "{}", ast.pretty_with(&annotation))?;
    }
    if last == Some(Stage::Types) {
        output::diagnostics(source, &warnings);
        return Ok(out.flush()?);
    }

    let ir = match lower::lower(&ast, &checked, &types, 0) {
        Ok(ir) => Rc::new(ir),
        Err(error) => return Err(with_warnings(error.into(), warnings)),
    };
    if args.dump_ir {
        writeln!(out, "{}", ir.pretty(ast.names()))?;
    }
    if last == Some(Stage::Ir) {
        output::diagnostics(source, &warnings);
        return Ok(out.flush()?);
    }

    let value = match eval::eval(&ir) {
        Ok(value) => value,
        Err(error) => return Err(with_warnings(error, warnings)),
    };
    output::diagnostics(source, &warnings);
    print_value(&value, &mut out)?;
    Ok(out.flush()?)
}

/// Prints `value` as text, unless it is `{}`. A string prints as its text,
/// unquoted, so a program can produce output verbatim. A record prints as
/// it would be written.
fn print_value(value: &Value, out: &mut impl Write) -> io::Result<()> {
    if matches!(value, Value::Empty) {
        Ok(())
    } else {
        writeln!(out, "{value}")
    }
}

/// Lexes and parses `source`, printing its tokens and its syntax tree if
/// they are asked for; `None` when nothing after them is.
fn syntax(
    args: &RunArgs,
    source: &SourceFile,
    out: &mut impl Write,
) -> Result<Option<Ast>, Failure> {
    let last = args.last_dump();
    let (tokens, names, strings) = lexer::tokenize(source)?;
    if args.dump_tokens {
        // Names by their text and string pieces by their quoted text, as in
        // `Ident(x)`, `Label(outer)` and `StrStart("a\n")`.
        for token in &tokens {
            let span = token.span;
            match token.kind {
                TokenKind::Ident(name) => writeln!(out, "Ident({}) @{span}", names.get(name))?,
                TokenKind::Label(name) => writeln!(out, "Label({}) @{span}", names.get(name))?,
                TokenKind::Str(id) => writeln!(out, "Str({}) @{span}", Quoted(strings.get(id)))?,
                TokenKind::StrStart(id) => {
                    writeln!(out, "StrStart({}) @{span}", Quoted(strings.get(id)))?;
                }
                TokenKind::StrMid(id) => {
                    writeln!(out, "StrMid({}) @{span}", Quoted(strings.get(id)))?;
                }
                TokenKind::StrEnd(id) => {
                    writeln!(out, "StrEnd({}) @{span}", Quoted(strings.get(id)))?;
                }
                kind => writeln!(out, "{kind:?} @{span}")?,
            }
        }
    }
    if last == Some(Stage::Tokens) {
        return Ok(None);
    }

    let ast = parser::parse(&tokens, names, strings, source)?;
    if args.dump_ast {
        writeln!(out, "{}", ast.pretty())?;
    }
    Ok((last != Some(Stage::Ast)).then_some(ast))
}

/// The failure of a stage after the checker: its `error`, with the
/// `warnings` the checker found, in source order.
fn with_warnings(error: Diagnostic, warnings: Vec<Diagnostic>) -> Failure {
    Failure::Program(diagnostic::merge(vec![error], warnings))
}

/// The name of the file that makes its directory a workspace's root.
const WORKSPACE: &str = "workspace.gcsl";

/// The root of the workspace `file` is a file of, and `file`'s path
/// relative to it, `/` separated: the nearest directory above `file`
/// holding a `workspace.gcsl`.
///
/// `None` when there is none, and also when `file` is not one of the
/// workspace's files (a name starting with `.` on the way, or not a
/// `.gcsl` file), or its path cannot be followed or is not UTF-8: such a
/// file runs alone, as a file outside any workspace does.
fn find_workspace(file: &Path) -> Option<(PathBuf, String)> {
    let file = fs::canonicalize(file).ok()?;
    let root = file
        .parent()?
        .ancestors()
        .find(|dir| dir.join(WORKSPACE).is_file())?;
    let mut parts = Vec::new();
    for part in file.strip_prefix(root).ok()?.components() {
        let Component::Normal(part) = part else {
            return None;
        };
        let part = part.to_str()?;
        if part.starts_with('.') {
            return None;
        }
        parts.push(part);
    }
    if file.extension().and_then(OsStr::to_str) != Some("gcsl") {
        return None;
    }
    Some((root.to_path_buf(), parts.join("/")))
}

/// A file of a workspace that could not be listed or read.
struct Unreadable {
    /// The path of the file or directory.
    path: PathBuf,
    /// Why it could not be read.
    error: io::Error,
}

/// The files of the workspace under `root`, each with its path relative to
/// `root`, `/` separated, in path order, byte by byte: every `.gcsl` file
/// below it, except those whose name, or a directory's on the way, starts
/// with `.`, and those under a directory holding its own `workspace.gcsl`,
/// which is a workspace of its own. Names that are not UTF-8 are skipped,
/// and so is a directory that is a symbolic link, which could lead back
/// into the tree; a file that is one is read as any file.
fn workspace_files(root: &Path) -> Result<Vec<(String, PathBuf)>, Unreadable> {
    let mut files = Vec::new();
    let mut dirs = vec![(String::new(), root.to_path_buf())];
    while let Some((prefix, dir)) = dirs.pop() {
        let unreadable = |error| Unreadable {
            path: dir.clone(),
            error,
        };
        for entry in fs::read_dir(&dir).map_err(unreadable)? {
            let entry = entry.map_err(unreadable)?;
            let name = entry.file_name();
            let Some(name) = name.to_str().filter(|name| !name.starts_with('.')) else {
                continue;
            };
            let path = entry.path();
            if path.is_dir() {
                let linked = entry.file_type().is_ok_and(|kind| kind.is_symlink());
                if !linked && !path.join(WORKSPACE).is_file() {
                    dirs.push((format!("{prefix}{name}/"), path));
                }
            } else if path.extension().and_then(OsStr::to_str) == Some("gcsl") {
                files.push((format!("{prefix}{name}"), path));
            }
        }
    }
    files.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    Ok(files)
}

/// How `path`, a full path, is shown: relative to the current directory
/// when it is under it, else in full.
fn shown(path: &Path) -> PathBuf {
    std::env::current_dir()
        .and_then(fs::canonicalize)
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
}

/// Runs `gcsl run` on `file`, the file given, read, whose path relative to
/// the workspace `root` is `relative`: reads the workspace's files, then
/// checks the workspace and runs `file` in it.
///
/// Each file is given to the workspace by its path relative to the root,
/// and its diagnostics are shown with its path from the current directory
/// (`file`'s as it was given).
fn run_workspace(args: &RunArgs, file: &SourceFile, root: &Path, relative: &str) -> ExitCode {
    let paths = match workspace_files(root) {
        Ok(paths) => paths,
        Err(Unreadable { path, error }) => {
            let path = shown(&path);
            output::error(format_args!("cannot read {}: {error}", path.display()));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    let mut sources = Vec::new();
    let mut shown_sources = Vec::new();
    for (name, path) in &paths {
        let (shown_name, text) = if name == relative {
            (file.name().to_owned(), file.text().to_owned())
        } else {
            let shown_name = shown(path).display().to_string();
            match fs::read_to_string(path) {
                Ok(text) => (shown_name, text),
                Err(error) => {
                    output::error(format_args!("cannot read {shown_name}: {error}"));
                    return ExitCode::from(EXIT_USAGE_OR_IO);
                }
            }
        };
        match (
            SourceFile::new(name.clone(), text.clone()),
            SourceFile::new(shown_name.clone(), text),
        ) {
            (Ok(source), Ok(shown_source)) => {
                sources.push(source);
                shown_sources.push(shown_source);
            }
            (Err(error), _) | (_, Err(error)) => {
                output::error(format_args!("cannot load {shown_name}: {error}"));
                return ExitCode::from(EXIT_USAGE_OR_IO);
            }
        }
    }
    let index = paths.iter().position(|(name, _)| name == relative);
    let Some(index) = index.and_then(|index| u32::try_from(index).ok()) else {
        // Not found among the files listed (a link, say): it runs alone.
        return run_alone(args, file);
    };
    match execute_workspace(args, file, &sources, &shown_sources, index) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Program(diagnostics)) => {
            output::diagnostics_in(&shown_sources, file, &diagnostics);
            ExitCode::from(EXIT_PROGRAM_ERROR)
        }
        Err(failure) => exit_for(failure),
    }
}

/// Runs the workspace of `sources` as [`execute`] runs one file: the
/// tokens and the syntax tree are `file`'s alone; the types and the IR are
/// `file`'s, file `index`, as checked in the workspace; the value is
/// `file`'s own, after the files it needs have run. Warnings are the whole
/// workspace's, shown against `shown`.
fn execute_workspace(
    args: &RunArgs,
    file: &SourceFile,
    sources: &[SourceFile],
    shown: &[SourceFile],
    index: u32,
) -> Result<(), Failure> {
    let last = args.last_dump();
    let mut out = io::stdout().lock();
    if (args.dump_tokens || args.dump_ast) && syntax(args, file, &mut out)?.is_none() {
        return Ok(out.flush()?);
    }
    let mut workspace = workspace::check(sources).map_err(Failure::Program)?;
    if args.dump_types {
        writeln!(out, "{}", workspace.dump_types(index).unwrap_or_default())?;
    }
    if args.dump_ir {
        writeln!(out, "{}", workspace.dump_ir(index).unwrap_or_default())?;
    }
    if last.is_some() {
        output::diagnostics_in(shown, file, workspace.warnings());
        return Ok(out.flush()?);
    }
    let outcome = workspace.run(index).map_err(Failure::Program)?;
    output::diagnostics_in(shown, file, &outcome.warnings);
    print_value(&outcome.value, &mut out)?;
    Ok(out.flush()?)
}
