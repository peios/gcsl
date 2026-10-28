//! The effect context: how a run's effects reach the system.
//!
//! # Effects
//!
//! Every effect of a run goes through its [`Effects`], the host's: running
//! a program, the file system's operations, and temporary directories.
//! GCSL's values hold paths as GCSL code names them, and so do the
//! commands it runs, whose words and working directory are those paths;
//! what GCSL itself does to the file system, reading, writing, copying and
//! listing, it does at the path of the host's file system that a path
//! names, [`Effects::host_path`]. A host that runs code in a sandbox, as
//! pekit runs a step, gives each command its sandbox
//! ([`Effects::command`]) and roots the paths in it, and GCSL never needs
//! to know what a sandbox is. Plain GCSL's context, [`Direct`], is the
//! system itself: a command as it is, a path as it is, the process's
//! working directory, and the temporary directories of the run's owner.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::TempDirs;

/// How a run's effects reach the system (see [Effects](#effects)).
pub trait Effects: std::fmt::Debug {
    /// The command that runs `command`, a program a run asks for, its words,
    /// working directory and environment as GCSL code gives them: `command`
    /// itself, or one the host makes of it, to run it in a sandbox say. The
    /// error, the reason as text, if the host will not run it.
    ///
    /// # Errors
    ///
    /// Why the host will not run the program.
    fn command(&self, command: Command) -> Result<Command, String> {
        Ok(command)
    }

    /// The command that runs `command` in the sandbox whose copy is at
    /// `root`, as GCSL code names it, with the copy as its `/`: `command`'s
    /// program and arguments, its working directory absolute in the copy, and
    /// exactly the sandbox's environment, as `s.exec` gives them (see
    /// [Sandboxes](crate::eval::sandbox#sandboxes)). `None`, as by default,
    /// for GCSL's own, under bubblewrap; a host that runs programs in
    /// sandboxes of its own gives one, since a sandbox in one of its
    /// sandboxes cannot always be made.
    fn sandbox_command(&self, root: &str, command: Command) -> Option<Result<Command, String>> {
        let _ = (root, command);
        None
    }

    /// Places the file at `source` at `target`, both paths of the host's
    /// file system, for `fs.compose_dir` or a sandbox's copy: a copy of its
    /// own, its mode kept, as by default, or what the host makes of it more
    /// cheaply, such as a hard link to a file that nothing can change.
    ///
    /// # Errors
    ///
    /// Why it could not be placed.
    fn place_file(&self, source: &Path, target: &Path) -> std::io::Result<()> {
        std::fs::copy(source, target).map(|_| ())
    }

    /// Tells the host that the directory `root`, as GCSL code names it, is a
    /// finished `fs.compose_dir`, which no GCSL code changes: a host can seal
    /// it, so that no program changes it either.
    fn composed(&self, root: &str) {
        let _ = root;
    }

    /// Makes the sandbox of the directory `source` whose copy is to be at
    /// `root`, a new, empty temporary directory, both as GCSL code names
    /// them, as the host's own, without a copy: as an overlay, say, whose
    /// writes are in `root` and whose reads ([`Effects::host_path`]) see them
    /// over `source`. `None`, as by default, for GCSL to copy `source` into
    /// `root`, as it does for a directory any program might change.
    fn make_sandbox(&self, source: &str, root: &str) -> Option<Result<(), String>> {
        let _ = (source, root);
        None
    }

    /// That a step's run read the context whose declaration's key's text is
    /// `key`, from the step's own context, the first time it did in the run
    /// (see [Graph mode](crate::eval::graph#graph-mode), "Reads"): what a
    /// host that gives a step a context's files only once it reads them
    /// stages them by, before the step goes on. By default, nothing.
    fn context_read(&self, key: &str) {
        let _ = key;
    }

    /// The path of the host's file system that `path`, an absolute path as
    /// GCSL code names it, is: what GCSL reads, writes, copies or lists
    /// there.
    ///
    /// # Errors
    ///
    /// Why the host gives `path` no place, one outside a sandbox say.
    fn host_path(&self, path: &str) -> Result<PathBuf, String> {
        Ok(PathBuf::from(path))
    }

    /// Whether the directory at `host`, a path of the host's file system
    /// met inside a directory GCSL lists or copies, is the host's own and
    /// no part of the directory, as pekit's outputs are no part of its
    /// workspace: left out of `d.files()` and of a copy, as if it were not
    /// there. By default, none is.
    fn hides(&self, host: &Path) -> bool {
        let _ = host;
        false
    }

    /// The working directory, as GCSL code names paths: where a program
    /// given no `cwd` runs, which `working_dir()` names, and what a relative
    /// path is against in a run with no table of directories (see [Where a
    /// path is](crate::eval::files#where-a-path-is)).
    ///
    /// # Errors
    ///
    /// Why there is none.
    fn working_dir(&self) -> Result<PathBuf, String> {
        std::env::current_dir().map_err(|error| error.to_string())
    }

    /// What the host makes of the keychain's secret `secret` (see
    /// [Keychains](crate::eval::keychain#keychains)): its public identity,
    /// and for a file the path its programs find it at. `None`, as by
    /// default, for its identity to be `profile/name` and a file's path its
    /// own.
    ///
    /// # Errors
    ///
    /// Why the host will not give the secret.
    fn keychain(&self, secret: &KeychainSecret<'_>) -> Option<Result<KeychainAnswer, String>> {
        let _ = secret;
        None
    }

    /// A new, empty directory for the run, as GCSL code names it, removed
    /// with what is in it when the run's owner is done; `None` for a run
    /// that has no owner to remove it, which no checked program calls for.
    fn temp_dir(&self) -> Option<Result<String, String>>;

    /// Whether the host isolates the effect of `builtin`, in graph mode, so
    /// that it may happen out of program order: an effect it does not
    /// isolate is made, as a node, only once nothing earlier in program
    /// order is still running (see [Graph
    /// mode](crate::eval::graph#graph-mode)). By default none is, and every
    /// effect is made in program order.
    fn isolates(&self, builtin: crate::check::Builtin) -> bool {
        let _ = builtin;
        false
    }
}

/// A secret a keychain gives, as a host is asked of it (see
/// [Keychains](crate::eval::keychain#keychains)): its name, the profile it is
/// of, what it holds, and for a file its path as GCSL code names it. It
/// has no `Debug`, since it holds the secret's value.
#[derive(Clone, Copy)]
pub struct KeychainSecret<'a> {
    /// Its name, `module-signing`.
    pub name: &'a str,
    /// The profile it is of, `dev`.
    pub profile: &'a str,
    /// What it holds: a file's contents, or a string's value.
    pub contents: &'a [u8],
    /// For a file, or a string read from one, the file's path.
    pub file: Option<&'a str>,
}

/// What a host makes of a keychain's secret (see
/// [Keychains](crate::eval::keychain#keychains)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeychainAnswer {
    /// Its public identity, what receipts record of it.
    pub identity: String,
    /// For a file, the path its programs find it at; `None` for the file's
    /// own.
    pub path: Option<String>,
}

/// Plain GCSL's effect context: the system itself (see
/// [Effects](#effects)).
#[derive(Clone, Copy, Debug)]
pub struct Direct<'a> {
    /// The owner of the run's temporary directories, if it has one.
    pub temp_dirs: Option<&'a TempDirs>,
}

impl Effects for Direct<'_> {
    fn temp_dir(&self) -> Option<Result<String, String>> {
        self.temp_dirs.map(TempDirs::make)
    }
}
