//! Evaluating calls: binding arguments and defaults, calling closures, tags and
//! the built-in functions.
//!
//! # Calls
//!
//! A closure is built by `MakeClosure` from its captured values, read in
//! the creating frame into one boxed slice, and a reference to the IR,
//! which the closure keeps alive: two allocations. `LoadCapture` reads the
//! running closure's captures, and `LoadCallee` yields the running closure
//! itself, which the evaluator holds while it runs, so a recursive function
//! refers to itself without holding itself. `Top` reads a slot of the top
//! level's frame, at base 0 of the value stack, from whatever frame it is
//! evaluated in: a top-level function of the input, or the cell of one of
//! its top-level fields, which another function or a `lazy` refers to there
//! rather than by a capture. A cell is written once, at its point, which
//! the schedule puts before every read; a `Top` that finds a cell of a kind
//! other than a record's (`Record`, `Mixed` or `Deferred`, whose unwritten
//! `Empty` is a valid `{}`) not yet written is an internal error diagnostic,
//! as a slot with no closure is for a function. Every
//! call, memoised call, future and await of a run, and every input of a
//! session, shares the one value stack whose bottom is the top level's
//! frame, which is what makes base 0 that frame. The checker sees to it
//! that the slot holds the function's closure before any read of it runs;
//! a read of a slot that holds no closure is still an internal error
//! diagnostic, not a panic.
//!
//! A call evaluates its callee to a closure through the `function` walker,
//! checks the nesting budget, and grows the stack by the callee's frame
//! size with `Vec::resize`; then it evaluates each argument in the caller's
//! frame, in source order, into its parameter's slot of the new frame, runs
//! the body in a new `Evaluator` whose base is the new frame's, and finally
//! truncates the stack back, whether the body succeeded or failed. An
//! argument whose value is an [`ArgValue::Default`], which a call gives a parameter
//! left out whose default is no constant, after the arguments written, in
//! parameter order, is the result of calling the default function the
//! callee's closure keeps at that capture, as a call in the caller's frame
//! is, with the caller the call gives its callee, and with the values of
//! the parameters before it, read from the new frame: so a default sees
//! the arguments before it, written or defaulted, and runs on the caller's
//! side of the call, before the callee's body starts, once for each call
//! that leaves its parameter out (`Evaluator::default_arg`). The
//! stack keeps its capacity, so once it has grown to a program's deepest
//! recursion, a call allocates nothing: not its frame, not its arguments'
//! slots, and nothing to look anything up, since every variable is a slot
//! or a capture index. `call` keeps only the closure, its frame's base and
//! the new evaluator across the body's evaluation.
//!
//! The callee of a `Call` may instead be a built-in function, a
//! [`Value::Builtin`]: an ordinary one (every one but `len` and `show`) is
//! a value of a function type that a variable, a parameter or a field may
//! hold, so the `function` walker yields a closure or a built-in, and a
//! call of a built-in applies it, its arguments evaluated in order as a
//! closure's are, each collected by the index of the parameter it fills,
//! with no frame (see [The built-in functions](#the-built-in-functions)).
//! This is the one place a node does not say what it does, since no node
//! could know which function a value is. A built-in whose typing is special
//! is never called through a value: the checker makes every call of one its
//! own node (`ListLen`, `Show`). A built-in's cell holds it from before the first
//! input runs (see [`crate::workspace`]); `Top` reads it as it reads a
//! function's slot.
//!
//! Calls nest on the Rust stack, one `call` and a few walker frames per
//! GCSL call, and so does every level of expression nesting between a
//! frame's body and a call in it; GCSL does not eliminate tail calls. So
//! calls and expressions share one budget of nesting levels,
//! [`MAX_DEPTH`], which keeps a program within [`crate::STACK_SIZE`]
//! whatever shape its recursion has. Each evaluator carries `levels`, the
//! nesting its enclosing frames have consumed where its frame's body
//! starts: 0 at the top level. A call at depth `d` in its frame's body (see
//! [`IrKind::Call`]), of a function whose body's height is `h` (see
//! [`crate::ir::FnCode::height`]), enters its callee's frame at
//! `entering = levels + d + CALL_LEVELS` levels, and fails with "call
//! nested too deeply" if `entering + h` exceeds `MAX_DEPTH`, before its
//! arguments are evaluated; otherwise the callee's body runs with
//! `levels = entering`. The whole height of the body is charged up front,
//! so that nothing inside the body, however deep, needs a check of its own:
//! the budget costs the evaluator three additions and a comparison per
//! call, and nothing per node.
//!
//! A function defined by an earlier input of a session can fail while a
//! later input runs; its error's spans then refer to the earlier input's
//! source. When an error comes out of the body of a closure whose IR is of
//! another input than the caller's, the call records that input on the
//! diagnostic, [`Diagnostic::input`], unless an inner call has already, so
//! the innermost foreign frame wins.
//!
//! # Located at the call
//!
//! A function item marked `#[track_caller]` reports the errors raised in
//! its own body at its call, as Rust's attribute of the name does: a
//! function of the standard library that wraps a built-in, as the
//! prelude's `exec` does, fails where it is called, not inside the
//! library. An error raised in the body, its span inside the body's and
//! no input recorded on it, is moved to the call that ran the body, the
//! call's span its own, and the labels inside the body dropped, its notes
//! kept ([`Evaluator::call_error`]). So a tracked function called from
//! another's body fails at that call, which the other then moves on to
//! its own call if it is tracked too; an error of a function the body
//! calls that is not tracked stays where it was raised, and so does one of
//! another input's code, a callback's, which has its input recorded.
//! A `fn` written inside a tracked body is part of it: an error raised in
//! one is moved too. Checking an error costs nothing until a call fails.
//!
//! # The built-in functions
//!
//! A call of an ordinary built-in function applies it to its arguments:
//!
//! - `int(x)` truncates its `Float` toward zero, failing with "integer
//!   overflow" when that is no `Int` (an infinity, or beyond `i64`), the
//!   operand labelled with its value, or "cannot truncate NaN to an Int";
//! - `float(x)` gives the `Float` nearest its `Int`;
//! - `exec(argv)` runs a program, one of the things GCSL does outside
//!   itself, beside `exec_live` and the file system's built-ins (below), and
//!   reports what it did. It runs `argv[0]`, found through `PATH` as
//!   the OS finds it (a name with a `/` is a path), with the arguments
//!   `argv[1..]`, each passed as it is, with no shell involved
//!   ([`std::process::Command`]). The program's standard input is the null
//!   device (`Stdio::null()`, not a closed descriptor), so it never reads
//!   what `gcsl`'s own standard input holds, a REPL's next line say; its
//!   standard output and standard error are captured whole, nothing of
//!   them shown as it runs; its working directory and environment are the
//!   `gcsl` process's. The call returns once the program has exited, with
//!   a record of the fields `status`, `stdout` and `stderr`, in that
//!   order: its exit status, or 128 plus the number of the signal that
//!   killed it, the shell's convention (`kill -9` gives 137); and its
//!   standard output and standard error decoded as UTF-8, each invalid
//!   sequence replaced by U+FFFD. A non-zero status is data, never an
//!   error. The record's layout is built once per run, by the driver that
//!   lays out the frame, its field names interned in the run's names, and
//!   shared by every record `exec` builds in the run, from whatever input
//!   or function calls it, since a call through a value is a plain `Call`
//!   that no node or IR could prepare it for; it reaches every frame
//!   beside the memo tables (see [`eval_with`](super::eval_with)). `exec` fails only when its
//!   program cannot run at all: "`exec` needs a program to run" for an
//!   empty `argv`, and "cannot run `name`" for a program that cannot be
//!   started, noting the OS's reason, as in "No such file or directory (os
//!   error 2)". Both are at the call, and end the run as every runtime
//!   error does. A memoised function around `exec` runs its program once
//!   for each distinct list of arguments, as its body runs once.
//! - `exec_live(argv)` is `exec`, the same program run the same way, the
//!   same record of the same layout, the same errors (but "`exec_live`
//!   needs a program to run"), but for one thing: while the program runs,
//!   what it writes to its standard output and to its standard error is
//!   ALSO copied, as it arrives, to `gcsl`'s own standard error, never to
//!   its standard output, which is a host's channel for results, and still
//!   captured whole into the record (`run_live`). Each stream is read on a
//!   thread of its own, both joined before the call returns, so a program
//!   that fills both pipes never waits on one while the other is read. The
//!   copy is line by line: each complete line, its newline included, is
//!   written as it is complete, in one write under the lock of standard
//!   error, so the lines of the two streams interleave in the order they
//!   are complete but never mix within a line, and a last line without a
//!   newline is written when its stream ends, as it is, with no newline
//!   added (`copy_lines`). The bytes are copied as they are, not decoded;
//!   the record's strings are decoded as `exec`'s are. A failure to write
//!   to `gcsl`'s standard error is ignored, the program running on and its
//!   output captured whole. It is what a build step whose program is silent
//!   for minutes until it finishes, a compiler, calls, so that its progress
//!   is seen; the prelude's `sh_live` is `sh` with it.
//! - `panic(message)` fails with the runtime error `message`, its own
//!   wording, at the call, with no label or note, ending the run as every
//!   runtime error does. A call of it is of kind `Never` (see
//!   [`crate::ir`]): every walker evaluates it as any call, through
//!   `Evaluator::call`, which then never returns a value, so no typed
//!   walker reaches for a value of its kind. A panic inside a function is
//!   reported where it happens, as any runtime error is: one in the
//!   prelude's `sh` is located in `<stdlib>/prelude.gcsl`.
//! - `split(s, sep)` gives the pieces of `s` between occurrences of `sep`,
//!   in order, as Rust's `str::split` does: `split("/a/b", "/")` is `["",
//!   "a", "b"]`, `split("", "/")` is `[""]`, and `split("a//b", "/")` is
//!   `["a", "", "b"]`. `sep` may be of any length; an empty one, which
//!   would match between every two characters, fails with "`split` needs a
//!   separator that is not empty", at the call.
//! - `join(parts, sep)` gives the strings of `parts` with `sep` between
//!   each pair, `""` for no parts.
//! - `starts_with(s, prefix)` and `ends_with(s, suffix)` compare bytes, as
//!   Rust's do: whether `s` begins, or ends, with the bytes of the other.
//! - `trim(s)` gives `s` without its leading and trailing whitespace, as
//!   Rust's `str::trim` finds it (Unicode's `White_Space`, so a no-break
//!   space too): `""` for a string of whitespace alone.
//! - `strip_prefix(s, prefix)` gives `s` without `prefix` if `s` begins
//!   with its bytes, as `starts_with` finds them, else `s` unchanged, as
//!   Rust's `str::strip_prefix(..).unwrap_or(s)`: once, so
//!   `strip_prefix("aab", "a")` is `"ab"`; an empty `prefix` gives `s`,
//!   and so does one that ends inside a character of `s`, which `s` cannot
//!   begin with. `strip_suffix(s, suffix)` does the same at the end, as
//!   `ends_with` finds it.
//! - `lines(s)` gives the lines of `s`, as Rust's `str::lines` does: split
//!   at each `\n`, a `\r` just before it removed too, with no empty last
//!   line after a final line ending. So `lines("")` is `[]`, `lines("\n")`
//!   is `[""]` and `lines("a\r\nb\r\n")` is `["a", "b"]`.
//! - `fields(s)` gives the pieces of `s` between runs of whitespace, none
//!   of them empty, as Rust's `str::split_whitespace` does (Unicode's
//!   `White_Space`, as `trim`'s): `[]` for `""` or whitespace alone.
//! - `compare_versions(a, b)` gives `-1`, `0` or `1` as `a` sorts before,
//!   equal to or after `b` in VERSION ORDER, the order of GNU `sort -V`'s
//!   core, which is dpkg's `verrevcmp` applied to the whole strings (no
//!   epoch or revision split off, and none of `sort -V`'s handling of file
//!   suffixes). Each string is taken, by its bytes, as runs alternately of
//!   non-digits and of ASCII digits, starting with non-digits, either run
//!   perhaps empty, and the runs compared pairwise in turn. Two non-digit
//!   runs compare byte by byte by a weight: `~` lightest, before even the
//!   end of the run, then the end of the run, then the ASCII letters by
//!   their byte, then every other byte by its byte plus 256 (dpkg's
//!   `order`), so `1.0~rc1` is before `1.0`, `1.0` before `1.0a` and
//!   `1.0.0`, and `a` before `.`. Two digit runs compare as numbers: their
//!   leading zeros skipped, the longer the greater, else by the first digit
//!   that differs, never parsed, so a run of any length is fine: `6.10` is
//!   after `6.6`. If every run is equal but the strings differ, as `1.01`
//!   and `1.1` do, or `""` and `"0"`, they compare by their bytes, so `0`
//!   means equal strings, as `sort -V` breaks such a tie.
//! - `sort_versions(vs)` gives the strings of `vs` ascending in that order,
//!   `compare_versions`' own comparison, by a stable sort: since only equal
//!   strings compare equal, a string there twice is there twice, side by
//!   side, and `[]` gives `[]` (`sort_versions`).
//!
//! These eleven are pure and fail only as said; none has a limit of its own
//! on the length of a string.
//!
//! `show(value)`, a built-in whose typing is special, is no call of a value
//! but a `Show` node of its argument, which the checker has awaited deeply
//! (`source_of`). It gives the value's GCSL SOURCE, which read back as an
//! expression, where its type is expected, is an equal value: exactly what
//! [`Value::repr`] shows, the one printer of values, so what `gcsl run`
//! shows of a value but for a string at the top, which is quoted as it is
//! inside a record or a list (`show("a")` is `"\"a\""`, `show(1)` is `"1"`,
//! `show({})` is `"{}"`, a record's fields in their order, its field names
//! quoted unless identifiers). A `Float` with no literal, NaN or an
//! infinity, is "`show` cannot render `nan` as source" (`inf`, `-inf`), at
//! the call, the first of the value in the order it is shown; and a future
//! still to run, of which the deep await leaves none, "cannot show a value
//! that is not ready yet". It is pure. Two values' sources do not read back
//! everywhere: `Int`'s least value, `-9223372036854775808`, whose digits
//! are no `Int` literal, and an empty list where no type is expected, which
//! has no element type to infer.
//!
//! The six of the file system, which a program reaches as the members of
//! the standard module `fs` (`fs.exists` is `fs_exists`, and so on), are
//! impure, and EVERY PATH THEY TAKE MUST BE ABSOLUTE: a relative one, `""`
//! included, is "`fs.read` needs an absolute path, found `x.txt`", before
//! anything is touched. A path is used as it is given, never normalised,
//! and nothing of them depends on the process's working directory. Each
//! failure is a runtime error at the call, naming the built-in as the
//! member of `fs` it is, whatever name the program called it by, the
//! path or paths, and the OS's own reason, in the message itself (where
//! `exec` notes it, its message naming only the program), ending the run
//! as every runtime error does (`file_system`):
//!
//! - `fs_exists(p)` (`fs.exists`) gives whether something is at `p`, symbolic
//!   links followed, so a link to nothing is `false`
//!   ([`Path::try_exists`](std::path::Path::try_exists)); what cannot be
//!   checked, through a directory that cannot be searched say, is "`fs.exists`
//!   could not check `/a`: REASON".
//! - `fs_read(p)` (`fs.read`) gives the contents of the file at `p`; one that
//!   cannot be read, missing or a directory, is "`fs.read` could not read `/a`:
//!   No such file or directory (os error 2)", and one that is not UTF-8
//!   "`fs.read`: `/a` is not UTF-8 text".
//! - `fs_mkdir(p)` (`fs.mkdir`) makes the directory `p` and every missing
//!   directory above it, as `mkdir -p`
//!   ([`fs::create_dir_all`](std::fs::create_dir_all)), a directory already
//!   there being fine, and is `{}`; a failure, a file in the way say, is
//!   "`fs.mkdir` could not create `/a`: REASON".
//! - `fs_copy(from, to)` (`fs.copy`) copies the FILE `from` to `to`, its
//!   contents and permissions, a file at `to` replaced
//!   ([`fs::copy`](std::fs::copy)), and is `{}`; `from` is checked to be
//!   absolute before `to`. A directory `from`, a missing one, or a directory of
//!   `to` that does not exist, is "`fs.copy` could not copy `/a` to `/b`:
//!   REASON".
//! - `fs_temp_dir()` (`fs.temp_dir`) makes a NEW empty directory, mode `0700`,
//!   and gives its absolute path, another each call: a call is impure, so never
//!   shared with another, and in a memoised function it runs once for each list
//!   of arguments, as its body does. The directories are the numbered children,
//!   `1`, `2` and so on, of one parent directory, `gcsl-PID-N` in the system's
//!   temporary directory, or in the temp root a workspace's host chose
//!   ([`crate::workspace::Workspace::set_temp_root`]), that the run's owner
//!   holds, a [`crate::workspace::Workspace`] (a lone file's run is one) or a
//!   [`crate::Session`], made by its first call and never before, and removed,
//!   with everything in it, as the owner is dropped, whether its runs succeeded
//!   or failed ([`TempDirs`](super::TempDirs)); a temp root is never removed, and one that is
//!   not absolute is a REASON below. It reaches every frame beside the memo
//!   tables, as the layout of `exec`'s records does (see [`eval_with`](super::eval_with)). A
//!   process killed before it drops the owner leaves the directory behind. One
//!   that cannot be made is "`fs.temp_dir` could not create a temporary
//!   directory: REASON"; and a call in a run without an owner, which only a
//!   test can make, is an internal error.
//! - `fs_write(p, contents)` (`fs.write`) writes `contents` to the file `p`,
//!   making it or replacing it whole, and is `{}`, ATOMICALLY: to a new
//!   temporary file beside it, `.NAME.gcsl-tmp-PID-N`, made exclusively,
//!   flushed to the disk and renamed over `p`, so a reader sees the old
//!   contents or the new, never a part
//!   ([`replace_file`](super::system::replace_file)). The directory of `p` must exist.
//!   A file it replaces keeps nothing, its permissions neither: the new file
//!   has the mode any new file gets, `0666` less the umask; and a symbolic link
//!   at `p` is replaced, not followed. Any failure, a missing directory, `p` a
//!   directory, is "`fs.write` could not write `/a/b`: REASON", and leaves no
//!   temporary file behind.

