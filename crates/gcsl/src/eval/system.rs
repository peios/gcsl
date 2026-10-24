//! The built-in functions that touch the world: running programs, with their
//! deadlines, the file system, and a run's temporary directories.

use std::cell::{Cell, OnceCell};
use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::thread::{self, ScopedJoinHandle};
use std::time::{Duration, Instant};

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{Layout, List, Record, Value};

use super::{Flow, mistyped};

/// Runs the program `argv` names, for the call `node` of `builtin`, `exec`
/// or `exec_live`, and returns what it did as a record of the layout
/// `exec_result` (see [The built-in functions](crate::eval#the-built-in-functions)):
/// `argv[0]` found through `PATH` as the OS finds it and given the
/// arguments `argv[1..]`, with no shell, its standard input the null
/// device, its standard output and standard error captured whole, its
/// working directory and environment this process's; once it has exited,
/// its exit status, or 128 plus the number of the signal that killed it,
/// and its output decoded as UTF-8, each invalid sequence replaced by
/// U+FFFD. For `exec_live` the program's standard output and standard
/// error are also copied, as they arrive, to this process's standard error
/// ([`run_live`]); nothing else differs. Fails with "`exec` needs a
/// program to run" for an empty `argv` (`exec_live` naming itself), and
/// with "cannot run `name`", noting the OS's own reason, for a program that
/// cannot be started, or whose output cannot be read or whose end cannot be
/// waited for, as [`Command::output`] reports each. A run without the
/// standard library has no `exec` layout, nor any `exec` to call, so
/// `exec_result` being `None` is an internal error, as is an element of
/// `argv` that is not a string, which the checker rules out.
#[inline(never)]
pub(super) fn exec(
    builtin: Builtin,
    argv: &List,
    (exec_result, deadline): (Option<&Rc<Layout>>, Option<Instant>),
    node: &IrNode,
) -> Flow<Option<Value>> {
    let Some(layout) = exec_result else {
        return mistyped(node, "`exec` called in a run without its result's layout");
    };
    let Some((program, arguments)) = argv.values().split_first() else {
        return Err(Diagnostic::error(
            format!("`{}` needs a program to run", builtin.name()),
            node.span,
        )
        .into());
    };
    let Value::Str(program) = program else {
        return mistyped(node, "a program of `exec` that is not a string");
    };
    let mut command = Command::new(&**program);
    for argument in arguments {
        let Value::Str(argument) = argument else {
            return mistyped(node, "an argument of `exec` that is not a string");
        };
        command.arg(&**argument);
    }
    command.stdin(Stdio::null());
    let live = builtin == Builtin::ExecLive;
    let output = match (live, deadline) {
        (true, None) => run_live(&mut command, node)?,
        (false, None) => command.output(),
        (_, Some(deadline)) => match run_until(&mut command, (live, deadline), node)? {
            Ok(Some(output)) => Ok(output),
            Ok(None) => return Ok(None),
            Err(error) => Err(error),
        },
    };
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return Err(
                Diagnostic::error(format!("cannot run `{program}`"), node.span)
                    .with_note(error.to_string())
                    .into(),
            );
        }
    };
    let status = match (output.status.code(), output.status.signal()) {
        (Some(code), _) => i64::from(code),
        (None, Some(signal)) => 128 + i64::from(signal),
        // A program waited for has exited or been killed.
        (None, None) => {
            return mistyped(node, "a program ended neither by exiting nor by a signal");
        }
    };
    let text = |bytes: &[u8]| Value::Str(Rc::from(String::from_utf8_lossy(bytes).as_ref()));
    Ok(Some(Value::Record(Rc::new(Record {
        layout: Rc::clone(layout),
        values: Box::new([
            Value::Int(status),
            text(&output.stdout),
            text(&output.stderr),
        ]),
    }))))
}

