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
//! path is resolved where it is written (see [Where a path
//! is](#where-a-path-is)), then normalised lexically, as `path.normalize`
//! does: `.` segments and repeated `/` dropped and each `..` taking the
//! segment before it, never above `/`; no symbolic link is resolved.
//! Nothing is read: a file that is not there is found missing when it is
//! read. An empty path is "`file` needs a path", and a working directory
//! that cannot be found is "cannot find the working directory", noted with
//! the OS's reason.
//!
//! # Where a path is
//!
//! A relative path is against the directory of the file the code that
//! makes it is written in, as a name is bound where it is written: the
//! directory of the running code's file, from the run's table
//! ([`Ir::dir`](crate::ir::Ir::dir)), the one `fileroot.this` names there.
//! So `file("lib.c")` in a function of `_lib/c.gcsl` is `_lib/lib.c`
//! whoever calls it, and a library's own files are its own wherever it is
//! used. A function that works on its caller's files takes them, a `File`
//! or a `Dir` its caller made, by a path relative to the caller's file. A
//! standard file's code works for its caller, as the prelude's `exec`
//! does, so its relative paths are its caller's (see [Callers and
//! helpers](crate::eval::callers#callers-and-helpers)). A run with no table
//! of directories, one with no workspace's context, is in the host's
//! working directory, [`Effects::working_dir`]. A relative
//! `cwd` of `exec` is a path too, made the directory it names before the
//! call runs or becomes an effect's node (`Evaluator::exec_options`).
//!
//! A program is given no file's directory: `exec` with a `cwd` of `""`
//! runs it in the host's working directory, the one a run has throughout,
//! a step's own directory under pekit, so that a memoised function that
//! runs a program is one call for whatever code calls it.
//! `working_dir()`, the built-in `working_dir`, is that directory, as a
//! `Dir`, where a step writes the files it makes.
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
//! cannot be read. A directory the host hides ([`Effects::hides`]), as
//! pekit hides its outputs from its workspace, is left out with all it
//! holds.
//!
//! `fs.compose_dir(entries)` (`Evaluator::compose_dir`) makes a new
//! directory of the run, removed with its temporary directories, and places
//! each entry of the record `entries` at its key, a path in it, relative,
//! `/` or `.` being the directory itself, in order: a `Dir`'s tree under
//! it, a `File` at it, and each file of a `List<File>` under it by its name.
//! Directories merge; everything is copied, a file's permissions kept and a
//! symbolic link copied as a link, but a directory the host hides, so that
//! a program writing in the new directory changes nothing it was made of.
//! Two entries placing one path is "two entries place `a/b`", a key that
//! would leave the directory is
//! "`../x` is outside the directory", and a file at the directory itself,
//! or a copy that fails, are errors too, each naming `fs.compose_dir`.

//! # Fetching
//!
//! `fetch(url, sha256)`, the built-in `fetch` (`fetch`), downloads the file
//! at `url` into a new temporary directory of the run, named by the last
//! segment of the URL's path, `download` if it has none, and checks it
//! against `sha256`, the SHA-256 digest of its contents in hex, any case:
//! the same digest is the same bytes, so the file is fixed wherever and
//! whenever it is fetched. It runs `curl` to download and `sha256sum` to
//! hash, through the run's [`Effects`] as any program, so GCSL itself needs
//! no network or hashing of its own. A pin that is no digest, 64 hex digits,
//! is an error before anything is downloaded; a file whose digest is not
//! the pin is "`fetch` of `URL` gave a file whose sha256 is `FOUND`, not
//! `PIN`", the file behind the URL having changed, which never builds from
//! it; and `sha256 = ""`, the default, downloads only to fail with the
//! digest it found, "`fetch` of `URL` is not pinned: its sha256 is `FOUND`",
//! for the author to pin it. A host that keeps files by their digests, as
//! pekit does, answers a call itself, from its store when it holds the
//! digest, so that a pinned file is downloaded once.
//!
//! `fetch_git(url, commit)`, the built-in `fetch_git` (`fetch_git`), makes a
//! new temporary directory of the run of the files of the git repository at
//! `url` as the commit `commit` has them, its submodules left out and its
//! own `.git` too: a repository's files at a commit are as fixed as a
//! digest's bytes, the commit's name being a digest of them. The pin is the
//! commit's whole name, 40 hex digits, any case; anything else, a branch or
//! a tag, which can move, is an error before anything is fetched. It runs
//! `git` through the run's [`Effects`]: a repository is made, the commit
//! fetched into it alone, shallowly, or, if the server won't give a commit
//! by its name, with everything else, checked out, checked to be the pin,
//! and its `.git` removed. A host that keeps repositories, as pekit does,
//! answers a call itself.
//!
//! # Unpacking
//!
//! `archive.unpack(a, strip)`, the built-in `archive_unpack` (`unpack`),
//! makes a new temporary directory of the run of what the tar archive `a`
//! holds, gzip, xz, bzip2 or zstd compressed or not, or the zip archive, as
//! a wheel or a jar is, which its first bytes tell, the first `strip`
//! components of each path left off, an entry with no more left out. It
//! runs `tar`, or `unzip`, through the run's [`Effects`], owners and the
//! time of each file not kept; `unzip` has no stripping of its own, so a
//! zip is unpacked whole first, and what is `strip` directories down moved
//! into place. A host may answer it itself, as pekit does, from its store when
//! it has unpacked the same archive before.

