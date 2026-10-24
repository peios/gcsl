//! Files: making a `File` of a path and reading one.
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

use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{File, Value};

use super::Flow;

/// Applies `file`, for the call `node`, to `path` (see [Files](#files)): the
/// file at `path`, made absolute against the working directory and
/// normalised lexically.
pub(super) fn make_file(path: &str, node: &IrNode) -> Flow<Value> {
    if path.is_empty() {
        return Err(Diagnostic::error("`file` needs a path", node.span).into());
    }
    let given = Path::new(path);
    let absolute = if given.is_absolute() {
        given.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(given),
            Err(error) => {
                return Err(
                    Diagnostic::error("cannot find the working directory", node.span)
                        .with_note(error.to_string())
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
    Ok(Value::File(Rc::new(File {
        path: Rc::from(text),
        identity: None,
    })))
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
pub(super) fn read_file(file: &File, node: &IrNode) -> Flow<Value> {
    let path = &*file.path;
    match std::fs::read(path).map(String::from_utf8) {
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