/// Runs `command` as [`exec`] does, but only until `deadline`, copying its
/// output to this process's standard error as it runs if `live`: its
/// output read by a thread each, and the program waited for by looking at
/// it in turn, at first often and then less, at most every
/// [`DEADLINE_POLL`], until it ends or the deadline passes, when it is
/// killed and waited for. `Ok(None)` if it was killed so (see [Retry and
/// timeout](crate::eval#retry-and-timeout)).
pub(super) fn run_until(
    command: &mut Command,
    (live, deadline): (bool, Instant),
    node: &IrNode,
) -> Flow<io::Result<Option<Output>>> {
    let mut child = match command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Ok(Err(error)),
    };
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return mistyped(node, "a program of `exec` without the pipes of its output");
    };
    let read: fn(std::process::ChildStdout) -> io::Result<Vec<u8>> =
        if live { copy_lines } else { read_all };
    let read_err: fn(std::process::ChildStderr) -> io::Result<Vec<u8>> =
        if live { copy_lines } else { read_all };
    let (status, stdout, stderr) = thread::scope(|scope| {
        let stdout = thread::Builder::new().spawn_scoped(scope, move || read(out));
        let stderr = thread::Builder::new().spawn_scoped(scope, move || read_err(err));
        let status = wait_until(&mut child, deadline);
        (
            status,
            stdout.map(ScopedJoinHandle::join),
            stderr.map(ScopedJoinHandle::join),
        )
    });
    let (Some(stdout), Some(stderr)) = (copied(stdout), copied(stderr)) else {
        return mistyped(node, "a thread reading a program's output panicked");
    };
    Ok(match (stdout, stderr, status) {
        (Ok(stdout), Ok(stderr), Ok(Some(status))) => Ok(Some(Output {
            status,
            stdout,
            stderr,
        })),
        (_, _, Ok(None)) => Ok(None),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error),
    })
}

/// The longest wait between two looks at a program [`run_until`] runs.
pub(super) const DEADLINE_POLL: Duration = Duration::from_millis(50);

/// Waits for `child` until `deadline`: its exit status, or `None` if the
/// deadline passed first, the program then killed and waited for, so that
/// its output's pipes close.
pub(super) fn wait_until(
    child: &mut std::process::Child,
    deadline: Instant,
) -> io::Result<Option<std::process::ExitStatus>> {
    let mut pause = Duration::from_millis(1);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            // Already ended, it cannot be killed; either way it is waited
            // for.
            let _ = child.kill();
            child.wait()?;
            return Ok(None);
        }
        thread::sleep(pause.min(deadline - now));
        pause = (pause * 2).min(DEADLINE_POLL);
    }
}

/// All that `stream` gives, to its end.
pub(super) fn read_all(mut stream: impl Read) -> io::Result<Vec<u8>> {
    let mut read = Vec::new();
    stream.read_to_end(&mut read)?;
    Ok(read)
}

/// Runs `command`, for the call `node` of `exec_live`, as
/// [`Command::output`] does, its standard output and standard error
/// captured whole, but copying each, as it arrives, to this process's
/// standard error, never to its standard output ([`copy_lines`]): each
/// stream is read on a thread of its own, and both threads are joined
/// before the program is waited for, so that neither stream can fill its
/// pipe and stop the program while the other is read. Gives what `output`
/// would: the output, or the first error of starting the program, of
/// starting the thread of its standard output or of its standard error, of
/// reading either, or of waiting for the program, in that order, the
/// program waited for once it has started, whatever else failed. A stream
/// whose thread cannot be started is closed unread, so the program is not
/// left waiting to write to it. A pipe `command` did not make, or a thread
/// that panicked, which nothing it runs can do, is an internal error.
#[inline(never)]
pub(super) fn run_live(command: &mut Command, node: &IrNode) -> Flow<io::Result<Output>> {
    let mut child = match command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Ok(Err(error)),
    };
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return mistyped(
            node,
            "a program of `exec_live` without the pipes of its output",
        );
    };
    let (stdout, stderr) = thread::scope(|scope| {
        let stdout = thread::Builder::new().spawn_scoped(scope, move || copy_lines(out));
        let stderr = thread::Builder::new().spawn_scoped(scope, move || copy_lines(err));
        (
            stdout.map(ScopedJoinHandle::join),
            stderr.map(ScopedJoinHandle::join),
        )
    });
    let status = child.wait();
    let (Some(stdout), Some(stderr)) = (copied(stdout), copied(stderr)) else {
        return mistyped(node, "a thread copying a program's output panicked");
    };
    Ok(match (stdout, stderr, status) {
        (Ok(stdout), Ok(stderr), Ok(status)) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error),
    })
}

/// What a thread of [`run_live`] gave, `copier`: what it read of its
/// stream, or the error of reading it or of starting the thread; `None` if
/// it panicked.
pub(super) fn copied(
    copier: io::Result<thread::Result<io::Result<Vec<u8>>>>,
) -> Option<io::Result<Vec<u8>>> {
    match copier {
        Ok(Ok(read)) => Some(read),
        Ok(Err(_)) => None,
        Err(error) => Some(Err(error)),
    }
}

