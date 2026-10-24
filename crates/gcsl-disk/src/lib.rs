//! Finding and reading a GCSL workspace on disk, for the programs that run
//! one: `gcsl` (`gcsl-cli`), and `pekit`, which starts from a directory.
//!
//! The `gcsl` library reads no file ([`gcsl::workspace`]): it is given the
//! files of a workspace as [`SourceFile`]s named by their paths relative to
//! the root, `/` separated, with the root's absolute path. This crate finds
//! and reads them. A *workspace* is the directory tree under the nearest
//! directory holding a file named `workspace.gcsl` ([`WORKSPACE`]), found
//! above a directory, the directory itself included ([`find_root`]), as a
//! program that starts from its working directory finds it, or above a
//! file ([`find_workspace`]), as `gcsl run FILE` does, with the file's path
//! relative to the root.
//!
//! Its files ([`workspace_files`]) are every `.gcsl` file of the tree, in
//! path order, except those whose name, or a directory's on the way, starts
//! with `.`, and those under a directory holding its own `workspace.gcsl`,
//! which is a workspace of its own: such directories are listed apart, so
//! that the library can say why an import of one finds nothing. A directory
//! that is a symbolic link, which could lead back into the tree, is
//! skipped; a file that is one is read as any file. [`read_workspace`]
//! reads every file into two lists of sources, in path order: named by
//! their paths relative to the root, as the library is given them, and
//! named as they are shown to the user ([`shown`]: relative to the current
//! directory when under it, else in full), which the diagnostics are
//! rendered against by input number. A file the caller has read already,
//! the file `gcsl run` was given, is given to it with the name it is shown
//! by, and is read once: its text is the one given.
//!
//! # Libraries
//!
//! A file imports a library by its path, `import "github.com/gcslib/gcc"`
//! (see [`gcsl::workspace`], "Libraries"). [`read_workspace`] reads every
//! library the workspace's files import, and every library those import,
//! each from the first of the *library roots* ([`library_roots`]) that has a
//! directory of its path: each directory of `$GCSL_LIBRARY_PATH`, then
//! `.gcsl/libraries/` under the workspace's root. A library is the `.gcsl`
//! files directly in its directory, named `<lib>/PATH/FILE` for the library,
//! wherever they were found, and shown as any file is. A library no root has
//! is not read: the library reports it at the import.
//!
//! The root must be absolute and canonical for the library's `fileroot`
//! module. [`find_workspace`] canonicalises the file it is given
//! (`fs::canonicalize`), once, so the root it finds above it is canonical
//! too; [`find_root`] takes its directory as it is given, so a caller gives
//! it a canonical one. Nothing is printed: what fails is returned
//! ([`Unreadable`], [`ReadError`]) for the caller to report.

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro
)]

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use gcsl::SourceFile;
use gcsl::source::SourceError;

/// The name of the file that makes its directory a workspace's root.
pub const WORKSPACE: &str = "workspace.gcsl";

/// The root of the workspace `dir` is in: the nearest directory at or above
/// `dir`, `dir` itself first, holding a file named `workspace.gcsl`; `None`
/// when there is none.
///
/// `dir` is taken as it is given, and the directories above it are found
/// from its components alone, so give it absolute and canonical
/// (`fs::canonicalize`), as the root the library is given must be: the root
/// found is then canonical too.
#[must_use]
pub fn find_root(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|dir| dir.join(WORKSPACE).is_file())
}

