//! Lowering: a checked [`Ast`] to [`Ir`].
//!
//! Lowering picks the type-specialised IR node for every operator, drops
//! `Paren` and `Ascribe` nodes (the checker has already coerced whatever an
//! ascription affects), turns integer literals typed `Float` into
//! `FloatConst` directly, and inserts `IntToFloat` for concrete `Int`
//! operands of `/`. Variables become `Load` and `Store` nodes on the slots
//! the checker assigned. Every block, including the file's body, becomes a
//! `Block` whose items are its declarations and non-final expression
//! statements, and whose result is its `= EXPR` or final expression
//! statement, or an explicit `Unit` node if it has neither.
//!
//! Lowering cannot fail on tables produced by [`crate::check::check`]. Any
//! combination the checker should have ruled out is reported as an
//! [`InternalError`] rather than a panic.

use crate::ast::{Ast, BinaryOp, ExprId, ExprKind, ItemKind, ItemRange, UnaryOp};
use crate::check::{Checked, Resolution, Slot, Types};
use crate::diagnostic::InternalError;
use crate::ir::{Ir, IrBuilder, IrId, IrKind, Operands};
use crate::source::Span;
use crate::types::Ty;
use crate::value::int_to_float;

/// Lowers `ast`, whose types and resolution are `checked`, to IR.
///
/// # Errors
///
/// Returns an [`InternalError`] if `checked` was not produced by checking
/// `ast`, or is otherwise inconsistent with it: a bug in the caller or the
/// checker, never an error in the user's program.
pub fn lower(ast: &Ast, checked: &Checked) -> Result<Ir, InternalError> {
    let Checked { types, resolution } = checked;
    let root_span = ast.root_span();
    if types.len() != ast.len() {
        return Err(InternalError {
            message: "type table does not belong to this AST",
            span: root_span,
        });
    }
    if resolution.expr_len() != ast.len() || resolution.item_len() != ast.item_count() {
        return Err(InternalError {
            message: "resolution does not belong to this AST",
            span: root_span,
        });
    }
    let mut lowerer = Lowerer {
        ast,
        types,
        resolution,
        // One IR node per AST node, less parentheses, plus any conversions,
        // stores and units.
        builder: IrBuilder::with_capacity(ast.len()),
        item_stack: Vec::new(),
    };
    let root = ast.root();
    let kind = lowerer.lower_body(root, root_span, lowerer.body_ty(root))?;
    let root = lowerer.builder.alloc(kind, root_span);
    Ok(lowerer.builder.finish(
        root,
        resolution.frame_size(),
        resolution.slot_names().to_vec(),
    ))
}

struct Lowerer<'a> {
    ast: &'a Ast,
    types: &'a Types,
    resolution: &'a Resolution,
    builder: IrBuilder,
    /// The items of the blocks being lowered, innermost last, moved to the
    /// builder's pool as each block ends, as the parser does for the AST.
    item_stack: Vec<IrId>,
}

