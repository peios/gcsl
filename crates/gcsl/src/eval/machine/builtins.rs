//! Calls of built-in functions in the machine: their arguments here, and
//! then the built-in applied by a walker
//! ([`Evaluator::apply_called`](super::Evaluator::apply_called)), or, in
//! graph mode, the effect's node.

use crate::check::Builtin;
use crate::ir::{ArgValue, IrId, IrKind, IrRange};

use super::super::calls::BuiltinArguments;
use super::{Frame, Machine, Next, Outcome, fail, give};

/// A call of a built-in running in the machine.
pub(super) struct BuiltinCall {
    /// The built-in.
    builtin: Builtin,
    /// The call node.
    node: IrId,
    /// Its arguments.
    args: IrRange,
    /// The argument evaluated next.
    next: u32,
    /// The arguments evaluated so far, by their parameters' indices.
    arguments: BuiltinArguments,
}

impl Machine<'_> {
    /// Evaluates the call node `id` of `builtin` with the arguments `args`,
    /// as [`Evaluator::call_builtin`](super::Evaluator::call_builtin) does.
    pub(super) fn builtin_call(&mut self, id: IrId, builtin: Builtin, args: IrRange) -> Next {
        self.builtin_next(Box::new(BuiltinCall {
            builtin,
            node: id,
            args,
            next: 0,
            arguments: Default::default(),
        }))
    }

    /// Evaluates the arguments of `call` from its next on, the simple ones
    /// at once, and then applies the built-in.
    fn builtin_next(&mut self, mut call: Box<BuiltinCall>) -> Next {
        loop {
            let ir = &self.running().ir;
            let Some(&arg) = ir.items(call.args).get(call.next as usize) else {
                let BuiltinCall {
                    builtin,
                    node,
                    arguments,
                    ..
                } = *call;
                return give(self.walker(|walker| {
                    walker.apply_called(builtin, arguments, walker.ir.get(node))
                }));
            };
            let IrKind::Arg {
                value: ArgValue::Node(value),
                ..
            } = ir.get(arg).kind
            else {
                return give(self.walker(|walker| {
                    walker
                        .builtin_argument(arg)
                        .map(|_| crate::value::Value::Empty)
                }));
            };
            if !ir.is_simple(value) {
                self.frames.push(Frame::BuiltinArgument(call));
                return Next::Eval(value);
            }
            match self.walker(|walker| walker.value(value)) {
                Ok(value) => {
                    if let Err(next) = self.builtin_argument_done(&mut call, value) {
                        return next;
                    }
                }
                Err(unwind) => return fail(unwind),
            }
        }
    }

    /// Takes up `call` with `result`, the value of its next argument.
    pub(super) fn builtin_argument(&mut self, mut call: Box<BuiltinCall>, result: Outcome) -> Next {
        match result {
            Ok(value) => match self.builtin_argument_done(&mut call, value) {
                Ok(()) => self.builtin_next(call),
                Err(next) => next,
            },
            Err(unwind) => Next::Give(Err(unwind)),
        }
    }

    /// Records `value` as the next argument of `call`, with its span.
    fn builtin_argument_done(
        &mut self,
        call: &mut BuiltinCall,
        value: crate::value::Value,
    ) -> Result<(), Next> {
        let ir = &self.running().ir;
        let found = ir
            .items(call.args)
            .get(call.next as usize)
            .and_then(|&arg| match ir.get(arg).kind {
                IrKind::Arg {
                    param,
                    value: ArgValue::Node(node),
                } => Some((param, ir.get(node).span)),
                _ => None,
            });
        let Some((param, span)) = found else {
            return Err(self.mistyped(call.node, "a built-in's argument is not a node's value"));
        };
        let Some(entry) = call.arguments.get_mut(param as usize) else {
            return Err(self.mistyped(call.node, "an argument for a parameter the built-in lacks"));
        };
        *entry = Some((value, span));
        call.next += 1;
        Ok(())
    }
}
