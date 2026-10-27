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

impl DefaultRun {
    /// Moves its frame `delta` slots up the stack, wrapping (see
    /// [Suspension](crate::eval::machine#suspension)).
    pub(super) fn rebase(&mut self, delta: usize) {
        self.base = self.base.wrapping_add(delta);
    }
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

/// An assertion running in the machine.
pub(super) struct AssertState {
    /// The `Assert` node.
    node: IrId,
    /// Its condition.
    cond: IrId,
    /// Its message.
    message: IrId,
    /// Its condition's operands, if it is a comparison, or else the
    /// condition itself.
    operands: Vec<IrId>,
    /// Their values so far.
    values: Vec<Value>,
}

impl Machine<'_> {
    /// Evaluates the `SecretValue` node `id` of `value`, read deeply if
    /// `deep`, as [`Evaluator::secret_value`](super::Evaluator::secret_value)
    /// does: its value, here, and then its identity recorded.
    pub(super) fn secret_value(&mut self, value: IrId, deep: bool) -> Next {
        self.frames.push(Frame::SecretValue(deep));
        Next::Eval(value)
    }

    /// Takes up a `SecretValue`, read deeply if `deep`, with `result`.
    pub(super) fn secret_value_done(&mut self, deep: bool, result: Outcome) -> Next {
        let result = result.map_err(|unwind| *unwind);
        give(self.walker(|walker| walker.secret_value_of(result, deep)))
    }

    /// Evaluates the `SecretWrap` node `id` of `computed`, revealed if
    /// `revealed`, as [`Evaluator::secret_wrap`](super::Evaluator::secret_wrap)
    /// does: `computed`, here, its operands' identities recorded as it
    /// goes, and then the secret it is.
    pub(super) fn secret_wrap(&mut self, id: IrId, computed: IrId, revealed: bool) -> Next {
        let depth = self.shared.identities.borrow().len();
        self.frames
            .push(Frame::SecretWrap(id, (computed, revealed), depth));
        Next::Eval(computed)
    }

    /// Takes up the `SecretWrap` node `id` with `result`, what its operation
    /// came to.
    pub(super) fn secret_wrap_done(
        &mut self,
        id: IrId,
        (wrapped, depth): ((IrId, bool), usize),
        result: Outcome,
    ) -> Next {
        let result = result.map_err(|unwind| *unwind);
        give(self.walker(|walker| walker.secret_wrap_of(wrapped, depth, result, walker.ir.get(id))))
    }

    /// Evaluates the `Assert` node `id` of `cond`, with the message
    /// `message`, as [`Evaluator::assert`](super::Evaluator::assert) does:
    /// its condition, or a comparison's operands, here, and then, if it
    /// fails, its message.
    pub(super) fn assert(&mut self, id: IrId, (cond, message): (IrId, IrId)) -> Next {
        let operands = match super::super::comparison_operands(self.running().ir.get(cond).kind) {
            Some((lhs, rhs)) => vec![lhs, rhs],
            None => vec![cond],
        };
        self.assert_next(Box::new(AssertState {
            node: id,
            cond,
            message,
            operands,
            values: Vec::new(),
        }))
    }

    /// Evaluates the operands of `state` from the next on, and then decides
    /// it.
    fn assert_next(&mut self, mut state: Box<AssertState>) -> Next {
        loop {
            let Some(&operand) = state.operands.get(state.values.len()) else {
                return self.assert_decide(state);
            };
            if !self.running().ir.is_simple(operand) {
                self.frames.push(Frame::Assert(state));
                return Next::Eval(operand);
            }
            match self.walker(|walker| walker.value(operand)) {
                Ok(value) => state.values.push(value),
                Err(unwind) => return fail(unwind),
            }
        }
    }

    /// Takes up `state` with `result`, the value of its next operand.
    pub(super) fn assert_operand(&mut self, mut state: Box<AssertState>, result: Outcome) -> Next {
        match result {
            Ok(value) => {
                state.values.push(value);
                self.assert_next(state)
            }
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Decides `state`, its operands evaluated: `{}` if it holds, and its
    /// failure, with its message, otherwise.
    fn assert_decide(&mut self, state: Box<AssertState>) -> Next {
        let decided = self.walker(|walker| {
            let condition = walker.ir.get(state.cond);
            match &state.values[..] {
                [lhs, rhs] => walker.compared(condition, (lhs, rhs)),
                [Value::Bool(holds)] => Ok(*holds),
                _ => {
                    super::super::mistyped(condition, "an assertion's condition that is not a Bool")
                }
            }
        });
        match decided {
            Ok(true) => Next::Give(Ok(Value::Empty)),
            Ok(false) => {
                let message = state.message;
                self.frames.push(Frame::AssertMessage(state));
                Next::Eval(message)
            }
            Err(unwind) => fail(unwind),
        }
    }

    /// Ends the failed assertion `state` with `result`, its message.
    pub(super) fn assert_failed(&mut self, state: &AssertState, result: Outcome) -> Next {
        let message = match result {
            Ok(Value::Str(message)) => message,
            Ok(_) => {
                return self.mistyped(state.node, "an assertion's message that is not a String");
            }
            Err(unwind) => return Next::Give(Err(unwind)),
        };
        let shown = match (&state.operands[..], &state.values[..]) {
            (&[lhs, rhs], [left, right]) => Some([(lhs, left.clone()), (rhs, right.clone())]),
            _ => None,
        };
        fail(self.walker(|walker| {
            walker.assertion_failed((state.cond, message), shown, walker.ir.get(state.node))
        }))
    }
}