use std::cmp::Ordering;
use std::rc::Rc;

use crate::{CALL_LEVELS, MAX_DEPTH};
use crate::ast::AttributeId;
use crate::check::{Builtin, TRACK_CALLER};
use crate::diagnostic::Diagnostic;
use crate::ir::{ArgValue, ConstId, FnKind, IrId, IrKind, IrNode, IrRange};
use crate::names::Name;
use crate::source::Span;
use crate::value::{Closure, Tag, Value, ValueKind, Variant, int_to_float};

use super::{
    Callee, DEFERRED_FUNCTION, Evaluator, Flow, MIXED, OVERFLOW, Unwind, compare_versions,
    mistyped, nested_too_deeply,
};
use super::builtins::{enum_builtin, secret_builtin};
use super::kinds::{as_function, float_to_int, into_callee, mixed};
use super::system::{exec, file_system, write_file};
use super::text::{join, sort_versions, split, string_list, strip_prefix, strip_suffix, trim};

impl Evaluator<'_> {
    /// Evaluates the `Call` `node` of `callee` with the arguments `args`,
    /// at depth `depth` in this frame's body (see the module documentation),
    /// and returns the body's value; or, if the callee is a closure of a
    /// memoised function, hands the call to [`Evaluator::memo_call`].
    pub(super) fn call(
        &mut self,
        (callee, args, depth): (IrId, IrRange, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let closure = match self.function(callee)? {
            Callee::Closure(closure) => closure,
            Callee::Builtin(builtin) => return self.call_builtin(builtin, args, node),
            Callee::Tag(tag) => return self.call_tag(tag, args, node),
        };
        let Some(code) = closure.code.find_function(closure.function) else {
            return mistyped(node, "a closure of a function its IR lacks");
        };
        if self.past_deadline() {
            return self.timed_out(node);
        }
        if code.kind == FnKind::Memo {
            return self.memo_call((&closure, code), (args, depth), node);
        }
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        // An argument may fail, or stop for a loop around the call, after
        // the frame has grown: either way the frame goes.
        if let Err(unwind) = self.arguments(args, base, (&closure, depth)) {
            self.stack.truncate(base);
            return Err(unwind);
        }
        let passed = self.callee_caller();
        if code.retry > 0 || code.timeout.is_some() {
            return self.guarded_call(&closure, base, (passed, entering), node);
        }
        let result = Evaluator {
            ir: &closure.code,
            stack: self.stack,
            base,
            captures: &closure.captures,
            callee: Some(&closure),
            memo: self.memo,
            shared: self.shared,
            caller: passed,
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        // A `return` for the function is its value; no other stop leaves
        // its body: the checker sees to it.
        result
            .or_else(|unwind| unwind.returned(node.span))
            .map_err(|error| self.call_error(error, &closure, node).into())
    }

    /// `error`, which came out of a call `node` of `closure`, as the
    /// calling frame reports it: moved to the call if `closure` is tracked
    /// and raised it in its own body (see [Located at the
    /// call](#located-at-the-call)), and otherwise given the input of
    /// `closure`'s code if that is another input than this frame's and
    /// none is recorded on it.
    pub(super) fn call_error(
        &self,
        mut error: Diagnostic,
        closure: &Closure,
        node: &IrNode,
    ) -> Diagnostic {
        if error.input.is_some() {
            return error;
        }
        if raised_in_tracked(&error, closure) {
            error.span = node.span;
            error.labels.retain(|label| label.input.is_some());
        } else if closure.code.input() != self.ir.input() {
            error.input = Some(closure.code.input());
        }
        error
    }

    /// Evaluates the `Call` `node` of the built-in function `builtin` with
    /// the arguments `args`: a call through a value of its function type (a
    /// parameter, a variable, the prelude's `int`), the one call whose node
    /// cannot say what it calls. The arguments are evaluated in order, as a
    /// closure's are, each collected by the index of the parameter it
    /// fills, and the built-in applied to them (see [The built-in
    /// functions](#the-built-in-functions)): `int` truncates its `Float`
    /// toward zero, failing with "integer overflow" when that is not an
    /// `Int` (an infinity, or beyond `i64`) and "cannot truncate NaN to an
    /// Int" for NaN; `float` gives the `Float` nearest its `Int`; `exec`
    /// runs its program, and `exec_live` runs it so, copying its output to
    /// this process's standard error as it runs ([`exec`]); `panic` fails
    /// with its message, at the call; `split` ([`split`]), `join`
    /// ([`join`]), `starts_with`,
    /// `ends_with`, `trim` ([`trim`]), `strip_prefix` ([`strip_prefix`]),
    /// `strip_suffix` ([`strip_suffix`]), `lines` and `fields`
    /// ([`string_list`]) work on their strings as Rust's `str` does,
    /// `compare_versions` ([`compare_versions`]) gives `-1`, `0` or `1` by
    /// version order, `sort_versions` ([`sort_versions`]) sorts by it, and
    /// A new temporary directory, for the call `node` of `fs_temp_dir`:
    /// removed with everything in it when the run's owner drops it.
    pub(super) fn temp_dir(&self, node: &IrNode) -> Flow<Value> {
        let Some(temp_dirs) = self.shared.temp_dirs else {
            return mistyped(node, "`fs.temp_dir` called in a run without an owner");
        };
        match temp_dirs.make() {
            Ok(dir) => Ok(Value::Str(Rc::from(dir))),
            Err(reason) => Err(Diagnostic::error(
                format!("`fs.temp_dir` could not create a temporary directory: {reason}"),
                node.span,
            )
            .into()),
        }
    }

    /// The tag of the `Tag` node `(key, of, shown)`: a variant as a value.
    pub(super) fn tag(&self, (key, of, shown): (Name, Name, ConstId)) -> Rc<Tag> {
        Rc::new(Tag {
            key,
            of,
            shown: Rc::clone(self.ir.string(shown)),
        })
    }

    /// The value of the enum of `tag`'s variant whose payload is the one
    /// argument `args` hold, for the `Call` `node` of the variant (see the
    /// checker's "Enums").
    #[inline(never)]
    pub(super) fn call_tag(&mut self, tag: Rc<Tag>, args: IrRange, node: &IrNode) -> Flow<Value> {
        let &[arg] = self.ir.items(args) else {
            return mistyped(node, "a variant called with other than one argument");
        };
        let IrKind::Arg {
            value: ArgValue::Node(value),
            ..
        } = self.ir.get(arg).kind
        else {
            return mistyped(node, "a variant's argument is not a node's value");
        };
        let payload = self.value(value)?;
        Ok(Value::Variant(Rc::new(Variant { tag, payload })))
    }

    /// the file system's, `fs_exists`, `fs_read`, `fs_mkdir`, `fs_copy`
    /// ([`file_system`]), `fs_temp_dir` ([`TempDirs::make`](super::TempDirs::make)) and `fs_write`
    /// ([`write_file`]), touch the file system at absolute paths, each
    /// failure a runtime error at the call naming the built-in as a member
    /// of `fs`. A built-in whose typing is special is never called
    /// through a value (the checker lowers each call of one to its own
    /// node, `len`'s to a `ListLen`, `show`'s to a `Show`), so reaching one
    /// here is an internal error.
    #[inline(never)]
    pub(super) fn call_builtin(
        &mut self,
        builtin: Builtin,
        args: IrRange,
        node: &IrNode,
    ) -> Flow<Value> {
        // Each parameter's argument, with its span, by the parameter's index.
        let mut arguments: [Option<(Value, Span)>; Builtin::MAX_PARAMS] = Default::default();
        for &arg in self.ir.items(args) {
            let arg = self.ir.get(arg);
            let IrKind::Arg {
                param,
                value: ArgValue::Node(value),
            } = arg.kind
            else {
                return mistyped(arg, "a built-in's argument is not a node's value");
            };
            let span = self.ir.get(value).span;
            let value = self.value(value)?;
            let Some(entry) = arguments.get_mut(param as usize) else {
                return mistyped(arg, "an argument for a parameter the built-in lacks");
            };
            *entry = Some((value, span));
        }
        let [first, second] = arguments;
        let second = second.map(|(value, _)| value);
        match (builtin, first, second) {
            (Builtin::Int, Some((Value::Float(x), span)), None) => match float_to_int(x) {
                Some(truncated) => Ok(Value::Int(truncated)),
                None if x.is_nan() => {
                    Err(Diagnostic::error("cannot truncate NaN to an Int", node.span).into())
                }
                None => Err(Diagnostic::error(OVERFLOW, node.span)
                    .with_context(span, Value::Float(x).to_string())
                    .into()),
            },
            (Builtin::Float, Some((Value::Int(x), _)), None) => Ok(Value::Float(int_to_float(x))),
            (Builtin::Exec | Builtin::ExecLive, Some((Value::List(words), _)), Some(options)) => {
                let deadline = self.shared.earliest.get();
                let words = (&words, &options);
                match exec(builtin, words, (self.shared.exec_result, deadline), node)? {
                    Some(value) => Ok(value),
                    None => self.timed_out(node),
                }
            }
            (Builtin::Panic, Some((Value::Str(message), _)), None) => {
                Err(Diagnostic::error(&*message, node.span).into())
            }
            (Builtin::Split, Some((Value::Str(s), _)), Some(Value::Str(sep))) => {
                split(&s, &sep, node)
            }
            (Builtin::Join, Some((Value::List(parts), _)), Some(Value::Str(sep))) => {
                join(&parts, &sep, node)
            }
            (Builtin::StartsWith, Some((Value::Str(s), _)), Some(Value::Str(prefix))) => {
                Ok(Value::Bool(s.starts_with(&*prefix)))
            }
            (Builtin::EndsWith, Some((Value::Str(s), _)), Some(Value::Str(suffix))) => {
                Ok(Value::Bool(s.ends_with(&*suffix)))
            }
            (Builtin::Trim, Some((Value::Str(s), _)), None) => Ok(Value::Str(trim(s))),
            (Builtin::StripPrefix, Some((Value::Str(s), _)), Some(Value::Str(prefix))) => {
                Ok(Value::Str(strip_prefix(s, &prefix)))
            }
            (Builtin::StripSuffix, Some((Value::Str(s), _)), Some(Value::Str(suffix))) => {
                Ok(Value::Str(strip_suffix(s, &suffix)))
            }
            (Builtin::Lines, Some((Value::Str(s), _)), None) => Ok(string_list(s.lines())),
            (Builtin::Fields, Some((Value::Str(s), _)), None) => {
                Ok(string_list(s.split_whitespace()))
            }
            (Builtin::CompareVersions, Some((Value::Str(a), _)), Some(Value::Str(b))) => {
                Ok(Value::Int(match compare_versions(&a, &b) {
                    Ordering::Less => -1,
                    Ordering::Equal => 0,
                    Ordering::Greater => 1,
                }))
            }
            (Builtin::SortVersions, Some((Value::List(versions), _)), None) => {
                sort_versions(&versions, node)
            }
            (Builtin::FsWrite, Some((Value::Str(p), _)), Some(Value::Str(contents))) => {
                write_file(&p, &contents, node)
            }
            (Builtin::FsTempDir, None, None) => self.temp_dir(node),
            (
                Builtin::FsExists | Builtin::FsRead | Builtin::FsMkdir | Builtin::FsCopy,
                Some((Value::Str(p), _)),
                to,
            ) => file_system(builtin, &p, to.as_ref(), node),
            (
                Builtin::VariantMatches | Builtin::VariantPayload | Builtin::PartialMatches,
                Some((first, _)),
                Some(second),
            ) => enum_builtin(builtin, (&first, &second), node),
            (
                Builtin::Secret | Builtin::SecretValue | Builtin::SecretReveal,
                Some((value, _)),
                identity,
            ) => secret_builtin(builtin, value, identity, node),
            _ => mistyped(
                node,
                "a built-in function called through a value with an argument of another kind",
            ),
        }
    }

    /// Evaluates the arguments `args` of a call of `callee` at depth `depth`
    /// in this frame's body, `Arg` nodes, in order, in this frame, writing
    /// each to its parameter's slot of the frame that starts at `base`. An
    /// argument a call gives a parameter's default that is no constant, a
    /// [`ArgValue::Default`], comes after the ones written, in parameter
    /// order, and is the value of the default function the callee keeps at
    /// its capture called with the values of the parameters before it, read
    /// from the new frame ([`Evaluator::default_arg`]).
    pub(super) fn arguments(
        &mut self,
        args: IrRange,
        base: usize,
        (callee, depth): (&Rc<Closure>, u16),
    ) -> Flow<()> {
        for &arg in self.ir.items(args) {
            let node = self.ir.get(arg);
            let IrKind::Arg { param, value } = node.kind else {
                return mistyped(node, "a call's argument is not an argument");
            };
            let value = match value {
                ArgValue::Node(value) => self.value(value)?,
                ArgValue::Default { capture, .. } => {
                    let earlier = base..base + param as usize;
                    self.default_arg((callee, capture), earlier, (depth, node))?
                }
            };
            let Some(slot) = self.stack.get_mut(base + param as usize) else {
                return mistyped(node, "an argument for a parameter outside the frame");
            };
            *slot = value;
        }
        Ok(())
    }

    /// The value a call at depth `depth` in this frame's body gives a
    /// parameter whose default is no constant, the argument `node`: the
    /// default function's closure, capture number `capture` of `callee`,
    /// called, as a call in this frame is, with the values of the stack's
    /// slots `earlier`, the parameters before it in the callee's new frame
    /// (see the checker's "Calls"). So the default runs on the caller's side
    /// of the call, with the caller's caller, before the callee's body
    /// starts.
    pub(super) fn default_arg(
        &mut self,
        (callee, capture): (&Rc<Closure>, u32),
        earlier: std::ops::Range<usize>,
        (depth, node): (u16, &IrNode),
    ) -> Flow<Value> {
        let Some(Value::Fn(default)) = callee.captures.get(capture as usize) else {
            return mistyped(node, "a default that its callee keeps no function for");
        };
        let default = Rc::clone(default);
        let Some(code) = default.code.find_function(default.function) else {
            return mistyped(node, "a closure of a function its IR lacks");
        };
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            return Err(nested_too_deeply(node).into());
        }
        if earlier.end > self.stack.len() {
            return mistyped(node, "a default's parameters lie outside its call's frame");
        }
        // The default's frame sits above the callee's, so its parameters
        // are copied up, in place: nothing is allocated once the stack has
        // grown, as for any call.
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        for (at, from) in earlier.enumerate() {
            let value = self.stack.get(from).cloned().unwrap_or(Value::Empty);
            if let Some(slot) = self.stack.get_mut(base + at) {
                *slot = value;
            }
        }
        let passed = self.callee_caller();
        let result = Evaluator {
            ir: &default.code,
            stack: self.stack,
            base,
            captures: &default.captures,
            callee: Some(&default),
            memo: self.memo,
            shared: self.shared,
            caller: passed,
            levels: entering,
        }
        .value(code.body);
        self.stack.truncate(base);
        match result.or_else(|unwind| unwind.returned(node.span)) {
            Err(mut error) if error.input.is_none() && default.code.input() != self.ir.input() => {
                error.input = Some(default.code.input());
                Err(error.into())
            }
            result => result.map_err(Unwind::from),
        }
    }

    /// Evaluates the `Call` `node` of `callee` with the arguments `args`, at
    /// depth `depth`, as a value that `extract` accepts: one of the kind the
    /// node states.
    pub(super) fn call_as<T>(
        &mut self,
        call: (IrId, IrRange, u16),
        node: &IrNode,
        extract: fn(Value) -> Option<T>,
    ) -> Flow<T> {
        if mixed(node) {
            return mistyped(node, MIXED);
        }
        match extract(self.call(call, node)?) {
            Some(value) => Ok(value),
            None => mistyped(
                node,
                "a call returned a value of another kind than its node",
            ),
        }
    }

    /// Evaluates a function-typed node to what a call of it calls: a
    /// closure, or a built-in function, a value of the `builtins` module
    /// called through a value of its function type (see
    /// [`Evaluator::call_builtin`]).
    pub(super) fn function(&mut self, id: IrId) -> Flow<Callee> {
        let node = self.ir.get(id);
        match node.kind {
            IrKind::Load(_, ValueKind::Deferred)
            | IrKind::LoadCapture(_, ValueKind::Deferred)
            | IrKind::Field {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Call {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::Index {
                kind: ValueKind::Deferred,
                ..
            }
            | IrKind::MakeFuture { .. } => mistyped(node, DEFERRED_FUNCTION),
            IrKind::Await { .. } => self.await_as(node, into_callee),
            IrKind::As { .. } => self.narrow_as(node, into_callee),
            IrKind::Index { list, index, .. } => self.index_as((list, index), node, into_callee),
            IrKind::Load(slot, _) => self.slot_as(slot, node, as_function),
            IrKind::LoadCapture(index, _) => self.capture_as(index, node, as_function),
            IrKind::LoadCallee => self.callee(node).map(Callee::Closure),
            IrKind::Tag { key, of, shown } => Ok(Callee::Tag(self.tag((key, of, shown)))),
            IrKind::ContextRead { .. } | IrKind::With { .. } | IrKind::Recover { .. } => {
                match into_callee(self.whole_node(node)?) {
                    Some(callee) => Ok(callee),
                    None => mistyped(node, "a context's value that is no function"),
                }
            }
            IrKind::Top(slot, _) => self.top_function(slot, node),
            IrKind::MakeClosure { function, captures } => {
                self.make_closure(function, captures).map(Callee::Closure)
            }
            IrKind::Meta(function) => self.meta(function, node),
            IrKind::Block { items, result } => {
                self.items(items)?;
                self.function(result)
            }
            IrKind::If { cond, then, else_ } => {
                let branch = self.branch(cond, then, else_)?;
                self.function(branch)
            }
            IrKind::Field { record, name, .. } => self.field_as((record, name), node, into_callee),
            IrKind::Call {
                callee,
                args,
                depth,
                ..
            } => self.call_as((callee, args, depth), node, into_callee),
            IrKind::Break { .. }
            | IrKind::Continue { .. }
            | IrKind::Return { .. }
            | IrKind::Catch { .. }
            | IrKind::For { .. }
            | IrKind::Loop { .. } => self.stopping(node, into_callee),
            _ => mistyped(node, "a node of another kind where a function was expected"),
        }
    }
}

/// Whether `error`, with no input recorded on it, was raised in the body of
/// `closure`'s function, marked `#[track_caller]` (see [Located at the
/// call](#located-at-the-call)): its span inside the body's.
pub(super) fn raised_in_tracked(error: &Diagnostic, closure: &Closure) -> bool {
    let tracked = closure
        .attributes()
        .iter()
        .any(|attribute| attribute.id == AttributeId::Builtin(TRACK_CALLER));
    tracked
        && closure
            .code
            .find_function(closure.function)
            .is_some_and(|code| {
                let body = closure.code.get(code.body).span;
                body.start <= error.span.start && error.span.end <= body.end
            })
}
