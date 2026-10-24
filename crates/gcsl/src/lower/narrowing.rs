//! Lowering type tests, for `is`, `as` and narrowing.

use crate::ast::{ExprId, ExprKind};
use crate::check::Slot;
use crate::diagnostic::InternalError;
use crate::ir::{IrId, IrKind, TestCode, TestId};
use crate::types::ValueTest;
use crate::value::ValueKind;

use super::Lowerer;

impl Lowerer<'_> {
    /// Lowers the `is` or `as` expression `id`, of kind `kind`: an `Is` of
    /// its operand, a `Bool`, writing the value it tests to the slot the
    /// checker gave it if it narrows an `if`'s branches, or an `As` of its
    /// operand, of the kind of its narrowed type; each with the test the
    /// checker planned (see [`Lowerer::lower_test`]). Apart from
    /// `lower_plain`, which is on the recursion path of every expression.
    pub(super) fn lower_type_test(
        &mut self,
        id: ExprId,
        kind: ValueKind,
    ) -> Result<IrKind, InternalError> {
        let expr = *self.ast.get(id);
        match (expr.kind, kind) {
            (ExprKind::Is { expr: operand, .. }, ValueKind::Bool) => {
                let (value, test) = self.lower_test(id, operand)?;
                let slot = self
                    .resolution
                    .test_of(id)
                    .map_or(Slot::NONE, |resolution| resolution.slot);
                Ok(IrKind::Is { value, test, slot })
            }
            (ExprKind::As { expr: operand, .. }, _) => {
                let (value, test) = self.lower_test(id, operand)?;
                Ok(IrKind::As { value, test, kind })
            }
            _ => Err(InternalError {
                message: "an `is` typed as something other than a Bool",
                span: expr.span,
            }),
        }
    }

    /// Lowers the operand `operand` of the `is` or `as` expression `id`,
    /// awaited where the checker marked it, which must not then be of kind
    /// [`ValueKind::Deferred`], and adds the test the checker planned for
    /// it to the IR, with the text of each field name the test looks at;
    /// returns the operand's node and the test.
    pub(super) fn lower_test(
        &mut self,
        id: ExprId,
        operand: ExprId,
    ) -> Result<(IrId, TestId), InternalError> {
        let span = self.ast.get(id).span;
        let Some(resolution) = self.resolution.test_of(id) else {
            return Err(InternalError {
                message: "an `is` or `as` has no test",
                span,
            });
        };
        if self.kind(operand)? == ValueKind::Deferred {
            return Err(InternalError {
                message: "the operand of an `is` or `as` may be a future",
                span,
            });
        }
        let code = TestCode {
            test: resolution.test.clone(),
            subject: resolution.subject.clone(),
            target: resolution.target.clone(),
        };
        let mut pending = vec![&resolution.test];
        while let Some(test) = pending.pop() {
            match test {
                ValueTest::Pass | ValueTest::Kind(_) | ValueTest::Enum(_) => {}
                ValueTest::Record(fields) => {
                    for (name, field) in fields {
                        self.builder
                            .add_name_text(*name, self.ast.names().get(*name));
                        pending.push(field);
                    }
                }
                ValueTest::RecordRest { fields, rest, .. } => {
                    for (name, field) in fields {
                        self.builder
                            .add_name_text(*name, self.ast.names().get(*name));
                        pending.push(field);
                    }
                    pending.push(rest);
                }
                ValueTest::List(element) => pending.push(element),
                ValueTest::Any(alternatives) => pending.extend(alternatives),
            }
        }
        let value = self.lower_expr(operand)?;
        Ok((value, self.builder.push_test(code)))
    }
}