/// The root of the workspace `file` is a file of, and `file`'s path
/// relative to it, `/` separated: the nearest directory above `file`
/// holding a `workspace.gcsl` ([`find_root`] of its directory), found from
/// `file`'s canonical path, so that the root is canonical.
///
/// `None` when there is none, and also when `file` is not one of the
/// workspace's files (a name starting with `.` on the way, or not a
/// `.gcsl` file), or its path cannot be followed or is not UTF-8: such a
/// file runs alone, as a file outside any workspace does.
#[must_use]
pub fn find_workspace(file: &Path) -> Option<(PathBuf, String)> {
    let file = fs::canonicalize(file).ok()?;
    let root = find_root(file.parent()?)?;
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

/// A file or directory of a workspace that could not be listed or read.
#[derive(Debug)]
pub struct Unreadable {
    /// The path of the file or directory.
    pub path: PathBuf,
    /// Why it could not be read.
    pub error: io::Error,
}

/// What [`workspace_files`] finds: the files, each with its path relative to
/// the root and its full path, and the directories left out as workspaces
/// of their own.
pub type Found = (Vec<(String, PathBuf)>, Vec<String>);

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
///
/// # Errors
///
/// The first directory that cannot be listed, or entry of one that cannot
/// be read, with why.
pub fn workspace_files(root: &Path) -> Result<Found, Unreadable> {
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
#[must_use]
pub fn shown(path: &Path) -> PathBuf {
    std::env::current_dir()
        .and_then(fs::canonicalize)
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
}

/// The files of a workspace, read ([`read_workspace`]).
#[derive(Clone, Debug)]
pub struct Sources {
    /// Every file, in path order, named by its path relative to the root,
    /// `/` separated: what the library is given, which numbers them, after
    /// the standard files it loads, in this order.
    pub relative: Vec<SourceFile>,
    /// The same files, in the same order, each named as it is shown
    /// ([`shown`]), but the file given, by the name it was given with: what
    /// diagnostics are rendered against.
    pub shown: Vec<SourceFile>,
    /// The directories left out as workspaces of their own, by their paths
    /// relative to the root (see [`workspace_files`]): what the library is
    /// told of them.
    pub nested: Vec<String>,
    /// The position among the files of the file given, if one was and it
    /// is one of them; `None` for a file given that is not (a symbolic link
    /// to a directory on its way, say), which then runs alone.
    pub given: Option<usize>,
}

/// Why a workspace could not be read ([`read_workspace`]). Shown, it is the
/// line the CLI prints after `error: `.
#[derive(Debug)]
pub enum ReadError {
    /// A file or directory could not be listed or read: "cannot read
    /// PATH: why".
    Unreadable {
        /// Its path, as it is shown ([`shown`]).
        path: PathBuf,
        /// Why it could not be read.
        error: io::Error,
    },
    /// A file was read, but cannot be a source: "cannot load NAME: why".
    Unloadable {
        /// Its name, as it is shown ([`shown`]).
        name: String,
        /// Why it cannot be a source.
        error: SourceError,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Unreadable { path, error } => {
                write!(f, "cannot read {}: {error}", path.display())
            }
            ReadError::Unloadable { name, error } => write!(f, "cannot load {name}: {error}"),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReadError::Unreadable { error, .. } => Some(error),
            ReadError::Unloadable { error, .. } => Some(error),
        }
    }
}

/// Reads every file of the workspace under `root` ([`workspace_files`]),
/// in path order, into the two lists of sources of [`Sources`], once each.
/// `given` is a file read already, by its path relative to `root` and as
/// its source, named as it is shown: if it is one of the files, its text is
/// the one given, and it is not read again.
///
/// # Errors
///
/// The first directory or file that cannot be listed or read, and the
/// first file that cannot be a source, in path order, each by its path as
/// it is shown.
pub fn read_workspace(
    root: &Path,
    given: Option<(&str, &SourceFile)>,
) -> Result<Sources, ReadError> {
    let (paths, nested) =
        workspace_files(root).map_err(|Unreadable { path, error }| ReadError::Unreadable {
            path: shown(&path),
            error,
        })?;
    let mut relative = Vec::new();
    // In path order, as the workspace numbers its files after the standard
    // files it loads.
    let mut shown_sources = Vec::new();
    for (name, path) in &paths {
        let (shown_name, text) = match given {
            Some((at, file)) if name == at => (file.name().to_owned(), file.text().to_owned()),
            _ => {
                let shown_path = shown(path);
                let shown_name = shown_path.display().to_string();
                match fs::read_to_string(path) {
                    Ok(text) => (shown_name, text),
                    Err(error) => {
                        return Err(ReadError::Unreadable {
                            path: shown_path,
                            error,
                        });
                    }
                }
            }
        };
        match (
            SourceFile::new(name.clone(), text.clone()),
            SourceFile::new(shown_name.clone(), text),
        ) {
            (Ok(source), Ok(shown_source)) => {
                relative.push(source);
                shown_sources.push(shown_source);
            }
            (Err(error), _) | (_, Err(error)) => {
                return Err(ReadError::Unloadable {
                    name: shown_name,
                    error,
                });
            }
        }
    }
    read_libraries(root, &mut relative, &mut shown_sources)?;
    let given = given.and_then(|(at, _)| relative.iter().position(|source| source.name() == at));
    Ok(Sources {
        relative,
        shown: shown_sources,
        nested,
        given,
    })
}

/// The directories libraries are looked for in, in order: each of
/// `$GCSL_LIBRARY_PATH`'s, then `.gcsl/libraries` under the workspace's root
/// `root` (see the module documentation, "Libraries").
#[must_use]
pub fn library_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = std::env::var_os("GCSL_LIBRARY_PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|dir| !dir.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    roots.push(root.join(".gcsl").join("libraries"));
    roots
}

/// Adds to `relative` and `shown_sources` the files of every library they import,
/// and every library those import, each found in the first of the library
/// roots of `root` that has it, and both lists put in the order the library
/// numbers its files in, by their names relative to the root (see the module
/// documentation, "Libraries"). A library no root has is left for the
/// library to report at its import.
fn read_libraries(
    root: &Path,
    relative: &mut Vec<SourceFile>,
    shown_sources: &mut Vec<SourceFile>,
) -> Result<(), ReadError> {
    let roots = library_roots(root);
    let mut wanted: Vec<String> = relative
        .iter()
        .flat_map(gcsl::workspace::library_imports)
        .collect();
    let mut done: Vec<String> = Vec::new();
    while let Some(library) = wanted.pop() {
        if done.contains(&library) {
            continue;
        }
        done.push(library.clone());
        let Some(dir) = roots
            .iter()
            .map(|at| at.join(&library))
            .find(|dir| dir.is_dir())
        else {
            continue;
        };
        let unreadable = |error| ReadError::Unreadable {
            path: shown(&dir),
            error,
        };
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(unreadable)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension() == Some(OsStr::new("gcsl"))
                    && path.is_file()
                    && !path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            })
            .collect();
        files.sort();
        for path in files {
            let text = fs::read_to_string(&path).map_err(|error| ReadError::Unreadable {
                path: shown(&path),
                error,
            })?;
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            let name = format!(
                "{}/{library}/{}",
                gcsl::workspace::LIBRARIES,
                file_name.unwrap_or_default()
            );
            let shown_name = shown(&path).display().to_string();
            let loaded = SourceFile::new(name, text.clone())
                .and_then(|source| Ok((source, SourceFile::new(shown_name.clone(), text)?)));
            let (source, shown_source) = loaded.map_err(|error| ReadError::Unloadable {
                name: shown_name,
                error,
            })?;
            wanted.extend(gcsl::workspace::library_imports(&source));
            relative.push(source);
            shown_sources.push(shown_source);
        }
    }
    // As the library sorts its files, by name.
    let mut pairs: Vec<(SourceFile, SourceFile)> = std::mem::take(relative)
        .into_iter()
        .zip(std::mem::take(shown_sources))
        .collect();
    pairs.sort_by(|a, b| a.0.name().as_bytes().cmp(b.0.name().as_bytes()));
    (*relative, *shown_sources) = pairs.into_iter().unzip();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory of the test `name`'s own, empty, canonical,
    /// under the system's temporary directory, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gcsl-disk-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(fs::canonicalize(dir).unwrap())
        }

        /// Writes `text` to the file at `path` below the directory, making
        /// the directories on the way.
        fn write(&self, path: &str, text: &str) -> PathBuf {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            path
        }

        /// Makes the directory at `path` below the directory.
        fn dir(&self, path: &str) -> PathBuf {
            let path = self.0.join(path);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_root_is_found_above_a_directory() {
        let scratch = Scratch::new("root");
        let root = &scratch.0;
        scratch.write(WORKSPACE, "");
        // Several directories deep, and the directory itself.
        let deep = scratch.dir("a/b/c/d");
        assert_eq!(find_root(&deep), Some(root.as_path()));
        assert_eq!(find_root(root), Some(root.as_path()));
        // The nearest: a nested workspace is a workspace of its own.
        scratch.write("a/b/workspace.gcsl", "");
        assert_eq!(find_root(&deep), Some(root.join("a/b").as_path()));
        assert_eq!(find_root(&root.join("a")), Some(root.as_path()));
        // A directory named like the file is no workspace's root.
        let other = Scratch::new("no-root");
        let inner = other.dir("x/workspace.gcsl/y");
        assert_eq!(find_root(&inner), None);
        assert_eq!(find_root(&other.0), None);
    }

    #[test]
    fn a_file_s_workspace_is_found_above_it() {
        let scratch = Scratch::new("file");
        let root = &scratch.0;
        scratch.write(WORKSPACE, "");
        let file = scratch.write("pkg/sub/app.gcsl", "x = 1");
        assert_eq!(
            find_workspace(&file),
            Some((root.clone(), "pkg/sub/app.gcsl".to_owned()))
        );
        // Not one of its files: a name starting with `.`, or not `.gcsl`.
        assert_eq!(find_workspace(&scratch.write(".hidden/a.gcsl", "")), None);
        assert_eq!(find_workspace(&scratch.write("notes.txt", "")), None);
    }

    #[test]
    fn the_files_of_a_workspace_are_listed() {
        let scratch = Scratch::new("files");
        let root = &scratch.0;
        scratch.write(WORKSPACE, "");
        scratch.write("z.gcsl", "");
        scratch.write("lib/a.gcsl", "");
        scratch.write("lib/notes.txt", "");
        // Names starting with `.`, a file's and a directory's.
        scratch.write(".dot.gcsl", "");
        scratch.write("lib/.b.gcsl", "");
        scratch.write(".hidden/c.gcsl", "");
        // A nested workspace, left out and listed apart.
        scratch.write("vendor/workspace.gcsl", "");
        scratch.write("vendor/v.gcsl", "");
        scratch.write("deep/er/workspace.gcsl", "");
        // A symbolic link to a directory, skipped; to a file, a file.
        std::os::unix::fs::symlink(root.join("lib"), root.join("linked")).unwrap();
        std::os::unix::fs::symlink(root.join("z.gcsl"), root.join("y.gcsl")).unwrap();
        let (files, nested) = workspace_files(root).unwrap();
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["lib/a.gcsl", "workspace.gcsl", "y.gcsl", "z.gcsl"]);
        assert_eq!(files[0].1, root.join("lib/a.gcsl"));
        assert_eq!(nested, ["deep/er", "vendor"]);
    }

    #[test]
    fn a_workspace_is_read_once_into_both_lists() {
        let scratch = Scratch::new("read");
        let root = &scratch.0;
        scratch.write(WORKSPACE, "w = 1");
        scratch.write("app/main.gcsl", "on disk");
        scratch.write("vendor/workspace.gcsl", "");
        // A file given is not read again: its text is the one given.
        let given = SourceFile::new("shown/main.gcsl", "given").unwrap();
        let sources = read_workspace(root, Some(("app/main.gcsl", &given))).unwrap();
        let named = |list: &[SourceFile]| -> Vec<(String, String)> {
            list.iter()
                .map(|source| (source.name().to_owned(), source.text().to_owned()))
                .collect()
        };
        assert_eq!(
            named(&sources.relative),
            [
                ("app/main.gcsl".to_owned(), "given".to_owned()),
                ("workspace.gcsl".to_owned(), "w = 1".to_owned()),
            ]
        );
        let shown_root = shown(&root.join(WORKSPACE)).display().to_string();
        assert_eq!(
            named(&sources.shown),
            [
                ("shown/main.gcsl".to_owned(), "given".to_owned()),
                (shown_root, "w = 1".to_owned()),
            ]
        );
        assert_eq!(sources.nested, ["vendor"]);
        assert_eq!(sources.given, Some(0));
        // Read from a directory, with no file given.
        let sources = read_workspace(root, None).unwrap();
        assert_eq!(sources.relative[0].text(), "on disk");
        assert_eq!(sources.given, None);
        // A file given that is none of the workspace's.
        let sources = read_workspace(root, Some(("elsewhere.gcsl", &given))).unwrap();
        assert_eq!(sources.given, None);
    }

    #[test]
    fn what_cannot_be_read_is_an_error() {
        let scratch = Scratch::new("unreadable");
        let missing = scratch.0.join("missing");
        let error = read_workspace(&missing, None).unwrap_err();
        assert!(matches!(error, ReadError::Unreadable { .. }), "{error:?}");
        let text = error.to_string();
        assert!(
            text.starts_with(&format!("cannot read {}: ", shown(&missing).display())),
            "{text}"
        );
    }
}
