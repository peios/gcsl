//! Files and directories: making a `File` or a `Dir` of a path, reading a
//! file, listing a directory and composing one.
//!
//! # Files
//!
//! A file, [`Value::File`], holds its path, absolute, and the identity of
//! its content as a host records it, `None` under plain GCSL, which never
//! looks at a file's content to make one (see the checker's
//! [Files](crate::check::files#files)). Two files are equal, and hash alike,
//! if both are; one shows as the call that makes it, `file("/src/main.c")`.
//! What a host does with the files a value holds, a step's arguments or its
//! result, it finds with [`Value::files`].
//!
//! `file(path)`, the built-in `file`, makes one (`make_file`). A relative
//! path is resolved against the working directory, the one a program that
//! `exec` runs is given by default, so that a file a program writes there
//! is found by the same relative path: this process's under plain GCSL, a
//! step's under a host that runs steps in directories of their own. The
//! path is then normalised lexically, as `path.normalize` does: `.`
//! segments and repeated `/` dropped and each `..` taking the segment
//! before it, never above `/`; no symbolic link is resolved. Nothing is read:
//! a file that is not there is found missing when it is read. An empty path
//! is "`file` needs a path", and a working directory that cannot be found
//! is "cannot find the working directory", noted with the OS's reason.
//!
//! `f.read()`, the built-in `file_read` (`read_file`), is the file's
//! contents, which must be UTF-8: "cannot read `/a/b`", noted with the
//! OS's reason, or "`/a/b` is not UTF-8 text" otherwise. Both are
//! `#[track_caller]` in the prelude, so their errors are reported at the
//! user's call.
//!
//! # Directories
//!
//! A directory, [`Value::Dir`], is a tree of files: a path and an identity,
//! as a file's, `None` under plain GCSL; a host finds the trees a value
//! holds with [`Value::dirs`]. `dir(path)` makes one as `file` does
//! (`make_file`). In one, `d.file(path)` and `d.dir(path)` are the file and
//! the directory at `path`, relative, a leading `/` meaning its root,
//! normalised lexically (`inside`): a path that would leave it is "`../x`
//! is not inside `/d`", and `d.file` needs a path of a file, not the
//! directory itself. Neither reads anything. `d.files()` (`dir_files`)
//! is every entry of its tree that is no directory, a symbolic link
//! included and never followed, sorted by path: "cannot list `/d`" if it
//! cannot be read.
//!
//! `fs.compose_dir(entries)` (`Evaluator::compose_dir`) makes a new
//! directory of the run, removed with its temporary directories, and places
//! each entry of the record `entries` at its key, a path in it, relative,
//! `/` or `.` being the directory itself, in order: a `Dir`'s tree under
//! it, a `File` at it, and each file of a `List<File>` under it by its name.
//! Directories merge; everything is copied, a file's permissions kept and a
//! symbolic link copied as a link, so that a program writing in the new
//! directory changes nothing it was made of. Two entries placing one path
//! is "two entries place `a/b`", a key that would leave the directory is
//! "`../x` is outside the directory", and a file at the directory itself,
//! or a copy that fails, are errors too, each naming `fs.compose_dir`.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{File, List, Value};

use super::effects::Effects;
use super::{Evaluator, Flow, mistyped};

/// Applies `file`, or `dir` if `builtin` is that, for the call `node`, to
/// `path` (see [Files](#files)): the file or directory at `path`, made
/// absolute against the working directory and normalised lexically.
pub(super) fn make_file(
    builtin: Builtin,
    path: &str,
    effects: &dyn Effects,
    node: &IrNode,
) -> Flow<Value> {
    if path.is_empty() {
        let message = format!("`{}` needs a path", builtin.name());
        return Err(Diagnostic::error(message, node.span).into());
    }
    let given = Path::new(path);
    let absolute = if given.is_absolute() {
        given.to_path_buf()
    } else {
        match effects.working_dir() {
            Ok(cwd) => cwd.join(given),
            Err(reason) => {
                return Err(
                    Diagnostic::error("cannot find the working directory", node.span)
                        .with_note(reason)
                        .into(),
                );
            }
        }
    };
    let Some(text) = normalised(&absolute) else {
        return Err(Diagnostic::error(
            format!("`{}` is not a UTF-8 path", absolute.display()),
            node.span,
        )
        .into());
    };
    Ok(entry(builtin == Builtin::Dir, Rc::from(text)))
}

/// A directory if `dir`, else a file, at `path`, of no identity.
fn entry(dir: bool, path: Rc<str>) -> Value {
    let file = Rc::new(File {
        path,
        identity: None,
    });
    if dir {
        Value::Dir(file)
    } else {
        Value::File(file)
    }
}