impl Lowerer<'_> {
    /// The type of a body: its result's type, or `Unit` if it has none.
    fn body_ty(&self, body: ItemRange) -> Ty {
        match self
            .ast
            .items(body)
            .last()
            .map(|&id| self.ast.item(id).kind)
        {
            Some(ItemKind::Result(value) | ItemKind::Expr(value)) => self.types.get(value),
            Some(ItemKind::Let { .. } | ItemKind::Rebind { .. }) | None => Ty::Unit,
        }
    }

    /// Lowers a body whose checked type is `ty` to a `Block`, which the
    /// caller allocates.
    fn lower_body(&mut self, body: ItemRange, span: Span, ty: Ty) -> Result<IrKind, InternalError> {
        let items = self.ast.items(body);
        let base = self.item_stack.len();
        let mut result = None;
        for (index, &id) in items.iter().enumerate() {
            let is_last = index + 1 == items.len();
            let item = self.ast.item(id);
            match item.kind {
                ItemKind::Let { value, .. } | ItemKind::Rebind { value, .. } => {
                    let slot = self.resolution.item_slot(id);
                    if slot == Slot::NONE {
                        return Err(InternalError {
                            message: "a declaration has no slot",
                            span: item.span,
                        });
                    }
                    let value = self.lower_expr(value)?;
                    let store = self.builder.alloc(IrKind::Store(slot, value), item.span);
                    self.item_stack.push(store);
                }
                ItemKind::Result(value) | ItemKind::Expr(value) if is_last => {
                    result = Some(self.lower_operand(value, ty)?);
                }
                ItemKind::Result(_) => {
                    return Err(InternalError {
                        message: "a result before the end of its block survived checking",
                        span: item.span,
                    });
                }
                ItemKind::Expr(value) => {
                    // Evaluated for effect only, so it must be Unit.
                    let statement = self.lower_operand(value, Ty::Unit)?;
                    self.item_stack.push(statement);
                }
            }
        }
        let result = match result {
            Some(result) => result,
            None if ty == Ty::Unit => self.builder.alloc(IrKind::Unit, span),
            None => {
                return Err(InternalError {
                    message: "a block without a result is not typed Unit",
                    span,
                });
            }
        };
        let items = self.builder.push_item_list(&self.item_stack[base..]);
        self.item_stack.truncate(base);
        Ok(IrKind::Block { items, result })
    }

    fn lower_expr(&mut self, id: ExprId) -> Result<IrId, InternalError> {
        let expr = *self.ast.get(id);
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
            (ExprKind::Paren(inner) | ExprKind::Ascribe { expr: inner, .. }, _) => {
                return self.lower_operand(inner, ty);
            }
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
            (
                ExprKind::Binary {
                    op,
                    op_start,
                    lhs,
                    rhs,
                },
                _,
            ) => self.lower_binary(op, op_start, lhs, rhs, ty, id)?,
            (ExprKind::Name(_), _) => {
                let slot = self.resolution.expr_slot(id);
                if slot == Slot::NONE {
                    return Err(InternalError {
                        message: "a variable use has no slot",
                        span,
                    });
                }
                IrKind::Load(slot, ty)
            }
            (ExprKind::Block(body), _) => self.lower_body(body, span, ty)?,
            (ExprKind::Float(_), Ty::Int) => {
                return Err(InternalError {
                    message: "float literal typed as Int",
                    span,
                });
            }
            (ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Unary { .. }, Ty::Unit) => {
                return Err(InternalError {
                    message: "numeric expression typed as Unit",
                    span,
                });
            }
        };
        Ok(self.builder.alloc(kind, span))
    }

    fn lower_binary(
        &mut self,
        op: BinaryOp,
        op_start: u32,
        lhs: ExprId,
        rhs: ExprId,
        ty: Ty,
        id: ExprId,
    ) -> Result<IrKind, InternalError> {
        let (make, operand_ty): (fn(Operands) -> IrKind, Ty) = match (op, ty) {
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
                return Ok(IrKind::FloatDiv(Operands { lhs, rhs, op_start }));
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
        Ok(make(Operands { lhs, rhs, op_start }))
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
            Ty::Unit => Err(InternalError {
                message: "operand type does not match its operator",
                span: self.ast.get(id).span,
            }),
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
    use crate::ast::{AstBuilder, Expr, Item};
    use crate::check::check;
    use crate::lexer::tokenize;
    use crate::names::Names;
    use crate::parser::parse;
    use crate::source::SourceFile;

    fn parse_text(text: &str) -> Ast {
        let source = SourceFile::new("test.gcsl", text).unwrap();
        let (tokens, names) = tokenize(&source).unwrap();
        parse(&tokens, names, &source).unwrap()
    }

    fn lower_text(text: &str) -> Ir {
        let ast = parse_text(text);
        lower(&ast, &check(&ast).unwrap()).unwrap()
    }

    fn dump(text: &str) -> String {
        let ast = parse_text(text);
        let ir = lower(&ast, &check(&ast).unwrap()).unwrap();
        ir.pretty(ast.names()).to_string()
    }

    /// The node a file of one expression lowers to: its root block's result.
    fn result_of(ir: &Ir) -> IrId {
        let IrKind::Block { items, result } = ir.get(ir.root()).kind else {
            panic!("the root is not a block: {:?}", ir.get(ir.root()));
        };
        assert!(ir.items(items).is_empty());
        result
    }

    /// `7 / 2` built by hand with both literals given the types in `types`,
    /// e.g. both concrete `Int`, the shape variables produce.
    fn concrete_int_division(types: Vec<Ty>) -> (Ast, Checked) {
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
                op_start: 2,
                lhs,
                rhs,
            },
            span: Span::new(0, 5),
        });
        let item = builder.alloc_item(Item {
            kind: ItemKind::Expr(div),
            span: Span::new(0, 5),
        });
        let root = builder.push_item_list(&[item]);
        let ast = builder.finish(root, Names::new());
        let checked = Checked {
            types: Types::new(types),
            resolution: Resolution::without_variables(3, 1),
        };
        (ast, checked)
    }

    #[test]
    fn untyped_division_operands_become_float_constants() {
        let ir = lower_text("7 / 2");
        let root = ir.get(result_of(&ir));
        let IrKind::FloatDiv(operands) = root.kind else {
            panic!("expected FloatDiv, got {root:?}");
        };
        assert_eq!(ir.get(operands.lhs).kind, IrKind::FloatConst(7.0));
        assert_eq!(ir.get(operands.rhs).kind, IrKind::FloatConst(2.0));
        assert_eq!(operands.op_start, 2);
        assert_eq!(
            dump("7 / 2"),
            "(block @0..5\n  (float-div @0..5\n    (float 7.0 @0..1)\n    (float 2.0 @4..5)))"
        );
    }

    #[test]
    fn concrete_int_division_operands_are_converted() {
        let (ast, checked) = concrete_int_division(vec![Ty::Int, Ty::Int, Ty::Float]);
        let ir = lower(&ast, &checked).unwrap();
        assert_eq!(
            ir.pretty(ast.names()).to_string(),
            "(block @0..5\n  (float-div @0..5\n    (int-to-float @0..1\n      (int 7 @0..1))\n    (int-to-float @4..5\n      (int 2 @4..5))))"
        );
    }

    #[test]
    fn concrete_int_from_floor_division_is_converted() {
        assert_eq!(
            dump("(7 // 2) / 2"),
            "(block @0..12\n  (float-div @0..12\n    (int-to-float @0..8\n      (int-floor-div @1..7\n        (int 7 @1..2)\n        (int 2 @6..7)))\n    (float 2.0 @11..12)))"
        );
    }

    #[test]
    fn parentheses_and_ascriptions_are_dropped() {
        // The root block covers the file's items, parentheses included.
        assert_eq!(dump("((1))"), "(block @0..5\n  (int 1 @2..3))");
        assert_eq!(dump("(1 : Float)"), "(block @0..11\n  (float 1.0 @1..2))");
    }

    #[test]
    fn operators_are_specialised_by_type() {
        let ir = lower_text("-(1 + 2 - 3 * 4 // 5 % 6)");
        let result = result_of(&ir);
        assert_eq!(ir.ty_of(result), Ty::Int);
        assert_eq!(
            dump("-(1 + 2 - 3 * 4 // 5 % 6)"),
            "(block @0..25\n  (int-neg @0..25\n    (int-sub @2..24\n      (int-add @2..7\n        (int 1 @2..3)\n        (int 2 @6..7))\n      (int-rem @10..24\n        (int-floor-div @10..20\n          (int-mul @10..15\n            (int 3 @10..11)\n            (int 4 @14..15))\n          (int 5 @19..20))\n        (int 6 @23..24)))))"
        );
        assert_eq!(
            dump("-(1.0 + 2 - 3 * 4.0)"),
            "(block @0..20\n  (float-neg @0..20\n    (float-sub @2..19\n      (float-add @2..9\n        (float 1.0 @2..5)\n        (float 2.0 @8..9))\n      (float-mul @12..19\n        (float 3.0 @12..13)\n        (float 4.0 @16..19)))))"
        );
    }

    #[test]
    fn blocks_variables_and_statements() {
        let text = "let x = 1\n\
                    {\n  let y: Float = 2\n  y := y * 2.0\n}\n\
                    x := x + 1\n\
                    = { x }";
        // Slots are named by the last variable assigned to them: slot 1
        // holds `y` inside the block and then the rebound `x` after it, so
        // it shows as `x#1` throughout.
        assert_eq!(
            dump(text),
            "(block @0..66\n\
             \x20 (store x#0 @0..9\n\
             \x20   (int 1 @8..9))\n\
             \x20 (block @10..47\n\
             \x20   (store x#1 @14..30\n\
             \x20     (float 2.0 @29..30))\n\
             \x20   (store y#2 @33..45\n\
             \x20     (float-mul @38..45\n\
             \x20       (load x#1 @38..39)\n\
             \x20       (float 2.0 @42..45)))\n\
             \x20   (unit @10..47))\n\
             \x20 (store x#1 @48..58\n\
             \x20   (int-add @53..58\n\
             \x20     (load x#0 @53..54)\n\
             \x20     (int 1 @57..58)))\n\
             \x20 (block @61..66\n\
             \x20   (load x#1 @63..64)))"
        );
    }

    #[test]
    fn empty_file_is_an_empty_block_with_a_unit_result() {
        assert_eq!(dump(""), "(block @0..0\n  (unit @0..0))");
        assert_eq!(dump("# nothing\n"), "(block @0..0\n  (unit @0..0))");
        let ir = lower_text("");
        assert_eq!(ir.ty_of(ir.root()), Ty::Unit);
        assert_eq!(ir.frame_size(), 0);
    }

    #[test]
    fn frame_size_comes_from_the_resolution() {
        assert_eq!(
            lower_text("let a = 1\n{ let b = 2; let c = 3 }").frame_size(),
            3
        );
    }

    #[test]
    fn large_untyped_literal_rounds_to_nearest_float() {
        let ir = lower_text("9007199254740993 / 1");
        let IrKind::FloatDiv(operands) = ir.get(result_of(&ir)).kind else {
            panic!("expected FloatDiv");
        };
        assert_eq!(
            ir.get(operands.lhs).kind,
            IrKind::FloatConst(9_007_199_254_740_992.0)
        );
    }

    #[test]
    fn untyped_entry_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![Ty::UntypedInt, Ty::Int, Ty::Float]);
        let error = lower(&ast, &checked).unwrap_err();
        assert_eq!(error.message, "an untyped integer survived type checking");
        assert_eq!(error.span, Span::new(0, 1));
    }

    #[test]
    fn error_entry_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![Ty::Int, Ty::Error, Ty::Float]);
        let error = lower(&ast, &checked).unwrap_err();
        assert_eq!(error.message, "a type error survived type checking");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn impossible_result_type_is_an_internal_error() {
        let (ast, checked) = concrete_int_division(vec![Ty::Int, Ty::Int, Ty::Int]);
        let error = lower(&ast, &checked).unwrap_err();
        assert_eq!(error.message, "operator has a result type it cannot have");
        assert_eq!(error.span, Span::new(0, 5));
    }

    #[test]
    fn mismatched_operand_type_is_an_internal_error() {
        let ast = parse_text("1 + 2");
        let checked = Checked {
            types: Types::new(vec![Ty::Int, Ty::Float, Ty::Int]),
            resolution: Resolution::without_variables(3, 1),
        };
        let error = lower(&ast, &checked).unwrap_err();
        assert_eq!(error.message, "operand type does not match its operator");
        assert_eq!(error.span, Span::new(4, 5));
    }

    #[test]
    fn unresolved_variable_is_an_internal_error() {
        let ast = parse_text("let x = 1\nx");
        let checked = check(&ast).unwrap();
        let unresolved = Checked {
            types: checked.types.clone(),
            resolution: Resolution::without_variables(2, 2),
        };
        let error = lower(&ast, &unresolved).unwrap_err();
        assert_eq!(error.message, "a declaration has no slot");
        assert_eq!(error.span, Span::new(0, 9));
    }

    #[test]
    fn tables_of_the_wrong_size_are_an_internal_error() {
        let (ast, _) = concrete_int_division(Vec::new());
        let checked = Checked {
            types: Types::new(vec![Ty::Int]),
            resolution: Resolution::without_variables(3, 1),
        };
        let error = lower(&ast, &checked).unwrap_err();
        assert_eq!(error.message, "type table does not belong to this AST");
        let checked = Checked {
            types: Types::new(vec![Ty::Int, Ty::Int, Ty::Float]),
            resolution: Resolution::without_variables(3, 2),
        };
        let error = lower(&ast, &checked).unwrap_err();
        assert_eq!(error.message, "resolution does not belong to this AST");
    }
}
