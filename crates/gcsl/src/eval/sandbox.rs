//! Sandboxes: a live copy of a directory that commands run in, as their `/`.
//!
//! # Sandboxes
//!
//! `d.sandbox(cwd, env)`, the built-in `dir_sandbox`
//! (`Evaluator::make_sandbox`), copies the tree of the directory `d` into
//! a new directory of the run, as `fs.compose_dir` copies one, its own, and
//! gives a sandbox of it, [`Value::Sandbox`]: the copy, and the working
//! directory, absolute in it, and environment its commands have by
//! default, made in the copy if it is not there. One directory backs any
//! number of sandboxes, each a copy of its own, and none changes it. Two
//! sandboxes are equal only if they are one.
//!
//! `s.exec(...words)`, the built-in `sandbox_exec` (`sandbox_exec`), runs
//! `words[0]`, with the arguments `words[1..]`, as `exec` runs a program,
//! but with the copy as its `/`: under bubblewrap, `bwrap`, in user and pid
//! namespaces of their own, the copy bound as `/`, a `/proc` and a `/dev`
//! of their own, a `/tmp` made in the copy if it has none, killed with
//! `bwrap` if it is killed, unless the host runs it itself
//! ([`Effects::sandbox_command`]). Its working directory is the sandbox's, or the
//! call's `cwd`, absolute in the copy or relative to the sandbox's; its
//! environment is exactly the sandbox's with the call's `env` set over it,
//! nothing of this process's, since the sandbox is the program's host, so
//! that `words[0]` is found through that environment's `PATH`, or the
//! default `/bin:/usr/bin`. What it writes is in the copy, for the next
//! command and for `s.file` and `s.dir` to read. The network is not shut
//! off: that is a build host's policy for the steps it isolates. Without
//! `bwrap` on `PATH`, a sandbox cannot make the copy its `/`, and a command
//! run against the host's own `/` would find what it should not, so it is
//! "a sandbox needs bubblewrap (`bwrap`), which is not installed".
//!
//! `s.file(path)` and `s.dir(path)` (`sandbox_entry`) are the file and the
//! directory at `path` in the copy, absolute in it or relative to the
//! sandbox's working directory, normalised lexically: a path that would
//! leave the copy is "`../x` is not inside the sandbox". Their paths are
//! the host's, of the copy, and what is there is what the commands left so
//! far: a later `exec` can change it. A host that runs steps in sandboxes
//! of its own, pekit, mounts the directory as an overlay rather than
//! copying it, and gives the paths as the commands see them.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::time::Instant;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{File, Layout, Sandbox, Value};

use super::effects::Effects;
use super::files::{Composer, relative};
use super::system::{ExecOptions, Program};
use super::{Evaluator, Flow, mistyped};

impl Evaluator<'_> {
    /// Applies `dir_sandbox`, for the call `node`, to the directory `dir`
    /// and the record `options` of its commands' default `cwd` and `env`
    /// (see [Sandboxes](#sandboxes)): a sandbox of a new copy of `dir`.
    pub(super) fn make_sandbox(&self, dir: &File, options: &Value, node: &IrNode) -> Flow<Value> {
        let effects = self.shared.effects;
        let Some(made) = effects.temp_dir() else {
            return mistyped(node, "a sandbox made in a run without an owner");
        };
        let Some(defaults) = ExecOptions::of_defaults(options) else {
            return mistyped(node, "options of a sandbox of another shape");
        };
        let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
        let Some(cwd) = relative(defaults.cwd) else {
            return fail(format!("`{}` is not inside the sandbox", defaults.cwd));
        };
        let made = made.and_then(|root| Ok((effects.host_path(&root)?, root)));
        let (host_root, root) = match made {
            Ok(made) => made,
            Err(reason) => {
                return fail(format!("a sandbox could not make its directory: {reason}"));
            }
        };
        match effects.make_sandbox(&dir.path, &root) {
            Some(Ok(())) => {}
            Some(Err(reason)) => {
                return fail(format!(
                    "a sandbox of `{}` could not be made: {reason}",
                    dir.path
                ));
            }
            None => {
                let mut copier = Composer {
                    root: host_root.clone(),
                    placed: HashSet::new(),
                    dirs: HashSet::new(),
                    effects,
                };
                if let Err(message) = copier.tree_at(&dir.path, "") {
                    return fail(format!(
                        "a sandbox could not copy `{}`: {message}",
                        dir.path
                    ));
                }
            }
        }
        // Its commands start in its working directory, which is its own to
        // make.
        if let Err(error) = std::fs::create_dir_all(host_root.join(&cwd)) {
            return fail(format!("a sandbox could not make `/{cwd}`: {error}"));
        }
        let env = defaults
            .env
            .iter()
            .map(|&(name, value)| (Rc::from(name), Rc::from(value)))
            .collect();
        Ok(Value::Sandbox(Rc::new(Sandbox {
            root: Rc::from(root),
            cwd: Rc::from(format!("/{cwd}")),
            env,
        })))
    }
}