/// Reads `stream`, a program's standard output or standard error, to its
/// end, and gives every byte of it, for [`run_live`], copying each line, its
/// newline included, to this process's standard error once it is complete,
/// in one write under the lock of standard error, so that the lines of the
/// two streams interleave but a line is never written inside another; a
/// last line without a newline is copied as the stream ends, as it is, no
/// newline added. The bytes are copied as they are, never decoded. A failure
/// to write to standard error is ignored, the reading going on whole; the
/// first failure to read, an interruption retried, is returned.
pub(super) fn copy_lines(stream: impl Read) -> io::Result<Vec<u8>> {
    let mut stream = BufReader::new(stream);
    let mut read = Vec::new();
    loop {
        let start = read.len();
        if stream.read_until(b'\n', &mut read)? == 0 {
            return Ok(read);
        }
        if let Some(line) = read.get(start..) {
            // A copy that cannot be shown changes nothing else.
            let _ = io::stderr().lock().write_all(line);
        }
    }
}

/// Applies `builtin`, one of the file system's built-in functions but
/// `fs_temp_dir`, for the call `node`, to the path `p` and, for `fs_copy`,
/// the path `to` it copies to (see [The built-in functions](crate::eval#the-built-in-functions)):
/// `fs_exists` whether something is at `p`, symbolic links followed
/// ([`Path::try_exists`]); `fs_read` the contents of the file at `p`, which
/// must be UTF-8; `fs_mkdir` makes the directory `p` and any missing parent
/// ([`fs::create_dir_all`]), `{}`; `fs_copy` copies the file `p` to `to`
/// ([`fs::copy`]), `{}`. Every path must be absolute, `p` first, and each is
/// used as it is given, never normalised and never against the working
/// directory: a relative one is "`fs.read` needs an absolute path, found
/// `x.txt`", before anything is touched. A failure of the operation is
/// "`fs.read` could not read `/a`: REASON", the OS's own text, as "No such
/// file or directory (os error 2)", "could not check", "could not create"
/// or "could not copy `/a` to `/b`" for the others, and a file that is not
/// UTF-8 "`fs.read`: `/a` is not UTF-8 text". Every error is at the call,
/// and names the built-in as the member of `fs` it is, whatever name it was
/// called by. A `to` that is not a string, or any other built-in, is an
/// internal error, which the checker rules out.
pub(super) fn file_system(
    builtin: Builtin,
    p: &str,
    to: Option<&Value>,
    node: &IrNode,
) -> Flow<Value> {
    let Some(name) = builtin.fs_name() else {
        return mistyped(node, "a built-in function that is not the file system's");
    };
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let to = match (builtin, to) {
        (Builtin::FsCopy, Some(Value::Str(to))) => Some(&**to),
        (Builtin::FsCopy, _) => {
            return mistyped(node, "a call of `fs.copy` with no path to copy to");
        }
        _ => None,
    };
    for path in std::iter::once(p).chain(to) {
        if !Path::new(path).is_absolute() {
            return fail(format!("`{name}` needs an absolute path, found `{path}`"));
        }
    }
    match (builtin, to) {
        (Builtin::FsExists, None) => match Path::new(p).try_exists() {
            Ok(exists) => Ok(Value::Bool(exists)),
            Err(error) => fail(format!("`{name}` could not check `{p}`: {error}")),
        },
        (Builtin::FsRead, None) => match fs::read(p).map(String::from_utf8) {
            Ok(Ok(text)) => Ok(Value::Str(Rc::from(text))),
            Ok(Err(_)) => fail(format!("`{name}`: `{p}` is not UTF-8 text")),
            Err(error) => fail(format!("`{name}` could not read `{p}`: {error}")),
        },
        (Builtin::FsMkdir, None) => match fs::create_dir_all(p) {
            Ok(()) => Ok(Value::Empty),
            Err(error) => fail(format!("`{name}` could not create `{p}`: {error}")),
        },
        (Builtin::FsCopy, Some(to)) => match fs::copy(p, to) {
            Ok(_) => Ok(Value::Empty),
            Err(error) => fail(format!("`{name}` could not copy `{p}` to `{to}`: {error}")),
        },
        _ => mistyped(
            node,
            "a built-in function of the file system with other arguments",
        ),
    }
}