use std::collections::HashSet;
use std::fs;
use std::process::Command;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{File, List, Record, Value};

use super::effects::Effects;
use super::{Evaluator, Flow, internal, mistyped};

impl Evaluator<'_> {
    /// Applies `file`, or `dir` if `builtin` is that, for the call `node`,
    /// to `path` (see [Files](#files)): the file or directory at `path`, made
    /// absolute where it is written (see [Where a path is](#where-a-path-is))
    /// and normalised lexically.
    pub(super) fn make_file(&mut self, builtin: Builtin, path: &str, node: &IrNode) -> Flow<Value> {
        if path.is_empty() {
            let message = format!("`{}` needs a path", builtin.name());
            return Err(Diagnostic::error(message, node.span).into());
        }
        let given = Path::new(path);
        let absolute = if given.is_absolute() {
            given.to_path_buf()
        } else {
            self.code_dir(node)?.join(given)
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

    /// The options `options` of a call `node` of `exec`, `exec_live` or
    /// `exec_secret`, a relative `cwd` of theirs made absolute where it is
    /// written (see [Where a path is](#where-a-path-is)); as they are if
    /// their `cwd` is absolute or `""`.
    pub(super) fn exec_options(&mut self, options: Value, node: &IrNode) -> Flow<Value> {
        let Value::Record(record) = &options else {
            return Ok(options);
        };
        let Some(at) = record.layout.texts.iter().position(|text| &**text == "cwd") else {
            return Ok(options);
        };
        let Some(Value::Str(cwd)) = record.values.get(at) else {
            return Ok(options);
        };
        if cwd.is_empty() || Path::new(&**cwd).is_absolute() {
            return Ok(options);
        }
        let joined = self.code_dir(node)?.join(&**cwd);
        let Some(text) = normalised(&joined) else {
            return Err(Diagnostic::error(
                format!("`{}` is not a UTF-8 path", joined.display()),
                node.span,
            )
            .into());
        };
        let mut values = record.values.clone();
        if let Some(slot) = values.get_mut(at) {
            *slot = Value::Str(Rc::from(text));
        }
        Ok(Value::Record(Rc::new(Record {
            layout: Rc::clone(&record.layout),
            values,
        })))
    }

    /// The directory a relative path is against in the code running the
    /// call `node` (see [Where a path is](#where-a-path-is)): its file's,
    /// its caller's for a standard file's code, or the host's working
    /// directory in a run with no table of directories.
    fn code_dir(&mut self, node: &IrNode) -> Flow<PathBuf> {
        if self.shared.dirs.is_empty() {
            return self.working_dir(node);
        }
        if self.ir.is_standard() {
            return Ok(PathBuf::from(&*self.caller_text(node)?));
        }
        self.shared
            .dirs
            .get(self.ir.dir() as usize)
            .map(|dir| PathBuf::from(&**dir))
            .ok_or_else(|| internal("code outside the run's directories", node.span).into())
    }

    /// Applies `working_dir` for the call `node` (see [Where a path
    /// is](#where-a-path-is)): the host's working directory, as a `Dir`.
    pub(super) fn working_dir_value(&mut self, node: &IrNode) -> Flow<Value> {
        let dir = self.working_dir(node)?;
        let Some(text) = normalised(&dir) else {
            return Err(Diagnostic::error(
                format!("`{}` is not a UTF-8 path", dir.display()),
                node.span,
            )
            .into());
        };
        Ok(entry(true, Rc::from(text)))
    }

    /// The host's working directory, as GCSL code names paths, for the call
    /// `node` (see [Where a path is](#where-a-path-is)).
    pub(super) fn working_dir(&mut self, node: &IrNode) -> Flow<PathBuf> {
        self.shared.effects.working_dir().map_err(|reason| {
            Diagnostic::error("cannot find the working directory", node.span)
                .with_note(reason)
                .into()
        })
    }
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
        .and_then(|host| walk(&host, (&dir.path, ""), (effects, &mut found)));
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
/// through a symbolic link, nor into one `effects` hides. A name that is not
/// UTF-8 is an error.
fn walk(
    host: &Path,
    (root, under): (&str, &str),
    (effects, found): (&dyn Effects, &mut Vec<String>),
) -> io::Result<()> {
    for entry in fs::read_dir(host)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|name| {
            io::Error::other(format!("`{}` is not a UTF-8 path", name.display()))
        })?;
        let inner = join(under, &name);
        if entry.file_type()?.is_dir() {
            if !effects.hides(&entry.path()) {
                walk(&entry.path(), (root, &inner), (effects, &mut *found))?;
            }
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
        let pairs: Vec<(Rc<str>, Value)> = match entries {
            Value::Empty => Vec::new(),
            Value::Record(record) => record
                .layout
                .texts
                .iter()
                .map(|text| Rc::from(&**text))
                .zip(record.values.iter().cloned())
                .collect(),
            _ => return mistyped(node, "entries of `fs.compose_dir` that are no record"),
        };
        self.compose_pairs(&pairs, ("fs.compose_dir", node))
    }

    /// Applies `fs_compose`, for the call `node`, to `entries`, a list of
    /// records of a path, `at`, and what to place there (see
    /// [Directories](#directories)): `fs.compose_dir`'s directory, each entry
    /// placed at its path.
    pub(super) fn compose_list(&self, entries: &Value, node: &IrNode) -> Flow<Value> {
        let Value::List(list) = entries else {
            return mistyped(node, "entries of `fs.compose` that are no list");
        };
        let mut pairs: Vec<(Rc<str>, Value)> = Vec::new();
        for entry in list.values() {
            let Value::Record(record) = entry else {
                return mistyped(node, "an entry of `fs.compose` that is no record");
            };
            let field = |name: &str| {
                let at = record
                    .layout
                    .texts
                    .iter()
                    .position(|text| &**text == name)?;
                record.values.get(at).cloned()
            };
            let (Some(Value::Str(at)), Some(place)) = (field("at"), field("place")) else {
                return mistyped(node, "an entry of `fs.compose` of another shape");
            };
            pairs.push((at, place));
        }
        self.compose_pairs(&pairs, ("fs.compose", node))
    }

    /// Makes a new directory of the run of `pairs`, each a path in it and
    /// what to place there (see [Directories](#directories)), for the call
    /// `node` of the member `shown`.
    fn compose_pairs(
        &self,
        pairs: &[(Rc<str>, Value)],
        (shown, node): (&str, &IrNode),
    ) -> Flow<Value> {
        let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
        let effects = self.shared.effects;
        let Some(made) = effects.temp_dir() else {
            return mistyped(node, "a composition made in a run without an owner");
        };
        let made = made.and_then(|root| Ok((effects.host_path(&root)?, root)));
        let (host_root, root) = match made {
            Ok(made) => made,
            Err(reason) => {
                return fail(format!(
                    "`{shown}` could not create a temporary directory: {reason}"
                ));
            }
        };
        let mut composer = Composer {
            root: host_root,
            placed: HashSet::new(),
            dirs: HashSet::new(),
            effects,
        };
        for (key, value) in pairs {
            let Some(at) = relative(key) else {
                return fail(format!("`{shown}`: `{key}` is outside the directory"));
            };
            if let Err(message) = composer.place(&at, value) {
                return fail(format!("`{shown}`: {message}"));
            }
        }
        effects.composed(&root);
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
    /// The paths of the directories made in it, relative: a tree's, and
    /// those a file is placed in.
    pub(super) dirs: HashSet<String>,
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
    /// any already there, its symbolic links copied as links, and none the
    /// effect context hides ([`Effects::hides`]).
    pub(super) fn tree(&mut self, source: &Path, at: &str) -> Result<(), String> {
        let target = self.root.join(at);
        fs::create_dir_all(&target).map_err(|error| format!("could not create `{at}`: {error}"))?;
        self.made(at);
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
                if !self.effects.hides(&entry.path()) {
                    self.tree(&entry.path(), &inner)?;
                }
            } else {
                // Its directory is made: the tree's own.
                self.put(&entry.path(), &inner, kind.is_symlink())?;
            }
        }
        Ok(())
    }

    /// Copies the file at `source` to `at`, its directories made, its
    /// permissions kept, a symbolic link as a link; an error if something
    /// is placed there already.
    fn file(&mut self, source: &Path, at: &str) -> Result<(), String> {
        let target = self.root.join(at);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create `{}`: {error}", parent.display()))?;
        }
        if let Some((parent, _)) = at.rsplit_once('/') {
            self.made(parent);
        }
        let link = fs::symlink_metadata(source)
            .map_err(|error| format!("could not copy `{}`: {error}", source.display()))?
            .file_type()
            .is_symlink();
        self.put(source, at, link)
    }

    /// Notes the directory `at` made, and every directory it is in.
    fn made(&mut self, at: &str) {
        let mut dir = at;
        while self.dirs.insert(dir.to_owned()) {
            let Some((parent, _)) = dir.rsplit_once('/') else {
                return;
            };
            dir = parent;
        }
    }

    /// Copies the file at `source`, a symbolic link if `link`, to `at`,
    /// whose directory is made, as [`Composer::file`] does.
    fn put(&mut self, source: &Path, at: &str, link: bool) -> Result<(), String> {
        // No file can be where a directory was made, a tree's or one a file
        // was placed in.
        if self.dirs.contains(at) || !self.placed.insert(at.to_owned()) {
            return Err(format!("two entries place `{at}`"));
        }
        let target = self.root.join(at);
        let copied = if link {
            fs::read_link(source).and_then(|link| std::os::unix::fs::symlink(link, &target))
        } else {
            self.effects.place_file(source, &target)
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

/// The SHA-256 digest a pin names, in lower case, or `None` if it is no
/// digest, 64 hex digits; `Some("")` for no pin.
fn pin_of(sha256: &str) -> Option<String> {
    let pin = sha256.trim().to_ascii_lowercase();
    let digest = pin.len() == 64 && pin.bytes().all(|byte| byte.is_ascii_hexdigit());
    (pin.is_empty() || digest).then_some(pin)
}

/// The name a file fetched from `url` is given: the last segment of its
/// path, its query and fragment left off, or `download` for none.
#[must_use]
pub fn fetched_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let last = path.rsplit('/').next().unwrap_or("");
    let host_only = !path.contains('/');
    if last.is_empty() || host_only || last == "." || last == ".." {
        "download".to_owned()
    } else {
        last.to_owned()
    }
}

/// Runs `command` through `effects`: its standard output, or why it could
/// not run or did not succeed.
fn run_through(effects: &dyn Effects, command: Command) -> Result<String, String> {
    let output = effects
        .command(command)?
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Applies `fetch`, for the call `node`, to `url` and `sha256` (see
/// [Fetching](#fetching)): the downloaded file, which has that digest.
pub(super) fn fetch(url: &str, sha256: &str, effects: &dyn Effects, node: &IrNode) -> Flow<Value> {
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let Some(pin) = pin_of(sha256) else {
        return fail(format!(
            "`fetch` of `{url}` is pinned by `{sha256}`, which is no SHA-256 digest: 64 \
             hexadecimal digits"
        ));
    };
    let Some(made) = effects.temp_dir() else {
        return mistyped(node, "`fetch` called in a run without an owner");
    };
    let path = match made {
        Ok(dir) => format!("{dir}/{}", fetched_name(url)),
        Err(reason) => {
            return fail(format!(
                "`fetch` could not create a temporary directory: {reason}"
            ));
        }
    };
    let host = match effects.host_path(&path) {
        Ok(host) => host,
        Err(reason) => return fail(format!("`fetch` cannot use `{path}`: {reason}")),
    };
    let mut curl = Command::new("curl");
    curl.args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--retry", "2", "--output"])
        .arg(&host)
        .arg(url);
    if let Err(reason) = run_through(effects, curl) {
        return fail(format!("`fetch` could not download `{url}`: {reason}"));
    }
    let mut sum = Command::new("sha256sum");
    sum.arg(&host);
    let found = match run_through(effects, sum) {
        Ok(listed) => listed
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase(),
        Err(reason) => return fail(format!("`fetch` could not hash `{url}`: {reason}")),
    };
    if pin.is_empty() {
        return Err(Diagnostic::error(
            format!("`fetch` of `{url}` is not pinned: its sha256 is `{found}`"),
            node.span,
        )
        .with_note(format!("pin it: `fetch(\"{url}\", sha256 = \"{found}\")`"))
        .into());
    }
    if found != pin {
        return Err(Diagnostic::error(
            format!("`fetch` of `{url}` gave a file whose sha256 is `{found}`, not `{pin}`"),
            node.span,
        )
        .with_note("the file behind the URL is not the one pinned, so nothing is built from it")
        .into());
    }
    Ok(entry(false, Rc::from(path)))
}

/// Applies `fetch_git`, for the call `node`, to `url` and `commit` (see
/// [Fetching](#fetching)): a new directory of the repository's files at
/// that commit.
pub(super) fn fetch_git(
    url: &str,
    commit: &str,
    effects: &dyn Effects,
    node: &IrNode,
) -> Flow<Value> {
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let pin = commit.trim().to_ascii_lowercase();
    if pin.len() != 40 || !pin.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return fail(format!(
            "`fetch_git` of `{url}` is pinned by `{commit}`, which is no commit's name: 40 \
             hexadecimal digits, not a branch or a tag, which can move"
        ));
    }
    let Some(made) = effects.temp_dir() else {
        return mistyped(node, "`fetch_git` called in a run without an owner");
    };
    let path = match made {
        Ok(dir) => format!("{dir}/{}", fetched_name(url).trim_end_matches(".git")),
        Err(reason) => {
            return fail(format!(
                "`fetch_git` could not create a temporary directory: {reason}"
            ));
        }
    };
    let host = match effects.host_path(&path) {
        Ok(host) => host,
        Err(reason) => return fail(format!("`fetch_git` cannot use `{path}`: {reason}")),
    };
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        command.arg("-C").arg(&host).args(args);
        run_through(effects, command)
    };
    let fetched = fs::create_dir_all(&host)
        .map_err(|error| error.to_string())
        .and_then(|()| git(&["init", "--quiet"]))
        .and_then(|_| {
            git(&["fetch", "--quiet", "--depth", "1", url, &pin])
                .or_else(|_| git(&["fetch", "--quiet", url]))
        })
        .and_then(|_| {
            git(&[
                "-c",
                "advice.detachedHead=false",
                "checkout",
                "--quiet",
                &pin,
            ])
        })
        .and_then(|_| git(&["rev-parse", "HEAD"]));
    let head = match fetched {
        Ok(head) => head.trim().to_owned(),
        Err(reason) => {
            return fail(format!(
                "`fetch_git` could not fetch `{pin}` of `{url}`: {reason}"
            ));
        }
    };
    if head != pin {
        return fail(format!(
            "`fetch_git` of `{url}` checked out `{head}`, not `{pin}`"
        ));
    }
    if let Err(error) = fs::remove_dir_all(host.join(".git")) {
        return fail(format!(
            "`fetch_git` could not remove the repository of `{url}`: {error}"
        ));
    }
    Ok(entry(true, Rc::from(path)))
}

/// Applies `archive_unpack`, for the call `node`, to `archive` and `strip`
/// (see [Unpacking](#unpacking)): a new directory of what it holds.
pub(super) fn unpack(
    archive: &File,
    strip: i64,
    effects: &dyn Effects,
    node: &IrNode,
) -> Flow<Value> {
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let path = &*archive.path;
    if strip < 0 {
        return fail(format!(
            "`archive.unpack` of `{path}` cannot leave off {strip} components: `strip` is 0 or \
             more"
        ));
    }
    let source = match effects.host_path(path) {
        Ok(source) => source,
        Err(reason) => return fail(format!("`archive.unpack` cannot use `{path}`: {reason}")),
    };
    let Some(made) = effects.temp_dir() else {
        return mistyped(node, "`archive.unpack` called in a run without an owner");
    };
    let root = match made {
        Ok(root) => root,
        Err(reason) => {
            return fail(format!(
                "`archive.unpack` could not create a temporary directory: {reason}"
            ));
        }
    };
    let target = match effects.host_path(&root) {
        Ok(target) => target,
        Err(reason) => return fail(format!("`archive.unpack` cannot use `{root}`: {reason}")),
    };
    let unpacked = if is_zip(&source) {
        unzip(&source, &target, strip, effects)
    } else {
        let mut tar = Command::new("tar");
        tar.arg("--extract")
            .arg("--file")
            .arg(&source)
            .arg("--directory")
            .arg(&target)
            .arg(format!("--strip-components={strip}"))
            .args(["--no-same-owner", "--touch"]);
        run_through(effects, tar).map(|_| ())
    };
    if let Err(reason) = unpacked {
        return fail(format!(
            "`archive.unpack` could not unpack `{path}`: {reason}"
        ));
    }
    Ok(entry(true, Rc::from(root)))
}

/// Whether the file at `path` begins as a zip archive does: a local file's
/// header, or the end of an empty archive's directory.
fn is_zip(path: &Path) -> bool {
    use std::io::Read as _;
    let mut start = [0u8; 4];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut start))
        .is_ok_and(|()| matches!(&start, b"PK\x03\x04" | b"PK\x05\x06"))
}

