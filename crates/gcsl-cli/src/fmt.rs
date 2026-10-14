//! `gcsl fmt [--check] [PATHS...]`: lay files out in GCSL's one style
//! ([`gcsl::fmt`]).
//!
//! What is formatted: each file given, whatever its name; every `.gcsl`
//! file under each directory given, as [`gcsl_disk::workspace_files`] finds
//! a workspace's (names starting with `.` left out), and those of the
//! workspaces nested in it too; with no paths, every file of the workspace
//! the working directory is in ([`gcsl_disk::find_root`]), its nested
//! workspaces left out, as they are the workspace's own; or, for `-`, the
//! text read from standard input, written to standard output, which
//! cannot be given with other paths. Each file is named in what is shown as
//! it was given, or for the files found, relative to the working directory
//! when they are under it ([`gcsl_disk::shown`]).
//!
//! A file is written only if formatting changes it, so a formatted file
//! keeps its modification time, and then atomically: the text goes to a
//! temporary file beside it, `.NAME.fmt-PID`, which a workspace's listing
//! leaves out, with the file's permissions, which is then renamed over it.
//! For a symbolic link, the file it leads to is written, and the link kept.
//! A file whose line breaks are `\r\n` is written with `\n`. A file with a
//! lexical or syntax error is left alone, its errors shown, and the others
//! are formatted. With `--check` nothing is written: the files that
//! formatting would change are listed on standard output, one per line
//! (`<stdin>` for `-`), and standard input's formatted text is not shown.
//!
//! The exit code is 2 if a path cannot be read or a file written, or the
//! usage is wrong; else 1 if a file has an error, or with `--check` if one
//! would change; else 0.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use gcsl::SourceFile;

use crate::{EXIT_PROGRAM_ERROR, EXIT_USAGE_OR_IO, output};

/// Arguments of `gcsl fmt`.
#[derive(Args)]
pub struct FmtArgs {
    /// Write nothing: list the files formatting would change, and exit with
    /// code 1 if it would change any.
    #[arg(long)]
    check: bool,
    /// Files to format, directories to format every `.gcsl` file under, or
    /// `-` to format standard input to standard output. With none, every
    /// file of the workspace the working directory is in.
    paths: Vec<PathBuf>,
}

/// How a run of `gcsl fmt` went, the worst of what happened to each file.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    /// Every file is formatted.
    #[default]
    Clean,
    /// A file has errors, or with `--check` would change: exit code 1.
    Failed,
    /// A path could not be read or a file written: exit code 2.
    Broken,
}

impl Outcome {
    /// The process's exit code.
    fn code(self) -> ExitCode {
        match self {
            Outcome::Clean => ExitCode::SUCCESS,
            Outcome::Failed => ExitCode::from(EXIT_PROGRAM_ERROR),
            Outcome::Broken => ExitCode::from(EXIT_USAGE_OR_IO),
        }
    }
}

/// Runs `gcsl fmt` and returns the process exit code.
pub fn main(args: &FmtArgs) -> ExitCode {
    if args.paths.iter().any(|path| path.as_os_str() == "-") {
        if args.paths.len() > 1 {
            output::error("`-` formats standard input, and cannot be given with other paths");
            return ExitCode::from(EXIT_USAGE_OR_IO);
        }
        return stdin(args.check).code();
    }
    let mut outcome = Outcome::Clean;
    let Ok(files) = files(&args.paths) else {
        return Outcome::Broken.code();
    };
    for (shown, path) in files {
        outcome = outcome.max(file(&shown, &path, args.check));
    }
    outcome.code()
}

/// Files to format, each with the name it is shown by and its path.
type Files = Vec<(String, PathBuf)>;

/// The files to format for `paths`; `Err` once what cannot be found is
/// reported.
fn files(paths: &[PathBuf]) -> Result<Files, ()> {
    let mut files = Vec::new();
    if paths.is_empty() {
        let cwd = std::env::current_dir().and_then(fs::canonicalize);
        let cwd = cwd.map_err(|error| {
            output::error(format_args!("cannot find the working directory: {error}"));
        })?;
        let Some(root) = gcsl_disk::find_root(&cwd) else {
            output::error(format_args!(
                "no `{}` in {} or above it: give the files or directories to format",
                gcsl_disk::WORKSPACE,
                cwd.display()
            ));
            return Err(());
        };
        let (found, _) = listed(root)?;
        files.extend(found);
        return Ok(files);
    }
    let mut broken = false;
    for path in paths {
        if path.is_dir() {
            match under(path) {
                Ok(found) => files.extend(found),
                Err(()) => broken = true,
            }
        } else if path.is_file() {
            files.push((path.display().to_string(), path.clone()));
        } else {
            output::error(format_args!(
                "cannot read {}: no such file or directory",
                path.display()
            ));
            broken = true;
        }
    }
    if broken { Err(()) } else { Ok(files) }
}

