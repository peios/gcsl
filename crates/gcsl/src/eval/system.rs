//! The built-in functions that touch the world: running programs, with their
//! deadlines, the file system, and a run's temporary directories.

use std::cell::{Cell, OnceCell};
use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::thread::{self, ScopedJoinHandle};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use crate::check::Builtin;
use crate::diagnostic::Diagnostic;
use crate::ir::IrNode;
use crate::value::{Layout, List, Record, Secret, Value};

use super::secrets::token;
use super::effects::Effects;
use super::{Flow, mistyped};

/// Runs the program `argv` names, for the call `node` of `builtin`, `exec`
/// or `exec_live`, as `options` says ([`ExecOptions`]), and returns what it
/// did as a record of the layout `exec_result` (see [The built-in
/// functions](crate::eval::calls#the-built-in-functions)): `argv[0]` found
/// through `PATH` as the OS finds it and given the arguments `argv[1..]`,
/// with no shell, its standard input the null device, its standard output
/// and standard error captured whole, its working directory the options'
/// `cwd`, or this process's for `""`, and its environment this process's
/// with the options' `env` set over it; once it has exited, its exit status,
/// or 128 plus the number of the signal that killed it, and its output
/// decoded as UTF-8, each invalid sequence replaced by U+FFFD. For
/// `exec_live` the program's standard output and standard error are also
/// copied, as they arrive, to this process's standard error ([`run_live`]).
/// With `until`, the program is killed once a line of either stream
/// contains the text ([`run_watched`]), its output so far the result's,
/// and its status 0. An `interactive` one has this process's standard
/// input, output and error, and its result's output is empty. With
/// `panic`, a status other than 0 is a failure of the call, "`cc` exited
/// with status 1", noted with its standard error unless `exec_live`
/// showed it already.
///
/// Fails with "`exec` needs a program to run" for an empty `argv`
/// (`exec_live` naming itself), with "cannot run `name`", noting the OS's
/// own reason, for a program that cannot be started, or whose output cannot
/// be read or whose end cannot be waited for, as [`Command::output`]
/// reports each, and with "`until` watches a program's output, which an
/// interactive one does not capture" for both. No word of `argv`, nor
/// variable of `env`, is a secret. `exec_secret`'s `secrets` are set over
/// `env`, each a revealed secret's value, and what it reports is a secret,
/// of an identity derived from `argv` and the options, every secret in them
/// its identity, so that two runs are one secret only if they are given the
/// same (see [Secrets](crate::eval::secrets#secrets)); its failure on a
/// status is "a program given a secret failed", naming nothing it did,
/// unmarked, so that `recover` and `#[retry]` handle it. A run without the
/// standard library has no `exec` layout, nor any `exec` to call, so
/// `exec_result` being `None` is an internal error, as is an element of
/// `argv` that is not a string, or options of another shape, which the
/// checker rules out.
#[inline(never)]
pub(super) fn exec(
    builtin: Builtin,
    (argv, options): (&List, &Value),
    (exec_result, deadline, effects): (Option<&Rc<Layout>>, Option<Instant>, &dyn Effects),
    node: &IrNode,
) -> Flow<Option<Value>> {
    let Some(layout) = exec_result else {
        return mistyped(node, "`exec` called in a run without its result's layout");
    };
    let raw_options = options;
    let Some(options) = ExecOptions::of(options) else {
        return mistyped(node, "options of `exec` of another shape");
    };
    let Some((program, arguments)) = argv.values().split_first() else {
        return Err(Diagnostic::error(
            format!("`{}` needs a program to run", builtin.name()),
            node.span,
        )
        .into());
    };
    let Some(shown) = word(program) else {
        return mistyped(node, "a program of `exec` that is not a string");
    };
    let mut command = Command::new(shown);
    for argument in arguments {
        let Some(argument) = word(argument) else {
            return mistyped(node, "an argument of `exec` that is not a string");
        };
        command.arg(argument);
    }
    if !options.cwd.is_empty() {
        command.current_dir(options.cwd);
    }
    for &(name, value) in &options.env {
        command.env(name, value);
    }
    // Its identity is what it was given, every secret as its identity, so
    // that two runs are one secret only if they are one run.
    let identity = (builtin == Builtin::ExecSecret).then(|| {
        let given = (token(&Value::List(argv.clone())), token(raw_options));
        format!("exec-secret({}, {})", given.0, given.1)
    });
    let ran = Program {
        builtin,
        shown,
        identity,
    };
    let mut command = ran.through(effects, command, node)?;
    ran.finish(&mut command, &options, (layout, deadline), node)
}