/// Unpacks the zip archive at `source` into `target` with `unzip`, through
/// `effects`, into a directory of its own inside it first, and then moves
/// each entry `strip` components down into `target` with those components
/// left off, as `tar --strip-components` does: an entry with no more is
/// left out.
fn unzip(source: &Path, target: &Path, strip: i64, effects: &dyn Effects) -> Result<(), String> {
    let whole = target.join(".unzipped");
    let mut unzip = Command::new("unzip");
    unzip.args(["-q", "-o"]).arg(source).arg("-d").arg(&whole);
    run_through(effects, unzip)?;
    let strip = usize::try_from(strip).unwrap_or(usize::MAX);
    let moved = move_stripped(&whole, target, strip).map_err(|error| error.to_string());
    let removed = fs::remove_dir_all(&whole).map_err(|error| error.to_string());
    moved.and(removed)
}

/// Moves what is `strip` directories down in `from` into `to`, merging a
/// directory into one already there.
fn move_stripped(from: &Path, to: &Path, strip: usize) -> std::io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let is_dir = entry.file_type()?.is_dir();
        if strip > 0 {
            if is_dir {
                move_stripped(&path, to, strip - 1)?;
            }
            continue;
        }
        let at = to.join(entry.file_name());
        if is_dir && fs::symlink_metadata(&at).is_ok_and(|meta| meta.is_dir()) {
            move_stripped(&path, &at, 0)?;
        } else {
            fs::rename(&path, &at)?;
        }
    }
    Ok(())
}

/// Applies `which`, for the call `node`, to `name`: the program of that name
/// on this process's `PATH`, the first directory's that has a file of the
/// name its user may run.
pub(super) fn which(name: &str, node: &IrNode) -> Flow<Value> {
    use std::os::unix::fs::PermissionsExt;
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    if name.is_empty() || name.contains('/') {
        return fail(format!(
            "`which` finds a program by its name, which `{name}` is not"
        ));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| {
            fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        });
    match found.and_then(|path| path.to_str().map(Rc::from)) {
        Some(path) => Ok(entry(false, path)),
        None => fail(format!("`{name}` is not on `PATH`")),
    }
}
