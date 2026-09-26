//! Lowering: a type-checked [`Ast`] to [`Ir`].
//!
//! Lowering picks the type-specialised IR node for every operator, drops
//! `Paren` nodes, turns integer literals typed `Float` into `FloatConst`
//! directly, and inserts `IntToFloat` for concrete `Int` operands of `/`.
//!
//! Lowering cannot fail on a table produced by [`crate::check::check`]. Any
//! combination the checker should have ruled out is reported as an
//! [`InternalError`] rather than a panic.

use crate::ast::{Ast, BinaryOp, ExprId, ExprKind, UnaryOp};
use crate::check::Types;
use crate::diagnostic::InternalError;
use crate::ir::{Ir, IrBuilder, IrId, IrKind};
use crate::types::Ty;
use crate::value::int_to_float;

/// Lowers `ast`, whose types are `types`, to IR.
///
/// # Errors
///
/// Returns an [`InternalError`] if `types` was not produced by checking
/// `ast`, or is otherwise inconsistent with it: a bug in the caller or the
/// checker, never an error in the user's program.
pub fn lower(ast: &Ast, types: &Types) -> Result<Ir, InternalError> {
    if types.len() != ast.len() {
        return Err(InternalError {
            message: "type table does not belong to this AST",
            span: ast.get(ast.root()).span,
        });
    }
    let mut lowerer = Lowerer {
        ast,
        types,
        // One IR node per AST node, less parentheses, plus any conversions.
        builder: IrBuilder::with_capacity(ast.len()),
    };
    let root = lowerer.lower_expr(ast.root())?;
    Ok(lowerer.builder.finish(root))
}

struct Lowerer<'a> {
    ast: &'a Ast,
    types: &'a Types,
    builder: IrBuilder,
}