/// A program [`exec`] or a sandbox's runs, as its result reports it.
pub(super) struct Program<'a> {
    /// What runs it: `exec`, `exec_live`, `exec_secret` or `sandbox_exec`.
    pub(super) builtin: Builtin,
    /// The program, as its errors name it.
    pub(super) shown: &'a str,
    /// The identity of what it reports, a secret, for `exec_secret`.
    pub(super) identity: Option<String>,
}

impl Program<'_> {
    /// The command that runs `command` through `effects`, the run's effect
    /// context (see [Effects](crate::eval::effects#effects)); "`exec` could
    /// not run `cc`: REASON", at the call, if it will not run it.
    pub(super) fn through(
        &self,
        effects: &dyn Effects,
        command: Command,
        node: &IrNode,
    ) -> Flow<Command> {
        effects.command(command).map_err(|reason| {
            let message = format!(
                "`{}` could not run `{}`: {reason}",
                self.builtin.name(),
                self.shown
            );
            Diagnostic::error(message, node.span).into()
        })
    }

    /// Runs `command`, the program, as `options` say, until `deadline` if
    /// there is one, and gives what it did, a record of `layout`, as
    /// [`exec`] says: `None` if the deadline passed first.
    pub(super) fn finish(
        self,
        command: &mut Command,
        options: &ExecOptions<'_>,
        (layout, deadline): (&Rc<Layout>, Option<Instant>),
        node: &IrNode,
    ) -> Flow<Option<Value>> {
        let Program {
            builtin,
            shown,
            identity,
        } = self;
        let live = builtin == Builtin::ExecLive;
        let Some(ran) = run(command, (live, deadline), options, node)? else {
            return Ok(None);
        };
        let (status, stdout, stderr) = match ran {
            Ok(ran) => ran,
            Err(error) => {
                return Err(
                    Diagnostic::error(format!("cannot run `{shown}`"), node.span)
                        .with_note(error.to_string())
                        .into(),
                );
            }
        };
        let text = |bytes: &[u8]| Rc::<str>::from(String::from_utf8_lossy(bytes).as_ref());
        let (stdout, stderr) = (text(&stdout), text(&stderr));
        let secret = identity.is_some();
        if options.panic && status != 0 {
            // A program given a secret may tell of it in its status and its
            // output, which are secrets: its failure names neither.
            if secret {
                return Err(Diagnostic::error("a program given a secret failed", node.span).into());
            }
            let mut failed =
                Diagnostic::error(format!("`{shown}` exited with status {status}"), node.span);
            if !live && !stderr.trim().is_empty() {
                failed = failed.with_note(format!("its standard error: {}", stderr.trim()));
            }
            return Err(failed.into());
        }
        let result = Value::Record(Rc::new(Record {
            layout: Rc::clone(layout),
            values: Box::new([Value::Int(status), Value::Str(stdout), Value::Str(stderr)]),
        }));
        let Some(identity) = identity else {
            return Ok(Some(result));
        };
        Ok(Some(Value::Secret(Rc::new(Secret {
            value: result,
            identity: Rc::from(identity),
            revealed: false,
        }))))
    }
}

