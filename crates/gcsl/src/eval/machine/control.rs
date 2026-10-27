//! Context and failure in the machine: `with`, a read of a context, whose
//! default runs as a call, and `recover`, as the walkers evaluate them (see
//! [Context](crate::eval::context#context) and
//! [Failure](crate::eval::failure#failure)).

use std::rc::Rc;

use crate::diagnostic::{Diagnostic, Mark as ErrorMark};
use crate::ir::{IrId, IrKind, IrRange};
use crate::names::Name;
use crate::value::{Closure, ContextFrame, Value};
use crate::{CALL_LEVELS, MAX_DEPTH};

use super::super::{Callee, Unwind, nested_too_deeply};
use super::{Activation, Frame, Machine, Next, Outcome, fail, give};

/// A `with` running in the machine.
pub(super) struct WithState {
    /// The `With` node.
    node: IrId,
    /// Its bindings and then its body.
    parts: IrRange,
    /// The bindings' values so far, with their keys.
    bound: Vec<(Name, Value)>,
}

/// A context's default running in the machine, as a call.
pub(super) struct DefaultRun {
    /// The read.
    node: IrId,
    /// The default's closure.
    closure: Rc<Closure>,
    /// Where its frame starts on the value stack.
    base: usize,
}

impl Machine<'_> {
    /// Evaluates the `With` node `id` of `parts`, as
    /// [`Evaluator::with`](super::Evaluator::with) does: each binding's
    /// value, in order, in the running context, and then the body in a new
    /// frame of the context, the running one restored after.
    pub(super) fn with(&mut self, id: IrId, parts: IrRange) -> Next {
        self.with_next(Box::new(WithState {
            node: id,
            parts,
            bound: Vec::new(),
        }))
    }

    /// Evaluates the bindings of `state` from the next on, and then its
    /// body.
    fn with_next(&mut self, mut state: Box<WithState>) -> Next {
        loop {
            let ir = &self.running().ir;
            let parts = ir.items(state.parts);
            let Some((&body, binds)) = parts.split_last() else {
                return self.mistyped(state.node, "a `with` with no body");
            };
            let Some(&item) = binds.get(state.bound.len()) else {
                let frame = ContextFrame {
                    id: self.memo.new_context_frame(),
                    binds: std::mem::take(&mut state.bound).into_boxed_slice(),
                    outer: self.shared.context.borrow().clone(),
                };
                let outer = self.shared.context.replace(Some(Rc::new(frame)));
                self.frames.push(Frame::WithBody(outer));
                return Next::Eval(body);
            };
            let IrKind::ContextBind { key, value } = ir.get(item).kind else {
                return self.mistyped(state.node, "a `with` binding that is none");
            };
            if !ir.is_simple(value) {
                self.frames.push(Frame::WithBinding(state, key));
                return Next::Eval(value);
            }
            match self.walker(|walker| walker.value(value)) {
                Ok(value) => state.bound.push((key, value)),
                Err(unwind) => return fail(unwind),
            }
        }
    }

    /// Takes up `state` with `result`, the value of its binding of `key`.
    pub(super) fn with_binding(
        &mut self,
        mut state: Box<WithState>,
        key: Name,
        result: Outcome,
    ) -> Next {
        match result {
            Ok(value) => {
                state.bound.push((key, value));
                self.with_next(state)
            }
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Ends a `with` whose body came to `result`, restoring `outer`, the
    /// context it was evaluated in.
    pub(super) fn with_done(&self, outer: Option<Rc<ContextFrame>>, result: Outcome) -> Next {
        self.shared.context.replace(outer);
        Next::Give(result)
    }

    /// Evaluates `body`, the body of a `recover` whose fallback is
    /// `fallback`.
    pub(super) fn recover(&mut self, body: IrId, fallback: IrId) -> Next {
        self.frames.push(Frame::Recover(fallback));
        Next::Eval(body)
    }

    /// Takes up a `recover` whose fallback is `fallback` with `result`,
    /// what its body came to, as
    /// [`Evaluator::recover`](super::Evaluator::recover) does: a failure
    /// with no mark runs the fallback, with its message the innermost a
    /// `Failure` reads.
    pub(super) fn recovered(&mut self, fallback: IrId, result: Outcome) -> Next {
        match result {
            Err(unwind) => match *unwind {
                Unwind::Error(error) if error.mark == ErrorMark::NONE => {
                    self.shared
                        .failures
                        .borrow_mut()
                        .push(Rc::from(error.message));
                    self.frames.push(Frame::Fallback);
                    Next::Eval(fallback)
                }
                unwind => fail(unwind),
            },
            ok => Next::Give(ok),
        }
    }

    /// Ends a `recover`'s fallback, which came to `result`.
    pub(super) fn fallback_done(&self, result: Outcome) -> Next {
        self.shared.failures.borrow_mut().pop();
        Next::Give(result)
    }

    /// Evaluates the context read `id` of the key `key`, whose default is
    /// `default`, at depth `depth`, as
    /// [`Evaluator::context_read`](super::Evaluator::context_read) does:
    /// the value of its nearest binding, or else its default's function
    /// called with no arguments, here, or the error of a context with
    /// neither.
    pub(super) fn context_read(
        &mut self,
        id: IrId,
        key: Name,
        (default, depth): (IrId, u16),
    ) -> Next {
        let context = self.shared.context.borrow().clone();
        self.memo.depend_context(key, context.as_ref());
        self.walker(|walker| walker.note_step_read(key, context.as_ref()));
        if let Some((_, value)) = ContextFrame::find(context.as_ref(), key) {
            return Next::Give(Ok(value.clone()));
        }
        let ir = &self.running().ir;
        if ir.get(default).kind == IrKind::Empty {
            let name = ir
                .name_text(key)
                .and_then(crate::names::key_context)
                .unwrap_or_default();
            let error = Diagnostic::error(
                format!("context `{name}` has no value here"),
                ir.get(id).span,
            )
            .with_note("no `with` sets it, and it declares no default");
            return fail(error);
        }
        let closure = match self.walker(|walker| walker.function(default)) {
            Ok(Callee::Closure(closure)) => closure,
            Ok(_) => return self.mistyped(id, "a context's default that is no closure"),
            Err(unwind) => return fail(unwind),
        };
        let Some(code) = closure.code.find_function(closure.function) else {
            return self.mistyped(id, "a closure of a function its IR lacks");
        };
        let (height, frame_size, body) =
            (code.height as usize, code.frame_size as usize, code.body);
        let entering = self.running().levels + usize::from(depth) + CALL_LEVELS;
        if entering + height > MAX_DEPTH {
            return fail(nested_too_deeply(self.running().ir.get(id)));
        }
        let base = self.stack.len();
        self.stack.resize(base + frame_size, Value::Empty);
        let caller = self.walker(|walker| {
            let passed = walker.callee_caller();
            walker.profile_begin((&closure.code, closure.function, code), true);
            passed
        });
        self.activations.push(Activation {
            ir: Rc::clone(&closure.code),
            base,
            callee: Some(Rc::clone(&closure)),
            owned: None,
            caller,
            levels: entering,
        });
        self.frames.push(Frame::Default(Box::new(DefaultRun {
            node: id,
            closure,
            base,
        })));
        Next::Eval(body)
    }

    /// Ends the default `run` of a context read, whose body came to
    /// `result`, as a call ends.
    pub(super) fn default_done(&mut self, run: &DefaultRun, result: Outcome) -> Next {
        self.activations.pop();
        self.walker(|walker| walker.profile_end());
        self.stack.truncate(run.base);
        let span = self.running().ir.get(run.node).span;
        let outcome = result.or_else(|unwind| unwind.returned(span));
        give(outcome.map_err(|error| {
            self.walker(|walker| {
                walker
                    .call_error(error, &run.closure, walker.ir.get(run.node))
                    .into()
            })
        }))
    }
}
