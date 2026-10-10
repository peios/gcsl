//! `gcsl run FILE`: run a file, or dump its intermediate stages.
//!
//! A file runs alone unless it is a file of a *workspace*: the directory
//! tree under the nearest directory above it that holds a file named
//! `workspace.gcsl` ([`gcsl::workspace`]). Then every `.gcsl` file of the
//! tree is read (except names starting with `.`, and directories that
//! hold their own `workspace.gcsl`, which are workspaces of their own),
//! the whole workspace is checked, and `FILE` runs after the files it
//! needs; its own value is printed, with the warnings of every file.
//! Finding and reading the files is this module's: the library reads no
//! file. Each file is given to the library by its path relative to the
//! root, and its diagnostics name it by its path from the current
//! directory, `FILE` by the path given. The dumps are `FILE`'s alone: its
//! tokens and syntax tree as a file's, its attributes among them
//! (`Attribute(shadowable)` and `InnerAttribute(no_prelude)` tokens,
//! `#[shadowable]` before an item and `#![no_prelude]` after `file` in the
//! tree), its types and IR as checked and lowered in the workspace, which
//! show nothing of the implicit import of the prelude a file has.
//!
//! A file run alone is checked and run as the workspace of one directory
//! holding just it ([`gcsl::workspace::check_lone_with_standard`]), and its
//! dumps are its own as a workspace file's are. Either way the standard
//! library is there: the prelude (but in a file marked `#![no_prelude]`),
//! and each standard module a file imports
//! (`import path`), its files the first inputs, which the check says only
//! once it has scanned the files
//! ([`gcsl::workspace::check_with_standard`]), so the sources diagnostics
//! are rendered against, by input number, are known only then. A
//! diagnostic of a standard file, or a part of one in it, is rendered
//! against its source, named `<stdlib>/prelude.gcsl` say. So is the panic
//! of a failing `sh`, which is the prelude's, its message ending in the
//! command's standard error, trimmed. A program a file runs with
//! `exec` or `sh` has `gcsl`'s working directory and environment and the
//! null device as its standard input, and its output is captured into the
//! value, never shown; a `panic`, like any runtime error, is rendered to
//! stderr with exit code 1.
//!
//! The library is also given where the files are, for the `fileroot`
//! module (`import fileroot`): a workspace's root, which `find_workspace`
//! found above the file's canonical path (`fs::canonicalize`, once, so the
//! root is canonical too), or a lone file's directory, the parent of its
//! canonical path, the working directory if that cannot be found; a path
//! that is not UTF-8 is given lossily. So `fileroot.this`,
//! `fileroot.workspace` and `fileroot.caller` are absolute paths, the same
//! whatever directory `gcsl` runs in, though `exec` still runs its programs
//! in `gcsl`'s working directory.
//!
//! The directories left out as nested workspaces are passed to the
//! library too, by their paths relative to the root
//! ([`gcsl::workspace::check_with_standard`]): an `import` of a path into
//! one is "no module", and only this module knows why, so that the error's
//! note can say "`vendor/` is a workspace of its own". An `import` of
//! anything but the standard library in a file that is not in a workspace
//! is an error of that file, reported by the checker with its other errors.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use gcsl::ast::Ast;
use gcsl::lexer::TokenKind;
use gcsl::strings::Quoted;
use gcsl::workspace::{self, Workspace};
use gcsl::{Diagnostic, SourceFile, Value, lexer, parser};

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

/// Runs `source` alone, as a file outside any workspace: the workspace of
/// one directory holding just it, beside the standard library
/// ([`gcsl::workspace::check_lone_with_standard`]), whose root, for the
/// `fileroot` module, is the file's directory ([`file_dir`]). Its
/// diagnostics are shown against it, and a standard file's against that
/// file, named `<stdlib>/prelude.gcsl` say.
fn run_alone(args: &RunArgs, source: &SourceFile) -> ExitCode {
    let dir = file_dir(&args.file);
    let check = || workspace::check_lone_with_standard(source, &dir);
    execute_workspace(args, source, check, (vec![source.clone()], 0))
}