/// What the options record of a call of `exec`, `exec_live` or
/// `exec_secret` says (see [`exec`]): each of its fields, as the checker
/// types them.
pub(super) struct ExecOptions<'a> {
    /// The program's working directory, `""` for this process's.
    pub(super) cwd: &'a str,
    /// The variables set over this process's environment for the program,
    /// and after them, `exec_secret`'s, each of a revealed secret's value.
    pub(super) env: Vec<(&'a str, &'a str)>,
    /// Whether the program has this process's terminal.
    pub(super) interactive: bool,
    /// Whether a status other than 0 fails the call.
    panic: bool,
    /// The text whose line ends the program, `""` for none.
    until: &'a str,
}

impl<'a> ExecOptions<'a> {
    /// The defaults of a sandbox's commands, `cwd` and `env`, that the
    /// record `value` holds; `None` for anything else.
    pub(super) fn of_defaults(value: &'a Value) -> Option<Self> {
        let Value::Record(record) = value else {
            return None;
        };
        let field = |name: &str| {
            record
                .layout
                .texts
                .iter()
                .position(|text| &**text == name)
                .and_then(|at| record.values.get(at))
        };
        let Some(Value::Str(cwd)) = field("cwd") else {
            return None;
        };
        let env = variables(field("env")?, |value| match value {
            Value::Str(text) => Some(&**text),
            _ => None,
        })?;
        Some(ExecOptions {
            cwd,
            env,
            interactive: false,
            panic: false,
            until: "",
        })
    }

    /// The options `value` holds, a record of the fields of
    /// [`Builtin::EXEC_OPTIONS`], or of [`Builtin::EXEC_SECRET_OPTIONS`],
    /// whose `secrets` are set over `env` and which have no `interactive`;
    /// `None` for anything else.
    pub(super) fn of(value: &'a Value) -> Option<Self> {
        let Value::Record(record) = value else {
            return None;
        };
        let field = |name: &str| {
            record
                .layout
                .texts
                .iter()
                .position(|text| &**text == name)
                .and_then(|at| record.values.get(at))
        };
        let text = |name: &str| match field(name) {
            Some(Value::Str(text)) => Some(&**text),
            _ => None,
        };
        let flag = |name: &str| match field(name) {
            Some(&Value::Bool(flag)) => Some(flag),
            _ => None,
        };
        let mut env = variables(field("env")?, |value| match value {
            Value::Str(text) => Some(&**text),
            _ => None,
        })?;
        if let Some(secrets) = field("secrets") {
            env.extend(variables(secrets, |value| match value {
                Value::Secret(secret) if secret.revealed => match &secret.value {
                    Value::Str(text) => Some(&**text),
                    _ => None,
                },
                _ => None,
            })?);
        }
        let interactive = match field("interactive") {
            Some(_) => flag("interactive")?,
            None => false,
        };
        Some(ExecOptions {
            cwd: text("cwd")?,
            env,
            interactive,
            panic: flag("panic")?,
            until: text("until")?,
        })
    }
}

/// The variables of `record`, an environment, each name with the text
/// `text` takes of its value; `None` if one has none.
fn variables<'a>(
    record: &'a Value,
    text: impl Fn(&'a Value) -> Option<&'a str>,
) -> Option<Vec<(&'a str, &'a str)>> {
    match record {
        Value::Empty => Some(Vec::new()),
        Value::Record(record) => record
            .layout
            .texts
            .iter()
            .zip(&record.values)
            .map(|(name, value)| Some((&**name, text(value)?)))
            .collect(),
        _ => None,
    }
}

/// What a program [`run`] runs did: its status, its standard output and its
/// standard error; or the error of running it.
type Ran = io::Result<(i64, Vec<u8>, Vec<u8>)>;