/// Applies `fs_write`, `fs.write`, for the call `node`: writes `contents` to
/// the file at `p`, made or replaced whole, and is `{}` (see [The built-in
/// functions](crate::eval#the-built-in-functions)). `p` must be absolute, as every
/// path of the file system's is, and is used as given: a relative one is
/// "`fs.write` needs an absolute path, found `x`", before anything is
/// touched. The write is ATOMIC ([`replace_file`]): a program reading `p`
/// sees the old contents or the new, whole, never a part. Any failure, the
/// directory of `p` missing or `p` a directory say, is "`fs.write` could
/// not write `/a/b`: REASON", the OS's own text, at the call, leaving no
/// temporary file behind.
pub(super) fn write_file(p: &str, contents: &str, node: &IrNode) -> Flow<Value> {
    let name = Builtin::FsWrite.fs_name().unwrap_or_default();
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let path = Path::new(p);
    if !path.is_absolute() {
        return fail(format!("`{name}` needs an absolute path, found `{p}`"));
    }
    match replace_file(path, contents.as_bytes()) {
        Ok(()) => Ok(Value::Empty),
        Err(reason) => fail(format!("`{name}` could not write `{p}`: {reason}")),
    }
}

/// The number of the next temporary file this process makes to write a
/// file through ([`replace_file`]).
pub(super) static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// How many names a temporary file of [`replace_file`] is tried under
/// before the write fails: each that is taken, by a file some other process
/// left behind, is passed over.
pub(super) const TEMP_FILE_ATTEMPTS: u32 = 16;

/// Writes `contents` to the file at `path`, an absolute path, atomically:
/// to a NEW temporary file in the same directory, `.NAME.gcsl-tmp-PID-N`
/// beside `NAME`, made exclusively, so that it can be no file already there
/// (a name taken is passed over for the next number, a few times), which is
/// then flushed to the disk ([`fs::File::sync_all`]) and renamed over
/// `path` ([`fs::rename`]), which replaces what is there in one step. The
/// file has the mode a new file gets, `0666` less what the process's umask
/// takes away, whatever mode a file it replaces had; a symbolic link at
/// `path` is replaced by the file, not followed. On any failure after it
/// was made, the temporary file is removed, its removal's own errors
/// ignored.
///
/// # Errors
///
/// The reason the file could not be written, as text: the OS's, as "No
/// such file or directory (os error 2)" for a directory that does not exist
/// and "Is a directory (os error 21)" for a `path` that is a directory, or
/// `path` naming no file at all, as `/` does.
pub(super) fn replace_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err("the path names no file".to_owned());
    };
    let mut taken = None;
    for _ in 0..TEMP_FILE_ATTEMPTS {
        let number = NEXT_TEMP_FILE.fetch_add(1, AtomicOrdering::Relaxed);
        let mut temp_name = OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(".gcsl-tmp-{}-{number}", std::process::id()));
        let temp = dir.join(temp_name);
        let file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                taken = Some(error);
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        let written = fill(file, contents).and_then(|()| fs::rename(&temp, path));
        return written.map_err(|error| {
            let _ = fs::remove_file(&temp);
            error.to_string()
        });
    }
    Err(taken.map_or_else(String::new, |error| error.to_string()))
}

/// Writes `contents` to `file`, a new file, and flushes it to the disk, for
/// [`replace_file`]; the file is closed as it is dropped.
///
/// # Errors
///
/// The OS's error, if writing or flushing failed.
pub(super) fn fill(mut file: fs::File, contents: &[u8]) -> io::Result<()> {
    file.write_all(contents)?;
    file.sync_all()
}