/// Applies `sandbox_exec`, for the call `node`, to the sandbox `sandbox`
/// and the record `request` of the `argv` to run and `exec`'s options (see
/// [Sandboxes](#sandboxes)): the program run in it under `bwrap`, until
/// `deadline` if there is one, its result a record of `layout`; `None` if
/// the deadline passed first.
pub(super) fn sandbox_exec(
    sandbox: &Sandbox,
    request: &Value,
    (layout, deadline, effects): (Option<&Rc<Layout>>, Option<Instant>, &dyn Effects),
    node: &IrNode,
) -> Flow<Option<Value>> {
    let (Some(layout), Some(options), Some(words)) =
        (layout, ExecOptions::of(request), argv(request))
    else {
        return mistyped(node, "a sandbox's command of another shape");
    };
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let Some(&shown) = words.first() else {
        return fail("`s.exec` needs a program to run".to_owned());
    };
    let cwd = if options.cwd.is_empty() {
        Some(sandbox.cwd.trim_start_matches('/').to_owned())
    } else if options.cwd.starts_with('/') {
        relative(options.cwd)
    } else {
        relative(&format!("{}/{}", sandbox.cwd, options.cwd))
    };
    let Some(cwd) = cwd else {
        return fail(format!("`{}` is not inside the sandbox", options.cwd));
    };
    // A program expects a `/tmp`; the copy is the sandbox's own to add one,
    // where the host's file system has it.
    if let Ok(root) = effects.host_path(&sandbox.root) {
        let _ = std::fs::create_dir_all(root.join("tmp"));
    }
    // The sandbox's environment, each variable the call sets replaced.
    let given = &options.env;
    let kept = sandbox
        .env
        .iter()
        .map(|(name, value)| (&**name, &**value))
        .filter(|&(name, _)| !given.iter().any(|&(other, _)| other == name));
    let env: Vec<(&str, &str)> = kept.chain(given.iter().copied()).collect();
    let mut inner = Command::new(shown);
    inner
        .args(&words[1..])
        .current_dir(format!("/{cwd}"))
        .env_clear()
        .envs(env.iter().copied());
    let program = Program {
        builtin: Builtin::SandboxExec,
        shown,
        identity: None,
    };
    let mut command = match effects.sandbox_command(&sandbox.root, inner) {
        Some(Ok(command)) => command,
        Some(Err(reason)) => {
            return fail(format!("`sandbox_exec` could not run `{shown}`: {reason}"));
        }
        None => {
            let Some(bwrap) = on_path("bwrap") else {
                return fail(
                    "a sandbox needs bubblewrap (`bwrap`), which is not installed".to_owned(),
                );
            };
            let mut command = Command::new(bwrap);
            command.args([
                "--bind",
                &sandbox.root,
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
            ]);
            command.args(["--unshare-user", "--unshare-pid", "--die-with-parent"]);
            command.args(["--chdir", &format!("/{cwd}"), "--clearenv"]);
            for (name, value) in &env {
                command.args(["--setenv", name, value]);
            }
            command.arg("--").args(&words);
            program.through(effects, command, node)?
        }
    };
    program.finish(&mut command, &options, (layout, deadline), node)
}

/// The words of `request`'s `argv`, strings all; `None` for anything else.
fn argv(request: &Value) -> Option<Vec<&str>> {
    let Value::Record(record) = request else {
        return None;
    };
    let at = record
        .layout
        .texts
        .iter()
        .position(|text| &**text == "argv")?;
    let Value::List(words) = record.values.get(at)? else {
        return None;
    };
    words
        .values()
        .iter()
        .map(|word| match word {
            Value::Str(text) => Some(&**text),
            _ => None,
        })
        .collect()
}

/// The first executable `name` among the directories of this process's
/// `PATH`, if any is.
fn on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// Applies `sandbox_file`, or `sandbox_dir` if `builtin` is that, for the
/// call `node`, to `sandbox` and `path` (see [Sandboxes](#sandboxes)): the
/// file or directory at `path` in its copy.
pub(super) fn sandbox_entry(
    builtin: Builtin,
    sandbox: &Sandbox,
    path: &str,
    node: &IrNode,
) -> Flow<Value> {
    let inside = if path.starts_with('/') {
        relative(path)
    } else {
        relative(&format!("{}/{path}", sandbox.cwd))
    };
    let Some(inside) = inside else {
        let message = format!("`{path}` is not inside the sandbox");
        return Err(Diagnostic::error(message, node.span).into());
    };
    let full = if inside.is_empty() {
        Rc::clone(&sandbox.root)
    } else {
        Rc::from(format!("{}/{inside}", sandbox.root))
    };
    let file = Rc::new(File {
        path: full,
        identity: None,
    });
    Ok(if builtin == Builtin::SandboxDir {
        Value::Dir(file)
    } else {
        Value::File(file)
    })
}
