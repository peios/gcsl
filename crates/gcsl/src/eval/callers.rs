//! Callers and helpers: the callee of a call, a caller's description, and
//! making closures.
//!
//! # Callers and helpers
//!
//! The `fileroot` module (see [`crate::workspace`], "The standard library")
//! tells code where files are. Two of its members are constants that
//! lowering makes `StrConst`s: `fileroot.this`, the directory of the file
//! the expression is written in, and `fileroot.workspace`, the root of the
//! run's workspace. The other two are the evaluator's.
//!
//! Every frame, a call's, an await's running a future's body, and the top
//! level's, has a *caller*: the directory of the file whose code made the
//! call it runs for, by its index in the run's table of directories, which
//! the driver built and every frame shares (each file's code names its own
//! directory by it, [`Ir::dir`](super::Ir::dir)), and a *helper flag*, false as it starts.
//! A call of a closure (a `def`, a `fn`, a memoised function, a function
//! value, whatever its callee's code) gives its callee, as the callee's
//! caller, the calling frame's own caller if the calling frame's helper
//! flag is set, and otherwise the directory of the file the calling code is
//! of, the IR the calling frame runs (a closure runs its code's IR, so a
//! closure of `_lib/ppk.gcsl` passes `_lib` on wherever it is called). A
//! built-in function is no frame, and changes nothing. A top level's caller
//! is its own file's directory, as its `fileroot.this` is. A caller has an
//! *origin*, a number from a counter of [`MemoTables`](super::MemoTables), new for each caller
//! set from a file and shared by every frame it is passed on to, which is
//! what the memoisation of [Memoised functions](crate::eval::memo#memoised-functions)
//! follows. It is given only when it is needed: to a memoised call's
//! frame, and to a frame as it passes its caller on to a callee or a future
//! it makes; until then a frame's is none (`NO_ORIGIN`), which no running
//! memoised call can share. So a plain call sets a small index, a flag and
//! no origin, and costs one comparison more.
//!
//! - `fileroot.caller` (`IrKind::Caller`) yields the text of the frame's
//!   caller, from the table, a reference-count increment, after recording
//!   it as an ambient input of the running memoised calls it can affect.
//! - A standard file's code is a helper always ([`Ir::is_standard`](super::Ir::is_standard)):
//!   its frames pass their caller on, and its relative paths are its
//!   caller's (see [Where a path is](crate::eval::files#where-a-path-is)),
//!   so that `exec`'s `cwd = "."`, which the prelude's `exec` hands the
//!   built-in, is the caller's directory.
//! - `fileroot.helper()` (`IrKind::Helper`) sets the frame's helper flag,
//!   for the rest of the frame: the calls it made before are unaffected,
//!   and every call it makes after passes its own caller on, so a library
//!   function that works on its caller's paths calls `fileroot.helper()`
//!   before calling another that does. It is `{}`. At a top level it does
//!   nothing: the top level's caller is its own file's directory, which it
//!   would pass on anyway.
//! - A `lazy` runs as if where it was written: the future captures, when
//!   it is made, the making frame's caller, helper flag and origin
//!   ([`crate::value::Future`]), and its body runs with them, whoever awaits
//!   it, so a step `lazy { ppk.commit(...) }` written in `test.gcsl` and
//!   awaited by a build driver elsewhere calls `commit` with `test.gcsl`'s
//!   directory.

use std::rc::Rc;

use crate::ast::FnId;
use crate::ir::{IrKind, IrNode, IrRange};
use crate::value::{Caller, Closure, Value};

use super::{Evaluator, Flow, internal};
use super::memo::{Ambient, NO_ORIGIN};

impl Evaluator<'_> {
    /// A new reference to the running closure, for the `LoadCallee`
    /// `node`.
    pub(super) fn callee(&self, node: &IrNode) -> Flow<Rc<Closure>> {
        self.callee
            .map(Rc::clone)
            .ok_or_else(|| internal("load of the callee outside a function", node.span).into())
    }

    /// Evaluates the `Caller` `node`, `fileroot.caller`: the text of this
    /// frame's caller, from the run's table, after recording it as an
    /// ambient dependency of every running memoised call of its origin (see
    /// [Callers and helpers](#callers-and-helpers)). A caller outside the
    /// table, which the driver never gives, is an internal error.
    #[inline(never)]
    pub(super) fn caller_text(&mut self, node: &IrNode) -> Flow<Rc<str>> {
        // An origin not given yet is no running memoised call's.
        if self.caller.origin != NO_ORIGIN {
            self.memo
                .depend(self.caller.origin, &Ambient::Caller(self.caller.dir));
        }
        self.shared
            .dirs
            .get(self.caller.dir as usize)
            .map(Rc::clone)
            .ok_or_else(|| internal("a caller outside the run's directories", node.span).into())
    }

    /// This frame's caller, its origin given now if it had none, as it must
    /// be before another frame shares it: a callee it passes its caller on
    /// to, or a future it makes (see [Callers and
    /// helpers](#callers-and-helpers)).
    pub(super) fn shared_caller(&mut self) -> Caller {
        if self.caller.origin == NO_ORIGIN {
            self.caller.origin = self.memo.new_origin();
        }
        self.caller
    }

    /// Evaluates `node`, an `Empty` or a `Helper`, both `{}`: a `Helper`,
    /// `fileroot.helper()`, first sets this frame's helper flag, so that the
    /// calls it makes from now on pass its own caller on (see [Callers and
    /// helpers](#callers-and-helpers)).
    pub(super) fn empty(&mut self, node: &IrNode) -> Value {
        if matches!(node.kind, IrKind::Helper) {
            self.caller.helper = true;
        }
        Value::Empty
    }

    /// The caller a call this frame makes gives its callee (see [Callers and
    /// helpers](#callers-and-helpers)): this frame's own, sharing its
    /// origin, given now if it had none, once `fileroot.helper()` has run in
    /// it or if its code is a standard file's; else the directory of the file this frame's code is of, with no
    /// origin yet, which the callee is given only if it needs one. The
    /// callee's helper flag is false.
    pub(super) fn callee_caller(&mut self) -> Caller {
        if self.caller.helper || self.ir.is_standard() {
            Caller {
                helper: false,
                ..self.shared_caller()
            }
        } else {
            Caller {
                dir: self.ir.dir(),
                helper: false,
                origin: NO_ORIGIN,
            }
        }
    }

    /// Builds a closure of `function` capturing the values of `captures`,
    /// read in this frame: one allocation for the captured values, sized
    /// exactly (none if there are none), and one for the closure, which
    /// shares the IR.
    pub(super) fn make_closure(&mut self, function: FnId, captures: IrRange) -> Flow<Rc<Closure>> {
        let items = self.ir.items(captures);
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            values.push(self.value(item)?);
        }
        Ok(Rc::new(Closure {
            code: Rc::clone(self.ir),
            function,
            captures: values.into_boxed_slice(),
        }))
    }
}