/// The `.gcsl` files under `dir`, those of the workspaces nested in it too,
/// each shown as [`gcsl_disk::shown`] shows it.
fn under(dir: &Path) -> Result<Files, ()> {
    let (mut files, nested) = listed(dir)?;
    for inner in nested {
        files.extend(under(&dir.join(inner))?);
    }
    Ok(files)
}

/// The files of the workspace under `root` ([`gcsl_disk::workspace_files`]),
/// each shown as [`gcsl_disk::shown`] shows it, and the directories nested
/// in it that are workspaces of their own.
fn listed(root: &Path) -> Result<(Files, Vec<String>), ()> {
    let (found, nested) = gcsl_disk::workspace_files(root).map_err(|unreadable| {
        output::error(format_args!(
            "cannot read {}: {}",
            gcsl_disk::shown(&unreadable.path).display(),
            unreadable.error
        ));
    })?;
    let files = found
        .into_iter()
        .map(|(_, path)| (gcsl_disk::shown(&path).display().to_string(), path))
        .collect();
    Ok((files, nested))
}

/// Formats the file at `path`, shown as `shown`: writes it if it changes,
/// or with `check` lists it.
fn file(shown: &str, path: &Path, check: bool) -> Outcome {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            output::error(format_args!("cannot read {shown}: {error}"));
            return Outcome::Broken;
        }
    };
    let source = match SourceFile::new(shown, text.as_str()) {
        Ok(source) => source,
        Err(error) => {
            output::error(format_args!("cannot load {shown}: {error}"));
            return Outcome::Broken;
        }
    };
    let formatted = match gcsl::fmt::format(&source) {
        Ok(formatted) => formatted,
        Err(diagnostics) => {
            output::diagnostics(&source, &diagnostics);
            return Outcome::Failed;
        }
    };
    if formatted == text {
        return Outcome::Clean;
    }
    if check {
        let _ = writeln!(io::stdout().lock(), "{shown}");
        return Outcome::Failed;
    }
    match write_atomically(path, &formatted) {
        Ok(()) => Outcome::Clean,
        Err(error) => {
            output::error(format_args!("cannot write {shown}: {error}"));
            Outcome::Broken
        }
    }
}

/// Writes `text` over the file at `path` atomically: to a temporary file
/// beside it, with its permissions, renamed over it. A symbolic link is
/// followed, and the file it leads to written, so that the link stays one.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let path = &fs::canonicalize(path)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.fmt-{}", std::process::id()));
    let written = fs::metadata(path).and_then(|metadata| {
        fs::write(&temporary, text)?;
        fs::set_permissions(&temporary, metadata.permissions())?;
        fs::rename(&temporary, path)
    });
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Formats standard input to standard output, or with `check` writes
/// nothing but `<stdin>` if it would change.
fn stdin(check: bool) -> Outcome {
    const NAME: &str = "<stdin>";
    let mut text = String::new();
    if let Err(error) = io::stdin().lock().read_to_string(&mut text) {
        output::error(format_args!("cannot read standard input: {error}"));
        return Outcome::Broken;
    }
    let source = match SourceFile::new(NAME, text.as_str()) {
        Ok(source) => source,
        Err(error) => {
            output::error(format_args!("cannot load {NAME}: {error}"));
            return Outcome::Broken;
        }
    };
    let formatted = match gcsl::fmt::format(&source) {
        Ok(formatted) => formatted,
        Err(diagnostics) => {
            output::diagnostics(&source, &diagnostics);
            return Outcome::Failed;
        }
    };
    let mut out = io::stdout().lock();
    let written = if check {
        if formatted == text {
            return Outcome::Clean;
        }
        writeln!(out, "{NAME}").map(|()| Outcome::Failed)
    } else {
        out.write_all(formatted.as_bytes()).map(|()| Outcome::Clean)
    };
    written
        .and_then(|outcome| out.flush().map(|()| outcome))
        .unwrap_or_else(|error| {
            output::error(format_args!("cannot write output: {error}"));
            Outcome::Broken
        })
}