impl Lowerer<'_> {
    fn lower_expr(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        let expr = self.ast.get(id);
        let span = expr.span;
        let ty = self.types.get(id);
        let kind = match (expr.kind, ty) {
            (_, Ty::UntypedInt) => {
                return Err(InternalError {
                    message: "an untyped integer survived type checking",
                    span,
                });
            }
            (_, Ty::Error) => {
                return Err(InternalError {
                    message: "a type error survived type checking",
                    span,
                });
            }
            (ExprKind::Paren(inner), _) => return self.lower_operand(inner, ty),
            (ExprKind::Int(value), Ty::Int) => IrKind::IntConst(value),
            (ExprKind::Int(value), Ty::Float) => IrKind::FloatConst(int_to_float(value)),
            (ExprKind::Float(value), Ty::Float) => IrKind::FloatConst(value),
            (
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    operand,
                },
                Ty::Int,
            ) => IrKind::IntNeg(self.lower_operand(operand, Ty::Int)?),
            (
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    operand,
                },
                Ty::Float,
            ) => IrKind::FloatNeg(self.lower_operand(operand, Ty::Float)?),
            (ExprKind::Binary { op, lhs, rhs }, _) => self.lower_binary(op, lhs, rhs, ty, id)?,
            (ExprKind::Float(_), Ty::Int) => {
                return Err(InternalError {
                    message: "float literal typed as Int",
                    span,
                });
            }
        };
        Ok(self.builder.alloc(kind, span))
    }

    fn lower_binary(
        &mut self,
        op: BinaryOp,
        lhs: ExprId,
        rhs: ExprId,
        ty: Ty,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        let (make, operand_ty): (fn(IrId, IrId) -> IrKind, Ty) = match (op, ty) {
            (BinaryOp::Add, Ty::Int) => (IrKind::IntAdd, Ty::Int),
            (BinaryOp::Sub, Ty::Int) => (IrKind::IntSub, Ty::Int),
            (BinaryOp::Mul, Ty::Int) => (IrKind::IntMul, Ty::Int),
            (BinaryOp::FloorDiv, Ty::Int) => (IrKind::IntFloorDiv, Ty::Int),
            (BinaryOp::Rem, Ty::Int) => (IrKind::IntRem, Ty::Int),
            (BinaryOp::Add, Ty::Float) => (IrKind::FloatAdd, Ty::Float),
            (BinaryOp::Sub, Ty::Float) => (IrKind::FloatSub, Ty::Float),
            (BinaryOp::Mul, Ty::Float) => (IrKind::FloatMul, Ty::Float),
            (BinaryOp::Div, Ty::Float) => {
                let lhs = self.lower_as_float(lhs)?;
                let rhs = self.lower_as_float(rhs)?;
                return Ok(IrKind::FloatDiv(lhs, rhs));
            }
            _ => {
                return Err(InternalError {
                    message: "operator has a result type it cannot have",
                    span: self.ast.get(id).span,
                });
            }
        };
        let lhs = self.lower_operand(lhs, operand_ty)?;
        let rhs = self.lower_operand(rhs, operand_ty)?;
        Ok(make(lhs, rhs))
    }

    /// Lowers an operand that must already have type `expected`.
    fn lower_operand(&mut self, id: ExprId, expected: Ty) -> Result<IrId, InternalError> {
        if self.types.get(id) == expected {
            self.lower_expr(id)
        } else {
            Err(InternalError {
                message: "operand type does not match its operator",
                span: self.ast.get(id).span,
            })
        }
    }

    /// Lowers an operand of `/`, converting a concrete `Int` to `Float`.
    fn lower_as_float(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        match self.types.get(id) {
            Ty::Float => self.lower_expr(id),
            Ty::Int => {
                let operand = self.lower_expr(id)?;
                Ok(self
                    .builder
                    .alloc(IrKind::IntToFloat(operand), self.ast.get(id).span))
            }
            Ty::UntypedInt => Err(InternalError {
                message: "an untyped integer survived type checking",
                span: self.ast.get(id).span,
            }),
            Ty::Error => Err(InternalError {
                message: "a type error survived type checking",
                span: self.ast.get(id).span,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AstBuilder, Expr};
    use crate::check::check;
    use crate::lexer::tokenize;
    use crate::parser::parse;
    use crate::source::{SourceFile, Span};

    fn lower_text(text: &str) -> Ir {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let ast = parse(&tokenize(&source).unwrap(), &source).unwrap();
        let types = check(&ast).unwrap();
        lower(&ast, &types).unwrap()
    }

    fn dump(text: &str) -> String {
        lower_text(text).pretty().to_string()
    }

    /// `7 / 2` built by hand with both literals typed as concrete `Int`,
    /// the shape variables will produce.
    fn concrete_int_division(types: Vec<Ty>) -> (Ast, Types) {
        let mut builder = AstBuilder::new();
        let lhs = builder.alloc(Expr {
            kind: ExprKind::Int(7),
            span: Span::new(0, 1),
        });
        let rhs = builder.alloc(Expr {
            kind: ExprKind::Int(2),
            span: Span::new(4, 5),
        });
        let div = builder.alloc(Expr {
            kind: ExprKind::Binary {
                op: BinaryOp::Div,
                lhs,
                rhs,
            },
            span: Span::new(0, 5),
        });
        (builder.finish(div), Types::new(types))
    }

    #[test]
    fn untyped_division_operands_become_float_constants() {
        let ir = lower_text("7 / 2");
        let root = ir.get(ir.root());
        let IrKind::FloatDiv(lhs, rhs) = root.kind else {
            panic!("expected FloatDiv, got {root:?}");
        };
        assert_eq!(ir.get(lhs).kind, IrKind::FloatConst(7.0));
        assert_eq!(ir.get(rhs).kind, IrKind::FloatConst(2.0));
        assert_eq!(
            ir.pretty().to_string(),
            "(float-div @0..5\n  (float 7.0 @0..1)\n  (float 2.0 @4..5))"
        );
    }

    #[test]
    fn concrete_int_division_operands_are_converted() {
        let (ast, types) = concrete_int_division(vec![Ty::Int, Ty::Int, Ty::Float]);
        let ir = lower(&ast, &types).unwrap();
        assert_eq!(
            ir.pretty().to_string(),
            "(float-div @0..5\n  (int-to-float @0..1\n    (int 7 @0..1))\n  (int-to-float @4..5\n    (int 2 @4..5)))"
        );
    }

    #[test]
    fn concrete_int_from_floor_division_is_converted() {
        assert_eq!(
            dump("(7 // 2) / 2"),
            "(float-div @0..12\n  (int-to-float @0..8\n    (int-floor-div @1..7\n      (int 7 @1..2)\n      (int 2 @6..7)))\n  (float 2.0 @11..12))"
        );
    }

    #[test]
    fn parentheses_are_dropped() {
        assert_eq!(dump("((1))"), "(int 1 @2..3)");
    }

    #[test]
    fn operators_are_specialised_by_type() {
        assert_eq!(
            dump("-(1 + 2 - 3 * 4 // 5 % 6)"),
            "(int-neg @0..25\n  (int-sub @2..24\n    (int-add @2..7\n      (int 1 @2..3)\n      (int 2 @6..7))\n    (int-rem @10..24\n      (int-floor-div @10..20\n        (int-mul @10..15\n          (int 3 @10..11)\n          (int 4 @14..15))\n        (int 5 @19..20))\n      (int 6 @23..24))))"
        );
        assert_eq!(
            dump("-(1.0 + 2 - 3 * 4.0)"),
            "(float-neg @0..20\n  (float-sub @2..19\n    (float-add @2..9\n      (float 1.0 @2..5)\n      (float 2.0 @8..9))\n    (float-mul @12..19\n      (float 3.0 @12..13)\n      (float 4.0 @16..19))))"
        );
    }

    #[test]
    fn large_untyped_literal_rounds_to_nearest_float() {
        let ir = lower_text("9007199254740993 / 1");
        let IrKind::FloatDiv(lhs, _) = ir.get(ir.root()).kind else {
            panic!("expected FloatDiv");
        };
        assert_eq!(
            ir.get(lhs).kind,
            IrKind::FloatConst(9_007_199_254_740_992.0)
        );
    }

    #[test]
    fn untyped_entry_is_an_internal_error() {
        let (ast, types) = concrete_int_division(vec![Ty::UntypedInt, Ty::Int, Ty::Float]);
        let error = lower(&ast, &types).unwrap_err();
        assert_eq!(error.message, "an untyped integer survived type checking");
        assert_eq!(error.span, Span::new(0, 1));
    }

    #[test]
    fn error_entry_is_an_internal_error() {
        let (ast, types) = concrete_int_division(vec![Ty::Int, Ty::Error, Ty::Float]);
        let error = lower(&ast, &types).unwrap_err();
        assert_eq!(error.message, "a type error survived type checking");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn impossible_result_type_is_an_internal_error() {
        let (ast, types) = concrete_int_division(vec![Ty::Int, Ty::Int, Ty::Int]);
        let error = lower(&ast, &types).unwrap_err();
        assert_eq!(error.message, "operator has a result type it cannot have");
        assert_eq!(error.span, Span::new(0, 5));
    }

    #[test]
    fn mismatched_operand_type_is_an_internal_error() {
        let source = SourceFile::new("test.gcsl", "1 + 2").unwrap();
        let ast = parse(&tokenize(&source).unwrap(), &source).unwrap();
        let types = Types::new(vec![Ty::Int, Ty::Float, Ty::Int]);
        let error = lower(&ast, &types).unwrap_err();
        assert_eq!(error.message, "operand type does not match its operator");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn table_of_the_wrong_size_is_an_internal_error() {
        let (ast, _) = concrete_int_division(Vec::new());
        let error = lower(&ast, &Types::new(vec![Ty::Int])).unwrap_err();
        assert_eq!(error.message, "type table does not belong to this AST");
    }
}