/// Applies `dir_file`, or `dir_dir` if `builtin` is that, for the call
/// `node`, to the directory `dir` and the path `path` in it (see
/// [Directories](#directories)): the file or directory at `path`, relative,
/// a leading `/` meaning `dir`'s root, normalised lexically. A path that
/// leaves `dir` is "`../x` is not inside `/d`"; `dir_file`'s needs a name,
/// "`d.file` needs a path inside the directory" otherwise.
pub(super) fn inside(builtin: Builtin, dir: &File, path: &str, node: &IrNode) -> Flow<Value> {
    let Some(relative) = relative(path) else {
        let message = format!("`{path}` is not inside `{}`", dir.path);
        return Err(Diagnostic::error(message, node.span).into());
    };
    let wants_dir = builtin == Builtin::DirDir;
    if relative.is_empty() && !wants_dir {
        let message = "`d.file` needs a path inside the directory";
        return Err(Diagnostic::error(message, node.span).into());
    }
    let joined = if relative.is_empty() {
        Rc::clone(&dir.path)
    } else if &*dir.path == "/" {
        Rc::from(format!("/{relative}"))
    } else {
        Rc::from(format!("{}/{relative}", dir.path))
    };
    Ok(entry(wants_dir, joined))
}

/// `path` relative, a leading `/` dropped, and normalised lexically: `.`
/// and repeated `/` dropped and each `..` taking the segment before it;
/// `None` if a `..` would leave the root. `""` for the root itself.
pub(super) fn relative(path: &str) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                kept.pop()?;
            }
            segment => kept.push(segment),
        }
    }
    Some(kept.join("/"))
}

/// Applies `dir_files`, for the call `node`, to `dir` (see
/// [Directories](#directories)): every entry of its tree that is no
/// directory, a symbolic link included and never followed, sorted by path.
pub(super) fn dir_files(dir: &File, effects: &dyn Effects, node: &IrNode) -> Flow<Value> {
    let mut found = Vec::new();
    let walked = effects
        .host_path(&dir.path)
        .map_err(io::Error::other)
        .and_then(|host| walk(&host, (&dir.path, ""), &mut found));
    if let Err(error) = walked {
        return Err(
            Diagnostic::error(format!("cannot list `{}`", dir.path), node.span)
                .with_note(error.to_string())
                .into(),
        );
    }
    found.sort_unstable();
    let files = found
        .into_iter()
        .map(|path| entry(false, Rc::from(path)))
        .collect();
    Ok(Value::List(List::from_vec(files)))
}

/// Pushes the path of every entry under `host`, a directory of the host's
/// file system, that is no directory onto `found`, as GCSL code names it:
/// `root`, the directory's own name for it, joined with its path under the
/// directory, `under` so far. It descends into directories but never
/// through a symbolic link. A name that is not UTF-8 is an error.
fn walk(host: &Path, (root, under): (&str, &str), found: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(host)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|name| {
            io::Error::other(format!("`{}` is not a UTF-8 path", name.display()))
        })?;
        let inner = join(under, &name);
        if entry.file_type()?.is_dir() {
            walk(&entry.path(), (root, &inner), found)?;
        } else if root == "/" {
            found.push(format!("/{inner}"));
        } else {
            found.push(format!("{root}/{inner}"));
        }
    }
    Ok(())
}

impl Evaluator<'_> {
    /// Applies `fs_compose_dir`, `fs.compose_dir`, for the call `node`, to
    /// `entries` (see [Directories](#directories)): a new directory of the
    /// run, each entry placed at its key in it, in order, the directory
    /// returned.
    pub(super) fn compose_dir(&self, entries: &Value, node: &IrNode) -> Flow<Value> {
        let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
        let effects = self.shared.effects;
        let Some(made) = effects.temp_dir() else {
            return mistyped(node, "`fs.compose_dir` called in a run without an owner");
        };
        let made = made.and_then(|root| Ok((effects.host_path(&root)?, root)));
        let (host_root, root) = match made {
            Ok(made) => made,
            Err(reason) => {
                return fail(format!(
                    "`fs.compose_dir` could not create a temporary directory: {reason}"
                ));
            }
        };
        let mut composer = Composer {
            root: host_root,
            placed: HashSet::new(),
            effects,
        };
        let pairs: Vec<(&str, &Value)> = match entries {
            Value::Empty => Vec::new(),
            Value::Record(record) => record
                .layout
                .texts
                .iter()
                .map(|text| &**text)
                .zip(record.values.iter())
                .collect(),
            _ => return mistyped(node, "entries of `fs.compose_dir` that are no record"),
        };
        for (key, value) in pairs {
            let Some(at) = relative(key) else {
                return fail(format!(
                    "`fs.compose_dir`: `{key}` is outside the directory"
                ));
            };
            if let Err(message) = composer.place(&at, value) {
                return fail(format!("`fs.compose_dir`: {message}"));
            }
        }
        Ok(entry(true, Rc::from(root)))
    }
}

/// What [`Evaluator::compose_dir`] has placed so far, in the directory it
/// is composing.
pub(super) struct Composer<'a> {
    /// The directory, as the host's file system has it.
    pub(super) root: PathBuf,
    /// The paths of what has been placed in it, relative, but directories.
    pub(super) placed: HashSet<String>,
    /// The effect context, which says where the entries placed are (see
    /// [Effects](crate::eval::effects#effects)).
    pub(super) effects: &'a dyn Effects,
}