/// The directory of `file`, the file given, as the library takes a lone
/// file's directory: absolute and canonical, symbolic links resolved
/// ([`fs::canonicalize`] of the file, then its parent), without a trailing
/// `/` but for `/` itself, and lossily if it is not UTF-8. The working
/// directory, canonical, when the file's cannot be found, as for a file
/// that is no file on disk.
fn file_dir(file: &Path) -> String {
    fs::canonicalize(file)
        .ok()
        .and_then(|file| file.parent().map(|dir| dir.to_string_lossy().into_owned()))
        .unwrap_or_else(workspace::working_directory)
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
        // `Ident(x)`, `Label(outer)`, `Attribute(shadowable)` and
        // `StrStart("a\n")`.
        for token in &tokens {
            let span = token.span;
            match token.kind {
                TokenKind::Ident(name) => writeln!(out, "Ident({}) @{span}", names.get(name))?,
                TokenKind::Label(name) => writeln!(out, "Label({}) @{span}", names.get(name))?,
                TokenKind::Attribute(name) => {
                    writeln!(out, "Attribute({}) @{span}", names.get(name))?;
                }
                TokenKind::InnerAttribute(name) => {
                    writeln!(out, "InnerAttribute({}) @{span}", names.get(name))?;
                }
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

/// What [`workspace_files`] finds: the files, each with its path relative to
/// the root and its full path, and the directories left out as workspaces
/// of their own.
type Found = (Vec<(String, PathBuf)>, Vec<String>);

/// The files of the workspace under `root`, each with its path relative to
/// `root`, `/` separated, in path order, byte by byte: every `.gcsl` file
/// below it, except those whose name, or a directory's on the way, starts
/// with `.`, and those under a directory holding its own `workspace.gcsl`,
/// which is a workspace of its own. Names that are not UTF-8 are skipped,
/// and so is a directory that is a symbolic link, which could lead back
/// into the tree; a file that is one is read as any file. With them, the
/// directories left out as workspaces of their own, relative to `root`
/// without a trailing `/`, in path order: the workspace is told of them so
/// that an import of one says why it finds nothing.
fn workspace_files(root: &Path) -> Result<Found, Unreadable> {
    let mut files = Vec::new();
    let mut nested = Vec::new();
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
                if linked {
                    continue;
                }
                if path.join(WORKSPACE).is_file() {
                    nested.push(format!("{prefix}{name}"));
                } else {
                    dirs.push((format!("{prefix}{name}/"), path));
                }
            } else if path.extension().and_then(OsStr::to_str) == Some("gcsl") {
                files.push((format!("{prefix}{name}"), path));
            }
        }
    }
    files.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    nested.sort_unstable();
    Ok((files, nested))
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
    let (paths, nested) = match workspace_files(root) {
        Ok(found) => found,
        Err(Unreadable { path, error }) => {
            let path = shown(&path);
            output::error(format_args!("cannot read {}: {error}", path.display()));
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
    };
    let mut sources = Vec::new();
    // In path order, as the workspace numbers its files after the standard
    // files it loads.
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
    let Some(position) = paths.iter().position(|(name, _)| name == relative) else {
        // Not found among the files listed (a link, say): it runs alone.
        return run_alone(args, file);
    };
    // The root is canonical: `find_workspace` found it above the file's
    // canonical path.
    let root = root.to_string_lossy();
    let check = || workspace::check_with_standard(&sources, &nested, &root);
    execute_workspace(args, file, check, (shown_sources, position))
}

/// What checking a workspace gives: the standard files it loaded, its
/// first files by input number, and the workspace, or the diagnostics.
type Checked = (Vec<SourceFile>, Result<Workspace, Vec<Diagnostic>>);

/// Runs the pipeline on the workspace `check` checks ([`pipeline`]), whose
/// files are `files`, as they are shown, in path order, `file` being the
/// one at `position` among them, and returns the exit code, after
/// rendering any diagnostics: each against the source of the input it
/// names, the standard files the check loaded coming before `files`, or
/// against `file` when it names none.
fn execute_workspace(
    args: &RunArgs,
    file: &SourceFile,
    check: impl FnOnce() -> Checked,
    (files, position): (Vec<SourceFile>, usize),
) -> ExitCode {
    let mut shown = files;
    match pipeline(args, file, check, position, &mut shown) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Program(diagnostics)) => {
            output::diagnostics_in(&shown, file, &diagnostics);
            ExitCode::from(EXIT_PROGRAM_ERROR)
        }
        Err(failure) => exit_for(failure),
    }
}

/// Runs the pipeline on the workspace `check` checks, printing each
/// requested dump and stopping after the last one; with no dumps, prints
/// `file`'s value, as text, unless it is `{}`. The tokens and the syntax
/// tree are `file`'s alone; the types and the IR are `file`'s, the file at
/// `position` among the workspace's files, `shown`, as checked in the
/// workspace, its own block only; the value is `file`'s own, after the
/// files it needs, and the standard files the check loaded, have run. Once
/// the check has returned, the standard files it loaded are put before the
/// files of `shown`, which then holds every source by input number.
/// Warnings are the whole workspace's, shown against `shown`, once the run
/// has succeeded (after the dumps, or before the value); if a later stage
/// fails, they are returned with its error instead, to be printed together.
fn pipeline(
    args: &RunArgs,
    file: &SourceFile,
    check: impl FnOnce() -> Checked,
    position: usize,
    shown: &mut Vec<SourceFile>,
) -> Result<(), Failure> {
    let last = args.last_dump();
    let mut out = io::stdout().lock();
    if (args.dump_tokens || args.dump_ast) && syntax(args, file, &mut out)?.is_none() {
        return Ok(out.flush()?);
    }
    let (mut standard, checked) = check();
    let index = u32::try_from(standard.len() + position).unwrap_or(u32::MAX);
    standard.append(shown);
    *shown = standard;
    let mut workspace = checked.map_err(Failure::Program)?;
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