/// Runs `command` for [`exec`] as `options` says, copying its output if
/// `live`, until `deadline` if there is one: what it did, or `None` if the
/// deadline passed first. `until` with `interactive` is a runtime error at
/// `node`. A program that may be killed, by its deadline or by `until`, and
/// does not have the terminal, is run in a process group of its own, so
/// that what it starts is killed with it ([`run_watched`]).
fn run(
    command: &mut Command,
    (live, deadline): (bool, Option<Instant>),
    options: &ExecOptions<'_>,
    node: &IrNode,
) -> Flow<Option<Ran>> {
    if options.interactive {
        if !options.until.is_empty() {
            return Err(Diagnostic::error(
                "`until` watches a program's output, which an interactive one does not capture",
                node.span,
            )
            .into());
        }
        return Ok(run_interactive(command, deadline));
    }
    command.stdin(Stdio::null());
    let until = Some(options.until).filter(|until| !until.is_empty());
    if deadline.is_some() || until.is_some() {
        return run_watched(command, (live, deadline, until), node);
    }
    let output = if live {
        run_live(command, node)?
    } else {
        command.output()
    };
    Ok(Some(output.and_then(|output| {
        Ok((status_of(output.status)?, output.stdout, output.stderr))
    })))
}

/// The status [`exec`] reports of a program that ended with `status`: its
/// exit status, or 128 plus the number of the signal that killed it.
fn status_of(status: ExitStatus) -> io::Result<i64> {
    match (status.code(), status.signal()) {
        (Some(code), _) => Ok(i64::from(code)),
        (None, Some(signal)) => Ok(128 + i64::from(signal)),
        // A program waited for has exited or been killed.
        (None, None) => Err(io::Error::other("ended neither by exiting nor by a signal")),
    }
}

/// Runs `command` with this process's standard input, output and error,
/// until `deadline` if there is one: its status and no output, or `None`
/// if the deadline passed first, when it is killed, alone, as it is in the
/// terminal's process group.
fn run_interactive(command: &mut Command, deadline: Option<Instant>) -> Option<Ran> {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Some(Err(error)),
    };
    let watch = Watch {
        deadline,
        met: None,
        open: &AtomicUsize::new(0),
        group: false,
    };
    Some(match watch.wait(&mut child) {
        Ok(Ended::Exited(status)) => {
            status_of(status).map(|status| (status, Vec::new(), Vec::new()))
        }
        Ok(Ended::TimedOut | Ended::Met) => return None,
        Err(error) => Err(error),
    })
}

/// Runs `command`, in a process group of its own, copying its output to
/// this process's standard error as it runs if `live`, and watching each
/// line of it, of either stream, for the text `until` if there is one:
/// its output read by a thread each, and the program waited for by looking
/// at it in turn, at first often and then less, at most every
/// [`DEADLINE_POLL`] ([`Watch::wait`]). Once a line contains `until`, the
/// group is killed, and its output so far given, with the status 0, even
/// if it had ended by itself; once `deadline` passes, before the program
/// and its output have ended, the group is killed and `None` given (see
/// [Retry and timeout](crate::eval::retry#retry-and-timeout)); otherwise
/// what it did, once it has ended and its output has.
fn run_watched(
    command: &mut Command,
    (live, deadline, until): (bool, Option<Instant>, Option<&str>),
    node: &IrNode,
) -> Flow<Option<Ran>> {
    let mut child = match command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Ok(Some(Err(error))),
    };
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return mistyped(node, "a program of `exec` without the pipes of its output");
    };
    let (met, open) = (AtomicBool::new(false), AtomicUsize::new(2));
    let (ended, stdout, stderr) = thread::scope(|scope| {
        let reader = Reader {
            until,
            met: &met,
            open: &open,
            live,
        };
        let stdout = thread::Builder::new().spawn_scoped(scope, move || reader.read(out));
        let stderr = thread::Builder::new().spawn_scoped(scope, move || reader.read(err));
        let watch = Watch {
            deadline,
            met: Some(&met),
            open: &open,
            group: true,
        };
        let ended = watch.wait(&mut child);
        (
            ended,
            stdout.map(ScopedJoinHandle::join),
            stderr.map(ScopedJoinHandle::join),
        )
    });
    let (Some(stdout), Some(stderr)) = (copied(stdout), copied(stderr)) else {
        return mistyped(node, "a thread reading a program's output panicked");
    };
    // The text was seen, whether or not the program ended first.
    let ended = match ended {
        Ok(Ended::Exited(_)) if met.load(AtomicOrdering::Acquire) => Ok(Ended::Met),
        ended => ended,
    };
    Ok(match (stdout, stderr, ended) {
        (_, _, Ok(Ended::TimedOut)) => None,
        (Ok(stdout), Ok(stderr), Ok(Ended::Met)) => Some(Ok((0, stdout, stderr))),
        (Ok(stdout), Ok(stderr), Ok(Ended::Exited(status))) => {
            Some(status_of(status).map(|status| (status, stdout, stderr)))
        }
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Some(Err(error)),
    })
}

