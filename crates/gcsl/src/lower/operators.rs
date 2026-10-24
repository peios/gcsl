//! Lowering constants and operators: unary and binary operators, quantities,
//! comparisons and interpolation.

use crate::ast::{BinaryOp, ExprId, PartRange, StrPart, UnaryOp};
use crate::check::Constant;
use crate::diagnostic::InternalError;
use crate::ir::{Cmp, IrId, IrKind, Operands, ShapedOperands};
use crate::names::Name;
use crate::source::Span;
use crate::types::TyId;
use crate::value::ValueKind;

use super::{Lowerer, comparison, integer_operator};

impl Lowerer<'_> {
    /// Lowers `constant`, a parameter's default or a field's (see the checker's
    /// "Field defaults"), to nodes spanning `span` that build it: an
    /// `IntConst`, `FloatConst`, `BoolConst` or `StrConst`, a `Quantity` of an
    /// `IntConst`, a `MakeList` of its elements' nodes, a `MakeRecord` of its
    /// fields', in the order they are written, with the layout of their names,
    /// shared as a record block's is, or `Empty` for `{}`.
    ///
    /// Recurses once per level of list and record nesting.
    pub(super) fn lower_constant(&mut self, constant: &Constant, span: Span) -> IrId {
        let kind = match constant {
            Constant::Int(value) => IrKind::IntConst(*value),
            Constant::Float(bits) => IrKind::FloatConst(f64::from_bits(*bits)),
            &Constant::Quantity(quantity, amount) => {
                IrKind::Quantity(quantity, self.builder.alloc(IrKind::IntConst(amount), span))
            }
            Constant::Bool(value) => IrKind::BoolConst(*value),
            Constant::Str(text) => IrKind::StrConst(self.builder.push_string(text)),
            Constant::Empty => IrKind::Empty,
            Constant::List(elements) => {
                let items: Vec<IrId> = elements
                    .iter()
                    .map(|element| self.lower_constant(element, span))
                    .collect();
                IrKind::MakeList {
                    items: self.builder.push_item_list(&items),
                    spreads: false,
                }
            }
            Constant::Record(fields) => {
                let values: Vec<IrId> = fields
                    .iter()
                    .map(|(_, value)| self.lower_constant(value, span))
                    .collect();
                let names: Vec<Name> = fields.iter().map(|&(name, _)| name).collect();
                IrKind::MakeRecord {
                    layout: self.layout_of(&names),
                    values: self.builder.push_item_list(&values),
                }
            }
        };
        self.builder.alloc(kind, span)
    }

    /// Lowers the prefix operator `op`, of kind `kind` and spanning `span`,
    /// applied to `operand`.
    pub(super) fn lower_unary(
        &mut self,
        op: UnaryOp,
        operand: ExprId,
        kind: ValueKind,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        Ok(match (op, kind) {
            (UnaryOp::Neg, ValueKind::Int) => {
                IrKind::IntNeg(self.lower_operand(operand, ValueKind::Int)?)
            }
            (UnaryOp::Neg, ValueKind::Float) => {
                IrKind::FloatNeg(self.lower_operand(operand, ValueKind::Float)?)
            }
            (UnaryOp::Neg, ValueKind::Quantity(quantity)) => {
                let negated = IrKind::IntNeg(self.lower_operand(operand, kind)?);
                IrKind::Quantity(quantity, self.builder.alloc(negated, span))
            }
            (UnaryOp::Not, ValueKind::Bool) => {
                IrKind::Not(self.lower_operand(operand, ValueKind::Bool)?)
            }
            _ => {
                return Err(InternalError {
                    message: "prefix operator has a result type it cannot have",
                    span,
                });
            }
        })
    }

    /// Lowers the parts of an interpolated string spanning `span` to an
    /// `Interp` whose items are a `StrConst` for each literal piece and the
    /// lowered expressions, in order.
    pub(super) fn lower_interpolation(
        &mut self,
        id: ExprId,
        parts: PartRange,
        span: Span,
    ) -> Result<IrKind, InternalError> {
        let base = self.item_stack.len();
        // An interpolation of a secret derives its text from its literal
        // parts too, so each but an empty one is one of its operands (see
        // the checker's "Secrets").
        let secret = self.resolution.secret_op(id).is_some();
        for &part in self.ast.parts(parts) {
            let item = match part {
                StrPart::Lit(text) => {
                    let text = self.ast.strings().get(text);
                    let empty = text.is_empty();
                    let text = self.builder.push_string(text);
                    let literal = self.builder.alloc(IrKind::StrConst(text), span);
                    if secret && !empty {
                        let operand = IrKind::SecretValue {
                            value: literal,
                            kind: ValueKind::String,
                            deep: false,
                        };
                        self.builder.alloc(operand, span)
                    } else {
                        literal
                    }
                }
                StrPart::Expr(expr) => self.lower_expr(expr)?,
            };
            self.item_stack.push(item);
        }
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::Interp(items))
    }

    /// Lowers the binary operator `op`, whose symbol starts at `op_start`,
    /// of kind `kind`, the expression `id`.
    pub(super) fn lower_binary(
        &mut self,
        op: BinaryOp,
        op_start: u32,
        (lhs, rhs): (ExprId, ExprId),
        kind: ValueKind,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        if let Some(cmp) = comparison(op) {
            return self.lower_comparison((op, cmp), op_start, (lhs, rhs), kind, id);
        }
        if let Some(make) = integer_operator(op) {
            let lhs_kind = self.kind(lhs)?;
            if matches!(kind, ValueKind::Quantity(_)) || matches!(lhs_kind, ValueKind::Quantity(_))
            {
                return self.lower_quantity_operator(make, op_start, (lhs, rhs), kind, id);
            }
        }
        let (make, operand_kind): (fn(Operands) -> IrKind, ValueKind) = match (op, kind) {
            (BinaryOp::Add, ValueKind::String) => (IrKind::StrConcat, ValueKind::String),
            (BinaryOp::And, ValueKind::Bool) => (IrKind::And, ValueKind::Bool),
            (BinaryOp::Or, ValueKind::Bool) => (IrKind::Or, ValueKind::Bool),
            (BinaryOp::Add, ValueKind::Int) => (IrKind::IntAdd, ValueKind::Int),
            (BinaryOp::Sub, ValueKind::Int) => (IrKind::IntSub, ValueKind::Int),
            (BinaryOp::Mul, ValueKind::Int) => (IrKind::IntMul, ValueKind::Int),
            (BinaryOp::FloorDiv, ValueKind::Int) => (IrKind::IntFloorDiv, ValueKind::Int),
            (BinaryOp::Rem, ValueKind::Int) => (IrKind::IntRem, ValueKind::Int),
            (BinaryOp::Range, ValueKind::List) => {
                (|operands| IrKind::Range(false, operands), ValueKind::Int)
            }
            (BinaryOp::RangeInclusive, ValueKind::List) => {
                (|operands| IrKind::Range(true, operands), ValueKind::Int)
            }
            (BinaryOp::Add, ValueKind::Float) => (IrKind::FloatAdd, ValueKind::Float),
            (BinaryOp::Sub, ValueKind::Float) => (IrKind::FloatSub, ValueKind::Float),
            (BinaryOp::Mul, ValueKind::Float) => (IrKind::FloatMul, ValueKind::Float),
            (BinaryOp::Div, ValueKind::Float) => {
                let lhs = self.lower_as_float(lhs)?;
                let rhs = self.lower_as_float(rhs)?;
                return Ok(IrKind::FloatDiv(Operands { lhs, rhs, op_start }));
            }
            _ => {
                return Err(InternalError {
                    message: "operator has a result type it cannot have",
                    span: self.ast.get(id).span,
                });
            }
        };
        let lhs = self.lower_operand(lhs, operand_kind)?;
        let rhs = self.lower_operand(rhs, operand_kind)?;
        Ok(make(Operands { lhs, rhs, op_start }))
    }

    /// Lowers the integer operator that `make` makes, whose symbol starts at
    /// `op_start`, of kind `kind`, the expression `id`, with an operand that
    /// is a quantity or a quantity's result (see the checker's
    /// "Quantities"): the `Int` node of each operand lowered as its own kind,
    /// which the `Int` walker takes the amount of, and, for a quantity's
    /// result, that node given its unit, a `Quantity` node.
    pub(super) fn lower_quantity_operator(
        &mut self,
        make: fn(Operands) -> IrKind,
        op_start: u32,
        (lhs, rhs): (ExprId, ExprId),
        kind: ValueKind,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        let (lhs_kind, rhs_kind) = (self.kind(lhs)?, self.kind(rhs)?);
        let lhs = self.lower_operand(lhs, lhs_kind)?;
        let rhs = self.lower_operand(rhs, rhs_kind)?;
        let operated = make(Operands { lhs, rhs, op_start });
        Ok(match kind {
            ValueKind::Quantity(quantity) => {
                let span = self.ast.get(id).span;
                IrKind::Quantity(quantity, self.builder.alloc(operated, span))
            }
            _ => operated,
        })
    }

    /// Lowers the comparison `op`, which tests `cmp` on every kind but
    /// records, of kind `kind`, to the node for its operands' kind, which
    /// the checker has made the same on both sides.
    pub(super) fn lower_comparison(
        &mut self,
        (op, cmp): (BinaryOp, Cmp),
        op_start: u32,
        (lhs, rhs): (ExprId, ExprId),
        kind: ValueKind,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        let span = self.ast.get(id).span;
        if kind != ValueKind::Bool {
            return Err(InternalError {
                message: "operator has a result type it cannot have",
                span,
            });
        }
        let operand_kind = self.kind(lhs)?;
        // Typed equality on records and lists compares over the operands'
        // type, once awaited.
        let shape = match (operand_kind, op) {
            (ValueKind::Record | ValueKind::List, BinaryOp::Eq | BinaryOp::Ne) => {
                Some(self.shape(self.used_type(lhs), span)?)
            }
            _ => None,
        };
        let lhs = self.lower_operand(lhs, operand_kind)?;
        let rhs = self.lower_operand(rhs, operand_kind)?;
        let operands = Operands { lhs, rhs, op_start };
        Ok(match (operand_kind, op, shape) {
            (ValueKind::Int | ValueKind::Quantity(_), ..) => IrKind::IntCmp(cmp, operands),
            (ValueKind::Float, ..) => IrKind::FloatCmp(cmp, operands),
            (ValueKind::String, ..) => IrKind::StrCmp(cmp, operands),
            (ValueKind::Bool, BinaryOp::Eq | BinaryOp::Same, _) => IrKind::BoolEq(true, operands),
            (ValueKind::Bool, BinaryOp::Ne | BinaryOp::NotSame, _) => {
                IrKind::BoolEq(false, operands)
            }
            (ValueKind::Record, BinaryOp::Eq, Some(shape)) => {
                IrKind::RecordEq(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::Record, BinaryOp::Ne, Some(shape)) => {
                IrKind::RecordNe(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::Record, BinaryOp::Same, _) => IrKind::RecordSame(operands),
            (ValueKind::Record, BinaryOp::NotSame, _) => IrKind::RecordNotSame(operands),
            (ValueKind::List, BinaryOp::Eq, Some(shape)) => {
                IrKind::ListEq(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::List, BinaryOp::Ne, Some(shape)) => {
                IrKind::ListNe(ShapedOperands { shape, lhs, rhs })
            }
            (ValueKind::List, BinaryOp::Same, _) => IrKind::ListSame(operands),
            (ValueKind::List, BinaryOp::NotSame, _) => IrKind::ListNotSame(operands),
            // The checker lets `==` compare no union and no function: two
            // of one enum, of a variant and payload each, or two variants,
            // which compare as wholes (see the checker's "Enums").
            (ValueKind::Mixed, BinaryOp::Same, _)
            | (ValueKind::Mixed | ValueKind::Fn, BinaryOp::Eq, _) => IrKind::ValueSame(operands),
            (ValueKind::Mixed, BinaryOp::NotSame, _)
            | (ValueKind::Mixed | ValueKind::Fn, BinaryOp::Ne, _) => IrKind::ValueNotSame(operands),
            (
                ValueKind::Bool
                | ValueKind::Record
                | ValueKind::List
                | ValueKind::Fn
                | ValueKind::Deferred
                | ValueKind::Mixed
                | ValueKind::Never,
                ..,
            ) => {
                return Err(InternalError {
                    message: "comparison of a type it cannot compare",
                    span,
                });
            }
        })
    }

    /// Lowers an operand that must already be of kind `expected`, where it
    /// is used. Where a value of kind `Deferred` is expected, a value of
    /// any kind is accepted, since a present value may be where a future
    /// may; where one of kind `Mixed` is, a union's, a value of any kind but
    /// `Deferred` is, since a value of any member's kind may be where the
    /// union may, but a future may not.
    pub(super) fn lower_operand(
        &mut self,
        id: ExprId,
        expected: ValueKind,
    ) -> Result<IrId, InternalError> {
        let kind = self.kind(id)?;
        // A node of kind `Never` never yields a value, so it may stand where
        // one of any kind is expected: every walker passes on what it does.
        let accepted = match expected {
            _ if kind == ValueKind::Never => true,
            ValueKind::Deferred => true,
            ValueKind::Mixed => kind != ValueKind::Deferred,
            _ => kind == expected,
        };
        if accepted {
            self.lower_expr(id)
        } else {
            Err(InternalError {
                message: "operand type does not match its operator",
                span: self.ast.get(id).span,
            })
        }
    }

    /// Lowers an operand of `/`, converting a concrete `Int` to `Float`.
    pub(super) fn lower_as_float(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        let span = self.ast.get(id).span;
        match self.used_type(id) {
            TyId::FLOAT => self.lower_expr(id),
            TyId::INT => {
                let operand = self.lower_expr(id)?;
                Ok(self.builder.alloc(IrKind::IntToFloat(operand), span))
            }
            TyId::UNTYPED_INT => Err(InternalError {
                message: "an untyped integer survived type checking",
                span,
            }),
            TyId::ERROR => Err(InternalError {
                message: "a type error survived type checking",
                span,
            }),
            _ => Err(InternalError {
                message: "operand type does not match its operator",
                span,
            }),
        }
    }
}