/// The temporary directories `fs.temp_dir` makes for a run, held by the
/// run's owner, a [`crate::workspace::Workspace`], which a lone file's run is
/// too, or a [`crate::Session`], and removed, with everything in them, as it
/// is dropped (see [The built-in functions](crate::eval#the-built-in-functions)).
///
/// They are the numbered children, `1`, `2` and so on, of one *parent*
/// directory per owner, made in the system's temporary directory
/// ([`std::env::temp_dir`]), or in the *temp root* a workspace's host chose
/// ([`crate::workspace::Workspace::set_temp_root`]), by the first call of
/// `fs.temp_dir` that the owner's runs make, never before: `gcsl-PID-N`, the
/// process's id and a number counted up for every parent the process makes,
/// so that no two owners of a process share one; a name some other process
/// left behind is passed over for the next number, a few times. Each
/// directory is made with mode `0700`, less what the process's umask takes
/// away. A temp root is made then too if it is missing, with its missing
/// parents ([`fs::create_dir_all`], the mode any new directory gets), and
/// is never removed. Dropping the owner removes the parent
/// ([`fs::remove_dir_all`]), errors ignored: so a process that ends without
/// dropping it, killed say, leaves it behind.
#[derive(Debug, Default)]
pub struct TempDirs {
    /// The directory the parent is made in, if the owner's host chose one:
    /// else the system's temporary directory.
    pub(super) root: Option<PathBuf>,
    /// The parent directory, once it is made.
    pub(super) parent: OnceCell<PathBuf>,
    /// The number of the last child made, 0 before the first.
    pub(super) made: Cell<u64>,
}

/// The number of the next parent directory of temporary directories this
/// process makes ([`TempDirs`]).
pub(super) static NEXT_PARENT: AtomicU64 = AtomicU64::new(0);

impl TempDirs {
    /// How many names a parent directory is tried under before making it
    /// fails: each that is taken, by a directory some other process left
    /// behind, is passed over.
    pub(super) const ATTEMPTS: u32 = 16;

    /// Temporary directories of a new owner, none made: nothing is made on
    /// disk until the first is asked for.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `root` the directory the parent is made in, the temp root,
    /// instead of the system's temporary directory. It is read only as the
    /// parent is made, by the first directory asked for: so once one has
    /// been, this changes nothing, the parent made staying in use. Nothing
    /// is checked or made now; a `root` that is not absolute is the error of
    /// every directory asked for.
    pub(crate) fn set_root(&mut self, root: PathBuf) {
        self.root = Some(root);
    }

    /// The parent directory, made now if it has not been, in the temp root,
    /// made first if it is missing, or the system's temporary directory.
    ///
    /// # Errors
    ///
    /// The reason it could not be made, as text: the OS's, or the temp root
    /// or the system's temporary directory not being an absolute path.
    pub(super) fn parent(&self) -> Result<&Path, String> {
        if let Some(parent) = self.parent.get() {
            return Ok(parent);
        }
        let system;
        let base = if let Some(root) = &self.root {
            if !root.is_absolute() {
                return Err(format!(
                    "the temporary directory root `{}` is not an absolute path",
                    root.display()
                ));
            }
            fs::create_dir_all(root).map_err(|error| error.to_string())?;
            root.as_path()
        } else {
            system = std::env::temp_dir();
            if !system.is_absolute() {
                return Err(format!(
                    "the system's temporary directory `{}` is not an absolute path",
                    system.display()
                ));
            }
            system.as_path()
        };
        let mut taken = None;
        for _ in 0..Self::ATTEMPTS {
            let number = NEXT_PARENT.fetch_add(1, AtomicOrdering::Relaxed);
            let candidate = base.join(format!("gcsl-{}-{number}", std::process::id()));
            match DirBuilder::new().mode(0o700).create(&candidate) {
                Ok(()) => return Ok(self.parent.get_or_init(|| candidate)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    taken = Some(error);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Err(taken.map_or_else(String::new, |error| error.to_string()))
    }

    /// Makes a new empty temporary directory, mode `0700`, the next numbered
    /// child of the parent, which is made first if it has not been, and
    /// returns its absolute path. A number something else has taken in the
    /// parent, a directory the program made there, is passed over.
    ///
    /// # Errors
    ///
    /// The reason the directory could not be made, as text: the OS's, or
    /// the temp root or the system's temporary directory not being an
    /// absolute path, or its path not being UTF-8.
    pub(super) fn make(&self) -> Result<String, String> {
        let parent = self.parent()?;
        loop {
            let number = self.made.get() + 1;
            self.made.set(number);
            let dir = parent.join(number.to_string());
            match DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => {
                    return dir
                        .into_os_string()
                        .into_string()
                        .map_err(|dir| format!("`{}` is not UTF-8", Path::new(&dir).display()));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

impl Drop for TempDirs {
    /// Removes the parent directory, with every temporary directory and
    /// everything in them, if it was made; errors are ignored.
    fn drop(&mut self) {
        if let Some(parent) = self.parent.get() {
            let _ = fs::remove_dir_all(parent);
        }
    }
}