/// How a program [`Watch::wait`] waits for ended.
enum Ended {
    /// It exited, or a signal killed it, by itself, and its output ended.
    Exited(ExitStatus),
    /// A line of its output had the text watched for, and it was killed.
    Met,
    /// The deadline passed, and it was killed.
    TimedOut,
}

/// What [`Watch::wait`] waits for a program by.
struct Watch<'a> {
    /// When it is killed, if it has not ended.
    deadline: Option<Instant>,
    /// Set once a line of its output has the text watched for, if any is.
    met: Option<&'a AtomicBool>,
    /// How many of its output's streams are still being read.
    open: &'a AtomicUsize,
    /// Whether it leads a process group of its own, killed whole.
    group: bool,
}

impl Watch<'_> {
    /// Waits for `child`, looking at it in turn, at first often and then
    /// less, at most every [`DEADLINE_POLL`], until it has ended and its
    /// output has, its text is met, or its deadline passes, killing it, and
    /// its group if it leads one, for either of the last two: so that what
    /// it started, holding its output open, cannot keep it running.
    fn wait(&self, child: &mut Child) -> io::Result<Ended> {
        let mut pause = Duration::from_millis(1);
        let mut status = None;
        loop {
            if self
                .met
                .is_some_and(|met| met.load(AtomicOrdering::Acquire))
            {
                self.kill(child, status.is_some())?;
                return Ok(Ended::Met);
            }
            if status.is_none() {
                status = child.try_wait()?;
            }
            if let Some(status) = status
                && self.open.load(AtomicOrdering::Acquire) == 0
            {
                return Ok(Ended::Exited(status));
            }
            let now = Instant::now();
            if self.deadline.is_some_and(|deadline| now >= deadline) {
                self.kill(child, status.is_some())?;
                return Ok(Ended::TimedOut);
            }
            let wait = self
                .deadline
                .map_or(pause, |deadline| pause.min(deadline - now));
            thread::sleep(wait);
            pause = (pause * 2).min(DEADLINE_POLL);
        }
    }

    /// Kills `child`, its process group whole if it leads one, and waits
    /// for it unless it has been already, `reaped`, so that its output's
    /// pipes close. One that has already ended, or a group already empty,
    /// cannot be killed, which changes nothing.
    fn kill(&self, child: &mut Child, reaped: bool) -> io::Result<()> {
        if self.group {
            // A program waited for, or not yet, keeps its group's number
            // while anything is in it.
            if let Ok(leader) = i32::try_from(child.id()) {
                let _ = killpg(Pid::from_raw(leader), Signal::SIGKILL);
            }
        } else {
            let _ = child.kill();
        }
        if !reaped {
            child.wait()?;
        }
        Ok(())
    }
}

/// The longest wait between two looks at a program [`Watch::wait`] waits
/// for.
pub(super) const DEADLINE_POLL: Duration = Duration::from_millis(50);

/// How a thread of [`run_watched`] reads a stream of a program's output.
#[derive(Clone, Copy)]
struct Reader<'a> {
    /// The text watched for in each line, if any is.
    until: Option<&'a str>,
    /// Set once a line has it.
    met: &'a AtomicBool,
    /// How many streams are still being read, made one fewer as this one
    /// ends.
    open: &'a AtomicUsize,
    /// Whether each line is copied to this process's standard error.
    live: bool,
}

