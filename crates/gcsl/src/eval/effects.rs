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

    /// The working directory, as GCSL code names paths, that a relative
    /// path is against.
    ///
    /// # Errors
    ///
    /// Why there is none.
    fn working_dir(&self) -> Result<PathBuf, String> {
        std::env::current_dir().map_err(|error| error.to_string())
    }

    /// A new, empty directory for the run, as GCSL code names it, removed
    /// with what is in it when the run's owner is done; `None` for a run
    /// that has no owner to remove it, which no checked program calls for.
    fn temp_dir(&self) -> Option<Result<String, String>>;
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
