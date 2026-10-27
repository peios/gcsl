//! Calling memoised functions: looking up, beginning and ending a call's entry
//! in its memo table.

use std::rc::Rc;

use crate::{CALL_LEVELS, MAX_DEPTH};
use crate::diagnostic::Diagnostic;
use crate::ir::{FnCode, IrNode, IrRange};
use crate::value::{Caller, Closure, Value};

use super::calls::raised_in_tracked;
use super::{Evaluator, Flow, depends_on_own_result, mistyped, nested_too_deeply};
use super::memo::{Lookup, Mark, MemoBegin, NO_ORIGIN, Replayed, Running, key_eq};

impl Evaluator<'_> {
    /// Evaluates the `Call` `node` of `closure`, whose code `code` is
    /// memoised, with the arguments `args`, at depth `depth` in this frame's
    /// body (see [Memoised functions](crate::eval::memo#memoised-functions)): returns what
    /// the closure returned for equal arguments before, if it has; fails if
    /// it is running with equal arguments; and otherwise runs the body as
    /// [`Evaluator::call`] does and records what it returns. The lookup and
    /// the recording are in helpers, so that this frame, on the recursion
    /// path of memoised recursion, keeps only what the body's evaluation
    /// needs across it.
    pub(super) fn memo_call(
        &mut self,
        (closure, code): (&Rc<Closure>, &FnCode),
        (args, depth): (IrRange, u16),
        node: &IrNode,
    ) -> Flow<Value> {
        let base = self.stack.len();
        let (mark, entering, caller) =
            match self.memo_begin((closure, code), (args, depth), node)? {
                MemoBegin::Found(value) => return Ok(value),
                MemoBegin::Started(mark) => (Some(mark), mark.entering, mark.caller),
                MemoBegin::Unkeyed(entering, caller) => (None, entering, caller),
            };
        // Its body is a scope of its own, in graph mode, keyed by the call's
        // callee and arguments (see [Graph mode](crate::eval::graph#graph-mode)).
        let scoped = self.shared.graph.is_some() && {
            let params = base..base + code.param_count as usize;
            let given: Vec<Value> = self.stack.get(params).unwrap_or_default().to_vec();
            let function = Value::Fn(Rc::clone(closure));
            let mut parts: Vec<&Value> = vec![&function];
            parts.extend(given.iter());
            self.enter_scope(&parts)
        };
        let result = if code.retry > 0 || code.timeout.is_some() {
            self.guarded_body((closure, code), base, (caller, entering), node)
        } else {
            Evaluator {
                ir: &closure.code,
                stack: self.stack,
                base,
                captures: &closure.captures,
                callee: Some(closure),
                memo: self.memo,
                shared: self.shared,
                caller,
                levels: entering,
            }
            .value(code.body)
        };
        self.leave_scope(scoped);
        self.stack.truncate(base);
        // The call is no longer running: nothing more depends on it.
        if mark.is_some() {
            self.memo.running.pop();
        }
        // A `return` for the function is its value, which `memo_end` then
        // records as any, here, inside the running entry; no other stop
        // leaves its body: the checker sees to it.
        let result = result.or_else(|unwind| unwind.returned(node.span));
        match mark {
            Some(mark) => self.memo_end(closure, mark, result, node),
            None => result.map_err(|error| self.memo_error(error, closure, false, node).into()),
        }
    }

    /// Begins the memoised call `node` of `closure`, whose code is `code`,
    /// at depth `depth` in this frame's body: grows the stack by the
    /// callee's frame, evaluates the arguments `args` into it, and looks
    /// them up in the closure's memo table, with the caller the call gives
    /// its callee (see [Callers and helpers](crate::eval::callers#callers-and-helpers)),
    /// allocating nothing. If the closure has returned for equal arguments
    /// and an equal value of every ambient input its entry recorded, the
    /// stack is truncated back and that value found, the entry's
    /// dependencies passed on to the running calls of this frame's origin
    /// if the callee would have shared it. Otherwise, if it is running so,
    /// or its body would take the nesting past the budget, the stack is
    /// truncated back and the error returned; and if neither, the arguments
    /// are recorded as running, the one allocation of the call's
    /// memoisation, the call is pushed on the running calls, and the frame
    /// is left for the body.
    pub(super) fn memo_begin(
        &mut self,
        (closure, code): (&Rc<Closure>, &FnCode),
        (args, depth): (IrRange, u16),
        node: &IrNode,
    ) -> Flow<MemoBegin> {
        let base = self.stack.len();
        self.stack
            .resize(base + code.frame_size as usize, Value::Empty);
        // An argument may fail, or stop for a loop around the call, after
        // the frame has grown: either way the frame goes, and no entry has
        // been made yet.
        if let Err(unwind) = self.arguments(args, base, (closure, depth)) {
            self.stack.truncate(base);
            return Err(unwind);
        }
        self.arguments_done();
        let caller = self.callee_caller();
        let params = base..base + code.param_count as usize;
        let Some(key) = self.stack.get(params.clone()) else {
            self.stack.truncate(base);
            return mistyped(node, "a call's parameters lie outside its frame");
        };
        // A callee that would share this frame's origin passes a hit's
        // dependencies on to the running calls of that origin, as a read
        // in it would.
        let inherited = self.caller.helper;
        let context = self.shared.context.borrow().clone();
        let hash = match self
            .memo
            .lookup(closure, key, (caller, inherited), context.as_ref())
        {
            Lookup::Missing(hash) => hash,
            Lookup::Done(value, replayed) => {
                self.stack.truncate(base);
                // As reads here, of the step whose unit is running.
                let context = self.shared.context.borrow().clone();
                for read in replayed {
                    match read {
                        Replayed::Context(key) => self.note_step_read(key, context.as_ref()),
                        Replayed::All => self.note_step_reads_all(),
                    }
                }
                self.memo_found();
                return Ok(MemoBegin::Found(value));
            }
            Lookup::Running => {
                self.stack.truncate(base);
                return Err(depends_on_own_result(node).into());
            }
        };
        let entering = self.levels + usize::from(depth) + CALL_LEVELS;
        if entering + code.height as usize > MAX_DEPTH {
            self.stack.truncate(base);
            return Err(nested_too_deeply(node).into());
        }
        let key = self.stack.get(params).unwrap_or_default();
        // Arguments that equal nothing, themselves included, as those
        // holding NaN, would make an entry no call could find.
        if !key_eq(key, key) {
            return Ok(MemoBegin::Unkeyed(entering, caller));
        }
        // A running call has an origin, by which the reads it depends on
        // find it.
        let caller = if caller.origin == NO_ORIGIN {
            Caller {
                origin: self.memo.new_origin(),
                ..caller
            }
        } else {
            caller
        };
        let entry = self.memo.start(closure, hash, key);
        let context = self.shared.context.borrow().clone();
        self.memo.running.push(Running {
            closure: Rc::as_ptr(closure),
            hash,
            entry,
            origin: caller.origin,
            context,
        });
        Ok(MemoBegin::Started(Mark {
            hash,
            entry,
            entering,
            caller,
        }))
    }

    /// Ends the memoised call `node` of `closure` that
    /// [`Evaluator::memo_begin`] started as `mark`, whose body gave
    /// `result`: records the value it returned, or, if it failed, removes
    /// the running mark, so that a later call runs the body again, and
    /// returns the error as [`Evaluator::memo_error`] reports it.
    pub(super) fn memo_end(
        &mut self,
        closure: &Rc<Closure>,
        mark: Mark,
        result: Result<Value, Diagnostic>,
        node: &IrNode,
    ) -> Flow<Value> {
        match result {
            Ok(value) => match self.memo.finish(closure, &mark, Some(&value)) {
                Some(_) => Ok(value),
                None => mistyped(node, "a memoised call's entry is missing"),
            },
            Err(error) => match self.memo.finish(closure, &mark, None) {
                Some(reentered) => Err(self.memo_error(error, closure, reentered, node).into()),
                None => mistyped(node, "a memoised call's entry is missing"),
            },
        }
    }

    /// `error`, raised while `closure` ran for the memoised call `node`, as
    /// it is reported: stamped with the closure's input when its code is of
    /// another session input than this frame's, as [`Evaluator::call`]
    /// stamps one, unless an inner frame has stamped an input on it
    /// already; and otherwise, if the error is that a call inside the body
    /// called the closure again with the same arguments (`reentered`),
    /// labelled "called here with the same arguments" at this call, as the
    /// await that ran a future that awaits itself is labelled.
    pub(super) fn memo_error(
        &self,
        error: Diagnostic,
        closure: &Closure,
        reentered: bool,
        node: &IrNode,
    ) -> Diagnostic {
        if error.input.is_some() {
            return error;
        }
        if closure.code.input() != self.ir.input() || raised_in_tracked(&error, closure) {
            return self.call_error(error, closure, node);
        }
        if reentered {
            error.with_context(node.span, "called here with the same arguments")
        } else {
            error
        }
    }
}