impl Reader<'_> {
    /// Reads `stream` to its end, copying each line as [`copy_lines`] does
    /// if `live`, and setting `met` once a line contains `until`; `open` is
    /// made one fewer as it ends, read whole or failing.
    fn read(self, stream: impl Read) -> io::Result<Vec<u8>> {
        let read = self.lines(stream);
        self.open.fetch_sub(1, AtomicOrdering::Release);
        read
    }

    /// What [`Reader::read`] reads, before it marks its stream ended.
    fn lines(self, stream: impl Read) -> io::Result<Vec<u8>> {
        let mut stream = BufReader::new(stream);
        let mut read = Vec::new();
        loop {
            let start = read.len();
            if stream.read_until(b'\n', &mut read)? == 0 {
                return Ok(read);
            }
            let Some(line) = read.get(start..) else {
                continue;
            };
            if self.live {
                // A copy that cannot be shown changes nothing else.
                let _ = io::stderr().lock().write_all(line);
            }
            if self
                .until
                .is_some_and(|until| String::from_utf8_lossy(line).contains(until))
            {
                self.met.store(true, AtomicOrdering::Release);
            }
        }
    }
}

/// The text of `value`, a word of `exec`'s `argv`, always a string, which
/// is never a secret's; `None` for anything else.
fn word(value: &Value) -> Option<&str> {
    match value {
        Value::Str(text) => Some(text),
        _ => None,
    }
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
/// `fs_temp_dir`, for the call `node`, to the path `p` and, for `fs_copy`, the
/// path `to` it copies to, or for `fs_set_executable` the flag (see [The built-in
/// functions](crate::eval::calls#the-built-in-functions)): `fs_exists` whether
/// something is at `p`, symbolic links followed ([`Path::try_exists`]);
/// `fs_read` the contents of the file at `p`, which must be UTF-8; `fs_mkdir`
/// makes the directory `p` and any missing parent ([`fs::create_dir_all`]),
/// `{}`; `fs_copy` copies the file `p` to `to` ([`fs::copy`]), `{}`;
/// `fs_set_executable` makes the file `p` executable or not, refusing one of
/// several links (see [`set_executable`]), `{}`. Every path
/// must be absolute, `p` first, and each is used as it is given, never
/// normalised and never against the working directory: a relative one is
/// "`fs.read` needs an absolute path, found `x.txt`", before anything is
/// touched. A failure of the operation is "`fs.read` could not read `/a`:
/// REASON", the OS's own text, as "No such file or directory (os error 2)",
/// "could not check", "could not create", "could not change" or "could not
/// copy `/a` to `/b`" for
/// the others, and a file that is not UTF-8 "`fs.read`: `/a` is not UTF-8
/// text". Every error is at the call, and names the built-in as the member of
/// `fs` it is, whatever name it was called by. A `to` that is not a string, or
/// a flag that is not a `Bool`, or
/// any other built-in, is an internal error, which the checker rules out.
pub(super) fn file_system(
    builtin: Builtin,
    (p, to): (&str, Option<&Value>),
    effects: &dyn Effects,
    node: &IrNode,
) -> Flow<Value> {
    let Some(name) = builtin.fs_name() else {
        return mistyped(node, "a built-in function that is not the file system's");
    };
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    let executable = match (builtin, to) {
        (Builtin::FsSetExecutable, Some(&Value::Bool(executable))) => Some(executable),
        (Builtin::FsSetExecutable, _) => {
            return mistyped(node, "a call of `fs.set_executable` with no flag");
        }
        _ => None,
    };
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
    // Where each path is on the host's file system (see
    // [Effects](crate::eval::effects#effects)).
    let host = |path: &str| {
        effects
            .host_path(path)
            .map_err(|reason| format!("`{name}` cannot use `{path}`: {reason}"))
    };
    let (p_shown, p) = match host(p) {
        Ok(found) => (p, found),
        Err(message) => return fail(message),
    };
    let to = match to.map(|to| host(to).map(|found| (to, found))).transpose() {
        Ok(to) => to,
        Err(message) => return fail(message),
    };
    match (builtin, to) {
        (Builtin::FsExists, None) => match p.try_exists() {
            Ok(exists) => Ok(Value::Bool(exists)),
            Err(error) => fail(format!("`{name}` could not check `{p_shown}`: {error}")),
        },
        (Builtin::FsRead, None) => match fs::read(&p).map(String::from_utf8) {
            Ok(Ok(text)) => Ok(Value::Str(Rc::from(text))),
            Ok(Err(_)) => fail(format!("`{name}`: `{p_shown}` is not UTF-8 text")),
            Err(error) => fail(format!("`{name}` could not read `{p_shown}`: {error}")),
        },
        (Builtin::FsMkdir, None) => match fs::create_dir_all(&p) {
            Ok(()) => Ok(Value::Empty),
            Err(error) => fail(format!("`{name}` could not create `{p_shown}`: {error}")),
        },
        (Builtin::FsSetExecutable, None) => match set_executable(&p, executable == Some(true)) {
            Ok(()) => Ok(Value::Empty),
            Err(reason) => fail(format!("`{name}` could not change `{p_shown}`: {reason}")),
        },
        (Builtin::FsCopy, Some((to_shown, to))) => match fs::copy(&p, to) {
            Ok(_) => Ok(Value::Empty),
            Err(error) => fail(format!(
                "`{name}` could not copy `{p_shown}` to `{to_shown}`: {error}"
            )),
        },
        _ => mistyped(
            node,
            "a built-in function of the file system with other arguments",
        ),
    }
}

/// Applies `fs_write`, `fs.write`, for the call `node`: writes `contents` to
/// the file at `p`, made or replaced whole, and is `{}` (see [The built-in
/// functions](crate::eval::calls#the-built-in-functions)). `p` must be absolute, as every
/// path of the file system's is, and is used as given: a relative one is
/// "`fs.write` needs an absolute path, found `x`", before anything is
/// touched. The write is ATOMIC ([`replace_file`]): a program reading `p`
/// sees the old contents or the new, whole, never a part. Any failure, the
/// directory of `p` missing or `p` a directory say, is "`fs.write` could
/// not write `/a/b`: REASON", the OS's own text, at the call, leaving no
/// temporary file behind.
/// Makes the file at `p` executable, by whoever can read it, if
/// `executable`, else by no one: refused for a file of several links, whose
/// mode is every link's, so that changing it through one name never changes
/// a file elsewhere, such as a host's stored copy.
fn set_executable(p: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = fs::metadata(p).map_err(|error| error.to_string())?;
    if !meta.is_file() {
        return Err("it is not a file".to_owned());
    }
    if meta.nlink() > 1 {
        return Err(format!(
            "it has {} links, which share its mode; copy it first",
            meta.nlink()
        ));
    }
    let mode = meta.permissions().mode();
    let mode = if executable {
        mode | ((mode & 0o444) >> 2)
    } else {
        mode & !0o111
    };
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).map_err(|error| error.to_string())
}

pub(super) fn write_file(
    p: &str,
    contents: &str,
    effects: &dyn Effects,
    node: &IrNode,
) -> Flow<Value> {
    let name = Builtin::FsWrite.fs_name().unwrap_or_default();
    let fail = |message: String| Err(Diagnostic::error(message, node.span).into());
    if !Path::new(p).is_absolute() {
        return fail(format!("`{name}` needs an absolute path, found `{p}`"));
    }
    let path = match effects.host_path(p) {
        Ok(path) => path,
        Err(reason) => return fail(format!("`{name}` cannot use `{p}`: {reason}")),
    };
    match replace_file(&path, contents.as_bytes()) {
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
/// is dropped (see [The built-in functions](crate::eval::calls#the-built-in-functions)).
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