impl Composer<'_> {
    /// The path of the host's file system that `path`, as GCSL code names
    /// it, is.
    fn host(&self, path: &str) -> Result<PathBuf, String> {
        self.effects
            .host_path(path)
            .map_err(|reason| format!("cannot use `{path}`: {reason}"))
    }

    /// Copies the tree of the directory at `path`, as GCSL code names it,
    /// under `at`, as [`Composer::tree`] does.
    pub(super) fn tree_at(&mut self, path: &str, at: &str) -> Result<(), String> {
        let source = self.host(path)?;
        self.tree(&source, at)
    }

    /// Places `value`, a directory, a file or a list of files, at `at`,
    /// relative to the root (see [Directories](#directories)): a
    /// directory's tree under it, a file at it, and each file of a list
    /// under it by its name. The error, a message, if two entries place one
    /// path, or a copy fails.
    fn place(&mut self, at: &str, value: &Value) -> Result<(), String> {
        match value {
            Value::Dir(dir) => self.tree_at(&dir.path, at),
            Value::File(file) if at.is_empty() => Err(format!(
                "a file cannot be the directory itself, as `{}` is",
                file.path
            )),
            Value::File(file) => {
                let source = self.host(&file.path)?;
                self.file(&source, at)
            }
            Value::List(files) => {
                for file in files.values() {
                    let Value::File(file) = file else {
                        return Err("a list of entries holds what is no file".to_owned());
                    };
                    let name = Path::new(&*file.path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("");
                    let source = self.host(&file.path)?;
                    self.file(&source, &join(at, name))?;
                }
                Ok(())
            }
            _ => Err("an entry that is no directory, file or list of files".to_owned()),
        }
    }

    /// Copies the tree at `source` under `at`, its directories merged with
    /// any already there, its symbolic links copied as links.
    pub(super) fn tree(&mut self, source: &Path, at: &str) -> Result<(), String> {
        let target = self.root.join(at);
        fs::create_dir_all(&target).map_err(|error| format!("could not create `{at}`: {error}"))?;
        let mut entries: Vec<_> = fs::read_dir(source)
            .map_err(|error| format!("could not list `{}`: {error}", source.display()))?
            .collect::<Result<_, _>>()
            .map_err(|error| format!("could not list `{}`: {error}", source.display()))?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(format!("`{}` is not a UTF-8 path", entry.path().display()));
            };
            let kind = entry
                .file_type()
                .map_err(|error| format!("could not read `{}`: {error}", entry.path().display()))?;
            let inner = join(at, name);
            if kind.is_dir() {
                self.tree(&entry.path(), &inner)?;
            } else {
                self.file(&entry.path(), &inner)?;
            }
        }
        Ok(())
    }

    /// Copies the file at `source` to `at`, its directories made, its
    /// permissions kept, a symbolic link as a link; an error if something
    /// is placed there already.
    fn file(&mut self, source: &Path, at: &str) -> Result<(), String> {
        if !self.placed.insert(at.to_owned()) || self.root.join(at).is_dir() {
            return Err(format!("two entries place `{at}`"));
        }
        let target = self.root.join(at);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create `{}`: {error}", parent.display()))?;
        }
        let copied = match fs::symlink_metadata(source) {
            Ok(meta) if meta.file_type().is_symlink() => {
                fs::read_link(source).and_then(|link| std::os::unix::fs::symlink(link, &target))
            }
            Ok(_) => fs::copy(source, &target).map(|_| ()),
            Err(error) => Err(error),
        };
        copied.map_err(|error| format!("could not copy `{}`: {error}", source.display()))
    }
}

/// `name` under the relative path `at`, `""` being the root.
fn join(at: &str, name: &str) -> String {
    if at.is_empty() {
        name.to_owned()
    } else {
        format!("{at}/{name}")
    }
}

/// `path`, absolute, normalised lexically (see [Files](#files)), as text;
/// `None` if it is not UTF-8.
fn normalised(path: &Path) -> Option<String> {
    let mut kept = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::Normal(segment) => kept.push(segment),
            // `..` above `/` is `/`, as the OS takes it.
            Component::ParentDir => {
                kept.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    kept.into_os_string().into_string().ok()
}

/// Applies `file_read`, for the call `node`, to `file` (see [Files](#files)):
/// its contents, which must be UTF-8.
pub(super) fn read_file(file: &File, effects: &dyn Effects, node: &IrNode) -> Flow<Value> {
    let path = &*file.path;
    let read = effects
        .host_path(path)
        .map_err(io::Error::other)
        .and_then(std::fs::read);
    match read.map(String::from_utf8) {
        Ok(Ok(text)) => Ok(Value::Str(Rc::from(text))),
        Ok(Err(_)) => {
            Err(Diagnostic::error(format!("`{path}` is not UTF-8 text"), node.span).into())
        }
        Err(error) => Err(
            Diagnostic::error(format!("cannot read `{path}`"), node.span)
                .with_note(error.to_string())
                .into(),
        ),
    }
}
